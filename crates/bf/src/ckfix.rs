//! The checkpoint fix part (C-17). When a checkpoint's check fails, or the checkpoint review files a
//! finding, a builder gets a fix to make in the checkpoint's own working copy. The fix may change only the
//! files of the checkpoint's units and add new test files in the test folders those units use. A reviewer
//! judges it, rounds and parking work as for a unit, and the engine then runs the checkpoint's checks
//! again on the new content. The engine records which units' files each fix touched.

pub mod design;

use crate::events::{self, CkFixOpen, State};
use crate::evidence::{self, Receipt};
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, refuse_owned, R};
use crate::run;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// What a checkpoint fix may change.
pub struct FileSet {
    /// The files of the checkpoint's units, as patterns inside the checkpoint's working copy folder.
    pub allowed: Vec<String>,
    /// The test folders those units use: new test files may be added there.
    pub test_dirs: Vec<String>,
    /// The units of the checkpoint.
    pub units: Vec<String>,
}

/// The files a fix may change right now: the checkpoint's units' files read from the records in force, the
/// files a failed format or lint output names, and the fix's grants (R-70, D-91).
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub allowed: Vec<String>,
    pub test_dirs: Vec<String>,
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

pub(crate) fn several_repos(p: &Project, pl: &Plan, k: &str) -> bool {
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

/// What the fix may change, worked out from the records and the grants as they stand now.
pub fn live(p: &Project, pl: &Plan, st: &State, f: &CkFixOpen) -> Live {
    let only = (f.source == "wave").then_some(f.units.as_slice());
    let mut set = file_set_of(p, pl, &f.checkpoint, only);
    if f.source == "check" && format_lint_only(&f.origin_failed) {
        let top = p.work_dir().join(&f.checkpoint);
        for path in named_in(&top, &f.origin_failed, &placed(p, pl, st, &f.checkpoint)).into_keys()
        {
            if !set.allowed.contains(&path) {
                set.allowed.push(path);
            }
        }
    }
    for path in crate::fixscope::granted(st, &f.checkpoint) {
        if !set.allowed.contains(&path) {
            set.allowed.push(path);
        }
    }
    Live {
        allowed: set.allowed,
        test_dirs: set.test_dirs,
    }
}

/// The list the fix was opened with, for when the records cannot be read.
fn frozen(f: &CkFixOpen) -> Live {
    Live {
        allowed: f.allowed.clone(),
        test_dirs: f.test_dirs.clone(),
    }
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
pub(crate) fn wts(p: &Project, pl: &Plan, k: &str) -> R<Vec<(String, PathBuf)>> {
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
pub(crate) fn failing_checks(
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
        if crate::enforce::is_off(st, c)
            || evidence::accepted_at(st, s(k, "id"), &cid, content).is_some()
        {
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

/// Whether the engine opens a fix for the failing checks `failing_checks` found: a check that could not run
/// goes to the human instead, and a switched-off check is not among them. `maybe_open` and the hold of a
/// checkpoint (`observe_holds`) read this one rule.
pub(crate) fn opens_fix(failed: &[Value], unreachable: bool) -> bool {
    !failed.is_empty() && !unreachable
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
        if !opens_fix(&failed, unreachable) {
            continue;
        }
        let _lock = crate::lock::engine(p, &format!("open a fix of {kid}"))?;
        let st2 = events::require_chain(p)?;
        if st2.ck_fix.is_some() || st2.sync_open.is_some() {
            return Ok(());
        }
        if open_locked(
            p,
            &pl,
            kid,
            "check",
            failed,
            (vec![], vec![]),
            &content,
            false,
            None,
        )? {
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
        (vec![], vec![]),
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
    (findings, finding_ids): (Vec<Value>, Vec<(u32, u32)>),
    content: &str,
    strict: bool,
    wave: Option<(u32, &[String])>,
) -> R<bool> {
    open_locked_by(
        p,
        pl,
        k,
        source,
        failed,
        (findings, finding_ids),
        content,
        strict,
        wave,
        None,
        None,
    )
}

/// The same, for a fix of review findings: `opened_by` is the way it was opened (`report`, `release`, `pick`
/// or `low`) and `human` the turn and the quote the human's words give, when they opened it. The replay
/// checks each way against the events that justify it.
#[allow(clippy::too_many_arguments)]
fn open_locked_by(
    p: &Project,
    pl: &Plan,
    k: &str,
    source: &str,
    failed: Vec<Value>,
    (findings, finding_ids): (Vec<Value>, Vec<(u32, u32)>),
    content: &str,
    strict: bool,
    wave: Option<(u32, &[String])>,
    human: Option<(u64, &str, Option<u64>)>,
    opened_by: Option<&str>,
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
    let mut set = file_set_of(p, pl, k, wave.map(|(_, u)| u));
    // A fix of only format or lint checks may also change the files the failed output names (R-70).
    if source == "check" && format_lint_only(&failed) {
        let top = p.work_dir().join(k);
        let st = events::replay(p)?;
        for path in named_in(&top, &failed, &placed(p, pl, &st, k)).into_keys() {
            if !set.allowed.contains(&path) {
                set.allowed.push(path);
            }
        }
    }
    // The work an aborted fix of this checkpoint saved is applied first (R-70); what no longer applies is named.
    let mut from_saved = String::new();
    let mut applied: Vec<String> = Vec::new();
    let mut not_applied: Vec<String> = Vec::new();
    {
        let st = events::replay(p)?;
        if let Some(folder) = crate::fixscope::pending_saved(p, &st, k) {
            let live = Live {
                allowed: set.allowed.clone(),
                test_dirs: set.test_dirs.clone(),
            };
            (applied, not_applied) = crate::fixscope::apply_saved(p, pl, k, &folder, &live)?;
            from_saved = folder;
        }
    }
    // `earlier` is the seq of an earlier opening whose words these are (a fix reopened after an abort).
    let by = human
        .map(|(t, q, earlier)| match earlier {
            Some(e) => json!({"quote": q.trim(), "earlier": e}),
            None => json!({"turn": t, "quote": q.trim()}),
        })
        .unwrap_or(Value::Null);
    let mut ev = json!({"checkpoint": k, "source": source, "failed": failed, "findings": findings, "human": by,
               "finding_ids": finding_ids.iter().map(|(r, i)| json!({"round": r, "index": i})).collect::<Vec<_>>(),
               "allowed": set.allowed, "test_dirs": set.test_dirs, "units": set.units,
               "before": heads(p, pl, k)?, "content": content,
               "wave": wave.map(|(w, _)| w).unwrap_or(0), "from_saved": from_saved});
    if let Some(how) = opened_by {
        ev["opened_by"] = json!(how);
    }
    events::append(p, "checkpoint.fix.open", ev)?;
    if !from_saved.is_empty() {
        crate::fixscope::note_reopened(p, &from_saved, &applied, &not_applied)?;
    }
    write_overlay(p)?;
    p.write_guard()?;
    Ok(true)
}

/// Open a fix of `k` for findings the human picked or released. The checkpoint's working copy must be clean.
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_chosen(
    p: &Project,
    pl: &Plan,
    k: &str,
    findings: Vec<Value>,
    ids: Vec<(u32, u32)>,
    content: &str,
    human: Option<(u64, &str, Option<u64>)>,
    opened_by: &str,
) -> R<()> {
    open_locked_by(
        p,
        pl,
        k,
        "review",
        vec![],
        (findings, ids),
        content,
        true,
        None,
        human,
        Some(opened_by),
    )?;
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
fn existing_test_files(p: &Project, f: &CkFixOpen, live: &Live) -> Vec<String> {
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
            let listed = live.allowed.iter().any(|g| util::path_matches(g, &path));
            if !listed && live.test_dirs.iter().any(|g| util::path_matches(g, &path)) {
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
        .filter(|f| matches!(f.phase.as_str(), "fix" | "review" | "scope"))
    {
        Some(f) => {
            let mut files: Vec<String> = Vec::new();
            let mut existing: Vec<String> = Vec::new();
            if matches!(f.phase.as_str(), "fix" | "scope") {
                // Read live: the records in force now and the grants (R-70).
                let live = match plan::load(p) {
                    Ok(pl) => live(p, &pl, &st, f),
                    Err(_) => frozen(f),
                };
                files.extend(live.allowed.iter().cloned());
                files.extend(live.test_dirs.iter().cloned());
                existing = existing_test_files(p, f, &live);
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
    let lv = match plan::load(p) {
        Ok(pl) => live(p, &pl, st, f),
        Err(_) => frozen(f),
    };
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
    if settled(f) {
        return json!({"role": "conductor", "action": "go-on-checkpoint-fix", "checkpoint": k,
            "note": crate::strings::t("accept.fix.settled.note", &[("k", k)]),
            "commands": [crate::strings::t("accept.fix.settled.command", &[("k", k)])],
            "abort_command": abort});
    }
    match f.phase.as_str() {
        "fix" | "scope" => {
            let d = json!({"role": "builder", "action": if f.source == "wave" { "fix-wave" } else { "fix-checkpoint" }, "checkpoint": k,
            "wave": f.wave,
            "source": f.source, "tier": p.policy.tiers.builder, "working_copy": wc,
            "brief": brief("builder"), "limits": lim,
            "may_change": lv.allowed.iter().chain(lv.test_dirs.iter()).collect::<Vec<_>>(),
            "granted": crate::fixscope::granted(st, k), "from_saved": f.from_saved,
            "failing_checks": f.failed.iter().filter_map(|x| x["check"].as_str()).collect::<Vec<_>>(),
            "findings": f.findings.iter().map(|x| json!({"title": x["title"], "severity": x["severity"], "where": x["where"]})).collect::<Vec<_>>(),
            "commands": [format!("bf brief {k} --role builder")], "abort_command": abort});
            d
        }
        "review" => {
            json!({"role": "reviewer", "action": if f.source == "wave" { "review-wave-fix" } else { "review-checkpoint-fix" }, "checkpoint": k,
            "wave": f.wave,
            "tier": p.policy.tiers.reviewer, "working_copy": wc, "brief": brief("reviewer"),
            "limits": lim, "round": f.rounds_log.len() + 1,
            "commands": [format!("bf brief {k} --role reviewer")], "abort_command": abort})
        }
        "low" => {
            let open = open_ids(f);
            let ids: Vec<String> = open.iter().map(|x| x.0.clone()).collect();
            json!({"role": "human", "action": "decide-low-findings", "checkpoint": k,
            "findings": open.iter().map(|(id, x)| json!({"id": id, "severity": x["severity"], "title": x["title"], "where": x["where"], "what_to_do": x["action"]})).collect::<Vec<_>>(),
            "note": crate::strings::t("accept.fix.low.note", &[("k", k), ("ids", &ids.join(","))]),
            "accept_command": crate::strings::t("accept.fix.low.accept_command", &[("k", k), ("ids", &ids.join(","))]),
            "release_command": crate::strings::t("accept.fix.low.release_command", &[("k", k)]),
            "abort_command": abort})
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

/// Whether the fix's review is settled: every finding of its latest round is accepted and the builder has not
/// worked since. The engine goes on to the checks and the end, with no further round (R-71).
pub(crate) fn settled(f: &CkFixOpen) -> bool {
    f.phase == "review" && f.rounds_log.last().map(|r| r["accepted"] == json!(true)) == Some(true)
}

/// The open findings of the fix's latest review round with their ids `f<round>.<number>`: those no accept covers.
pub(crate) fn open_ids(f: &CkFixOpen) -> Vec<(String, Value)> {
    let Some(h) = f
        .history
        .last()
        .filter(|h| h["round"].as_u64().unwrap_or(0) >= 1)
    else {
        return vec![];
    };
    let round = h["round"].as_u64().unwrap_or(0) as u32;
    h["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(n, _)| {
            !f.accepted
                .iter()
                .any(|a| a.round == round && a.index as usize == n + 1)
        })
        .map(|(n, x)| (format!("f{round}.{}", n + 1), x.clone()))
        .collect()
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
            &[
                "diff",
                "-z",
                "--name-status",
                "--no-renames",
                &before,
                "HEAD",
            ],
        )?;
        // NUL-separated pairs of a status and an unquoted path, whatever characters the name holds.
        let mut parts = d.split('\0');
        while let (Some(st), Some(path)) = (parts.next(), parts.next()) {
            if st.is_empty() {
                continue;
            }
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
pub(crate) fn within(f: &Live, path: &str, status: char) -> bool {
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

/// The repository's checkout, the working copy, the path inside it and the commit the fix started from, for a
/// path of the checkpoint's files.
pub(crate) fn locate(
    p: &Project,
    pl: &Plan,
    f: &CkFixOpen,
    path: &str,
) -> R<Option<(PathBuf, PathBuf, String, String)>> {
    let several = several_repos(p, pl, &f.checkpoint);
    for r in run::checkpoint_repos(p, pl, &f.checkpoint)? {
        let (name, wt) = (r.name.clone(), run::checkpoint_wt(p, pl, &f.checkpoint, r)?);
        let rel = if several {
            match path.strip_prefix(&format!("{name}/")) {
                Some(r) => r,
                None => continue,
            }
        } else {
            path
        };
        let before = f
            .before
            .iter()
            .find(|(r, _)| *r == name)
            .map(|(_, h)| h.clone())
            .unwrap_or_default();
        if before.is_empty() {
            return Ok(None);
        }
        return Ok(Some((r.path.clone(), wt, rel.to_string(), before)));
    }
    Ok(None)
}

fn fmt_path(p: &Project) -> PathBuf {
    p.state_dir().join("ckfix_fmt.json")
}

/// The files of the open fix of `k` accepted as format only, with the hash of the content accepted.
pub fn fmt_load(p: &Project, k: &str, open_seq: u64) -> BTreeMap<String, String> {
    let v: Value = fs::read_to_string(fmt_path(p))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let e = &v[k];
    if e["open_seq"].as_u64() != Some(open_seq) {
        return BTreeMap::new();
    }
    e["files"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(a, b)| (a.clone(), b.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn fmt_save(p: &Project, k: &str, open_seq: u64, files: &BTreeMap<String, String>) -> R<()> {
    let mut v: Value = fs::read_to_string(fmt_path(p))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &Value| v.is_object())
        .unwrap_or_else(|| json!({}));
    v[k] = json!({"open_seq": open_seq, "files": files});
    let _ = fs::create_dir_all(p.state_dir());
    fs::write(fmt_path(p), v.to_string()).map_err(|e| {
        crate::refusal::err(crate::strings::t(
            "cli.ckfix.format.save_failed",
            &[("why", &e.to_string())],
        ))
    })?;
    Ok(())
}

/// The kind of a check from its command: `format` for `cargo fmt` and `rustfmt`, `lint` for `cargo clippy`,
/// and nothing for any other command or for one that chains several.
fn command_kind(command: &str) -> &'static str {
    if command.contains("&&") || command.contains(';') || command.contains('|') {
        return "";
    }
    let w: Vec<&str> = command.split_whitespace().collect();
    let sub = match w.as_slice() {
        ["cargo", x, ..] if x.starts_with('+') => w.get(2).copied(),
        ["cargo", x, ..] => Some(*x),
        [x, ..] if *x == "rustfmt" || *x == "cargo-fmt" => Some("fmt"),
        [x, ..] if *x == "cargo-clippy" => Some("clippy"),
        _ => None,
    };
    match sub {
        Some("fmt") => "format",
        Some("clippy") => "lint",
        _ => "",
    }
}

/// Whether every failed check is a format or a lint check.
fn format_lint_only(failed: &[Value]) -> bool {
    !failed.is_empty()
        && failed
            .iter()
            .all(|x| !command_kind(x["command"].as_str().unwrap_or("")).is_empty())
}

/// A path a failed output names, as a path inside the checkpoint's working copy folder; None for one that
/// is not a file there, climbs out of the folder or lies in a folder of the engine or of git.
fn named_path(top: &Path, raw: &str, keep: &dyn Fn(&str) -> bool) -> Option<String> {
    let raw = raw.trim().trim_end_matches(':');
    let mut rel = PathBuf::from(raw);
    if rel.is_absolute() {
        let canon = fs::canonicalize(top).unwrap_or_else(|_| top.to_path_buf());
        rel = rel
            .strip_prefix(top)
            .or_else(|_| rel.strip_prefix(&canon))
            .ok()?
            .to_path_buf();
    }
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        match c {
            std::path::Component::Normal(x) => parts.push(x.to_string_lossy().to_string()),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    let first = parts.first()?.as_str();
    if matches!(first, ".git" | "factory" | ".bf") {
        return None;
    }
    let path = parts.join("/");
    (keep(&path) && top.join(&path).is_file()).then_some(path)
}

/// The places a grant may reach, applied to the paths a failed output names: the rule a unit's file request
/// meets (`scope::hard_rule`), read for the checkpoint's own working copy folder (F-282).
fn placed<'a>(
    p: &'a Project,
    pl: &'a Plan,
    st: &'a State,
    k: &'a str,
) -> impl Fn(&str) -> bool + 'a {
    move |path| crate::scope::hard_rule(p, pl, st, k, path).is_none()
}

/// The files of the failed format and lint outputs, each with the source lines the output names (`-->` of
/// clippy and the numbered lines under it); rustfmt's `Diff in <path>` names a file with no line.
fn named_in(
    top: &Path,
    failed: &[Value],
    keep: &dyn Fn(&str) -> bool,
) -> BTreeMap<String, BTreeSet<u32>> {
    let mut out: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for x in failed {
        let mut cur: Option<String> = None;
        for l in x["tail"].as_str().unwrap_or("").lines() {
            let t = l.trim_start();
            if let Some(rest) = t.strip_prefix("Diff in ") {
                let rest = match rest.split_once(" at line ") {
                    Some((a, _)) => a,
                    None => {
                        let r = rest.trim_end_matches(':');
                        match r.rsplit_once(':') {
                            Some((a, n)) if n.chars().all(|c| c.is_ascii_digit()) => a,
                            _ => r,
                        }
                    }
                };
                if let Some(path) = named_path(top, rest, keep) {
                    out.entry(path).or_default();
                }
            } else if let Some(rest) = t.strip_prefix("--> ") {
                let mut it = rest.trim().rsplitn(3, ':');
                let (col, line, path) = (it.next(), it.next(), it.next());
                cur = None;
                if let (Some(_), Some(line), Some(path)) = (col, line, path) {
                    if let (Some(path), Ok(n)) = (named_path(top, path, keep), line.parse::<u32>())
                    {
                        out.entry(path.clone()).or_default().insert(n);
                        cur = Some(path);
                    }
                }
            } else if let Some(path) = &cur {
                let num: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !num.is_empty() && t[num.len()..].trim_start().starts_with('|') {
                    if let Ok(n) = num.parse::<u32>() {
                        out.entry(path.clone()).or_default().insert(n);
                    }
                }
            }
        }
    }
    out
}

/// One hunk of a diff: the old side's first line and length, then the new side's.
type Hunk = (u32, u32, u32, u32);

/// The hunks of each file's diff from `before` to the head, as `git diff -U0` gives them.
fn changed_hunks(p: &Project, pl: &Plan, f: &CkFixOpen) -> R<BTreeMap<String, Vec<Hunk>>> {
    let several = several_repos(p, pl, &f.checkpoint);
    let mut out: BTreeMap<String, Vec<Hunk>> = BTreeMap::new();
    for (name, wt) in wts(p, pl, &f.checkpoint)? {
        let Some((_, before)) = f.before.iter().find(|(r, _)| *r == name) else {
            continue;
        };
        let d = gitx::must(
            &wt,
            &[
                "-c",
                "core.quotePath=false",
                "diff",
                "-U0",
                "--no-renames",
                before,
                "HEAD",
            ],
        )?;
        let mut cur = String::new();
        for l in d.lines() {
            if let Some(rest) = l.strip_prefix("diff --git a/") {
                // Both sides name one path (no renames); a binary file has no `+++` line.
                let n = rest.len().saturating_sub(3) / 2;
                let x = rest.get(..n).unwrap_or(rest);
                cur = if several {
                    format!("{name}/{x}")
                } else {
                    x.to_string()
                };
                out.entry(cur.clone()).or_default();
            } else if l.starts_with("diff --git ") {
                // A name git still quotes (a quote, a backslash, a newline): no hunk is credited to another file.
                cur = String::new();
            } else if l.starts_with("old mode ")
                || l.starts_with("new mode ")
                || l.starts_with("new file mode ")
                || l.starts_with("deleted file mode ")
            {
                // A mode change is never a rewrite of named lines: a hunk no lint line can account for.
                out.entry(cur.clone()).or_default().push((0, 0, 0, 0));
            } else if let Some(rest) = l.strip_prefix("+++ ") {
                cur = match rest.strip_prefix("b/") {
                    Some(x) if several => format!("{name}/{x}"),
                    Some(x) => x.to_string(),
                    None => String::new(), // /dev/null: a deleted file
                };
                out.entry(cur.clone()).or_default();
            } else if let Some(h) = hunk_of(l) {
                out.entry(cur.clone()).or_default().push(h);
            }
        }
    }
    Ok(out)
}

/// The hunk a `@@ -a,b +c,d @@` line gives; a side with no length is one line long.
fn hunk_of(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let mut sides = rest.split(' ');
    let old = sides.next()?;
    let new = sides.next()?.strip_prefix('+')?;
    let one = |x: &str| -> Option<(u32, u32)> {
        let (a, b) = x.split_once(',').unwrap_or((x, "1"));
        Some((a.parse().ok()?, b.parse().ok()?))
    };
    let ((a, b), (c, d)) = (one(old)?, one(new)?);
    Some((a, b, c, d))
}

/// Whether a hunk only rewrites lines the lint output names: every old line is named, and the new side is
/// no longer than the old, so it adds nothing beside them. A pure insertion is never accounted for.
fn hunk_accounted(lines: &BTreeSet<u32>, (a, b, _, d): Hunk) -> bool {
    b > 0 && d <= b && (a..a + b).all(|n| lines.contains(&n))
}

/// Whether every change of the fix is accounted for: each file is proved format only, or is named by the
/// lint output with each old line it changes among the lines named and nothing added beside them.
fn lint_accounts(
    p: &Project,
    pl: &Plan,
    st: &State,
    f: &CkFixOpen,
    by_format: &BTreeSet<String>,
) -> R<bool> {
    let top = p.work_dir().join(&f.checkpoint);
    let named = named_in(&top, &f.origin_failed, &placed(p, pl, st, &f.checkpoint));
    let diff = changed_hunks(p, pl, f)?;
    let files = changed(p, pl, f)?;
    if files.is_empty() {
        return Ok(false);
    }
    // Walk every file the fix changed: one with no text hunk (a binary file, a mode-only change, a
    // deletion) or with a mode line cannot be accounted for by lint output and goes to review.
    for (path, _) in files {
        let hunks = diff.get(&path).cloned().unwrap_or_default();
        // A mode change (the (0,0,0,0) marker) is never format only: proved or not, it goes to review.
        if hunks.contains(&(0, 0, 0, 0)) {
            return Ok(false);
        }
        if by_format.contains(&path) {
            continue;
        }
        let Some(lines) = named.get(&path) else {
            return Ok(false);
        };
        if hunks.is_empty() || !hunks.iter().all(|h| hunk_accounted(lines, *h)) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The checkpoint's command checks that are switched on: the ones that ran again on the fix.
fn ran_again(p: &Project, pl: &Plan, k: &str) -> R<Vec<String>> {
    let switched_off = run::skipped_k_checks(p, pl, k)?;
    Ok(pl
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
        .collect())
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
    let Some(mut f) = st.ck_fix.clone().filter(|f| f.checkpoint == k) else {
        return open_from_review(p, &st, &pl, k, result, role, findings, token, via_round);
    };
    // A request for files left open in an old log holds nothing now: the builder goes on.
    if f.phase == "scope" {
        f.phase = "fix".into();
    }
    // The review is settled (every finding accepted): any report of `built` goes on to the checks and the end,
    // with no further round. A stop between the accept and the checks leaves the fix here (R-71).
    if settled(&f) {
        if result != "built" {
            return Err(refuse_owned(
                crate::strings::t("accept.fix.settled.what", &[("k", k)]),
                crate::strings::t("accept.fix.settled.because", &[("result", result)]),
                vec![crate::strings::t("accept.fix.settled.command", &[("k", k)])],
                crate::strings::t("accept.fix.settled.check", &[]),
            ));
        }
        return go_on(p, &pl, &f);
    }
    let expected = match f.phase.as_str() {
        "fix" => "builder",
        "review" => "reviewer",
        "low" => {
            return Err(refuse_owned(
                crate::strings::t("accept.fix.low.report.what", &[("k", k)]),
                crate::strings::t("accept.fix.low.report.because", &[]),
                vec![crate::strings::t("accept.fix.low.report.step", &[("k", k)])],
                crate::strings::t("accept.fix.low.report.check", &[]),
            ));
        }
        "gap" => {
            return Err(refuse(
                format!("report on the fix of {k}"),
                "the fix needs what the plan never gave it: the human decides by amendment",
                &[
                    "`bf next` names what is missing.",
                    "`bf checkpoint fix <K> --abort` puts the checkpoint back as it was.",
                ],
                "`bf status` shows the fix back at work or gone.",
            ));
        }
        _ => {
            return Err(refuse(
                format!("report on the fix of {k}"),
                "the fix is parked: the human decides",
                &[
                    "The human releases it (`bf checkpoint fix <K> --release --quote \"...\"`) or the conductor aborts it (`--abort`).",
                ],
                "`bf status` shows the fix back at work.",
            ));
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
            format!(
                "a report needs the token of the {expected}'s directive; none, or the wrong one, was given"
            ),
            &["`bf next --json` shows the directive with its `token`; pass it with `--token`."],
            "`bf status` shows the fix in its new phase.",
        ));
    }
    match (f.phase.as_str(), result) {
        // The attempts limit bounds it: the report that reaches the limit parks the fix as any blocked report does.
        ("fix", "blocked") if !f.origin_failed.is_empty() && f.attempts + 1 < p.policy.limits.max_attempts && {
            // D-110: a block over checks that are all marked `unreachable` parks nothing.
            let receipts = evidence::load_receipts(p)?;
            let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
            f.origin_failed.iter().all(|o| {
                evidence::standing_outcome(&receipts, s(o, "check"), &content)
                    .and_then(|r| r.outcome())
                    == Some("unreachable")
            })
        } =>
        {
            let checks: Vec<String> = f
                .origin_failed
                .iter()
                .map(|o| s(o, "check").to_string())
                .collect();
            events::append(
                p,
                "checkpoint.fix.blocked",
                json!({"checkpoint": k, "note": note, "checks": checks}),
            )?;
            Ok(json!({"checkpoint": k, "phase": f.phase, "parked": false, "due_again": checks}))
        }
        ("fix", "blocked") => {
            let n = f.attempts + 1;
            let reason = if note.is_empty() {
                "the builder is blocked"
            } else {
                note
            };
            events::append(
                p,
                "checkpoint.fix.park",
                json!({"checkpoint": k, "reason": reason, "attempts": n}),
            )?;
            write_overlay(p)?;
            p.write_guard()?;
            Ok(json!({"checkpoint": k, "phase": "parked"}))
        }
        ("fix", "gap") => {
            // A gap names each file it needs, and a file already in the fix's list is refused (R-70).
            let what = crate::strings::t("fix.scope.gapfiles.what", &[("k", k)]);
            let check = crate::strings::t("fix.scope.gapfiles.none_check", &[]);
            let named = crate::fixscope::named_files(note);
            if named.is_empty() {
                return Err(refuse_owned(
                    what,
                    crate::strings::t("fix.scope.gapfiles.none", &[]),
                    vec![
                        crate::strings::t("fix.scope.gapfiles.none_step", &[]),
                        crate::strings::t("fix.scope.gapfiles.none_ask", &[("k", k)]),
                    ],
                    check,
                ));
            }
            let live = live(p, &pl, &st, &f);
            if let Some(path) = named
                .iter()
                .find(|n| crate::fixscope::in_list(p, k, &live, n))
            {
                return Err(refuse_owned(
                    what,
                    crate::strings::t("fix.scope.gapfiles.listed", &[("path", path)]),
                    vec![
                        crate::strings::t("fix.scope.gapfiles.listed_step", &[("path", path)]),
                        crate::strings::t("fix.scope.gapfiles.listed_ask", &[("k", k)]),
                    ],
                    check,
                ));
            }
            let missing: Vec<String> = note
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            plan_gap(
                p,
                k,
                &missing,
                "the builder reports a need the plan never gave this checkpoint",
            )
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
                        &[
                            "Commit the fix in the checkpoint's working copy (`git add` the files, `git commit`), then report again.",
                        ],
                        "`bf status` shows the fix in review.",
                    ));
                }
            }
            let files = changed(p, &pl, &f)?;
            // R-58: a file accepted as format only must still equal the content accepted.
            for (path, sha) in fmt_load(p, k, f.open_seq) {
                if let Some((_, wt, rel, _)) = locate(p, &pl, &f, &path)? {
                    let now = fs::read(wt.join(&rel)).ok().map(|b| util::sha256_hex(&b));
                    let changed_now = files.iter().any(|x| x.0 == path);
                    if changed_now && now.as_deref() != Some(sha.as_str()) {
                        return Err(refuse_owned(
                            crate::strings::t("cli.ckfix.format.stale.what", &[("k", k)]),
                            crate::strings::t("cli.ckfix.format.stale.because", &[("path", &path)]),
                            vec![crate::strings::t(
                                "cli.ckfix.format.stale.step",
                                &[("path", &path)],
                            )],
                            crate::strings::t("cli.ckfix.format.stale.check", &[]),
                        ));
                    }
                }
            }
            let live = live(p, &pl, &st, &f);
            let outside_any = files.iter().any(|(path, st)| !within(&live, path, *st));
            let skip_wanted = f.source == "check" && format_lint_only(&f.origin_failed);
            let mut proven: BTreeMap<String, Value> = BTreeMap::new();
            // A file outside the lists is the reviewer's to judge (D-115); only a format-only change is proven here.
            if outside_any || skip_wanted {
                for (path, status) in &files {
                    if *status != 'M' || (within(&live, path, *status) && !skip_wanted) {
                        continue;
                    }
                    let Some((repo, wt, rel, before)) = locate(p, &pl, &f, path)? else {
                        continue;
                    };
                    // A file the rules below refuse or name is no format-only change.
                    if crate::scope::hard_rule(p, &pl, &st, k, path).is_some() {
                        continue;
                    }
                    let rel = crate::scope::normalize_path(&rel).unwrap_or(rel);
                    if let Ok(v) = crate::scope::fmtproof::prove_in_fix(p, &repo, &wt, &before, &rel) {
                        proven.insert(path.clone(), v);
                    }
                }
            }
            // A file outside the lists is the reviewer's to judge, except one no change may touch: a place the
            // guard protects, a link, a look-alike name, a file outside the repository. Those are refused.
            for (path, status) in &files {
                if within(&live, path, *status) {
                    continue;
                }
                if let Some(rule) = never_allowed(p, &pl, &st, k, path) {
                    return Err(refuse_owned(
                        crate::strings::t("ckfix_recommended.never.what", &[("k", k)]),
                        crate::strings::t(
                            "ckfix_recommended.never.because",
                            &[("path", &shown(path)), ("rule", &rule)],
                        ),
                        vec![crate::strings::t(
                            "ckfix_recommended.never.step",
                            &[("path", &shown(path))],
                        )],
                        crate::strings::t("ckfix_recommended.never.check", &[]),
                    ));
                }
            }
            if files.is_empty() {
                return Err(refuse_owned(
                    crate::strings::t("fix.scope.no_change.what", &[("k", k)]),
                    crate::strings::t("fix.scope.no_change.because", &[]),
                    vec![
                        crate::strings::t("fix.scope.no_change.step", &[]),
                        crate::strings::t("fix.scope.brief.no_change_gap", &[("k", k)]),
                    ],
                    crate::strings::t("fix.scope.no_change.check", &[]),
                ));
            }
            // R-32: a probe that is not yet a permanent test is refused before any check runs again.
            let absent = probe_tests_missing(p, &pl, &f)?;
            if !absent.is_empty() {
                let mut steps = absent.clone();
                steps.push(crate::strings::t("cli.ckfix.probe_missing.step", &[]));
                return Err(refuse_owned(
                    crate::strings::t("cli.ckfix.probe_missing.what", &[("k", k)]),
                    crate::strings::t(
                        "cli.ckfix.probe_missing.because",
                        &[("n", &absent.len().to_string())],
                    ),
                    steps,
                    crate::strings::t("cli.ckfix.probe_missing.check", &[]),
                ));
            }
            crate::fixscope::landing(p, &pl, &f)?;
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
                    crate::strings::t("ckfix_recommended.fail.repair", &[])
                });
                return Err(refuse_owned(
                    format!("report the fix of {k}"),
                    format!("the checkpoint's checks still fail: {}", failed.len()),
                    steps,
                    "`bf status` shows the fix in review.",
                ));
            }
            let accepted: BTreeMap<String, String> = files
                .iter()
                .filter(|(path, status)| !within(&live, path, *status))
                .filter_map(|(path, _)| {
                    let v = proven.get(path)?;
                    Some((path.clone(), s(&v["format"], "result_sha256").to_string()))
                })
                .collect();
            fmt_save(p, k, f.open_seq, &accepted)?;
            events::append(
                p,
                "checkpoint.fix.built",
                json!({"checkpoint": k, "files": files.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
                    "note": note}),
            )?;
            write_overlay(p)?;
            p.write_guard()?;
            // R-70: a fix of only format or lint checks whose every change the failed output names needs
            // no review round: the checks passed above, and the checkpoint's others ran again with them.
            let by_format: BTreeSet<String> = proven.keys().cloned().collect();
            if skip_wanted && lint_accounts(p, &pl, &st, &f, &by_format)? {
                let again = ran_again(p, &pl, k)?;
                let mut v = finish(p, &pl, &f, again)?;
                v["reviewed"] = json!(false);
                v["note"] = json!(crate::strings::t("cli.ckfix.lint.finished", &[("k", k)]));
                return Ok(v);
            }
            Ok(json!({"checkpoint": k, "phase": "review"}))
        }
        ("review", "findings") => {
            let listed = finding_list(findings);
            if listed.is_empty() {
                return Err(refuse(
                    format!("report findings on the fix of {k}"),
                    "no finding was given",
                    &[
                        "Give the findings (`--findings -` with a JSON list), or report `built` when the fix is right.",
                    ],
                    "`bf status` shows the fix in its new phase.",
                ));
            }
            // R-71: a fix review that files only low findings asks the human to accept or fix them. A low
            // finding the reviewer marks small (a test or a strings key inside the fix's own files) goes
            // back to the builder with no human step.
            let only_low = listed.iter().all(|x| s(x, "severity") == "low");
            let all_small = listed.iter().all(|x| x["small"].as_bool() == Some(true));
            let over = f.rounds + 1 > p.policy.limits.max_rounds;
            let to = if only_low && (!all_small || over) {
                "low"
            } else if !over {
                "fix"
            } else {
                "parked"
            };
            record_round(
                p,
                &pl,
                &f,
                "findings",
                to,
                &listed,
                match to {
                    "low" => crate::strings::t("accept.fix.low.reason", &[]),
                    "parked" => crate::strings::t(
                        "accept.fix.over_rounds",
                        &[("max", &p.policy.limits.max_rounds.to_string())],
                    ),
                    _ => String::new(),
                },
            )?;
            Ok(json!({"checkpoint": k, "phase": to}))
        }
        ("review", "built") => {
            record_round(p, &pl, &f, "clean", "done", &[], String::new())?;
            go_on(p, &pl, &f)
        }
        (ph, r) => Err(refuse(
            format!("report {r} on the fix of {k}"),
            format!("a `{r}` report does not fit a fix in `{ph}`"),
            &[
                "The builder reports built, blocked or gap; the reviewer reports findings or built (clean).",
            ],
            "`bf status` shows the fix in its new phase.",
        )),
    }
}

/// The fix's review is settled (clean, or every finding accepted): the checks run again on the new content,
/// through the engine, and the fix is finished. A wave fix runs the wave checks again, and that run is the
/// wave's new result.
pub(crate) fn go_on(p: &Project, pl: &Plan, f: &CkFixOpen) -> R<Value> {
    let k = f.checkpoint.as_str();
    if f.source == "wave" {
        let failed = crate::wavecheck::rerun(p, pl, k, f.wave, true)?;
        if !failed.is_empty() {
            let to = attempt(p, f, &failed)?;
            return Ok(
                json!({"checkpoint": k, "phase": if to == "parked" { "parked" } else { "fix" },
                "failed_checks": failed}),
            );
        }
        let again: Vec<String> = crate::wavecheck::records(pl)
            .iter()
            .map(|c| s(c, "id").to_string())
            .collect();
        return finish(p, pl, f, again);
    }
    let failed = run::run_k_checks(p, pl, k)?;
    if !failed.is_empty() {
        let to = attempt(p, f, &failed)?;
        return Ok(
            json!({"checkpoint": k, "phase": if to == "parked" { "parked" } else { "fix" },
            "failed_checks": failed}),
        );
    }
    // Only the command checks that are switched on ran again; agent-judged ones did not.
    let ran_again = ran_again(p, pl, k)?;
    finish(p, pl, f, ran_again)
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
            &[
                "Units are reported on with `bf report <U>`.",
                "A checkpoint is reported on only by its reviewer (findings), or while a sync or a fix of it is open.",
            ],
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
        return Err(bad(
            "no fix is open for it, and this is not a review of a checkpoint whose units have landed",
        ));
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
            &[
                "Give the findings (`--findings -` with a JSON list of `{title, severity, where, action}`).",
            ],
            "`bf status` shows the fix at work.",
        ));
    }
    let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
    // The findings of a review round carry the round they were filed in, and their numbers inside it.
    let ids: Vec<(u32, u32)> = if via_round {
        crate::ckreview::rounds(p, k)?
            .iter()
            .rev()
            .find(|r| r.result.as_deref() == Some("findings"))
            .map(|r| (1..=listed.len() as u32).map(|i| (r.round, i)).collect())
            .unwrap_or_default()
    } else {
        vec![]
    };
    open_locked_by(
        p,
        pl,
        k,
        "review",
        vec![],
        (listed, ids),
        &content,
        true,
        None,
        None,
        Some("report"),
    )?;
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
    // R-58: a file accepted as format only carries its mark in a field of its own, apart from the units.
    let marks: serde_json::Map<String, Value> = fmt_load(p, &f.checkpoint, f.open_seq)
        .keys()
        .filter(|path| files.iter().any(|x| &x.0 == *path))
        .map(|path| (path.clone(), json!(crate::scope::fmtproof::MARK_FORMAT)))
        .collect();
    units.sort();
    let names: Vec<String> = files.iter().map(|x| x.0.clone()).collect();
    events::append(
        p,
        "checkpoint.fix.done",
        json!({"checkpoint": f.checkpoint, "files": names, "units": units, "by_file": by_file, "marks": marks}),
    )?;
    write_overlay(p)?;
    p.write_guard()?;
    Ok(
        json!({"checkpoint": f.checkpoint, "phase": "done", "files": names, "units": units,
        "checks_run_again": rerun}),
    )
}

/// `bf checkpoint fix <K> --abort` and `--release --quote "..."`.
pub fn command(
    p: &Project,
    k: &str,
    quote: &str,
    abort: bool,
    release: bool,
    open: bool,
) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("fix {k}"))?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    if open {
        return reopen(p, &st, k);
    }
    let Some(f) = st.ck_fix.clone().filter(|f| f.checkpoint == k) else {
        return Err(refuse_owned(
            crate::strings::t("cli.ckfix.settle.what", &[("k", k)]),
            crate::strings::t("cli.ckfix.settle.none_open", &[]),
            vec![
                crate::strings::t("cli.ckfix.settle.step_status", &[]),
                crate::strings::t("cli.ckfix.open.step_open", &[]),
            ],
            crate::strings::t("cli.ckfix.settle.check", &[]),
        ));
    };
    let pl = plan::load(p)?;
    if abort {
        // The work is saved before anything is reset; a failure to save resets nothing.
        let saved = crate::fixscope::save_work(p, &pl, &f)?;
        for (name, wt) in wts(p, &pl, k)? {
            if let Some((_, before)) = f.before.iter().find(|(r, _)| *r == name) {
                gitx::must(&wt, &["reset", "--hard", "-q", before])?;
                gitx::must(&wt, &["clean", "-fdq"])?;
            }
        }
        events::append(
            p,
            "checkpoint.fix.abort",
            json!({"checkpoint": k, "saved": saved}),
        )?;
        write_overlay(p)?;
        p.write_guard()?;
        return Ok(
            json!({"checkpoint": k, "aborted": true, "saved": saved, "message": crate::strings::t("fix.scope.abort.message", &[])}),
        );
    }
    if release {
        if !matches!(f.phase.as_str(), "parked" | "low") {
            return Err(refuse_owned(
                crate::strings::t("accept.fix.release.what", &[("k", k)]),
                crate::strings::t("accept.fix.release.because", &[("phase", &f.phase)]),
                vec![crate::strings::t("accept.fix.step_next", &[])],
                crate::strings::t("accept.fix.release.check", &[]),
            ));
        }
        run::observe_holds(p)?;
        let st = events::require_chain(p)?;
        let bound = match events::decision_bound(&st, k, f.park_seq, u64::MAX) {
            Ok(b) => b,
            Err(h) => {
                return Err(crate::ckreview::hold_refusal(
                    &crate::strings::t("accept.fix.release.hold_what", &[]),
                    k,
                    &h,
                ))
            }
        };
        let (turn, _) = events::pick_turn(&st, bound, events::Decide::Approve, quote)?;
        events::append(
            p,
            "checkpoint.fix.release",
            json!({"checkpoint": k, "turn": turn, "quote": quote.trim()}),
        )?;
        write_overlay(p)?;
        p.write_guard()?;
        return Ok(json!({"checkpoint": k, "released": true}));
    }
    Err(refuse_owned(
        crate::strings::t("cli.ckfix.needs.what", &[("k", k)]),
        crate::strings::t("cli.ckfix.open.needs", &[]),
        vec![crate::strings::t("cli.ckfix.open.needs_step", &[])],
        crate::strings::t("cli.ckfix.needs.check", &[]),
    ))
}

/// `bf checkpoint fix <K> --open`: a fix for the failed checks of the checkpoint's current content, when
/// none is open. The engine lock is held by the caller.
fn reopen(p: &Project, st: &State, k: &str) -> R<Value> {
    let what = crate::strings::t("cli.ckfix.open.what", &[("k", k)]);
    let no = |because: String, step: String| {
        refuse_owned(
            what.clone(),
            because,
            vec![step],
            crate::strings::t("cli.ckfix.open.worked", &[]),
        )
    };
    let step = crate::strings::t("cli.ckfix.open.step_status", &[]);
    if st.ck_fix.is_some() {
        return Err(no(
            crate::strings::t("cli.ckfix.open.fix_open", &[("k", k)]),
            step,
        ));
    }
    if st.sync_open.is_some() {
        return Err(no(crate::strings::t("cli.ckfix.open.sync_open", &[]), step));
    }
    let pl = plan::load(p)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(no(
            crate::strings::t("cli.ckfix.open.no_checkpoint", &[]),
            step,
        ));
    };
    let after = str_list(kr, "after");
    if st.closed.contains_key(k)
        || after.is_empty()
        || !after.iter().all(|u| st.unit_state(u) == "landed")
    {
        return Err(no(
            crate::strings::t("cli.ckfix.open.not_landed", &[]),
            step,
        ));
    }
    let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
    let receipts = evidence::load_receipts(p)?;
    let (failed, unreachable) = failing_checks(&pl, st, &receipts, kr, &content);
    if unreachable {
        return Err(no(
            crate::strings::t("cli.ckfix.open.unreachable", &[("k", k)]),
            step,
        ));
    }
    if failed.is_empty() || content.is_empty() {
        return Err(no(
            crate::strings::t("cli.ckfix.open.none_failed", &[("k", k)]),
            step,
        ));
    }
    let ids: Vec<String> = failed
        .iter()
        .filter_map(|x| x["check"].as_str().map(String::from))
        .collect();
    open_locked(
        p,
        &pl,
        k,
        "check",
        failed,
        (vec![], vec![]),
        &content,
        true,
        None,
    )?;
    Ok(json!({"checkpoint": k, "opened": true, "failed_checks": ids}))
}

/// The files of a finished fix marked format only, from its `marks`: the files in order.
fn marked_files(marks: &Value) -> Vec<(String, String)> {
    marks
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(f, v)| (f.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// The checkpoint page's line for a finished fix: it says whether a review round happened, taken from the
/// fix's rounds, and ends with the files accepted as format only.
pub fn done_page(rounds_log: &[Value], marks: &Value) -> String {
    let clean = rounds_log
        .last()
        .is_some_and(|r| r["result"].as_str() == Some("clean"));
    let key = if !clean {
        "cli.ckfix.done.no_review"
    } else {
        "cli.ckfix.done.reviewed"
    };
    format!("{}{}", crate::strings::t(key, &[]), marks_page(marks))
}

/// The sentence of a fix's page row that names its files accepted as format only, with a leading space;
/// empty when it has none (R-58).
pub fn marks_page(marks: &Value) -> String {
    let files: Vec<String> = marked_files(marks)
        .into_iter()
        .map(|(f, _)| format!("`{f}`"))
        .collect();
    if files.is_empty() {
        return String::new();
    }
    format!(
        " {}",
        crate::strings::t("cli.ckfix.format.page", &[("files", &files.join(", "))])
    )
}

/// The checkpoint reviewer's section on the files of finished fixes of `k` accepted as format only, each
/// with its mark; empty when no fix has one (R-58).
pub fn marks_md(st: &State, k: &str) -> String {
    let mut rows = String::new();
    for d in st.ck_fixes.iter().filter(|d| d.checkpoint == k) {
        let marks = marked_files(&d.marks);
        if marks.is_empty() {
            continue;
        }
        for path in &d.files {
            let mark = marks
                .iter()
                .find(|(f, _)| f == path)
                .map(|(_, m)| crate::strings::t("cli.ckfix.format.checkpoint_mark", &[("mark", m)]))
                .unwrap_or_default();
            rows.push_str(&crate::strings::t(
                "cli.ckfix.format.checkpoint_row",
                &[("fix", &d.id), ("path", path), ("mark", &mark)],
            ));
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    crate::strings::t("cli.ckfix.format.checkpoint", &[("rows", &rows)])
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
            "units": d.units, "by_file": d.by_file, "marks": d.marks, "review_rounds": d.rounds_log,
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
    if role == "builder" && !f.blocked_notes.is_empty() {
        md.push_str(&format!(
            "## {}\n\n",
            crate::strings::t("load.brief.blocked_title", &[])
        ));
        for n in &f.blocked_notes {
            md.push_str(&format!("- {n}\n"));
        }
        md.push_str(&format!(
            "\n{}\n\n",
            crate::strings::t("load.brief.blocked_rule", &[])
        ));
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
    let accepted = fmt_load(p, k, f.open_seq);
    if role == "reviewer" && !accepted.is_empty() {
        md.push_str(&crate::strings::t(
            "cli.ckfix.format.reviewer",
            &[("mark", crate::scope::fmtproof::MARK_FORMAT)],
        ));
        for path in accepted.keys() {
            md.push_str(&format!(
                "- `{path}` ({})\n",
                crate::scope::fmtproof::MARK_FORMAT
            ));
        }
        md.push('\n');
    }
    if role == "builder" {
        let st = events::replay(p)?;
        let lv = live(p, &pl, &st, f);
        md.push_str(&format!(
            "## {}\n\n{}\n\n",
            crate::strings::t("ckfix_recommended.brief.list_title", &[]),
            crate::strings::t("ckfix_recommended.brief.list_intro", &[])
        ));
        for g in lv.allowed.iter().take(80) {
            md.push_str(&format!("- `{g}`\n"));
        }
        for g in &lv.test_dirs {
            md.push_str(&format!(
                "- `{g}` {}\n",
                crate::strings::t("fix.scope.brief.new_tests", &[])
            ));
        }
        md.push_str(&builder_extras(p, &st, f));
        md.push('\n');
        md.push_str(&outside_section(p, &pl, f, role)?);
        md.push_str(&format!(
            "\n## {}\n\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n",
            crate::strings::t("fix.scope.brief.rules_title", &[]),
            crate::strings::t("fix.scope.brief.rule_weaken", &[]),
            crate::strings::t("fix.scope.brief.rule_commit", &[]),
            crate::strings::t(
                "fix.scope.brief.rule_run",
                &[("wc", &wc.display().to_string())]
            ),
            crate::strings::t("fix.scope.brief.rule_report", &[("k", k)]),
            crate::strings::t("ckfix_recommended.brief.outside_rule", &[]),
            crate::strings::t("fix.scope.brief.gap", &[("k", k)]),
            crate::strings::t("fix.scope.brief.rule_blocked", &[("k", k)])
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
        md.push_str(&outside_section(p, &pl, f, role)?);
        md.push_str(&claim_section(p, f));
        md.push_str(&format!(
            "{}\n",
            crate::strings::t("ckfix_recommended.brief.judge", &[("k", k)])
        ));
    }
    if role != "builder" {
        md.push('\n');
    }
    md.push_str(&format!(
        "- {}\n",
        crate::strings::t("cli.evidence.brief.rule.checks_engine", &[])
    ));
    if role != "builder" {
        md.push_str(&format!(
            "- {}\n",
            crate::strings::t("cli.evidence.brief.rule.reviewer_receipts", &[])
        ));
    }
    let path = p.briefs_dir().join(format!("{k}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// The builder's brief on saved work (R-70).
fn builder_extras(p: &Project, st: &State, f: &CkFixOpen) -> String {
    use crate::strings::t;
    let mut md = String::new();
    if !f.from_saved.is_empty() {
        let note = crate::fixscope::reopened(p, &f.from_saved);
        md.push_str(&format!(
            "\n## {}\n\n{}\n\n",
            t("fix.scope.brief.saved_title", &[]),
            t("fix.scope.brief.saved_intro", &[("saved", &f.from_saved)])
        ));
        for a in note["applied"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
        {
            md.push_str(&format!(
                "{}\n",
                t("fix.scope.brief.saved_applied", &[("path", a)])
            ));
        }
        for n in note["not_applied"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
        {
            md.push_str(&format!(
                "{}\n",
                t("fix.scope.brief.saved_not", &[("why", n)])
            ));
        }
    }
    let _ = st;
    md
}

/// A path as the brief prints it inside a code span: a backtick, a backslash and a control character are
/// written as escapes, so no name can close the span or start a line of its own.
fn shown(path: &str) -> String {
    let mut o = String::new();
    for c in path.chars() {
        if c == '`' {
            o.push_str("\\`");
        } else if c == '\\' || c.is_control() {
            o.extend(c.escape_default());
        } else {
            o.push(c);
        }
    }
    o
}

/// The rule that forbids a change to `path` whatever the lists say: a protected place, a link, a look-alike
/// name, a file outside the repository. A file held by a unit that may run beside is named, not refused.
fn never_allowed(p: &Project, pl: &Plan, st: &State, k: &str, path: &str) -> Option<String> {
    let rule = crate::scope::hard_rule(p, pl, st, k, path)?;
    crate::scope::holder_beside(p, pl, st, k, path)
        .is_none()
        .then_some(rule)
}

/// The files the fix changed that lie outside the lists of the checkpoint's units, with the line that
/// says so, for the fix's reviewer. A format-only change accepted by the engine is not listed.
fn outside_section(p: &Project, pl: &Plan, f: &CkFixOpen, role: &str) -> R<String> {
    use crate::strings::t;
    let st = events::replay(p)?;
    let lv = live(p, pl, &st, f);
    let accepted = fmt_load(p, &f.checkpoint, f.open_seq);
    let out: Vec<String> = changed(p, pl, f)?
        .into_iter()
        .filter(|(path, status)| !within(&lv, path, *status) && !accepted.contains_key(path))
        .map(|(path, _)| path)
        .collect();
    if out.is_empty() {
        return Ok(String::new());
    }
    let mut md = format!(
        "## {}\n\n{}\n\n",
        t("ckfix_recommended.brief.outside_title", &[]),
        t(
            if role == "builder" {
                "ckfix_recommended.brief.outside_intro_builder"
            } else {
                "ckfix_recommended.brief.outside_intro"
            },
            &[]
        )
    );
    for path in &out {
        let rule = crate::scope::hard_rule(p, pl, &st, &f.checkpoint, path);
        let path = shown(path);
        let line = match rule {
            Some(rule) => t(
                "ckfix_recommended.brief.outside_file_rule",
                &[("path", &path), ("rule", &rule)],
            ),
            None => t("ckfix_recommended.brief.outside_file", &[("path", &path)]),
        };
        md.push_str(&format!("{line}\n"));
    }
    md.push('\n');
    Ok(md)
}

/// The longest note of the fix builder the brief prints.
const CLAIM_MAX: usize = 2000;

/// The note of the fix builder's latest report, read from its `checkpoint.fix.built` event.
pub fn built_note(p: &Project, f: &CkFixOpen) -> Option<String> {
    let log = events::load_log(p).ok()?;
    log.events
        .iter()
        .rev()
        .filter(|e| {
            e["kind"] == "checkpoint.fix.built"
                && e["checkpoint"] == f.checkpoint.as_str()
                && e["seq"].as_u64().unwrap_or(0) > f.open_seq
        })
        .map(|e| s(e, "note").to_string())
        .next()
}

/// The builder's report as a claim: quoted line by line and cut, so no line of it reads as part of the brief.
fn claim_section(p: &Project, f: &CkFixOpen) -> String {
    use crate::strings::t;
    let Some(note) = built_note(p, f) else {
        return String::new();
    };
    let mut md = format!(
        "## {}\n\n{}\n\n",
        t("ckfix_recommended.brief.claim_title", &[]),
        t(
            "ckfix_recommended.brief.claim_result",
            &[("result", "built")]
        )
    );
    if note.trim().is_empty() {
        md.push_str(&format!(
            "{}\n\n",
            t("ckfix_recommended.brief.claim_none", &[])
        ));
        return md;
    }
    let cut = note.chars().count() > CLAIM_MAX;
    let kept: String = note.chars().take(CLAIM_MAX).collect();
    let kept = kept.replace("\r\n", "\n").replace('\r', "\n");
    for line in kept.lines() {
        md.push_str(&format!("> {line}\n"));
    }
    md.push('\n');
    if cut {
        md.push_str(&format!(
            "{}\n\n",
            t(
                "ckfix_recommended.brief.claim_cut",
                &[("max", &CLAIM_MAX.to_string())]
            )
        ));
    }
    md
}

/// The file the guard reads, for tests and tools.
pub fn overlay_path(p: &Project) -> PathBuf {
    p.state_dir().join("ckfix.json")
}

#[cfg(test)]
mod ckfix_reopen {
    use super::*;

    #[test]
    fn the_kind_of_a_check_is_read_from_its_command() {
        assert_eq!(command_kind("cargo fmt --check"), "format");
        assert_eq!(
            command_kind("cargo +nightly fmt --all -- --check"),
            "format"
        );
        assert_eq!(command_kind("rustfmt --check src/a.rs"), "format");
        assert_eq!(
            command_kind("cargo clippy --all-targets -- -D warnings"),
            "lint"
        );
        assert_eq!(command_kind("cargo nextest run fmt"), "");
        assert_eq!(command_kind("cargo fmt --check && cargo test"), "");
    }

    #[test]
    fn the_output_names_files_and_lines() {
        let top = std::env::temp_dir().join(format!("bf-ckfix-named-{}", std::process::id()));
        fs::create_dir_all(top.join("src")).unwrap();
        fs::write(top.join("src/a.rs"), "x\n").unwrap();
        fs::write(top.join("src/b.rs"), "x\n").unwrap();
        let tail = format!(
            "Diff in {}/src/a.rs:3:\n-x\n+y\nwarning: w\n  --> src/b.rs:7:5\n   |\n7  |     let a = 1;\n8  |     let b = 2;\n   |\nDiff in ../outside.rs:1:\n--> .git/config:1:1\n",
            top.display()
        );
        let n = named_in(&top, &[json!({"tail": tail})], &|_| true);
        let _ = fs::remove_dir_all(&top);
        assert_eq!(n.len(), 2, "{n:?}");
        assert!(n["src/a.rs"].is_empty());
        assert_eq!(
            n["src/b.rs"].iter().copied().collect::<Vec<_>>(),
            vec![7, 8]
        );
    }

    #[test]
    fn a_hunk_is_accounted_for_only_when_it_rewrites_named_lines_and_adds_nothing() {
        let named: BTreeSet<u32> = [4, 5].into_iter().collect();
        assert_eq!(hunk_of("@@ -4 +4 @@ fn a()"), Some((4, 1, 4, 1)));
        assert_eq!(hunk_of("@@ -4,2 +4,3 @@"), Some((4, 2, 4, 3)));
        assert_eq!(hunk_of("@@ -3,0 +4,2 @@"), Some((3, 0, 4, 2)));
        assert_eq!(hunk_of("diff --git a b"), None);
        assert!(hunk_accounted(&named, (4, 1, 4, 1)));
        assert!(hunk_accounted(&named, (4, 2, 4, 1)));
        assert!(hunk_accounted(&named, (4, 1, 4, 0)));
        // A pure insertion beside a named line, and a rewrite that grows, are not accounted for.
        assert!(!hunk_accounted(&named, (4, 0, 5, 3)));
        assert!(!hunk_accounted(&named, (3, 0, 4, 1)));
        assert!(!hunk_accounted(&named, (4, 1, 4, 2)));
        assert!(!hunk_accounted(&named, (3, 2, 3, 2)));
    }

    #[test]
    fn a_named_path_the_place_rule_refuses_is_dropped() {
        let top = std::env::temp_dir().join(format!("bf-ckfix-keep-{}", std::process::id()));
        fs::create_dir_all(top.join("src")).unwrap();
        fs::write(top.join("src/a.rs"), "x\n").unwrap();
        fs::write(top.join("src/b.rs"), "x\n").unwrap();
        let tail = "Diff in src/a.rs:1:\nDiff in src/b.rs:1:\n";
        let n = named_in(&top, &[json!({"tail": tail})], &|path| path != "src/b.rs");
        let _ = fs::remove_dir_all(&top);
        assert_eq!(n.keys().cloned().collect::<Vec<_>>(), vec!["src/a.rs"]);
    }
}
