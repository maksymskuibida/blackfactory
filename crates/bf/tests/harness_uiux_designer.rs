//! The UI/UX designer's role file limits the agent to the screens and journeys its brief names,
//! names the contract and the guides, and keeps splitting and judging away from it.

use std::fs;
use std::path::Path;

const FILE: &str = "harness/claude/agents/uiux-designer.md";

fn text() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(FILE);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{FILE}: {e}"))
}

fn has(text: &str, needles: &[&str]) {
    for n in needles {
        assert!(text.contains(n), "{FILE} should contain `{n}`");
    }
}

#[test]
fn the_file_has_its_name_and_tools() {
    let t = text();
    assert!(t.starts_with("---\nname: uiux-designer\n"));
    has(&t, &["model: opus", "tools:"]);
}

#[test]
fn the_file_limits_the_agent_to_the_named_screens_and_journeys() {
    has(
        &text(),
        &[
            "Draft only the screens `S` and journeys `J` the brief names, and nothing else",
            "for the `ui` stage, or for the UI/UX stage of an amendment",
            "Draft a record the brief does not name",
            "only for the IDs the split gave your stage",
        ],
    );
}

#[test]
fn the_file_names_the_contract_the_guides_and_the_states() {
    has(
        &text(),
        &[
            "the contract you design against",
            "the UI/UX guides of the harness",
            "empty, loading, filled, error",
            "a decision record that says so and why",
        ],
    );
}

#[test]
fn the_file_never_splits_and_never_judges() {
    has(
        &text(),
        &[
            "You never report a split and never ask for further agents",
            "Judge your own screens. The UI/UX verifier judges them",
            "you never attach a receipt",
        ],
    );
}

#[test]
fn the_file_stays_short() {
    assert!(text().lines().count() < 100);
}
