//! A-73 hooks (U-108): a log written as events of today's kinds and fields only replays to the states
//! that today's replay gives, with the A-73 fields at rest. These assertions hold after every A-73
//! unit has landed, since an older log must replay unchanged (D-82).

mod common;
use common::*;
use serde_json::{json, Map, Value};
use std::fs;

/// Append events (kind and fields) to the log, hash-chained, and move the head with them.
fn write_log(fx: &Fx, events: &[(&str, Value)]) {
    let p = fx.project();
    let key = p.read_key().unwrap();
    let log = p.log_path();
    let text = fs::read_to_string(&log).unwrap();
    let mut count = text.lines().count();
    let mut prev = text
        .lines()
        .last()
        .map(|l| {
            serde_json::from_str::<Value>(l).unwrap()["h"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .unwrap_or_default();
    let mut out = text;
    for (kind, fields) in events {
        count += 1;
        let mut m = Map::new();
        m.insert("seq".into(), json!(count));
        m.insert("at".into(), json!("2026-01-01T00:00:00Z"));
        m.insert("kind".into(), json!(kind));
        m.insert("prev".into(), json!(prev));
        for (k, v) in fields.as_object().unwrap() {
            m.insert(k.clone(), v.clone());
        }
        let h = bf::events::event_hash(&key, &prev, &m);
        m.insert("h".into(), json!(h));
        prev = h;
        out.push_str(&serde_json::to_string(&Value::Object(m)).unwrap());
        out.push('\n');
    }
    fs::write(&log, out).unwrap();
    let mut head = bf::events::read_head(&p).unwrap_or_default();
    head.count = count as u64;
    head.last_hash = prev;
    bf::events::write_head(&p, &head).unwrap();
}

fn old_log(fx: &Fx) {
    let base = fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .count() as u64;
    // The turn comes after the narrowing and before the approval: its seq is known in advance.
    let turn = base + 13;
    write_log(
        fx,
        &[
            ("unit.start", json!({"unit": "U-a", "base_commit": "c1"})),
            ("unit.start", json!({"unit": "U-b", "base_commit": "c1"})),
            ("unit.start", json!({"unit": "U-c", "base_commit": "c1"})),
            ("unit.start", json!({"unit": "U-d", "base_commit": "c1"})),
            (
                "unit.report",
                json!({"unit": "U-a", "to": "blocked", "result": "blocked", "cost": 0.0, "note": "needs one more thing"}),
            ),
            (
                "unit.scope.refused",
                json!({"unit": "U-b", "refusal": "hard", "from": "running", "cost": 0.25, "note": "",
                "files": [{"path": "src/shared.rs", "rule": "held by U-x"}]}),
            ),
            (
                "unit.report",
                json!({"unit": "U-c", "to": "review", "result": "ok", "cost": 0.0, "note": ""}),
            ),
            (
                "unit.land",
                json!({"unit": "U-c", "repo": "main", "checkpoint": "K-1", "hash": "h1", "merge": "m1", "run": "run-a"}),
            ),
            (
                "unit.report",
                json!({"unit": "U-d", "to": "fixing", "result": "findings", "cost": 0.0, "note": ""}),
            ),
            (
                "unit.park",
                json!({"unit": "U-d", "reason": "idle for 61 minutes"}),
            ),
            (
                "amend.open",
                json!({"amend": "A-1", "ids": ["R-1"], "impact": null, "impact_ids": ["U-c"]}),
            ),
            (
                "amend.narrow",
                json!({"amend": "A-1", "keep": ["U-c"], "waive": [], "reasons": {}}),
            ),
            (
                "human.turn",
                json!({"text": "Yes, approve it.", "transcript": ""}),
            ),
            (
                "amend.approve",
                json!({"amend": "A-1", "turn": turn, "quote": "approve it",
                "reopen_units": ["U-c"], "reopen_checkpoints": [], "reopen_checks": [],
                "receipt_floor": 0, "renamed": []}),
            ),
        ],
    );
}

#[test]
fn an_older_log_replays_to_the_same_states() {
    let fx = Fx::new();
    old_log(&fx);
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.chain_ok, "{:?}", st.chain_error);
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
    let at = |id: &str| {
        let u = &st.units[id];
        (u.state.clone(), u.rounds, u.attempts, u.prev_state.clone())
    };
    let t = |s: &str, r: u32, a: u32, p: &str| (s.to_string(), r, a, p.to_string());
    println!(
        "{:?}",
        st.units
            .iter()
            .map(|(k, u)| (
                k.clone(),
                u.state.clone(),
                u.rounds,
                u.attempts,
                u.prev_state.clone()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(at("U-a"), t("blocked", 0, 1, "running"));
    assert_eq!(at("U-b"), t("blocked", 0, 1, "running"));
    assert_eq!(at("U-c"), t("running", 0, 0, ""));
    assert_eq!(at("U-d"), t("parked", 1, 0, "fixing"));
    assert_eq!(st.units["U-c"].reopened_by, "A-1");
    assert_eq!(st.units["U-d"].parked_reason, "idle for 61 minutes");
    assert_eq!(st.scope["U-b"].refused.len(), 1);
}

#[test]
fn the_new_fields_stay_at_rest() {
    let fx = Fx::new();
    old_log(&fx);
    let st = bf::events::replay(&fx.project()).unwrap();
    assert_eq!(st.units.len(), 4);
    for (id, u) in &st.units {
        assert!(!u.paused, "{id} is paused");
        assert!(!u.reconsider, "{id} is to be reconsidered");
    }
    assert!(st.orders.is_empty());
}

#[test]
fn pause_and_resume_refuse_with_the_four_parts_without_a_run() {
    let fx = Fx::new();
    for sub in ["pause", "resume"] {
        let o = fx.bf(&["unit", sub, "U-a"]);
        assert_ne!(o.code, 0, "{sub}: {}", o.all());
        assert_four_parts(&o.all());
    }
}
