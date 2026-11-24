//! U-87: `bf status` reads each repository's branches in one git call and loads the plan once; the plan
//! check reads the run's branches only for a check that names a test target. The outputs stay as they were.
//! The calls are counted with a script that logs each one and runs the real git.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The real git of the fixture's policy, as the policy names it.
fn real_git(fx: &Fx) -> String {
    let pol = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    let line = pol.lines().find(|l| l.starts_with("git = ")).unwrap();
    line.split('"').nth(1).unwrap().to_string()
}

/// A folder holding a `git` script that logs its arguments, one call to a line, and runs the real git.
fn logger(fx: &Fx) -> (PathBuf, PathBuf) {
    let dir = fx.root.parent().unwrap().join("gitlog");
    fs::create_dir_all(&dir).unwrap();
    let log = dir.join("calls.log");
    fs::write(&log, "").unwrap();
    let script = dir.join("git");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real_git(fx)
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (dir, log)
}

fn calls(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| l.to_string())
        .collect()
}

/// The policy names the logging script as its git, with the sandbox off for the test.
fn count_through_policy(fx: &Fx) -> PathBuf {
    let (dir, log) = logger(fx);
    let real = real_git(fx);
    fx.set_policy(
        &format!("git = \"{real}\""),
        &format!("git = \"{}\"", dir.join("git").display()),
    );
    fx.set_policy("sandbox = true", "sandbox = false");
    log
}

#[test]
fn status_makes_at_most_two_git_calls_on_a_project_without_a_checkpoint_branch() {
    let fx = Fx::sandboxed();
    let log = count_through_policy(&fx);
    fs::write(&log, "").unwrap();
    let o = fx.must(&["status", "--json"]);
    let c = calls(&log);
    assert!(c.len() <= 2, "git calls of `bf status`: {c:#?}");
    assert!(c.iter().any(|l| l.contains("for-each-ref")), "{c:#?}");
    assert!(o.json()["main"]["tip"].as_str().unwrap().len() >= 40);
}

fn path_with(dir: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", dir.display())
}

fn ls_tree_calls(fx: &Fx) -> usize {
    let (dir, log) = logger(fx);
    let _ = fx.bf_env(
        &fx.root,
        &["plan", "check", "--json"],
        "",
        &[("PATH", &path_with(&dir))],
    );
    calls(&log).iter().filter(|l| l.contains("ls-tree")).count()
}

#[test]
fn the_plan_check_runs_no_ls_tree_when_no_check_names_a_target() {
    let fx = Fx::new();
    fx.tiny();
    assert_eq!(ls_tree_calls(&fx), 0);
}

#[test]
fn the_plan_check_runs_one_ls_tree_when_a_check_names_a_test_target() {
    let fx = Fx::new();
    fx.tiny();
    let o = fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"target\"\nhow=\"command\"\ncommand=\"cargo test --test nothing_here\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("a check that names a target");
    assert_eq!(ls_tree_calls(&fx), 1);
}

#[test]
fn the_main_tip_is_what_rev_parse_gives() {
    let fx = Fx::new();
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main"]["tip"], git(&fx.root, &["rev-parse", "main"]));
    assert_eq!(st["repos"][0]["tip"], st["main"]["tip"]);
}

#[test]
fn a_tag_named_like_the_main_branch_gives_the_tip_rev_parse_gives() {
    let fx = Fx::new();
    git(&fx.root, &["tag", "main", "HEAD~1"]);
    let want = git(&fx.root, &["rev-parse", "main"]);
    assert_ne!(want, git(&fx.root, &["rev-parse", "refs/heads/main"]));
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main"]["tip"], want, "{st}");
}

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn start_and_land(fx: &Fx) {
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it",
        "--scope",
        "default",
    ]);
    fx.human("Yes, set the checkpoint review that way.");
    fx.must(&["checks", "set", "--quote", "set the checkpoint review that way"]);
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
        &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"],
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn main_ahead_and_the_review_rounds_are_reported() {
    let fx = Fx::new();
    plan(&fx);
    start_and_land(&fx);
    fx.bf(&["brief", "K-1", "--role", "reviewer", "--json"]);
    let body: Value = json!({
        "areas": [{"area": "the close rule", "how": "read"}],
        "findings": [{"title": "wrong", "severity": "high", "where": "src/a.txt:1", "action": "fix"}],
    });
    let o = fx.bf_stdin(
        &fx.root,
        &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"],
        &body.to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The human works on main: the checkpoint branch lacks that commit.
    fs::write(fx.root.join("extra.txt"), "on main\n").unwrap();
    git(&fx.root, &["add", "extra.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main_ahead"][0]["checkpoint"], "K-1", "{st}");
    assert_eq!(st["main_ahead"][0]["commits"], 1, "{st}");
    assert_eq!(st["checkpoint_reviews"][0]["checkpoint"], "K-1", "{st}");
    assert_eq!(st["checkpoint_reviews"][0]["rounds"][0]["round"], 1, "{st}");
    assert_eq!(st["checkpoint_reviews"][0]["rounds"][0]["findings"], 1, "{st}");
    assert_eq!(st["main"]["tip"], git(&fx.root, &["rev-parse", "main"]));
}
