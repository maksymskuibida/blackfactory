//! K-7 review round 3 probes: forms the design roles bar still lets through.
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

/// rustup run with -- before the toolchain, or run option after the toolchain.
#[test]
fn probe_k7_r3_rustup_more_forms() {
    let got = passing(&[
        "rustup run -- stable cargo test",
        "rustup run stable --install cargo test",
    ]);
    assert!(got.is_empty(), "allowed for a designer: {got:?}");
}

/// git bundle and archive of a checkout reached by --git-dir=, GIT_DIR or --remote while -C names another folder.
#[test]
fn probe_k7_r3_git_dir_copies() {
    let got = passing(&[
        "git -C /tmp --git-dir=/nonexistent-code/.git bundle create /tmp/x.bundle --all",
        "GIT_DIR=/nonexistent-code/.git git -C /tmp bundle create /tmp/x.bundle --all",
        "git -C /tmp archive --remote=/nonexistent-code -o /tmp/x.tar HEAD",
        "zip -R /tmp/c.zip *",
    ]);
    assert!(got.is_empty(), "allowed for a designer: {got:?}");
}

/// Controls: forms that must stay allowed.
#[test]
fn probe_k7_r3_controls() {
    let got = passing(&["cargo --version", "cargo tree", "cargo metadata --format-version 1", "rustup run stable cargo --version"]);
    assert_eq!(got.len(), 4, "refused: {got:?}");
}
