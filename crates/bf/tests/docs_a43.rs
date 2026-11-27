//! A-43 and A-47 in the engine document, the architecture document, the conductor's checkpoint instructions
//! and the verifier's role file (U-140): each text names the commands and rules the audit rounds, the
//! confirmed findings and the human's own findings add.

use std::fs;
use std::path::{Path, PathBuf};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn has(text: &str, what: &str, needles: &[&str]) {
    for n in needles {
        assert!(text.contains(n), "{what} should contain `{n}`");
    }
}

#[test]
fn the_engine_document_names_the_audit_rounds_the_confirmed_findings_and_the_human_findings() {
    let engine = read("docs/engine.md");
    has(
        &engine,
        "docs/engine.md",
        &[
            "audit-checkpoint",
            "confirmed",
            "still_open",
            "answered",
            "--check",
            "--human",
            "--fix-quote",
            "--fix-findings",
            "f<round>.<number>",
            "bf unit accept",
            "the source `human`",
            "checkpoint_reviews",
        ],
    );
}

#[test]
fn the_architecture_document_names_the_audit_roles_and_the_finding_outcomes() {
    let arch = read("docs/architecture.md");
    has(
        &arch,
        "docs/architecture.md",
        &[
            "audit round",
            "answered",
            "confirmed by the checkpoint review",
        ],
    );
}

#[test]
fn the_conductors_checkpoint_file_names_the_audit_action_and_the_human_filing() {
    let ck = read("harness/claude/skills/factory/checkpoint.md");
    has(
        &ck,
        "checkpoint.md",
        &[
            "audit-checkpoint",
            "--human",
            "--fix-quote",
            "--fix-findings",
            "the same message",
        ],
    );
    assert!(ck.lines().count() <= 200, "T-7");
}

#[test]
fn the_verifiers_file_no_longer_attaches_an_outcome_to_a_checkpoints_agent_check() {
    let v = read("harness/claude/agents/verifier.md");
    assert!(
        !v.contains("bf evidence attach <check> <file> --outcome"),
        "the verifier is told to attach an outcome"
    );
    has(
        &v,
        "verifier.md",
        &["audit-checkpoint", "command checks only"],
    );
    has(
        &v,
        "verifier.md",
        &[
            "--role verifier --check <X> --token <token> --result findings --findings -",
            "Attach no outcome",
            "`bf brief` and `bf report`",
        ],
    );
    assert!(v.lines().count() <= 200, "T-7");
}

#[test]
fn the_documents_hold_no_em_dash_in_what_this_unit_added() {
    for f in ["docs/engine.md", "docs/architecture.md"] {
        let t = read(f);
        for key in ["Audit rounds (R-77)", "The human's own findings (R-81)"] {
            if let Some(i) = t.find(key) {
                let end = t[i..].find("\n\n").map(|x| i + x).unwrap_or(t.len());
                assert!(!t[i..end].contains('\u{2014}'), "{f}: {key}");
            }
        }
    }
}
