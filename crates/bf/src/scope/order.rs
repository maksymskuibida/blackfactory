//! U-106 (A-73): an order between two units of a checkpoint, set after the plan was made. Hook
//! only: the bodies keep today's behaviour.

use crate::events::State;
use crate::plan::Plan;
use crate::refusal::R;
use crate::store::Project;
use serde_json::Value;

/// Replay of `unit.order`.
pub fn replay(_st: &mut State, _ev: &Value, _seq: u64, _at: &str) {}

/// Whether `holder` comes after `unit`: today, whether it depends on it in the plan.
pub fn after(pl: &Plan, _st: &State, holder: &str, unit: &str) -> bool {
    crate::scope::depends_on(pl, holder, unit)
}

/// Place `unit` after a holder of the asked files when the rules allow it; true when it wrote an
/// order, so the caller reads the state again.
pub fn place(_p: &Project, _pl: &Plan, _st: &State, _unit: &str, _paths: &[String]) -> R<bool> {
    Ok(false)
}
