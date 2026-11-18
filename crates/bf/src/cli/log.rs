//! `bf log`.

use super::{out, pretty, project_unchecked, Args};
use crate::events;
use crate::refusal::{from_key, R};
use crate::strings;
use serde_json::{json, Value};

pub(crate) fn cmd_log(args: &Args) -> R<()> {
    let p = project_unchecked(args)?;
    let log = events::load_log(&p)?;
    if args.has("verify") {
        let v =
            json!({"chain_ok": log.chain_ok, "events": log.events.len(), "error": log.chain_error});
        out(args, &v, || {
            if log.chain_ok {
                strings::t(
                    "cli.log.verify.ok",
                    &[("events", &log.events.len().to_string())],
                )
            } else {
                strings::t(
                    "cli.log.verify.broken",
                    &[("error", &log.chain_error.clone().unwrap_or_default())],
                )
            }
        });
        if !log.chain_ok {
            std::process::exit(2);
        }
        return Ok(());
    }
    // One event whole, with every field, over several lines (R-52).
    if args.has("whole") {
        let want = args
            .flag("whole")
            .map(|x| x.to_string())
            .or_else(|| args.pos.get(1).cloned())
            .unwrap_or_default();
        let found = want
            .parse::<u64>()
            .ok()
            .and_then(|n| log.events.iter().find(|e| e["seq"] == n));
        return match found {
            Some(e) => {
                println!("{}", pretty(e));
                Ok(())
            }
            None => Err(from_key("cli.log.whole_none", &[("number", &want)])),
        };
    }
    let n = args
        .flag("tail")
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(log.events.len());
    let skip = log.events.len().saturating_sub(n);
    let evs: Vec<&Value> = log.events.iter().skip(skip).collect();
    if args.has("json") {
        println!("{}", pretty(&json!(evs)));
    } else {
        let mut shortened = false;
        for e in evs {
            let (mut m, cut) = crate::amend::shorten_event(e);
            if let Some(o) = m.as_object_mut() {
                for k in ["prev", "h", "snapshot", "impact"] {
                    o.remove(k);
                }
            }
            let line = serde_json::to_string(&m)?;
            if cut {
                shortened = true;
                println!("{line} {}", strings::t("cli.log.contents_left_out", &[]));
            } else {
                println!("{line}");
            }
        }
        if shortened {
            println!("{}", strings::t("cli.log.whole_hint", &[]));
        }
    }
    Ok(())
}
