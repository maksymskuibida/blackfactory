//! `bf checkpoint close|sync|fix|wave-check|finding|review`.

use super::{out, project, Args};
use crate::refusal::{refuse, refuse_owned, R};
use crate::store::s;
use crate::{enforce, run};
use serde_json::Value;

pub(crate) fn cmd_checkpoint(args: &Args) -> R<()> {
    let p = project(args)?;
    // A landing turn holds the checkpoint's working copy and the content its suite runs on: one test of
    // its lock keeps a sync, a fix and a close out until it ends.
    // The command takes the turn lock itself and keeps it to its end, so a turn cannot start between
    // the test and the work.
    let mut _turn = None;
    if let (Some(sub @ ("sync" | "fix" | "close")), Some(k)) = (
        args.pos.get(1).map(|x| x.as_str()),
        args.pos.get(2).map(|x| x.as_str()),
    ) {
        match crate::lock::turn(&p, k)? {
            Some(g) => _turn = Some(g),
            None => {
                return Err(crate::refusal::from_key(
                    "land_merge.turn.refused",
                    &[("sub", sub), ("k", k)],
                ));
            }
        }
        run::land::hold_point("checkpoint-command", k);
    }
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
                        .map(|a| a
                            .iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", "))
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
                match v["saved"].as_str().filter(|x| !x.is_empty()) {
                    Some(saved) => crate::strings::t(
                        "fix.scope.saved.abort_done",
                        &[("k", k), ("saved", saved)],
                    ),
                    None => crate::strings::t("cli.ckfix.abort.done", &[("k", k)]),
                }
            } else {
                crate::strings::t("cli.ckfix.release.done", &[("k", k)])
            }
        });
        return Ok(());
    }
    if args.pos.get(1).map(|x| x.as_str()) == Some("scope") {
        let k = args.pos_at(2, "a checkpoint ID")?;
        let list = match args.flag("verdict") {
            Some("-") => serde_json::from_str::<Value>(&super::read_stdin()).map_err(|e| {
                refuse_owned(
                    crate::strings::t("cli.ckscope.stdin.what", &[]),
                    crate::strings::t("cli.ckscope.stdin.because", &[("e", &e.to_string())]),
                    vec![crate::strings::t("cli.ckscope.stdin.step", &[])],
                    crate::strings::t("cli.ckscope.done_when", &[]),
                )
            })?,
            Some(f) => super::read_data_in(&p, f)?,
            None => {
                return Err(refuse_owned(
                    crate::strings::t("cli.ckscope.need.what", &[]),
                    crate::strings::t("cli.ckscope.need.because", &[]),
                    vec![crate::strings::t("cli.ckscope.need.step", &[])],
                    crate::strings::t("cli.ckscope.done_when", &[]),
                ))
            }
        };
        let v = crate::fixscope::verdict(&p, k, &list, args.flag("token"))?;
        out(args, &v, || {
            let files: Vec<String> = v["granted"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(
                    v["refused"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| &r["path"]),
                )
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            if v["granted"].as_array().is_none_or(|a| a.is_empty())
                && (v["refusal"] == "extension" || v["refusal"] == "retry_spent")
            {
                // Each file by its own verdict: an extension or a spent retry is not granted and goes by a
                // gap report; any other retry file may be asked for again.
                let spent = v["refusal"] == "retry_spent";
                let (stuck, again): (Vec<&serde_json::Value>, Vec<&serde_json::Value>) = v
                    ["refused"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .partition(|r| {
                        r["verdict"] == "extension" || (spent && r["verdict"] == "retry_spent")
                    });
                let names = |l: &[&serde_json::Value]| {
                    l.iter()
                        .filter_map(|r| r["path"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let mut lines = vec![crate::strings::t(
                    "fix.retry.verdict.not_granted",
                    &[("k", k), ("files", &names(&stuck))],
                )];
                if !again.is_empty() {
                    lines.push(crate::strings::t(
                        "fix.scope.verdict.retry",
                        &[("k", k), ("files", &names(&again))],
                    ));
                }
                return lines.join("\n");
            }
            let key = if !v["granted"].as_array().is_none_or(|a| a.is_empty()) {
                "fix.scope.verdict.granted"
            } else if v["phase"] == "gap" {
                "fix.scope.verdict.human"
            } else {
                "fix.scope.verdict.retry"
            };
            crate::strings::t(key, &[("k", k), ("files", &files.join(", "))])
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
                &[
                    "Run `bf checkpoint finding <K> --accept low --quote \"<the human's words>\"`, `--decline r1.2 --reason \"<why>\" --quote \"<the human's words>\"` or `--fix-findings r1.2,r1.3 --quote \"<the human's words>\"`. The words are needed for a critical, high or medium finding.",
                ],
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
    if args.pos.get(1).map(|x| x.as_str()) == Some("check") {
        let k = args.pos_at(2, "a checkpoint ID")?;
        let ids = args.flag("accept").unwrap_or("");
        let v = run::checkpoint_check_accept(
            &p,
            k,
            ids,
            args.flag("quote").unwrap_or(""),
            super::real_zone(&p).is_some(),
        )?;
        out(args, &v, || {
            crate::strings::t("check_accept.done", &[("k", k), ("checks", &ids.replace(',', ", "))])
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
        return Err(refuse_owned(
            crate::strings::t("check_accept.usage.what", &[]),
            crate::strings::t("check_accept.usage.because", &[]),
            vec![crate::strings::t("check_accept.usage.step", &[])],
            crate::strings::t("check_accept.usage.worked", &[]),
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
        for a in v["accepted_checks"].as_array().into_iter().flatten() {
            t.push('\n');
            t.push_str(&crate::strings::t(
                "check_accept.close.line",
                &[("check", s(a, "check")), ("mark", s(a, "mark")), ("quote", s(a, "quote"))],
            ));
        }
        t
    });
    Ok(())
}
