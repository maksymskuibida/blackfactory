//! U-105 (A-73): a blocked unit the plan repair has since freed is reconsidered. Hook only: the
//! bodies keep today's behaviour.

use crate::events::State;
use crate::scope::NextEntry;
use crate::store::Project;
use serde_json::Value;

/// Replay step run after every event.
pub fn after_event(_st: &mut State, _kind: &str, _ev: &Value, _seq: u64, _at: &str) {}

/// What `bf next` gives for a blocked unit, before the waiting rule.
pub fn next_entry(_p: &Project, _st: &State, _id: &str, _limits: Value) -> Option<NextEntry> {
    None
}

/// Whether a request for files is accepted from a blocked unit.
pub fn request_from(_st: &State, _unit: &str) -> bool {
    false
}
