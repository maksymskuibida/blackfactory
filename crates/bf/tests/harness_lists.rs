//! The role and skill files treat a unit's list of files as a recommendation (D-115, D-117): no
//! request for files, no scope review, no `bf unit scope`; the reviewer may ask for a security audit
//! and reads the builder's report as a claim to verify.

use std::fs;
use std::path::{Path, PathBuf};

fn read(rel: &str) -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const BUILDER: &str = "harness/claude/agents/builder.md";
const REVIEWER: &str = "harness/claude/agents/reviewer.md";
const LOOP: &str = "harness/claude/skills/factory/build-loop.md";
const AUDITOR: &str = "harness/claude/agents/security-auditor.md";

fn has(rel: &str, wants: &[&str]) {
    let t = read(rel);
    for w in wants {
        assert!(t.contains(w), "{rel} lacks: {w}");
    }
}

#[test]
fn none_of_the_four_files_holds_the_removed_request() {
    for rel in [BUILDER, REVIEWER, LOOP, AUDITOR] {
        let t = read(rel);
        for bad in ["--need-files", "review-scope", "bf unit scope"] {
            assert!(!t.contains(bad), "{rel} still holds {bad}");
        }
    }
}

#[test]
fn the_builder_changes_another_file_and_says_why() {
    has(
        BUILDER,
        &[
            "list of files is a recommendation",
            "say why in your report",
            "There is no request for files",
            "`--result gap`",
        ],
    );
}

#[test]
fn the_reviewer_may_fail_for_files_outside_the_list_and_ask_for_an_audit() {
    has(
        REVIEWER,
        &[
            "names each file and the changes in it that you see as not needed",
            "`asks = \"security\"`",
            "end your turn without a report",
            "Treat the builder's report as a reason to check, never as a fact",
            "say in the review which claim you could not verify",
            "You review cold. You have not seen the builder's session",
        ],
    );
}

#[test]
fn the_may_not_line_allows_the_report_and_forbids_a_transcript() {
    let t = read(REVIEWER);
    assert!(!t.contains("Read the builder's notes or any session transcript"));
    for w in [
        "as a claim to verify",
        "Read a session transcript, a note of the builder that is not in the brief",
    ] {
        assert!(t.contains(w), "reviewer.md lacks: {w}");
    }
}

#[test]
fn the_skill_starts_a_security_audit_and_never_asks_for_a_grant() {
    has(
        LOOP,
        &[
            "security-audit",
            "Start a security-auditor agent",
            "never asks for a grant of files",
            "suggests an amendment that sets the order and the lists",
            "a list changes only by an amendment",
        ],
    );
}

#[test]
fn the_auditor_files_findings_and_reports_built() {
    let t = read(AUDITOR);
    assert!(t.contains("--result built --role security-auditor"));
    assert!(!t.contains("--result findings"));
}

#[test]
fn a_builders_brief_holds_the_new_rule_and_no_request() {
    let t = read("crates/bf/strings/en/brief_rules.toml");
    assert!(t.contains("The list of files is a recommendation"));
    assert!(t.contains("give the reason in your report"));
    assert!(!t.contains("--need-files"));
}
