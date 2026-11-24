//! U-118 (D-140, D-143): inside one `bf unit land` a content hash or a branch read is used again while the
//! engine has written nothing that could change it, and the repeated reads are dropped. A landing makes
//! the same events, receipts and refusals as with the memo switched off (`BF_TEST_NO_LAND_MEMO`), and a
//! change to the unit's files after its review, or by its check while the landing runs it, is still caught.
//! The git calls are counted with a script that logs each one and runs the real git (as in `status_cost`).

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The real git of the fixture's policy, as the policy names it.
fn real_git(fx: &Fx) -> String {
    let pol = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    let line = pol.lines().find(|l| l.starts_with("git = ")).unwrap();
    line.split('"').nth(1).unwrap().to_string()
}

/// The policy names a script as its git that logs its arguments, one call to a line, and runs the real
/// git; the sandbox is off for the test. The log file is returned.
fn count_through_policy(fx: &Fx) -> PathBuf {
    let dir = fx.root.parent().unwrap().join("gitlog");
    fs::create_dir_all(&dir).unwrap();
    let log = dir.join("calls.log");
    fs::write(&log, "").unwrap();
    let script = dir.join("git");
    let real = real_git(fx);
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    fx.set_policy(
        &format!("git = \"{real}\""),
        &format!("git = \"{}\"", script.display()),
    );
    fx.set_policy("sandbox = true", "sandbox = false");
    log
}

fn calls(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| l.to_string())
        .collect()
}

/// Two units of K-1 with their own file and check, and a run whose suite is `sh suite.sh`. The main
/// branch holds a `suite.sh` that passes; U-1 may bring its own.
fn plan(fx: &Fx) {
    fs::write(fx.root.join("suite.sh"), "exit 0\n").unwrap();
    let mut recs = vec![(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string(),
    )];
    for (n, f) in [("1", "src/a.txt\",\"suite.sh"), ("2", "src/b.txt")] {
        recs.push(("unit", format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u{n}\"\ndelivers=[\"R-1\"]\nfiles=[\"{f}\"]\ncheckpoint=\"K-1\"\n")));
        recs.push(("check", format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n")));
    }
    recs.push(("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"k\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".into()));
    recs.push(("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\"]\nchecks=[\"X-9\"]\n".into()));
    for (k, t) in recs {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.commit_all("records");
    fx.human("yes, make this a test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, make this a test run with simulated approvals",
    ]);
    fx.must(&["run", "start"]);
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    fx.must(&["checks", "unit", "propose", "override", "suite=sh suite.sh"]);
    fx.human("Use the suite for every unit.");
    fx.must(&[
        "checks",
        "unit",
        "set",
        "--quote",
        "Use the suite for every unit",
    ]);
    fx.must(&["gate", "open", "plan"]);
    fx.must(&["gate", "approve", "plan", "--simulated"]);
}

fn start(fx: &Fx, u: &str) -> PathBuf {
    PathBuf::from(
        fx.must(&["unit", "start", u, "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

/// Write the unit's files, run its checks and have it reviewed clean.
fn build(fx: &Fx, u: &str, wc: &Path, files: &[(&str, &str)]) {
    for (f, text) in files {
        let path = wc.join(f);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let x = format!("X-{}", &u[2..]);
    fx.must_in(wc, &["evidence", "run", &x, "--unit", u, "--", "true"]);
    fx.must_in(
        wc,
        &[
            "evidence",
            "run",
            "attached:suite",
            "--unit",
            u,
            "--",
            "sh",
            "suite.sh",
        ],
    );
    fx.must(&["report", u, "--result", "built"]);
    fx.must(&["report", u, "--result", "built", "--role", "reviewer"]);
}

/// The landing to compare: one fixture made by `setup`, then `bf unit land U-1`, with the memo on or off.
struct Landing {
    fx: Fx,
    out: Out,
    events_before: usize,
    calls: Vec<String>,
}

/// `during`, when given, runs while the landing holds just before it takes the engine lock.
fn land(setup: fn(&Fx), memo: bool, during: Option<fn(&Fx)>) -> Landing {
    let fx = Fx::sandboxed();
    let log = count_through_policy(&fx);
    setup(&fx);
    let events_before = events(&fx).len();
    fs::write(&log, "").unwrap();
    let mut env: Vec<(&str, &str)> = Vec::new();
    if !memo {
        env.push(("BF_TEST_NO_LAND_MEMO", "1"));
    }
    let out = match during {
        None => fx.bf_env(&fx.root, &["unit", "land", "U-1", "--json"], "", &env),
        Some(f) => {
            use std::io::{BufRead, BufReader};
            use std::process::{Command, Stdio};
            let mut child = Command::new(bin())
                .args(["unit", "land", "U-1", "--json"])
                .current_dir(&fx.root)
                .env("BF_STATE_HOME", &fx.state_base)
                .env("BF_HOLD_AT", "land-before-lock")
                .envs(env.iter().copied())
                .stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut held = String::new();
            let mut err = BufReader::new(child.stderr.take().unwrap());
            err.read_line(&mut held).unwrap();
            assert!(held.contains("K-1"), "{held}");
            f(&fx);
            drop(child.stdin.take());
            let mut rest = String::new();
            std::io::Read::read_to_string(&mut err, &mut rest).unwrap();
            let o = child.wait_with_output().unwrap();
            Out {
                code: o.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                stderr: rest,
            }
        }
    };
    let calls = calls(&log);
    Landing {
        fx,
        out,
        events_before,
        calls,
    }
}

fn events(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// A value with what differs between two fixtures made the same way taken out: the fixture's folders,
/// the time, the chain of the log and the ids of commits (each commit holds its time).
fn norm(fx: &Fx, v: &Value, key: &str) -> Value {
    const MASKED: [&str; 9] = [
        "at",
        "prev",
        "h",
        "merge",
        "commit",
        "head",
        "base_commit",
        "tip",
        "run",
    ];
    if MASKED.contains(&key) {
        return Value::String("<masked>".into());
    }
    match v {
        Value::String(s) => {
            let root = fx.root.to_string_lossy().to_string();
            let s = s.replace(&root, "<root>").replace(
                &fx.root
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                "<root>",
            );
            Value::String(mask_commits(&s))
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| norm(fx, x, "")).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, x)| (k.clone(), norm(fx, x, k))).collect())
        }
        _ => v.clone(),
    }
}

/// A text with the commit a refusal names after "now at " masked (a commit holds its time).
fn mask_commits(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("now at ") {
        let (head, tail) = rest.split_at(i + "now at ".len());
        out.push_str(head);
        let n = tail.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        out.push_str(if n > 0 { "<commit>" } else { "" });
        rest = &tail[n..];
    }
    out.push_str(rest);
    out
}

/// The receipts, field by field, without the fields that hold a time or a seal.
fn receipts(fx: &Fx) -> Vec<Value> {
    let mut v: Vec<Value> = bf::evidence::load_receipts(&fx.project())
        .unwrap()
        .into_iter()
        .map(|r| {
            let mut x = r.v.clone();
            if let Some(o) = x.as_object_mut() {
                for k in [
                    "at",
                    "seal",
                    "duration_ms",
                    "secs",
                    "started",
                    "ended",
                    "id",
                    "load",
                    "cores",
                    "seconds",
                ] {
                    o.remove(k);
                }
            }
            norm(fx, &x, "")
        })
        .collect();
    v.sort_by_key(|x| x.to_string());
    v
}

/// The same landing with the memo on and off: the same exit, output, events and receipts.
fn same_with_and_without_the_memo(setup: fn(&Fx)) -> (Landing, Landing) {
    same_when(setup, None)
}

/// The same, with `during` run while each landing holds before the engine lock.
fn same_when(setup: fn(&Fx), during: Option<fn(&Fx)>) -> (Landing, Landing) {
    let on = land(setup, true, during);
    let off = land(setup, false, during);
    assert_eq!(
        on.out.code,
        off.out.code,
        "{}\n---\n{}",
        on.out.all(),
        off.out.all()
    );
    // The output's JSON names the short commit of the merge, which `norm` masks.
    let stdout = |l: &Landing| match serde_json::from_str::<Value>(&l.out.stdout) {
        Ok(j) => norm(&l.fx, &j, ""),
        Err(_) => norm(&l.fx, &Value::String(l.out.stdout.clone()), ""),
    };
    assert_eq!(stdout(&on), stdout(&off));
    // The warning of a busy machine names the load of the moment, which is no output of the landing.
    let stderr = |l: &Landing| {
        let kept: Vec<&str> = l
            .out
            .stderr
            .lines()
            .filter(|x| !x.contains("the machine is busy"))
            .collect();
        norm(&l.fx, &Value::String(kept.join("\n")), "")
    };
    assert_eq!(stderr(&on), stderr(&off));
    let new = |l: &Landing| -> Vec<Value> {
        events(&l.fx)[l.events_before..]
            .iter()
            .map(|e| norm(&l.fx, e, ""))
            .collect()
    };
    let (a, b) = (new(&on), new(&off));
    assert_eq!(a.len(), b.len(), "{a:#?}\n---\n{b:#?}");
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x, y);
    }
    assert_eq!(receipts(&on.fx), receipts(&off.fx));
    (on, off)
}

fn clean(fx: &Fx) {
    plan(fx);
    let wc = start(fx, "U-1");
    build(fx, "U-1", &wc, &[("src/a.txt", "unit 1\n")]);
}

#[test]
fn a_clean_landing_of_one_unit_makes_at_most_27_git_calls() {
    let (on, off) = same_with_and_without_the_memo(clean);
    assert_eq!(on.out.json()["landed"], true, "{}", on.out.all());
    assert!(
        on.calls.len() <= 27,
        "git calls of `bf unit land`: {:#?}",
        on.calls
    );
    assert!(off.calls.len() > on.calls.len(), "{:#?}", off.calls);
}

#[test]
fn a_landing_makes_the_same_events_receipts_and_output_with_the_memo_as_without() {
    let (on, _) = same_with_and_without_the_memo(clean);
    let kinds: Vec<String> = events(&on.fx)[on.events_before..]
        .iter()
        .map(|e| e["kind"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(kinds.iter().any(|k| k == "unit.land"), "{kinds:?}");
}

fn changed_after_review(fx: &Fx) {
    clean(fx);
    fs::write(
        fx.project().work_dir().join("U-1/src/a.txt"),
        "changed after the review\n",
    )
    .unwrap();
}

#[test]
fn a_change_to_the_unit_after_its_review_is_still_sent_back_to_review() {
    let (on, _) = same_with_and_without_the_memo(changed_after_review);
    assert_ne!(on.out.code, 0, "{}", on.out.all());
    assert!(
        on.out
            .all()
            .contains("is not the content its last clean review judged"),
        "{}",
        on.out.all()
    );
    let st = bf::events::replay(&on.fx.project()).unwrap();
    assert_eq!(st.unit_state("U-1"), "review");
}

/// U-2 lands first, so the checkpoint holds a file U-1 has not; U-1 brings a suite script that, in the
/// checkpoint's copy only, appends to a tracked file of U-1.
fn writing_check(fx: &Fx) {
    plan(fx);
    let w1 = start(fx, "U-1");
    let w2 = start(fx, "U-2");
    build(fx, "U-2", &w2, &[("src/b.txt", "unit 2\n")]);
    fx.must(&["unit", "land", "U-2"]);
    let script = "case \"$PWD\" in */K-1) echo written by the check >> src/a.txt ;; esac\nexit 0\n";
    build(
        fx,
        "U-1",
        &w1,
        &[("src/a.txt", "unit 1\n"), ("suite.sh", script)],
    );
}

#[test]
fn a_check_that_changes_a_tracked_file_during_the_landing_is_caught() {
    let (on, _) = same_with_and_without_the_memo(writing_check);
    assert_ne!(on.out.code, 0, "{}", on.out.all());
    assert!(
        on.out
            .all()
            .contains("the command changed the files of the working copy while it ran"),
        "{}",
        on.out.all()
    );
    let st = bf::events::replay(&on.fx.project()).unwrap();
    assert_eq!(st.unit_state("U-1"), "reviewed");
}

/// U-1 is reviewed; U-2 is started and holds no commit of its own yet.
fn unlanded_work(fx: &Fx) {
    plan(fx);
    let w1 = start(fx, "U-1");
    start(fx, "U-2");
    build(fx, "U-1", &w1, &[("src/a.txt", "unit 1\n")]);
}

/// While the landing waits, U-2's builder commits on its branch and another hand puts that never-landed
/// work on the main branch: the branch read before the lock saw U-2 with no work and main without it.
fn move_main(fx: &Fx) {
    let w2 = fx.project().work_dir().join("U-2");
    fs::create_dir_all(w2.join("src")).unwrap();
    fs::write(w2.join("src/b.txt"), "unit 2\n").unwrap();
    git(&w2, &["add", "src/b.txt"]);
    git(&w2, &["commit", "-q", "-m", "U-2 work"]);
    git(&fx.root, &["merge", "-q", "--no-edit", "bf/U-2"]);
}

#[test]
fn a_branch_moved_while_the_landing_waits_for_the_engine_lock_is_seen_under_it() {
    let (on, _) = same_when(unlanded_work, Some(move_main));
    assert_ne!(on.out.code, 0, "{}", on.out.all());
    assert!(
        on.out.all().contains("contains the commits of U-2"),
        "{}",
        on.out.all()
    );
    // The refusal came under the lock, before any merge or run of the suite.
    let kinds: Vec<String> = events(&on.fx)[on.events_before..]
        .iter()
        .map(|e| e["kind"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(!kinds.iter().any(|k| k == "unit.land"), "{kinds:?}");
    assert!(
        !on.calls.iter().any(|c| c.contains(" merge --no-ff")),
        "{:#?}",
        on.calls
    );
}

/// The memo itself: a write through git forgets what it may have changed.
#[test]
fn the_memo_forgets_what_a_write_may_have_changed() {
    use bf::gitx;
    let fx = Fx::new();
    let dir = fx.root.clone();
    let _memo = gitx::memo::begin();
    let h1 = gitx::workdir_hash(&dir, true).unwrap();
    assert_eq!(gitx::workdir_hash_memo(&dir, true).unwrap(), h1);
    // A write of a file in the copy by git (a checkout of a new file from a commit) clears the hash.
    fs::write(dir.join("memo.txt"), "one\n").unwrap();
    git(&dir, &["add", "memo.txt"]);
    git(&dir, &["commit", "-q", "-m", "memo"]);
    fs::remove_file(dir.join("memo.txt")).unwrap();
    gitx::must(&dir, &["checkout", "-q", "--", "memo.txt"]).unwrap();
    let h2 = gitx::workdir_hash_memo(&dir, true).unwrap();
    assert_ne!(h1, h2, "the hash came from the memo after a write");
    // A declared commit clears its copy's hash.
    fs::write(dir.join("memo.txt"), "two\n").unwrap();
    let h3 = gitx::workdir_hash(&dir, true).unwrap();
    gitx::add_all_without_junk(&dir).unwrap();
    gitx::must_moving(&dir, &["commit", "-q", "-m", "two"], &["main"], true).unwrap();
    fs::write(dir.join("memo.txt"), "three\n").unwrap();
    assert_ne!(gitx::workdir_hash_memo(&dir, true).unwrap(), h3);
    // A branch made by a write is seen.
    assert!(!gitx::branch_exists(&dir, "memo-branch"));
    gitx::must(&dir, &["branch", "memo-branch"]).unwrap();
    assert!(gitx::branch_exists(&dir, "memo-branch"));
    // A branch read is used again only until a write moves the branch.
    let tip = gitx::heads(&dir).tip("memo-branch").unwrap();
    gitx::must_moving(
        &dir,
        &["branch", "-f", "memo-branch", "HEAD~1"],
        &["memo-branch"],
        false,
    )
    .unwrap();
    assert_ne!(gitx::heads(&dir).tip("memo-branch").unwrap(), tip);
}
