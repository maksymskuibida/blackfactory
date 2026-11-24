//! U-102 (A-73, R-108): the conductor pauses and resumes a unit, and the idle clock stops while a unit
//! is paused or waits for the human.

mod common;
use common::*;
use std::fs;
use std::time::Duration;

/// A project whose idle limit is zero, with a run and U-1 at work.
fn at_work() -> Fx {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("idle_minutes = 60", "idle_minutes = 0");
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx
}

/// Only `bf next` reads the real clock, so the one test that goes through it still waits.
fn wait_a_while() {
    std::thread::sleep(Duration::from_millis(1100));
}

/// The idle check judged two seconds from now, with the clock handed to the library.
fn later() -> u64 {
    bf::util::now_secs() + 2
}

fn stuck(fx: &Fx) -> serde_json::Value {
    bf::run::stuck_at(&fx.project(), true, later()).unwrap()
}

fn parked_now(fx: &Fx) -> usize {
    let v = stuck(fx);
    println!("stuck: {v}");
    v["parked_now"].as_array().unwrap().len()
}

fn state_of(fx: &Fx, id: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .map(|u| u["state"].as_str().unwrap().to_string())
        .unwrap_or_default()
}

#[test]
fn a_paused_unit_is_not_parked_and_gets_no_directive() {
    let fx = at_work();
    let o = fx.must(&["unit", "pause", "U-1", "--json"]).json();
    assert!(o["line"].as_str().unwrap().contains("bf unit resume U-1"));
    wait_a_while();
    assert_eq!(parked_now(&fx), 0);
    let n = fx.must(&["next", "--json"]).json();
    println!("next: {n}");
    let wait: Vec<String> = n["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        wait.iter()
            .any(|w| w.contains("U-1") && w.contains("paused") && w.contains("bf unit resume U-1")),
        "{n}"
    );
    assert!(
        !n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["unit"] == "U-1"),
        "{n}"
    );
    assert_eq!(state_of(&fx, "U-1"), "running");
}

#[test]
fn after_a_resume_the_unit_is_parked_again() {
    let fx = at_work();
    fx.must(&["unit", "pause", "U-1"]);
    assert_eq!(parked_now(&fx), 0);
    let o = fx.must(&["unit", "resume", "U-1", "--json"]).json();
    assert!(o["line"].as_str().unwrap().contains("U-1"));
    assert_eq!(parked_now(&fx), 1);
    bf::run::stuck_at(&fx.project(), false, later()).unwrap();
    assert_eq!(state_of(&fx, "U-1"), "parked");
}

#[test]
fn a_unit_with_an_open_question_is_not_parked() {
    let fx = at_work();
    fx.must(&["ask", "--unit", "U-1", "--title", "Which colour?"]);
    assert_eq!(parked_now(&fx), 0);
}

#[test]
fn a_unit_named_by_an_open_amendment_is_not_parked() {
    let fx = at_work();
    assert_eq!(parked_now(&fx), 1);
    fx.must(&["amend", "open", "U-1"]);
    assert_eq!(parked_now(&fx), 0);
}

#[test]
fn a_blocked_unit_and_a_second_pause_are_refused() {
    let fx = at_work();
    fx.must(&["report", "U-1", "--result", "blocked", "--note", "needs a thing"]);
    let o = fx.refused(&["unit", "pause", "U-1"]);
    assert_four_parts(&o.all());
    let fx = at_work();
    fx.must(&["unit", "pause", "U-1"]);
    let o = fx.refused(&["unit", "pause", "U-1"]);
    assert_four_parts(&o.all());
    assert!(o.all().contains("paused already"), "{}", o.all());
    fx.must(&["unit", "resume", "U-1"]);
    let o = fx.refused(&["unit", "resume", "U-1"]);
    assert_four_parts(&o.all());
}

#[test]
fn a_report_of_the_paused_unit_clears_the_pause() {
    let fx = at_work();
    fx.must(&["unit", "pause", "U-1"]);
    let wc = fx.work("U-1");
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "x\n").unwrap();
    fx.must(&["report", "U-1", "--result", "built"]);
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(!st.units["U-1"].paused);
    assert_eq!(state_of(&fx, "U-1"), "review");
    // A pause is taken again for a unit in review.
    fx.must(&["unit", "pause", "U-1"]);
}

#[test]
fn help_names_the_new_commands() {
    let fx = Fx::new();
    let h = fx.must(&["help"]).stdout;
    assert!(h.contains("unit start|land|release|pause|resume|sync"), "{h}");
    assert!(h.contains("unit scope|restore|format"), "{h}");
}

/// Append a `unit.start` dated long ago, hash-chained, and move the head (copied from a73_hooks).
fn start_long_ago(fx: &Fx, unit: &str) {
    use serde_json::{json, Map, Value};
    let p = fx.project();
    let key = p.read_key().unwrap();
    let log = p.log_path();
    let text = fs::read_to_string(&log).unwrap();
    let count = text.lines().count() + 1;
    let prev = serde_json::from_str::<Value>(text.lines().last().unwrap()).unwrap()["h"]
        .as_str()
        .unwrap()
        .to_string();
    let mut m = Map::new();
    m.insert("seq".into(), json!(count));
    m.insert("at".into(), json!("2026-01-01T00:00:00Z"));
    m.insert("kind".into(), json!("unit.start"));
    m.insert("prev".into(), json!(prev));
    m.insert("unit".into(), json!(unit));
    m.insert("base_commit".into(), json!("c1"));
    let h = bf::events::event_hash(&key, &prev, &m);
    m.insert("h".into(), json!(h.clone()));
    let mut out = text;
    out.push_str(&serde_json::to_string(&Value::Object(m)).unwrap());
    out.push('\n');
    fs::write(&log, out).unwrap();
    let mut head = bf::events::read_head(&p).unwrap_or_default();
    head.count = count as u64;
    head.last_hash = h;
    bf::events::write_head(&p, &head).unwrap();
}

#[test]
fn the_answer_to_a_question_restarts_the_clock() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    start_long_ago(&fx, "U-1");
    // The idle limit is the policy's 60 minutes and the unit has been silent far longer.
    assert_eq!(parked_now(&fx), 1);
    let dir = tempfile::tempdir().unwrap();
    let qf = dir.path().join("q.json");
    fs::write(
        &qf,
        serde_json::json!({"title": "Which colour?", "kind": "single",
            "options": [{"key": "A", "text": "Blue", "proposed": true, "why": "Calm."}]})
        .to_string(),
    )
    .unwrap();
    let o = fx
        .must(&["ask", "--unit", "U-1", "--file", qf.to_str().unwrap(), "--json"])
        .json();
    let q = o[0]["id"].as_str().unwrap().to_string();
    assert_eq!(parked_now(&fx), 0);
    fx.human("Blue, please.");
    let a = dir.path().join("a.json");
    fs::write(
        &a,
        serde_json::json!({"answer": {"keys": ["A"]}}).to_string(),
    )
    .unwrap();
    fx.must(&["answer", &q, "--file", a.to_str().unwrap()]);
    // The question is answered, and the clock started at the answer: nothing is parked.
    assert_eq!(parked_now(&fx), 0);
}
