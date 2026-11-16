//! The command groups `checks`, `scan`, `legal`, `preset`, `uiux` and `roles`, each served by a module
//! of its own.

use super::{out, pretty, project, Args};
use crate::refusal::R;

/// The new command groups, each served by a module of its own.
pub(crate) fn cmd_group(args: &Args, group: &str) -> R<()> {
    let sub = args.pos.get(1).map(|x| x.as_str()).unwrap_or("");
    let rest: Vec<String> = args.pos.iter().skip(2).cloned().collect();
    // The list of the named checks needs no project.
    if group == "checks" && matches!(sub, "" | "list") {
        let v = crate::checks::run(sub, &rest)?;
        out(args, &v, || crate::checks::text(&v));
        return Ok(());
    }
    let p = project(args)?;
    let v = match group {
        "checks" => crate::checks::run_in(&p, sub, &rest, args.flag("scope"), args.flag("quote"))?,
        "scan" => crate::scan::run(sub, &rest)?,
        "legal" => crate::legal::run(sub, &rest)?,
        "preset" => crate::preset::run(sub, &rest)?,
        "uiux" => crate::uiux::run(sub, &rest)?,
        _ => crate::roles::run_with(&p, sub, &rest, args.flag("quote"))?,
    };
    out(args, &v, || {
        if group == "checks" {
            crate::checks::text(&v)
        } else {
            pretty(&v)
        }
    });
    Ok(())
}
