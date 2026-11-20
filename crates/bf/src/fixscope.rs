//! A checkpoint fix's file requests (D-91, H-22, R-70). The builder of a fix asks for files beyond its
//! list (`request`), the scope reviewer grants them for that fix only (`verdict`), and a file whose
//! rounds are spent or whose request is a hard refusal goes to the human as a plan gap. The events and
//! their replay live in `events.rs`. The work of an aborted fix is saved and applied again here.

use crate::ckfix::{self, Live};
use crate::events::{self, CkFixOpen, State};
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse_owned, Fail, R};
use crate::run;
use crate::scope::retry;
use crate::store::{s, Project};
use crate::strings::t;
use crate::util;
use serde_json::{json, Value};
use std::fs;

fn paths(files: &[Value]) -> Vec<String> {
    files.iter().map(|f| s(f, "path").to_string()).collect()
}

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

fn bad(k: &str, file: &str, rule: &str) -> Fail {
    refuse_owned(
        t("fix.scope.request.what", &[("k", k)]),
        t("fix.scope.request.bad", &[("file", file), ("rule", rule)]),
        vec![
            t("fix.scope.request.bad_step_list", &[]),
            t("fix.scope.request.bad_step_why", &[]),
            t("fix.scope.request.bad_step_again", &[("k", k)]),
        ],
        t("fix.scope.request.done_when", &[]),
    )
}

fn parse(p: &Project, k: &str, list: &Value) -> R<Vec<Value>> {
    let arr = list
        .as_array()
        .cloned()
        .or_else(|| list.get("files").and_then(|f| f.as_array().cloned()))
        .ok_or_else(|| {
            bad(
                k,
                &t("fix.scope.label.request", &[]),
                &t("fix.scope.request.not_list", &[]),
            )
        })?;
    if arr.is_empty() {
        return Err(bad(
            k,
            &t("fix.scope.label.request", &[]),
            &t("fix.scope.request.empty", &[]),
        ));
    }
    let top = p.work_dir().join(k);
    let mut out: Vec<Value> = Vec::new();
    for f in &arr {
        let given = s(f, "path").trim().to_string();
        if given.is_empty() {
            return Err(bad(
                k,
                &t("fix.scope.label.entry", &[]),
                &t("fix.scope.request.no_path", &[]),
            ));
        }
        let path = if given.starts_with('/')
            || given.contains('\\')
            || given.chars().any(|c| c.is_control())
        {
            given.clone()
        } else {
            match crate::scope::normalize_path(&given) {
                Some(n) => n,
                None if given.split('/').any(|c| c == "..") => given.clone(),
                None => return Err(bad(k, &given, &t("fix.scope.request.folder", &[]))),
            }
        };
        if path.contains(['*', '?', '[', ']', '{', '}']) {
            return Err(bad(k, &given, &t("fix.scope.request.pattern", &[])));
        }
        if given.ends_with('/') || path == "." || top.join(&path).is_dir() {
            return Err(bad(k, &given, &t("fix.scope.request.folder", &[])));
        }
        if out.iter().any(|x| s(x, "path") == path) {
            return Err(bad(k, &path, &t("fix.scope.request.twice", &[])));
        }
        if s(f, "why").trim().is_empty() {
            return Err(bad(k, &path, &t("fix.scope.request.no_why", &[])));
        }
        let exists = top.join(&path).is_file();
        if s(f, "patch").trim().is_empty() {
            let key = if exists {
                "fix.scope.request.no_patch_exists"
            } else {
                "fix.scope.request.no_patch_new"
            };
            return Err(bad(k, &path, &t(key, &[])));
        }
        out.push(json!({"path": path, "why": s(f, "why").trim(), "patch": s(f, "patch"), "new": !exists}));
    }
    Ok(out)
}

/// A request for a file whose rounds are spent, or whose patch is the one already judged, is refused
/// naming the file. `patch` is None when only the rounds are looked at.
fn retry_refusal(
    p: &Project,
    k: &str,
    f: &CkFixOpen,
    path: &str,
    patch: Option<&str>,
) -> Option<Fail> {
    let lim = retry::limit(p);
    let (because, step) = if retry::spent(&f.refused, path, lim) {
        (
            t(
                "fix.scope.request.spent",
                &[("rounds", &retry::rounds_used(&f.refused, path).to_string())],
            ),
            t("fix.scope.request.spent_step", &[]),
        )
    } else if patch.is_some_and(|pt| {
        retry::latest_is_retry(&f.refused, path)
            && retry::last_patch(&f.refused, path).is_some_and(|l| l.trim() == pt.trim())
    }) {
        (
            t("fix.scope.request.same_patch", &[]),
            t("fix.scope.request.same_patch_step", &[]),
        )
    } else {
        return None;
    };
    Some(refuse_owned(
        t("fix.scope.request.what", &[("k", k)]),
        t(
            "fix.scope.request.bad",
            &[("file", path), ("rule", &because)],
        ),
        vec![step],
        t("fix.scope.request.done_when", &[]),
    ))
}

fn gap(p: &Project, k: &str, missing: &[String], reason: &str) -> R<()> {
    events::append(
        p,
        "checkpoint.fix.gap",
        json!({"checkpoint": k, "missing": missing, "reason": reason}),
    )?;
    ckfix::write_overlay(p)?;
    p.write_guard()?;
    Ok(())
}

/// The builder's request for files beyond the fix's list
/// (`bf report <K> --role builder --result blocked --need-files -`).
pub fn request(
    p: &Project,
    k: &str,
    role: Option<&str>,
    token: Option<&str>,
    note: &str,
    list: &Value,
) -> R<Value> {
    let what = t("fix.scope.request.what", &[("k", k)]);
    let _lock = crate::lock::engine(p, &what)?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let pl = plan::load(p)?;
    let done = t("fix.scope.request.done_when", &[]);
    let Some(f) = st.ck_fix.clone().filter(|f| f.checkpoint == k) else {
        return Err(refuse_owned(
            what,
            t("fix.scope.request.none_open", &[("k", k)]),
            vec![t("fix.scope.request.none_step", &[])],
            done,
        ));
    };
    if role.is_some_and(|r| r != "builder") {
        return Err(refuse_owned(
            what,
            t("fix.scope.request.role", &[]),
            vec![t("fix.scope.request.role_step", &[("k", k)])],
            done,
        ));
    }
    if f.phase != "fix" {
        let asked = list
            .as_array()
            .or_else(|| list.get("files").and_then(|x| x.as_array()))
            .cloned()
            .unwrap_or_default();
        for a in &asked {
            let path = crate::scope::normalize_path(s(a, "path")).unwrap_or_default();
            if let Some(e) = retry_refusal(p, k, &f, &path, None) {
                return Err(e);
            }
        }
        let because = if f.phase == "scope" {
            t("fix.scope.request.with_reviewer", &[])
        } else {
            t("fix.scope.request.not_at_work", &[("phase", &f.phase)])
        };
        let step = if f.phase == "scope" {
            t("fix.scope.request.with_reviewer_step", &[])
        } else {
            t("fix.scope.request.not_at_work_step", &[])
        };
        return Err(refuse_owned(what, because, vec![step], done));
    }
    if p.policy.roles.tokens && !run::holds_token(p, &format!("{k}:builder"), token) {
        return Err(refuse_owned(
            what,
            t("fix.scope.request.token", &[("k", k)]),
            vec![t("fix.scope.request.token_step", &[])],
            t("fix.scope.request.token_check", &[]),
        ));
    }
    let live = ckfix::live(p, &pl, &st, &f);
    let files = parse(p, k, list)?;
    for x in &files {
        let path = s(x, "path");
        if in_list(p, k, &live, path) {
            return Err(bad(k, path, &t("fix.scope.request.in_list", &[])));
        }
    }
    for x in &files {
        if let Some(e) = retry_refusal(p, k, &f, s(x, "path"), Some(s(x, "patch"))) {
            return Err(e);
        }
    }
    let tok = format!("{k}:builder");
    if !p.policy.review.scope_review {
        let missing: Vec<String> = files
            .iter()
            .map(|x| {
                t(
                    "fix.scope.gap.line",
                    &[("path", s(x, "path")), ("why", s(x, "why"))],
                )
            })
            .collect();
        run::spend_token(p, &tok)?;
        gap(p, k, &missing, &t("fix.scope.gap.reason_off", &[]))?;
        return Ok(json!({"checkpoint": k, "phase": "gap", "scope_request": paths(&files)}));
    }
    let hard: Vec<Value> = files
        .iter()
        .filter_map(|x| {
            crate::scope::hard_rule(p, &pl, &st, k, s(x, "path"))
                .map(|r| json!({"path": s(x, "path"), "rule": r}))
        })
        .collect();
    if p.policy.roles.tokens {
        run::spend_token(p, &tok)?;
    }
    events::append(
        p,
        "checkpoint.fix.scope.request",
        json!({"checkpoint": k, "files": files, "note": note}),
    )?;
    if !hard.is_empty() {
        events::append(
            p,
            "checkpoint.fix.scope.refuse",
            json!({"checkpoint": k, "refusal": "hard", "files": hard}),
        )?;
        let missing: Vec<String> = hard
            .iter()
            .map(|h| {
                t(
                    "fix.scope.gap.line",
                    &[("path", s(h, "path")), ("why", s(h, "rule"))],
                )
            })
            .collect();
        gap(p, k, &missing, &t("fix.scope.gap.reason_hard", &[]))?;
        return Ok(json!({"checkpoint": k, "phase": "gap", "refused": hard, "plan_repair": true}));
    }
    ckfix::write_overlay(p)?;
    p.write_guard()?;
    Ok(json!({"checkpoint": k, "phase": "scope", "scope_request": paths(&files)}))
}

/// The scope reviewer's verdict (`bf checkpoint scope <K> --verdict -`).
pub fn verdict(p: &Project, k: &str, verdict: &Value, token: Option<&str>) -> R<Value> {
    let what = t("fix.scope.verdict.what", &[("k", k)]);
    let _lock = crate::lock::engine(p, &what)?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let pl = plan::load(p)?;
    let done = t("fix.scope.verdict.done_when", &[]);
    let Some(f) = st
        .ck_fix
        .clone()
        .filter(|f| f.checkpoint == k && f.phase == "scope")
    else {
        return Err(refuse_owned(
            what,
            t("fix.scope.verdict.none_open", &[]),
            vec![t("fix.scope.verdict.none_step", &[])],
            done,
        ));
    };
    if p.policy.roles.tokens && !run::holds_token(p, &format!("{k}:reviewer"), token) {
        return Err(refuse_owned(
            what,
            t("fix.scope.verdict.token", &[("k", k)]),
            vec![t("fix.scope.verdict.token_step", &[])],
            done,
        ));
    }
    let hint = |why: String| {
        refuse_owned(
            what.clone(),
            why,
            vec![
                t("fix.scope.verdict.hint", &[]),
                t("fix.scope.verdict.bad_step_all", &[]),
            ],
            done.clone(),
        )
    };
    let arr = verdict
        .as_array()
        .cloned()
        .or_else(|| verdict.get("verdicts").and_then(|v| v.as_array().cloned()))
        .ok_or_else(|| hint(t("fix.scope.verdict.not_list", &[])))?;
    let wanted = paths(&f.request);
    let mut seen: Vec<String> = Vec::new();
    for v in &arr {
        let path = s(v, "path");
        if !wanted.iter().any(|w| w == path) {
            return Err(hint(t("fix.scope.verdict.not_asked", &[("path", path)])));
        }
        if seen.iter().any(|x| x == path) {
            return Err(hint(t("fix.scope.verdict.twice", &[("path", path)])));
        }
        if let Err(why) = retry::check_verdict_item(v) {
            return Err(hint(format!("{path}: {why}")));
        }
        if s(v, "why").trim().is_empty() {
            return Err(hint(t("fix.scope.verdict.no_why", &[("path", path)])));
        }
        seen.push(path.to_string());
    }
    if let Some(m) = wanted.iter().find(|w| !seen.contains(w)) {
        return Err(hint(t("fix.scope.verdict.missing", &[("path", m)])));
    }
    if p.policy.roles.tokens {
        run::spend_token(p, &format!("{k}:reviewer"))?;
    }
    let lim = retry::limit(p);
    let mut granted: Vec<Value> = Vec::new();
    let mut refused: Vec<Value> = Vec::new();
    let mut retries: Vec<Value> = Vec::new();
    let mut all: Vec<Value> = Vec::new();
    let mut late = false;
    for v in &arr {
        let path = s(v, "path");
        let asked = f
            .request
            .iter()
            .find(|x| s(x, "path") == path)
            .cloned()
            .unwrap_or_default();
        let round = retry::rounds_used(&f.refused, path) + 1;
        let mut entry =
            json!({"path": path, "why": s(v, "why"), "patch": s(&asked, "patch"), "round": round});
        match s(v, "verdict") {
            "extension" => {
                entry["verdict"] = json!("extension");
                refused.push(entry.clone());
            }
            "retry" => {
                entry["verdict"] = json!(retry::verdict_word(&f.refused, path, lim));
                entry["must"] = json!(s(v, "must").trim());
                retries.push(entry.clone());
            }
            _ => {
                entry["verdict"] = json!("needed");
                if let Some(r) = crate::scope::hard_rule(p, &pl, &st, k, path) {
                    late = true;
                    entry["rule"] = json!(r);
                    refused.push(entry.clone());
                } else {
                    granted.push(json!({"path": path, "why": s(&asked, "why"), "reviewer": s(v, "why"),
                        "patch": s(&asked, "patch"), "new": asked["new"].as_bool().unwrap_or(false)}));
                }
            }
        }
        all.push(entry);
    }
    let hold = !retries.is_empty() && refused.is_empty();
    let mut phase = "fix";
    let mut refusal = "";
    if hold {
        refusal = if retries.iter().any(|x| s(x, "verdict") == "retry_spent") {
            "retry_spent"
        } else {
            "retry"
        };
        events::append(
            p,
            "checkpoint.fix.scope.refuse",
            json!({"checkpoint": k, "refusal": refusal, "files": all}),
        )?;
        if refusal == "retry_spent" {
            let missing: Vec<String> = retries
                .iter()
                .map(|r| {
                    t(
                        "fix.scope.gap.line_must",
                        &[
                            ("path", s(r, "path")),
                            ("why", s(r, "why")),
                            ("must", s(r, "must")),
                        ],
                    )
                })
                .collect();
            gap(p, k, &missing, &t("fix.scope.gap.reason_spent", &[]))?;
            phase = "gap";
        }
    } else if !refused.is_empty() {
        // Nothing is granted: a request is answered once, and the human decides what the reviewer refused.
        let files: Vec<Value> = all
            .iter()
            .map(|e| {
                let mut e = e.clone();
                if s(&e, "verdict") == "retry_spent" {
                    e["verdict"] = json!("retry");
                }
                e
            })
            .collect();
        refusal = if late { "hard_late" } else { "extension" };
        events::append(
            p,
            "checkpoint.fix.scope.refuse",
            json!({"checkpoint": k, "refusal": refusal, "files": files}),
        )?;
        let missing: Vec<String> = refused
            .iter()
            .map(|r| {
                let why = if late && !s(r, "rule").is_empty() {
                    s(r, "rule")
                } else {
                    s(r, "why")
                };
                t(
                    "fix.scope.gap.line",
                    &[("path", s(r, "path")), ("why", why)],
                )
            })
            .collect();
        let reason = t(
            if late {
                "fix.scope.gap.reason_hard"
            } else {
                "fix.scope.gap.reason_extension"
            },
            &[],
        );
        gap(p, k, &missing, &reason)?;
        phase = "gap";
    } else {
        events::append(
            p,
            "checkpoint.fix.scope.grant",
            json!({"checkpoint": k, "files": granted, "verdicts": arr}),
        )?;
    }
    ckfix::write_overlay(p)?;
    p.write_guard()?;
    let granted_paths = if refusal.is_empty() {
        paths(&granted)
    } else {
        vec![]
    };
    Ok(json!({"checkpoint": k, "result": "scope", "phase": phase,
        "granted": granted_paths, "refused": if hold { retries } else { refused }, "refusal": refusal}))
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
