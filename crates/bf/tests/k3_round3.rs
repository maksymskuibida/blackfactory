//! The fix of the K-3 checkpoint review, round 3: a kept record of an amendment's impact set is not put
//! while the amendment waits, and the fingerprint the human's reading is checked against covers the
//! kept records, so an edit by any route after the narrowing makes approval refuse.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

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

/// U-1 with the checks X-1 and X-4.
fn two_checks() -> Fx {
    let fx = Fx::tiny_project();
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

/// The amendment A-1 over X-1, narrowed keeping the whole impact set; X-4 is among the kept.
fn narrowed_keeping_x4(fx: &Fx) -> Vec<String> {
    let opened = fx
        .must(&["amend", "open", "--repair", "U-1", "--json"])
        .json();
    let keep = ids(&opened["impact_ids"]);
    assert!(keep.contains(&"X-4".to_string()), "{keep:?}");
    narrow(fx, &keep);
    keep
}

#[test]
fn a_kept_record_is_not_put_while_the_amendment_waits_and_the_printed_steps_end_in_success() {
    let fx = two_checks();
    let changed = X4.replace("test -d src", "true");
    fx.must(&["amend", "open", "--repair", "U-1", "--json"]);
    let o = put_check(&fx, &changed);
    assert!(
        o.code != 0,
        "a kept record was put while waiting: {}",
        o.all()
    );
    assert!(o.all().contains("amend revise"), "{}", o.all());
    // The steps: name it with revise, put it while the amendment waits, narrow, the human reads, approve.
    let revised = fx.must(&["amend", "revise", "U-1", "X-4", "--json"]).json();
    let keep = ids(&revised["impact_ids"]);
    assert!(keep.contains(&"X-4".to_string()), "{keep:?}");
    let o = put_check(&fx, &changed);
    assert_eq!(o.code, 0, "put after naming it: {}", o.all());
    narrow(&fx, &keep);
    fx.human("Yes, change both checks.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "change both checks"]);
    assert_eq!(o.code, 0, "the last step: {}", o.all());
    let o = put_check(&fx, &changed);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn an_edit_of_a_kept_record_outside_put_after_the_narrowing_makes_approval_refuse() {
    let fx = two_checks();
    let keep = narrowed_keeping_x4(&fx);
    let _ = &keep;
    // An edit of the file by hand, not by `bf record put`.
    let p = fx.project();
    let mut rec = p.get_record("X-4").unwrap().unwrap();
    rec["command"] = json!("true");
    p.write_record(&rec).unwrap();
    fx.human("Yes, that is fine.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "that is fine"]);
    assert!(
        o.code != 0,
        "approved over an edited kept record: {}",
        o.all()
    );
    assert!(o.all().contains("narrowed"), "{}", o.all());
    // Revise, narrow again, the human reads again: approval passes.
    fx.must(&["amend", "revise", "U-1", "X-4"]);
    narrow(&fx, &keep);
    fx.human("Yes, I read the new narrowing, approve it.");
    let o = fx.bf(&[
        "amend",
        "approve",
        "A-1",
        "--quote",
        "read the new narrowing",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn with_no_change_at_all_approval_passes() {
    let fx = two_checks();
    narrowed_keeping_x4(&fx);
    fx.human("Yes, approve it.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "approve it"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let _: PathBuf = fx.root.clone();
}
