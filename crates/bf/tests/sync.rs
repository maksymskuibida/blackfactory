//! `bf checkpoint sync`: the human has the open checkpoint at the tip of the stack take in the
//! repository's local main branch, at a quiet point.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn plan(fx: &Fx, repo: &str) {
    plan_with(fx, repo, "test -f src/a.txt");
}

/// The plan, with one command for the unit's check and the checkpoint's own check: the sync runs the
/// unit's check (the one every unit of the checkpoint is held to), the close needs the checkpoint's own.
fn plan_with(fx: &Fx, repo: &str, k_check: &str) {
    plan_two(fx, repo, k_check, k_check);
}

/// The plan, with the command of the unit's check and the command of the checkpoint's own check.
fn plan_two(fx: &Fx, repo: &str, unit_check: &str, k_check: &str) {
    let r = if repo.is_empty() {
        String::new()
    } else {
        format!("repo=\"{repo}\"\n")
    };
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\n{r}checkpoint=\"K-1\"\n")),
        ("check", format!("id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"{unit_check}\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n")),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{k_check}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn real_run(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
}

fn land_unit(fx: &Fx) {
    land_unit_with(fx, &["test", "-f", "src/a.txt"]);
}

/// Land the unit, running its check with the command the plan gave it.
fn land_unit_with(fx: &Fx, cmd: &[&str]) {
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    let mut args = vec!["evidence", "run", "X-1", "--unit", "U-1", "--"];
    args.extend_from_slice(cmd);
    fx.must_in(&wc, &args);
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
    let sync = |q: &str| fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", q]);
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
            "--local",
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
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    // The words are needed: a human turn with the quote in it.
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "nobody said this"]);
    assert_eq!(o.code, 2);
    let before = tip(code, "bf/K-1");
    fx.human("Yes, take main into the first checkpoint.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
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
    // The sync reaches for the remote first, so even a sync with nothing to take in needs the human's words.
    fx.human("Yes, take main into the checkpoint once more.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
            "--quote",
            "take main into",
            "--json",
        ])
        .json();
    assert_eq!(o["nothing_to_take_in"], true);
    // The checkpoint's own check is open again; the landed unit's receipts still count.
    assert_eq!(fx.bf(&["evidence", "verify", "--unit", "U-1"]).code, 0);
    // The sync ran the unit's check on the joined content, as a run that settles nothing: the
    // verification lists it apart, and the close is refused until the checkpoint's own check ran.
    let v = fx
        .bf(&["evidence", "verify", "--checkpoint", "K-1", "--json"])
        .stdout;
    assert!(v.contains("sync:X-1"), "{v}");
    assert_eq!(fx.bf(&["checkpoint", "close", "K-1"]).code, 2);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
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
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    human_commit(&root, "docs/extra.txt", "main\n");
    let wt = fx.work("K-1");
    git(&wt, &["merge", "--no-edit", "main"]);
    assert!(!fx
        .must(&["log", "--json"])
        .stdout
        .contains("checkpoint.sync"));
    fx.must_in(
        &wt,
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    assert_eq!(fx.merge_checkpoint("K-1").code, 0);
}

fn guard_says(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> String {
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

/// The stack and the main branch both changed src/a.txt: a sync conflicts and opens.
fn open_conflict(fx: &Fx, code: &Path) -> String {
    prepare_conflict(fx, code);
    sync_into_conflict(fx, code)
}

fn prepare_conflict(fx: &Fx, code: &Path) {
    real_run(fx);
    land_unit(fx);
    human_commit(code, "src/a.txt", "from main\n");
}

fn sync_into_conflict(fx: &Fx, code: &Path) -> String {
    let before = tip(code, "bf/K-1");
    fx.human("Yes, bring main into the checkpoint, and let the agents sort out conflicts.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
            "--quote",
            "bring main into the checkpoint",
            "--json",
        ])
        .json();
    assert_eq!(o["synced"], false, "{o}");
    assert_eq!(o["sync_open"], true, "{o}");
    assert_eq!(o["conflicted"][0], "src/a.txt", "{o}");
    before
}

fn resolve_file(wt: &Path) {
    fs::write(wt.join("src/a.txt"), "from the unit\nfrom main\n").unwrap();
    git(wt, &["add", "src/a.txt"]);
    git(wt, &["commit", "-q", "--no-edit"]);
}

fn directive(fx: &Fx, action: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "one directive while a sync is open: {n}");
    assert_eq!(ds[0]["action"], action, "{n}");
    ds[0].clone()
}

fn walk_resolve(fx: &Fx, code: &Path) {
    prepare_conflict(fx, code);
    fx.set_policy("tokens = false", "tokens = true");
    let before = sync_into_conflict(fx, code);
    let k = fx.work("K-1");
    // The merge is left in progress, and everything about it is found from status and next alone.
    assert!(
        k.join("src/a.txt").exists() && git(&k, &["status", "--porcelain"]).contains("src/a.txt")
    );
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["sync_open"]["checkpoint"], "K-1", "{st}");
    assert_eq!(st["sync_open"]["phase"], "resolve");
    assert_eq!(st["sync_open"]["files"][0], "src/a.txt");
    assert!(fx
        .must(&["status"])
        .stdout
        .contains("a sync of the main branch into K-1 is open"));
    let d = directive(fx, "resolve-sync");
    assert_eq!(d["role"], "builder");
    assert_eq!(d["checkpoint"], "K-1");
    assert_eq!(d["files"][0], "src/a.txt");
    assert_eq!(d["working_copy"], k.to_string_lossy().as_ref());
    assert!(
        d["token"].as_str().unwrap().len() > 8 && d["tier"].is_string() && d["brief"].is_string(),
        "{d}"
    );
    let token = d["token"].as_str().unwrap().to_string();
    // No unit starts and the checkpoint does not close while the sync is open.
    assert_eq!(fx.bf(&["unit", "start", "U-1"]).code, 2);
    assert_eq!(fx.bf(&["checkpoint", "close", "K-1"]).code, 2);
    // The brief names the file and what each side changed.
    let brief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        brief.contains("src/a.txt")
            && brief.contains("What the stack changed")
            && brief.contains("from the unit")
            && brief.contains("from main"),
        "{brief}"
    );
    // The guard: the conflicted file may be written here, nothing else; the engine aborts, not the agent.
    assert_ne!(
        guard_says(
            fx,
            &k,
            "Write",
            json!({"file_path": k.join("src/a.txt").to_string_lossy()})
        ),
        "deny"
    );
    // The list is a recommendation: a file neither side changed may be written too.
    assert_ne!(
        guard_says(
            fx,
            &k,
            "Write",
            json!({"file_path": k.join("src/other.txt").to_string_lossy()})
        ),
        "deny"
    );
    assert_eq!(
        guard_says(
            fx,
            &k,
            "Write",
            json!({"file_path": k.join(".git/config").to_string_lossy()})
        ),
        "deny"
    );
    for cmd in ["git merge --abort", "git reset --hard HEAD~1", "git stash"] {
        assert_eq!(
            guard_says(fx, &k, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
    // A shell write in the checkpoint's own copy is judged as the write tools are.
    for cmd in [
        "echo x > src/a.txt",
        "sed -i '' 's/x/y/' src/a.txt",
        "tee src/a.txt < /dev/null",
    ] {
        assert_ne!(
            guard_says(fx, &k, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
    for cmd in ["echo x > src/other.txt", "rm src/other.txt"] {
        assert_ne!(
            guard_says(fx, &k, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
    assert_ne!(
        guard_says(
            fx,
            &k,
            "Bash",
            json!({"command": "git add src/a.txt && git commit --no-edit"})
        ),
        "deny"
    );
    assert_ne!(
        guard_says(
            fx,
            &k,
            "Bash",
            json!({"command": format!("bf report K-1 --role builder --result built --token {token}")})
        ),
        "deny"
    );
    // Reporting with the conflict still there, or with a wrong token, is refused.
    let o = fx.bf_in(
        &k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--token", &token,
        ],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("unmerged") || o.stderr.contains("not concluded"),
        "{}",
        o.stderr
    );
    let o = fx.bf_in(
        &k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--token", "wrong",
        ],
    );
    assert_eq!(o.code, 2);
    // Leftover conflict markers are refused even when the merge was concluded.
    fs::write(
        k.join("src/a.txt"),
        "<<<<<<< ours\nfrom the unit\n=======\nfrom main\n>>>>>>> theirs\n",
    )
    .unwrap();
    git(&k, &["add", "src/a.txt"]);
    git(&k, &["commit", "-q", "--no-edit"]);
    let o = fx.bf_in(
        &k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--token", &token,
        ],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("conflict markers"), "{}", o.stderr);
    // A file that was not in conflict, changed outside the guard, is refused at the report.
    fs::write(k.join("src/a.txt"), "from the unit\nfrom main\n").unwrap();
    fs::write(k.join("src/sneaky.txt"), "not asked for\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "resolve, and more"]);
    let o = fx.bf_in(
        &k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--token", &token,
        ],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr
            .contains("src/sneaky.txt was changed but is not a file either side changed"),
        "{}",
        o.stderr
    );
    git(&k, &["rm", "-q", "src/sneaky.txt"]);
    git(&k, &["commit", "-q", "-m", "take the extra file out"]);
    fx.must_in(
        &k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--token", &token,
        ],
    );
    // The review: the reviewer's brief shows the resolution against both sides; a finding sends it back.
    let d = directive(fx, "review-sync");
    assert_eq!(d["role"], "reviewer");
    let rt = d["token"].as_str().unwrap().to_string();
    assert_ne!(rt, token);
    let rbrief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "reviewer"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        rbrief.contains("compared with the stack's side")
            && rbrief.contains("compared with the main branch's side"),
        "{rbrief}"
    );
    let findings = fx.root.join(".bf/f.json");
    fs::write(
        &findings,
        json!([{"title": "Order", "where": "src/a.txt:1", "action": "Put main's line first"}])
            .to_string(),
    )
    .unwrap();
    fx.must_in(
        &k,
        &[
            "report",
            "K-1",
            "--role",
            "reviewer",
            "--result",
            "findings",
            "--findings",
            findings.to_str().unwrap(),
            "--token",
            &rt,
        ],
    );
    let d = directive(fx, "resolve-sync");
    assert_eq!(d["limits"]["round"], 1);
    let token2 = d["token"].as_str().unwrap().to_string();
    let brief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(brief.contains("Put main's line first"), "{brief}");
    fs::write(k.join("src/a.txt"), "from main\nfrom the unit\n").unwrap();
    git(&k, &["add", "src/a.txt"]);
    git(&k, &["commit", "-q", "-m", "main's line first"]);
    fx.must_in(
        &k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--token", &token2,
        ],
    );
    let d = directive(fx, "review-sync");
    let rt2 = d["token"].as_str().unwrap().to_string();
    fx.must_in(
        &k,
        &[
            "report", "K-1", "--role", "reviewer", "--result", "built", "--token", &rt2,
        ],
    );
    // Settled: recorded as a sync, the note quiet, the checkpoint's checks open again, units may start.
    let log = fx.must(&["log", "--json"]).stdout;
    assert!(
        log.contains("checkpoint.sync\"") || log.contains("\"checkpoint.sync\""),
        "{log}"
    );
    let st = fx.must(&["status", "--json"]).json();
    assert!(
        st["sync_open"].is_null() && st["main_ahead"].as_array().unwrap().is_empty(),
        "{st}"
    );
    assert_ne!(tip(code, "bf/K-1"), before);
    // The sync's runs settle nothing: the checkpoint's own check runs at verification, then it closes.
    fx.must_in(
        &k,
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    assert_eq!(fx.merge_checkpoint("K-1").code, 0);
    assert_eq!(
        fs::read_to_string(code.join("src/a.txt")).unwrap(),
        "from main\nfrom the unit\n"
    );
}

#[test]
fn a_conflict_is_resolved_by_the_directive_holder_and_reviewed_in_a_project_folder() {
    let fx = Fx::new();
    plan(&fx, "");
    let root = fx.root.clone();
    walk_resolve(&fx, &root);
}

#[test]
fn a_conflict_is_resolved_by_the_directive_holder_and_reviewed_in_a_workspace() {
    let fx = Fx::workspace(&["app"]);
    let code = fx.root.join("code");
    fs::create_dir_all(&code).unwrap();
    git(&code, &["init", "-q", "-b", "main"]);
    fs::write(code.join("README.md"), "# code\n").unwrap();
    git(&code, &["add", "-A"]);
    git(&code, &["commit", "-q", "-m", "init"]);
    fx.must(&["repo", "add", "code", "code"]);
    plan(&fx, "code");
    walk_resolve(&fx, &code);
}

#[test]
fn abort_puts_the_checkpoint_back_exactly_and_a_parked_sync_waits_for_the_human() {
    let fx = Fx::new();
    plan(&fx, "");
    let root = fx.root.clone();
    let before = open_conflict(&fx, &root);
    // Abort needs no human turn and restores the exact earlier tip.
    let o = fx
        .must(&["checkpoint", "sync", "K-1", "--abort", "--json"])
        .json();
    assert_eq!(o["aborted"], true, "{o}");
    assert_eq!(tip(&root, "bf/K-1"), before);
    assert_eq!(git(&fx.work("K-1"), &["status", "--porcelain"]), "");
    assert!(fx.must(&["status", "--json"]).json()["sync_open"].is_null());
    assert!(fx
        .must(&["log", "--json"])
        .stdout
        .contains("checkpoint.sync.abort"));
    fx.refused(&["checkpoint", "sync", "K-1", "--abort"]);
    // Try again; a resolver that is blocked, and a reviewer that finds too much, park it.
    fx.set_policy("max_rounds = 3", "max_rounds = 1");
    fx.human("Yes, try bringing main in again.");
    fx.must(&[
        "checkpoint",
        "sync",
        "K-1",
        "--local",
        "--quote",
        "try bringing main in again",
    ]);
    let k = fx.work("K-1");
    fx.must_in(
        &k,
        &[
            "report",
            "K-1",
            "--role",
            "builder",
            "--result",
            "blocked",
            "--note",
            "I cannot tell which line is right",
        ],
    );
    let d = directive(&fx, "decide-sync");
    assert_eq!(d["role"], "human");
    assert!(d["release_command"].as_str().unwrap().contains("--release"));
    // Only the human releases it, in their words.
    fx.refused(&[
        "checkpoint",
        "sync",
        "K-1",
        "--release",
        "--quote",
        "release it",
    ]);
    fx.human("Yes, release it and try again.");
    fx.must(&[
        "checkpoint",
        "sync",
        "K-1",
        "--release",
        "--quote",
        "release it",
    ]);
    directive(&fx, "resolve-sync");
    // A hand resolution while a sync is open is recorded with `--resolved` (the human's way).
    resolve_file(&k);
    // A report on the open sync is refused cleanly while another engine command holds the lock.
    {
        let held = bf::lock::engine(&fx.project(), "hold").unwrap();
        let o = fx.bf_env(
            &k,
            &["report", "K-1", "--role", "builder", "--result", "built"],
            "",
            &[("BF_LOCK_WAIT_MS", "300")],
        );
        assert_ne!(o.code, 0, "{}", o.all());
        assert!(
            o.all().contains("another engine command is running"),
            "{}",
            o.all()
        );
        drop(held);
    }
    fx.human("I resolved it myself, thanks.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--resolved",
            "--quote",
            "resolved it myself",
            "--json",
        ])
        .json();
    assert_eq!(o["synced"], true, "{o}");
    assert!(o["skipped_checks"].is_array(), "{o}");
    assert!(fx.must(&["status", "--json"]).json()["sync_open"].is_null());
}

/// The main branch adds a file that breaks the checkpoint's check once joined: no textual conflict.
fn walk_repair(fx: &Fx, code: &Path) {
    real_run(fx);
    land_unit_with(fx, &["test", "!", "-f", "src/banned.txt"]);
    human_commit(
        code,
        "src/banned.txt",
        "added on main, before the run's rule\n",
    );
    let before = tip(code, "bf/K-1");
    fx.human("Yes, bring main into the first checkpoint.");
    let t = std::time::Instant::now();
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
            "--quote",
            "bring main into the first checkpoint",
            "--json",
        ])
        .json();
    eprintln!(
        "sync with one failing and no passing check run: {:?}",
        t.elapsed()
    );
    // Merged without a conflict, and the check fails on the joined content: the sync stays open.
    assert_eq!(o["synced"], false, "{o}");
    assert_eq!(o["sync_open"], true, "{o}");
    assert!(o["conflicted"].as_array().unwrap().is_empty(), "{o}");
    // The failing check is the unit's, which every unit of the checkpoint is held to; the checkpoint's
    // own X-2 was not run.
    assert_eq!(o["failed_checks"][0]["check"], "X-1", "{o}");
    assert_eq!(o["failed_checks"].as_array().unwrap().len(), 1, "{o}");
    let k = fx.work("K-1");
    assert!(k.join("src/banned.txt").exists(), "the merge is kept");
    let d = directive(fx, "resolve-sync");
    assert_eq!(d["failing_checks"][0], "X-1", "{d}");
    assert!(
        d["files"].as_array().unwrap().is_empty(),
        "no conflicted file: {d}"
    );
    assert!(
        d["may_change"].to_string().contains("src/banned.txt"),
        "{d}"
    );
    assert_eq!(fx.bf(&["unit", "start", "U-1"]).code, 2);
    // The brief names the failing check, its command and output tail, and what may be changed.
    let brief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        brief.contains("X-1")
            && !brief.contains("X-2")
            && brief.contains("checkpoint's own checks run only at verification")
            && brief.contains("FAILS")
            && brief.contains("test ! -f src/banned.txt")
            && brief.contains("What you may change"),
        "{brief}"
    );
    // The guard: a file either side changed, and a new test file beside it; nothing else.
    assert_ne!(
        guard_says(
            fx,
            &k,
            "Write",
            json!({"file_path": k.join("src/banned.txt").to_string_lossy()})
        ),
        "deny"
    );
    assert_ne!(
        guard_says(
            fx,
            &k,
            "Write",
            json!({"file_path": k.join("src/a_test.txt").to_string_lossy()})
        ),
        "deny"
    );
    // The list is a recommendation: a file neither side changed may be written too.
    assert_ne!(
        guard_says(
            fx,
            &k,
            "Write",
            json!({"file_path": k.join("src/other.txt").to_string_lossy()})
        ),
        "deny"
    );
    assert_eq!(
        guard_says(
            fx,
            &k,
            "Write",
            json!({"file_path": k.join(".git/config").to_string_lossy()})
        ),
        "deny"
    );
    // Done outside the guard, a change to a file neither side changed is refused at the report.
    fs::remove_file(k.join("src/banned.txt")).unwrap();
    fs::write(k.join("src/other.txt"), "not asked for\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "repair, and more"]);
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr
            .contains("src/other.txt was changed but is not a file either side changed"),
        "{}",
        o.stderr
    );
    git(&k, &["rm", "-q", "src/other.txt"]);
    git(&k, &["commit", "-q", "-m", "only the repair"]);
    // The repair holds: the engine runs the checks through itself, then the reviewer sees the whole fix.
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    let d = directive(fx, "review-sync");
    assert_eq!(d["role"], "reviewer");
    let rbrief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "reviewer"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        rbrief.contains("compared with git's automatic merge") && rbrief.contains("banned.txt"),
        "{rbrief}"
    );
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
    // Recorded; the sync's runs settle nothing, so the checkpoint's own check runs at verification.
    assert!(fx.must(&["status", "--json"]).json()["sync_open"].is_null());
    assert!(fx
        .must(&["log", "--json"])
        .stdout
        .contains("\"checkpoint.sync\""));
    assert_ne!(tip(code, "bf/K-1"), before);
    assert_eq!(fx.bf(&["checkpoint", "close", "K-1"]).code, 2);
    fx.must_in(
        &k,
        &[
            "evidence",
            "run",
            "X-2",
            "--",
            "test",
            "!",
            "-f",
            "src/banned.txt",
        ],
    );
    assert_eq!(fx.merge_checkpoint("K-1").code, 0);
    assert!(!code.join("src/banned.txt").exists());
}

#[test]
fn a_sync_without_a_conflict_whose_joined_content_fails_a_check_is_repaired_in_a_project_folder() {
    let fx = Fx::new();
    plan_with(&fx, "", "test ! -f src/banned.txt");
    let root = fx.root.clone();
    walk_repair(&fx, &root);
}

#[test]
fn a_sync_without_a_conflict_whose_joined_content_fails_a_check_is_repaired_in_a_workspace() {
    let fx = Fx::workspace(&["app"]);
    let code = fx.root.join("code");
    fs::create_dir_all(&code).unwrap();
    git(&code, &["init", "-q", "-b", "main"]);
    fs::write(code.join("README.md"), "# code\n").unwrap();
    git(&code, &["add", "-A"]);
    git(&code, &["commit", "-q", "-m", "init"]);
    fx.must(&["repo", "add", "code", "code"]);
    plan_with(&fx, "code", "test ! -f src/banned.txt");
    walk_repair(&fx, &code);
}

#[test]
fn abort_after_a_failed_check_restores_the_earlier_tip_and_the_sync_that_passes_records_at_once() {
    let fx = Fx::new();
    plan_with(&fx, "", "test ! -f src/banned.txt");
    let root = fx.root.clone();
    real_run(&fx);
    land_unit_with(&fx, &["test", "!", "-f", "src/banned.txt"]);
    human_commit(&root, "src/banned.txt", "added on main\n");
    let before = tip(&root, "bf/K-1");
    fx.human("Yes, bring main into the first checkpoint.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
            "--quote",
            "bring main into the first checkpoint",
            "--json",
        ])
        .json();
    assert_eq!(o["sync_open"], true);
    assert_ne!(tip(&root, "bf/K-1"), before, "the merge was made");
    // Abort puts the exact earlier tip back, with no human turn.
    fx.must(&["checkpoint", "sync", "K-1", "--abort"]);
    assert_eq!(tip(&root, "bf/K-1"), before);
    assert!(!fx.work("K-1").join("src/banned.txt").exists());
    assert!(fx.must(&["status", "--json"]).json()["sync_open"].is_null());
    // A sync whose joined content passes is recorded at once (no review).
    git(&root, &["rm", "-q", "src/banned.txt"]);
    git(&root, &["commit", "-q", "-m", "main fixes itself"]);
    fx.human("Yes, bring main in again.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
            "--quote",
            "bring main in again",
            "--json",
        ])
        .json();
    assert_eq!(o["synced"], true, "{o}");
}

/// A sync runs only the checks that are switched on, and says which it left out.
#[test]
fn a_sync_leaves_out_a_switched_off_check_and_says_so() {
    let fx = Fx::new();
    fx.tiny();
    // A checkpoint check that is named, so it is switched off, whose command fails.
    let x3 = "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"false\"\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nscope=\"checkpoint\"\nnamed=\"accessibility\"\n";
    let f = fx.root.join("x3.toml");
    fs::write(&f, x3).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let _ = fs::remove_file(&f);
    fx.commit_all("x3");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fs::create_dir_all(fx.root.join("docs")).unwrap();
    fs::write(fx.root.join("docs/extra.txt"), "main\n").unwrap();
    git(&fx.root, &["add", "docs/extra.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, take main into the checkpoint.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
            "--quote",
            "take main into the checkpoint",
            "--json",
        ])
        .json();
    assert_eq!(o["synced"], true, "{o}");
    assert_eq!(o["skipped_checks"], json!(["X-3"]), "{o}");
    assert!(o["sync_open"].is_null(), "{o}");
}

/// The resolver's brief lists only the checks the sync will run.
#[test]
fn the_resolvers_brief_leaves_out_a_switched_off_check() {
    let fx = Fx::new();
    fx.tiny();
    let x3 = "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"false\"\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nscope=\"checkpoint\"\nnamed=\"accessibility\"\n";
    let f = fx.root.join("x3.toml");
    fs::write(&f, x3).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let _ = fs::remove_file(&f);
    fx.commit_all("x3");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    // The same file changes on main: the sync conflicts and a resolver is briefed.
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/hello.txt"), "main says hi\n").unwrap();
    git(&fx.root, &["add", "src/hello.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, take main into the checkpoint.");
    let o = fx.bf(&[
        "checkpoint",
        "sync",
        "K-1",
        "--local",
        "--quote",
        "take main into the checkpoint",
        "--json",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.must(&["brief", "K-1", "--role", "builder"]);
    let path = o.stdout.trim().lines().last().unwrap().trim().to_string();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("src/hello.txt"), "{text}");
    assert!(!text.contains("X-3"), "{text}");
}

/// The checks of the verification of a checkpoint, as the rows it prints.
fn verified_checks(fx: &Fx, k: &str) -> Vec<String> {
    let o = fx.bf(&["evidence", "verify", "--checkpoint", k, "--json"]);
    o.json()["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["check"].as_str().unwrap().to_string())
        .collect()
}

fn sync_k1(fx: &Fx, words: &str) -> Value {
    fx.human(&format!("Yes, {words}."));
    fx.must(&["checkpoint", "sync", "K-1", "--local", "--quote", words, "--json"])
        .json()
}

/// A checkpoint whose own check differs from the check its unit is held to: the sync runs the unit's
/// check, never the checkpoint's own, and its runs let nothing close.
#[test]
fn a_sync_runs_the_checks_every_unit_is_held_to_and_not_the_checkpoints_own() {
    let fx = Fx::new();
    plan_two(&fx, "", "test -f src/a.txt", "test -e src/a.txt");
    let root = fx.root.clone();
    real_run(&fx);
    land_unit(&fx);
    human_commit(&root, "docs/extra.txt", "written on main\n");
    let o = sync_k1(&fx, "bring main in");
    assert_eq!(o["synced"], true, "{o}");
    // The list of checks the sync ran, as the verification shows them.
    let ran = verified_checks(&fx, "K-1");
    println!("the checks the sync ran: {ran:?}");
    assert!(ran.contains(&"sync:X-1".to_string()), "{ran:?}");
    assert!(
        !ran.iter().any(|c| c.contains("X-2")),
        "the checkpoint's own check is not run by the sync: {ran:?}"
    );
    // The sync's runs settle nothing: the checkpoint does not close on them.
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(
        fx.bf(&["evidence", "verify", "--checkpoint", "K-1"]).code,
        2
    );
    // At verification the checkpoint's own check runs on the content being closed, and it closes.
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-e", "src/a.txt"],
    );
    assert_eq!(
        fx.bf(&["evidence", "verify", "--checkpoint", "K-1"]).code,
        0
    );
    assert_eq!(fx.merge_checkpoint("K-1").code, 0);
}

/// A failing check of the units opens the repair, and the refusal names the sync check and its command.
#[test]
fn a_failing_check_every_unit_is_held_to_opens_the_repair_and_the_refusal_names_it() {
    let fx = Fx::new();
    plan_two(&fx, "", "test ! -f src/banned.txt", "test -e src/a.txt");
    let root = fx.root.clone();
    real_run(&fx);
    land_unit_with(&fx, &["test", "!", "-f", "src/banned.txt"]);
    human_commit(&root, "src/banned.txt", "added on main\n");
    let o = sync_k1(&fx, "bring main in");
    assert_eq!(o["sync_open"], true, "{o}");
    assert_eq!(o["failed_checks"][0]["check"], "X-1", "{o}");
    assert_eq!(
        o["failed_checks"][0]["command"], "test ! -f src/banned.txt",
        "{o}"
    );
    println!("the checks the sync ran: {:?}", verified_checks(&fx, "K-1"));
    assert!(!verified_checks(&fx, "K-1")
        .iter()
        .any(|c| c.contains("X-2")));
    // Reporting the resolution while the check still fails is refused, naming the check and command.
    let k = fx.work("K-1");
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("X-1") && o.stderr.contains("test ! -f src/banned.txt"),
        "{}",
        o.stderr
    );
}

/// Units that share no command: the sync runs the checkpoint's own command checks, as before.
#[test]
fn a_checkpoint_whose_units_share_no_command_is_synced_with_its_own_checks() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fs::create_dir_all(fx.root.join("docs")).unwrap();
    fs::write(fx.root.join("docs/extra.txt"), "main\n").unwrap();
    git(&fx.root, &["add", "docs/extra.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    let o = sync_k1(&fx, "take main into the checkpoint");
    assert_eq!(o["synced"], true, "{o}");
    let ran = verified_checks(&fx, "K-1");
    println!("the checks the sync ran: {ran:?}");
    assert!(ran.contains(&"X-3".to_string()), "{ran:?}");
    assert!(!ran.iter().any(|c| c.starts_with("sync:")), "{ran:?}");
}
