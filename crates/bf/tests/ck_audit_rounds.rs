//! U-132 (R-77, D-159, TH-21): an agent check of a checkpoint that judges the built result is an audit whose
//! rounds are review rounds of the checkpoint: the round takes the next number and carries the check as its
//! source, its findings are `r<round>.<number>`, the engine writes the check's receipt, and the human decides
//! the findings by their ids.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit, one checkpoint listing a command check, the review (X-3), a security audit (X-4), a legal audit
/// (X-5) and the `extra` agent checks `(id, named)`.
fn project(fx: &Fx, extra: &[(&str, &str)]) {
    let mut ids = vec!["X-2", "X-3", "X-4", "X-5"]
        .into_iter()
        .map(|x| format!("\"{x}\""))
        .collect::<Vec<_>>();
    let mut recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("journey", "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\nsteps=[\"open\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\",\"J-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".to_string()),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"security\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"security-audit\"\n".to_string()),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"legal\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"legal-audit\"\n".to_string()),
    ];
    for (id, named) in extra {
        ids.push(format!("\"{id}\""));
        recs.push(("check", format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"{named}\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"{named}\"\n")));
    }
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\",\"J-1\"]\nchecks=[{}]\n", ids.join(","))));
    for (k, t) in recs {
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
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

/// The checkpoint review runs and files nothing: the audits may start.
fn pass_review(fx: &Fx) {
    fx.must(&["brief", "K-1", "--role", "reviewer"]);
    fx.must_in(&fx.work("K-1-review"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"]);
    let o = fx.bf_stdin(
        &fx.root,
        &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"],
        &json!({"areas": areas(), "findings": []}).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

fn areas() -> Value {
    json!([{"area": "the close rule", "how": "read"}])
}

fn finding(title: &str, severity: &str) -> Value {
    json!({"title": title, "severity": severity, "where": "src/a.txt:1", "action": "change it"})
}

fn audit_brief(fx: &Fx, role: &str, check: &str) -> Out {
    fx.bf(&["brief", "K-1", "--role", role, "--check", check, "--json"])
}

fn audit_report(fx: &Fx, role: &str, check: &str, body: Value, token: Option<&str>) -> Out {
    let mut args = vec![
        "report", "K-1", "--role", role, "--check", check, "--result", "findings", "--findings", "-", "--json",
    ];
    if let Some(t) = token {
        args.extend(["--token", t]);
    }
    fx.bf_stdin(&fx.root, &args, &body.to_string())
}

fn rounds(fx: &Fx) -> Vec<Value> {
    fx.must(&["status", "--json"]).json()["checkpoint_reviews"][0]["rounds"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn reviews(fx: &Fx) -> u64 {
    fx.must(&["status", "--json"]).json()["checkpoint_reviews"][0]["reviews"]
        .as_u64()
        .unwrap()
}

/// The receipts the engine wrote for a check.
fn receipts_of(fx: &Fx, check: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for e in fs::read_dir(fx.root.join("factory/receipts")).unwrap().flatten() {
        if e.path().extension().and_then(|x| x.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(v) = toml::from_str::<Value>(&fs::read_to_string(e.path()).unwrap_or_default()) {
            if v["check"] == check && v["type"] == "attach" {
                out.push(v);
            }
        }
    }
    out
}

fn log_text(fx: &Fx) -> String {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap()
}

#[test]
fn an_audit_round_takes_the_next_round_number_and_its_source() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    let o = audit_brief(&fx, "security-auditor", "X-4");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = audit_brief(&fx, "legal-auditor", "X-5");
    assert_eq!(o.code, 0, "{}", o.all());
    // A second brief on the same content gives the same round.
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let rs = rounds(&fx);
    let shape: Vec<(u64, String)> = rs
        .iter()
        .map(|r| (r["round"].as_u64().unwrap(), r["source"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(
        shape,
        vec![(1, "".to_string()), (2, "X-4".to_string()), (3, "X-5".to_string())],
        "{rs:?}"
    );
    // A brief of the wrong role, or of a check that is no audit, is refused.
    assert_ne!(audit_brief(&fx, "legal-auditor", "X-4").code, 0);
    assert_ne!(audit_brief(&fx, "verifier", "X-2").code, 0);
}

#[test]
fn a_clean_audit_writes_the_checks_receipt_and_counts_no_review() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(reviews(&fx), 1);
    audit_brief(&fx, "security-auditor", "X-4");
    assert!(receipts_of(&fx, "X-4").is_empty());
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("a low thing", "low")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let rs = receipts_of(&fx, "X-4");
    assert_eq!(rs.len(), 1, "{rs:?}");
    assert!(rs[0]["reason"].as_str().unwrap().contains("round 2"), "{rs:?}");
    // The audit did not count against the limit of reviews, and the review's own receipt is still its own.
    assert_eq!(reviews(&fx), 1);
    assert_eq!(receipts_of(&fx, "X-3").len(), 1);
    assert!(receipts_of(&fx, "X-5").is_empty());
}

#[test]
fn an_audit_finding_is_accepted_and_declined_by_its_review_id() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    audit_brief(&fx, "security-auditor", "X-4");
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("first", "high"), finding("second", "medium")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(receipts_of(&fx, "X-4").is_empty());
    // A filed again finding the human has not ended is a new finding; one they ended is refused.
    fx.human("Accept the first one, it is fine.");
    fx.must(&["checkpoint", "finding", "K-1", "--accept", "r2.1", "--quote", "Accept the first one, it is fine."]);
    fx.human("Decline the second one, it is wrong.");
    fx.must(&[
        "checkpoint", "finding", "K-1", "--decline", "r2.2", "--reason", "it is wrong", "--quote",
        "Decline the second one, it is wrong.",
    ]);
    let outcomes = rounds(&fx)[1]["outcomes"].to_string();
    assert!(outcomes.contains("accepted by the human") && outcomes.contains("declined: it is wrong"), "{outcomes}");
    // Both ended: a repeat round may not file them again.
    audit_brief(&fx, "security-auditor", "X-4");
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("first", "high")]}),
        None,
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("accepted by the human"), "{}", o.all());
}

#[test]
fn the_receipt_is_written_when_the_human_ended_every_blocking_finding() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    audit_brief(&fx, "legal-auditor", "X-5");
    let o = audit_report(
        &fx,
        "legal-auditor",
        "X-5",
        json!({"areas": areas(), "findings": [finding("one", "high"), finding("two", "critical")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Accept the first.");
    fx.must(&["checkpoint", "finding", "K-1", "--accept", "r2.1", "--quote", "Accept the first."]);
    assert!(receipts_of(&fx, "X-5").is_empty(), "one blocking finding is still open");
    fx.human("Accept the second as well.");
    fx.must(&["checkpoint", "finding", "K-1", "--accept", "r2.2", "--quote", "Accept the second as well."]);
    let rs = receipts_of(&fx, "X-5");
    assert_eq!(rs.len(), 1, "{rs:?}");
    let why = rs[0]["reason"].as_str().unwrap();
    assert!(why.contains("r2.1") && why.contains("r2.2"), "{why}");
}

#[test]
fn an_audit_report_with_the_reviewers_token_is_refused() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    fx.set_policy("tokens = false", "tokens = true");
    let p = fx.project();
    let reviewer = bf::run::issue_token(&p, "K-1:reviewer").unwrap();
    let audit = bf::run::issue_token(&p, "K-1:audit:X-4").unwrap();
    audit_brief(&fx, "security-auditor", "X-4");
    let body = json!({"areas": areas(), "findings": []});
    let o = audit_report(&fx, "security-auditor", "X-4", body.clone(), Some(&reviewer));
    assert_ne!(o.code, 0, "{}", o.all());
    let o = audit_report(&fx, "security-auditor", "X-4", body.clone(), None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(receipts_of(&fx, "X-4").is_empty());
    let o = audit_report(&fx, "security-auditor", "X-4", body, Some(&audit));
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(receipts_of(&fx, "X-4").len(), 1);
}

#[test]
fn an_audit_at_the_limit_with_a_high_finding_parks_the_checkpoint() {
    let fx = Fx::new();
    project(&fx, &[]);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    pass_review(&fx);
    audit_brief(&fx, "security-auditor", "X-4");
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("bad", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["parked"], true, "{}", o.all());
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["checkpoint_reviews"][0]["parked"], true, "{st}");
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["directives"].as_array().unwrap().iter().any(|d| d["action"] == "decide-checkpoint-review"),
        "{n}"
    );
}

#[test]
fn a_uiux_report_that_leaves_a_failed_step_unnamed_is_refused() {
    let fx = Fx::new();
    project(&fx, &[("X-6", "built-uiux-verification")]);
    pass_review(&fx);
    let wc = fx.work("K-1");
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("seen.txt");
    fs::write(&f, "what was seen\n").unwrap();
    fx.must_in(
        &wc,
        &["evidence", "attach", "X-6", f.to_str().unwrap(), "--outcome", "failed", "--says", "J-1 step 1: the page does not open"],
    );
    audit_brief(&fx, "uiux-verifier", "X-6");
    let o = audit_report(&fx, "uiux-verifier", "X-6", json!({"areas": areas(), "findings": []}), None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("J-1 step 1"), "{}", o.all());
    let mut named = finding("the page does not open", "high");
    named["step"] = json!("J-1 step 1");
    let o = audit_report(&fx, "uiux-verifier", "X-6", json!({"areas": areas(), "findings": [named]}), None);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(receipts_of(&fx, "X-6").is_empty());
}

#[test]
fn the_close_names_an_open_audit_finding_with_its_source() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    audit_brief(&fx, "security-auditor", "X-4");
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    let text = o.all();
    assert!(
        text.contains("r2.1") && text.contains("X-4") && text.contains("high") && text.contains("leaks a secret"),
        "{text}"
    );
}

#[test]
fn a_log_of_todays_engine_replays_to_the_same_rounds() {
    let fx = Fx::new();
    project(&fx, &[]);
    fx.must(&["brief", "K-1", "--role", "reviewer"]);
    fx.must_in(&fx.work("K-1-review"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"]);
    let o = fx.bf_stdin(
        &fx.root,
        &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"],
        &json!({"areas": areas(), "findings": [finding("old style", "low")]}).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The review's events carry no source, as before, and read as the review's.
    for line in log_text(&fx).lines().filter(|l| l.contains("checkpoint.review.")) {
        let v: Value = serde_json::from_str(line).unwrap();
        assert!(v.get("source").is_none(), "{line}");
    }
    let rs = rounds(&fx);
    assert_eq!(rs.len(), 1, "{rs:?}");
    assert_eq!(rs[0]["source"], "");
    assert_eq!(reviews(&fx), 1);
    assert_eq!(receipts_of(&fx, "X-3").len(), 1);
}

#[test]
fn the_design_review_checks_a_checkpoint_lists_are_no_audit_source() {
    let fx = Fx::new();
    project(
        &fx,
        &[("X-6", "amendment-review"), ("X-7", "design-uiux-verification")],
    );
    let pl = bf::plan::load(&fx.project()).unwrap();
    let src = bf::ckaudit::sources(&pl, "K-1");
    assert_eq!(src, vec!["X-4".to_string(), "X-5".to_string()], "{src:?}");
    assert!(!bf::ckaudit::is_source(&pl, "K-1", "X-3"));
    assert!(!bf::ckaudit::is_source(&pl, "K-1", "X-6"));
    assert!(!bf::ckaudit::is_source(&pl, "K-1", "X-7"));
    pass_review(&fx);
    for check in ["X-6", "X-7"] {
        let o = audit_brief(&fx, "verifier", check);
        assert_ne!(o.code, 0, "{}", o.all());
    }
}

#[test]
fn the_audit_brief_names_the_report_command_and_its_json_fields() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    let o = audit_brief(&fx, "security-auditor", "X-4");
    assert_eq!(o.code, 0, "{}", o.all());
    let path = o.json()["path"].as_str().unwrap().to_string();
    assert!(path.ends_with("K-1-audit-X-4.md"), "{path}");
    let md = fs::read_to_string(path).unwrap();
    assert!(
        md.contains("bf report K-1 --role security-auditor --check X-4 --token <token> --result findings --findings -"),
        "{md}"
    );
    for word in ["`areas`", "`findings`", "`probes`", "`title`", "`severity`", "`where`", "`action`"] {
        assert!(md.contains(word), "{word}: {md}");
    }
    assert!(md.contains("<summary>U-1</summary>") && md.contains("K-1-audit-X-4.diff"), "{md}");
    assert!(md.contains("R-1"), "{md}");
}

#[test]
fn one_audits_high_finding_does_not_stop_a_second_audit() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("a hole", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The open high finding of the security audit holds neither the legal audit's brief nor its report.
    let o = audit_brief(&fx, "legal-auditor", "X-5");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = audit_report(
        &fx,
        "legal-auditor",
        "X-5",
        json!({"areas": areas(), "findings": []}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let shape: Vec<(u64, String)> = rounds(&fx)
        .iter()
        .map(|r| (r["round"].as_u64().unwrap(), r["source"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(
        shape,
        vec![(1, "".to_string()), (2, "X-4".to_string()), (3, "X-5".to_string())]
    );
}

#[test]
fn next_offers_no_review_or_decide_directive_while_an_audit_is_due() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("a hole", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The legal audit is still due on this content.
    let n = fx.must(&["next", "--json"]).json();
    let actions: Vec<String> = n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["action"].as_str().map(String::from))
        .collect();
    for bad in ["review-checkpoint", "decide-checkpoint-review"] {
        assert!(!actions.iter().any(|a| a == bad), "{bad} offered: {n}");
    }
}
