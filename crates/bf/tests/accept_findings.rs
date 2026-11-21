//! R-71, D-89: a checkpoint-fix review that files only low findings asks the human to accept them, the human
//! accepts any finding of a parked fix review in their own words, a fix reopened after an abort keeps the
//! human's earlier words, and the engine goes on when every finding is accepted.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const K_CHECK: &str = "grep -q fixed src/a.txt";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{K_CHECK}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn land_unit(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

fn directive(fx: &Fx) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "one directive while a fix is open: {n}");
    ds[0].clone()
}

/// A fix of the failed checkpoint check, built and waiting for its review.
fn fix_in_review(fx: &Fx) -> PathBuf {
    plan(fx);
    land_unit(fx);
    let k = fx.work("K-1");
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    assert_eq!(directive(fx)["action"], "fix-checkpoint");
    build(fx, &k, "fixed\n");
    k
}

fn build(fx: &Fx, k: &Path, text: &str) {
    fs::write(k.join("src/a.txt"), text).unwrap();
    git(k, &["add", "-A"]);
    git(k, &["commit", "-q", "-m", "the fix"]);
    fx.must_in(k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    fx.must_in(k, &["report", "K-1", "--role", "builder", "--result", "built"]);
}

fn review(fx: &Fx, k: &Path, findings: Value) -> Out {
    let o = fx.bf_stdin(
        k,
        &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"],
        &findings.to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    o
}

fn low(title: &str) -> Value {
    json!({"title": title, "severity": "low", "where": "src/a.txt", "action": "tidy"})
}

fn medium() -> Value {
    json!({"title": "edge case", "severity": "medium", "where": "src/a.txt", "action": "handle it"})
}

fn events(fx: &Fx, kind: &str) -> Vec<Value> {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == kind)
        .collect()
}

#[test]
fn accept_findings_a_fix_review_that_files_only_lows_ends_in_decide_low_findings_and_the_accept_goes_on() {
    let fx = Fx::new();
    let k = fix_in_review(&fx);
    let o = review(&fx, &k, json!([low("name"), low("comment")]));
    // The report names the phase, never an empty state.
    assert!(o.stdout.contains("K-1 is now low"), "{}", o.stdout);
    let d = directive(&fx);
    assert_eq!(d["action"], "decide-low-findings", "{d}");
    assert_eq!(d["role"], "human", "{d}");
    assert_eq!(d["findings"][0]["id"], "f1.1", "{d}");
    assert_eq!(d["findings"][1]["id"], "f1.2", "{d}");
    // The builder's and the reviewer's reports wait for the human.
    let o = fx.bf_in(&k, &["report", "K-1", "--role", "builder", "--result", "built"]);
    assert_ne!(o.code, 0, "{}", o.all());
    // Words written before the review decide nothing; the human's words after it do.
    fx.human("Both are fine, accept them.");
    let o = fx.must(&[
        "checkpoint", "finding", "K-1", "--accept", "low", "--quote", "Both are fine, accept them.", "--json",
    ]);
    let v = o.json();
    assert_eq!(v["settled_review"], true, "{v}");
    assert_eq!(v["fix"]["phase"], "done", "{v}");
    // Each accept is logged with the finding, its level and the words.
    let log = events(&fx, "checkpoint.fix.finding.settle");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["levels"], json!(["low", "low"]));
    assert_eq!(log[0]["quote"], "Both are fine, accept them.");
    assert_eq!(events(&fx, "checkpoint.fix.done").len(), 1);
    // No further review round was asked for.
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["action"] != "review-checkpoint-fix" && d["action"] != "fix-checkpoint"),
        "{n}"
    );
}

#[test]
fn accept_findings_a_small_low_finding_goes_back_to_the_builder_with_no_human_step() {
    let fx = Fx::new();
    let k = fix_in_review(&fx);
    let mut small = low("a test is missing");
    small["small"] = json!(true);
    let o = review(&fx, &k, json!([small]));
    assert!(o.stdout.contains("K-1 is now fix"), "{}", o.stdout);
    let d = directive(&fx);
    assert_eq!(d["action"], "fix-checkpoint", "{d}");
    assert_eq!(d["role"], "builder", "{d}");
}

#[test]
fn accept_findings_a_medium_finding_of_a_parked_fix_review_is_accepted_in_the_humans_words() {
    let fx = Fx::new();
    let k = fix_in_review(&fx);
    fx.set_policy("max_rounds = 3", "max_rounds = 1");
    review(&fx, &k, json!([medium()]));
    assert_eq!(directive(&fx)["action"], "fix-checkpoint");
    build(&fx, &k, "fixed again\n");
    review(&fx, &k, json!([medium()]));
    let d = directive(&fx);
    assert_eq!(d["action"], "decide-checkpoint-fix", "{d}");
    // No words, no accept.
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--accept", "f2.1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("--quote"), "{}", o.stderr);
    // A finding the review never filed is refused.
    fx.human("It is a known edge, accept it.");
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--accept", "f2.9", "--quote", "accept it"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("f2.9"), "{}", o.stderr);
    // A fix's finding cannot be named together with the checkpoint review's.
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--accept", "f2.1,r1.1", "--quote", "accept it"]);
    assert_eq!(o.code, 2, "{}", o.all());
    let v = fx
        .must(&["checkpoint", "finding", "K-1", "--accept", "f2.1", "--quote", "accept it", "--json"])
        .json();
    assert_eq!(v["fix"]["phase"], "done", "{v}");
    let log = events(&fx, "checkpoint.fix.finding.settle");
    assert_eq!(log.last().unwrap()["quote"], "accept it");
    // A fix's finding is accepted, never declined.
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--decline", "f2.1", "--reason", "no", "--quote", "accept it"]);
    assert_eq!(o.code, 2, "{}", o.all());
}

#[test]
fn accept_findings_a_second_round_skips_a_check_that_passed_and_the_log_shows_the_receipt() {
    let fx = Fx::new();
    let k = fix_in_review(&fx);
    review(&fx, &k, json!([medium()]));
    build(&fx, &k, "fixed more\n");
    let skips = events(&fx, "checkpoint.check.skip");
    assert!(skips.len() >= 2, "{skips:?}");
    for e in &skips {
        assert_eq!(e["check"], "X-2", "{e}");
        assert!(e["receipt"].as_str().unwrap_or("").len() > 3, "{e}");
        assert!(e["note"].as_str().unwrap().contains("not run again"), "{e}");
    }
}

#[test]
fn accept_findings_the_human_releases_a_low_fix_and_the_builder_gets_the_remaining_lows() {
    let fx = Fx::new();
    let k = fix_in_review(&fx);
    review(&fx, &k, json!([low("name"), low("comment")]));
    assert_eq!(directive(&fx)["action"], "decide-low-findings");
    fx.human("Accept the name one, fix the other.");
    fx.must(&[
        "checkpoint", "finding", "K-1", "--accept", "f1.1", "--quote", "Accept the name one",
    ]);
    // One low is still open: the fix still waits for the human.
    let d = directive(&fx);
    assert_eq!(d["action"], "decide-low-findings", "{d}");
    assert_eq!(d["findings"].as_array().unwrap().len(), 1, "{d}");
    fx.human("Fix the other one.");
    fx.must(&["checkpoint", "fix", "K-1", "--release", "--quote", "Fix the other one."]);
    let d = directive(&fx);
    assert_eq!(d["action"], "fix-checkpoint", "{d}");
    assert_eq!(d["findings"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["findings"][0]["title"], "comment", "{d}");
}

#[test]
fn accept_findings_a_stop_after_the_accept_leaves_a_fix_that_goes_on_with_no_further_round() {
    let fx = Fx::new();
    let k = fix_in_review(&fx);
    review(&fx, &k, json!([low("name")]));
    fx.human("Fine, accept it.");
    // The accept is on the log, and the process stopped before the checks ran: only the event is written.
    let turn = fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|e| e["kind"] == "human.turn")
        .unwrap()["seq"]
        .as_u64()
        .unwrap();
    let p = fx.project();
    bf::events::append(
        &p,
        "checkpoint.fix.finding.settle",
        json!({"checkpoint": "K-1", "fate": "accepted", "ids": [{"round": 1, "index": 1}],
               "levels": ["low"], "turn": turn, "quote": "Fine, accept it."}),
    )
    .unwrap();
    let st = bf::events::replay(&p).unwrap();
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
    // The next directive is not a review round: it carries the fix on.
    let d = directive(&fx);
    assert_eq!(d["action"], "go-on-checkpoint-fix", "{d}");
    // A report that is not `built` is refused; `built` runs the checks and ends the fix.
    let o = fx.bf_in(&k, &["report", "K-1", "--result", "blocked"]);
    assert_ne!(o.code, 0, "{}", o.all());
    let o = fx.must_in(&k, &["report", "K-1", "--result", "built", "--json"]);
    assert_eq!(o.json()["phase"], "done", "{}", o.stdout);
    assert_eq!(events(&fx, "checkpoint.fix.done").len(), 1);
}
