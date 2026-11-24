//! U-88 (D-109, D-113): a landed unit's working copy and a closed checkpoint's working copies are
//! removed, the branches stay, and a unit sent back to work gets its copy again from its branch.

mod common;
use common::*;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn fixture() -> Fx {
    Fx::tiny_run()
}

/// Start, build and report a unit; land it and return the output of the landing.
fn land(fx: &Fx, id: &str, file: &str, check: &str) -> serde_json::Value {
    let o = fx.must(&["unit", "start", id, "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join(file), format!("{id} content\n")).unwrap();
    fx.must_in(&wc, &["evidence", "run", check, "--unit", id, "--", "test", "-f", file]);
    fx.must(&["report", id, "--result", "built"]);
    fx.must(&["report", id, "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", id, "--json"]).json()
}

fn worktrees(fx: &Fx) -> String {
    git(&fx.root, &["worktree", "list"])
}

fn branch_ok(fx: &Fx, a: &str, b: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(&fx.root)
        .args(["merge-base", "--is-ancestor", a, b])
        .status()
        .unwrap()
        .success()
}

#[test]
fn a_landed_units_copy_is_gone_and_its_branch_stands() {
    let fx = fixture();
    let out = land(&fx, "U-1", "src/hello.txt", "X-1");
    let p = fx.project();
    let wc = fx.work("U-1");
    assert!(!wc.exists(), "the folder is removed: {out}");
    assert_eq!(out["removed"].as_str().unwrap(), wc.to_string_lossy());
    assert!(out["kept"].is_null(), "{out}");
    assert!(!worktrees(&fx).contains("U-1"), "{}", worktrees(&fx));
    let (ub, cpb) = (p.unit_branch("U-1"), p.checkpoint_branch("K-1"));
    assert!(bf::gitx::branch_exists(&fx.root, &ub));
    assert!(branch_ok(&fx, &ub, &cpb));
}

#[test]
fn a_landed_copy_with_an_uncommitted_file_is_kept_and_named() {
    let fx = fixture();
    land(&fx, "U-1", "src/hello.txt", "X-1");
    let p = fx.project();
    let wc = fx.work("U-1");
    git(&fx.root, &["worktree", "add", &wc.to_string_lossy(), &p.unit_branch("U-1")]);
    fs::write(wc.join("src/stray.txt"), "mine\n").unwrap();
    match bf::run::cleanup::remove_unit_copy(&p, "U-1").unwrap() {
        bf::run::cleanup::Removal::Kept(why) => {
            assert!(why.contains("not committed"), "{why}")
        }
        bf::run::cleanup::Removal::Removed(_) => panic!("a copy with work in it was removed"),
    }
    assert!(wc.join("src/stray.txt").exists());
    // Once the file is gone the same call removes it, and a second call on the gone folder is no error.
    fs::remove_file(wc.join("src/stray.txt")).unwrap();
    assert!(matches!(
        bf::run::cleanup::remove_unit_copy(&p, "U-1").unwrap(),
        bf::run::cleanup::Removal::Removed(_)
    ));
    assert!(!wc.exists());
    assert!(matches!(
        bf::run::cleanup::remove_unit_copy(&p, "U-1").unwrap(),
        bf::run::cleanup::Removal::Removed(_)
    ));
}

#[test]
fn a_close_removes_the_three_kinds_of_folder_and_a_merge_still_works() {
    // A real run: a run with simulated approvals can never merge onto the main branch.
    let fx = Fx::tiny_project();
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    fx.must(&["gate", "open", "plan"]);
    fx.human("The plan looks right. I approve it.");
    fx.must(&["gate", "approve", "plan", "--quote", "The plan looks right. I approve it."]);
    land(&fx, "U-1", "src/hello.txt", "X-1");
    land(&fx, "U-2", "src/bye.txt", "X-2");
    let k = fx.work("K-1");
    assert!(k.join(".git").exists());
    // A review copy and a wave copy of the checkpoint, as the engine makes them: detached worktrees.
    for name in ["K-1-review", "K-1-wave-1"] {
        git(&fx.root, &["worktree", "add", "--detach", &fx.work(name).to_string_lossy(), "HEAD"]);
    }
    fx.must_in(
        &k,
        &["evidence", "run", "X-3", "--", "ls", "src/hello.txt", "src/bye.txt"],
    );
    fx.must(&["checkpoint", "close", "K-1"]);
    for name in ["K-1", "K-1-review", "K-1-wave-1"] {
        assert!(!fx.work(name).exists(), "{name} is removed");
    }
    assert!(!worktrees(&fx).contains(".bf/work"), "{}", worktrees(&fx));
    assert!(bf::gitx::branch_exists(&fx.root, &fx.project().checkpoint_branch("K-1")));
    // Closed already, no copy: the merge takes the hash of the close event.
    let o = fx.merge_checkpoint("K-1");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["merged"], true, "{}", o.all());
    assert!(fx.root.join("src/hello.txt").exists());
    assert!(!fx.work("K-1").exists());
}

#[test]
fn a_landed_unit_sent_back_by_an_amendment_gets_its_copy_at_the_first_brief() {
    let fx = fixture();
    land(&fx, "U-1", "src/hello.txt", "X-1");
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    let landed_hash = st.units["U-1"].landed_hash.clone();
    assert!(!fx.work("U-1").exists());
    let n = fs::read_to_string(p.log_path()).unwrap().lines().count() as u64;
    let ev = |kind: &str, v: serde_json::Value| {
        bf::events::append(&p, kind, v).unwrap();
    };
    ev("amend.open", json!({"amend": "A-1", "ids": ["R-1"], "impact": null, "impact_ids": ["U-1"]}));
    ev("amend.narrow", json!({"amend": "A-1", "keep": ["U-1"], "waive": [], "reasons": {}}));
    ev("human.turn", json!({"text": "Yes, approve it.", "transcript": ""}));
    ev(
        "amend.approve",
        json!({"amend": "A-1", "turn": n + 3, "quote": "approve it", "reopen_units": ["U-1"],
            "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0, "renamed": []}),
    );
    let st = bf::events::replay(&p).unwrap();
    assert_eq!(st.units["U-1"].state, "running", "{:?}", st.ignored);
    assert!(!fx.work("U-1").exists());
    fx.must(&["brief", "U-1", "--role", "builder"]);
    let wc = fx.work("U-1");
    assert!(wc.join(".git").exists(), "the copy is made again");
    assert_eq!(p.content_hash(&wc).unwrap(), landed_hash);
}
