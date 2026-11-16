//! `bf report <unit> --result ...`.

use super::{out, project, read_data_in, read_stdin, Args};
use crate::refusal::{refuse, R};
use crate::run;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub(crate) fn cmd_report(args: &Args) -> R<()> {
    let p = project(args)?;
    let unit = args.pos_at(1, "a unit ID")?;
    let result = args.need("result", "a result: built, findings, fixed or blocked")?;
    let need_files = match args.flag("need-files") {
        Some(f) => {
            let raw = if f == "-" {
                read_stdin()
            } else {
                let safe = p.safe_input(Path::new(f))?;
                fs::read_to_string(safe).map_err(|e| {
                    refuse(
                        "read the request for files",
                        format!("{f} cannot be read: {e}"),
                        &["Pipe the JSON list into the command with `--need-files -`."],
                        "The command prints the new state of the unit.",
                    )
                })?
            };
            Some(serde_json::from_str::<Value>(&raw).map_err(|e| {
                refuse(
                    "read the request for files",
                    format!("the input is not valid JSON: {e}"),
                    &["Pipe a JSON list such as [{\"path\": \"tests/a.rs\", \"why\": \"...\", \"patch\": \"...\"}] into the command."],
                    "The command prints the new state of the unit.",
                )
            })?)
        }
        None => None,
    };
    let findings_flag = args.flag("findings").or(args.flag("verdict"));
    let findings = match findings_flag {
        Some("-") => Some(serde_json::from_str::<Value>(&read_stdin()).map_err(|e| {
            refuse(
                "read findings from stdin",
                format!("the input is not valid JSON: {e}"),
                &["Pipe a JSON list such as [{\"title\": \"...\", \"severity\": \"high\"}] into the command."],
                "The command prints the new state of the unit.",
            )
        })?),
        Some(f) => Some(read_data_in(&p, f)?),
        None => None,
    };
    // The spend is the conductor's to report, from the session's real usage. With a budget cap
    // a report without it is refused, and text that is not a number never counts as zero.
    let cost = match args.flag("cost") {
        Some(c) => c.trim().parse::<f64>().ok().filter(|x| x.is_finite() && *x >= 0.0).ok_or_else(|| {
            refuse(
                "report with this cost",
                format!("`{c}` is not a number of at least 0"),
                &["Pass the spend as a plain number, for example `--cost 0.42`."],
                "The command prints the unit's new state.",
            )
        })?,
        None if p.policy.budget.cap > 0.0 => {
            return Err(refuse(
                "report without a cost",
                "the policy has a budget cap, so every report must say what the work cost",
                &["The conductor reads the spend of the session and adds `--cost <amount>` to the report."],
                "The command prints the unit's new state.",
            ))
        }
        None => 0.0,
    };
    if let Some(list) = &need_files {
        if result != "blocked" {
            return Err(refuse(
                "ask for files",
                "a request for files comes with `--result blocked`",
                &["Report `bf report <unit> --result blocked --token <token> --need-files -` with the list on stdin."],
                "The command prints the new state of the unit.",
            ));
        }
        let v = crate::scope::request(
            &p,
            unit,
            args.flag("note").unwrap_or(""),
            cost,
            args.flag("token"),
            list,
        )?;
        out(args, &v, || {
            let refused: Vec<String> = v["refused"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| {
                    format!(
                        "{}: {}",
                        r["path"].as_str().unwrap_or(""),
                        r["rule"].as_str().unwrap_or("")
                    )
                })
                .collect();
            if refused.is_empty() {
                format!(
                    "{unit} asked for files; a reviewer judges the request ({})",
                    v["state"].as_str().unwrap_or("")
                )
            } else {
                format!(
                    "{unit} is now {}; refused: {}",
                    v["state"].as_str().unwrap_or(""),
                    refused.join("; ")
                )
            }
        });
        return Ok(());
    }
    let v = run::report(
        &p,
        unit,
        result,
        args.flag("role"),
        args.flag("note").unwrap_or(""),
        findings.as_ref(),
        cost,
        args.flag("token"),
    )?;
    out(args, &v, || {
        format!("{unit} is now {}", v["state"].as_str().unwrap_or(""))
    });
    Ok(())
}
