//! U-18: only one amendment is open at a time (R-38). It is revised or withdrawn before a new one opens.

mod common;
use common::Fx;
use std::fs;

fn narrow_all(fx: &Fx, a: &str, opened: &serde_json::Value) {
    let f = fx.root.join("narrow.json");
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    fs::write(
        &f,
        serde_json::json!({"amend": a, "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

fn amendments(fx: &Fx) -> Vec<serde_json::Value> {
    fx.must(&["status", "--json"]).json()["amendments"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn opening_a_second_amendment_is_refused_and_names_the_open_one() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    // Same records or other records: either way it is refused.
    for ids in [["R-2"], ["R-1"]] {
        let o = fx.refused(&["amend", "open", ids[0]]);
        assert!(o.stderr.contains("A-1"), "{}", o.stderr);
        assert!(o.stderr.contains("revise"), "{}", o.stderr);
        assert!(o.stderr.contains("withdraw"), "{}", o.stderr);
    }
    assert_eq!(amendments(&fx).len(), 1);
}

#[test]
fn revising_changes_the_records_and_keeps_the_id() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    let o = fx.must(&["amend", "revise", "R-1", "--json"]).json();
    assert_eq!(o["amend"], "A-1");
    let a = amendments(&fx);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["id"], "A-1");
    assert_eq!(a[0]["status"], "open");
    assert_eq!(a[0]["ids"], serde_json::json!(["R-1"]));
    // Still one open amendment: a new one is refused.
    fx.refused(&["amend", "open", "R-2"]);
}

#[test]
fn revising_without_an_open_amendment_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    fx.refused(&["amend", "revise", "R-1"]);
}

#[test]
fn a_revised_amendment_needs_a_human_message_after_the_revision() {
    let fx = Fx::new();
    fx.tiny();
    // U-19: both sets cover every stage and link their requirement, so only the revision stops approval.
    for (k, t) in [
        ("component", "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n"),
        ("component", "id=\"C-8\"\nkind=\"component\"\ntitle=\"Greeting wording\"\nlinks=[\"R-1\"]\n"),
        ("decision", "id=\"D-91\"\nkind=\"decision\"\ntitle=\"The screens are not affected: none shows the wording\"\nmaker=\"agent\"\nstage=\"ui\"\n"),
        ("decision", "id=\"D-92\"\nkind=\"decision\"\ntitle=\"The plan is not affected: the units already exist\"\nmaker=\"agent\"\nstage=\"plan\"\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, "A-1", &opened);
    // The human says yes to the narrowed amendment, then it is revised.
    fx.human("Yes, approve the change to the second requirement.");
    let revised = fx
        .must(&["amend", "revise", "R-1", "C-8", "D-91", "D-92", "--json"])
        .json();
    // The earlier message does not count, and the revised amendment must be narrowed again.
    fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    narrow_all(&fx, "A-1", &revised);
    fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    fx.human("Yes, approve the revised change.");
    fx.must(&[
        "amend",
        "approve",
        "A-1",
        "--quote",
        "approve the revised change",
    ]);
    assert_eq!(amendments(&fx)[0]["status"], "approved");
}

#[test]
fn withdrawing_makes_room_for_a_new_amendment() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    fx.refused(&["amend", "open", "R-1"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "wrong records"]);
    let o = fx.must(&["amend", "open", "R-1", "--json"]).json();
    assert_eq!(o["amend"], "A-2");
    let a = amendments(&fx);
    assert_eq!(a[0]["status"], "withdrawn");
    assert_eq!(a[1]["status"], "open");
}
