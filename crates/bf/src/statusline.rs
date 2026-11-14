//! The one-line progress report: a sign per state, the unit IDs in that state, and the review round.
//! The conductor shows this line as the whole progress report; it does not compose it.

use crate::refusal::R;
use crate::store::{s, str_list, Project};
use crate::{events, evidence, plan, run};
use std::collections::BTreeMap;

/// The sign and the words for a state. `None` for states that are not work in progress (planned, ready).
pub fn sign(state: &str) -> Option<(&'static str, &'static str)> {
    Some(match state {
        "running" => ("▶", "building"),
        "review" => ("◐", "in review"),
        "fixing" => ("↻", "fixing"),
        "reviewed" => ("●", "reviewed, ready to land"),
        "landed" => ("✔", "landed"),
        "blocked" => ("⚠", "blocked"),
        "parked" => ("⚠", "parked"),
        "verifying" => ("◎", "checkpoint being verified"),
        "fix-review" => ("◐", "checkpoint fix in review"),
        "fix-fixing" => ("↻", "checkpoint being fixed"),
        "closed" => ("■", "checkpoint closed"),
        "wave-running" => ("◌", "wave check running"),
        "wave-passed" => ("✓", "wave check passed"),
        "wave-failed" => ("✗", "wave check failed"),
        "wave-fix-fixing" => ("↻", "wave check being fixed"),
        "wave-fix-review" => ("◐", "wave check fix in review"),
        _ => return None,
    })
}

/// One item: an ID, its state and its review round (0 when it has none to show).
pub type Item = (String, String, u32);

/// Build the line. Items in the same state (and round) share one segment.
pub fn render(items: &[Item]) -> String {
    let order = [
        "running",
        "review",
        "fixing",
        "reviewed",
        "landed",
        "blocked",
        "fix-review",
        "fix-fixing",
        "verifying",
        "wave-running",
        "wave-failed",
        "wave-fix-fixing",
        "wave-fix-review",
        "wave-passed",
        "closed",
    ];
    // Blocked and parked units share one sign and so one segment.
    let mut groups: BTreeMap<usize, Vec<(&str, u32)>> = BTreeMap::new();
    let mut stuck: Vec<(&str, &str)> = Vec::new();
    for (id, state, round) in items {
        let key = if state == "parked" { "blocked" } else { state };
        if let Some(ix) = order.iter().position(|o| *o == key) {
            groups.entry(ix).or_default().push((id, *round));
            if key == "blocked" {
                stuck.push((id, state));
            }
        }
    }
    let mut parts = Vec::new();
    for (ix, units) in groups {
        let (sg, words) = sign(order[ix]).unwrap();
        if order[ix] == "blocked" {
            let all_same = stuck.iter().all(|(_, s)| *s == stuck[0].1);
            let t = if all_same {
                let ids: Vec<&str> = stuck.iter().map(|(i, _)| *i).collect();
                format!("{sg} {} {}", ids.join(", "), stuck[0].1)
            } else {
                let each: Vec<String> = stuck.iter().map(|(i, s)| format!("{i} {s}")).collect();
                format!("{sg} {}", each.join(", "))
            };
            parts.push(t);
            continue;
        }
        // One segment per state. When every unit is in the same round it is said once at the end;
        // otherwise each unit carries its own round.
        let same = units.iter().all(|(_, r)| *r == units[0].1);
        let ids: Vec<String> = units
            .iter()
            .map(|(id, r)| {
                if !same && *r > 0 {
                    format!("{id} (round {r})")
                } else {
                    id.to_string()
                }
            })
            .collect();
        let mut t = format!("{sg} {} {words}", ids.join(", "));
        if same && units[0].1 > 0 {
            t.push_str(&format!(", round {}", units[0].1));
        }
        parts.push(t);
    }
    if parts.is_empty() {
        "no unit has started".to_string()
    } else {
        parts.join(" · ")
    }
}

/// The items of the run now: units with their state, checkpoints being verified or closed.
pub fn items(p: &Project) -> R<Vec<Item>> {
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let mut out: Vec<Item> = Vec::new();
    for (id, state, _) in run::unit_states(p, &pl, &st, &receipts) {
        let rounds = st.units.get(&id).map(|u| u.rounds).unwrap_or(0);
        // The round under review is the one after the findings so far; a fixing unit is in the round
        // whose findings it fixes.
        let round = match state.as_str() {
            "review" => rounds + 1,
            "fixing" => rounds,
            _ => 0,
        };
        out.push((id, state, round));
    }
    for k in &pl.checkpoints {
        let kid = s(k, "id").to_string();
        let after = str_list(k, "after");
        // The latest wave check of a checkpoint that is not closed: running, passed or failed. A failed
        // one with its fix open is shown as the fix.
        let wave_fix_open = st
            .ck_fix
            .as_ref()
            .map(|f| f.checkpoint == kid && f.source == "wave")
            .unwrap_or(false);
        if !st.closed.contains_key(&kid) && !wave_fix_open {
            if let Some((w, r)) = st
                .wave_runs
                .iter()
                .filter(|((c, _), _)| *c == kid)
                .map(|((_, w), r)| (*w, r))
                .max_by_key(|(w, _)| *w)
            {
                let state = match r.state.as_str() {
                    "running" => "wave-running",
                    "passed" => "wave-passed",
                    "failed" => "wave-failed",
                    _ => "",
                };
                if !state.is_empty() {
                    out.push((format!("{kid} wave {w}"), state.into(), 0));
                }
            }
        }
        if st.closed.contains_key(&kid) {
            out.push((kid, "closed".into(), 0));
        } else if let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == kid) {
            // A checkpoint with a fix open: fixing (round of the findings it fixes), in review (the round
            // under review), or waiting for the human like a parked unit. A wave fix says so.
            let judged = f.rounds_log.len() as u32;
            let label = if f.source == "wave" {
                format!("{kid} wave {}", f.wave)
            } else {
                kid.clone()
            };
            let wave = f.source == "wave";
            match f.phase.as_str() {
                "review" if wave => out.push((label, "wave-fix-review".into(), judged + 1)),
                "fix" if wave => out.push((label, "wave-fix-fixing".into(), judged)),
                "review" => out.push((kid, "fix-review".into(), judged + 1)),
                "fix" => out.push((kid, "fix-fixing".into(), judged)),
                "gap" => out.push((kid, "blocked".into(), 0)),
                _ => out.push((kid, "parked".into(), 0)),
            }
        } else if !after.is_empty() && after.iter().all(|u| st.unit_state(u) == "landed") {
            out.push((kid, "verifying".into(), 0));
        }
    }
    Ok(out)
}

pub fn line(p: &Project) -> R<String> {
    Ok(render(&items(p)?))
}
