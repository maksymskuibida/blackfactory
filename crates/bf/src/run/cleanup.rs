//! The clean-up of working copies (D-109, D-113): a landed unit's copy and a closed checkpoint's copies are
//! removed with their build folders, and a unit sent back to work gets its copy again from its branch.
//! No event is written: the log says what may be removed, so a removal a crash cut off is finished by the
//! next start, land, close or `bf clean`.

use super::*;
use crate::strings::t;

pub(crate) mod disk;
pub(crate) mod sweep;

/// What `remove_unit_copy` did.
pub enum Removal {
    /// The working copy is gone (it may have been gone already): its path.
    Removed(PathBuf),
    /// The working copy stays: why.
    Kept(String),
}

/// Take one working copy out of its repository and remove what is left of the folder. A folder that is
/// already gone is no error.
fn remove_copy(repo: &Path, path: &Path) {
    if path.join(".git").exists() {
        let at = path.to_string_lossy();
        let _ = gitx::run(repo, &["worktree", "remove", "--force", &at], &[]);
    }
    let _ = gitx::run(repo, &["worktree", "prune"], &[]);
    if path.exists() {
        let _ = fs::remove_dir_all(path);
        let _ = gitx::run(repo, &["worktree", "prune"], &[]);
    }
}

/// Why a landed unit's working copy must stay, or `None` when it may go: the unit is landed, its branch is
/// merged into the checkpoint branch and the copy holds nothing uncommitted.
pub fn unit_copy_kept(p: &Project, unit: &str) -> R<Option<String>> {
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let state = st.unit_state(unit).to_string();
    let path = p.work_dir().join(unit);
    if state != "landed" {
        return Ok(Some(t(
            "cleanup.kept.state",
            &[("unit", unit), ("state", &state)],
        )));
    }
    let Some(u) = pl.unit(unit) else {
        return Ok(Some(t("cleanup.kept.plan", &[("unit", unit)])));
    };
    let repo = p.repo_of_unit(u)?.clone();
    let ub = p.unit_branch(unit);
    let cpb = p.checkpoint_branch(s(u, "checkpoint"));
    if !path.exists() {
        return Ok(None);
    }
    if !gitx::branch_exists(&repo.path, &ub) || !gitx::is_ancestor(&repo.path, &ub, &cpb) {
        return Ok(Some(t(
            "cleanup.kept.unmerged",
            &[("unit", unit), ("branch", &ub), ("checkpoint", &cpb)],
        )));
    }
    if path.join(".git").exists() && gitx::has_uncommitted(&path) {
        return Ok(Some(t(
            "cleanup.kept.uncommitted",
            &[("unit", unit), ("path", &path.to_string_lossy())],
        )));
    }
    Ok(None)
}

/// Remove a landed unit's working copy when its branch is merged into the checkpoint branch and the copy
/// holds nothing uncommitted. The unit's branch stays.
pub fn remove_unit_copy(p: &Project, unit: &str) -> R<Removal> {
    if let Some(why) = unit_copy_kept(p, unit)? {
        return Ok(Removal::Kept(why));
    }
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Ok(Removal::Kept(t("cleanup.kept.plan", &[("unit", unit)])));
    };
    let repo = p.repo_of_unit(u)?.clone();
    let path = p.work_dir().join(unit);
    if !path.exists() {
        let _ = gitx::run(&repo.path, &["worktree", "prune"], &[]);
        return Ok(Removal::Removed(path));
    }
    remove_copy(&repo.path, &path);
    Ok(Removal::Removed(path))
}

/// The folders a checkpoint owns under `.bf/work`: its own, its review copy and every wave copy, as far
/// as they exist.
pub fn checkpoint_roots(p: &Project, k: &str) -> Vec<PathBuf> {
    let work = p.work_dir();
    let mut roots = vec![work.join(k), work.join(format!("{k}-review"))];
    let wave = format!("{k}-wave-");
    if let Ok(rd) = fs::read_dir(&work) {
        let mut waves: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.strip_prefix(&wave)
                    .map(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
                    .unwrap_or(false)
            })
            .map(|e| e.path())
            .collect();
        waves.sort();
        roots.extend(waves);
    }
    roots.retain(|r| r.exists());
    roots
}

/// Remove the working copies of a closed checkpoint: its own (one for each repository), its review copy
/// and every wave copy.
pub fn remove_checkpoint_copies(p: &Project, pl: &Plan, k: &str) -> R<()> {
    let st = events::replay(p)?;
    if !st.closed.contains_key(k) {
        return Ok(());
    }
    let repos: Vec<Repo> = checkpoint_repos(p, pl, k)?.into_iter().cloned().collect();
    let roots = checkpoint_roots(p, k);
    for root in roots {
        if !root.exists() {
            continue;
        }
        for r in &repos {
            remove_copy(&r.path, &root);
            remove_copy(&r.path, &root.join(&r.name));
        }
        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
    }
    for r in &repos {
        let _ = gitx::run(&r.path, &["worktree", "prune"], &[]);
    }
    Ok(())
}

/// A unit's working copy from its branch: made again when the folder is missing, then brought up to the
/// checkpoint branch as a start does. Returns how far the branch is still behind. A unit with no branch
/// has nothing to make the copy from: nothing happens.
pub fn ensure_unit_copy(p: &Project, unit: &str) -> R<u64> {
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Ok(0);
    };
    let repo = p.repo_of_unit(u)?.clone();
    let ub = p.unit_branch(unit);
    if !gitx::branch_exists(&repo.path, &ub) {
        return Ok(0);
    }
    let path = p.work_dir().join(unit);
    if !path.join(".git").exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        gitx::must(
            &repo.path,
            &["worktree", "add", &path.to_string_lossy(), &ub],
        )?;
    }
    crash_point("unit-start-after-git");
    // A unit whose branch already exists may have fallen behind: a clean one with no commits of its own
    // is brought up to the checkpoint branch (cause start); anything else is left as it is.
    let st = events::replay(p)?;
    Ok(unit_sync::auto_ff(p, &st, unit, "start"))
}
