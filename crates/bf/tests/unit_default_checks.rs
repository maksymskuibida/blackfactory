//! The checks every unit must pass (R-76, D-53, TH-32): a list in the project's settings that the engine
//! attaches to every unit; a run, a checkpoint and a unit can override it, the most specific wins, and the
//! plan page shows every override with its reason.

mod common;
use common::*;
use std::fs;
use std::path::PathBuf;

const LINT: &str = "test -f src/a.txt";

fn plan_with(fx: &Fx, unit_extra: &str, checkpoint_extra: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n{unit_extra}")),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n{checkpoint_extra}")),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

/// The wave-check question of the run's plan stage, answered with no wave checks.
fn answer_wave_question(fx: &Fx) {
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
}

fn approve_plan(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
}

/// The unit is built and reviewed; its own check has a receipt. It could land.
fn built_and_reviewed(fx: &Fx) -> PathBuf {
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "content\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-1",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "src/a.txt",
        ],
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    wc
}

fn with_project_list(fx: &Fx) {
    fx.set_policy(
        "list = []",
        &format!("list = [{{ name = \"lint\", command = \"{LINT}\" }}]"),
    );
}

#[test]
fn the_list_is_empty_as_shipped_and_the_engine_asks_for_nothing_then() {
    let fx = Fx::shipped();
    assert!(fx.project().policy.unit_checks.list.is_empty());
    let text = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    assert!(
        text.contains("[unit_checks]") && text.contains("list = []"),
        "{text}"
    );
}

#[test]
fn the_engine_demands_a_receipt_of_each_attached_check_before_a_unit_lands() {
    let fx = Fx::new();
    with_project_list(&fx);
    plan_with(&fx, "", "");
    approve_plan(&fx);
    fx.must(&["run", "start"]);
    let wc = built_and_reviewed(&fx);
    // The brief of a builder and of a reviewer lists the check beside the unit's own, with its source.
    for role in ["builder", "reviewer"] {
        let b =
            fs::read_to_string(fx.must(&["brief", "U-1", "--role", role]).stdout.trim()).unwrap();
        assert!(
            b.contains("attached:lint")
                && b.contains(LINT)
                && b.contains("the project's list")
                && b.contains("X-1"),
            "{role}: {b}"
        );
    }
    // No plan repeated it, and the unit cannot land without it.
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(o.stderr.contains("attached:lint"), "{}", o.stderr);
    // A receipt for another command does not count: the engine runs exactly the command of the list.
    let o = fx.bf_in(
        &wc,
        &[
            "evidence",
            "run",
            "attached:lint",
            "--unit",
            "U-1",
            "--",
            "true",
        ],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "attached:lint",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "src/a.txt",
        ],
    );
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn a_failing_attached_check_holds_the_unit_back() {
    let fx = Fx::new();
    fx.set_policy(
        "list = []",
        "list = [{ name = \"strict\", command = \"test -f nothing.txt\" }]",
    );
    plan_with(&fx, "", "");
    approve_plan(&fx);
    fx.must(&["run", "start"]);
    let wc = built_and_reviewed(&fx);
    // The run fails and its receipt records the failure; a failure is never proof.
    fx.bf_in(
        &wc,
        &[
            "evidence",
            "run",
            "attached:strict",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "nothing.txt",
        ],
    );
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(o.stderr.contains("attached:strict"), "{}", o.stderr);
}

#[test]
fn a_unit_overrides_the_list_with_a_reason_and_the_plan_page_marks_it() {
    let fx = Fx::new();
    with_project_list(&fx);
    plan_with(
        &fx,
        "own_checks = []\nown_checks_reason = \"a documentation unit: nothing to lint\"\n",
        "",
    );
    // The page shows the override and its reason before the human approves the plan.
    let page = fx.must(&["view", "plan", "--json"]).stdout;
    assert!(
        page.contains("overrides the list")
            && page.contains("a documentation unit: nothing to lint"),
        "{page}"
    );
    approve_plan(&fx);
    fx.must(&["run", "start"]);
    built_and_reviewed(&fx);
    // The most specific list wins: this unit has none, so it lands on its own check alone.
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn an_override_needs_a_reason() {
    let fx = Fx::new();
    let o = fx.put(
        "unit",
        "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\nown_checks = []\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("own_checks_reason"), "{}", o.stderr);
    let o = fx.put(
        "checkpoint",
        "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"x\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nunit_checks = []\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("unit_checks_reason"), "{}", o.stderr);
}

#[test]
fn a_checkpoint_overrides_for_its_units_and_a_unit_overrides_the_checkpoint() {
    let fx = Fx::new();
    with_project_list(&fx);
    // The checkpoint names another command for its units, with a reason; the plan page marks it.
    plan_with(
        &fx,
        "",
        "unit_checks = [{ name = \"later\", command = \"test -f src/a.txt\" }]\nunit_checks_reason = \"a later checkpoint needs a command an earlier one builds\"\n",
    );
    let page = fx.must(&["view", "plan", "--json"]).stdout;
    assert!(
        page.contains("a later checkpoint needs a command an earlier one builds"),
        "{page}"
    );
    approve_plan(&fx);
    fx.must(&["run", "start"]);
    built_and_reviewed(&fx);
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(
        o.stderr.contains("attached:later") && !o.stderr.contains("attached:lint"),
        "{}",
        o.stderr
    );
}

#[test]
fn the_run_chooses_on_the_humans_words_and_after_the_plan_gate_only_an_amendment_changes_it() {
    let fx = Fx::new();
    with_project_list(&fx);
    plan_with(&fx, "", "");
    fx.must(&["run", "start"]);
    let q = fx.must(&["checks", "unit", "question", "--json"]).json();
    assert!(
        q["question"]["options"].to_string().contains("project"),
        "{q}"
    );
    assert_eq!(q["project_list"][0]["name"], "lint", "{q}");
    // No one has said anything: nothing is recorded.
    fx.must(&["checks", "unit", "propose", "none"]);
    fx.refused(&["checks", "unit", "set", "--quote", "none"]);
    fx.human("None of those checks for this run.");
    fx.must(&["checks", "unit", "set", "--quote", "None of those checks"]);
    let s = fx.must(&["checks", "unit", "state", "--json"]).json();
    assert_eq!(s["run_choice"]["mode"], "none", "{s}");
    answer_wave_question(&fx);
    approve_plan(&fx);
    let o = fx.refused(&["checks", "unit", "propose", "project"]);
    assert!(o.stderr.contains("amendment"), "{}", o.stderr);
    // The run chose none: the unit lands on its own check.
    built_and_reviewed(&fx);
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn a_runs_override_replaces_the_project_list() {
    let fx = Fx::new();
    with_project_list(&fx);
    plan_with(&fx, "", "");
    fx.must(&["run", "start"]);
    fx.must(&[
        "checks",
        "unit",
        "propose",
        "override",
        "fmt=test -f src/a.txt",
    ]);
    fx.human("Use fmt instead of the project's list.");
    fx.must(&["checks", "unit", "set", "--quote", "Use fmt instead"]);
    answer_wave_question(&fx);
    approve_plan(&fx);
    built_and_reviewed(&fx);
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(
        o.stderr.contains("attached:fmt") && !o.stderr.contains("attached:lint"),
        "{}",
        o.stderr
    );
    let page = fx.must(&["view", "plan", "--json"]).stdout;
    assert!(page.contains("the run's list"), "{page}");
}

#[test]
fn only_the_human_changes_the_projects_list() {
    let fx = Fx::new();
    fx.must(&["run", "start"]);
    // The command refuses a project-wide scope: the list is in the policy file, which the human edits.
    let o = fx.refused(&["checks", "unit", "propose", "none", "--scope", "default"]);
    assert!(o.stderr.contains("policy"), "{}", o.stderr);
}
