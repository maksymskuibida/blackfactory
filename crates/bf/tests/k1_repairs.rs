//! The repairs from the review of the first checkpoint: a withdrawal needs a reason, narrow and
//! revise pick the same open amendment, the amendment listing has no replacing-amendment field,
//! and an approval counts only after the latest narrowing.

mod common;
use common::Fx;
use std::fs;

fn amendments(fx: &Fx) -> Vec<serde_json::Value> {
    fx.must(&["status", "--json"]).json()["amendments"]
        .as_array()
        .unwrap()
        .clone()
}

fn narrow(fx: &Fx, body: serde_json::Value) {
    let f = fx.root.join("narrow.json");
    fs::write(&f, body.to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

#[test]
fn withdrawing_needs_a_reason_and_keeps_it() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    let o = fx.refused(&["amend", "withdraw", "A-1"]);
    assert!(o.stderr.contains("reason"), "{}", o.stderr);
    fx.refused(&["amend", "withdraw", "A-1", "--reason", "  "]);
    assert_eq!(amendments(&fx)[0]["status"], "open");
    fx.must(&["amend", "withdraw", "A-1", "--reason", "wrong record"]);
    let a = amendments(&fx);
    assert_eq!(a[0]["status"], "withdrawn");
    assert_eq!(a[0]["withdrawn_why"], "wrong record");
    assert!(a[0].get("replaced_by").is_none(), "{}", a[0]);
}

#[test]
fn a_withdrawn_amendment_cannot_be_approved() {
    let fx = Fx::new();
    fx.tiny();
    let opened = fx.must(&["amend", "open", "R-2", "--json"]).json();
    narrow(
        &fx,
        serde_json::json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}),
    );
    fx.must(&["amend", "withdraw", "A-1", "--reason", "changed my mind"]);
    fx.human("Yes, approve it.");
    fx.refused(&["amend", "approve", "A-1", "--quote", "approve it"]);
}

#[test]
fn narrow_without_a_name_takes_the_open_amendment_after_a_revision() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    let revised = fx.must(&["amend", "revise", "R-1", "--json"]).json();
    // No amendment named in the file: the open one is narrowed.
    narrow(
        &fx,
        serde_json::json!({"keep": revised["impact_ids"], "waive": []}),
    );
    fx.human("Yes, approve the amendment.");
    fx.must(&["amend", "approve", "--quote", "approve the amendment"]);
    assert_eq!(amendments(&fx)[0]["status"], "approved");
}

#[test]
fn a_message_before_the_latest_narrowing_does_not_approve() {
    let fx = Fx::new();
    fx.tiny();
    let opened = fx.must(&["amend", "open", "R-2", "--json"]).json();
    fx.human("Yes, approve the amendment.");
    narrow(
        &fx,
        serde_json::json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}),
    );
    fx.refused(&[
        "amend",
        "approve",
        "A-1",
        "--quote",
        "approve the amendment",
    ]);
}

#[test]
fn replay_ignores_an_approval_cited_before_the_latest_narrowing() {
    let fx = Fx::new();
    fx.tiny();
    let opened = fx.must(&["amend", "open", "R-2", "--json"]).json();
    fx.human("Yes, approve the amendment.");
    narrow(
        &fx,
        serde_json::json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}),
    );
    // The approval is written by hand, citing the human turn that came before the narrowing.
    let log = fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    let turn = log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|e| e["kind"] == "human.turn")
        .and_then(|e| e["seq"].as_u64())
        .expect("a human turn in the log");
    let ev = bf::events::append(
        &fx.project(),
        "amend.approve",
        serde_json::json!({"amend": "A-1", "turn": turn, "quote": "approve the amendment",
            "reopen_units": [], "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0}),
    )
    .unwrap();
    let seq = ev["seq"].as_u64().unwrap();
    let st = fx.must(&["status", "--json"]).json();
    assert_ne!(st["amendments"][0]["status"], "approved", "{st}");
    let ignored = st["ignored_events"].to_string();
    assert!(ignored.contains(&format!("event {seq}")), "{ignored}");
}
