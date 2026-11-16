//! R-95: a unit lands only on the content its last clean review judged; `bf evidence verify --unit`
//! holds the reviewer to the checks landing requires and flags a pass that followed a failure at the
//! same content. Also the review receipts: the fields they seal (A-63, A-73).

mod common;
use common::*;
use std::fs;
use std::path::{Path, PathBuf};

const LINT: &str = "test -f src/a.txt";

fn records(fx: &Fx, check_command: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", format!("id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"{check_command}\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n")),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

/// A run with no attached checks, the unit started and its file written.
fn setup(check_command: &str) -> (Fx, PathBuf) {
    let fx = Fx::new();
    records(&fx, check_command);
    fx.start_test_run();
    let wc = start_unit(&fx);
    (fx, wc)
}

fn start_unit(fx: &Fx) -> PathBuf {
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "one\n").unwrap();
    wc
}

fn run_check(fx: &Fx, wc: &Path, extra: &[&str]) -> Out {
    let mut a = vec!["evidence", "run", "X-1", "--unit", "U-1"];
    a.extend_from_slice(extra);
    a.extend(["--", "test", "-f", "src/a.txt"]);
    fx.must_in(wc, &a)
}

fn built_and_reviewed(fx: &Fx, wc: &Path) {
    run_check(fx, wc, &[]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
}

fn state(fx: &Fx) -> String {
    let p = fx.project();
    bf::events::replay(&p).unwrap().unit_state("U-1").to_string()
}

#[test]
fn a_check_every_unit_must_pass_is_held_by_verify_as_landing_holds_it() {
    let fx = Fx::new();
    fx.set_policy(
        "list = []",
        &format!("list = [{{ name = \"lint\", command = \"{LINT}\" }}]"),
    );
    records(&fx, "test -f src/a.txt");
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    fx.must(&["checks", "unit", "propose", "project"]);
    fx.human("Use the project's list.");
    fx.must(&["checks", "unit", "set", "--quote", "Use the project's list"]);
    let wc = start_unit(&fx);
    let lint = |extra: &[&str]| {
        let mut a = vec!["evidence", "run", "attached:lint", "--unit", "U-1"];
        a.extend_from_slice(extra);
        a.extend(["--", "test", "-f", "src/a.txt"]);
        fx.must_in(&wc, &a)
    };
    // The attached check ran at older content only.
    lint(&[]);
    fs::write(wc.join("src/a.txt"), "two\n").unwrap();
    built_and_reviewed(&fx, &wc);
    let o = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stdout.contains("attached:lint"), "{}", o.all());
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(o.stderr.contains("attached:lint"), "{}", o.stderr);
    // After it runs at the current content both agree.
    lint(&[]);
    let o = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(o.stdout.contains("attached:lint"), "{}", o.all());
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn a_unit_edited_after_its_clean_review_goes_back_to_review_and_lands_after_a_clean_report() {
    let (fx, wc) = setup("test -f src/a.txt");
    built_and_reviewed(&fx, &wc);
    assert_eq!(state(&fx), "reviewed");
    // An edit after the review, with fresh receipts for every check.
    fs::write(wc.join("src/a.txt"), "edited\n").unwrap();
    run_check(&fx, &wc, &[]);
    let p = fx.project();
    let brief = p.briefs_dir().join("U-1-reviewer.md");
    let _ = fs::remove_file(&brief);
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(
        o.stderr.contains("last clean review") && o.stderr.contains("round 1"),
        "{}",
        o.stderr
    );
    assert!(
        o.stderr.contains("back in review") && o.stderr.contains("bf next"),
        "{}",
        o.stderr
    );
    assert_eq!(state(&fx), "review");
    assert!(brief.exists(), "the reviewer's brief is written again");
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    assert_eq!(state(&fx), "reviewed");
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx), "landed");
}

#[test]
fn a_unit_reviewed_clean_and_not_edited_lands() {
    let (fx, wc) = setup("test -f src/a.txt");
    built_and_reviewed(&fx, &wc);
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx), "landed");
}

#[test]
fn a_run_that_does_not_require_review_lands_after_an_edit() {
    let fx = Fx::new();
    fx.set_policy("required = true", "required = false");
    records(&fx, "test -f src/a.txt");
    fx.start_test_run();
    let wc = start_unit(&fx);
    run_check(&fx, &wc, &[]);
    fx.must(&["report", "U-1", "--result", "built"]);
    assert_eq!(state(&fx), "reviewed");
    fs::write(wc.join("src/a.txt"), "edited\n").unwrap();
    run_check(&fx, &wc, &[]);
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx), "landed");
}

fn flag_path(fx: &Fx) -> PathBuf {
    fx.root.join(".bf/work/flag.txt")
}

fn run_flag(fx: &Fx, wc: &Path, extra: &[&str]) -> Out {
    let mut a = vec!["evidence", "run", "X-1", "--unit", "U-1", "--json"];
    a.extend_from_slice(extra);
    a.extend(["--", "test", "-f", "../flag.txt"]);
    fx.bf_in(wc, &a)
}

#[test]
fn a_pass_after_a_failure_at_the_same_content_is_marked_and_verify_still_passes() {
    let (fx, wc) = setup("test -f ../flag.txt");
    let failing = run_flag(&fx, &wc, &[]);
    assert_ne!(failing.code, 0, "{}", failing.all());
    fs::write(flag_path(&fx), "x").unwrap();
    let passing = run_flag(&fx, &wc, &["--attempt", "2", "--reason", "the flag was missing"]);
    assert_eq!(passing.code, 0, "{}", passing.all());
    let (f, g) = (
        failing.json()["id"].as_str().unwrap().to_string(),
        passing.json()["id"].as_str().unwrap().to_string(),
    );
    let o = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let line = format!("PASS-AFTER-FAIL X-1 failing {f} passing {g}");
    assert!(o.stdout.lines().any(|l| l == line), "{}", o.all());
    let o = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1", "--json"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let m = &o.json()["pass_after_failure"];
    assert_eq!(m.as_array().unwrap().len(), 1, "{m}");
    assert_eq!(
        (m[0]["check"].as_str(), m[0]["failing"].as_str(), m[0]["passing"].as_str()),
        (Some("X-1"), Some(f.as_str()), Some(g.as_str()))
    );
}

#[test]
fn a_pass_after_a_failure_at_older_content_is_not_marked() {
    let (fx, wc) = setup("test -f ../flag.txt");
    let failing = run_flag(&fx, &wc, &[]);
    assert_ne!(failing.code, 0, "{}", failing.all());
    fs::write(flag_path(&fx), "x").unwrap();
    fs::write(wc.join("src/a.txt"), "other\n").unwrap();
    let passing = run_flag(&fx, &wc, &[]);
    assert_eq!(passing.code, 0, "{}", passing.all());
    let o = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!o.stdout.contains("PASS-AFTER-FAIL"), "{}", o.all());
    let o = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1", "--json"]);
    assert!(o.json()["pass_after_failure"].as_array().unwrap().is_empty());
    // A pass with no failure before it is not marked either.
    let (fx, wc) = setup("test -f src/a.txt");
    run_check(&fx, &wc, &[]);
    let o = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    assert!(!o.stdout.contains("PASS-AFTER-FAIL"), "{}", o.all());
}

// Review receipts: what the outcome on a design review writes and seals.

const REVIEW_CHECK: &str = "id=\"X-11\"\nkind=\"check\"\ntitle=\"idea-review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"idea-review\"\n";

fn review_setup() -> (Fx, PathBuf) {
    let fx = Fx::with_checks();
    fx.tiny();
    assert_eq!(fx.put("check", REVIEW_CHECK).code, 0);
    fs::write(fx.root.join("factory/idea.md"), "A small greeting tool.\n").unwrap();
    let file = fx.root.join("review.md");
    fs::write(&file, "I read every record of the idea.\n").unwrap();
    (fx, file)
}

fn attach_met(fx: &Fx, file: &Path) -> Out {
    fx.must(&[
        "evidence",
        "attach",
        "X-11",
        file.to_str().unwrap(),
        "--outcome",
        "met",
        "--says",
        "I read every record of the idea",
        "--json",
    ])
}

fn receipt_text(fx: &Fx, id: &str) -> (PathBuf, String) {
    let path = fx.project().receipts_dir().join(format!("{id}.toml"));
    let text = fs::read_to_string(&path).unwrap();
    (path, text)
}

fn field(text: &str, key: &str) -> String {
    let v: toml::Value = toml::from_str(text).unwrap();
    v[key].as_str().unwrap_or("").to_string()
}

#[test]
fn a_review_attached_across_a_record_change_carries_different_hashes() {
    let (fx, file) = review_setup();
    let first = attach_met(&fx, &file).json()["id"].as_str().unwrap().to_string();
    let f = fx.root.join("replace.toml");
    fs::write(
        &f,
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists, changed\"\nacceptance=[\"src/hello.txt exists\"]\n",
    )
    .unwrap();
    let o = fx.bf(&["record", "put", "requirement", "--file", f.to_str().unwrap(), "--replace"]);
    assert_eq!(o.code, 0, "{}", o.all());
    fs::write(fx.root.join("factory/idea.md"), "A small greeting tool, changed.\n").unwrap();
    let second = attach_met(&fx, &file).json()["id"].as_str().unwrap().to_string();
    let (_, a) = receipt_text(&fx, &first);
    let (_, b) = receipt_text(&fx, &second);
    assert_eq!(field(&a, "review_kind"), "idea");
    assert_eq!(field(&b, "review_kind"), "idea");
    assert!(!field(&a, "records_hash").is_empty());
    assert_ne!(field(&a, "records_hash"), field(&b, "records_hash"));
}

#[test]
fn a_changed_records_hash_on_a_sealed_review_receipt_is_refused_by_verify() {
    let (fx, file) = review_setup();
    let id = attach_met(&fx, &file).json()["id"].as_str().unwrap().to_string();
    let o = fx.bf(&["evidence", "verify"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let (path, text) = receipt_text(&fx, &id);
    let old = field(&text, "records_hash");
    let fake = "0".repeat(old.len());
    fs::write(&path, text.replace(&old, &fake)).unwrap();
    let o = fx.bf(&["evidence", "verify"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stdout.contains("seal=BAD"), "{}", o.all());
}
