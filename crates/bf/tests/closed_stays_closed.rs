//! U-94: an approved amendment never puts a closed checkpoint back into the run of checks, one that would
//! send a landed unit of a closed checkpoint back to work is refused, and the plan check runs at the
//! approval of every amendment (D-112, D-113).

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn put_ok(fx: &Fx, kind: &str, text: &str) {
    let o = fx.put(kind, text);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// Four parts, four units, three checkpoints: K-1 holds U-1, K-2 holds U-2 and U-3, K-3 holds U-4.
fn plan4(fx: &Fx) {
    for n in 1..=4 {
        put_ok(
            fx,
            "requirement",
            &format!("id=\"R-{n}\"\nkind=\"requirement\"\ntitle=\"Part {n}\"\nacceptance=[\"src/{n}.txt exists\"]\n"),
        );
    }
    for (n, k) in [(1, 1), (2, 2), (3, 2), (4, 3)] {
        put_ok(
            fx,
            "unit",
            &format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"Unit {n}\"\ndelivers=[\"R-{n}\"]\nfiles=[\"src/{n}.txt\"]\ncheckpoint=\"K-{k}\"\nestimate=1\n"),
        );
        put_ok(
            fx,
            "check",
            &format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"Check {n}\"\nhow=\"command\"\ncommand=\"test -f src/{n}.txt\"\nproves=[\"R-{n}\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n"),
        );
    }
    put_ok(fx, "check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"First\"\nhow=\"command\"\ncommand=\"test -f src/1.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    put_ok(fx, "check", "id=\"X-6\"\nkind=\"check\"\ntitle=\"Middle\"\nhow=\"command\"\ncommand=\"ls src/2.txt src/3.txt\"\nproves=[\"R-2\",\"R-3\"]\nscope=\"checkpoint\"\n");
    put_ok(fx, "check", "id=\"X-7\"\nkind=\"check\"\ntitle=\"Last\"\nhow=\"command\"\ncommand=\"test -f src/4.txt\"\nproves=[\"R-4\"]\nscope=\"checkpoint\"\n");
    put_ok(fx, "checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Start\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-5\"]\nstop_why=\"No stop.\"\n");
    put_ok(fx, "checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\ntitle=\"Middle\"\nafter=[\"U-2\",\"U-3\"]\nproves=[\"R-2\",\"R-3\"]\nchecks=[\"X-6\"]\nstop_why=\"No stop.\"\n");
    put_ok(fx, "checkpoint", "id=\"K-3\"\nkind=\"checkpoint\"\ntitle=\"End\"\nafter=[\"U-4\"]\nproves=[\"R-4\"]\nchecks=[\"X-7\"]\nstop_why=\"No stop.\"\n");
    fx.commit_all("records");
}

/// K-1 closed with U-1 landed in it, as the log records it.
fn close_k1(fx: &Fx) {
    let p = fx.project();
    bf::events::append(
        &p,
        "unit.land",
        json!({"unit": "U-1", "hash": "h", "run": "r"}),
    )
    .unwrap();
    bf::events::append(
        &p,
        "checkpoint.close",
        json!({"checkpoint": "K-1", "merged": false, "hash": "h"}),
    )
    .unwrap();
}

/// Reword a part, with a link to another part, and say in a sentence per stage that the others are not
/// affected: the amendment over it is a design amendment that covers every stage.
fn reword(fx: &Fx, id: &str) {
    let other = if id == "R-1" { "R-2" } else { "R-1" };
    put_ok_replace(
        fx,
        "requirement",
        &format!("id=\"{id}\"\nkind=\"requirement\"\ntitle=\"Part {id} reworded\"\nlinks=[\"{other}\"]\nacceptance=[\"the file exists\"]\n"),
    );
}

fn not_affected(fx: &Fx) {
    for (d, stage) in [("D-91", "ui"), ("D-92", "architecture"), ("D-93", "plan")] {
        put_ok(
            fx,
            "decision",
            &format!("id=\"{d}\"\nkind=\"decision\"\ntitle=\"The {stage} stage is not affected by this change\"\nmaker=\"agent\"\nstage=\"{stage}\"\n"),
        );
    }
}

fn put_ok_replace(fx: &Fx, kind: &str, text: &str) {
    let f = fx.root.join("replace.toml");
    fs::write(&f, text).unwrap();
    fx.must(&["record", "put", kind, "--file", f.to_str().unwrap(), "--replace"]);
}

fn open(fx: &Fx, ids: &[&str]) -> Value {
    not_affected(fx);
    let mut args = vec!["amend", "open"];
    args.extend(ids);
    args.extend(["D-91", "D-92", "D-93"]);
    args.push("--json");
    fx.must(&args).json()
}

fn impact(v: &Value) -> Vec<String> {
    v["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn narrow_file(fx: &Fx, keep: &[String], reasons: Value) -> String {
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": keep, "waive": [], "reasons": reasons}).to_string(),
    )
    .unwrap();
    f.to_str().unwrap().to_string()
}

fn last_approval(fx: &Fx) -> Value {
    let log = bf::events::load_log(&fx.project()).unwrap();
    log.events
        .iter()
        .rev()
        .find(|e| e["kind"] == "amend.approve")
        .cloned()
        .expect("an approval in the log")
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn closed_stays_closed_a_reworded_record_of_a_closed_checkpoint_leaves_it_closed() {
    let fx = Fx::new();
    plan4(&fx);
    close_k1(&fx);
    reword(&fx, "R-1");
    reword(&fx, "R-2");
    let opened = open(&fx, &["R-1", "R-2"]);
    let keep: Vec<String> = impact(&opened).into_iter().filter(|x| x != "U-1").collect();
    assert!(keep.contains(&"X-1".to_string()) && keep.contains(&"K-1".to_string()), "{opened}");
    let f = narrow_file(&fx, &keep, json!({"U-1": "landed in a closed checkpoint"}));
    fx.must(&["amend", "narrow", "--file", &f]);
    fx.human("Yes, approve the reworded parts.");
    let o = fx.must(&["amend", "approve", "A-1", "--quote", "approve the reworded parts", "--json"]).json();
    assert_eq!(o["closed_kept"], json!([{"checkpoint": "K-1", "ids": ["R-1"]}]), "{o}");

    let ev = last_approval(&fx);
    // The open checkpoint that proves a changed ID is re-opened; the closed one is not.
    assert_eq!(strs(&ev["reopen_checkpoints"]), ["K-2"], "{ev}");
    let checks = strs(&ev["reopen_checks"]);
    for kept in ["X-1", "X-5"] {
        assert!(!checks.contains(&kept.to_string()), "{kept} reopened: {ev}");
    }
    assert!(checks.contains(&"X-6".to_string()), "{ev}");
    assert_eq!(ev["closed_kept"], json!([{"checkpoint": "K-1", "ids": ["R-1"]}]));

    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.closed.contains_key("K-1"), "K-1 was re-opened");
    assert!(!st.reopen_floor.contains_key("X-5") && !st.reopen_floor.contains_key("X-1"));
    assert!(st.reopen_floor.contains_key("X-6"));
    assert_eq!(st.units["U-1"].state, "landed");

    // No check of the closed checkpoint is asked for again.
    let next = fx.bf(&["next", "--json"]).all();
    assert!(
        !next.contains("run-checkpoint-checks") || !next.contains("K-1"),
        "{next}"
    );
    // `bf log` prints the list.
    let log = fx.must(&["log", "--tail", "1"]).all();
    assert!(log.contains("closed_kept"), "{log}");
}

#[test]
fn closed_stays_closed_the_narrowing_refuses_a_landed_unit_of_a_closed_checkpoint() {
    let fx = Fx::new();
    plan4(&fx);
    close_k1(&fx);
    reword(&fx, "R-1");
    let opened = open(&fx, &["R-1"]);
    let f = narrow_file(&fx, &impact(&opened), json!({}));
    let o = fx.refused(&["amend", "narrow", "--file", &f]);
    let all = o.all();
    assert!(all.contains("U-1") && all.contains("K-1"), "{all}");
    assert!(all.contains("new unit"), "{all}");
    assert!(!all.contains("U-2"), "{all}");
}

#[test]
fn closed_stays_closed_the_approval_refuses_what_an_old_narrowing_kept() {
    let fx = Fx::new();
    plan4(&fx);
    close_k1(&fx);
    reword(&fx, "R-1");
    let opened = open(&fx, &["R-1"]);
    let p = fx.project();
    bf::events::append(
        &p,
        "amend.narrow",
        json!({"amend": "A-1", "keep": impact(&opened), "waive": [], "reasons": {}}),
    )
    .unwrap();
    fx.human("Yes, approve it.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve it"]);
    let all = o.all();
    assert!(all.contains("U-1") && all.contains("K-1"), "{all}");
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.closed.contains_key("K-1"));
}

fn package(root: &Path) {
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nautotests = false\n\n[[test]]\nname = \"all\"\npath = \"tests/all.rs\"\n",
    )
    .unwrap();
    fs::write(root.join("tests/all.rs"), "").unwrap();
    fs::write(root.join("src/lib.rs"), "").unwrap();
}

#[test]
fn closed_stays_closed_a_changed_check_that_names_a_missing_test_target_is_refused_at_the_approval() {
    let fx = Fx::new();
    package(&fx.root);
    plan4(&fx);
    put_ok_replace(
        &fx,
        "check",
        "id=\"X-7\"\nkind=\"check\"\ntitle=\"Last\"\nhow=\"command\"\ncommand=\"cargo nextest run --package=demo --test nope\"\nproves=[\"R-4\"]\nscope=\"checkpoint\"\n",
    );
    let opened = fx.must(&["amend", "open", "--repair", "X-7", "--json"]).json();
    let f = narrow_file(&fx, &impact(&opened), json!({}));
    fx.must(&["amend", "narrow", "--file", &f]);
    fx.human("Yes, approve the changed check.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the changed check"]);
    let all = o.all();
    assert!(all.contains("nope"), "{all}");
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.amends.iter().all(|a| a.status != "approved"));
}

#[test]
fn closed_stays_closed_the_replay_of_an_old_approval_still_reopens_a_closed_checkpoint() {
    let fx = Fx::new();
    plan4(&fx);
    close_k1(&fx);
    let p = fx.project();
    bf::events::append(&p, "amend.open", json!({"amend": "A-1", "ids": ["R-1"], "impact": null, "impact_ids": ["U-1"]})).unwrap();
    bf::events::append(&p, "amend.narrow", json!({"amend": "A-1", "keep": ["U-1"], "waive": [], "reasons": {}})).unwrap();
    fx.human("Yes, approve the amendment.");
    let turn = bf::events::replay(&p).unwrap().turns.last().unwrap().seq;
    bf::events::append(
        &p,
        "amend.approve",
        json!({"amend": "A-1", "turn": turn, "quote": "approve the amendment", "reopen_units": ["U-1"],
               "reopen_checkpoints": ["K-1"], "reopen_checks": ["X-5"], "receipt_floor": 0, "renamed": []}),
    )
    .unwrap();
    let st = bf::events::replay(&p).unwrap();
    assert!(!st.closed.contains_key("K-1"));
    assert!(st.reopen_floor.contains_key("X-5"));
    assert_eq!(st.units["U-1"].state, "running");
}

#[test]
fn closed_stays_closed_a_check_listed_by_a_closed_and_a_reopened_checkpoint_keeps_counting() {
    let fx = Fx::new();
    plan4(&fx);
    // X-5 belongs to the closed K-1 and to K-2, which the amendment re-opens.
    put_ok_replace(
        &fx,
        "checkpoint",
        "id=\"K-2\"\nkind=\"checkpoint\"\ntitle=\"Middle\"\nafter=[\"U-2\",\"U-3\"]\nproves=[\"R-2\",\"R-3\"]\nchecks=[\"X-6\",\"X-5\"]\nstop_why=\"No stop.\"\n",
    );
    fx.commit_all("shared check");
    close_k1(&fx);
    reword(&fx, "R-1");
    reword(&fx, "R-2");
    let opened = open(&fx, &["R-1", "R-2"]);
    let keep: Vec<String> = impact(&opened).into_iter().filter(|x| x != "U-1").collect();
    let f = narrow_file(&fx, &keep, json!({"U-1": "landed in a closed checkpoint"}));
    fx.must(&["amend", "narrow", "--file", &f]);
    fx.human("Yes, approve the reworded parts.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the reworded parts"]);
    let ev = last_approval(&fx);
    assert_eq!(strs(&ev["reopen_checkpoints"]), ["K-2"], "{ev}");
    let checks = strs(&ev["reopen_checks"]);
    assert!(checks.contains(&"X-6".to_string()), "{ev}");
    assert!(!checks.contains(&"X-5".to_string()), "{ev}");
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.closed.contains_key("K-1"));
    assert!(!st.reopen_floor.contains_key("X-5"));
}
