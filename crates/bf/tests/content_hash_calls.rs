//! U-117 (C-7): the content hash takes junk and the engine's folders out of its temporary index with
//! one call, and writes the same tree as the old sequence (`add -A`, two `rm --cached`, `write-tree`).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str], index: Option<&Path>) -> String {
    let mut c = Command::new("git");
    c.current_dir(dir).args(args);
    if let Some(i) = index {
        c.env("GIT_INDEX_FILE", i);
    }
    let o = c.output().unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn put(top: &Path, rel: &str, text: &str) {
    let p = top.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// A working copy that tracks OS junk, a `factory/` file and a `.bf/` file, and holds an untracked file.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let top = d.path().to_path_buf();
    git(&top, &["init", "-q"], None);
    git(&top, &["config", "user.email", "t@example.com"], None);
    git(&top, &["config", "user.name", "t"], None);
    put(&top, "src/a.txt", "a\n");
    put(&top, "src/.DS_Store", "junk\n");
    put(&top, "src/._a.txt", "junk\n");
    put(&top, "Thumbs.db", "junk\n");
    put(&top, "factory/x.toml", "x\n");
    put(&top, ".bf/y", "y\n");
    git(&top, &["add", "-A", "-f"], None);
    git(&top, &["commit", "-q", "-m", "x"], None);
    put(&top, "src/a.txt", "changed\n");
    put(&top, "new.txt", "new\n");
    put(&top, "factory/z.toml", "z\n");
    (d, top)
}

/// The tree the old sequence makes, on its own temporary index.
fn old_tree(top: &Path, exclude_factory: bool) -> String {
    let idx = top.join(".git/old-index");
    fs::copy(top.join(".git/index"), &idx).unwrap();
    let i = Some(idx.as_path());
    let mut add = vec!["add", "-A", "--", "."];
    if exclude_factory {
        add.push(":(exclude)factory");
    }
    add.extend([
        ":(exclude,glob)**/.DS_Store",
        ":(exclude,glob)**/._*",
        ":(exclude,glob)**/Thumbs.db",
    ]);
    git(top, &add, i);
    git(
        top,
        &[
            "rm",
            "-r",
            "--cached",
            "-q",
            "--ignore-unmatch",
            "--",
            ":(glob)**/.DS_Store",
            ":(glob)**/._*",
            ":(glob)**/Thumbs.db",
        ],
        i,
    );
    if exclude_factory {
        git(
            top,
            &[
                "rm",
                "-r",
                "--cached",
                "-q",
                "--ignore-unmatch",
                "--",
                "factory",
                ".bf",
            ],
            i,
        );
    }
    let t = git(top, &["write-tree"], i);
    fs::remove_file(&idx).unwrap();
    t
}

#[test]
fn the_hash_equals_the_old_sequences_tree_with_exclude_factory() {
    let (_d, top) = repo();
    let want = old_tree(&top, true);
    assert_eq!(bf::gitx::content_hash(&top).unwrap(), want);
}

#[test]
fn the_hash_equals_the_old_sequences_tree_without_exclude_factory() {
    let (_d, top) = repo();
    let want = old_tree(&top, false);
    assert_eq!(bf::gitx::workdir_hash(&top, false).unwrap(), want);
}

#[test]
fn the_hash_with_exclude_factory_makes_three_git_calls() {
    let (_d, top) = repo();
    let outside = tempfile::tempdir().unwrap();
    let bin = outside.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = bin.join("calls.log");
    fs::write(&log, "").unwrap();
    let script = bin.join("git");
    let real = [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/usr/bin/git",
    ]
    .into_iter()
    .find(|g| Path::new(g).exists())
    .unwrap();
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
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    bf::gitx::configure(&script.to_string_lossy(), &path, false, "", "");
    let want = old_tree(&top, true);
    fs::write(&log, "").unwrap();
    let got = bf::gitx::content_hash(&top).unwrap();
    assert_eq!(got, want);
    let calls: Vec<String> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| l.to_string())
        .collect();
    assert_eq!(calls.len(), 3, "{calls:#?}");
    assert_eq!(calls.iter().filter(|l| l.contains("--cached")).count(), 1);
}
