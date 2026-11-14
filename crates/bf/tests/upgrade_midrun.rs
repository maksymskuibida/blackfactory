//! U-29: a project that is in the middle of a run when the engine is upgraded has check settings that
//! are not decided yet. Every refusal names the way out: ask the human to switch the check off for the run.

mod common;
use common::Fx;

/// A run begun before the engine knew the checks: it is active and nothing was decided.
fn midrun() -> Fx {
    let fx = Fx::with_checks();
    fx.must(&["run", "start"]);
    fx.tiny();
    fx
}

fn names_the_way_out(text: &str) {
    assert!(
        text.contains("bf checks propose --scope run"),
        "no way out named:\n{text}"
    );
    assert!(text.contains("switch"), "no way out named:\n{text}");
}

#[test]
fn the_plan_check_treats_a_run_in_progress_as_undecided_and_names_the_way_out() {
    let fx = midrun();
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("no check settings are decided"),
        "{}",
        o.stderr
    );
    names_the_way_out(&o.stderr);
}

#[test]
fn stopping_the_run_names_the_way_out_and_the_human_choice_unblocks_it() {
    let fx = midrun();
    let o = fx.refused(&["run", "stop"]);
    assert!(
        o.stderr.contains("No check settings are decided"),
        "{}",
        o.stderr
    );
    names_the_way_out(&o.stderr);
    // The human switches every check off for the run, in chat, and the run can be stopped.
    fx.run_checks_all_off();
    fx.must(&["plan", "check"]);
    fx.must(&["run", "stop"]);
}

#[test]
fn every_refusal_for_a_missing_proof_names_the_way_out() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.must(&["run", "start"]);
    let o = fx.refused(&["run", "stop"]);
    names_the_way_out(&o.stderr);
    assert!(o.stderr.contains("`accessibility`"), "{}", o.stderr);
}

#[test]
fn closing_a_checkpoint_names_the_way_out_for_each_check_without_proof() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx.must_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "run",
            "X-3",
            "--",
            "ls",
            "src/hello.txt",
            "src/bye.txt",
        ],
    );
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    names_the_way_out(&o.stderr);
    assert!(
        o.stderr
            .contains("bf checks propose --scope run accessibility=off"),
        "{}",
        o.stderr
    );
    assert!(!o.stderr.contains("<check>"), "{}", o.stderr);
}
