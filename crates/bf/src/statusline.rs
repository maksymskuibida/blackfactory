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
        "closed" => ("■", "checkpoint closed"),
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
        "parked",
        "verifying",
        "closed",
    ];
    let mut groups: BTreeMap<usize, Vec<(&str, u32)>> = BTreeMap::new();
    for (id, state, round) in items {
        if let Some(ix) = order.iter().position(|o| o == state) {
            groups.entry(ix).or_default().push((id, *round));
        }
    }
    let mut parts = Vec::new();
    for (ix, units) in groups {
        let (sg, words) = sign(order[ix]).unwrap();
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
        if st.closed.contains_key(&kid) {
            out.push((kid, "closed".into(), 0));
        } else if !after.is_empty() && after.iter().all(|u| st.unit_state(u) == "landed") {
            out.push((kid, "verifying".into(), 0));
        }
    }
    Ok(out)
}

pub fn line(p: &Project) -> R<String> {
    Ok(render(&items(p)?))
}
