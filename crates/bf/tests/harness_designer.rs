//! The designer's role file: every design stage with what to draft and how its coverage is shown,
//! the split into stage designers and its limits, and the reconciling.

use std::fs;
use std::path::Path;

fn designer() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join("harness/claude/agents/designer.md")).unwrap()
}

fn has(text: &str, needles: &[&str]) {
    for n in needles {
        assert!(text.contains(n), "designer.md should contain `{n}`");
    }
}

/// Each stage says what to draft and how its coverage is shown.
#[test]
fn the_designer_file_names_every_stage_with_its_drafts_and_coverage() {
    has(
        &designer(),
        &[
            "**Requirements.**",
            "testable acceptance lines",
            "the journeys too",
            "**UI/UX.**",
            "screens with their states",
            "**Architecture and threats.**",
            "decisions with the options you rejected",
            "targets that can be measured",
            "`protects`",
            "`attack`",
            "`mitigation`",
            "**Legal and markets.**",
            "the markets record and the legal records",
            "**Units plan.**",
            "units of at most 8 points that own disjoint files",
            "the exact command and what each proves",
            "checkpoints with what each proves",
            "The reviewer measures",
            "Coverage: the requirements and journeys in the set, or a decision",
            "Coverage: the screens and journeys in the set, or a decision",
            "Coverage: the components, decisions and threats in the set, or a decision",
            "Coverage: the markets record and legal records in the set, or a decision",
            "Coverage: the units, checks and checkpoints in the set, or a decision",
            "Every record you add or change is named in the amendment's set",
            "A stage you do not touch gets a decision record saying that it is not affected and why",
        ],
    );
}

/// The split, who the stage designers are, and its limits.
#[test]
fn the_designer_file_carries_the_split_and_its_limits() {
    has(
        &designer(),
        &[
            "Draft first, then decide: design the whole amendment yourself, or ask for stage designers",
            "`bf review split`",
            "the IDs of the amendment's set its stage designer drafts",
            "give each ID to one stage only",
            "only for a stage the split named, at most one per stage and amendment",
            "the engine accepts your records only for those IDs",
            "You never split: you cannot report a split, so no agent asks for further agents",
            "the designer limited to one stage for requirements, architecture and the units plan",
            "the UI/UX designer for screens and journeys",
            "the lawyer for legal records and the markets record",
            "the security expert for the threats of the architecture stage",
        ],
    );
}

/// The reconciling, and the line target of the harness text, per file.
#[test]
fn the_designer_file_carries_the_reconciling_and_stays_short() {
    let t = designer();
    has(
        &t,
        &[
            "## Reconciling",
            "read all their drafts together",
            "Make the links agree across stages",
            "Check that every stage has its records or its decision",
            "then report the amendment designed",
            "The amendment review runs after that",
        ],
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for rel in [
        "harness/claude/skills/factory/SKILL.md",
        "harness/claude/agents/builder.md",
        "harness/claude/agents/reviewer.md",
        "harness/claude/agents/verifier.md",
        "harness/claude/agents/designer.md",
    ] {
        let n = fs::read_to_string(root.join(rel)).unwrap().lines().count();
        assert!(n <= 200, "{rel} has {n} lines");
    }
}
