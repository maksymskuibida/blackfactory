//! The human-absent flag (U-67, R-72, D-93, D-94): set only by the human's own command in a conductor
//! session of the run, released only by their yes there, and kept in the run's log.

mod common;
use common::*;
use serde_json::{json, Value};

/// One payload of the `UserPromptSubmit` hook; `extra` adds fields to it.
fn send(fx: &Fx, session: &str, text: &str, extra: Value) -> Out {
    let mut payload = json!({
        "session_id": session,
        "cwd": fx.root.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "UserPromptSubmit",
        "prompt": text,
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        payload[k] = v;
    }
    let o = fx.bf_stdin(&fx.root, &["human-turn"], &payload.to_string());
    assert_eq!(o.code, 0, "the hook exits 0: {}", o.all());
    o
}

fn say(fx: &Fx, session: &str, text: &str) -> Out {
    send(fx, session, text, json!({}))
}

const CMD: &str = "/factory:human-absent";
const BRIEF: &str = "Task (from the session that started this one):\nDo the work.";

fn status(fx: &Fx) -> Value {
    fx.must(&["absent", "status", "--json"]).json()
}

fn on(fx: &Fx) -> bool {
    status(fx)["on"] == true
}

fn kinds(fx: &Fx) -> Vec<String> {
    std::fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["kind"].as_str().map(String::from))
        .collect()
}

fn human_text(o: &Out) -> String {
    o.json()["systemMessage"].as_str().unwrap_or("").to_string()
}

fn context_text(o: &Out) -> String {
    o.json()["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[test]
fn command_alone_in_a_conductor_session_sets_the_flag() {
    let fx = Fx::new();
    assert!(!on(&fx));
    let o = say(&fx, "s1", CMD);
    assert!(on(&fx));
    let st = status(&fx);
    assert_eq!(st["session"], "s1");
    assert!(st["since"].is_string());
    assert!(kinds(&fx).contains(&"absent.set".to_string()));
    let h = human_text(&o);
    assert!(h.contains("`yes`"), "the set message says how it ends: {h}");
    assert!(h.contains("permission mode"), "default mode asks: {h}");
    assert!(h.contains("checkpoint"), "it names the checkpoint in hand: {h}");
    assert!(context_text(&o).contains("bf next"));
}

#[test]
fn what_is_not_the_humans_command_sets_nothing() {
    let fx = Fx::new();
    send(&fx, "a", CMD, json!({"agent_type": "builder"}));
    say(&fx, "b", &format!("<cross-session-message from=\"x\">{CMD}</cross-session-message>"));
    say(&fx, "c", &format!("{BRIEF}\n{CMD}"));
    say(&fx, "d", &format!("please run {CMD} now"));
    assert!(!on(&fx));
    assert!(kinds(&fx).iter().any(|k| k == "absent.refused"));
}

#[test]
fn a_session_started_by_another_session_sets_and_marks_nothing() {
    let fx = Fx::new();
    say(&fx, "nested", BRIEF);
    say(&fx, "nested", CMD);
    assert!(!on(&fx));
    say(&fx, "main", CMD);
    assert!(on(&fx));
    let o = say(&fx, "nested", "hello there");
    assert!(status(&fx)["returning"].as_array().unwrap().is_empty());
    assert!(human_text(&o).contains("conductor session"));
    assert!(!human_text(&o).contains("Are you back"));
}

#[test]
fn yes_releases_only_after_another_message_in_the_same_session() {
    let fx = Fx::new();
    say(&fx, "s1", CMD);
    let o = say(&fx, "s1", "yes");
    assert!(on(&fx), "yes as the first message does not release");
    assert!(human_text(&o).contains("Are you back"));
    let o = say(&fx, "s1", "YES!");
    assert!(!on(&fx));
    assert!(human_text(&o).contains("off"));
    assert!(context_text(&o).contains("Report"));
    assert!(status(&fx)["last_release"]["by"] == "message");
}

#[test]
fn a_new_conductor_session_releases_it() {
    let fx = Fx::new();
    say(&fx, "s1", CMD);
    say(&fx, "s2", "I am back, tell me more");
    assert!(on(&fx));
    say(&fx, "s2", "yes");
    assert!(!on(&fx));
}

#[test]
fn yes_in_a_session_where_it_is_not_returning_does_not_release() {
    let fx = Fx::new();
    say(&fx, "s1", CMD);
    say(&fx, "s1", "what is going on");
    say(&fx, "s2", "yes");
    assert!(on(&fx));
    let ret = status(&fx)["returning"].clone();
    assert!(ret.as_array().unwrap().contains(&json!("s1")));
}

#[test]
fn the_question_is_shown_to_the_human_and_the_conductor_on_every_message() {
    let fx = Fx::new();
    say(&fx, "s1", CMD);
    for text in ["first", "second"] {
        let o = say(&fx, "s1", text);
        assert!(human_text(&o).contains("`yes`"));
        assert!(context_text(&o).contains("ask this question"));
    }
}

#[test]
fn the_command_again_is_a_stay_that_clears_the_marks() {
    let fx = Fx::new();
    say(&fx, "s1", CMD);
    say(&fx, "s1", "hello");
    assert!(!status(&fx)["returning"].as_array().unwrap().is_empty());
    let o = say(&fx, "s1", CMD);
    assert!(on(&fx));
    assert!(status(&fx)["returning"].as_array().unwrap().is_empty());
    assert!(human_text(&o).contains("fully on again"));
    assert!(context_text(&o).contains("bf next"));
    assert!(kinds(&fx).contains(&"absent.stay".to_string()));
    assert!(status(&fx)["last_stay"].is_object());
}

#[test]
fn a_hand_written_set_for_another_or_a_nested_session_is_ignored() {
    let fx = Fx::new();
    say(&fx, "nested", BRIEF);
    say(&fx, "nested", CMD);
    say(&fx, "main", "hello");
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    let nested_turn = st.turns.iter().find(|t| t.text == CMD).unwrap().seq;
    let main_turn = st.turns.iter().find(|t| t.text == "hello").unwrap().seq;
    for (s, t) in [
        ("nested", nested_turn),
        ("main", nested_turn),
        ("main", main_turn),
    ] {
        bf::events::append(&p, "absent.set", json!({"session": s, "turn": t})).unwrap();
    }
    assert!(!on(&fx));
}

#[test]
fn a_session_mark_is_written_once_and_never_changes() {
    let fx = Fx::new();
    say(&fx, "s1", "hello");
    say(&fx, "s1", BRIEF);
    say(&fx, "s1", CMD);
    assert!(on(&fx), "the first payload was no brief, so s1 stays a conductor session");
    let n = std::fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["kind"] == "absent.session" && v["session"] == "s1")
        .count();
    assert_eq!(n, 1);
}

#[test]
fn with_the_switch_off_the_command_sets_nothing() {
    let fx = Fx::new();
    fx.set_policy("[guard]", "[absent]\nenabled = false\n\n[guard]");
    let o = say(&fx, "s1", CMD);
    assert!(!on(&fx));
    assert!(human_text(&o).contains("enabled = false"));
}

#[test]
fn the_signed_table_carries_the_flag() {
    let fx = Fx::new();
    say(&fx, "s1", CMD);
    say(&fx, "s1", "hello");
    let t: Value =
        serde_json::from_str(&std::fs::read_to_string(fx.project().guard_path()).unwrap()).unwrap();
    assert_eq!(t["absent"]["on"], true);
    assert_eq!(t["absent"]["returning"], json!(["s1"]));
    assert!(t["command_tools"].is_array() && t["input_tools"].is_array());
}

#[test]
fn no_subcommand_sets_the_flag() {
    let fx = Fx::new();
    let o = fx.refused(&["absent", "set"]);
    assert!(o.all().contains("bf absent status"));
    assert!(!on(&fx));
}
