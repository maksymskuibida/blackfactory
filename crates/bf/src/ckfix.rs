//! The checkpoint fix part (C-17). When a checkpoint's check fails, or the checkpoint review files a
//! finding, a builder gets a fix to make in the checkpoint's own working copy. The fix may change only the
//! files of the checkpoint's units and add new test files in the test folders those units use. A reviewer
//! judges it, rounds and parking work as for a unit, and the engine then runs the checkpoint's checks
//! again on the new content. The engine records which units' files each fix touched.

use crate::events::{self, CkFixOpen, State};
use crate::evidence::{self, Receipt};
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, refuse_owned, R};
use crate::run;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// What a checkpoint fix may change.
pub struct FileSet {
    /// The files of the checkpoint's units, as patterns inside the checkpoint's working copy folder.
    pub allowed: Vec<String>,
    /// The test folders those units use: new test files may be added there.
    pub test_dirs: Vec<String>,
    /// The units of the checkpoint.
    pub units: Vec<String>,
}

/// The test folder a file lies in: the path up to and including the first folder named for tests.
fn test_folder(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return None;
    }
    let dirs = &parts[..parts.len() - 1];
    let ix = dirs.iter().position(|d| {
        let d = d.to_lowercase();
        d == "spec" || d == "specs" || d.contains("test")
    })?;
    let folder = dirs[..=ix].join("/");
    if folder.contains('*') {
        return None;
    }
    Some(format!("{folder}/"))
}

fn several_repos(p: &Project, pl: &Plan, k: &str) -> bool {
    run::checkpoint_repos(p, pl, k)
        .map(|r| r.len() > 1)
        .unwrap_or(false)
}

/// The unit's file patterns as they read inside the checkpoint's working copy folder.
/// The list of the record plus the unit's grants, landed or not (R-84).
fn unit_patterns(p: &Project, pl: &Plan, st: &State, k: &str, u: &Value) -> Vec<String> {
    let pre = if several_repos(p, pl, k) {
        p.repo_of_unit(u)
            .map(|r| format!("{}/", r.name))
            .unwrap_or_default()
    } else {
        String::new()
    };
    crate::scope::unit_files(st, u)
        .into_iter()
        .map(|f| format!("{pre}{f}"))
        .collect()
}

/// The set of files a fix of `k` may change: the union of the file lists of the checkpoint's units, plus the
/// test folders those units use (for new test files).
pub fn file_set(p: &Project, pl: &Plan, k: &str) -> FileSet {
    file_set_of(p, pl, k, None)
}

/// The same, held to some of the checkpoint's units (`only`): a wave fix may change only the files of the
/// units that landed since the last wave check passed, and new test files beside them.
pub fn file_set_of(p: &Project, pl: &Plan, k: &str, only: Option<&[String]>) -> FileSet {
    let st = events::replay(p).unwrap_or_default();
    let mut set = FileSet {
        allowed: vec![],
        test_dirs: vec![],
        units: vec![],
    };
    for u in pl
        .units
        .iter()
        .filter(|u| s(u, "checkpoint") == k)
        .filter(|u| {
            only.map(|o| o.iter().any(|x| x == s(u, "id")))
                .unwrap_or(true)
        })
    {
        set.units.push(s(u, "id").to_string());
        for f in unit_patterns(p, pl, &st, k, u) {
            if let Some(d) = test_folder(&f) {
                if !set.test_dirs.contains(&d) {
                    set.test_dirs.push(d);
                }
            }
            if !set.allowed.contains(&f) {
                set.allowed.push(f);
            }
        }
    }
    set
}

/// The units a changed file belongs to: those whose file lists hold it; a new test file that no list holds
/// belongs to the units that use its test folder.
pub fn units_of_file(p: &Project, pl: &Plan, k: &str, file: &str) -> Vec<String> {
    let mut by_list: Vec<String> = Vec::new();
    let mut by_folder: Vec<String> = Vec::new();
    let st = events::replay(p).unwrap_or_default();
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let id = s(u, "id").to_string();
        let pats = unit_patterns(p, pl, &st, k, u);
        if pats.iter().any(|g| util::path_matches(g, file)) {
            by_list.push(id);
        } else if pats
            .iter()
            .filter_map(|g| test_folder(g))
            .any(|d| util::path_matches(&d, file))
        {
            by_folder.push(id);
        }
    }
    if by_list.is_empty() {
        by_folder
    } else {
        by_list
    }
}

/// The repositories of a checkpoint with the folder of each working copy.
fn wts(p: &Project, pl: &Plan, k: &str) -> R<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for r in run::checkpoint_repos(p, pl, k)? {
        out.push((r.name.clone(), run::checkpoint_wt(p, pl, k, r)?));
    }
    Ok(out)
}

fn heads(p: &Project, pl: &Plan, k: &str) -> R<Vec<Value>> {
    let mut out = Vec::new();
    for (name, wt) in wts(p, pl, k)? {
        let h = gitx::must(&wt, &["rev-parse", "HEAD"]).unwrap_or_default();
        out.push(json!({"repo": name, "head": h}));
    }
    Ok(out)
}

/// The checks of `k` that fail at `content`, and whether one of them could not run at all.
fn failing_checks(
    pl: &Plan,
    st: &State,
    receipts: &[Receipt],
    k: &Value,
    content: &str,
) -> (Vec<Value>, bool) {
    let mut failed = Vec::new();
    let mut unreachable = false;
    for cid in str_list(k, "checks") {
        let Some(c) = pl.checks.iter().find(|c| s(c, "id") == cid) else {
            continue;
        };
        if crate::enforce::is_off(st, c) {
            continue;
        }
        let command = evidence::command_words(c).join(" ");
        for r in receipts
            .iter()
            .rev()
            .filter(|r| r.check() == cid && r.content() == content)
        {
            if r.passes() {
                break;
            }
            match r.outcome() {
                Some("unreachable") => {
                    unreachable = true;
                    break;
                }
                Some(_) => {
                    failed.push(json!({"check": cid, "command": command, "exit": null,
                        "tail": s(&r.v, "reason")}));
                    break;
                }
                None => {}
            }
            if r.is_run() && r.seal_ok && r.v["exit"].as_i64().unwrap_or(0) != 0 {
                failed.push(
                    json!({"check": cid, "command": command, "exit": r.v["exit"],
                    "tail": s(&r.v, "output_tail")}),
                );
                break;
            }
        }
    }
    (failed, unreachable)
}

/// Open a fix for a failed check of a checkpoint whose units have all landed. Called by `bf next`, so no
/// question reaches the human for a check that merely failed. A check that could not run goes to the human.
pub fn maybe_open(p: &Project) -> R<()> {
    let st = events::require_chain(p)?;
    if st.active_run().is_none() || st.sync_open.is_some() || st.ck_fix.is_some() {
        return Ok(());
    }
    if !st.gate_passed("plan", None) {
        return Ok(());
    }
    if p.policy.budget.cap > 0.0 && st.cost >= p.policy.budget.cap {
        return Ok(());
    }
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        let after = str_list(k, "after");
        if st.closed.contains_key(kid)
            || after.is_empty()
            || !after.iter().all(|u| st.unit_state(u) == "landed")
        {
            continue;
        }
        let wc = p.work_dir().join(kid);
        if !wc.join(".git").exists() && !wc.is_dir() {
            continue;
        }
        let content = p.content_hash(&wc).unwrap_or_default();
        if content.is_empty() || st.ck_declined.contains(&format!("{kid}@{content}")) {
            continue;
        }
        let (failed, unreachable) = failing_checks(&pl, &st, &receipts, k, &content);
        if failed.is_empty() || unreachable {
            continue;
        }
        let _lock = crate::lock::engine(p, &format!("open a fix of {kid}"))?;
        let st2 = events::require_chain(p)?;
        if st2.ck_fix.is_some() || st2.sync_open.is_some() {
            return Ok(());
        }
        if open_locked(p, &pl, kid, "check", failed, vec![], &content, false, None)? {
            return Ok(());
        }
    }
    Ok(())
}

/// Open a fix of a failed wave check of `k`: held to `units`, the ones that landed since the last wave
/// check passed. The caller holds the engine lock.
pub(crate) fn open_wave(
    p: &Project,
    pl: &Plan,
    k: &str,
    wave: u32,
    failed: Vec<Value>,
    units: &[String],
    content: &str,
) -> R<bool> {
    open_locked(
        p,
        pl,
        k,
        "wave",
        failed,
        vec![],
        content,
        false,
        Some((wave, units)),
    )
}

/// Write the `checkpoint.fix.open` event. `strict` refuses on a working copy with uncommitted changes;
/// otherwise such a checkpoint is left alone (`bf doctor` names it).
#[allow(clippy::too_many_arguments)]
fn open_locked(
    p: &Project,
    pl: &Plan,
    k: &str,
    source: &str,
    failed: Vec<Value>,
    findings: Vec<Value>,
    content: &str,
    strict: bool,
    wave: Option<(u32, &[String])>,
) -> R<bool> {
    for (_, wt) in wts(p, pl, k)? {
        if gitx::has_uncommitted(&wt) {
            if strict {
                return Err(refuse(
                    format!("open a fix of {k}"),
                    "the checkpoint's working copy has uncommitted changes",
                    &["Commit or remove them, then report again."],
                    "The working copy is clean.",
                ));
            }
            return Ok(false);
        }
    }
    let set = file_set_of(p, pl, k, wave.map(|(_, u)| u));
    events::append(
        p,
        "checkpoint.fix.open",
        json!({"checkpoint": k, "source": source, "failed": failed, "findings": findings,
               "allowed": set.allowed, "test_dirs": set.test_dirs, "units": set.units,
               "before": heads(p, pl, k)?, "content": content,
               "wave": wave.map(|(w, _)| w).unwrap_or(0)}),
    )?;
    write_overlay(p)?;
    p.write_guard()?;
    Ok(true)
}

/// Open a fix of `k` for findings the human picked or released. The checkpoint's working copy must be clean.
pub(crate) fn open_chosen(
    p: &Project,
    pl: &Plan,
    k: &str,
    findings: Vec<Value>,
    content: &str,
) -> R<()> {
    open_locked(p, pl, k, "review", vec![], findings, content, true, None)?;
    Ok(())
}

/// The probes that exposed the findings this fix is for, which the fix must keep as permanent tests: each
/// is a file in the checkpoint's working copy, committed, that holds the stored probe, and its command must
/// give the expected answer. Returns what is still missing.
fn probe_tests_missing(p: &Project, pl: &Plan, f: &CkFixOpen) -> R<Vec<String>> {
    let k = &f.checkpoint;
    let top = p.work_dir().join(k);
    let mut missing = Vec::new();
    for x in f
        .origin_findings
        .iter()
        .filter(|x| !s(x, "probe").is_empty())
    {
        let name = s(x, "probe");
        let file = s(x, "probe_file");
        let wt = wts(p, pl, k)?;
        // The file's path starts with the repository folder when the checkpoint has several.
        let in_repo = wt.iter().find_map(|(n, dir)| {
            let rel = if several_repos(p, pl, k) {
                file.strip_prefix(&format!("{n}/"))?.to_string()
            } else {
                file.to_string()
            };
            Some((dir.clone(), rel))
        });
        let Some((dir, rel)) = in_repo else {
            missing.push(format!(
                "probe `{name}`: its file `{file}` is in no repository of {k}"
            ));
            continue;
        };
        let committed = gitx::must(&dir, &["show", &format!("HEAD:{rel}")]).ok();
        let stored = fs::read_to_string(p.root.join(s(x, "probe_stored"))).unwrap_or_default();
        match committed {
            None => {
                missing.push(format!(
                    "probe `{name}`: add `{file}` to the suite and commit it"
                ));
                continue;
            }
            Some(body) if !stored.trim().is_empty() && !body.contains(stored.trim()) => {
                missing.push(format!(
                    "probe `{name}`: `{file}` does not hold the probe the reviewer stored"
                ));
                continue;
            }
            Some(_) => {}
        }
        let argv: Vec<String> = match x.get("probe_words").and_then(|w| w.as_array()) {
            Some(a) => a
                .iter()
                .filter_map(|w| w.as_str().map(|w| w.to_string()))
                .collect(),
            None => s(x, "probe_command")
                .split_whitespace()
                .map(|w| w.to_string())
                .collect(),
        };
        if argv.is_empty() {
            missing.push(format!("probe `{name}`: it has no command to run"));
            continue;
        }
        let want = x["probe_expect_exit"].as_i64().unwrap_or(0);
        let (exit, tail, _, _, _) =
            evidence::exec_capture(p, &top, &argv, p.policy.evidence.tail_bytes)?;
        if exit != want {
            missing.push(format!(
                "probe `{name}`: `{}` gave exit {exit}, and the expected answer is exit {want} ({}). {}",
                s(x, "probe_command"),
                s(x, "probe_expect"),
                tail.lines().last().unwrap_or("")
            ));
        }
    }
    Ok(missing)
}

/// The paths in the units' test folders that existed when the fix opened and that no unit lists: a fix may
/// add new test files there, never change one that was already there.
fn existing_test_files(p: &Project, f: &CkFixOpen) -> Vec<String> {
    let Ok(pl) = plan::load(p) else {
        return vec![];
    };
    let several = several_repos(p, &pl, &f.checkpoint);
    let Ok(repos) = wts(p, &pl, &f.checkpoint) else {
        return vec![];
    };
    let mut out = Vec::new();
    for (name, wt) in repos {
        let Some((_, before)) = f.before.iter().find(|(r, _)| *r == name) else {
            continue;
        };
        let listing =
            gitx::must(&wt, &["ls-tree", "-r", "--name-only", before]).unwrap_or_default();
        for l in listing.lines() {
            let path = if several {
                format!("{name}/{l}")
            } else {
                l.to_string()
            };
            let listed = f.allowed.iter().any(|g| util::path_matches(g, &path));
            if !listed && f.test_dirs.iter().any(|g| util::path_matches(g, &path)) {
                out.push(path);
            }
        }
    }
    out
}

/// Hand the guard the file set of the fix that is open (and nothing when none is): a signed file in the
/// engine's state folder that the guard overlays on the checkpoint's working copy.
pub fn write_overlay(p: &Project) -> R<()> {
    let st = events::replay(p)?;
    let path = p.state_dir().join("ckfix.json");
    match st
        .ck_fix
        .as_ref()
        .filter(|f| matches!(f.phase.as_str(), "fix" | "review"))
    {
        Some(f) => {
            let mut files: Vec<String> = Vec::new();
            let mut existing: Vec<String> = Vec::new();
            if f.phase == "fix" {
                files.extend(f.allowed.iter().cloned());
                files.extend(f.test_dirs.iter().cloned());
                existing = existing_test_files(p, f);
            }
            let body = json!({"checkpoint": f.checkpoint, "files": files, "existing": existing});
            let key = p.read_key().unwrap_or_default();
            let sig = util::hmac_sha256_hex(key.trim().as_bytes(), body.to_string().as_bytes());
            util::atomic_write(
                &path,
                json!({"body": body, "sig": sig}).to_string().as_bytes(),
            )?;
        }
        None => {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

/// The guard's side: put the file set of the open fix on the checkpoint's working copy in `table`. A file
/// that is missing, unsigned or signed with another key changes nothing.
pub fn overlay(table: &mut Value) {
    let Some(dir) = table["state_dir"].as_str().map(PathBuf::from) else {
        return;
    };
    let Ok(text) = fs::read_to_string(dir.join("ckfix.json")) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Ok(key) = fs::read_to_string(dir.join("key")) else {
        return;
    };
    let body = v["body"].clone();
    let want = util::hmac_sha256_hex(key.trim().as_bytes(), body.to_string().as_bytes());
    if v["sig"].as_str() != Some(want.as_str()) {
        return;
    }
    let id = body["checkpoint"].as_str().unwrap_or("").to_string();
    if let Some(ws) = table["worktrees"].as_array_mut() {
        for w in ws.iter_mut() {
            if w["kind"] == "checkpoint" && w["id"].as_str() == Some(id.as_str()) {
                w["files"] = body["files"].clone();
                w["existing"] = body["existing"].clone();
                w["sync"] = json!(true);
            }
        }
    }
}

/// The directive for the open fix.
pub fn directive(p: &Project, st: &State, f: &CkFixOpen) -> Value {
    let k = &f.checkpoint;
    let wc = p.work_dir().join(k).to_string_lossy().to_string();
    let lim = json!({"max_rounds": p.policy.limits.max_rounds, "round": f.rounds,
        "max_attempts": p.policy.limits.max_attempts, "attempts": f.attempts,
        "idle_minutes": p.policy.limits.idle_minutes,
        "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }});
    let brief = |role: &str| {
        p.briefs_dir()
            .join(format!("{k}-{role}.md"))
            .to_string_lossy()
            .to_string()
    };
    let abort = format!("bf checkpoint fix {k} --abort");
    match f.phase.as_str() {
        "fix" => {
            json!({"role": "builder", "action": if f.source == "wave" { "fix-wave" } else { "fix-checkpoint" }, "checkpoint": k,
            "wave": f.wave,
            "source": f.source, "tier": p.policy.tiers.builder, "working_copy": wc,
            "brief": brief("builder"), "limits": lim,
            "may_change": f.allowed.iter().chain(f.test_dirs.iter()).collect::<Vec<_>>(),
            "failing_checks": f.failed.iter().filter_map(|x| x["check"].as_str()).collect::<Vec<_>>(),
            "findings": f.findings.iter().map(|x| json!({"title": x["title"], "severity": x["severity"], "where": x["where"]})).collect::<Vec<_>>(),
            "commands": [format!("bf brief {k} --role builder")], "abort_command": abort})
        }
        "review" => {
            json!({"role": "reviewer", "action": if f.source == "wave" { "review-wave-fix" } else { "review-checkpoint-fix" }, "checkpoint": k,
            "wave": f.wave,
            "tier": p.policy.tiers.reviewer, "working_copy": wc, "brief": brief("reviewer"),
            "limits": lim, "round": f.rounds_log.len() + 1,
            "commands": [format!("bf brief {k} --role reviewer")], "abort_command": abort})
        }
        "gap" => json!({"role": "human", "action": "plan-gap", "checkpoint": k,
            "missing": f.missing,
            "note": format!("The fix of {k} needs what the plan never gave it: {}. Decide by amendment: a new unit, or an existing one extended. `{abort}` puts the checkpoint back as it was.", f.missing.join("; ")),
            "abort_command": abort}),
        _ => json!({"role": "human", "action": "decide-checkpoint-fix", "checkpoint": k,
            "note": format!("The fix of {k} is parked ({}). The human says whether to try again (`bf checkpoint fix {k} --release --quote \"<their words>\"`) or to go back (`{abort}`).", f.reason),
            "release_command": format!("bf checkpoint fix {k} --release --quote \"<the human's words>\""),
            "abort_command": abort}),
    }
}

/// Whether a report on `k` belongs to the checkpoint fix part: a fix of `k` is open, or the checkpoint
/// review files findings with no fix open yet.
pub fn wants(st: &State, k: &str, result: &str, role: Option<&str>) -> bool {
    if let Some(f) = &st.ck_fix {
        return f.checkpoint == k;
    }
    st.sync_open.is_none() && result == "findings" && role == Some("reviewer")
}

fn finding_list(findings: Option<&Value>) -> Vec<Value> {
    findings
        .and_then(|f| {
            f.as_array()
                .cloned()
                .or_else(|| f.get("findings").and_then(|x| x.as_array().cloned()))
        })
        .unwrap_or_default()
}

/// The files the fix changed since it opened, as paths inside the checkpoint's working copy folder, with
/// git's status letter.
fn changed(p: &Project, pl: &Plan, f: &CkFixOpen) -> R<Vec<(String, char)>> {
    let several = several_repos(p, pl, &f.checkpoint);
    let mut out = Vec::new();
    for (name, wt) in wts(p, pl, &f.checkpoint)? {
        let before = f
            .before
            .iter()
            .find(|(r, _)| *r == name)
            .map(|(_, h)| h.clone())
            .unwrap_or_default();
        if before.is_empty() {
            continue;
        }
        let d = gitx::must(
            &wt,
            &["diff", "--name-status", "--no-renames", &before, "HEAD"],
        )?;
        for l in d.lines() {
            let Some((st, path)) = l.split_once('\t') else {
                continue;
            };
            let path = if several {
                format!("{name}/{path}")
            } else {
                path.to_string()
            };
            out.push((path, st.chars().next().unwrap_or('M')));
        }
    }
    Ok(out)
}

/// Whether the engine may let the fix change `path`: a file of the checkpoint's units, or a new file in a
/// test folder those units use.
fn within(f: &CkFixOpen, path: &str, status: char) -> bool {
    f.allowed.iter().any(|g| util::path_matches(g, path))
        || (status == 'A' && f.test_dirs.iter().any(|g| util::path_matches(g, path)))
}

fn fail_lines(failed: &[Value]) -> Vec<String> {
    failed
        .iter()
        .map(|x| {
            format!(
                "{} fails ({}): {}",
                x["check"].as_str().unwrap_or(""),
                x["command"].as_str().unwrap_or(""),
                x["tail"]
                    .as_str()
                    .unwrap_or("")
                    .lines()
                    .last()
                    .unwrap_or("")
            )
        })
        .collect()
}

/// A failing attempt: the fix goes back to its builder, or is parked when the attempts reach the limit.
fn attempt(p: &Project, f: &CkFixOpen, failed: &[Value]) -> R<&'static str> {
    let over = f.attempts + 1 >= p.policy.limits.max_attempts;
    let to = if over { "parked" } else { "fix" };
    events::append(
        p,
        "checkpoint.fix.attempt",
        json!({"checkpoint": f.checkpoint, "to": to, "failed": failed,
               "reason": if over { format!("{} failing attempts in a row", f.attempts + 1) } else { String::new() }}),
    )?;
    write_overlay(p)?;
    p.write_guard()?;
    Ok(to)
}

/// `bf report <K>` for a checkpoint fix. The checkpoint review files findings (opening a fix), the builder
/// says built, blocked or gap, and the fix's reviewer says findings or built (clean).
pub fn report(
    p: &Project,
    k: &str,
    result: &str,
    role: Option<&str>,
    note: &str,
    findings: Option<&Value>,
    token: Option<&str>,
) -> R<Value> {
    report_inner(p, k, result, role, note, findings, token, false)
}

/// The findings of a checkpoint review round that was recorded (with its areas): open the fix from them.
/// Only the review round calls this; a bare reviewer report never does.
pub fn report_from_round(
    p: &Project,
    k: &str,
    note: &str,
    findings: Option<&Value>,
    token: Option<&str>,
) -> R<Value> {
    report_inner(
        p,
        k,
        "findings",
        Some("reviewer"),
        note,
        findings,
        token,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn report_inner(
    p: &Project,
    k: &str,
    result: &str,
    role: Option<&str>,
    note: &str,
    findings: Option<&Value>,
    token: Option<&str>,
    via_round: bool,
) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("report on the fix of {k}"))?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let pl = plan::load(p)?;
    let Some(f) = st.ck_fix.clone().filter(|f| f.checkpoint == k) else {
        return open_from_review(p, &st, &pl, k, result, role, findings, token, via_round);
    };
    let expected = match f.phase.as_str() {
        "fix" => "builder",
        "review" => "reviewer",
        "gap" => {
            return Err(refuse(
                format!("report on the fix of {k}"),
                "the fix needs what the plan never gave it: the human decides by amendment",
                &["`bf next` names what is missing.", "`bf checkpoint fix <K> --abort` puts the checkpoint back as it was."],
                "`bf status` shows the fix back at work or gone.",
            ))
        }
        _ => {
            return Err(refuse(
                format!("report on the fix of {k}"),
                "the fix is parked: the human decides",
                &["The human releases it (`bf checkpoint fix <K> --release --quote \"...\"`) or the conductor aborts it (`--abort`)."],
                "`bf status` shows the fix back at work.",
            ))
        }
    };
    if role.map(|r| r != expected).unwrap_or(false) {
        return Err(refuse(
            format!("report as {}", role.unwrap_or("")),
            format!("the fix of {k} waits for its {expected}"),
            &["Report with the role the directive named."],
            "`bf status` shows the fix in its new phase.",
        ));
    }
    if p.policy.roles.tokens && !run::holds_token(p, &format!("{k}:{expected}"), token) {
        return Err(refuse(
            format!("report on the fix of {k} as {expected}"),
            format!("a report needs the token of the {expected}'s directive; none, or the wrong one, was given"),
            &["`bf next --json` shows the directive with its `token`; pass it with `--token`."],
            "`bf status` shows the fix in its new phase.",
        ));
    }
    match (f.phase.as_str(), result) {
        ("fix", "blocked") => {
            let n = f.attempts + 1;
            let reason = if note.is_empty() { "the builder is blocked" } else { note };
            events::append(p, "checkpoint.fix.park", json!({"checkpoint": k, "reason": reason, "attempts": n}))?;
            write_overlay(p)?;
            p.write_guard()?;
            Ok(json!({"checkpoint": k, "phase": "parked"}))
        }
        ("fix", "gap") => {
            let missing: Vec<String> = if note.trim().is_empty() {
                vec!["something the plan never gave this checkpoint (the builder gave no detail)".into()]
            } else {
                note.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
            };
            plan_gap(p, k, &missing, "the builder reports a need the plan never gave this checkpoint")
        }
        ("fix", "built" | "fixed") => {
            for (name, wt) in wts(p, &pl, k)? {
                let merging = gitx::run(&wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[])
                    .map(|o| o.ok)
                    .unwrap_or(false);
                if merging || gitx::has_uncommitted(&wt) {
                    return Err(refuse(
                        format!("report the fix of {k}"),
                        format!("{name}: the working copy has uncommitted changes"),
                        &["Commit the fix in the checkpoint's working copy (`git add` the files, `git commit`), then report again."],
                        "`bf status` shows the fix in review.",
                    ));
                }
            }
            let files = changed(p, &pl, &f)?;
            let outside: Vec<String> = files
                .iter()
                .filter(|(path, st)| !within(&f, path, *st))
                .map(|(path, st)| {
                    if *st == 'A' || *st == 'M' || *st == 'D' {
                        format!("{path} is outside the files of {k}'s units and is not a new test file")
                    } else {
                        format!("{path} is outside the files of {k}'s units")
                    }
                })
                .collect();
            if !outside.is_empty() {
                return plan_gap(p, k, &outside, "the fix changed files the plan never gave to this checkpoint");
            }
            if files.is_empty() {
                return Err(refuse(
                    format!("report the fix of {k}"),
                    "no file changed since the fix opened",
                    &["Make the change and commit it in the checkpoint's working copy, then report again.", "If the fix needs something outside the checkpoint's files, report `--result gap --note \"what is missing\"`."],
                    "`bf status` shows the fix in review.",
                ));
            }
            // A wave fix is tried on the wave checks; it counts only after the reviewer's clean round.
            let failed = if f.source == "wave" {
                crate::wavecheck::rerun(p, &pl, k, f.wave, false)?
            } else {
                run::run_k_checks(p, &pl, k)?
            };
            if !failed.is_empty() {
                let to = attempt(p, &f, &failed)?;
                let mut steps = fail_lines(&failed);
                steps.push(if to == "parked" {
                    "The fix is parked: the human decides.".into()
                } else {
                    "Repair it (only the files named in your brief), commit, run the failing checks again with `bf evidence run`, then report again.".into()
                });
                return Err(refuse_owned(
                    format!("report the fix of {k}"),
                    format!("the checkpoint's checks still fail: {}", failed.len()),
                    steps,
                    "`bf status` shows the fix in review.",
                ));
            }
            let absent = probe_tests_missing(p, &pl, &f)?;
            if !absent.is_empty() {
                let mut steps = absent.clone();
                steps.push("Add each probe to the suite as a permanent test with its expected answer, commit it, then report again.".into());
                return Err(refuse_owned(
                    format!("report the fix of {k}"),
                    format!("a probe that exposed a fault is not yet a permanent test that passes: {}", absent.len()),
                    steps,
                    "`bf status` shows the fix in review.",
                ));
            }
            events::append(p, "checkpoint.fix.built", json!({"checkpoint": k, "files": files.iter().map(|x| x.0.clone()).collect::<Vec<_>>()}))?;
            write_overlay(p)?;
            p.write_guard()?;
            Ok(json!({"checkpoint": k, "phase": "review"}))
        }
        ("review", "findings") => {
            let listed = finding_list(findings);
            if listed.is_empty() {
                return Err(refuse(
                    format!("report findings on the fix of {k}"),
                    "no finding was given",
                    &["Give the findings (`--findings -` with a JSON list), or report `built` when the fix is right."],
                    "`bf status` shows the fix in its new phase.",
                ));
            }
            let over = f.rounds + 1 > p.policy.limits.max_rounds;
            record_round(p, &pl, &f, "findings", if over { "parked" } else { "fix" }, &listed, if over { format!("more than {} review rounds", p.policy.limits.max_rounds) } else { String::new() })?;
            Ok(json!({"checkpoint": k, "phase": if over { "parked" } else { "fix" }}))
        }
        ("review", "built") => {
            record_round(p, &pl, &f, "clean", "done", &[], String::new())?;
            // Clean: the checkpoint's checks run again on the new content, through the engine. A wave
            // fix runs the wave checks again, and that run is the wave's new result.
            if f.source == "wave" {
                let failed = crate::wavecheck::rerun(p, &pl, k, f.wave, true)?;
                if !failed.is_empty() {
                    let to = attempt(p, &f, &failed)?;
                    return Ok(json!({"checkpoint": k, "phase": if to == "parked" { "parked" } else { "fix" },
                        "failed_checks": failed}));
                }
                let again: Vec<String> = crate::wavecheck::records(&pl)
                    .iter()
                    .map(|c| s(c, "id").to_string())
                    .collect();
                return finish(p, &pl, &f, again);
            }
            let failed = run::run_k_checks(p, &pl, k)?;
            if !failed.is_empty() {
                let to = attempt(p, &f, &failed)?;
                return Ok(json!({"checkpoint": k, "phase": if to == "parked" { "parked" } else { "fix" },
                    "failed_checks": failed}));
            }
            // Only the command checks that are switched on ran again; agent-judged ones did not.
            let switched_off = run::skipped_k_checks(p, &pl, k)?;
            let ran_again: Vec<String> = pl
                .checkpoint(k)
                .map(|kr| str_list(kr, "checks"))
                .unwrap_or_default()
                .into_iter()
                .filter(|cid| {
                    !switched_off.contains(cid)
                        && pl
                            .checks
                            .iter()
                            .any(|c| s(c, "id") == cid && s(c, "how") == "command")
                })
                .collect();
            finish(p, &pl, &f, ran_again)
        }
        (ph, r) => Err(refuse(
            format!("report {r} on the fix of {k}"),
            format!("a `{r}` report does not fit a fix in `{ph}`"),
            &["The builder reports built, blocked or gap; the reviewer reports findings or built (clean)."],
            "`bf status` shows the fix in its new phase.",
        )),
    }
}

/// The fix is a plan gap: record it and refuse, naming what is missing.
fn plan_gap(p: &Project, k: &str, missing: &[String], reason: &str) -> R<Value> {
    events::append(
        p,
        "checkpoint.fix.gap",
        json!({"checkpoint": k, "missing": missing, "reason": reason}),
    )?;
    write_overlay(p)?;
    p.write_guard()?;
    Err(refuse_owned(
        format!("accept the fix of {k}"),
        format!("{reason}: {}", missing.join("; ")),
        vec![
            "This is a plan gap, and the human decides it: a new unit, or an existing one extended, through an amendment.".to_string(),
            format!("`bf checkpoint fix {k} --abort` puts the checkpoint back as it was."),
        ],
        "`bf next` shows the plan gap for the human.",
    ))
}

/// The checkpoint review filed findings and no fix is open: open one for a builder.
#[allow(clippy::too_many_arguments)]
fn open_from_review(
    p: &Project,
    st: &State,
    pl: &Plan,
    k: &str,
    result: &str,
    role: Option<&str>,
    findings: Option<&Value>,
    token: Option<&str>,
    via_round: bool,
) -> R<Value> {
    let bad = |because: &str| {
        refuse(
            format!("report on {k}"),
            because.to_string(),
            &["Units are reported on with `bf report <U>`.", "A checkpoint is reported on only by its reviewer (findings), or while a sync or a fix of it is open."],
            "`bf status` shows the new state.",
        )
    };
    let Some(kr) = pl.checkpoint(k) else {
        return Err(bad("no such checkpoint"));
    };
    let after = str_list(kr, "after");
    if result != "findings"
        || role != Some("reviewer")
        || st.sync_open.is_some()
        || st.closed.contains_key(k)
        || after.is_empty()
        || !after.iter().all(|u| st.unit_state(u) == "landed")
    {
        return Err(bad("no fix is open for it, and this is not a review of a checkpoint whose units have landed"));
    }
    // A checkpoint's review report goes through the review round (it needs an open round and the areas
    // probed), and a parked checkpoint is released only by the human. The older path stays only for a
    // checkpoint whose review is switched off for the run.
    if !via_round && (crate::ckreview::required(st) || crate::ckreview::parked(p, st, k)?) {
        return Err(refuse(
            format!("report findings on {k}"),
            "the review of a checkpoint reports through its review round, with the areas probed; no round is open, or the checkpoint is parked for the human",
            &[
                "Run `bf brief <K> --role reviewer` to open the round, then report with the areas probed.",
                "A parked checkpoint is released only by the human, in their own words.",
            ],
            "`bf next` shows what the checkpoint waits for.",
        ));
    }
    if p.policy.roles.tokens && !run::holds_token(p, &format!("{k}:reviewer"), token) {
        return Err(refuse(
            format!("report findings on {k}"),
            "a report needs the token of the reviewer's directive; none, or the wrong one, was given",
            &["Pass the token the conductor holds for the checkpoint's reviewer with `--token`."],
            "`bf status` shows the fix at work.",
        ));
    }
    let listed = finding_list(findings);
    if listed.is_empty() {
        return Err(refuse(
            format!("report findings on {k}"),
            "no finding was given",
            &["Give the findings (`--findings -` with a JSON list of `{title, severity, where, action}`)."],
            "`bf status` shows the fix at work.",
        ));
    }
    let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
    open_locked(p, pl, k, "review", vec![], listed, &content, true, None)?;
    Ok(json!({"checkpoint": k, "phase": "fix", "fix_opened": true}))
}

/// Record a review round of the fix: its number, the content it judged and what it covered.
fn record_round(
    p: &Project,
    pl: &Plan,
    f: &CkFixOpen,
    result: &str,
    to: &str,
    listed: &[Value],
    reason: String,
) -> R<()> {
    let round = f.rounds_log.len() + 1;
    let covered = if round == 1 {
        "the whole fix, cold".to_string()
    } else {
        format!(
            "the earlier findings, and what changed since round {}",
            round - 1
        )
    };
    let content = p
        .content_hash(&p.work_dir().join(&f.checkpoint))
        .unwrap_or_default();
    events::append(
        p,
        "checkpoint.fix.reviewed",
        json!({"checkpoint": f.checkpoint, "round": round, "content": content,
               "heads": heads(p, pl, &f.checkpoint)?, "covered": covered,
               "result": result, "to": to, "findings": listed, "reason": reason}),
    )?;
    write_overlay(p)?;
    p.write_guard()?;
    Ok(())
}

/// The fix is reviewed clean and its checks pass: record which units' files it touched.
fn finish(p: &Project, pl: &Plan, f: &CkFixOpen, rerun: Vec<String>) -> R<Value> {
    let files = changed(p, pl, f)?;
    let mut by_file = serde_json::Map::new();
    let mut units: Vec<String> = Vec::new();
    for (path, _) in &files {
        let us = units_of_file(p, pl, &f.checkpoint, path);
        for u in &us {
            if !units.contains(u) {
                units.push(u.clone());
            }
        }
        by_file.insert(path.clone(), json!(us));
    }
    units.sort();
    let names: Vec<String> = files.iter().map(|x| x.0.clone()).collect();
    events::append(
        p,
        "checkpoint.fix.done",
        json!({"checkpoint": f.checkpoint, "files": names, "units": units, "by_file": by_file}),
    )?;
    write_overlay(p)?;
    p.write_guard()?;
    Ok(
        json!({"checkpoint": f.checkpoint, "phase": "done", "files": names, "units": units,
        "checks_run_again": rerun}),
    )
}

/// `bf checkpoint fix <K> --abort` and `--release --quote "..."`.
pub fn command(p: &Project, k: &str, quote: &str, abort: bool, release: bool) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("fix {k}"))?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let Some(f) = st.ck_fix.clone().filter(|f| f.checkpoint == k) else {
        return Err(refuse(
            format!("settle the fix of {k}"),
            "no fix is open for it",
            &["`bf status` shows an open fix when there is one."],
            "`bf status` shows no open fix.",
        ));
    };
    let pl = plan::load(p)?;
    if abort {
        for (name, wt) in wts(p, &pl, k)? {
            if let Some((_, before)) = f.before.iter().find(|(r, _)| *r == name) {
                gitx::must(&wt, &["reset", "--hard", "-q", before])?;
                gitx::must(&wt, &["clean", "-fdq"])?;
            }
        }
        events::append(p, "checkpoint.fix.abort", json!({"checkpoint": k}))?;
        write_overlay(p)?;
        p.write_guard()?;
        return Ok(
            json!({"checkpoint": k, "aborted": true, "message": "the checkpoint is as it was before the fix started; the human decides what to do with the failure"}),
        );
    }
    if release {
        if f.phase != "parked" {
            return Err(refuse(
                format!("release the fix of {k}"),
                format!("it is `{}`, not parked", f.phase),
                &["`bf next` shows what it waits for."],
                "`bf status` shows the fix back at work.",
            ));
        }
        let (turn, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
        events::append(
            p,
            "checkpoint.fix.release",
            json!({"checkpoint": k, "turn": turn, "quote": quote.trim()}),
        )?;
        write_overlay(p)?;
        p.write_guard()?;
        return Ok(json!({"checkpoint": k, "released": true}));
    }
    Err(refuse(
        format!("run `bf checkpoint fix {k}`"),
        "it needs --abort or --release",
        &["Run `bf checkpoint fix <K> --abort`, or `bf checkpoint fix <K> --release --quote \"<the human's words>\"` for a parked fix."],
        "The command prints the result.",
    ))
}

/// The open fix and every finished fix, for `bf status`.
pub fn status_json(st: &State) -> Value {
    json!({
        "open": st.ck_fix.as_ref().map(|f| json!({
            "checkpoint": f.checkpoint, "source": f.source, "phase": f.phase, "rounds": f.rounds,
            "attempts": f.attempts, "reason": f.reason, "missing": f.missing,
            "may_change": f.allowed.iter().chain(f.test_dirs.iter()).collect::<Vec<_>>(),
            "review_rounds": f.rounds_log,
        })),
        "done": st.ck_fixes.iter().map(|d| json!({
            "id": d.id, "checkpoint": d.checkpoint, "source": d.source, "files": d.files,
            "units": d.units, "by_file": d.by_file, "review_rounds": d.rounds_log,
        })).collect::<Vec<_>>(),
    })
}

/// The ids of the finished fixes attributed to a unit.
pub fn fixes_of_unit(st: &State, unit: &str) -> Vec<String> {
    st.ck_fixes
        .iter()
        .filter(|d| d.units.iter().any(|u| u == unit))
        .map(|d| d.id.clone())
        .collect()
}

fn limited(t: String, max: usize) -> String {
    let lines: Vec<&str> = t.lines().collect();
    if lines.len() > max {
        format!(
            "{}\n... ({} more lines)\n",
            lines[..max].join("\n"),
            lines.len() - max
        )
    } else {
        t
    }
}

/// The brief of the builder or the reviewer of the open fix of `k`.
pub fn brief(p: &Project, f: &CkFixOpen, role: &str) -> R<PathBuf> {
    let k = &f.checkpoint;
    let pl = plan::load(p)?;
    let wc = p.work_dir().join(k);
    let mut md = format!("# Brief: the fix of {k} as {role}\n\n");
    md.push_str(&format!(
        "Checkpoint {k} {}. The fix is made in the checkpoint's own working copy, `{}`.\n\n",
        match f.source.as_str() {
            "check" => "failed a check".to_string(),
            "wave" => format!(
                "failed the wave check of wave {}, which ran on a copy of its content after that wave landed. The fix is held to the files of the units that landed since the last wave check passed ({}), and new test files beside them",
                f.wave,
                f.units.join(", ")
            ),
            _ => "drew findings in its review".to_string(),
        },
        wc.display()
    ));
    if !f.failed.is_empty() {
        md.push_str("## The failing checks\n\n");
        for x in &f.failed {
            md.push_str(&format!(
                "- {} (`{}`), exit {}. Output tail:\n\n```\n{}\n```\n\n",
                x["check"].as_str().unwrap_or(""),
                x["command"].as_str().unwrap_or(""),
                x["exit"],
                limited(x["tail"].as_str().unwrap_or("").to_string(), 60)
            ));
        }
    }
    if !f.findings.is_empty() {
        md.push_str("## Findings to fix\n\n");
        for x in &f.findings {
            md.push_str(&format!(
                "- [{}] {} at {}: {}\n",
                x["severity"].as_str().unwrap_or(""),
                x["title"].as_str().unwrap_or(""),
                x["where"].as_str().unwrap_or(""),
                x["action"].as_str().unwrap_or("")
            ));
            if !s(x, "probe").is_empty() {
                md.push_str(&format!(
                    "  - A probe exposed it: `{}`, kept in `{}`. Add it to the suite as a permanent test at `{}`, with its expected answer ({}; exit {}), and commit it. Run it with `{}`. The fix does not count until that test is there and passes.\n",
                    s(x, "probe"),
                    s(x, "probe_stored"),
                    s(x, "probe_file"),
                    s(x, "probe_expect"),
                    x["probe_expect_exit"],
                    s(x, "probe_command")
                ));
            }
        }
        md.push('\n');
    }
    if role == "builder" {
        md.push_str("## What you may change\n\nOnly the files of this checkpoint's units, and new test files in the test folders they use:\n\n");
        for g in f.allowed.iter().take(80) {
            md.push_str(&format!("- `{g}`\n"));
        }
        for g in &f.test_dirs {
            md.push_str(&format!("- `{g}` (new test files only)\n"));
        }
        md.push_str(&format!(
            "\n## Rules\n\n- Make the failing checks pass by fixing the code, never by weakening a test: do not delete, skip or loosen a test or an assertion to get a pass. A reviewer looks for it.\n- Commit the fix in the working copy (`git add` the files, `git commit`). Do not reset, check out, merge or rebase anything.\n- Run the failing checks again with `bf evidence run <check> -- <its command>` from inside `{}`.\n- Report with `bf report {k} --role builder --result built --token <token>`.\n- If the fix needs a file outside the list above, or behaviour that no record describes, do not work around it: `bf report {k} --role builder --result gap --note \"what is missing\"`. The human then decides by amendment.\n- If you cannot go on: `bf report {k} --role builder --result blocked --note \"why\"`.\n",
            wc.display()
        ));
    } else {
        let round = f.rounds_log.len() + 1;
        md.push_str(&format!("## Review round {round}\n\n"));
        if round == 1 {
            md.push_str("This is the first round and it is cold: no earlier report is given. Read the whole fix and everything it touches.\n\n");
        } else {
            md.push_str("This is a later round. Check that each finding below is fixed, and start from the changes made since the content the round before judged. Do not review the whole scope again when nothing points there; you may file a new finding on what changed or on something a fix broke.\n\n");
        }
        md.push_str("You may read anywhere in the working copy when you need to in order to judge a change: what it touches, calls or could break, and the code it relies on.\n\n");
        if !f.history.is_empty() {
            md.push_str("## Findings of the earlier rounds\n\n");
            for h in &f.history {
                for x in h["findings"].as_array().into_iter().flatten() {
                    md.push_str(&format!(
                        "- round {}: [{}] {} at {}: {} (the builder reports it fixed; check)\n",
                        h["round"],
                        x["severity"].as_str().unwrap_or(""),
                        x["title"].as_str().unwrap_or(""),
                        x["where"].as_str().unwrap_or(""),
                        x["action"].as_str().unwrap_or("")
                    ));
                }
            }
            md.push('\n');
        }
        let last = f.rounds_log.last();
        for (name, wt) in wts(p, &pl, k)? {
            let before = f
                .before
                .iter()
                .find(|(r, _)| *r == name)
                .map(|(_, h)| h.clone())
                .unwrap_or_default();
            if before.is_empty() {
                continue;
            }
            if let Some(l) = last {
                let judged = l["heads"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|h| h["repo"] == name.as_str())
                    .and_then(|h| h["head"].as_str())
                    .unwrap_or("")
                    .to_string();
                if !judged.is_empty() {
                    md.push_str(&format!(
                        "## Changes since round {} (`{name}`)\n\n```diff\n{}\n```\n\n",
                        l["round"],
                        limited(
                            gitx::must(&wt, &["diff", &judged, "HEAD"]).unwrap_or_default(),
                            400
                        )
                    ));
                }
            }
            md.push_str(&format!(
                "## The whole fix (`{name}`)\n\n```diff\n{}\n```\n\n",
                limited(
                    gitx::must(&wt, &["diff", &before, "HEAD"]).unwrap_or_default(),
                    400
                )
            ));
        }
        md.push_str(&format!(
            "## What to judge\n\nDoes the fix solve the failure without weakening a test? Does it stay inside the files of the checkpoint's units (and new test files)? Report with `bf report {k} --role reviewer --result findings --findings -` (a JSON list of `{{title, severity, where, action}}`), or `--result built` when the fix is right. Pass the token of your directive with `--token`.\n"
        ));
    }
    let path = p.briefs_dir().join(format!("{k}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// The file the guard reads, for tests and tools.
pub fn overlay_path(p: &Project) -> PathBuf {
    p.state_dir().join("ckfix.json")
}
