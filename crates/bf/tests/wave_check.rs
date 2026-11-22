//! The wave checks (R-60, R-61, R-62, C-20, T-5): after each wave has landed the engine offers a step that
//! runs the project's wave checks on a copy of the content, beside the next wave's units; a failure opens a
//! fix held to the units that landed since the last pass, and no unit lands until the check has passed.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const GOOD: &str = "grep -q good src/a.txt";

/// Two waves in one checkpoint: U-1 first, U-2 after it. A wave check and a checkpoint check run the same
/// command.
fn plan(fx: &Fx, wave_check: bool) {
    let mut recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"first\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"second\"\ndelivers=[\"R-1\"]\nfiles=[\"src/b.txt\"]\ndepends_on=[\"U-1\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c1\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"c3\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n".to_string()),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{GOOD}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\", \"U-2\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ];
    if wave_check {
        recs.push(("check", format!("id=\"X-9\"\nkind=\"check\"\ntitle=\"wave\"\nhow=\"command\"\ncommand=\"{GOOD}\"\nproves=[]\nscope=\"wave\"\n")));
    }
    for (k, t) in recs {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

/// The run starts and its two questions are answered before the plan gate: the run keeps the wave checks
/// the plan holds (a change of the plan's wave checks after the gate would be a change of approved
/// records), and no checks for every unit.
fn start(fx: &Fx) {
    fx.must(&["run", "start"]);
    let held = fx.must(&["checks", "wave", "state", "--json"]).json()["plan_checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    match held.first() {
        Some(c) => {
            let cmd = c["command"].as_str().unwrap().to_string();
            fx.must(&["checks", "wave", "propose", "override", &cmd]);
            fx.human("Use that wave check.");
            fx.must(&["checks", "wave", "set", "--quote", "Use that wave check"]);
        }
        None => {
            fx.must(&["checks", "wave", "propose", "none"]);
            fx.human("No wave checks for this run.");
            fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
        }
    }
    fx.must(&["checks", "unit", "propose", "none"]);
    fx.human("No checks for every unit in this run.");
    fx.must(&[
        "checks",
        "unit",
        "set",
        "--quote",
        "No checks for every unit",
    ]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
}

/// Build U-1 with the given content of src/a.txt, review it and land it.
fn land_first(fx: &Fx, content: &str) {
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), content).unwrap();
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
    fx.must(&["unit", "land", "U-1"]);
}

/// Build and review U-2, up to the point where it could land.
fn review_second(fx: &Fx) {
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-2", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/b.txt"), "second\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-3",
            "--unit",
            "U-2",
            "--",
            "test",
            "-f",
            "src/b.txt",
        ],
    );
    fx.must(&["report", "U-2", "--result", "built"]);
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
}

fn directives(fx: &Fx) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .clone()
}

fn find<'a>(ds: &'a [Value], action: &str) -> Option<&'a Value> {
    ds.iter().find(|d| d["action"] == action)
}

fn wave_state(fx: &Fx, k: &str, w: u64) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["wave_checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["checkpoint"] == k && x["wave"] == w)
        .map(|x| x["state"].as_str().unwrap().to_string())
        .unwrap_or_default()
}

fn commit(dir: &std::path::Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

#[test]
fn a_landed_wave_offers_the_step_and_the_next_waves_units_at_the_same_time() {
    let fx = Fx::new();
    plan(&fx, true);
    start(&fx);
    // Nothing has landed: no step yet.
    assert!(find(&directives(&fx), "wave-check").is_none());
    land_first(&fx, "good\n");
    let ds = directives(&fx);
    let step = find(&ds, "wave-check").unwrap_or_else(|| panic!("no wave check step: {ds:?}"));
    assert_eq!(step["role"], "engine", "{step}");
    assert_eq!(step["checkpoint"], "K-1", "{step}");
    assert_eq!(step["wave"], 1, "{step}");
    assert!(
        step.get("token").is_none(),
        "the engine's step needs no worker's token: {step}"
    );
    assert_eq!(
        step["command"], "bf checkpoint wave-check K-1 --wave 1",
        "{step}"
    );
    // The units of the next wave are offered at the same time.
    let next = find(&ds, "start-unit").unwrap_or_else(|| panic!("U-2 is not offered: {ds:?}"));
    assert_eq!(next["unit"], "U-2", "{next}");
}

#[test]
fn the_step_runs_on_a_copy_and_records_a_receipt_bound_to_that_content() {
    let fx = Fx::new();
    plan(&fx, true);
    start(&fx);
    land_first(&fx, "good\n");
    let content = fx.project().content_hash(&fx.work("K-1")).unwrap();
    let r = fx
        .must(&["checkpoint", "wave-check", "K-1", "--wave", "1", "--json"])
        .json();
    assert_eq!(r["state"], "passed", "{r}");
    assert_eq!(r["content"], content.as_str(), "{r}");
    // The copy is thrown away: nothing stays where later landings could change it.
    assert!(!fx.work("K-1-wave-1").exists());
    // The receipt: the check, its command, its result and the content it ran on.
    let rc = bf::evidence::load_receipts(&fx.project()).unwrap();
    // The run's answer wrote its own record for the wave check.
    let wave_id = fx.must(&["checks", "wave", "state", "--json"]).json()["plan_checks"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let wave = rc
        .iter()
        .find(|r| r.check() == wave_id)
        .expect("a wave receipt");
    assert!(wave.passes() && wave.content() == content);
    assert_eq!(wave.v["command"], GOOD);
    // The same command on the same content counts for the checkpoint's check.
    let k = rc
        .iter()
        .find(|r| r.check() == "X-2")
        .expect("it counts for X-2");
    assert!(k.passes() && k.content() == content);
    assert_eq!(wave_state(&fx, "K-1", 1), "passed");
    // A passed wave is not offered again.
    assert!(find(&directives(&fx), "wave-check").is_none());
    // Nor can it be run again by hand.
    fx.refused(&["checkpoint", "wave-check", "K-1", "--wave", "1"]);
}

#[test]
fn no_unit_lands_until_the_wave_check_before_it_has_passed() {
    let fx = Fx::new();
    plan(&fx, true);
    start(&fx);
    land_first(&fx, "good\n");
    // U-2 is built and reviewed without waiting for the check.
    review_second(&fx);
    let o = fx.refused(&["unit", "land", "U-2"]);
    assert!(o.stderr.contains("wave check of wave 1"), "{}", o.stderr);
    fx.must(&["checkpoint", "wave-check", "K-1", "--wave", "1"]);
    fx.must(&["unit", "land", "U-2"]);
    // The last wave has landed: its check is offered, and the checkpoint waits for it, because it counts
    // for the checkpoint's own check at the content the checkpoint closes on.
    let ds = directives(&fx);
    assert!(find(&ds, "wave-check").is_some(), "{ds:?}");
    assert!(find(&ds, "run-checkpoint-checks").is_none(), "{ds:?}");
    assert!(find(&ds, "close-checkpoint").is_none(), "{ds:?}");
    fx.refused(&["checkpoint", "close", "K-1"]);
    fx.must(&["checkpoint", "wave-check", "K-1", "--wave", "2"]);
    let ds = directives(&fx);
    // The run counts for X-2: the verifier is not asked to run it again.
    assert!(find(&ds, "run-checkpoint-checks").is_none(), "{ds:?}");
    assert!(find(&ds, "close-checkpoint").is_some(), "{ds:?}");
}

#[test]
fn a_failed_wave_check_opens_a_fix_held_to_the_units_landed_since_the_last_pass() {
    let fx = Fx::new();
    plan(&fx, true);
    start(&fx);
    land_first(&fx, "bad\n");
    let r = fx
        .must(&["checkpoint", "wave-check", "K-1", "--wave", "1", "--json"])
        .json();
    assert_eq!(r["state"], "failed", "{r}");
    // A fix for a builder, not a question to the human; the next wave's units are offered beside it.
    let ds = directives(&fx);
    let fix = find(&ds, "fix-wave").unwrap_or_else(|| panic!("no wave fix: {ds:?}"));
    assert_eq!(fix["role"], "builder", "{fix}");
    assert_eq!(fix["wave"], 1, "{fix}");
    let may = fix["may_change"].to_string();
    assert!(
        may.contains("src/a.txt") && !may.contains("src/b.txt"),
        "{may}"
    );
    assert!(
        find(&ds, "start-unit").is_some(),
        "U-2 is still offered: {ds:?}"
    );
    assert!(ds.iter().all(|d| d["role"] != "human"), "{ds:?}");
    // The brief holds the failing command and its output.
    let b = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        b.contains(GOOD) && b.contains("wave check of wave 1"),
        "{b}"
    );
    // The progress line shows the state.
    let line = fx.must(&["progress"]).stdout;
    assert!(line.contains("K-1 wave 1 wave check being fixed"), "{line}");
    // No unit lands while it is being fixed.
    review_second(&fx);
    let o = fx.refused(&["unit", "land", "U-2"]);
    assert!(
        o.stderr.contains("being fixed") || o.stderr.contains("failed"),
        "{}",
        o.stderr
    );
    // The builder fixes the unit's file, in the checkpoint's own working copy.
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "good\n").unwrap();
    commit(&k, "fix");
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let line = fx.must(&["progress"]).stdout;
    assert!(
        line.contains("K-1 wave 1 wave check fix in review"),
        "{line}"
    );
    // Reviewed clean: the wave check runs again and passes; the fix is attributed to the unit.
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(wave_state(&fx, "K-1", 1), "passed");
    let st = fx.must(&["status", "--json"]).json();
    let done = &st["checkpoint_fix"]["done"][0];
    assert_eq!(done["source"], "wave", "{st}");
    assert_eq!(done["units"][0], "U-1", "{st}");
    fx.must(&["unit", "land", "U-2"]);
}

#[test]
fn no_land_is_offered_while_a_wave_fix_of_its_checkpoint_is_open() {
    let fx = Fx::new();
    plan(&fx, true);
    start(&fx);
    land_first(&fx, "bad\n");
    fx.must(&["checkpoint", "wave-check", "K-1", "--wave", "1", "--json"]);
    review_second(&fx);
    // U-2 is reviewed, and its checkpoint's wave fix is open: no land is offered, and the unit waits.
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert!(
        ds.iter().all(|d| d["action"] != "land"),
        "a land directive while a wave fix is open: {n}"
    );
    let waiting = n["waiting"].to_string();
    assert!(waiting.contains("U-2"), "{n}");
    assert!(!waiting.is_empty() && waiting.contains("K-1"), "{n}");
}

#[test]
fn a_wave_fix_that_changes_a_file_outside_its_set_reports_built() {
    let fx = Fx::new();
    plan(&fx, true);
    start(&fx);
    land_first(&fx, "bad\n");
    fx.must(&["checkpoint", "wave-check", "K-1", "--wave", "1"]);
    fx.must(&["next", "--json"]);
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "good\n").unwrap();
    fs::write(k.join("elsewhere.txt"), "outside\n").unwrap();
    commit(&k, "fix with a file nobody listed");
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let ds = directives(&fx);
    assert!(find(&ds, "plan-gap").is_none(), "{ds:?}");
    let review = find(&ds, "review-wave-fix").unwrap_or_else(|| panic!("{ds:?}"));
    assert_eq!(review["role"], "reviewer", "{review}");
}

#[test]
fn a_plan_with_no_wave_check_has_no_step_and_no_hold() {
    let fx = Fx::new();
    plan(&fx, false);
    start(&fx);
    land_first(&fx, "good\n");
    review_second(&fx);
    assert!(find(&directives(&fx), "wave-check").is_none());
    fx.must(&["unit", "land", "U-2"]);
    fx.refused(&["checkpoint", "wave-check", "K-1", "--wave", "1"]);
}

#[test]
fn the_wave_check_step_needs_no_token_and_a_worker_cannot_run_it() {
    let fx = Fx::new();
    plan(&fx, true);
    start(&fx);
    land_first(&fx, "good\n");
    // From inside a working copy the engine refuses: planning and landing belong to the conductor.
    // The landing removed the unit's folder; the call still comes from where it stood.
    let wc = fx.work("U-1");
    std::fs::create_dir_all(&wc).unwrap();
    let o = fx.bf_in(&wc, &["checkpoint", "wave-check", "K-1", "--wave", "1"]);
    assert_eq!(o.code, 2, "{}", o.all());
}

/// Run the step in the background with a command that takes a while, and wait until it has started.
fn start_slow_step(fx: &Fx) -> std::process::Child {
    let mut child = Command::new(bin())
        .env("BF_STATE_HOME", &fx.state_base)
        .args(["checkpoint", "wave-check", "K-1", "--wave", "1"])
        .current_dir(&fx.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if wave_state(fx, "K-1", 1) == "running" {
            return child;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("the wave check did not start");
}

fn slow_plan(fx: &Fx) {
    // The wave check takes a few seconds; the checkpoint check stays as it is.
    plan(fx, false);
    let r = fx.put(
        "check",
        "id=\"X-9\"\nkind=\"check\"\ntitle=\"slow wave\"\nhow=\"command\"\ncommand=\"sleep 3\"\nproves=[]\nscope=\"wave\"\n",
    );
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    fx.commit_all("slow wave check");
}

#[test]
fn the_next_waves_units_are_offered_while_a_wave_check_still_runs() {
    let fx = Fx::new();
    slow_plan(&fx);
    start(&fx);
    land_first(&fx, "good\n");
    let mut child = start_slow_step(&fx);
    let ds = directives(&fx);
    let next = find(&ds, "start-unit").unwrap_or_else(|| panic!("U-2 is not offered: {ds:?}"));
    assert_eq!(next["unit"], "U-2");
    // The running step is not offered a second time.
    assert!(find(&ds, "wave-check").is_none(), "{ds:?}");
    let line = fx.must(&["progress"]).stdout;
    assert!(line.contains("K-1 wave 1 wave check running"), "{line}");
    // The unit is built without waiting; only its landing waits.
    review_second(&fx);
    let o = fx.refused(&["unit", "land", "U-2"]);
    assert!(o.stderr.contains("still running"), "{}", o.stderr);
    assert!(child.wait().unwrap().success());
    assert_eq!(wave_state(&fx, "K-1", 1), "passed");
    fx.must(&["unit", "land", "U-2"]);
}

#[test]
fn timing_the_next_command_is_not_held_by_a_running_wave_check() {
    let fx = Fx::new();
    slow_plan(&fx);
    start(&fx);
    land_first(&fx, "good\n");
    let mut child = start_slow_step(&fx);
    let t = std::time::Instant::now();
    fx.must(&["next", "--json"]);
    fx.must(&["status", "--json"]);
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "the next command waited for the wave check: {:?}",
        t.elapsed()
    );
    assert!(child.wait().unwrap().success());
}

// ---- R-61: the default, the question, the override ----

fn run_only(fx: &Fx) {
    plan(fx, false);
    fx.must(&["run", "start"]);
}

#[test]
fn a_project_with_no_default_is_not_an_error_and_the_question_offers_the_choices() {
    let fx = Fx::new();
    let o = fx.must(&["checks", "wave", "state", "--json"]).json();
    assert!(o["default"].is_null(), "{o}");
    run_only(&fx);
    let q = fx
        .must(&[
            "checks",
            "wave",
            "question",
            "cargo test=the whole suite",
            "--json",
        ])
        .json();
    let opts = q["question"]["options"].to_string();
    assert!(
        opts.contains("propose") && opts.contains("cargo test"),
        "{opts}"
    );
    assert!(opts.contains("the whole suite"), "{opts}");
    // A proposal with no reason is refused.
    fx.refused(&["checks", "wave", "question", "cargo test"]);
    assert!(opts.contains("override") && opts.contains("none"), "{opts}");
}

#[test]
fn the_runs_choice_is_recorded_only_on_the_humans_words_and_the_plan_holds_it() {
    let fx = Fx::new();
    run_only(&fx);
    let p = fx
        .must(&["checks", "wave", "propose", "override", GOOD, "--json"])
        .json();
    assert_eq!(p["changed"], false, "{p}");
    // Nothing was said by the human yet.
    fx.refused(&["checks", "wave", "set", "--quote", "yes"]);
    assert!(fx.must(&["checks", "wave", "state", "--json"]).json()["run_choice"].is_null());
    fx.human("Yes, use that wave check for this run.");
    let s = fx
        .must(&[
            "checks",
            "wave",
            "set",
            "--quote",
            "use that wave check",
            "--json",
        ])
        .json();
    assert_eq!(s["run_choice"]["source"], "override", "{s}");
    // The plan holds its own copy as a check record, and the page shows where it came from.
    assert_eq!(s["plan_checks"][0]["command"], GOOD, "{s}");
    let page = fx.must(&["view", "plan", "--json"]).stdout;
    assert!(
        page.contains("Wave checks") && page.contains("from an override for this run"),
        "{page}"
    );
    // An override does not become the project default by itself.
    assert!(s["default"].is_null(), "{s}");
    assert!(!fx.root.join("factory/wave.json").exists());
}

#[test]
fn the_default_changes_only_on_a_yes_and_a_later_run_can_use_it() {
    let fx = Fx::new();
    run_only(&fx);
    fx.must(&["checks", "wave", "propose", "override", GOOD]);
    fx.human("Yes, that one.");
    fx.must(&["checks", "wave", "set", "--quote", "that one"]);
    // The follow-up question: should it also become the default?
    fx.must(&[
        "checks", "wave", "propose", "override", GOOD, "--scope", "default",
    ]);
    assert!(fx.must(&["checks", "wave", "state", "--json"]).json()["default"].is_null());
    fx.human("Yes, make it the project default.");
    let s = fx
        .must(&[
            "checks",
            "wave",
            "set",
            "--scope",
            "default",
            "--quote",
            "make it the project default",
            "--json",
        ])
        .json();
    assert_eq!(s["default"]["enabled"], true, "{s}");
    assert_eq!(
        s["default"]["checks"][0]["command"],
        serde_json::json!(["grep", "-q", "good", "src/a.txt"]),
        "{s}"
    );
    // The pages show it.
    assert!(fx.root.join("factory/wave.json").exists());
    let page = fx.must(&["view", "plan", "--json"]).stdout;
    assert!(page.contains("Project default"), "{page}");
    // Using the default needs one.
    let p = fx
        .must(&["checks", "wave", "propose", "use-default", "--json"])
        .json();
    assert_eq!(p["value"]["source"], "default", "{p}");
}

#[test]
fn a_default_is_refused_when_there_is_none_and_the_choice_of_no_wave_checks_is_said_on_the_plan() {
    let fx = Fx::new();
    run_only(&fx);
    fx.refused(&["checks", "wave", "propose", "use-default"]);
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    let page = fx.must(&["view", "plan", "--json"]).stdout;
    assert!(page.contains("This plan has no wave check"), "{page}");
    assert!(page.contains("the run chose no wave checks"), "{page}");
}

#[test]
fn after_the_plan_gate_the_runs_wave_checks_change_only_by_an_amendment() {
    let fx = Fx::new();
    run_only(&fx);
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    fx.must(&["checks", "unit", "propose", "none"]);
    fx.human("No checks for every unit.");
    fx.must(&["checks", "unit", "set", "--quote", "No checks for every"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    let o = fx.refused(&["checks", "wave", "propose", "override", GOOD]);
    assert!(o.stderr.contains("amendment"), "{}", o.stderr);
}

#[test]
fn the_plan_gate_does_not_open_while_the_plan_questions_are_unanswered() {
    let fx = Fx::new();
    run_only(&fx);
    let o = fx.refused(&["gate", "open", "plan"]);
    assert!(
        o.stderr.contains("bf checks wave question")
            && o.stderr.contains("bf checks unit question"),
        "{}",
        o.stderr
    );
    let ds = directives(&fx);
    assert!(find(&ds, "answer-wave-checks").is_some(), "{ds:?}");
    assert!(find(&ds, "answer-unit-checks").is_some(), "{ds:?}");
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    let o = fx.refused(&["gate", "open", "plan"]);
    assert!(
        !o.stderr.contains("bf checks wave question")
            && o.stderr.contains("bf checks unit question"),
        "{}",
        o.stderr
    );
    fx.must(&["checks", "unit", "propose", "none"]);
    fx.human("No checks for every unit.");
    fx.must(&["checks", "unit", "set", "--quote", "No checks for every"]);
    fx.must(&["gate", "open", "plan"]);
}

#[test]
fn a_later_wave_unit_waits_for_the_earlier_wave_check_though_that_wave_has_not_fully_landed() {
    let fx = Fx::new();
    plan(&fx, true);
    // A second wave-1 unit that U-2 does not depend on.
    for (k, t) in [
        ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"third\"\ndelivers=[\"R-1\"]\nfiles=[\"src/c.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"c4\"\nhow=\"command\"\ncommand=\"test -f src/c.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let path = fx.project().record_path("K-1").unwrap();
    let k = fs::read_to_string(&path)
        .unwrap()
        .replace("\"U-2\"", "\"U-2\", \"U-3\"");
    fs::write(&path, k).unwrap();
    fx.commit_all("more records");
    start(&fx);
    land_first(&fx, "good\n");
    review_second(&fx);
    // U-3 of wave 1 is still unlanded, so the wave-1 check cannot have run; U-2 must still wait.
    let o = fx.refused(&["unit", "land", "U-2"]);
    assert!(o.stderr.contains("wave check of wave 1"), "{}", o.stderr);
    // The unlanded wave is not deadlocked: U-3 lands, the check runs, then U-2 lands.
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-3", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::write(wc.join("src/c.txt"), "third\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-4",
            "--unit",
            "U-3",
            "--",
            "test",
            "-f",
            "src/c.txt",
        ],
    );
    fx.must(&["report", "U-3", "--result", "built"]);
    fx.must(&["report", "U-3", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-3"]);
    fx.refused(&["unit", "land", "U-2"]);
    fx.must(&["checkpoint", "wave-check", "K-1", "--wave", "1"]);
    fx.must(&["unit", "land", "U-2"]);
}

#[test]
fn a_wave_check_record_needs_a_command_and_belongs_to_no_unit() {
    let fx = Fx::new();
    let o = fx.put(
        "check",
        "id=\"X-9\"\nkind=\"check\"\ntitle=\"w\"\nhow=\"human\"\nproves=[]\nscope=\"wave\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    let o = fx.put(
        "check",
        "id=\"X-9\"\nkind=\"check\"\ntitle=\"w\"\nhow=\"command\"\ncommand=\"true\"\nproves=[]\nscope=\"wave\"\nunit=\"U-1\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
}
