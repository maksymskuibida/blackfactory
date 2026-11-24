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
        if let Some(a) = v["human_absent"].as_object() {
            t.push_str(&format!(
                "{}\n",
                crate::strings::t(
                    "absent_briefs.next.line",
                    &[
                        ("since", a["since"].as_str().unwrap_or("")),
                        ("session", a["session"].as_str().unwrap_or("")),
                        (
                            "instruction",
                            &crate::absent::instruction().replace('\n', " ")
                        ),
                    ]
                )
            ));
        }
        if let Some(a) = v["human_absent"].as_object() {
            let list = a["returning"]
                .as_array()
                .map(|x| {
                    x.iter()
                        .filter_map(|y| y.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            if !list.is_empty() {
                t.push_str(&format!(
                    "{}\n",
                    crate::strings::t("absent_briefs.next.returning", &[("sessions", &list)])
                ));
            }
            if let Some(step) = a["waits_at"].as_str().filter(|x| !x.is_empty()) {
                t.push_str(&format!(
                    "{}\n",
                    crate::strings::t("absent_briefs.next.waits_at", &[("step", step)])
                ));
            }
        }
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
            if d["waits"] == true {
                t.pop();
                t.push_str(&format!(
                    " {}\n",
                    crate::strings::t("absent_briefs.next.waits", &[])
                ));
            }
        }
        for w in v["waiting"].as_array().into_iter().flatten() {
            t.push_str(&format!("  waiting: {}\n", w.as_str().unwrap_or("")));
        }
        t
    });
    Ok(())
}
