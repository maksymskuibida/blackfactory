//! U-105 (A-73): the marks of a builder's plain blocked report. No directive asks the builder to look
//! again any more; the conductor judges the block (D-102) and the human decides after.

use crate::events::State;
use serde_json::Value;

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

/// The reviewer's `retry` refusal a second look is open for: none, since no reviewer judges a request.
pub fn open_retry(_st: &State, _unit: &str) -> Option<Value> {
    None
}

/// Whether a request for files is accepted from a blocked unit.
pub fn request_from(st: &State, unit: &str) -> bool {
    open(st, unit)
}
