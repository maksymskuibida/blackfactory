//! `bf plan check`.

use super::{out, pretty, project, Args};
use crate::plan;
use crate::refusal::{refuse, R};
use serde_json::json;

pub(crate) fn cmd_plan(args: &Args) -> R<()> {
    let p = project(args)?;
    if args.pos.get(1).map(|x| x.as_str()) != Some("check") {
        return Err(refuse(
            "run `bf plan`",
            "only `bf plan check` exists",
            &["Run `bf plan check`."],
            "The command prints `plan ok`.",
        ));
    }
    let pl = plan::load(&p)?;
    let waves = pl.waves();
    let v = json!({"ok": pl.problems.is_empty(), "problems": pl.problems, "waves": waves});
    if args.has("json") && !pl.problems.is_empty() {
        println!("{}", pretty(&v));
        std::process::exit(2);
    }
    plan::check_or_refuse(&pl)?;
    out(args, &v, || {
        format!(
            "plan ok: {} units in {} waves, {} checkpoints",
            pl.units.len(),
            waves.len(),
            pl.checkpoints.len()
        )
    });
    Ok(())
}
