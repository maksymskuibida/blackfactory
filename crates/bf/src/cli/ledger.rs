//! `bf ledger check|impact`.

use super::{out, pretty, project, Args};
use crate::refusal::{refuse, R};
use crate::{enforce, ledger, plan};
use serde_json::{json, Value};

pub(crate) fn cmd_ledger(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`check` or `impact`")? {
        "check" => {
            let pl = plan::load(&p)?;
            let plan_only = args.has("plan");
            let rows = ledger::check(&p, &pl, plan_only)?;
            let gaps: Vec<&ledger::Row> = rows.iter().filter(|r| !r.missing.is_empty()).collect();
            if args.has("json") && !gaps.is_empty() {
                println!(
                    "{}",
                    pretty(
                        &json!({"ok": false, "gaps": gaps.iter().map(|g| json!({"id": g.id, "title": g.title, "missing": g.missing})).collect::<Vec<_>>()})
                    )
                );
                std::process::exit(2);
            }
            ledger::refuse_if_gaps(&rows, plan_only)?;
            let off = if plan_only {
                json!([])
            } else {
                enforce::refuse_if_unproven(&p, &pl)?
            };
            let unproven: Vec<Value> = rows
                .iter()
                .filter(|r| !r.off.is_empty())
                .map(|r| json!({"id": r.id, "checks": r.off, "run": r.off_runs.join(", ")}))
                .collect();
            out(
                args,
                &json!({"ok": true, "parts": rows.len(), "checks_off": off, "unproven_off": unproven}),
                || {
                    let o: Vec<Value> = off.as_array().cloned().unwrap_or_default();
                    let mut t = format!("ledger closed: {} designed parts linked", rows.len());
                    if !o.is_empty() {
                        t.push_str(&format!("\nchecks switched off: {}", enforce::off_text(&o)));
                    }
                    if !unproven.is_empty() {
                        t.push_str(&format!(
                            "\nnot demanded, their checks are off: {}",
                            enforce::unproven_text(&unproven)
                        ));
                    }
                    t
                },
            );
        }
        "impact" => {
            if args.pos.len() < 3 {
                return Err(refuse(
                    "compute an impact set",
                    "no IDs were given",
                    &["Run `bf ledger impact R-1 C-2`."],
                    "The command prints the linked IDs.",
                ));
            }
            let pl = plan::load(&p)?;
            let v = ledger::impact(&p, &pl, &args.pos[2..])?;
            out(args, &v, || pretty(&v));
        }
        _ => {
            return Err(refuse(
                "run `bf ledger`",
                "unknown subcommand",
                &["Use `bf ledger check` or `bf ledger impact <ID>...`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}
