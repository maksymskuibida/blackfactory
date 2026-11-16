//! `bf id new <kind>`: reserve the next record ID.

use super::{kind_arg, out, project, Args};
use crate::refusal::{refuse, R};
use serde_json::json;

pub(crate) fn cmd_id(args: &Args) -> R<()> {
    let p = project(args)?;
    let sub = args.pos_at(1, "`new`")?;
    if sub != "new" {
        return Err(refuse(
            "run `bf id`",
            format!("unknown subcommand `{sub}`"),
            &["Use `bf id new <kind>`."],
            "The command prints an ID.",
        ));
    }
    let k = kind_arg(args.pos_at(2, "a record kind")?)?;
    let id = p.reserve_id(k)?;
    out(args, &json!({"id": id}), || id.clone());
    Ok(())
}
