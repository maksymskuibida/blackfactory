//! K-8 (R-78, D-160): an audit finding the source names in `still_open` is open again: it opens a fix as a
//! reopened review finding does, and once the human ends it the audit check gets its receipt.

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

fn actions(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["action"].as_str().map(String::from))
        .collect()
}

fn legal_clean(fx: &Fx) {
    assert_eq!(audit_brief(fx, "legal-auditor", "X-5").code, 0);
    let o = audit_report(fx, "legal-auditor", "X-5", json!({"areas": areas(), "findings": []}), None);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn probe_k8_reopened_audit_finding_accepted_gives_receipt() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(&fx, "security-auditor", "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}), None);
    assert_eq!(o.code, 0, "{}", o.all());
    legal_clean(&fx);
    assert!(actions(&fx).contains(&"fix-checkpoint".to_string()));
    fx.pass_checkpoint_fix("changed by the fix\n");
    review_brief(&fx);
    let o = review_report(&fx, json!({"areas": areas(), "findings": []}));
    assert_eq!(o.code, 0, "{}", o.all());
    legal_clean(&fx);
    let o = audit_brief(&fx, "security-auditor", "X-4");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = audit_report(&fx, "security-auditor", "X-4",
        json!({"areas": areas(), "findings": [], "still_open": [{"id": "r2.1", "why": "still leaks"}]}), None);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("I accept the leak, it is fine for now.");
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--accept", "r2.1", "--reason", "fine for now",
        "--quote", "I accept the leak, it is fine for now."]);
    assert_eq!(o.code, 0, "{}", o.all());
    let acts = actions(&fx);
    assert!(
        receipts_of(&fx, "X-4").len() == 1 || acts.contains(&"audit-checkpoint".to_string()),
        "no X-4 receipt at the content and no audit offered: {acts:?}"
    );
}

#[test]
fn a_reopened_audit_finding_opens_a_fix() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(&fx, "security-auditor", "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}), None);
    assert_eq!(o.code, 0, "{}", o.all());
    legal_clean(&fx);
    assert!(actions(&fx).contains(&"fix-checkpoint".to_string()));
    fx.pass_checkpoint_fix("changed by the fix\n");
    review_brief(&fx);
    let o = review_report(&fx, json!({"areas": areas(), "findings": []}));
    assert_eq!(o.code, 0, "{}", o.all());
    legal_clean(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(&fx, "security-auditor", "X-4",
        json!({"areas": areas(), "findings": [], "still_open": [{"id": "r2.1", "why": "still leaks"}]}), None);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(receipts_of(&fx, "X-4").is_empty());
    assert!(actions(&fx).contains(&"fix-checkpoint".to_string()), "{:?}", actions(&fx));
}
