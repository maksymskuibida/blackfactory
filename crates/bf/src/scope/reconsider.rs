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
        "unit.release" | "unit.park" | "unit.scope.refused" => {
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

fn open(st: &State, unit: &str) -> bool {
    st.units
        .get(unit)
        .is_some_and(|u| u.reconsider && u.state == "blocked")
}

/// What `bf next` gives for a blocked unit, before the waiting rule.
pub fn next_entry(p: &Project, st: &State, id: &str, limits: Value) -> Option<NextEntry> {
    if !open(st, id) {
        return None;
    }
    let pl = crate::plan::load(p).ok()?;
    let u = pl.unit(id)?;
    Some(NextEntry::Directive(json!({
        "role": "builder", "action": "reconsider", "unit": id,
        "tier": crate::run::tier_for(p, u, "builder"),
        "working_copy": p.work_dir().join(id).to_string_lossy(),
        "brief": p.briefs_dir().join(format!("{id}-builder.md")).to_string_lossy(),
        "limits": limits,
        "commands": [format!("bf brief {id} --role builder")],
    })))
}

/// Whether a request for files is accepted from a blocked unit.
pub fn request_from(st: &State, unit: &str) -> bool {
    open(st, unit)
}
