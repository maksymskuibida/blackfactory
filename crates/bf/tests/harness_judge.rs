//! The conductor's skill, the builder's role file and the engine document say how the repair judge and a
//! checkpoint fix's requests for files work (R-69, R-70, R-71, D-90). Each test reads the shipped text.

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

fn skill_all() -> String {
    let dir = repo("harness/claude/skills/factory");
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

fn has(text: &str, what: &str, needles: &[&str]) {
    for n in needles {
        assert!(text.contains(n), "{what} should contain `{n}`");
    }
}

#[test]
fn the_skill_says_when_to_start_the_judge() {
    has(
        &skill_all(),
        "the skill",
        &[
            "`judge-repair`",
            "a subagent of type `repair-judge`",
            "without stopping and without asking",
            "--from-design",
            "ends with no verdict taken",
            "Never start a second judge for that repair",
            "bf amend seen",
            "X-104, X-105 and X-106",
            "X-114, X-115 and X-116",
            "X-119, X-120 and X-121",
            "`tests`, `full_suite`, `heavy` or PL-1",
        ],
    );
}

#[test]
fn the_skill_says_what_to_run_on_a_checkpoint_fix() {
    has(
        &skill_all(),
        "the skill",
        &[
            "bf checkpoint fix <K> --open",
            "bf checkpoint scope <K> --verdict -",
            "--accept low",
        ],
    );
}

#[test]
fn the_builder_changes_files_of_a_checkpoint_fix_and_says_why() {
    has(
        &read("harness/claude/agents/builder.md"),
        "builder.md",
        &[
            "The brief's list of files is a recommendation",
            "say why in your report",
            "A checkpoint fix's builder does the same",
            "`--result gap`",
        ],
    );
}

#[test]
fn the_engine_document_describes_the_judge_and_the_fix_path() {
    has(
        &read("docs/engine.md"),
        "engine.md",
        &[
            "**The judged plan repair (D-90).**",
            "`judge_scope_percent`",
            "`judge_chain_waves`",
            "amend.judge.failed",
            "repair_judge",
            "trusts it and cannot check it",
            "bf amend seen",
            "**The checkpoint fix's path in R-70.**",
            "saves its work",
        ],
    );
}

#[test]
fn the_touched_skill_files_stay_under_the_line_target() {
    for rel in [
        "harness/claude/skills/factory/SKILL.md",
        "harness/claude/skills/factory/checkpoint.md",
        "harness/claude/skills/factory/repair.md",
        "harness/claude/agents/builder.md",
    ] {
        let n = read(rel).lines().count();
        assert!(n <= 200, "{rel} has {n} lines");
    }
}
