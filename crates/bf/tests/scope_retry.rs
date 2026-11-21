//! A-19 (U-76): the scope reviewer may answer a file request with `retry` and the words a corrected patch
//! must meet. Nothing is granted, the builder asks again within the review rounds, and the human is asked
//! only when the rounds run out or the file is an extension.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const OLD_TEST: &str = "fn t() {\n    assert_eq!(1, 1);\n    assert_eq!(2, 2);\n}\n";

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// Three units on one checkpoint: U-1 (the builder that asks), U-2 (depends on U-1, holds shared/x.txt)
/// and U-3 (may run beside U-1, holds beside/y.txt). The files below exist in the main checkout.
fn project(fx: &Fx) {
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n"),
        ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"B\"\nacceptance=[\"shared/x.txt exists\"]\n"),
        ("requirement", "id=\"R-3\"\nkind=\"requirement\"\ntitle=\"C\"\nacceptance=[\"beside/y.txt exists\"]\n"),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Parts\"\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ntext=\"Build a. The guide docs/guide.md describes it.\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\nestimate=2\n"),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"Part B\"\ndelivers=[\"R-2\"]\nfiles=[\"shared/x.txt\"]\ndepends_on=[\"U-1\"]\ncheckpoint=\"K-1\"\nestimate=1\n"),
        ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Part C\"\ndelivers=[\"R-3\"]\nfiles=[\"beside/y.txt\"]\ncheckpoint=\"K-1\"\nestimate=1\n"),
        ("handoff", "id=\"H-1\"\nkind=\"handoff\"\nfrom=\"U-1\"\nto=\"U-2\"\nthing=\"a\"\ncontract=\"b\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"B\"\nhow=\"command\"\ncommand=\"test -f shared/x.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"C\"\nhow=\"command\"\ncommand=\"test -f beside/y.txt\"\nproves=[\"R-3\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\",\"R-2\",\"R-3\",\"C-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"All\"\nafter=[\"U-1\",\"U-2\",\"U-3\"]\nproves=[\"R-1\",\"R-2\",\"R-3\",\"C-1\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    write(&fx.root, "src/a.rs", "fn a() {}\n");
    write(&fx.root, "src/c.rs", "fn c() {}\n");
    write(&fx.root, "tests/old_test.rs", OLD_TEST);
    write(&fx.root, "Cargo.toml", "[package]\nname = \"x\"\n");
    write(&fx.root, "harness/claude/agents/extra.md", "role\n");
    write(&fx.root, ".github/workflows/ci.yml", "name: ci\n");
    write(&fx.root, "docs/guide.md", "guide\n");
    write(&fx.root, "shared/x.txt", "x\n");
    write(&fx.root, "beside/y.txt", "y\n");
    fx.commit_all("records and files");
    fx.start_test_run();
}

fn start(fx: &Fx) -> PathBuf {
    PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

fn entry(path: &str, why: &str, patch: &str) -> Value {
    json!({"path": path, "why": why, "patch": patch})
}

fn ask(fx: &Fx, wc: &Path, list: Value) -> Out {
    fx.bf_stdin(
        wc,
        &[
            "report",
            "U-1",
            "--result",
            "blocked",
            "--need-files",
            "-",
            "--json",
        ],
        &list.to_string(),
    )
}

fn judge(fx: &Fx, wc: &Path, list: Value) -> Out {
    fx.bf_stdin(
        wc,
        &[
            "report",
            "U-1",
            "--role",
            "reviewer",
            "--result",
            "scope",
            "--verdict",
            "-",
            "--json",
        ],
        &list.to_string(),
    )
}

fn judge_plain(fx: &Fx, wc: &Path, list: Value) -> Out {
    fx.bf_stdin(
        wc,
        &[
            "report",
            "U-1",
            "--role",
            "reviewer",
            "--result",
            "scope",
            "--verdict",
            "-",
        ],
        &list.to_string(),
    )
}

fn needed(path: &str) -> Value {
    json!({"path": path, "verdict": "needed", "why": "the unit's change needs it"})
}

fn unit(fx: &Fx, id: &str) -> Value {
    let st = fx.must(&["status", "--json"]).json();
    st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .unwrap()
        .clone()
}

fn guard_says(fx: &Fx, cwd: &Path, file: &Path) -> String {
    let payload = json!({"tool_name": "Write", "tool_input": {"file_path": file.to_string_lossy()}, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => "abstain".into(),
        t => serde_json::from_str::<Value>(t).unwrap()["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap()
            .to_string(),
    }
}

fn directive(fx: &Fx, unit: &str, role: &str) -> Option<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["unit"] == unit && d["role"] == role)
        .cloned()
}

fn retry_item(path: &str, must: &str) -> Value {
    json!({"path": path, "verdict": "retry", "why": "the patch drops an assertion", "must": must})
}

fn patch_n(n: u32) -> String {
    format!("--- a/tests/old_test.rs\n+++ b/tests/old_test.rs\n@@ -2,3 +2,4 @@\n     assert_eq!(1, 1);\n+    assert_eq!({n}, {n});\n     assert_eq!(2, 2);\n")
}

fn ask_old(fx: &Fx, wc: &Path, n: u32) -> Out {
    ask(
        fx,
        wc,
        json!([entry(
            "tests/old_test.rs",
            "R-1: the change breaks it",
            &patch_n(n)
        )]),
    )
}

fn state_of(fx: &Fx) -> String {
    unit(fx, "U-1")["state"].as_str().unwrap().to_string()
}

fn actions(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["unit"] == "U-1")
        .map(|d| d["action"].as_str().unwrap().to_string())
        .collect()
}

fn page(fx: &Fx) -> String {
    fx.must(&["view", "run", "--json"]).json().to_string()
}

fn refused_words(fx: &Fx) -> Vec<String> {
    let st = bf::events::replay(&fx.project()).unwrap();
    st.scope
        .get("U-1")
        .map(|sc| {
            sc.refused
                .iter()
                .map(|r| r["refusal"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_retry_grants_nothing_and_opens_a_second_look_that_names_the_file_and_the_must() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    assert_eq!(ask_old(&fx, &wc, 3).code, 0);
    let o = judge(
        &fx,
        &wc,
        json!([retry_item("tests/old_test.rs", "keep both old assertions")]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["granted"], json!([]));
    assert_eq!(refused_words(&fx), vec!["retry"]);
    assert_eq!(state_of(&fx), "blocked");
    let d = directive(&fx, "U-1", "builder").expect("a builder directive");
    assert_eq!(d["action"], "reconsider", "{d}");
    assert_eq!(d["retry"][0]["path"], "tests/old_test.rs");
    assert_eq!(d["retry"][0]["must"], "keep both old assertions");
    assert!(!actions(&fx).contains(&"decide-unit".to_string()));
    assert_ne!(guard_says(&fx, &wc, &wc.join("tests/old_test.rs")), "deny");
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(
        brief.contains("keep both old assertions") && brief.contains("round 1 of 3"),
        "{brief}"
    );
    assert!(brief.contains("--need-files -"), "{brief}");
    // The page shows the request for a corrected patch, not a wait for a plan repair.
    let pg = page(&fx);
    assert!(
        pg.contains("keep both old assertions") && !pg.contains("plan repair"),
        "{pg}"
    );
}

#[test]
fn the_corrected_request_is_judged_in_round_two_and_a_needed_verdict_grants_it() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    ask_old(&fx, &wc, 3);
    judge(
        &fx,
        &wc,
        json!([retry_item("tests/old_test.rs", "keep both old assertions")]),
    );
    let o = ask_old(&fx, &wc, 4);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(state_of(&fx), "running");
    let d = directive(&fx, "U-1", "reviewer").expect("a reviewer directive");
    fx.must(&["brief", "U-1", "--role", "reviewer"]);
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(brief.contains("round 2 of 3"), "{brief}");
    assert!(
        brief.contains("keep both old assertions")
            && brief.contains(&patch_n(3).lines().nth(4).unwrap().to_string()),
        "{brief}"
    );
    let o = judge(&fx, &wc, json!([needed("tests/old_test.rs")]));
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["granted"], json!(["tests/old_test.rs"]));
    assert_eq!(state_of(&fx), "running");
    assert!(page(&fx).contains("granted by the scope reviewer"));
    assert_eq!(
        guard_says(&fx, &wc, &wc.join("tests/old_test.rs")),
        "abstain"
    );
}

#[test]
fn three_retries_are_three_judgments_the_third_is_spent_and_a_fourth_request_is_refused() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    for n in 3..6 {
        let o = ask_old(&fx, &wc, n);
        assert_eq!(o.code, 0, "{}", o.all());
        let o = judge(
            &fx,
            &wc,
            json!([retry_item("tests/old_test.rs", "keep both old assertions")]),
        );
        assert_eq!(o.code, 0, "{}", o.all());
    }
    assert_eq!(refused_words(&fx), vec!["retry", "retry", "retry_spent"]);
    assert_eq!(state_of(&fx), "blocked");
    assert!(
        actions(&fx).contains(&"decide-unit".to_string()),
        "{:?}",
        actions(&fx)
    );
    assert!(!actions(&fx).contains(&"reconsider".to_string()));
    assert!(page(&fx).contains("waits for the human"));
}

#[test]
fn a_spent_file_is_refused_naming_the_rounds() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    for n in 3..6 {
        ask_old(&fx, &wc, n);
        judge(
            &fx,
            &wc,
            json!([retry_item("tests/old_test.rs", "keep both old assertions")]),
        );
    }
    let o = ask_old(&fx, &wc, 9);
    assert_ne!(o.code, 0);
    assert!(
        o.all().contains("tests/old_test.rs") && o.all().contains("3 times"),
        "{}",
        o.all()
    );
}

#[test]
fn after_the_humans_release_a_spent_file_has_fresh_rounds() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    for n in 3..6 {
        ask_old(&fx, &wc, n);
        judge(
            &fx,
            &wc,
            json!([retry_item("tests/old_test.rs", "keep both old assertions")]),
        );
    }
    fx.human("Release it, please, and carry on.");
    // The release gives fresh rounds: the request is taken again.
    fx.must(&["unit", "release", "U-1", "--quote", "Release it, please"]);
    let o = ask_old(&fx, &wc, 6);
    assert_eq!(
        o.code,
        0,
        "after a release the file has fresh rounds: {}",
        o.all()
    );
}

#[test]
fn a_request_with_needed_and_retry_files_grants_nothing_and_the_needed_file_is_asked_again() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    let docs = "--- a/docs/guide.md\n+++ b/docs/guide.md\n@@ -1 +1,2 @@\n guide\n+more\n";
    let o = ask(
        &fx,
        &wc,
        json!([
            entry("docs/guide.md", "R-1: the doc", docs),
            entry("tests/old_test.rs", "R-1: breaks", &patch_n(3))
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = judge(
        &fx,
        &wc,
        json!([
            needed("docs/guide.md"),
            retry_item("tests/old_test.rs", "keep both old assertions")
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["granted"], json!([]));
    assert_eq!(state_of(&fx), "blocked");
    let d = directive(&fx, "U-1", "builder").unwrap();
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(
        brief.contains("docs/guide.md") && brief.contains("judged needed"),
        "{brief}"
    );
    let o = ask(
        &fx,
        &wc,
        json!([
            entry("docs/guide.md", "R-1: the doc", docs),
            entry("tests/old_test.rs", "R-1: breaks", &patch_n(4))
        ]),
    );
    assert_eq!(
        o.code,
        0,
        "the needed file may be asked with the same patch: {}",
        o.all()
    );
    let o = judge(
        &fx,
        &wc,
        json!([needed("docs/guide.md"), needed("tests/old_test.rs")]),
    );
    assert_eq!(
        o.json()["granted"],
        json!(["docs/guide.md", "tests/old_test.rs"])
    );
}

#[test]
fn needed_extension_and_retry_grant_the_needed_file_count_no_round_and_go_to_the_human() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    let docs = "--- a/docs/guide.md\n+++ b/docs/guide.md\n@@ -1 +1,2 @@\n guide\n+more\n";
    let c = "--- a/src/c.rs\n+++ b/src/c.rs\n@@ -1 +1,2 @@\n fn c() {}\n+fn d() {}\n";
    ask(
        &fx,
        &wc,
        json!([
            entry("docs/guide.md", "R-1", docs),
            entry("src/c.rs", "R-1", c),
            entry("tests/old_test.rs", "R-1", &patch_n(3))
        ]),
    );
    let ext = json!({"path": "src/c.rs", "verdict": "extension", "why": "another feature"});
    let o = judge(
        &fx,
        &wc,
        json!([
            needed("docs/guide.md"),
            ext,
            retry_item("tests/old_test.rs", "keep both old assertions")
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["granted"], json!(["docs/guide.md"]));
    assert_eq!(refused_words(&fx), vec!["extension"]);
    let st = bf::events::replay(&fx.project()).unwrap();
    let files = st.scope["U-1"].refused[0]["files"].to_string();
    assert!(
        files.contains("keep both old assertions") && files.contains("\"retry\""),
        "{files}"
    );
    assert!(actions(&fx).contains(&"decide-unit".to_string()));
    assert!(!actions(&fx).contains(&"reconsider".to_string()));
    let refusals = st.scope["U-1"].refused.clone();
    assert_eq!(
        bf::scope::retry::rounds_used(&refusals, "tests/old_test.rs"),
        0
    );
}

#[test]
fn the_same_patch_again_and_a_file_in_the_list_are_each_refused_naming_the_file() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    ask_old(&fx, &wc, 3);
    judge(
        &fx,
        &wc,
        json!([retry_item("tests/old_test.rs", "keep both old assertions")]),
    );
    let o = ask_old(&fx, &wc, 3);
    assert_ne!(o.code, 0);
    assert!(
        o.all().contains("tests/old_test.rs") && o.all().contains("the same"),
        "{}",
        o.all()
    );
    assert_eq!(state_of(&fx), "blocked");
    let o = ask(
        &fx,
        &wc,
        json!([entry(
            "src/a.rs",
            "R-1",
            "--- a\n+++ b\n@@ -1 +1 @@\n-a\n+b\n"
        )]),
    );
    assert_ne!(o.code, 0);
    assert!(o.all().contains("src/a.rs"), "{}", o.all());
}

#[test]
fn a_retry_without_a_must_and_an_unknown_verdict_are_refused() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    ask_old(&fx, &wc, 3);
    for bad in [
        json!({"path": "tests/old_test.rs", "verdict": "retry", "why": "x"}),
        json!({"path": "tests/old_test.rs", "verdict": "retry", "why": "x", "must": "  "}),
        json!({"path": "tests/old_test.rs", "verdict": "maybe", "why": "x", "must": "y"}),
    ] {
        let o = judge_plain(&fx, &wc, json!([bad]));
        assert_ne!(o.code, 0, "{}", o.all());
        assert!(o.all().contains("tests/old_test.rs"), "{}", o.all());
    }
    assert!(directive(&fx, "U-1", "reviewer").is_some());
}

#[test]
fn every_new_text_is_a_key_of_the_strings_file() {
    let text = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/strings/en/scope_retry.toml"
    ))
    .unwrap();
    for key in [
        "scope.retry.verdict.must",
        "scope.retry.request.spent",
        "scope.retry.request.same_patch",
        "scope.retry.brief.file",
        "scope.retry.review.question",
        "scope.retry.row.title",
        "scope.retry.row.spent_title",
    ] {
        assert!(text.contains(&format!("\"{key}\"")), "{key}");
    }
}
