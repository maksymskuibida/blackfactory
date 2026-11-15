//! Runs: start, status, next directives, reports, briefs, unit working copies,
//! landing, checkpoint closing, stuck units and cost.

use crate::events::{self, State};
use crate::evidence::{self, Receipt};
use crate::gates;
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, refuse_owned, R};
use crate::schema;
use crate::store::{s, str_list, Project, Repo};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Test hook: exit abruptly at a named point when `BF_CRASH_AT` names it.
pub fn crash_point(name: &str) {
    if std::env::var("BF_CRASH_AT")
        .map(|v| v == name)
        .unwrap_or(false)
    {
        std::process::exit(137);
    }
}

/// A finding is open until its outcome says fixed or accepted.
pub fn is_open(f: &Value) -> bool {
    !matches!(s(f, "outcome"), "fixed" | "accepted")
}

/// The plan questions the active run has not answered (R-89): the directive action, the command that
/// asks it and what it asks. Empty when no run is active.
pub(crate) fn unanswered_questions(st: &State) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut out = vec![];
    if let Some(r) = st.active_run() {
        if r.wave.is_none() {
            out.push((
                "answer-wave-checks",
                "bf checks wave question",
                "which wave checks this run uses",
            ));
        }
        if r.unit_checks.is_none() {
            out.push((
                "answer-unit-checks",
                "bf checks unit question",
                "which checks every unit of this run must pass",
            ));
        }
    }
    out
}

pub(crate) fn need_run(st: &State) -> R<String> {
    st.active_run().map(|r| r.id.clone()).ok_or_else(|| {
        refuse(
            "continue without a run",
            "no run is active",
            &["Run `bf run start`."],
            "`bf status` shows an active run.",
        )
    })
}

pub fn run_start(p: &Project) -> R<Value> {
    let st = events::require_chain(p)?;
    if let Some(r) = st.active_run() {
        return Err(refuse(
            "start a run",
            format!("run {} is already active", r.id),
            &["Continue it with `bf next`, or end it with `bf run stop`."],
            "`bf status` shows one active run.",
        ));
    }
    let pl = plan::load(p)?;
    plan::check_or_refuse(&pl)?;
    let id = format!("run-{}", st.runs.len() + 1);
    let grant = st.pending_grants.last().copied();
    events::append(
        p,
        "run.start",
        json!({"run": id, "grant": grant, "checks_off": st.checks_default}),
    )?;
    drop_role_tokens(p)?;
    p.write_guard()?;
    Ok(json!({"run": id, "test_mode": grant.is_some(), "checks_off": st.checks_default}))
}

pub fn run_stop(p: &Project) -> R<Value> {
    let st = events::require_chain(p)?;
    let id = need_run(&st)?;
    let pl = plan::load(p)?;
    let off = crate::enforce::refuse_if_unproven_for(p, &pl, "stop the run", "bf run stop")?;
    let unproven = crate::enforce::off_unproven_summary(&pl, &st);
    events::append(
        p,
        "run.stop",
        json!({"run": id, "checks_off": off, "unproven_off": unproven}),
    )?;
    drop_role_tokens(p)?;
    p.write_guard()?;
    Ok(json!({"run": id, "stopped": true, "unproven_off": unproven}))
}

/// Computed state of every unit: planned, ready, or the logged state.
pub fn unit_states(
    p: &Project,
    pl: &Plan,
    st: &State,
    receipts: &[Receipt],
) -> Vec<(String, String, Option<String>)> {
    let mut out = Vec::new();
    for u in &pl.units {
        let id = s(u, "id").to_string();
        let logged = st.unit_state(&id);
        if !logged.is_empty() {
            out.push((id, logged.to_string(), None));
        } else if let Some(why) = plan::not_ready(p, pl, st, &id, receipts) {
            out.push((id, "planned".into(), Some(why)));
        } else {
            out.push((id, "ready".into(), None));
        }
    }
    out
}

/// The latest link per page name, as `[{name, url, at}]`.
pub fn pages_json(st: &State) -> Vec<Value> {
    st.pages
        .iter()
        .map(|(n, (u, at))| json!({"name": n, "url": u, "at": at}))
        .collect()
}

/// `bf page set <name> <url>`: record where a page was published, so that a new session finds it. The
/// link stays in the workspace's log; nothing writes it anywhere else.
pub fn page_set(p: &Project, name: &str, url: &str) -> R<Value> {
    events::require_chain(p)?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(refuse(
            "record a page link",
            "the page name must be letters, digits, `-`, `_` or `.` (for example `questions`, `gate-architecture-r1`, `plan`)",
            &["Use the page's name without `.html`."],
            "`bf page list` shows it.",
        ));
    }
    let scheme = format!("{}{}", "https", ":/\u{2f}");
    if !url.starts_with(&scheme) || url.len() < 12 || url.contains(char::is_whitespace) {
        return Err(refuse(
            "record a page link",
            "the link must be a secure web address (https) without spaces",
            &["Pass the address the page was published at, the way the browser shows it."],
            "`bf page list` shows it.",
        ));
    }
    events::append(p, "page.published", json!({"name": name, "url": url}))?;
    Ok(json!({"name": name, "url": url}))
}

pub fn status(p: &Project) -> R<Value> {
    let st = events::replay(p)?;
    // The positions are worked out once (each costs git calls); the combined one is built from them.
    let repo_positions = main_positions(p, &st)?;
    let main = combine_positions(&repo_positions);
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let states = unit_states(p, &pl, &st, &receipts);
    let mut counts = serde_json::Map::new();
    for (_, s_, _) in &states {
        let c = counts.entry(s_.clone()).or_insert(json!(0));
        *c = json!(c.as_u64().unwrap_or(0) + 1);
    }
    let run = st.active_run();
    let test = match (run.map(|r| r.test_mode), st.pending_grants.is_empty()) {
        (Some(true), _) => "granted",
        (_, false) => "pending",
        _ => "none",
    };
    Ok(json!({
        "project": p.config.project,
        "run": run.map(|r| r.id.clone()),
        "test_mode": test,
        "chain_ok": st.chain_ok,
        "events": st.event_count,
        "units": states.iter().map(|(i, s_, why)| json!({"id": i, "state": s_,
            "waiting_for": match crate::scope::open_request(&st, i) {
                Some(r) => json!(format!("a scope review of {} requested file(s)", r.files.len())),
                None => json!(why),
            },
            "scope_request": crate::scope::request_json(&st, i),
            "scope_grants": crate::scope::grants_json(&st, i),
            "checkpoint_fixes": crate::ckfix::fixes_of_unit(&st, i),
            "review_rounds": st.units.get(i).map(|u| u.review_rounds.clone()).unwrap_or_default()})).collect::<Vec<_>>(),
        "checkpoint_fix": crate::ckfix::status_json(&st),
        "wave_checks": crate::wavecheck::status_json(&st),
        "plan_questions": crate::wavecheck::plan_questions(&st),
        "checkpoint_reviews": crate::ckreview::status_json(p, &st),
        "unit_counts": counts,
        "gates": st.gates.iter().map(gates::gate_json).collect::<Vec<_>>(),
        "amendments": gates::amendments_json(&st),
        "checkpoints_closed": st.closed.keys().collect::<Vec<_>>(),
        "spent": st.cost,
        "budget_cap": p.policy.budget.cap,
        "ignored_events": st.ignored,
        "frozen_drift": gates::drift(p, &st)?,
        "main": {"ok": !main.moved, "detail": main.detail, "note": main.note, "tip": main.tip},
        "repos": p.repos.iter().zip(repo_positions.iter()).map(|(r, m)| json!({
            "name": r.name, "path": r.path.to_string_lossy(), "branch": r.branch,
            "ok": !m.moved, "tip": m.tip, "detail": m.detail, "note": m.note,
        })).collect::<Vec<_>>(),
        "workspace": p.workspace,
        "main_ahead": main_ahead(p, &st, &pl),
        "sync_open": st.sync_open.as_ref().map(|o| json!({
            "checkpoint": o.checkpoint, "phase": o.phase, "rounds": o.rounds, "reason": o.reason,
            "files": sync_files(o),
            "next": "bf next gives the directive",
        })),
        "pages": pages_json(&st),
        "human_turns_note": st.human_turns_note().unwrap_or_default(),
        "skipped_gates": st.skipped.iter().map(|(k, q, _)| json!({"gate": k, "quote": q})).collect::<Vec<_>>(),
        "policy": format!("{:?}", p.policy_state),
        "bad_records": pl.bad.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>(),
    }))
}

fn tier_for(p: &Project, unit: &Value, role: &str) -> String {
    if let Some(t) = unit.get("tier").and_then(|v| v.as_str()) {
        if role == "builder" {
            return t.to_string();
        }
    }
    match role {
        "reviewer" => p.policy.tiers.reviewer.clone(),
        "verifier" => p.policy.tiers.verifier.clone(),
        _ => p.policy.tiers.builder.clone(),
    }
}

fn budget_check(p: &Project, st: &State) -> R<()> {
    let cap = p.policy.budget.cap;
    if cap > 0.0 && st.cost >= cap {
        return Err(refuse(
            "start new work",
            format!("the budget cap is reached: spent {:.2} of {:.2} {}", st.cost, cap, p.policy.budget.unit),
            &["Ask the human whether to raise `budget.cap` in factory/policy.toml (only they edit it).", "Run `bf cost status` to see where it went."],
            "`bf next` returns directives again.",
        ));
    }
    Ok(())
}

fn next_raw(p: &Project) -> R<Value> {
    let st = events::require_chain(p)?;
    let run = need_run(&st)?;
    budget_check(p, &st)?;
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let mut directives: Vec<Value> = Vec::new();
    let mut waiting: Vec<String> = Vec::new();

    // With the gates in order, the one to offer is the first that is not approved or skipped: decide it
    // if it is open, otherwise open it, or say which record is missing for it to open.
    if p.policy.gates.in_order && !st.gate_passed("plan", None) {
        let kind = gates::first_unpassed_design_gate(&st);
        if let Some(g) = st.latest_gate(kind, None).filter(|g| g.status == "open") {
            directives.push(
                json!({"role": "human", "action": "decide-gate", "gate": g.id, "gate_kind": kind,
                "note": format!("Ask the human to approve or reject the {kind} gate in chat.")}),
            );
        } else if let Some(why) = gates::open_blocker(p, &st, kind) {
            let mut d = json!({"role": "conductor", "action": "add-records", "gate_kind": kind, "missing": why,
                "note": format!("The {kind} gate cannot open yet: {why}. Write what it needs, then ask again.")});
            if kind == "ui" {
                // A product with no interface needs no screen: the human can skip the gate.
                d["skip_command"] = json!("bf gate skip ui --quote \"<the human's words>\"");
                d["note"] = json!(format!("The ui gate cannot open yet: {why}. Write a screen or journey, or, for a product with no interface, ask the human whether to skip the ui gate and record their words with the skip command."));
            }
            directives.push(d);
        } else {
            directives.push(json!({"role": "human", "action": "open-gate", "gate_kind": kind, "command": format!("bf gate open {kind}")}));
        }
        return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
    }
    match st.latest_gate("plan", None) {
        Some(g) if g.status == "approved" => {
            // A run that has not answered a plan question is asked it now; no unit starts until then.
            if !pl.units.is_empty() {
                for (action, command, _) in unanswered_questions(&st) {
                    let (what, set) = if action == "answer-wave-checks" {
                        ("which wave checks this run uses (the project default, an override or none)", "bf checks wave")
                    } else {
                        ("which checks every unit of this run must pass (the project's list, an override or none)", "bf checks unit")
                    };
                    directives.push(json!({"role": "human", "action": action, "command": command,
                        "note": format!("Ask the human {what}, then record the answer with `{set} propose` and `{set} set --quote`. No unit starts until both questions are answered.")}));
                }
            }
        }
        Some(g) if g.status == "open" => {
            directives.push(json!({"role": "human", "action": "decide-gate", "gate": g.id, "note": "Ask the human to approve or reject the plan in chat."}));
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
        }
        _ => {
            // The run's two plan questions come first: the plan gate does not open until both are answered.
            if let Some(r) = st.active_run().filter(|_| !pl.units.is_empty()) {
                if r.wave.is_none() {
                    directives.push(json!({"role": "human", "action": "answer-wave-checks", "command": "bf checks wave question",
                        "note": "Ask the human which wave checks this run uses (the project default, an override or none), then record the answer with `bf checks wave propose` and `bf checks wave set --quote`."}));
                }
                if r.unit_checks.is_none() {
                    directives.push(json!({"role": "human", "action": "answer-unit-checks", "command": "bf checks unit question",
                        "note": "Ask the human which checks every unit of this run must pass (the project's list, an override or none), then record the answer with `bf checks unit propose` and `bf checks unit set --quote`."}));
                }
            }
            directives.push(json!({"role": "human", "action": "open-gate", "gate_kind": "plan", "command": "bf gate open plan"}));
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
        }
    }
    // An open sync of the main branch into a checkpoint: nothing else starts until it is settled.
    if let Some(so) = &st.sync_open {
        let k = &so.checkpoint;
        let wc = p.work_dir().join(k).to_string_lossy().to_string();
        let lim = json!({"max_rounds": p.policy.limits.max_rounds, "round": so.rounds, "idle_minutes": p.policy.limits.idle_minutes,
               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }});
        let brief = |role: &str| {
            p.briefs_dir()
                .join(format!("{k}-{role}.md"))
                .to_string_lossy()
                .to_string()
        };
        let mut d = match so.phase.as_str() {
            "resolve" => json!({"role": "builder", "action": "resolve-sync", "checkpoint": k,
                "tier": p.policy.tiers.builder, "working_copy": wc, "brief": brief("builder"), "limits": lim,
                "files": sync_files(so),
                "may_change": sync_allowed(so),
                "failing_checks": so.failed.iter().filter_map(|f| f["check"].as_str()).collect::<Vec<_>>(),
                "commands": [format!("bf brief {k} --role builder")],
                "abort_command": format!("bf checkpoint sync {k} --abort")}),
            "review" => json!({"role": "reviewer", "action": "review-sync", "checkpoint": k,
                "tier": p.policy.tiers.reviewer, "working_copy": wc, "brief": brief("reviewer"), "limits": lim,
                "files": sync_files(so),
                "commands": [format!("bf brief {k} --role reviewer")],
                "abort_command": format!("bf checkpoint sync {k} --abort")}),
            _ => json!({"role": "human", "action": "decide-sync", "checkpoint": k,
                "note": format!("The sync of {k} is parked ({}). The human says whether to try again (`bf checkpoint sync {k} --release --quote \"<their words>\"`) or to go back (`bf checkpoint sync {k} --abort`).", so.reason),
                "release_command": format!("bf checkpoint sync {k} --release --quote \"<the human's words>\""),
                "abort_command": format!("bf checkpoint sync {k} --abort")}),
        };
        d["note_sync"] = json!(format!(
            "A sync of the main branch into {k} is open; no unit starts until it is settled."
        ));
        directives.push(d);
        return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
    }
    // An open fix of a checkpoint: it is settled before anything else starts.
    if let Some(f) = &st.ck_fix {
        let mut d = crate::ckfix::directive(p, &st, f);
        if f.source == "wave" {
            // A fix of a wave check does not hold the next wave back: its units are built without
            // waiting, and only their landing waits for the wave check to pass.
            d["note_fix"] = json!(format!(
                "A fix of the wave check of wave {} of {} is open. The units of the next wave are offered below and are built without waiting; none of them lands until the wave check has passed.",
                f.wave, f.checkpoint
            ));
            directives.push(d);
        } else {
            d["note_fix"] = json!(format!(
                "A fix of {} is open; nothing else starts until it is settled.",
                f.checkpoint
            ));
            directives.push(d);
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
        }
    }
    let limits = |us_rounds: u32| {
        json!({"max_rounds": p.policy.limits.max_rounds, "round": us_rounds, "idle_minutes": p.policy.limits.idle_minutes,
               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }})
    };
    // Questions a worker put to the human hold that unit (or checkpoint) only; the others go on.
    let open_unit_questions: Vec<Value> = p
        .list_records(schema::kind_by_name("question"))?
        .into_iter()
        .filter(|q| !s(q, "unit").is_empty() && q.get("answer").is_none())
        .collect();
    let held = |id: &str| open_unit_questions.iter().any(|q| s(q, "unit") == id);
    for q in &open_unit_questions {
        directives.push(json!({"role": "human", "action": "answer-question", "question": s(q, "id"), "unit": s(q, "unit"),
            "title": s(q, "title"),
            "note": format!("{} asks: {}. Show the human the question; when they have answered, `bf answer {} --file <answer>`. Only {} waits.", s(q, "unit"), s(q, "title"), s(q, "id"), s(q, "unit"))}));
    }
    for (id, state, why) in unit_states(p, &pl, &st, &receipts) {
        let u = pl.unit(&id).unwrap();
        if held(&id) && matches!(state.as_str(), "ready" | "running" | "fixing" | "review") {
            waiting.push(format!(
                "{id}: waiting for the human's answer to its question"
            ));
            continue;
        }
        let wc = p.work_dir().join(&id).to_string_lossy().to_string();
        let rounds = st.units.get(&id).map(|x| x.rounds).unwrap_or(0);
        let brief = |role: &str| {
            p.briefs_dir()
                .join(format!("{id}-{role}.md"))
                .to_string_lossy()
                .to_string()
        };
        match state.as_str() {
            "ready" if !unanswered_questions(&st).is_empty() => {
                waiting.push(format!("{id}: waiting for the human's answers to the run's plan questions"))
            }
            "ready" => directives.push(json!({"role": "builder", "action": "start-unit", "unit": id, "tier": tier_for(p, u, "builder"),
                "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                "commands": [format!("bf unit start {id}"), format!("bf brief {id} --role builder")]})),
            "running" | "fixing" if crate::scope::open_request(&st, &id).is_some() => {
                // A builder's request for files is with a cold reviewer: its directive replaces the
                // builder's until the verdict is in.
                directives.push(crate::scope::directive(p, &st, &id, limits(rounds)));
            }
            "running" | "fixing" => {
                let mut d = json!({"role": "builder", "action": if state == "fixing" { "fix" } else { "build" }, "unit": id,
                    "tier": tier_for(p, u, "builder"), "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                    "commands": [format!("bf brief {id} --role builder")]});
                if let Some(a) = st.units.get(&id).map(|x| x.reopened_by.clone()).filter(|a| !a.is_empty()) {
                    d["amendment"] = json!(a);
                }
                directives.push(d);
            }
            "review" => directives.push(json!({"role": "reviewer", "action": "review", "unit": id, "tier": tier_for(p, u, "reviewer"),
                "working_copy": wc, "brief": brief("reviewer"), "limits": limits(rounds),
                "commands": [format!("bf brief {id} --role reviewer")]})),
            "reviewed" => directives.push(json!({"role": "engine", "action": "land", "unit": id, "command": format!("bf unit land {id}")})),
            "blocked" | "parked" => {
                let mut note = format!("{id} is {state}; the human decides whether to re-plan or release it.");
                if let Some(r) = st.scope.get(&id).and_then(|sc| sc.refused.last()).filter(|_| state == "blocked") {
                    let files: Vec<String> = r["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|f| {
                            format!(
                                "{}: {}",
                                s(f, "path"),
                                if s(f, "rule").is_empty() { s(f, "why") } else { s(f, "rule") }
                            )
                        })
                        .collect();
                    note.push_str(&format!(" A request for files was refused ({}): {}. The plan repair is the human's.", s(r, "refusal"), files.join("; ")));
                }
                directives.push(json!({"role": "human", "action": "decide-unit", "unit": id, "state": state, "note": note}))
            }
            "planned" => waiting.push(format!("{id}: {}", why.unwrap_or_default())),
            _ => {}
        }
    }
    // The step after a wave has landed: each wave check runs on a copy of the content, beside the units of
    // the next wave. The engine runs it; it needs no worker's token.
    for (k, w) in crate::wavecheck::due(p, &pl, &st) {
        directives.push(crate::wavecheck::directive(&k, w, &pl));
    }
    for (k, w) in crate::wavecheck::running(p, &pl, &st) {
        waiting.push(format!("{k}: the wave check of wave {w} is running"));
    }
    // Checkpoints whose units have all landed.
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        let after = str_list(k, "after");
        if st.closed.contains_key(kid) {
            continue;
        }
        // The last wave's check counts for the checkpoint's own checks when it ran on the content the
        // checkpoint closes on, so the checkpoint waits for it.
        let wave_held = crate::wavecheck::close_blockers(&pl, &st, kid);
        if !wave_held.is_empty()
            && !after.is_empty()
            && after.iter().all(|u| st.unit_state(u) == "landed")
        {
            waiting.push(format!("{kid}: {}", wave_held.join("; ")));
            if st.ck_fix.is_none()
                && st
                    .wave_runs
                    .iter()
                    .any(|((c, _), r)| c == kid && r.state == "failed")
            {
                directives.push(json!({"role": "human", "action": "decide-wave-check", "checkpoint": kid,
                    "note": format!("A wave check of {kid} failed and its fix was turned down. The human decides: change the plan by amendment, or have the fix tried again.")}));
            }
            continue;
        }
        if held(kid) {
            waiting.push(format!(
                "{kid}: waiting for the human's answer to its question"
            ));
            continue;
        }
        if !after.is_empty() && after.iter().all(|u| st.unit_state(u) == "landed") {
            let wc = p.work_dir().join(kid).to_string_lossy().to_string();
            let content = p.content_hash(&p.work_dir().join(kid)).unwrap_or_default();
            // A check the human switched off is not asked for.
            let missing: Vec<String> = str_list(k, "checks")
                .into_iter()
                .filter(|c| {
                    !pl.checks
                        .iter()
                        .any(|r| s(r, "id") == c && crate::enforce::is_off(&st, r))
                })
                .filter(|c| evidence::satisfied_at(&receipts, c, &content).is_none())
                // The checkpoint review is settled by its reviewer, never by the verifier.
                .filter(|c| {
                    !pl.checks.iter().any(|r| {
                        s(r, "id") == c && s(r, "named") == crate::checks::CHECKPOINT_REVIEW
                    })
                })
                .collect();
            let review_needed = crate::ckreview::needed(p, &st, kid, &content)?;
            let human_needed =
                crate::enforce::stops(k) && !st.gate_passed("checkpoint-step", Some(kid));
            let marked: Vec<Value> = str_list(k, "checks")
                .iter()
                .filter_map(|c| {
                    evidence::standing_outcome(&receipts, c, &content).map(|r| {
                        json!({"check": c, "outcome": r.outcome().unwrap_or(""), "reason": s(&r.v, "reason")})
                    })
                })
                .collect();
            if !marked.is_empty() {
                directives.push(json!({"role": "human", "action": "decide-check", "checkpoint": kid, "checks": marked,
                    "note": "A check failed or could not be run. The human decides: fix the cause and have the verifier run it again (a passing run replaces the mark), accept the gap by amendment, or change the plan."}));
            } else if !missing.is_empty() {
                directives.push(json!({"role": "verifier", "action": "run-checkpoint-checks", "checkpoint": kid, "checks": missing,
                    "tier": p.policy.tiers.verifier, "working_copy": wc,
                    "brief": p.briefs_dir().join(format!("{kid}-verifier.md")).to_string_lossy(),
                    "limits": {"max_rounds": p.policy.limits.max_rounds, "idle_minutes": p.policy.limits.idle_minutes,
                               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }},
                    "commands": [format!("bf brief {kid} --role verifier")]}));
                if review_needed {
                    directives.push(crate::ckreview::directive(p, &st, kid)?);
                }
            } else if review_needed {
                directives.push(crate::ckreview::directive(p, &st, kid)?);
            } else if human_needed {
                let open = st
                    .latest_gate("checkpoint-step", Some(kid))
                    .map(|g| g.status == "open")
                    .unwrap_or(false);
                let step_gate = st
                    .latest_gate("checkpoint-step", Some(kid))
                    .filter(|g| g.status == "open")
                    .map(|g| g.id.clone());
                directives.push(json!({"role": "human", "action": if open { "decide-gate" } else { "open-gate" }, "gate_kind": "checkpoint-step", "checkpoint": kid, "gate": step_gate,
                    "on_human_step": "stop",
                    "stop_why": s(k, "stop_why"),
                    "note": s(k, "human_step")}));
            } else {
                // Offer the close only when the engine would accept it.
                let waived = gates::waived_ids(&st);
                let mut gaps: Vec<String> = str_list(k, "proves")
                    .into_iter()
                    .filter(|id| !waived.contains_key(id))
                    .filter(|id| {
                        let cs: Vec<&Value> = pl
                            .checks
                            .iter()
                            .filter(|c| {
                                str_list(k, "checks").iter().any(|x| x == s(c, "id"))
                                    && str_list(c, "proves").contains(id)
                            })
                            .collect();
                        !crate::enforce::id_proven(&st, &receipts, &cs, kid, &content)
                    })
                    .map(|id| format!("{id}: no passing receipt at this content from a check the checkpoint lists"))
                    .collect();
                let (check_gaps, unproven) =
                    crate::enforce::checkpoint_gaps(p, &pl, &st, &receipts, k, &content)?;
                gaps.extend(check_gaps);
                if gaps.is_empty() {
                    directives.push(json!({"role": "engine", "action": "close-checkpoint", "checkpoint": kid, "command": format!("bf checkpoint close {kid}")}));
                } else {
                    directives.push(json!({"role": "conductor", "action": "fix-checkpoint-gaps", "checkpoint": kid, "gaps": gaps,
                        "steps": crate::enforce::way_out_steps(&st, &unproven),
                        "note": format!("`bf checkpoint close {kid}` would refuse: {}. Settle each gap, or ask the human to switch the check off for the run.", gaps.join("; "))}));
                }
            }
            if !review_needed {
                if let Some(d) = crate::ckreview::low_directive(p, &st, kid)? {
                    directives.push(d);
                }
            }
        } else {
            waiting.push(format!("{kid}: waiting for its units to land"));
        }
    }
    Ok(json!({"run": run, "directives": directives, "waiting": waiting}))
}

/// Whether `token` is the current token for `key` (`<unit or checkpoint>:<role>`), as shown by `bf next`.
pub fn holds_token(p: &Project, key: &str, token: Option<&str>) -> bool {
    match (read_tokens(p).get(key).and_then(|t| t.as_str()), token) {
        (Some(h), Some(t)) => h == t.trim(),
        _ => false,
    }
}

/// The one-use token for `key` (`<unit>:<role>`): the one already handed out, or a new one kept in
/// the state folder like the tokens of the existing roles.
pub fn issue_token(p: &Project, key: &str) -> R<String> {
    let _tl = crate::lock::tokens(p)?;
    let mut v = read_tokens(p);
    if let Some(t) = v.get(key).and_then(|t| t.as_str()) {
        return Ok(t.to_string());
    }
    let tok = util::random_hex(12)?;
    v[key] = json!(tok);
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(tok)
}

fn read_tokens(p: &Project) -> Value {
    std::fs::read_to_string(p.tokens_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

/// `bf tokens` are the way a report says which role is speaking. The token of every open worker
/// directive is kept in the state folder, keyed by unit and role, and `bf next` shows it every time it
/// is asked: a conductor that resumes, or a new one, always gets the token of the work already handed
/// out. A token is spent by one successful report, and tokens of directives that no longer exist are
/// dropped.
fn sync_tokens(p: &Project, v: &mut Value) -> R<()> {
    let _tl = crate::lock::tokens(p)?;
    let mut tokens = read_tokens(p);
    // Old format (hash -> {unit, role}) is dropped.
    if tokens
        .as_object()
        .map(|o| o.values().any(|x| x.is_object()))
        .unwrap_or(false)
    {
        tokens = json!({});
    }
    let mut fresh = json!({});
    if let Some(ds) = v["directives"].as_array_mut() {
        for d in ds.iter_mut() {
            let role = d["role"].as_str().unwrap_or("").to_string();
            // A verifier's directive belongs to a checkpoint; the others to a unit.
            let unit = d["unit"]
                .as_str()
                .or_else(|| d["checkpoint"].as_str())
                .unwrap_or("")
                .to_string();
            // While a check of a checkpoint is marked failed or unreachable the human decides; the
            // verifier who is asked to look again still needs its token, so it stays on that directive.
            let (role, field) = if d["action"] == "decide-check" {
                ("verifier".to_string(), "verifier_token")
            } else {
                (role, "token")
            };
            if (matches!(role.as_str(), "builder" | "reviewer" | "verifier")
                || crate::roles::is_new_role(&role))
                && !unit.is_empty()
            {
                let key = format!("{unit}:{role}");
                let tok = match tokens.get(&key).and_then(|t| t.as_str()) {
                    Some(t) => t.to_string(),
                    None => util::random_hex(12)?,
                };
                d[field] = json!(tok);
                fresh[&key] = json!(tok);
            }
        }
    }
    // A unit held by an open question has no directive, but its worker's token stays: the same token
    // is given every time, until a report spends it.
    let held: Vec<String> = p
        .list_records(schema::kind_by_name("question"))?
        .into_iter()
        .filter(|q| !s(q, "unit").is_empty() && q.get("answer").is_none())
        .map(|q| s(&q, "unit").to_string())
        .collect();
    if let Some(o) = tokens.as_object() {
        for (k, t) in o {
            let keep = k
                .split_once(':')
                .map(|(u, r)| {
                    held.iter().any(|h| h == u)
                        && (matches!(r, "builder" | "reviewer" | "verifier")
                            || crate::roles::is_new_role(r))
                })
                .unwrap_or(false);
            if keep && fresh.get(k).is_none() {
                fresh[k] = t.clone();
            }
        }
    }
    // Tokens handed out by `bf roles token` stay until spent.
    if let Some(o) = tokens.as_object() {
        for (k, t) in o {
            let is_new = k
                .split_once(':')
                .map(|(_, r)| crate::roles::is_new_role(r))
                .unwrap_or(false);
            if is_new && fresh.get(k).is_none() {
                fresh[k] = t.clone();
            }
        }
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&fresh)?.as_bytes())?;
    Ok(())
}

pub(crate) fn check_token(p: &Project, unit: &str, role: &str, token: Option<&str>) -> R<String> {
    let key = format!("{unit}:{role}");
    let have = read_tokens(p)
        .get(&key)
        .and_then(|t| t.as_str())
        .map(|t| t.to_string());
    if let (Some(h), Some(t)) = (&have, token) {
        if h == t.trim() {
            return Ok(key);
        }
    }
    Err(refuse(
        format!("report on {unit} as {role}"),
        format!("a report needs the token of the {role}'s directive for {unit}; none, or the wrong one, was given"),
        &[
            "The conductor runs `bf next --json`; each builder and reviewer directive carries its `token`.",
            "The worker reports with `--token <token>`. A builder cannot report as the reviewer: the reviewer's directive exists only while the unit is in review.",
        ],
        "`bf status` shows the unit in its new state.",
    ))
}

pub(crate) fn spend_token(p: &Project, key: &str) -> R<()> {
    let _tl = crate::lock::tokens(p)?;
    let mut v = read_tokens(p);
    if let Some(o) = v.as_object_mut() {
        o.remove(key);
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(())
}

/// The next directives, plus a plain statement of whether anything can start and, if not, why.
pub fn next(p: &Project) -> R<Value> {
    // The idle check runs here, so nobody has to remember `bf stuck`.
    if events::replay(p)?.active_run().is_some() {
        stuck(p, false)?;
    }
    // A checkpoint whose check failed gets a fix for a builder, not a question to the human.
    if events::replay(p)?.active_run().is_some() {
        crate::ckfix::maybe_open(p)?;
        // A wave check that failed gets a fix too, held to the units that landed since the last pass.
        crate::wavecheck::maybe_open(p)?;
    }
    let mut v = next_raw(p)?;
    if p.policy.roles.tokens {
        sync_tokens(p, &mut v)?;
    }
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let dirs = v["directives"].as_array().cloned().unwrap_or_default();
    let can_start = dirs.iter().any(|d| d["role"] != "human");
    let human_needed = dirs.iter().any(|d| d["role"] == "human");
    let all_landed = !pl.units.is_empty()
        && pl
            .units
            .iter()
            .all(|u| st.unit_state(s(u, "id")) == "landed");
    let all_closed = pl
        .checkpoints
        .iter()
        .all(|k| st.closed.contains_key(s(k, "id")));
    let reason = if can_start {
        Value::Null
    } else if all_landed && all_closed {
        json!("run done: every unit has landed and every checkpoint is closed")
    } else if human_needed {
        let parked: Vec<String> = st
            .units
            .iter()
            .filter(|(_, u)| matches!(u.state.as_str(), "parked" | "blocked"))
            .map(|(id, u)| format!("{id} ({})", u.state))
            .collect();
        if let Some(d) = dirs.iter().find(|d| d["action"] == "decide-check") {
            json!(format!(
                "a check of {} failed or is unreachable; the human decides",
                d["checkpoint"].as_str().unwrap_or("a checkpoint")
            ))
        } else if parked.is_empty() {
            json!("waiting on a human step; see the human directives")
        } else {
            json!(format!(
                "units parked or blocked: {}; the human decides",
                parked.join(", ")
            ))
        }
    } else {
        let w: Vec<String> = v["waiting"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        json!(format!(
            "nothing is ready: {}",
            if w.is_empty() {
                "no unit is waiting either".into()
            } else {
                w.join("; ")
            }
        ))
    };
    // A directive that waits on the human for a gate or for questions carries the page's link when one
    // was recorded.
    if let Some(ds) = v["directives"].as_array_mut() {
        for d in ds.iter_mut() {
            if d["role"] != "human" {
                continue;
            }
            let page = match d["action"].as_str().unwrap_or("") {
                "decide-gate" => d["gate"].as_str().map(|g| format!("gate-{g}")),
                "answer-question" => Some("questions".to_string()),
                _ => None,
            };
            if let Some((url, _)) = page.and_then(|n| st.pages.get(&n)) {
                d["page_url"] = json!(url);
            }
        }
    }
    v["main_ahead"] = json!(main_ahead(p, &st, &pl));
    v["human_turns_note"] = json!(st.human_turns_note().unwrap_or_default());
    if let Some(n) = st.human_turns_note() {
        eprintln!("note: {n}");
    }
    v["can_start"] = json!(can_start);
    v["human_needed"] = json!(human_needed);
    v["idle_reason"] = reason;
    Ok(v)
}

/// One of the five new roles reports with its own token; the unit's state does not move. The design
/// stage (`design`) needs no run and no plan; a unit or a checkpoint needs a run. A role reports only
/// on a subject at its stage.
fn role_report(
    p: &Project,
    subject: &str,
    result: &str,
    r: &str,
    note: &str,
    cost: f64,
    token: Option<&str>,
) -> R<Value> {
    let st = events::require_chain(p)?;
    if subject != crate::roles::DESIGN {
        need_run(&st)?;
    }
    if !["built", "blocked"].contains(&result) {
        return Err(refuse(
            format!("report {result} as {r}"),
            "a new role reports `built` (done) or `blocked`",
            &["Use --result built or --result blocked; findings and threats are filed with `bf record put`."],
            "The command prints the unit's state.",
        ));
    }
    crate::roles::check_subject(p, subject, r)?;
    if p.policy.roles.tokens {
        let key = check_token(p, subject, r, token)?;
        spend_token(p, &key)?;
    }
    events::append(
        p,
        "unit.role-report",
        json!({"unit": subject, "result": result, "role": r, "note": note, "cost": cost.max(0.0)}),
    )?;
    let state = if subject == crate::roles::DESIGN {
        String::new()
    } else {
        st.unit_state(subject).to_string()
    };
    Ok(json!({"unit": subject, "result": result, "state": state, "role": r}))
}

/// A role's token ends with the run it was issued in: starting or stopping a run drops every token
/// of the new roles (a design-stage token is dropped when the first run starts).
fn drop_role_tokens(p: &Project) -> R<()> {
    let _tl = crate::lock::tokens(p)?;
    let mut v = read_tokens(p);
    if let Some(o) = v.as_object_mut() {
        o.retain(|k, _| {
            !k.split_once(':')
                .map(|(_, r)| crate::roles::is_new_role(r))
                .unwrap_or(false)
        });
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn report(
    p: &Project,
    unit: &str,
    result: &str,
    role: Option<&str>,
    note: &str,
    findings: Option<&Value>,
    cost: f64,
    token: Option<&str>,
) -> R<Value> {
    if let Some(r) = role.filter(|r| crate::roles::is_new_role(r)) {
        return role_report(p, unit, result, r, note, cost, token);
    }
    if result == "scope" && !unit.starts_with("K-") {
        return crate::scope::verdict(p, unit, role, note, findings, cost, token);
    }
    if unit.starts_with("K-") {
        let st = events::require_chain(p)?;
        if crate::ckreview::wants(p, &st, unit, role) {
            return crate::ckreview::report(p, unit, result, note, findings, token);
        }
        if crate::ckfix::wants(&st, unit, result, role) {
            return crate::ckfix::report(p, unit, result, role, note, findings, token);
        }
        return sync_report(p, unit, result, role, note, findings, token);
    }
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let pl = plan::load(p)?;
    if pl.unit(unit).is_none() {
        return Err(refuse(
            format!("report on {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "`bf status` lists the unit.",
        ));
    }
    if !["built", "findings", "fixed", "blocked"].contains(&result) {
        return Err(refuse(
            format!("report `{result}`"),
            "that is not a result the engine knows",
            &["Use one of: built, findings, fixed, blocked."],
            "The command prints the unit's new state.",
        ));
    }
    let cur = st.unit_state(unit).to_string();
    if crate::scope::open_request(&st, unit).is_some() {
        return Err(refuse(
            format!("report {result} on {unit}"),
            "a request of this unit for files is with the scope reviewer; the unit goes on when the verdict is in",
            &["`bf next` shows the review-scope directive; its reviewer reports the verdict."],
            "`bf status` shows the unit without an open request.",
        ));
    }
    let expected_role = if cur == "review" {
        "reviewer"
    } else {
        "builder"
    };
    // A unit in review is judged by the reviewer, who must say so: a builder's repeated `built`
    // must never read as a clean review.
    if cur == "review" && role != Some("reviewer") {
        return Err(refuse(
            format!("report {result} on {unit}"),
            format!("{unit} is waiting for its reviewer; reports on it need `--role reviewer`"),
            &["The cold reviewer reports with `bf report <unit> --result findings|built --role reviewer`.", "A builder waits until the unit comes back as `fixing`."],
            "`bf status` shows the unit in its new state.",
        ));
    }
    if let Some(r) = role {
        if r != expected_role && !cur.is_empty() {
            return Err(refuse(
                format!("report as {r}"),
                format!("{unit} is `{cur}`, so the next report comes from the {expected_role}"),
                &["Report with the role the unit is waiting for."],
                "The command prints the unit's new state.",
            ));
        }
    }
    // Findings given in a file, and findings already filed with `bf record put finding`.
    let listed: Vec<Value> = findings
        .and_then(|f| {
            f.as_array()
                .cloned()
                .or_else(|| f.get("findings").and_then(|x| x.as_array().cloned()))
        })
        .unwrap_or_default();
    let open_filed = p
        .list_records(schema::kind_by_name("finding"))?
        .iter()
        .filter(|f| s(f, "unit") == unit && is_open(f))
        .count();
    // A reviewer with nothing to report says `findings` with no finding on file: the unit moves on.
    let result = if result == "findings" && cur == "review" && listed.is_empty() && open_filed == 0
    {
        "built"
    } else {
        result
    };
    let token_hash = if p.policy.roles.tokens && !cur.is_empty() {
        Some(check_token(p, unit, expected_role, token)?)
    } else {
        None
    };
    let review_required = p.policy.review.required;
    let mut event_kind = "unit.report";
    let to: String = match (cur.as_str(), result) {
        ("running", "built") | ("fixing", "built") | ("fixing", "fixed") => {
            if review_required {
                "review".into()
            } else {
                "reviewed".into()
            }
        }
        ("running" | "fixing", "blocked") => "blocked".into(),
        ("review", "findings") => {
            let rounds = st.units.get(unit).map(|u| u.rounds).unwrap_or(0) + 1;
            if rounds > p.policy.limits.max_rounds {
                event_kind = "unit.report+park";
                "parked".into()
            } else {
                "fixing".into()
            }
        }
        ("review", "built") => "reviewed".into(),
        (c, r) => {
            let why = match c {
                "" => format!("{unit} has not been started"),
                "reviewed" | "landed" => format!("{unit} is already `{c}`"),
                "blocked" | "parked" => format!("{unit} is `{c}` and waits for the human"),
                _ => format!("a `{r}` report does not fit a unit that is `{c}`"),
            };
            return Err(refuse(
                format!("report {r} on {unit}"),
                why,
                &["Check the state with `bf status`.", "Builders report built, fixed or blocked; reviewers report findings or built (clean)."],
                "`bf status` shows the unit in its new state.",
            ));
        }
    };
    // Findings.
    let mut finding_ids: Vec<String> = Vec::new();
    if result == "findings" {
        let kind = schema::kind_by_name("finding").unwrap();
        for f in listed {
            let id = p.next_id(kind)?;
            let mut rec = json!({"id": id, "kind": "finding", "unit": unit, "outcome": "open"});
            for k in ["title", "severity", "where", "action", "text"] {
                if let Some(v) = f.get(k) {
                    rec[k] = v.clone();
                }
            }
            schema::validate_or_refuse(kind, &rec)?;
            let id = p.write_numbered(kind, &mut rec)?;
            finding_ids.push(id);
        }
    }
    let findings_all = p.list_records(schema::kind_by_name("finding"))?;
    let mine: Vec<&Value> = findings_all
        .iter()
        .filter(|f| s(f, "unit") == unit)
        .collect();
    if cur == "review" && result == "built" {
        let open: Vec<&str> = mine
            .iter()
            .filter(|f| is_open(f))
            .map(|f| s(f, "id"))
            .collect();
        if !open.is_empty() {
            return Err(refuse(
                format!("report {unit} clean"),
                format!("findings are still open: {}", open.join(", ")),
                &[
                    "Have the builder fix them and report `fixed`.",
                    "Or have the human accept them (outcome accepted).",
                ],
                "`bf report` is accepted and the unit is `reviewed`.",
            ));
        }
    }
    if cur == "fixing" {
        for f in mine.iter().filter(|f| is_open(f)) {
            let mut r = (*f).clone();
            r["outcome"] = json!("fixed");
            p.write_record(&r)?;
        }
    }
    let cost_total = cost.max(0.0);
    if let Some(h) = &token_hash {
        spend_token(p, h)?;
    }
    let ev_kind = "unit.report";
    let _ = event_kind;
    let mut ev = json!({"unit": unit, "result": result, "role": expected_role, "from": cur, "to": to, "note": note, "cost": cost_total, "findings": finding_ids});
    if cur == "review" {
        // The round this report closes: its number, the content it judged and what it covered.
        let round = st.units.get(unit).map(|u| u.rounds).unwrap_or(0) + 1;
        let wc = p.work_dir().join(unit);
        ev["round"] = json!(round);
        ev["content"] = json!(p.content_hash(&wc).unwrap_or_default());
        ev["head"] = json!(gitx::must(&wc, &["rev-parse", "HEAD"]).unwrap_or_default());
        ev["covered"] = json!(if st
            .units
            .get(unit)
            .map(|u| u.review_rounds.is_empty())
            .unwrap_or(true)
        {
            "everything in its scope".to_string()
        } else {
            "the earlier findings and what changed since the round before".to_string()
        });
    }
    events::append(p, ev_kind, ev)?;
    if to == "parked" {
        events::append(
            p,
            "unit.park",
            json!({"unit": unit, "reason": format!("more than {} review rounds", p.policy.limits.max_rounds)}),
        )?;
    }
    p.write_guard()?;
    Ok(json!({"unit": unit, "result": result, "state": to, "findings": finding_ids}))
}

fn brief_checkpoint(p: &Project, k: &str, role: &str) -> R<PathBuf> {
    if matches!(role, "builder" | "reviewer") {
        if let Some(so) = events::replay(p)?.sync_open.filter(|o| o.checkpoint == k) {
            return brief_sync(p, &so, role);
        }
        if let Some(f) = events::replay(p)?.ck_fix.filter(|f| f.checkpoint == k) {
            return crate::ckfix::brief(p, &f, role);
        }
    }
    if role == "reviewer" {
        return crate::ckreview::brief(p, k);
    }
    if role != "verifier" {
        return Err(refuse(
            format!("write a brief for {k} as {role}"),
            "a checkpoint is briefed to its verifier only",
            &["Use --role verifier, or give a unit ID."],
            "The command prints the path of the brief.",
        ));
    }
    let pl = plan::load(p)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(refuse(
            format!("brief {k}"),
            "no such checkpoint",
            &["Use a K- ID from `bf record list --kind checkpoint`."],
            "The command prints a path.",
        ));
    };
    let mut md = format!(
        "# Brief: {k} as verifier\n\nCheckpoint: {}\n\n",
        s(kr, "title")
    );
    md.push_str("You are a blind verifier: judge the built result against the contract below. Do not read the source.\n\n## What this checkpoint proves\n\n");
    for id in str_list(kr, "proves") {
        if let Some(r) = p.get_record(&id)? {
            md.push_str(&format!("### {} {}\n\n", id, s(&r, "title")));
            if !s(&r, "text").is_empty() {
                md.push_str(&format!("{}\n\n", s(&r, "text")));
            }
            for a in str_list(&r, "acceptance") {
                md.push_str(&format!("- Acceptance: {a}\n"));
            }
            md.push('\n');
        }
    }
    md.push_str("## Checks\n\n");
    let wc = p.work_dir().join(k);
    for cid in str_list(kr, "checks") {
        if let Some(c) = pl.checks.iter().find(|c| s(c, "id") == cid) {
            let cmd = match c.get("command") {
                Some(Value::String(x)) => x.clone(),
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            md.push_str(&format!(
                "- {} ({}), proves {}: ",
                cid,
                s(c, "title"),
                str_list(c, "proves").join(", ")
            ));
            if s(c, "how") == "command" {
                md.push_str(&format!(
                    "run `bf evidence run {cid} -- {cmd}` inside `{}`\n",
                    wc.display()
                ));
            } else {
                md.push_str(&format!("{} check: store the proof with `bf evidence attach {cid} <file>` inside `{}`\n", s(c, "how"), wc.display()));
            }
        }
    }
    if !s(kr, "human_step").is_empty() {
        md.push_str(&format!("\n## Human step\n\n{}\n", s(kr, "human_step")));
    }
    md.push_str(&format!(
        "\n## Outcomes\n\nRun each check from inside `{wc}` so the receipt is for the checkpoint's content. Then, for each ID you judged:\n\n- met: the receipt from `bf evidence run` is the proof.\n- failed: the check ran and does not match the contract, or it fails without a receipt you can show: `bf evidence fail <check> --reason \"what you saw\"`.\n- unreachable: you could not run it: `bf evidence unreachable <check> --reason \"what blocked you\"`. This is not a pass. The checkpoint then waits for the human.\n\nGive your outcomes in your last message too.\n\n## Rules\n\n- Never edit anything under `factory/` or `.bf/` (the working copy named above is the only place under `.bf/` you may use).\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot judge without the human's decision, ask: `bf ask --title \"<the question>\"`. This checkpoint then waits for the answer.\n",
        wc = wc.display()
    ));
    let path = p.briefs_dir().join(format!("{k}-verifier.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

pub fn brief(p: &Project, unit: &str, role: &str) -> R<PathBuf> {
    if unit.starts_with("K-") {
        return brief_checkpoint(p, unit, role);
    }
    if crate::roles::is_new_role(role) {
        return crate::roles::brief(p, unit, role);
    }
    if !["builder", "reviewer", "verifier"].contains(&role) {
        return Err(refuse(
            format!("write a brief for role `{role}`"),
            "that is not a role the engine briefs",
            &["Use --role builder, reviewer, verifier, uiux-verifier, security-expert, security-auditor, lawyer or legal-auditor."],
            "The command prints the path of the brief.",
        ));
    }
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("brief {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints a path.",
        ));
    };
    let st = events::replay(p)?;
    if role == "reviewer" && crate::scope::open_request(&st, unit).is_some() {
        return crate::scope::brief_review(p, unit);
    }
    let mut md = String::new();
    md.push_str(&format!("# Brief: {} as {}\n\n", unit, role));
    md.push_str(&format!("Unit: {}\n\n", s(u, "title")));
    if !s(u, "text").is_empty() {
        md.push_str(&format!("{}\n\n", s(u, "text")));
    }
    md.push_str("## What this unit delivers\n\n");
    for id in str_list(u, "delivers") {
        if let Some(r) = p.get_record(&id)? {
            md.push_str(&format!("### {} {}\n\n", id, s(&r, "title")));
            if !s(&r, "text").is_empty() {
                md.push_str(&format!("{}\n\n", s(&r, "text")));
            }
            for a in str_list(&r, "acceptance") {
                md.push_str(&format!("- Acceptance: {a}\n"));
            }
            md.push('\n');
        }
    }
    let received: Vec<&Value> = pl.handoffs.iter().filter(|h| s(h, "to") == unit).collect();
    if !received.is_empty() {
        md.push_str("## Contracts you receive\n\n");
        for h in received {
            md.push_str(&format!(
                "- {}: {} from {} ({})\n",
                s(h, "id"),
                s(h, "thing"),
                s(h, "from"),
                s(h, "contract")
            ));
        }
        md.push('\n');
    }
    let given: Vec<&Value> = pl
        .handoffs
        .iter()
        .filter(|h| s(h, "from") == unit)
        .collect();
    if !given.is_empty() {
        md.push_str("## Contracts you hand over\n\n");
        for h in given {
            md.push_str(&format!(
                "- {}: {} to {} ({})\n",
                s(h, "id"),
                s(h, "thing"),
                s(h, "to"),
                s(h, "contract")
            ));
        }
        md.push('\n');
    }
    md.push_str("## Checks\n\n");
    for c in pl.unit_checks(unit) {
        let cmd = match c.get("command") {
            Some(Value::String(x)) => x.clone(),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        md.push_str(&format!(
            "- {} ({}): `{}`. Run it with `bf evidence run {} --unit {} -- {}`\n",
            s(c, "id"),
            s(c, "title"),
            cmd,
            s(c, "id"),
            unit,
            cmd
        ));
    }
    // The checks every unit must pass: the engine attaches them, and the unit cannot land without them.
    let (attached, _, _) = crate::checks::attached_for_unit(p, &pl, &st, unit);
    for (a, line) in attached
        .iter()
        .zip(crate::checks::attached_lines(p, &pl, &st, unit))
    {
        md.push_str(&format!(
            "- {line}. Run it with `bf evidence run {} --unit {unit} -- {}`\n",
            a.id(),
            a.command_line()
        ));
    }
    md.push('\n');
    if role != "verifier" {
        md.push_str("## Where you work\n\n");
        md.push_str(&format!(
            "Working copy: `{}`\n\n",
            p.work_dir().join(unit).display()
        ));
        if p.workspace {
            if let Ok(r) = p.repo_of_unit(u) {
                md.push_str(&format!(
                    "Repository: `{}` (the working copy is a git worktree of {}; the paths below are relative to it)\n\n",
                    r.name,
                    r.path.display()
                ));
            }
        }
        md.push_str("Files you may change:\n");
        for f in crate::scope::unit_files(&st, u) {
            md.push_str(&format!("- `{f}`\n"));
        }
        md.push('\n');
        md.push_str(&crate::scope::undo_md(&st, unit));
        if role == "reviewer" {
            md.push_str(&crate::scope::grants_md_unit(&st, unit));
        }
    } else {
        md.push_str("## Where you work\n\nYou are a blind verifier: judge the built result against the contract above. Do not read the source.\n\n");
    }
    if let Some(a) = st
        .units
        .get(unit)
        .map(|x| x.reopened_by.clone())
        .filter(|a| !a.is_empty())
    {
        md.push_str(&format!("## Amendment {a}\n\nThis unit was sent back to work because the human approved amendment {a}. The records it delivers changed: read them again (`bf record get <ID>`) and change what no longer fits. The receipts for its checks from before the amendment no longer count; run the checks again.\n\n"));
    }
    let qs = p.list_records(schema::kind_by_name("question"))?;
    let asked: Vec<&Value> = qs.iter().filter(|q| s(q, "unit") == unit).collect();
    if !asked.is_empty() {
        md.push_str("## Questions you asked the human\n\n");
        for q in asked {
            match q.get("answer") {
                Some(_) => md.push_str(&format!(
                    "- {} {}: answered: {}\n",
                    s(q, "id"),
                    s(q, "title"),
                    crate::store::answer_text(q)
                )),
                None => md.push_str(&format!(
                    "- {} {}: not answered yet; do not guess, wait for the answer\n",
                    s(q, "id"),
                    s(q, "title")
                )),
            }
        }
        md.push('\n');
    }
    let fs_all = p.list_records(schema::kind_by_name("finding"))?;
    let mine: Vec<&Value> = fs_all.iter().filter(|f| s(f, "unit") == unit).collect();
    if !mine.is_empty() {
        md.push_str("## Findings\n\n");
        for f in mine {
            md.push_str(&format!(
                "- {} [{}] {} at {}: {} ({})\n",
                s(f, "id"),
                s(f, "severity"),
                s(f, "title"),
                s(f, "where"),
                s(f, "action"),
                s(f, "outcome")
            ));
        }
        md.push('\n');
    }
    if role == "reviewer" {
        md.push_str("## Receipts\n\nVerify these instead of re-running the checks:\n\n");
        for r in evidence::load_receipts(p)? {
            if pl.unit_checks(unit).iter().any(|c| s(c, "id") == r.check())
                || (r.check().starts_with(crate::checks::ATTACHED_PREFIX)
                    && s(&r.v, "unit") == unit)
            {
                md.push_str(&format!(
                    "- {} for {}: sealed={}, exit={}, content {}\n",
                    r.id(),
                    r.check(),
                    r.seal_ok,
                    r.v["exit"],
                    r.content()
                ));
            }
        }
        md.push('\n');
    }
    if role == "reviewer" {
        let wc = p.work_dir().join(unit);
        let prior: Vec<Value> = st
            .units
            .get(unit)
            .map(|x| x.review_rounds.clone())
            .unwrap_or_default();
        md.push_str(&format!("## Review round {}\n\n", prior.len() + 1));
        match prior.last() {
            None => md.push_str("This is the first round and it is cold: no earlier report is given. Read everything in the unit's scope.\n\n"),
            Some(l) => {
                md.push_str("This is a later round. The findings above are every earlier finding with its outcome: check that each open one is fixed, and start from the changes made since the content the round before judged. Do not review the whole scope again when nothing points there. You may file a new finding on what changed, on something a fix broke, or on anything you came across while following a change.\n\n");
                let head = l["head"].as_str().unwrap_or("");
                if !head.is_empty() {
                    let d = gitx::must(&wc, &["diff", head]).unwrap_or_default();
                    let lines: Vec<&str> = d.lines().collect();
                    md.push_str(&format!(
                        "### Changes since round {}\n\n```diff\n{}\n```\n\n",
                        l["round"],
                        if lines.len() > 400 { format!("{}\n... ({} more lines)", lines[..400].join("\n"), lines.len() - 400) } else { d.clone() }
                    ));
                }
            }
        }
        md.push_str("In any round you may read outside the changes, anywhere in the working copy, when you need to in order to judge them: what a change touches, calls or could break, and the code it relies on.\n\n");
    }
    md.push_str("## Limits\n\n");
    md.push_str(&format!(
        "- Review rounds: up to {} (so far {}).\n- Idle limit: {} minutes.\n",
        p.policy.limits.max_rounds,
        st.units.get(unit).map(|x| x.rounds).unwrap_or(0),
        p.policy.limits.idle_minutes
    ));
    md.push_str(&format!(
        "\n## Rules\n\n- Report with `bf report`.\n- Work only inside your working copy, `{}`. It lives under the engine's `.bf/work/` folder on purpose; do not touch anything else under `.bf/`, and leave the `factory/` folder inside your working copy alone: records change through the engine only.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot go on without the human's decision, ask: `bf ask --title \"<the question>\" --context \"<why>\" [--options \"A=first;B=second\"]`. Your unit then waits for the answer and the others go on; the answer is in your next brief.\n- A check counts only when `bf evidence run` ran it.\n",
        p.work_dir().join(unit).display()
    ));
    let path = p.briefs_dir().join(format!("{unit}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// The checkpoint that comes just before `k` in plan order, if any.
pub fn previous_checkpoint(pl: &Plan, k: &str) -> Option<String> {
    let i = pl.checkpoints.iter().position(|c| s(c, "id") == k)?;
    if i == 0 {
        None
    } else {
        Some(s(&pl.checkpoints[i - 1], "id").to_string())
    }
}

/// The repositories a checkpoint's units change, in the workspace's order.
pub fn checkpoint_repos<'a>(p: &'a Project, pl: &Plan, k: &str) -> R<Vec<&'a Repo>> {
    let mut out: Vec<&Repo> = Vec::new();
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let r = p.repo_of_unit(u)?;
        if !out.iter().any(|x| x.name == r.name) {
            out.push(r);
        }
    }
    if out.is_empty() {
        if let Some(r) = p.repos.first() {
            out.push(r);
        }
    }
    out.sort_by_key(|r| p.repos.iter().position(|x| x.name == r.name));
    Ok(out)
}

/// Where a checkpoint's working copy of a repository lives: the checkpoint's folder itself when its
/// units change one repository, otherwise a subfolder named for each repository.
pub fn checkpoint_wt(p: &Project, pl: &Plan, k: &str, repo: &Repo) -> R<PathBuf> {
    let root = p.work_dir().join(k);
    Ok(if checkpoint_repos(p, pl, k)?.len() <= 1 {
        root
    } else {
        root.join(&repo.name)
    })
}

fn ensure_checkpoint_worktree(p: &Project, k: &str, repo: &Repo) -> R<PathBuf> {
    let main = &repo.branch;
    // Each checkpoint builds on the one before it in plan order (the first on the main branch), so the
    // last checkpoint's content holds everything and the final verification runs over the whole result.
    let pl = plan::load(p)?;
    let st = events::replay(p)?;
    let prev = previous_checkpoint(&pl, k).filter(|c| st.closed.contains_key(c));
    let base = match &prev {
        Some(c) if gitx::branch_exists(&repo.path, &p.checkpoint_branch(c)) => {
            p.checkpoint_branch(c)
        }
        _ => main.clone(),
    };
    if !gitx::branch_exists(&repo.path, main) {
        return Err(refuse(
            "create a working copy",
            format!("the main branch `{main}` of the repository `{}` does not exist or has no commit", repo.name),
            &[
                "Make an initial commit on the main branch.",
                "Or set the branch the project files name (`main_branch` in factory/config.toml or factory/workspace.toml) to the branch you use.",
            ],
            "`git rev-parse <branch>` prints a commit.",
        ));
    }
    let cpb = p.checkpoint_branch(k);
    if !gitx::branch_exists(&repo.path, &cpb) {
        gitx::must(&repo.path, &["branch", &cpb, &base])?;
    }
    let wt = checkpoint_wt(p, &pl, k, repo)?;
    if !wt.join(".git").exists() {
        if let Some(parent) = wt.parent() {
            fs::create_dir_all(parent)?;
        }
        gitx::must(
            &repo.path,
            &["worktree", "add", &wt.to_string_lossy(), &cpb],
        )?;
    }
    // Work that landed in the earlier checkpoint after this one was cut (an amendment re-opened it) is
    // brought in before more lands here.
    if let Some(c) = &prev {
        let pb = p.checkpoint_branch(c);
        if gitx::branch_exists(&repo.path, &pb) && !gitx::is_ancestor(&repo.path, &pb, &cpb) {
            let out = gitx::run(
                &wt,
                &[
                    "merge",
                    "--no-ff",
                    "--no-edit",
                    "-m",
                    &format!("Bring in {c}"),
                    &pb,
                ],
                &[],
            )?;
            if !out.ok {
                let _ = gitx::run(&wt, &["merge", "--abort"], &[]);
                return Err(refuse(
                    format!("bring {c} into {k}"),
                    format!("the merge conflicts: {}", out.stderr.lines().next().unwrap_or("")),
                    &["The human resolves the conflict between the two checkpoint branches by hand."],
                    "`bf unit land` merges.",
                ));
            }
        }
    }
    Ok(wt)
}

pub fn unit_start(p: &Project, unit: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("start {unit}"))?;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    budget_check(p, &st)?;
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("start {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints a working copy path.",
        ));
    };
    if !st.gate_passed("plan", None) {
        return Err(refuse(
            format!("start {unit}"),
            "the plan gate has not been approved",
            &[
                "Open the gate with `bf gate open plan` and show the plan page.",
                "The human approves in chat; then run `bf gate approve plan`.",
            ],
            "`bf gate status` shows the plan gate approved.",
        ));
    }
    gates::refuse_if_drift(p, &st, &format!("start {unit}"))?;
    if st.unit_state(unit).is_empty() {
        if let Some((_, command, what)) = unanswered_questions(&st).first() {
            return Err(refuse(
                format!("start {unit}"),
                format!("the run has not answered the question {what}: no unit starts until both plan questions are answered"),
                &[&format!("Run `{command}`, ask the human, then `propose` and `set --quote \"<their words>\"`; `bf next` lists both questions."), "Then start the unit again."],
                "`bf status` shows both plan questions answered.",
            ));
        }
    }
    if let Some(so) = &st.sync_open {
        return Err(refuse(
            format!("start {unit}"),
            format!("a sync of the main branch into {} is open ({}): no unit starts until it is settled", so.checkpoint, so.phase),
            &["`bf next` gives the directive that settles it.", &format!("To go back to how it was: `bf checkpoint sync {} --abort`.", so.checkpoint)],
            "`bf status` shows no open sync.",
        ));
    }
    let cur = st.unit_state(unit).to_string();
    // Starting a unit that is already in flight only makes sure its working copy exists.
    if !cur.is_empty() && !matches!(cur.as_str(), "running" | "review" | "fixing" | "reviewed") {
        return Err(refuse(
            format!("start {unit}"),
            format!("it is already `{cur}`"),
            &["Use `bf next` to see what is waiting."],
            "`bf status` shows the unit in its state.",
        ));
    }
    if cur.is_empty() {
        let receipts = evidence::load_receipts(p)?;
        if let Some(why) = plan::not_ready(p, &pl, &st, unit, &receipts) {
            return Err(refuse(
                format!("start {unit}"),
                why,
                &["Land the units it waits for first (`bf next` lists them)."],
                "`bf next` lists the unit as ready.",
            ));
        }
    }
    let k = s(u, "checkpoint").to_string();
    let repo = p.repo_of_unit(u)?.clone();
    let cpb = p.checkpoint_branch(&k);
    ensure_checkpoint_worktree(p, &k, &repo)?;
    let ub = p.unit_branch(unit);
    let path = p.work_dir().join(unit);
    if !path.join(".git").exists() {
        if gitx::branch_exists(&repo.path, &ub) {
            gitx::must(
                &repo.path,
                &["worktree", "add", &path.to_string_lossy(), &ub],
            )?;
        } else {
            gitx::must(
                &repo.path,
                &["worktree", "add", "-b", &ub, &path.to_string_lossy(), &cpb],
            )?;
        }
    }
    crash_point("unit-start-after-git");
    if cur.is_empty() {
        let base_commit = gitx::must(&repo.path, &["rev-parse", &ub])?;
        events::append(
            p,
            "unit.start",
            json!({"unit": unit, "repo": repo.name, "branch": ub, "path": path.to_string_lossy(), "base": cpb, "base_commit": base_commit}),
        )?;
    }
    p.write_guard()?;
    Ok(
        json!({"unit": unit, "repo": repo.name, "working_copy": path.to_string_lossy(), "branch": ub, "base": cpb, "resumed": !cur.is_empty()}),
    )
}

pub fn unit_land(p: &Project, unit: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("land {unit}"))?;
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("land {unit}"))?;
    refuse_if_main_moved(p, &st)?;
    let run = need_run(&st)?;
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("land {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints the merge.",
        ));
    };
    let cur = st.unit_state(unit).to_string();
    if cur == "landed" {
        return Ok(json!({"unit": unit, "landed": true, "already": true}));
    }
    if cur != "reviewed" {
        return Err(refuse(
            format!("land {unit}"),
            format!(
                "it is `{}`; only a reviewed unit lands",
                if cur.is_empty() { "not started" } else { &cur }
            ),
            &[
                "Build it, have it reviewed clean, and report both with `bf report`.",
                "`bf next` shows what is waiting.",
            ],
            "`bf status` shows the unit `reviewed`.",
        ));
    }
    let open: Vec<String> = p
        .list_records(schema::kind_by_name("finding"))?
        .iter()
        .filter(|f| s(f, "unit") == unit && is_open(f))
        .map(|f| s(f, "id").to_string())
        .collect();
    if !open.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            format!("open findings: {}", open.join(", ")),
            &["Fix them and have the reviewer report clean."],
            "No open findings remain.",
        ));
    }
    if let Some(why) = crate::wavecheck::land_blocker(&pl, &st, unit) {
        return Err(refuse(
            format!("land {unit}"),
            why,
            &[
                "`bf next` offers the wave check step, or the fix of a failed one.",
                "The unit may be built and reviewed meanwhile; only its landing waits.",
            ],
            "`bf status` shows the wave check passed.",
        ));
    }
    let k = s(u, "checkpoint").to_string();
    let repo = p.repo_of_unit(u)?.clone();
    let cpb = p.checkpoint_branch(&k);
    let ub = p.unit_branch(unit);
    let wt = p.work_dir().join(unit);
    if !wt.join(".git").exists() {
        return Err(refuse(
            format!("land {unit}"),
            "its working copy is missing",
            &["Run `bf unit start` to recreate it."],
            "The working copy exists.",
        ));
    }
    let checks = pl.unit_checks(unit);
    if checks.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            "the unit has no check, so nothing could prove it",
            &["Add a check record with `unit = \"<id>\"` through the plan."],
            "`bf plan check` passes.",
        ));
    }
    let content = p.content_hash(&wt)?;
    let receipts = evidence::load_receipts(p)?;
    let mut missing: Vec<String> = Vec::new();
    for c in &checks {
        let cid = s(c, "id");
        if evidence::satisfied_unit_at(&receipts, cid, &content).is_none() {
            let stale = receipts.iter().any(|r| r.check() == cid && r.passes());
            missing.push(format!(
                "{cid}: {} - run `bf evidence run {cid} --unit {unit} -- <its command>` inside {}",
                if stale {
                    "the receipts on file are for other content"
                } else {
                    "no passing sealed receipt"
                },
                wt.display()
            ));
        }
    }
    // R-76: the checks every unit must pass apply beside the unit's own, and no plan repeats them.
    missing.extend(crate::checks::attached_missing(
        p, &pl, &st, &receipts, unit, &content, &wt,
    ));
    if !missing.is_empty() {
        let mut steps = missing;
        steps.push(format!("Run `bf unit land {unit}` again."));
        return Err(refuse_owned(
            format!("land {unit}"),
            format!(
                "receipts do not match the content now in the working copy (hash {})",
                &content[..12.min(content.len())]
            ),
            steps,
            "`bf evidence verify --unit` prints every check ok.",
        ));
    }
    if gitx::has_uncommitted(&wt) {
        gitx::add_all_without_junk(&wt)?;
        gitx::must(
            &wt,
            &["commit", "-q", "-m", &format!("{unit}: {}", s(u, "title"))],
        )?;
    }
    // Scope: every changed path must be inside the unit's files.
    let diff = gitx::must(
        &repo.path,
        &["diff", "--name-only", &format!("{cpb}...{ub}")],
    )?;
    let scope = crate::scope::unit_files(&st, u);
    let outside: Vec<&str> = diff
        .lines()
        .filter(|f| !scope.iter().any(|g| util::path_matches(g, f)))
        .collect();
    if !outside.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            format!("the branch changes files outside the unit's scope: {}", outside.join(", ")),
            &["Revert those changes in the working copy (`git checkout -- <file>` or remove them), commit, and run the checks again.", "If the scope is wrong, the human changes the plan."],
            "`git diff --name-only` lists only files inside the scope.",
        ));
    }
    crate::scope::landing_check(&st, unit, &repo.path, &format!("{cpb}...{ub}"))?;
    let cpwt = ensure_checkpoint_worktree(p, &k, &repo)?;
    if !gitx::is_ancestor(&repo.path, &ub, &cpb) {
        let before = gitx::must(&cpwt, &["rev-parse", "HEAD"])?;
        let out = gitx::run(
            &cpwt,
            &[
                "merge",
                "--no-ff",
                "--no-edit",
                "-m",
                &format!("Land {unit}: {}", s(u, "title")),
                &ub,
            ],
            &[],
        )?;
        if !out.ok {
            // Put the working copy back exactly as it was before the merge started.
            let _ = gitx::run(&cpwt, &["merge", "--abort"], &[]);
            let _ = gitx::run(&cpwt, &["reset", "--hard", &before], &[]);
            let first = out.stderr.lines().next().unwrap_or("").to_string();
            let conflict = out.stdout.contains("CONFLICT")
                || out.stdout.contains("Automatic merge failed")
                || out.stderr.contains("CONFLICT");
            if !conflict {
                return Err(refuse(
                    format!("land {unit}"),
                    format!("git could not merge into {cpb}: {first}"),
                    &["The checkpoint's working copy was put back as it was. Run `bf unit land` again."],
                    "`bf unit land` prints the merge.",
                ));
            }
            return Err(refuse(
                format!("land {unit}"),
                format!("the merge into {cpb} conflicts: {first}"),
                &["Rebase or merge the checkpoint branch into the unit's working copy and resolve the conflict there.", "Run the checks again, then `bf unit land` again."],
                "`bf unit land` prints the merge.",
            ));
        }
    }
    crash_point("land-after-merge");
    let tip = gitx::head_short(&cpwt);
    events::append(
        p,
        "unit.land",
        json!({"unit": unit, "repo": repo.name, "checkpoint": k, "hash": content, "merge": tip, "run": run}),
    )?;
    p.write_guard()?;
    Ok(
        json!({"unit": unit, "landed": true, "checkpoint": k, "branch": cpb, "merge": tip, "content_hash": content}),
    )
}

/// Where the main branch stands against what the engine recorded in the log head.
pub struct MainPos {
    /// The repository this is about.
    pub repo: String,
    /// The branch is at a commit the engine did not put there and the human has not accepted.
    pub moved: bool,
    /// Why, in words: the work it holds that should not be there.
    pub detail: String,
    pub tip: String,
    /// A quiet line when the human's own commits moved the branch; it blocks nothing.
    pub note: String,
}

/// Compare the main branch with the commit the engine last put there (or the baseline set at `bf init`
/// or accepted by the human). A move by the human that only touches `factory/` and `.gitignore` is
/// taken as theirs (they commit their records). Anything else is a move to account for. Independently,
/// work of a unit that never landed, or landed in a test run, on the main branch is always one.
pub fn repo_main_position(p: &Project, st: &State, repo: &Repo) -> R<MainPos> {
    let main = &repo.branch;
    let rname = repo.name.clone();
    let ok = |tip: &str| MainPos {
        repo: rname.clone(),
        moved: false,
        detail: String::new(),
        tip: tip.to_string(),
        note: String::new(),
    };
    if !gitx::branch_exists(&repo.path, main) {
        // A branch that was there and is gone is a change to account for.
        if events::read_head(p)
            .and_then(|h| h.commit_of(&repo.name))
            .is_some()
        {
            return Ok(MainPos {
                repo: repo.name.clone(),
                moved: true,
                detail: format!("the main branch `{main}` does not exist any more"),
                tip: String::new(),
                note: String::new(),
            });
        }
        return Ok(ok(""));
    }
    let tip = gitx::must(&repo.path, &["rev-parse", main])?;
    let recorded = events::read_head(p).and_then(|h| h.commit_of(&repo.name));
    if recorded.as_deref() == Some(tip.as_str()) {
        return Ok(ok(&tip));
    }
    // Work that should never be on main, by whatever way it got there.
    for (id, u) in &st.units {
        let test_landed =
            u.state == "landed" && st.runs.iter().any(|r| r.id == u.landed_run && r.test_mode);
        let unlanded = u.state != "landed";
        if !(test_landed || unlanded) || u.base_commit.is_empty() {
            continue;
        }
        let ub = p.unit_branch(id);
        if !gitx::branch_exists(&repo.path, &ub) {
            continue;
        }
        let work = gitx::must(
            &repo.path,
            &["rev-list", "--count", &format!("{}..{}", u.base_commit, ub)],
        )
        .unwrap_or_default();
        if work.trim() == "0" || work.is_empty() {
            continue;
        }
        if gitx::is_ancestor(&repo.path, &ub, main) {
            let why = if test_landed {
                "landed in a test run"
            } else {
                "never landed"
            };
            return Ok(MainPos {
                repo: repo.name.clone(),
                moved: true,
                detail: format!(
                    "the main branch (now at {}) contains the commits of {id}, which {why}",
                    short(&tip)
                ),
                tip,
                note: String::new(),
            });
        }
    }
    // Anything else is the human's own commits (the project's scaffolding, their records, their code):
    // noted in one quiet line, never a reason to stop.
    let Some(rec) = recorded else {
        return Ok(ok(&tip));
    };
    let count = gitx::must(
        &repo.path,
        &["rev-list", "--count", &format!("{rec}..{tip}")],
    )
    .unwrap_or_default();
    let mut m = ok(&tip);
    m.note = format!(
        "the main branch has moved on from {} to {} by commits the engine did not make ({} new); they hold no unlanded or test-run work",
        short(&rec),
        short(&tip),
        if count.trim().is_empty() { "some".to_string() } else { count.trim().to_string() }
    );
    Ok(m)
}

/// Every repository's main branch against what the engine recorded.
pub fn main_positions(p: &Project, st: &State) -> R<Vec<MainPos>> {
    p.repos
        .iter()
        .map(|r| repo_main_position(p, st, r))
        .collect()
}

/// All repositories together: moved if any is; the details and notes of each, named by repository when
/// there are several.
pub fn main_position(p: &Project, st: &State) -> R<MainPos> {
    Ok(combine_positions(&main_positions(p, st)?))
}

fn combine_positions(all: &[MainPos]) -> MainPos {
    let several = all.len() > 1;
    let label = |m: &MainPos, t: &str| {
        if t.is_empty() {
            String::new()
        } else if several {
            format!("[{}] {t}", m.repo)
        } else {
            t.to_string()
        }
    };
    MainPos {
        repo: all.first().map(|m| m.repo.clone()).unwrap_or_default(),
        moved: all.iter().any(|m| m.moved),
        detail: all
            .iter()
            .map(|m| label(m, &m.detail))
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        tip: all.first().map(|m| m.tip.clone()).unwrap_or_default(),
        note: all
            .iter()
            .map(|m| label(m, &m.note))
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn short(c: &str) -> &str {
    &c[..c.len().min(10)]
}

/// Refuse while the main branch is somewhere the engine does not account for.
pub fn refuse_if_main_moved(p: &Project, st: &State) -> R<()> {
    let m = main_position(p, st)?;
    if !m.moved {
        return Ok(());
    }
    Err(refuse(
        "continue while the main branch is where the engine did not put it",
        m.detail,
        &[
            "Do not push anything. Tell the human what happened: the main branch holds work of a unit that never landed, or of a test run.",
            "They move the branch back with git to the last good commit, or have the work reviewed and landed through the engine.",
        ],
        "`bf status` shows `main: ok`.",
    ))
}

fn was_closed_already(st: &events::State, k: &str) -> bool {
    st.closed.contains_key(k)
}

pub fn checkpoint_close(p: &Project, k: &str, merge: bool) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("close {k}"))?;
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("close {k}"))?;
    refuse_if_main_moved(p, &st)?;
    let pl = plan::load(p)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(refuse(
            format!("close {k}"),
            "no such checkpoint",
            &["Use a checkpoint ID from `bf record list --kind checkpoint`."],
            "The command prints the closure.",
        ));
    };
    if let Some(so) = st.sync_open.as_ref().filter(|o| o.checkpoint == k) {
        return Err(refuse(
            format!("close {k}"),
            format!("a sync of the main branch into it is open ({})", so.phase),
            &[
                "`bf next` gives the directive that settles it.",
                &format!("Or go back: `bf checkpoint sync {k} --abort`."),
            ],
            "`bf status` shows no open sync.",
        ));
    }
    if let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == k) {
        let n = st.ck_fixes.iter().filter(|d| d.checkpoint == k).count() + 1;
        return Err(refuse(
            format!("close {k}"),
            format!(
                "a fix of it is open ({k}-fix-{n}, opened by {}, now in `{}`)",
                match f.source.as_str() {
                    "check" => "a failed check",
                    "wave" => "a failed wave check",
                    _ => "findings",
                },
                f.phase
            ),
            &[
                "`bf next` gives the directive that settles the fix.",
                &format!("Or put the checkpoint back as it was: `bf checkpoint fix {k} --abort`."),
            ],
            "`bf status` shows no open fix.",
        ));
    }
    let after = str_list(kr, "after");
    let unlanded: Vec<&String> = after
        .iter()
        .filter(|u| st.unit_state(u) != "landed")
        .collect();
    if !unlanded.is_empty() {
        return Err(refuse(
            format!("close {k}"),
            format!(
                "units not landed: {}",
                unlanded
                    .iter()
                    .map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            &["Land them with `bf unit land`; `bf next` shows where each one is."],
            "`bf status` shows them landed.",
        ));
    }
    let held = crate::wavecheck::close_blockers(&pl, &st, k);
    if !held.is_empty() {
        return Err(refuse_owned(
            format!("close {k}"),
            "a wave check has not passed",
            held.iter()
                .map(|h| format!("{h}."))
                .chain([
                    "`bf next` offers the wave check step, or the fix of a failed one.".to_string(),
                ])
                .collect(),
            "`bf status` shows every wave check of the checkpoint passed.",
        ));
    }
    // The checkpoint's working copy: its folder, which holds one working copy per repository when its
    // units change several.
    let cpwt = p.work_dir().join(k);
    let cp_repos: Vec<Repo> = checkpoint_repos(p, &pl, k)?.into_iter().cloned().collect();
    for r in &cp_repos {
        let wt = checkpoint_wt(p, &pl, k, r)?;
        if !wt.join(".git").exists() {
            return Err(refuse(
                format!("close {k}"),
                format!(
                    "the checkpoint's working copy in the repository `{}` does not exist",
                    r.name
                ),
                &["Land a unit of this checkpoint in that repository first."],
                "`bf checkpoint close` succeeds.",
            ));
        }
        if gitx::has_uncommitted(&wt) {
            return Err(refuse(
                format!("close {k}"),
                "the checkpoint's working copy has uncommitted changes, so no receipt describes the branch",
                &[&format!("Inspect `git -C {} status`.", wt.display()), "Remove the changes; checkpoint content only changes by landing units."],
                "The working copy is clean.",
            ));
        }
    }
    let content = p.content_hash(&cpwt)?;
    let receipts = evidence::load_receipts(p)?;
    let waived = gates::waived_ids(&st);
    let mut gaps: Vec<String> = Vec::new();
    let mut via_amend: Vec<Value> = Vec::new();
    for id in str_list(kr, "proves") {
        if let Some(a) = waived.get(&id) {
            via_amend.push(json!({"id": id, "amendment": a}));
            continue;
        }
        let checks: Vec<&Value> = pl
            .checks
            .iter()
            .filter(|c| {
                str_list(kr, "checks").iter().any(|x| x == s(c, "id"))
                    && str_list(c, "proves").contains(&id)
            })
            .collect();
        // A command or agent check is settled by its receipt at this content; a human check only by the
        // human's own gate. An attached file proves nothing about a human check.
        let ok = crate::enforce::id_proven(&st, &receipts, &checks, k, &content);
        let marked: Vec<String> = checks
            .iter()
            .filter_map(|c| {
                evidence::standing_outcome(&receipts, s(c, "id"), &content).map(|r| {
                    format!(
                        "{} is marked {} ({})",
                        s(c, "id"),
                        r.outcome().unwrap_or(""),
                        s(&r.v, "reason")
                    )
                })
            })
            .collect();
        if !ok && !marked.is_empty() {
            gaps.push(format!(
                "{id}: {}; the human decides whether to fix the cause and run it again, to accept the gap by amendment, or to change the plan",
                marked.join("; ")
            ));
        } else if !ok {
            let names: Vec<&str> = checks.iter().map(|c| s(c, "id")).collect();
            gaps.push(format!(
                "{id}: no sealed passing receipt at this content (hash {}) from {}; run `bf evidence run <check> -- <its command>` inside {}, or ask the human to approve an amendment",
                &content[..12.min(content.len())],
                if names.is_empty() { "any check".to_string() } else { names.join(" or ") },
                cpwt.display()
            ));
        }
    }
    if crate::enforce::stops(kr) && !st.gate_passed("checkpoint-step", Some(k)) {
        gaps.push(format!("the stop `{}` has no approved checkpoint-step gate: run `bf gate open checkpoint-step --ref {k}`", if s(kr, "human_step").is_empty() { s(kr, "stop_why") } else { s(kr, "human_step") }));
    }
    let (mut check_gaps, mut unproven_names) =
        crate::enforce::checkpoint_gaps(p, &pl, &st, &receipts, kr, &content)?;
    // R-49: no checkpoint closes before its review ran on the content being closed.
    if !was_closed_already(&st, k) && crate::ckreview::needed(p, &st, k, &content)? {
        check_gaps.push(format!(
            "the checkpoint review has not run on this content (hash {}): `bf next` gives the reviewer's directive",
            &content[..12.min(content.len())]
        ));
        unproven_names.push(crate::checks::CHECKPOINT_REVIEW.to_string());
    }
    let has_check_gaps = !check_gaps.is_empty();
    gaps.extend(check_gaps);
    if !gaps.is_empty() {
        let mut steps = gaps.clone();
        if has_check_gaps {
            steps.extend(crate::enforce::way_out_steps(&st, &unproven_names));
        }
        steps.push(format!("Run `bf checkpoint close {k}` again."));
        return Err(refuse_owned(
            format!("close {k}"),
            format!("{} ID(s) have no proof", gaps.len()),
            steps,
            "`bf checkpoint close` prints the closure.",
        ));
    }
    let was_closed = st.closed.contains_key(k);
    let mut merged = false;
    if merge {
        // Main moves only with a human act after the closure: an approved checkpoint-step gate opened
        // after the close event. (D-16: main moves when a human merges.)
        let closed_seq = st.closed.get(k).map(|c| c.seq);
        let Some(closed_seq) = closed_seq else {
            return Err(refuse(
                format!("merge {k} onto the main branch"),
                "the checkpoint is not closed yet; merging follows the closure and a human act",
                &[&format!("Run `bf checkpoint close {k}` first.")],
                "`bf status` lists the checkpoint as closed.",
            ));
        };
        // The last checkpoint holds everything: it merges only when every required check has proof.
        if crate::enforce::last_checkpoint(&pl)
            .map(|l| s(l, "id") == k)
            .unwrap_or(false)
        {
            crate::enforce::refuse_if_unproven_for(
                p,
                &pl,
                &format!("merge {k} onto the main branch"),
                &format!("bf checkpoint close {k} --merge"),
            )?;
        }
        // A run with simulated approvals can never reach the main branch.
        let mut owned: Vec<String> = after.clone();
        for c in &pl.checkpoints {
            if s(c, "id") == k {
                break;
            }
            owned.extend(str_list(c, "after"));
        }
        let tainted_unit = owned.iter().any(|u| {
            let lr = st
                .units
                .get(u)
                .map(|x| x.landed_run.clone())
                .unwrap_or_default();
            st.runs.iter().any(|r| r.id == lr && r.test_mode)
        });
        let tainted_gate = ["idea", "requirements", "ui", "architecture", "plan"]
            .iter()
            .any(|g| {
                st.latest_gate(g, None)
                    .map(|x| x.simulated)
                    .unwrap_or(false)
            })
            || st
                .latest_gate("checkpoint-step", Some(k))
                .map(|x| x.simulated)
                .unwrap_or(false);
        if tainted_unit || tainted_gate {
            return Err(refuse(
                format!("merge {k} onto the main branch"),
                "the run contains a simulated approval or test-mode work; such a run can never land on the main branch",
                &["Leave the checkpoint branch as it is for inspection.", "Redo the work in a real run: the human approves each gate in their own words."],
                "`bf checkpoint close` without --merge still records the closure.",
            ));
        }
        let approved_after = st.gates.iter().any(|g| {
            g.kind == "checkpoint-step"
                && g.refr.as_deref() == Some(k)
                && g.status == "approved"
                && !g.simulated
                && g.open_seq > closed_seq
        });
        if !approved_after {
            return Err(refuse(
                format!("merge {k} onto the main branch"),
                "merging onto the main branch needs the human's go-ahead after the checkpoint closed",
                &[
                    &format!("Open the gate: `bf gate open checkpoint-step --ref {k}` and show the human the checkpoint page."),
                    "When the human says in chat to merge it, `bf gate approve <gate> --quote \"<their words>\"`.",
                    &format!("Then run `bf checkpoint close {k} --merge` again."),
                ],
                "The command prints the merge.",
            ));
        }
        // Every repository the checkpoint touches: its main checkout must be on its main branch first.
        for r in &cp_repos {
            let main = &r.branch;
            if gitx::current_branch(&r.path).as_deref() != Some(main.as_str()) {
                return Err(refuse(
                    format!("merge {k} onto {main}"),
                    format!("the main checkout of `{}` is not on `{main}`", r.name),
                    &[
                        &format!(
                            "Switch the main checkout ({}) to `{main}` (the human's step).",
                            r.path.display()
                        ),
                        "Run the command again.",
                    ],
                    "The command prints the merge commit.",
                ));
            }
        }
        let cpb = p.checkpoint_branch(k);
        for r in &cp_repos {
            let main = r.branch.clone();
            if !gitx::is_ancestor(&r.path, &cpb, &main) {
                let out = gitx::run(
                    &r.path,
                    &[
                        "merge",
                        "--no-ff",
                        "--no-edit",
                        "-m",
                        &format!("Close {k}: {}", s(kr, "title")),
                        &cpb,
                    ],
                    &[],
                )?;
                if !out.ok {
                    let _ = gitx::run(&r.path, &["merge", "--abort"], &[]);
                    return Err(refuse(
                        format!("merge {k} onto {main}"),
                        format!("git refused the merge in `{}`: {}", r.name, out.stderr.lines().next().unwrap_or("")),
                        &["Commit or stash local changes that touch the same files, or resolve the conflict by hand (the human's step).", "Run the command again."],
                        "The command prints the merge commit.",
                    ));
                }
            }
            merged = true;
            // Remember the commit the engine put on the main branch.
            if let Ok(tip) = gitx::must(&r.path, &["rev-parse", &main]) {
                events::set_commits(p, &[(r.name.clone(), tip)])?;
            }
        }
    }
    let unproven_off: Vec<Value> = crate::enforce::off_unproven(&pl, &st, kr)
        .into_iter()
        .map(|(id, c)| json!({"id": id, "checks": c}))
        .collect();
    if !was_closed || merged {
        // The waiver is fixed here: the ledger reads it from the log, not from a later run's settings.
        let off_names: Vec<String> = crate::checks::off_closing(&st).keys().cloned().collect();
        events::append(
            p,
            "checkpoint.close",
            json!({"checkpoint": k, "merged": merged, "hash": content, "unproven_off": unproven_off, "off_names": off_names, "run": crate::enforce::latest_run_id(&st)}),
        )?;
        p.write_guard()?;
    }
    Ok(
        json!({"checkpoint": k, "closed": true, "already": was_closed && !merged, "merged": merged, "content_hash": content, "amendments": via_amend, "checks_off": crate::enforce::off_summary(&st),
            "unproven_off": unproven_off}),
    )
}

/// `bf unit release <U> --quote ...`: the human puts a parked or blocked unit back to work. Its round and
/// attempt counts start again; the log keeps the whole history.
pub fn unit_release(p: &Project, unit: &str, quote: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let Some(u) = st.units.get(unit) else {
        return Err(refuse(
            format!("release {unit}"),
            "that unit has not been started",
            &["Use a unit ID from `bf status`."],
            "`bf status` shows the unit back at work.",
        ));
    };
    if !matches!(u.state.as_str(), "parked" | "blocked") {
        return Err(refuse(
            format!("release {unit}"),
            format!(
                "it is `{}`; only a parked or blocked unit is released",
                u.state
            ),
            &["`bf next` shows what each unit is waiting for."],
            "`bf status` shows the unit back at work.",
        ));
    }
    let to = match u.prev_state.as_str() {
        "review" if u.parked_reason.starts_with("more than") => "fixing",
        "review" => "review",
        "fixing" => "fixing",
        _ => "running",
    };
    let (seq, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    events::append(
        p,
        "unit.release",
        json!({"unit": unit, "turn": seq, "quote": quote.trim(), "from": u.state, "to": to, "reason": u.parked_reason}),
    )?;
    p.write_guard()?;
    Ok(json!({"unit": unit, "state": to, "released": true}))
}

/// A builder that keeps failing its checks is parked after `max_attempts` in a row.
pub fn park_if_looping(p: &Project, unit: &str) -> R<()> {
    let st = events::replay(p)?;
    let Some(u) = st.units.get(unit) else {
        return Ok(());
    };
    if matches!(u.state.as_str(), "running" | "fixing")
        && u.attempts >= p.policy.limits.max_attempts
    {
        events::append(
            p,
            "unit.park",
            json!({"unit": unit, "reason": format!("{} failing check runs or blocked reports in a row", u.attempts)}),
        )?;
        p.write_guard()?;
    }
    Ok(())
}

/// Where the main branch holds commits the open stack of checkpoints lacks: for each repository, the
/// newest open checkpoint that has a branch there and how many commits of the main branch it lacks.
/// Information only; `bf checkpoint sync` takes them in.
pub fn main_ahead(p: &Project, st: &State, pl: &Plan) -> Vec<Value> {
    let mut out = Vec::new();
    for r in &p.repos {
        if !gitx::branch_exists(&r.path, &r.branch) {
            continue;
        }
        for k in pl.checkpoints.iter().rev() {
            let kid = s(k, "id");
            if st.closed.contains_key(kid) {
                continue;
            }
            let cpb = p.checkpoint_branch(kid);
            if !gitx::branch_exists(&r.path, &cpb) {
                continue;
            }
            let n = gitx::must(
                &r.path,
                &["rev-list", "--count", &format!("{cpb}..{}", r.branch)],
            )
            .unwrap_or_default();
            let n: u64 = n.trim().parse().unwrap_or(0);
            if n > 0 {
                out.push(json!({
                    "repo": r.name, "checkpoint": kid, "commits": n,
                    "command": format!("bf checkpoint sync {kid} --quote \"<the human's words>\""),
                }));
            }
            break;
        }
    }
    out
}

/// `bf checkpoint sync <K> --quote "..." [--resolved]`: the human has the checkpoint branch take in the
/// repository's local main branch (no network: pulling into main is an ordinary git command done before).
/// Only at a quiet point; a conflict leaves the merge in progress in the checkpoint's working copy
/// (`--abort` puts everything back). Checks the human switched off are not run, and the answer names them.
/// What `bf checkpoint sync` was asked to do.
#[derive(Clone, Copy, Default)]
pub struct SyncMode {
    /// The human merged by hand: record it.
    pub resolved: bool,
    /// Put the checkpoint back as it was before an open sync started (no human turn needed).
    pub abort: bool,
    /// The human releases a parked sync back to its builder.
    pub release: bool,
}

pub fn checkpoint_sync(p: &Project, k: &str, quote: &str, mode: SyncMode) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("sync {k}"))?;
    let resolved = mode.resolved;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    if mode.abort {
        return sync_abort(p, &st, k);
    }
    if mode.release {
        return sync_release(p, &st, k, quote);
    }
    let pl = plan::load(p)?;
    if let Some(so) = st.sync_open.clone() {
        if so.checkpoint == k && resolved {
            return sync_hand_resolved(p, &st, &pl, &so, quote);
        }
        return Err(refuse(
            format!("sync {k}"),
            format!("a sync of {} is open ({}): its conflict is being resolved", so.checkpoint, so.phase),
            &[
                "`bf next` gives the directive that settles it (`resolve-sync`, `review-sync` or, when parked, the human's decision).",
                &format!("To go back to how the checkpoint was: `bf checkpoint sync {} --abort`.", so.checkpoint),
            ],
            "`bf status` shows no open sync.",
        ));
    }
    refuse_if_main_moved(p, &st)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(refuse(
            format!("sync {k}"),
            "no such checkpoint",
            &["Use a checkpoint ID from `bf record list --kind checkpoint`."],
            "The command prints the sync.",
        ));
    };
    if st.closed.contains_key(k) {
        return Err(refuse(
            format!("sync {k}"),
            "the checkpoint is closed; the main branch is taken in by the checkpoint at the tip of the stack",
            &["Sync the newest open checkpoint instead (`bf status` says which)."],
            "`bf status` shows the checkpoint open.",
        ));
    }
    let repos: Vec<Repo> = checkpoint_repos(p, &pl, k)?.into_iter().cloned().collect();
    // The tip of the stack: no later checkpoint has started.
    let after: Vec<&Value> = pl
        .checkpoints
        .iter()
        .skip_while(|c| s(c, "id") != k)
        .skip(1)
        .collect();
    for c in after {
        let cid = s(c, "id");
        let started = p
            .repos
            .iter()
            .any(|r| gitx::branch_exists(&r.path, &p.checkpoint_branch(cid)));
        if started {
            return Err(refuse(
                format!("sync {k}"),
                format!("{cid} already builds on it: only the checkpoint at the tip of the stack takes the main branch in"),
                &[&format!("Sync {cid}, or the newest open checkpoint, instead.")],
                "`bf checkpoint sync` prints the sync.",
            ));
        }
    }
    // A quiet point: no unit between its start and its landing.
    let busy: Vec<String> = st
        .units
        .iter()
        .filter(|(_, u)| {
            matches!(
                u.state.as_str(),
                "running" | "review" | "fixing" | "reviewed" | "blocked" | "parked"
            )
        })
        .map(|(id, u)| format!("{id} ({})", u.state))
        .collect();
    if !busy.is_empty() {
        return Err(refuse(
            format!("sync {k}"),
            format!("units are between start and landing: {}", busy.join(", ")),
            &[
                "Finish and land them (`bf next` shows where each one is).",
                "A parked or blocked unit is released by the human (`bf unit release <U> --quote \"...\"`) and then finished.",
                "Run the command again at the quiet point.",
            ],
            "`bf status` shows no unit running, in review, fixing, blocked or parked.",
        ));
    }
    // What each repository's checkpoint branch lacks.
    struct Step {
        repo: Repo,
        wt: PathBuf,
        main_tip: String,
        old_tip: String,
        lacks: bool,
        hand_merged: bool,
    }
    let cpb = p.checkpoint_branch(k);
    let mut steps: Vec<Step> = Vec::new();
    for r in &repos {
        let main_tip = gitx::must(&r.path, &["rev-parse", &r.branch]).map_err(|_| {
            refuse(
                format!("sync {k}"),
                format!(
                    "the main branch `{}` of `{}` does not exist",
                    r.branch, r.name
                ),
                &["Check the branch the project files name."],
                "`bf status` shows the repository.",
            )
        })?;
        let wt = ensure_checkpoint_worktree(p, k, r)?;
        if gitx::has_uncommitted(&wt) {
            return Err(refuse(
                format!("sync {k}"),
                "the checkpoint's working copy has uncommitted changes",
                &[&format!(
                    "Commit or remove them (`git -C {} status`), then run the command again.",
                    wt.display()
                )],
                "The working copy is clean.",
            ));
        }
        let lacks = !gitx::is_ancestor(&r.path, &main_tip, &cpb);
        let parents =
            gitx::must(&wt, &["rev-list", "--parents", "-n", "1", "HEAD"]).unwrap_or_default();
        let is_merge = parents.split_whitespace().count() >= 3;
        let known = st
            .synced
            .iter()
            .any(|(kk, rr, mm)| kk == k && rr == &r.name && mm == &main_tip);
        let old_tip = gitx::must(&wt, &["rev-parse", "HEAD"]).unwrap_or_default();
        steps.push(Step {
            repo: r.clone(),
            wt,
            main_tip,
            old_tip,
            lacks,
            hand_merged: resolved && !lacks && !known && is_merge,
        });
    }
    if resolved {
        if let Some(s0) = steps.iter().find(|x| x.lacks) {
            return Err(refuse(
                format!("sync {k}"),
                format!("the checkpoint branch does not contain the main branch of `{}` yet", s0.repo.name),
                &[
                    &format!("In {}: `git merge {}`, resolve the conflicts, `git add`, `git commit`.", s0.wt.display(), s0.repo.branch),
                    &format!("Then run `bf checkpoint sync {k} --resolved --quote \"<the human's words>\"` again."),
                ],
                "`bf checkpoint sync` records the merge.",
            ));
        }
    }
    let todo: Vec<&Step> = steps.iter().filter(|x| x.lacks || x.hand_merged).collect();
    if todo.is_empty() {
        return Ok(
            json!({"checkpoint": k, "synced": false, "nothing_to_take_in": true,
            "message": "the checkpoint already holds everything on the main branch; nothing changed"}),
        );
    }
    // The human's words first: nothing is merged for a sync that could not be recorded.
    let (turn, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    // Merge, one repository after another. A conflict is left in progress in the checkpoint's working
    // copy for the factory's own agents to resolve; the human's part is the sentence that asked.
    let floor = evidence::highest_receipt_number(p);
    let mut done: Vec<(String, String, String)> = Vec::new();
    let mut open_repos: Vec<Value> = Vec::new();
    let mut conflicted: Vec<String> = Vec::new();
    let several = steps.len() > 1;
    for step in steps.iter() {
        if !step.lacks {
            if step.hand_merged {
                done.push((
                    step.repo.name.clone(),
                    step.main_tip.clone(),
                    gitx::head_short(&step.wt),
                ));
            }
            continue;
        }
        let out = gitx::run(
            &step.wt,
            &[
                "merge",
                "--no-ff",
                "--no-edit",
                "-m",
                &format!("Take {} into {k}", step.repo.branch),
                &step.main_tip,
            ],
            &[],
        )?;
        let allowed = allowed_set(&step.wt, &step.old_tip, &step.main_tip);
        if out.ok {
            let c = gitx::head_short(&step.wt);
            done.push((step.repo.name.clone(), step.main_tip.clone(), c.clone()));
            open_repos.push(json!({"repo": step.repo.name, "main": step.main_tip, "before": step.old_tip, "files": [], "merge": c, "allowed": allowed}));
        } else {
            let files: Vec<String> =
                gitx::must(&step.wt, &["diff", "--name-only", "--diff-filter=U"])
                    .unwrap_or_default()
                    .lines()
                    .map(|l| l.to_string())
                    .collect();
            for f in &files {
                conflicted.push(if several {
                    format!("{}/{f}", step.repo.name)
                } else {
                    f.clone()
                });
            }
            open_repos.push(json!({"repo": step.repo.name, "main": step.main_tip, "before": step.old_tip, "files": files, "merge": "", "allowed": allowed}));
        }
    }
    if !conflicted.is_empty() {
        events::append(
            p,
            "checkpoint.sync.open",
            json!({"checkpoint": k, "repos": open_repos, "turn": turn, "quote": quote.trim(), "receipt_floor": floor, "failed": []}),
        )?;
        p.write_guard()?;
        return Ok(
            json!({"checkpoint": k, "synced": false, "sync_open": true, "conflicted": conflicted,
            "message": "the main branch conflicts with the checkpoint; the merge is left in progress in the checkpoint's working copy and `bf next` gives the directive that resolves it (`bf checkpoint sync <K> --abort` puts everything back)"}),
        );
    }
    // The joined content is proven before the sync counts: the sync's checks (the command checks every
    // unit of the checkpoint is held to) run on it. The checkpoint's own checks run at verification.
    let failed = run_sync_checks(p, &pl, &st, k, floor)?;
    let skipped = skipped_k_checks(p, &pl, k)?;
    if !failed.is_empty() {
        events::append(
            p,
            "checkpoint.sync.open",
            json!({"checkpoint": k, "repos": open_repos, "turn": turn, "quote": quote.trim(), "receipt_floor": floor, "failed": failed}),
        )?;
        p.write_guard()?;
        return Ok(
            json!({"checkpoint": k, "synced": false, "sync_open": true, "conflicted": [],
            "failed_checks": failed, "skipped_checks": skipped,
            "message": "the main branch merged without a conflict, but the sync's checks fail on the joined content; the merge is kept and `bf next` gives the directive that repairs it (`bf checkpoint sync <K> --abort` puts everything back)"}),
        );
    }
    let reopen_checks = str_list(kr, "checks");
    events::append(
        p,
        "checkpoint.sync",
        json!({
            "checkpoint": k,
            "repos": done.iter().map(|(r, m, c)| json!({"repo": r, "main": m, "merge": c})).collect::<Vec<_>>(),
            "turn": turn, "quote": quote.trim(),
            "reopen_checks": reopen_checks, "receipt_floor": floor,
        }),
    )?;
    // The repository's main branch is where the engine now counts from: the stack holds it.
    let moved: Vec<(String, String)> = done
        .iter()
        .map(|(name, main, _)| (name.clone(), main.clone()))
        .collect();
    events::set_commits(p, &moved)?;
    p.write_guard()?;
    Ok(
        json!({"checkpoint": k, "synced": true, "skipped_checks": skipped,
        "repos": done.iter().map(|(r, m, c)| json!({"repo": r, "main": m, "merge": c})).collect::<Vec<_>>()}),
    )
}

fn sync_wt(p: &Project, k: &str, repo: &str) -> R<PathBuf> {
    let pl = plan::load(p)?;
    let r = p.repo(repo)?;
    checkpoint_wt(p, &pl, k, r)
}

/// The files the resolver of a sync may change in a repository: those either side changed since the merge
/// base, and new test files beside them (a file with `test` in its name in the same folder, or any file
/// in a folder named for tests).
fn allowed_set(wt: &Path, before: &str, main: &str) -> Vec<String> {
    let base = gitx::must(wt, &["merge-base", before, main]).unwrap_or_default();
    let mut files: Vec<String> = Vec::new();
    for side in [before, main] {
        if base.is_empty() {
            break;
        }
        let d = gitx::must(wt, &["diff", "--name-only", &base, side]).unwrap_or_default();
        for f in d.lines() {
            if !files.iter().any(|x| x == f) {
                files.push(f.to_string());
            }
        }
    }
    let mut dirs: Vec<String> = Vec::new();
    for f in &files {
        let d = f
            .rsplit_once('/')
            .map(|x| x.0.to_string())
            .unwrap_or_default();
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    files.truncate(3000);
    for d in dirs {
        let last = d.rsplit('/').next().unwrap_or("").to_lowercase();
        if last.contains("test") {
            files.push(format!("{d}/"));
        } else if d.is_empty() {
            files.push("*test*".into());
        } else {
            files.push(format!("{d}/*test*"));
        }
    }
    files
}

/// The command checks of a checkpoint that the human switched off: a sync leaves them out, as the close does.
pub(crate) fn skipped_k_checks(p: &Project, pl: &Plan, k: &str) -> R<Vec<String>> {
    let Some(kr) = pl.checkpoint(k) else {
        return Ok(vec![]);
    };
    let st = events::require_chain(p)?;
    Ok(str_list(kr, "checks")
        .into_iter()
        .filter(|cid| {
            pl.checks.iter().any(|c| {
                s(c, "id") == *cid && s(c, "how") == "command" && crate::enforce::is_off(&st, c)
            })
        })
        .collect())
}

/// Run the checkpoint's own command checks that are switched on, on the content now in its working copy,
/// through the engine (real receipts). Returns the failures: check, command, exit, output tail. Agent-judged checks and a
/// checkpoint with none skip this.
pub(crate) fn run_k_checks(p: &Project, pl: &Plan, k: &str) -> R<Vec<Value>> {
    let Some(kr) = pl.checkpoint(k) else {
        return Ok(vec![]);
    };
    let off = skipped_k_checks(p, pl, k)?;
    let top = p.work_dir().join(k);
    let mut failed = Vec::new();
    for cid in str_list(kr, "checks") {
        let Some(c) = pl.checks.iter().find(|c| s(c, "id") == cid) else {
            continue;
        };
        if s(c, "how") != "command" || off.contains(&cid) {
            continue;
        }
        let argv = evidence::command_words(c);
        // The engine runs the checks again on content they may already have run on: that is a deliberate
        // repeat, stated as one.
        let attempt = evidence::next_attempt(p, &cid, &argv, &top)?;
        let repeat = (attempt > 1).then_some((
            attempt,
            "the engine runs the checkpoint's checks again after a fix or a sync",
        ));
        match evidence::run_with(p, &cid, None, &argv, &top, repeat) {
            Ok(r) if r["exit"].as_i64() == Some(0) => {}
            Ok(r) => failed.push(
                json!({"check": cid, "command": argv.join(" "), "exit": r["exit"], "tail": s(&r, "output_tail")}),
            ),
            Err(e) => failed.push(
                json!({"check": cid, "command": argv.join(" "), "exit": null, "tail": format!("{e:?}")}),
            ),
        }
    }
    Ok(failed)
}

/// A check a sync runs: the label it is shown under (the unit check's ID, or the name of a check every
/// unit must pass) and its command.
#[derive(Clone, Debug)]
pub(crate) struct SyncCheck {
    pub label: String,
    pub argv: Vec<String>,
}

/// The command checks every unit of the checkpoint is held to, each distinct command once: those of
/// its unit check records and of the list of checks every unit must pass, that all its units share.
/// `None` when the units share no command (or there is no unit): the sync then runs the checkpoint's own
/// command checks, as it always did.
pub(crate) fn sync_checks(p: &Project, pl: &Plan, st: &State, k: &str) -> Option<Vec<SyncCheck>> {
    let mut per_unit: Vec<Vec<SyncCheck>> = Vec::new();
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let uid = s(u, "id");
        let mut list: Vec<SyncCheck> = Vec::new();
        for c in pl.unit_checks(uid) {
            if s(c, "how") != "command" || crate::enforce::is_off(st, c) {
                continue;
            }
            list.push(SyncCheck {
                label: s(c, "id").to_string(),
                argv: evidence::command_words(c),
            });
        }
        for a in crate::checks::attached_for_unit(p, pl, st, uid).0 {
            list.push(SyncCheck {
                label: a.name.clone(),
                argv: a.command.clone(),
            });
        }
        per_unit.push(list);
    }
    let first = per_unit.first()?;
    let mut shared: Vec<SyncCheck> = Vec::new();
    for c in first {
        if c.argv.is_empty()
            || shared.iter().any(|x| x.argv == c.argv)
            || !per_unit[1..]
                .iter()
                .all(|l| l.iter().any(|x| x.argv == c.argv))
        {
            continue;
        }
        shared.push(c.clone());
    }
    (!shared.is_empty()).then_some(shared)
}

/// Run the sync's checks on the content now in the checkpoint's working copy. Returns the failures:
/// check, command, exit, output tail. The runs of the units' shared checks are recorded as sync runs, which
/// settle nothing; a checkpoint whose units share no command runs its own command checks (real receipts).
pub(crate) fn run_sync_checks(
    p: &Project,
    pl: &Plan,
    st: &State,
    k: &str,
    sync: u64,
) -> R<Vec<Value>> {
    let Some(list) = sync_checks(p, pl, st, k) else {
        return run_k_checks(p, pl, k);
    };
    let top = p.work_dir().join(k);
    let mut failed = Vec::new();
    for c in list {
        match evidence::sync_run(p, k, &c.label, &c.argv, &top, sync) {
            Ok(r) if r["exit"].as_i64() == Some(0) => {}
            Ok(r) => failed.push(
                json!({"check": c.label, "command": c.argv.join(" "), "exit": r["exit"], "tail": s(&r, "output_tail")}),
            ),
            Err(e) => failed.push(
                json!({"check": c.label, "command": c.argv.join(" "), "exit": null, "tail": format!("{e:?}")}),
            ),
        }
    }
    Ok(failed)
}

/// What the resolver may change, as patterns inside the checkpoint's working copy folder (prefixed by the
/// repository name when several are touched).
pub fn sync_allowed(so: &events::SyncOpen) -> Vec<String> {
    let several = so.repos.len() > 1;
    so.repos
        .iter()
        .flat_map(|r| {
            r.allowed.iter().map(move |f| {
                if several {
                    format!("{}/{f}", r.repo)
                } else {
                    f.clone()
                }
            })
        })
        .collect()
}

/// The conflicted files of an open sync, as paths inside the checkpoint's working copy folder.
pub fn sync_files(so: &events::SyncOpen) -> Vec<String> {
    let several = so.repos.len() > 1;
    so.repos
        .iter()
        .flat_map(|r| {
            r.files.iter().map(move |f| {
                if several {
                    format!("{}/{f}", r.repo)
                } else {
                    f.clone()
                }
            })
        })
        .collect()
}

/// Finish an open sync: the checkpoint holds the main branch. Records `checkpoint.sync`.
fn finish_sync(
    p: &Project,
    pl: &Plan,
    so: &events::SyncOpen,
    human: Option<(u64, &str)>,
) -> R<Value> {
    let kr = pl.checkpoint(&so.checkpoint);
    let reopen_checks = kr.map(|k| str_list(k, "checks")).unwrap_or_default();
    let mut repos: Vec<Value> = Vec::new();
    for r in &so.repos {
        let wt = sync_wt(p, &so.checkpoint, &r.repo)?;
        repos.push(json!({"repo": r.repo, "main": r.main, "merge": gitx::head_short(&wt)}));
    }
    let mut ev = json!({
        "checkpoint": so.checkpoint, "repos": repos,
        "reopen_checks": reopen_checks, "receipt_floor": so.floor,
    });
    match human {
        Some((turn, quote)) => {
            ev["turn"] = json!(turn);
            ev["quote"] = json!(quote.trim());
        }
        None => ev["via_open"] = json!(true),
    }
    events::append(p, "checkpoint.sync", ev)?;
    let moved: Vec<(String, String)> = so
        .repos
        .iter()
        .map(|r| (r.repo.clone(), r.main.clone()))
        .collect();
    events::set_commits(p, &moved)?;
    p.write_guard()?;
    let skipped = skipped_k_checks(p, pl, &so.checkpoint)?;
    Ok(
        json!({"checkpoint": so.checkpoint, "synced": true, "skipped_checks": skipped, "repos": repos}),
    )
}

/// What is still wrong with a resolution: unmerged paths, conflict markers, a merge not concluded or
/// not holding the main branch, or files changed beyond the conflicted ones (compared with git's own
/// automatic merge).
fn sync_problems(p: &Project, so: &events::SyncOpen) -> R<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for r in &so.repos {
        let wt = sync_wt(p, &so.checkpoint, &r.repo)?;
        let unmerged =
            gitx::must(&wt, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        for f in unmerged.lines() {
            out.push(format!(
                "{}: {f} is still unmerged (resolve it, then `git add {f}`)",
                r.repo
            ));
        }
        for f in &r.files {
            let text = fs::read_to_string(wt.join(f)).unwrap_or_default();
            if text
                .lines()
                .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "=======")
            {
                out.push(format!("{}: {f} still has conflict markers", r.repo));
            }
        }
        let merging = gitx::run(&wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[])
            .map(|o| o.ok)
            .unwrap_or(false);
        if merging {
            out.push(format!(
                "{}: the merge is not concluded (`git commit --no-edit` in {})",
                r.repo,
                wt.display()
            ));
        } else if gitx::has_uncommitted(&wt) {
            out.push(format!(
                "{}: the working copy has uncommitted changes",
                r.repo
            ));
        }
        if !merging && !gitx::is_ancestor(&wt, &r.main, "HEAD") {
            out.push(format!(
                "{}: the checkpoint does not contain the main branch's commit {}",
                r.repo,
                &r.main[..r.main.len().min(10)]
            ));
        }
        if !merging && !r.before.is_empty() {
            // Only the conflicted files may differ from what git's own merge produced.
            if let Ok(o) = gitx::run(
                &wt,
                &["merge-tree", "--write-tree", &r.before, &r.main],
                &[],
            ) {
                if let Some(tree) = o
                    .stdout
                    .lines()
                    .next()
                    .filter(|t| t.len() >= 40 && t.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    let diff =
                        gitx::must(&wt, &["diff", "--name-only", tree, "HEAD"]).unwrap_or_default();
                    for f in diff.lines() {
                        if !r.allowed.iter().any(|g| util::path_matches(g, f)) {
                            out.push(format!(
                                "{}: {f} was changed but is not a file either side changed (nor a new test file beside one)",
                                r.repo
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn sync_abort(p: &Project, st: &State, k: &str) -> R<Value> {
    let Some(so) = st.sync_open.clone().filter(|o| o.checkpoint == k) else {
        return Err(refuse(
            format!("abort the sync of {k}"),
            "no sync is open for it",
            &["`bf status` shows an open sync when there is one."],
            "`bf status` shows no open sync.",
        ));
    };
    for r in &so.repos {
        let wt = sync_wt(p, k, &r.repo)?;
        let _ = gitx::run(&wt, &["merge", "--abort"], &[]);
        if !r.before.is_empty() {
            gitx::must(&wt, &["reset", "--hard", "-q", &r.before])?;
        }
    }
    events::append(p, "checkpoint.sync.abort", json!({"checkpoint": k}))?;
    p.write_guard()?;
    Ok(
        json!({"checkpoint": k, "aborted": true, "message": "the checkpoint is as it was before the sync started"}),
    )
}

fn sync_release(p: &Project, st: &State, k: &str, quote: &str) -> R<Value> {
    let Some(so) = st.sync_open.clone().filter(|o| o.checkpoint == k) else {
        return Err(refuse(
            format!("release the sync of {k}"),
            "no sync is open for it",
            &["`bf status` shows an open sync when there is one."],
            "`bf status` shows no open sync.",
        ));
    };
    if so.phase != "parked" {
        return Err(refuse(
            format!("release the sync of {k}"),
            format!("it is `{}`, not parked", so.phase),
            &["`bf next` shows what it waits for."],
            "`bf status` shows the sync back at work.",
        ));
    }
    let (turn, _) = events::pick_turn(st, 0, events::Decide::Approve, quote)?;
    events::append(
        p,
        "checkpoint.sync.release",
        json!({"checkpoint": k, "turn": turn, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    Ok(json!({"checkpoint": k, "released": true}))
}

/// The human resolved the merge by hand in the checkpoint's working copy while a sync was open.
fn sync_hand_resolved(
    p: &Project,
    st: &State,
    pl: &Plan,
    so: &events::SyncOpen,
    quote: &str,
) -> R<Value> {
    let problems = sync_problems(p, so)?;
    if !problems.is_empty() {
        let mut steps = problems.clone();
        steps.push(format!(
            "Then run `bf checkpoint sync {} --resolved --quote \"<the human's words>\"` again.",
            so.checkpoint
        ));
        return Err(refuse_owned(
            format!("sync {}", so.checkpoint),
            format!("the merge is not finished: {}", problems.len()),
            steps,
            "`bf checkpoint sync` records the merge.",
        ));
    }
    // The joined content must be proven too.
    let failed = run_sync_checks(p, pl, st, &so.checkpoint, so.floor)?;
    if !failed.is_empty() {
        let mut steps: Vec<String> = failed
            .iter()
            .map(|f| {
                format!(
                    "{} fails ({})",
                    f["check"].as_str().unwrap_or(""),
                    f["command"].as_str().unwrap_or("")
                )
            })
            .collect();
        steps.push("Repair the joined content, commit, then run the command again.".into());
        return Err(refuse_owned(
            format!("sync {}", so.checkpoint),
            format!(
                "the sync's checks fail on the joined content: {}",
                failed.len()
            ),
            steps,
            "`bf checkpoint sync` records the merge.",
        ));
    }
    let (turn, _) = events::pick_turn(st, 0, events::Decide::Approve, quote)?;
    finish_sync(p, pl, so, Some((turn, quote)))
}

/// `bf report <K>` while a sync is open: the resolver says built or blocked; the reviewer says findings
/// or clean.
pub fn sync_report(
    p: &Project,
    k: &str,
    result: &str,
    role: Option<&str>,
    note: &str,
    findings: Option<&Value>,
    token: Option<&str>,
) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("report on the sync of {k}"))?;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let Some(so) = st.sync_open.clone().filter(|o| o.checkpoint == k) else {
        return Err(refuse(
            format!("report on {k}"),
            "no sync is open for it; a checkpoint is not reported on",
            &["Units are reported on with `bf report <U>`."],
            "`bf status` shows the unit's new state.",
        ));
    };
    let expected = match so.phase.as_str() {
        "resolve" => "builder",
        "review" => "reviewer",
        _ => {
            return Err(refuse(
                format!("report on the sync of {k}"),
                "the sync is parked: the human decides",
                &["The human releases it (`bf checkpoint sync <K> --release --quote \"...\"`) or the conductor aborts it (`--abort`)."],
                "`bf status` shows the sync back at work.",
            ))
        }
    };
    if role.map(|r| r != expected).unwrap_or(false) {
        return Err(refuse(
            format!("report as {}", role.unwrap_or("")),
            format!("the sync of {k} waits for its {expected}"),
            &["Report with the role the directive named."],
            "`bf status` shows the sync in its new phase.",
        ));
    }
    if p.policy.roles.tokens && !holds_token(p, &format!("{k}:{expected}"), token) {
        return Err(refuse(
            format!("report on the sync of {k} as {expected}"),
            format!("a report needs the token of the {expected}'s directive; none, or the wrong one, was given"),
            &["`bf next --json` shows the directive with its `token`; pass it with `--token`."],
            "`bf status` shows the sync in its new phase.",
        ));
    }
    let pl = plan::load(p)?;
    match (so.phase.as_str(), result) {
        ("resolve", "blocked") => {
            events::append(p, "checkpoint.sync.park", json!({"checkpoint": k, "reason": if note.is_empty() { "the resolver is blocked" } else { note }}))?;
            p.write_guard()?;
            Ok(json!({"checkpoint": k, "phase": "parked"}))
        }
        ("resolve", "built" | "fixed") => {
            let problems = sync_problems(p, &so)?;
            if !problems.is_empty() {
                let mut steps = problems.clone();
                steps.push("Fix them, then report again.".into());
                return Err(refuse_owned(
                    format!("report the resolution of {k}"),
                    format!("the resolution is not finished: {} problem(s)", problems.len()),
                    steps,
                    "`bf status` shows the sync in review.",
                ));
            }
            // The joined content must be proven: the sync's checks run on it, through the engine, and
            // the runs are recorded as sync runs.
            let failed = run_sync_checks(p, &pl, &st, k, so.floor)?;
            if !failed.is_empty() {
                let mut steps: Vec<String> = failed
                    .iter()
                    .map(|f| {
                        format!(
                            "{} fails ({}): {}",
                            f["check"].as_str().unwrap_or(""),
                            f["command"].as_str().unwrap_or(""),
                            f["tail"].as_str().unwrap_or("").lines().last().unwrap_or("")
                        )
                    })
                    .collect();
                steps.push("Repair it (only files either side changed, or new test files beside them), commit, run only the tests you change, then report again: the engine runs the sync's checks when you report.".into());
                return Err(refuse_owned(
                    format!("report the resolution of {k}"),
                    format!("the sync's checks fail on the joined content: {}", failed.len()),
                    steps,
                    "`bf status` shows the sync in review.",
                ));
            }
            // Untouched since a clean merge: nothing was resolved, so there is nothing to review.
            let untouched = so.repos.iter().all(|r| {
                r.files.is_empty()
                    && sync_wt(p, k, &r.repo)
                        .map(|wt| gitx::head_short(&wt) == r.merge)
                        .unwrap_or(false)
            });
            if untouched {
                return finish_sync(p, &pl, &so, None);
            }
            events::append(p, "checkpoint.sync.built", json!({"checkpoint": k}))?;
            p.write_guard()?;
            let skipped = skipped_k_checks(p, &pl, k)?;
            Ok(json!({"checkpoint": k, "phase": "review", "skipped_checks": skipped}))
        }
        ("review", "findings") => {
            let listed: Vec<Value> = findings
                .and_then(|f| f.as_array().cloned().or_else(|| f.get("findings").and_then(|x| x.as_array().cloned())))
                .unwrap_or_default();
            if listed.is_empty() {
                return Err(refuse(
                    format!("report findings on the sync of {k}"),
                    "no finding was given",
                    &["Give the findings (`--findings -` with a JSON list), or report `built` when the resolution is right."],
                    "`bf status` shows the sync in its new phase.",
                ));
            }
            let rounds = so.rounds + 1;
            let over = rounds > p.policy.limits.max_rounds;
            events::append(
                p,
                "checkpoint.sync.reviewed",
                json!({"checkpoint": k, "to": if over { "parked" } else { "resolve" }, "findings": listed,
                       "reason": if over { format!("more than {} review rounds", p.policy.limits.max_rounds) } else { String::new() }}),
            )?;
            p.write_guard()?;
            Ok(json!({"checkpoint": k, "phase": if over { "parked" } else { "resolve" }}))
        }
        ("review", "built") => {
            // The sync was asked for in the human's words when it opened.
            let v = finish_sync(p, &pl, &so, None)?;
            Ok(v)
        }
        (ph, r) => Err(refuse(
            format!("report {r} on the sync of {k}"),
            format!("a `{r}` report does not fit a sync in `{ph}`"),
            &["The resolver reports built or blocked; the reviewer reports findings or built (clean)."],
            "`bf status` shows the sync in its new phase.",
        )),
    }
}

/// The brief for the resolver or the reviewer of an open sync.
fn brief_sync(p: &Project, so: &events::SyncOpen, role: &str) -> R<PathBuf> {
    let k = &so.checkpoint;
    let mut md = format!("# Brief: the sync of {k} as {role}\n\n");
    md.push_str(&format!(
        "The human asked for the main branch to be taken into checkpoint {k}. It conflicts in the files below; the merge is in progress in the checkpoint's working copy `{}`.\n\n",
        p.work_dir().join(k).display()
    ));
    let limited = |t: String| -> String {
        let lines: Vec<&str> = t.lines().collect();
        if lines.len() > 200 {
            format!(
                "{}\n... ({} more lines)\n",
                lines[..200].join("\n"),
                lines.len() - 200
            )
        } else {
            t
        }
    };
    for r in &so.repos {
        if r.files.is_empty() {
            continue;
        }
        let wt = sync_wt(p, k, &r.repo)?;
        let base = gitx::must(&wt, &["merge-base", &r.before, &r.main]).unwrap_or_default();
        md.push_str(&format!("## Repository `{}`\n\n", r.repo));
        for f in &r.files {
            md.push_str(&format!("### `{f}`\n\n"));
            if role == "reviewer" {
                md.push_str("The resolution compared with the stack's side:\n\n```diff\n");
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &r.before, "HEAD", "--", f]).unwrap_or_default(),
                ));
                md.push_str(
                    "\n```\n\nThe resolution compared with the main branch's side:\n\n```diff\n",
                );
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &r.main, "HEAD", "--", f]).unwrap_or_default(),
                ));
                md.push_str("\n```\n\n");
            } else {
                md.push_str("What the stack changed:\n\n```diff\n");
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &base, &r.before, "--", f]).unwrap_or_default(),
                ));
                md.push_str("\n```\n\nWhat the main branch changed:\n\n```diff\n");
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &base, &r.main, "--", f]).unwrap_or_default(),
                ));
                md.push_str("\n```\n\n");
            }
        }
    }
    // The sync's checks on the joined content: the command checks every unit of the checkpoint is held
    // to; a checkpoint whose units share no command is checked by its own command checks.
    let pl = plan::load(p)?;
    if let Some(kr) = pl.checkpoint(k) {
        let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
        let receipts = evidence::load_receipts(p)?;
        let st = events::require_chain(p)?;
        let shared = sync_checks(p, &pl, &st, k);
        let mut entries: Vec<(String, String)> = Vec::new();
        match &shared {
            Some(list) => {
                for c in list {
                    entries.push((c.label.clone(), c.argv.join(" ")));
                }
            }
            None => {
                for cid in str_list(kr, "checks") {
                    let Some(c) = pl
                        .checks
                        .iter()
                        .find(|c| s(c, "id") == cid && s(c, "how") == "command")
                    else {
                        continue;
                    };
                    // A check the human switched off is not run by the sync, so the brief does not ask for it.
                    if crate::enforce::is_off(&st, c) {
                        continue;
                    }
                    entries.push((cid, evidence::command_words(c).join(" ")));
                }
            }
        }
        let shown = !entries.is_empty();
        if shown {
            md.push_str("## The sync's checks on the joined content\n\n");
        }
        for (cid, argv) in &entries {
            let last = receipts.iter().rev().find(|r| {
                r.content() == content
                    && if shared.is_some() {
                        s(&r.v, "type") == "sync"
                            && r.check() == k
                            && s(&r.v, "reason") == cid
                            && s(&r.v, "command") == argv
                    } else {
                        r.check() == cid && r.is_run()
                    }
            });
            let passes = |r: &Receipt| {
                if shared.is_some() {
                    r.seal_ok && r.v["exit"].as_i64() == Some(0)
                } else {
                    r.passes()
                }
            };
            match last {
                Some(r) if passes(r) => {
                    md.push_str(&format!("- {cid} (`{argv}`): passes on this content.\n"))
                }
                Some(r) => {
                    let tail = s(&r.v, "output_tail");
                    let lines: Vec<&str> = tail.lines().collect();
                    let from = lines.len().saturating_sub(40);
                    md.push_str(&format!(
                        "- {cid} (`{argv}`): FAILS (exit {}). Output tail:\n\n```\n{}\n```\n\n",
                        r.v["exit"],
                        lines[from..].join("\n")
                    ));
                }
                None => md.push_str(&format!(
                    "- {cid} (`{argv}`): not run on this content yet.\n"
                )),
            }
        }
        if shown {
            md.push_str(&format!(
                "\nThe engine runs these checks when the resolver reports, on the content in `{}`. The checkpoint's own checks run only at verification, never in a sync. Run only the tests you change, selected by name; never the checkpoint's own checks or the extended suite.\n\n",
                p.work_dir().join(k).display()
            ));
        }
    }
    if role == "builder" {
        md.push_str("## What you may change\n\nOnly files either side changed since the merge base, and new test files beside them:\n\n");
        for g in sync_allowed(so).iter().take(60) {
            md.push_str(&format!("- `{g}`\n"));
        }
        md.push('\n');
    }
    if role == "reviewer" {
        // The whole fix (resolution and repairs) against git's own automatic merge.
        for r in &so.repos {
            let wt = sync_wt(p, k, &r.repo)?;
            if let Ok(o) = gitx::run(
                &wt,
                &["merge-tree", "--write-tree", &r.before, &r.main],
                &[],
            ) {
                if let Some(tree) = o
                    .stdout
                    .lines()
                    .next()
                    .filter(|t| t.len() >= 40 && t.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    md.push_str(&format!("## Everything changed compared with git's automatic merge (`{}`)\n\n```diff\n", r.repo));
                    md.push_str(&limited(
                        gitx::must(&wt, &["diff", tree, "HEAD"]).unwrap_or_default(),
                    ));
                    md.push_str("\n```\n\n");
                }
            }
        }
    }
    if role == "reviewer" {
        md.push_str("## What to judge\n\nFor each file: does the resolution keep what the stack added AND what the main branch added, with no line lost or doubled by mistake? Report with `bf report <K> --role reviewer --result findings --findings -` (a JSON list of `{title, where, action}`), or `--result built` when every file is right. Pass the token of your directive with `--token`.\n");
    } else {
        md.push_str(&format!(
            "## Rules\n\n- Change only the conflicted files named above, in the checkpoint's working copy; nothing else. Keep both sides' intent.\n- Remove every conflict marker (`<<<<<<<`, `=======`, `>>>>>>>`).\n- Then `git add` those files and run `git commit --no-edit` there to conclude the merge. Do not abort, reset or check out anything: the engine aborts.\n- Report with `bf report {k} --role builder --result built --token <token>`. If you cannot resolve it: `bf report {k} --role builder --result blocked --note \"why\"`.\n"
        ));
        if !so.findings.is_empty() {
            md.push_str("\n## Findings of the review to fix\n\n");
            for f in &so.findings {
                md.push_str(&format!(
                    "- {} at {}: {}\n",
                    f["title"].as_str().unwrap_or(""),
                    f["where"].as_str().unwrap_or(""),
                    f["action"].as_str().unwrap_or("")
                ));
            }
        }
    }
    let path = p.briefs_dir().join(format!("{k}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

pub fn stuck(p: &Project, dry: bool) -> R<Value> {
    let st = events::require_chain(p)?;
    let now = util::now_secs();
    let limit = p.policy.limits.idle_minutes * 60;
    let mut parked = Vec::new();
    let mut listed = Vec::new();
    for (id, u) in &st.units {
        if u.state == "parked" {
            listed.push(json!({"unit": id, "reason": u.parked_reason}));
        } else if matches!(u.state.as_str(), "running" | "review" | "fixing") {
            let idle = now.saturating_sub(util::parse_rfc3339(&u.last_at));
            if idle > limit {
                parked
                    .push(json!({"unit": id, "reason": format!("idle for {} minutes", idle / 60)}));
            }
        }
    }
    if !dry {
        for x in &parked {
            events::append(
                p,
                "unit.park",
                json!({"unit": x["unit"], "reason": x["reason"]}),
            )?;
        }
        if !parked.is_empty() {
            p.write_guard()?;
        }
    }
    Ok(json!({"parked_now": parked, "already_parked": listed, "dry_run": dry}))
}

pub fn cost_status(p: &Project) -> R<Value> {
    let st = events::replay(p)?;
    let cap = p.policy.budget.cap;
    Ok(json!({
        "spent": st.cost, "cap": cap, "unit": p.policy.budget.unit,
        "left": if cap > 0.0 { json!(cap - st.cost) } else { Value::Null },
        "tiers": {"builder": p.policy.tiers.builder, "reviewer": p.policy.tiers.reviewer, "verifier": p.policy.tiers.verifier},
        "per_unit": st.units.iter().map(|(k, v)| json!({"unit": k, "cost": v.cost, "rounds": v.rounds})).collect::<Vec<_>>(),
    }))
}
