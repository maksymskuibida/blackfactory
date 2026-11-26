//! U-133 (R-77, D-159, TH-21): `bf next` offers each due audit of a checkpoint beside the verifier's checks, no
//! worker marks an audit check met or failed, and no fix opens until every audit due has reported, then one fix
//! takes the failed checks and every audit's blocking findings.

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
    let uiux = if extra.iter().any(|(_, n)| *n == "built-uiux-verification") {
        "built-uiux-verification=on"
    } else {
        "checkpoint-review=on"
    };
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on",
        uiux,
        "security-audit=on",
        "legal-audit=on",
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

fn next(fx: &Fx) -> Value {
    fx.must(&["next", "--json"]).json()
}

fn dirs(n: &Value, action: &str) -> Vec<Value> {
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["action"] == action)
        .cloned()
        .collect()
}

fn fix_opens(fx: &Fx) -> Vec<Value> {
    log_text(fx)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == "checkpoint.fix.open")
        .collect()
}

/// The verifier marks the checkpoint's own command check failed at the content now in its working copy.
fn fail_x2(fx: &Fx) {
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["evidence", "fail", "X-2", "--reason", "the file is gone"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

fn file_audit(fx: &Fx, role: &str, check: &str, findings: Vec<Value>) {
    assert_eq!(audit_brief(fx, role, check).code, 0);
    let o = audit_report(
        fx,
        role,
        check,
        json!({"areas": areas(), "findings": findings}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn the_audits_are_offered_after_the_review_passed_and_not_before() {
    let fx = Fx::new();
    project(&fx, &[]);
    assert!(dirs(&next(&fx), "audit-checkpoint").is_empty());
    pass_review(&fx);
    let n = next(&fx);
    let ds = dirs(&n, "audit-checkpoint");
    let shape: Vec<(String, String, String)> = ds
        .iter()
        .map(|d| {
            (
                d["check"].as_str().unwrap().into(),
                d["role"].as_str().unwrap().into(),
                d["checkpoint"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            (
                "X-4".to_string(),
                "security-auditor".to_string(),
                "K-1".to_string()
            ),
            (
                "X-5".to_string(),
                "legal-auditor".to_string(),
                "K-1".to_string()
            )
        ],
        "{n}"
    );
    for d in &ds {
        let (role, check) = (d["role"].as_str().unwrap(), d["check"].as_str().unwrap());
        assert_eq!(
            d["commands"][0],
            format!("bf brief K-1 --role {role} --check {check}"),
            "{d}"
        );
        assert!(
            d["brief"]
                .as_str()
                .unwrap()
                .ends_with(&format!("K-1-audit-{check}.md")),
            "{d}"
        );
        assert!(d["working_copy"].as_str().unwrap().ends_with("K-1"), "{d}");
        assert!(d["tier"].is_string() && d["limits"].is_object(), "{d}");
    }
    // A reported audit is no longer offered at that content.
    file_audit(&fx, "security-auditor", "X-4", vec![]);
    let left: Vec<String> = dirs(&next(&fx), "audit-checkpoint")
        .iter()
        .map(|d| d["check"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(left, vec!["X-5".to_string()]);
}

#[test]
fn the_verifiers_directive_names_no_audit_source() {
    let fx = Fx::new();
    project(&fx, &[]);
    let n = next(&fx);
    let v = dirs(&n, "run-checkpoint-checks");
    assert_eq!(v.len(), 1, "{n}");
    let checks: Vec<String> = v[0]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    assert!(checks.contains(&"X-2".to_string()), "{checks:?}");
    for audit in ["X-4", "X-5"] {
        assert!(!checks.contains(&audit.to_string()), "{checks:?}");
    }
}

#[test]
fn an_attach_of_met_on_the_security_audit_check_is_refused() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    let file = fx.root.join("note.txt");
    fs::write(&file, "I looked\n").unwrap();
    let f = file.to_string_lossy().to_string();
    for outcome in ["met", "failed"] {
        let o = fx.bf(&[
            "evidence",
            "attach",
            "X-4",
            &f,
            "--outcome",
            outcome,
            "--says",
            "I read it all",
        ]);
        assert_ne!(o.code, 0, "{}", o.all());
        assert!(o.all().contains("audit-checkpoint"), "{}", o.all());
    }
    let o = fx.bf(&["evidence", "fail", "X-4", "--reason", "it leaks"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("audit-checkpoint"), "{}", o.all());
    assert!(receipts_of(&fx, "X-4").is_empty());
}

#[test]
fn an_unreachable_on_the_uiux_check_is_taken_and_writes_no_receipt() {
    let fx = Fx::new();
    project(&fx, &[("X-6", "built-uiux-verification")]);
    pass_review(&fx);
    let o = fx.bf(&[
        "evidence",
        "unreachable",
        "X-6",
        "--reason",
        "the browser tool is missing",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(receipts_of(&fx, "X-6").is_empty());
}

#[test]
fn a_checkpoint_listing_amendment_review_and_design_uiux_verification_gives_no_audit_for_them() {
    let fx = Fx::new();
    project(
        &fx,
        &[
            ("X-6", "amendment-review"),
            ("X-7", "design-uiux-verification"),
        ],
    );
    pass_review(&fx);
    let n = next(&fx);
    let audited: Vec<String> = dirs(&n, "audit-checkpoint")
        .iter()
        .map(|d| d["check"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(audited, vec!["X-4".to_string(), "X-5".to_string()], "{n}");
    for other in ["X-3", "X-6", "X-7"] {
        assert!(!audited.contains(&other.to_string()));
    }
}

#[test]
fn the_design_reviewers_attach_of_met_on_amendment_review_is_taken() {
    let fx = Fx::new();
    project(&fx, &[("X-6", "amendment-review")]);
    let file = fx.root.join("note.txt");
    fs::write(&file, "I looked\n").unwrap();
    let o = fx.bf(&[
        "evidence",
        "attach",
        "X-6",
        &file.to_string_lossy(),
        "--outcome",
        "met",
        "--says",
        "I read the amendment",
    ]);
    // Whatever else the design review's own rules say, the audit refusal is not among them.
    assert!(!o.all().contains("audit-checkpoint"), "{}", o.all());
}

#[test]
fn a_failed_check_opens_no_fix_while_an_audit_round_is_open() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    fail_x2(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    next(&fx);
    assert!(fix_opens(&fx).is_empty(), "{:?}", fix_opens(&fx));
    // The legal audit is still due after the security audit reported.
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("a hole", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    next(&fx);
    assert!(fix_opens(&fx).is_empty(), "{:?}", fix_opens(&fx));
}

#[test]
fn one_fix_takes_the_failed_check_and_both_audits_findings() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    fail_x2(&fx);
    file_audit(
        &fx,
        "security-auditor",
        "X-4",
        vec![finding("a hole", "high")],
    );
    file_audit(
        &fx,
        "legal-auditor",
        "X-5",
        vec![
            finding("a notice is missing", "medium"),
            finding("a typo", "low"),
        ],
    );
    next(&fx);
    let opens = fix_opens(&fx);
    assert_eq!(opens.len(), 1, "{opens:?}");
    let e = &opens[0];
    assert_eq!(e["failed"].as_array().unwrap().len(), 1, "{e}");
    assert_eq!(e["failed"][0]["check"], "X-2");
    let titles: Vec<String> = e["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        titles,
        vec!["a hole".to_string(), "a notice is missing".to_string()]
    );
    let ids: Vec<(u64, u64)> = e["finding_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["round"].as_u64().unwrap(), i["index"].as_u64().unwrap()))
        .collect();
    assert_eq!(ids, vec![(2, 1), (3, 1)]);
    // The opening replays: the fix stands in the status.
    assert!(fx
        .must(&["status", "--json"])
        .all()
        .contains("a notice is missing"));
}

#[test]
fn the_close_is_not_offered_while_an_audit_is_due() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    let n = next(&fx);
    assert!(dirs(&n, "close-checkpoint").is_empty(), "{n}");
    assert_eq!(dirs(&n, "audit-checkpoint").len(), 2, "{n}");
    file_audit(&fx, "security-auditor", "X-4", vec![]);
    file_audit(&fx, "legal-auditor", "X-5", vec![]);
    let n = next(&fx);
    assert!(dirs(&n, "audit-checkpoint").is_empty(), "{n}");
}

#[test]
fn the_uiux_auditor_attaches_a_step_with_the_token_of_its_audit_directive() {
    let fx = Fx::new();
    project(&fx, &[("X-6", "built-uiux-verification")]);
    pass_review(&fx);
    fx.set_policy("tokens = false", "tokens = true");
    let n = next(&fx);
    let d = dirs(&n, "audit-checkpoint")
        .into_iter()
        .find(|d| d["check"] == "X-6")
        .unwrap_or_else(|| panic!("the uiux audit is offered: {n}"));
    // The tier is the role's own (F-640).
    assert_eq!(d["role"], "uiux-verifier", "{d}");
    let verifier_tier = dirs(&n, "audit-checkpoint")
        .into_iter()
        .find(|x| x["role"] == "security-auditor")
        .unwrap()["tier"]
        .clone();
    assert_ne!(d["tier"], verifier_tier, "{d}");
    let token = d["token"].as_str().expect("a token").to_string();
    let file = fx.root.join("seen.txt");
    fs::write(&file, "what was seen\n").unwrap();
    let f = file.to_string_lossy().to_string();
    let o = fx.bf_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "attach",
            "X-6",
            &f,
            "--outcome",
            "met",
            "--says",
            "J-1 step 1: the page opens",
            "--token",
            &token,
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!o.all().contains("audit-checkpoint"), "{}", o.all());
    // The step receipt carries the check, the journey and the step; it has no `type`.
    let mut step_receipts = 0;
    for e in fs::read_dir(fx.root.join("factory/receipts"))
        .unwrap()
        .flatten()
    {
        if let Ok(v) = toml::from_str::<Value>(&fs::read_to_string(e.path()).unwrap_or_default()) {
            if v["check"] == "X-6" && v["journey"] == "J-1" && v["step"] == "1" {
                step_receipts += 1;
            }
        }
    }
    assert_eq!(step_receipts, 1, "a step receipt for X-6: {}", o.all());
}
