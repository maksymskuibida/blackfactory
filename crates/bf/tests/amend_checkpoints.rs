//! U-69: inside an amendment a checkpoint is inserted or split by renaming the later checkpoints one
//! number forward (R-64). Nothing changes until the human approves.

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;

fn put_ok(fx: &Fx, kind: &str, text: &str) {
    let o = fx.put(kind, text);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// Four parts, four units, three checkpoints: K-1 holds U-1, K-2 holds U-2 and U-3, K-3 holds U-4.
/// With `shared`, U-2 and U-3 name the same file and sit in different waves, and K-2 holds a fifth
/// unit (U-5, check X-8) as well.
fn plan4(fx: &Fx, shared: bool) {
    let top = if shared { 5 } else { 4 };
    for n in 1..=top {
        put_ok(
            fx,
            "requirement",
            &format!("id=\"R-{n}\"\nkind=\"requirement\"\ntitle=\"Part {n}\"\nacceptance=[\"src/{n}.txt exists\"]\n"),
        );
    }
    let mut units = vec![(1, 1), (2, 2), (3, 2), (4, 3)];
    if shared {
        units.push((5, 2));
    }
    for (n, k) in units {
        let x = if n == 5 { 8 } else { n };
        let file = if shared && (n == 2 || n == 3) {
            "src/shared.txt".to_string()
        } else {
            format!("src/{n}.txt")
        };
        let dep = if shared && n == 3 {
            "depends_on=[\"U-1\"]\n"
        } else {
            ""
        };
        put_ok(
            fx,
            "unit",
            &format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"Unit {n}\"\ndelivers=[\"R-{n}\"]\nfiles=[\"{file}\"]\n{dep}checkpoint=\"K-{k}\"\nestimate=1\n"),
        );
        put_ok(
            fx,
            "check",
            &format!("id=\"X-{x}\"\nkind=\"check\"\ntitle=\"Check {n}\"\nhow=\"command\"\ncommand=\"test -f src/{n}.txt\"\nproves=[\"R-{n}\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n"),
        );
    }
    let (after, proves) = if shared {
        (r#"["U-2","U-3","U-5"]"#, r#"["R-2","R-3","R-5"]"#)
    } else {
        (r#"["U-2","U-3"]"#, r#"["R-2","R-3"]"#)
    };
    put_ok(fx, "check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"First\"\nhow=\"command\"\ncommand=\"test -f src/1.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    put_ok(fx, "check", &format!("id=\"X-6\"\nkind=\"check\"\ntitle=\"Middle\"\nhow=\"command\"\ncommand=\"ls src/2.txt src/3.txt\"\nproves={proves}\nscope=\"checkpoint\"\n"));
    put_ok(fx, "check", "id=\"X-7\"\nkind=\"check\"\ntitle=\"Last\"\nhow=\"command\"\ncommand=\"test -f src/4.txt\"\nproves=[\"R-4\"]\nscope=\"checkpoint\"\n");
    put_ok(fx, "checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Start\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-5\"]\nstop_why=\"No stop.\"\n");
    put_ok(fx, "checkpoint", &format!("id=\"K-2\"\nkind=\"checkpoint\"\ntitle=\"Middle\"\nafter={after}\nproves={proves}\nchecks=[\"X-6\"]\nstop=true\nstop_why=\"Look at the middle.\"\n"));
    put_ok(fx, "checkpoint", "id=\"K-3\"\nkind=\"checkpoint\"\ntitle=\"End\"\nafter=[\"U-4\"]\nproves=[\"R-4\"]\nchecks=[\"X-7\"]\nstop_why=\"No stop.\"\n");
    fx.commit_all("records");
}

fn rec(fx: &Fx, id: &str) -> Value {
    fx.must(&["record", "get", id, "--json"]).json()
}

fn list(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} in {v}"))
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

/// Narrow the open amendment with its whole impact set and have it approved.
fn approve(fx: &Fx, opened: &Value) {
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
    fx.human("Yes, approve the change to the checkpoints.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the change"]);
}

fn open(fx: &Fx, id: &str) -> Value {
    fx.must(&["amend", "open", "--repair", id, "--json"]).json()
}

#[test]
fn an_insertion_after_renames_the_later_checkpoints_and_everything_that_named_them() {
    let fx = Fx::new();
    plan4(&fx, false);
    let opened = open(&fx, "K-2");
    let o = fx
        .must(&["amend", "checkpoint", "after", "K-1", "U-3", "--json"])
        .json();
    assert_eq!(
        o["renamed"],
        json!([{"from": "K-2", "to": "K-3"}, {"from": "K-3", "to": "K-4"}])
    );
    assert_eq!(o["plan_problems"], json!([]), "{o}");
    // Nothing is written before the human approves.
    assert_eq!(list(&rec(&fx, "K-2"), "after"), ["U-2", "U-3"]);
    assert_eq!(rec(&fx, "U-4")["checkpoint"], "K-3");
    approve(&fx, &opened);

    // The new checkpoint holds the unit that moved and the check of the whole checkpoint.
    let k2 = rec(&fx, "K-2");
    assert_eq!(list(&k2, "after"), ["U-3"]);
    assert_eq!(list(&k2, "proves"), ["R-3"]);
    assert_eq!(list(&k2, "checks"), ["X-8"]);
    // The stop for the human follows the unit away from the checkpoint it left.
    assert_eq!(k2["stop"], true, "{k2}");
    let x8 = rec(&fx, "X-8");
    assert_eq!(x8["scope"], "checkpoint");
    assert_eq!(x8["command"], "test -f src/3.txt");
    assert!(x8.get("unit").is_none(), "{x8}");
    // The renamed ones keep their title, units, proven parts, checks and stop.
    let k3 = rec(&fx, "K-3");
    assert_eq!(k3["title"], "Middle");
    assert_eq!(list(&k3, "after"), ["U-2"]);
    assert_eq!(list(&k3, "proves"), ["R-2"]);
    assert_eq!(list(&k3, "checks"), ["X-6"]);
    assert_eq!(k3["stop"], true);
    assert_eq!(k3["stop_why"], "Look at the middle.");
    let k4 = rec(&fx, "K-4");
    assert_eq!(k4["title"], "End");
    assert_eq!(list(&k4, "after"), ["U-4"]);
    assert_eq!(list(&k4, "checks"), ["X-7"]);
    // Every unit follows its checkpoint.
    for (u, k) in [
        ("U-1", "K-1"),
        ("U-2", "K-3"),
        ("U-3", "K-2"),
        ("U-4", "K-4"),
    ] {
        assert_eq!(rec(&fx, u)["checkpoint"], k, "{u}");
    }
    assert_eq!(rec(&fx, "K-1")["title"], "Start");
    // The result is a plan that passes the plan check, and the amendment is a repair.
    fx.must(&["plan", "check"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["amendments"][0]["status"], "approved", "{st}");
    assert_eq!(st["repairs"], json!(["A-1"]), "{st}");
}

#[test]
fn an_insertion_before_takes_the_number_of_the_named_checkpoint() {
    let fx = Fx::new();
    plan4(&fx, false);
    let opened = open(&fx, "K-3");
    fx.must(&["amend", "checkpoint", "before", "K-3", "U-3"]);
    approve(&fx, &opened);
    assert_eq!(list(&rec(&fx, "K-2"), "after"), ["U-2"]);
    assert_eq!(list(&rec(&fx, "K-3"), "after"), ["U-3"]);
    assert_eq!(rec(&fx, "K-4")["title"], "End");
    assert_eq!(rec(&fx, "U-4")["checkpoint"], "K-4");
    assert_eq!(rec(&fx, "U-3")["checkpoint"], "K-3");
    fx.must(&["plan", "check"]);
}

#[test]
fn a_split_keeps_the_named_units_and_puts_the_rest_on_a_checkpoint_right_after() {
    let fx = Fx::new();
    plan4(&fx, false);
    let opened = open(&fx, "K-2");
    let o = fx
        .must(&["amend", "checkpoint", "split", "K-2", "U-2", "--json"])
        .json();
    assert_eq!(o["renamed"], json!([{"from": "K-3", "to": "K-4"}]), "{o}");
    approve(&fx, &opened);
    let k2 = rec(&fx, "K-2");
    assert_eq!(list(&k2, "after"), ["U-2"]);
    assert_eq!(list(&k2, "proves"), ["R-2"]);
    assert_eq!(k2["stop"], true);
    let k3 = rec(&fx, "K-3");
    assert_eq!(list(&k3, "after"), ["U-3"]);
    assert_eq!(list(&k3, "proves"), ["R-3"]);
    // The stop for the human is not left behind by the units that move.
    assert_eq!(k3["stop"], true, "{k3}");
    assert_eq!(rec(&fx, "K-4")["title"], "End");
    for (u, k) in [("U-2", "K-2"), ("U-3", "K-3"), ("U-4", "K-4")] {
        assert_eq!(rec(&fx, u)["checkpoint"], k, "{u}");
    }
    fx.must(&["plan", "check"]);
}

#[test]
fn units_that_share_files_are_put_one_after_the_other() {
    let fx = Fx::new();
    plan4(&fx, true);
    let opened = open(&fx, "K-2");
    // U-5 stays on K-2; U-2 and U-3 go onto the checkpoint right after it.
    fx.must(&[
        "amend",
        "checkpoint",
        "split",
        "K-2",
        "U-5",
        "--title",
        "Both",
    ]);
    approve(&fx, &opened);
    let u3 = rec(&fx, "U-3");
    assert!(list(&u3, "depends_on").contains(&"U-2".to_string()), "{u3}");
    assert_eq!(rec(&fx, "K-3")["title"], "Both");
    assert_eq!(list(&rec(&fx, "K-3"), "after"), ["U-2", "U-3"]);
    fx.must(&["plan", "check"]);
}

#[test]
fn a_checkpoint_that_has_started_is_refused_by_name() {
    let fx = Fx::new();
    plan4(&fx, false);
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    open(&fx, "K-2");
    // Everything from K-1 on would be renamed, and K-1 holds U-1, which has started.
    let o = fx.refused(&["amend", "checkpoint", "before", "K-1", "U-3"]);
    assert!(o.stderr.contains("K-1"), "{}", o.stderr);
    assert!(o.stderr.contains("U-1"), "{}", o.stderr);
    assert!(o.stderr.contains("started"), "{}", o.stderr);
    // Nothing moved.
    assert_eq!(rec(&fx, "K-1")["title"], "Start");
    assert_eq!(rec(&fx, "U-3")["checkpoint"], "K-2");
    // After the checkpoint that started, the later ones can be renamed.
    fx.must(&["amend", "checkpoint", "after", "K-1", "U-3"]);
}

#[test]
fn a_unit_that_has_started_cannot_move() {
    let fx = Fx::new();
    plan4(&fx, false);
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    open(&fx, "K-3");
    let o = fx.refused(&["amend", "checkpoint", "after", "K-3", "U-1"]);
    assert!(o.stderr.contains("U-1"), "{}", o.stderr);
    assert!(o.stderr.contains("started"), "{}", o.stderr);
}

#[test]
fn only_a_checkpoint_can_be_named_and_none_may_be_left_without_a_unit() {
    let fx = Fx::new();
    plan4(&fx, false);
    open(&fx, "K-2");
    let o = fx.refused(&["amend", "checkpoint", "after", "R-1", "U-3"]);
    assert!(o.stderr.contains("R-1"), "{}", o.stderr);
    let o = fx.refused(&["amend", "checkpoint", "after", "K-1", "U-4"]);
    assert!(o.stderr.contains("K-3"), "{}", o.stderr);
    assert!(o.stderr.contains("no unit"), "{}", o.stderr);
    let o = fx.refused(&["amend", "checkpoint", "split", "K-2", "U-2", "U-3"]);
    assert!(o.stderr.contains("K-2"), "{}", o.stderr);
    let o = fx.refused(&["amend", "checkpoint", "after", "K-1", "U-9"]);
    assert!(o.stderr.contains("U-9"), "{}", o.stderr);
}

#[test]
fn the_command_needs_an_open_amendment() {
    let fx = Fx::new();
    plan4(&fx, false);
    let o = fx.refused(&["amend", "checkpoint", "after", "K-1", "U-3"]);
    assert!(o.stderr.contains("no amendment is open"), "{}", o.stderr);
}

#[test]
fn the_plan_check_runs_on_the_result_before_approval() {
    let fx = Fx::new();
    plan4(&fx, false);
    let opened = open(&fx, "K-2");
    fx.must(&["amend", "checkpoint", "after", "K-1", "U-3"]);
    // The check of the middle checkpoint stops proving R-2 once the amendment is read.
    let f = fx.root.join("x6.toml");
    fs::write(
        &f,
        "id=\"X-6\"\nkind=\"check\"\ntitle=\"Middle\"\nhow=\"command\"\ncommand=\"ls src/2.txt src/3.txt\"\nproves=[\"R-3\"]\nscope=\"checkpoint\"\n",
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
    let keep: Vec<String> = list(&opened, "impact_ids");
    let n = fx.root.join("narrow.json");
    fs::write(
        &n,
        json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", n.to_str().unwrap()]);
    fx.human("Yes, approve the change to the checkpoints.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert!(o.stderr.contains("plan check"), "{}", o.stderr);
    assert!(o.stderr.contains("K-3"), "{}", o.stderr);
    // Still open, and nothing was renamed.
    assert_eq!(rec(&fx, "K-3")["title"], "End");
}

#[test]
fn the_amendment_page_shows_a_renamed_checkpoint_as_moved_under_its_old_number() {
    let fx = Fx::new();
    plan4(&fx, false);
    open(&fx, "K-2");
    fx.must(&["amend", "checkpoint", "after", "K-1", "U-3"]);
    let v = fx.must(&["view", "run", "--json"]).json();
    let plan = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "A-1-plan")
        .unwrap_or_else(|| panic!("{v}"));
    let rows = plan["rows"].as_array().unwrap();
    let row = |id: &str| rows.iter().find(|r| r["id"] == id);
    let moved = row("K-2").unwrap_or_else(|| panic!("{plan}"));
    assert_eq!(moved["title"], "Middle");
    assert_eq!(moved["handled"]["text"], "moved to K-3", "{moved}");
    let end = row("K-3").unwrap_or_else(|| panic!("{plan}"));
    assert_eq!(end["handled"]["text"], "moved to K-4", "{end}");
    assert!(plan["title"].as_str().unwrap().contains("plan repair"));
    // The new checkpoint is listed as new.
    assert!(rows.iter().any(|r| r["handled"]["text"] == "new"), "{plan}");
}

#[test]
fn an_amendment_with_an_insertion_and_no_design_part_counts_as_a_repair() {
    let fx = Fx::new();
    plan4(&fx, false);
    // Opened without the mark, over plan records only.
    fx.must(&["amend", "open", "K-2"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["repairs"], json!([]), "{st}");
    fx.must(&["amend", "checkpoint", "after", "K-1", "U-3"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["repairs"], json!(["A-1"]), "{st}");
    assert_eq!(st["amendments"][0]["repair"], true, "{st}");
}

#[test]
fn a_write_that_fails_partway_leaves_the_plan_as_it_was_and_can_be_tried_again() {
    let fx = Fx::new();
    plan4(&fx, false);
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    let pos: Vec<String> = ["after", "K-1", "U-3"]
        .iter()
        .map(|x| x.to_string())
        .collect();
    let op = bf::amend::op_from_args(&pos, None).unwrap();
    let edit = bf::amend::edit_of(&p, &st, &[op]).unwrap();
    let dir = fx.root.join("factory/records");
    let names = ["K-2", "K-3", "K-4", "U-3"];
    let read = || -> Vec<String> {
        names
            .iter()
            .map(|n| fs::read_to_string(dir.join(format!("{n}.toml"))).unwrap_or_default())
            .collect()
    };
    let before = read();
    // The new check is written last: a directory in its place makes that write fail.
    let blocker = dir.join("X-8.toml");
    fs::create_dir(&blocker).unwrap();
    assert!(bf::amend::write_edit(&p, &edit).is_err());
    assert_eq!(
        read(),
        before,
        "records written before the failure were put back"
    );
    fs::remove_dir(&blocker).unwrap();
    bf::amend::write_edit(&p, &edit).unwrap();
    assert_ne!(read(), before);
    assert_eq!(rec(&fx, "U-3")["checkpoint"], "K-2");
}
