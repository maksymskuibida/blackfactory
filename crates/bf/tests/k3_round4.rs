//! The fix of the K-3 checkpoint review, round 4: the narrowing lists everything that differs from the
//! last approved content, kept records included, and records the fingerprint of exactly that list. The
//! amendment page shows the stored list, and approval binds what the human read.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;

const X4: &str = "id=\"X-4\"\nkind=\"check\"\ntitle=\"Folder\"\nhow=\"command\"\ncommand=\"test -d src\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n";

fn put_check(fx: &Fx, text: &str) -> Out {
    let f = fx.root.join("x.toml");
    fs::write(&f, text).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let _ = fs::remove_file(&f);
    o
}

fn two_checks() -> Fx {
    let fx = Fx::new();
    fx.tiny();
    let o = put_check(&fx, X4);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("x4");
    fx.start_test_run();
    fx
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn narrow(fx: &Fx, keep: &[String]) {
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

/// The list the latest narrowing stored.
fn stored_changes(fx: &Fx) -> Vec<Value> {
    let log = fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    let ev: Value = log
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|e| e["kind"] == "amend.narrow")
        .expect("a narrowing");
    ev["changes"].as_array().cloned().unwrap_or_default()
}

fn changed_ids(fx: &Fx) -> Vec<String> {
    stored_changes(fx)
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

fn page_has_row(fx: &Fx, section: &str, id: &str) -> bool {
    let v = fx.must(&["view", "run", "--json"]).json();
    v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["id"] == section)
        .flat_map(|s| s["rows"].as_array().unwrap().clone())
        .any(|r| r["id"] == id)
}

fn edit_x4(fx: &Fx) {
    let p = fx.project();
    let mut rec = p.get_record("X-4").unwrap().unwrap();
    rec["command"] = json!("true");
    p.write_record(&rec).unwrap();
}

#[test]
fn the_seven_step_route_lists_the_kept_record_as_changed_in_the_narrowing_and_on_the_page() {
    let fx = two_checks();
    fx.must(&["amend", "open", "--repair", "U-1", "--json"]);
    fx.must(&["amend", "revise", "U-1", "X-4"]);
    let o = put_check(&fx, &X4.replace("test -d src", "true"));
    assert_eq!(o.code, 0, "{}", o.all());
    let revised = fx.must(&["amend", "revise", "U-1", "--json"]).json();
    let keep = ids(&revised["impact_ids"]);
    assert!(keep.contains(&"X-4".to_string()), "{keep:?}");
    narrow(&fx, &keep);
    let list = stored_changes(&fx);
    let x4 = list.iter().find(|c| c["id"] == "X-4").expect("X-4 listed");
    assert_eq!(x4["how"], "changed");
    assert_eq!(x4["kept"], true);
    assert_eq!(x4["record"]["command"], "true");
    assert!(page_has_row(&fx, "A-1-changes", "X-4"));
    // What the human read is what is approved.
    fx.human("Yes, X-4 changes too.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "X-4 changes too"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_hand_edit_of_a_kept_record_before_the_narrowing_is_listed_as_changed() {
    let fx = two_checks();
    let opened = fx
        .must(&["amend", "open", "--repair", "U-1", "--json"])
        .json();
    let keep = ids(&opened["impact_ids"]);
    edit_x4(&fx);
    narrow(&fx, &keep);
    assert!(changed_ids(&fx).contains(&"X-4".to_string()));
    assert!(page_has_row(&fx, "A-1-changes", "X-4"));
}

#[test]
fn a_change_after_the_narrowing_makes_approval_refuse() {
    let fx = two_checks();
    let opened = fx
        .must(&["amend", "open", "--repair", "U-1", "--json"])
        .json();
    narrow(&fx, &ids(&opened["impact_ids"]));
    edit_x4(&fx);
    fx.human("Yes, approve it.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "approve it"]);
    assert!(o.code != 0, "{}", o.all());
}

#[test]
fn with_no_change_the_narrowing_lists_only_what_the_amendment_changes_and_approval_passes() {
    let fx = two_checks();
    let opened = fx
        .must(&["amend", "open", "--repair", "U-1", "--json"])
        .json();
    narrow(&fx, &ids(&opened["impact_ids"]));
    assert!(changed_ids(&fx).is_empty(), "{:?}", changed_ids(&fx));
    fx.human("Yes, approve it.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "approve it"]);
    assert_eq!(o.code, 0, "{}", o.all());
}
