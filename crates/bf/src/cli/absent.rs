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
    let v = absent::status_json(&st);
    out(args, &v, || absent::status_text(&st));
    Ok(())
}
