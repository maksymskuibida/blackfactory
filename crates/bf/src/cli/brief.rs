//! `bf brief <unit> --role <role>`.

use super::{out, project, Args};
use crate::refusal::R;
use crate::run;
use serde_json::json;

pub(crate) fn cmd_brief(args: &Args) -> R<()> {
    let p = project(args)?;
    let unit = args.pos_at(1, "a unit ID")?;
    let role = args.need("role", "a role: builder, reviewer or verifier")?;
    let path = run::brief_checked(&p, unit, role, args.flag("check"))?;
    out(args, &json!({"path": path.to_string_lossy()}), || {
        path.to_string_lossy().to_string()
    });
    Ok(())
}
