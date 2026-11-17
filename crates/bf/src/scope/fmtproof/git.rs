//! Raw git reads at the landing diff's base.

use std::path::Path;
use std::process::{Command, Stdio};

/// Raw git output: the shared git helper trims and decodes text, which a byte compare cannot use.
pub(super) fn git(prog: &str, dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut c = Command::new(prog);
    c.arg("-C").arg(dir).args(args).stdin(Stdio::null());
    for (k, _) in std::env::vars_os() {
        if k.to_str().is_some_and(|k| k.starts_with("GIT_")) {
            c.env_remove(k);
        }
    }
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    c.output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
}

pub(super) fn at(g: &str, repo: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    git(g, repo, &["cat-file", "blob", &format!("{rev}:{path}")])
}

pub(super) fn blob_id(g: &str, repo: &Path, rev: &str, path: &str) -> Option<String> {
    let o = git(
        g,
        repo,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{rev}:{path}^{{blob}}"),
        ],
    )?;
    Some(String::from_utf8_lossy(&o).trim().to_string())
}

/// The merge base of the checkpoint branch and the unit's branch, as the landing diff takes it.
pub(super) fn merge_base(g: &str, repo: &Path, a: &str, b: &str) -> Option<String> {
    let o = git(g, repo, &["merge-base", a, b])?;
    Some(String::from_utf8_lossy(&o).trim().to_string()).filter(|x| !x.is_empty())
}

/// The nearest file with one of the names in a folder above `path` at the base: (its path, content).
pub(super) fn nearest(
    g: &str,
    repo: &Path,
    base: &str,
    path: &str,
    names: &[&str],
) -> Option<(String, Vec<u8>)> {
    let mut dir = path;
    loop {
        dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
        for n in names {
            let f = if dir.is_empty() {
                n.to_string()
            } else {
                format!("{dir}/{n}")
            };
            if let Some(b) = at(g, repo, base, &f) {
                return Some((f, b));
            }
        }
        if dir.is_empty() {
            return None;
        }
    }
}

/// The edition as cargo passes it, read at the base: the package's, the workspace's when the package
/// says `edition.workspace = true`, else 2015.
pub(super) fn edition(g: &str, repo: &Path, base: &str, path: &str) -> String {
    let parse = |b: &[u8]| {
        String::from_utf8_lossy(b)
            .parse::<toml::Table>()
            .unwrap_or_default()
    };
    let Some((cp, b)) = nearest(g, repo, base, path, &["Cargo.toml"]) else {
        return "2015".into();
    };
    let t = parse(&b);
    let ed = t.get("package").and_then(|p| p.get("edition"));
    if let Some(e) = ed.and_then(|e| e.as_str()) {
        return e.to_string();
    }
    // Only a package that says `edition.workspace = true` looks further. The search for the workspace's
    // Cargo.toml starts in the folder above the one just read and ends at the repository root, so it
    // always moves up.
    // A package at the workspace root inherits from its own file, so that file is read first.
    if ed.is_some() {
        if let Some(e) = t
            .get("workspace")
            .and_then(|w| w.get("package"))
            .and_then(|p| p.get("edition"))
        {
            return e.as_str().unwrap_or("2015").to_string();
        }
    }
    let mut cur = ed.map(|_| cp);
    while let Some(f) = cur {
        // `nearest` looks in the folder of the path it is given, so the folder of `f` makes it start
        // one folder higher. A Cargo.toml at the root has nothing above it.
        let Some((folder, _)) = f.rsplit_once('/') else {
            break;
        };
        let Some((wp, wb)) = nearest(g, repo, base, folder, &["Cargo.toml"]) else {
            break;
        };
        if let Some(e) = parse(&wb)
            .get("workspace")
            .and_then(|w| w.get("package"))
            .and_then(|p| p.get("edition"))
        {
            return e.as_str().unwrap_or("2015").to_string();
        }
        cur = Some(wp);
    }
    "2015".into()
}
