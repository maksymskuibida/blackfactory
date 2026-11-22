//! A checkpoint fix's files (D-115, D-116). A fix's file list is a recommendation: its builder's request for
//! files and the scope review of it are gone, and both entry points refuse. The events of the old requests
//! replay in `events.rs`. The work of an aborted fix is saved and applied again here.

use crate::ckfix::{self, Live};
use crate::events::{self, CkFixOpen, State};
use crate::gitx;
use crate::plan::Plan;
use crate::refusal::{refuse_owned, R};
use crate::store::{s, Project};
use crate::strings::t;
use crate::util;
use serde_json::{json, Value};
use std::fs;

/// The files granted to the open fix of `checkpoint`, in order.
pub fn granted(st: &State, checkpoint: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == checkpoint) {
        for g in &f.grants {
            for p in &g.files {
                if !p.is_empty() && !out.contains(p) {
                    out.push(p.clone());
                }
            }
        }
    }
    out
}

/// Whether `path` is already in the fix's list: a file of the list, or a new file of a test folder.
pub fn in_list(p: &Project, k: &str, live: &Live, path: &str) -> bool {
    let lower = path.to_lowercase();
    live.allowed
        .iter()
        .any(|g| util::path_matches(&g.to_lowercase(), &lower))
        || (live.test_dirs.iter().any(|g| util::path_matches(g, path))
            && !p.work_dir().join(k).join(path).exists())
}

/// The files a gap note names: words that read as a path (a folder part, or a name with an extension).
/// A `:<line>` or `:<line>:<column>` suffix is a place in the file, not part of its path.
fn strip_line(w: &str) -> &str {
    let mut w = w;
    while let Some((head, tail)) = w.rsplit_once(':') {
        if !head.is_empty() && !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            w = head;
        } else {
            break;
        }
    }
    w
}

pub fn named_files(note: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in note.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let w = w.trim_matches(|c: char| "`'\"()[]:".contains(c));
        let w = w.trim_end_matches('.');
        let w = strip_line(w);
        if w.is_empty() || w.starts_with("http") {
            continue;
        }
        let ext_ok = w.rsplit_once('.').is_some_and(|(stem, e)| {
            !stem.is_empty()
                && !e.is_empty()
                && e.len() <= 8
                && e.chars().all(|c| c.is_ascii_alphanumeric())
                && !e.chars().all(|c| c.is_ascii_digit())
        });
        if !(w.contains('/') || ext_ok) {
            continue;
        }
        let path = crate::scope::normalize_path(w).unwrap_or_else(|| w.to_string());
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// A fix builder's `--need-files` (`bf report <K> --role builder --result blocked --need-files -`) is
/// refused: a fix's file list is a recommendation, so the builder changes the file and says why in its report.
pub fn request(
    p: &Project,
    k: &str,
    role: Option<&str>,
    token: Option<&str>,
    note: &str,
    list: &Value,
) -> R<Value> {
    let _ = (p, role, token, note, list);
    Err(refuse_owned(
        t("ckfix_recommended.request.what", &[("k", k)]),
        t("ckfix_recommended.request.because", &[]),
        vec![
            t("ckfix_recommended.request.step_change", &[]),
            t("ckfix_recommended.request.step_report", &[("k", k)]),
        ],
        t("ckfix_recommended.request.check", &[]),
    ))
}

/// A scope verdict (`bf checkpoint scope <K> --verdict -`) is refused: no fix has a request to judge.
pub fn verdict(p: &Project, k: &str, verdict: &Value, token: Option<&str>) -> R<Value> {
    let _ = (p, verdict, token);
    Err(refuse_owned(
        t("ckfix_recommended.verdict.what", &[("k", k)]),
        t("ckfix_recommended.verdict.because", &[]),
        vec![t("ckfix_recommended.verdict.step", &[("k", k)])],
        t("ckfix_recommended.verdict.check", &[]),
    ))
}

/// A removed line of a granted existing test or configuration file that the grant's patch does not
/// remove is refused, naming the file and the line.
pub fn landing(p: &Project, pl: &Plan, f: &CkFixOpen) -> R<()> {
    let k = &f.checkpoint;
    for g in &f.grants {
        for (i, path) in g.files.iter().enumerate() {
            let test = path
                .to_lowercase()
                .split('/')
                .any(|c| c.contains("test") || c == "spec" || c == "specs");
            if !(test || crate::scope::is_config(path)) {
                continue;
            }
            let Some((_, wt, rel, before)) = ckfix::locate(p, pl, f, path)? else {
                continue;
            };
            let existed = gitx::run(&wt, &["cat-file", "-e", &format!("{before}:{rel}")], &[])
                .map(|o| o.ok)
                .unwrap_or(false);
            if !existed {
                continue;
            }
            let mut in_patch: Vec<String> = g
                .patches
                .get(i)
                .map(|x| x.as_str())
                .unwrap_or("")
                .lines()
                .filter(|l| l.starts_with('-') && !l.starts_with("---"))
                .map(|l| l[1..].trim_end().to_string())
                .collect();
            let d = gitx::must(&wt, &["diff", "-U0", &before, "--", &rel]).unwrap_or_default();
            for l in d.lines() {
                if l.starts_with('-') && !l.starts_with("---") {
                    let line = l[1..].trim_end();
                    if let Some(at) = in_patch.iter().position(|x| x == line) {
                        in_patch.remove(at);
                    } else {
                        return Err(refuse_owned(
                            t("fix.scope.landing.what", &[("k", k)]),
                            t(
                                "fix.scope.landing.because",
                                &[("path", path), ("line", line.trim())],
                            ),
                            vec![t("fix.scope.landing.step", &[("path", path)])],
                            t("fix.scope.landing.check", &[]),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Save the working copy's changes before an abort resets it: the diff of each repository against the
/// commit the fix started from, and its new files. Returns the folder (from the project root), or an
/// empty text when there was nothing to save.
pub fn save_work(p: &Project, pl: &Plan, f: &CkFixOpen) -> R<String> {
    let k = &f.checkpoint;
    let base = p.evidence_dir().join("fix-saves").join(k);
    let n = fs::read_dir(&base)
        .map(|d| {
            d.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
        + 1;
    let dir = base.join(n.to_string());
    let mut repos: Vec<Value> = Vec::new();
    for (name, wt) in ckfix::wts(p, pl, k)? {
        let Some((_, before)) = f.before.iter().find(|(r, _)| *r == name) else {
            continue;
        };
        let diff = gitx::must(
            &wt,
            &[
                "diff",
                "--no-color",
                "--no-renames",
                "--no-ext-diff",
                before,
            ],
        )?;
        let others = gitx::must(&wt, &["ls-files", "--others", "--exclude-standard"])?;
        let news: Vec<String> = others
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if diff.trim().is_empty() && news.is_empty() {
            continue;
        }
        fs::create_dir_all(&dir)?;
        let mut diff_name = Value::Null;
        if !diff.trim().is_empty() {
            let file = format!("{name}.diff");
            util::atomic_write(&dir.join(&file), format!("{diff}\n").as_bytes())?;
            diff_name = json!(file);
        }
        for rel in &news {
            let to = dir.join("files").join(&name).join(rel);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(wt.join(rel), &to)?;
        }
        repos.push(json!({"repo": name, "diff": diff_name, "new_files": news}));
    }
    if repos.is_empty() {
        return Ok(String::new());
    }
    util::atomic_write(
        &dir.join("saved.json"),
        json!({"checkpoint": k, "repos": repos})
            .to_string()
            .as_bytes(),
    )?;
    Ok(format!("factory/evidence/fix-saves/{k}/{n}"))
}

/// The folder of saved work the next fix of `k` starts from: the latest abort's, when it saved some and
/// no fix started from it yet.
pub fn pending_saved(p: &Project, st: &State, k: &str) -> Option<String> {
    let last = st.ck_fix_aborts.iter().rev().find(|a| a.checkpoint == k)?;
    if last.saved.is_empty() {
        return None;
    }
    let used = events::load_log(p)
        .ok()?
        .events
        .iter()
        .any(|e| s(e, "kind") == "checkpoint.fix.open" && s(e, "from_saved") == last.saved);
    (!used).then(|| last.saved.clone())
}

/// Apply the saved work to the checkpoint's working copy, file by file. Returns the files applied and a
/// sentence for each file that no longer applies.
pub fn apply_saved(
    p: &Project,
    pl: &Plan,
    k: &str,
    saved: &str,
    live: &Live,
) -> R<(Vec<String>, Vec<String>)> {
    let dir = p.root.join(saved);
    let meta: Value = fs::read_to_string(dir.join("saved.json"))
        .ok()
        .and_then(|x| serde_json::from_str(&x).ok())
        .unwrap_or_default();
    let several = ckfix::several_repos(p, pl, k);
    let mut applied: Vec<String> = Vec::new();
    let mut not: Vec<String> = Vec::new();
    let tmp = dir.join("apply.tmp.patch");
    for r in meta["repos"].as_array().into_iter().flatten() {
        let name = s(r, "repo");
        let Some((_, wt)) = ckfix::wts(p, pl, k)?.into_iter().find(|(n, _)| n == name) else {
            continue;
        };
        let full = |rel: &str| {
            if several {
                format!("{name}/{rel}")
            } else {
                rel.to_string()
            }
        };
        if let Some(file) = r["diff"].as_str() {
            let text = fs::read_to_string(dir.join(file)).unwrap_or_default();
            let mut chunks: Vec<String> = Vec::new();
            for l in text.lines() {
                if l.starts_with("diff --git ") || chunks.is_empty() {
                    chunks.push(String::new());
                }
                if let Some(c) = chunks.last_mut() {
                    c.push_str(l);
                    c.push('\n');
                }
            }
            for c in chunks.iter().filter(|c| c.starts_with("diff --git ")) {
                let head = c.lines().next().unwrap_or("");
                let rel = head
                    .rsplit_once(" b/")
                    .map(|(_, b)| b)
                    .unwrap_or("")
                    .to_string();
                if rel.is_empty() {
                    continue;
                }
                let path = full(&rel);
                let status = if c.contains("\nnew file mode") {
                    'A'
                } else {
                    'M'
                };
                if !ckfix::within(live, &path, status) {
                    not.push(t("fix.scope.saved.outside", &[("path", &path)]));
                    continue;
                }
                util::atomic_write(&tmp, c.as_bytes())?;
                let ok = gitx::run(
                    &wt,
                    &["apply", "--whitespace=nowarn", &tmp.to_string_lossy()],
                    &[],
                )
                .map(|o| o.ok)
                .unwrap_or(false);
                if ok {
                    applied.push(path);
                } else {
                    not.push(t("fix.scope.saved.not_apply", &[("path", &path)]));
                }
            }
        }
        for rel in r["new_files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
        {
            let path = full(rel);
            if !ckfix::within(live, &path, 'A') {
                not.push(t("fix.scope.saved.outside", &[("path", &path)]));
            } else if wt.join(rel).exists() {
                not.push(t("fix.scope.saved.exists", &[("path", &path)]));
            } else {
                let to = wt.join(rel);
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(dir.join("files").join(name).join(rel), &to)?;
                applied.push(path);
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    Ok((applied, not))
}

/// What the reopened fix did with the saved work, kept beside it: read by the builder's brief.
pub fn reopened(p: &Project, saved: &str) -> Value {
    fs::read_to_string(p.root.join(saved).join("reopened.json"))
        .ok()
        .and_then(|x| serde_json::from_str(&x).ok())
        .unwrap_or_default()
}

/// Keep what the reopened fix did with the saved work.
pub fn note_reopened(p: &Project, saved: &str, applied: &[String], not: &[String]) -> R<()> {
    util::atomic_write(
        &p.root.join(saved).join("reopened.json"),
        json!({"applied": applied, "not_applied": not})
            .to_string()
            .as_bytes(),
    )?;
    Ok(())
}
