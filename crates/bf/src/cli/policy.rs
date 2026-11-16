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
        None => {
            // The policy in force: the kinds of work with their tier, effort and any model. Advising has
            // no directive and no token; this is where the conductor reads its entry.
            let w = crate::run::work_policy(&p);
            out(args, &json!({"work": w}), || {
                let mut t = String::new();
                for k in crate::store::WORK_KINDS {
                    let e = &w[k];
                    t.push_str(&format!(
                        "{k}: tier {} effort {}",
                        e["tier"].as_str().unwrap_or(""),
                        e["effort"].as_str().unwrap_or("")
                    ));
                    if let Some(m) = e["model"].as_object() {
                        for (h, name) in m {
                            t.push_str(&format!(" model {h}={}", name.as_str().unwrap_or("")));
                        }
                    }
                    t.push('\n');
                }
                t
            });
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
