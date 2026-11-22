//! U-109 (A-73, D-87): `bf unit sync` brings the checkpoint branch into a waiting unit's branch; start
//! and release fast-forward a clean unit with no commits of its own.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn plan(fx: &Fx) {
    let mut recs = vec![(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string(),
    )];
    for (n, f) in [(1, "a"), (2, "b"), (3, "c")] {
        recs.push(("unit", format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/{f}.txt\"]\ncheckpoint=\"K-1\"\n")));
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

fn proof(fx: &Fx, wc: &Path, n: &str) {
    let (x, u) = (format!("X-{n}"), format!("U-{n}"));
    fx.must_in(wc, &["evidence", "run", &x, "--unit", &u, "--", "true"]);
}

fn tip(wc: &Path) -> String {
    git(wc, &["rev-parse", "HEAD"])
}

fn own_commit(wc: &Path, file: &str) {
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join(file), "own\n").unwrap();
    git(wc, &["add", "-A"]);
    git(wc, &["commit", "-q", "-m", "own"]);
}

#[test]
fn a_unit_with_no_commits_of_its_own_is_fast_forwarded() {
    let fx = Fx::new();
    plan(&fx);
    let wc = start(&fx, "U-2");
    let before = bf::events::replay(&fx.project()).unwrap().units["U-2"].clone();
    advance(&fx);
    let cp = git(&fx.root, &["rev-parse", "bf/K-1"]);
    let o = fx.must(&["unit", "sync", "U-2", "--json"]).json();
    assert_eq!(o["how"], "fast-forward", "{o}");
    assert_eq!(tip(&wc), cp);
    let ev = syncs(&fx);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["new_base"], cp.as_str());
    assert_eq!(ev[0]["old_base"], before.base_commit.as_str());
    let after = bf::events::replay(&fx.project()).unwrap().units["U-2"].clone();
    let (a, b) = ((&after.state, after.rounds), (&before.state, before.rounds));
    assert_eq!((a, after.attempts), (b, before.attempts));
    assert_eq!(after.base_commit, cp);
    let again = fx.refused(&["unit", "sync", "U-2"]);
    assert!(again.all().contains("nothing to do"), "{}", again.all());
}

#[test]
fn a_unit_with_commits_gets_a_merge_and_lands_with_its_own_scope() {
    let fx = Fx::new();
    plan(&fx);
    let wc = start(&fx, "U-2");
    own_commit(&wc, "src/b.txt");
    advance(&fx);
    let o = fx.must(&["unit", "sync", "U-2", "--json"]).json();
    assert_eq!(o["how"], "merge", "{o}");
    assert!(wc.join("src/a.txt").exists() && wc.join("src/c.txt").exists());
    assert_eq!(git(&wc, &["status", "--porcelain"]), "");
    proof(&fx, &wc, "2");
    fx.must(&["report", "U-2", "--result", "built"]);
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-2"]);
}

#[test]
fn a_conflict_is_refused_and_leaves_the_working_copy_as_it_was() {
    let fx = Fx::new();
    plan(&fx);
    let wc = start(&fx, "U-2");
    own_commit(&wc, "src/a.txt");
    advance(&fx);
    let (head, status) = (tip(&wc), git(&wc, &["status", "--porcelain"]));
    let o = fx.refused(&["unit", "sync", "U-2"]);
    assert!(o.all().contains("src/a.txt"), "{}", o.all());
    assert_eq!(tip(&wc), head);
    assert_eq!(git(&wc, &["status", "--porcelain"]), status);
    assert_eq!(fs::read_to_string(wc.join("src/a.txt")).unwrap(), "own\n");
    assert!(syncs(&fx).is_empty());
}

#[test]
fn a_dirty_copy_and_a_unit_in_review_are_refused() {
    let fx = Fx::new();
    plan(&fx);
    let wc = start(&fx, "U-2");
    advance(&fx);
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/b.txt"), "loose\n").unwrap();
    let o = fx.refused(&["unit", "sync", "U-2"]);
    assert!(o.all().contains("working copy"), "{}", o.all());
    proof(&fx, &wc, "2");
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "own"]);
    fx.must(&["report", "U-2", "--result", "built"]);
    let o = fx.refused(&["unit", "sync", "U-2"]);
    assert!(o.all().contains("review"), "{}", o.all());
    assert!(syncs(&fx).is_empty());
}

#[test]
fn a_receipt_sealed_before_the_sync_is_stale_after_it() {
    let fx = Fx::new();
    plan(&fx);
    let wc = start(&fx, "U-2");
    own_commit(&wc, "src/b.txt");
    proof(&fx, &wc, "2");
    advance(&fx);
    fx.must_in(&wc, &["evidence", "verify", "--unit", "U-2"]);
    fx.must(&["unit", "sync", "U-2"]);
    let v = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-2"]);
    assert_ne!(v.code, 0, "{}", v.all());
}

#[test]
fn start_fast_forwards_a_made_unit_that_is_behind() {
    let fx = Fx::new();
    plan(&fx);
    let main = git(&fx.root, &["rev-parse", "HEAD"]);
    start(&fx, "U-1");
    git(&fx.root, &["branch", "bf/U-2", &main]);
    advance(&fx);
    let o = fx.must(&["unit", "start", "U-2", "--json"]).json();
    assert!(o.get("behind").is_none(), "{o}");
    let wc = PathBuf::from(o["working_copy"].as_str().unwrap());
    assert_eq!(tip(&wc), git(&fx.root, &["rev-parse", "bf/K-1"]));
    assert_eq!(syncs(&fx)[0]["cause"], "start");
}

#[test]
fn release_syncs_a_unit_without_commits_by_fast_forward_and_one_with_commits_by_merge() {
    let fx = Fx::new();
    plan(&fx);
    let (w2, w3) = (start(&fx, "U-2"), start(&fx, "U-3"));
    own_commit(&w3, "src/c.txt");
    for u in ["U-2", "U-3"] {
        fx.must(&["report", u, "--result", "blocked"]);
    }
    fx.build_unit("U-1", "src/a.txt", "X-1", &["true"]);
    for (u, q) in [("U-2", "Release it"), ("U-3", "Release it too")] {
        fx.human(&format!("{q}, please."));
        let o = fx.must(&["unit", "release", u, "--quote", q, "--json"]).json();
        assert!(o.get("behind").is_none(), "{o}");
    }
    assert_eq!(tip(&w2), git(&fx.root, &["rev-parse", "bf/K-1"]));
    assert_eq!(syncs(&fx).len(), 2);
}
