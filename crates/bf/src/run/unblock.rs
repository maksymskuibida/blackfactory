//! U-81 (R-74, D-93, D-102): the conductor puts a unit blocked by its builder's stated reason back to
//! work, at most twice, once it judges the reason fixed. `bf unit unblock <U> --why "..." [--wait|--human]`.
//!
//! The guard refuses the command to every payload that names an agent type. A stated limit (R-74
//! acceptance 21): the guard trusts the harness's agent type as R-69 does, so a second top-level
//! session of the workspace that the harness does not mark as a subagent cannot be told from the
//! conductor.

use super::*;
use crate::scope::NextEntry;
use crate::strings::t;

/// How many times the conductor may release one unit, counted since the human's own latest act on it.
pub const LIMIT: u32 = 2;

/// A blocked unit that is the conductor's to judge.
#[derive(Clone, Debug)]
pub struct Block {
    /// The log position of the builder's blocked report.
    pub report_seq: u64,
    /// The builder's reason: the note of that report.
    pub reason: String,
    /// The releases left.
    pub left: u32,
}

fn seq_of(e: &Value) -> u64 {
    e["seq"].as_u64().unwrap_or(0)
}

/// The latest event that blocked or parked the unit.
fn latest_block<'a>(events: &'a [Value], unit: &str) -> Option<&'a Value> {
    events.iter().rev().find(|e| {
        s(e, "unit") == unit
            && match s(e, "kind") {
                "unit.report" => s(e, "to") == "blocked",
                "unit.scope.refused" | "unit.park" => true,
                _ => false,
            }
    })
}

/// Whether an event is an act of the human that starts the count of releases again for the unit.
fn human_act(e: &Value, unit: &str) -> bool {
    match s(e, "kind") {
        "unit.release" => s(e, "unit") == unit,
        "amend.approve" => {
            s(e, "by") != "judge"
                && (str_list(e, "reopen_units").iter().any(|u| u == unit)
                    || e["unblock_units"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| s(x, "unit") == unit)))
        }
        _ => false,
    }
}

/// The log position of the human's own latest act on the unit; 0 with none.
fn human_floor(events: &[Value], unit: &str) -> u64 {
    events
        .iter()
        .filter(|e| human_act(e, unit))
        .map(seq_of)
        .max()
        .unwrap_or(0)
}

/// The conductor's releases (`judgment = "fixed"`) of the unit since the human's own latest act on it.
/// A judge's approval never starts the count again (finding F-304).
fn counted_releases<'a>(events: &'a [Value], unit: &str) -> Vec<&'a Value> {
    let floor = human_floor(events, unit);
    events
        .iter()
        .filter(|e| {
            s(e, "kind") == "unit.unblock"
                && s(e, "unit") == unit
                && s(e, "judgment") == "fixed"
                && seq_of(e) > floor
        })
        .collect()
}

/// Each counted release as a line, numbered as its own event numbers it.
fn release_items(events: &[Value], unit: &str) -> Vec<String> {
    counted_releases(events, unit)
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let n = e["release"].as_u64().unwrap_or(i as u64 + 1);
            t(
                "unblock.release_item",
                &[("n", &n.to_string()), ("why", s(e, "why"))],
            )
        })
        .collect()
}

/// Every conductor release of the unit, those from before the human's own latest act marked as such.
fn all_release_items(events: &[Value], unit: &str) -> Vec<String> {
    let floor = human_floor(events, unit);
    events
        .iter()
        .filter(|e| {
            s(e, "kind") == "unit.unblock" && s(e, "unit") == unit && s(e, "judgment") == "fixed"
        })
        .enumerate()
        .map(|(i, e)| {
            let n = e["release"].as_u64().unwrap_or(i as u64 + 1);
            let key = if seq_of(e) > floor {
                "unblock.release_item"
            } else {
                "unblock.release_item.before"
            };
            t(key, &[("n", &n.to_string()), ("why", s(e, "why"))])
        })
        .collect()
}

/// How many releases the conductor used since the human's own latest act on the unit.
pub fn releases_used(events: &[Value], unit: &str) -> u32 {
    counted_releases(events, unit).len() as u32
}

fn judgeable_in(p: &Project, events: &[Value], st: &State, unit: &str) -> Option<Block> {
    if st.unit_state(unit) != "blocked" || crate::scope::open_request(st, unit).is_some() {
        return None;
    }
    let ev = latest_block(events, unit)?;
    if s(ev, "kind") != "unit.report" || s(ev, "result") != "blocked" {
        return None;
    }
    let reason = s(ev, "note").trim().to_string();
    let report_seq = seq_of(ev);
    if reason.is_empty()
        || events.iter().any(|e| {
            s(e, "kind") == "unit.scope.refused" && s(e, "unit") == unit && seq_of(e) > report_seq
        })
    {
        return None;
    }
    let pl = plan::load(p).ok()?;
    if crate::scope::reconsider::next_entry(p, st, unit, json!({})).is_some()
        || crate::scope::wait::next_entry(p, &pl, st, unit).is_some()
    {
        return None;
    }
    Some(Block {
        report_seq,
        reason,
        left: LIMIT.saturating_sub(releases_used(events, unit)),
    })
}

/// Whether the unit is the conductor's to judge, and with what.
pub fn judgeable(p: &Project, st: &State, unit: &str) -> Option<Block> {
    let log = events::load_log(p).ok()?;
    judgeable_in(p, &log.events, st, unit)
}

/// The latest `wait` or `human` judgment the conductor gave for that blocking report.
fn judgment_for<'a>(events: &'a [Value], unit: &str, report_seq: u64) -> Option<&'a Value> {
    events.iter().rev().find(|e| {
        s(e, "kind") == "unit.unblock"
            && s(e, "unit") == unit
            && e["report_seq"].as_u64() == Some(report_seq)
            && matches!(s(e, "judgment"), "wait" | "human")
    })
}

/// A landing or an approving amendment came after the wait.
fn wait_over(events: &[Value], wait_seq: u64) -> bool {
    events
        .iter()
        .any(|e| matches!(s(e, "kind"), "unit.land" | "amend.approve") && seq_of(e) > wait_seq)
}

fn stalled_in(p: &Project, pl: &Plan, st: &State, events: &[Value], unit: &str) -> bool {
    let Some(b) = judgeable_in(p, events, st, unit) else {
        return false;
    };
    let Some(w) = judgment_for(events, unit, b.report_seq).filter(|w| s(w, "judgment") == "wait")
    else {
        return false;
    };
    if wait_over(events, seq_of(w)) {
        return false;
    }
    if st.units.iter().any(|(id, u)| {
        id != unit
            && matches!(
                u.state.as_str(),
                "running" | "review" | "fixing" | "reviewed"
            )
    }) {
        return false;
    }
    if st
        .amends
        .iter()
        .any(|a| matches!(a.status.as_str(), "open" | "narrowed"))
    {
        return false;
    }
    let receipts = evidence::load_receipts(p).unwrap_or_default();
    !unit_states(p, pl, st, &receipts)
        .iter()
        .any(|(id, state, _)| id != unit && state == "ready")
}

/// After a `wait` judgment, whether nothing is left that could end the hold (finding F-305).
pub fn stalled(p: &Project, pl: &Plan, st: &State, unit: &str) -> bool {
    events::load_log(p)
        .map(|l| stalled_in(p, pl, st, &l.events, unit))
        .unwrap_or(false)
}

/// What `bf next` gives for a blocked unit after the reconsider and wait entries and before the
/// human's `decide-unit`; `None` leaves the unit to the human.
pub(crate) fn next_entry(p: &Project, pl: &Plan, st: &State, unit: &str) -> Option<NextEntry> {
    let log = events::load_log(p).ok()?;
    let b = judgeable_in(p, &log.events, st, unit)?;
    if let Some(j) = judgment_for(&log.events, unit, b.report_seq) {
        // With no release left the unit stays with the human, whatever the conductor judged.
        if s(j, "judgment") == "human" || b.left == 0 {
            return None;
        }
        if !wait_over(&log.events, seq_of(j)) {
            if stalled_in(p, pl, st, &log.events, unit) {
                return None;
            }
            return Some(NextEntry::Waiting(t(
                "unblock.next.wait.line",
                &[("unit", unit), ("reason", &b.reason), ("why", s(j, "why"))],
            )));
        }
    }
    if b.left == 0 {
        return None;
    }
    let left = b.left.to_string();
    let vals = [("unit", unit)];
    Some(NextEntry::Directive(json!({
        "role": "conductor", "action": "judge-block", "unit": unit,
        "reason": b.reason, "releases_left": b.left,
        "note": t("unblock.next.judge.note", &[("unit", unit), ("reason", &b.reason), ("left", &left)]),
        "commands": [
            t("unblock.next.cmd.fixed", &vals),
            t("unblock.next.cmd.wait", &vals),
            t("unblock.next.cmd.human", &vals),
        ],
    })))
}

/// The words added to the human's `decide-unit` note: each conductor release of the unit with its
/// words, and the conductor's `human` or stalled `wait` judgment.
pub(crate) fn human_note(p: &Project, pl: &Plan, st: &State, unit: &str) -> String {
    let Ok(log) = events::load_log(p) else {
        return String::new();
    };
    let mut out = String::new();
    let items = all_release_items(&log.events, unit);
    if !items.is_empty() {
        out.push_str(&t("unblock.note.releases", &[("list", &items.join("; "))]));
    }
    if let Some(b) = judgeable_in(p, &log.events, st, unit) {
        if let Some(j) = judgment_for(&log.events, unit, b.report_seq) {
            let human = s(j, "judgment") == "human";
            if human
                || (!wait_over(&log.events, seq_of(j)) && stalled_in(p, pl, st, &log.events, unit))
            {
                let key = if human {
                    "unblock.note.human"
                } else {
                    "unblock.note.stalled"
                };
                out.push_str(&t(key, &[("reason", &b.reason), ("why", s(j, "why"))]));
            }
        }
    }
    out
}

/// The `unit.unblock` event with `judgment = "fixed"` that put the unit back to work, while nothing
/// else has happened to it since.
fn latest_release<'a>(events: &'a [Value], st: &State, unit: &str) -> Option<&'a Value> {
    let last = st.units.get(unit)?.last_seq;
    events.iter().find(|e| {
        seq_of(e) == last
            && s(e, "kind") == "unit.unblock"
            && s(e, "unit") == unit
            && s(e, "judgment") == "fixed"
    })
}

/// The builder's directive of a unit the conductor released carries what it was released for.
pub(crate) fn decorate(p: &Project, st: &State, unit: &str, d: &mut Value) {
    let Ok(log) = events::load_log(p) else {
        return;
    };
    if let Some(e) = latest_release(&log.events, st, unit) {
        d["unblocked"] =
            json!({"reason": s(e, "reason"), "why": s(e, "why"), "release": e["release"]});
    }
}

/// The sections of a builder's brief: how to write a blocked report, and the conductor's release.
pub(crate) fn brief_md(p: &Project, st: &State, unit: &str) -> String {
    let mut md = format!(
        "## {}\n\n{}\n\n",
        t("unblock.brief.report.title", &[]),
        t("unblock.brief.report.body", &[])
    );
    if let Ok(log) = events::load_log(p) {
        if let Some(e) = latest_release(&log.events, st, unit) {
            md.push_str(&format!(
                "## {}\n\n{}\n\n",
                t("unblock.brief.released.title", &[]),
                t(
                    "unblock.brief.released.body",
                    &[
                        ("reason", s(e, "reason")),
                        ("why", s(e, "why")),
                        ("release", &e["release"].to_string()),
                    ]
                )
            ));
        }
    }
    md
}

fn no(unit: &str, because: String, step: String) -> crate::refusal::Fail {
    refuse_owned(
        t("unblock.refused", &[("unit", unit)]),
        because,
        vec![step],
        t("unblock.worked", &[("unit", unit)]),
    )
}

/// Why the unit is not the conductor's to judge, in the strings' words.
fn why_not(p: &Project, events: &[Value], st: &State, unit: &str) -> (String, String) {
    let v = [("unit", unit)];
    let other = t("unblock.step.other", &v);
    let Some(u) = st.units.get(unit) else {
        return (
            t("unblock.because.unknown", &v),
            t("unblock.step.unknown", &v),
        );
    };
    if u.state != "blocked" {
        return (
            t(
                "unblock.because.state",
                &[("unit", unit), ("state", &u.state)],
            ),
            other,
        );
    }
    if crate::scope::open_request(st, unit).is_some() {
        return (t("unblock.because.request_open", &v), other);
    }
    match latest_block(events, unit) {
        Some(e) if s(e, "kind") == "unit.report" && s(e, "result") == "blocked" => {
            if s(e, "note").trim().is_empty() {
                (t("unblock.because.no_note", &v), other)
            } else {
                (t("unblock.because.look_again", &v), other)
            }
        }
        _ => {
            let _ = p;
            (t("unblock.because.not_report", &v), other)
        }
    }
}

/// `bf unit unblock <U> --why "..." [--wait|--human]`.
pub fn unit_unblock(p: &Project, unit: &str, why: &str, wait: bool, human: bool) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("unblock {unit}"))?;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let v = [("unit", unit)];
    let why = why.trim();
    if why.is_empty() {
        return Err(no(
            unit,
            t("unblock.because.empty_why", &v),
            t("unblock.step.empty_why", &v),
        ));
    }
    if wait && human {
        return Err(no(
            unit,
            t("unblock.because.both_flags", &v),
            t("unblock.step.both_flags", &v),
        ));
    }
    let log = events::load_log(p)?;
    let Some(b) = judgeable_in(p, &log.events, &st, unit) else {
        let (because, step) = why_not(p, &log.events, &st, unit);
        return Err(no(unit, because, step));
    };
    let judgment = if wait {
        "wait"
    } else if human {
        "human"
    } else {
        "fixed"
    };
    let used = releases_used(&log.events, unit);
    if judgment != "human" && b.left == 0 {
        let list = release_items(&log.events, unit);
        return Err(no(
            unit,
            t(
                "unblock.because.no_release",
                &[
                    ("unit", unit),
                    ("used", &used.to_string()),
                    ("releases", &list.join("; ")),
                ],
            ),
            t("unblock.step.no_release", &v),
        ));
    }
    let u = &st.units[unit];
    let to = match u.prev_state.as_str() {
        "review" if u.parked_reason.starts_with("more than") => "fixing",
        "review" => "review",
        "fixing" => "fixing",
        _ => "running",
    };
    let release = used + 1;
    let mut ev = json!({"unit": unit, "report_seq": b.report_seq, "reason": b.reason, "why": why,
        "judgment": judgment, "from": u.state});
    if judgment == "fixed" {
        ev["release"] = json!(release);
        ev["to"] = json!(to);
    }
    events::append(p, "unit.unblock", ev)?;
    p.write_guard()?;
    let mut out = json!({"unit": unit, "judgment": judgment, "reason": b.reason, "why": why});
    match judgment {
        "fixed" => {
            out["state"] = json!(to);
            out["release"] = json!(release);
            let st = events::replay(p)?;
            let behind = unit_sync::auto_ff(p, &st, unit, "unblock");
            let _ = brief::write_brief(p, unit, "builder");
            let mut line = t(
                "unblock.out.line",
                &[
                    ("unit", unit),
                    ("state", to),
                    ("release", &release.to_string()),
                    ("limit", &LIMIT.to_string()),
                ],
            );
            if behind > 0 {
                out["behind"] = json!(behind);
                line.push_str(&t("unblock.out.behind", &[("behind", &behind.to_string())]));
            }
            out["line"] = json!(line);
        }
        "wait" => out["line"] = json!(t("unblock.out.wait", &v)),
        _ => out["line"] = json!(t("unblock.out.human", &v)),
    }
    Ok(out)
}
