//! U-26: the checkpoint review (R-49, R-50, R-55, R-56, R-58, C-18, D-29, D-30, D-35): a checkpoint does not
//! close before a cold review of the whole checkpoint ran on its content; the reviewer works in a copy of its
//! own, runs the checkpoint's command checks and its probes through the engine, files findings that open the
//! checkpoint fix, and judges a check that failed and then passed.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit, one checkpoint, a command check that the reviewer runs (`cmd`), and the review as a named
/// check. Only the review is switched on.
fn project(fx: &Fx, cmd: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{cmd}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it",
        "--scope",
        "default",
    ]);
    fx.human("Yes, the checkpoint review is on.");
    fx.must(&["checks", "set", "--quote", "the checkpoint review is on"]);
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
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-1",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "src/a.txt",
        ],
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

fn directive(fx: &Fx, action: &str) -> Option<Value> {
    let n = fx.must(&["next", "--json"]).json();
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == action)
        .cloned()
}

fn brief_text(fx: &Fx) -> String {
    let o = fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

/// The areas every report names, and how each was probed (R-70).
fn areas() -> Value {
    json!([
        {"area": "the close rule", "how": "read"},
        {"area": "the file check", "how": "built and run"}
    ])
}

fn findings(fx: &Fx, list: Value) -> Out {
    let body = json!({"areas": areas(), "findings": list});
    fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "K-1",
            "--role",
            "reviewer",
            "--result",
            "findings",
            "--findings",
            "-",
        ],
        &body.to_string(),
    )
}

/// A clean report: the areas probed and no findings.
fn clean(fx: &Fx) -> Out {
    findings(fx, json!([]))
}

#[test]
fn the_review_is_a_named_check_that_is_on_by_default() {
    let c = bf::checks::by_name("checkpoint-review").expect("in the catalogue");
    assert!(c.required);
    assert_eq!(c.stage, bf::checks::Stage::Built);
    assert_eq!(c.proof, bf::checks::Proof::Agent);
    assert_eq!(c.family.name(), "review");
}

#[test]
fn a_checkpoint_does_not_close_before_its_review_ran_on_the_content() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(
        o.stderr.contains("checkpoint review has not run")
            && o.stderr.contains("checkpoint-review"),
        "{}",
        o.stderr
    );
    // The verifier is asked for the command check; the reviewer has a directive of its own, on the strong tier.
    let d = directive(&fx, "review-checkpoint").expect("a reviewer directive");
    assert_eq!(d["role"], "reviewer", "{d}");
    assert_eq!(d["tier"], "strong", "{d}");
    // The verifier never settles the review's check.
    let v = directive(&fx, "run-checkpoint-checks").unwrap();
    assert_eq!(v["checks"], json!(["X-2"]), "{v}");
}

#[test]
fn the_reviewer_gets_the_whole_diff_every_units_brief_and_a_copy_of_its_own() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    let md = brief_text(&fx);
    assert!(md.contains("Brief: K-1 as reviewer"), "{md}");
    assert!(
        md.contains("The whole diff") && md.contains("K-1-review.diff"),
        "{md}"
    );
    assert!(md.contains("src/a.txt"), "{md}");
    assert!(
        md.contains("<summary>U-1</summary>") && md.contains("Brief: U-1 as builder"),
        "{md}"
    );
    assert!(md.contains("try to get around"), "{md}");
    assert!(
        md.contains("bf evidence run X-2 -- test -f src/a.txt"),
        "{md}"
    );
    assert!(md.contains("--probe K-1"), "{md}");
    assert!(
        md.contains("not run it again") || md.contains("at most once"),
        "{md}"
    );
    let copy = fx.work("K-1-review");
    assert!(copy.join("src/a.txt").is_file());
    // The copy holds the checkpoint's content, so a check run there counts for the checkpoint.
    let p = fx.project();
    assert_eq!(
        p.content_hash(&copy).unwrap(),
        p.content_hash(&fx.work("K-1")).unwrap()
    );
}

#[test]
fn a_clean_review_needs_the_command_checks_run_by_the_reviewer_and_then_the_checkpoint_closes() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let o = clean(&fx);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("bf evidence run X-2"), "{}", o.stderr);
    let copy = fx.work("K-1-review");
    fx.must_in(
        &copy,
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    // The same command on the same content is not run twice.
    let o = fx.bf_in(
        &copy,
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert_eq!(clean(&fx).code, 0);
    assert!(!copy.exists(), "the copy is thrown away");
    let c = fx.must(&["checkpoint", "close", "K-1", "--json"]).json();
    assert_eq!(c["closed"], true, "{c}");
    // The review is not asked for again.
    assert!(directive(&fx, "review-checkpoint").is_none());
}

#[test]
fn a_finding_needs_a_severity_and_a_file_and_opens_the_checkpoint_fix() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let o = findings(
        &fx,
        json!([{"title": "t", "severity": "huge", "where": "src/a.txt:1", "action": "a"}]),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("severity"), "{}", o.stderr);
    let o = findings(
        &fx,
        json!([{"title": "t", "severity": "high", "action": "a"}]),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("names no file"), "{}", o.stderr);
    let o = findings(
        &fx,
        json!([{"title": "no edge case", "severity": "high", "where": "src/a.txt:1", "action": "handle it"}]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = directive(&fx, "fix-checkpoint").expect("the fix opened");
    assert_eq!(d["role"], "builder", "{d}");
    assert!(!fx.work("K-1-review").exists());
    // The fix passes: the review runs again on the new content, in round 2, with the earlier findings.
    fx.pass_checkpoint_fix("changed by the fix\n");
    let md = brief_text(&fx);
    assert!(
        md.contains("round 2") && md.contains("Earlier rounds"),
        "{md}"
    );
    assert!(
        md.contains("no edge case") && md.contains("Outcome: fixed by"),
        "{md}"
    );
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(
        st["checkpoint_reviews"][0]["rounds"][0]["result"], "findings",
        "{st}"
    );
    assert_eq!(st["checkpoint_reviews"][0]["rounds"][1]["round"], 2, "{st}");
}

#[test]
fn the_review_runs_again_on_new_content_within_the_limit_of_rounds() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    let f = json!([{"title": "x", "severity": "high", "where": "src/a.txt:1", "action": "a"}]);
    for round in 1..=3 {
        brief_text(&fx);
        assert_eq!(findings(&fx, f.clone()).code, 0, "round {round}");
        if round < 3 {
            fx.pass_checkpoint_fix(&format!("changed by the fix {round}\n"));
        }
    }
    // Three reviews with a high finding still open: the checkpoint parks, no fix opens (R-72), the human
    // decides, and no fourth brief is written.
    assert!(directive(&fx, "fix-checkpoint").is_none());
    let d = directive(&fx, "decide-checkpoint-review").expect("the human decides");
    assert_eq!(d["role"], "human", "{d}");
    assert!(directive(&fx, "review-checkpoint").is_none());
    fx.refused(&["brief", "K-1", "--role", "reviewer"]);
}

#[test]
fn switched_off_on_the_humans_words_the_checkpoint_closes_and_says_why() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    // An agent cannot switch it off on its own.
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=off:faster",
        "--scope",
        "run",
    ]);
    let o = fx.refused(&["checks", "set", "--quote", "switch it off"]);
    assert!(!o.stderr.is_empty());
    fx.human("Yes, switch the checkpoint review off for this run, it is a throwaway.");
    fx.must(&[
        "checks",
        "set",
        "--quote",
        "switch the checkpoint review off for this run",
    ]);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let c = fx.must(&["checkpoint", "close", "K-1", "--json"]).json();
    let off = c["checks_off"].to_string();
    assert!(
        off.contains("checkpoint-review") && off.contains("faster"),
        "{c}"
    );
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert!(s["run_checks"].to_string().contains("faster"), "{s}");
}

#[test]
fn no_one_but_the_reviewer_settles_the_review() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    let f = fx.root.join("proof.txt");
    fs::write(&f, "looked\n").unwrap();
    let o = fx.bf_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "attach",
            "X-3",
            f.to_str().unwrap(),
            "--outcome",
            "met",
            "--says",
            "I looked",
        ],
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.stderr.contains("settled only by the checkpoint reviewer"),
        "{}",
        o.stderr
    );
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["evidence", "fail", "X-3", "--reason", "no"],
    );
    assert_ne!(o.code, 0, "{}", o.all());
}

#[test]
fn a_probe_is_recorded_and_settles_nothing() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    // Outside the reviewer's copy there is no probe.
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["evidence", "run", "--probe", "K-1", "--", "true"],
    );
    assert_ne!(o.code, 0, "{}", o.all());
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    let o = fx.must_in(
        &copy,
        &[
            "evidence",
            "run",
            "--probe",
            "K-1",
            "--json",
            "--",
            "sh",
            "-c",
            "echo probing; exit 3",
        ],
    );
    let v = o.json();
    assert_eq!(v["type"], "probe", "{v}");
    assert_eq!(v["exit"], 3, "{v}");
    assert!(
        v["output_tail"].as_str().unwrap().contains("probing"),
        "{v}"
    );
    assert!(
        v["command"].as_str().unwrap().contains("echo probing"),
        "{v}"
    );
    assert_eq!(v["content_hash"].as_str().unwrap().len(), 40, "{v}");
    // A probe that passes proves nothing: no receipt counts, and verification lists none.
    fx.must_in(&copy, &["evidence", "run", "--probe", "K-1", "--", "true"]);
    let p = fx.project();
    let receipts = bf::evidence::load_receipts(&p).unwrap();
    assert!(receipts.iter().filter(|r| r.v["type"] == "probe").count() == 2);
    assert!(receipts
        .iter()
        .all(|r| r.v["type"] != "probe" || !r.passes()));
    let content = p.content_hash(&fx.work("K-1")).unwrap();
    assert!(bf::evidence::satisfied_at(&receipts, "K-1", &content).is_none());
    // A probe file in the copy changes its content, so a check run there is not the checkpoint's.
    fs::write(copy.join("probe_test.txt"), "x\n").unwrap();
    fx.must_in(
        &copy,
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let receipts = bf::evidence::load_receipts(&p).unwrap();
    assert!(bf::evidence::satisfied_at(&receipts, "X-2", &content).is_none());
    let o = clean(&fx);
    assert_ne!(o.code, 0, "{}", o.all());
}

#[test]
fn the_reviewers_copy_cannot_move_the_checkpoint_branch() {
    let prof = bf::sandbox::review_profile("(version 1)\n", &["/repo".to_string()]);
    assert!(prof.contains("/repo/.git/refs"), "{prof}");
    assert!(prof.contains("/repo/.git/packed-refs"), "{prof}");
    assert!(prof.contains("/repo/.git/HEAD"), "{prof}");
    let work = std::path::Path::new("/p/.bf/work");
    assert!(bf::sandbox::is_review_copy(
        work,
        &work.join("K-1-review/sub")
    ));
    assert!(!bf::sandbox::is_review_copy(work, &work.join("K-1")));
    assert!(!bf::sandbox::is_review_copy(work, &work.join("U-1-review")));
}

#[test]
fn a_pass_after_a_failure_counts_only_with_the_reviewers_judgement() {
    for (judgement, counts, flagged) in [
        ("flaky", true, true),
        ("other", true, false),
        ("failure", false, false),
    ] {
        let fx = Fx::new();
        let flag = fx.root.parent().unwrap().join("flag");
        project(&fx, &format!("test -f {}", flag.display()));
        brief_text(&fx);
        let copy = fx.work("K-1-review");
        let cmd = [
            "test".to_string(),
            "-f".to_string(),
            flag.display().to_string(),
        ];
        let mut a: Vec<&str> = vec!["evidence", "run", "X-2", "--"];
        a.extend(cmd.iter().map(|x| x.as_str()));
        // First it fails; then, on a deliberate repeat, it passes.
        assert_eq!(fx.bf_in(&copy, &a).code, 1);
        fs::write(&flag, "x").unwrap();
        let o = fx.bf_in(&copy, &a);
        assert_ne!(
            o.code,
            0,
            "a repeat is refused without its number: {}",
            o.all()
        );
        assert!(
            o.stderr.contains("already ran 1 time") && o.stderr.contains("--attempt 2"),
            "{}",
            o.stderr
        );
        let mut b: Vec<&str> = vec![
            "evidence",
            "run",
            "X-2",
            "--attempt",
            "2",
            "--reason",
            "the file was not there yet",
            "--",
        ];
        b.extend(cmd.iter().map(|x| x.as_str()));
        fx.must_in(&copy, &b);
        // Without a judgement the pass does not count, and the review cannot be clean.
        let p = fx.project();
        let content = p.content_hash(&fx.work("K-1")).unwrap();
        let rs = bf::evidence::load_receipts(&p).unwrap();
        assert!(bf::evidence::satisfied_at(&rs, "X-2", &content).is_none());
        assert_ne!(clean(&fx).code, 0);
        let o = fx.bf_in(
            &copy,
            &["evidence", "run", "X-2", "--judge", judgement, "--json"],
        );
        assert_ne!(o.code, 0, "a judgement needs a reason: {}", o.all());
        let o = fx.must_in(
            &copy,
            &[
                "evidence",
                "run",
                "X-2",
                "--judge",
                judgement,
                "--reason",
                "it depends on a file that came late",
                "--json",
            ],
        );
        assert!(o.json()["reason"].as_str().unwrap().starts_with(judgement));
        let rs = bf::evidence::load_receipts(&p).unwrap();
        assert_eq!(
            bf::evidence::satisfied_at(&rs, "X-2", &content).is_some(),
            counts,
            "{judgement}"
        );
        let flaky_flag = rs.iter().any(|r| r.v["flag"] == "flaky");
        assert_eq!(flaky_flag, flagged, "{judgement}");
        if counts {
            let o = clean(&fx);
            assert_eq!(o.code, 0, "{}", o.all());
            fx.must(&["checkpoint", "close", "K-1"]);
        } else {
            // The failure stands: the check goes to the checkpoint fix.
            assert_ne!(clean(&fx).code, 0);
            let d = directive(&fx, "fix-checkpoint").expect("the fix opened");
            assert_eq!(d["failing_checks"][0], "X-2", "{d}");
        }
    }
}

#[test]
fn the_checkpoint_page_shows_the_review_rounds_and_the_flaky_flag() {
    let fx = Fx::new();
    let flag = fx.root.parent().unwrap().join("flag-page");
    project(&fx, &format!("test -f {}", flag.display()));
    let page = |fx: &Fx| {
        let v = fx.must(&["view", "checkpoint", "K-1"]).json();
        v["sections"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "K-1-review")
            .map(|r| r["body"].to_string())
            .expect("a review row")
    };
    let before = page(&fx);
    assert!(before.contains("Review rounds: none yet"), "{before}");
    assert!(before.contains("Flaky: none flagged"), "{before}");
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    let cmd = [
        "test".to_string(),
        "-f".to_string(),
        flag.display().to_string(),
    ];
    let mut a: Vec<&str> = vec!["evidence", "run", "X-2", "--"];
    a.extend(cmd.iter().map(|x| x.as_str()));
    assert_eq!(fx.bf_in(&copy, &a).code, 1);
    fs::write(&flag, "x").unwrap();
    let mut b: Vec<&str> = vec![
        "evidence",
        "run",
        "X-2",
        "--attempt",
        "2",
        "--reason",
        "late file",
        "--",
    ];
    b.extend(cmd.iter().map(|x| x.as_str()));
    fx.must_in(&copy, &b);
    fx.must_in(
        &copy,
        &[
            "evidence",
            "run",
            "X-2",
            "--judge",
            "flaky",
            "--reason",
            "late file",
            "--json",
        ],
    );
    let after = page(&fx);
    assert!(after.contains("Review round 1 judged content"), "{after}");
    assert!(after.contains("Flaky: X-2"), "{after}");
}

// ---- F-100: the guard's zone of the reviewer's copy

fn write_decision(fx: &Fx, cwd: &std::path::Path, path: &std::path::Path) -> (String, String) {
    let payload = json!({
        "tool_name": "Write",
        "tool_input": {"file_path": path.to_string_lossy(), "content": "x\n"},
        "cwd": cwd.to_string_lossy(),
    });
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => ("pass".into(), String::new()),
        s => {
            let v: Value = serde_json::from_str(s).unwrap();
            let h = &v["hookSpecificOutput"];
            (
                h["permissionDecision"].as_str().unwrap().to_string(),
                h["permissionDecisionReason"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
        }
    }
}

#[test]
fn the_reviewer_writes_freely_in_its_own_copy() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    let (d, why) = write_decision(&fx, &copy, &copy.join("src/probe_test.rs"));
    assert_ne!(d, "deny", "{why}");
}

#[test]
fn a_unit_copy_or_the_checkpoint_copy_cannot_write_into_the_reviewers_copy() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    let target = copy.join("src/a.txt");
    for from in [fx.work("U-1"), fx.work("K-1")] {
        // A landed unit's copy is removed after its landing: the folder only stands in for where the writer is.
        std::fs::create_dir_all(&from).unwrap();
        let (d, why) = write_decision(&fx, &from, &target);
        assert_eq!(d, "deny", "from {}: {why}", from.display());
        assert!(why.contains("reviewer's own copy"), "{why}");
    }
}

#[test]
fn protected_paths_inside_the_reviewers_copy_stay_denied_to_the_reviewer() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    let (d, _) = write_decision(&fx, &copy, &copy.join("factory/records/R-1.toml"));
    assert_eq!(d, "deny");
    let (d, _) = write_decision(&fx, &copy, &copy.join(".git/config"));
    assert_eq!(d, "deny");
}

#[test]
fn checkpoint_review_settles_when_every_open_finding_is_accepted_and_asks_for_no_further_round() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let o = findings(
        &fx,
        json!([
            {"title": "no edge case", "severity": "medium", "where": "src/a.txt:1", "action": "handle it"},
            {"title": "a name", "severity": "low", "where": "src/a.txt:1", "action": "rename it"}
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    assert!(directive(&fx, "decide-checkpoint-fix").is_some());
    fx.human("The edge case never happens here, accept both.");
    fx.must(&[
        "checkpoint", "finding", "K-1", "--accept", "r1.1,r1.2", "--quote", "accept both",
    ]);
    // Every finding is accepted: the review is settled, no round 2 is asked for, nobody is asked again.
    assert!(directive(&fx, "review-checkpoint").is_none());
    assert!(directive(&fx, "decide-checkpoint-fix").is_none());
    assert!(directive(&fx, "decide-low-findings").is_none());
    // The engine goes on to the checkpoint's own checks, with no further review round.
    let run = directive(&fx, "run-checkpoint-checks").expect("the checks run next");
    assert_eq!(run["checks"], json!(["X-2"]), "{run}");
}

/// A reopen of the findings of `from` (an opening in the log) that cites `from` as the earlier words.
fn again_event(from: Value) -> Value {
    json!({"checkpoint": "K-1", "source": "review", "failed": [], "findings": from["findings"],
        "finding_ids": from["finding_ids"],
        "human": {"quote": from["human"]["quote"], "earlier": from["seq"]},
        "allowed": [], "test_dirs": [], "units": [], "before": [], "content": from["content"],
        "wave": 0, "from_saved": "", "opened_by": "reopen"})
}

#[test]
fn checkpoint_review_a_fix_reopened_after_an_abort_keeps_the_humans_earlier_words() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let o = findings(
        &fx,
        json!([{"title": "no edge case", "severity": "high", "where": "src/a.txt:1", "action": "handle it"}]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    // The fix was opened by the report, not by the human: no earlier words, so words are needed.
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    fx.human("Yes, fix the edge case.");
    fx.must(&[
        "checkpoint", "finding", "K-1", "--fix-findings", "r1.1", "--quote", "fix the edge case",
    ]);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    // The same findings again need no new words: the human's earlier words stand.
    let o = fx.must(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1"]);
    assert!(o.stdout.contains("a fix opened"), "{}", o.all());
    let open: Vec<Value> = fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == "checkpoint.fix.open" && e["human"]["quote"].is_string())
        .collect();
    assert_eq!(open.len(), 2, "{open:?}");
    assert_eq!(open[0]["opened_by"], "pick", "{open:?}");
    assert_eq!(open[1]["opened_by"], "reopen", "{open:?}");
    assert_eq!(open[1]["human"]["earlier"], open[0]["seq"], "{open:?}");
    assert_eq!(open[0]["human"]["quote"], open[1]["human"]["quote"], "{open:?}");
    // The replay counts it: a fix is open for the finding, and no event of the log is ignored.
    let st = bf::events::replay(&fx.project()).unwrap();
    let f = st.ck_fix.expect("the reopened fix is open in the replayed state");
    assert_eq!(f.phase, "fix");
    assert_eq!(f.origin_ids, vec![(1, 1)]);
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
    assert!(directive(&fx, "fix-checkpoint").is_some());
    // A second reopen with no abort between, and a reopen written while a fix is at work, are ignored: the
    // reopened fix stays as it is, and one abort pays for one reopen.
    let first_open_seq = f.open_seq;
    let again = again_event(open[1].clone());
    bf::events::append(&fx.project(), "checkpoint.fix.open", again).unwrap();
    let st = bf::events::replay(&fx.project()).unwrap();
    assert_eq!(st.ck_fix.unwrap().open_seq, first_open_seq, "a second reopen replaced the open fix");
    assert!(
        st.ignored.iter().any(|x| x.contains("earlier words")),
        "{:?}",
        st.ignored
    );
    // No fix is open and no abort came after the latest counted opening (the fix ended on its own): a reopen
    // citing that opening is not counted either, so the abort rule stands on its own, apart from the holds.
    let p = fx.project();
    bf::events::append(
        &p,
        "checkpoint.fix.done",
        json!({"checkpoint": "K-1", "files": [], "units": [], "by_file": {}, "marks": {}}),
    )
    .unwrap();
    let st = bf::events::replay(&p).unwrap();
    assert!(st.ck_fix.is_none(), "the fix did not end");
    let ignored_before = st.ignored.len();
    bf::events::append(&p, "checkpoint.fix.open", again_event(open[1].clone())).unwrap();
    let st = bf::events::replay(&p).unwrap();
    assert!(st.ck_fix.is_none(), "a reopen with no abort since the opening was counted");
    assert_eq!(st.ignored.len(), ignored_before + 1, "{:?}", st.ignored);
}

#[test]
fn checkpoint_review_a_reopen_citing_earlier_words_that_are_no_aborted_opening_is_ignored() {
    let fx = Fx::new();
    project(&fx, "test -f src/a.txt");
    brief_text(&fx);
    let o = findings(
        &fx,
        json!([{"title": "no edge case", "severity": "high", "where": "src/a.txt:1", "action": "handle it"}]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    fx.human("Yes, fix the edge case.");
    fx.must(&[
        "checkpoint", "finding", "K-1", "--fix-findings", "r1.1", "--quote", "fix the edge case",
    ]);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let log: Vec<Value> = fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    let open = log
        .iter()
        .rev()
        .find(|e| e["kind"] == "checkpoint.fix.open")
        .unwrap()
        .clone();
    let p = fx.project();
    let fake = |how: &str, human: Value| {
        json!({"checkpoint": "K-1", "source": "review", "failed": [], "findings": open["findings"],
               "finding_ids": open["finding_ids"], "human": human, "allowed": [],
               "test_dirs": [], "units": [], "before": [], "content": open["content"], "wave": 0,
               "from_saved": "", "opened_by": how})
    };
    let quote = open["human"]["quote"].clone();
    // A reopen citing an opening that never existed.
    bf::events::append(
        &p,
        "checkpoint.fix.open",
        fake("reopen", json!({"quote": quote, "earlier": 999_999})),
    )
    .unwrap();
    let st = bf::events::replay(&p).unwrap();
    assert!(st.ck_fix.is_none(), "a forged reopen opened a fix");
    assert!(
        st.ignored.iter().any(|x| x.contains("earlier words")),
        "{:?}",
        st.ignored
    );
    // An opening that is in the log but that the replay ignored (it cites a turn already used) is no earlier
    // words either, even when a later fix of the same findings is aborted.
    // A pick that cites a turn already used is in the log but the replay ignores it.
    bf::events::append(&p, "checkpoint.fix.open", fake("pick", open["human"].clone())).unwrap();
    let ignored_seq = bf::events::load_log(&p).unwrap().events.last().unwrap()["seq"]
        .as_u64()
        .unwrap();
    let st = bf::events::replay(&p).unwrap();
    assert!(st.ck_fix.is_none(), "a pick that cites a used turn opened a fix");
    // A genuine fix of the same findings, opened on new words and aborted, comes after the ignored opening.
    fx.human("Once more, fix the edge case.");
    fx.must(&[
        "checkpoint", "finding", "K-1", "--fix-findings", "r1.1", "--quote", "fix the edge case",
    ]);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    bf::events::append(
        &p,
        "checkpoint.fix.open",
        fake("reopen", json!({"quote": quote, "earlier": ignored_seq})),
    )
    .unwrap();
    let st = bf::events::replay(&p).unwrap();
    assert!(st.ck_fix.is_none(), "an ignored opening gave its words again");
}
