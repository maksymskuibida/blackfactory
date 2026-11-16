//! `bf policy accept|diff`.

use super::{out, project_unchecked, Args};
use crate::gates;
use crate::refusal::{refuse, R};
use serde_json::json;

pub(crate) fn cmd_policy(args: &Args) -> R<()> {
    let p = project_unchecked(args)?;
    match args.pos.get(1).map(|x| x.as_str()) {
        Some("accept") => {
            let v = gates::accept_policy(&p, args.flag("quote").unwrap_or(""))?;
            out(args, &v, || "policy accepted".to_string());
        }
        Some("diff") => {
            let d = gates::policy_diff(&p);
            out(args, &json!({"diff": d}), || d.clone());
        }
        _ => {
            return Err(refuse(
                "run `bf policy`",
                "only `accept` and `diff` exist",
                &["`bf policy diff` shows what changed; `bf policy accept --quote \"<the human's words>\"` takes it."],
                "`bf status` runs again.",
            ))
        }
    }
    Ok(())
}
