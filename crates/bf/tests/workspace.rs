//! Workspace mode: the factory (records, log, harness) lives in a folder of its own and points at one or
//! more code repositories. Nothing of the factory is written into a code repository.

mod common;
use common::*;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

fn put_all(fx: &Fx, recs: &[(&str, &str)]) {
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("records");
}

/// A real approval of the plan, as the human does it.
fn approve_plan(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
}

fn build(fx: &Fx, unit: &str, file: &str, check: &str, cmd: &[&str]) {
    let wc = PathBuf::from(
        fx.must(&["unit", "start", unit, "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join(Path::new(file).parent().unwrap())).unwrap();
    fs::write(wc.join(file), format!("{unit}\n")).unwrap();
    let mut a = vec!["evidence", "run", check, "--unit", unit, "--"];
    a.extend_from_slice(cmd);
    fx.must_in(&wc, &a);
    fx.must(&["report", unit, "--result", "built"]);
    fx.must(&["report", unit, "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", unit]);
}

/// The code repositories get nothing but branches and commits: no factory, no harness, no ignore entries.
fn assert_code_repo_is_clean(fx: &Fx, name: &str) {
    let d = fx.repo_dir(name);
    assert!(!d.join("factory").exists(), "factory/ in {name}");
    assert!(!d.join(".bf").exists(), ".bf in {name}");
    assert!(!d.join(".claude").exists(), ".claude in {name}");
    assert!(!d.join(".gitignore").exists(), ".gitignore in {name}");
    assert_eq!(git(&d, &["status", "--porcelain"]), "", "{name} is dirty");
}

#[test]
fn a_workspace_with_one_external_repository_goes_through_the_whole_flow() {
    let fx = Fx::workspace(&["app"]);
    // The workspace file lists the repository, and status says where its main branch stands.
    let wsf = fs::read_to_string(fx.root.join("factory/workspace.toml")).unwrap();
    assert!(
        wsf.contains("name = \"app\"") && wsf.contains("path = \"../app\""),
        "{wsf}"
    );
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["workspace"], true);
    assert_eq!(st["repos"][0]["name"], "app");
    assert_eq!(st["repos"][0]["ok"], true);
    assert!(fx
        .must(&["status"])
        .stdout
        .contains("repository app: main at"));
    put_all(
        &fx,
        &[
            ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting\"\nacceptance=[\"src/hello.txt exists\"]\n"),
            ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\"]\nfiles=[\"src/hello.txt\"]\ncheckpoint=\"K-1\"\n"),
            ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"hello\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
            ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"hello again\"\nhow=\"command\"\ncommand=\"ls src/hello.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
            ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"All\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
        ],
    );
    approve_plan(&fx);
    fx.must(&["plan", "check"]);
    fx.must(&["run", "start"]);
    // The unit's working copy is a worktree of the code repository, placed in the workspace.
    let o = fx.must(&["unit", "start", "U-1", "--json"]).json();
    let wc = PathBuf::from(o["working_copy"].as_str().unwrap());
    assert_eq!(wc, fx.root.join(".bf/work/U-1"));
    assert_eq!(o["repo"], "app");
    assert!(git(&fx.repo_dir("app"), &["worktree", "list"]).contains(".bf/work/U-1"));
    // Commands from inside the working copy find the workspace, not the code repository.
    let st = fx.must_in(&wc, &["status", "--json"]).json();
    assert_eq!(st["workspace"], true, "{st}");
    assert_eq!(st["project"], "ws");
    // Build, review, land: the merge happens inside the repository.
    build(
        &fx,
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    assert!(git(&fx.repo_dir("app"), &["branch", "--list", "bf/K-1"]).contains("bf/K-1"));
    // Close the checkpoint over its own working copy, then merge with the human's go-ahead.
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "ls", "src/hello.txt"],
    );
    let out = fx.merge_checkpoint("K-1");
    assert_eq!(out.code, 0, "{}", out.all());
    assert_eq!(
        fs::read_to_string(fx.repo_dir("app").join("src/hello.txt")).unwrap(),
        "U-1\n"
    );
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["repos"][0]["ok"], true, "{st}");
    assert_eq!(fx.bf(&["ledger", "check"]).code, 0);
    assert_code_repo_is_clean(&fx, "app");
    let d = fx.bf(&["doctor"]);
    assert!(!d.stdout.contains("PROBLEM"), "{}", d.stdout);
}

#[test]
fn a_workspace_with_two_repositories_closes_one_checkpoint_over_both() {
    let fx = Fx::workspace(&["app", "api"]);
    put_all(
        &fx,
        &[
            ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"Both\"\nacceptance=[\"both exist\"]\n"),
            ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"App side\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\nrepo=\"app\"\ncheckpoint=\"K-1\"\n"),
            ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"Api side\"\ndelivers=[\"R-1\"]\nfiles=[\"src/b.txt\"]\nrepo=\"api\"\ncheckpoint=\"K-1\"\n"),
            ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"a\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
            ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"b\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
            ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"both\"\nhow=\"command\"\ncommand=\"ls app/src/a.txt api/src/b.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
            ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Both\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\"]\nchecks=[\"X-3\"]\n"),
        ],
    );
    approve_plan(&fx);
    fx.must(&["plan", "check"]);
    fx.must(&["run", "start"]);
    build(&fx, "U-1", "src/a.txt", "X-1", &["test", "-f", "src/a.txt"]);
    build(&fx, "U-2", "src/b.txt", "X-2", &["test", "-f", "src/b.txt"]);
    // Each unit changed its own repository; each repository has the checkpoint branch.
    assert!(git(&fx.repo_dir("app"), &["branch", "--list", "bf/K-1"]).contains("bf/K-1"));
    assert!(git(&fx.repo_dir("api"), &["branch", "--list", "bf/K-1"]).contains("bf/K-1"));
    // The checkpoint's working copy holds one worktree per repository; the check runs over both.
    let kdir = fx.work("K-1");
    assert!(kdir.join("app/src/a.txt").exists() && kdir.join("api/src/b.txt").exists());
    fx.must_in(
        &kdir,
        &[
            "evidence",
            "run",
            "X-3",
            "--",
            "ls",
            "app/src/a.txt",
            "api/src/b.txt",
        ],
    );
    let out = fx.merge_checkpoint("K-1");
    assert_eq!(out.code, 0, "{}", out.all());
    assert!(fx.repo_dir("app").join("src/a.txt").exists());
    assert!(fx.repo_dir("api").join("src/b.txt").exists());
    assert!(!fx.repo_dir("app").join("src/b.txt").exists());
    // The main-branch note is kept per repository.
    let st = fx.must(&["status", "--json"]).json();
    let repos = st["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 2, "{st}");
    assert!(repos.iter().all(|r| r["ok"] == true), "{st}");
    git(
        &fx.repo_dir("api"),
        &["commit", "--allow-empty", "-q", "-m", "mine"],
    );
    let st = fx.must(&["status", "--json"]).json();
    let api = repos.iter().position(|r| r["name"] == "api").unwrap();
    let app = 1 - api;
    assert!(
        st["repos"][api]["note"]
            .as_str()
            .unwrap()
            .contains("moved on"),
        "{st}"
    );
    assert_eq!(st["repos"][app]["note"], "");
    assert_eq!(fx.bf(&["ledger", "check"]).code, 0);
    assert_code_repo_is_clean(&fx, "app");
    assert_code_repo_is_clean(&fx, "api");
}

#[test]
fn a_unit_must_name_its_repository_when_there_are_several_and_the_name_must_exist() {
    let fx = Fx::workspace(&["app", "api"]);
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    fx.put("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\n");
    fx.put("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\nrepo=\"web\"\ncheckpoint=\"K-1\"\n");
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("U-1 names no repository") && o.stderr.contains("`web`"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_worker_cannot_write_into_a_code_repositorys_checkout_but_the_human_can() {
    let fx = Fx::workspace(&["app"]);
    put_all(
        &fx,
        &[
            ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
            ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/hello.txt\"]\ncheckpoint=\"K-1\"\n"),
            ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
            ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n"),
        ],
    );
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    let decide = |cwd: &Path, tool: &str, input: serde_json::Value| -> String {
        let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
        let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
        assert_eq!(o.code, 0, "{}", o.all());
        match o.stdout.trim() {
            "" | "{}" => "abstain".into(),
            s => serde_json::from_str::<serde_json::Value>(s).unwrap()["hookSpecificOutput"]
                ["permissionDecision"]
                .as_str()
                .unwrap()
                .to_string(),
        }
    };
    let app = fx.repo_dir("app");
    // The worker: its own working copy, yes; the code repository's checkout, no.
    assert_ne!(
        decide(
            &wc,
            "Write",
            json!({"file_path": wc.join("src/hello.txt").to_string_lossy()})
        ),
        "deny"
    );
    assert_eq!(
        decide(
            &wc,
            "Write",
            json!({"file_path": app.join("src/hello.txt").to_string_lossy()})
        ),
        "deny"
    );
    assert_eq!(
        decide(
            &wc,
            "Bash",
            json!({"command": format!("echo x > {}/src/hello.txt", app.display())})
        ),
        "deny"
    );
    // The factory is protected from the workspace's root as before, for the conductor too.
    assert_eq!(
        decide(
            &fx.root,
            "Write",
            json!({"file_path": fx.root.join("factory/policy.toml").to_string_lossy()})
        ),
        "deny"
    );
    // Nobody plants git hooks in the code repository; the human edits its files freely.
    assert_eq!(
        decide(
            &fx.root,
            "Write",
            json!({"file_path": app.join(".git/hooks/post-commit").to_string_lossy()})
        ),
        "deny"
    );
    assert_ne!(
        decide(
            &fx.root,
            "Write",
            json!({"file_path": app.join("README.md").to_string_lossy()})
        ),
        "deny"
    );
}

#[test]
fn the_same_path_in_two_repositories_is_no_clash_and_repo_add_has_rules() {
    let fx = Fx::workspace(&["app", "api"]);
    put_all(
        &fx,
        &[
            ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
            ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/x.txt\"]\nrepo=\"app\"\ncheckpoint=\"K-1\"\n"),
            ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/x.txt\"]\nrepo=\"api\"\ncheckpoint=\"K-1\"\n"),
            ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/x.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
            ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/x.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
            ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"k\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n"),
        ],
    );
    fx.must(&["plan", "check"]);
    // A repository must lie apart from the workspace, be a git repository, and have its branch.
    let o = fx.refused(&["repo", "add", "inside", "."]);
    assert!(
        o.stderr.contains("apart from the workspace"),
        "{}",
        o.stderr
    );
    let plain = fx.repo_dir("app").parent().unwrap().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let o = fx.refused(&["repo", "add", "plain", plain.to_str().unwrap()]);
    assert!(
        o.stderr.contains("not the top of a git repository"),
        "{}",
        o.stderr
    );
    let o = fx.refused(&["repo", "add", "app", "../app"]);
    assert!(o.stderr.contains("already lists"), "{}", o.stderr);
    assert_eq!(
        fx.must(&["repo", "list", "--json"])
            .json()
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_project_that_is_not_a_workspace_cannot_add_repositories() {
    let fx = Fx::new();
    let o = fx.refused(&["repo", "add", "x", "/tmp"]);
    assert!(o.stderr.contains("not a workspace"), "{}", o.stderr);
    let l = fx.must(&["repo", "list", "--json"]).json();
    assert_eq!(l.as_array().unwrap().len(), 1);
}
