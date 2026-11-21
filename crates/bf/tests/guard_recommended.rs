//! U-98: a unit's and a checkpoint fix's file list is a recommendation (D-115, D-116). The guard lets a
//! builder write any file of its own working copy that is not protected; every other refusal stays.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn says(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> String {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
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

fn write_tool(fx: &Fx, cwd: &Path, path: &Path) -> String {
    says(fx, cwd, "Write", json!({"file_path": path.to_string_lossy(), "content": "x"}))
}

fn bash(fx: &Fx, cwd: &Path, cmd: &str) -> String {
    says(fx, cwd, "Bash", json!({"command": cmd}))
}

fn unit_place() -> (Fx, PathBuf) {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    (fx, wc)
}

#[test]
fn a_unit_may_write_a_file_its_list_does_not_hold() {
    let (fx, wc) = unit_place();
    for rel in ["src/other.txt", "README.md", "lib/deep/new.rs", "docs/x.md"] {
        assert_ne!(write_tool(&fx, &wc, &wc.join(rel)), "deny", "{rel}");
    }
    for cmd in ["echo x > src/other.txt", "touch README.md", "mkdir -p lib && echo x > lib/a.rs"] {
        assert_ne!(bash(&fx, &wc, cmd), "deny", "{cmd}");
    }
    let edit = json!({"file_path": wc.join("src/other.txt").to_string_lossy(), "old_string": "a", "new_string": "b"});
    assert_ne!(says(&fx, &wc, "Edit", edit), "deny");
}

#[test]
fn protected_paths_stay_refused_in_a_unit() {
    let (fx, wc) = unit_place();
    for rel in [
        "factory/policy.toml",
        "factory/records/R-1.toml",
        "factory/log/events.jsonl",
        "factory/evidence/x",
        ".git/hooks/pre-commit",
        ".git/config",
    ] {
        assert_eq!(write_tool(&fx, &wc, &wc.join(rel)), "deny", "{rel}");
    }
    for cmd in ["echo x > factory/policy.toml", "rm factory/records/R-1.toml", "echo x > .git/config"] {
        assert_eq!(bash(&fx, &wc, cmd), "deny", "{cmd}");
    }
}

#[test]
fn writes_outside_its_own_copy_stay_refused() {
    let (fx, wc) = unit_place();
    // The main checkout, and a folder of working copies.
    assert_eq!(write_tool(&fx, &wc, &fx.root.join("src/other.txt")), "deny");
    assert_eq!(write_tool(&fx, &wc, &fx.root.join(".bf/work/U-9/src/a.txt")), "deny");
    assert_eq!(bash(&fx, &wc, &format!("echo x > {}/src/other.txt", fx.root.display())), "deny");
    // The engine's state folder and the harness settings.
    assert_eq!(write_tool(&fx, &wc, &fx.root.join(".claude/settings.json")), "deny");
    assert_eq!(write_tool(&fx, &wc, &fx.root.join(".bf/drafts/x.md")), "deny");
}

#[test]
fn a_link_and_a_write_without_a_name_stay_refused() {
    let (fx, wc) = unit_place();
    fs::create_dir_all(wc.join("src")).unwrap();
    let target = fx.root.join("factory/policy.toml");
    symlink(&target, wc.join("src/link.txt")).unwrap();
    assert_eq!(write_tool(&fx, &wc, &wc.join("src/link.txt")), "deny");
    let blob = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(bash(&fx, &wc, "git update-index --index-info"), "deny");
    assert_eq!(bash(&fx, &wc, "git checkout-index -a"), "deny");
    assert_ne!(bash(&fx, &wc, &format!("git update-index --add --cacheinfo 100644,{blob},src/other.txt")), "deny");
}

#[test]
fn every_git_refusal_stays() {
    let (fx, wc) = unit_place();
    for cmd in [
        "git push",
        "git merge main",
        "git worktree add ../x",
        "git update-ref refs/heads/x HEAD",
        "git rebase main",
        "git cherry-pick abc",
        "git reset --hard main",
        "git checkout main",
        "git checkout main -- src/hello.txt",
        "git switch main",
        "git checkout HEAD~3",
        "git checkout abc1234",
        "git checkout -",
        "git checkout -b x HEAD~5",
        "git checkout --detach",
    ] {
        assert_eq!(bash(&fx, &wc, cmd), "deny", "{cmd}");
    }
}

#[test]
fn the_hook_commands_and_the_human_turn_stay_refused() {
    let (fx, wc) = unit_place();
    for cmd in ["bf human-turn", "bf guard", "timeout 5 bf human-turn"] {
        assert_eq!(bash(&fx, &wc, cmd), "deny", "{cmd}");
    }
}

#[test]
fn a_working_copy_that_is_not_at_work_refuses_every_write() {
    let (fx, wc) = unit_place();
    // A copy of a unit the table does not hold (landed or parked).
    let gone = wc.parent().unwrap().join("U-77");
    fs::create_dir_all(gone.join("src")).unwrap();
    assert_eq!(write_tool(&fx, &wc, &gone.join("src/a.txt")), "deny");
    assert_eq!(write_tool(&fx, &gone, &gone.join("src/a.txt")), "deny");
}

#[test]
fn the_repair_judge_stays_refused_its_commands() {
    let (fx, wc) = unit_place();
    let payload = json!({"tool_name": "Write", "tool_input": {"file_path": wc.join("src/a.txt").to_string_lossy()},
        "cwd": wc.to_string_lossy(), "agent_type": "repair-judge"});
    let o = fx.bf_stdin(&wc, &["guard"], &payload.to_string());
    let v: Value = serde_json::from_str(o.stdout.trim()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny", "{}", o.all());
}

// ---- a checkpoint fix

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
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn a_checkpoint_fix_may_write_a_file_outside_its_lists_but_not_a_protected_or_foreign_one() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    assert_ne!(write_tool(&fx, &k, &k.join("src/a.txt")), "deny");
    assert_ne!(write_tool(&fx, &k, &k.join("src/other.txt")), "deny");
    assert_ne!(write_tool(&fx, &k, &k.join("docs/new.md")), "deny");
    assert_eq!(write_tool(&fx, &k, &k.join(".git/config")), "deny");
    assert_eq!(write_tool(&fx, &k, &k.join("factory/policy.toml")), "deny");
    assert_eq!(write_tool(&fx, &k, &fx.root.join("src/other.txt")), "deny");
}

#[test]
fn a_checkpoint_with_no_fix_or_sync_open_refuses_every_write() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    assert_eq!(write_tool(&fx, &k, &k.join("src/other.txt")), "deny");
}

// ---- a session writes freely only in its own copy (F-411)

fn two_units() -> (Fx, PathBuf, PathBuf) {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"a\"]\n"),
        ("requirement", "id=\"R-3\"\nkind=\"requirement\"\ntitle=\"C\"\nacceptance=[\"c\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"A\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\n"),
        ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"C\"\ndelivers=[\"R-3\"]\nfiles=[\"beside/y.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"C\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-3\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\",\"R-3\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"All\"\nafter=[\"U-1\",\"U-3\"]\nproves=[\"R-1\",\"R-3\"]\nchecks=[\"X-4\"]\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/a.rs"), "fn a() {}\n").unwrap();
    fx.commit_all("records and files");
    fx.start_test_run();
    let start = |u: &str| {
        PathBuf::from(fx.must(&["unit", "start", u, "--json"]).json()["working_copy"].as_str().unwrap())
    };
    let w1 = start("U-1");
    let w3 = start("U-3");
    (fx, w1, w3)
}

#[test]
fn another_session_writes_only_the_listed_files_of_an_unit_at_work() {
    let (fx, w1, w3) = two_units();
    // From the main checkout and from a second unit at work: the listed file only, nothing more.
    for cwd in [fx.root.clone(), w3.clone()] {
        assert_eq!(write_tool(&fx, &cwd, &w1.join("src/other.rs")), "deny", "{}", cwd.display());
        assert_eq!(write_tool(&fx, &cwd, &w1.join("README.md")), "deny", "{}", cwd.display());
    }
    // The unit itself writes both.
    assert_ne!(write_tool(&fx, &w1, &w1.join("src/other.rs")), "deny");
    assert_ne!(write_tool(&fx, &w3, &w3.join("README.md")), "deny");
    // A unit never writes into the other unit's copy, listed file or not.
    assert_eq!(write_tool(&fx, &w3, &w1.join("src/a.rs")), "deny");
}

#[test]
fn a_unit_does_not_write_into_a_checkpoint_copy_and_a_parked_copy_refuses_every_write() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    // The builder of the fix writes in K-1; a session in another copy or in the main checkout does not.
    assert_ne!(write_tool(&fx, &k, &k.join("src/other.txt")), "deny");
    let other = k.parent().unwrap().join("U-1");
    fs::create_dir_all(&other).unwrap();
    assert_eq!(write_tool(&fx, &other, &k.join("src/other.txt")), "deny");
    assert_eq!(write_tool(&fx, &fx.root, &k.join("src/other.txt")), "deny");
    // The copy of the landed unit is not at work.
    assert_eq!(write_tool(&fx, &other, &other.join("src/a.txt")), "deny");
}

#[test]
fn the_state_folder_the_panel_the_briefs_the_table_and_extra_paths_stay_refused() {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("protected_extra = []", "protected_extra = [\"secrets/\"]");
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    for p in [fx.state_dir().join("key"), fx.root.join(".bf/briefs/U-1-builder.md"), fx.root.join(".bf/guard.json")] {
        assert_eq!(write_tool(&fx, &wc, &p), "deny", "{}", p.display());
    }
    for rel in [".claude/skills/factory-panel/x.md", ".claude/settings.json", ".bf/guard.json", "secrets/x.txt"] {
        assert_eq!(write_tool(&fx, &wc, &wc.join(rel)), "deny", "{rel}");
    }
}

#[test]
fn a_checkout_of_a_revision_is_refused() {
    let (fx, wc) = unit_place();
    for cmd in [
        "git checkout HEAD~3",
        "git checkout abc1234",
        "git checkout v1.0",
        "git checkout -",
        "git checkout -b x HEAD~5",
        "git checkout -B x",
        "git checkout --detach",
        "git checkout HEAD~1 -- src/hello.txt",
        "git checkout somebranch",
    ] {
        assert_eq!(bash(&fx, &wc, cmd), "deny", "{cmd}");
    }
    // A path of the copy, with or without `--`, and `HEAD`, stay open.
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    for cmd in ["git checkout src/hello.txt", "git checkout -- src/hello.txt", "git checkout HEAD -- src/hello.txt"] {
        assert_ne!(bash(&fx, &wc, cmd), "deny", "{cmd}");
    }
}

// ---- the resolver of an open sync

fn plan_sync(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

#[test]
fn the_resolver_of_an_open_sync_writes_any_unprotected_file_and_no_protected_one() {
    let fx = Fx::new();
    plan_sync(&fx);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    let code = fx.root.clone();
    fs::create_dir_all(code.join("src")).unwrap();
    fs::write(code.join("src/a.txt"), "from main\n").unwrap();
    git(&code, &["add", "src/a.txt"]);
    git(&code, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main into the checkpoint, and let the agents sort out conflicts.");
    let o = fx
        .must(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main into the checkpoint", "--json"])
        .json();
    assert_eq!(o["sync_open"], true, "{o}");
    let k = fx.work("K-1");
    // The conflicted file, a file neither side changed, and a new folder.
    for rel in ["src/a.txt", "src/other.txt", "docs/new.md"] {
        assert_ne!(write_tool(&fx, &k, &k.join(rel)), "deny", "{rel}");
    }
    for rel in ["factory/policy.toml", ".git/config", ".git/hooks/pre-commit"] {
        assert_eq!(write_tool(&fx, &k, &k.join(rel)), "deny", "{rel}");
    }
    // Another session does not write there.
    assert_eq!(write_tool(&fx, &fx.root, &k.join("src/other.txt")), "deny");
}
