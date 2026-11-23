//! A unit that waited for a file another unit held is no longer held back (D-115): the replay of an
//! old `unit.scope.resume` stays, and every other function finds nothing.

use crate::events::State;
use crate::plan::Plan;
use crate::refusal::R;
use crate::store::Project;
use serde_json::{json, Value};

/// Replay: a unit still blocked goes to the listed state; rounds and attempts stay.
pub fn replay(st: &mut State, ev: &Value, seq: u64, at: &str) {
    let (id, to) = (
        ev["unit"].as_str().unwrap_or(""),
        ev["to"].as_str().unwrap_or(""),
    );
    let Some(u) = st.units.get_mut(id) else {
        return;
    };
    if u.state != "blocked" || !matches!(to, "running" | "fixing") {
        return;
    }
    u.state = to.to_string();
    u.last_seq = seq;
    u.last_at = at.to_string();
    let from = ev["refusal"].as_u64();
    if let Some(r) = st.scope.get_mut(id).and_then(|sc| {
        sc.refused
            .iter_mut()
            .rev()
            .find(|r| r["seq"].as_u64() == from)
    }) {
        r["resumed_seq"] = json!(seq);
    }
}

/// No unit waits for a holder any more, so there is nothing to resume.
pub fn resume_waiting(_p: &Project) -> R<()> {
    Ok(())
}

/// A refusal carries no list of holders any more.
pub fn refusal_fields(
    _p: &Project,
    _pl: &Plan,
    _st: &State,
    _unit: &str,
    _files: &[Value],
) -> Value {
    json!({})
}

/// The directive that asked a conductor for a plan repair when units waited for each other. No unit
/// waits any more, so nothing calls it; the plan that follows takes it out with its table row.
pub fn circle_directive(id: &str, ring: &[String], files: &[String]) -> Value {
    let note = crate::strings::t(
        "wait.circle.note",
        &[
            ("id", id),
            ("ring", &ring.join(" -> ")),
            ("files", &files.join(", ")),
            ("ids", &ring.join(" ")),
        ],
    );
    json!({"role": "conductor", "action": "open-plan-repair", "unit": id, "state": "blocked", "ring": ring, "files": files, "note": note})
}
