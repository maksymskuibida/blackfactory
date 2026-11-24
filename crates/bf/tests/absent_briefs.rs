//! The engine's instruction in the briefs while the run's human-absent flag is on, and the flag in
//! `bf next` and `bf status` (U-71, R-74, R-72, D-97).

mod common;
use common::*;
use serde_json::json;
use std::fs;

const CMD: &str = "/factory:human-absent";

fn say(fx: &Fx, session: &str, text: &str) {
    let payload = json!({
        "session_id": session,
        "cwd": fx.root.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "UserPromptSubmit",
        "prompt": text,
    });
    let o = fx.bf_stdin(&fx.root, &["human-turn"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

fn brief(fx: &Fx) -> String {
    fs::read_to_string(fx.project().briefs_dir().join("U-1-builder.md")).unwrap()
}

#[test]
fn a_brief_written_while_the_flag_is_on_begins_with_the_block() {
    let fx = Fx::tiny_run();
    say(&fx, "s1", CMD);
    fx.must(&["unit", "start", "U-1"]);
    let b = brief(&fx);
    assert!(b.starts_with("<!-- bf:human-absent:begin -->"), "{b}");
    assert!(b.contains("bf ask"), "{b}");
    // A second stamp adds no second block.
    fx.must(&["unit", "start", "U-1"]);
    assert_eq!(brief(&fx).matches("bf:human-absent:begin").count(), 1);
}

#[test]
fn a_brief_written_with_the_flag_off_has_no_block_and_the_release_restores_it() {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    let plain = brief(&fx);
    assert!(!plain.contains("human-absent"));
    say(&fx, "s1", CMD);
    // A brief written before the set is not stamped by a later command.
    fx.must(&["status"]);
    assert_eq!(brief(&fx), plain);
    fx.must(&["unit", "start", "U-1"]);
    assert!(brief(&fx).starts_with("<!-- bf:human-absent:begin -->"));
    say(&fx, "s1", "hello");
    say(&fx, "s1", "yes");
    assert_eq!(brief(&fx), plain, "byte for byte as it was written");
}

#[test]
fn next_and_status_show_the_flag() {
    let fx = Fx::tiny_run();
    say(&fx, "s1", CMD);
    fx.must(&["ask", "--unit", "U-2", "--title", "Which greeting?"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["human_absent"]["session"], "s1");
    assert!(n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["role"] == "human"));
    assert!(n["human_absent"]["instruction"]
        .as_str()
        .unwrap()
        .contains("bf ask"));
    assert!(n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["action"] == "start-unit"));
    for d in n["directives"].as_array().unwrap() {
        if d["role"] == "human" {
            assert_eq!(d["waits"], true);
        }
    }
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["absent"]["on"], true);
    let text = fx.must(&["status"]).all();
    assert!(text.contains("human absent"), "{text}");
}

#[test]
fn only_a_checkpoint_step_left_names_where_the_run_waits() {
    let fx = Fx::tiny_project();
    let f = fx.root.join("factory/records/K-1.toml");
    let t = fs::read_to_string(&f).unwrap();
    fs::write(
        &f,
        format!("{t}human_step = \"Read both messages\"\non_human_step = \"stop\"\n"),
    )
    .unwrap();
    fx.commit_all("a checkpoint with a human step");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx.must_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "run",
            "X-3",
            "--",
            "ls",
            "src/hello.txt",
            "src/bye.txt",
        ],
    );
    say(&fx, "s1", CMD);
    let n = fx.must(&["next", "--json"]).json();
    let human: Vec<_> = n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["role"] == "human")
        .collect();
    assert!(!human.is_empty());
    for d in &human {
        assert_eq!(d["waits"], true);
    }
    let at = n["human_absent"]["waits_at"].as_str().unwrap_or("");
    assert!(at.contains("K-1"), "{at}");
    assert!(n["idle_reason"]
        .as_str()
        .unwrap()
        .contains("only steps of the human"));
    let text = fx.must(&["next"]).all();
    assert!(text.contains("only steps of the human"), "{text}");
}

#[test]
fn a_brief_that_embeds_a_stamped_brief_is_stamped_at_its_top() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("K-1-reviewer.md");
    let body = format!(
        "# Brief: K-1 as reviewer\n\n## The brief of every unit\n\n{}# Brief: U-1 as builder\n",
        bf::absent::brief_block()
    );
    fs::write(&path, &body).unwrap();
    bf::absent::stamp_briefs(std::slice::from_ref(&path)).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with(&bf::strings::t("absent_briefs.begin", &[])),
        "{text}"
    );
    // A second stamp leaves a brief that starts with a block alone.
    bf::absent::stamp_briefs(std::slice::from_ref(&path)).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}
