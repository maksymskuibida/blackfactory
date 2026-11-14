//! `bf checkpoint sync`: the human has the open checkpoint at the tip of the stack take in the
//! repository's local main branch, at a quiet point.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn plan(fx: &Fx, repo: &str) {
    let r = if repo.is_empty() {
        String::new()
    } else {
        format!("repo=\"{repo}\"\n")
    };
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\n{r}checkpoint=\"K-1\"\n")),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"ls src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    fx.commit_all("records");
}

fn real_run(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
}

fn land_unit(fx: &Fx) {
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

/// A commit by the human on the repository's main branch.
fn human_commit(repo: &Path, file: &str, text: &str) {
    fs::create_dir_all(repo.join(file).parent().unwrap()).unwrap();
    fs::write(repo.join(file), text).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-q", "-m", "work on main"]);
}

fn tip(repo: &Path, branch: &str) -> String {
    git(repo, &["rev-parse", branch])
}

fn walk(fx: &Fx, code: &Path, repo_name: &str) {
    real_run(fx);
    let sync = |q: &str| fx.bf(&["checkpoint", "sync", "K-1", "--quote", q]);
    // A unit is between start and landing: refused, naming it.
    fx.must(&["unit", "start", "U-1"]);
    fx.human("Yes, bring main in now.");
    let o = sync("bring main in");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("U-1 (running)"), "{}", o.stderr);
    // Finish the unit; the checkpoint branch is open and the stack's tip.
    let wc = fx.work("U-1");
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
    let o = sync("bring main in");
    assert_eq!(o.code, 2, "a reviewed unit is not landed: {}", o.all());
    fx.must(&["unit", "land", "U-1"]);
    // Nothing on main yet: nothing to take in, nothing changes.
    let events = fx.must(&["log", "--json"]).json().as_array().unwrap().len();
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--quote",
            "bring main in",
            "--json",
        ])
        .json();
    assert_eq!(o["nothing_to_take_in"], true, "{o}");
    assert_eq!(
        fx.must(&["log", "--json"]).json().as_array().unwrap().len(),
        events
    );
    // The human works on main: status says the stack lacks it and what to run; `bf next` carries it too.
    human_commit(code, "docs/extra.txt", "written on main\n");
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main_ahead"][0]["repo"], repo_name, "{st}");
    assert_eq!(st["main_ahead"][0]["checkpoint"], "K-1");
    assert_eq!(st["main_ahead"][0]["commits"], 1);
    assert!(fx
        .must(&["status"])
        .stdout
        .contains("bf checkpoint sync K-1"));
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["main_ahead"][0]["checkpoint"], "K-1", "{n}");
    assert!(
        n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["action"] != "sync"),
        "information, never a directive: {n}"
    );
    // A checkpoint receipt from before the sync exists; the sync makes it stop counting.
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "ls", "src/a.txt"],
    );
    // The words are needed: a human turn with the quote in it.
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--quote", "nobody said this"]);
    assert_eq!(o.code, 2);
    let before = tip(code, "bf/K-1");
    fx.human("Yes, take main into the first checkpoint.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--quote",
            "take main into the first checkpoint",
            "--json",
        ])
        .json();
    assert_eq!(o["synced"], true, "{o}");
    assert_ne!(tip(code, "bf/K-1"), before);
    assert!(git(code, &["ls-tree", "-r", "--name-only", "bf/K-1"]).contains("docs/extra.txt"));
    assert!(fx
        .must(&["log", "--json"])
        .stdout
        .contains("checkpoint.sync"));
    // The note goes quiet; a second sync is a no-op.
    let st = fx.must(&["status", "--json"]).json();
    assert!(st["main_ahead"].as_array().unwrap().is_empty(), "{st}");
    assert_eq!(st["repos"][0]["note"], "", "{st}");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--quote",
            "take main into",
            "--json",
        ])
        .json();
    assert_eq!(o["nothing_to_take_in"], true);
    // The checkpoint's own check is open again; the landed unit's receipts still count.
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(
        o.stderr.contains("no sealed passing receipt"),
        "{}",
        o.stderr
    );
    assert_eq!(fx.bf(&["evidence", "verify", "--unit", "U-1"]).code, 0);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "ls", "src/a.txt"],
    );
    let out = fx.merge_checkpoint("K-1");
    assert_eq!(out.code, 0, "{}", out.all());
    assert!(code.join("docs/extra.txt").exists() && code.join("src/a.txt").exists());
}

#[test]
fn in_a_project_folder_sync_at_a_quiet_point_then_close_and_merge() {
    let fx = Fx::new();
    plan(&fx, "");
    let root = fx.root.clone();
    walk(&fx, &root, "code");
}

#[test]
fn in_a_workspace_with_a_nested_repository_sync_then_close_and_merge() {
    let fx = Fx::workspace(&["app"]);
    let code = fx.root.join("code");
    fs::create_dir_all(&code).unwrap();
    git(&code, &["init", "-q", "-b", "main"]);
    fs::write(code.join("README.md"), "# code\n").unwrap();
    git(&code, &["add", "-A"]);
    git(&code, &["commit", "-q", "-m", "init"]);
    fx.must(&["repo", "add", "code", "code"]);
    plan(&fx, "code");
    walk(&fx, &code, "code");
}

#[test]
fn a_conflict_changes_nothing_and_a_hand_merge_is_recorded_with_resolved() {
    let fx = Fx::new();
    plan(&fx, "");
    real_run(&fx);
    land_unit(&fx);
    let root = fx.root.clone();
    // The human changes the same file on main differently.
    human_commit(&root, "src/a.txt", "from main\n");
    let before = tip(&root, "bf/K-1");
    fx.human("Yes, bring main in, and I will sort out any conflict.");
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--quote", "bring main in"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("src/a.txt"), "{}", o.stderr);
    assert!(o.stderr.contains("--resolved"), "{}", o.stderr);
    assert_eq!(tip(&root, "bf/K-1"), before, "the branch is as it was");
    assert_eq!(
        git(&fx.work("K-1"), &["status", "--porcelain"]),
        "",
        "no merge left half done"
    );
    assert!(!fx
        .must(&["log", "--json"])
        .stdout
        .contains("checkpoint.sync"));
    // `--resolved` before the merge is done says what to do.
    let o = fx.bf(&[
        "checkpoint",
        "sync",
        "K-1",
        "--resolved",
        "--quote",
        "bring main in",
    ]);
    assert_eq!(o.code, 2);
    assert!(
        o.stderr.contains("does not contain the main branch"),
        "{}",
        o.stderr
    );
    // The human merges by hand in the checkpoint's working copy and commits.
    let wt = fx.work("K-1");
    let m = Command::new("git")
        .args(["-C", wt.to_str().unwrap(), "merge", "--no-edit", "main"])
        .output()
        .unwrap();
    assert!(!m.status.success());
    fs::write(wt.join("src/a.txt"), "resolved by the human\n").unwrap();
    git(&wt, &["add", "src/a.txt"]);
    git(
        &wt,
        &["-c", "core.editor=true", "commit", "-q", "--no-edit"],
    );
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--resolved",
            "--quote",
            "bring main in",
            "--json",
        ])
        .json();
    assert_eq!(o["synced"], true, "{o}");
    assert!(fx
        .must(&["log", "--json"])
        .stdout
        .contains("checkpoint.sync"));
    let st = fx.must(&["status", "--json"]).json();
    assert!(st["main_ahead"].as_array().unwrap().is_empty(), "{st}");
    // Recorded once: running it again changes nothing.
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--resolved",
            "--quote",
            "bring main in",
            "--json",
        ])
        .json();
    assert_eq!(o["nothing_to_take_in"], true, "{o}");
    let _: Value = o;
}

#[test]
fn a_hand_merge_without_the_command_is_not_refused_and_its_receipts_go_stale() {
    // What happens today when the human merges main into the checkpoint branch by hand: nothing refuses;
    // the checkpoint's receipts are for the old content and stop counting; the log has no record.
    let fx = Fx::new();
    plan(&fx, "");
    real_run(&fx);
    land_unit(&fx);
    let root = fx.root.clone();
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "ls", "src/a.txt"],
    );
    human_commit(&root, "docs/extra.txt", "main\n");
    let wt = fx.work("K-1");
    git(&wt, &["merge", "--no-edit", "main"]);
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(
        o.stderr.contains("no sealed passing receipt"),
        "{}",
        o.stderr
    );
    assert!(!fx
        .must(&["log", "--json"])
        .stdout
        .contains("checkpoint.sync"));
    fx.must_in(&wt, &["evidence", "run", "X-2", "--", "ls", "src/a.txt"]);
    assert_eq!(fx.merge_checkpoint("K-1").code, 0);
}
