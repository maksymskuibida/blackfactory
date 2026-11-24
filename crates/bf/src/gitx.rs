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

/// What git runs with, from the policy (captured at `bf init`): the program, the PATH to give it, and
/// what its sandbox profile is built from.
#[derive(Clone, Default)]
struct GitCfg {
    git: String,
    path: String,
    /// The policy asks for the sandbox.
    sandbox: bool,
    /// The state folder with symlinks resolved.
    state_dir: String,
    /// The project root.
    root: String,
}

static GIT_CFG: std::sync::Mutex<Option<GitCfg>> = std::sync::Mutex::new(None);

/// The git program, the PATH to give it, and what the sandbox profile is built from: whether the
/// policy asks for the sandbox, the resolved state folder and the root. Whether the sandbox works is
/// asked only at the first git call that runs, never here.
pub fn configure(git: &str, path: &str, sandbox: bool, state_dir: &str, root: &str) {
    *GIT_CFG.lock().unwrap() = Some(GitCfg {
        git: git.to_string(),
        path: path.to_string(),
        sandbox,
        state_dir: state_dir.to_string(),
        root: root.to_string(),
    });
}

/// The profile a git call runs under, if any. Asks `sandbox::available()` (once per process) only
/// when the policy asks for the sandbox.
fn profile_of(cfg: &GitCfg) -> Option<String> {
    if cfg.sandbox && crate::sandbox::available() {
        Some(crate::sandbox::git_profile(&cfg.state_dir, &cfg.root))
    } else {
        None
    }
}

pub fn run(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> R<Out> {
    run_with(dir, args, envs, false)
}

/// A git call that may reach the network. Only the sync's update of the main branch uses it.
pub fn run_net(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> R<Out> {
    run_with(dir, args, envs, true)
}

fn run_with(dir: &Path, args: &[&str], envs: &[(&str, &str)], net: bool) -> R<Out> {
    if let Some(o) = memo::answer(dir, args, envs) {
        return Ok(o);
    }
    let o = run_git(dir, args, envs, net)?;
    memo::learn(dir, args, envs, &o);
    Ok(o)
}

fn run_git(dir: &Path, args: &[&str], envs: &[(&str, &str)], net: bool) -> R<Out> {
    let cfg = GIT_CFG.lock().unwrap().clone().unwrap_or_default();
    let profile = profile_of(&cfg);
    let (prog, path) = (cfg.git, cfg.path);
    let profile = if net {
        profile.map(|p| crate::sandbox::allow_network(&p))
    } else {
        profile
    };
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

/// `must` for a write whose reach the caller names: the branches it moves, and whether it may change
/// the files of the working copy `dir` (or its index). The landing's memo forgets exactly that; a write
/// that goes through `must` or `run` instead makes the memo forget everything it holds.
pub fn must_moving(dir: &Path, args: &[&str], moves: &[&str], files: bool) -> R<String> {
    memo::declared(moves, files, || must(dir, args))
}

/// The landing's memo (U-118, D-140, D-143). It lives for one call of `bf unit land`, from
/// `memo::begin` until the scope it returns is dropped, and holds what git answered while the engine
/// has written nothing that could change the answer:
///
/// - a content hash by working copy, read again only through `workdir_hash_memo`, which no check run
///   and no comparison with a review uses (TH-13): every other hash is taken anew and only stored;
/// - the branches and tags of a repository (`for-each-ref`), which also answer `show-ref` of a branch;
/// - `rev-list --count` of a range.
///
/// Each engine write clears it: a write through `must_moving` clears the hash of its working copy (when
/// it may change files) and the branches it names; any other git call that is not a read clears every
/// hash, every branch read and every count. Outside a landing the memo is off and every call runs.
pub mod memo {
    use super::Out;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// The format `gitx::heads` reads the branches and tags with.
    pub(super) const HEADS_FORMAT: &str = "--format=%(objectname) %(refname)";

    #[derive(Default)]
    struct Memo {
        /// Content hashes by working copy folder and whether `factory/` is left out.
        hashes: HashMap<(PathBuf, bool), String>,
        /// The `for-each-ref` output by repository folder, with the branches moved since it was read.
        refs: HashMap<PathBuf, (String, HashSet<String>)>,
        /// `rev-list --count` answers by folder and range.
        counts: HashMap<(PathBuf, String), String>,
    }

    static MEMO: Mutex<Option<Memo>> = Mutex::new(None);

    thread_local! {
        /// The reach of the write `must_moving` runs now: the branches it moves and whether it may
        /// change files.
        static DECLARED: RefCell<Option<(Vec<String>, bool)>> = const { RefCell::new(None) };
    }

    /// While this lives, the memo is on; dropping it turns the memo off and forgets everything.
    pub struct Scope(());

    impl Drop for Scope {
        fn drop(&mut self) {
            *MEMO.lock().unwrap() = None;
        }
    }

    /// Turn the memo on for one landing. Test seam: `BF_TEST_NO_LAND_MEMO` keeps it off.
    pub fn begin() -> Scope {
        #[cfg(feature = "test_seam")]
        if std::env::var_os("BF_TEST_NO_LAND_MEMO").is_some() {
            return Scope(());
        }
        *MEMO.lock().unwrap() = Some(Memo::default());
        Scope(())
    }

    /// A range between two full commit ids: its count can never change, whoever writes.
    fn fixed(range: &str) -> bool {
        let hex = |s: &str| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit());
        range
            .split_once("..")
            .is_some_and(|(a, b)| hex(a) && hex(b))
    }

    /// The engine lock is let go or taken again: another process may have moved a branch or written a
    /// working copy meanwhile. Every branch read and count goes (a count between two commit ids stays),
    /// and every hash but those of `keep`, the checkpoint's copy that the landing turn's own lock keeps
    /// to this process; a check run in it is judged by hashes taken anew, which are stored here.
    pub fn unlocked(keep: &Path) {
        if let Some(m) = MEMO.lock().unwrap().as_mut() {
            m.refs.clear();
            m.counts.retain(|(_, r), _| fixed(r));
            m.hashes.retain(|(d, _), _| d.starts_with(keep));
        }
    }

    pub(super) fn declared<T>(moves: &[&str], files: bool, f: impl FnOnce() -> T) -> T {
        let moves: Vec<String> = moves.iter().map(|s| s.to_string()).collect();
        let old = DECLARED.with(|d| d.replace(Some((moves, files))));
        let r = f();
        DECLARED.with(|d| *d.borrow_mut() = old);
        r
    }

    /// A git call that writes no ref, no file of a working copy and no real index. A call on a
    /// temporary index (the content hash) is one.
    fn is_read(args: &[&str], envs: &[(&str, &str)]) -> bool {
        if envs.iter().any(|(k, _)| *k == "GIT_INDEX_FILE") {
            return true;
        }
        matches!(
            args.first().copied(),
            Some(
                "for-each-ref"
                    | "rev-list"
                    | "rev-parse"
                    | "show-ref"
                    | "merge-base"
                    | "symbolic-ref"
                    | "status"
                    | "log"
                    | "diff"
                    | "ls-files"
                    | "ls-tree"
                    | "cat-file"
                    | "show"
                    | "merge-tree"
            )
        )
    }

    /// The answer the memo holds for this call, if it may be used.
    pub(super) fn answer(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Option<Out> {
        if !envs.is_empty() {
            return None;
        }
        let g = MEMO.lock().unwrap();
        let m = g.as_ref()?;
        let out = |ok: bool, stdout: &str| Out {
            ok,
            stdout: stdout.to_string(),
            stderr: String::new(),
        };
        match args {
            ["for-each-ref", f, "refs/heads/", "refs/tags/"] if *f == HEADS_FORMAT => {
                let (raw, moved) = m.refs.get(dir)?;
                moved.is_empty().then(|| out(true, raw))
            }
            ["show-ref", "--verify", "--quiet", r] => {
                let name = r.strip_prefix("refs/heads/")?;
                let (raw, moved) = m.refs.get(dir)?;
                if moved.contains(name) {
                    return None;
                }
                let has = raw
                    .lines()
                    .any(|l| l.split_once(' ').is_some_and(|(_, x)| x == *r));
                Some(out(has, ""))
            }
            ["rev-list", "--count", range] => m
                .counts
                .get(&(dir.to_path_buf(), range.to_string()))
                .map(|c| out(true, c)),
            _ => None,
        }
    }

    fn forget_hashes_of(m: &mut Memo, dir: &Path) {
        m.hashes
            .retain(|(d, _), _| !(d.starts_with(dir) || dir.starts_with(d)));
    }

    /// Keep what a read answered; forget what a write may have changed.
    pub(super) fn learn(dir: &Path, args: &[&str], envs: &[(&str, &str)], o: &Out) {
        let mut g = MEMO.lock().unwrap();
        let Some(m) = g.as_mut() else {
            return;
        };
        if is_read(args, envs) {
            if !o.ok || !envs.is_empty() {
                return;
            }
            match args {
                ["for-each-ref", f, "refs/heads/", "refs/tags/"] if *f == HEADS_FORMAT => {
                    m.refs
                        .insert(dir.to_path_buf(), (o.stdout.clone(), HashSet::new()));
                }
                ["rev-list", "--count", range] => {
                    m.counts
                        .insert((dir.to_path_buf(), range.to_string()), o.stdout.clone());
                }
                _ => {}
            }
            return;
        }
        match DECLARED.with(|d| d.borrow().clone()) {
            Some((moves, files)) => {
                for (_, moved) in m.refs.values_mut() {
                    moved.extend(moves.iter().cloned());
                }
                if !moves.is_empty() {
                    m.counts.retain(|(_, r), _| fixed(r));
                }
                if files {
                    forget_hashes_of(m, dir);
                }
            }
            None => {
                m.refs.clear();
                m.counts.retain(|(_, r), _| fixed(r));
                m.hashes.clear();
            }
        }
    }

    pub(super) fn store_hash(dir: &Path, exclude_factory: bool, h: &str) {
        if let Some(m) = MEMO.lock().unwrap().as_mut() {
            m.hashes
                .insert((dir.to_path_buf(), exclude_factory), h.to_string());
        }
    }

    pub(super) fn hash(dir: &Path, exclude_factory: bool) -> Option<String> {
        MEMO.lock()
            .unwrap()
            .as_ref()?
            .hashes
            .get(&(dir.to_path_buf(), exclude_factory))
            .cloned()
    }
}

/// The local branches of a repository with their commits, and its tags with their ids, from one call.
#[derive(Default, Clone, Debug)]
pub struct Heads {
    pub branches: std::collections::HashMap<String, String>,
    pub tags: std::collections::HashMap<String, String>,
}

impl Heads {
    /// What `branch_exists` answers.
    pub fn has(&self, branch: &str) -> bool {
        self.branches.contains_key(branch)
    }

    /// What `rev-parse <name>` gives for a branch: a tag of the same name is taken first.
    pub fn tip(&self, branch: &str) -> Option<String> {
        self.tags
            .get(branch)
            .or_else(|| self.branches.get(branch))
            .cloned()
    }
}

/// Every local branch and tag of the repository, from one git call. Empty when git cannot read it.
pub fn heads(dir: &Path) -> Heads {
    let mut h = Heads::default();
    let Ok(o) = must(
        dir,
        &[
            "for-each-ref",
            memo::HEADS_FORMAT,
            "refs/heads/",
            "refs/tags/",
        ],
    ) else {
        return h;
    };
    for l in o.lines() {
        let Some((sha, r)) = l.split_once(' ') else {
            continue;
        };
        if let Some(b) = r.strip_prefix("refs/heads/") {
            h.branches.insert(b.to_string(), sha.to_string());
        } else if let Some(t) = r.strip_prefix("refs/tags/") {
            h.tags.insert(t.to_string(), sha.to_string());
        }
    }
    h
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

/// The files an operating system writes by itself into a folder it shows, by name, at any
/// depth: the file manager's `.DS_Store`, the AppleDouble files (names beginning `._`) and
/// `Thumbs.db`. The one list; they are never content, never an uncommitted change, never landed.
pub const OS_JUNK_GLOBS: [&str; 3] = ["**/.DS_Store", "**/._*", "**/Thumbs.db"];

/// Git pathspecs that leave the junk out. They need no file in the repository and no setting.
pub fn junk_excludes() -> Vec<String> {
    OS_JUNK_GLOBS
        .iter()
        .map(|g| format!(":(exclude,glob){g}"))
        .collect()
}

/// `args`, then `--`, then the given leading pathspecs, then the junk exclusions.
fn with_junk(args: &[&str], lead: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    v.push("--".to_string());
    v.extend(lead.iter().map(|s| s.to_string()));
    v.extend(junk_excludes());
    v
}

fn run_owned(dir: &Path, args: &[String], envs: &[(&str, &str)]) -> R<Out> {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(dir, &a, envs)
}

/// `git add -A` of the whole working copy, leaving the OS junk out (used before the landing commit).
pub fn add_all_without_junk(dir: &Path) -> R<()> {
    // It writes the index of `dir` and moves no branch.
    let o = memo::declared(&[], true, || {
        run_owned(dir, &with_junk(&["add", "-A"], &["."]), &[])
    })?;
    if o.ok {
        Ok(())
    } else {
        Err(err(format!("git add -A failed: {}", o.stderr)))
    }
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
    content_hash_with(top, true)
}

/// The hash of a working copy, or, for a folder that holds one working copy per repository (a
/// checkpoint over several repositories), a hash of their hashes by repository name.
pub fn workdir_hash(dir: &Path, exclude_factory: bool) -> R<String> {
    let h = workdir_hash_fresh(dir, exclude_factory)?;
    memo::store_hash(dir, exclude_factory, &h);
    Ok(h)
}

/// `workdir_hash` from the landing's memo when it holds one for `dir`, else taken anew. Never for the
/// hash right before or after a check run, nor for one compared with what a review judged (TH-13).
pub fn workdir_hash_memo(dir: &Path, exclude_factory: bool) -> R<String> {
    match memo::hash(dir, exclude_factory) {
        Some(h) => Ok(h),
        None => workdir_hash(dir, exclude_factory),
    }
}

fn workdir_hash_fresh(dir: &Path, exclude_factory: bool) -> R<String> {
    if dir.join(".git").exists() {
        return content_hash_with(dir, exclude_factory);
    }
    let mut names: Vec<String> = fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.path().join(".git").exists())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    if names.is_empty() {
        return Err(err(format!("{} holds no git working copy", dir.display())));
    }
    let mut text = String::new();
    for n in names {
        text.push_str(&format!(
            "{n} {}\n",
            content_hash_with(&dir.join(&n), exclude_factory)?
        ));
    }
    Ok(crate::util::sha256_hex(text.as_bytes()))
}

#[cfg(feature = "test_seam")]
use std::time::SystemTime;

fn content_hash_with(top: &Path, exclude_factory: bool) -> R<String> {
    let gitdir = git_dir_of(top)
        .ok_or_else(|| err(format!("{} is not a git working copy", top.display())))?;
    let tmp = gitdir.join(format!("bf-index-{}", std::process::id()));
    let real = gitdir.join("index");
    if real.is_file() {
        fs::copy(&real, &tmp)?;
        // Test seam (C-3): make the copy look newer, as a copy that does not keep times does.
        #[cfg(feature = "test_seam")]
        if std::env::var_os("BF_TEST_INDEX_COPY_FORWARD").is_some() {
            fs::OpenOptions::new()
                .write(true)
                .open(&tmp)?
                .set_modified(SystemTime::now() + std::time::Duration::from_secs(60))?;
        }
        // Git finds racy files by comparing their times with the index file's own modified time.
        // Not every platform keeps that time on copy, so set it from the real index.
        let modified = fs::metadata(&real)?.modified()?;
        fs::OpenOptions::new()
            .write(true)
            .open(&tmp)?
            .set_modified(modified)?;
    } else {
        let _ = fs::remove_file(&tmp);
    }
    let tmp_s = tmp.to_string_lossy().to_string();
    let envs = [("GIT_INDEX_FILE", tmp_s.as_str())];
    let result = (|| -> R<String> {
        let lead: &[&str] = if exclude_factory {
            &[".", ":(exclude)factory"]
        } else {
            &["."]
        };
        let o = run_owned(top, &with_junk(&["add", "-A"], lead), &envs)?;
        if !o.ok {
            return Err(err(format!("git add failed: {}", o.stderr)));
        }
        // Tracked junk stays in the copied index: take it out of the temporary index.
        let mut rm = vec![
            "rm".to_string(),
            "-r".into(),
            "--cached".into(),
            "-q".into(),
            "--ignore-unmatch".into(),
            "--".into(),
        ];
        rm.extend(OS_JUNK_GLOBS.iter().map(|g| format!(":(glob){g}")));
        if exclude_factory {
            rm.push("factory".into());
            rm.push(".bf".into());
        }
        let o = run_owned(top, &rm, &envs)?;
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

/// Whether a working copy holds changes that are not committed, or its state cannot be read (a broken
/// link to the repository, a failed `git status`): what cannot be read is taken as work worth keeping.
pub fn uncommitted_or_unreadable(dir: &Path) -> bool {
    match run_owned(dir, &with_junk(&["status", "--porcelain"], &["."]), &[]) {
        Ok(o) => !o.ok || !o.stdout.is_empty(),
        Err(_) => true,
    }
}

pub fn has_uncommitted(dir: &Path) -> bool {
    run_owned(dir, &with_junk(&["status", "--porcelain"], &["."]), &[])
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}
