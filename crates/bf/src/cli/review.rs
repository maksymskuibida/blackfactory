//! `bf review token|split|brief`: the tokens and briefs of the design reviewer and of the stage agents
//! of an amendment, and the general designer's split.

use super::{out, pretty, project, Args};
use crate::refusal::{refuse, R};
use crate::roles::design;
use serde_json::json;

pub(crate) fn cmd_review(args: &Args) -> R<()> {
    let p = project(args)?;
    let subject = args.flag("subject");
    let v = match args.pos.get(1).map(|x| x.as_str()) {
        Some("token") => {
            if let Some(stage) = args.flag("stage") {
                let amend = args.need("subject", "the amendment the stage belongs to")?;
                let role = args.need("role", "the role of the stage agent")?;
                design::issue_stage_token(&p, amend, stage, role)?
            } else {
                design::issue_kind_token(&p, args.pos_at(2, "a review kind")?, subject)?
            }
        }
        Some("split") => {
            let amend = args.pos_at(2, "an amendment ID")?;
            let stages = args.multi.get("stage").cloned().unwrap_or_default();
            design::split(&p, amend, &stages, args.flag("token"))?
        }
        Some("brief") => {
            let what = args.pos_at(2, "a review kind or a stage")?;
            let path = if args.flag("role").is_some() || design::stage_only(what) {
                let amend = args.need("subject", "the amendment the stage belongs to")?;
                design::brief_stage(&p, what, amend, args.flag("role"))?
            } else {
                design::brief_kind(&p, what, subject)?
            };
            json!({"path": path.to_string_lossy()})
        }
        _ => {
            return Err(refuse(
                "run `bf review`",
                "unknown subcommand",
                &["Use `bf review token <kind> [--subject A-n]`, `bf review token --subject A-n --stage <stage> --role <role>`, `bf review split <A-n> --stage <stage>=<ID,ID> --token <token>` or `bf review brief <kind or stage> [--subject A-n]`."],
                "The command prints its result.",
            ))
        }
    };
    out(args, &v, || match v["path"].as_str() {
        Some(path) => path.to_string(),
        None => pretty(&v),
    });
    Ok(())
}
