//! U-26 (R-57, D-36): the evidence run command itself refuses the same command with the same parameters on
//! the same content, unless the repeat is stated with its attempt number and a reason.

mod common;
use common::*;
use std::fs;

fn setup() -> (Fx, std::path::PathBuf) {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n"),
    ] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    fx.commit_all("records");
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "one\n").unwrap();
    (fx, wc)
}

const RUN: [&str; 8] = [
    "evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f",
];

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut v: Vec<&str> = RUN[..5].to_vec();
    v.splice(3..3, extra.iter().copied());
    v.extend(["--", "test", "-f", "src/a.txt"]);
    v
}

#[test]
fn a_repeat_on_the_same_content_is_refused_and_says_how_to_repeat_on_purpose() {
    let (fx, wc) = setup();
    fx.must_in(&wc, &args(&[]));
    let o = fx.bf_in(&wc, &args(&[]));
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("already ran 1 time"), "{}", o.stderr);
    assert!(
        o.stderr.contains("--attempt 2") && o.stderr.contains("--reason"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_deliberate_repeat_states_the_next_attempt_and_a_reason_and_both_are_kept() {
    let (fx, wc) = setup();
    fx.must_in(&wc, &args(&[]));
    // The wrong number, and no reason.
    let o = fx.bf_in(&wc, &args(&["--attempt", "3", "--reason", "again"]));
    assert!(o.stderr.contains("next attempt is 2"), "{}", o.stderr);
    let o = fx.bf_in(&wc, &args(&["--attempt", "2"]));
    assert!(o.stderr.contains("needs a reason"), "{}", o.stderr);
    let o = fx.must_in(
        &wc,
        &args(&[
            "--attempt",
            "2",
            "--reason",
            "the machine was busy",
            "--json",
        ]),
    );
    let v = o.json();
    assert_eq!(v["attempt"], 2, "{v}");
    assert_eq!(v["reason"], "the machine was busy", "{v}");
    // The third run is attempt 3, not 2.
    let o = fx.bf_in(&wc, &args(&["--attempt", "2", "--reason", "again"]));
    assert!(o.stderr.contains("next attempt is 3"), "{}", o.stderr);
    // The seal covers the attempt: a receipt edited by hand is not valid any more.
    let p = fx.project();
    let rs = bf::evidence::load_receipts(&p).unwrap();
    assert!(rs.iter().all(|r| r.seal_ok));
    let id = v["id"].as_str().unwrap();
    let path = p.receipts_dir().join(format!("{id}.toml"));
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("attempt = 2", "attempt = 9")).unwrap();
    let rs = bf::evidence::load_receipts(&p).unwrap();
    assert!(rs.iter().any(|r| r.id() == id && !r.seal_ok));
}

fn rcpt(id: &str, ty: &str, exit: i64, judgement: &str) -> bf::evidence::Receipt {
    bf::evidence::Receipt {
        v: serde_json::json!({"id": id, "type": ty, "check": "X-1", "content_hash": "c1", "exit": exit, "judgement": judgement}),
        seal_ok: true,
        stale: false,
        how: "command".into(),
    }
}

#[test]
fn the_suggested_repeat_keeps_the_unit_of_the_run() {
    let (fx, wc) = setup();
    fx.must_in(&wc, &args(&[]));
    let o = fx.bf_in(&wc, &args(&[]));
    assert!(o.stderr.contains("--unit U-1 --attempt 2"), "{}", o.stderr);
}

#[test]
fn a_unit_check_that_failed_and_then_passed_at_the_same_content_counts() {
    let rs = vec![
        rcpt("rc-00001", "run", 1, ""),
        rcpt("rc-00002", "run", 0, ""),
    ];
    assert!(bf::evidence::satisfied_unit_at(&rs, "X-1", "c1").is_some());
    // The checkpoint's rule still waits for the reviewer's judgement.
    assert!(bf::evidence::satisfied_at(&rs, "X-1", "c1").is_none());
}

#[test]
fn verify_of_a_unit_counts_a_check_that_failed_then_passed_on_a_repeat() {
    let (fx, wc) = setup();
    // A command whose outcome depends on something outside the content: it fails, then passes.
    let marker = std::env::temp_dir().join(format!("bf-flaky-{}", std::process::id()));
    let _ = fs::remove_file(&marker);
    // The checked file is a link to the marker, so the content stays the same while the outcome changes.
    fs::remove_file(wc.join("src/a.txt")).unwrap();
    std::os::unix::fs::symlink(&marker, wc.join("src/a.txt")).unwrap();
    let run = |extra: &[&str]| fx.bf_in(&wc, &args(extra));
    assert_ne!(run(&[]).code, 0);
    fs::write(&marker, "x").unwrap();
    let o = run(&["--attempt", "2", "--reason", "the marker was late"]);
    let _ = fs::remove_file(&marker);
    assert_eq!(o.code, 0, "{}", o.all());
    let v = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    assert_eq!(v.code, 0, "{}", v.all());
}

#[test]
fn a_judgement_must_come_after_the_latest_failure_before_the_pass() {
    let mut rs = vec![
        rcpt("rc-00001", "run", 1, ""),
        rcpt("rc-00002", "run", 0, ""),
        rcpt("rc-00003", "judge", 0, "flaky"),
        rcpt("rc-00004", "run", 1, ""),
        rcpt("rc-00005", "run", 0, ""),
    ];
    assert!(bf::evidence::satisfied_at(&rs, "X-1", "c1").is_none());
    rs.push(rcpt("rc-00006", "judge", 0, "other"));
    assert!(bf::evidence::satisfied_at(&rs, "X-1", "c1").is_some());
}

#[test]
fn the_page_row_shows_a_repeats_attempt_and_reason() {
    let mut r = rcpt("rc-00002", "run", 0, "");
    r.v["attempt"] = serde_json::json!(2);
    r.v["reason"] = serde_json::json!("the machine was busy");
    let row = bf::view::evidence_row(&r);
    let reason = row["reason"].as_str().unwrap();
    assert!(
        reason.contains("attempt 2") && reason.contains("machine was busy"),
        "{row}"
    );
    let plain = bf::view::evidence_row(&rcpt("rc-00001", "run", 0, ""));
    assert!(plain.get("reason").is_none(), "{plain}");
}

#[test]
fn a_run_on_changed_content_is_not_a_repeat() {
    let (fx, wc) = setup();
    fx.must_in(&wc, &args(&[]));
    fs::write(wc.join("src/a.txt"), "two\n").unwrap();
    fx.must_in(&wc, &args(&[]));
}
