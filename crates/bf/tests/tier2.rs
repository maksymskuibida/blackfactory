//! Tier 2 commands: questions, cost, stuck, doctor, review rounds and parking.

mod common;
use common::*;
use serde_json::json;
use std::fs;

#[test]
fn questions_come_before_their_gate() {
    let fx = Fx::new();
    let dir = tempfile::tempdir().unwrap();
    let q = dir.path().join("q.json");
    fs::write(&q, json!({"title": "Where do records live?", "context": "Short.", "kind": "single", "stage": "architecture",
        "options": [{"key": "A", "text": "In the repository", "proposed": true, "why": "Versioned with code."}]}).to_string()).unwrap();
    let o = fx
        .must(&["ask", "--file", q.to_str().unwrap(), "--json"])
        .json();
    assert_eq!(o[0]["id"], "Q-1");
    assert_eq!(
        fx.must(&["questions", "--open", "--json"])
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The stage's gate will not open with a question unanswered.
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(o.stderr.contains("Q-1"), "{}", o.stderr);
    // The question page is a view of its own.
    let v = fx.must(&["view", "questions", "architecture"]).json();
    assert_eq!(v["questions"][0]["id"], "Q-1");
    assert_eq!(v["answer"]["how"], "form");
    let a = dir.path().join("a.json");
    fs::write(&a, json!({"answer": {"keys": ["A"]}}).to_string()).unwrap();
    fx.human("done");
    fx.must(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    assert_eq!(
        fx.must(&["questions", "--open", "--json"])
            .json()
            .as_array()
            .unwrap()
            .len(),
        0
    );
    fx.must(&["gate", "open", "architecture"]);
    let v = fx.must(&["view", "questions"]).json();
    assert_eq!(v["questions"][0]["answer"]["keys"][0], "A");
}

#[test]
fn review_rounds_end_in_parking_and_stuck_lists_it() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "x\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("f.json");
    fs::write(&f, json!([{"title": "Wrong greeting", "severity": "high", "where": "src/hello.txt:1", "action": "Say hello"}]).to_string()).unwrap();
    // Findings cannot be reported without a list, and only by the reviewer on a built unit.
    fx.refused(&[
        "report", "U-1", "--result", "findings", "--role", "reviewer",
    ]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.refused(&["report", "U-1", "--result", "built"]); // already in review
    for round in 1..=3 {
        let o = fx
            .must(&[
                "report",
                "U-1",
                "--result",
                "findings",
                "--role",
                "reviewer",
                "--findings",
                f.to_str().unwrap(),
                "--json",
            ])
            .json();
        assert_eq!(o["state"], "fixing", "round {round}");
        // A unit with open findings cannot be reported clean or landed.
        fx.refused(&["unit", "land", "U-1"]);
        fx.must(&["report", "U-1", "--result", "fixed"]);
    }
    let o = fx
        .must(&[
            "report",
            "U-1",
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--findings",
            f.to_str().unwrap(),
            "--json",
        ])
        .json();
    assert_eq!(o["state"], "parked", "{o}");
    let s = fx.must(&["stuck", "--json"]).json();
    assert_eq!(s["already_parked"][0]["unit"], "U-1");
    let n = fx.must(&["next", "--json"]).json();
    assert!(n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["role"] == "human" && d["unit"] == "U-1"));
    // The findings are records with stable IDs.
    let l = fx
        .must(&["record", "list", "--kind", "finding", "--json"])
        .json();
    assert_eq!(l.as_array().unwrap().len(), 4);
    assert_eq!(l[0]["id"], "F-1");
    // A way back: only the human releases a parked unit, in their words.
    fx.refused(&["unit", "release", "U-1", "--quote", "Yes, release it."]);
    fx.human("Yes, release it and let the builder try again.");
    let r = fx
        .must(&[
            "unit",
            "release",
            "U-1",
            "--quote",
            "Yes, release it",
            "--json",
        ])
        .json();
    assert_eq!(r["state"], "fixing");
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["role"], "builder", "{n}");
    assert_eq!(n["directives"][0]["action"], "fix");
    assert_eq!(n["directives"][0]["limits"]["round"], 0);
    // The history stays in the log, and the same words cannot release it twice.
    let log = fx.must(&["log", "--json"]).json().to_string();
    assert!(log.contains("unit.release") && log.contains("unit.park"));
    fx.refused(&["unit", "release", "U-1", "--quote", "Yes, release it"]);
    // A unit that is not parked or blocked is not released.
    fx.human("Yes, release it again.");
    let o = fx.refused(&[
        "unit",
        "release",
        "U-1",
        "--quote",
        "Yes, release it again.",
    ]);
    assert!(
        o.stderr.contains("only a parked or blocked unit"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_builder_that_keeps_failing_its_check_is_parked_and_a_blocked_unit_can_be_released() {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("max_attempts = 10", "max_attempts = 3");
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    for i in 1..=3 {
        let o = fx.bf_in(
            &wc,
            &[
                "evidence",
                "run",
                "X-1",
                "--unit",
                "U-1",
                "--",
                "test",
                "-f",
                "src/hello.txt",
            ],
        );
        assert_eq!(o.code, 1, "try {i}: {}", o.all());
    }
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["units"][0]["state"], "parked", "{st}");
    let n = fx.must(&["next", "--json"]).json();
    assert!(n["idle_reason"].as_str().unwrap().contains("parked"), "{n}");
    fx.human("Go on, release it.");
    fx.must(&["unit", "release", "U-1", "--quote", "release it"]);
    assert_eq!(
        fx.must(&["status", "--json"]).json()["units"][0]["state"],
        "running"
    );
    // A blocked report counts as an attempt, and a blocked unit is released the same way.
    fx.must(&[
        "report",
        "U-1",
        "--result",
        "blocked",
        "--note",
        "need a decision",
    ]);
    assert_eq!(
        fx.must(&["status", "--json"]).json()["units"][0]["state"],
        "blocked"
    );
    fx.human("It is fine, release it now.");
    fx.must(&["unit", "release", "U-1", "--quote", "release it now"]);
    assert_eq!(
        fx.must(&["status", "--json"]).json()["units"][0]["state"],
        "running"
    );
}

#[test]
fn bf_next_parks_an_idle_unit_by_itself() {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("idle_minutes = 60", "idle_minutes = 0");
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    // Nobody calls `bf stuck`: `bf next` runs the idle check.
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["role"] == "human" && d["unit"] == "U-1"),
        "{n}"
    );
}

#[test]
fn a_reviewer_cannot_pass_a_unit_with_open_findings() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("f.json");
    fs::write(&f, json!([{"title": "t"}]).to_string()).unwrap();
    fx.must(&[
        "report",
        "U-1",
        "--result",
        "findings",
        "--role",
        "reviewer",
        "--findings",
        f.to_str().unwrap(),
    ]);
    fx.must(&["report", "U-1", "--result", "fixed"]);
    // The fix is back in review; findings are marked fixed, so a clean review is accepted.
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["units"][0]["state"], "reviewed");
}

#[test]
fn the_budget_cap_stops_new_work() {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("cap = 0.0", "cap = 1.5");
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "built", "--cost", "0.9"]);
    fx.must(&["next"]);
    fx.must(&[
        "report", "U-1", "--result", "built", "--role", "reviewer", "--cost", "0.7",
    ]);
    let o = fx.refused(&["next"]);
    assert!(o.stderr.contains("budget cap"), "{}", o.stderr);
    fx.refused(&["unit", "start", "U-2"]);
    let c = fx.must(&["cost", "status", "--json"]).json();
    assert_eq!(c["cap"], 1.5);
    assert!((c["spent"].as_f64().unwrap() - 1.6).abs() < 1e-9);
}

#[test]
fn doctor_and_perf_run() {
    let fx = Fx::new();
    fx.tiny();
    let d = fx.must(&["doctor", "--json"]).json();
    assert_eq!(d["ok"], true, "{d}");
    let p = fx.must(&["doctor", "--perf", "--json"]).json();
    let rows = p["rows"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["command"] == "guard (child process)"));
    assert!(rows.iter().all(|r| r["median_ms"].is_number()));
}

#[test]
fn the_plan_check_names_every_kind_of_problem() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    fx.put("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"a\"\ndelivers=[\"R-1\"]\nfiles=[\"src/\"]\ndepends_on=[\"U-2\"]\ncheckpoint=\"K-1\"\n");
    fx.put("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"b\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ndepends_on=[\"U-1\"]\ncheckpoint=\"K-1\"\n");
    fx.put("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"c\"\ndelivers=[\"R-1\"]\nfiles=[\"src/b.rs\"]\ncheckpoint=\"K-1\"\n");
    fx.put("unit", "id=\"U-4\"\nkind=\"unit\"\ntitle=\"d\"\ndelivers=[\"R-1\"]\nfiles=[\"src/b.rs\"]\ncheckpoint=\"K-1\"\n");
    fx.put(
        "handoff",
        "id=\"H-1\"\nkind=\"handoff\"\nfrom=\"U-3\"\nto=\"U-9\"\nthing=\"x\"\n",
    );
    let o = fx.refused(&["plan", "check"]);
    for want in [
        "cycle",
        "both touch",
        "no receiver",
        "has no record",
        "has no check",
    ] {
        assert!(
            o.stderr.contains(want),
            "missing `{want}` in:\n{}",
            o.stderr
        );
    }
}
