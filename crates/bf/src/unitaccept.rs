//! The human's accept of a unit's findings (D-91, H-24). The event and its replay live in `events.rs`
//! and `accept.rs`; the stub here changes nothing until U-61 fills it.

use crate::refusal::R;
use crate::store::Project;
use serde_json::{json, Value};

/// Accept findings of a parked unit in the human's own words. A stub: it writes nothing.
pub fn accept(_p: &Project, _unit: &str, _findings: &[String], _quote: &str) -> R<Value> {
    Ok(json!({}))
}
