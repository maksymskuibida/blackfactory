//! The share of the context a session may fill (R-73, D-50, TH-30): saved by the status line, read by
//! the hook on the human's message.

mod common;
use common::*;
use serde_json::json;

const SID: &str = "0b6f1c3e-aaaa-bbbb-cccc-1234567890ab";

pub fn status_input(fx: &Fx, share: serde_json::Value) {
    let input = json!({
        "session_id": SID,
        "model": {"id": "claude-opus-5-5", "display_name": "Opus 5.5"},
        "context_window": {"used_percentage": share},
        "effort": {"level": "high"}
    });
    let o = fx.bf_stdin(&fx.root, &["progress"], &input.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn context_share_over_the_setting_makes_a_note() {
    let fx = Fx::new();
    status_input(&fx, json!(55.0));
    let o = fx.human("go on");
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(o.stdout.contains("55%"), "{}", o.stdout);
    assert!(o.stdout.contains("40%"), "{}", o.stdout);
    assert!(o.stdout.contains("new session, compact, or continue"));
    assert!(o.stdout.contains("Do not compact yourself"));
}

#[test]
fn context_share_under_the_setting_makes_no_note() {
    let fx = Fx::new();
    status_input(&fx, json!(30.0));
    assert_eq!(fx.human("go on").stdout, "");
}

#[test]
fn context_share_missing_makes_no_note() {
    let fx = Fx::new();
    assert_eq!(fx.human("go on").stdout, "");
    status_input(&fx, json!(null));
    assert_eq!(fx.human("go on").stdout, "");
}

#[test]
fn context_share_old_makes_no_note() {
    let fx = Fx::new();
    status_input(&fx, json!(90.0));
    let path = fx.root.join(".bf/session-state.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v[SID]["saved_at"] = json!(1);
    std::fs::write(&path, v.to_string()).unwrap();
    assert_eq!(fx.human("go on").stdout, "");
}

#[test]
fn context_share_setting_is_in_the_shipped_policy_and_follows_the_human() {
    let fx = Fx::new();
    let pol = std::fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    assert!(pol.contains("max_share = 40"), "{pol}");
    fx.set_policy("max_share = 40", "max_share = 60");
    status_input(&fx, json!(55.0));
    assert_eq!(fx.human("go on").stdout, "");
    status_input(&fx, json!(65.0));
    assert!(fx.human("go on").stdout.contains("60%"));
}

#[test]
fn the_status_line_returns_when_stdin_stays_open_and_silent() {
    use std::process::{Command, Stdio};
    let fx = Fx::new();
    let mut c = Command::new(bin())
        .env("BF_STATE_HOME", &fx.state_base)
        .arg("progress")
        .current_dir(&fx.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _held_open = c.stdin.take().unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(20) {
        if c.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let returned = start.elapsed() < std::time::Duration::from_secs(20);
    if !returned {
        let _ = c.kill();
    }
    let status = c.wait().unwrap();
    assert!(
        returned && status.success(),
        "bf progress hung on an open stdin"
    );
}

#[test]
fn the_status_line_still_prints_with_no_input() {
    let fx = Fx::new();
    let o = fx.bf_stdin(&fx.root, &["progress"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!o.stdout.trim().is_empty());
}
