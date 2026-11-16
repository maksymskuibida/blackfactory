//! `bf test-mode grant`.

use super::{out, project, Args};
use crate::gates;
use crate::refusal::{refuse, R};

pub(crate) fn cmd_test_mode(args: &Args) -> R<()> {
    let p = project(args)?;
    if args.pos.get(1).map(|x| x.as_str()) != Some("grant") {
        return Err(refuse(
            "run `bf test-mode`",
            "only `grant` exists",
            &["Run `bf test-mode grant`."],
            "The command prints the grant.",
        ));
    }
    let v = gates::grant(&p, args.flag("quote").unwrap_or(""))?;
    out(args, &v, || {
        format!(
            "test mode granted for {}",
            v["applies_to"].as_str().unwrap_or("")
        )
    });
    Ok(())
}
