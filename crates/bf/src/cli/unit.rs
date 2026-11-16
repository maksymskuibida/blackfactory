//! `bf unit start|land|release|scope`.

use super::{out, project, Args};
use crate::refusal::{refuse, R};
use crate::run;

pub(crate) fn cmd_unit(args: &Args) -> R<()> {
    let p = project(args)?;
    let sub = args.pos_at(1, "`start`, `land` or `release`")?;
    let unit = args.pos_at(2, "a unit ID")?;
    let v = match sub {
        "start" => run::unit_start(&p, unit)?,
        "land" => run::unit_land(&p, unit)?,
        "release" => run::unit_release(&p, unit, args.flag("quote").unwrap_or(""))?,
        "scope" => {
            let n = args
                .need("revert", "the number of the grant to withdraw")?
                .trim()
                .parse::<u32>()
                .map_err(|_| {
                    refuse(
                        "withdraw a grant",
                        "`--revert` takes the number of a grant",
                        &["`bf status --json` lists each unit's scope_grants with their numbers."],
                        "The command prints the withdrawal.",
                    )
                })?;
            crate::scope::revert(&p, unit, n, args.flag("quote").unwrap_or(""))?
        }
        _ => {
            return Err(refuse(
                "run `bf unit`",
                "unknown subcommand",
                &["Use `bf unit start <U>`, `bf unit land <U>` or `bf unit release <U> --quote \"...\"`."],
                "The command prints the result.",
            ))
        }
    };
    out(args, &v, || match sub {
        "start" => format!(
            "{unit} working copy: {}",
            v["working_copy"].as_str().unwrap_or("")
        ),
        "release" => format!(
            "{unit} is back at work ({})",
            v["state"].as_str().unwrap_or("")
        ),
        "scope" => format!(
            "grant {} of {unit} is withdrawn ({})",
            v["withdrawn"],
            v["state"].as_str().unwrap_or("")
        ),
        _ => format!(
            "{unit} landed on {}",
            v["branch"].as_str().unwrap_or("its checkpoint branch")
        ),
    });
    Ok(())
}
