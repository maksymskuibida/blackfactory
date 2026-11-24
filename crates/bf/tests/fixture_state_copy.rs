//! U-122 (D-140, F-3): a lifecycle state (a run with a unit started and its check run) is built once by
//! a build function and copied per test. After each copy `git worktree repair` points the working copies
//! at the copy, `bf init` writes the signed guard table again, and a canary checks the copy's paths and
//! its log. The build folder is removed, so any old path the engine still reads fails the test using it.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// The tiny project's run with U-1 started, its file written and its check X-1 run: U-1's working copy
/// is a git worktree, and the log's `unit.start` event names its absolute path.
fn u1_checked(fx: &Fx) {
    fx.tiny();
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
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
            "src/hello.txt",
        ],
    );
}

const NAME: &str = "fixture_state_copy::u1_checked";

/// A private store for the forms that must not share the shared store's keys.
fn private(dir: &Path) -> TemplateStore {
    TemplateStore::at(dir, &format!("{}|{}", binary_identity(), test_identity()))
}

fn log(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// Report U-1 built and reviewed, and land it. Gives the `unit.land` event.
fn land_u1(fx: &Fx) -> Value {
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    log(fx)
        .into_iter()
        .rev()
        .find(|e| e["kind"] == "unit.land")
        .expect("no unit.land event")
}

fn u1_state(fx: &Fx) -> String {
    bf::events::replay(&fx.project()).unwrap().units["U-1"]
        .state
        .clone()
}

#[test]
fn a_copied_state_passes_the_canary_and_its_build_folder_is_gone() {
    let (fx, build_dir) = Fx::state_copy_in(
        &TemplateStore::states(),
        NAME,
        Switch::Off,
        u1_checked,
        StateOpts::default(),
    );
    assert!(!build_dir.exists(), "the build folder is still there");
    assert!(fx.work("U-1").join(".git").is_file());
    assert_eq!(state_canary(&fx, &build_dir), Vec::<String>::new());
    // The unit's working copy is a working copy of this copy's repository.
    let wts = git(&fx.root, &["worktree", "list", "--porcelain"]);
    assert!(
        wts.contains(&format!("worktree {}", fx.work("U-1").display())),
        "{wts}"
    );
}

#[test]
fn a_planted_build_path_fails_the_canary() {
    let (fx, build_dir) = Fx::state_copy_in(
        &TemplateStore::states(),
        NAME,
        Switch::Off,
        u1_checked,
        StateOpts::default(),
    );
    let planted = fx.root.join("notes.txt");
    fs::write(&planted, format!("see {}/proj\n", build_dir.display())).unwrap();
    let found = state_canary(&fx, &build_dir);
    assert!(
        found
            .iter()
            .any(|f| f.starts_with("path: ") && f.contains(&planted.display().to_string())),
        "{found:?}"
    );
    // The named history files keep the path and are not findings.
    assert!(fs::read_to_string(fx.project().log_path())
        .unwrap()
        .contains(&build_dir.display().to_string()));
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn a_copy_without_the_repair_fails_the_worktree_part_of_the_canary() {
    let (fx, build_dir) = Fx::state_copy_in(
        &TemplateStore::states(),
        NAME,
        Switch::Off,
        u1_checked,
        StateOpts {
            repair: false,
            canary: false,
            keep_build: false,
        },
    );
    let found = state_canary(&fx, &build_dir);
    assert!(
        found.iter().any(|f| f.starts_with("worktree: ")),
        "{found:?}"
    );
    // Repaired, the same copy passes.
    fx.repair_worktrees();
    assert_eq!(state_canary(&fx, &build_dir), Vec::<String>::new());
}

#[test]
fn the_kept_form_keeps_its_build_folder_and_the_normal_form_does_not() {
    let d = TempDir::new().unwrap();
    let store = private(d.path());
    let (_kept, kept_dir) = Fx::state_copy_in(
        &store,
        NAME,
        Switch::Off,
        u1_checked,
        StateOpts {
            keep_build: true,
            ..StateOpts::default()
        },
    );
    let (_plain, plain_dir) =
        Fx::state_copy_in(&store, NAME, Switch::Off, u1_checked, StateOpts::default());
    assert!(kept_dir.is_dir(), "the kept build folder is gone");
    assert!(kept_dir.join("proj/.bf/work/U-1").is_dir());
    assert!(
        !plain_dir.exists(),
        "the normal build folder is still there"
    );
    assert_ne!(kept_dir, plain_dir);
    fs::remove_dir_all(&kept_dir).unwrap();
}

#[test]
fn the_first_event_of_a_copy_chains_to_the_copied_head_and_the_land_behaves_as_in_the_source() {
    // The source: the same state built through the full `bf` calls in this test.
    let src = Fx::new_with(Switch::Off);
    u1_checked(&src);
    let src_land = land_u1(&src);
    // The copy.
    let fx = Fx::state_copy_with(NAME, Switch::Off, u1_checked);
    let before = log(&fx);
    let last = before.last().unwrap().clone();
    let head: Value =
        serde_json::from_str(&fs::read_to_string(fx.project().head_path()).unwrap()).unwrap();
    assert_eq!(
        head["last_hash"], last["h"],
        "the copied head is the copied log's end"
    );
    let land = land_u1(&fx);
    let after = log(&fx);
    let first_new = &after[before.len()];
    assert_eq!(first_new["prev"], last["h"], "{first_new}");
    assert_eq!(
        first_new["seq"].as_u64().unwrap(),
        last["seq"].as_u64().unwrap() + 1
    );
    fx.must(&["log", "--verify"]);
    assert_eq!(u1_state(&fx), "landed");
    assert_eq!(u1_state(&src), "landed");
    assert_eq!(
        land["hash"], src_land["hash"],
        "the landed content is the same"
    );
    assert!(fx.work("K-1").join("src/hello.txt").is_file());
    assert!(!fx.work("U-1").exists());
}

#[test]
fn two_copies_of_one_state_land_the_same_unit_with_the_same_outcome() {
    let a = Fx::state_copy_with(NAME, Switch::Off, u1_checked);
    let b = Fx::state_copy_with(NAME, Switch::Off, u1_checked);
    assert_ne!(a.root, b.root);
    let la = land_u1(&a);
    let lb = land_u1(&b);
    assert_eq!(u1_state(&a), "landed");
    assert_eq!(u1_state(&b), "landed");
    assert_eq!(la["hash"], lb["hash"]);
    assert_eq!(
        fs::read_to_string(a.work("K-1").join("src/hello.txt")).unwrap(),
        fs::read_to_string(b.work("K-1").join("src/hello.txt")).unwrap()
    );
    a.must(&["log", "--verify"]);
    b.must(&["log", "--verify"]);
}

#[test]
fn a_check_on_a_copied_state_meets_the_sandbox_when_the_probe_succeeds() {
    let fx = Fx::state_copy_with(NAME, Switch::All, u1_checked);
    let want = if sandbox_probe() {
        "sandbox-exec"
    } else {
        eprintln!("sandbox_probe = false");
        "none"
    };
    assert_eq!(rerun_x1(&fx), want);
    // With the switch off, the same state's check runs without it.
    let off = Fx::state_copy_with(NAME, Switch::Off, u1_checked);
    assert_eq!(rerun_x1(&off), "none");
}

/// Run U-1's check again in the copy on changed content, and give its receipt's `sandbox` value.
fn rerun_x1(fx: &Fx) -> String {
    let wc = fx.work("U-1");
    fs::write(wc.join("src/hello.txt"), "hello again\n").unwrap();
    let n = bf::evidence::load_receipts(&fx.project()).unwrap().len();
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
            "src/hello.txt",
        ],
    );
    assert_eq!(
        bf::evidence::load_receipts(&fx.project()).unwrap().len(),
        n + 1
    );
    receipt_sandbox_of(fx, "X-1")
}

/// U-119 (1a), (4a): every state of the converted files that follows the environment's switch meets the
/// sandbox under `BF_TEST_SANDBOX_REQUIRE=1` on macOS; otherwise this asserts nothing.
#[test]
fn required_sandbox_is_met_for_state_copy() {
    let fx = Fx::state_copy(NAME, u1_checked);
    require_sandbox_from_env(&fx);
    crate::checkpoint_hold::require_sandbox_for_states();
    crate::checkpoint_decision_window::require_sandbox_for_states();
    crate::ckfix_reopen::require_sandbox_for_states();
    crate::checkpoint_findings_hold::require_sandbox_for_states();
    crate::repair_judge::require_sandbox_for_states();
    crate::wave_check::require_sandbox_for_states();
}

// U-123 (D-140): each state of the four converted files, copied, passes the canary and lands, fixes or
// reopens as the same state built through the full `bf` calls does. Each file holds its own comparison,
// since its build functions take its own `Fx`.

#[test]
fn the_ckfix_reopen_states_copied_fix_and_reopen_as_their_sources() {
    crate::ckfix_reopen::copies_behave_as_their_sources();
}

#[test]
fn the_checkpoint_findings_hold_states_copied_fix_and_reopen_as_their_sources() {
    crate::checkpoint_findings_hold::copies_behave_as_their_sources();
}

#[test]
fn the_repair_judge_state_copied_is_approved_by_a_go_as_its_source() {
    crate::repair_judge::copies_behave_as_their_sources();
}

#[test]
fn the_wave_check_states_copied_land_and_fix_as_their_sources() {
    crate::wave_check::copies_behave_as_their_sources();
}
