//! U-105 (R-107, D-86): after a builder's plain blocked report the engine gives one more builder
//! directive to look again; a plain block made from it goes to the human.

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

fn ask(fx: &Fx, wc: &Path) -> Out {
    let t = fx.take_token("U-1", "builder");
    let patch = "--- a\n+++ b\n@@ -1 +1 @@\n-a\n+b\n";
    let l = json!([{"path": "src/other.rs", "why": "needed", "patch": patch}]).to_string();
    fx.bf_stdin(
        wc,
        &[
            "report",
            "U-1",
            "--result",
            "blocked",
            "--need-files",
            "-",
            "--token",
            &t,
        ],
        &l,
    )
}

#[test]
fn a_plain_block_gives_a_reconsider_directive_with_the_note_in_its_brief() {
    let (fx, wc) = setup(None);
    block(&fx, &wc, "my change breaks a test elsewhere");
    assert!(log(&fx).contains("\"reconsider\":true"));
    let d = directive(&fx, "builder").expect("a builder directive");
    assert_eq!(d["action"], "reconsider");
    assert!(!d["token"].as_str().unwrap_or("").is_empty());
    let n = fx.must(&["next", "--json"]).json();
    assert!(!n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["action"] == "decide-unit"));
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(
        brief.contains("my change breaks a test elsewhere"),
        "{brief}"
    );
    assert!(brief.contains("--need-files"));
}

#[test]
fn a_request_from_it_opens_a_review_without_a_release() {
    let (fx, wc) = setup(None);
    block(&fx, &wc, "needs a file");
    let before = state(&fx).1;
    let o = ask(&fx, &wc);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(directive(&fx, "reviewer").is_some_and(|d| d["action"] == "review-scope"));
    assert!(!log(&fx).contains("unit.release"));
    let (s, a) = state(&fx);
    assert_eq!(s, "running");
    assert_eq!(a, before + 1, "the request counts as one attempt");
}

#[test]
fn a_restore_from_it_puts_the_unit_back_to_running() {
    let (fx, wc) = setup(None);
    write(&wc, "src/other.rs", "fn   o( ) { }\n");
    block(&fx, &wc, "a stray file");
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
    let t = fx.take_token("U-1", "builder");
    let o = fx.bf_in(&wc, &["report", "U-1", "--result", "built", "--token", &t]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(state(&fx).0, "review");
}

#[test]
fn a_plain_block_from_it_goes_to_the_human() {
    let (fx, wc) = setup(None);
    block(&fx, &wc, "first");
    block(&fx, &wc, "a real gap in the design");
    assert!(log(&fx).contains("\"confirmed\":true"));
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert!(ds
        .iter()
        .any(|d| d["action"] == "decide-unit" && d["unit"] == "U-1"));
    assert!(!ds.iter().any(|d| d["action"] == "reconsider"));
    assert_eq!(state(&fx).0, "blocked");
}

#[test]
fn a_unit_that_keeps_blocking_is_parked_at_the_attempt_limit() {
    let (fx, wc) = setup(Some(2));
    block(&fx, &wc, "needs a file");
    let o = ask(&fx, &wc);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(state(&fx).0, "parked");
}

#[test]
fn every_builder_brief_holds_the_three_lines() {
    let (fx, _wc) = setup(None);
    let o = fx.must(&["brief", "U-1", "--role", "builder"]);
    let _ = o;
    let path = fx.project().briefs_dir().join("U-1-builder.md");
    let brief = fs::read_to_string(path).unwrap();
    assert!(brief.contains("is a request with `--need-files`, never a plain blocked report"));
    assert!(brief.contains("bf unit restore U-1 <path> --token <token>"));
    assert!(brief.contains("format only"));
    assert!(brief.contains("for a gap in the design only"));
}

#[test]
fn block_and_restore_cannot_repeat_past_the_attempt_limit() {
    let (fx, wc) = setup(Some(1));
    write(&wc, "src/other.rs", "fn   o( ) { }\n");
    block(&fx, &wc, "a stray file");
    let t = fx.take_token("U-1", "builder");
    fx.must_in(
        &wc,
        &["unit", "restore", "U-1", "src/other.rs", "--token", &t],
    );
    assert_eq!(state(&fx).0, "parked");
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
    let d = directive(&fx, "builder").unwrap();
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(brief.contains("You reported this unit blocked."), "{brief}");
    assert!(!brief.contains("and said"), "{brief}");
}
