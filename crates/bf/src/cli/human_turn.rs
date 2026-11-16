//! `bf human-turn`: the hook that records a message of the human.

use super::{pretty, project, read_stdin, start_dir, Args};
use crate::gates;
use crate::refusal::{err, R};
use crate::store::Project;
use serde_json::{json, Value};
use std::path::PathBuf;

pub(crate) fn cmd_human_turn(args: &Args) -> R<()> {
    let payload: Value = serde_json::from_str(&read_stdin())
        .map_err(|e| err(format!("the hook payload is not JSON: {e}")))?;
    let dir = payload
        .get("cwd")
        .and_then(|c| c.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| start_dir(args));
    let p = Project::discover(&dir).or_else(|_| project(args))?;
    let (recorded, ev) = gates::human_turn(&p, &payload)?;
    if args.has("json") {
        println!("{}", pretty(&json!({"recorded": recorded, "event": ev})));
    }
    Ok(())
}
