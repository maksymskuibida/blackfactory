//! The state of every unit, the pages' links and `bf status`.

use super::*;

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
        } else if let Some(a) = super::next::amend_hold(st, &id) {
            // An open amendment holds the unit: it is not ready, the same as in `bf next` (F-197, F-211).
            let why = crate::strings::t("next.unit_held.line", &[("unit", &id), ("amend", &a)]);
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
    let heads: Vec<gitx::Heads> = p.repos.iter().map(|r| gitx::heads(&r.path)).collect();
    let repo_positions = super::checkpoint::main_positions_in(p, &st, &heads)?;
    let main = combine_positions(&repo_positions);
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let states = unit_states(p, &pl, &st, &receipts);
    let log_events = events::load_log(p).map(|l| l.events).unwrap_or_default();
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
            "sync_conflicts": super::unit_sync::unmerged(&p.work_dir().join(i)),
            "sync_aborted": super::unit_sync::aborted_in(p, &log_events, i),
            "scope_request": crate::scope::request_json(&st, i),
            "scope_grants": crate::scope::grants_json(&st, i),
            "checkpoint_fixes": crate::ckfix::fixes_of_unit(&st, i),
            "review_rounds": st.units.get(i).map(|u| u.review_rounds.clone()).unwrap_or_default()})).collect::<Vec<_>>(),
        "checkpoint_fix": crate::ckfix::status_json(&st),
        "wave_checks": crate::wavecheck::status_json(&st),
        "plan_questions": crate::wavecheck::plan_questions(&st),
        "checkpoint_reviews": crate::ckreview::status_json(p, &st, &pl),
        "unit_counts": counts,
        "gates": st.gates.iter().map(gates::gate_json).collect::<Vec<_>>(),
        "amendments": gates::amendments_json(&st),
        "repairs": st.amends.iter().filter(|a| crate::amend::is_repair(a)).map(|a| a.id.clone()).collect::<Vec<_>>(),
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
        "main_ahead": super::sync::main_ahead_in(p, &st, &pl, &heads),
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
