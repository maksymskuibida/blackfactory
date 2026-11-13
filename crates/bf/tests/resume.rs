//! T-2: a run killed between steps leaves consistent state, and a landed unit is never redone.

mod common;
use common::*;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn unit_state(fx: &Fx, id: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .unwrap()["state"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn t2_killed_during_unit_start_resumes_without_a_second_start() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.bf_env(
        &fx.root,
        &["unit", "start", "U-1"],
        "",
        &[("BF_CRASH_AT", "unit-start-after-git")],
    );
    assert_eq!(o.code, 137, "{}", o.all());
    // Git has the branch and working copy, the log has no start: the unit is still just ready.
    assert!(fx.work("U-1").join(".git").exists());
    assert_eq!(unit_state(&fx, "U-1"), "ready");
    let o = fx.must(&["unit", "start", "U-1", "--json"]).json();
    assert_eq!(o["resumed"], false);
    assert_eq!(unit_state(&fx, "U-1"), "running");
    let log = fx.must(&["log", "--json"]).json();
    assert_eq!(
        log.as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "unit.start")
            .count(),
        1
    );
    // Starting again is a resume of the same working copy, not a new one.
    let o = fx.must(&["unit", "start", "U-1", "--json"]).json();
    assert_eq!(o["resumed"], true);
    fx.must(&["log", "--verify"]);
}

#[test]
fn t2_killed_after_the_merge_lands_exactly_once_and_is_not_redone() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hi\n").unwrap();
    fx.must_in(
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
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);

    let o = fx.bf_env(
        &fx.root,
        &["unit", "land", "U-1"],
        "",
        &[("BF_CRASH_AT", "land-after-merge")],
    );
    assert_eq!(o.code, 137, "{}", o.all());
    // The branch is merged, the log does not know yet.
    assert!(bf::gitx::is_ancestor(&fx.root, "bf/U-1", "bf/K-1"));
    assert_eq!(unit_state(&fx, "U-1"), "reviewed");
    // U-2 must not be offered yet: its dependency has not landed in the log.
    let next = fx.must(&["next", "--json"]).json();
    assert!(next["directives"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["unit"] != "U-2"));

    let o = fx.must(&["unit", "land", "U-1", "--json"]).json();
    assert_eq!(o["landed"], true);
    assert_eq!(unit_state(&fx, "U-1"), "landed");
    assert_eq!(
        git(&fx.root, &["rev-list", "--merges", "--count", "bf/K-1"]),
        "1",
        "the unit must be merged exactly once"
    );

    // A landed unit is not redone: no directive, no restart, landing again is a no-op.
    let next = fx.must(&["next", "--json"]).json();
    let d = next["directives"].as_array().unwrap();
    assert!(d.iter().all(|x| x["unit"] != "U-1"), "{next}");
    assert!(d.iter().any(|x| x["unit"] == "U-2"), "{next}");
    let o = fx.refused(&["unit", "start", "U-1"]);
    assert!(o.stderr.contains("landed"), "{}", o.stderr);
    let o = fx.must(&["unit", "land", "U-1", "--json"]).json();
    assert_eq!(o["already"], true);
    assert_eq!(
        git(&fx.root, &["rev-list", "--merges", "--count", "bf/K-1"]),
        "1"
    );
}

#[test]
fn t2_a_torn_last_line_in_the_log_is_ignored_and_healed() {
    let fx = Fx::new();
    fx.human("first");
    let log = fx.root.join("factory/log/events.jsonl");
    let mut f = fs::OpenOptions::new().append(true).open(&log).unwrap();
    f.write_all(b"{\"seq\":99,\"kind\":\"human.tu").unwrap(); // killed mid-write: no newline
    drop(f);
    fx.must(&["log", "--verify"]);
    fx.human("second");
    let text = fs::read_to_string(&log).unwrap();
    assert!(!text.contains("human.tu\""), "{text}");
    assert_eq!(text.lines().count(), 3); // init, first, second
    fx.must(&["log", "--verify"]);
}

#[test]
fn t2_editing_the_log_is_detected() {
    let fx = Fx::new();
    fx.human("first");
    fx.human("second");
    fx.must(&["log", "--verify"]);
    let log = fx.root.join("factory/log/events.jsonl");
    let t = fs::read_to_string(&log).unwrap().replace("first", "FIRST");
    fs::write(&log, t).unwrap();
    let o = fx.bf(&["log", "--verify"]);
    assert_eq!(o.code, 2);
    let o = fx.refused(&["gate", "open", "idea"]);
    assert!(o.stderr.contains("hash chain"), "{}", o.stderr);
}
