//! U-60: a symbolic link does not hide where a write ends up (R-90, D-64, TH-39). The guard follows a link
//! itself, an unresolved one too, and judges the place it leads to; it refuses the git plumbing that turns a
//! worker's file into a link; a scope request never grants a link. The smaller rules of the same review
//! (a path held in a variable, a version number printed, the case of a file a unit beside holds, equal
//! removed lines at landing) are here too.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

const OLD_TEST: &str = "fn t() {\n    assert_eq!(1, 1);\n    assert_eq!(2, 2);\n    assert_eq!(2, 2);\n}\n";

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// Two units on one checkpoint: U-1 lists src/a.rs; U-3 may run beside it and lists beside/y.txt.
fn project(fx: &Fx) {
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n"),
        ("requirement", "id=\"R-3\"\nkind=\"requirement\"\ntitle=\"C\"\nacceptance=[\"beside/y.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ntext=\"Build a.\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\n"),
        ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Part C\"\ndelivers=[\"R-3\"]\nfiles=[\"beside/y.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"C\"\nhow=\"command\"\ncommand=\"test -f beside/y.txt\"\nproves=[\"R-3\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\",\"R-3\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"All\"\nafter=[\"U-1\",\"U-3\"]\nproves=[\"R-1\",\"R-3\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    write(&fx.root, "src/a.rs", "fn a() {}\n");
    write(&fx.root, "src/other.rs", "fn o() {}\n");
    write(&fx.root, "tests/old_test.rs", OLD_TEST);
    write(&fx.root, "beside/y.txt", "y\n");
    write(&fx.root, ".claude/settings.json", "{}\n");
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

/// The guard's answer to a tool call: "pass", or "deny" with its reason.
fn judge(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> (String, String) {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    match o.stdout.trim() {
        "" | "{}" => ("pass".into(), String::new()),
        t => {
            let v: Value = serde_json::from_str(t).unwrap();
            let h = &v["hookSpecificOutput"];
            (
                h["permissionDecision"].as_str().unwrap_or("?").to_string(),
                h["permissionDecisionReason"].as_str().unwrap_or("").to_string(),
            )
        }
    }
}

fn bash(fx: &Fx, cwd: &Path, cmd: &str) -> (String, String) {
    judge(fx, cwd, "Bash", json!({"command": cmd}))
}

fn denied(fx: &Fx, cwd: &Path, cmd: &str) -> String {
    let (d, r) = bash(fx, cwd, cmd);
    assert_eq!(d, "deny", "{cmd}: {r}");
    r
}

fn passes(fx: &Fx, cwd: &Path, cmd: &str) {
    let (d, r) = bash(fx, cwd, cmd);
    assert_eq!(d, "pass", "{cmd}: {r}");
}

fn make_link(wc: &Path, rel: &str, target: &str) {
    let at = wc.join(rel);
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    let _ = fs::remove_file(&at);
    symlink(target, &at).unwrap();
}

// ---- the route around the guard, step by step

#[test]
fn step_one_writing_the_object_store_alone_is_harmless() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    let target = format!("{}/.claude/settings.local.json", fx.root.display());
    passes(&fx, &wc, &format!("printf %s {target} | git hash-object -w --stdin"));
}

#[test]
fn step_two_a_link_made_with_git_plumbing_is_refused() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    let blob = "0123456789abcdef0123456789abcdef01234567";
    for cmd in [
        format!("git update-index --add --cacheinfo 120000,{blob},src/a.rs"),
        format!("git update-index --add --cacheinfo 120000 {blob} src/a.rs"),
        format!("git update-index --add --cacheinfo 0120000,{blob},src/a.rs"),
        format!("git update-index --add --cacheinfo 120000,{blob},src/a.rs && git checkout-index -f src/a.rs"),
        "git update-index --index-info".to_string(),
        "git update-index --index-inf".to_string(),
    ] {
        let r = denied(&fx, &wc, &cmd);
        assert!(r.contains("update-index"), "{cmd}: {r}");
    }
}

#[test]
fn step_two_the_paths_the_plumbing_writes_are_held_to_the_scope() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    let blob = "0123456789abcdef0123456789abcdef01234567";
    // A named file of the copy passes whether or not the unit lists it.
    passes(&fx, &wc, &format!("git update-index --add --cacheinfo 100644,{blob},src/other.rs"));
    passes(&fx, &wc, "git checkout-index -f src/other.rs");
    for cmd in [
        "git checkout-index -a",
        "git checkout-index -fa",
        "git checkout-index --all",
        "git checkout-index --stdin",
        "git checkout-index --prefix=out/ src/a.rs",
    ] {
        denied(&fx, &wc, cmd);
    }
    // A file the unit lists, a normal mode, and the object store alone pass.
    passes(&fx, &wc, &format!("git update-index --add --cacheinfo 100644,{blob},src/a.rs"));
    passes(&fx, &wc, "git checkout-index -f src/a.rs");
}

#[test]
fn the_plumbing_is_judged_in_a_checkpoint_working_copy_too() {
    let fx = Fx::new();
    project(&fx);
    let blob = "0123456789abcdef0123456789abcdef01234567";
    // The main checkout is not held to a list: only the places the guard protects count there.
    let r = denied(&fx, &fx.root, &format!("git update-index --add --cacheinfo 100644,{blob},factory/policy.toml"));
    assert!(r.contains("factory/policy.toml"), "{r}");
    // A checkpoint's working copy exists once a unit of it has landed.
    fx.build_unit("U-1", "src/a.rs", "X-1", &["test", "-f", "src/a.rs"]);
    let kwc = fx.work("K-1");
    assert!(kwc.join(".git").exists(), "no checkpoint working copy at {}", kwc.display());
    for cmd in [
        format!("git update-index --add --cacheinfo 120000,{blob},src/a.rs"),
        format!("git update-index --add --cacheinfo 120000 {blob} src/a.rs"),
        "git update-index --index-info".to_string(),
        "git checkout-index -a".to_string(),
        "git checkout-index -fa".to_string(),
        "git checkout-index --all".to_string(),
    ] {
        denied(&fx, &kwc, &cmd);
    }
}

#[test]
fn step_three_a_write_through_a_dangling_link_is_judged_where_the_link_leads() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    let root = fx.root.display().to_string();
    assert!(!fx.root.join(".claude/settings.local.json").exists());
    let wr = |rel: &str| {
        judge(&fx, &wc, "Write", json!({"file_path": wc.join(rel).to_string_lossy(), "content": "{}"}))
    };
    // Absolute, to the hook settings that do not exist yet.
    make_link(&wc, "src/a.rs", &format!("{root}/.claude/settings.local.json"));
    let (d, r) = wr("src/a.rs");
    assert_eq!(d, "deny", "{r}");
    assert!(r.contains(".claude/settings.local.json"), "{r}");
    // Relative, taken from the folder that holds the link: src, the copy, work, .bf, the project.
    make_link(&wc, "src/a.rs", "../../../../.claude/settings.local.json");
    let (d, r) = wr("src/a.rs");
    assert_eq!(d, "deny", "{r}");
    assert!(r.contains(".claude/settings.local.json"), "{r}");
    // A dangling link to a factory record that does not exist.
    make_link(&wc, "src/a.rs", &format!("{root}/factory/records/R-99.toml"));
    let (d, r) = wr("src/a.rs");
    assert_eq!(d, "deny", "{r}");
    assert!(r.contains("factory/records/R-99.toml"), "{r}");
    // Link after link.
    make_link(&wc, "src/step2", &format!("{root}/.claude/settings.local.json"));
    make_link(&wc, "src/step1", "step2");
    make_link(&wc, "src/a.rs", "step1");
    let (d, r) = wr("src/a.rs");
    assert_eq!(d, "deny", "{r}");
    assert!(r.contains(".claude/settings.local.json"), "{r}");
    // A folder that is a dangling link to a protected folder.
    make_link(&wc, "src/dir", &format!("{root}/.claude"));
    let (d, r) = wr("src/dir/settings.local.json");
    assert_eq!(d, "deny", "{r}");
    assert!(r.contains(".claude/settings.local.json"), "{r}");
    // The same through the shell, and through an Edit.
    make_link(&wc, "src/a.rs", &format!("{root}/.claude/settings.local.json"));
    let r = denied(&fx, &wc, "echo x > src/a.rs");
    assert!(r.contains(".claude/settings.local.json"), "{r}");
    let (d, r) = judge(
        &fx,
        &wc,
        "Edit",
        json!({"file_path": wc.join("src/a.rs").to_string_lossy(), "old_string": "a", "new_string": "b"}),
    );
    assert_eq!(d, "deny", "{r}");
}

#[test]
fn a_dangling_link_that_leads_outside_the_scope_is_a_write_outside_it() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    // Inside the copy but not in the unit's list: the list is a recommendation, so the write passes.
    make_link(&wc, "src/a.rs", "other_missing.rs");
    let (d, r) = judge(&fx, &wc, "Write", json!({"file_path": wc.join("src/a.rs").to_string_lossy(), "content": "x"}));
    assert_eq!(d, "pass", "{r}");
    // A link to another place in the project.
    make_link(&wc, "src/a.rs", &format!("{}/src/new_file.rs", fx.root.display()));
    let (d, r) = judge(&fx, &wc, "Write", json!({"file_path": wc.join("src/a.rs").to_string_lossy(), "content": "x"}));
    assert_eq!(d, "deny", "{r}");
    // A link whose target is a file of the copy that the unit does not list passes too.
    make_link(&wc, "src/a.rs", "../src/a_copy.rs");
    let (d, r) = judge(&fx, &wc, "Write", json!({"file_path": wc.join("src/a.rs").to_string_lossy(), "content": "x"}));
    assert_eq!(d, "pass", "{r}");
    // A link loop ends at the bound and is not followed for ever; it stays inside the copy.
    make_link(&wc, "src/a.rs", "loop_b");
    make_link(&wc, "loop_b", "src/a.rs");
    let (d, r) = judge(&fx, &wc, "Write", json!({"file_path": wc.join("src/a.rs").to_string_lossy(), "content": "x"}));
    assert_eq!(d, "pass", "{r}");
}

// ---- a scope request never grants a link

fn ask(fx: &Fx, wc: &Path, list: Value) -> Out {
    fx.bf_stdin(
        wc,
        &["report", "U-1", "--result", "blocked", "--need-files", "-", "--json"],
        &list.to_string(),
    )
}

#[test]
fn a_scope_request_for_a_link_or_a_path_under_one_is_a_hard_refusal() {
    for path in ["tests/lnk.rs", "linkdir/a.rs"] {
        let fx = Fx::new();
        project(&fx);
        let wc = start(&fx);
        let root = fx.root.display().to_string();
        make_link(&wc, "tests/lnk.rs", &format!("{root}/.claude/settings.local.json"));
        make_link(&wc, "linkdir", "src");
        let o = ask(&fx, &wc, json!([{"path": path, "why": "R-1", "patch": "--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n"}]));
        assert_eq!(o.code, 0, "{path}: {}", o.all());
        let v = o.json();
        assert_eq!(v["state"], "blocked", "{path}: {v}");
        let r = v["refused"][0].to_string();
        assert!(r.contains("symbolic link"), "{path}: {r}");
        let st = fx.must(&["status", "--json"]).json();
        let u = st["units"].as_array().unwrap().iter().find(|u| u["id"] == "U-1").cloned().unwrap();
        assert_eq!(u["scope_request"], Value::Null, "{path}: {u}");
    }
    // A plain file is not refused for being a file.
    let fx = Fx::new();
    let wc = project_with_beside(&fx);
    let o = ask(&fx, &wc, json!([{"path": "tests/old_test.rs", "why": "R-1", "patch": "--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n"}]));
    assert!(!o.all().contains("symbolic link"), "{}", o.all());
}

// ---- the smaller rules of the same review

#[test]
fn a_path_held_in_a_variable_is_judged_for_os_replace_and_truncate_sync() {
    let fx = Fx::new();
    project(&fx);
    let _wc = start(&fx);
    for cmd in [
        "python3 -c \"import os; p='factory/policy.toml'; os.replace('/tmp/x', p)\"",
        "python3 -c \"import os; p='.claude/settings.json'; os.replace('/tmp/x', p)\"",
        "python3 -c \"import os; p='.bf/guard.json'; os.replace('/tmp/x', p)\"",
        "node -e \"const p='.claude/settings.json'; require('fs').truncateSync(p)\"",
        "node -e \"const p='factory/policy.toml'; require('fs').truncateSync(p)\"",
    ] {
        denied(&fx, &fx.root, cmd);
    }
    // Honest neighbours stay open.
    passes(&fx, &fx.root, "python3 -c \"import os; os.replace('/tmp/k-a.txt', '/tmp/k-b.txt')\"");
    passes(&fx, &fx.root, "python3 -c \"s='a-b'; print(s.replace('-', '_'))\"");
}

#[test]
fn a_version_number_is_not_a_path_beside_a_delete_through_a_variable() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx);
    passes(&fx, &wc, "python3 -c \"import os; print('v1.2'); p='src/a.rs'; os.remove(p)\"");
    passes(&fx, &wc, "python3 -c \"import os; p='src/a.rs'; print('ok.'); os.remove(p)\"");
    passes(&fx, &wc, "python3 -c \"import os; print('bf 0.2.1'); os.remove('src/a.rs')\"");
    passes(&fx, &wc, "python3 -c \"print('v1.2')\"");
    // A name with a letter after the dot is still a path, and one handed to a file call is too.
    passes(&fx, &wc, "python3 -c \"import os; p='src/a.rs'; q='src/other.rs'; os.remove(q)\"");
    passes(&fx, &wc, "python3 -c \"import os; p='src/a.rs'; os.remove('v1.2')\"");
    // A protected path is refused.
    denied(&fx, &wc, "python3 -c \"import os; p='src/a.rs'; q='factory/policy.toml'; os.remove(q)\"");
}

fn project_with_beside(fx: &Fx) -> PathBuf {
    project(fx);
    start(fx)
}

fn judge_scope(fx: &Fx, wc: &Path, list: Value) -> Out {
    fx.bf_stdin(
        wc,
        &["report", "U-1", "--role", "reviewer", "--result", "scope", "--verdict", "-", "--json"],
        &list.to_string(),
    )
}

#[test]
fn a_case_variant_of_a_file_a_unit_beside_holds_is_refused() {
    let fx = Fx::new();
    let wc = project_with_beside(&fx);
    // A holder that has not started is put after the asker (A-73); the refusal is for one that has.
    fx.must(&["unit", "start", "U-3"]);
    let o = ask(&fx, &wc, json!([{"path": "beside/Y.TXT", "why": "R-1", "patch": "--- a\n+++ b\n@@ -1 +1 @@\n-y\n+z\n"}]));
    assert_eq!(o.code, 0, "{}", o.all());
    let v = o.json();
    assert_eq!(v["state"], "blocked", "{v}");
    let r = v["refused"][0].to_string();
    assert!(r.contains("U-3 holds it"), "{r}");
}

#[test]
fn a_file_the_unit_already_holds_is_found_whatever_its_case() {
    let fx = Fx::new();
    let wc = project_with_beside(&fx);
    let o = ask(&fx, &wc, json!([{"path": "SRC/A.RS", "why": "R-1", "patch": "--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n"}]));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.all().contains("already inside the unit's files"), "{}", o.all());
}

#[test]
fn landing_counts_each_removed_line_of_a_granted_test_file() {
    let fx = Fx::new();
    let wc = project_with_beside(&fx);
    let patch = "--- a/tests/old_test.rs\n+++ b/tests/old_test.rs\n@@ -3,1 +3,0 @@\n-    assert_eq!(2, 2);\n";
    let o = ask(&fx, &wc, json!([{"path": "tests/old_test.rs", "why": "R-1", "patch": patch}]));
    assert_eq!(o.code, 0, "{}", o.all());
    let o = judge_scope(&fx, &wc, json!([{"path": "tests/old_test.rs", "verdict": "needed", "why": "the change needs it"}]));
    assert_eq!(o.code, 0, "{}", o.all());
    // The patch removes one of two equal lines; the builder removes both.
    write(&wc, "tests/old_test.rs", "fn t() {\n    assert_eq!(1, 1);\n}\n");
    write(&wc, "src/a.rs", "fn a() { 1; }\n");
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.rs"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(
        o.stderr.contains("tests/old_test.rs") && o.stderr.contains("assert_eq!(2, 2);"),
        "{}",
        o.stderr
    );
}
