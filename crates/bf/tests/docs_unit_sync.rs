//! The engine document and the build loop say that a conflict met by `bf unit sync` or at a
//! landing goes to the unit's builder.

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

fn section<'a>(text: &'a str, head: &str) -> &'a str {
    let start = text
        .find(head)
        .unwrap_or_else(|| panic!("no section {head}"));
    let rest = &text[start + head.len()..];
    let end = rest.find("\n### ").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn sync_section_names_sync_conflicts_and_is_not_refused_on_conflict() {
    let t = read("docs/engine.md");
    let s = section(&t, "### `bf unit sync <U>`");
    assert!(s.contains("sync_conflicts"));
    assert!(
        !s.contains("on a conflict, a dirty"),
        "no refusal on a conflict"
    );
    assert!(!s.contains("refused, leaving the working copy exactly as it was, on a conflict"));
}

#[test]
fn land_refuses_line_does_not_abort_for_any_conflict() {
    let t = read("docs/engine.md");
    let line = t
        .lines()
        .find(|l| l.starts_with("Refuses when: the plan gate is not approved; the unit is not ready or not reviewed"))
        .expect("the land Refuses when line");
    assert!(!line.contains("the merge conflicts (the merge is aborted)"));
    assert!(line.contains("protected place is aborted"));
}

#[test]
fn build_loop_sends_conflicts_to_the_builder() {
    let t = read("harness/claude/skills/factory/build-loop.md");
    for n in ["bf unit sync", "sync_conflicts", "bf brief"] {
        assert!(t.contains(n), "build-loop.md should contain `{n}`");
    }
    assert!(!t.contains("no step of the loop needs it"));
    assert!(t.lines().count() <= 200);
}
