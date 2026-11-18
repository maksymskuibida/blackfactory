//! `bf next`.

use super::{out, project, Args};
use crate::refusal::R;
use crate::run;

pub(crate) fn cmd_next(args: &Args) -> R<()> {
    let p = project(args)?;
    let v = run::next(&p)?;
    out(args, &v, || {
        let mut t = match v["run"].as_str() {
            Some(r) if !r.is_empty() => format!("run {r}\n"),
            _ => String::new(),
        };
        for d in v["directives"].as_array().into_iter().flatten() {
            t.push_str(&format!(
                "  {} {} {} {} {}\n",
                d["role"].as_str().unwrap_or(""),
                d["action"].as_str().unwrap_or(""),
                d["unit"]
                    .as_str()
                    .or(d["checkpoint"].as_str())
                    .or(d["gate"].as_str())
                    .or(d["gate_kind"].as_str())
                    .unwrap_or(""),
                d["tier"].as_str().unwrap_or(""),
                d["effort"].as_str().unwrap_or("")
            ));
        }
        for w in v["waiting"].as_array().into_iter().flatten() {
            t.push_str(&format!("  waiting: {}\n", w.as_str().unwrap_or("")));
        }
        t
    });
    Ok(())
}
