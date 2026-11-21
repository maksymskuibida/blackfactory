//! U-101 (A-73): units that wait for a file another unit holds, and their automatic resume (D-82).

use crate::events::{self, State};
use crate::plan::{self, Plan};
use crate::refusal::R;
use crate::scope::{hard_rule, holder_beside, NextEntry};
use crate::store::{s, Project};
use serde_json::{json, Value};

const HELD: &str = " holds it and may run beside ";

fn held_only(rule: &str) -> bool {
    rule.contains(HELD) && rule.ends_with("only an order change fixes it")
}

fn paths_of(files: &Value) -> Vec<String> {
    let v = files.as_array().cloned().unwrap_or_default();
    v.iter().map(|f| s(f, "path").to_string()).collect()
}

fn holders_of(p: &Project, pl: &Plan, st: &State, unit: &str, paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in paths {
        if let Some(h) = holder_beside(p, pl, st, unit, f) {
            if !out.contains(&h) {
                out.push(h);
            }
        }
    }
    out
}

fn latest_block<'a>(events: &'a [Value], st: &State, unit: &str) -> Option<&'a Value> {
    if st.unit_state(unit) != "blocked" {
        return None;
    }
    events.iter().rev().find(|e| {
        s(e, "unit") == unit
            && match s(e, "kind") {
                "unit.report" => s(e, "to") == "blocked",
                "unit.scope.refused" | "unit.park" => true,
                _ => false,
            }
    })
}

fn waiting<'a>(events: &'a [Value], st: &State, unit: &str) -> Option<&'a Value> {
    latest_block(events, st, unit).filter(|e| {
        s(e, "kind") == "unit.scope.refused"
            && e["wait_for"].as_array().is_some_and(|a| !a.is_empty())
    })
}

/// Replay: a unit still blocked goes to the listed state; rounds and attempts stay.
pub fn replay(st: &mut State, ev: &Value, seq: u64, at: &str) {
    let (id, to) = (s(ev, "unit"), s(ev, "to"));
    let Some(u) = st.units.get_mut(id) else {
        return;
    };
    if u.state != "blocked" || !matches!(to, "running" | "fixing") {
        return;
    }
    u.state = to.to_string();
    u.last_seq = seq;
    u.last_at = at.to_string();
    let from = ev["refusal"].as_u64();
    if let Some(r) = st.scope.get_mut(id).and_then(|sc| {
        sc.refused
            .iter_mut()
            .rev()
            .find(|r| r["seq"].as_u64() == from)
    }) {
        r["resumed_seq"] = json!(seq);
    }
}

/// Resume the units that wait for a file now free; called after `amend.approve` and `unit.land`.
pub fn resume_waiting(p: &Project) -> R<()> {
    let log = events::load_log(p)?;
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let mut wrote = false;
    for (id, u) in st.units.iter().filter(|(_, u)| u.state == "blocked") {
        let Some(ev) = waiting(&log.events, &st, id) else {
            continue;
        };
        let paths = paths_of(&ev["files"]);
        if paths
            .iter()
            .any(|f| hard_rule(p, &pl, &st, id, f).is_some())
        {
            continue;
        }
        wrote = true;
        if u.attempts >= p.policy.limits.max_attempts {
            let why = format!("{} failing check runs or blocked reports in a row: it asked again for files a unit beside it holds", u.attempts);
            events::append(p, "unit.park", json!({"unit": id, "reason": why}))?;
            continue;
        }
        let to = [s(ev, "from"), u.prev_state.as_str(), "running"]
            .into_iter()
            .find(|x| matches!(*x, "running" | "fixing"))
            .unwrap_or("running");
        let cause = "every file it asked for is free: its holder landed or an approved amendment put the holder after it";
        events::append(
            p,
            "unit.scope.resume",
            json!({"unit": id, "refusal": ev["seq"], "files": paths, "to": to, "cause": cause}),
        )?;
    }
    if wrote {
        p.write_guard()?;
    }
    Ok(())
}

pub fn decorate(st: &State, id: &str, d: &mut Value) {
    let Some(u) = st.units.get(id) else {
        return;
    };
    let Some(r) = st.scope.get(id).and_then(|sc| sc.refused.last()) else {
        return;
    };
    if r["resumed_seq"].as_u64() == Some(u.last_seq) {
        d["ask_again"] = json!(paths_of(&r["files"]));
    }
}

fn circle(p: &Project, pl: &Plan, st: &State, log: &[Value], id: &str) -> Option<Vec<String>> {
    let mut seen: Vec<String> = Vec::new();
    let mut stack = vec![vec![id.to_string()]];
    while let Some(path) = stack.pop() {
        let last = path.last()?.clone();
        let Some(ev) = waiting(log, st, &last) else {
            continue;
        };
        for h in holders_of(p, pl, st, &last, &paths_of(&ev["files"])) {
            if h == id {
                return Some(path);
            }
            if !seen.contains(&h) {
                seen.push(h.clone());
                let mut n = path.clone();
                n.push(h);
                stack.push(n);
            }
        }
    }
    None
}

pub fn next_entry(p: &Project, pl: &Plan, st: &State, id: &str) -> Option<NextEntry> {
    let log = events::load_log(p).ok()?;
    let ev = waiting(&log.events, st, id)?;
    if st.units.get(id)?.attempts >= p.policy.limits.max_attempts {
        return None;
    }
    let paths = paths_of(&ev["files"]);
    if let Some(c) = circle(p, pl, st, &log.events, id) {
        let ring = format!("{} -> {id}", c.join(" -> "));
        // A plan repair already open for a unit of the ring is the step in hand.
        if let Some(a) = st.amends.iter().find(|a| {
            matches!(a.status.as_str(), "open" | "narrowed")
                && c.iter().any(|u| a.ids.contains(u) || a.keep.contains(u))
        }) {
            return Some(NextEntry::Waiting(crate::strings::t(
                "wait.circle.open.line",
                &[("id", id), ("ring", &ring), ("amend", &a.id)],
            )));
        }
        let mut units = c.clone();
        units.sort();
        units.dedup();
        let note = crate::strings::t(
            "wait.circle.note",
            &[
                ("id", id),
                ("ring", &ring),
                ("files", &paths.join(", ")),
                ("ids", &units.join(" ")),
            ],
        );
        return Some(NextEntry::Directive(
            json!({"role": "conductor", "action": "open-plan-repair", "unit": id, "state": "blocked", "ring": c, "files": paths, "note": note}),
        ));
    }
    let mut holders = holders_of(p, pl, st, id, &paths);
    if holders.is_empty() {
        holders = ev["wait_for"]
            .as_array()?
            .iter()
            .map(|h| h.as_str().unwrap_or("").to_string())
            .collect();
    }
    Some(NextEntry::Waiting(format!(
        "{id}: waiting for {} held by {}; it goes back to work by itself when they land or are put after it",
        paths.join(", "),
        holders.join(", ")
    )))
}

pub fn refusal_fields(p: &Project, pl: &Plan, st: &State, unit: &str, files: &[Value]) -> Value {
    if !matches!(st.unit_state(unit), "running" | "fixing")
        || files.is_empty()
        || !files.iter().all(|f| held_only(s(f, "rule")))
    {
        return json!({});
    }
    let paths: Vec<String> = files.iter().map(|f| s(f, "path").to_string()).collect();
    let holders = holders_of(p, pl, st, unit, &paths);
    if holders.is_empty() {
        return json!({});
    }
    json!({"wait_for": holders})
}
