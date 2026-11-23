//! U-105 (R-107, D-86), D-102: after a builder's plain blocked report the engine gives no second builder
//! directive; the conductor judges the block (`bf unit unblock`), and the unit then goes on.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// One unit that lists src/a.rs, on a project with another file at the base.
fn setup(attempts: Option<u32>) -> (Fx, PathBuf) {
    let fx = Fx::new();
    if let Some(n) = attempts {
        fx.set_policy("max_attempts = 10", &format!("max_attempts = {n}"));
    }
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ntext=\"Build a.\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"All\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.set_policy("tokens = false", "tokens = true");
    write(&fx.root, "src/a.rs", "fn a() {}\n");
    write(&fx.root, "src/other.rs", "fn o() {}\n");
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

fn state(fx: &Fx) -> (String, u32) {
    let st = bf::events::replay(&fx.project()).unwrap();
    let u = st.units.get("U-1").unwrap();
    (u.state.clone(), u.attempts)
}

fn directive(fx: &Fx, role: &str) -> Option<Value> {
    let n = fx.must(&["next", "--json"]).json();
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["unit"] == "U-1" && d["role"] == role)
        .cloned()
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

/// The conductor's judgment that ends the hold: the unit is at work again and a builder directive follows.
fn unblock(fx: &Fx) {
    fx.must(&["unit", "unblock", "U-1", "--why", "the cause is gone"]);
}

#[test]
fn a_plain_block_gives_the_conductor_a_judgment_and_no_second_look_for_the_builder() {
    let (fx, wc) = setup(None);
    block(&fx, &wc, "my change breaks a test elsewhere");
    assert!(log(&fx).contains("\"reconsider\":true"));
    assert!(directive(&fx, "builder").is_none());
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert!(ds.iter().any(|d| d["action"] == "judge-block"));
    assert!(!ds.iter().any(|d| d["action"] == "reconsider"));
    unblock(&fx);
    let d = directive(&fx, "builder").expect("a builder directive");
    assert_eq!(d["action"], "build");
    assert!(!d["token"].as_str().unwrap_or("").is_empty());
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(
        brief.contains("my change breaks a test elsewhere"),
        "{brief}"
    );
    assert!(!brief.contains("--need-files"));
    assert!(brief.contains("The list of files is a recommendation"));
}

#[test]
fn a_restore_from_it_puts_the_unit_back_to_running() {
    let (fx, wc) = setup(None);
    write(&wc, "src/other.rs", "fn   o( ) { }\n");
    block(&fx, &wc, "a stray file");
    unblock(&fx);
    let t = fx.take_token("U-1", "builder");
    fx.must_in(
        &wc,
        &["unit", "restore", "U-1", "src/other.rs", "--token", &t],
    );
    assert_eq!(state(&fx).0, "running");
    assert_eq!(
        fs::read_to_string(wc.join("src/other.rs")).unwrap(),
        "fn o() {}\n"
    );
    assert!(directive(&fx, "builder").is_some_and(|d| d["action"] == "build"));
}

#[test]
fn a_built_report_from_it_goes_to_review() {
    let (fx, wc) = setup(None);
    block(&fx, &wc, "format only");
    unblock(&fx);
    let t = fx.take_token("U-1", "builder");
    let o = fx.bf_in(&wc, &["report", "U-1", "--result", "built", "--token", &t]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(state(&fx).0, "review");
}

#[test]
fn a_plain_block_without_a_note_goes_to_the_human_and_not_to_the_conductor() {
    let (fx, wc) = setup(None);
    block(&fx, &wc, "first");
    unblock(&fx);
    block(&fx, &wc, "");
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    let own: Vec<&Value> = ds.iter().filter(|d| d["unit"] == "U-1").collect();
    assert_eq!(own.len(), 1, "{n}");
    assert_eq!(own[0]["action"], "decide-unit", "{n}");
    assert_eq!(own[0]["role"], "human", "{n}");
    assert!(!ds.iter().any(|d| d["action"] == "judge-block" || d["action"] == "reconsider"));
    assert_eq!(state(&fx).0, "blocked");
}

#[test]
fn every_builder_brief_holds_the_three_lines() {
    let (fx, _wc) = setup(None);
    let o = fx.must(&["brief", "U-1", "--role", "builder"]);
    let _ = o;
    let path = fx.project().briefs_dir().join("U-1-builder.md");
    let brief = fs::read_to_string(path).unwrap();
    assert!(brief.contains("The list of files is a recommendation"));
    assert!(brief.contains("give the reason in your report"));
    assert!(!brief.contains("--need-files"));
    assert!(brief.contains("bf unit restore U-1 <path> --token <token>"));
    assert!(brief.contains("format only"));
    assert!(brief.contains("for a gap in the design only"));
}

#[test]
fn a_block_counts_as_an_attempt_and_waits_for_the_conductor() {
    let (fx, wc) = setup(Some(1));
    write(&wc, "src/other.rs", "fn   o( ) { }\n");
    block(&fx, &wc, "a stray file");
    assert_eq!(state(&fx), ("blocked".to_string(), 1));
    assert!(directive(&fx, "builder").is_none());
}

#[test]
fn a_block_without_a_note_leaves_the_said_clause_out() {
    let (fx, wc) = setup(None);
    let t = fx.take_token("U-1", "builder");
    let o = fx.bf_in(
        &wc,
        &["report", "U-1", "--result", "blocked", "--token", &t],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["brief", "U-1", "--role", "builder"]);
    let brief = fs::read_to_string(fx.project().briefs_dir().join("U-1-builder.md")).unwrap();
    assert!(brief.contains("You reported this unit blocked."), "{brief}");
    assert!(!brief.contains("and said"), "{brief}");
}
