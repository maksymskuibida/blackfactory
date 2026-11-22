//! U-89 (D-109): the sweep bounds the build folder of a working copy no agent is using and removes the
//! left-over working copies and briefs the engine will not read again.

mod common;
use common::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture() -> Fx {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fs::write(fx.root.join(".gitignore"), "target/\n.bf/\n").unwrap();
    fx.commit_all("ignore the build folder");
    fx
}

fn idle_zero(fx: &Fx) {
    fx.set_policy("idle_minutes = 60", "idle_minutes = 0");
}

fn sweep(fx: &Fx) -> Vec<bf::run::cleanup::sweep::Removed> {
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    bf::run::cleanup::sweep::sweep(&p, &st)
}

fn start(fx: &Fx, id: &str) -> PathBuf {
    let o = fx.must(&["unit", "start", id, "--json"]);
    PathBuf::from(o.json()["working_copy"].as_str().unwrap())
}

/// An ignored build folder and a new source file that git does not ignore.
fn dirty(wc: &Path) {
    fs::create_dir_all(wc.join("target/debug")).unwrap();
    fs::write(wc.join("target/debug/big.bin"), vec![7u8; 4096]).unwrap();
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/new.rs"), "fn main() {}\n").unwrap();
}

fn build_and_review(fx: &Fx, id: &str, wc: &Path, file: &str, check: &str) {
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join(file), format!("{id} content\n")).unwrap();
    fx.must_in(wc, &["evidence", "run", check, "--unit", id, "--", "test", "-f", file]);
    fx.must(&["report", id, "--result", "built"]);
    fx.must(&["report", id, "--result", "built", "--role", "reviewer"]);
}

#[test]
fn the_ignored_folder_of_a_blocked_unit_goes_and_a_new_file_stays() {
    let fx = fixture();
    let wc = start(&fx, "U-1");
    dirty(&wc);
    fx.must(&["report", "U-1", "--result", "blocked", "--note", "waiting"]);
    assert_eq!(bf::events::replay(&fx.project()).unwrap().unit_state("U-1"), "blocked");
    let out = sweep(&fx);
    assert!(!wc.join("target").exists(), "{out:?}");
    assert!(wc.join("src/new.rs").exists());
    assert!(out.iter().any(|r| r.kind == "build" && r.bytes >= 4096), "{out:?}");
}

#[test]
fn a_unit_with_an_event_inside_the_idle_limit_keeps_its_folder() {
    let fx = fixture();
    let wc = start(&fx, "U-1");
    dirty(&wc);
    assert!(sweep(&fx).is_empty());
    assert!(wc.join("target/debug/big.bin").exists());
}

#[test]
fn a_running_unit_with_a_live_builder_token_keeps_its_folder_past_the_limit() {
    let fx = fixture();
    idle_zero(&fx);
    fx.set_policy("tokens = false", "tokens = true");
    let wc = start(&fx, "U-1");
    dirty(&wc);
    fx.take_token("U-1", "builder");
    let tokens = fs::read_to_string(fx.state_dir().join("tokens.json")).unwrap();
    assert!(tokens.contains("U-1:builder"), "{tokens}");
    assert!(sweep(&fx).is_empty());
    assert!(wc.join("target/debug/big.bin").exists());
}

#[test]
fn a_reviewed_unit_with_no_token_and_an_old_event_loses_its_folder() {
    let fx = fixture();
    idle_zero(&fx);
    let wc = start(&fx, "U-1");
    build_and_review(&fx, "U-1", &wc, "src/hello.txt", "X-1");
    dirty(&wc);
    assert_eq!(bf::events::replay(&fx.project()).unwrap().unit_state("U-1"), "reviewed");
    sweep(&fx);
    assert!(!wc.join("target").exists());
    assert!(wc.join("src/new.rs").exists());
    assert!(wc.join("src/hello.txt").exists());
}

#[test]
fn a_running_check_holds_the_folder() {
    let fx = fixture();
    idle_zero(&fx);
    let wc = start(&fx, "U-1");
    dirty(&wc);
    fx.must(&["report", "U-1", "--result", "blocked", "--note", "waiting"]);
    let p = fx.project();
    fs::create_dir_all(p.receipts_dir()).unwrap();
    let mark = p.receipts_dir().join(format!(".output-{}.txt", std::process::id()));
    fs::write(&mark, "").unwrap();
    assert!(sweep(&fx).is_empty());
    assert!(wc.join("target").exists());
    fs::remove_file(&mark).unwrap();
    sweep(&fx);
    assert!(!wc.join("target").exists());
}

#[test]
fn a_checkpoint_folder_goes_when_nothing_is_open_and_the_limit_has_passed() {
    let fx = fixture();
    let wc = start(&fx, "U-1");
    build_and_review(&fx, "U-1", &wc, "src/hello.txt", "X-1");
    fx.must(&["unit", "land", "U-1"]);
    let k = fx.work("K-1");
    assert!(k.join(".git").exists());
    dirty(&k);
    let o = sweep(&fx);
    assert!(o.iter().all(|r| r.kind != "build"), "inside the limit {o:?}");
    assert!(k.join("target").exists());
    idle_zero(&fx);
    sweep(&fx);
    assert!(!k.join("target").exists());
    assert!(k.join("src/new.rs").exists());
}

#[test]
fn a_checkpoint_folder_stays_while_its_fix_is_open() {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"grep -q fixed src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fs::write(fx.root.join(".gitignore"), "target/\n.bf/\n").unwrap();
    fx.commit_all("records");
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = start(&fx, "U-1");
    build_and_review(&fx, "U-1", &wc, "src/a.txt", "X-1");
    fx.must(&["unit", "land", "U-1"]);
    let k = fx.work("K-1");
    let o = fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    assert!(!o.all().is_empty());
    fx.must(&["next", "--json"]);
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.ck_fix.is_some(), "a fix is open {}", o.all());
    idle_zero(&fx);
    dirty(&k);
    assert!(sweep(&fx).iter().all(|r| r.kind != "build"));
    assert!(k.join("target/debug/big.bin").exists());
}

#[test]
fn a_stale_review_copy_and_a_folder_of_a_unit_not_in_the_plan_go() {
    let fx = fixture();
    for name in ["K-1-review", "U-9"] {
        git(&fx.root, &["worktree", "add", "--detach", &fx.work(name).to_string_lossy(), "HEAD"]);
    }
    let out = sweep(&fx);
    assert!(!fx.work("K-1-review").exists(), "{out:?}");
    assert!(!fx.work("U-9").exists(), "{out:?}");
    assert!(!git(&fx.root, &["worktree", "list"]).contains(".bf/work"));
}

#[test]
fn a_folder_not_in_the_plan_with_an_uncommitted_file_stays_and_is_named() {
    let fx = fixture();
    let w = fx.work("U-9");
    git(&fx.root, &["worktree", "add", "--detach", &w.to_string_lossy(), "HEAD"]);
    fs::write(w.join("mine.txt"), "work\n").unwrap();
    let out = sweep(&fx);
    assert!(w.join("mine.txt").exists());
    let r = out.iter().find(|r| r.path == w).expect("named");
    assert!(r.kept.as_deref().unwrap_or("").contains("U-9"), "{r:?}");
}

#[test]
fn the_three_kinds_of_brief_go_and_the_brief_of_a_running_unit_stays() {
    use bf::events::{Amend, Closed, UnitSt};
    let fx = fixture();
    let p = fx.project();
    fs::create_dir_all(p.briefs_dir()).unwrap();
    for n in [
        "U-1-builder.md",
        "K-1-reviewer.md",
        "A-3-designer.md",
        "A-4-designer-idea.md",
        "U-2-builder.md",
        "A-5-designer.md",
        "U-1-notes.txt",
    ] {
        fs::write(p.briefs_dir().join(n), "x\n").unwrap();
    }
    let mut st = bf::events::replay(&p).unwrap();
    st.units.insert("U-1".into(), UnitSt { state: "landed".into(), ..Default::default() });
    st.units.insert("U-2".into(), UnitSt { state: "running".into(), ..Default::default() });
    st.closed.insert("K-1".into(), Closed::default());
    for (id, status) in [("A-3", "approved"), ("A-4", "withdrawn"), ("A-5", "open")] {
        st.amends.push(Amend { id: id.into(), status: status.into(), ..Default::default() });
    }
    let out = bf::run::cleanup::sweep::sweep(&p, &st);
    let gone = |n: &str| !p.briefs_dir().join(n).exists();
    assert!(gone("U-1-builder.md") && gone("K-1-reviewer.md"));
    assert!(gone("A-3-designer.md") && gone("A-4-designer-idea.md"));
    assert!(!gone("U-2-builder.md") && !gone("A-5-designer.md") && !gone("U-1-notes.txt"));
    assert_eq!(out.iter().filter(|r| r.kind == "brief").count(), 4, "{out:?}");
}

fn snapshot(dirs: &[PathBuf]) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(d: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if let Ok(rd) = fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.insert(p.clone(), fs::read(&p).unwrap_or_default());
                }
            }
        }
    }
    let mut m = BTreeMap::new();
    for d in dirs {
        walk(d, &mut m);
    }
    m
}

#[test]
fn what_the_sweep_never_touches_is_the_same_after_it() {
    let fx = fixture();
    idle_zero(&fx);
    let wc = start(&fx, "U-1");
    build_and_review(&fx, "U-1", &wc, "src/hello.txt", "X-1");
    dirty(&wc);
    let p = fx.project();
    for d in ["drafts", "pages"] {
        fs::create_dir_all(fx.root.join(".bf").join(d)).unwrap();
        fs::write(fx.root.join(".bf").join(d).join("f.txt"), format!("{d}\n")).unwrap();
    }
    let dirs = vec![
        fx.root.join("factory"),
        fx.root.join(".bf/drafts"),
        fx.root.join(".bf/pages"),
        fx.state_dir(),
    ];
    let before = snapshot(&dirs);
    let branches = git(&fx.root, &["branch", "--list"]);
    let out = sweep(&fx);
    assert!(!out.is_empty());
    assert_eq!(before, snapshot(&dirs));
    assert_eq!(branches, git(&fx.root, &["branch", "--list"]));
    assert!(p.work_dir().exists());
}

#[test]
fn a_folder_that_is_no_working_copy_stays_and_is_named() {
    let fx = fixture();
    let w = fx.work("U-9");
    fs::create_dir_all(&w).unwrap();
    fs::write(w.join("notes.txt"), "mine\n").unwrap();
    let out = sweep(&fx);
    assert!(w.join("notes.txt").exists(), "{out:?}");
    let r = out.iter().find(|r| r.path == w).expect("named");
    assert!(r.kept.as_deref().unwrap_or("").contains("U-9"), "{r:?}");
}

#[test]
fn a_copy_whose_git_link_is_broken_stays_and_is_named() {
    let fx = fixture();
    let w = fx.work("U-9");
    fs::create_dir_all(&w).unwrap();
    fs::write(w.join(".git"), "gitdir: /nonexistent/place/.git/worktrees/U-9\n").unwrap();
    fs::write(w.join("notes.txt"), "mine\n").unwrap();
    let out = sweep(&fx);
    assert!(w.join("notes.txt").exists(), "{out:?}");
    let r = out.iter().find(|r| r.path == w).expect("named");
    assert!(r.kept.as_deref().unwrap_or("").contains("U-9"), "{r:?}");
}
