//! `bf clean [--dry-run] [--json]`: remove what the engine no longer needs under `.bf/work` (D-109).

use super::{out, project, Args};
use crate::refusal::R;
use crate::run::cleanup::disk;

pub(crate) fn cmd_clean(args: &Args) -> R<()> {
    let p = project(args)?;
    let _lock = crate::lock::engine(&p, "clean")?;
    let done = disk::clean(&p, args.has("dry-run"))?;
    out(args, &done.json(), || done.text());
    Ok(())
}
