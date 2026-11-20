//! `bf status` and `bf progress`.

use super::{out, project, Args};
use crate::refusal::R;
use crate::run;
use serde_json::{json, Value};

pub(crate) fn cmd_status(args: &Args) -> R<()> {
    let p = project(args)?;
    let v = run::status(&p)?;
    out(args, &v, || status_text(&v));
    Ok(())
}

pub(crate) fn cmd_progress(args: &Args) -> R<()> {
    let p = project(args)?;
    let l = crate::statusline::line(&p)?;
    out(args, &json!({"line": l}), || l.clone());
    Ok(())
}

fn status_text(v: &Value) -> String {
    let mut t = format!(
        "project: {}\nrun: {}\ntest mode: {}\nlog: {} events, chain {}\n",
        v["project"].as_str().unwrap_or(""),
        v["run"].as_str().unwrap_or("none"),
        v["test_mode"].as_str().unwrap_or(""),
        v["events"],
        if v["chain_ok"] == true {
            "ok"
        } else {
            "BROKEN"
        }
    );
    if let Some(n) = v["main"]["note"].as_str().filter(|n| !n.is_empty()) {
        t.push_str(&format!("note: {n}\n"));
    }
    if let Some(n) = v["human_turns_note"].as_str().filter(|n| !n.is_empty()) {
        t.push_str(&format!("note: {n}\n"));
    }
    if let Some(o) = v["sync_open"].as_object() {
        t.push_str(&format!(
            "note: a sync of the main branch into {} is open ({}); conflicted: {}; `bf next` gives the directive, `bf checkpoint sync {} --abort` goes back\n",
            o["checkpoint"].as_str().unwrap_or(""),
            o["phase"].as_str().unwrap_or(""),
            o["files"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            o["checkpoint"].as_str().unwrap_or(""),
        ));
    }
    for m in v["main_ahead"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "note: the main branch of {} holds {} commit(s) that {} lacks; `bf checkpoint sync {} --quote \"<the human's words>\"` takes them in\n",
            m["repo"].as_str().unwrap_or(""),
            m["commits"],
            m["checkpoint"].as_str().unwrap_or(""),
            m["checkpoint"].as_str().unwrap_or(""),
        ));
    }
    for pg in v["pages"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "page {}: {}\n",
            pg["name"].as_str().unwrap_or(""),
            pg["url"].as_str().unwrap_or("")
        ));
    }
    for b in v["bad_records"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "record file {} cannot be read: {}\n",
            b["file"].as_str().unwrap_or(""),
            b["error"].as_str().unwrap_or("")
        ));
    }
    if v["workspace"] == true {
        for r in v["repos"].as_array().into_iter().flatten() {
            t.push_str(&format!(
                "repository {}: {} at {} ({}){}\n",
                r["name"].as_str().unwrap_or(""),
                r["branch"].as_str().unwrap_or(""),
                r["tip"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(10)
                    .collect::<String>(),
                r["path"].as_str().unwrap_or(""),
                if r["ok"] == false { " - MOVED" } else { "" }
            ));
        }
    }
    if v["main"]["ok"] == false {
        t.push_str(&format!(
            "main: MOVED - {}\n",
            v["main"]["detail"].as_str().unwrap_or("")
        ));
    }
    for u in v["units"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "  {} {}\n",
            u["id"].as_str().unwrap_or(""),
            u["state"].as_str().unwrap_or("")
        ));
        for (field, key) in [
            ("sync_conflicts", "status.unit.sync_conflicts"),
            ("sync_aborted", "status.unit.sync_aborted"),
        ] {
            let files: Vec<&str> = u[field]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str())
                .collect();
            if !files.is_empty() {
                t.push_str(&format!(
                    "    {}\n",
                    crate::strings::t(key, &[("files", &files.join(", "))])
                ));
            }
        }
    }
    for g in v["gates"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "  gate {} {}\n",
            g["id"].as_str().unwrap_or(""),
            g["status"].as_str().unwrap_or("")
        ));
    }
    for a in v["amendments"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "  amendment {} {}\n",
            a["id"].as_str().unwrap_or(""),
            a["status"].as_str().unwrap_or("")
        ));
    }
    t
}
