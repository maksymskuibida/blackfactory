//! `bf head accept`.

use super::{out, project_unchecked, Args};
use crate::events;
use crate::refusal::{refuse, R};

pub(crate) fn cmd_head(args: &Args) -> R<()> {
    let p = project_unchecked(args)?;
    if args.pos.get(1).map(|x| x.as_str()) != Some("accept") {
        return Err(refuse(
            "run `bf head`",
            "only `bf head accept` exists",
            &["After the human says in chat to accept the log as it stands: `bf head accept --quote \"<their words>\"`."],
            "`bf log --verify` prints `chain ok`.",
        ));
    }
    let v = events::accept_head(&p, args.flag("quote").unwrap_or(""))?;
    p.write_guard()?;
    out(args, &v, || {
        format!("accepted the log ({} events); the head is set", v["events"])
    });
    Ok(())
}
