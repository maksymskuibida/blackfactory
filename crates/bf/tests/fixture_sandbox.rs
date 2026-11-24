//! U-119 (D-140, D-144): the test fixture runs the engine with the sandbox off by default, on for the
//! sandboxed constructors and `Fx::shipped()`, and on everywhere with `BF_TEST_SANDBOX=all`. Each test
//! reads the probe first, so it never asserts a sandbox this process cannot have (a nested
//! `sandbox-exec` inside an engine-run check may not start).

mod common;
use common::*;

/// The receipt's `sandbox` value expected when the fixture asks for the sandbox: `sandbox-exec` where
/// the probe succeeds, `none` where it fails, as the engine reports an unavailable sandbox.
fn expected_on() -> &'static str {
    if sandbox_probe() {
        "sandbox-exec"
    } else {
        eprintln!("sandbox_probe = false");
        "none"
    }
}

#[test]
fn a_default_fixture_with_the_switch_off_runs_checks_without_the_sandbox() {
    let _ = sandbox_probe();
    let fx = Fx::new_with(Switch::Off);
    assert_eq!(receipt_sandbox(&fx), "none");
}

#[test]
fn a_fixture_with_checks_and_the_switch_off_runs_checks_without_the_sandbox() {
    let _ = sandbox_probe();
    let fx = Fx::with_checks_with(Switch::Off);
    assert_eq!(receipt_sandbox(&fx), "none");
}

#[test]
fn a_workspace_with_the_switch_off_runs_checks_without_the_sandbox() {
    let _ = sandbox_probe();
    let fx = Fx::workspace_with(&["app"], Switch::Off);
    assert_eq!(receipt_sandbox(&fx), "none");
}

#[test]
fn a_sandboxed_fixture_runs_checks_under_the_sandbox() {
    let want = expected_on();
    let fx = Fx::sandboxed();
    assert_eq!(receipt_sandbox(&fx), want);
}

#[test]
fn a_sandboxed_fixture_with_checks_runs_checks_under_the_sandbox() {
    let want = expected_on();
    let fx = Fx::with_checks_sandboxed();
    assert_eq!(receipt_sandbox(&fx), want);
}

#[test]
fn a_sandboxed_workspace_runs_checks_under_the_sandbox() {
    let want = expected_on();
    let fx = Fx::workspace_sandboxed(&["app"]);
    assert_eq!(receipt_sandbox(&fx), want);
}

#[test]
fn the_shipped_fixture_runs_checks_under_the_sandbox() {
    let want = expected_on();
    let fx = Fx::shipped();
    assert_eq!(receipt_sandbox(&fx), want);
}

#[test]
fn a_default_fixture_with_the_switch_all_runs_checks_under_the_sandbox() {
    let want = expected_on();
    let fx = Fx::new_with(Switch::All);
    assert_eq!(receipt_sandbox(&fx), want);
}

#[test]
fn a_fixture_with_checks_and_the_switch_all_runs_checks_under_the_sandbox() {
    let want = expected_on();
    let fx = Fx::with_checks_with(Switch::All);
    assert_eq!(receipt_sandbox(&fx), want);
}

#[test]
fn a_workspace_with_the_switch_all_runs_checks_under_the_sandbox() {
    let want = expected_on();
    let fx = Fx::workspace_with(&["app"], Switch::All);
    assert_eq!(receipt_sandbox(&fx), want);
}

/// With `BF_TEST_SANDBOX_REQUIRE=1` on macOS, every fixture kind built from the environment's switch
/// must meet the sandbox; without it, this asserts nothing.
#[test]
fn required_sandbox_is_met() {
    require_sandbox_from_env(&Fx::new());
    require_sandbox_from_env(&Fx::with_checks());
    require_sandbox_from_env(&Fx::workspace(&["app"]));
}
