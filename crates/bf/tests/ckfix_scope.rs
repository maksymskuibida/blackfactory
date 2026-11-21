//! R-70, D-91: a checkpoint fix asks the scope reviewer for more files, is held to a list read live, names
//! its files in a gap, and an abort saves its work for the next fix of the checkpoint.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const K_CHECK: &str = "grep -q fixed src/a.txt";

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
    fs::write(fx.root.join("tests/common/mod.rs"), "pub fn helper() {}\n").unwrap();
    fs::write(fx.root.join("tests/attacks.rs"), "// attacks\nfn a() {}\n").unwrap();
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

fn failing_run(fx: &Fx) -> PathBuf {
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
    k
}

fn one_directive(fx: &Fx, action: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "one directive while a fix is open: {n}");
    assert_eq!(ds[0]["action"], action, "{n}");
    ds[0].clone()
}

fn guard_says(fx: &Fx, cwd: &Path, file: &str) -> String {
    let payload = json!({"tool_name": "Write",
        "tool_input": {"file_path": cwd.join(file).to_string_lossy()}, "cwd": cwd.to_string_lossy()});
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

fn answer(fx: &Fx, _k: &Path, v: Value) -> Out {
    fx.bf_stdin(
        &fx.root,
        &["checkpoint", "scope", "K-1", "--verdict", "-"],
        &v.to_string(),
    )
}

fn patch_for(path: &str, add: &str) -> String {
    format!("--- a/{path}\n+++ b/{path}\n@@ -1 +1,2 @@\n context\n+{add}\n")
}

fn open_fix(fx: &Fx) -> PathBuf {
    plan(fx);
    land_unit(fx);
    let k = failing_run(fx);
    one_directive(fx, "fix-checkpoint");
    k
}

#[test]
fn ckfix_scope_a_fix_that_needs_two_existing_test_files_is_granted_with_no_amendment_and_no_human()
{
    let fx = Fx::new();
    let k = open_fix(&fx);
    // The list is only a recommendation now: the guard allows the write before any grant.
    assert_ne!(guard_says(&fx, &k, "tests/attacks.rs"), "deny");
    let o = ask(
        &fx,
        &k,
        json!([
            {"path": "tests/common/mod.rs", "why": "the helper the fix calls", "patch": patch_for("tests/common/mod.rs", "pub fn more() {}")},
            {"path": "tests/attacks.rs", "why": "the attack the fix changes", "patch": patch_for("tests/attacks.rs", "fn b() {}")},
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The scope reviewer is asked, not the human.
    let d = one_directive(&fx, "review-scope");
    assert_eq!(d["role"], "reviewer", "{d}");
    assert!(d["files"].to_string().contains("tests/attacks.rs"), "{d}");
    let brief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "reviewer"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        brief.contains("tests/common/mod.rs") && brief.contains("pub fn more()"),
        "{brief}"
    );
    let o = answer(
        &fx,
        &k,
        json!([
            {"path": "tests/common/mod.rs", "verdict": "needed", "why": "the fix calls it"},
            {"path": "tests/attacks.rs", "verdict": "needed", "why": "the fix changes it"},
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(
        d["may_change"].to_string().contains("tests/attacks.rs"),
        "{d}"
    );
    assert_eq!(d["granted"].as_array().unwrap().len(), 2, "{d}");
    assert_ne!(guard_says(&fx, &k, "tests/attacks.rs"), "deny");
    assert_ne!(guard_says(&fx, &k, "tests/common/mod.rs"), "deny");
    assert_ne!(guard_says(&fx, &k, "src/other.txt"), "deny");
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    assert_eq!(bf::fixscope::granted(&st, "K-1").len(), 2);
    assert!(st.amends.is_empty());
    // The grant holds for this fix only: after an abort the next fix does not have it.
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(
        !d["may_change"].to_string().contains("tests/attacks.rs"),
        "{d}"
    );
    assert!(d["granted"].as_array().unwrap().is_empty(), "{d}");
}

#[test]
fn ckfix_scope_a_request_for_a_file_already_in_the_list_or_with_no_patch_is_refused() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let o = ask(
        &fx,
        &k,
        json!([{"path": "src/a.txt", "why": "w", "patch": "p"}]),
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("src/a.txt") && o.stderr.contains("already inside"),
        "{}",
        o.stderr
    );
    let o = ask(&fx, &k, json!([{"path": "tests/attacks.rs", "why": "w"}]));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("no patch"), "{}", o.stderr);
    one_directive(&fx, "fix-checkpoint");
}

#[test]
fn ckfix_scope_a_hard_rule_file_goes_to_the_human_as_a_plan_gap() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let o = ask(
        &fx,
        &k,
        json!([{"path": ".git/config", "why": "w", "patch": "p"}]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "plan-gap");
    assert_eq!(d["role"], "human", "{d}");
}

#[test]
fn ckfix_scope_an_extension_goes_to_the_human_and_grants_nothing() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    ask(
        &fx,
        &k,
        json!([{"path": "tests/attacks.rs", "why": "w", "patch": patch_for("tests/attacks.rs", "x")}]),
    );
    let o = answer(
        &fx,
        &k,
        json!([{"path": "tests/attacks.rs", "verdict": "extension", "why": "another feature"}]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The fix is back at work; the way on is a gap report that names the file, which goes to the human.
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d["not_granted"].to_string().contains("tests/attacks.rs"), "{d}");
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(bf::fixscope::granted(&st, "K-1").is_empty());
    let o = fx.bf_in(
        &k,
        &[
            "report", "K-1", "--role", "builder", "--result", "gap", "--note",
            "tests/attacks.rs: another feature",
        ],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    let d = one_directive(&fx, "plan-gap");
    assert!(d["missing"].to_string().contains("tests/attacks.rs"), "{d}");
}

#[test]
fn ckfix_scope_a_retry_is_asked_again_with_a_corrected_patch_until_the_rounds_are_spent() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let first = patch_for("tests/attacks.rs", "one");
    ask(
        &fx,
        &k,
        json!([{"path": "tests/attacks.rs", "why": "w", "patch": first}]),
    );
    let o = answer(
        &fx,
        &k,
        json!([{"path": "tests/attacks.rs", "verdict": "retry", "why": "removes a line", "must": "show the removed line"}]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d["granted"].as_array().unwrap().is_empty(), "{d}");
    let b = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        b.contains("show the removed line") && b.contains("tests/attacks.rs"),
        "{b}"
    );
    // The same patch is refused, naming the file; a corrected one is judged.
    let o = ask(
        &fx,
        &k,
        json!([{"path": "tests/attacks.rs", "why": "w", "patch": first}]),
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("tests/attacks.rs") && o.stderr.contains("same as the one"),
        "{}",
        o.stderr
    );
    let max = fx.project().policy.limits.max_rounds;
    for round in 2..=max {
        let o = ask(
            &fx,
            &k,
            json!([{"path": "tests/attacks.rs", "why": "w", "patch": patch_for("tests/attacks.rs", &format!("r{round}"))}]),
        );
        assert_eq!(o.code, 0, "{}", o.all());
        one_directive(&fx, "review-scope");
        let o = answer(
            &fx,
            &k,
            json!([{"path": "tests/attacks.rs", "verdict": "retry", "why": "still short", "must": "do more"}]),
        );
        assert_eq!(o.code, 0, "{}", o.all());
    }
    // The last round's retry leaves the fix at work, and the file cannot be asked for again.
    one_directive(&fx, "fix-checkpoint");
    let o = ask(
        &fx,
        &k,
        json!([{"path": "tests/attacks.rs", "why": "w", "patch": patch_for("tests/attacks.rs", "again")}]),
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("tests/attacks.rs") && o.stderr.contains("spent"),
        "{}",
        o.stderr
    );
}

#[test]
fn ckfix_scope_a_gap_names_its_files_and_a_file_of_the_list_is_refused() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let gap = |note: &str| {
        fx.bf_in(
            &k,
            &[
                "report", "K-1", "--role", "builder", "--result", "gap", "--note", note,
            ],
        )
    };
    let o = gap("needs behaviour no record describes");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("names none"), "{}", o.stderr);
    let o = gap("src/a.txt needs more behaviour");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("src/a.txt is already in the files"),
        "{}",
        o.stderr
    );
    // A place in the file (`:12`, `:12:3`) is not part of its path.
    for note in [
        "src/a.txt:12 needs more behaviour",
        "see src/a.txt:12:3, it needs more",
    ] {
        let o = gap(note);
        assert_eq!(o.code, 2, "{}", o.all());
        assert!(
            o.stderr.contains("src/a.txt is already in the files"),
            "{}",
            o.stderr
        );
    }
    one_directive(&fx, "fix-checkpoint");
    let o = gap("src/new_module.txt: behaviour no record describes");
    assert_eq!(o.code, 2, "{}", o.all());
    let d = one_directive(&fx, "plan-gap");
    assert!(
        d["missing"].to_string().contains("src/new_module.txt"),
        "{d}"
    );
}

#[test]
fn ckfix_scope_an_abort_saves_the_work_and_the_next_fix_starts_from_it() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "half done\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "half"]);
    fs::write(k.join("src/other.txt"), "outside\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "outside"]);
    fs::create_dir_all(k.join("tests")).unwrap();
    fs::write(k.join("tests/new_test.txt"), "a new test\n").unwrap();
    let v = fx
        .must(&["checkpoint", "fix", "K-1", "--abort", "--json"])
        .json();
    let saved = v["saved"].as_str().unwrap().to_string();
    assert!(saved.starts_with("factory/evidence/fix-saves/K-1/"), "{v}");
    assert!(fx.root.join(&saved).join("saved.json").is_file());
    assert_eq!(
        fs::read_to_string(k.join("src/a.txt")).unwrap(),
        "from the unit\n"
    );
    assert!(!k.join("tests/new_test.txt").exists());
    // The failing check opens a new fix, which starts from the saved work.
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["from_saved"], saved.as_str(), "{d}");
    let note = fs::read_to_string(fx.root.join(&saved).join("reopened.json")).unwrap_or_default();
    assert_eq!(
        fs::read_to_string(k.join("src/a.txt")).unwrap(),
        "half done\n",
        "{note}"
    );
    assert_eq!(
        fs::read_to_string(k.join("tests/new_test.txt")).unwrap(),
        "a new test\n"
    );
    assert!(!k.join("src/other.txt").exists());
    let b = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        b.contains(&saved) && b.contains("src/other.txt") && b.contains("outside the files"),
        "{b}"
    );
    // Only the next fix starts from it: an abort with nothing to save saves nothing.
    git(&k, &["add", "-A"]);
    git(&k, &["stash", "-q", "-u"]);
    let v = fx
        .must(&["checkpoint", "fix", "K-1", "--abort", "--json"])
        .json();
    assert_eq!(v["saved"], "", "{v}");
}
