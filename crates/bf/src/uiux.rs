//! `bf uiux`: the UI/UX evidence. This file belongs to this command group alone; the command line
//! calls its entry point.

use crate::refusal::{refuse, R};
use serde_json::Value;

/// Entry point of `bf uiux <sub> ...`. Wired, not built yet: it refuses until its unit lands.
pub fn run(sub: &str, _rest: &[String]) -> R<Value> {
    Err(refuse(
        format!("run `bf uiux {sub}`"),
        "this command group is wired but not built in this version of the engine",
        &["Use another command; docs/engine.md lists the ones that exist."],
        "The command prints its result.",
    ))
}
