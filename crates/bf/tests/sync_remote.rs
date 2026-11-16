//! `bf checkpoint sync` brings the main branch in from the remote first, by fast-forward only.
//! The remote is a local bare repository, so no network is needed.

mod common;
use common::*;
use std::fs;
use std::path::{Path, PathBuf};

fn setup() -> (Fx, PathBuf) {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
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
    fx.must(&["unit", "land", "U-1"]);
    let root = fx.root.clone();
    (fx, root)
}

/// A bare repository next to the project, made the project's `origin` and filled from its main branch.
fn add_remote(code: &Path) -> PathBuf {
    let bare = code.parent().unwrap().join("remote.git");
    git(
        code.parent().unwrap(),
        &["init", "-q", "--bare", bare.to_str().unwrap()],
    );
    git(code, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(code, &["push", "-q", "origin", "main"]);
    bare
}

/// A second clone that commits to the remote's main branch.
fn remote_commit(bare: &Path, file: &str) {
    let other = bare.parent().unwrap().join("other");
    git(
        bare.parent().unwrap(),
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    fs::write(other.join(file), "from the remote\n").unwrap();
    git(&other, &["add", file]);
    git(&other, &["commit", "-q", "-m", "remote work"]);
    git(&other, &["push", "-q", "origin", "main"]);
    fs::remove_dir_all(&other).unwrap();
}

fn sync(fx: &Fx, words: &str, extra: &[&str]) -> Out {
    fx.human(words);
    let mut a = vec!["checkpoint", "sync", "K-1", "--quote", words, "--json"];
    a.extend_from_slice(extra);
    fx.bf(&a)
}

#[test]
fn the_sync_fast_forwards_main_from_the_remote_then_merges_it() {
    let (fx, code) = setup();
    let bare = add_remote(&code);
    remote_commit(&bare, "from_remote.txt");
    let bare_before = git(&bare, &["rev-parse", "main"]);
    let o = sync(&fx, "pull main in from the remote", &[]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["synced"], true, "{}", o.all());
    assert_eq!(o.json()["main_update"][0]["updated"], true);
    assert!(
        code.join("from_remote.txt").exists(),
        "main was fast-forwarded"
    );
    assert!(git(&code, &["ls-tree", "-r", "--name-only", "bf/K-1"]).contains("from_remote.txt"));
    assert_eq!(
        git(&bare, &["rev-parse", "main"]),
        bare_before,
        "nothing is pushed"
    );
}

#[test]
fn a_main_branch_that_cannot_be_fast_forwarded_is_refused_and_nothing_is_merged() {
    let (fx, code) = setup();
    let bare = add_remote(&code);
    remote_commit(&bare, "from_remote.txt");
    fs::write(code.join("local.txt"), "local\n").unwrap();
    git(&code, &["add", "local.txt"]);
    git(&code, &["commit", "-q", "-m", "local work"]);
    let main_before = git(&code, &["rev-parse", "main"]);
    let k_before = git(&code, &["rev-parse", "bf/K-1"]);
    let o = sync(&fx, "pull main in from the remote", &[]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.all().contains("cannot be fast-forwarded"), "{}", o.all());
    assert!(o.all().contains("nothing was merged"), "{}", o.all());
    assert_eq!(git(&code, &["rev-parse", "main"]), main_before);
    assert_eq!(git(&code, &["rev-parse", "bf/K-1"]), k_before);
    assert!(!fx
        .must(&["log", "--json"])
        .stdout
        .contains("checkpoint.sync"));
    // Not even `--local` goes around it.
    let o = sync(&fx, "pull main in from the remote, locally", &["--local"]);
    assert_eq!(o.code, 2, "{}", o.all());
}

#[test]
fn with_no_remote_the_sync_says_so_and_goes_on_only_when_told() {
    let (fx, code) = setup();
    fs::write(code.join("local.txt"), "local\n").unwrap();
    git(&code, &["add", "local.txt"]);
    git(&code, &["commit", "-q", "-m", "local work"]);
    let k_before = git(&code, &["rev-parse", "bf/K-1"]);
    let o = sync(&fx, "take the local main in", &[]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.all().contains("no remote"), "{}", o.all());
    assert!(o.all().contains("--local"), "{}", o.all());
    assert_eq!(git(&code, &["rev-parse", "bf/K-1"]), k_before);
    let o = sync(&fx, "take the local main in, local only", &["--local"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["synced"], true, "{}", o.all());
    assert!(o.json()["main_update"][0]["note"]
        .as_str()
        .unwrap()
        .contains("no remote"));
}

#[test]
fn a_remote_out_of_reach_is_said_and_goes_on_only_when_told() {
    let (fx, code) = setup();
    let bare = add_remote(&code);
    fs::write(code.join("local.txt"), "local\n").unwrap();
    git(&code, &["add", "local.txt"]);
    git(&code, &["commit", "-q", "-m", "local work"]);
    fs::remove_dir_all(&bare).unwrap();
    let o = sync(&fx, "take main in", &[]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.all().contains("out of reach"), "{}", o.all());
    let o = sync(&fx, "take main in, local only", &["--local"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(o.json()["main_update"][0]["note"]
        .as_str()
        .unwrap()
        .contains("out of reach"));
}

#[test]
fn without_the_humans_words_the_remote_is_not_reached() {
    let (fx, code) = setup();
    let bare = add_remote(&code);
    remote_commit(&bare, "from_remote.txt");
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--quote", "nobody said this"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(!code.join("from_remote.txt").exists());
}
