//! `bf view` and `bf render`.

use super::{out, pretty, project, Args};
use crate::refusal::R;
use crate::{util, view};
use serde_json::json;
use std::path::Path;

pub(crate) fn cmd_view(args: &Args) -> R<()> {
    let p = project(args)?;
    let kind = args.pos_at(1, "a page kind")?;
    let v = view::build(&p, kind, args.pos.get(2).map(|x| x.as_str()))?;
    println!("{}", pretty(&v));
    Ok(())
}

pub(crate) fn cmd_render(args: &Args) -> R<()> {
    let p = project(args)?;
    let kind = args.pos_at(1, "a page kind")?;
    let tpl = args.need("template", "an HTML template")?;
    let outp = args.need("out", "an output file")?;
    let v = view::build(&p, kind, args.pos.get(2).map(|x| x.as_str()))?;
    let html = view::render(&v, &p.safe_input(Path::new(tpl))?)?;
    let target = p.safe_output(Path::new(outp))?;
    util::atomic_write(&target, html.as_bytes())?;
    out(args, &json!({"out": outp, "bytes": html.len()}), || {
        format!("wrote {outp} ({} bytes)", html.len())
    });
    Ok(())
}
