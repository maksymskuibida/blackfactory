//! U-22: the close of a run is enforced at the content being closed (R-3, R-4, C-3, D-15): the
//! choice of checks is decided before the plan check, a built check counts at the last checkpoint,
//! the last run's choice judges the close after the run stops, and the mirror file of the project
//! default is compared with the log by the doctor.

mod common;
use common::Fx;
use std::fs;

#[test]
fn a_project_with_no_check_settings_is_told_to_decide_them_before_the_plan_check() {
    let fx = Fx::with_checks();
    fx.tiny();
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("no check settings are decided"),
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("bf checks propose"), "{}", o.stderr);
    // Once the human decided, the plan check names what the plan lacks.
    fx.checks_decided_as_default();
    let o = fx.refused(&["plan", "check"]);
    assert!(!o.stderr.contains("no check settings"), "{}", o.stderr);
    assert!(o.stderr.contains("`accessibility`"), "{}", o.stderr);
}

#[test]
fn the_plan_does_not_demand_check_records_for_the_threat_model_and_the_markets_statement() {
    let fx = Fx::with_checks();
    fx.checks_decided_as_default();
    fx.tiny();
    let o = fx.refused(&["plan", "check"]);
    assert!(!o.stderr.contains("`threat-model`"), "{}", o.stderr);
    assert!(!o.stderr.contains("`markets-statement`"), "{}", o.stderr);
}

#[test]
fn a_named_check_record_must_belong_to_a_unit_or_be_listed_by_a_checkpoint() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.must(&["plan", "check"]);
    // A record that names the check, but no unit owns it and no checkpoint lists it.
    let o = fx.put("check", "id=\"X-50\"\nkind=\"check\"\ntitle=\"Loose\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"secret-scan\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("X-50") && o.stderr.contains("nothing runs it"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_built_check_must_be_listed_by_the_last_checkpoint() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    // Take the secret scan out of the last checkpoint: its record is then listed nowhere.
    let k = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\n";
    let f = fx.root.join("k1.toml");
    fs::write(&f, k).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "checkpoint",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("`secret-scan`") && o.stderr.contains("last checkpoint K-1"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_check_switched_on_is_refused_when_the_frozen_plan_cannot_meet_it() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    // The plan is frozen and has no check record for the secret scan.
    let o = fx.refused(&["checks", "propose", "secret-scan=on", "--scope", "run"]);
    assert!(
        o.stderr.contains("secret-scan") && o.stderr.contains("frozen"),
        "{}",
        o.stderr
    );
    let o = fx.refused(&["checks", "propose", "secret-scan=on", "--scope", "default"]);
    assert!(o.stderr.contains("secret-scan"), "{}", o.stderr);
}

#[test]
fn the_close_is_judged_by_the_choice_of_the_last_run_after_the_run_stops() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.start_test_run();
    fx.run_checks_all_off();
    fx.must(&["run", "stop"]);
    // The default still requires every check, but the close follows the run that was stopped.
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert_eq!(s["scope"], "default");
    let o = fx.refused(&["ledger", "check"]);
    assert!(!o.stderr.contains("accessibility"), "{}", o.stderr);
}

#[test]
fn a_run_cannot_be_stopped_while_a_required_check_has_no_proof() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.start_test_run();
    let o = fx.refused(&["run", "stop"]);
    assert!(
        o.stderr.contains("accessibility") && o.stderr.contains("X-92"),
        "{}",
        o.stderr
    );
    // The refusal names the act that was refused and the command to run again.
    assert!(
        o.stderr.contains("stop the run") && o.stderr.contains("Run `bf run stop` again"),
        "{}",
        o.stderr
    );
    assert!(!o.stderr.contains("bf ledger check"), "{}", o.stderr);
    fx.run_checks_all_off();
    fx.must(&["run", "stop"]);
}

#[test]
fn the_doctor_reports_a_decided_default_whose_mirror_file_is_gone() {
    let fx = Fx::with_checks();
    fx.checks_decided_as_default();
    fs::remove_file(fx.root.join("factory").join("checks.json")).unwrap();
    let o = fx.bf(&["doctor"]);
    assert!(
        o.all().contains("mirror file is missing")
            && !o.all().contains("no check settings are recorded"),
        "{}",
        o.all()
    );
}

#[test]
fn the_doctor_compares_the_mirror_file_of_the_default_with_the_log() {
    let fx = Fx::with_checks();
    let o = fx.must(&["doctor"]);
    assert!(o.stdout.contains("check settings: ok"), "{}", o.stdout);
    fx.must(&[
        "checks",
        "propose",
        "secret-scan=off:docs only",
        "--scope",
        "default",
    ]);
    fx.human("Yes, the secret scan is off for the project.");
    fx.must(&[
        "checks",
        "set",
        "--quote",
        "secret scan is off for the project",
    ]);
    let o = fx.must(&["doctor"]);
    assert!(o.stdout.contains("check settings: ok"), "{}", o.stdout);
    // The file is edited by hand: the log is the truth and the doctor says they differ.
    fs::write(
        fx.root.join("factory").join("checks.json"),
        "{\"off\": {}}\n",
    )
    .unwrap();
    let o = fx.bf(&["doctor"]);
    assert!(o.all().contains("differs"), "{}", o.all());
}

#[test]
fn a_scan_cannot_be_settled_by_a_human_step_or_an_agents_word() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.must(&["plan", "check"]);
    // The secret scan is a command; a record that names it with another way of proof is refused.
    let f = fx.root.join("x.toml");
    fs::write(&f, "id=\"X-91\"\nkind=\"check\"\ntitle=\"secret-scan\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"secret-scan\"\n").unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("X-91")
            && o.stderr.contains("proven by command")
            && o.stderr.contains("`how` is `agent`"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_design_check_switched_on_after_its_gate_is_refused_when_its_product_is_missing() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["gate", "open", "architecture"]);
    fx.must(&["gate", "approve", "architecture", "--simulated"]);
    let o = fx.refused(&["checks", "propose", "threat-model=on", "--scope", "run"]);
    assert!(
        o.stderr.contains("threat-model") && o.stderr.contains("design product"),
        "{}",
        o.stderr
    );
}
