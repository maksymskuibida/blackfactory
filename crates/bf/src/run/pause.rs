//! U-102 (A-73): `bf unit pause` and `bf unit resume`. Hook only: the bodies keep today's
//! behaviour.

use crate::events::State;
use crate::refusal::R;
use crate::store::Project;
use serde_json::Value;

/// Replay of `unit.pause` and `unit.resume`.
pub(crate) fn replay(_st: &mut State, _ev: &Value, _seq: u64, _at: &str) {}

/// Replay step run after every event that reaches the end of the match.
pub(crate) fn after_event(_st: &mut State, _kind: &str, _ev: &Value) {}

/// Whether `bf stuck` leaves the unit alone.
pub(crate) fn idle_exempt(_p: &Project, _st: &State, _id: &str) -> bool {
    false
}

/// The line `bf next` puts in `waiting` for a unit that gets nothing else.
pub(crate) fn waiting_line(_st: &State, _id: &str) -> Option<String> {
    None
}

pub(crate) fn unit_pause(_p: &Project, _unit: &str) -> R<Value> {
    Err(crate::scope::not_built("pause"))
}

pub(crate) fn unit_resume(_p: &Project, _unit: &str) -> R<Value> {
    Err(crate::scope::not_built("resume"))
}
