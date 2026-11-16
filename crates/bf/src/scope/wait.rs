//! U-101 (A-73): units that wait for a file another unit holds, and their automatic resume. Hook
//! only: the bodies keep today's behaviour.

use crate::events::State;
use crate::plan::Plan;
use crate::refusal::R;
use crate::scope::NextEntry;
use crate::store::Project;
use serde_json::{json, Value};

/// Replay of `unit.scope.resume`.
pub fn replay(_st: &mut State, _ev: &Value, _seq: u64, _at: &str) {}

/// Resume the units that wait for a file now free; called after `amend.approve` and `unit.land`.
pub fn resume_waiting(_p: &Project) -> R<()> {
    Ok(())
}

/// Add to the builder's directive of a running or fixing unit.
pub fn decorate(_st: &State, _id: &str, _d: &mut Value) {}

/// What `bf next` gives for a blocked unit, if not the human's decision.
pub fn next_entry(_p: &Project, _pl: &Plan, _st: &State, _id: &str) -> Option<NextEntry> {
    None
}

/// Keys added to the `unit.scope.refused` event of a hard refusal.
pub fn refusal_fields(
    _p: &Project,
    _pl: &Plan,
    _st: &State,
    _unit: &str,
    _files: &[Value],
) -> Value {
    json!({})
}
