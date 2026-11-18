//! U-5: withdrawing an amendment puts the records it changed back to the content last approved and
//! takes out a record it added (R-44); a refusal for a draft of the plan never waits for an approval
//! that cannot come (R-45).

mod common;
use common::Fx;
use serde_json::Value;
use std::fs;

fn put_file(fx: &Fx, name: &str, text: &str, replace: bool) {
    let f = fx.root.join(name);
    fs::write(&f, text).unwrap();
    let mut a = vec!["record", "put", "check", "--file", f.to_str().unwrap()];
    if replace {
        a.push("--replace");
    }
    fx.must(&a);
}

fn check_text(id: &str, command: &str, unit: &str) -> String {
    format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"Check {id}\"\nhow=\"command\"\ncommand=\"{command}\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"{unit}\"\n")
}

fn get(fx: &Fx, id: &str) -> Value {
    fx.must(&["record", "get", id, "--json"]).json()
}

#[test]
fn amend_withdraw_puts_two_changed_records_back_and_takes_out_the_added_one() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let (x1, x2) = (get(&fx, "X-1"), get(&fx, "X-2"));
    fx.must(&["amend", "open", "--repair", "X-1", "X-2", "X-4"]);
    put_file(
        &fx,
        "a.toml",
        &check_text("X-1", "test -s src/hello.txt", "U-1"),
        true,
    );
    put_file(
        &fx,
        "b.toml",
        &check_text("X-2", "test -s src/bye.txt", "U-2"),
        true,
    );
    put_file(&fx, "c.toml", &check_text("X-4", "true", "U-1"), false);
    assert_ne!(get(&fx, "X-1"), x1);
    let o = fx.must(&["amend", "withdraw", "A-1", "--reason", "wrong commands"]);
    for id in ["X-1", "X-2", "X-4"] {
        assert!(o.stdout.contains(id), "{}", o.stdout);
    }
    // The records equal their approved content again; the added one is out of the records in force.
    assert_eq!(get(&fx, "X-1"), x1);
    assert_eq!(get(&fx, "X-2"), x2);
    assert!(!fx.root.join("factory/records/X-4.toml").exists());
    // The log keeps that the amendment existed and was withdrawn.
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["amendments"][0]["status"], "withdrawn", "{st}");
}

#[test]
fn amend_withdraw_of_an_amendment_that_changed_nothing_says_so() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    let o = fx.must(&["amend", "withdraw", "A-1", "--reason", "changed my mind"]);
    assert!(o.stdout.contains("A-1"), "{}", o.stdout);
}

/// The refusal of a check run on a draft with no amendment open: it names what can be done and never a
/// step that waits for an approval.
#[test]
fn amend_withdraw_draft_refusal_without_an_amendment_waits_for_nothing() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    // A change by hand after an amendment named the record: no amendment is open, none can approve it.
    fx.must(&["amend", "open", "--repair", "X-1"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "not needed"]);
    let f = fx.root.join("factory/records/X-1.toml");
    let text = fs::read_to_string(&f).unwrap();
    fs::write(&f, text.replace("test -f", "test -s")).unwrap();
    let o = fx.refused(&[
        "evidence",
        "run",
        "X-1",
        "--unit",
        "U-1",
        "--",
        "test",
        "-s",
        "src/hello.txt",
    ]);
    let lower = o.stderr.to_lowercase();
    assert!(!lower.contains("wait"), "{}", o.stderr);
    assert!(lower.contains("no amendment"), "{}", o.stderr);
    assert!(o.stderr.contains("bf amend open X-1"), "{}", o.stderr);
    // The lines of the evidence list say the same: no approval is coming, and the steps that work.
    let v = fx.bf(&["evidence", "verify", "--unit", "U-1"]);
    let all = v.all();
    assert!(all.contains("X-1"), "{all}");
    assert!(!all.contains("has not approved"), "{all}");
    assert!(all.contains("bf amend open X-1"), "{all}");
}

/// With another amendment open, the step to open one is not given (it is refused): the step is to name
/// the record in the open one.
#[test]
fn amend_withdraw_draft_refusal_with_another_amendment_open_names_revise() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "not needed"]);
    let f = fx.root.join("factory/records/X-1.toml");
    let text = fs::read_to_string(&f).unwrap();
    fs::write(&f, text.replace("test -f", "test -s")).unwrap();
    fx.must(&["amend", "open", "--repair", "X-2"]);
    let o = fx.refused(&[
        "evidence",
        "run",
        "X-1",
        "--unit",
        "U-1",
        "--",
        "test",
        "-s",
        "src/hello.txt",
    ]);
    assert!(o.stderr.contains("A-2"), "{}", o.stderr);
    assert!(o.stderr.contains("bf amend revise"), "{}", o.stderr);
    assert!(!o.stderr.contains("bf amend open X-1"), "{}", o.stderr);
    let v = fx.bf(&["evidence", "verify", "--unit", "U-1"]).all();
    assert!(v.contains("bf amend revise"), "{v}");
    assert!(!v.contains("has not approved"), "{v}");
}

/// R-44: a withdrawal before any narrowing takes out the added record, and the log keeps its content;
/// `bf log` leaves that content out of the event's line.
#[test]
fn amend_withdraw_before_narrowing_keeps_the_removed_content_in_the_log() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["amend", "open", "--repair", "X-1", "X-4"]);
    put_file(
        &fx,
        "c.toml",
        &check_text("X-4", "true", "U-1").replace("Check X-4", "ZebraAddedTitle"),
        false,
    );
    fx.must(&["amend", "withdraw", "A-1", "--reason", "not needed"]);
    assert!(!fx.root.join("factory/records/X-4.toml").exists());
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    assert!(log.contains("ZebraAddedTitle"), "the log lost the content");
    let o = fx.must(&["log", "--tail", "3"]);
    let line = o
        .stdout
        .lines()
        .find(|l| l.contains("\"amend.withdraw\""))
        .expect("withdraw event");
    assert!(!line.contains("ZebraAddedTitle"), "{line}");
    assert!(line.contains("X-4") && line.contains("[contents left out]"), "{line}");
    let n = serde_json::from_str::<Value>(line.split(" [contents").next().unwrap()).unwrap()["seq"]
        .as_u64()
        .unwrap();
    let w = fx.must(&["log", "--whole", &n.to_string()]);
    assert!(w.stdout.contains("ZebraAddedTitle"), "{}", w.stdout);
    fx.must(&["log", "--verify"]);
}

/// With an amendment open the same refusal names it.
#[test]
fn amend_withdraw_draft_refusal_with_an_amendment_names_it() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    put_file(
        &fx,
        "a.toml",
        &check_text("X-1", "test -s src/hello.txt", "U-1"),
        true,
    );
    let o = fx.refused(&[
        "evidence",
        "run",
        "X-1",
        "--unit",
        "U-1",
        "--",
        "test",
        "-s",
        "src/hello.txt",
    ]);
    assert!(o.stderr.contains("A-1"), "{}", o.stderr);
}

/// R-45: the checkpoint gap of a draft checkpoint with no amendment open, in the note of `bf next`,
/// names the steps that work and never says the human has not approved.
#[test]
fn amend_withdraw_checkpoint_gap_note_names_steps_without_an_approval_to_wait_for() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-3", "--", "ls", "src/hello.txt", "src/bye.txt"],
    );
    fx.must(&["amend", "open", "--repair", "K-1"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "not needed"]);
    let f = fx.root.join("factory/records/K-1.toml");
    let text = fs::read_to_string(&f).unwrap();
    fs::write(&f, text.replace("Both messages", "Both messages, edited")).unwrap();
    let n = fx.must(&["next", "--json"]).json();
    let d = n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == "fix-checkpoint-gaps")
        .unwrap_or_else(|| panic!("{n}"));
    let note = d["note"].as_str().unwrap();
    assert!(note.contains("K-1"), "{note}");
    assert!(!note.contains("has not approved"), "{note}");
    assert!(note.contains("bf amend open K-1"), "{note}");
    let plain = fx.must(&["next"]).all();
    assert!(!plain.contains("has not approved"), "{plain}");
}
