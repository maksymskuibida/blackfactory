//! U-53 (C-7): the copied index keeps the real index's modified time, so a file rewritten with the
//! same size in the same tick as the index is still hashed by its content.

use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

fn git(dir: &Path, args: &[&str], index: Option<&Path>) -> String {
    let mut c = Command::new("git");
    c.current_dir(dir).args(args);
    if let Some(i) = index {
        c.env("GIT_INDEX_FILE", i);
    }
    let o = c.output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn set_time(p: &Path, t: SystemTime) {
    File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
}

fn repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let top = d.path().to_path_buf();
    git(&top, &["init", "-q"], None);
    git(&top, &["config", "user.email", "t@example.com"], None);
    git(&top, &["config", "user.name", "t"], None);
    git(&top, &["config", "core.trustctime", "false"], None);
    git(&top, &["config", "core.checkStat", "minimal"], None);
    (d, top)
}

#[test]
fn a_same_size_rewrite_in_the_tick_of_the_index_is_hashed_by_content() {
    let (_d, top) = repo();
    let f = top.join("a.txt");
    let past = SystemTime::now() - Duration::from_secs(3600);
    fs::write(&f, "one\n").unwrap();
    set_time(&f, past);
    git(&top, &["add", "a.txt"], None);
    git(&top, &["commit", "-q", "-m", "x"], None);
    // Rewrite with the same size, the same file time, and give the index that same time.
    fs::write(&f, "two\n").unwrap();
    set_time(&f, past);
    set_time(&top.join(".git/index"), past);

    // Make the copy look newer than it is, as a platform that does not keep times on copy does.
    std::env::set_var("BF_TEST_INDEX_COPY_FORWARD", "1");
    let first = bf::gitx::content_hash(&top).unwrap();
    let second = bf::gitx::content_hash(&top).unwrap();
    assert_eq!(first, second, "two hashes of unchanged content must agree");

    let fresh = top.join(".git/fresh-index");
    git(&top, &["add", "-A"], Some(&fresh));
    let tree = git(&top, &["write-tree"], Some(&fresh));
    let committed = git(&top, &["rev-parse", "HEAD^{tree}"], None);
    assert_ne!(tree, committed, "the rewrite must change the tree");
    let direct = git(&top, &["hash-object", "a.txt"], None);
    let listed = git(&top, &["ls-tree", &tree, "a.txt"], None);
    assert!(listed.contains(&direct), "fresh tree holds the new content");
    // content_hash is a tree-hash of the same content, so it matches the fresh tree.
    assert_eq!(first, tree);
}
