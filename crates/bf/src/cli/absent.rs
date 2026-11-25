//! `bf absent status`: the run's human-absent flag. No subcommand sets or releases it (R-72).

use super::{out, project, Args};
use crate::absent;
use crate::events;
use crate::refusal::{refuse, R};
use crate::strings::t;

pub(crate) fn cmd_absent(args: &Args) -> R<()> {
    if args.pos.get(1).map(|x| x.as_str()) != Some("status") {
        return Err(refuse(
            t("absent.refuse.what", &[]),
            t("absent.refuse.why", &[]),
            &[&t("absent.refuse.step", &[])],
            t("absent.refuse.worked", &[]),
        ));
    }
    let p = project(args)?;
    let st = events::replay(&p)?;
    let mut v = absent::status_json(&st);
    let never: Vec<&str> = crate::hooks::unreached(&p)
        .iter()
        .map(|e| e.event)
        .collect();
    let journal = crate::hooks::journal_tail(&p, 5);
    v["hooks_never_reached"] = serde_json::json!(never);
    v["waits"] = serde_json::json!(journal);
    out(args, &v, || {
        let mut lines = vec![absent::status_text(&st)];
        if !never.is_empty() {
            lines.push(t(
                "absent.status.unreached",
                &[("events", &crate::hooks::unreached_list(&p))],
            ));
        }
        for l in &journal {
            lines.push(t("absent.status.waits", &[("line", l)]));
        }
        lines.join("\n")
    });
    Ok(())
}
