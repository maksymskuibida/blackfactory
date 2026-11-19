//! The accept of findings on a parked review (D-91, H-23, H-24): the replay of `checkpoint.fix.finding.settle`
//! and `unit.finding.accept`, and the stub of the fix review's command.

use crate::events::{
    decision_bound, decision_turn_ok, id_pairs, Accepted, Decide, State,
};
use crate::refusal::R;
use crate::store::Project;
use serde_json::{json, Value};
use std::collections::HashMap;

fn ignore(st: &mut State, key: &str, seq: u64) {
    st.ignored
        .push(crate::strings::t(key, &[("seq", &seq.to_string())]));
}

/// Settle findings of the open fix's review in the human's own words. A stub: it writes nothing.
pub fn settle_fix(_p: &Project, _checkpoint: &str, _ids: &[(u32, u32)], _quote: &str) -> R<Value> {
    Ok(json!({}))
}

/// Replay of `checkpoint.fix.finding.settle`: counts only with a valid human turn after the fix review
/// that filed the findings, by the rule of `checkpoint.finding.settle`.
pub(crate) fn replay_fix(st: &mut State, ix: &HashMap<u64, usize>, ev: &Value, seq: u64) {
    let sv = |k: &str| ev[k].as_str().unwrap_or("").to_string();
    let ck = sv("checkpoint");
    let ids = id_pairs(&ev["ids"]);
    if ids.is_empty() || sv("fate") != "accepted" {
        return ignore(st, "events.k2.ignored.fix_settle_form", seq);
    }
    // Every finding named must be one of a review round of the open fix.
    let mut levels = Vec::new();
    let mut base = 0;
    let fix_open;
    {
        let Some(o) = st.ck_fix.as_ref().filter(|o| o.checkpoint == ck) else {
            return ignore(st, "events.k2.ignored.fix_settle_none", seq);
        };
        fix_open = o.open_seq;
        for (r, i) in &ids {
            let f = o
                .history
                .iter()
                .rev()
                .find(|h| h["round"].as_u64() == Some(*r as u64) && *r >= 1)
                .and_then(|h| h["findings"].as_array())
                .and_then(|l| l.get((*i as usize).wrapping_sub(1)));
            let Some(f) = f else {
                return ignore(st, "events.k2.ignored.fix_settle_unknown", seq);
            };
            levels.push(f["severity"].as_str().unwrap_or("").to_string());
            base = base.max(
                o.round_seqs
                    .iter()
                    .filter(|(round, _)| round == r)
                    .map(|(_, q)| *q)
                    .max()
                    .unwrap_or(0),
            );
        }
    }
    let bound = match decision_bound(st, &ck, base, seq) {
        Ok(b) => b,
        Err(_) => return ignore(st, "events.k2.ignored.fix_settle_hold", seq),
    };
    let tseq = ev["turn"].as_u64().unwrap_or(0);
    if !decision_turn_ok(st, ix, tseq, bound, seq, &sv("quote"), Decide::Approve) {
        return ignore(st, "events.k2.ignored.fix_settle_turn", seq);
    }
    st.turns[ix[&tseq]].used = true;
    let mut added = Vec::new();
    for ((round, index), level) in ids.iter().zip(levels) {
        added.push(Accepted {
            scope: "fix".into(),
            subject: ck.clone(),
            id: String::new(),
            round: *round,
            index: *index,
            level,
            turn: tseq,
            quote: sv("quote"),
            seq,
            fix_open,
        });
    }
    let Some(o) = st.ck_fix.as_mut() else { return };
    for a in &added {
        if !o
            .accepted
            .iter()
            .any(|x| (x.fix_open, x.round, x.index) == (a.fix_open, a.round, a.index))
        {
            o.accepted.push(a.clone());
        }
    }
    // The open findings are those of the latest review round that no accept covers.
    let latest = o.history.last().cloned();
    if let Some(h) = latest.filter(|h| h["round"].as_u64().unwrap_or(0) >= 1) {
        let round = h["round"].as_u64().unwrap_or(0) as u32;
        let list = h["findings"].as_array().cloned().unwrap_or_default();
        let was_open = !o.findings.is_empty();
        o.findings = list
            .into_iter()
            .enumerate()
            .filter(|(n, _)| {
                !o.accepted
                    .iter()
                    .any(|a| a.round == round && a.index as usize == n + 1)
            })
            .map(|(_, f)| f)
            .collect();
        if was_open && o.findings.is_empty() {
            // Every open finding is accepted: the review is settled as clean.
            o.rounds_log
                .push(json!({"round": round, "result": "clean", "accepted": true}));
            o.phase = "review".into();
            o.reason.clear();
        }
    }
    st.accepted.extend(added);
}

/// Replay of `unit.finding.accept`: counts only with a valid human turn after the unit's latest review.
pub(crate) fn replay_unit(st: &mut State, ix: &HashMap<u64, usize>, ev: &Value, seq: u64) {
    let sv = |k: &str| ev[k].as_str().unwrap_or("").to_string();
    let unit = sv("unit");
    let listed: Vec<(String, String)> = ev["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let (id, level) = match f.as_str() {
                Some(id) => (id.to_string(), ev["levels"][id].as_str().unwrap_or("").to_string()),
                None => (
                    f["id"].as_str()?.to_string(),
                    f["severity"].as_str().unwrap_or("").to_string(),
                ),
            };
            Some((id, level))
        })
        .collect();
    let Some(after) = st.units.get(&unit).map(|u| u.review_seq) else {
        return ignore(st, "events.k2.ignored.unit_accept_none", seq);
    };
    if listed.is_empty() {
        return ignore(st, "events.k2.ignored.unit_accept_form", seq);
    }
    // An accept names only findings the unit's reports listed and that are still open. The log cannot
    // know a finding filed only with `bf record put finding`; it is not counted here (H-24).
    let open = st.units.get(&unit).map(|u| u.open_findings.clone()).unwrap_or_default();
    if listed.iter().any(|(id, _)| !open.contains(id)) {
        return ignore(st, "events.k2.ignored.unit_accept_unknown", seq);
    }
    let tseq = ev["turn"].as_u64().unwrap_or(0);
    if !decision_turn_ok(st, ix, tseq, after, seq, &sv("quote"), Decide::Approve) {
        return ignore(st, "events.k2.ignored.unit_accept_turn", seq);
    }
    st.turns[ix[&tseq]].used = true;
    for (id, level) in &listed {
        st.accepted.push(Accepted {
            scope: "unit".into(),
            subject: unit.clone(),
            id: id.clone(),
            round: 0,
            index: 0,
            level: level.clone(),
            turn: tseq,
            quote: sv("quote"),
            seq,
            fix_open: 0,
        });
    }
    if let Some(u) = st.units.get_mut(&unit) {
        let had = !u.open_findings.is_empty();
        u.open_findings.retain(|o| !listed.iter().any(|(id, _)| id == o));
        // A parked unit whose open findings are all fixed or accepted goes on to `reviewed`.
        if had && u.open_findings.is_empty() && u.state == "parked" {
            u.state = "reviewed".into();
            u.parked_reason.clear();
            u.attempts = 0;
        }
        u.last_seq = seq;
    }
}
