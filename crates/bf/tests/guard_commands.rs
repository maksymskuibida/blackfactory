//! The guard's list of known `bf` commands covers every command the engine has.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn decision(fx: &Fx, cwd: &Path, cmd: &str) -> (String, String) {
    let payload =
        json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => ("abstain".into(), String::new()),
        s => {
            let v: Value = serde_json::from_str(s).unwrap();
            let h = &v["hookSpecificOutput"];
            (
                h["permissionDecision"].as_str().unwrap().to_string(),
                h["permissionDecisionReason"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
        }
    }
}

fn each_known(cmd: &str) {
    let fx = Fx::new();
    let line = format!("bf {cmd}");
    let (d, why) = decision(&fx, &fx.root, &line);
    assert_ne!(
        d, "deny",
        "`{line}` must pass the guard from the project folder: {why}"
    );
    assert!(
        !why.contains("not a plain subcommand"),
        "`{line}` is unknown to the guard: {why}"
    );
}

#[test]
fn checks_is_known() {
    each_known("checks list");
}
#[test]
fn progress_is_known() {
    each_known("progress");
}
#[test]
fn scan_is_known() {
    each_known("scan list");
}
#[test]
fn legal_is_known() {
    each_known("legal list");
}
#[test]
fn preset_is_known() {
    each_known("preset list");
}
#[test]
fn uiux_is_known() {
    each_known("uiux list");
}

#[test]
fn a_made_up_command_is_still_refused() {
    let fx = Fx::new();
    let (d, _) = decision(&fx, &fx.root, "bf frobnicate");
    assert_eq!(d, "deny");
}

#[test]
fn a_working_copy_may_not_run_the_new_commands() {
    let fx = Fx::tiny_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    for c in [
        "checks list",
        "progress",
        "scan list",
        "legal list",
        "preset list",
        "uiux list",
    ] {
        let (d, _) = decision(&fx, &wc, &format!("bf {c}"));
        assert_eq!(d, "deny", "`bf {c}` is the conductor's, not a worker's");
    }
    for c in [
        "record put threat --file t.toml",
        "record put legal --file l.toml",
        "record put finding --file f.toml",
    ] {
        let (d, _) = decision(&fx, &wc, &format!("bf {c}"));
        assert_ne!(
            d, "deny",
            "`bf {c}` passes the guard; the engine ties it to the role"
        );
    }
    let (d, _) = decision(&fx, &wc, "bf record put requirement --file r.toml");
    assert_eq!(d, "deny");
}
