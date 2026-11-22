//! The sweep of working copies and briefs the engine will not read again (D-109, U-89).
//!
//! It walks only `.bf/work` and `.bf/briefs`. Four kinds of thing go: the build output (what git
//! ignores) of a working copy no agent is using, a left-over working copy, and the briefs of work that is
//! over. Nothing is written to the log: the log says what may go, so a sweep a crash cut off is finished by
//! the next one. A sweep never fails; what it cannot remove it leaves for the next.

use super::*;
use crate::events::State;
use crate::strings::t;
use crate::{ckreview, lock};

/// One thing the sweep took away, or left and named.
#[derive(Clone, Debug)]
pub struct Removed {
    /// What it was: `build` (the ignored files of a unit's or checkpoint's working copy), `unit`,
    /// `checkpoint`, `review`, `wave`, `stray` (a folder of a unit the plan does not hold) or `brief`.
    pub kind: &'static str,
    /// The folder or file, inside `.bf/work` or `.bf/briefs`.
    pub path: PathBuf,
    /// The bytes it held, measured before the removal.
    pub bytes: u64,
    /// When the thing stays: why, from the strings. `None` for what was removed.
    pub kept: Option<String>,
}

impl Removed {
    fn gone(kind: &'static str, path: PathBuf, bytes: u64) -> Removed {
        Removed {
            kind,
            path,
            bytes,
            kept: None,
        }
    }
    fn stays(kind: &'static str, path: PathBuf, why: String) -> Removed {
        Removed {
            kind,
            path,
            bytes: 0,
            kept: Some(why),
        }
    }
}

/// The size of a file or folder, never following a link.
fn size_of(path: &Path) -> u64 {
    let Ok(m) = fs::symlink_metadata(path) else {
        return 0;
    };
    if m.is_dir() {
        fs::read_dir(path)
            .map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum())
            .unwrap_or(0)
    } else {
        m.len()
    }
}

/// Whether a process of that number is alive.
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only tests whether the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether a check runs now: a run keeps its output in `.output-<pid>.txt` beside the receipts until it
/// ends, and the file names the process. The file does not name the unit, so one running check holds
/// every build folder.
fn check_runs(p: &Project) -> bool {
    let Ok(rd) = fs::read_dir(p.receipts_dir()) else {
        return false;
    };
    rd.flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_string();
        n.strip_prefix(".output-")
            .and_then(|r| r.strip_suffix(".txt"))
            .and_then(|r| r.parse::<i32>().ok())
            .map(alive)
            .unwrap_or(false)
    })
}

/// Whether a token of `id` (a key `<id>:<role>`) is live.
fn token_live(tokens: &Value, id: &str) -> bool {
    let pre = format!("{id}:");
    tokens
        .as_object()
        .map(|o| o.keys().any(|k| k.starts_with(&pre)))
        .unwrap_or(false)
}

/// Whether `at` (an RFC 3339 time) is at least the idle limit ago.
fn idle(p: &Project, at: &str) -> bool {
    util::now_secs().saturating_sub(util::parse_rfc3339(at)) >= p.policy.limits.idle_minutes * 60
}

/// A name that is one path part: nothing from a plan or a log leads out of the folder.
fn plain(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && name != "." && name != ".."
}

/// The working copies inside a folder: the folder itself, or each repository's folder under it.
fn copies_in(root: &Path) -> Vec<PathBuf> {
    if root.join(".git").exists() {
        return vec![root.to_path_buf()];
    }
    let mut out: Vec<PathBuf> = fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|d| d.join(".git").exists())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// What git ignores in a working copy, removed with `git clean -dfX`: no folder name is written here.
/// Tracked files and files that are not ignored stay. Gives what was removed.
fn clean_build(root: &Path, kind: &'static str) -> Vec<Removed> {
    let mut out = Vec::new();
    for dir in copies_in(root) {
        let Ok(dry) = gitx::run(&dir, &["clean", "-dnX"], &[]) else {
            continue;
        };
        if !dry.ok {
            continue;
        }
        let listed: Vec<PathBuf> = dry
            .stdout
            .lines()
            .filter_map(|l| l.strip_prefix("Would remove "))
            .map(|r| dir.join(r.trim_end_matches('/')))
            .collect();
        if listed.is_empty() {
            continue;
        }
        let bytes: u64 = listed.iter().map(|x| size_of(x)).sum();
        if gitx::run(&dir, &["clean", "-dfX"], &[])
            .map(|o| o.ok)
            .unwrap_or(false)
        {
            out.push(Removed::gone(kind, dir, bytes));
        }
    }
    out
}

/// Take a folder of working copies away, with their `git worktree` entries. `repos` are the repositories
/// that may hold the entries.
fn remove_root(repos: &[PathBuf], root: &Path) {
    for r in repos {
        remove_copy(r, root);
        if !root.join(".git").exists() {
            if let Ok(rd) = fs::read_dir(root) {
                for e in rd.flatten() {
                    if e.path().join(".git").exists() {
                        remove_copy(r, &e.path());
                    }
                }
            }
        }
    }
    if root.exists() {
        let _ = fs::remove_dir_all(root);
    }
    for r in repos {
        let _ = gitx::run(r, &["worktree", "prune"], &[]);
    }
}

/// `U-12`: the shape of a unit's ID, for a folder the plan does not name.
fn unit_shaped(name: &str) -> bool {
    name.strip_prefix("U-")
        .map(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false)
}

/// `K-3-review` gives (`K-3`, `review`) and `K-3-wave-2` gives (`K-3`, `wave-2`).
fn checkpoint_copy(name: &str) -> Option<(String, String)> {
    let (k, rest) = name.strip_prefix("K-").and_then(|r| {
        let digits = r.chars().take_while(|c| c.is_ascii_digit()).count();
        (digits > 0).then(|| (format!("K-{}", &r[..digits]), r[digits..].to_string()))
    })?;
    let what = rest.strip_prefix('-')?;
    let ok = what == "review"
        || what
            .strip_prefix("wave-")
            .map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false);
    ok.then(|| (k, what.to_string()))
}

/// The time of the newest event that belongs to a checkpoint: its own, and those of its units.
fn latest_event(log: &events::Log, pl: &Plan, k: &str) -> String {
    let units: Vec<&str> = pl
        .units
        .iter()
        .filter(|u| s(u, "checkpoint") == k)
        .map(|u| s(u, "id"))
        .collect();
    log.events
        .iter()
        .filter(|e| {
            e["checkpoint"].as_str() == Some(k)
                || e["unit"]
                    .as_str()
                    .map(|u| units.contains(&u))
                    .unwrap_or(false)
        })
        .filter_map(|e| e["at"].as_str())
        .max_by_key(|a| util::parse_rfc3339(a))
        .unwrap_or("")
        .to_string()
}

/// Whether anything of an open checkpoint is open: a fix, a sync, a review round, a wave check or a
/// landing turn.
fn checkpoint_busy(p: &Project, st: &State, k: &str) -> bool {
    st.ck_fix
        .as_ref()
        .map(|f| f.checkpoint == k)
        .unwrap_or(false)
        || st
            .sync_open
            .as_ref()
            .map(|x| x.checkpoint == k)
            .unwrap_or(false)
        || review_open(p, k)
        || st
            .wave_runs
            .values()
            .any(|w| w.checkpoint == k && w.state == "running")
        || lock::turn_open(p, k)
}

fn review_open(p: &Project, k: &str) -> bool {
    match ckreview::rounds(p, k) {
        Ok(rs) => rs.last().map(|r| r.result.is_none()).unwrap_or(false),
        Err(_) => true,
    }
}

/// Sweep. Gives what it removed and what it left and named.
pub fn sweep(p: &Project, st: &State) -> Vec<Removed> {
    let mut out = Vec::new();
    let Ok(pl) = plan::load(p) else {
        return out;
    };
    let log = events::load_log(p).ok();
    let tokens = super::super::tokens::read_tokens(p);
    let checking = check_runs(p);
    let work = p.work_dir();
    let repos: Vec<PathBuf> = p.repos.iter().map(|r| r.path.clone()).collect();

    // (1) The build output of a unit's working copy that nobody is using.
    for u in &pl.units {
        let id = s(u, "id");
        let Some(us) = st.units.get(id) else { continue };
        if !plain(id) || us.state == "landed" || !work.join(id).exists() {
            continue;
        }
        if checking || token_live(&tokens, id) {
            continue;
        }
        let due = match us.state.as_str() {
            "parked" | "blocked" => true,
            _ => idle(p, &us.last_at),
        };
        if due {
            out.extend(clean_build(&work.join(id), "build"));
        }
    }

    // (2) The build output of an open checkpoint's working copy.
    if let Some(log) = &log {
        for c in &pl.checkpoints {
            let k = s(c, "id");
            if !plain(k) || st.closed.contains_key(k) || !work.join(k).exists() {
                continue;
            }
            if checking
                || token_live(&tokens, k)
                || checkpoint_busy(p, st, k)
                || !idle(p, &latest_event(log, &pl, k))
            {
                continue;
            }
            out.extend(clean_build(&work.join(k), "build"));
        }
    }

    // (3) Left-over working copies.
    for u in &pl.units {
        let id = s(u, "id");
        if !plain(id) || st.unit_state(id) != "landed" || !work.join(id).exists() {
            continue;
        }
        let path = work.join(id);
        let bytes = size_of(&path);
        match remove_unit_copy(p, id) {
            Ok(Removal::Removed(_)) => out.push(Removed::gone("unit", path, bytes)),
            Ok(Removal::Kept(why)) => out.push(Removed::stays("unit", path, why)),
            Err(_) => {}
        }
    }
    for c in &pl.checkpoints {
        let k = s(c, "id");
        if !plain(k) || !st.closed.contains_key(k) {
            continue;
        }
        let mut roots = vec![work.join(k), work.join(format!("{k}-review"))];
        if let Ok(rd) = fs::read_dir(&work) {
            let pre = format!("{k}-wave-");
            roots.extend(
                rd.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(&pre))
                    .map(|e| e.path()),
            );
        }
        let held: Vec<(PathBuf, u64)> = roots
            .into_iter()
            .filter(|r| r.exists())
            .map(|r| {
                let b = size_of(&r);
                (r, b)
            })
            .collect();
        if held.is_empty() {
            continue;
        }
        let _ = remove_checkpoint_copies(p, &pl, k);
        for (r, b) in held {
            if !r.exists() {
                out.push(Removed::gone("checkpoint", r, b));
            }
        }
    }
    let mut names: Vec<String> = fs::read_dir(&work)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    for name in names {
        let path = work.join(&name);
        if !plain(&name) || !path.is_dir() {
            continue;
        }
        if let Some((k, what)) = checkpoint_copy(&name) {
            let open = if what == "review" {
                review_open(p, &k)
            } else {
                let n: u32 = what.trim_start_matches("wave-").parse().unwrap_or(0);
                st.wave_runs
                    .get(&(k.clone(), n))
                    .map(|w| w.state == "running")
                    .unwrap_or(false)
            };
            if open || st.closed.contains_key(&k) {
                continue;
            }
            let bytes = size_of(&path);
            remove_root(&repos, &path);
            let kind = if what == "review" { "review" } else { "wave" };
            if !path.exists() {
                out.push(Removed::gone(kind, path, bytes));
            }
            continue;
        }
        let started = st
            .units
            .get(&name)
            .map(|u| !u.state.is_empty())
            .unwrap_or(false);
        let in_plan = pl.unit(&name).is_some();
        let stray = if in_plan {
            !started
        } else {
            unit_shaped(&name)
        };
        if !stray || token_live(&tokens, &name) {
            continue;
        }
        let copies = copies_in(&path);
        let dirty = copies.is_empty() || copies.iter().any(|d| gitx::has_uncommitted(d));
        if dirty {
            out.push(Removed::stays(
                "stray",
                path.clone(),
                t(
                    "cleanup.kept.stray",
                    &[("name", &name), ("path", &path.to_string_lossy())],
                ),
            ));
            continue;
        }
        let bytes = size_of(&path);
        remove_root(&repos, &path);
        if !path.exists() {
            out.push(Removed::gone("stray", path, bytes));
        }
    }

    // (4) Briefs of work that is over.
    let mut over: Vec<String> = st
        .units
        .iter()
        .filter(|(_, u)| u.state == "landed")
        .map(|(id, _)| id.clone())
        .collect();
    over.extend(st.closed.keys().cloned());
    over.extend(
        st.amends
            .iter()
            .filter(|a| matches!(a.status.as_str(), "approved" | "withdrawn"))
            .map(|a| a.id.clone()),
    );
    if let Ok(rd) = fs::read_dir(p.briefs_dir()) {
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !name.ends_with(".md") || !f.is_file() {
                continue;
            }
            if over
                .iter()
                .any(|id| plain(id) && name.starts_with(&format!("{id}-")))
            {
                let bytes = size_of(&f);
                if fs::remove_file(&f).is_ok() {
                    out.push(Removed::gone("brief", f, bytes));
                }
            }
        }
    }
    out
}
