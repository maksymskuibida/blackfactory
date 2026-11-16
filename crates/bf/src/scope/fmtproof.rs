//! U-104 (A-73): `bf unit format <U> <path>...` and the landing rule for formatted files. Hook
//! only: not built yet.

use crate::events::State;
use crate::refusal::R;
use crate::store::Project;
use serde_json::Value;
use std::path::Path;

pub fn format(_p: &Project, _unit: &str, _paths: &[String]) -> R<Value> {
    Err(crate::scope::not_built("format"))
}

/// The result for one granted file at landing, when this rule decides it; None leaves the file to
/// the removed-lines rule.
pub fn landing(
    _st: &State,
    _unit: &str,
    _repo: &Path,
    _range: &str,
    _file: &Value,
) -> Option<R<()>> {
    None
}
