//! Git access. Reads and writes both use the `git` command; it is never called
//! from `bf guard`. The engine runs the `git` program; it does not use a Rust git library.

use crate::refusal::{err, R};
use crate::store::git_dir_of;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The git program and the PATH to give it, from the policy (captured at `bf init`).
static GIT_CFG: std::sync::Mutex<Option<(String, String, Option<String>)>> =
    std::sync::Mutex::new(None);

/// The git program, the PATH to give it, and the sandbox profile to run it under (if any).
pub fn configure(git: &str, path: &str, profile: Option<String>) {
    *GIT_CFG.lock().unwrap() = Some((git.to_string(), path.to_string(), profile));
}

pub fn run(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> R<Out> {
    let (prog, path, profile) = GIT_CFG
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| (String::new(), String::new(), None));
    let program = if prog.is_empty() {
        "git".to_string()
    } else {
        prog
    };
    // Under the sandbox the program is named in full: `sandbox-exec` does not search a path.
    let program = if profile.is_some() && !program.contains('/') {
        path.split(':')
            .map(|d| format!("{d}/{program}"))
            .find(|c| Path::new(c).is_file())
            .unwrap_or(program)
    } else {
        program
    };
    let mut c = match &profile {
        Some(pr) => crate::sandbox::command(pr, &program),
        None => Command::new(&program),
    };
    c.arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Black Factory",
            "-c",
            "user.email=bf@localhost",
            "-c",
            "commit.gpgsign=false",
        ])
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
        ])
        .args(args)
        .stdin(Stdio::null());
    // A clean environment for git: the caller's GIT_DIR, GIT_WORK_TREE, GIT_INDEX_FILE and the
    // like must not decide which repository is read; the user's and the system's git config do not apply.
    for (k, _) in std::env::vars_os().filter_map(|(k, v)| k.to_str().map(|k| (k.to_string(), v))) {
        if k.starts_with("GIT_") {
            c.env_remove(k);
        }
    }
    if !path.is_empty() {
        c.env("PATH", &path);
    }
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    for (k, v) in envs {
        c.env(k, v);
    }
    let o = c
        .output()
        .map_err(|e| err(format!("cannot run git: {e}")))?;
    Ok(Out {
        ok: o.status.success(),
        stdout: String::from_utf8_lossy(&o.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
    })
}

pub fn must(dir: &Path, args: &[&str]) -> R<String> {
    let o = run(dir, args, &[])?;
    if o.ok {
        Ok(o.stdout)
    } else {
        Err(err(format!("git {} failed: {}", args.join(" "), o.stderr)))
    }
}

pub fn branch_exists(dir: &Path, name: &str) -> bool {
    run(
        dir,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
        &[],
    )
    .map(|o| o.ok)
    .unwrap_or(false)
}

pub fn is_ancestor(dir: &Path, a: &str, b: &str) -> bool {
    run(dir, &["merge-base", "--is-ancestor", a, b], &[])
        .map(|o| o.ok)
        .unwrap_or(false)
}

pub fn current_branch(dir: &Path) -> Option<String> {
    run(dir, &["symbolic-ref", "--short", "-q", "HEAD"], &[])
        .ok()
        .filter(|o| o.ok)
        .map(|o| o.stdout)
}

pub fn head_short(dir: &Path) -> String {
    run(dir, &["rev-parse", "--short", "HEAD"], &[])
        .ok()
        .filter(|o| o.ok)
        .map(|o| o.stdout)
        .unwrap_or_default()
}

pub fn is_repo(dir: &Path) -> bool {
    run(dir, &["rev-parse", "--git-dir"], &[])
        .map(|o| o.ok)
        .unwrap_or(false)
}

/// Content hash of a working copy.
///
/// It is the git tree-object hash of every file under the working copy's top
/// folder that is tracked, staged, or untracked and not ignored, taken as the
/// files are on disk right now, leaving out the folders `factory/` and `.bf/`.
/// It is computed in a temporary copy of the index, so the real index and the
/// repository history are not touched. Three git calls: `add -A`,
/// `rm --cached` (drops stale tracked `factory/` and `.bf/` entries), `write-tree`.
pub fn content_hash(top: &Path) -> R<String> {
    let gitdir = git_dir_of(top)
        .ok_or_else(|| err(format!("{} is not a git working copy", top.display())))?;
    let tmp = gitdir.join(format!("bf-index-{}", std::process::id()));
    let real = gitdir.join("index");
    if real.is_file() {
        fs::copy(&real, &tmp)?;
    } else {
        let _ = fs::remove_file(&tmp);
    }
    let tmp_s = tmp.to_string_lossy().to_string();
    let envs = [("GIT_INDEX_FILE", tmp_s.as_str())];
    let result = (|| -> R<String> {
        let o = run(top, &["add", "-A", "--", ".", ":(exclude)factory"], &envs)?;
        if !o.ok {
            return Err(err(format!("git add failed: {}", o.stderr)));
        }
        let o = run(
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
            &envs,
        )?;
        if !o.ok {
            return Err(err(format!("git rm --cached failed: {}", o.stderr)));
        }
        let o = run(top, &["write-tree"], &envs)?;
        if !o.ok {
            return Err(err(format!("git write-tree failed: {}", o.stderr)));
        }
        Ok(o.stdout)
    })();
    let _ = fs::remove_file(&tmp);
    result
}

pub fn has_uncommitted(dir: &Path) -> bool {
    run(dir, &["status", "--porcelain"], &[])
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}
