//! `bf doctor [--perf]`.

use super::{out, project_unchecked, Args};
use crate::doctor;
use crate::refusal::{err, R};

pub(crate) fn cmd_doctor(args: &Args) -> R<()> {
    let p = project_unchecked(args)?;
    if args.has("perf") {
        let exe = std::env::current_exe().map_err(|e| err(format!("cannot find myself: {e}")))?;
        let v = doctor::perf(&p, &exe)?;
        out(args, &v, || {
            v["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    format!(
                        "{:<24} median {:>8.2} ms   p99 {:>8.2} ms",
                        r["command"].as_str().unwrap_or(""),
                        r["median_ms"].as_f64().unwrap_or(0.0),
                        r["p99_ms"].as_f64().unwrap_or(0.0)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
    } else {
        let v = doctor::doctor(&p)?;
        out(args, &v, || {
            v["checks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    format!(
                        "{}: {} ({})",
                        c["check"].as_str().unwrap_or(""),
                        if c["ok"] == true { "ok" } else { "PROBLEM" },
                        c["note"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
        if v["ok"] != true {
            std::process::exit(2);
        }
    }
    Ok(())
}
