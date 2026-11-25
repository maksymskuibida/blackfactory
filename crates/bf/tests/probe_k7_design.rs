//! K-7 review probes: shell forms by which a design role still builds, tests or copies a checkout.
use bf::guard::decide;
use serde_json::json;

const ROOT: &str = "/nonexistent-root";

fn allowed(cmd: &str) -> bool {
    let table = json!({"v": 1, "root": ROOT, "allow": "abstain", "protected": [],
        "repos": [{"path": ROOT}, {"path": "/nonexistent-code"}],
        "work_prefix": "/nonexistent-root/.bf/work", "deny_commands": []});
    let v = json!({"tool_name": "Bash", "tool_input": {"command": cmd},
        "session_id": "s1", "cwd": ROOT, "agent_type": "designer"});
    decide(&v, &table).allow
}

fn passing(cmds: &[&str]) -> Vec<String> {
    cmds.iter().filter(|c| allowed(c)).map(|c| c.to_string()).collect()
}

/// `rustup run` is read as a wrapper only when `run` is the word right after `rustup` and the
/// command right after the toolchain.
#[test]
fn probe_k7_design_rustup_forms() {
    let got = passing(&[
        "rustup +nightly run stable cargo test",
        "rustup run stable -- cargo test",
        "rustup run stable -- cargo nextest run",
    ]);
    assert!(got.is_empty(), "allowed for a designer: {got:?}");
}

/// Cargo subcommands and plain tools that build, run tests or copy a repository whole history.
#[test]
fn probe_k7_design_build_and_copy_forms() {
    let got = passing(&[
        "echo test | xargs cargo",
        "cargo install --path /nonexistent-code",
        "cargo miri test",
        "cargo llvm-cov",
        "git -C /nonexistent-code bundle create /tmp/x.bundle --all",
        "git -C /nonexistent-code archive -o /tmp/x.tar HEAD",
        "zip -r /tmp/c.zip /nonexistent-code",
    ]);
    assert!(got.is_empty(), "allowed for a designer: {got:?}");
}
