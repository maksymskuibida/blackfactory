//! The fix of the K-3 checkpoint review, round 5: the changes section of the amendment page shows
//! every field that differs between the approved content and the content now, old and new, for every
//! kind of record. It reads the stored narrowing, which holds both.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;

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

/// The lines of the row of `id` in the changes section of the page.
fn lines(fx: &Fx, id: &str) -> Vec<String> {
    let v = fx.must(&["view", "run", "--json"]).json();
    let row = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["id"] == "A-1-changes")
        .flat_map(|s| s["rows"].as_array().unwrap().clone())
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no row for {id}"));
    row["body"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["text"].as_str().unwrap_or("").to_string())
        .collect()
}

fn opened() -> (Fx, Vec<String>) {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx
        .must(&["amend", "open", "--repair", "U-1", "--json"])
        .json();
    let keep = ids(&o["impact_ids"]);
    (fx, keep)
}

fn edit(fx: &Fx, id: &str, f: impl Fn(&mut Value)) {
    let p = fx.project();
    let mut rec = p.get_record(id).unwrap().unwrap();
    f(&mut rec);
    p.write_record(&rec).unwrap();
}

#[test]
fn a_kept_unit_edited_to_no_unit_checks_shows_the_field_on_the_page() {
    let (fx, keep) = opened();
    assert!(keep.contains(&"U-2".to_string()), "{keep:?}");
    edit(&fx, "U-2", |r| r["own_checks"] = json!([]));
    narrow(&fx, &keep);
    let l = lines(&fx, "U-2");
    assert_eq!(l, vec!["own_checks: added []".to_string()], "{l:?}");
}

#[test]
fn an_old_and_a_new_value_show_in_that_order() {
    let (fx, keep) = opened();
    edit(&fx, "U-2", |r| r["estimate"] = json!(5));
    narrow(&fx, &keep);
    let l = lines(&fx, "U-2");
    assert_eq!(l, vec!["estimate: was 1, now 5".to_string()], "{l:?}");
}

#[test]
fn a_handoff_change_shows_its_changed_field() {
    let (fx, keep) = opened();
    assert!(keep.contains(&"H-1".to_string()), "{keep:?}");
    edit(&fx, "H-1", |r| r["contract"] = json!("A new tone."));
    narrow(&fx, &keep);
    let l = lines(&fx, "H-1");
    assert_eq!(
        l,
        vec!["contract: was Same tone as the greeting., now A new tone.".to_string()],
        "{l:?}"
    );
}

#[test]
fn a_changed_check_command_still_shows() {
    let (fx, keep) = opened();
    assert!(keep.contains(&"X-2".to_string()), "{keep:?}");
    edit(&fx, "X-2", |r| r["command"] = json!("true"));
    narrow(&fx, &keep);
    let l = lines(&fx, "X-2");
    assert_eq!(
        l,
        vec!["command: was test -f src/bye.txt, now true".to_string()],
        "{l:?}"
    );
}

#[test]
fn a_new_record_shows_its_fields_as_added() {
    let (fx, _) = opened();
    fx.must(&["amend", "revise", "U-1", "X-9"]);
    let o = fx.put(
        "check",
        "id=\"X-9\"\nkind=\"check\"\ntitle=\"Folder\"\nhow=\"command\"\ncommand=\"test -d src\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let revised = fx.must(&["amend", "revise", "U-1", "X-9", "--json"]).json();
    narrow(&fx, &ids(&revised["impact_ids"]));
    let l = lines(&fx, "X-9");
    assert!(
        l.contains(&"command: added test -d src".to_string()),
        "{l:?}"
    );
    assert!(l.contains(&"scope: added unit".to_string()), "{l:?}");
    assert!(l.iter().all(|x| x.contains(": added ")), "{l:?}");
}

#[test]
fn a_change_after_the_narrowing_of_an_old_field_still_voids_the_reading() {
    let (fx, keep) = opened();
    edit(&fx, "H-1", |r| r["contract"] = json!("One."));
    narrow(&fx, &keep);
    edit(&fx, "H-1", |r| r["contract"] = json!("Two."));
    fx.human("Yes, approve it.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "approve it"]);
    assert!(o.code != 0, "{}", o.all());
}
