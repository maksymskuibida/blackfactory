//! The engine writes a unit's briefs at the state change that needs them, and refuses a reviewer's report
//! on a unit in review while the reviewer's brief file is missing.

mod common;
use common::*;
use std::fs;
use std::path::PathBuf;

fn brief_path(fx: &Fx, role: &str) -> PathBuf {
    fx.project().briefs_dir().join(format!("U-1-{role}.md"))
}

fn started() -> Fx {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    fx
}

const FINDING: &str = r#"[{"title":"Spelling of the greeting","severity":"medium","where":"src/hello.txt:1","action":"Fix the spelling"}]"#;

fn report_findings(fx: &Fx) -> Out {
    fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "U-1",
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--findings",
            "-",
            "--json",
        ],
        FINDING,
    )
}

#[test]
fn the_builders_brief_exists_after_the_unit_starts() {
    let fx = started();
    let text = fs::read_to_string(brief_path(&fx, "builder")).unwrap();
    assert!(text.contains("Greeting"), "{text}");
    // Starting it again leaves a brief in place after it was lost.
    fs::remove_file(brief_path(&fx, "builder")).unwrap();
    fx.must(&["unit", "start", "U-1"]);
    assert!(brief_path(&fx, "builder").exists());
}

#[test]
fn the_reviewers_brief_exists_after_the_builder_reports_built() {
    let fx = started();
    assert!(!brief_path(&fx, "reviewer").exists());
    fx.must(&["report", "U-1", "--result", "built"]);
    let text = fs::read_to_string(brief_path(&fx, "reviewer")).unwrap();
    assert!(text.contains("Greeting"), "{text}");
}

#[test]
fn the_builders_brief_is_written_again_with_the_finding() {
    let fx = started();
    fx.must(&["report", "U-1", "--result", "built"]);
    fs::remove_file(brief_path(&fx, "builder")).unwrap();
    let o = report_findings(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["state"], "fixing");
    let text = fs::read_to_string(brief_path(&fx, "builder")).unwrap();
    assert!(text.contains("Spelling of the greeting"), "{text}");
}

#[test]
fn the_reviewers_brief_is_written_again_after_the_fix() {
    let fx = started();
    fx.must(&["report", "U-1", "--result", "built"]);
    report_findings(&fx);
    fs::remove_file(brief_path(&fx, "reviewer")).unwrap();
    fx.must(&["report", "U-1", "--result", "fixed"]);
    let text = fs::read_to_string(brief_path(&fx, "reviewer")).unwrap();
    assert!(text.contains("Spelling of the greeting"), "{text}");
}

#[test]
fn a_report_without_the_reviewers_brief_is_refused_until_it_is_written() {
    let fx = started();
    fx.must(&["report", "U-1", "--result", "built"]);
    fs::remove_file(brief_path(&fx, "reviewer")).unwrap();
    let clean = ["report", "U-1", "--result", "built", "--role", "reviewer"];
    let o = fx.bf(&clean);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("bf brief U-1 --role reviewer"),
        "{}",
        o.stderr
    );
    assert_four_parts(&o.stderr);
    let o = report_findings(&fx);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(!fx.root.join("factory/records/F-1.toml").exists());
    let st = fx.must(&["status", "--json"]).json();
    let u = st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == "U-1")
        .unwrap()
        .clone();
    assert_eq!(u["state"], "review", "{u}");
    fx.must(&["brief", "U-1", "--role", "reviewer"]);
    let o = fx.bf(&clean);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_brief_that_cannot_be_written_is_said_and_no_older_one_is_left() {
    let fx = started();
    fx.must(&["report", "U-1", "--result", "built"]);
    let o = report_findings(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(brief_path(&fx, "reviewer").exists());
    // A receipt that cannot be read makes the reviewer's brief fail.
    let bad = fx.project().receipts_dir().join("rc-99999.toml");
    fs::create_dir_all(&bad).unwrap();
    let o = fx.bf(&["report", "U-1", "--result", "fixed"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(
        o.stderr.contains("could not be written")
            && o.stderr.contains("bf brief U-1 --role reviewer"),
        "{}",
        o.all()
    );
    assert!(!brief_path(&fx, "reviewer").exists());
    fs::remove_dir(&bad).unwrap();
    let st = fx.must(&["status", "--json"]).json();
    let u = st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == "U-1")
        .unwrap()
        .clone();
    assert_eq!(u["state"], "review", "{u}");
}
