//! U-50 (R-32): `run_k_checks` does not run again a check that already passed on this content, and the log
//! holds no new run of its command. (The refusal of a missing probe before any check runs is in `ckfix.rs`,
//! which this unit does not own: it is pinned with the later K-1 fix.)

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const K_CHECK: &str = "grep -q fixed src/a.txt";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\nfull_suite=\"fixture\"\n".to_string()),
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

/// How many runs of X-2 the log holds.
fn x2_runs_in_log(fx: &Fx) -> usize {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == "evidence.run" && e["check"] == "X-2")
        .count()
}

#[test]
fn run_k_checks_skips_a_check_that_passed_on_this_content_and_the_log_holds_no_new_run() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    // The verifier's run fails on the content as landed.
    let k = fx.work("K-1");
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "the fix"]);
    // The fix's builder runs the check through the engine.
    fx.must_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    let before = x2_runs_in_log(&fx);
    assert!(before >= 2, "the runs were not logged: {before}");
    // The report runs run_k_checks on the same content: no new run of the command.
    fx.must_in(&k, &["report", "K-1", "--role", "builder", "--result", "built"]);
    assert_eq!(x2_runs_in_log(&fx), before, "the report ran X-2 again");
}

#[test]
fn run_k_checks_runs_a_check_again_when_the_content_changed() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "the fix"]);
    // No passing receipt on this content: the report must run the check.
    let before = x2_runs_in_log(&fx);
    fx.must_in(&k, &["report", "K-1", "--role", "builder", "--result", "built"]);
    assert_eq!(x2_runs_in_log(&fx), before + 1, "the report did not run X-2 on a new content");
}

#[test]
fn run_k_checks_runs_a_check_again_when_its_record_changed_and_its_command_did_not() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "the fix"]);
    fx.must_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    // The check record changes (its title), its command text stays the same: the receipt is stale (R-32).
    let opened = fx.must(&["amend", "open", "--repair", "X-2", "--json"]).json();
    let rec = fx.root.join("x2-changed.toml");
    fs::write(
        &rec,
        format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2 changed\"\nhow=\"command\"\ncommand=\"{K_CHECK}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
    )
    .unwrap();
    fx.must(&["record", "put", "check", "--file", rec.to_str().unwrap(), "--replace"]);
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(&f, serde_json::json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, approve the change to the check.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    let before = x2_runs_in_log(&fx);
    fx.must_in(&k, &["report", "K-1", "--role", "builder", "--result", "built"]);
    assert_eq!(x2_runs_in_log(&fx), before + 1, "the report skipped X-2 on a stale receipt");
}
