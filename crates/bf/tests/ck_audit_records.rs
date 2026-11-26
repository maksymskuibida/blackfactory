//! U-138 (R-77, D-164, TH-21, FU-15): at a checkpoint an auditor files findings only through its audit round, so a
//! finding record, a legal record and `bf legal finding` from a checkpoint's copy are refused and name the report;
//! a legal entry that needs a lawyer is also filed by the engine as a marked legal record, and a changed legal
//! finding is asked again.

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

fn log_text(fx: &Fx) -> String {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap()
}

fn file_audit(fx: &Fx, role: &str, check: &str, findings: Vec<Value>) {
    assert_eq!(audit_brief(fx, role, check).code, 0);
    let o = audit_report(fx, role, check, json!({"areas": areas(), "findings": findings}), None);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn put_in_copy(fx: &Fx, kind: &str, text: &str) -> Out {
    let dir = fx.work("K-1");
    let f = dir.join("rec.toml");
    fs::write(&f, text).unwrap();
    fx.bf_in(&dir, &["record", "put", kind, "--file", f.to_str().unwrap()])
}

fn built_checkpoint(fx: &Fx) {
    project(fx, &[]);
    pass_review(fx);
}

fn legal_entry(title: &str) -> Value {
    json!({"title": title, "severity": "low", "where": "src/a.txt:1", "action": "ask a lawyer",
           "text": "the notice may be missing", "lawyer_check": true, "market": "Atlantis"})
}

fn answer(fx: &Fx, q: &str, key: &str) -> Out {
    let a = fx.root.join("ans.json");
    fs::write(&a, json!({"choice": key}).to_string()).unwrap();
    fx.bf(&["answer", q, "--file", a.to_str().unwrap()])
}

#[test]
fn an_auditors_finding_record_from_a_checkpoint_copy_is_refused_and_names_the_report() {
    let fx = Fx::new();
    built_checkpoint(&fx);
    let o = put_in_copy(
        &fx,
        "finding",
        "kind=\"finding\"\ntitle=\"A secret\"\nunit=\"U-1\"\nseverity=\"high\"\nwhere=\"src/a.txt\"\naction=\"Rotate it\"\n",
    );
    assert_ne!(o.code, 0, "{}", o.all());
    let t = o.all();
    assert!(
        t.contains("bf report K-1 --role security-auditor --check X-4 --token <token> --result findings --findings -")
            && t.contains("bf next"),
        "{t}"
    );
    assert!(!fx.must(&["record", "list", "--kind", "finding"]).stdout.contains("A secret"));
}

#[test]
fn a_legal_auditors_legal_record_from_a_checkpoint_copy_is_refused() {
    let fx = Fx::new();
    built_checkpoint(&fx);
    let o = put_in_copy(&fx, "legal", "kind=\"legal\"\ntitle=\"Cookie notice\"\n");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("--role legal-auditor --check X-5"), "{}", o.all());
    assert!(!fx.must(&["record", "list", "--kind", "legal"]).stdout.contains("Cookie"));
}

#[test]
fn a_legal_auditors_legal_finding_command_from_a_checkpoint_copy_is_refused_and_names_the_report() {
    let fx = Fx::new();
    built_checkpoint(&fx);
    let dir = fx.work("K-1");
    let f = dir.join("f.json");
    fs::write(
        &f,
        json!({"title": "Consent", "markets": ["Atlantis"], "risk": "consent", "researched": false,
               "lawyer_check": true})
        .to_string(),
    )
    .unwrap();
    let o = fx.bf_in(&dir, &["legal", "finding", f.to_str().unwrap()]);
    assert_ne!(o.code, 0, "{}", o.all());
    let t = o.all();
    assert!(
        t.contains("bf report K-1 --role legal-auditor --check X-5 --token <token> --result findings --findings -")
            && t.contains("bf next"),
        "{t}"
    );
    assert!(fx.must(&["record", "list", "--kind", "legal"]).stdout.trim().is_empty());
}

#[test]
fn a_legal_entry_with_lawyer_check_files_a_marked_legal_record_and_holds_the_close() {
    let fx = Fx::new();
    built_checkpoint(&fx);
    file_audit(&fx, "security-auditor", "X-4", vec![]);
    // An entry that needs a lawyer must name its market; the other-source case is a separate test.
    audit_brief(&fx, "legal-auditor", "X-5");
    let mut stray = legal_entry("stray");
    stray.as_object_mut().unwrap().remove("market");
    let o = audit_report(&fx, "legal-auditor", "X-5", json!({"areas": areas(), "findings": [stray]}), None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("market"), "{}", o.all());
    let o = audit_report(&fx, "legal-auditor", "X-5", json!({"areas": areas(), "findings": [legal_entry("Consent is missing")]}), None);
    assert_eq!(o.code, 0, "{}", o.all());
    let l = fx.must(&["legal", "list", "--json"]).json();
    let row = &l["findings"][0];
    assert_eq!(row["id"], "L-1", "{l}");
    assert_eq!(row["lawyer_check"], true, "{l}");
    assert_eq!(row["title"], "Consent is missing", "{l}");
    let rec = fx.must(&["record", "get", "L-1", "--json"]).json();
    assert_eq!(rec["market"], "Atlantis");
    assert_eq!(rec["severity"], "low");
    assert_eq!(rec["action"], "ask a lawyer");
    assert_eq!(rec["text"], "the notice may be missing");
    // The record's id is stored on the entry, which is still a finding of the round.
    let done: Vec<Value> = log_text(&fx)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == "checkpoint.review.done" && e["source"] == "X-5")
        .collect();
    assert_eq!(done[0]["findings"][0]["legal_record"], "L-1", "{done:?}");
    // The close waits for the acknowledgement.
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("L-1") && o.all().contains("not acknowledged"), "{}", o.all());
    let q = fx.must(&["legal", "questions", "--json"]).json();
    assert_eq!(q["asked"][0]["finding"], "L-1");
}

#[test]
fn an_entry_with_lawyer_check_in_another_source_is_refused() {
    let fx = Fx::new();
    built_checkpoint(&fx);
    audit_brief(&fx, "security-auditor", "X-4");
    let o = audit_report(&fx, "security-auditor", "X-4", json!({"areas": areas(), "findings": [legal_entry("x")]}), None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("lawyer_check"), "{}", o.all());
    assert!(fx.must(&["record", "list", "--kind", "legal"]).stdout.trim().is_empty());
}

#[test]
fn a_changed_legal_finding_is_asked_again_and_only_the_new_answer_counts() {
    let fx = Fx::new();
    built_checkpoint(&fx);
    file_audit(&fx, "security-auditor", "X-4", vec![]);
    file_audit(&fx, "legal-auditor", "X-5", vec![legal_entry("Consent is missing")]);
    let q1 = fx.must(&["legal", "questions", "--json"]).json()["asked"][0]["question"]
        .as_str()
        .unwrap()
        .to_string();
    fx.human("I read the finding.");
    assert_eq!(answer(&fx, &q1, "A").code, 0);
    assert!(bf::legal::unacknowledged(&fx.project()).unwrap().is_empty());
    // The finding changes after the acknowledgement: it is asked again and the old answer no longer counts.
    let f = fx.root.join("changed.json");
    fs::write(
        &f,
        json!({"id": "L-1", "title": "Consent is missing in a new place", "markets": ["Atlantis"],
               "risk": "consent", "researched": false, "lawyer_check": true, "severity": "low"})
        .to_string(),
    )
    .unwrap();
    fx.must(&["legal", "finding", f.to_str().unwrap()]);
    assert_eq!(bf::legal::unacknowledged(&fx.project()).unwrap().len(), 1);
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("L-1"), "{}", o.all());
    let q2 = fx.must(&["legal", "questions", "--json"]).json()["asked"][0]["question"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(q1, q2);
    fx.human("Yes, I acknowledge the changed finding.");
    assert_eq!(answer(&fx, &q2, "A").code, 0);
    assert!(bf::legal::unacknowledged(&fx.project()).unwrap().is_empty());
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_repeat_legal_round_with_the_same_entry_keeps_one_legal_record() {
    let fx = Fx::new();
    built_checkpoint(&fx);
    file_audit(&fx, "security-auditor", "X-4", vec![]);
    let mut e = legal_entry("Consent is missing");
    e["severity"] = json!("high");
    file_audit(&fx, "legal-auditor", "X-5", vec![e.clone()]);
    file_audit(&fx, "legal-auditor", "X-5", vec![e]);
    let l = fx.must(&["legal", "list", "--json"]).json();
    assert_eq!(l["findings"].as_array().unwrap().len(), 1, "{l}");
}
