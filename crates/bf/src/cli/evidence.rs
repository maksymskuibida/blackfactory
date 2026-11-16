//! `bf evidence run|attach|verify|fail|unreachable`.

use super::{out, pretty, project, Args};
use crate::evidence;
use crate::refusal::{refuse, R};
use crate::store::s;
use serde_json::json;
use std::path::Path;

pub(crate) fn cmd_evidence(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`run`, `attach` or `verify`")? {
        "run" => {
            let check = args.pos_at(2, "a check ID")?;
            // The checkpoint reviewer's two other acts go through the same command, which every
            // working copy may run: a probe run, and the judgement of a check that failed, then passed.
            if args.has("probe") {
                let v = crate::ckreview::probe(&p, check, &args.rest)?;
                out(args, &v, || {
                    format!("{} probe: exit {}", s(&v, "id"), v["exit"])
                });
                return Ok(());
            }
            if let Some(j) = args.flag("judge") {
                let v = crate::ckreview::judge(
                    &p,
                    check,
                    j,
                    args.flag("reason").unwrap_or(""),
                    args.flag("token"),
                )?;
                out(args, &v, || format!("{} judged {check}: {j}", s(&v, "id")));
                return Ok(());
            }
            let repeat = match args.flag("attempt") {
                Some(a) => Some((
                    a.trim().parse::<u64>().map_err(|_| {
                        refuse(
                            "repeat a run",
                            format!("`{a}` is not a number of attempt"),
                            &["Give the number of the attempt: one more than the runs already recorded."],
                            "The command prints a receipt.",
                        )
                    })?,
                    args.flag("reason").unwrap_or(""),
                )),
                None => None,
            };
            let v = evidence::run_with(
                &p,
                check,
                args.flag("unit"),
                &args.rest,
                &p.top.clone(),
                repeat,
            )?;
            let ok = v["exit"].as_i64() == Some(0);
            out(args, &v, || {
                format!(
                    "{} {}: exit {}, content {}",
                    s(&v, "id"),
                    check,
                    v["exit"],
                    &s(&v, "content_hash")[..12.min(s(&v, "content_hash").len())]
                )
            });
            if !ok {
                std::process::exit(1);
            }
        }
        "attach" => {
            let check = args.pos_at(2, "a check ID")?;
            let file = args.pos_at(3, "a file")?;
            let v = evidence::attach(
                &p,
                check,
                Path::new(file),
                args.flag("unit"),
                &evidence::Judgment {
                    outcome: args.flag("outcome"),
                    says: args.flag("says").unwrap_or(""),
                    token: args.flag("token"),
                },
            )?;
            out(args, &v, || {
                format!("{} attached {} to {}", s(&v, "id"), file, check)
            });
        }
        kind @ ("fail" | "unreachable") => {
            let check = args.pos_at(2, "a check ID")?;
            let reason = args.flag("reason").unwrap_or("");
            let v = evidence::outcome(&p, check, kind, reason, args.flag("unit"))?;
            out(args, &v, || {
                format!("{} recorded: {check} is {kind} ({reason})", s(&v, "id"))
            });
        }
        "verify" => {
            let (rows, ok) = evidence::verify(&p, args.flag("unit"), args.flag("checkpoint"))?;
            let v = json!({"ok": ok, "receipts": rows.iter().map(|r| json!({"receipt": r.receipt, "check": r.check, "seal_ok": r.seal_ok, "content_ok": r.content_ok, "exit": r.exit})).collect::<Vec<_>>()});
            if args.has("json") {
                println!("{}", pretty(&v));
            } else {
                for r in &rows {
                    println!(
                        "{} {} seal={} content={} exit={}",
                        r.receipt,
                        r.check,
                        if r.seal_ok { "ok" } else { "BAD" },
                        r.content_ok
                            .map(|c| if c { "ok" } else { "OTHER" })
                            .unwrap_or("-"),
                        r.exit.map(|e| e.to_string()).unwrap_or("-".into())
                    );
                }
                println!("{}", if ok { "evidence ok" } else { "evidence NOT ok" });
            }
            if !ok {
                std::process::exit(2);
            }
        }
        _ => {
            return Err(refuse(
                "run `bf evidence`",
                "unknown subcommand",
                &["Use `run`, `attach` or `verify`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}
