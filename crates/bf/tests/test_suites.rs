//! The default and extended suites: the test runner's settings leave the timing tests out of the
//! default suite and run them, each alone, in the extended suite; role files carry the busy-machine rule.

use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn settings() -> String {
    fs::read_to_string(repo().join(".config/nextest.toml")).unwrap()
}

/// The text of one profile section, up to the next top-level profile header.
fn profile(text: &str, name: &str) -> String {
    let head = format!("[profile.{name}]");
    let start = text
        .find(&head)
        .unwrap_or_else(|| panic!("no profile {name}"));
    let rest = &text[start + head.len()..];
    let end = rest.find("\n[profile.").unwrap_or(rest.len());
    let body = &rest[..end];
    // Keep the overrides of the profile too: they start with `[[profile.<name>.overrides]]`.
    body.to_string()
}

fn filter_of(section: &str) -> String {
    section
        .lines()
        .find(|l| l.trim_start().starts_with("default-filter"))
        .unwrap_or_else(|| panic!("no default-filter in:\n{section}"))
        .to_string()
}

const KINDS: [&str; 3] = [
    "^timing::",
    "timing_",
    "controls::the_sandbox_costs_a_little_and_the_policy_can_switch_it_off",
];

#[test]
fn the_default_and_ci_profiles_leave_out_every_kind_of_timing_test() {
    let text = settings();
    for p in ["default", "ci"] {
        let f = filter_of(&profile(&text, p));
        assert!(f.contains("not ("), "{p} must exclude: {f}");
        for k in KINDS {
            assert!(f.contains(k), "{p} does not leave out {k}: {f}");
        }
    }
}

#[test]
fn the_extended_profile_runs_everything_and_each_timing_test_alone() {
    let text = settings();
    let ext = profile(&text, "extended");
    assert!(
        filter_of(&ext).contains("all()"),
        "extended runs every test"
    );
    assert!(
        !filter_of(&ext).contains("not ("),
        "extended must not leave tests out"
    );
    assert!(
        ext.contains("threads-required = \"num-test-threads\""),
        "a timing test must take every thread slot"
    );
    let overrides = ext
        .split("[[profile.extended.overrides]]")
        .nth(1)
        .expect("extended has an override for the timing tests");
    for k in KINDS {
        assert!(overrides.contains(k), "the override misses {k}");
    }
}

#[test]
fn every_role_file_that_runs_checks_has_the_busy_machine_rule() {
    let dir = repo().join("harness/claude/agents");
    let mut seen = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let text = fs::read_to_string(&p).unwrap();
        if text.contains("bf evidence") {
            seen += 1;
            let name = p.display();
            assert!(
                text.contains("On a busy machine"),
                "{name} lacks the rule on a busy machine"
            );
            assert!(
                text.contains("do not wait") || text.contains("No sleep, retry or wait loop"),
                "{name} does not forbid waiting"
            );
        }
    }
    assert!(seen >= 3, "expected the builder, verifier and reviewer");
}

fn role(name: &str) -> String {
    fs::read_to_string(repo().join(format!("harness/claude/agents/{name}.md"))).unwrap()
}

#[test]
fn the_builders_role_file_has_the_rule_for_a_sync() {
    let text = role("builder");
    for want in [
        "For a sync: run only the tests you change, selected by name",
        "never the checkpoint's own checks or the extended suite",
        "the engine runs the sync's checks",
    ] {
        assert!(text.contains(want), "the builder's role file lacks: {want}");
    }
}

#[test]
fn the_reviewers_role_file_has_the_rule_for_the_review_of_a_sync() {
    let text = role("reviewer");
    for want in [
        "For the review of a sync: list which checks the sync ran and on which content",
        "bf evidence verify --checkpoint <K>",
        "confirm they are the brief's sync checks, passing on the current content",
        "A sync check missing or failing there is a finding",
    ] {
        assert!(
            text.contains(want),
            "the reviewer's role file lacks: {want}"
        );
    }
}

#[test]
fn the_engine_document_names_the_checks_a_sync_runs_and_the_wave_and_unit_check_commands() {
    let text = fs::read_to_string(repo().join("docs/engine.md")).unwrap();
    for want in [
        "The sync's checks are the command checks every unit of the checkpoint is held to",
        "does not run the checkpoint's own checks in a sync",
        "bf checkpoint wave-check <K>",
        "bf checks wave question|propose|set|state",
        "bf checks unit question|propose|set|state",
        "[unit_checks]",
        "unit_checks_reason",
        "own_checks_reason",
    ] {
        assert!(text.contains(want), "docs/engine.md lacks: {want}");
    }
}
