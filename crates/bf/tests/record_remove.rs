//! U-68: `bf record remove <U-n>`, the one way a unit record leaves the plan (R-73, finding F-226): only a
//! unit, only inside an open plan repair that names it, never a started unit, never while a record still
//! names it. The narrowing lists it as removed, a withdrawal puts it back, and after the approval its ID
//! is never put again without an amendment.

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;

const U3: &str = "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Third\"\ndelivers=[\"R-2\"]\nfiles=[\"src/third.txt\"]\ncheckpoint=\"K-1\"\nestimate=1\n";
const X4: &str = "id=\"X-4\"\nkind=\"check\"\ntitle=\"Third file\"\nhow=\"command\"\ncommand=\"test -f src/third.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-3\"\n";
const X4_MOVED: &str = "id=\"X-4\"\nkind=\"check\"\ntitle=\"Third file\"\nhow=\"command\"\ncommand=\"test -f src/third.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n";
const K1: &str = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\",\"U-3\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\n";
const K1_OLD: &str = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\n";

/// The tiny project with a third unit that only a check and the checkpoint name, in an approved plan.
fn project() -> Fx {
    let fx = Fx::tiny_project();
    for (k, t) in [("unit", U3), ("check", X4)] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    put_replace(&fx, "checkpoint", "K-1", K1);
    fx.commit_all("a third unit");
    fx.start_test_run();
    fx
}

fn put_replace(fx: &Fx, kind: &str, id: &str, text: &str) {
    let f = fx.root.join(format!("{id}.toml"));
    fs::write(&f, text).unwrap();
    fx.must(&["record", "put", kind, "--file", f.to_str().unwrap(), "--replace"]);
}

fn narrow(fx: &Fx, opened: &Value) -> Value {
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap(), "--json"]).json()
}

fn open_repair(fx: &Fx, ids: &[&str]) -> Value {
    let mut a = vec!["amend", "open", "--repair"];
    a.extend_from_slice(ids);
    a.push("--json");
    fx.must(&a).json()
}

fn free_the_unit(fx: &Fx) {
    put_replace(fx, "check", "X-4", X4_MOVED);
    put_replace(fx, "checkpoint", "K-1", K1_OLD);
}

#[test]
fn record_remove_is_refused_for_a_record_that_is_not_a_unit() {
    let fx = project();
    open_repair(&fx, &["X-4", "U-3", "K-1"]);
    let o = fx.refused(&["record", "remove", "R-1"]);
    assert!(o.stderr.contains("R-1 is not a unit"), "{}", o.stderr);
    let o = fx.refused(&["record", "remove", "X-4"]);
    assert!(o.stderr.contains("X-4 is not a unit"), "{}", o.stderr);
    assert!(fx.root.join("factory/records/R-1.toml").exists());
}

#[test]
fn record_remove_is_refused_with_no_plan_repair_open_that_names_the_unit() {
    let fx = project();
    let o = fx.refused(&["record", "remove", "U-3"]);
    assert!(o.stderr.contains("no plan repair that is open names U-3"), "{}", o.stderr);
    // An open repair that names other records only does not count either.
    open_repair(&fx, &["X-4"]);
    let o = fx.refused(&["record", "remove", "U-3"]);
    assert!(o.stderr.contains("names U-3"), "{}", o.stderr);
    assert!(fx.root.join("factory/records/U-3.toml").exists());
}

#[test]
fn record_remove_is_refused_for_a_unit_that_has_started() {
    let fx = project();
    fx.must(&["unit", "start", "U-1"]);
    open_repair(&fx, &["U-1"]);
    let o = fx.refused(&["record", "remove", "U-1"]);
    assert!(o.stderr.contains("U-1 is running"), "{}", o.stderr);
    assert!(fx.root.join("factory/records/U-1.toml").exists());
}

#[test]
fn record_remove_is_refused_while_a_record_still_names_the_unit_and_names_each() {
    let fx = project();
    open_repair(&fx, &["U-3", "X-4", "K-1", "U-1"]);
    let o = fx.refused(&["record", "remove", "U-3"]);
    assert!(o.stderr.contains("K-1") && o.stderr.contains("X-4"), "{}", o.stderr);
    // Another unit that depends on it, and a handoff that names it, are named too.
    let o = fx.refused(&["record", "remove", "U-1"]);
    for id in ["U-2", "H-1", "X-1"] {
        assert!(o.stderr.contains(id), "{}", o.stderr);
    }
    assert!(fx.root.join("factory/records/U-3.toml").exists());
}

#[test]
fn record_remove_takes_the_unit_out_is_listed_as_removed_and_a_withdrawal_puts_it_back() {
    let fx = project();
    let was = fs::read_to_string(fx.root.join("factory/records/U-3.toml")).unwrap();
    let opened = open_repair(&fx, &["U-3", "X-4", "K-1"]);
    free_the_unit(&fx);
    let o = fx.must(&["record", "remove", "U-3"]);
    assert!(o.stdout.contains("removed U-3"), "{}", o.stdout);
    assert!(!fx.root.join("factory/records/U-3.toml").exists());
    narrow(&fx, &opened);
    let log = fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    let ev: Value = log
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|e| e["kind"] == "amend.narrow")
        .expect("a narrowing");
    let changes = ev["changes"].as_array().expect("changes").clone();
    let u3 = changes.iter().find(|c| c["id"] == "U-3").expect("U-3 listed");
    assert_eq!(u3["how"], "removed", "{u3}");
    assert_eq!(u3["was"]["id"], "U-3", "{u3}");
    let w = fx.must(&["amend", "withdraw", "A-1", "--reason", "keep the unit"]);
    assert!(w.stdout.contains("U-3"), "{}", w.stdout);
    assert_eq!(fs::read_to_string(fx.root.join("factory/records/U-3.toml")).unwrap(), was);
}

#[test]
fn record_remove_approved_takes_the_unit_out_of_the_plan_for_good() {
    let fx = project();
    let opened = open_repair(&fx, &["U-3", "X-4", "K-1"]);
    free_the_unit(&fx);
    fx.must(&["record", "remove", "U-3"]);
    narrow(&fx, &opened);
    fx.human("Yes, approve the removal of the third unit.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the removal of the third unit"]);
    fx.must(&["plan", "check"]);
    // The same ID is put again only by an amendment.
    let f = fx.root.join("again.toml");
    fs::write(&f, U3).unwrap();
    let o = fx.refused(&["record", "put", "unit", "--file", f.to_str().unwrap()]);
    assert!(o.stderr.contains("U-3"), "{}", o.stderr);
    assert!(!fx.root.join("factory/records/U-3.toml").exists());
}

/// A later approval that writes `missing` again for a removed unit keeps it removed: only an approval
/// that writes a real hash brings it back.
#[test]
fn record_remove_a_second_approval_of_missing_keeps_the_unit_removed() {
    let fx = project();
    let opened = open_repair(&fx, &["U-3", "X-4", "K-1"]);
    free_the_unit(&fx);
    fx.must(&["record", "remove", "U-3"]);
    narrow(&fx, &opened);
    fx.human("Yes, approve the removal of the third unit.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the removal of the third unit"]);
    let p = fx.project();
    assert!(!bf::gates::approved_missing(&bf::events::replay(&p).unwrap(), "U-3"));
    // A second repair is approved with content that holds the ID as `missing` again.
    let opened2 = fx.must(&["amend", "open", "--repair", "X-4", "--json"]).json();
    let keep: Vec<String> = opened2["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow2.json");
    fs::write(&f, json!({"amend": "A-2", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap(), "--json"]);
    fx.human("Yes, approve the second repair too.");
    let turn = bf::events::replay(&p).unwrap().turns.last().unwrap().seq;
    bf::events::append(
        &p,
        "amend.approve",
        json!({"amend": "A-2", "turn": turn, "quote": "approve the second repair too",
            "content": {"U-3": {"h": "missing"}},
            "reopen_units": [], "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0}),
    )
    .unwrap();
    let st = bf::events::replay(&p).unwrap();
    let a2 = st.amends.iter().find(|a| a.id == "A-2").unwrap();
    assert_eq!(a2.approved.as_ref().unwrap()["U-3"]["h"], "missing");
    assert!(!bf::gates::approved_missing(&st, "U-3"));
}

/// R-43 acceptance 4: a log of an earlier engine that bound a unit after its approval replays unchanged;
/// only a unit that was in force and removed stays out.
#[test]
fn record_remove_a_legacy_unit_bind_after_approval_replays_unchanged() {
    let fx = project();
    let opened = open_repair(&fx, &["U-3"]);
    narrow(&fx, &opened);
    fx.human("Yes, approve the repair.");
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    let turn = st.turns.last().unwrap().seq;
    bf::events::append(
        &p,
        "amend.approve",
        json!({"amend": "A-1", "turn": turn, "quote": "approve the repair",
            "content": {"U-7": {"h": "missing"}},
            "reopen_units": [], "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0}),
    )
    .unwrap();
    let before = bf::events::replay(&p).unwrap();
    assert!(bf::gates::approved_missing(&before, "U-7"));
    bf::events::append(&p, "amend.bind", json!({"id": "U-7", "content_h": "abc"})).unwrap();
    let after = bf::events::replay(&p).unwrap();
    assert!(!bf::gates::approved_missing(&after, "U-7"));
    assert_eq!(after.approved_content["U-7"].1["h"], "abc");
}
