//! `bf stuck`.

use super::{out, pretty, project, Args};
use crate::refusal::R;
use crate::run;

pub(crate) fn cmd_stuck(args: &Args) -> R<()> {
    let p = project(args)?;
    let v = run::stuck(&p, args.has("dry-run"))?;
    out(args, &v, || pretty(&v));
    Ok(())
}
