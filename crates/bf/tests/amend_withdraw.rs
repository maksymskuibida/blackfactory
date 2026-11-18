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
