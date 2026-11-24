//! R-71, R-95: findings are accepted only while the review that filed them is what waits for the human. A unit
//! or a checkpoint fix parked while it was being fixed holds work no reviewer saw, so an accept of the
//! findings of the earlier review does not make it reviewed.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

// ---- a unit parked while fixing ----

fn finding(fx: &Fx, id: &str, severity: &str) {
    let o = fx.put(
        "finding",
        &format!("id=\"{id}\"\nkind=\"finding\"\ntitle=\"t {id}\"\nunit=\"U-1\"\nseverity=\"{severity}\"\n"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

fn state_of(fx: &Fx, id: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["units"].as_array().unwrap().iter().find(|u| u["id"] == id).unwrap()["state"]
        .as_str()
        .unwrap()
        .to_string()
}

fn unit_with_findings(to: &str) -> Fx {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    finding(&fx, "F-1", "high");
    bf::events::append(
        &fx.project(),
        "unit.report",
        json!({"unit": "U-1", "result": "findings", "from": "review", "to": to,
               "findings": ["F-1"], "round": 1, "content": "c", "head": "h", "covered": "all"}),
    )
    .unwrap();
    fx
}

#[test]
fn accept_park_review_a_unit_parked_while_fixing_cannot_have_its_findings_accepted() {
    let fx = unit_with_findings("fixing");
    bf::events::append(
        &fx.project(),
        "unit.park",
        json!({"unit": "U-1", "reason": "idle for 90 minutes"}),
    )
    .unwrap();
    let words = "I accept that finding as it is";
    fx.human(words);
    let o = fx.bf(&["unit", "accept", "U-1", "F-1", "--quote", words]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("idle for 90 minutes"), "{}", o.all());
    assert_eq!(state_of(&fx, "U-1"), "parked");
}

#[test]
fn accept_park_review_an_accept_written_by_hand_for_a_unit_parked_while_fixing_changes_nothing() {
    let fx = unit_with_findings("fixing");
    let p = fx.project();
    bf::events::append(&p, "unit.park", json!({"unit": "U-1", "reason": "idle for 90 minutes"})).unwrap();
    let words = "I accept that finding as it is";
    fx.human(words);
    let turn = bf::events::replay(&p).unwrap().turns.last().unwrap().seq;
    bf::events::append(
        &p,
        "unit.finding.accept",
        json!({"unit": "U-1", "findings": [{"id": "F-1", "severity": "high"}], "records_only": [],
               "turn": turn, "quote": words}),
    )
    .unwrap();
    assert_eq!(state_of(&fx, "U-1"), "parked");
}

#[test]
fn accept_park_review_a_unit_parked_in_its_review_still_has_its_findings_accepted() {
    let fx = unit_with_findings("parked");
    bf::events::append(
        &fx.project(),
        "unit.park",
        json!({"unit": "U-1", "reason": "more than 3 review rounds"}),
    )
    .unwrap();
    let words = "I accept that finding as it is";
    fx.human(words);
    fx.must(&["unit", "accept", "U-1", "F-1", "--quote", words]);
    assert_eq!(state_of(&fx, "U-1"), "reviewed");
}

// ---- a checkpoint fix parked by an attempt or a park ----

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

fn build(fx: &Fx, k: &Path, text: &str) {
    fs::write(k.join("src/a.txt"), text).unwrap();
    git(k, &["add", "-A"]);
    git(k, &["commit", "-q", "-m", "the fix"]);
    fx.must_in(k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    fx.must_in(k, &["report", "K-1", "--role", "builder", "--result", "built"]);
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
fn accept_park_review_a_fix_parked_by_a_park_cannot_have_the_last_reviews_findings_accepted() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    // `bf next` opens the fix of the failed check.
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    build(&fx, &k, "fixed\n");
    let low = |t: &str| json!({"title": t, "severity": "low", "where": "src/a.txt", "action": "tidy"});
    let o = fx.bf_stdin(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"],
        &json!([low("name"), low("comment")]).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The fix is then parked for another reason, while the review's findings are still listed.
    bf::events::append(
        &fx.project(),
        "checkpoint.fix.park",
        json!({"checkpoint": "K-1", "reason": "idle for 90 minutes"}),
    )
    .unwrap();
    fx.human("Both are fine, accept them.");
    let o = fx.bf(&[
        "checkpoint", "finding", "K-1", "--accept", "f1.1,f1.2", "--quote", "Both are fine, accept them.",
    ]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("idle for 90 minutes"), "{}", o.all());
    assert!(events(&fx, "checkpoint.fix.finding.settle").is_empty());
    assert!(events(&fx, "checkpoint.fix.done").is_empty());
}
