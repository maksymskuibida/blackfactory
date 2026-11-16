//! `bf gate open|skip|approve|reject|status`.

use super::{out, project, Args};
use crate::refusal::{refuse, R};
use crate::{events, gates};
use serde_json::json;

pub(crate) fn cmd_gate(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`open`, `approve`, `reject` or `status`")? {
        "open" => {
            let kind = args.pos_at(2, "a gate kind")?;
            let g = gates::open(&p, kind, args.flag("ref"))?;
            out(args, &gates::gate_json(&g), || g.id.clone());
        }
        "skip" => {
            let kind = args.pos_at(2, "a gate kind (ui)")?;
            let v = gates::skip(&p, kind, args.flag("quote").unwrap_or(""))?;
            out(args, &v, || format!("the {kind} gate is skipped"));
        }
        "approve" => {
            let g = gates::approve(
                &p,
                args.pos_at(2, "a gate ID")?,
                args.has("simulated"),
                args.flag("quote").unwrap_or(""),
            )?;
            out(args, &gates::gate_json(&g), || {
                format!(
                    "approved {}{}",
                    g.id,
                    if g.simulated { " (simulated)" } else { "" }
                )
            });
        }
        "reject" => {
            let g = gates::reject(
                &p,
                args.pos_at(2, "a gate ID")?,
                args.flag("note").unwrap_or(""),
                args.flag("quote").unwrap_or(""),
            )?;
            out(args, &gates::gate_json(&g), || format!("rejected {}", g.id));
        }
        "status" => {
            let st = events::replay(&p)?;
            let v = json!(st.gates.iter().map(gates::gate_json).collect::<Vec<_>>());
            out(args, &v, || {
                if st.gates.is_empty() {
                    "no gates yet".into()
                } else {
                    st.gates
                        .iter()
                        .map(|g| {
                            format!(
                                "{} {}{}{}",
                                g.id,
                                g.status,
                                if g.simulated { " (simulated)" } else { "" },
                                if g.quote.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        " - decided on the words: \"{}\" (the human's whole message: \"{}\"){}",
                                        g.quote,
                                        g.message,
                                        if g.caution.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" - caution: {}", g.caution)
                                        } + &if g.transcript.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" - transcript: {}", g.transcript)
                                        }
                                    )
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            });
        }
        _ => {
            return Err(refuse(
                "run `bf gate`",
                "unknown subcommand",
                &["Use `open`, `approve`, `reject` or `status`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}
