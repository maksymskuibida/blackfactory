//! `bf amend open|revise|narrow|approve|withdraw`.

use super::{out, pretty, project, read_data_in, Args};
use crate::gates;
use crate::refusal::{refuse, R};

pub(crate) fn cmd_amend(args: &Args) -> R<()> {
    let p = project(args)?;
    let v = match args.pos.get(1).map(|x| x.as_str()) {
        Some("open") => gates::amend_open_marked(&p, &args.pos[2..], args.has("repair"))?,
        Some("checkpoint") => {
            gates::amend_checkpoint(&p, &args.pos[2..], args.flag("title"))?
        }
        Some("revise") => gates::amend_revise(&p, &args.pos[2..])?,
        Some("narrow") => gates::amend_narrow(
            &p,
            &read_data_in(&p, args.need("file", "a narrowing file")?)?,
        )?,
        Some("withdraw") => gates::amend_withdraw(
            &p,
            args.pos.get(2).map(|x| x.as_str()),
            args.flag("reason").unwrap_or(""),
        )?,
        Some("approve") => gates::amend_approve(
            &p,
            args.pos.get(2).map(|x| x.as_str()),
            args.flag("quote").unwrap_or(""),
        )?,
        _ => {
            return Err(refuse(
                "run `bf amend`",
                "unknown subcommand",
                &["Use `bf amend open [--repair] <ID>...`, `revise <ID>...`, `checkpoint before|after|split <K-n> <U-n>...`, `narrow --file <f>`, `approve` or `withdraw <A-n>`."],
                "The command prints the amendment.",
            ))
        }
    };
    out(args, &v, || {
        if v["withdrawn"] == true {
            crate::amend::withdraw_text(&v)
        } else {
            pretty(&v)
        }
    });
    Ok(())
}
