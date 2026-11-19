//! A checkpoint fix's file requests (D-91, H-22). The events and their replay live in `events.rs`;
//! the stubs here change nothing until the unit that owns them fills them.

use crate::events::State;
use crate::refusal::R;
use crate::store::Project;
use serde_json::{json, Value};

/// The builder's request for files beyond the fix's list. A stub: it writes nothing.
pub fn request(_p: &Project, _checkpoint: &str, _files: &[Value]) -> R<Value> {
    Ok(json!({}))
}

/// The scope reviewer's verdict on a request. A stub: it writes nothing.
pub fn verdict(_p: &Project, _checkpoint: &str, _verdict: &Value) -> R<Value> {
    Ok(json!({}))
}

/// The files granted to the open fix of `checkpoint`. A stub: none.
pub fn granted(_st: &State, _checkpoint: &str) -> Vec<String> {
    Vec::new()
}
