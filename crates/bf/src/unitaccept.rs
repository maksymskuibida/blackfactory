//! The human's accept of a unit's findings (D-91, H-24, R-71). The event and its replay live in
//! `events.rs` and `accept.rs`; this file checks the command, writes the event and marks the records.

use crate::events::{self, require_chain};
use crate::refusal::{refuse, R};
use crate::store::Project;
use crate::strings::t;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// Accept findings of a parked unit in the human's own words. Any of the open findings may be named,
/// some now and the rest later; at least one must be listed by the log. Each named record gets
/// `outcome = accepted`, and the unit is reviewed once none is left open.
pub fn accept(p: &Project, unit: &str, findings: &[String], quote: &str) -> R<Value> {
    let what = t("cli.unit_accept.what", &[("unit", unit)]);
    let st = require_chain(p)?;
    let Some(u) = st.units.get(unit) else {
        return Err(refuse(
            what,
            t("cli.unit_accept.no_unit", &[]),
            &[&t("cli.unit_accept.no_unit.step", &[])],
            t("cli.unit_accept.worked", &[("unit", unit)]),
        ));
    };
    if u.state != "parked" {
        return Err(refuse(
            what,
            t("cli.unit_accept.not_parked", &[("state", &u.state)]),
            &[&t("cli.unit_accept.not_parked.step", &[])],
            t("cli.unit_accept.worked", &[("unit", unit)]),
        ));
    }
    // The open findings: those the log lists, and those only a record of the unit holds.
    let mut open: BTreeMap<String, String> = BTreeMap::new();
    for id in &u.open_findings {
        open.insert(id.clone(), String::new());
    }
    let mut log_ids: Vec<String> = u.open_findings.clone();
    let recs = p.list_records(crate::schema::kind_by_name("finding"))?;
    for r in &recs {
        if s(r, "unit") == unit && crate::run::is_open(r) {
            let id = s(r, "id").to_string();
            open.insert(id, s(r, "severity").to_string());
        }
    }
    let open_list = open.keys().cloned().collect::<Vec<_>>().join(", ");
    if findings.is_empty() {
        return Err(refuse(
            what,
            t("cli.unit_accept.none_named", &[]),
            &[&t(
                "cli.unit_accept.none_named.step",
                &[("unit", unit), ("open", &open_list)],
            )],
            t("cli.unit_accept.worked", &[("unit", unit)]),
        ));
    }
    let unknown: Vec<&String> = findings.iter().filter(|f| !open.contains_key(*f)).collect();
    if !unknown.is_empty() {
        let ids = unknown
            .iter()
            .map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(refuse(
            what,
            t("cli.unit_accept.unknown", &[("ids", &ids), ("unit", unit)]),
            &[&t("cli.unit_accept.unknown.step", &[("open", &open_list)])],
            t("cli.unit_accept.worked", &[("unit", unit)]),
        ));
    }
    // The replay counts only findings the log lists; an accept that names none of them would be an event
    // it ignores, so it is refused before anything is written.
    log_ids.retain(|id| findings.contains(id));
    if log_ids.is_empty() {
        return Err(refuse(
            what,
            t("cli.unit_accept.records_only", &[("unit", unit)]),
            &[&t("cli.unit_accept.records_only.step", &[("unit", unit)])],
            t("cli.unit_accept.worked", &[("unit", unit)]),
        ));
    }
    // Words that are not the human's, or that come before the review, are refused here.
    let (turn, _) = events::pick_turn(&st, u.review_seq, events::Decide::Approve, quote)?;
    // A finding only a record holds is marked below, with the ones the log lists.
    let listed: Vec<Value> = log_ids
        .iter()
        .map(|id| json!({"id": id, "severity": open.get(id).cloned().unwrap_or_default()}))
        .collect();
    let only_records: Vec<&String> = findings.iter().filter(|f| !log_ids.contains(f)).collect();
    events::append(
        p,
        "unit.finding.accept",
        json!({"unit": unit, "findings": listed, "records_only": only_records,
               "turn": turn, "quote": quote.trim()}),
    )?;
    for id in findings {
        if let Some(mut rec) = p.get_record(id)? {
            rec["outcome"] = json!("accepted");
            p.write_record(&rec)?;
        }
    }
    p.write_guard()?;
    let st = events::replay(p)?;
    let state = st
        .units
        .get(unit)
        .map(|u| u.state.clone())
        .unwrap_or_default();
    let ids = findings.join(", ");
    Ok(
        json!({"unit": unit, "accepted": findings, "state": state, "quote": quote.trim(),
              "line": t("cli.unit_accept.line", &[("unit", unit), ("ids", &ids), ("state", &state)])}),
    )
}
