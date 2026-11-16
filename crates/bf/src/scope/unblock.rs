//! U-100 (A-73): what an approved amendment does for units that wait on a refused file. Hook only:
//! the bodies keep today's behaviour.

use crate::events::{Amend, State};
use crate::store::Project;
use serde_json::{json, Value};

/// Replay, inside the branch of `amend.approve` that approves, after the units are re-opened.
pub fn replay(_st: &mut State, _ev: &Value, _amend: &str, _seq: u64, _at: &str) {}

/// The object whose keys are added to the `amend.approve` event data; its `unblock_units` is also
/// shown in the command's output. Empty: no key is added.
pub fn approve_fields(
    _p: &Project,
    _st: &State,
    _amendment: &Amend,
    _reopen_units: &[String],
) -> Value {
    json!({})
}
