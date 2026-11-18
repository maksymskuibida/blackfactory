//! The documents and instruction files say what the third checkpoint's code does. Each test reads
//! the shipped text and checks the lines the unit promises.

use std::fs;
use std::path::{Path, PathBuf};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)
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

fn read(rel: &str) -> String {
    fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The text of a `## ` section, from its heading to the next one.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = text
        .find(&format!("\n## {heading}"))
        .unwrap_or_else(|| panic!("no section {heading}"));
    let rest = &text[start + 1..];
    match rest[3..].find("\n## ") {
        Some(end) => &rest[..end + 3],
        None => rest,
    }
}

#[test]
fn the_role_file_holds_the_pass_after_fail_and_missing_brief_lines() {
    let t = read("harness/claude/agents/reviewer.md");
    for want in [
        "PASS-AFTER-FAIL",
        "failed and then passed at this content",
        "File a finding for each such check, or say in your closing sentence why the failure is harmless",
        "failing receipt's output",
        "if the file is missing, your report is refused, so stop and say so",
    ] {
        assert!(t.contains(want), "the role file lacks: {want}");
    }
}

#[test]
fn the_role_file_has_the_machine_load_paragraph_only_in_the_checkpoint_section() {
    let t = read("harness/claude/agents/reviewer.md");
    let lower = t.to_lowercase();
    assert_eq!(
        lower.matches("busy").count(),
        1,
        "the paragraph appears once in the file"
    );
    let checkpoint = section(&t, "Reviewing a whole checkpoint");
    assert!(
        checkpoint.contains("On a busy machine"),
        "the checkpoint section holds the paragraph"
    );
    let doing = section(&t, "Do");
    assert!(
        !doing.to_lowercase().contains("busy"),
        "the numbered steps do not hold it"
    );
}

#[test]
fn each_harness_instruction_file_stays_within_two_hundred_lines() {
    for dir in ["harness/claude/skills", "harness/claude/agents"] {
        let mut stack = vec![repo(dir)];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().map(|x| x == "md").unwrap_or(false) {
                    let n = fs::read_to_string(&p).unwrap().lines().count();
                    assert!(n <= 200, "{} has {n} lines", p.display());
                }
            }
        }
    }
}

#[test]
fn the_skill_says_the_engine_wrote_the_brief() {
    let t = read("harness/claude/skills/factory/build-loop.md");
    let at = t.find("## Starting a worker").expect("the starting section");
    let step2 = t[at..]
        .lines()
        .find(|l| l.starts_with("2. "))
        .expect("step 2");
    for want in ["The engine already wrote the unit's brief", "refreshes it"] {
        assert!(step2.contains(want), "step 2 lacks: {want}");
    }
}

#[test]
fn the_skill_names_each_review_check_and_the_review_command() {
    let t = skill_all();
    for want in [
        "`idea-review`",
        "`requirements-review`",
        "`architecture-review`",
        "`plan-review`",
        "`amendment-review`",
        "bf review",
        "bf review finding",
        "bf review release",
        "bf review split",
        "max_rounds",
        "general designer",
    ] {
        assert!(t.contains(want), "the skill lacks: {want}");
    }
}

#[test]
fn the_engine_document_names_the_panel_folder_among_the_protected_paths() {
    let t = read("docs/engine.md");
    let at = t.find("**What the table holds**").expect("the table paragraph");
    let line = t[at..].lines().next().unwrap();
    assert!(
        line.contains(".claude/skills/factory-panel/"),
        "the protected paths lack the installed panel folder"
    );
}

#[test]
fn the_engine_document_holds_a_section_for_the_checkpoint_insertion_command() {
    let t = read("docs/engine.md");
    let heading = t
        .lines()
        .find(|l| l.starts_with("### `bf amend checkpoint"))
        .expect("a section for the command");
    assert!(heading.contains("before|after|split"), "{heading}");
    let at = t.find(heading).unwrap();
    let body = &t[at..];
    let end = body[heading.len()..].find("\n### ").unwrap_or(body.len() - heading.len());
    let body = &body[..heading.len() + end];
    for want in ["```bash", "bf amend checkpoint after K-3", "one number forward", "moved"] {
        assert!(body.contains(want), "the section lacks: {want}");
    }
}
