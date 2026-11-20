//! `bf unit start|land|release|unblock|scope|pause|resume|accept|restore|format|sync`.

use super::{out, project, Args};
use crate::refusal::{refuse, R};
use crate::run;
use crate::strings::t;

pub(crate) fn cmd_unit(args: &Args) -> R<()> {
    let p = project(args)?;
    let sub = args.pos_at(1, &t("cli.unit_accept.subcommands", &[]))?;
    let unit = args.pos_at(2, "a unit ID")?;
    let paths: Vec<String> = args.pos.iter().skip(3).cloned().collect();
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
        "unblock" => {
            let why = args.flag("why").unwrap_or("");
            run::unit_unblock(&p, unit, why, args.has("wait"), args.has("human"))?
        }
        "pause" => run::pause::unit_pause(&p, unit)?,
        "resume" => run::pause::unit_resume(&p, unit)?,
        "accept" => {
            let ids: Vec<String> = paths
                .iter()
                .flat_map(|x| x.split(',').map(str::to_string).collect::<Vec<_>>())
                .filter(|x| !x.is_empty())
                .collect();
            crate::unitaccept::accept(&p, unit, &ids, args.flag("quote").unwrap_or(""))?
        }
        "restore" => crate::scope::restore::restore(&p, unit, &paths, args.flag("token"))?,
        "format" => crate::scope::fmtproof::format(&p, unit, &paths)?,
        "sync" => run::unit_sync::unit_sync(&p, unit)?,
        _ => {
            return Err(refuse(
                "run `bf unit`",
                "unknown subcommand",
                &[&t("cli.unit_accept.usage", &[])],
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
        "unblock" | "pause" | "resume" | "accept" | "restore" | "format" | "sync" => {
            v["line"].as_str().unwrap_or("").to_string()
        }
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
