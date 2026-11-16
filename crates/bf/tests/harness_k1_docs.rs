//! The conductor's instructions, the engine document, the records document and the engine's help
//! say what the first checkpoint's code does. Each test reads the shipped text and checks the lines
//! the unit promises.

mod common;
use common::*;
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
fn the_skill_names_each_new_demand() {
    let skill = read("harness/claude/skills/factory/SKILL.md");
    has(
        &skill,
        "the skill",
        &[
            "named = \"design-uiux-verification\"",
            "--outcome met",
            "--token",
            "an empty list for a product with no market",
            "listed in `checks` by the last checkpoint",
            "before `bf plan check`",
            "`bf roles list`",
            "`uiux-verifier`, `security-expert`, `security-auditor`, `lawyer`, `legal-auditor`, `design-reviewer`, `designer` and `uiux-designer`",
            "bf amend withdraw <A-n> --reason",
            "it works in the main checkout",
        ],
    );
    assert!(
        !skill.contains("withdraws the earlier one"),
        "the skill still states the replaced rule"
    );
}

#[test]
fn the_documents_state_the_corrected_rules() {
    has(
        &read("docs/engine.md"),
        "engine.md",
        &["can file findings only while a unit is in review, until a later unit changes that"],
    );
    has(
        &read("docs/using.md"),
        "using.md",
        &["Stopping a run needs every required check proven or switched off"],
    );
}

#[test]
fn the_skill_forbids_the_conductor_to_use_a_token() {
    let skill = read("harness/claude/skills/factory/SKILL.md");
    has(
        &skill,
        "the skill",
        &[
            "You never use a worker's token yourself",
            "settle a check, attach an outcome, file a review or file a report with a token",
            "relay its own result and findings unchanged",
            "the engine does not enforce",
        ],
    );
}

#[test]
fn the_harness_text_stays_under_the_line_target() {
    for rel in [
        "harness/claude/skills/factory/SKILL.md",
        "harness/claude/agents/builder.md",
        "harness/claude/agents/reviewer.md",
        "harness/claude/agents/verifier.md",
    ] {
        let n = read(rel).lines().count();
        assert!(n <= 200, "{rel} has {n} lines");
    }
}

#[test]
fn the_engine_document_describes_the_refusals_and_the_limit() {
    let doc = read("docs/engine.md");
    has(
        &doc,
        "engine.md",
        &[
            "**`run stop` is refused**",
            "It also refuses while the check settings are undecided",
            "a threat is a designed part",
            "a marked legal finding is not acknowledged",
            "opening the `ui` gate while the design-stage UI/UX verification",
            "bf roles list",
            "bf roles token design <role>",
            "**The design subject.**",
            "bf brief design --role <role>",
            "`factory/checks.json`",
            "A token tells roles apart and does not keep the conductor out",
        ],
    );
}

#[test]
fn the_records_document_matches_the_schema() {
    let doc = read("docs/records.md");
    has(
        &doc,
        "records.md",
        &[
            "| `named` |",
            "| threat | `protects` | string | **Required.**",
            "| threat | `attack` | string | **Required.**",
            "| threat | `mitigation` | string | **Required.**",
            "`unit.role-report`",
            "`tokens.reset`",
            "`checkpoint.sync.open`",
            "`evidence.outcome`",
            "`factory/checks.json`",
        ],
    );
}

#[test]
fn the_help_lists_the_checks_and_roles_commands() {
    let fx = Fx::new();
    let out = fx.bf(&["--help"]);
    let all = out.all();
    has(
        &all,
        "the help",
        &[
            "checks list|state|propose|set",
            "roles list|show|token",
            "withdraw",
        ],
    );
}
