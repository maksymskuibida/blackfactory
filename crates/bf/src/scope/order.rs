//! An order between two units that an old request made (D-85) no longer holds a unit back (D-115):
//! the replay of `unit.order` stays, and `place` finds nothing and `after` follows the records only.

use crate::events::State;
use crate::plan::Plan;
use crate::refusal::R;
use crate::store::{s, Project};
use serde_json::{json, Value};

/// Replay of `unit.order`.
pub fn replay(st: &mut State, ev: &Value, seq: u64, _at: &str) {
    let (unit, after) = (s(ev, "unit"), s(ev, "after"));
    if unit.is_empty() || after.is_empty() {
        return;
    }
    st.orders.push(json!({
        "unit": unit, "after": after, "path": s(ev, "path"), "seq": seq
    }));
}

/// Whether `holder` comes after `unit` because the plan's records say so, directly or through other
/// units. An order of the log finds nothing: no request orders units any more.
pub fn after(pl: &Plan, _st: &State, holder: &str, unit: &str) -> bool {
    crate::scope::depends_on(pl, holder, unit)
}

/// No unit is put after an asker, so nothing is written.
pub fn place(_p: &Project, _pl: &Plan, _st: &State, _unit: &str, _paths: &[String]) -> R<bool> {
    Ok(false)
}
