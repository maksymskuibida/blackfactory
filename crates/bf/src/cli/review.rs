//! `bf review token|split|brief|finding|release`: the tokens and briefs of the design reviewer and of the
//! stage agents of an amendment, the general designer's split, and the human's answers to a review
//! (accepting findings, releasing a parked review).

use super::{out, pretty, project, real_zone, Args};
use crate::checks::review as rules;
use crate::events;
use crate::refusal::{refuse, R};
use crate::roles::design;
use crate::store::{s, Project};
use serde_json::json;

/// A gate that has a design review, or an amendment.
fn check_subject(what: &str, subject: &str) -> R<()> {
    if rules::GATES.contains(&subject) || (subject.starts_with("A-") && subject.len() > 2) {
        return Ok(());
    }
    Err(refuse(
        what,
        format!("`{subject}` is neither a gate with a design review nor an amendment"),
        &["Name idea, requirements, architecture or plan, or an amendment such as A-3."],
        "`bf status` shows the reviews.",
    ))
}

/// The human's answers are given from the main checkout, never from a working copy.
fn refuse_in_copy(p: &Project, what: &str) -> R<()> {
    if real_zone(p).is_some() {
        return Err(refuse(
            what,
            "the human's answer to a review is recorded from the main checkout, not from a working copy",
            &["Ask the conductor to run it from the main checkout."],
            "`bf status` shows the review.",
        ));
    }
    Ok(())
}

/// `bf review finding <subject> --accept <F-ids> --quote <words>`.
fn finding(p: &Project, args: &Args) -> R<serde_json::Value> {
    let subject = args.pos_at(2, "a gate or an amendment ID")?;
    let what = format!("accept findings of the review of {subject}");
    check_subject(&what, subject)?;
    refuse_in_copy(p, &what)?;
    let ids: Vec<String> = args
        .need("accept", "the findings to accept, as F-1,F-2")?
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|x| !x.is_empty())
        .map(|x| x.to_string())
        .collect();
    let quote = args.flag("quote").unwrap_or("").trim().to_string();
    // Each finding belongs to the subject named: the answer is for one review.
    for id in &ids {
        if let Some(rec) = p.get_record(id)?.filter(|r| r["kind"] == "finding") {
            let (gate, amend) = (s(&rec, "gate"), s(&rec, "amendment"));
            let mine = if subject.starts_with("A-") {
                amend == subject
            } else {
                amend.is_empty() && gate == subject
            };
            if !mine {
                return Err(refuse(
                    what,
                    format!("{id} is not a finding of {subject}"),
                    &["Name the subject the finding belongs to."],
                    "The command prints the accepted findings.",
                ));
            }
        }
    }
    let turn = rules::check_accept(p, &ids, &quote)?;
    events::append(
        p,
        rules::ACCEPT_EVENT,
        json!({"subject": subject, "ids": ids, "turn": turn, "quote": quote}),
    )?;
    for id in &ids {
        let mut rec = p.require_record(id)?;
        rec["outcome"] = json!("accepted");
        p.write_record(&rec)?;
    }
    p.write_guard()?;
    Ok(json!({"subject": subject, "accepted": ids, "quote": quote}))
}

/// `bf review release <subject> --quote <words>`.
fn release(p: &Project, args: &Args) -> R<serde_json::Value> {
    let subject = args.pos_at(2, "a gate or an amendment ID")?;
    let what = format!("release the review of {subject}");
    check_subject(&what, subject)?;
    refuse_in_copy(p, &what)?;
    let quote = args.flag("quote").unwrap_or("").trim().to_string();
    let turn = rules::check_release(p, subject, &quote)?;
    events::append(
        p,
        rules::RELEASE_EVENT,
        json!({"subject": subject, "turn": turn, "quote": quote}),
    )?;
    Ok(json!({"subject": subject, "released": true, "quote": quote}))
}

pub(crate) fn cmd_review(args: &Args) -> R<()> {
    let p = project(args)?;
    let subject = args.flag("subject");
    let v = match args.pos.get(1).map(|x| x.as_str()) {
        Some("token") => {
            if let Some(stage) = args.flag("stage") {
                let amend = args.need("subject", "the amendment the stage belongs to")?;
                let role = args.need("role", "the role of the stage agent")?;
                design::issue_stage_token(&p, amend, stage, role)?
            } else {
                design::issue_kind_token(&p, args.pos_at(2, "a review kind")?, subject)?
            }
        }
        Some("split") => {
            let amend = args.pos_at(2, "an amendment ID")?;
            let stages = args.multi.get("stage").cloned().unwrap_or_default();
            design::split(&p, amend, &stages, args.flag("token"))?
        }
        Some("finding") => finding(&p, args)?,
        Some("release") => release(&p, args)?,
        Some("brief") => {
            let what = args.pos_at(2, "a review kind or a stage")?;
            let path = if args.flag("role").is_some() || design::stage_only(what) {
                let amend = args.need("subject", "the amendment the stage belongs to")?;
                design::brief_stage(&p, what, amend, args.flag("role"))?
            } else {
                design::brief_kind(&p, what, subject)?
            };
            json!({"path": path.to_string_lossy()})
        }
        _ => {
            return Err(refuse(
                "run `bf review`",
                "unknown subcommand",
                &["Use `bf review token <kind> [--subject A-n]`, `bf review token --subject A-n --stage <stage> --role <role>`, `bf review split <A-n> --stage <stage>=<ID,ID> --token <token>`, `bf review brief <kind or stage> [--subject A-n]`, `bf review finding <gate or A-n> --accept <F-ids> --quote <words>` or `bf review release <gate or A-n> --quote <words>`."],
                "The command prints its result.",
            ))
        }
    };
    out(args, &v, || match v["path"].as_str() {
        Some(path) => path.to_string(),
        None => pretty(&v),
    });
    Ok(())
}
