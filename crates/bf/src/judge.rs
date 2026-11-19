//! The repair judge's side of the log (D-91, H-21). The events and their replay live in `events.rs`;
//! this module holds what a later unit fills.

use crate::events::State;
use serde_json::Value;

/// Whether an `amend.approve` written by the judge (`by = "judge"`, with `judge_seq`) counts on replay.
/// A stub until the judge's own unit: no approval by the judge counts.
pub fn approve_counts(_st: &State, _ev: &Value) -> bool {
    false
}
