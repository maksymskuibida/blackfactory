//! D-115, D-116, D-117: a checkpoint fix's file list is a recommendation. A fix that changes a file outside
//! it reports built, the brief of its reviewer names the file and quotes the builder's note as a claim, a
//! request for files is refused, and a log with an old request or grant replays and the fix goes on.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const K_CHECK: &str = "grep -q fixed src/a.txt";
const A: &str = "tests/attacks.rs";
const M: &str = "tests/common/mod.rs";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{K_CHECK}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fs::create_dir_all(fx.root.join("tests/common")).unwrap();
    fs::write(fx.root.join(M), "pub fn helper() {}\n").unwrap();
    fs::write(fx.root.join(A), "// attacks\nfn a() {}\n").unwrap();
    fs::write(fx.root.join("tests/other.rs"), "// other\nfn o() {}\n").unwrap();
    fx.commit_all("records");
}

fn land_unit(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
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
            "src/a.txt",
        ],
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

fn one_directive(fx: &Fx, action: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "one directive while a fix is open: {n}");
    assert_eq!(ds[0]["action"], action, "{n}");
    ds[0].clone()
}

fn open_fix(fx: &Fx) -> PathBuf {
    plan(fx);
    land_unit(fx);
    let k = fx.work("K-1");
    let o = fx.bf_in(
        &k,
        &[
            "evidence",
            "run",
            "X-2",
            "--",
            "grep",
            "-q",
            "fixed",
            "src/a.txt",
        ],
    );
    assert!(!o.all().is_empty());
    one_directive(fx, "fix-checkpoint");
    k
}

fn ask(fx: &Fx, k: &Path, files: Value) -> Out {
    fx.bf_stdin(
        k,
        &[
            "report",
            "K-1",
            "--role",
            "builder",
            "--result",
            "blocked",
            "--need-files",
            "-",
        ],
        &files.to_string(),
    )
}

fn patch_for(path: &str, add: &str) -> String {
    format!("--- a/{path}\n+++ b/{path}\n@@ -1 +1,2 @@\n context\n+{add}\n")
}

fn req(path: &str, add: &str) -> Value {
    json!({"path": path, "why": "the fix needs it", "patch": patch_for(path, add)})
}

fn builder_brief(fx: &Fx) -> String {
    fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap()
}

fn commit(k: &Path) {
    git(k, &["add", "-A"]);
    git(k, &["commit", "-q", "-m", "fix"]);
}

fn built(fx: &Fx, k: &Path, note: &str) -> Out {
    fx.bf_in(
        k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--note", note,
        ],
    )
}

fn reviewer_brief(fx: &Fx) -> String {
    fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "reviewer"])
            .stdout
            .trim(),
    )
    .unwrap()
}

fn built_events(fx: &Fx) -> Vec<Value> {
    bf::events::load_log(&fx.project())
        .unwrap()
        .events
        .into_iter()
        .filter(|e| e["kind"] == "checkpoint.fix.built")
        .collect()
}

#[test]
fn ckfix_recommended_a_file_outside_the_lists_reports_built_and_the_reviewer_is_shown_it() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("src/outside.txt"), "needed by the fix\n").unwrap();
    commit(&k);
    let o = built(&fx, &k, "outside.txt holds the helper the fix needs");
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "review-checkpoint-fix");
    assert_eq!(d["role"], "reviewer", "{d}");
    let b = reviewer_brief(&fx);
    assert!(
        b.contains("`src/outside.txt` lies outside the lists of the checkpoint's units"),
        "{b}"
    );
    assert!(!b.contains("`src/a.txt` lies outside"), "{b}");
    assert!(
        b.contains("> outside.txt holds the helper the fix needs"),
        "{b}"
    );
    assert!(b.contains("a reason to check, never a fact"), "{b}");
    let ev = built_events(&fx);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["note"], "outside.txt holds the helper the fix needs");
    // The reviewer's clean report ends the fix and the content is the checkpoint's.
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        fs::read_to_string(k.join("src/outside.txt")).unwrap(),
        "needed by the fix\n"
    );
}

#[test]
fn ckfix_recommended_the_builders_brief_says_the_list_is_a_recommendation_and_its_report_gives_the_reason(
) {
    let fx = Fx::new();
    open_fix(&fx);
    let b = builder_brief(&fx);
    assert!(b.contains("not a limit"), "{b}");
    assert!(
        b.contains("Give the reason for each such file in the note of your report"),
        "{b}"
    );
    assert!(!b.contains("--need-files"), "{b}");
}

#[test]
fn ckfix_recommended_a_note_is_quoted_line_by_line_and_cut_at_2000_characters() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    commit(&k);
    let mut note = String::from("# Not a heading\n---\n## Nor this\n\n");
    note.push_str(&"x".repeat(2500));
    let o = built(&fx, &k, &note);
    assert_eq!(o.code, 0, "{}", o.all());
    let ev = built_events(&fx);
    assert_eq!(ev[0]["note"], note.as_str());
    let b = reviewer_brief(&fx);
    assert!(
        b.contains("\n> # Not a heading\n> ---\n> ## Nor this\n> \n> xxx"),
        "{b}"
    );
    assert!(b.contains("(The note was cut at 2000 characters.)"), "{b}");
    let quoted: usize = b
        .lines()
        .filter(|l| l.starts_with("> "))
        .map(|l| l.matches('x').count())
        .sum();
    assert!(quoted > 0 && quoted <= 2000, "{quoted}");
}

#[test]
fn ckfix_recommended_a_request_for_files_is_refused_with_the_text_of_its_key() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let o = ask(&fx, &k, json!([req(A, "one")]));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr
            .contains("a fix's file list is a recommendation, so nothing is asked for or granted"),
        "{}",
        o.stderr
    );
    one_directive(&fx, "fix-checkpoint");
    let st = bf::events::replay(&fx.project()).unwrap();
    let f = st.ck_fix.unwrap();
    assert!(f.request.is_empty() && f.refused.is_empty() && f.grants.is_empty());
}

#[test]
fn ckfix_recommended_a_log_with_an_open_request_and_a_grant_replays_and_the_fix_goes_on() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let p = fx.project();
    let files = json!([{"path": A, "why": "w", "patch": patch_for(A, "one"), "new": false}]);
    bf::events::append(
        &p,
        "checkpoint.fix.scope.request",
        json!({"checkpoint": "K-1", "files": files, "note": ""}),
    )
    .unwrap();
    // A request open at the change no longer holds the fix: the builder is still the one at work.
    let st = bf::events::replay(&p).unwrap();
    assert_eq!(st.ck_fix.as_ref().unwrap().phase, "scope");
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["role"], "builder", "{d}");
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    commit(&k);
    let o = built(&fx, &k, "");
    assert_eq!(o.code, 0, "{}", o.all());
    one_directive(&fx, "review-checkpoint-fix");
}

#[test]
fn ckfix_recommended_a_grant_in_an_old_log_replays_as_before() {
    let fx = Fx::new();
    open_fix(&fx);
    let p = fx.project();
    let files = json!([{"path": A, "why": "w", "patch": patch_for(A, "one"), "new": false}]);
    bf::events::append(
        &p,
        "checkpoint.fix.scope.request",
        json!({"checkpoint": "K-1", "files": files}),
    )
    .unwrap();
    bf::events::append(
        &p,
        "checkpoint.fix.scope.grant",
        json!({"checkpoint": "K-1", "files": files, "verdicts": []}),
    )
    .unwrap();
    let st = bf::events::replay(&p).unwrap();
    let f = st.ck_fix.clone().unwrap();
    assert_eq!(f.phase, "fix");
    assert_eq!(bf::fixscope::granted(&st, "K-1"), vec![A.to_string()]);
    assert!(st.ignored.is_empty());
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["granted"].as_array().unwrap().len(), 1, "{d}");
}

fn refused_with_rule(fx: &Fx, k: &Path, name: &str, rule_part: &str) {
    let o = built(fx, k, "why");
    assert_eq!(o.code, 2, "{name}: {}", o.all());
    assert!(
        o.stderr.contains(name) && o.stderr.contains(rule_part),
        "{name}: {}",
        o.stderr
    );
    one_directive(fx, "fix-checkpoint");
}

#[test]
fn ckfix_recommended_a_protected_file_is_refused_with_its_rule() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::create_dir_all(k.join(".claude")).unwrap();
    fs::write(k.join(".claude/x.txt"), "x\n").unwrap();
    commit(&k);
    refused_with_rule(&fx, &k, ".claude/x.txt", "a place the guard protects");
}

#[test]
fn ckfix_recommended_a_symbolic_link_is_refused_with_its_rule() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    std::os::unix::fs::symlink("/etc/hosts", k.join("src/link.txt")).unwrap();
    commit(&k);
    refused_with_rule(&fx, &k, "src/link.txt", "symbolic link");
}

#[test]
fn ckfix_recommended_a_look_alike_name_is_refused_with_its_rule() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("src/\u{17f}rc.txt"), "x\n").unwrap();
    commit(&k);
    refused_with_rule(
        &fx,
        &k,
        "src/\u{17f}rc.txt",
        "case-folds to an ASCII letter",
    );
}

#[test]
fn ckfix_recommended_a_plain_outside_file_is_named_to_the_reviewer_and_to_the_builder_of_a_later_round(
) {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("src/outside.txt"), "x\n").unwrap();
    commit(&k);
    assert_eq!(built(&fx, &k, "because").code, 0);
    let b = reviewer_brief(&fx);
    assert!(
        b.contains("`src/outside.txt` lies outside the lists of the checkpoint's units"),
        "{b}"
    );
    assert!(!b.contains("Rule:"), "{b}");
    let o = fx.bf_stdin(
        &k,
        &[
            "report",
            "K-1",
            "--role",
            "reviewer",
            "--result",
            "findings",
            "--findings",
            "-",
        ],
        &json!([{"title": "t", "severity": "medium", "where": "src/a.txt", "action": "a"}])
            .to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    one_directive(&fx, "fix-checkpoint");
    let b = builder_brief(&fx);
    assert!(
        b.contains("`src/outside.txt` lies outside the lists of the checkpoint's units"),
        "{b}"
    );
    assert!(
        b.contains("Give the reason for each in the note of your report"),
        "{b}"
    );
    assert!(b.contains("recommendation"), "{b}");
}

#[test]
fn ckfix_recommended_a_plain_non_ascii_name_outside_the_lists_reports_built_and_is_named() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("src/caf\u{e9}.txt"), "x\n").unwrap();
    commit(&k);
    let o = built(&fx, &k, "why");
    assert_eq!(o.code, 0, "{}", o.all());
    let b = reviewer_brief(&fx);
    assert!(
        b.contains("`src/caf\u{e9}.txt` lies outside the lists of the checkpoint's units"),
        "{b}"
    );
}

#[test]
fn ckfix_recommended_a_non_ascii_new_test_file_inside_the_test_folder_is_not_outside() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("tests/caf\u{e9}.txt"), "x\n").unwrap();
    commit(&k);
    assert_eq!(built(&fx, &k, "why").code, 0);
    let b = reviewer_brief(&fx);
    assert!(!b.contains("lies outside"), "{b}");
}

#[test]
fn ckfix_recommended_a_name_with_a_newline_is_refused_and_printed_as_an_escape() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("src/odd\nname`.txt"), "x\n").unwrap();
    commit(&k);
    let o = built(&fx, &k, "why");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("src/odd\\nname\\`.txt cannot be changed"),
        "{}",
        o.stderr
    );
}
