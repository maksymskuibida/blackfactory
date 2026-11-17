//! U-69: an amendment opened as a plan repair skips the stage rules, and the engine refuses the mark
//! when the set holds a design part (R-41).

mod common;
use common::Fx;
use serde_json::json;
use std::fs;

fn put_ok(fx: &Fx, kind: &str, text: &str) {
    let o = fx.put(kind, text);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn narrow_all(fx: &Fx, opened: &serde_json::Value) {
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

#[test]
fn a_repair_naming_a_requirement_is_refused_by_name() {
    let fx = Fx::new();
    fx.tiny();
    put_ok(
        &fx,
        "component",
        "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\n",
    );
    let o = fx.refused(&["amend", "open", "--repair", "X-1", "R-2", "C-9"]);
    assert!(o.stderr.contains("R-2"), "{}", o.stderr);
    assert!(o.stderr.contains("requirement"), "{}", o.stderr);
    assert!(o.stderr.contains("C-9"), "{}", o.stderr);
    assert!(o.stderr.contains("component"), "{}", o.stderr);
    assert!(!o.stderr.contains("X-1 ("), "{}", o.stderr);
    // Nothing was opened.
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["amendments"], json!([]), "{st}");
}

#[test]
fn every_kind_of_design_part_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    for (kind, id, extra) in [
        ("screen", "S-1", ""),
        ("journey", "J-1", ""),
        (
            "threat",
            "TH-1",
            "protects=\"the file\"\nattack=\"deleting it\"\nmitigation=\"a check\"\n",
        ),
        ("target", "T-1", ""),
        ("decision", "D-1", ""),
    ] {
        put_ok(
            &fx,
            kind,
            &format!("id=\"{id}\"\nkind=\"{kind}\"\ntitle=\"A {kind}\"\n{extra}"),
        );
        let o = fx.refused(&["amend", "open", "--repair", id]);
        assert!(o.stderr.contains(id), "{}", o.stderr);
        assert!(o.stderr.contains(kind), "{}", o.stderr);
    }
}

#[test]
fn a_repair_is_approved_with_no_stage_sentences_and_no_links() {
    let fx = Fx::new();
    fx.tiny();
    let opened = fx
        .must(&["amend", "open", "--repair", "X-1", "--json"])
        .json();
    assert_eq!(opened["repair"], true, "{opened}");
    // The check's command changes inside the amendment.
    let f = fx.root.join("x1.toml");
    fs::write(
        &f,
        "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -s src/hello.txt\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n",
    )
    .unwrap();
    fx.must(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    narrow_all(&fx, &opened);
    // Still the human's approval: without their words it is refused.
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert!(!o.stderr.contains("stage(s)"), "{}", o.stderr);
    fx.human("Yes, approve the new command.");
    fx.must(&[
        "amend",
        "approve",
        "A-1",
        "--quote",
        "approve the new command",
    ]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["amendments"][0]["status"], "approved", "{st}");
    assert_eq!(st["amendments"][0]["repair"], true, "{st}");
    assert_eq!(st["repairs"], json!(["A-1"]), "{st}");
}

#[test]
fn the_same_change_without_the_mark_still_needs_the_stages() {
    let fx = Fx::new();
    fx.tiny();
    let opened = fx.must(&["amend", "open", "X-1", "--json"]).json();
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the new command.");
    let o = fx.refused(&[
        "amend",
        "approve",
        "A-1",
        "--quote",
        "approve the new command",
    ]);
    assert!(o.stderr.contains("stage(s)"), "{}", o.stderr);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["repairs"], json!([]), "{st}");
    assert_eq!(st["amendments"][0]["repair"], false, "{st}");
}

#[test]
fn a_repair_cannot_be_revised_to_hold_a_design_part() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let o = fx.refused(&["amend", "revise", "X-1", "R-1"]);
    assert!(o.stderr.contains("R-1"), "{}", o.stderr);
    // The amendment still covers what it did.
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["amendments"][0]["ids"], json!(["X-1"]), "{st}");
}

#[test]
fn the_amendment_page_says_which_amendments_are_repairs() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let v = fx.must(&["view", "run", "--json"]).json();
    let secs = v["sections"].as_array().unwrap();
    let list = secs.iter().find(|s| s["id"] == "amendments").unwrap();
    let text = list["rows"][0]["handled"]["text"].as_str().unwrap();
    assert!(text.contains("plan repair"), "{text}");
    let plan = secs.iter().find(|s| s["id"] == "A-1-plan").unwrap();
    assert!(
        plan["title"].as_str().unwrap().contains("plan repair"),
        "{plan}"
    );
    // A repair is not shown as missing its stage sentences.
    let ui = secs.iter().find(|s| s["id"] == "A-1-ui").unwrap();
    assert!(!ui["title"].as_str().unwrap().contains("neither"), "{ui}");
}
