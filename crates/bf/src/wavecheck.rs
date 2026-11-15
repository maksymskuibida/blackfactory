//! The wave check part (C-20). After each wave of units has landed, the engine runs the project's wave
//! checks on a copy of the checkpoint's content, beside the units of the next wave, and records a receipt
//! for each run. A failure opens a fix held to the units that landed since the last pass (the checkpoint
//! fix part does the work), and no unit lands on the checkpoint until the check of the wave before has
//! passed. This module also keeps the settings: the project's default wave checks and the choice each run
//! makes at its plan stage (R-61), both written only on the human's own words.

use crate::events::{self, State, WaveRun};
use crate::evidence;
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, refuse_owned, R};
use crate::run;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

// ---- the plan side: which waves a checkpoint has and which of them have landed ----

/// The wave check records of the plan: checks with `scope = "wave"` that a command proves.
pub fn records(pl: &Plan) -> Vec<&Value> {
    pl.checks
        .iter()
        .filter(|c| s(c, "scope") == "wave" && s(c, "how") == "command")
        .collect()
}

/// The wave numbers of a checkpoint's units, in order, without repeats.
pub fn waves_of(pl: &Plan, k: &str) -> Vec<u32> {
    let mut v: Vec<u32> = pl
        .units
        .iter()
        .filter(|u| s(u, "checkpoint") == k)
        .filter_map(|u| pl.wave.get(s(u, "id")).copied())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The units of a checkpoint that belong to waves up to and including `w`.
fn units_upto(pl: &Plan, k: &str, w: u32) -> Vec<String> {
    pl.units
        .iter()
        .filter(|u| s(u, "checkpoint") == k)
        .filter(|u| pl.wave.get(s(u, "id")).map(|x| *x <= w).unwrap_or(false))
        .map(|u| s(u, "id").to_string())
        .collect()
}

/// A wave of a checkpoint has landed when every unit of the checkpoint in that wave or an earlier one has.
pub fn landed(pl: &Plan, st: &State, k: &str, w: u32) -> bool {
    let us = units_upto(pl, k, w);
    !us.is_empty() && us.iter().all(|u| st.unit_state(u) == "landed")
}

pub fn run_of<'a>(st: &'a State, k: &str, w: u32) -> Option<&'a WaveRun> {
    st.wave_runs.get(&(k.to_string(), w))
}

fn is_passed(st: &State, k: &str, w: u32) -> bool {
    run_of(st, k, w)
        .map(|r| r.state == "passed")
        .unwrap_or(false)
}

/// Whether a wave check that started is overdue: no result for longer than the idle limit.
fn overdue(p: &Project, r: &WaveRun) -> bool {
    util::now_secs().saturating_sub(util::parse_rfc3339(&r.start_at))
        > p.policy.limits.idle_minutes * 60
}

/// The wave checks in the plan are switched on for the run: the plan holds them and the run did not
/// choose none.
pub fn active(pl: &Plan) -> bool {
    !records(pl).is_empty()
}

/// The steps the engine offers now: (checkpoint, wave) pairs whose wave has landed and whose check has
/// not run, or ran on content that changed meanwhile, or has been running longer than the idle limit.
pub fn due(p: &Project, pl: &Plan, st: &State) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    if st.active_run().is_none()
        || !st.gate_passed("plan", None)
        || st.sync_open.is_some()
        || !active(pl)
    {
        return out;
    }
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        if st.closed.contains_key(kid)
            || st.ck_fix.as_ref().map(|f| f.checkpoint == kid) == Some(true)
        {
            continue;
        }
        for w in waves_of(pl, kid) {
            if !landed(pl, st, kid, w) {
                break;
            }
            match run_of(st, kid, w) {
                Some(r) if r.state == "passed" => continue,
                Some(r) if r.state == "failed" => break,
                Some(r) if r.state == "running" && !overdue(p, r) => break,
                _ => {
                    out.push((kid.to_string(), w));
                    break;
                }
            }
        }
    }
    out
}

/// The waves of a checkpoint that are running (a check is under way).
pub fn running(p: &Project, pl: &Plan, st: &State) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        for w in waves_of(pl, kid) {
            if run_of(st, kid, w)
                .map(|r| r.state == "running" && !overdue(p, r))
                .unwrap_or(false)
            {
                out.push((kid.to_string(), w));
            }
        }
    }
    out
}

/// Why a unit may not land yet: a wave of its checkpoint before its own has landed and its check has not
/// passed (it still runs, is being fixed, or has not started). None when nothing holds it.
pub fn land_blocker(pl: &Plan, st: &State, unit: &str) -> Option<String> {
    if !active(pl) {
        return None;
    }
    let u = pl.unit(unit)?;
    let k = s(u, "checkpoint");
    let mine = *pl.wave.get(unit)?;
    for w in waves_of(pl, k) {
        if w >= mine || is_passed(st, k, w) {
            continue;
        }
        let why = match run_of(st, k, w).map(|r| r.state.as_str()) {
            _ if !landed(pl, st, k, w) => {
                "not every unit of that wave has landed, so its check cannot run yet"
            }
            Some("running") => "its check is still running",
            Some("failed") => "its check failed and is being fixed",
            Some("stale") => "its check ran on content that changed meanwhile and must run again",
            _ => "its check has not run yet",
        };
        return Some(format!(
            "the wave check of wave {w} of {k} has not passed ({why}), and no unit lands on {k} until it has"
        ));
    }
    None
}

/// What holds a checkpoint's close back: a landed wave whose check has not passed.
pub fn close_blockers(pl: &Plan, st: &State, k: &str) -> Vec<String> {
    if !active(pl) {
        return vec![];
    }
    waves_of(pl, k)
        .into_iter()
        .filter(|w| landed(pl, st, k, *w) && !is_passed(st, k, *w))
        .map(|w| {
            format!(
                "the wave check of wave {w} of {k} has not passed ({})",
                run_of(st, k, w)
                    .map(|r| r.state.clone())
                    .unwrap_or_else(|| "not run yet".into())
            )
        })
        .collect()
}

// ---- running the checks on a copy ----

fn copy_path(p: &Project, k: &str, wave: u32) -> PathBuf {
    p.work_dir().join(format!("{k}-wave-{wave}"))
}

fn remove_copy(p: &Project, pl: &Plan, k: &str, wave: u32) {
    let root = copy_path(p, k, wave);
    if !root.exists() {
        return;
    }
    if let Ok(repos) = run::checkpoint_repos(p, pl, k) {
        let several = repos.len() > 1;
        for r in repos {
            let dest = if several {
                root.join(&r.name)
            } else {
                root.clone()
            };
            let _ = gitx::run(
                &r.path,
                &["worktree", "remove", "--force", &dest.to_string_lossy()],
                &[],
            );
            let _ = gitx::run(&r.path, &["worktree", "prune"], &[]);
        }
    }
    let _ = fs::remove_dir_all(&root);
}

/// A fresh copy of the checkpoint's content: a detached worktree of each repository at the commit the
/// checkpoint branch stands on, so what runs there cannot reach the branch and later landings cannot
/// change what is checked.
fn make_copy(p: &Project, pl: &Plan, k: &str, wave: u32) -> R<PathBuf> {
    remove_copy(p, pl, k, wave);
    let root = copy_path(p, k, wave);
    let repos = run::checkpoint_repos(p, pl, k)?;
    let several = repos.len() > 1;
    for r in repos {
        let wt = run::checkpoint_wt(p, pl, k, r)?;
        let head = gitx::must(&wt, &["rev-parse", "HEAD"])?;
        let dest = if several {
            root.join(&r.name)
        } else {
            root.clone()
        };
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        gitx::must(
            &r.path,
            &[
                "worktree",
                "add",
                "--detach",
                &dest.to_string_lossy(),
                &head,
            ],
        )?;
    }
    Ok(root)
}

struct Executed {
    content: String,
    failed: Vec<Value>,
    receipts: Vec<String>,
    results: Vec<Value>,
    stale: bool,
}

/// Run each wave check on a fresh copy of the checkpoint's content, through the evidence code, so each
/// gets a sealed receipt bound to that content. A command that passed also counts for a checkpoint check
/// with the same command.
fn execute(p: &Project, pl: &Plan, k: &str, wave: u32) -> R<Executed> {
    let wt_root = p.work_dir().join(k);
    let copy = make_copy(p, pl, k, wave)?;
    let result = (|| -> R<Executed> {
        let content = p.content_hash(&copy)?;
        let mut failed = Vec::new();
        let mut receipts = Vec::new();
        let mut results = Vec::new();
        for c in records(pl) {
            let cid = s(c, "id").to_string();
            let argv = evidence::command_words(c);
            let attempt = evidence::next_attempt(p, &cid, &argv, &copy)?;
            let repeat = (attempt > 1).then_some((
                attempt,
                "the engine runs the wave check again on content it already ran on",
            ));
            match evidence::run_with(p, &cid, None, &argv, &copy, repeat) {
                Ok(r) => {
                    receipts.push(s(&r, "id").to_string());
                    let exit = r["exit"].as_i64();
                    results.push(
                        json!({"check": cid, "command": argv.join(" "), "exit": exit,
                        "receipt": s(&r, "id")}),
                    );
                    if exit == Some(0) {
                        count_for_checkpoint(p, pl, k, &cid, &argv, &r)?;
                    } else {
                        failed.push(
                            json!({"check": cid, "command": argv.join(" "), "exit": r["exit"],
                            "tail": s(&r, "output_tail")}),
                        );
                    }
                }
                Err(e) => {
                    let tail = format!("{e:?}");
                    results.push(json!({"check": cid, "command": argv.join(" "), "exit": null, "receipt": null}));
                    failed.push(json!({"check": cid, "command": argv.join(" "), "exit": null, "tail": tail}));
                }
            }
        }
        // TH-23: the result is bound to the content the copy had. If the checkpoint's own content moved
        // while the checks ran, the result is not taken for the content now.
        let now = p.content_hash(&wt_root).unwrap_or_default();
        Ok(Executed {
            stale: failed.is_empty() && now != content,
            content,
            failed,
            receipts,
            results,
        })
    })();
    remove_copy(p, pl, k, wave);
    result
}

/// A wave check that passed on the content a checkpoint closes on, with the same command as one of the
/// checkpoint's checks, counts for that check: the engine writes the same run as a receipt of the
/// checkpoint's check, so the check is not run again.
fn count_for_checkpoint(
    p: &Project,
    pl: &Plan,
    k: &str,
    wave_check: &str,
    argv: &[String],
    receipt: &Value,
) -> R<()> {
    let Some(kr) = pl.checkpoint(k) else {
        return Ok(());
    };
    let st = events::replay(p)?;
    for cid in str_list(kr, "checks") {
        let Some(c) = pl.checks.iter().find(|c| s(c, "id") == cid) else {
            continue;
        };
        if s(c, "how") != "command"
            || s(c, "scope") == "wave"
            || evidence::command_words(c) != argv
            || crate::enforce::is_off(&st, c)
        {
            continue;
        }
        let mut v = receipt.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("id");
            o.remove("seal");
            o.remove("check_hash");
            o.insert("check".into(), json!(cid));
            o.insert("via_wave_check".into(), json!(wave_check));
        }
        evidence::write_receipt(p, v)?;
    }
    Ok(())
}

fn require_k(pl: &Plan, k: &str) -> R<()> {
    if pl.checkpoint(k).is_none() {
        return Err(refuse(
            format!("run a wave check for {k}"),
            "no such checkpoint",
            &["Use a checkpoint ID from `bf status`."],
            "The command prints the result of the wave check.",
        ));
    }
    Ok(())
}

/// `bf checkpoint wave-check <K> --wave <n>`: the step the engine offers when a wave has landed. The
/// conductor starts it in the background; it needs no worker's token and holds no lock while the checks
/// run, so the next command is never held up.
pub fn run_step(p: &Project, k: &str, wave: u32) -> R<Value> {
    let pl;
    {
        let _lock = crate::lock::engine(p, &format!("start the wave check of {k}"))?;
        let st = events::require_chain(p)?;
        run::need_run(&st)?;
        pl = plan::load(p)?;
        require_k(&pl, k)?;
        if !st.gate_passed("plan", None) {
            return Err(refuse(
                format!("run the wave check of {k}"),
                "the plan gate has not been approved",
                &["`bf next` shows what comes first."],
                "`bf status` shows the plan gate approved.",
            ));
        }
        if !active(&pl) {
            return Err(refuse(
                format!("run the wave check of {k}"),
                "the plan holds no wave check",
                &["The human decides the wave checks at the plan stage: `bf checks wave state` shows what was decided."],
                "`bf checks wave state` lists the wave checks.",
            ));
        }
        if st.closed.contains_key(k) {
            return Err(refuse(
                format!("run the wave check of {k}"),
                "the checkpoint is closed",
                &["`bf next` shows what is waiting."],
                "`bf status` shows the checkpoint closed.",
            ));
        }
        if !waves_of(&pl, k).contains(&wave) || !landed(&pl, &st, k, wave) {
            return Err(refuse(
                format!("run the wave check of wave {wave} of {k}"),
                "that wave has not landed: some unit of it, or of an earlier wave of the checkpoint, is not landed yet",
                &["`bf next` offers the step when the last unit of a wave has landed."],
                "`bf next` offers the wave check.",
            ));
        }
        if st.ck_fix.as_ref().map(|f| f.checkpoint == k) == Some(true) {
            return Err(refuse(
                format!("run the wave check of {k}"),
                "a fix of the checkpoint is open: the check runs again when it is done",
                &["`bf next` gives the fix's directive."],
                "`bf status` shows no open fix.",
            ));
        }
        if let Some(r) = run_of(&st, k, wave) {
            let why = match r.state.as_str() {
                "passed" => Some("it passed already"),
                "running" if !overdue(p, r) => Some("it is running"),
                "failed" => Some("it failed, and a fix runs it again"),
                _ => None,
            };
            if let Some(why) = why {
                return Err(refuse(
                    format!("run the wave check of wave {wave} of {k}"),
                    why,
                    &["`bf status` shows the state of each wave check."],
                    "`bf status` shows the wave check passed.",
                ));
            }
        }
        let content = p.content_hash(&p.work_dir().join(k))?;
        events::append(
            p,
            "wave.check.start",
            json!({"checkpoint": k, "wave": wave, "content": content}),
        )?;
    }
    let ex = match execute(p, &pl, k, wave) {
        Ok(e) => e,
        Err(e) => {
            // Do not leave the wave marked as running when the copy could not even be made.
            let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
            events::append(
                p,
                "wave.check.run",
                json!({"checkpoint": k, "wave": wave, "content": content, "state": "stale",
                       "failed": [], "receipts": []}),
            )?;
            return Err(e);
        }
    };
    record(p, k, wave, &ex)
}

fn record(p: &Project, k: &str, wave: u32, ex: &Executed) -> R<Value> {
    let state = if !ex.failed.is_empty() {
        "failed"
    } else if ex.stale {
        "stale"
    } else {
        "passed"
    };
    events::append(
        p,
        "wave.check.run",
        json!({"checkpoint": k, "wave": wave, "content": ex.content, "state": state,
               "failed": ex.failed, "receipts": ex.receipts}),
    )?;
    Ok(
        json!({"checkpoint": k, "wave": wave, "state": state, "content": ex.content,
        "results": ex.results, "failed": ex.failed}),
    )
}

/// The wave checks run again on the checkpoint's content after a fix. With `record` the result is the
/// wave's new result; without it the run only tells the builder whether the fix works.
pub fn rerun(p: &Project, pl: &Plan, k: &str, wave: u32, record_it: bool) -> R<Vec<Value>> {
    let ex = execute(p, pl, k, wave)?;
    if record_it {
        record(p, k, wave, &ex)?;
    }
    Ok(ex.failed)
}

// ---- the fix ----

/// Open a fix for a failed wave check, held to the units that landed since the last wave check passed.
/// Called by `bf next`, so a failed wave check never reaches the human.
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
    let failed_waves: Vec<(String, u32)> = st
        .wave_runs
        .iter()
        .filter(|(_, r)| r.state == "failed")
        .map(|(key, _)| key.clone())
        .collect();
    if failed_waves.is_empty() {
        return Ok(());
    }
    let pl = plan::load(p)?;
    for (k, w) in failed_waves {
        if st.closed.contains_key(&k) {
            continue;
        }
        let wc = p.work_dir().join(&k);
        let content = p.content_hash(&wc).unwrap_or_default();
        if content.is_empty() || st.ck_declined.contains(&format!("{k}@{content}")) {
            continue;
        }
        let failed = run_of(&st, &k, w)
            .map(|r| r.failed.clone())
            .unwrap_or_default();
        // The units that landed since the last wave check passed: the waves after the last pass, up to
        // this one.
        let last_pass = waves_of(&pl, &k)
            .into_iter()
            .filter(|x| *x < w && is_passed(&st, &k, *x))
            .max()
            .unwrap_or(0);
        let units: Vec<String> = units_upto(&pl, &k, w)
            .into_iter()
            .filter(|u| pl.wave.get(u).map(|x| *x > last_pass).unwrap_or(false))
            .collect();
        let _lock = crate::lock::engine(p, &format!("open a fix of the wave check of {k}"))?;
        let st2 = events::require_chain(p)?;
        if st2.ck_fix.is_some() || st2.sync_open.is_some() {
            return Ok(());
        }
        if crate::ckfix::open_wave(p, &pl, &k, w, failed, &units, &content)? {
            return Ok(());
        }
    }
    Ok(())
}

// ---- the settings: the project default and the run's choice ----

fn command_checks(commands: &[String]) -> R<Vec<Value>> {
    if commands.is_empty() || commands.iter().any(|c| c.trim().is_empty()) {
        return Err(refuse(
            "propose wave checks",
            "an override needs at least one command, each non-empty",
            &["Give each command as one quoted argument, for example `bf checks wave propose override \"cargo test\"`."],
            "`bf checks wave propose` prints the proposal.",
        ));
    }
    let mut out = Vec::new();
    for c in commands {
        let words = crate::checks::shell_words(c);
        if let Some(w) = evidence::unquoted_operator_word(c)
            .or_else(|| evidence::shell_operator_word(&words, false))
        {
            return Err(refuse(
                "propose wave checks",
                evidence::operator_message(&format!("wave check `{}`", c.trim()), &w),
                &[
                    "Propose one wave check per program.",
                    "Or keep an argument with a space whole by quoting it: the engine stores the words as a list.",
                ],
                "`bf checks wave propose` prints the proposal.",
            ));
        }
        out.push(json!({"title": format!("Wave check: {}", c.trim()), "command": words}));
    }
    Ok(out)
}

fn pick_scope(st: &State, scope: Option<&str>) -> R<String> {
    let sc = scope.map(|x| x.to_string()).unwrap_or_else(|| {
        if st.active_run().is_some() {
            "run"
        } else {
            "default"
        }
        .to_string()
    });
    if !matches!(sc.as_str(), "run" | "default" | "both") {
        return Err(refuse(
            "propose settings",
            format!("the scope `{sc}` is not `run`, `default` or `both`"),
            &["Use `--scope run` for this run, `--scope default` for the project default, or `--scope both`."],
            "The command prints the proposal.",
        ));
    }
    if sc != "default" && st.active_run().is_none() {
        return Err(refuse(
            "propose settings for a run",
            "no run is active",
            &["Start a run with `bf run start`, or use `--scope default`."],
            "The command prints the proposal.",
        ));
    }
    Ok(sc)
}

/// Whether the active run already has an answer for a setting: `wave` or `unitchecks`.
pub(crate) fn run_answered(st: &State, topic: &str) -> bool {
    st.active_run()
        .map(|r| {
            if topic == "wave" {
                r.wave.is_some()
            } else {
                r.unit_checks.is_some()
            }
        })
        .unwrap_or(false)
}

/// Settings of a run are decided at its plan stage; once the plan gate has passed an answer the run
/// already has changes only by an amendment. A question the run has not answered may still be answered
/// once (R-89).
pub(crate) fn refuse_after_plan_gate(st: &State, what: &str, topic: &str) -> R<()> {
    if st.gate_passed("plan", None) && run_answered(st, topic) {
        return Err(refuse(
            format!("change {what} of the run"),
            "the plan gate has passed, so the run's settings are frozen with the plan",
            &["The human changes them by an amendment: `bf amend open`."],
            "`bf checks wave state` shows what the run uses.",
        ));
    }
    Ok(())
}

/// Record a proposal that the human is about to be asked about.
pub(crate) fn propose(p: &Project, topic: &str, scope: &str, value: Value) -> R<Value> {
    let ev = events::append(
        p,
        &format!("{topic}.propose"),
        json!({"scope": scope, "value": value}),
    )?;
    Ok(
        json!({"proposal": ev["seq"], "topic": topic, "scope": scope, "value": value, "changed": false}),
    )
}

/// Record the human's decision on the proposal that is waiting. Returns the proposal's scope and value.
pub(crate) fn confirm(
    p: &Project,
    topic: &str,
    scope: Option<&str>,
    quote: Option<&str>,
) -> R<(String, Value)> {
    let st = events::require_chain(p)?;
    let Some(prop) = st
        .setting_proposals
        .iter()
        .rev()
        .find(|x| x.topic == topic)
        .filter(|x| !x.used)
        .cloned()
    else {
        return Err(refuse(
            "record the settings",
            "no proposal is waiting: none was shown to the human yet, or the last one was already recorded",
            &[
                "Run the `propose` command and show the human the result.",
                "Ask the human in chat, then run `set` with `--quote`.",
            ],
            "The `state` command shows what is decided.",
        ));
    };
    if scope.map(|x| x != prop.scope).unwrap_or(false) {
        return Err(refuse(
            "record settings that differ from the proposal",
            "the human saw the latest proposal; the command may record only that one",
            &["Run `propose` again with the scope you mean and show the human the result."],
            "The `state` command shows what is decided.",
        ));
    }
    if prop.scope != "default" {
        let active = st.active_run().map(|r| r.id.clone()).unwrap_or_default();
        if active.is_empty() || prop.run != active {
            return Err(refuse(
                "record settings for a run",
                "the proposal was made for a run that is no longer the active one, or no run is active",
                &["Run `propose` again in the active run and show the human the result."],
                "The `state` command shows what is decided.",
            ));
        }
        refuse_after_plan_gate(&st, "the settings", topic)?;
        // A wave proposal made before the plan gate may be set after it: compare with the approved
        // records before anything is recorded.
        if topic == "wave" && st.gate_passed("plan", None) {
            keeps_plan_records(p, &prop.value)?;
        }
    }
    let q = quote.unwrap_or("");
    let (turn, _) = events::pick_turn(&st, prop.seq, events::Decide::Approve, q)?;
    events::append(
        p,
        &format!("{topic}.set"),
        json!({"scope": prop.scope, "proposal": prop.seq, "turn": turn, "quote": q.trim(),
               "value": prop.value}),
    )?;
    Ok((prop.scope, prop.value))
}

/// The question the human is asked about wave checks at the plan stage.
pub fn question(st: &State, proposed: &[String]) -> R<Value> {
    let default = st.wave_default.clone();
    // Each proposed command comes with its reason, written `command=reason`.
    let mut with_reasons = Vec::new();
    for a in proposed {
        match a.rsplit_once('=') {
            Some((c, r)) if !c.trim().is_empty() && !r.trim().is_empty() => {
                with_reasons.push(json!({"command": c.trim(), "reason": r.trim()}));
            }
            _ if default.is_none() => {
                return Err(refuse(
                    "propose wave-check commands",
                    format!("`{a}` has no reason: each proposed command needs one"),
                    &["Write each proposal as `<command>=<reason>`, for example `cargo test=the whole suite`."],
                    "`bf checks wave question` prints the question with each reason.",
                ));
            }
            _ => {}
        }
    }
    let proposed = with_reasons;
    let mut options = vec![];
    match &default {
        Some(d) => options.push(json!({"choice": "use-default",
            "text": "Use the project default",
            "shows": d})),
        None => options.push(json!({"choice": "propose",
            "text": "The project has no default: choose commands from the project's own test commands, or no wave checks",
            "proposed": proposed})),
    }
    options
        .push(json!({"choice": "override", "text": "Override it for this run: give the commands"}));
    options.push(json!({"choice": "none", "text": "No wave checks for this run"}));
    Ok(
        json!({"ask": "Which wave checks does this run use?", "project_default": default,
        "options": options,
        "then": "After an override or a first choice, ask whether it also becomes the project default; it changes only on a yes."}),
    )
}

/// `bf checks wave <propose|set|state|question>`.
pub fn settings(
    p: &Project,
    rest: &[String],
    scope: Option<&str>,
    quote: Option<&str>,
) -> R<Value> {
    let sub = rest.first().map(|x| x.as_str()).unwrap_or("state");
    match sub {
        "state" | "question" => {
            let st = events::replay(p)?;
            let pl = plan::load(p)?;
            let mut v = state_value(&st, &pl);
            if sub == "question" {
                v["question"] = question(&st, &rest[1.min(rest.len())..])?;
            }
            Ok(v)
        }
        "propose" => {
            let st = events::require_chain(p)?;
            let sc = pick_scope(&st, scope)?;
            if sc != "default" {
                refuse_after_plan_gate(&st, "the wave checks", "wave")?;
            }
            let mode = rest.get(1).map(|x| x.as_str()).unwrap_or("");
            let cmds: Vec<String> = rest.iter().skip(2).cloned().collect();
            let value = match mode {
                "use-default" => {
                    let Some(d) = st.wave_default.clone() else {
                        return Err(refuse(
                            "propose the project default",
                            "the project has no default wave checks",
                            &["Propose commands instead: `bf checks wave propose override \"<command>\"`, or `none`."],
                            "`bf checks wave state` shows the default.",
                        ));
                    };
                    if sc != "run" {
                        return Err(refuse(
                            "propose the project default as the default",
                            "`use-default` is a choice for a run",
                            &["Use `--scope run`."],
                            "The command prints the proposal.",
                        ));
                    }
                    if d["enabled"] == true {
                        json!({"source": "default", "checks": d["checks"]})
                    } else {
                        json!({"source": "none", "checks": []})
                    }
                }
                "override" => {
                    json!({"source": "override", "checks": command_checks(&cmds)?})
                }
                "none" => json!({"source": "none", "checks": []}),
                _ => {
                    return Err(refuse(
                        "propose wave checks",
                        format!("`{mode}` is not `use-default`, `override` or `none`"),
                        &["Run `bf checks wave question` for the question to ask the human."],
                        "The command prints the proposal.",
                    ))
                }
            };
            if sc != "default" && st.gate_passed("plan", None) {
                keeps_plan_records(p, &value)?;
            }
            propose(p, "wave", &sc, value)
        }
        "set" => {
            let (sc, value) = confirm(p, "wave", scope, quote)?;
            let st = events::replay(p)?;
            if sc != "run" {
                write_default_mirror(p, &st)?;
            }
            // After the plan gate the approved records stay as they are: the answer was checked at
            // propose to keep them.
            if sc != "default" && !st.gate_passed("plan", None) {
                write_records(p, &value)?;
            }
            let pl = plan::load(p)?;
            Ok(state_value(&st, &pl))
        }
        other => Err(refuse(
            "run `bf checks wave`",
            format!("unknown subcommand `{other}`"),
            &["Run `bf checks wave question`, `propose`, `set` or `state`."],
            "The command prints the state of the wave checks.",
        )),
    }
}

fn write_default_mirror(p: &Project, st: &State) -> R<()> {
    let body = serde_json::to_string_pretty(&json!({"default": st.wave_default}))?;
    util::atomic_write(
        &p.factory().join("wave.json"),
        format!("{body}\n").as_bytes(),
    )?;
    Ok(())
}

/// After the plan gate the wave check records are approved: an answer may only keep them.
fn keeps_plan_records(p: &Project, value: &Value) -> R<()> {
    let pl = plan::load(p)?;
    let mut held: Vec<Vec<String>> = records(&pl)
        .iter()
        .map(|r| evidence::command_words(r))
        .collect();
    let mut given: Vec<Vec<String>> = value["checks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(evidence::command_words)
        .collect();
    held.sort();
    given.sort();
    if held != given {
        return Err(refuse(
            "propose wave checks that differ from the approved plan",
            "the plan gate has passed, so the wave check records of the plan are approved and this answer would change them",
            &[
                "Answer with the plan's own wave checks: `bf checks wave state` shows them (`override` with the same commands, or `none` when the plan has none).",
                "To change them the human opens an amendment: `bf amend open`.",
            ],
            "`bf checks wave state` shows what the plan holds.",
        ));
    }
    Ok(())
}

/// Put the run's wave checks in the plan as check records: the plan holds its own copy of the default.
fn write_records(p: &Project, value: &Value) -> R<()> {
    let kind = crate::schema::kind_by_name("check").unwrap();
    for r in p.list_records(Some(kind))? {
        if s(&r, "scope") == "wave" {
            let _ = fs::remove_file(p.record_path(s(&r, "id"))?);
        }
    }
    let source = value["source"].as_str().unwrap_or("override");
    for c in value["checks"].as_array().cloned().unwrap_or_default() {
        let mut rec = json!({
            "kind": "check",
            "title": s(&c, "title"),
            "text": format!("Run after each wave of units has landed, on a copy of the checkpoint's content. From the {}.",
                if source == "default" { "project default" } else { "run's override" }),
            "how": "command",
            "command": evidence::command_words(&c),
            "proves": [],
            "scope": "wave",
        });
        p.write_numbered(kind, &mut rec)?;
    }
    Ok(())
}

pub fn state_value(st: &State, pl: &Plan) -> Value {
    let run = st.runs.last();
    json!({
        "default": st.wave_default,
        "run": run.map(|r| r.id.clone()),
        "run_choice": run.and_then(|r| r.wave.clone()),
        "plan_checks": records(pl).iter().map(|c| json!({"id": s(c, "id"), "command": evidence::command_words(c).join(" ")})).collect::<Vec<_>>(),
        "waves": status_json(st),
    })
}

/// Every wave check run, for `bf status`.
pub fn status_json(st: &State) -> Value {
    json!(st
        .wave_runs
        .values()
        .map(|r| json!({"checkpoint": r.checkpoint, "wave": r.wave, "state": r.state,
            "content": r.content, "failed": r.failed.iter().filter_map(|x| x["check"].as_str()).collect::<Vec<_>>()}))
        .collect::<Vec<_>>())
}

/// The directive for a step the engine offers.
pub fn directive(k: &str, wave: u32, pl: &Plan) -> Value {
    json!({"role": "engine", "action": "wave-check", "checkpoint": k, "wave": wave,
        "background": true,
        "checks": records(pl).iter().map(|c| s(c, "id").to_string()).collect::<Vec<_>>(),
        "command": format!("bf checkpoint wave-check {k} --wave {wave}"),
        "note": format!("Start this in the background: the units of the next wave are offered at the same time and are built without waiting; only their landing waits for wave {wave} of {k} to pass.")})
}

/// A section of the plan page: the wave checks the run uses and where they came from, or the plain
/// statement that the plan has none.
pub fn plan_section(pl: &Plan, st: &State) -> Value {
    let recs = records(pl);
    let choice = st.runs.last().and_then(|r| r.wave.clone());
    let came = match choice.as_ref().and_then(|c| c["source"].as_str()) {
        Some("default") => "from the project default",
        Some("override") => "from an override for this run",
        Some("none") => "the run chose no wave checks",
        _ => "not decided yet: the human is asked at the plan stage",
    };
    let mut rows: Vec<Value> = recs
        .iter()
        .map(|c| {
            json!({"id": s(c, "id"), "title": s(c, "title"), "body": [
                {"type": "code", "text": evidence::command_words(c).join(" ")},
                {"type": "text", "text": format!("Runs after each wave has landed, on a copy of the content. {came}.")}]})
        })
        .collect();
    if rows.is_empty() {
        rows.push(json!({"id": "wave-none", "title": "This plan has no wave check", "body": [
            {"type": "text", "text": format!("No wave check runs between waves: a failure shows only at the checkpoint ({came}).")}]}));
    }
    let default_text = match &st.wave_default {
        Some(d) if d["enabled"] == true => format!(
            "Project default: {}.",
            d["checks"]
                .as_array()
                .map(|a| a
                    .iter()
                    .map(|x| evidence::command_words(x).join(" "))
                    .collect::<Vec<_>>()
                    .join("; "))
                .unwrap_or_default()
        ),
        Some(_) => "Project default: no wave checks.".to_string(),
        None => "The project has no default for wave checks.".to_string(),
    };
    json!({"id": "wave-checks", "title": "Wave checks", "hint": default_text, "open": true,
        "blocks": [], "rows": rows})
}

/// A section of the plan page for the checks every unit must pass: for each unit, which apply and where
/// they come from, with each override and its reason marked.
pub fn unit_checks_section(p: &Project, pl: &Plan, st: &State) -> Value {
    let mut rows = Vec::new();
    for k in &pl.checkpoints {
        if k.get("unit_checks").is_some() {
            rows.push(json!({"id": s(k, "id"), "title": format!("{} overrides the checks every unit must pass", s(k, "id")),
                "label": "medium", "body": [{"type": "text", "text": format!("Reason: {}", s(k, "unit_checks_reason"))}]}));
        }
    }
    for u in &pl.units {
        let id = s(u, "id");
        let (list, source, reason) = crate::checks::attached_for_unit(p, pl, st, id);
        let lines = crate::checks::attached_lines(p, pl, st, id);
        let mut body: Vec<Value> = vec![];
        if lines.is_empty() {
            body.push(json!({"type": "text", "text": format!("No check is attached to this unit (source: {source}).")}));
        } else {
            body.push(json!({"type": "list", "items": lines}));
        }
        if u.get("own_checks").is_some() {
            body.push(
                json!({"type": "text", "text": format!("This unit overrides the list: {reason}")}),
            );
        }
        let _ = list;
        rows.push(json!({"id": format!("{id}-checks"), "title": format!("{id}: checks every unit must pass"), "body": body}));
    }
    json!({"id": "unit-checks", "title": "Checks every unit must pass", "hint": "The most specific list wins: the unit's, the checkpoint's, the run's, the project's",
        "open": false, "blocks": [], "rows": rows})
}

/// The question the plan stage still has to put to the human, for `bf status`.
pub fn plan_questions(st: &State) -> Value {
    let run = st.active_run();
    json!({
        "wave_checks_decided": run.map(|r| r.wave.is_some()).unwrap_or(false),
        "unit_checks_decided": run.map(|r| r.unit_checks.is_some()).unwrap_or(false),
    })
}

/// The error a caller gets for a wave number that is not a number.
pub fn parse_wave(v: Option<&str>) -> R<u32> {
    v.and_then(|x| x.trim().parse::<u32>().ok()).ok_or_else(|| {
        refuse_owned(
            "run a wave check",
            "it needs the wave number",
            vec![
                "Run `bf checkpoint wave-check <K> --wave <n>`; `bf next` gives the exact command."
                    .to_string(),
            ],
            "The command prints the result of the wave check.",
        )
    })
}
