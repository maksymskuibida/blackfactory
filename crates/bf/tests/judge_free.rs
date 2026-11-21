//! U-68: the script rules of a free reshuffle (R-73, D-95), judged on a fixture plan: what passes, and
//! each refusal by name.

use bf::judge::free::{measures, refusals, Facts, Slice};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Clone)]
struct P {
    units: Vec<Value>,
    handoffs: Vec<Value>,
    cps: Vec<Value>,
    checks: Vec<Value>,
}

fn unit(id: &str, cp: &str, delivers: &[&str], files: &[&str], deps: &[&str]) -> Value {
    json!({"id": id, "kind": "unit", "title": format!("Unit {id}"), "text": format!("Text of {id}"),
        "delivers": delivers, "depends_on": deps, "files": files, "checkpoint": cp,
        "full_suite": "whole suite"})
}

fn check(id: &str, unit: &str, proves: &[&str], names: &str) -> Value {
    json!({"id": id, "kind": "check", "title": id, "how": "command",
        "command": format!("cargo nextest run --package=bf {names}"),
        "proves": proves, "scope": "unit", "unit": unit})
}

fn cp(id: &str, after: &[&str], proves: &[&str]) -> Value {
    json!({"id": id, "kind": "checkpoint", "title": id, "after": after, "proves": proves,
        "checks": [], "stop_why": "A stop."})
}

fn base() -> P {
    P {
        units: vec![
            unit("U-1", "K-1", &["R-1", "R-4"], &["crates/bf/src/a.rs", "crates/bf/tests/aa.rs"], &[]),
            unit("U-2", "K-1", &["R-2"], &["crates/bf/src/b.rs", "crates/bf/tests/bb.rs"], &[]),
            unit("U-3", "K-2", &["R-3"], &["crates/bf/src/c.rs", "crates/bf/tests/cc.rs"], &[]),
            unit("U-5", "K-2", &["R-5"], &["crates/bf/src/e.rs", "crates/bf/tests/ee.rs"], &[]),
        ],
        handoffs: vec![json!({"id": "H-1", "kind": "handoff", "from": "U-1", "to": "U-3", "thing": "a thing"})],
        cps: vec![
            cp("K-1", &["U-1", "U-2"], &["R-1", "R-2"]),
            cp("K-2", &["U-3", "U-5"], &["R-3"]),
        ],
        checks: vec![
            check("X-1", "U-1", &["R-1"], "aa::"),
            check("X-2", "U-2", &["R-2"], "bb::"),
            check("X-3", "U-3", &["R-3"], "cc::"),
            check("X-5", "U-5", &["R-5"], "ee::"),
        ],
    }
}

fn slice(p: &P) -> Slice<'_> {
    Slice {
        units: &p.units,
        handoffs: &p.handoffs,
        checkpoints: &p.cps,
        checks: &p.checks,
    }
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|x| x.to_string()).collect()
}

fn run_with(
    after: &P,
    changes: &[Value],
    ops: usize,
    landed: &[&str],
    closed: &[&str],
    problems: &[String],
) -> Vec<String> {
    let before = base();
    let design = set(&["R-1", "R-2", "R-3", "R-4", "R-5", "R-6"]);
    let (landed, closed) = (set(landed), set(closed));
    refusals(&Facts {
        before: slice(&before),
        after: slice(after),
        changes,
        ops,
        waive: &[],
        landed: &landed,
        closed: &closed,
        design: &design,
        new_problems: problems,
    })
}

fn run(after: &P) -> Vec<String> {
    run_with(after, &[], 0, &[], &[], &[])
}

fn find<'a>(list: &'a mut [Value], id: &str) -> &'a mut Value {
    list.iter_mut().find(|r| r["id"] == id).unwrap()
}

fn drop_id(list: &mut Vec<Value>, id: &str) {
    list.retain(|r| r["id"] != id);
}

fn refused(out: &[String], needle: &str) {
    assert!(
        out.iter().any(|l| l.contains(needle)),
        "expected a refusal naming `{needle}`, got {out:#?}"
    );
}

fn passes(out: &[String]) {
    assert!(out.is_empty(), "expected no refusal, got {out:#?}");
}

// ---- what passes ----

#[test]
fn judge_free_an_unchanged_plan_passes() {
    passes(&run(&base()));
}

#[test]
fn judge_free_a_split_with_new_texts_and_a_new_check_passes_and_is_measured() {
    let mut p = base();
    find(&mut p.units, "U-1")["delivers"] = json!(["R-1"]);
    find(&mut p.units, "U-1")["text"] = json!("A new text for the first half");
    p.units.push(unit("U-4", "K-1", &["R-4"], &["crates/bf/tests/split_t.rs"], &[]));
    p.cps[0]["after"] = json!(["U-1", "U-2", "U-4"]);
    p.checks.push(check("X-4", "U-4", &["R-4"], "split_t:: aa::"));
    passes(&run(&p));
    let m = measures(&base().units, &p.units, &["K-1".to_string(), "K-2".to_string()]);
    let reworded: Vec<&str> = m["reworded"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(reworded, ["U-1", "U-4"]);
    assert_eq!(m["checkpoints"]["K-1"]["before"]["waves"], 1);
    assert_eq!(m["checkpoints"]["K-1"]["after"]["widest"], 3);
    assert_eq!(m["removed"], json!([]));
}

#[test]
fn judge_free_a_unit_whose_title_and_text_alone_changed_passes() {
    let mut p = base();
    find(&mut p.units, "U-2")["title"] = json!("Another title");
    find(&mut p.units, "U-2")["text"] = json!("Entirely other words");
    passes(&run(&p));
    let m = measures(&base().units, &p.units, &["K-1".to_string()]);
    assert_eq!(m["reworded"], json!(["U-2"]));
}

#[test]
fn judge_free_a_merge_with_the_check_and_its_test_files_moved_passes() {
    let mut p = base();
    drop_id(&mut p.units, "U-2");
    let u1 = find(&mut p.units, "U-1");
    u1["delivers"] = json!(["R-1", "R-4", "R-2"]);
    u1["files"] = json!(["crates/bf/src/a.rs", "crates/bf/tests/aa.rs", "crates/bf/src/b.rs", "crates/bf/tests/bb.rs"]);
    p.cps[0]["after"] = json!(["U-1"]);
    find(&mut p.checks, "X-2")["unit"] = json!("U-1");
    passes(&run(&p));
    let m = measures(&base().units, &p.units, &["K-1".to_string()]);
    assert_eq!(m["removed"], json!(["U-2"]));
}

#[test]
fn judge_free_a_unit_moved_to_an_earlier_checkpoint_passes() {
    let mut p = base();
    find(&mut p.units, "U-3")["checkpoint"] = json!("K-1");
    p.cps[0]["after"] = json!(["U-1", "U-2", "U-3"]);
    p.cps[1]["after"] = json!(["U-5"]);
    passes(&run(&p));
}

// ---- what is refused ----

#[test]
fn judge_free_a_changed_check_command_is_refused() {
    let mut p = base();
    find(&mut p.checks, "X-1")["command"] = json!("cargo nextest run --package=bf zz::");
    refused(&run(&p), "X-1: a check in force changes its field command");
}

#[test]
fn judge_free_a_removed_unit_check_is_refused() {
    let mut p = base();
    drop_id(&mut p.checks, "X-5");
    refused(&run(&p), "X-5: a check in force is removed");
}

#[test]
fn judge_free_a_new_check_of_scope_checkpoint_is_refused() {
    let mut p = base();
    let mut c = check("X-9", "U-1", &["R-1"], "aa::");
    c["scope"] = json!("checkpoint");
    p.checks.push(c);
    refused(&run(&p), "X-9: an added check is not a command check of scope unit");
}

fn with_new_unit(command: &str) -> P {
    let mut p = base();
    p.units.push(unit("U-4", "K-1", &["R-6"], &["crates/bf/tests/new_t.rs"], &[]));
    p.cps[0]["after"] = json!(["U-1", "U-2", "U-4"]);
    let mut c = check("X-4", "U-4", &["R-6"], "new_t::");
    c["command"] = json!(command);
    p.checks.push(c);
    p
}

#[test]
fn judge_free_a_new_unit_check_of_true_is_refused() {
    refused(&run(&with_new_unit("true")), "X-4: the command is not of a program");
}

#[test]
fn judge_free_a_new_unit_check_with_no_test_name_is_refused() {
    refused(
        &run(&with_new_unit("cargo nextest run --package=bf")),
        "X-4: the command names no test",
    );
}

#[test]
fn judge_free_a_new_unit_check_naming_only_another_units_tests_is_refused() {
    refused(
        &run(&with_new_unit("cargo nextest run --package=bf aa::")),
        "X-4: the command names no test file that is new",
    );
    refused(
        &run(&with_new_unit("cargo nextest run --package=bf unknown_t::")),
        "X-4: unknown_t is no test file",
    );
    passes(&run(&with_new_unit("cargo nextest run --package=bf new_t:: aa::")));
}

#[test]
fn judge_free_a_new_unit_check_with_another_command_shape_is_refused() {
    refused(
        &run(&with_new_unit("cargo test new_t::")),
        "X-4: the command is not the command of a unit check in force",
    );
}

#[test]
fn judge_free_a_new_unit_check_that_proves_what_the_unit_does_not_deliver_is_refused() {
    let mut p = with_new_unit("cargo nextest run --package=bf new_t::");
    find(&mut p.checks, "X-4")["proves"] = json!(["R-6", "R-1"]);
    refused(&run(&p), "X-4: an added check proves R-1");
}

#[test]
fn judge_free_a_new_check_for_a_unit_that_stays_is_refused() {
    let mut p = base();
    p.checks.push(check("X-9", "U-1", &["R-1"], "aa::"));
    refused(&run(&p), "X-9: an added check is for U-1");
}

#[test]
fn judge_free_a_changed_stop_and_proves_of_a_checkpoint_are_refused() {
    let mut p = base();
    p.cps[0]["stop"] = json!(true);
    p.cps[1]["proves"] = json!(["R-3", "R-1"]);
    let out = run(&p);
    refused(&out, "K-1: a checkpoint changes its field stop");
    refused(&out, "K-2: a checkpoint changes its field proves");
}

#[test]
fn judge_free_a_changed_requirement_and_a_new_decision_are_refused() {
    let p = base();
    let changes = [
        json!({"id": "R-1", "how": "changed", "kept": false, "record": {"id": "R-1", "kind": "requirement"}, "was": {"id": "R-1", "kind": "requirement"}}),
        json!({"id": "D-9", "how": "new", "kept": false, "record": {"id": "D-9", "kind": "decision"}, "was": null}),
    ];
    let out = run_with(&p, &changes, 0, &[], &[], &[]);
    refused(&out, "R-1: a record of kind requirement changes");
    refused(&out, "D-9: a record of kind decision changes");
}

#[test]
fn judge_free_an_item_whose_only_unit_moved_later_is_refused() {
    let mut p = base();
    find(&mut p.units, "U-2")["checkpoint"] = json!("K-2");
    p.cps[0]["after"] = json!(["U-1"]);
    p.cps[1]["after"] = json!(["U-3", "U-5", "U-2"]);
    let out = run(&p);
    refused(&out, "U-2 moves to K-2");
    refused(&out, "K-1: the item R-2");
}

#[test]
fn judge_free_an_item_dropped_from_every_unit_is_refused() {
    let mut p = base();
    find(&mut p.units, "U-1")["delivers"] = json!(["R-1"]);
    refused(&run(&p), "K-1: the item R-4");
}

#[test]
fn judge_free_an_item_taken_from_a_later_unit_that_an_earlier_checkpoint_already_delivered_is_refused() {
    // R-4 is delivered by U-1 of K-1; here U-5 of K-2 also delivers it before, and U-5 loses it.
    let mut before = base();
    find(&mut before.units, "U-5")["delivers"] = json!(["R-5", "R-4"]);
    let after = base();
    let design = set(&["R-1", "R-2", "R-3", "R-4", "R-5"]);
    let none = BTreeSet::new();
    let out = refusals(&Facts {
        before: Slice { units: &before.units, handoffs: &before.handoffs, checkpoints: &before.cps, checks: &before.checks },
        after: slice(&after),
        changes: &[],
        ops: 0,
        waive: &[],
        landed: &none,
        closed: &none,
        design: &design,
        new_problems: &[],
    });
    refused(&out, "K-2: the item R-4");
}

#[test]
fn judge_free_an_item_taken_from_a_unit_that_stays_while_its_check_proves_it_is_refused() {
    let mut p = base();
    find(&mut p.units, "U-1")["delivers"] = json!(["R-4"]);
    // R-1 moves to U-2 of the same checkpoint, so the cover holds, but X-1 still proves it for U-1.
    find(&mut p.units, "U-2")["delivers"] = json!(["R-2", "R-1"]);
    refused(&run(&p), "U-1 stays and no longer delivers R-1");
}

#[test]
fn judge_free_a_checkpoint_left_with_no_unit_is_refused_by_name() {
    let mut p = base();
    for u in ["U-3", "U-5"] {
        find(&mut p.units, u)["checkpoint"] = json!("K-1");
    }
    p.cps[0]["after"] = json!(["U-1", "U-2", "U-3", "U-5"]);
    p.cps[1]["after"] = json!([]);
    refused(&run(&p), "K-2: the checkpoint is left with no unit in `after`");
    // A closed one is not asked, but it may not change.
    let out = run_with(&p, &[], 0, &[], &["K-2"], &[]);
    assert!(!out.iter().any(|l| l.contains("left with no unit")));
    refused(&out, "K-2: a closed checkpoint changes");
}

#[test]
fn judge_free_a_checkpoint_whose_last_unit_was_merged_away_is_refused() {
    let mut p = base();
    drop_id(&mut p.units, "U-3");
    drop_id(&mut p.units, "U-5");
    find(&mut p.units, "U-1")["delivers"] = json!(["R-1", "R-4", "R-3", "R-5"]);
    p.cps[1]["after"] = json!([]);
    refused(&run(&p), "K-2: the checkpoint is left with no unit");
}

#[test]
fn judge_free_a_checkpoint_added_removed_or_operated_on_is_refused() {
    let mut p = base();
    p.cps.push(cp("K-3", &["U-5"], &["R-5"]));
    refused(&run(&p), "K-3: a checkpoint is added");
    let mut q = base();
    q.cps.pop();
    refused(&run(&q), "K-2: a checkpoint is removed");
    refused(&run_with(&base(), &[], 1, &[], &[], &[]), "a checkpoint operation");
}

#[test]
fn judge_free_a_new_unit_that_delivers_an_unknown_id_or_nothing_is_refused() {
    let mut p = base();
    p.units.push(unit("U-4", "K-1", &["R-77"], &[], &[]));
    p.units.push(unit("U-6", "K-1", &[], &[], &[]));
    p.cps[0]["after"] = json!(["U-1", "U-2", "U-4", "U-6"]);
    let out = run(&p);
    refused(&out, "U-4 delivers R-77, which is no record of the design");
    refused(&out, "U-6 delivers no item");
}

#[test]
fn judge_free_a_unit_check_moved_to_a_unit_that_does_not_deliver_what_it_proves_is_refused() {
    let mut p = base();
    drop_id(&mut p.units, "U-2");
    p.cps[0]["after"] = json!(["U-1"]);
    find(&mut p.units, "U-1")["delivers"] = json!(["R-1", "R-4"]);
    find(&mut p.checks, "X-2")["unit"] = json!("U-1");
    let out = run(&p);
    refused(&out, "X-2: the unit check moves to U-1, which does not deliver R-2");
}

#[test]
fn judge_free_a_unit_check_moved_to_a_later_checkpoint_or_away_from_a_staying_unit_is_refused() {
    let mut p = base();
    drop_id(&mut p.units, "U-2");
    p.cps[0]["after"] = json!(["U-1"]);
    find(&mut p.units, "U-3")["delivers"] = json!(["R-3", "R-2"]);
    find(&mut p.checks, "X-2")["unit"] = json!("U-3");
    refused(&run(&p), "X-2: the unit check moves to U-3 of K-2, a later checkpoint than K-1");
    let mut q = base();
    find(&mut q.checks, "X-5")["unit"] = json!("U-3");
    refused(&run(&q), "X-5: the unit check moves away from U-5, which stays");
}

#[test]
fn judge_free_a_landed_unit_and_a_closed_checkpoint_may_not_change() {
    let mut p = base();
    find(&mut p.units, "U-2")["text"] = json!("changed");
    refused(&run_with(&p, &[], 0, &["U-2"], &[], &[]), "U-2: a landed unit changes");
    let mut q = base();
    q.cps[0]["after"] = json!(["U-1", "U-2", "U-3"]);
    find(&mut q.units, "U-3")["checkpoint"] = json!("K-1");
    refused(&run_with(&q, &[], 0, &[], &["K-1"], &[]), "K-1: a closed checkpoint changes");
}

#[test]
fn judge_free_a_unit_that_stays_keeps_its_own_check_fields() {
    for (field, value) in [
        ("tests", json!("cargo nextest run --package=bf aa::")),
        ("full_suite", json!("another sentence")),
        ("repo", json!("other")),
        ("own_checks", json!([])),
        ("own_checks_reason", json!("a reason")),
    ] {
        let mut p = base();
        find(&mut p.units, "U-2")[field] = value;
        refused(&run(&p), &format!("U-2 stays and its field {field} changes"));
    }
}

#[test]
fn judge_free_a_new_unit_carries_no_own_checks_and_a_tests_command_only_in_the_check_form() {
    let mut p = with_new_unit("cargo nextest run --package=bf new_t::");
    find(&mut p.units, "U-4")["own_checks"] = json!([]);
    refused(&run(&p), "U-4 is added with the field own_checks");
    let mut q = with_new_unit("cargo nextest run --package=bf new_t::");
    find(&mut q.units, "U-4")["tests"] = json!("true");
    refused(&run(&q), "U-4 tests: the command is not of a program");
    let mut ok = with_new_unit("cargo nextest run --package=bf new_t::");
    find(&mut ok.units, "U-4")["tests"] = json!("cargo nextest run --package=bf new_t::");
    passes(&run(&ok));
}

#[test]
fn judge_free_a_protected_path_is_not_moved_added_or_removed() {
    let guard = "crates/bf/src/guard.rs";
    let mut before_guard = base();
    find(&mut before_guard.units, "U-2")["files"] = json!([guard, "crates/bf/tests/bb.rs"]);
    // The plan in force holds guard.rs in U-2: build the facts by hand.
    let design = set(&["R-1", "R-2", "R-3", "R-4", "R-5"]);
    let none = BTreeSet::new();
    let facts = |after: &P| {
        refusals(&Facts {
            before: Slice { units: &before_guard.units, handoffs: &before_guard.handoffs, checkpoints: &before_guard.cps, checks: &before_guard.checks },
            after: Slice { units: &after.units, handoffs: &after.handoffs, checkpoints: &after.cps, checks: &after.checks },
            changes: &[],
            ops: 0,
            waive: &[],
            landed: &none,
            closed: &none,
            design: &design,
            new_problems: &[],
        })
    };
    let mut same = before_guard.clone();
    find(&mut same.units, "U-2")["text"] = json!("reworded");
    passes(&facts(&same));
    let mut given = before_guard.clone();
    find(&mut given.units, "U-2")["files"] = json!(["crates/bf/tests/bb.rs"]);
    find(&mut given.units, "U-1")["files"] = json!(["crates/bf/src/a.rs", "crates/bf/tests/aa.rs", guard]);
    let out = facts(&given);
    refused(&out, "U-1: the protected path crates/bf/src/guard.rs");
    refused(&out, "U-2: the protected path crates/bf/src/guard.rs");
    let mut gone = before_guard.clone();
    drop_id(&mut gone.units, "U-2");
    drop_id(&mut gone.checks, "X-2");
    gone.cps[0]["after"] = json!(["U-1"]);
    find(&mut gone.units, "U-1")["delivers"] = json!(["R-1", "R-4", "R-2"]);
    refused(&facts(&gone), "U-2: the unit holds the protected path");
}

#[test]
fn judge_free_a_test_file_of_a_check_in_force_stays_with_its_unit() {
    let mut p = base();
    find(&mut p.units, "U-2")["files"] = json!(["crates/bf/src/b.rs"]);
    find(&mut p.units, "U-1")["files"] = json!(["crates/bf/src/a.rs", "crates/bf/tests/aa.rs", "crates/bf/tests/bb.rs"]);
    let out = run(&p);
    refused(&out, "U-1: the test file crates/bf/tests/bb.rs");
    refused(&out, "U-2: the test file crates/bf/tests/bb.rs");
}

#[test]
fn judge_free_handoffs_may_change_from_and_to_only() {
    let mut p = base();
    find(&mut p.handoffs, "H-1")["from"] = json!("U-2");
    passes(&run(&p));
    find(&mut p.handoffs, "H-1")["thing"] = json!("other");
    refused(&run(&p), "H-1: a handoff changes its field thing");
    let mut q = base();
    q.handoffs.push(json!({"id": "H-2", "kind": "handoff", "from": "U-1", "to": "U-2", "thing": "t"}));
    refused(&run(&q), "H-2: a handoff is added");
    let mut r = base();
    r.handoffs.clear();
    refused(&run(&r), "H-1: a handoff is removed");
}

#[test]
fn judge_free_a_new_plan_problem_is_refused() {
    let out = run_with(&base(), &[], 0, &[], &[], &["U-9 has no check".to_string()]);
    refused(&out, "U-9 has no check");
}

#[test]
fn judge_free_a_unit_that_depends_on_a_later_checkpoint_is_refused_by_pair() {
    // U-1 of K-1 would depend on U-3 of K-2.
    let mut p = base();
    find(&mut p.units, "U-1")["depends_on"] = json!(["U-3"]);
    refused(&run(&p), "U-1 of K-1 depends on U-3 of K-2");
    // A unit moved earlier while it depends on a unit that stays later.
    let mut q = base();
    find(&mut q.units, "U-3")["checkpoint"] = json!("K-1");
    find(&mut q.units, "U-5")["depends_on"] = json!(["U-3"]);
    q.cps[0]["after"] = json!(["U-1", "U-2", "U-3"]);
    q.cps[1]["after"] = json!(["U-5"]);
    passes(&run(&q));
    let mut r = base();
    find(&mut r.units, "U-5")["depends_on"] = json!(["U-3"]);
    find(&mut r.units, "U-5")["checkpoint"] = json!("K-1");
    r.cps[0]["after"] = json!(["U-1", "U-2", "U-5"]);
    r.cps[1]["after"] = json!(["U-3"]);
    let out = run(&r);
    refused(&out, "U-5 of K-1 depends on U-3 of K-2");
}

#[test]
fn judge_free_the_pair_the_plan_in_force_already_had_is_not_refused() {
    let mut before = base();
    find(&mut before.units, "U-1")["depends_on"] = json!(["U-3"]);
    let design = set(&["R-1", "R-2", "R-3", "R-4", "R-5"]);
    let none = BTreeSet::new();
    let mut after = before.clone();
    find(&mut after.units, "U-1")["text"] = json!("reworded");
    let out = refusals(&Facts {
        before: Slice { units: &before.units, handoffs: &before.handoffs, checkpoints: &before.cps, checks: &before.checks },
        after: Slice { units: &after.units, handoffs: &after.handoffs, checkpoints: &after.cps, checks: &after.checks },
        changes: &[],
        ops: 0,
        waive: &[],
        landed: &none,
        closed: &none,
        design: &design,
        new_problems: &[],
    });
    passes(&out);
}

#[test]
fn judge_free_the_test_name_rule_is_the_one_load_reads() {
    use bf::plan::targets::test_name_of_path as name;
    assert_eq!(name("crates/bf/tests/foo.rs").as_deref(), Some("foo"));
    assert_eq!(name("crates/bf/tests/dir/mod.rs").as_deref(), Some("dir"));
    assert_eq!(name("crates/bf/tests/data.txt").as_deref(), Some("data.txt"));
    // A module folder named `tests` under `src/` is no test file.
    assert_eq!(name("crates/bf/src/tests/foo.rs"), None);
    assert_eq!(name("crates/bf/src/foo.rs"), None);
}
