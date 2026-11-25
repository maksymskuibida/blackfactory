//! The engine's hook command for the harness's other hooks: the event's payload on standard input, the
//! answer on standard output. It never fails the harness: every error ends in exit 0 with no output.

use super::{pretty, read_stdin, start_dir, Args};
use crate::refusal::R;
use crate::store::Project;
use serde_json::Value;
use std::path::PathBuf;

pub(crate) fn cmd_hook(args: &Args) -> R<()> {
    let event = args.pos.get(1).map(|x| x.as_str()).unwrap_or("");
    let payload: Value = serde_json::from_str(&read_stdin()).unwrap_or(Value::Null);
    let dir = payload
        .get("cwd")
        .and_then(|c| c.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| start_dir(args));
    let Ok(p) = Project::discover(&dir).or_else(|_| Project::discover(&start_dir(args))) else {
        return Ok(());
    };
    if let Ok(Some(v)) = crate::hooks::run(&p, event, &payload) {
        println!("{}", pretty(&v));
    }
    Ok(())
}
