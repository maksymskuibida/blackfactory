//! `bf page set|list`.

use super::{out, project, Args};
use crate::refusal::{refuse, R};
use crate::store::s;
use crate::{events, run};
use serde_json::json;

pub(crate) fn cmd_page(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`set` or `list`")? {
        "set" => {
            let v = run::page_set(
                &p,
                args.pos_at(2, "a page name")?,
                args.pos_at(3, "the page's secure web address")?,
            )?;
            out(args, &v, || {
                format!("{} is at {}", s(&v, "name"), s(&v, "url"))
            });
        }
        "list" => {
            let st = events::replay(&p)?;
            let v = json!(run::pages_json(&st));
            out(args, &v, || {
                run::pages_json(&st)
                    .iter()
                    .map(|r| format!("{} {}", s(r, "name"), s(r, "url")))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        _ => {
            return Err(refuse(
                "run `bf page`",
                "unknown subcommand",
                &["Use `bf page set <name> <url>` or `bf page list`."],
                "The command prints the result.",
            ))
        }
    }
    Ok(())
}
