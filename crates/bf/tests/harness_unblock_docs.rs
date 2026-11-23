//! The conductor's release of a blocked unit and the automatic sync are told in the shipped
//! documents. Each test reads the text and checks the lines the unit promises.

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
fn build_loop_has_judge_block_with_its_three_commands() {
    let t = read("harness/claude/skills/factory/build-loop.md");
    has(
        &t,
        "build-loop.md",
        &[
            "`judge-block`",
            "bf unit unblock <U> --why",
            "bf unit unblock <U> --wait --why",
            "bf unit unblock <U> --human --why",
            "when in doubt choose `--wait`",
            "sync_conflicts",
            "two conductor releases",
        ],
    );
}

#[test]
fn skill_holds_the_approved_unblock_sentence() {
    let t = read("harness/claude/skills/factory/SKILL.md");
    has(
        &t,
        "SKILL.md",
        &["`bf unit release` stays the human's; `bf unit unblock <U> --why \"...\"` is the conductor's own judgment that the reason a builder gave for a block is fixed, allowed at most twice for a unit, and it is not one of the human's acts."],
    );
}

#[test]
fn build_loop_no_longer_runs_a_sync_before_a_worker() {
    let t = read("harness/claude/skills/factory/build-loop.md");
    assert!(
        !t.contains("run `bf unit sync <U>`"),
        "the old sync step must be gone"
    );
}

#[test]
fn builder_states_its_reason_and_resolves_named_conflicts() {
    let t = read("harness/claude/agents/builder.md");
    has(
        &t,
        "builder.md",
        &[
            "carries in `--note` what holds the unit and what would end the hold",
            "the conductor releases the unit from those words",
            "names conflicted files",
            "since the report finishes the merge",
        ],
    );
}

#[test]
fn engine_doc_describes_unblock_and_the_automatic_sync() {
    let t = read("docs/engine.md");
    has(
        &t,
        "docs/engine.md",
        &[
            "bf unit unblock <U> --why",
            "the judgment (`fixed`, `wait` or `human`)",
            "`unit.unblock`",
            "The bound of two releases is counted since the latest act of the human",
            "The count of two starts again at the human's `bf unit release`",
            "an amendment approved by the judge never resets it",
            "After two releases of the unit, plain `--why` and `--wait` are refused, and `--human` is still accepted",
            "for `--wait` and `--human` given together",
            "a report with an empty note is not the conductor's to judge",
            "A release keeps the round and attempt counts",
            "whose hook payload names an agent type",
            "goes to the conductor's `judge-block` when it carries a reason",
            "The automatic sync",
            "`unit.sync`",
            "`bf unit land` and the command `bf unit sync` do not change",
        ],
    );
}

#[test]
fn skill_files_stay_within_200_lines() {
    for rel in [
        "harness/claude/skills/factory/SKILL.md",
        "harness/claude/skills/factory/build-loop.md",
        "harness/claude/agents/builder.md",
    ] {
        assert!(read(rel).lines().count() <= 200, "{rel} is over 200 lines");
    }
}
