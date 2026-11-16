//! A message that names a required model on its first line (R-75, D-52, TH-31).

mod common;
use common::*;
use serde_json::json;

const SID: &str = "0b6f1c3e-aaaa-bbbb-cccc-1234567890ab";

fn status(fx: &Fx, model: serde_json::Value) {
    let mut input = json!({"session_id": SID, "context_window": {"used_percentage": 10.0}});
    input["model"] = model;
    fx.bf_stdin(&fx.root, &["progress"], &input.to_string());
}

fn on(fx: &Fx, id: &str, name: &str, effort: &str) {
    let mut input = json!({
        "session_id": SID,
        "model": {"id": id, "display_name": name},
        "context_window": {"used_percentage": 10.0}
    });
    if !effort.is_empty() {
        input["effort"] = json!({"level": effort});
    }
    fx.bf_stdin(&fx.root, &["progress"], &input.to_string());
}

fn blocked(o: &Out) -> Option<serde_json::Value> {
    let j: serde_json::Value = serde_json::from_str(o.stdout.trim()).ok()?;
    (j["decision"] == "block").then_some(j)
}

#[test]
fn required_model_matching_session_goes_through() {
    let fx = Fx::new();
    on(&fx, "claude-opus-5-5", "Opus 5.5", "high");
    let o = fx.human("Required model: Opus 5.5 High\nstart the design");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.stdout, "", "{}", o.stdout);
}

#[test]
fn required_model_wrong_model_is_refused_naming_the_model_and_returning_the_text() {
    let fx = Fx::new();
    on(&fx, "claude-sonnet-5-5", "Sonnet 5.5", "medium");
    let o = fx.human("Required model: Opus 5.5 High\nstart the design");
    assert_eq!(o.code, 0, "{}", o.all());
    let j = blocked(&o).expect(&o.stdout);
    let r = j["reason"].as_str().unwrap();
    println!("{r}");
    assert!(r.contains("Opus 5.5 high"), "{r}");
    assert!(r.contains("Sonnet 5.5 medium"), "{r}");
    assert!(r.contains("start the design"), "{r}");
}

#[test]
fn required_model_wrong_effort_is_refused() {
    let fx = Fx::new();
    on(&fx, "claude-opus-5-5", "Opus 5.5", "low");
    let o = fx.human("Required model: Opus 5.5 High\nwork");
    let j = blocked(&o).expect(&o.stdout);
    assert!(j["reason"].as_str().unwrap().contains("Opus 5.5 high"));
}

#[test]
fn required_model_not_recorded_when_refused() {
    let fx = Fx::new();
    on(&fx, "claude-sonnet-5-5", "Sonnet 5.5", "low");
    let before = std::fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    fx.human("Required model: Opus 5.5 High\nwork");
    let after = std::fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn required_model_unreadable_session_refuses_and_says_so() {
    let fx = Fx::new();
    let o = fx.human("Required model: Opus 5.5 High\nwork");
    let j = blocked(&o).expect(&o.stdout);
    let r = j["reason"].as_str().unwrap();
    assert!(r.contains("could not read"), "{r}");
    assert!(r.contains("work"), "{r}");
    // A model but no effort readable.
    on(&fx, "claude-opus-5-5", "Opus 5.5", "");
    let o = fx.human("Required model: Opus 5.5 High\nwork");
    let r = blocked(&o).expect(&o.stdout)["reason"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(r.contains("could not read the effort"), "{r}");
    // No model at all.
    status(&fx, json!({}));
    let o = fx.human("Required model: Opus 5.5\nwork");
    assert!(blocked(&o).is_some(), "{}", o.stdout);
}

#[test]
fn required_model_without_effort_checks_only_the_model() {
    let fx = Fx::new();
    on(&fx, "claude-opus-5-5", "Opus 5.5", "low");
    assert_eq!(fx.human("Required model: Opus 5.5\nwork").stdout, "");
    assert!(blocked(&fx.human("Required model: Opus 4.1\nwork")).is_some());
}

#[test]
fn required_model_must_name_the_whole_model() {
    let fx = Fx::new();
    on(&fx, "claude-opus-5-5", "Opus 5.5", "high");
    assert!(blocked(&fx.human("Required model: Opus 5\nwork")).is_some());
    assert!(blocked(&fx.human("Required model: Opus\nwork")).is_some());
    assert!(blocked(&fx.human("Required model: 5.5\nwork")).is_some());
    assert_eq!(fx.human("Required model: opus 5.5\nwork").stdout, "");
    assert_eq!(
        fx.human("Required model: claude-opus-5-5 high\nwork")
            .stdout,
        ""
    );
    on(&fx, "claude-opus-5-5-20260101", "", "high");
    assert_eq!(fx.human("Required model: Opus 5.5\nwork").stdout, "");
    on(&fx, "claude-opus-5", "Opus 5", "high");
    assert!(blocked(&fx.human("Required model: Opus 5.5\nwork")).is_some());
}

#[test]
fn required_model_message_without_the_line_is_not_checked() {
    let fx = Fx::new();
    on(&fx, "claude-sonnet-5-5", "Sonnet 5.5", "low");
    assert_eq!(fx.human("hello\nRequired model: Opus 5.5 High").stdout, "");
    assert_eq!(fx.human("hello").stdout, "");
}

#[test]
fn required_model_line_is_read_only_on_the_first_line() {
    let fx = Fx::new();
    on(&fx, "claude-sonnet-5-5", "Sonnet 5.5", "low");
    assert_eq!(fx.human("note: Required model: Opus 5.5").stdout, "");
}
