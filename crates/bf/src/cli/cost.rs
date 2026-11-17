//! `bf cost`.

use super::{out, project, Args};
use crate::refusal::R;
use crate::run;

pub(crate) fn cmd_cost(args: &Args) -> R<()> {
    let p = project(args)?;
    let v = run::cost_status(&p)?;
    out(args, &v, || {
        format!(
            "spent {:.2} of {} {}",
            v["spent"].as_f64().unwrap_or(0.0),
            v["cap"],
            v["unit"].as_str().unwrap_or("")
        )
    });
    Ok(())
}
