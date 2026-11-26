//! U-137 (R-77, R-78, S-7, D-162, D-62): the checkpoint page and `bf status` show each finding's source and
//! its state, an answered finding with the fix that answered it and the source that confirms it, and the audit
//! rounds as rows of their own that do not count against the limit.

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

fn page(fx: &Fx) -> Value {
    fx.must(&["view", "checkpoint", "K-1", "--json"]).json()
}

fn section_rows(page: &Value, id: &str) -> Vec<Value> {
    page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no section {id}: {page}"))["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn row_of(page: &Value, id: &str) -> Value {
    section_rows(page, "findings")
        .into_iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no finding row {id}: {page}"))
}

#[test]
fn the_page_shows_an_audit_finding_with_its_source_and_its_round() {
    let fx = Fx::new();
    project(&fx, &[]);
    pass_review(&fx);
    assert_eq!(audit_brief(&fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(
        &fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high"), finding("a small thing", "low")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let pg = page(&fx);
    let high = row_of(&pg, "r2.1");
    assert_eq!(high["source"], "security_audit", "{high}");
    assert_eq!(high["source_check"], "X-4", "{high}");
    assert_eq!(high["state"], "open", "{high}");
    assert_eq!(high["label"], "high", "{high}");
    assert!(
        high["source_text"]
            .as_str()
            .unwrap()
            .contains("security audit")
            && high["source_text"].as_str().unwrap().contains("X-4"),
        "{high}"
    );
    // The blocking findings come first, the low ones after them.
    let ids: Vec<String> = section_rows(&pg, "findings")
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec!["r2.1", "r2.2"], "{ids:?}");
    // The audit round is a row of its own with its tag, its check and the note that it is not counted.
    let rows = section_rows(&pg, "reviews");
    let audit = rows.iter().find(|r| r["round_number"] == 2).unwrap();
    assert_eq!(audit["source_check"], "X-4", "{audit}");
    assert!(
        audit["tags"][0]["text"].as_str().unwrap().contains("audit"),
        "{audit}"
    );
    assert!(
        audit
            .to_string()
            .contains("does not count against the limit"),
        "{audit}"
    );
    let own = rows.iter().find(|r| r["round_number"] == 1).unwrap();
    assert_eq!(own["source"], "review", "{own}");
    assert!(own.get("tags").is_none(), "{own}");
}

#[test]
fn an_answered_finding_names_its_fix_and_the_source_that_confirms_it() {
    let fx = Fx::new();
    answered_review_finding(&fx);
    let pg = page(&fx);
    let row = row_of(&pg, "r1.1");
    assert_eq!(row["state"], "answered", "{row}");
    assert_eq!(row["source"], "review", "{row}");
    let body = row["body"].to_string();
    assert!(
        body.contains("Answered by")
            && body.contains("-fix-1")
            && body.contains("the checkpoint review"),
        "{body}"
    );
    // The round that confirms it ends it as fixed, and the row names that round.
    review_brief(&fx);
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let row = row_of(&page(&fx), "r1.1");
    assert_eq!(row["state"], "fixed", "{row}");
    assert!(
        row["body"].to_string().contains("confirmed by round 2"),
        "{row}"
    );
}

#[test]
fn the_status_json_carries_source_and_state() {
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
    let v = fx.must(&["status", "--json"]).json();
    let k = &v["checkpoint_reviews"][0];
    let audit_round = k["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["round"] == 2)
        .unwrap();
    assert_eq!(audit_round["source"], "X-4", "{audit_round}");
    let f = k["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "r2.1")
        .unwrap();
    assert_eq!(f["source"], "X-4", "{f}");
    assert_eq!(f["source_kind"], "security_audit", "{f}");
    assert_eq!(f["state"], "open", "{f}");
    let o = outcome_of(&fx, 2, "r2.1");
    assert_eq!(o["source"], "X-4", "{o}");
    assert_eq!(o["state"], "open", "{o}");
    // The plain status names the open finding with its source.
    let plain = fx.must(&["status"]).stdout;
    assert!(
        plain.contains("r2.1") && plain.contains("leaks a secret") && plain.contains("X-4"),
        "{plain}"
    );
}

#[test]
fn every_new_page_text_comes_from_a_key() {
    // The keys of this unit exist, and a missing key gives nothing, never other words (D-62).
    for k in [
        "page.finding.source.review",
        "page.finding.source.fix_review",
        "page.finding.source.security_audit",
        "page.finding.source.legal_audit",
        "page.finding.source.uiux",
        "page.finding.source.audit",
        "page.finding.state.answered",
        "page.review.kind.audit",
        "page.review.audit.not_counted",
    ] {
        let text = fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("strings/en/pages_audit.toml"),
        )
        .unwrap();
        assert!(text.contains(&format!("\"{k}\" = ")), "{k}");
    }
    let src =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/view.rs")).unwrap();
    let start = src.find("fn checkpoint_finding_rows").unwrap();
    let end = src.find("fn checkpoint_view").unwrap();
    let body = &src[start..end];
    // No English sentence in the code of the new rows: each text goes through `t(` and a key.
    for word in ["Answered by", "limit of reviews.\"", "Where:", "Fixed by"] {
        assert!(!body.contains(word), "{word} is in code");
    }
}

#[test]
fn a_fix_review_finding_shows_with_the_fix_review_source_and_the_sections_keep_their_order() {
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
            "findings",
            "--findings",
            "-",
        ],
        &json!({"areas": areas(), "findings": [finding("a fix nit", "low")]}).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let pg = page(&fx);
    let row = row_of(&pg, "f1.1");
    assert_eq!(row["source"], "fix_review", "{row}");
    assert_eq!(row["label"], "low", "{row}");
    assert_eq!(row["state"], "open", "{row}");
    assert!(
        row["source_text"].as_str().unwrap().contains("fix review"),
        "{row}"
    );
    let order: Vec<String> = pg["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    let at = |id: &str| {
        order
            .iter()
            .position(|x| x == id)
            .unwrap_or_else(|| panic!("{order:?}"))
    };
    assert!(
        at("findings") < at("fixes")
            && at("fixes") < at("reviews")
            && at("reviews") < at("proofs")
            && at("proofs") < at("units"),
        "{order:?}"
    );
}

/// The builder commits `text` in the checkpoint copy and reports built; the fix's reviewer files `findings`
/// (or reports clean when there are none).
fn fix_round(fx: &Fx, text: &str, findings: Vec<Value>) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), text).unwrap();
    common::git(&k, &["add", "-A"]);
    common::git(&k, &["commit", "-q", "-m", "fix"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    if findings.is_empty() {
        fx.must_in(
            &k,
            &["report", "K-1", "--role", "reviewer", "--result", "built"],
        );
        return;
    }
    let o = fx.bf_stdin(
        &k,
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
        &json!({"areas": areas(), "findings": findings}).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_fix_review_finding_takes_the_state_of_its_own_fix() {
    let fx = Fx::new();
    project(&fx, &[]);
    review_brief(&fx);
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [finding("wrong answer", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // Fix 1: its review files a medium finding, a later round of the same fix clears it.
    fix_round(&fx, "one\n", vec![finding("fix nit", "medium")]);
    fix_round(&fx, "two\n", vec![]);
    // The review still finds r1.1 open, and the human has a second fix opened for it.
    review_brief(&fx);
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [],
               "still_open": [{"id": "r1.1", "why": "still wrong"}]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Fix r1.1 once more.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1",
        "--quote",
        "Fix r1.1 once more.",
        "--json",
    ]);
    // Fix 2 files its own f1.1 and leaves it open.
    fix_round(&fx, "three\n", vec![finding("second fix nit", "medium")]);
    let pg = page(&fx);
    let rows: Vec<Value> = section_rows(&pg, "findings")
        .into_iter()
        .filter(|r| r["id"] == "f1.1")
        .collect();
    assert_eq!(rows.len(), 2, "{pg}");
    let of = |fix: &str| {
        rows.iter()
            .find(|r| r["fix"].as_str().unwrap().ends_with(fix))
            .unwrap_or_else(|| panic!("no row of {fix}: {pg}"))
            .clone()
    };
    assert_eq!(of("-fix-1")["state"], "fixed", "{}", of("-fix-1"));
    assert_eq!(of("-fix-2")["state"], "open", "{}", of("-fix-2"));
}
