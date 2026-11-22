//! U-105 (A-73): after a builder's plain blocked report the engine gives one more builder
//! directive, to look again; a plain block made from that directive goes to the human.

use crate::events::State;
use crate::scope::NextEntry;
use crate::store::Project;
use serde_json::{json, Value};

/// Replay step run after every event.
pub fn after_event(st: &mut State, kind: &str, ev: &Value, seq: u64, at: &str) {
    let unit = ev["unit"].as_str().unwrap_or("").to_string();
    match kind {
        // Set by a plain blocked report that asks for a second look; cleared by any other report.
        "unit.report" => {
            if let Some(u) = st.units.get_mut(&unit) {
                u.reconsider = ev["reconsider"].as_bool().unwrap_or(false);
            }
        }
        // A request made from the blocked unit, or a restore that names where to go, puts the unit
        // back where it was blocked from; rounds and attempts stay as the events left them.
        "unit.scope.request" | "unit.scope.restore" => {
            if let Some(u) = st.units.get_mut(&unit) {
                let was_open = u.reconsider;
                u.reconsider = false;
                let to = if kind == "unit.scope.request" {
                    (ev["from"] == "blocked").then(|| u.prev_state.clone())
                } else {
                    ev["to"].as_str().map(str::to_string)
                };
                if let Some(to) = to.filter(|_| was_open || kind == "unit.scope.request") {
                    if u.state == "blocked" && matches!(to.as_str(), "running" | "fixing") {
                        u.state = to;
                        u.last_seq = seq;
                        u.last_at = at.to_string();
                    }
                }
            }
        }
        "unit.release" | "unit.park" => {
            if let Some(u) = st.units.get_mut(&unit) {
                u.reconsider = false;
            }
        }
        // No reviewer's `retry` gives a second look any more. A refusal of an old log leaves the unit
        // blocked in the replay, and `open` gives its builder the directive back.
        "unit.scope.refused" => {
            if let Some(u) = st.units.get_mut(&unit) {
                u.reconsider = false;
            }
        }
        _ => {}
    }
    // Whatever moved the unit out of blocked ends the second look.
    for u in st.units.values_mut() {
        if u.reconsider && u.state != "blocked" {
            u.reconsider = false;
        }
    }
}

/// The state a reconsidered unit was blocked from.
pub fn back_state(st: &State, unit: &str) -> String {
    match st.units.get(unit).map(|u| u.prev_state.as_str()) {
        Some("fixing") => "fixing".into(),
        _ => "running".into(),
    }
}

/// A unit blocked by a plain report that asks for a second look, or by a refusal of an old log that
/// no one has answered since: a request or a wait no longer holds a unit, so the builder goes on.
fn open(st: &State, unit: &str) -> bool {
    st.units.get(unit).is_some_and(|u| {
        u.state == "blocked"
            && (u.reconsider
                || st
                    .scope
                    .get(unit)
                    .and_then(|sc| sc.refused.last())
                    .is_some_and(|r| r["seq"].as_u64() == Some(u.last_seq)))
    })
}

/// What `bf next` gives for a blocked unit, before the waiting rule.
pub fn next_entry(p: &Project, st: &State, id: &str, limits: Value) -> Option<NextEntry> {
    if !open(st, id) {
        return None;
    }
    let pl = crate::plan::load(p).ok()?;
    let u = pl.unit(id)?;
    let d = json!({
        "role": "builder", "action": "reconsider", "unit": id,
        "tier": crate::run::tier_for(p, u, "builder"),
        "working_copy": p.work_dir().join(id).to_string_lossy(),
        "brief": p.briefs_dir().join(format!("{id}-builder.md")).to_string_lossy(),
        "limits": limits,
        "commands": [format!("bf brief {id} --role builder")],
    });
    Some(NextEntry::Directive(d))
}

/// The reviewer's `retry` refusal a second look is open for: none, since no reviewer judges a request.
pub fn open_retry(_st: &State, _unit: &str) -> Option<Value> {
    None
}

/// Whether a request for files is accepted from a blocked unit.
pub fn request_from(st: &State, unit: &str) -> bool {
    open(st, unit)
}
