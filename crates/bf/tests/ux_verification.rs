//! U-4 (R-6, R-7, R-9, C-5, TH-6): the two UI/UX verifications take their evidence step by step.

mod common;
use common::Fx;
use std::fs;

fn setup(fx: &Fx, named: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("journey", "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\nsteps=[\"open\",\"read\"]\n".to_string()),
        ("screen", "id=\"S-1\"\nkind=\"screen\"\ntitle=\"Greeting\"\n".to_string()),
        ("check", format!("id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"{named}\"\n")),
    ] {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
}

fn evidence_file(dir: &std::path::Path, name: &str) -> String {
    let f = dir.join(name);
    fs::write(&f, "what was seen\n").unwrap();
    f.to_str().unwrap().to_string()
}

fn step(fx: &Fx, file: &str, outcome: &str, says: &str) -> common::Out {
    fx.bf(&["evidence", "attach", "X-1", file, "--outcome", outcome, "--says", says])
}

fn outside() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn the_verification_is_refused_until_every_journey_step_has_evidence() {
    let fx = Fx::new();
    setup(&fx, "design-uiux-verification");
    let tmp = outside();
    let f = evidence_file(tmp.path(), "a.txt");
    let whole = "I looked at every screen and journey";
    let o = step(&fx, &f, "met", whole);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("J-1 step 1 has no evidence"), "{}", o.stderr);
    assert_eq!(step(&fx, &f, "met", "J-1 step 1: the page opens").code, 0);
    let o = step(&fx, &f, "met", whole);
    assert!(o.stderr.contains("J-1 step 2 has no evidence"), "{}", o.stderr);
    assert!(!o.stderr.contains("J-1 step 1 has"), "{}", o.stderr);
    assert_eq!(step(&fx, &f, "met", "J-1 step 2: the greeting reads hello").code, 0);
    // Steps alone are not enough: the screen and the accessibility result need their own proof.
    let o = step(&fx, &f, "met", whole);
    assert!(o.stderr.contains("S-1 has no evidence"), "{}", o.stderr);
    assert!(o.stderr.contains("accessibility has no evidence"), "{}", o.stderr);
    assert_eq!(step(&fx, &f, "met", "S-1 state default: greeting shown").code, 0);
    let o = step(&fx, &f, "met", whole);
    assert!(!o.stderr.contains("S-1 has no evidence"), "{}", o.stderr);
    assert!(o.stderr.contains("accessibility has no evidence"), "{}", o.stderr);
    assert_eq!(step(&fx, &f, "met", "accessibility: checked every criterion").code, 0);
    let o = step(&fx, &f, "met", whole);
    assert_eq!(o.code, 0, "{}", o.all());
    let c = fx.must(&["uiux", "coverage", "X-1", "--json"]).json();
    assert_eq!(c["ok"], true, "{c}");
    assert_eq!(c["steps"].as_array().unwrap().len(), 2);
    assert_eq!(c["accessibility"]["status"], "met", "{c}");
    assert_eq!(c["accessibility"]["standard"], "WCAG 2.2 level AA");
}

fn cover_all(fx: &Fx, f: &str) {
    for s in [
        "J-1 step 1: ok",
        "J-1 step 2: ok",
        "S-1 state default: ok",
        "accessibility: checked",
    ] {
        assert_eq!(step(fx, f, "met", s).code, 0);
    }
}

#[test]
fn a_changed_journey_makes_the_whole_verification_stale() {
    let fx = Fx::new();
    setup(&fx, "design-uiux-verification");
    let tmp = outside();
    let f = evidence_file(tmp.path(), "a.txt");
    cover_all(&fx, &f);
    assert_eq!(step(&fx, &f, "met", "all covered").code, 0);
    let rec = tmp.path().join("j.toml");
    fs::write(
        &rec,
        "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\nsteps=[\"open\",\"read\",\"close\"]\n",
    )
    .unwrap();
    let o = fx.bf(&["record", "put", "journey", "--file", rec.to_str().unwrap(), "--replace"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let c = fx.bf(&["uiux", "coverage", "X-1", "--json"]).json();
    assert_eq!(c["ok"], false, "{c}");
    let o = step(&fx, &f, "met", "all covered again");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("J-1 step 3 has no evidence"), "{}", o.stderr);
}

#[test]
fn the_accessibility_standard_comes_from_the_design_records_and_failures_name_where() {
    let fx = Fx::new();
    setup(&fx, "built-uiux-verification");
    let o = fx.put(
        "decision",
        "id=\"D-1\"\nkind=\"decision\"\ntitle=\"Accessibility standard\"\nchosen=\"WCAG 2.2 level AAA\"\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let tmp = outside();
    let f = evidence_file(tmp.path(), "a.txt");
    assert_eq!(step(&fx, &f, "unreachable", "S-1 state default: not shown, 2.4.7 unknown").code, 0);
    assert_eq!(step(&fx, &f, "failed", "accessibility: focus is invisible, fails 2.4.7").code, 0);
    let c = fx.bf(&["uiux", "coverage", "X-1", "--json"]).json();
    assert_eq!(c["standard"], "WCAG 2.2 level AAA", "{c}");
    assert_eq!(c["accessibility"]["status"], "failed", "{c}");
    let wheres: Vec<String> = c["failed_criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["where"].as_str().unwrap().to_string())
        .collect();
    assert!(wheres.contains(&"S-1 state default".to_string()), "{c}");
    assert!(wheres.contains(&"accessibility check".to_string()), "{c}");
}

#[test]
fn the_built_verification_own_met_may_not_attach_a_file_from_the_built_result() {
    let fx = Fx::new();
    setup(&fx, "built-uiux-verification");
    let tmp = outside();
    let f = evidence_file(tmp.path(), "a.txt");
    // The file inside the built result exists before the steps are covered: adding it later would
    // change the content the step evidence is bound to.
    let inside = evidence_file(&fx.root, "summary.txt");
    cover_all(&fx, &f);
    let o = step(&fx, &inside, "met", "all covered");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("inside the built result"), "{}", o.stderr);
    let o = step(&fx, &f, "met", "all covered");
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn an_unreachable_or_failed_step_is_never_passed_and_a_failed_step_names_its_criterion() {
    let fx = Fx::new();
    setup(&fx, "design-uiux-verification");
    let tmp = outside();
    let f = evidence_file(tmp.path(), "a.txt");
    assert_eq!(step(&fx, &f, "unreachable", "J-1 step 1: the page never loaded").code, 0);
    assert_eq!(
        step(&fx, &f, "failed", "J-1 step 2: text contrast is below 4.5:1, fails 1.4.3").code,
        0
    );
    let o = step(&fx, &f, "met", "all done");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("could not be reached"), "{}", o.stderr);
    assert!(o.stderr.contains("J-1 step 2 failed"), "{}", o.stderr);
    let c = fx.bf(&["uiux", "coverage", "X-1", "--json"]).json();
    assert_eq!(c["ok"], false);
    assert_eq!(c["steps"][0]["status"], "unreachable");
    assert_eq!(c["failed_criteria"][0]["criterion"], "1.4.3");
    assert_eq!(c["failed_criteria"][0]["where"], "J-1 step 2");
    assert_eq!(c["standard"], "WCAG 2.2 level AA");
    // The step receipts are not a pass of the check.
    let v = fx.bf(&["evidence", "verify", "--json"]);
    assert!(!v.stdout.contains("\"exit\": 0"), "{}", v.stdout);
}

#[test]
fn a_step_that_is_not_recorded_is_refused() {
    let fx = Fx::new();
    setup(&fx, "design-uiux-verification");
    let tmp = outside();
    let f = evidence_file(tmp.path(), "a.txt");
    let o = step(&fx, &f, "met", "J-1 step 3: nothing there");
    assert_ne!(o.code, 0);
    assert!(o.stderr.contains("no step 3"), "{}", o.stderr);
    let o = step(&fx, &f, "met", "J-9 step 1: nothing there");
    assert!(o.stderr.contains("no record J-9"), "{}", o.stderr);
}

#[test]
fn step_evidence_goes_stale_when_the_design_changes() {
    let fx = Fx::new();
    setup(&fx, "design-uiux-verification");
    let tmp = outside();
    let f = evidence_file(tmp.path(), "a.txt");
    step(&fx, &f, "met", "J-1 step 1: ok");
    step(&fx, &f, "met", "J-1 step 2: ok");
    let rec = tmp.path().join("j.toml");
    fs::write(
        &rec,
        "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\nsteps=[\"open\",\"read\",\"close\"]\n",
    )
    .unwrap();
    let o = fx.bf(&["record", "put", "journey", "--file", rec.to_str().unwrap(), "--replace"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let c = fx.bf(&["uiux", "coverage", "X-1", "--json"]).json();
    assert_eq!(c["ok"], false, "{c}");
    assert_eq!(c["steps"][0]["status"], "missing");
}

#[test]
fn built_result_evidence_may_not_come_from_the_built_result() {
    let fx = Fx::new();
    setup(&fx, "built-uiux-verification");
    let inside = evidence_file(&fx.root, "shot.txt");
    let o = step(&fx, &inside, "met", "J-1 step 1: the page opens");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("inside the built result"), "{}", o.stderr);
    let tmp = outside();
    let f = evidence_file(tmp.path(), "shot.txt");
    assert_eq!(step(&fx, &f, "met", "J-1 step 1: the page opens").code, 0);
    assert_eq!(step(&fx, &f, "met", "J-1 step 2: reads hello").code, 0);
    assert_eq!(step(&fx, &f, "met", "S-1 state default: shown").code, 0);
    assert_eq!(step(&fx, &f, "met", "accessibility: checked").code, 0);
    assert_eq!(step(&fx, &f, "met", "every journey was driven").code, 0);
}

#[test]
fn coverage_refuses_a_check_that_is_not_a_ui_verification() {
    let fx = Fx::new();
    setup(&fx, "design-uiux-verification");
    let o = fx.bf(&["uiux", "coverage", "R-1"]);
    assert_ne!(o.code, 0);
    let o = fx.bf(&["uiux", "other"]);
    assert_ne!(o.code, 0);
}

#[test]
fn the_step_sentence_is_read_strictly() {
    use bf::uiux::{criteria, parse_step};
    assert!(parse_step("J-1 step 2: saw it").is_some());
    assert!(parse_step("J-1 step 0: saw it").is_none());
    assert!(parse_step("J-1 step 2:").is_none());
    assert!(parse_step("S-1 state empty list: blank").is_some());
    assert!(parse_step("X-1 step 1: no").is_none());
    assert_eq!(criteria("fails 1.4.3 and 2.4.7, not 12.4.3x"), vec!["1.4.3", "2.4.7"]);
}
