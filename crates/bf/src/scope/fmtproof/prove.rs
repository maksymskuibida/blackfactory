//! The proof that one file is format only.

use super::exec::exec;
use super::git::{at, blob_id, edition, nearest};
use crate::store::{s, Project};
use crate::util;
use serde_json::{json, Value};
use std::path::Path;

pub(super) struct Ctx<'a> {
    pub(super) p: &'a Project,
    pub(super) g: String,
    pub(super) repo: &'a Path,
    pub(super) wc: &'a Path,
    pub(super) base: String,
    pub(super) fmt: Option<Result<Value, String>>,
}

/// The proof for one file: its entry for the grant, or the reason it is not format only.
pub(super) fn prove(c: &Ctx, path: &str) -> Result<Value, String> {
    let (g, repo, base) = (c.g.as_str(), c.repo, c.base.as_str());
    let (old, now) = (at(g, repo, base, path), std::fs::read(c.wc.join(path)).ok());
    let (old, now) = match (old, now) {
        (Some(o), Some(n)) => (o, n),
        (None, Some(_)) => return Err("new: it does not exist at the base".into()),
        (Some(_), None) => return Err("deleted: it is missing from the working copy".into()),
        _ => return Err("it exists neither at the base nor in the working copy".into()),
    };
    if old == now {
        return Err("unchanged: it equals its content at the base".into());
    }
    if !path.ends_with(".rs") {
        return Err(
            "no formatter for this kind of file (this version knows rustfmt for .rs files)".into(),
        );
    }
    let fmt = c.fmt.clone().unwrap_or(Err("formatter not found".into()))?;
    let ed = edition(g, repo, base, path);
    let cfg = nearest(g, repo, base, path, &[".rustfmt.toml", "rustfmt.toml"]);
    let dir = std::env::temp_dir().join(format!(
        "bf-fmt-{}",
        util::random_hex(8).map_err(|e| e.to_string())?
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Deviation from the brief ("with none, no flag"): --config-path is always passed, with an empty
    // rustfmt.toml when the base has no settings file. Without the flag rustfmt, run in the working
    // copy, would read a rustfmt.toml the builder added there (or one of the user's own), and the
    // result would no longer come from the base's settings. Reported for the design to record.
    let w = std::fs::write(
        dir.join("rustfmt.toml"),
        cfg.as_ref().map_or(&[][..], |c| &c.1[..]),
    );
    let dir_s = dir.to_string_lossy().to_string();
    let out = w.map_err(|e| e.to_string()).and_then(|_| {
        let a = ["--edition", ed.as_str(), "--config-path", dir_s.as_str()];
        exec(s(&fmt, "run"), &c.p.policy.exec.path, &a, c.wc, &old)
    });
    let _ = std::fs::remove_dir_all(&dir);
    if out.map_err(|e| format!("formatter failed: {e}"))? != now {
        return Err(
            "more than formatting: rustfmt on the base content does not give this file".into(),
        );
    }
    let blob = |x: &str| blob_id(g, repo, base, x).unwrap_or_default();
    Ok(
        json!({"path": path, "why": "format only", "new": false, "patch": "", "format": {
        "base": base, "base_blob": blob(path), "result_sha256": util::sha256_hex(&now),
        "formatter": fmt, "edition": ed, "config_blob": cfg.map(|c| blob(&c.0)).unwrap_or_default()}}),
    )
}
