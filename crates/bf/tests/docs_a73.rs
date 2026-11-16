//! A-73 in the engine document and the conductor's instructions (U-107): each text names the
//! commands and rules the A-73 code units add.

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
fn the_engine_document_names_the_a73_commands_and_rules() {
    let doc = read("docs/engine.md");
    has(
        &doc,
        "engine.md",
        &["unit pause", "unit restore", "unit format", "reconsider"],
    );
    assert!(
        doc.contains("wait_for") || doc.contains("waiting"),
        "engine.md should describe the wait for a held file"
    );
}

#[test]
fn the_engine_document_names_unit_sync() {
    let doc = read("docs/engine.md");
    has(&doc, "engine.md", &["unit sync", "unit.sync"]);
}

#[test]
fn the_skill_names_the_commands_and_what_still_goes_to_the_human() {
    let skill = read("harness/claude/skills/factory/SKILL.md");
    has(
        &skill,
        "SKILL.md",
        &[
            "unit format",
            "unit pause",
            "unit sync",
            "park at the round or attempt limit",
            "extension verdict",
            "protected place",
            "confirmed block",
        ],
    );
}
