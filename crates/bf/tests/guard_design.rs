//! U-97: a designer and a design reviewer design and judge only (D-111, D-113). The guard refuses them
//! cargo, `bf evidence run` and a copy of a repository; their briefs and role files say so.

mod common;
use bf::guard::decide;
use bf::strings::t;
use common::Fx;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const ROOT: &str = "/nonexistent-root";
const TYPES: &[&str] = &["designer", "design-reviewer", "uiux-designer"];

fn table() -> Value {
    json!({
        "v": 1,
        "root": ROOT,
        "allow": "abstain",
        "protected": [],
        "repos": [{"path": ROOT}, {"path": "/nonexistent-code"}],
        "work_prefix": "/nonexistent-root/.bf/work",
        "deny_commands": [],
    })
}

fn run(agent: Option<&str>, cmd: &str) -> bf::guard::Decision {
    let mut v = json!({"tool_name": "Bash", "tool_input": {"command": cmd},
        "session_id": "s1", "cwd": ROOT});
    if let Some(a) = agent {
        v["agent_type"] = json!(a);
    }
    decide(&v, &table())
}

const REFUSED: &[&str] = &[
    "cargo build",
    "CARGO_ALIAS_X=test cargo x",
    "export CARGO_ALIAS_X=test; cargo x",
    "cargo --config 'alias.x=\"test\"' x",
    "cargo t",
    "cargo b --release",
    "cargo c",
    "cargo r -- x",
    "cargo-nextest nextest run",
    "cargo-clippy clippy",
    "/usr/bin/cargo-nextest nextest run",
    "cargo check --workspace",
    "cargo clippy",
    "cargo test foo",
    "cargo nextest run --package=bf",
    "cargo +stable run -- x",
    "cargo --manifest-path a/Cargo.toml bench",
    "bf evidence run X-1 --unit U-1 -- true",
    "cd /x && cargo test",
    "cp -R /nonexistent-code /tmp/c",
    "cp -r /nonexistent-root/.bf/work/U-1 /tmp/c",
    "cp -a /nonexistent-code /tmp/c",
    "rsync -a /nonexistent-code/ /tmp/c/",
    "ditto /nonexistent-code /tmp/c",
    "tar cf /tmp/c.tar /nonexistent-code",
    "tar -C /nonexistent-code -cf /tmp/c.tar .",
    "git clone /nonexistent-code /tmp/c",
    "git worktree add /tmp/c",
];

const ALLOWED: &[&str] = &[
    "cargo --version",
    "git log",
    "bf record get R-1",
    "cat /nonexistent-code/README.md",
    "grep -rn foo /nonexistent-code",
    "cp notes.md /tmp/notes.md",
    "tar tf /tmp/c.tar",
];

#[test]
fn each_design_type_is_refused_each_named_command_with_the_keys_text() {
    for agent in TYPES {
        for cmd in REFUSED {
            let d = run(Some(agent), cmd);
            assert!(!d.allow, "{agent}: {cmd} passed");
            let key = if cmd.contains("evidence") {
                "guard_design.evidence.because"
            } else if cmd.contains("cargo") {
                "guard_design.cargo.because"
            } else {
                "guard_design.copy.because"
            };
            assert!(d.reason.contains(&t(key, &[])), "{agent}: {cmd}: {}", d.reason);
        }
    }
}

#[test]
fn reading_and_small_commands_stay_free_for_the_design_types() {
    for agent in TYPES {
        for cmd in ALLOWED {
            let d = run(Some(agent), cmd);
            assert!(d.allow, "{agent}: {cmd}: {}", d.reason);
        }
    }
}

#[test]
fn a_session_of_another_type_is_judged_as_before() {
    for agent in [None, Some("builder"), Some("reviewer")] {
        for cmd in REFUSED {
            if cmd.contains("evidence") || cmd.contains("cargo") {
                continue;
            }
            let d = run(agent, cmd);
            assert!(
                !d.reason.contains(&t("guard_design.copy.because", &[])),
                "{agent:?}: {cmd}"
            );
        }
        for cmd in ["cargo build", "cargo nextest run --package=bf", "bf evidence run X-1 --unit U-1 -- true"] {
            let d = run(agent, cmd);
            for k in ["cargo", "evidence", "copy"] {
                assert!(
                    !d.reason.contains(&t(&format!("guard_design.{k}.because"), &[])),
                    "{agent:?}: {cmd}"
                );
            }
        }
    }
}

fn brief_at(fx: &Fx, args: &[&str]) -> String {
    let mut a = args.to_vec();
    a.push("--json");
    let path = fx.must(&a).json()["path"].as_str().unwrap().to_string();
    fs::read_to_string(path).unwrap()
}

#[test]
fn the_three_kinds_of_brief_hold_the_section() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2", "C-1", "--json"]);
    let title = t("guard_design.brief.title", &[]);
    let designer = t("guard_design.brief.designer", &[]);
    let reviewer = t("guard_design.brief.reviewer", &[]);
    assert!(!title.is_empty() && !designer.is_empty() && !reviewer.is_empty());
    let rev = brief_at(&fx, &["review", "brief", "amendment", "--subject", "A-1"]);
    assert!(rev.contains(&title) && rev.contains(reviewer.trim()), "{rev}");
    let g = bf::run::issue_token(&fx.project(), "A-1:designer").unwrap();
    fx.must(&["review", "split", "A-1", "--stage", "requirements=R-2", "--token", &g]);
    let stage = brief_at(
        &fx,
        &["review", "brief", "requirements", "--subject", "A-1", "--role", "designer"],
    );
    assert!(stage.contains(&title) && stage.contains(designer.trim()), "{stage}");
    let general = brief_at(&fx, &["brief", "A-1", "--role", "designer"]);
    assert!(general.contains(&title) && general.contains(designer.trim()), "{general}");
}

fn role_file(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join("harness/claude/agents").join(name)).unwrap()
}

#[test]
fn the_two_role_files_hold_the_sentences() {
    let d = role_file("designer.md");
    for s in [
        "A designer designs and judges only",
        "run a build tool or copy the checkout",
        "name the gap in your report",
        "an investigator, started by the conductor for that one question",
    ] {
        assert!(d.contains(s), "designer.md lacks `{s}`");
    }
    let r = role_file("design-reviewer.md");
    for s in [
        "You judge only",
        "check each claim against the code and the receipts",
        "do not repeat a measurement",
        "Ask for a measurement only when a claim has none",
    ] {
        assert!(r.contains(s), "design-reviewer.md lacks `{s}`");
    }
}
