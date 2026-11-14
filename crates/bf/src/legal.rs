//! `bf legal`: the markets statement and the legal audit with its legal findings. This file belongs
//! to this command group alone; the command line calls its entry point.

use crate::refusal::{refuse, R};
use crate::store::Project;
use serde_json::Value;

/// Handoff H-3: the legal findings that are marked and not yet acknowledged, one line each. The close
/// of a checkpoint and of the run asks for them and refuses while there is one. The engine's
/// enforcement part calls it; the legal module fills it in.
pub fn unacknowledged(_p: &Project) -> R<Vec<String>> {
    Ok(Vec::new())
}

/// Entry point of `bf legal <sub> ...`. Wired, not built yet: it refuses until its unit lands.
pub fn run(sub: &str, _rest: &[String]) -> R<Value> {
    Err(refuse(
        format!("run `bf legal {sub}`"),
        "this command group is wired but not built in this version of the engine",
        &["Use another command; docs/engine.md lists the ones that exist."],
        "The command prints its result.",
    ))
}
