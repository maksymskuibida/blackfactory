//! The design fix of a checkpoint (component: checkpoint design fix). The place is made by unit U-1; the
//! stub of the marked records is U-63's (H-27) and U-25 fills it.

use crate::events::State;
use std::sync::Mutex;

/// The answer a test sets for the stub; empty in the engine.
static STUB_ANSWER: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The records that the marked items of the stored fix designs name (H-27): a plan repair that names one
/// is never judged, its judge token is refused. No record until U-25 fills it.
pub fn marked_records(_st: &State) -> Vec<String> {
    STUB_ANSWER.lock().map(|v| v.clone()).unwrap_or_default()
}

/// Set the answer of the stub (for a test of the judge's refusal; U-25 replaces the stub and this).
pub fn set_stub_answer(records: Vec<String>) {
    if let Ok(mut v) = STUB_ANSWER.lock() {
        *v = records;
    }
}
