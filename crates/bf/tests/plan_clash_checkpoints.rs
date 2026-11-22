//! U-95: two units of one checkpoint that no `depends_on` path orders share no files, whatever their
//! wave numbers; units of different checkpoints are never compared (FU-40, D-112).

mod common;
use common::*;

fn unit(id: &str, file: &str, deps: &[&str], cp: &str) -> String {
    let deps: Vec<String> = deps.iter().map(|d| format!("\"{d}\"")).collect();
    format!(
        "id=\"{id}\"\nkind=\"unit\"\ntitle=\"extra\"\ndelivers=[\"R-1\"]\nfiles=[\"{file}\"]\ndepends_on=[{}]\ncheckpoint=\"{cp}\"\nestimate=1\n",
        deps.join(",")
    )
}

fn clash(fx: &Fx, a: &str, b: &str) -> bool {
    let o = fx.bf(&["plan", "check"]);
    let all = o.all();
    all.contains("can run together") && all.contains(a) && all.contains(b) && all.contains("both touch")
}

fn any_clash(fx: &Fx) -> bool {
    fx.bf(&["plan", "check"]).all().contains("both touch")
}

#[test]
fn one_checkpoint_different_waves_no_path_is_a_problem() {
    let fx = Fx::new();
    fx.tiny();
    // U-2 is wave 2 (bye.txt); U-5 is wave 1 and shares bye.txt, with no path to U-2.
    fx.put("unit", &unit("U-5", "src/bye.txt", &[], "K-1"));
    assert!(clash(&fx, "U-2", "U-5"), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn a_direct_path_orders_the_pair() {
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-5", "src/bye.txt", &["U-2"], "K-1"));
    assert!(!any_clash(&fx), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn a_path_through_a_third_unit_orders_the_pair() {
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-3", "src/mid.txt", &["U-2"], "K-1"));
    fx.put("unit", &unit("U-5", "src/bye.txt", &["U-3"], "K-1"));
    assert!(!any_clash(&fx), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn units_of_different_checkpoints_are_not_compared() {
    let fx = Fx::new();
    fx.tiny();
    // Same wave as U-1, same file, other checkpoint.
    fx.put("unit", &unit("U-5", "src/hello.txt", &[], "K-2"));
    fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"Five\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-5\"\n");
    fx.put("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Later\"\nafter=[\"U-5\"]\nproves=[\"R-1\"]\nchecks=[\"X-9\"]\n");
    // The plan is valid otherwise, so the assertion isolates the clash rule.
    let o = fx.bf(&["plan", "check"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn one_checkpoint_one_wave_is_still_a_problem() {
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-5", "src/hello.txt", &[], "K-1"));
    assert!(clash(&fx, "U-1", "U-5"), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn the_waves_field_is_unchanged() {
    let fx = Fx::new();
    fx.tiny();
    let j = fx.bf(&["plan", "check", "--json"]).json();
    assert_eq!(j["waves"], serde_json::json!({"1": ["U-1"], "2": ["U-2"]}), "{j}");
}

// ---- landed units, the run's own shapes, and the approval comparison ----

use serde_json::json;
use std::fs;

const U1: &str = "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/hello.txt\"]\ncheckpoint=\"K-1\"\nestimate=2\nfull_suite=\"fixture\"\n";

fn u2(files: &str, deps: &str) -> String {
    format!("id=\"U-2\"\nkind=\"unit\"\ntitle=\"Farewell\"\ndelivers=[\"R-2\"]\nfiles={files}\ndepends_on={deps}\ncheckpoint=\"K-1\"\nestimate=1\nfull_suite=\"fixture\"\n")
}

fn replace(fx: &Fx, kind: &str, text: &str) {
    let f = fx.root.join("replace.toml");
    fs::write(&f, text).unwrap();
    fx.must(&["record", "put", kind, "--file", f.to_str().unwrap(), "--replace"]);
}

/// A run in which U-1 has landed and U-2 (which waits for it) has not.
fn run_with_u1_landed() -> Fx {
    let fx = Fx::new();
    fx.tiny();
    fx.checks_all_off();
    fx.start_test_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    fx
}

/// Write a unit record straight to the records folder, as a run's older plan holds it.
fn write_unit(fx: &Fx, text: &str, id: &str) {
    fs::write(fx.root.join(format!("factory/records/{id}.toml")), text).unwrap();
}

#[test]
fn a_pair_with_a_landed_unit_is_left_out_and_one_with_none_landed_is_not() {
    let fx = run_with_u1_landed();
    // U-5 (not built) shares hello.txt with the landed U-1 and bye.txt with the unbuilt U-2.
    write_unit(
        &fx,
        &unit("U-5", "src/hello.txt", &[], "K-1").replace("estimate=1", "estimate=1\nfull_suite=\"x\""),
        "U-5",
    );
    assert!(!any_clash(&fx), "{}", fx.bf(&["plan", "check"]).all());
    write_unit(
        &fx,
        &unit("U-5", "src/bye.txt", &[], "K-1").replace("estimate=1", "estimate=1\nfull_suite=\"x\""),
        "U-5",
    );
    assert!(clash(&fx, "U-2", "U-5"), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn a_fixture_with_no_run_compares_every_pair() {
    let fx = Fx::new();
    fx.tiny();
    // No run has started: U-1 counts as not landed.
    fx.put("unit", &unit("U-5", "src/hello.txt", &[], "K-1"));
    assert!(clash(&fx, "U-1", "U-5"), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn the_shapes_of_the_runs_own_pairs_pass() {
    // A unit not built that shares a file with a landed unit of its checkpoint (U-100 and U-67), and
    // two landed units of a closed checkpoint that share a file with no path (U-55 and U-81).
    let fx = run_with_u1_landed();
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-3", "--", "ls", "src/hello.txt", "src/bye.txt"],
    );
    fx.merge_checkpoint("K-1");
    write_unit(&fx, &u2("[\"src/hello.txt\",\"src/bye.txt\"]", "[]"), "U-2");
    write_unit(
        &fx,
        &unit("U-5", "src/hello.txt", &[], "K-1").replace("estimate=1", "estimate=1\nfull_suite=\"x\""),
        "U-5",
    );
    assert!(!any_clash(&fx), "{}", fx.bf(&["plan", "check"]).all());
}

/// Open an amendment for `ids`, make the edits, keep the whole impact and try to approve.
fn amend_and_approve(fx: &Fx, ids: &[&str], edit: impl Fn(&Fx)) -> common::Out {
    let a = [&["amend", "open", "--repair", "--json"][..], ids].concat();
    let keep: Vec<String> =
        serde_json::from_value(fx.must(&a).json()["impact_ids"].clone()).unwrap();
    edit(fx);
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, approve the change.");
    fx.bf(&["amend", "approve", "A-1", "--quote", "approve the change"])
}

fn u3(file: &str) -> String {
    unit("U-3", file, &[], "K-1") + "full_suite=\"fixture\"\n"
}

const K1: &str = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\",\"U-3\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\n";

/// K-1 holds U-1 (landed), U-2 (waits for U-1) and U-3 (unbuilt, a file of its own, no path to U-1).
fn run_with_three() -> Fx {
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-3", "src/three.txt", &[], "K-1"));
    fx.put("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"Three\"\nhow=\"command\"\ncommand=\"test -f src/three.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-3\"\n");
    replace(&fx, "checkpoint", K1);
    fx.checks_all_off();
    fx.start_test_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    fx
}

#[test]
fn a_landed_unit_sent_back_to_work_that_clashes_is_refused_when_its_record_changes() {
    let fx = run_with_three();
    let o = amend_and_approve(&fx, &["U-1", "U-3"], |fx| {
        replace(fx, "unit", &U1.replace("Greeting", "Greeting again"));
        replace(fx, "unit", &u3("src/hello.txt"));
    });
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(clash_text(&o.all(), "U-1", "U-3"), "{}", o.all());
}

/// A design amendment of R-1 (which U-1 delivers) with U-3 changed to take `file`: U-1's own record
/// does not change, the amendment keeps its whole impact and the human approves it.
fn design_amend_and_approve(fx: &Fx, file: &str) -> common::Out {
    let o = fx
        .must(&["amend", "open", "R-1", "U-3", "C-9", "D-91", "D-92", "--json"])
        .json();
    let keep: Vec<String> = serde_json::from_value(o["impact_ids"].clone()).unwrap();
    fx.put("component", "id=\"C-9\"\nkind=\"component\"\ntitle=\"Greeting wording\"\nlinks=[\"R-1\"]\n");
    for (id, stage) in [("D-91", "ui"), ("D-92", "plan")] {
        fx.put(
            "decision",
            &format!("id=\"{id}\"\nkind=\"decision\"\ntitle=\"The {stage} stage is not affected\"\nmaker=\"agent\"\nstage=\"{stage}\"\n"),
        );
    }
    replace(fx, "requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists, said again\"\nacceptance=[\"src/hello.txt exists\"]\n");
    replace(fx, "unit", &u3(file));
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, approve the change to the greeting.");
    fx.bf(&["amend", "approve", "A-1", "--quote", "approve the change"])
}

#[test]
fn a_landed_unit_sent_back_to_work_that_clashes_is_refused_when_only_a_record_it_delivers_changes() {
    let fx = run_with_three();
    let o = design_amend_and_approve(&fx, "src/hello.txt");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(clash_text(&o.all(), "U-1", "U-3"), "{}", o.all());
}

#[test]
fn the_same_amendments_without_a_shared_file_are_approved() {
    let fx = run_with_three();
    let o = amend_and_approve(&fx, &["U-1", "U-3"], |fx| {
        replace(fx, "unit", &U1.replace("Greeting", "Greeting again"));
        replace(fx, "unit", &u3("src/other.txt"));
    });
    assert_eq!(o.code, 0, "{}", o.all());
    let fx = run_with_three();
    let o = design_amend_and_approve(&fx, "src/other.txt");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn clash_text(all: &str, a: &str, b: &str) -> bool {
    all.contains("can run together") && all.contains(a) && all.contains(b)
}

#[test]
fn a_checkpoint_operation_beside_a_landed_unit_that_clashes_is_approved() {
    // K-2 holds U-4 and U-5, not started. U-3 of K-1 now shares a file with the landed U-1.
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-3", "src/three.txt", &[], "K-1"));
    fx.put("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"Three\"\nhow=\"command\"\ncommand=\"test -f src/three.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-3\"\n");
    replace(&fx, "checkpoint", K1);
    for n in [3, 4] {
        fx.put("requirement", &format!("id=\"R-{n}\"\nkind=\"requirement\"\ntitle=\"Part {n}\"\nacceptance=[\"src/{n}.txt exists\"]\n"));
    }
    for (n, r, d) in [(5, 3, "U-4"), (6, 4, "U-5")] {
        let u = unit(d, &format!("src/{r}.txt"), &["U-1"], "K-2").replace("\"R-1\"", &format!("\"R-{r}\""));
        fx.put("unit", &u);
        fx.put("check", &format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"Part {r}\"\nhow=\"command\"\ncommand=\"test -f src/{r}.txt\"\nproves=[\"R-{r}\"]\nscope=\"unit\"\nunit=\"{d}\"\n"));
    }
    fx.put("check", "id=\"X-7\"\nkind=\"check\"\ntitle=\"Both\"\nhow=\"command\"\ncommand=\"ls src/3.txt src/4.txt\"\nproves=[\"R-3\",\"R-4\"]\nscope=\"checkpoint\"\n");
    fx.put("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Later\"\nafter=[\"U-4\",\"U-5\"]\nproves=[\"R-3\",\"R-4\"]\nchecks=[\"X-7\"]\n");
    fx.start_test_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    write_unit(&fx, &u3("src/hello.txt"), "U-3");
    let o = fx.must(&["amend", "open", "--repair", "--json", "K-2"]);
    let keep: Vec<String> = serde_json::from_value(o.json()["impact_ids"].clone()).unwrap();
    fx.must(&["amend", "checkpoint", "split", "K-2", "U-4"]);
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, approve the change.");
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert_eq!(o.code, 0, "{}", o.all());
}
