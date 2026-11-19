//! `bf checkpoint close|sync|fix|wave-check|finding|review`.

use super::{out, project, Args};
use crate::refusal::{refuse, R};
use crate::store::s;
use crate::{enforce, run};
use serde_json::Value;

pub(crate) fn cmd_checkpoint(args: &Args) -> R<()> {
    let p = project(args)?;
    if args.pos.get(1).map(|x| x.as_str()) == Some("sync") {
        let k = args.pos_at(2, "a checkpoint ID")?;
        let v = run::checkpoint_sync(
            &p,
            k,
            args.flag("quote").unwrap_or(""),
            run::SyncMode {
                resolved: args.has("resolved"),
                abort: args.has("abort"),
                release: args.has("release"),
                local: args.has("local"),
            },
        )?;
        out(args, &v, || {
            if v["synced"] == true {
                format!("{k} took in the main branch")
            } else if v["sync_open"] == true {
                format!(
                    "the main branch conflicts in: {}; the merge is open in the checkpoint's working copy and `bf next` gives the directive that resolves it",
                    v["conflicted"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default()
                )
            } else if v["aborted"] == true {
                format!("{k} is as it was before the sync")
            } else if v["released"] == true {
                format!("the sync of {k} is back with its builder")
            } else {
                "nothing to take in".to_string()
            }
        });
        return Ok(());
    }
    if args.pos.get(1).map(|x| x.as_str()) == Some("fix") {
        let k = args.pos_at(2, "a checkpoint ID")?;
        let v = crate::ckfix::command(
            &p,
            k,
            args.flag("quote").unwrap_or(""),
            args.has("abort"),
            args.has("release"),
            args.has("open"),
        )?;
        out(args, &v, || {
            if v["opened"] == true {
                let named: Vec<&str> = v["failed_checks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str())
                    .collect();
                crate::strings::t(
                    "cli.ckfix.open.done",
                    &[("k", k), ("checks", &named.join(", "))],
                )
            } else if v["aborted"] == true {
                crate::strings::t("cli.ckfix.abort.done", &[("k", k)])
            } else {
                crate::strings::t("cli.ckfix.release.done", &[("k", k)])
            }
        });
        return Ok(());
    }
    if args.pos.get(1).map(|x| x.as_str()) == Some("wave-check") {
        let k = args.pos_at(2, "a checkpoint ID")?;
        let wave = crate::wavecheck::parse_wave(args.flag("wave"))?;
        let v = crate::wavecheck::run_step(&p, k, wave)?;
        out(args, &v, || {
            format!("wave {wave} of {k}: wave check {}", s(&v, "state"))
        });
        return Ok(());
    }
    if args.pos.get(1).map(|x| x.as_str()) == Some("finding") {
        let k = args.pos_at(2, "a checkpoint ID")?;
        let quote = args.flag("quote").unwrap_or("");
        let reason = args.flag("reason").unwrap_or("");
        let v = if let Some(ids) = args.flag("fix-findings") {
            crate::ckreview::fix_chosen(&p, k, ids, quote)?
        } else if let Some(ids) = args.flag("accept") {
            crate::ckreview::settle(&p, k, ids, "accepted", reason, quote)?
        } else if let Some(ids) = args.flag("decline") {
            crate::ckreview::settle(&p, k, ids, "declined", reason, quote)?
        } else {
            return Err(refuse(
                "run `bf checkpoint finding`",
                "it needs --accept, --decline or --fix-findings, with the findings named",
                &["Run `bf checkpoint finding <K> --accept low --quote \"<the human's words>\"`, `--decline r1.2 --reason \"<why>\" --quote \"<the human's words>\"` or `--fix-findings r1.2,r1.3 --quote \"<the human's words>\"`. The words are needed for a critical, high or medium finding."],
                "The command prints the result.",
            ));
        };
        out(args, &v, || {
            if v["fix_opened"] == true {
                let named: Vec<&str> = v["findings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str())
                    .collect();
                format!("{k}: a fix opened for {}", named.join(", "))
            } else {
                format!("{k}: findings settled")
            }
        });
        return Ok(());
    }
    if args.pos.get(1).map(|x| x.as_str()) == Some("review") {
        let k = args.pos_at(2, "a checkpoint ID")?;
        if !args.has("release") {
            return Err(refuse(
                "run `bf checkpoint review`",
                "it needs --release",
                &["Run `bf checkpoint review <K> --release --quote \"<the human's words>\"`."],
                "The command prints the result.",
            ));
        }
        let v = crate::ckreview::release(&p, k, args.flag("quote").unwrap_or(""))?;
        out(args, &v, || {
            format!("{k} is released; its findings go to a fix")
        });
        return Ok(());
    }
    if args.pos.get(1).map(|x| x.as_str()) != Some("close") {
        return Err(refuse(
            "run `bf checkpoint`",
            "only `bf checkpoint close`, `bf checkpoint sync`, `bf checkpoint fix`, `bf checkpoint finding` and `bf checkpoint review` exist",
            &["Run `bf checkpoint close <K>`, `bf checkpoint sync <K> --quote \"...\"` or `bf checkpoint fix <K> --abort`."],
            "The command prints the result.",
        ));
    }
    let k = args.pos_at(2, "a checkpoint ID")?;
    let v = run::checkpoint_close(&p, k, args.has("merge"))?;
    out(args, &v, || {
        let mut t = format!(
            "{k} closed{}",
            if v["merged"] == true {
                " and merged onto the main branch"
            } else {
                ""
            }
        );
        let un: Vec<Value> = v["unproven_off"].as_array().cloned().unwrap_or_default();
        if !un.is_empty() {
            t.push_str(&format!(
                "\nnot proven, their checks are off: {}",
                enforce::unproven_text(&un)
            ));
        }
        t
    });
    Ok(())
}
