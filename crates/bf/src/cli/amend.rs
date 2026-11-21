//! `bf amend open|revise|narrow|approve|judge|seen|withdraw`.

use super::{out, pretty, project, read_data_in, Args};
use crate::gates;
use crate::refusal::{refuse_owned, R};
use crate::strings;

/// The checkpoint a repair is opened from (`--from-design <K>`, or `--from-design=<K>`), and the
/// positional IDs without it. The flag is not a value flag of the parser, so its value is read from the
/// command line itself when it came as a separate word.
fn from_design(args: &Args) -> (Option<String>, Vec<String>) {
    let ids: Vec<String> = args.pos[2..].to_vec();
    if let Some(v) = args.flag("from-design") {
        return (Some(v.to_string()), ids);
    }
    if !args.has("from-design") {
        return (None, ids);
    }
    let raw: Vec<String> = std::env::args().collect();
    let value = raw
        .iter()
        .position(|x| x == "--from-design")
        .and_then(|i| raw.get(i + 1))
        .filter(|x| !x.starts_with("--"))
        .cloned();
    match value {
        Some(v) => {
            let mut rest = ids;
            if let Some(i) = rest.iter().position(|x| *x == v) {
                rest.remove(i);
            }
            (Some(v), rest)
        }
        None => (Some(String::new()), ids),
    }
}

pub(crate) fn cmd_amend(args: &Args) -> R<()> {
    let p = project(args)?;
    let v = match args.pos.get(1).map(|x| x.as_str()) {
        Some("open") => {
            let (from, ids) = from_design(args);
            gates::amend_open_marked(&p, &ids, args.has("repair"), from.as_deref())?
        }
        Some("judge") => crate::judge::judge(
            &p,
            args.pos.get(2).map(|x| x.as_str()).unwrap_or(""),
            args.need("file", &strings::t("cli.judge.need_file", &[]))?,
            args.flag("token"),
            args.need("model", &strings::t("cli.judge.need_model", &[]))?,
        )?,
        Some("seen") => crate::judge::seen(
            &p,
            args.pos.get(2).map(|x| x.as_str()).unwrap_or(""),
            args.flag("quote").unwrap_or(""),
        )?,
        Some("checkpoint") => gates::amend_checkpoint(&p, &args.pos[2..], args.flag("title"))?,
        Some("revise") => gates::amend_revise(&p, &args.pos[2..])?,
        Some("narrow") => gates::amend_narrow(
            &p,
            &read_data_in(&p, args.need("file", "a narrowing file")?)?,
        )?,
        Some("withdraw") => gates::amend_withdraw(
            &p,
            args.pos.get(2).map(|x| x.as_str()),
            args.flag("reason").unwrap_or(""),
        )?,
        Some("approve") => gates::amend_approve(
            &p,
            args.pos.get(2).map(|x| x.as_str()),
            args.flag("quote").unwrap_or(""),
        )?,
        _ => {
            return Err(refuse_owned(
                strings::t("cli.amend.unknown.refused", &[]),
                strings::t("cli.amend.unknown.because", &[]),
                vec![strings::t("cli.amend.unknown.do_this", &[])],
                strings::t("cli.amend.unknown.worked_when", &[]),
            ))
        }
    };
    out(args, &v, || {
        if v["withdrawn"] == true {
            crate::amend::withdraw_text(&v)
        } else {
            pretty(&v)
        }
    });
    Ok(())
}
