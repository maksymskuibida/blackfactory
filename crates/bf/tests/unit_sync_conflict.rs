//! U-112 (D-130): a conflict that `bf unit sync` meets is left in the unit's working copy for its
//! builder, as the automatic sync and a conflicting landing leave theirs. The merge lasts through a
//! block and a park, the report finishes it, the next reviewer is shown what the sync took in and how
//! its conflicts were resolved, and the unit lands with no human step. A conflict in a protected path is
//! aborted.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn plan(fx: &Fx) {
    let mut recs = vec![(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string(),
    )];
    for (n, f) in [(1, "a"), (2, "b")] {
        recs.push(("unit", format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/{f}.txt\"]\ncheckpoint=\"K-1\"\n")));
        recs.push(("check", format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n")));
    }
    recs.push(("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"k\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".into()));
    recs.push(("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\"]\nchecks=[\"X-9\"]\n".into()));
    for (k, t) in recs {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.commit_all("records");
    fx.start_test_run();
}

fn start(fx: &Fx, u: &str) -> PathBuf {
    PathBuf::from(
        fx.must(&["unit", "start", u, "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

fn write(wc: &Path, file: &str, text: &str) {
    if let Some(d) = wc.join(file).parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(wc.join(file), text).unwrap();
}

fn proof(fx: &Fx, wc: &Path, n: &str) {
    let (x, u) = (format!("X-{n}"), format!("U-{n}"));
    fx.must_in(wc, &["evidence", "run", &x, "--unit", &u, "--", "true"]);
}

fn state(fx: &Fx, u: &str) -> bf::events::UnitSt {
    bf::events::replay(&fx.project()).unwrap().units[u].clone()
}

fn merging(wc: &Path) -> bool {
    let g = git(wc, &["rev-parse", "--git-path", "MERGE_HEAD"]);
    let p = PathBuf::from(&g);
    (if p.is_absolute() { p } else { wc.join(g) }).exists()
}

fn log(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

fn syncs(fx: &Fx) -> Vec<Value> {
    log(fx)
        .into_iter()
        .filter(|e| e["kind"] == "unit.sync")
        .collect()
}

fn directive(fx: &Fx, u: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["unit"] == u)
        .unwrap_or_else(|| panic!("no directive for {u}: {n}"))
        .clone()
}

fn brief(fx: &Fx, u: &str, role: &str) -> String {
    fs::read_to_string(fx.project().briefs_dir().join(format!("{u}-{role}.md"))).unwrap()
}

fn guard(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> String {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => "abstain".into(),
        s => serde_json::from_str::<Value>(s).unwrap()["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap()
            .to_string(),
    }
}

/// U-1 and U-2 both write `file`. U-2 is built, sent back with findings, and its work committed (the
/// copy is clean, `src/b.txt` is its own). Then U-1 lands. Returns U-2's copy.
fn conflicting(fx: &Fx, file: &str) -> PathBuf {
    let (w1, w2) = (start(fx, "U-1"), start(fx, "U-2"));
    write(&w2, file, "two\n");
    write(&w2, "src/b.txt", "b1\n");
    proof(fx, &w2, "2");
    fx.must(&["report", "U-2", "--result", "built"]);
    let finding =
        r#"[{"title":"Wording","severity":"medium","where":"src/b.txt:1","action":"Fix it"}]"#;
    let r = fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "U-2",
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--findings",
            "-",
        ],
        finding,
    );
    assert_eq!(r.code, 0, "{}", r.all());
    git(&w2, &["add", "-A"]);
    git(&w2, &["commit", "-q", "-m", "work"]);
    write(&w1, file, "one\n");
    write(&w1, "src/from_one.txt", "brought in from one\n");
    proof(fx, &w1, "1");
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    w2
}

/// The builder's resolution: the marks removed from `file`, the file added.
fn resolve(wc: &Path, file: &str) {
    write(wc, file, "resolved\n");
    git(wc, &["add", file]);
}

#[test]
fn unit_sync_conflict_is_left_for_the_builder_finished_at_the_report_and_lands() {
    let fx = Fx::new();
    plan(&fx);
    let w2 = conflicting(&fx, "src/shared.txt");
    let token = directive(&fx, "U-2")["token"].clone();
    let head = git(&w2, &["rev-parse", "HEAD"]);
    let o = fx.must(&["unit", "sync", "U-2", "--json"]).json();
    assert!(o["conflicts"].to_string().contains("src/shared.txt"), "{o}");
    assert!(
        o["line"].as_str().unwrap().contains("src/shared.txt"),
        "{o}"
    );
    assert!(!o["line"].as_str().unwrap().contains("human"), "{o}");
    assert!(merging(&w2));
    assert!(fs::read_to_string(w2.join("src/shared.txt"))
        .unwrap()
        .contains("<<<<<<<"));
    assert_eq!(git(&w2, &["rev-parse", "HEAD"]), head);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    let last = ev.last().unwrap();
    assert_eq!(last["unit"], "U-2");
    assert_eq!(last["cause"], "sync");
    assert_eq!(last["conflicts"], json!(["src/shared.txt"]));
    // The builder's directive and brief carry the file, with the same token as before the sync.
    let d = directive(&fx, "U-2");
    assert_eq!(d["sync_conflicts"], json!(["src/shared.txt"]), "{d}");
    assert_eq!(d["token"], token);
    assert!(brief(&fx, "U-2", "builder").contains("src/shared.txt"));
    // A second sync is refused while the merge is in progress, naming the marked file.
    let again = fx.refused(&["unit", "sync", "U-2"]);
    assert!(again.all().contains("src/shared.txt"), "{}", again.all());
    assert_eq!(syncs(&fx).len(), 1);
    // A report with a mark left is refused naming the file.
    write(&w2, "src/b.txt", "b2\n");
    proof(&fx, &w2, "2");
    let r = fx.bf_in(&w2, &["report", "U-2", "--result", "fixed"]);
    assert_ne!(r.code, 0, "{}", r.all());
    assert!(r.all().contains("src/shared.txt"), "{}", r.all());
    assert!(merging(&w2));
    // The builder resolves and adds the file; the report commits the merge, with the edit of the other file.
    resolve(&w2, "src/shared.txt");
    proof(&fx, &w2, "2");
    let r = fx.bf_in(&w2, &["report", "U-2", "--result", "fixed"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(!merging(&w2));
    assert_eq!(
        git(&w2, &["rev-list", "--parents", "-n1", "HEAD"])
            .split(' ')
            .count(),
        3
    );
    let cpb = git(&fx.root, &["rev-parse", "bf/K-1"]);
    git(&w2, &["merge-base", "--is-ancestor", &cpb, "HEAD"]);
    assert_eq!(git(&w2, &["show", "HEAD:src/shared.txt"]), "resolved");
    assert_eq!(git(&w2, &["show", "HEAD:src/b.txt"]), "b2");
    assert_eq!(state(&fx, "U-2").state, "review");
    // The reviewer is shown what the sync took in and the resolution, not the checkpoint's file as the unit's.
    let rb = brief(&fx, "U-2", "reviewer");
    assert!(rb.contains("What the sync took in"), "{rb}");
    assert!(rb.contains("src/from_one.txt"), "{rb}");
    assert!(
        rb.contains("resolving the landing conflict") && rb.contains("+resolved"),
        "{rb}"
    );
    let since = rb.split("### Changes since round").nth(1).expect(&rb);
    assert!(!since.contains("from_one"), "{rb}");
    assert!(since.contains("b2"), "{rb}");
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-2"]);
    assert_eq!(state(&fx, "U-2").state, "landed");
}

#[test]
fn unit_sync_conflict_in_a_file_another_unit_lists_is_left_and_the_guard_lets_the_edit_pass() {
    let fx = Fx::new();
    plan(&fx);
    let w2 = conflicting(&fx, "src/a.txt");
    let o = fx.must(&["unit", "sync", "U-2", "--json"]).json();
    assert!(o["conflicts"].to_string().contains("src/a.txt"), "{o}");
    assert!(merging(&w2));
    let target = w2.join("src/a.txt");
    let d = guard(
        &fx,
        &w2,
        "Edit",
        json!({"file_path": target.to_string_lossy(), "old_string": "x", "new_string": "y"}),
    );
    assert_ne!(d, "deny");
    resolve(&w2, "src/a.txt");
    proof(&fx, &w2, "2");
    let r = fx.bf_in(&w2, &["report", "U-2", "--result", "fixed"]);
    assert_eq!(r.code, 0, "{}", r.all());
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-2"]);
    assert_eq!(state(&fx, "U-2").state, "landed");
}

#[test]
fn unit_sync_conflict_in_a_protected_path_is_aborted_and_the_copy_is_as_it_was() {
    let fx = Fx::new();
    plan(&fx);
    let w2 = conflicting(&fx, ".claude/settings.json");
    let (head, status) = (
        git(&w2, &["rev-parse", "HEAD"]),
        git(&w2, &["status", "--porcelain"]),
    );
    let o = fx.refused(&["unit", "sync", "U-2"]);
    assert!(o.all().contains(".claude/settings.json"), "{}", o.all());
    assert!(
        !o.all().contains("human resolves") && !o.all().contains("lands the unit first"),
        "{}",
        o.all()
    );
    assert!(!merging(&w2));
    assert_eq!(git(&w2, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&w2, &["status", "--porcelain"]), status);
    assert_eq!(syncs(&fx).last().unwrap()["how"], "aborted");
}

/// A conflict left by the sync, then the unit blocked or parked and released: nothing is merged, committed
/// or recorded again, and the directive and the brief still name the file.
fn lasts_through(release: impl Fn(&Fx)) {
    let fx = Fx::new();
    plan(&fx);
    let w2 = conflicting(&fx, "src/shared.txt");
    fx.must(&["unit", "sync", "U-2"]);
    write(&w2, "src/b.txt", "edited during the merge\n");
    release(&fx);
    let n = syncs(&fx).len();
    let head = git(&w2, &["rev-parse", "HEAD"]);
    let marks = fs::read_to_string(w2.join("src/shared.txt")).unwrap();
    assert!(merging(&w2) && marks.contains("<<<<<<<"));
    assert_eq!(
        fs::read_to_string(w2.join("src/b.txt")).unwrap(),
        "edited during the merge\n"
    );
    assert_eq!(git(&w2, &["rev-parse", "HEAD"]), head);
    assert_eq!(n, 1);
    assert_eq!(
        directive(&fx, "U-2")["sync_conflicts"],
        json!(["src/shared.txt"])
    );
    assert!(brief(&fx, "U-2", "builder").contains("src/shared.txt"));
}

#[test]
fn unit_sync_conflict_lasts_through_a_block_and_an_unblock() {
    lasts_through(|fx| {
        for note in ["first", "needs U-1 to land"] {
            let o = fx.bf_in(
                &fx.work("U-2"),
                &["report", "U-2", "--result", "blocked", "--note", note],
            );
            assert_eq!(o.code, 0, "{}", o.all());
        }
        fx.must(&["unit", "unblock", "U-2", "--why", "go on"]);
    });
}

#[test]
fn unit_sync_conflict_lasts_through_a_park_and_a_release() {
    lasts_through(|fx| {
        bf::events::append(
            &fx.project(),
            "unit.park",
            json!({"unit": "U-2", "reason": "idle for 90 minutes"}),
        )
        .unwrap();
        fx.human("Release it, please.");
        fx.must(&["unit", "release", "U-2", "--quote", "Release it"]);
    });
}

#[test]
fn unit_sync_conflict_at_a_landing_is_left_for_the_builder_and_the_unit_lands_after_review() {
    let fx = Fx::new();
    plan(&fx);
    let (w1, w2) = (start(&fx, "U-1"), start(&fx, "U-2"));
    for (w, n, text) in [(&w1, "1", "one\n"), (&w2, "2", "two\n")] {
        write(w, "src/shared.txt", text);
        proof(&fx, w, n);
        let u = format!("U-{n}");
        fx.must(&["report", &u, "--result", "built"]);
        fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
    }
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-2").state, "fixing");
    assert_eq!(
        directive(&fx, "U-2")["sync_conflicts"],
        json!(["src/shared.txt"])
    );
    resolve(&w2, "src/shared.txt");
    proof(&fx, &w2, "2");
    let r = fx.bf_in(&w2, &["report", "U-2", "--result", "fixed"]);
    assert_eq!(r.code, 0, "{}", r.all());
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-2"]);
    assert_eq!(state(&fx, "U-2").state, "landed");
}

#[test]
fn unit_sync_conflict_guard_lets_git_add_pass_and_refuses_merge_continue_and_abort() {
    let fx = Fx::new();
    plan(&fx);
    let w2 = conflicting(&fx, "src/shared.txt");
    fx.must(&["unit", "sync", "U-2"]);
    assert_ne!(
        guard(
            &fx,
            &w2,
            "Bash",
            json!({"command": "git add src/shared.txt"})
        ),
        "deny"
    );
    for cmd in ["git merge --continue", "git merge --abort"] {
        assert_eq!(
            guard(&fx, &w2, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
}
