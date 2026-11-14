//! U-33: accepting a legal record by the human leaves a trace in the log: the record, the human's
//! message and the words taken from it. The message is used up, and only a message written after the
//! record's latest change counts.

mod common;
use common::Fx;
use std::fs;
use std::thread::sleep;
use std::time::Duration;

fn legal(id: &str, title: &str, outcome: &str) -> String {
    format!(
        "id=\"{id}\"\nkind=\"legal\"\ntitle=\"{title}\"\nseverity=\"low\"\noutcome=\"{outcome}\"\n"
    )
}

/// Put a legal record from the main checkout, replacing what is there, with the human's words.
fn put_legal(fx: &Fx, text: &str, quote: Option<&str>) -> common::Out {
    let f = fx.root.join("legal-put.toml");
    fs::write(&f, text).unwrap();
    let mut a = vec![
        "record",
        "put",
        "legal",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ];
    if let Some(q) = quote {
        a.extend(["--quote", q]);
    }
    fx.bf(&a)
}

#[test]
fn accepting_a_legal_record_appends_an_event_with_the_record_the_message_and_the_quote() {
    let fx = Fx::new();
    assert_eq!(
        put_legal(&fx, &legal("L-1", "Cookie notice", "open"), None).code,
        0
    );
    sleep(Duration::from_millis(1100));
    fx.human("Fine, I accept the cookie notice risk as it is.");
    let o = put_legal(
        &fx,
        &legal("L-1", "Cookie notice", "accepted"),
        Some("I accept the cookie notice risk"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    let ev: Vec<serde_json::Value> = log
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|e: &serde_json::Value| e["kind"] == "legal.accept")
        .collect();
    assert_eq!(ev.len(), 1, "{log}");
    assert_eq!(ev[0]["record"], "L-1");
    assert_eq!(ev[0]["quote"], "I accept the cookie notice risk");
    assert_eq!(
        ev[0]["message"],
        "Fine, I accept the cookie notice risk as it is."
    );
    let st = bf::events::replay(&fx.project()).unwrap();
    assert_eq!(st.legal_accepts.len(), 1);
}

#[test]
fn the_message_that_accepted_one_record_cannot_accept_another() {
    let fx = Fx::new();
    for (id, t) in [("L-1", "One"), ("L-2", "Two")] {
        assert_eq!(put_legal(&fx, &legal(id, t, "open"), None).code, 0);
    }
    sleep(Duration::from_millis(1100));
    fx.human("I accept both of them, go ahead.");
    let o = put_legal(&fx, &legal("L-1", "One", "accepted"), Some("I accept both"));
    assert_eq!(o.code, 0, "{}", o.all());
    let o = put_legal(&fx, &legal("L-2", "Two", "accepted"), Some("I accept both"));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("already used"), "{}", o.stderr);
    // A new message accepts the second.
    fx.human("Yes, I accept the second one as well.");
    let o = put_legal(
        &fx,
        &legal("L-2", "Two", "accepted"),
        Some("I accept the second one"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_message_in_the_same_second_as_the_change_does_not_count() {
    let fx = Fx::new();
    assert_eq!(
        put_legal(&fx, &legal("L-1", "Cookie notice", "open"), None).code,
        0
    );
    sleep(Duration::from_millis(1100));
    fx.human("I accept the cookie notice risk.");
    // The change follows at once, in the same whole second as the message.
    assert_eq!(
        put_legal(&fx, &legal("L-1", "Cookie notice, changed", "open"), None).code,
        0
    );
    let o = put_legal(
        &fx,
        &legal("L-1", "Cookie notice, changed", "accepted"),
        Some("I accept the cookie notice risk"),
    );
    assert_eq!(o.code, 2, "{}", o.all());
}

#[test]
fn a_message_from_before_the_latest_change_of_the_record_does_not_count() {
    let fx = Fx::new();
    assert_eq!(
        put_legal(&fx, &legal("L-1", "Cookie notice", "open"), None).code,
        0
    );
    sleep(Duration::from_millis(1100));
    fx.human("I accept the cookie notice risk.");
    sleep(Duration::from_millis(1100));
    // The record changes after the human spoke: they did not see this.
    assert_eq!(
        put_legal(
            &fx,
            &legal("L-1", "Cookie notice and tracking", "open"),
            None
        )
        .code,
        0
    );
    let o = put_legal(
        &fx,
        &legal("L-1", "Cookie notice and tracking", "accepted"),
        Some("I accept the cookie notice risk"),
    );
    assert_eq!(o.code, 2, "{}", o.all());
    // After the change, a new message does.
    sleep(Duration::from_millis(1100));
    fx.human("Yes, I accept the cookie notice and tracking risk.");
    let o = put_legal(
        &fx,
        &legal("L-1", "Cookie notice and tracking", "accepted"),
        Some("I accept the cookie notice and tracking risk"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

fn legal_full(id: &str, title: &str, text: &str, outcome: &str) -> String {
    format!(
        "id=\"{id}\"\nkind=\"legal\"\ntitle=\"{title}\"\ntext=\"{text}\"\nseverity=\"high\"\noutcome=\"{outcome}\"\n"
    )
}

#[test]
fn the_put_that_accepts_cannot_also_change_what_is_accepted() {
    let fx = Fx::new();
    let o = put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "open"),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    sleep(Duration::from_millis(1100));
    fx.human("Fine, I accept the notice risk as it is.");
    let o = put_legal(
        &fx,
        &legal_full("L-1", "Something else", "Other text.", "accepted"),
        Some("I accept the notice risk"),
    );
    assert_eq!(o.code, 2, "{}", o.all());
    let r = fx.bf(&["record", "get", "L-1", "--json"]).stdout;
    assert!(r.contains("Notice") && r.contains("open"), "{r}");
    // The same content with the outcome accepted passes.
    let o = put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "accepted"),
        Some("I accept the notice risk"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn an_accepted_record_that_changes_goes_back_to_open() {
    let fx = Fx::new();
    put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "open"),
        None,
    );
    sleep(Duration::from_millis(1100));
    fx.human("Fine, I accept the notice risk as it is.");
    let o = put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "accepted"),
        Some("I accept the notice risk"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = put_legal(
        &fx,
        &legal_full("L-1", "Other", "Other text.", "accepted"),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let r = fx.bf(&["record", "get", "L-1", "--json"]).json();
    assert_eq!(r["outcome"], "open", "{r}");
}

#[test]
fn a_new_record_cannot_be_created_as_accepted() {
    let fx = Fx::new();
    fx.human("I accept the risk, go on.");
    sleep(Duration::from_millis(1100));
    let o = put_legal(
        &fx,
        &legal_full(
            "L-7",
            "Never seen",
            "Written after the message.",
            "accepted",
        ),
        Some("I accept the risk"),
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert_ne!(fx.bf(&["record", "get", "L-7"]).code, 0);
}

#[test]
fn the_put_that_resets_an_accepted_record_says_so() {
    let fx = Fx::new();
    put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "open"),
        None,
    );
    sleep(Duration::from_millis(1100));
    fx.human("Fine, I accept the notice risk as it is.");
    let o = put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "accepted"),
        Some("I accept the notice risk"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let f = fx.root.join("legal-put.toml");
    std::fs::write(&f, legal_full("L-1", "Other", "Other text.", "accepted")).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "legal",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(o.stdout.contains("back to open"), "{}", o.all());
}

#[test]
fn the_json_of_the_put_says_the_record_went_back_to_open() {
    let fx = Fx::new();
    put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "open"),
        None,
    );
    sleep(Duration::from_millis(1100));
    fx.human("Fine, I accept the notice risk as it is.");
    put_legal(
        &fx,
        &legal_full("L-1", "Notice", "One sentence is missing.", "accepted"),
        Some("I accept the notice risk"),
    );
    let f = fx.root.join("legal-put.toml");
    std::fs::write(&f, legal_full("L-1", "Other", "Other text.", "accepted")).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "legal",
        "--file",
        f.to_str().unwrap(),
        "--replace",
        "--json",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["reset_to_open"], true, "{}", o.all());
}
