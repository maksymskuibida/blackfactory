//! U-81 (R-74, D-93, D-102): the conductor puts a unit blocked by its builder's stated reason back
//! to work, at most twice, and gives the human the unit when the releases run out.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// U-1 (and U-2 when `two`) on a project with the files at the base; U-1 is started.
fn setup(two: bool) -> (Fx, PathBuf) {
    let fx = Fx::new();
    let mut recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n".into()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ntext=\"Build a.\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\n".into()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".into()),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".into()),
    ];
    let after = if two {
        recs.push(("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"Part B\"\ntext=\"Build b.\"\ndelivers=[\"R-1\"]\nfiles=[\"src/b.rs\"]\ncheckpoint=\"K-1\"\n".into()));
        recs.push(("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"B\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n".into()));
        "[\"U-1\", \"U-2\"]"
    } else {
        "[\"U-1\"]"
    };
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"All\"\nafter={after}\nproves=[\"R-1\"]\nchecks=[\"X-4\"]\n")));
    for (k, t) in &recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.set_policy("tokens = false", "tokens = true");
    write(&fx.root, "src/a.rs", "fn a() {}\n");
    write(&fx.root, "src/b.rs", "fn b() {}\n");
    fx.commit_all("files");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    (fx, wc)
}

fn log(fx: &Fx) -> String {
    fs::read_to_string(fx.project().log_path()).unwrap()
}

fn block(fx: &Fx, wc: &Path, note: &str) {
    let t = fx.take_token("U-1", "builder");
    let o = fx.bf_in(
        wc,
        &[
            "report", "U-1", "--result", "blocked", "--note", note, "--token", &t,
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

/// A builder's blocked report: the conductor judges it at once, with no second look for the builder.
fn confirmed(fx: &Fx, wc: &Path, note: &str) {
    block(fx, wc, note);
}

fn directive(fx: &Fx, action: &str) -> Option<Value> {
    let n = fx.must(&["next", "--json"]).json();
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == action && d["unit"] == "U-1")
        .cloned()
}

fn waiting(fx: &Fx) -> Vec<String> {
    let n = fx.must(&["next", "--json"]).json();
    n["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect()
}

fn state(fx: &Fx) -> (String, u32) {
    let st = bf::events::replay(&fx.project()).unwrap();
    let u = st.units.get("U-1").unwrap();
    (u.state.clone(), u.attempts)
}

#[test]
fn conductor_unblock_a_confirmed_block_gives_the_conductor_judge_block() {
    let (fx, wc) = setup(false);
    confirmed(&fx, &wc, "src/a.rs is held by U-2 beside it");
    let d = directive(&fx, "judge-block").expect("judge-block");
    assert_eq!(d["role"], "conductor");
    assert_eq!(d["releases_left"], 2);
    assert!(d["reason"].as_str().unwrap().contains("held by U-2"));
    assert_eq!(d["commands"].as_array().unwrap().len(), 3);
    assert!(directive(&fx, "decide-unit").is_none());
}

#[test]
fn conductor_unblock_a_release_puts_the_unit_back_and_keeps_the_attempts() {
    let (fx, wc) = setup(false);
    confirmed(&fx, &wc, "needs U-2 to land");
    let before = state(&fx).1;
    let o = fx.must(&["unit", "unblock", "U-1", "--why", "U-2 landed"]);
    assert!(o.all().contains("release 1"), "{}", o.all());
    assert_eq!(state(&fx), ("running".to_string(), before));
    let l = log(&fx);
    assert!(
        l.contains("unit.unblock") && l.contains("U-2 landed") && l.contains("needs U-2 to land")
    );
    let d = directive(&fx, "build").expect("a builder directive");
    assert_eq!(d["unblocked"]["why"], "U-2 landed");
    assert_eq!(d["unblocked"]["release"], 1);
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(
        brief.contains("U-2 landed") && brief.contains("needs U-2 to land"),
        "{brief}"
    );
}

#[test]
fn conductor_unblock_the_third_block_goes_to_the_human_naming_both_releases() {
    let (fx, wc) = setup(false);
    confirmed(&fx, &wc, "reason one");
    fx.must(&["unit", "unblock", "U-1", "--why", "first fix"]);
    confirmed(&fx, &wc, "reason two");
    assert_eq!(directive(&fx, "judge-block").unwrap()["releases_left"], 1);
    fx.must(&["unit", "unblock", "U-1", "--why", "second fix"]);
    confirmed(&fx, &wc, "reason three");
    assert!(directive(&fx, "judge-block").is_none());
    let d = directive(&fx, "decide-unit").expect("decide-unit");
    let note = d["note"].as_str().unwrap();
    assert!(
        note.contains("first fix") && note.contains("second fix"),
        "{note}"
    );
    let o = fx.refused(&["unit", "unblock", "U-1", "--why", "again"]);
    assert!(o.all().contains("first fix"), "{}", o.all());
}

#[test]
fn conductor_unblock_the_human_release_starts_the_count_again() {
    let (fx, wc) = setup(false);
    for w in ["a", "b"] {
        confirmed(&fx, &wc, "reason");
        fx.must(&["unit", "unblock", "U-1", "--why", w]);
    }
    let st = bf::events::replay(&fx.project()).unwrap();
    let evs = bf::events::load_log(&fx.project()).unwrap().events;
    assert_eq!(bf::run::unblock::releases_used(&evs, "U-1"), 2);
    assert!(st.units.contains_key("U-1"));
    confirmed(&fx, &wc, "reason");
    fx.human("Release U-1, please.");
    let o = fx.bf(&["unit", "release", "U-1", "--quote", "Release U-1, please."]);
    assert_eq!(o.code, 0, "{}", o.all());
    let evs = bf::events::load_log(&fx.project()).unwrap().events;
    assert_eq!(bf::run::unblock::releases_used(&evs, "U-1"), 0);
}

#[test]
fn conductor_unblock_wait_waits_for_a_landing_and_stalls_to_the_human() {
    let (fx, wc) = setup(true);
    confirmed(&fx, &wc, "U-2 holds what I need");
    // U-2 is ready to start, so the wait is a waiting line.
    fx.must(&[
        "unit",
        "unblock",
        "U-1",
        "--wait",
        "--why",
        "U-2 has not landed",
    ]);
    assert!(waiting(&fx)
        .iter()
        .any(|w| w.starts_with("U-1:") && w.contains("U-2 has not landed")));
    assert!(directive(&fx, "judge-block").is_none());
    assert_eq!(state(&fx).0, "blocked");
    // Alone and with no other unit ready, the wait can end no more: the human gets the unit.
    let (fx, wc) = setup(false);
    confirmed(&fx, &wc, "U-9 holds what I need");
    fx.must(&["unit", "unblock", "U-1", "--wait", "--why", "not yet"]);
    let d = directive(&fx, "decide-unit").expect("decide-unit");
    let note = d["note"].as_str().unwrap();
    assert!(
        note.contains("U-9 holds what I need") && note.contains("not yet"),
        "{note}"
    );
}

#[test]
fn conductor_unblock_human_judgment_goes_to_the_human_with_the_words() {
    let (fx, wc) = setup(false);
    confirmed(&fx, &wc, "needs a decision");
    fx.must(&[
        "unit",
        "unblock",
        "U-1",
        "--human",
        "--why",
        "only the human can say",
    ]);
    let d = directive(&fx, "decide-unit").expect("decide-unit");
    assert!(d["note"]
        .as_str()
        .unwrap()
        .contains("only the human can say"));
    assert!(log(&fx).contains("\"judgment\":\"human\""));
    assert_eq!(state(&fx).0, "blocked");
}

#[test]
fn conductor_unblock_refusals_name_the_reason() {
    let (fx, wc) = setup(false);
    let o = fx.refused(&["unit", "unblock", "U-1", "--why", "x"]);
    assert!(o.all().contains("running"), "{}", o.all());
    block(&fx, &wc, "second");
    let o = fx.refused(&["unit", "unblock", "U-1", "--why", "  "]);
    assert!(o.all().contains("`--why` is empty"), "{}", o.all());
    let o = fx.refused(&["unit", "unblock", "U-1", "--wait", "--human", "--why", "x"]);
    assert!(o.all().contains("both given"), "{}", o.all());
    assert!(!log(&fx).contains("unit.unblock"));
}

#[test]
fn conductor_unblock_an_empty_note_stays_with_the_human() {
    let (fx, wc) = setup(false);
    confirmed(&fx, &wc, "");
    assert!(directive(&fx, "judge-block").is_none());
    assert!(directive(&fx, "decide-unit").is_some());
    let o = fx.refused(&["unit", "unblock", "U-1", "--why", "x"]);
    assert!(o.all().contains("no note"), "{}", o.all());
}

#[test]
fn conductor_unblock_the_guard_refuses_any_agent_type_and_passes_the_conductor() {
    let (fx, _wc) = setup(false);
    let run = |agent: Option<&str>| -> String {
        let mut payload = json!({"tool_name": "Bash", "tool_input": {"command": "bf unit unblock U-1 --why \"x\""},
            "cwd": fx.root.to_string_lossy(), "session_id": "s"});
        if let Some(a) = agent {
            payload["agent_type"] = json!(a);
            payload["agent_id"] = json!("agent-1");
        }
        let o = fx.bf_stdin(&fx.root, &["guard"], &payload.to_string());
        assert_eq!(o.code, 0, "{}", o.all());
        o.stdout
    };
    for a in ["builder", "repair-judge", "designer"] {
        let out = run(Some(a));
        assert!(out.contains("deny") && out.contains(a), "{a}: {out}");
    }
    assert!(!run(None).contains("deny"));
}

#[test]
fn conductor_unblock_a_judge_approval_gives_no_new_release_and_a_human_one_does() {
    let ev = |seq: u64, v: Value| {
        let mut v = v;
        v["seq"] = json!(seq);
        v
    };
    let rel = |seq| {
        ev(
            seq,
            json!({"kind": "unit.unblock", "unit": "U-1", "judgment": "fixed"}),
        )
    };
    let judged = ev(
        3,
        json!({"kind": "amend.approve", "by": "judge", "reopen_units": ["U-1"]}),
    );
    let human = ev(5, json!({"kind": "amend.approve", "reopen_units": ["U-1"]}));
    let used = bf::run::unblock::releases_used;
    assert_eq!(used(&[rel(1), rel(2), judged.clone()], "U-1"), 2);
    assert_eq!(
        used(
            &[rel(1), rel(2), judged.clone(), rel(4), human.clone()],
            "U-1"
        ),
        0
    );
    assert_eq!(
        used(&[rel(1), rel(2), judged, rel(4), human, rel(6)], "U-1"),
        1
    );
}

fn release_twice(fx: &Fx, wc: &Path, words: [&str; 2]) {
    for w in words {
        confirmed(fx, wc, "reason");
        fx.must(&["unit", "unblock", "U-1", "--why", w]);
    }
}

#[test]
fn conductor_unblock_the_note_and_refusal_count_since_the_human_release() {
    let (fx, wc) = setup(false);
    release_twice(&fx, &wc, ["old one", "old two"]);
    confirmed(&fx, &wc, "reason");
    fx.human("Release U-1, please.");
    let o = fx.bf(&["unit", "release", "U-1", "--quote", "Release U-1, please."]);
    assert_eq!(o.code, 0, "{}", o.all());
    release_twice(&fx, &wc, ["new one", "new two"]);
    confirmed(&fx, &wc, "reason");
    let note = directive(&fx, "decide-unit").expect("decide-unit")["note"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(note.contains("release 1: new one") && note.contains("release 2: new two"), "{note}");
    assert!(note.contains("release 1 (before the human's own release of the unit): old one"), "{note}");
    assert!(note.contains("release 2 (before the human's own release of the unit): old two"), "{note}");
    assert!(!note.contains("release 3"), "{note}");
    let o = fx.refused(&["unit", "unblock", "U-1", "--why", "again"]);
    assert!(o.all().contains("new one") && !o.all().contains("old one"), "{}", o.all());
}

#[test]
fn conductor_unblock_with_no_release_left_a_wait_is_refused_and_the_human_keeps_the_unit() {
    let (fx, wc) = setup(true);
    release_twice(&fx, &wc, ["first", "second"]);
    confirmed(&fx, &wc, "reason");
    let o = fx.refused(&["unit", "unblock", "U-1", "--wait", "--why", "later"]);
    assert!(o.all().contains("first"), "{}", o.all());
    assert!(directive(&fx, "decide-unit").is_some());
    assert!(waiting(&fx).iter().all(|w| !w.starts_with("U-1:")));
    // A `human` judgment is still taken.
    fx.must(&["unit", "unblock", "U-1", "--human", "--why", "only the human"]);
    assert!(directive(&fx, "decide-unit").is_some());
}
