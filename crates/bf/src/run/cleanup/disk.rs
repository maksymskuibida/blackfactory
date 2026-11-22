//! The look at the disk a run holds (U-90, D-109): what `.bf/work` holds, what the volume has free, the
//! two warnings, and the clean-up `bf clean` and a low-disk unit start run. The disk only warns; it never
//! refuses a start.

use super::*;
use crate::strings::t;
use std::collections::HashSet;

const GB: u64 = 1024 * 1024 * 1024;

/// The sum of the sizes of the files under a path. A link is counted as the link and never followed.
pub(crate) fn held_at(path: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !md.is_dir() {
        return md.len();
    }
    let Ok(rd) = fs::read_dir(path) else {
        return 0;
    };
    rd.flatten().map(|e| held_at(&e.path())).sum()
}

/// The bytes the working copies of the run hold: the files under `.bf/work`.
pub(crate) fn held(p: &Project) -> u64 {
    held_at(&p.work_dir())
}

/// The free bytes of the volume that holds `.bf/work` (or the nearest folder above it that exists).
pub(crate) fn free(p: &Project) -> u64 {
    let mut at = p.work_dir();
    while !at.exists() {
        match at.parent() {
            Some(up) => at = up.to_path_buf(),
            None => break,
        }
    }
    let Ok(c) = std::ffi::CString::new(at.as_os_str().as_encoded_bytes().to_vec()) else {
        return 0;
    };
    // SAFETY: `c` is a valid C string and `st` is a zeroed struct the call fills.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return 0;
        }
        (st.f_bavail as u64).saturating_mul(st.f_frsize as u64)
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / GB as f64)
}

/// What the engine says of the disk: the total and the free space, and the two warnings.
pub(crate) struct Look {
    pub held: u64,
    pub free: u64,
    /// The first warning: the working copies hold more than `warn_gb`.
    pub warn: Option<String>,
    /// The stronger warning: the volume has less than `low_free_gb` free.
    pub low: Option<String>,
}

pub(crate) fn look(p: &Project) -> Look {
    let (held, free) = (held(p), free(p));
    let pol = &p.policy.disk;
    let warn = (held > pol.warn_gb.saturating_mul(GB))
        .then(|| t("clean.disk.warn", &[("limit", &pol.warn_gb.to_string())]));
    let low =
        is_low(p, free).then(|| t("clean.disk.low", &[("limit", &pol.low_free_gb.to_string())]));
    Look {
        held,
        free,
        warn,
        low,
    }
}

/// Whether the free space is below the policy's `low_free_gb`.
pub(crate) fn is_low(p: &Project, free: u64) -> bool {
    free < p.policy.disk.low_free_gb.saturating_mul(GB)
}

impl Look {
    /// The one line of `bf doctor`: a total only, no path, with the warnings that apply.
    pub fn line(&self) -> String {
        let mut parts = vec![t(
            "clean.disk.line",
            &[("held", &gb(self.held)), ("free", &gb(self.free))],
        )];
        parts.extend(self.warn.clone());
        parts.extend(self.low.clone());
        parts.join(" ")
    }
}

/// What a clean-up did, or with `dry_run` would do.
pub(crate) struct Clean {
    pub dry_run: bool,
    /// Path (relative to the project) and bytes, measured before the removal.
    pub removed: Vec<(String, u64)>,
    /// Path (relative to the project) and the reason it stays.
    pub left: Vec<(String, String)>,
}

impl Clean {
    pub fn freed(&self) -> u64 {
        self.removed.iter().map(|(_, b)| *b).sum()
    }

    pub fn json(&self) -> Value {
        json!({
            "dry_run": self.dry_run,
            "removed": self.removed.iter().map(|(p, b)| json!({"path": p, "bytes": b})).collect::<Vec<_>>(),
            "left": self.left.iter().map(|(p, r)| json!({"path": p, "reason": r})).collect::<Vec<_>>(),
            "freed_bytes": self.freed(),
        })
    }

    pub fn text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        let key = if self.dry_run {
            "clean.line.would"
        } else {
            "clean.line.removed"
        };
        for (path, bytes) in &self.removed {
            lines.push(t(key, &[("path", path), ("bytes", &bytes.to_string())]));
        }
        for (path, reason) in &self.left {
            lines.push(t("clean.line.left", &[("path", path), ("reason", reason)]));
        }
        if self.removed.is_empty() {
            lines.push(t("clean.line.nothing", &[]));
        } else {
            let key = if self.dry_run {
                "clean.line.total_dry"
            } else {
                "clean.line.total"
            };
            lines.push(t(key, &[("bytes", &self.freed().to_string())]));
        }
        lines.join("\n")
    }
}

/// The unit or checkpoint a folder under `.bf/work` belongs to, by its name: `U-3`, `K-2`, `K-2-review`,
/// `K-2-wave-1`.
fn owner_of(name: &str) -> Option<String> {
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if let Some(n) = name.strip_prefix("U-") {
        return digits(n).then(|| name.to_string());
    }
    let rest = name.strip_prefix("K-")?;
    let (n, tail) = match rest.split_once('-') {
        Some((n, tail)) => (n, Some(tail)),
        None => (rest, None),
    };
    if !digits(n) {
        return None;
    }
    let ok = match tail {
        None | Some("review") => true,
        Some(w) => w.strip_prefix("wave-").is_some_and(digits),
    };
    ok.then(|| format!("K-{n}"))
}

fn rel(p: &Project, path: &Path) -> String {
    path.strip_prefix(&p.root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

/// Remove, by the engine's own rules, what is already left under `.bf/work`: the working copy of every
/// landed unit and the copies of every closed checkpoint. A copy with a live token stays, and each thing
/// that stays is listed with its reason. With `dry_run` nothing is removed. The caller holds the engine
/// lock.
pub(crate) fn clean(p: &Project, dry_run: bool) -> R<Clean> {
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let live: HashSet<String> = tokens::read_tokens(p)
        .as_object()
        .map(|o| {
            o.keys()
                .filter_map(|k| k.split(':').next().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut names: Vec<String> = fs::read_dir(p.work_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut out = Clean {
        dry_run,
        removed: Vec::new(),
        left: Vec::new(),
    };
    let mut done: HashSet<String> = HashSet::new();
    for name in names {
        let path = p.work_dir().join(&name);
        let Some(owner) = owner_of(&name) else {
            out.left.push((rel(p, &path), t("clean.left.unknown", &[])));
            continue;
        };
        if live.contains(&owner) {
            out.left
                .push((rel(p, &path), t("clean.left.token", &[("owner", &owner)])));
            continue;
        }
        if owner.starts_with("U-") {
            if let Some(why) = unit_copy_kept(p, &owner)? {
                out.left.push((rel(p, &path), why));
                continue;
            }
            let bytes = held_at(&path);
            if !dry_run {
                remove_unit_copy(p, &owner)?;
            }
            out.removed.push((rel(p, &path), bytes));
            continue;
        }
        if !st.closed.contains_key(&owner) {
            out.left
                .push((rel(p, &path), t("clean.left.open", &[("owner", &owner)])));
            continue;
        }
        if !done.insert(owner.clone()) {
            continue;
        }
        let roots = checkpoint_roots(p, &owner);
        let sizes: Vec<u64> = roots.iter().map(|r| held_at(r)).collect();
        if !dry_run {
            remove_checkpoint_copies(p, &pl, &owner)?;
        }
        for (root, bytes) in roots.iter().zip(sizes) {
            if !dry_run && root.exists() {
                out.left.push((rel(p, root), t("clean.left.stays", &[])));
            } else {
                out.removed.push((rel(p, root), bytes));
            }
        }
    }
    Ok(out)
}
