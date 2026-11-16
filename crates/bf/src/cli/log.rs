//! `bf log`.

use super::{out, pretty, project_unchecked, Args};
use crate::events;
use crate::refusal::R;
use serde_json::{json, Value};

pub(crate) fn cmd_log(args: &Args) -> R<()> {
    let p = project_unchecked(args)?;
    let log = events::load_log(&p)?;
    if args.has("verify") {
        let v =
            json!({"chain_ok": log.chain_ok, "events": log.events.len(), "error": log.chain_error});
        out(args, &v, || {
            if log.chain_ok {
                format!("chain ok ({} events)", log.events.len())
            } else {
                format!(
                    "chain BROKEN: {}",
                    log.chain_error.clone().unwrap_or_default()
                )
            }
        });
        if !log.chain_ok {
            std::process::exit(2);
        }
        return Ok(());
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
        for e in evs {
            let mut m = e.clone();
            if let Some(o) = m.as_object_mut() {
                for k in ["prev", "h", "snapshot", "impact"] {
                    o.remove(k);
                }
            }
            println!("{}", serde_json::to_string(&m)?);
        }
    }
    Ok(())
}
