//! U-103 (A-73): `bf unit restore <U> <path>...`. Hook only: not built yet.

use crate::refusal::R;
use crate::store::Project;
use serde_json::Value;

pub fn restore(_p: &Project, _unit: &str, _paths: &[String], _token: Option<&str>) -> R<Value> {
    Err(crate::scope::not_built("restore"))
}
