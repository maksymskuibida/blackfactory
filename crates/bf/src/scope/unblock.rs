//! U-100 (A-73): an approved amendment puts back to work the blocked units it re-opens or whose
//! checks it changes, with no release (R-107, D-82). The write command decides, the replay reads.

use crate::events::{Amend, State};
use crate::store::{s, Project};
use serde_json::{json, Value};

/// The state a blocked unit goes back to: the mapping of `bf unit release`.
fn back_to(prev_state: &str, parked_reason: &str) -> &'static str {
    match prev_state {
        "review" if parked_reason.starts_with("more than") => "fixing",
        "review" => "review",
        "fixing" => "fixing",
        _ => "running",
    }
}

/// Replay, inside the branch of `amend.approve` that approves, after the units are re-opened.
/// It reads only the list in the event: an approval without it changes nothing here.
pub fn replay(st: &mut State, ev: &Value, amend: &str, seq: u64, at: &str) {
    let Some(list) = ev.get("unblock_units").and_then(|v| v.as_array()) else {
        return;
    };
    for e in list {
        let (Some(unit), Some(to)) = (e["unit"].as_str(), e["to"].as_str()) else {
            continue;
        };
        if let Some(u) = st.units.get_mut(unit) {
            if u.state == "blocked" {
                u.state = to.to_string();
                u.parked_reason.clear();
                u.reopened_by = amend.to_string();
                u.last_seq = seq;
                u.last_at = at.to_string();
            }
        }
    }
}

/// The object whose keys are added to the `amend.approve` event data; its `unblock_units` is also
/// shown in the command's output. Empty: no key is added.
pub fn approve_fields(
    p: &Project,
    st: &State,
    amendment: &Amend,
    reopen_units: &[String],
) -> Value {
    // The records as the approval leaves them; a plan that cannot be read names no check owner.
    let owners: Vec<String> = crate::plan::load(p)
        .map(|pl| {
            pl.checks
                .iter()
                .filter(|c| {
                    let id = s(c, "id").to_string();
                    amendment.keep.contains(&id) && !amendment.waive.contains(&id)
                })
                .map(|c| s(c, "unit").to_string())
                .collect()
        })
        .unwrap_or_default();
    let list: Vec<Value> = st
        .units
        .iter()
        .filter(|(id, u)| {
            u.state == "blocked" && (reopen_units.contains(id) || owners.contains(id))
        })
        .map(|(id, u)| json!({"unit": id, "to": back_to(&u.prev_state, &u.parked_reason)}))
        .collect();
    if list.is_empty() {
        json!({})
    } else {
        json!({"unblock_units": list})
    }
}
