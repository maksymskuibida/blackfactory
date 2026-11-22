//! `bf record put|get|list`.

use super::{kind_arg, out, pretty, project, read_data_in, real_zone, Args};
use crate::refusal::{err, refuse, R};
use crate::store::{s, str_list, Project};
use crate::{events, gates, plan, run, schema, util};
use serde_json::{json, Value};
use std::fs;

pub(crate) fn cmd_record(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`put`, `get` or `list`")? {
        "put" => cmd_record_put(args, &p)?,
        "remove" => cmd_record_remove(args, &p)?,
        "get" => {
            let id = args.pos_at(2, "a record ID")?;
            let rec = p.require_record(id)?;
            if args.has("json") {
                println!("{}", pretty(&rec));
            } else {
                print!("{}", toml::to_string_pretty(&rec)?);
            }
        }
        "list" => {
            let kind = match args.flag("kind") {
                Some(k) => Some(kind_arg(k)?),
                None => None,
            };
            let recs = p.list_records(kind)?;
            out(args, &json!(recs), || {
                recs.iter()
                    .map(|r| format!("{} {}", s(r, "id"), s(r, "title")))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        _ => {
            return Err(refuse(
                "run `bf record`",
                "unknown subcommand",
                &["Use `bf record put <kind> --file <f>`, `get <id>` or `list [--kind k]`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}

/// Take the lock that serialises ID allocation and record writes (two puts at once must not collide).
fn lock_records(p: &Project) -> R<fs::File> {
    fs::create_dir_all(p.dot_bf())?;
    let f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join("records.lock"))?;
    f.lock()
        .map_err(|e| err(format!("cannot lock the records: {e}")))?;
    Ok(f)
}

/// The records on disk that still name the unit `id`: in `depends_on`, `after`, `unit`, `from` or `to`.
fn records_naming(p: &Project, id: &str) -> R<Vec<String>> {
    let mut named: Vec<String> = vec![];
    for r in p.list_records(None)? {
        let rid = s(&r, "id");
        if rid == id {
            continue;
        }
        let hit = str_list(&r, "depends_on").iter().any(|x| x == id)
            || str_list(&r, "after").iter().any(|x| x == id)
            || ["unit", "from", "to"].iter().any(|k| s(&r, k) == id);
        if hit {
            named.push(rid.to_string());
        }
    }
    named.sort();
    Ok(named)
}

/// `bf record remove <U-n>`: the one way a unit record leaves the plan (R-73, finding F-226). Only a unit,
/// only while an open plan repair names it (the rule that lets a put change a named record), never one
/// that has started, and never while a record on disk still names it. The record's file goes through the
/// store, as a put writes one; the narrowing lists the unit as removed, and a withdrawal puts it back.
fn cmd_record_remove(args: &Args, p: &Project) -> R<()> {
    let id = args.pos_at(2, "a unit ID")?.to_string();
    let _lock = lock_records(p)?;
    let st = events::require_chain(p)?;
    let refused = |stem: &str, vals: &[(&str, &str)]| -> crate::refusal::Fail {
        crate::refusal::from_key(stem, vals)
    };
    let is_unit = schema::kind_of_id(&id).is_some_and(|k| k.name == "unit");
    if !is_unit {
        return Err(crate::refusal::from_key(
            "cli.record.remove.not_unit",
            &[("id", &id)],
        ));
    }
    if p.get_record(&id)?.is_none() {
        return Err(crate::refusal::from_key(
            "cli.record.remove.no_record",
            &[("id", &id)],
        ));
    }
    let Some(amend) = gates::waiting_repair_naming(&st, &id) else {
        return Err(crate::refusal::from_key(
            "cli.record.remove.no_repair",
            &[("id", &id)],
        ));
    };
    let state = st.unit_state(&id);
    if !state.is_empty() {
        return Err(refused(
            "cli.record.remove.started",
            &[("id", &id), ("state", state)],
        ));
    }
    let named = records_naming(p, &id)?;
    if !named.is_empty() {
        return Err(crate::refusal::from_key(
            "cli.record.remove.named",
            &[("id", &id), ("records", &named.join(", "))],
        ));
    }
    let amend_id = amend.id.clone();
    fs::remove_file(p.record_path(&id)?)?;
    p.write_guard()?;
    out(args, &json!({"removed": id, "amend": amend_id}), || {
        crate::strings::t(
            "cli.record.remove.done",
            &[("id", &id), ("amend", &amend_id)],
        )
    });
    Ok(())
}

fn cmd_record_put(args: &Args, p: &Project) -> R<()> {
    let kind = kind_arg(args.pos_at(2, "a record kind")?)?;
    let mut rec = read_data_in(p, args.need("file", "a record file (TOML or JSON)")?)?;
    let _lock = lock_records(p)?;
    // A worker's only record is a new, open finding for a unit that is in review. Marking a finding
    // fixed or accepted is not a worker's to do.
    if let Some((_, zone_id)) = real_zone(p) {
        if matches!(kind.name, "threat" | "legal") {
            // A threat or a legal record is put only with the token of a role that writes that kind.
            let held = crate::roles::writers_of(kind.name)
                .iter()
                .any(|r| run::holds_token(p, &format!("{zone_id}:{r}"), args.flag("token")));
            if !held {
                return Err(refuse(
                    format!("put a {} from a working copy", kind.name),
                    format!(
                        "a {} is put only with the token of a role that writes it ({}); none, or another role's, was given",
                        kind.name,
                        crate::roles::writers_of(kind.name).join(" or ")
                    ),
                    &["Put it with `--token <token>`; the conductor issues the token with `bf roles token <unit> <role>`."],
                    format!("`bf record get` shows the {}.", kind.name),
                ));
            }
        } else {
            let st = events::replay(p)?;
            let unit = s(&rec, "unit").to_string();
            let outcome = s(&rec, "outcome").to_string();
            // An auditor files a finding for its unit in any state once the unit has started; on a
            // checkpoint, for a started unit that the checkpoint comes after. The token says who is
            // speaking; a checkpoint's copy has no other way to file a finding.
            let in_checkpoint = zone_id.starts_with("K-");
            let holds_auditor = crate::roles::finding_auditors()
                .iter()
                .any(|r| run::holds_token(p, &format!("{zone_id}:{r}"), args.flag("token")));
            let its_unit = if in_checkpoint {
                plan::load(p)?
                    .checkpoint(&zone_id)
                    .map(|k| str_list(k, "after").contains(&unit))
                    .unwrap_or(false)
            } else {
                unit == zone_id
            };
            let open = outcome.is_empty() || outcome == "open";
            let auditor = kind.name == "finding"
                && holds_auditor
                && open
                && !st.unit_state(&unit).is_empty()
                && its_unit;
            let worker = !in_checkpoint
                && kind.name == "finding"
                && st.unit_state(&unit) == "review"
                && open;
            if !auditor && !worker {
                return Err(refuse(
                format!("put a {} from a working copy", kind.name),
                "a worker may file a new, open finding for a unit that is in review, and an auditor with its token for a started unit of its own copy (a checkpoint's auditor: a unit the checkpoint comes after); every other record belongs to the planner and the human, and fixed and accepted are set by the engine and the human",
                &["Report with `bf report <unit> --result findings --role reviewer` while the unit is in review.", "An auditor files with `--token <token>`, for a started unit of its copy.", "To accept a finding, ask the human; they put it from the main checkout."],
                "`bf record get` shows the finding as open.",
            ));
            }
            // A finding put from a working copy is always new: it never replaces one that exists, whatever
            // its outcome.
            let taken = match rec.get("id").and_then(|v| v.as_str()) {
                Some(id) => p.get_record(id)?.is_some(),
                None => false,
            };
            if taken {
                return Err(refuse(
                    format!("put the finding {} from a working copy", s(&rec, "id")),
                    "a record with that ID exists; a working copy files new findings only, and never changes one that is filed",
                    &["Put the finding without an `id`: the engine gives it the next free one."],
                    "`bf record get` shows the filed finding as it was.",
                ));
            }
        }
    }
    if rec.get("kind").is_none() {
        rec["kind"] = json!(kind.name);
    }
    let mut assigned = false;
    if rec.get("id").is_none() {
        rec["id"] = json!(p.next_id(kind)?);
        assigned = true;
    }
    schema::validate_or_refuse(kind, &rec)?;
    let mut id = s(&rec, "id").to_string();
    let st = events::require_chain(p)?;
    let old = p.get_record(&id)?;
    // A finding of a gate or an amendment is put only as the review rules allow: never accepted by a
    // put, and only with the token of its review kind.
    if kind.name == "finding" {
        crate::checks::review::may_put_finding(p, &st, &rec, old.as_ref(), args.flag("token"))?;
        // The finding on file is judged too: a finding of a design review holds its gate or amendment,
        // and what holds it (the gate, the amendment, a request for a stage review, the severity) is
        // changed or removed only with the token of the kind that filed it.
        if let Some(o) = old.as_ref() {
            let (og, oa) = (s(o, "gate"), s(o, "amendment"));
            let changed = ["gate", "amendment", "asks", "severity"]
                .iter()
                .any(|k| s(o, k) != s(&rec, k));
            let reviewed = if oa.is_empty() {
                crate::checks::review::GATES.contains(&og)
            } else {
                true
            };
            if changed && reviewed && p.policy.roles.tokens {
                let (subject, kind) = if oa.is_empty() {
                    (og.to_string(), og.to_string())
                } else {
                    (
                        oa.to_string(),
                        if og.is_empty() {
                            "amendment".to_string()
                        } else {
                            og.to_string()
                        },
                    )
                };
                if !crate::run::holds_token(
                    p,
                    &crate::checks::review::token_key(&subject, &kind),
                    args.flag("token"),
                ) {
                    return Err(refuse(
                        format!("change the finding {}", s(&rec, "id")),
                        format!("it is a finding of the review of {subject}, and its gate, amendment, request for a stage review or severity changes only with the token of its kind ({kind}); none, or the wrong one, was given"),
                        &["The design reviewer passes the token `bf review token` issued for its kind."],
                        "`bf record get` shows the finding as it was filed.",
                    ));
                }
            }
        }
    }
    let mut accepted_by: Option<(u64, String)> = None;
    // A legal record is accepted by the human alone: not by a role that writes it, and from the main
    // checkout only with the words of a human message.
    if kind.name == "legal"
        && s(&rec, "outcome") == "accepted"
        && old.as_ref().map(|o| s(o, "outcome")) != Some("accepted")
    {
        if real_zone(p).is_some() {
            return Err(refuse(
                format!("accept the legal record {id} from a working copy"),
                "accepting a legal finding is the human's alone; no role may set the outcome accepted",
                &["Leave the outcome open or fixed and report it; the human accepts it."],
                format!("`bf record get {id}` shows the outcome."),
            ));
        }
        // The human accepts a record that exists: a new record is never born accepted, and the step
        // that accepts changes nothing but the outcome.
        let Some(stored) = old.as_ref() else {
            return Err(refuse(
                format!("create the legal record {id} as accepted"),
                "a record that did not exist cannot have been seen and accepted by the human",
                &["Put the record with the outcome open, show it to the human, and let them accept it."],
                format!("`bf record get {id}` shows the outcome open."),
            ));
        };
        let mut same = rec.clone();
        same["outcome"] = stored.get("outcome").cloned().unwrap_or(Value::Null);
        if &same != stored {
            return Err(refuse(
                format!("accept the legal record {id} and change it in the same step"),
                "the human accepts the record as it is stored; this file also changes its content",
                &[
                    "Put the changed record first, with the outcome as it is now.",
                    "Show it to the human; after their message, put the same content with the outcome accepted.",
                ],
                format!("`bf record get {id}` shows the content the human accepted."),
            ));
        }
        // Only a message written after the record's latest change counts: the human accepts what they
        // saw, not what it was before it was changed.
        let changed_at = old
            .as_ref()
            .and_then(|_| p.record_path(&id).ok())
            .and_then(|f| fs::metadata(f).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // A turn is stamped to the whole second; one at the same instant as the change, or in the
        // same second before it, counts as before the change.
        let after = st
            .turns
            .iter()
            .filter(|t| util::parse_rfc3339(&t.at) * 1000 <= changed_at)
            .map(|t| t.seq)
            .max()
            .unwrap_or(0);
        accepted_by = Some(events::pick_turn(
            &st,
            after,
            events::Decide::Approve,
            args.flag("quote").unwrap_or(""),
        )?);
    }
    // Once a stage's gate is approved, its records change, and new ones appear, only by amendment.
    if let Some(stage) = gates::stage_of_kind(kind.name) {
        let changes = old.as_ref().map(|o| o != &rec).unwrap_or(true);
        if st.gate_passed(stage, None)
            && changes
            && !gates::amended(&st, &id)
            && !gates::is_approved_content(&st, &rec)
        {
            if old.is_none() {
                return Err(crate::refusal::from_key(
                    "cli.amend.new_record_late",
                    &[("id", &id), ("stage", stage)],
                ));
            }
            return Err(crate::refusal::from_key(
                "cli.amend.replace_gated",
                &[("id", &id), ("stage", stage)],
            ));
        }
    }
    if let Some(old) = &old {
        if old != &rec && !args.has("replace") && kind.name != "finding" {
            return Err(refuse(
                format!("put {id}"),
                format!("{id} is already a record (\"{}\") and this file is different; putting it would overwrite that record", s(old, "title")),
                &[
                    &format!("If you meant a new record, put a fresh ID in the file: `bf id new {}` prints the next free one.", kind.name),
                    &format!("If you meant to revise {id}, run the same command again with `--replace`."),
                ],
                format!("`bf record get {id}` shows the record you intended."),
            ));
        }
    }
    // A record of a design stage, or of an amendment that is not a plan repair, is accepted only with
    // the token of the designer the engine issued for it (R-67), unless the human lifted the rule.
    let made = gates::designer_accept(
        p,
        &st,
        kind.name,
        &id,
        args.flag("token"),
        args.flag("model"),
    )?;
    // An accepted legal record that changes is no longer what the human accepted: it goes back to open.
    let mut reset_open = false;
    if kind.name == "legal" && accepted_by.is_none() && s(&rec, "outcome") == "accepted" {
        if let Some(old) = &old {
            if s(old, "outcome") == "accepted" && old != &rec {
                rec["outcome"] = json!("open");
                reset_open = true;
            }
        }
    }
    if assigned {
        // A number the engine chose is taken with `create_new` and a retry, so records put at the
        // same moment each keep their own file.
        id = p.write_numbered(kind, &mut rec)?;
    } else {
        p.write_record(&rec)?;
    }
    p.write_guard()?;
    if let Some(m) = &made {
        gates::design_made(p, &id, kind.name, m)?;
    }
    if let Some((turn, message)) = accepted_by {
        // The acceptance leaves a trace: the record, the human's message and the words taken from it.
        // The message is used up.
        events::append(
            p,
            "legal.accept",
            json!({"record": id, "turn": turn, "message": message,
                "quote": args.flag("quote").unwrap_or("").trim()}),
        )?;
    }
    out(
        args,
        &json!({"put": id, "assigned": assigned, "reset_to_open": reset_open}),
        || {
            let mut t = if assigned {
                format!("put {id} (the file had no id, so the engine assigned {id}; use this id when you refer to the record)")
            } else {
                format!("put {id}")
            };
            if reset_open {
                t.push_str(&format!(
                    "\n{id} was accepted and its content changed, so its outcome is back to open; the human must accept it again"
                ));
            }
            t
        },
    );
    Ok(())
}
