//! U-109 (A-73): `bf unit sync <U>`. Hook only: not built yet.

use crate::events::State;
use crate::refusal::R;
use crate::store::Project;
use serde_json::Value;

/// Replay of `unit.sync`.
pub(crate) fn replay(_st: &mut State, _ev: &Value, _seq: u64, _at: &str) {}

pub(crate) fn unit_sync(_p: &Project, _unit: &str) -> R<Value> {
    Err(crate::scope::not_built("sync"))
}
