//! U-134 (R-78, D-160, TH-21): a fix opened under this engine leaves the findings it was opened for answered,
//! and only the next round of the source that filed a finding confirms it fixed or opens it again.

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
        "checkpoint-review=on",
        "security-audit=on",
        "idea-review=off:the test does not exercise it",
        "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it",
        "plan-review=off:the test does not exercise it",
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

/// The checkpoint review runs and files nothing: the audits may start.
fn pass_review(fx: &Fx) {
    fx.must(&["brief", "K-1", "--role", "reviewer"]);
    fx.must_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let o = fx.bf_stdin(
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
        "report",
        "K-1",
        "--role",
        role,
        "--check",
        check,
        "--result",
        "findings",
        "--findings",
        "-",
        "--json",
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
    for e in fs::read_dir(fx.root.join("factory/receipts"))
        .unwrap()
        .flatten()
    {
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

fn review_brief(fx: &Fx) -> String {
    let o = fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

/// The review's report on an open round: its copy runs the checkpoint's command check, then it reports `body`.
fn review_report(fx: &Fx, body: Value) -> Out {
    // A repeat of the run on the same content is refused and settles nothing: the first run is the receipt.
    fx.bf_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
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

fn outcome_of(fx: &Fx, round: u64, id: &str) -> Value {
    let r = rounds(fx)
        .into_iter()
        .find(|r| r["round"] == round)
        .unwrap();
    r["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("no outcome {id}: {r}"))
}

fn actions(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["action"].as_str().map(String::from))
        .collect()
}

/// The review files one high finding (round 1) and its fix passes: the finding is answered.
fn answered_review_finding(fx: &Fx) {
    project(fx, &[]);
    review_brief(fx);
    let o = review_report(
        fx,
        json!({"areas": areas(), "findings": [finding("wrong answer", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.pass_checkpoint_fix("changed by the fix\n");
}

#[test]
fn a_finished_fix_leaves_its_findings_answered_and_the_close_refused() {
    let fx = Fx::new();
    answered_review_finding(&fx);
    let o = outcome_of(&fx, 1, "r1.1");
    assert_eq!(o["state"], "answered", "{o}");
    let fate = o["fate"].as_str().unwrap();
    assert!(
        fate.starts_with("answered by ") && fate.contains("waits for the checkpoint review"),
        "{fate}"
    );
    // The fix opened under this engine carries the marker, and the finding is not the human's to decide.
    assert!(log_text(&fx)
        .lines()
        .any(|l| l.contains("checkpoint.fix.open") && l.contains("\"confirm\":true")));
    assert!(
        !actions(&fx).iter().any(|a| a == "decide-checkpoint-fix"),
        "{:?}",
        actions(&fx)
    );
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1",
        "--quote",
        "Fix it again.",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("answered by"), "{}", o.all());
    // It holds the close.
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r1.1"), "{}", o.all());
}

#[test]
fn the_next_review_confirms_a_review_finding_and_it_ends_fixed() {
    let fx = Fx::new();
    answered_review_finding(&fx);
    let md = review_brief(&fx);
    assert!(
        md.contains("Answered findings to confirm")
            && md.contains("r1.1")
            && md.contains("`confirmed`")
            && md.contains("`still_open`"),
        "{md}"
    );
    assert!(md.contains("wrong answer") && md.contains("-fix-1"), "{md}");
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = outcome_of(&fx, 1, "r1.1");
    assert_eq!(o["state"], "fixed", "{o}");
    let fate = o["fate"].as_str().unwrap();
    assert!(
        fate.starts_with("fixed by ")
            && fate.contains("-fix-1")
            && fate.contains("confirmed by round 2"),
        "{fate}"
    );
    assert_eq!(receipts_of(&fx, "X-3").len(), 1);
    assert!(log_text(&fx)
        .lines()
        .any(|l| l.contains("checkpoint.review.done") && l.contains("\"confirmed\":[\"r1.1\"]")));
}

#[test]
fn the_security_audit_confirms_its_own_finding_and_not_the_review_its() {
    let fx = Fx::new();
    project(&fx, &[]);
    // The review files a low finding (round 1), the security audit a high one (round 2); one fix answers both.
    review_brief(&fx);
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [finding("a small thing", "low")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Fix both of them.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1,r2.1",
        "--quote",
        "Fix both of them.",
    ]);
    fx.pass_checkpoint_fix("changed by the fix\n");
    assert_eq!(outcome_of(&fx, 1, "r1.1")["state"], "answered");
    assert_eq!(outcome_of(&fx, 2, "r2.1")["state"], "answered");
    // The review's brief lists its own finding only, and its report may not confirm the audit's.
    let md = review_brief(&fx);
    assert!(
        md.contains("r1.1") && md.contains("Answered findings to confirm"),
        "{md}"
    );
    assert!(!md.contains("leaks a secret"), "{md}");
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1", "r2.1"]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r2.1"), "{}", o.all());
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The audit's brief lists its own finding only, and its report may not confirm the review's.
    let o = audit_brief(&fx, "security-auditor", "X-4");
    assert_eq!(o.code, 0, "{}", o.all());
    let md = fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap();
    assert!(
        md.contains("r2.1")
            && md.contains("leaks a secret")
            && md.contains("Answered findings to confirm"),
        "{md}"
    );
    assert!(!md.contains("a small thing"), "{md}");
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1", "r2.1"]}),
        None,
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r1.1"), "{}", o.all());
    assert!(receipts_of(&fx, "X-4").is_empty());
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [], "confirmed": ["r2.1"]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = outcome_of(&fx, 2, "r2.1");
    assert_eq!(o["state"], "fixed", "{o}");
    assert!(
        o["fate"].as_str().unwrap().contains("confirmed by round 4"),
        "{o}"
    );
    assert_eq!(receipts_of(&fx, "X-4").len(), 1);
}

#[test]
fn a_report_that_leaves_out_an_answered_finding_is_refused() {
    let fx = Fx::new();
    answered_review_finding(&fx);
    review_brief(&fx);
    // Left out.
    let o = review_report(&fx, json!({"areas": areas(), "findings": []}));
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r1.1"), "{}", o.all());
    // Named twice.
    let body = json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"], "still_open": [{"id": "r1.1", "why": "no"}]});
    let o = review_report(&fx, body);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.all().contains("r1.1") && o.all().contains("more than once"),
        "{}",
        o.all()
    );
    // A finding that is not an answered one.
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1", "r7.3"]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r7.3"), "{}", o.all());
    // A finding kept open needs its reason.
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "still_open": [{"id": "r1.1"}]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.all().contains("r1.1") && o.all().contains("why"),
        "{}",
        o.all()
    );
    // Nothing was recorded, and the finding is still answered.
    assert_eq!(outcome_of(&fx, 1, "r1.1")["state"], "answered");
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_fix_review_that_names_a_finding_confirmed_is_refused() {
    let fx = Fx::new();
    project(&fx, &[]);
    review_brief(&fx);
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [finding("wrong answer", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "changed by the fix\n").unwrap();
    common::git(&k, &["add", "-A"]);
    common::git(&k, &["commit", "-q", "-m", "fix"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    let o = fx.bf_stdin(
        &k,
        &[
            "report",
            "K-1",
            "--role",
            "reviewer",
            "--result",
            "built",
            "--findings",
            "-",
        ],
        &json!({"confirmed": ["r1.1"]}).to_string(),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("confirmed"), "{}", o.all());
    assert_eq!(outcome_of(&fx, 1, "r1.1")["state"], "open");
}

#[test]
fn a_finding_still_open_is_open_again_under_its_own_id_and_opens_a_fix() {
    let fx = Fx::new();
    answered_review_finding(&fx);
    review_brief(&fx);
    let body = json!({"areas": areas(), "findings": [],
        "still_open": [{"id": "r1.1", "why": "the answer is still wrong"}]});
    let o = review_report(&fx, body);
    assert_eq!(o.code, 0, "{}", o.all());
    // Open again under its own id, holding the close, with no receipt for the review.
    let o = outcome_of(&fx, 1, "r1.1");
    assert_eq!(o["state"], "open", "{o}");
    assert_eq!(o["fate"], "open");
    assert!(receipts_of(&fx, "X-3").is_empty());
    assert!(
        actions(&fx).iter().any(|a| a == "decide-checkpoint-fix"),
        "{:?}",
        actions(&fx)
    );
    assert_ne!(fx.bf(&["checkpoint", "close", "K-1"]).code, 0);
    // It goes the way of any open finding: the human has a fix opened for it.
    fx.human("Fix r1.1 once more.");
    let o = fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1",
        "--quote",
        "Fix r1.1 once more.",
        "--json",
    ]);
    assert_eq!(o.json()["fix_opened"], true, "{}", o.all());
    fx.pass_checkpoint_fix("changed again\n");
    assert_eq!(outcome_of(&fx, 1, "r1.1")["state"], "answered");
    // The latest fix answers it, and the round after that one confirms it.
    review_brief(&fx);
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = outcome_of(&fx, 1, "r1.1");
    assert!(
        o["fate"].as_str().unwrap().contains("-fix-2")
            && o["fate"].as_str().unwrap().contains("confirmed by round 3"),
        "{o}"
    );
}

#[test]
fn a_switched_off_source_leaves_its_finding_fixed_at_the_fix_end() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&[
        "checks",
        "propose",
        "security-audit=off:the human does not want it",
        "--scope",
        "run",
    ]);
    fx.human("Switch the security audit off for this run.");
    fx.must(&[
        "checks",
        "set",
        "--quote",
        "Switch the security audit off for this run.",
    ]);
    fx.human("Fix the secret.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r2.1",
        "--quote",
        "Fix the secret.",
    ]);
    fx.pass_checkpoint_fix("changed by the fix\n");
    let o = outcome_of(&fx, 2, "r2.1");
    assert_eq!(o["state"], "fixed", "{o}");
    assert!(
        o["fate"].as_str().unwrap().starts_with("fixed by ")
            && !o["fate"].as_str().unwrap().contains("confirmed"),
        "{o}"
    );
}

#[test]
fn a_source_switched_off_after_the_fix_ended_leaves_its_finding_fixed_and_the_close_open_to_it() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Fix the secret.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r2.1",
        "--quote",
        "Fix the secret.",
    ]);
    fx.pass_checkpoint_fix("changed by the fix\n");
    assert_eq!(outcome_of(&fx, 2, "r2.1")["state"], "answered");
    fx.must(&[
        "checks",
        "propose",
        "security-audit=off:the human does not want it",
        "--scope",
        "run",
    ]);
    fx.human("Switch the security audit off for this run.");
    fx.must(&[
        "checks",
        "set",
        "--quote",
        "Switch the security audit off for this run.",
    ]);
    let o = outcome_of(&fx, 2, "r2.1");
    assert_eq!(o["state"], "fixed", "{o}");
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert!(!o.all().contains("r2.1"), "{}", o.all());
}

#[test]
fn a_log_with_a_fix_of_todays_engine_replays_to_fixed() {
    let fx = Fx::new();
    answered_review_finding(&fx);
    // The same log as an engine before this one wrote it: no marker on the opening, nothing more on the end.
    let mut log = bf::events::load_log(&fx.project()).unwrap();
    for e in log.events.iter_mut() {
        let obj = e.as_object_mut().unwrap();
        if obj["kind"] == "checkpoint.fix.open" {
            assert!(obj.remove("confirm").is_some());
        }
        if obj["kind"] == "checkpoint.fix.done" {
            obj.remove("after");
        }
    }
    let st = bf::events::replay_log(&log);
    let f = &bf::ckreview::rounds(&fx.project(), "K-1").unwrap()[0].findings[0];
    let (fate, by) = bf::ckreview::fate(&st, "K-1", 1, 0, f).unwrap();
    assert_eq!(fate, "fixed");
    assert!(by.contains("-fix-1"), "{by}");
    assert!(bf::ckreview::fate_text(&st, "K-1", 1, 0, f).starts_with("fixed by "));
    // And the log as written reads answered.
    let st = bf::events::replay(&fx.project()).unwrap();
    assert_eq!(
        bf::ckreview::fate(&st, "K-1", 1, 0, f).unwrap().0,
        "answered"
    );
}
