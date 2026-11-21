//! Runs: start, status, next directives, reports, briefs, unit working copies,
//! landing, checkpoint closing, stuck units and cost.

use crate::events::{self, Hold, State};
use crate::evidence::{self, Receipt};
use crate::gates;
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, refuse_owned, R};
use crate::schema;
use crate::store::{s, str_list, Project, Repo};
use crate::util;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

mod brief;
mod checkpoint;
pub mod expert;
mod land;
mod next;
pub(crate) mod pause;
mod report;
mod roles;
mod status;
mod sync;
mod tokens;
pub mod unblock;
mod unit;
pub(crate) mod unit_sync;

pub use brief::*;
pub use checkpoint::*;
pub use land::*;
pub use next::*;
pub use report::*;
pub(crate) use roles::*;
pub use status::*;
pub use sync::*;
pub use tokens::*;
pub use unblock::unit_unblock;
pub use unit::*;

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

/// Log the holds the engine can see on each checkpoint whose units have all landed and that is not closed: a
/// check marked failed or unreachable at its content, an unanswered worker's question whose unit is the
/// checkpoint, and a wave check that holds its close. Each change against the logged hold (a start, a new
/// cause or what it names, an end) is one `checkpoint.hold` event. It takes no lock beyond what its caller
/// holds and appends through `events::append`.
pub fn observe_holds(p: &Project) -> R<()> {
    let st = events::replay(p)?;
    if !st.chain_ok || st.active_run().is_none() {
        return Ok(());
    }
    let pl = plan::load(p)?;
    let mut want: BTreeMap<String, (&'static str, String)> = BTreeMap::new();
    let open_questions: Vec<Value> = p
        .list_records(schema::kind_by_name("question"))?
        .into_iter()
        .filter(|q| !s(q, "unit").is_empty() && q.get("answer").is_none())
        .collect();
    let mut receipts: Option<Vec<Receipt>> = None;
    let over_budget = budget_reached(p, &st);
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        let after = str_list(k, "after");
        if st.closed.contains_key(kid)
            || after.is_empty()
            || !after.iter().all(|u| st.unit_state(u) == "landed")
        {
            continue;
        }
        if over_budget {
            want.insert(kid.to_string(), ("budget", String::new()));
            continue;
        }
        let wave = crate::wavecheck::close_blockers(&pl, &st, kid);
        if !wave.is_empty() {
            want.insert(kid.to_string(), ("wave", wave.join("; ")));
            continue;
        }
        if let Some(q) = open_questions.iter().find(|q| s(q, "unit") == kid) {
            want.insert(kid.to_string(), ("question", s(q, "id").to_string()));
            continue;
        }
        if receipts.is_none() {
            receipts = Some(evidence::load_receipts(p)?);
        }
        let rs = receipts.as_deref().unwrap_or(&[]);
        let content = p.content_hash(&p.work_dir().join(kid)).unwrap_or_default();
        let marked: Vec<String> = str_list(k, "checks")
            .into_iter()
            .filter(|c| evidence::standing_outcome(rs, c, &content).is_some())
            .collect();
        if !marked.is_empty() {
            want.insert(kid.to_string(), ("marked", marked.join(",")));
        }
    }
    let mut ids: Vec<String> = st.ck_holds.keys().cloned().collect();
    ids.extend(
        want.keys()
            .filter(|k| !st.ck_holds.contains_key(*k))
            .cloned(),
    );
    for kid in ids {
        let logged = st
            .ck_holds
            .get(&kid)
            .map(|h| (h.cause.clone(), h.detail.clone()));
        let now = want.get(&kid).map(|(c, d)| (c.to_string(), d.clone()));
        if logged == now {
            continue;
        }
        let (cause, detail) = now.unwrap_or_default();
        events::append(
            p,
            "checkpoint.hold",
            json!({"checkpoint": kid, "cause": cause, "detail": detail}),
        )?;
    }
    Ok(())
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
        } else if matches!(u.state.as_str(), "running" | "review" | "fixing")
            && !pause::idle_exempt(p, &st, id)
        {
            let idle = now.saturating_sub(util::parse_rfc3339(&u.last_at));
            let asked = u.state == "review" && expert::asked(p, &st, id)?.is_some();
            if idle > expert::park_limit(limit, asked) {
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
