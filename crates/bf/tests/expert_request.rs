// U-105 (D-117): the reviewer of a unit in review asks for a security audit with a finding; `bf next` offers
// the security auditor that work in place of the review; the auditor's report (built or blocked) closes the
// ask and gives the review back.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const ASK: &str = "kind=\"finding\"\ntitle=\"Look at the key handling\"\ntext=\"The unit reads a secret from the environment; check how it is stored.\"\nunit=\"U-1\"\nasks=\"security\"\n";
const MEDIUM: &str = "kind=\"finding\"\ntitle=\"The secret is logged\"\nunit=\"U-1\"\nseverity=\"medium\"\nwhere=\"src/hello.txt\"\naction=\"Stop logging it\"\n";

/// U-1 in review with tokens on; returns its working copy.
fn in_review(fx: &Fx) -> PathBuf {
    fx.tiny();
    fx.start_test_run();
    fx.set_policy("tokens = false", "tokens = true");
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/hello.txt"]);
    let tb = fx.take_token("U-1", "builder");
    fx.must(&["report", "U-1", "--result", "built", "--token", &tb]);
    wc
}

fn put_in(fx: &Fx, wc: &std::path::Path, text: &str, token: Option<&str>) -> Out {
    // The record file stays outside the working copy, so its content is not changed.
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("rec.toml");
    fs::write(&f, text).unwrap();
    let mut a = vec!["record", "put", "finding", "--file", f.to_str().unwrap()];
    if let Some(t) = token {
        a.extend_from_slice(&["--token", t]);
    }
    fx.bf_in(wc, &a)
}

fn directives(fx: &Fx, unit: &str) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["unit"] == unit)
        .cloned()
        .collect()
}

fn state(fx: &Fx, id: &str) -> String {
    fx.must(&["status", "--json"]).json()["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .map(|u| u["state"].as_str().unwrap().to_string())
        .unwrap_or_default()
}

fn finding(fx: &Fx, id: &str) -> Value {
    fx.must(&["record", "get", id, "--json"]).json()
}

fn audit_report(fx: &Fx, wc: &std::path::Path, result: &str, note: &str) {
    let tok = fx.take_token("U-1", "security-auditor");
    fx.must_in(wc, &["report", "U-1", "--result", result, "--role", "security-auditor", "--token", &tok, "--note", note]);
}

#[test]
fn an_ask_gives_the_audit_directive_with_a_token_and_no_review() {
    let fx = Fx::new();
    let wc = in_review(&fx);
    assert_eq!(directives(&fx, "U-1")[0]["action"], "review");
    let o = put_in(&fx, &wc, ASK, None);
    assert_eq!(o.code, 0, "{}", o.all());
    let ds = directives(&fx, "U-1");
    assert_eq!(ds.len(), 1, "{ds:?}");
    let d = &ds[0];
    assert_eq!(d["action"], "security-audit");
    assert_eq!(d["role"], "security-auditor");
    assert!(d["token"].as_str().is_some_and(|t| !t.is_empty()), "{d}");
    assert!(d["finding"].as_str().is_some_and(|f| f.starts_with("F-")), "{d}");
    assert!(ds.iter().all(|d| d["action"] != "review"));
    let b = fx.must(&["brief", "U-1", "--role", "security-auditor", "--json"]).json();
    let md = fs::read_to_string(b["path"].as_str().unwrap()).unwrap();
    assert!(md.contains("check how it is stored"), "{md}");
}

#[test]
fn an_ask_that_names_no_unit_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    let f = fx.root.join("ask.toml");
    fs::write(&f, "kind=\"finding\"\ntitle=\"t\"\nasks=\"security\"\n").unwrap();
    let o = fx.bf(&["record", "put", "finding", "--file", f.to_str().unwrap()]);
    assert!(o.code != 0, "{}", o.all());
    let cause = bf::strings::t("cli.schema.finding.asks_security_unit", &[]);
    assert!(!cause.is_empty());
    assert!(o.stderr.contains(&cause), "{}", o.all());
}

#[test]
fn two_asks_give_one_directive_and_the_report_closes_both() {
    let fx = Fx::new();
    let wc = in_review(&fx);
    assert_eq!(put_in(&fx, &wc, ASK, None).code, 0);
    assert_eq!(put_in(&fx, &wc, &ASK.replace("key handling", "logging"), None).code, 0);
    assert_eq!(directives(&fx, "U-1").len(), 1);
    audit_report(&fx, &wc, "built", "done");
    let asks: Vec<Value> = ["F-1", "F-2"].iter().map(|i| finding(&fx, i)).collect();
    for a in &asks {
        assert_eq!(a["outcome"], "fixed", "{a}");
        assert!(a["action"].as_str().unwrap().contains("security audit"), "{a}");
    }
    // The review is back, once.
    let ds = directives(&fx, "U-1");
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0]["action"], "review");
}

#[test]
fn the_auditor_still_reports_on_a_unit_with_no_ask() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.set_policy("tokens = false", "tokens = true");
    fx.must(&["unit", "start", "U-1"]);
    let t = fx.must(&["roles", "token", "U-1", "security-auditor", "--json"]).json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    fx.must(&["report", "U-1", "--result", "built", "--role", "security-auditor", "--token", &t]);
    assert_eq!(state(&fx, "U-1"), "running");
}

#[test]
fn a_clean_audit_gives_the_review_back_and_the_unit_lands() {
    let fx = Fx::new();
    let wc = in_review(&fx);
    assert_eq!(put_in(&fx, &wc, ASK, None).code, 0);
    audit_report(&fx, &wc, "built", "nothing found");
    let ds = directives(&fx, "U-1");
    assert_eq!(ds[0]["action"], "review");
    let tr = ds[0]["token"].as_str().unwrap().to_string();
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer", "--token", &tr]);
    assert_eq!(state(&fx, "U-1"), "reviewed");
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-1"), "landed");
}

#[test]
fn a_finding_of_the_auditor_sends_the_unit_to_fixing() {
    let fx = Fx::new();
    let wc = in_review(&fx);
    assert_eq!(put_in(&fx, &wc, ASK, None).code, 0);
    let tok = fx.take_token("U-1", "security-auditor");
    let o = put_in(&fx, &wc, MEDIUM, Some(&tok));
    assert_eq!(o.code, 0, "{}", o.all());
    audit_report(&fx, &wc, "built", "one finding");
    let ds = directives(&fx, "U-1");
    assert_eq!(ds[0]["action"], "review");
    let tr = ds[0]["token"].as_str().unwrap().to_string();
    fx.must(&["report", "U-1", "--result", "findings", "--role", "reviewer", "--token", &tr]);
    assert_eq!(state(&fx, "U-1"), "fixing");
    let b = fx.must(&["brief", "U-1", "--role", "builder", "--json"]).json();
    let md = fs::read_to_string(b["path"].as_str().unwrap()).unwrap();
    assert!(md.contains("The secret is logged"), "{md}");
}

#[test]
fn a_blocked_audit_closes_the_ask_with_its_reason_and_gives_the_review_back() {
    let fx = Fx::new();
    let wc = in_review(&fx);
    assert_eq!(put_in(&fx, &wc, ASK, None).code, 0);
    audit_report(&fx, &wc, "blocked", "the scanner cannot run here");
    let a = finding(&fx, "F-1");
    assert_eq!(a["outcome"], "fixed", "{a}");
    assert!(a["action"].as_str().unwrap().ends_with("the scanner cannot run here"), "{a}");
    let ds = directives(&fx, "U-1");
    assert_eq!(ds[0]["action"], "review");
    let b = fx.must(&["brief", "U-1", "--role", "reviewer", "--json"]).json();
    let md = fs::read_to_string(b["path"].as_str().unwrap()).unwrap();
    assert!(md.contains("the scanner cannot run here"), "{md}");
}

#[test]
fn a_unit_with_an_open_ask_waits_twice_the_idle_limit() {
    use bf::run::expert::park_limit;
    assert_eq!(park_limit(3600, false), 3600);
    assert_eq!(park_limit(3600, true), 7200);
    let fx = Fx::new();
    let wc = in_review(&fx);
    let p = fx.project();
    let limit = p.policy.limits.idle_minutes * 60;
    let base = bf::util::now_secs();
    let parked = |now: u64| -> bool {
        let v = bf::run::stuck_at(&p, true, now).unwrap();
        v["parked_now"].as_array().unwrap().iter().any(|x| x["unit"] == "U-1")
    };
    // Control: no ask, parked after one limit.
    assert!(!parked(base));
    assert!(parked(base + limit + 120), "a unit with no ask is parked after one limit");
    // With an open ask: left alone past one limit, parked past two.
    assert_eq!(put_in(&fx, &wc, ASK, None).code, 0);
    assert!(!parked(base + limit + 120), "an open ask doubles the limit");
    assert!(parked(base + 2 * limit + 120), "past two limits it is parked");
}

#[test]
fn a_unit_with_no_ask_is_reviewed_and_lands_as_before() {
    let fx = Fx::new();
    let _wc = in_review(&fx);
    let ds = directives(&fx, "U-1");
    assert_eq!(ds[0]["action"], "review");
    let tr = ds[0]["token"].as_str().unwrap().to_string();
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer", "--token", &tr]);
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-1"), "landed");
}

#[test]
fn a_reviewer_report_while_an_ask_is_open_is_refused_and_the_ask_stays_open() {
    let fx = Fx::new();
    let wc = in_review(&fx);
    assert_eq!(put_in(&fx, &wc, ASK, None).code, 0);
    // No reviewer directive is offered while the ask is open, so the report carries no real token.
    let o = fx.bf(&["report", "U-1", "--result", "findings", "--role", "reviewer", "--token", "none"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("security audit"), "{}", o.stderr);
    assert_eq!(state(&fx, "U-1"), "review");
    let o = finding(&fx, "F-1");
    assert!(o["outcome"].is_null() || o["outcome"] == "open", "{o}");
}
