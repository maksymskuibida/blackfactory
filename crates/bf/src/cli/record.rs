//! `bf record put|get|list`.

use super::{kind_arg, out, pretty, project, read_data_in, real_zone, Args};
use crate::refusal::{err, refuse, R};
use crate::store::{s, Project};
use crate::{events, gates, run, schema, util};
use serde_json::{json, Value};
use std::fs;

pub(crate) fn cmd_record(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`put`, `get` or `list`")? {
        "put" => cmd_record_put(args, &p)?,
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
            if kind.name != "finding"
                || st.unit_state(&unit) != "review"
                || !(outcome.is_empty() || outcome == "open")
            {
                return Err(refuse(
                format!("put a {} from a working copy", kind.name),
                "a worker may file a new, open finding for a unit that is in review; every other record belongs to the planner and the human, and fixed and accepted are set by the engine and the human",
                &["Report with `bf report <unit> --result findings --role reviewer` while the unit is in review.", "To accept a finding, ask the human; they put it from the main checkout."],
                "`bf record get` shows the finding as open.",
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
        if st.gate_passed(stage, None) && changes && !gates::amended(&st, &id) {
            return Err(refuse(
                format!("{} {id}", if old.is_some() { "replace" } else { "add" }),
                format!("the {stage} gate is approved; an approved stage changes only by amendment, and that includes new records"),
                &[
                    &format!("Run `bf amend open {id}` to see the impact."),
                    "Narrow it with `bf amend narrow --file <f>`; the human approves it.",
                    "Put the record again.",
                ],
                format!("`bf record get {id}` shows the new content."),
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
