//! The run's two plan questions (R-89, D-63): which wave checks the run uses and which checks every unit
//! must pass are asked whenever the active run has not answered one, before or after its plan gate, and no
//! unit of the run starts until both are answered.

mod common;
use common::*;
use std::fs;
use std::path::Path;

fn actions(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["action"].as_str().unwrap_or("").to_string())
        .collect()
}

/// The conductor's order: the plan check, the plan gate approved, then the run starts.
fn conductor_order(fx: &Fx) {
    fx.tiny();
    fx.human("yes, make this a test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, make this a test run with simulated approvals",
    ]);
    let o = fx.must(&["plan", "check"]);
    assert!(o.stdout.contains("plan ok"), "{}", o.all());
    fx.must(&["gate", "open", "plan"]);
    fx.must(&["gate", "approve", "plan", "--simulated"]);
    fx.must(&["run", "start"]);
}

/// The conductor's order with a plan that holds one wave check.
fn conductor_order_with_wave_check(fx: &Fx) {
    fx.tiny();
    let r = fx.put(
        "check",
        "id=\"X-9\"\nkind=\"check\"\ntitle=\"wave\"\nhow=\"command\"\ncommand=\"test -d src\"\nproves=[]\nscope=\"wave\"\n",
    );
    assert_eq!(r.code, 0, "{}", r.all());
    fx.commit_all("wave record");
    fx.human("yes, make this a test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, make this a test run with simulated approvals",
    ]);
    let o = fx.must(&["plan", "check"]);
    assert!(o.stdout.contains("plan ok"), "{}", o.all());
    fx.must(&["gate", "open", "plan"]);
    fx.must(&["gate", "approve", "plan", "--simulated"]);
    fx.must(&["run", "start"]);
}

#[test]
fn after_the_plan_gate_an_answer_that_keeps_the_plans_wave_checks_leaves_the_records_alone() {
    let fx = Fx::new();
    conductor_order_with_wave_check(&fx);
    // An answer that would change the approved records is refused at propose, pointing to an amendment.
    let o = fx.refused(&["checks", "wave", "propose", "none"]);
    assert!(o.stderr.contains("amend"), "{}", o.stderr);
    let o = fx.refused(&["checks", "wave", "propose", "override", "test -f other"]);
    assert!(o.stderr.contains("amend"), "{}", o.stderr);
    // The plan's own command is accepted and writes nothing.
    fx.must(&["checks", "wave", "propose", "override", "test -d src"]);
    fx.human("Use that wave check.");
    fx.must(&["checks", "wave", "set", "--quote", "Use that wave check"]);
    assert!(fx.root.join("factory/records/X-9.toml").exists());
    fx.must(&["checks", "unit", "propose", "none"]);
    fx.human("No checks for every unit in this run.");
    fx.must(&[
        "checks",
        "unit",
        "set",
        "--quote",
        "No checks for every unit",
    ]);
    // The next unit start is not refused as drift.
    fx.must(&["unit", "start", "U-1"]);
}

#[test]
fn a_wave_proposal_made_before_the_plan_gate_is_refused_at_set_when_it_changes_the_plans_checks() {
    let fx = Fx::new();
    fx.tiny();
    fx.human("yes, make this a test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, make this a test run with simulated approvals",
    ]);
    fx.must(&["plan", "check"]);
    fx.must(&["gate", "open", "plan"]);
    fx.must(&["run", "start"]);
    // Proposed while the plan gate has not passed: accepted, the plan holds no wave check.
    fx.must(&["checks", "wave", "propose", "override", "test -d src"]);
    fx.must(&["gate", "approve", "plan", "--simulated"]);
    fx.human("Use that wave check.");
    let o = fx.refused(&["checks", "wave", "set", "--quote", "Use that wave check"]);
    assert!(o.stderr.contains("amend open"), "{}", o.stderr);
    // Nothing was recorded: the run has no wave answer.
    let o = fx.must(&["checks", "wave", "state"]);
    assert!(!o.stdout.contains("test -d src"), "{}", o.all());
}

#[test]
fn next_asks_both_questions_after_the_plan_gate_and_offers_no_unit_start() {
    let fx = Fx::new();
    conductor_order(&fx);
    let a = actions(&fx);
    assert!(a.contains(&"answer-wave-checks".to_string()), "{a:?}");
    assert!(a.contains(&"answer-unit-checks".to_string()), "{a:?}");
    assert!(!a.contains(&"start-unit".to_string()), "{a:?}");
}

#[test]
fn a_unit_start_is_refused_naming_the_question_and_its_command() {
    let fx = Fx::new();
    conductor_order(&fx);
    let o = fx.refused(&["unit", "start", "U-1"]);
    assert!(
        o.stderr.contains("which wave checks this run uses"),
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("bf checks wave question"), "{}", o.stderr);
    // One answered, the other still holds the start back.
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    let o = fx.refused(&["unit", "start", "U-1"]);
    assert!(o.stderr.contains("bf checks unit question"), "{}", o.stderr);
}

#[test]
fn both_answers_are_accepted_after_the_plan_gate_and_a_second_change_is_frozen() {
    let fx = Fx::new();
    conductor_order(&fx);
    fx.answer_plan_questions();
    let a = actions(&fx);
    assert!(a.contains(&"start-unit".to_string()), "{a:?}");
    assert!(!a.contains(&"answer-wave-checks".to_string()), "{a:?}");
    assert!(!a.contains(&"answer-unit-checks".to_string()), "{a:?}");
    fx.must(&["unit", "start", "U-1"]);
    // A change of an answer the run already has stays refused after the plan gate.
    let o = fx.refused(&["checks", "wave", "propose", "none"]);
    assert!(o.stderr.contains("frozen"), "{}", o.stderr);
    let o = fx.refused(&["checks", "unit", "propose", "none"]);
    assert!(o.stderr.contains("frozen"), "{}", o.stderr);
}

#[test]
fn before_the_plan_gate_an_unanswered_question_is_still_a_plan_problem() {
    let fx = Fx::new();
    fx.tiny();
    fx.human("yes, make this a test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, make this a test run with simulated approvals",
    ]);
    fx.must(&["run", "start"]);
    let o = fx.bf(&["plan", "check"]);
    assert!(
        o.all().contains("bf checks wave question") && o.all().contains("bf checks unit question"),
        "{}",
        o.all()
    );
}

/// Every markdown file of the factory skill, joined.
fn skill_all() -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness/claude/skills/factory");
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    names.sort();
    names
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_skill_names_the_questions_and_the_directive_actions_and_help_lists_the_commands() {
    let skill = skill_all();
    for n in [
        "answer-wave-checks",
        "answer-unit-checks",
        "review-checkpoint",
        "`wave-check` (engine",
        "decide-checkpoint-review",
        "decide-low-findings",
        "decide-checkpoint-fix",
        "decide-wave-check",
        "bf checks wave|unit question",
        "its reviewer and its fix builder",
    ] {
        assert!(skill.contains(n), "the skill should contain `{n}`");
    }
    let fx = Fx::new();
    let help = fx.bf(&["help"]).stdout;
    for n in ["checkpoint finding|review|fix|sync", "unit scope"] {
        assert!(help.contains(n), "bf help should list `{n}`:\n{help}");
    }
}
