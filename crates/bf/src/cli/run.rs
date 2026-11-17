//! `bf run start|stop`.

use super::{out, project, Args};
use crate::refusal::{refuse, R};
use crate::{enforce, run};
use serde_json::Value;

pub(crate) fn cmd_run(args: &Args) -> R<()> {
    let p = project(args)?;
    let v = match args.pos.get(1).map(|x| x.as_str()) {
        Some("start") => run::run_start(&p)?,
        Some("stop") => run::run_stop(&p)?,
        _ => {
            return Err(refuse(
                "run `bf run`",
                "unknown subcommand",
                &["Use `bf run start` or `bf run stop`."],
                "The command prints the run.",
            ))
        }
    };
    out(args, &v, || {
        let mut t = format!(
            "run {} {}",
            v["run"].as_str().unwrap_or(""),
            if v["test_mode"] == true {
                "(test mode)"
            } else {
                ""
            }
        );
        let un: Vec<Value> = v["unproven_off"].as_array().cloned().unwrap_or_default();
        if !un.is_empty() {
            t.push_str(&format!(
                "\nnot proven, their checks are off: {}",
                enforce::unproven_text(&un)
            ));
        }
        t
    });
    Ok(())
}
