//! U-102 (A-73): `bf unit pause` and `bf unit resume`, and the idle clock that stops while a unit
//! is paused or waits for the human.

use crate::events::{self, State};
use crate::refusal::{refuse, R};
use crate::schema;
use crate::store::{s, Project};
use serde_json::{json, Value};

/// Replay of `unit.pause` and `unit.resume`.
pub(crate) fn replay(st: &mut State, ev: &Value, _seq: u64, _at: &str) {
    let paused = ev["kind"].as_str() == Some("unit.pause");
    if let Some(u) = ev["unit"].as_str().and_then(|id| st.units.get_mut(id)) {
        u.paused = paused;
    }
}

/// Replay step run after every event that reaches the end of the match.
pub(crate) fn after_event(st: &mut State, kind: &str, ev: &Value) {
    if matches!(
        kind,
        "unit.report"
            | "unit.scope.request"
            | "unit.scope.grant"
            | "unit.scope.refused"
            | "unit.park"
            | "unit.release"
            | "unit.land"
    ) {
        if let Some(u) = ev["unit"].as_str().and_then(|id| st.units.get_mut(id)) {
            u.paused = false;
        }
    }
}

/// Whether the unit has a question without an answer, or one the human answered within the idle
/// limit: the answer starts the clock again, though it is logged without the unit.
fn waited_for_the_human(p: &Project, id: &str) -> bool {
    let limit = p.policy.limits.idle_minutes * 60;
    let now = crate::util::now_secs();
    p.list_records(schema::kind_by_name("question"))
        .map(|qs| {
            qs.iter().filter(|q| s(q, "unit") == id).any(|q| {
                q.get("answer").is_none()
                    || now.saturating_sub(crate::util::parse_rfc3339(s(q, "answered_at"))) <= limit
            })
        })
        .unwrap_or(false)
}

fn named_by_open_amendment(st: &State, id: &str) -> bool {
    st.amends.iter().any(|a| {
        matches!(a.status.as_str(), "open" | "narrowed")
            && (a.ids.iter().any(|x| x == id) || a.keep.iter().any(|x| x == id))
    })
}

/// Whether `bf stuck` leaves the unit alone.
pub(crate) fn idle_exempt(p: &Project, st: &State, id: &str) -> bool {
    st.units.get(id).is_some_and(|u| u.paused)
        || waited_for_the_human(p, id)
        || named_by_open_amendment(st, id)
}

/// The line `bf next` puts in `waiting` for a unit that gets nothing else.
pub(crate) fn waiting_line(st: &State, id: &str) -> Option<String> {
    st.units
        .get(id)
        .filter(|u| u.paused)
        .map(|_| format!("{id}: paused; `bf unit resume {id}` puts it back to work"))
}

fn known<'a>(st: &'a State, cmd: &str, unit: &str) -> R<&'a events::UnitSt> {
    st.units.get(unit).ok_or_else(|| {
        refuse(
            format!("{cmd} {unit}"),
            "that unit has not been started",
            &["Use a unit ID from `bf status`."],
            "`bf status` lists the unit.",
        )
    })
}

pub(crate) fn unit_pause(p: &Project, unit: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    crate::run::need_run(&st)?;
    let u = known(&st, "pause", unit)?;
    let what = format!("pause {unit}");
    let hint = "`bf next` shows what each unit is waiting for.";
    if u.paused {
        return Err(refuse(
            what,
            "it is paused already",
            &["`bf unit resume <U>` puts it back to work."],
            hint,
        ));
    }
    if !matches!(u.state.as_str(), "running" | "review" | "fixing") {
        return Err(refuse(
            what,
            format!(
                "it is `{}`; only a running, review or fixing unit is paused",
                u.state
            ),
            &[hint],
            "`bf status` shows the unit paused.",
        ));
    }
    if st.scope.get(unit).is_some_and(|x| x.open.is_some()) {
        return Err(refuse(
            what,
            "it has an open request for files",
            &["Wait for the answer to the request; `bf next` shows it."],
            "`bf status` shows the unit paused.",
        ));
    }
    events::append(p, "unit.pause", json!({"unit": unit, "from": u.state}))?;
    p.write_guard()?;
    Ok(json!({"unit": unit, "paused": true, "state": u.state,
        "line": format!("{unit} is paused; `bf unit resume {unit}` puts it back to work")}))
}

pub(crate) fn unit_resume(p: &Project, unit: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    crate::run::need_run(&st)?;
    let u = known(&st, "resume", unit)?;
    if !u.paused {
        return Err(refuse(
            format!("resume {unit}"),
            "it is not paused",
            &["`bf next` shows what each unit is waiting for."],
            "`bf status` shows the unit at work.",
        ));
    }
    events::append(p, "unit.resume", json!({"unit": unit, "state": u.state}))?;
    p.write_guard()?;
    Ok(json!({"unit": unit, "paused": false, "state": u.state,
        "line": format!("{unit} is back at work ({})", u.state)}))
}
