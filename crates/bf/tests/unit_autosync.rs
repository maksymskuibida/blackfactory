//! U-82 (R-74 acceptance 22 to 27, D-93, D-102): the engine syncs a unit with its checkpoint branch by
//! itself at each start of its work, commits the builder's changes first, leaves a conflict in the
//! unit's own files to the builder and aborts any other.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Three units on one checkpoint; U-2 may also write `src/a.txt` when `shared` is set.
fn plan(fx: &Fx, _shared: bool) {
    let mut recs = vec![(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string(),
    )];
    for (n, f) in [(1, "a"), (2, "b"), (3, "c")] {
        let extra = "";
        recs.push(("unit", format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/{f}.txt\"{extra}]\ncheckpoint=\"K-1\"\n")));
        recs.push(("check", format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n")));
    }
    recs.push(("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"k\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".into()));
    recs.push(("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\",\"U-2\",\"U-3\"]\nproves=[\"R-1\"]\nchecks=[\"X-9\"]\n".into()));
    for (k, t) in recs {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.commit_all("records");
    fx.start_test_run();
}

fn start(fx: &Fx, u: &str) -> PathBuf {
    PathBuf::from(fx.must(&["unit", "start", u, "--json"]).json()["working_copy"].as_str().unwrap())
}

/// Land U-1 and U-3 so the checkpoint branch moves ahead of a unit started before.
fn advance(fx: &Fx) {
    fx.build_unit("U-1", "src/a.txt", "X-1", &["true"]);
    fx.build_unit("U-3", "src/c.txt", "X-3", &["true"]);
}

fn syncs(fx: &Fx) -> Vec<Value> {
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    log.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["kind"] == "unit.sync")
        .collect()
}

fn write(wc: &Path, file: &str, text: &str) {
    let dir = wc.join(file).parent().map(|d| d.to_path_buf()).unwrap();
    fs::create_dir_all(dir).unwrap();
    fs::write(wc.join(file), text).unwrap();
}

fn proof(fx: &Fx, wc: &Path, n: &str) {
    let (x, u) = (format!("X-{n}"), format!("U-{n}"));
    fx.must_in(wc, &["evidence", "run", &x, "--unit", &u, "--", "true"]);
}

fn directive(fx: &Fx, unit: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["unit"] == unit && d["role"] == "builder")
        .cloned()
        .unwrap_or_else(|| panic!("no builder directive for {unit}: {n}"))
}

/// Commit a change to `file` on the checkpoint branch, in the checkpoint's working copy.
fn ck_commit(fx: &Fx, file: &str, text: &str) {
    let list = git(&fx.root, &["worktree", "list", "--porcelain"]);
    let ck = list
        .split("\n\n")
        .find(|b| b.contains("refs/heads/bf/K-1"))
        .and_then(|b| b.lines().next())
        .map(|l| PathBuf::from(l.trim_start_matches("worktree ")))
        .expect("the checkpoint worktree");
    if let Some(d) = ck.join(file).parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(ck.join(file), text).unwrap();
    git(&ck, &["add", "-A"]);
    git(&ck, &["commit", "-q", "-m", "theirs"]);
}

const PROTECTED: &str = ".claude/settings.json";

fn merging(wc: &Path) -> bool {
    let g = git(wc, &["rev-parse", "--git-path", "MERGE_HEAD"]);
    let p = PathBuf::from(&g);
    (if p.is_absolute() { p } else { wc.join(g) }).exists()
}

#[test]
fn unit_autosync_start_commits_the_builders_work_and_merges() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, "src/b.txt", "mine\n");
    advance(&fx);
    let cp = git(&fx.root, &["rev-parse", "bf/K-1"]);
    fx.must(&["unit", "start", "U-2"]);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["cause"], "start");
    assert_eq!(ev[0]["how"], "merge");
    assert_eq!(ev[0]["new_base"], cp.as_str());
    assert_eq!(ev[0]["conflicts"].as_array().unwrap().len(), 0);
    let wip = ev[0]["wip_commit"].as_str().unwrap();
    assert!(!wip.is_empty());
    let msg = git(&wc, &["log", "-1", "--format=%s", wip]);
    assert!(msg.contains("U-2") && msg.contains("start"), "{msg}");
    assert_eq!(fs::read_to_string(wc.join("src/b.txt")).unwrap(), "mine\n");
    assert!(wc.join("src/a.txt").exists() && wc.join("src/c.txt").exists());
    assert_eq!(git(&wc, &["status", "--porcelain"]), "");
}

#[test]
fn unit_autosync_a_unit_with_no_work_is_fast_forwarded() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    advance(&fx);
    fx.must(&["unit", "start", "U-2"]);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1);
    assert_eq!((ev[0]["cause"].as_str(), ev[0]["how"].as_str()), (Some("start"), Some("fast-forward")));
    assert_eq!(ev[0]["wip_commit"], "");
    assert_eq!(git(&wc, &["rev-parse", "HEAD"]), git(&fx.root, &["rev-parse", "bf/K-1"]));
}

#[test]
fn unit_autosync_a_fix_round_start_syncs_the_same_way() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, "src/b.txt", "mine\n");
    proof(&fx, &wc, "2");
    fx.must(&["report", "U-2", "--result", "built"]);
    let finding = r#"[{"title":"Wording","severity":"medium","where":"src/b.txt:1","action":"Fix it"}]"#;
    let r = fx.bf_stdin(&fx.root, &["report", "U-2", "--result", "findings", "--role", "reviewer", "--findings", "-"], finding);
    assert_eq!(r.code, 0, "{}", r.all());
    advance(&fx);
    assert!(syncs(&fx).is_empty());
    let o = fx.bf(&["unit", "start", "U-2"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!((ev[0]["cause"].as_str(), ev[0]["how"].as_str()), (Some("start"), Some("merge")));
    assert!(!ev[0]["wip_commit"].as_str().unwrap().is_empty());
    assert_eq!(fs::read_to_string(wc.join("src/b.txt")).unwrap(), "mine\n");
}

#[test]
fn unit_autosync_release_syncs_with_its_cause() {
    let fx = Fx::new();
    plan(&fx, false);
    let w2 = start(&fx, "U-2");
    write(&w2, "src/b.txt", "mine\n");
    fx.must(&["report", "U-2", "--result", "blocked"]);
    advance(&fx);
    fx.human("Release it, please.");
    fx.must(&["unit", "release", "U-2", "--quote", "Release it"]);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!((ev[0]["cause"].as_str(), ev[0]["how"].as_str()), (Some("release"), Some("merge")));
    assert!(!ev[0]["wip_commit"].as_str().unwrap().is_empty());
}

#[test]
fn unit_autosync_unblock_takes_in_what_landed_meanwhile() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, "src/b.txt", "mine\n");
    for note in ["first", "needs U-1 to land"] {
        let r = fx.bf_in(&wc, &["report", "U-2", "--result", "blocked", "--note", note]);
        assert_eq!(r.code, 0, "{}", r.all());
    }
    advance(&fx);
    fx.must(&["unit", "unblock", "U-2", "--why", "U-1 landed"]);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!((ev[0]["cause"].as_str(), ev[0]["how"].as_str()), (Some("unblock"), Some("merge")));
    assert!(!ev[0]["wip_commit"].as_str().unwrap().is_empty());
    assert_eq!(fs::read_to_string(wc.join("src/b.txt")).unwrap(), "mine\n");
    assert!(wc.join("src/a.txt").exists());
}

#[test]
fn unit_autosync_a_conflict_in_the_units_own_file_is_left_for_the_builder() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, "src/b.txt", "mine b\n");
    // Something else lands a change to the unit's own file on the checkpoint branch.
    ck_commit(&fx, "src/b.txt", "theirs\n");
    let cp = git(&fx.root, &["rev-parse", "bf/K-1"]);
    fx.must(&["unit", "start", "U-2"]);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["how"], "merge");
    assert_eq!(ev[0]["conflicts"], serde_json::json!(["src/b.txt"]));
    assert_eq!(ev[0]["new_base"], cp.as_str());
    assert!(merging(&wc));
    assert!(fs::read_to_string(wc.join("src/b.txt")).unwrap().contains("<<<<<<<"));
    // The directive and the brief name the file.
    let d = directive(&fx, "U-2");
    assert_eq!(d["sync_conflicts"], serde_json::json!(["src/b.txt"]));
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(brief.contains("src/b.txt") && brief.contains("Sync conflicts"), "{brief}");
    // A report with marks left is refused, naming the file.
    let o = fx.bf_in(&wc, &["report", "U-2", "--result", "built"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("src/b.txt"), "{}", o.all());
    assert!(merging(&wc));
    // With the marks resolved the report is taken and the merge is finished with one commit.
    write(&wc, "src/b.txt", "resolved\n");
    proof(&fx, &wc, "2");
    let o = fx.bf_in(&wc, &["report", "U-2", "--result", "built"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!merging(&wc));
    let parents = git(&wc, &["log", "-1", "--format=%p"]);
    assert_eq!(parents.split_whitespace().count(), 2, "{parents}");
    assert!(git(&wc, &["log", "-1", "--format=%s"]).contains("U-2"));
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-2"]);
}

#[test]
fn unit_autosync_a_conflict_outside_the_units_files_is_left_for_the_builder() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, "src/a.txt", "own\n");
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "own"]);
    write(&wc, "src/b.txt", "loose\n");
    advance(&fx);
    fx.must(&["unit", "start", "U-2"]);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["how"], "merge");
    assert_eq!(ev[0]["conflicts"], serde_json::json!(["src/a.txt"]));
    assert!(merging(&wc));
    assert!(fs::read_to_string(wc.join("src/a.txt")).unwrap().contains("<<<<<<<"));
    let d = directive(&fx, "U-2");
    assert_eq!(d["sync_conflicts"], serde_json::json!(["src/a.txt"]));
    assert!(d.get("sync_aborted").is_none());
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(brief.contains("src/a.txt") && brief.contains("Sync conflicts"), "{brief}");
}

#[test]
fn unit_autosync_a_conflict_in_a_protected_path_is_aborted_and_named() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, PROTECTED, "own\n");
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "own"]);
    write(&wc, "src/b.txt", "loose\n");
    ck_commit(&fx, PROTECTED, "theirs\n");
    let base = bf::events::replay(&fx.project()).unwrap().units["U-2"].base_commit.clone();
    fx.must(&["unit", "start", "U-2"]);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["how"], "aborted");
    assert_eq!(ev[0]["files"], serde_json::json!([PROTECTED]));
    assert!(!merging(&wc));
    assert_eq!(fs::read_to_string(wc.join(PROTECTED)).unwrap(), "own\n");
    assert_eq!(git(&wc, &["status", "--porcelain"]), "");
    assert_eq!(fs::read_to_string(wc.join("src/b.txt")).unwrap(), "loose\n");
    let after = bf::events::replay(&fx.project()).unwrap().units["U-2"].base_commit.clone();
    assert_eq!(after, base);
    let d = directive(&fx, "U-2");
    assert_eq!(d["sync_aborted"], serde_json::json!([PROTECTED]));
    assert!(d.get("sync_conflicts").is_none());
}

#[test]
fn unit_autosync_a_unit_in_review_is_not_synced_and_a_receipt_before_a_sync_goes_stale() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, "src/b.txt", "mine\n");
    proof(&fx, &wc, "2");
    fx.must(&["report", "U-2", "--result", "built"]);
    advance(&fx);
    let o = fx.bf(&["unit", "start", "U-2"]);
    let _ = o;
    assert!(syncs(&fx).is_empty());
    assert!(!wc.join("src/a.txt").exists());
    // A receipt sealed before a sync that brought changes in no longer counts.
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, "src/b.txt", "mine\n");
    proof(&fx, &wc, "2");
    advance(&fx);
    fx.must_in(&wc, &["evidence", "verify", "--unit", "U-2"]);
    fx.must(&["unit", "start", "U-2"]);
    assert_eq!(syncs(&fx).len(), 1);
    let v = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-2"]);
    assert_ne!(v.code, 0, "{}", v.all());
}

#[test]
fn unit_autosync_an_aborted_notice_ends_when_the_merge_is_resolved_by_hand() {
    let fx = Fx::new();
    plan(&fx, false);
    let wc = start(&fx, "U-2");
    write(&wc, PROTECTED, "own\n");
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "own"]);
    ck_commit(&fx, PROTECTED, "theirs\n");
    fx.must(&["unit", "start", "U-2"]);
    assert_eq!(syncs(&fx)[0]["how"], "aborted");
    assert_eq!(directive(&fx, "U-2")["sync_aborted"], serde_json::json!([PROTECTED]));
    let st = fx.must(&["status", "--json"]).json();
    let u2 = st["units"].as_array().unwrap().iter().find(|u| u["id"] == "U-2").unwrap().clone();
    assert_eq!(u2["sync_aborted"], serde_json::json!([PROTECTED]));
    // The human merges by hand and resolves the conflict.
    let cp = git(&fx.root, &["rev-parse", "bf/K-1"]);
    let _ = fx.bf_in(&wc, &["status"]);
    let o = std::process::Command::new("git").current_dir(&wc).args(["merge", "--no-edit", "-q", &cp]).output().unwrap();
    assert!(!o.status.success());
    write(&wc, PROTECTED, "resolved\n");
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "merge"]);
    fx.must(&["unit", "start", "U-2"]);
    let d = directive(&fx, "U-2");
    assert!(d.get("sync_aborted").is_none(), "{d}");
    let brief = fs::read_to_string(d["brief"].as_str().unwrap()).unwrap();
    assert!(!brief.contains("Sync aborted"), "{brief}");
    let st = fx.must(&["status", "--json"]).json();
    let u2 = st["units"].as_array().unwrap().iter().find(|u| u["id"] == "U-2").unwrap().clone();
    assert_eq!(u2["sync_aborted"].as_array().unwrap().len(), 0, "{u2}");
}
