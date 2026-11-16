//! U-106 (A-73, D-85): an order between two units of a checkpoint, set after the plan was made. When
//! a requested file is held beside by a unit that has not started, the engine puts that unit after the
//! asker in the run log, never in the records, and only when that makes no cycle.

use crate::events::{self, State};
use crate::plan::Plan;
use crate::refusal::R;
use crate::store::{s, str_list, Project};
use serde_json::{json, Value};

/// Replay of `unit.order`.
pub fn replay(st: &mut State, ev: &Value, seq: u64, _at: &str) {
    let (unit, after) = (s(ev, "unit"), s(ev, "after"));
    if unit.is_empty() || after.is_empty() {
        return;
    }
    st.orders.push(json!({
        "unit": unit, "after": after, "path": s(ev, "path"), "seq": seq
    }));
}

/// Whether the records alone make `a` come after `b`.
fn by_records(pl: &Plan, a: &str, b: &str) -> bool {
    crate::scope::depends_on(pl, a, b)
}

/// The orders of the log that count: one is ignored when the records make the asker depend on the
/// holder (an amendment did it later), so the records win and no cycle forms.
fn live<'a>(pl: &Plan, st: &'a State) -> impl Iterator<Item = &'a Value> {
    let skip: Vec<bool> = st
        .orders
        .iter()
        .map(|o| by_records(pl, s(o, "after"), s(o, "unit")))
        .collect();
    st.orders
        .iter()
        .zip(skip)
        .filter(|(_, sk)| !sk)
        .map(|(o, _)| o)
}

/// Whether `holder` comes after `unit`: it depends on it in the plan, or an order of the log puts it
/// there, directly or through other units.
pub fn after(pl: &Plan, st: &State, holder: &str, unit: &str) -> bool {
    let mut seen: Vec<String> = Vec::new();
    let mut todo = vec![holder.to_string()];
    let orders: Vec<&Value> = live(pl, st).collect();
    while let Some(x) = todo.pop() {
        if seen.contains(&x) {
            continue;
        }
        if by_records(pl, &x, unit) {
            return true;
        }
        for o in orders.iter().filter(|o| s(o, "unit") == x) {
            if s(o, "after") == unit {
                return true;
            }
            todo.push(s(o, "after").to_string());
        }
        if let Some(u) = pl.unit(&x) {
            todo.extend(str_list(u, "depends_on"));
        }
        seen.push(x);
    }
    false
}

/// Place `unit` after a holder of the asked files when the rules allow it; true when it wrote an
/// order, so the caller reads the state again.
pub fn place(p: &Project, pl: &Plan, st: &State, unit: &str, paths: &[String]) -> R<bool> {
    let mut wrote = false;
    let mut cur: Option<State> = None;
    loop {
        let st_now: &State = cur.as_ref().unwrap_or(st);
        let mut found: Option<(String, String)> = None;
        for path in paths {
            let Some(h) = crate::scope::holder_beside(p, pl, st_now, unit, path) else {
                continue;
            };
            let free = st_now.unit_state(&h).is_empty()
                && !after(pl, st_now, unit, &h)
                && !st_now
                    .orders
                    .iter()
                    .any(|o| s(o, "unit") == h && s(o, "after") == unit);
            if free {
                found = Some((h, path.clone()));
                break;
            }
        }
        let Some((h, path)) = found else {
            return Ok(wrote);
        };
        events::append(
            p,
            "unit.order",
            json!({"unit": h, "after": unit, "path": path,
                   "cause": "the unit holds a requested file and has not started"}),
        )?;
        wrote = true;
        cur = Some(events::replay(p)?);
    }
}
