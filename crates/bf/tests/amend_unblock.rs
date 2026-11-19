//! U-100 (A-73): an approved amendment unblocks the units it re-opens or whose checks it changes.
mod common;
use common::Fx;
use serde_json::{json, Map, Value};
use std::fs;

fn narrow_to(fx: &Fx, keep: &[String]) {
    let f = fx.root.join("narrow.json");
    let body = json!({"amend": "A-1", "keep": keep, "waive": []});
    fs::write(&f, body.to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

fn approve_cmd(fx: &Fx) -> Value {
    fx.human("Yes, approve the change.");
    let a = [
        "amend",
        "approve",
        "A-1",
        "--quote",
        "approve the change",
        "--json",
    ];
    fx.must(&a).json()
}

/// Open an amendment for `ids` as a plan repair, keep its whole impact and approve it.
fn approve(fx: &Fx, ids: &[&str]) -> Value {
    let a = [&["amend", "open", "--repair", "--json"][..], ids].concat();
    let keep: Vec<String> =
        serde_json::from_value(fx.must(&a).json()["impact_ids"].clone()).unwrap();
    narrow_to(fx, &keep);
    approve_cmd(fx)
}

fn run_with_two_units() -> Fx {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx
}

fn state(fx: &Fx, unit: &str) -> bf::events::UnitSt {
    bf::events::replay(&fx.project()).unwrap().units[unit].clone()
}

fn block(fx: &Fx) {
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "blocked"]);
}

fn went_to(out: &Value, to: &str) {
    assert_eq!(
        out["unblocked"],
        json!([{"unit": "U-1", "to": to}]),
        "{out}"
    );
}

fn no_release(fx: &Fx) -> bool {
    !fs::read_to_string(fx.project().log_path())
        .unwrap()
        .contains("\"unit.release\"")
}

fn change_check_command(fx: &Fx, id: &str, unit: &str, file: &str) {
    let f = fx.root.join("x.toml");
    let toml = format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"Changed\"\nhow=\"command\"\ncommand=\"test -s {file}\"\nproves=[\"R-1\", \"H-1\"]\nscope=\"unit\"\nunit=\"{unit}\"\n");
    fs::write(&f, toml).unwrap();
    let f = f.to_str().unwrap();
    fx.must(&["record", "put", "check", "--file", f, "--replace"]);
}

#[test]
fn a_blocked_unit_named_by_the_amendment_is_running_with_no_release() {
    let fx = run_with_two_units();
    block(&fx);
    assert_eq!(state(&fx, "U-1").state, "blocked");
    let out = approve(&fx, &["U-1"]);
    went_to(&out, "running");
    let u = state(&fx, "U-1");
    assert_eq!(u.state, "running");
    assert_eq!(u.reopened_by, "A-1");
    assert_eq!((u.rounds, u.attempts), (0, 0));
    assert!(no_release(&fx));
}

#[test]
fn a_unit_blocked_from_fixing_goes_back_to_fixing() {
    let fx = run_with_two_units();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.put(
        "finding",
        "id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\n",
    );
    fx.must(&[
        "report", "U-1", "--result", "findings", "--role", "reviewer",
    ]);
    assert_eq!(state(&fx, "U-1").state, "fixing");
    fx.must(&["report", "U-1", "--result", "blocked"]);
    let out = approve(&fx, &["U-1"]);
    went_to(&out, "fixing");
    assert_eq!(state(&fx, "U-1").state, "fixing");
    assert!(no_release(&fx));
}

#[test]
fn a_unit_whose_check_alone_changes_keeps_its_attempts() {
    let fx = run_with_two_units();
    block(&fx);
    let before = state(&fx, "U-1");
    fx.must(&["amend", "open", "--repair", "X-1"]);
    change_check_command(&fx, "X-1", "U-1", "src/hello.txt");
    narrow_to(&fx, &["X-1".to_string()]);
    let out = approve_cmd(&fx);
    went_to(&out, "running");
    let u = state(&fx, "U-1");
    assert_eq!(u.state, "running");
    assert_eq!((u.rounds, u.attempts), (before.rounds, before.attempts));
    assert_eq!(u.reopened_by, "A-1");
    assert!(no_release(&fx));
}

/// Append events (kind and fields) to the log, hash-chained, and move the head with them.
fn write_log(fx: &Fx, events: &[(&str, Value)]) {
    let p = fx.project();
    let key = p.read_key().unwrap();
    let mut out = fs::read_to_string(p.log_path()).unwrap();
    let last: Value = serde_json::from_str(out.lines().last().unwrap()).unwrap();
    let mut prev = last["h"].as_str().unwrap().to_string();
    let mut count = out.lines().count();
    for (kind, fields) in events {
        count += 1;
        let mut m = Map::new();
        for (k, v) in [("seq", json!(count)), ("at", json!("2026-01-01T00:00:00Z"))] {
            m.insert(k.into(), v);
        }
        m.insert("kind".into(), json!(kind));
        m.insert("prev".into(), json!(prev));
        m.extend(fields.as_object().unwrap().clone());
        prev = bf::events::event_hash(&key, &prev, &m);
        m.insert("h".into(), json!(prev));
        out.push_str(&format!("{}\n", Value::Object(m)));
    }
    fs::write(p.log_path(), out).unwrap();
    let mut head = bf::events::read_head(&p).unwrap_or_default();
    head.count = count as u64;
    head.last_hash = prev;
    bf::events::write_head(&p, &head).unwrap();
}

#[test]
fn a_parked_unit_stays_parked() {
    let fx = run_with_two_units();
    fx.must(&["unit", "start", "U-1"]);
    write_log(
        &fx,
        &[(
            "unit.park",
            json!({"unit": "U-1", "reason": "more than 3 review rounds"}),
        )],
    );
    let before = state(&fx, "U-1");
    let out = approve(&fx, &["U-1"]);
    assert_eq!(out["unblocked"], json!([]), "{out}");
    let u = state(&fx, "U-1");
    assert_eq!(u.state, "parked");
    assert_eq!(u.parked_reason, "more than 3 review rounds");
    assert_eq!((u.rounds, u.attempts), (before.rounds, before.attempts));
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    assert!(!log.contains("unblock_units"));
}

#[test]
fn an_approval_without_the_list_replays_to_the_same_states() {
    let fx = run_with_two_units();
    block(&fx);
    let n = fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .count() as u64;
    let open = json!({"amend": "A-1", "ids": ["R-1"], "impact": null, "impact_ids": ["U-1"]});
    let narrow = json!({"amend": "A-1", "keep": ["U-1"], "waive": [], "reasons": {}});
    let turn = json!({"text": "Yes, approve it.", "transcript": ""});
    let approve = json!({"amend": "A-1", "turn": n + 3, "quote": "approve it", "reopen_units": ["U-1"],
        "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0, "renamed": []});
    let events = [
        ("amend.open", open),
        ("amend.narrow", narrow),
        ("human.turn", turn),
        ("amend.approve", approve),
    ];
    write_log(&fx, &events);
    let u = state(&fx, "U-1");
    assert_eq!((u.state.as_str(), u.rounds, u.attempts), ("blocked", 0, 0));
    assert_eq!(u.reopened_by, "A-1");
}
