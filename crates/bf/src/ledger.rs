//! The traceability ledger: which designed parts have units, checks and evidence.

use crate::events::State;
use crate::evidence::{self, Receipt};
use crate::plan::Plan;
use crate::refusal::{refuse_owned, R};
use crate::store::{s, str_list, Project};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub units: Vec<String>,
    pub checkpoints: Vec<String>,
    pub checks: Vec<String>,
    pub evidence: Vec<String>,
    pub missing: Vec<&'static str>,
}

/// Evidence for a part: a sealed passing receipt, at the content a checkpoint closed on, from one of that
/// checkpoint's checks that proves the part; or, for a human check, the checkpoint's approved step gate.
/// A receipt from a version that no longer exists counts for nothing.
fn evidence_for(pl: &Plan, receipts: &[Receipt], st: &State, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        let Some(closed) = st.closed.get(kid) else {
            continue;
        };
        if !str_list(k, "proves").iter().any(|x| x == id) {
            continue;
        }
        for c in &pl.checks {
            if !str_list(k, "checks").iter().any(|x| x == s(c, "id"))
                || !str_list(c, "proves").iter().any(|x| x == id)
            {
                continue;
            }
            if s(c, "how") == "human" {
                if st.gate_passed("checkpoint-step", Some(kid)) {
                    out.push(format!("gate:{kid}"));
                }
            } else if let Some(r) = evidence::satisfied_at(receipts, s(c, "id"), &closed.hash) {
                out.push(r.id().to_string());
            }
        }
    }
    out
}

pub fn rows(pl: &Plan, receipts: &[Receipt], with_evidence: bool, st: &State) -> Vec<Row> {
    let mut by_unit: HashMap<String, Vec<String>> = HashMap::new();
    for u in &pl.units {
        for d in str_list(u, "delivers") {
            by_unit.entry(d).or_default().push(s(u, "id").to_string());
        }
    }
    let mut by_check: HashMap<String, Vec<String>> = HashMap::new();
    for c in &pl.checks {
        for d in str_list(c, "proves") {
            by_check.entry(d).or_default().push(s(c, "id").to_string());
        }
    }
    let mut by_cp: HashMap<String, Vec<String>> = HashMap::new();
    for k in &pl.checkpoints {
        for d in str_list(k, "proves") {
            by_cp.entry(d).or_default().push(s(k, "id").to_string());
        }
    }
    let mut out = Vec::new();
    for d in &pl.designed {
        let id = s(d, "id").to_string();
        let units = by_unit.get(&id).cloned().unwrap_or_default();
        let checks = by_check.get(&id).cloned().unwrap_or_default();
        let cps = by_cp.get(&id).cloned().unwrap_or_default();
        let evidence: Vec<String> = if with_evidence {
            evidence_for(pl, receipts, st, &id)
        } else {
            vec![]
        };
        let mut missing = Vec::new();
        if units.is_empty() {
            missing.push("unit");
        }
        if checks.is_empty() {
            missing.push("check");
        }
        if with_evidence && evidence.is_empty() {
            missing.push("evidence");
        }
        out.push(Row {
            id,
            title: s(d, "title").to_string(),
            kind: s(d, "kind").to_string(),
            units,
            checkpoints: cps,
            checks,
            evidence,
            missing,
        });
    }
    out
}

pub fn check(p: &Project, pl: &Plan, plan_only: bool) -> R<Vec<Row>> {
    let receipts = if plan_only {
        vec![]
    } else {
        evidence::load_receipts(p)?
    };
    let st = crate::events::replay(p)?;
    if !plan_only {
        crate::run::refuse_if_main_moved(p, &st)?;
    }
    Ok(rows(pl, &receipts, !plan_only, &st))
}

pub fn refuse_if_gaps(rows: &[Row], plan_only: bool) -> R<()> {
    let gaps: Vec<&Row> = rows.iter().filter(|r| !r.missing.is_empty()).collect();
    if gaps.is_empty() {
        return Ok(());
    }
    let mut steps: Vec<String> = Vec::new();
    for g in gaps.iter().take(40) {
        steps.push(format!(
            "{} ({}) has no {} linked: add it to a unit's `delivers`, a check's `proves`{}.",
            g.id,
            g.title,
            g.missing.join(", no "),
            if plan_only {
                ""
            } else {
                ", or run the check with `bf evidence run`"
            }
        ));
    }
    if gaps.len() > 40 {
        steps.push(format!(
            "...and {} more; run `bf ledger check --json` for all of them.",
            gaps.len() - 40
        ));
    }
    steps.push("Run `bf ledger check` again.".into());
    Err(refuse_owned(
        "close the ledger",
        format!("{} designed part(s) are not fully linked", gaps.len()),
        steps,
        "`bf ledger check` prints `ledger closed` and exits 0.",
    ))
}

/// `bf ledger impact`: everything linked to the given IDs.
pub fn impact(p: &Project, pl: &Plan, ids: &[String]) -> R<Value> {
    let receipts = evidence::load_receipts(p)?;
    let mut units: BTreeSet<String> = BTreeSet::new();
    for u in &pl.units {
        let del = str_list(u, "delivers");
        if ids.iter().any(|i| del.contains(i) || i == s(u, "id")) {
            units.insert(s(u, "id").to_string());
        }
    }
    // Units that depend on an impacted unit are impacted too.
    loop {
        let before = units.len();
        for u in &pl.units {
            if str_list(u, "depends_on").iter().any(|d| units.contains(d)) {
                units.insert(s(u, "id").to_string());
            }
        }
        if units.len() == before {
            break;
        }
    }
    let handoffs: BTreeSet<String> = pl
        .handoffs
        .iter()
        .filter(|h| {
            units.contains(s(h, "from"))
                || units.contains(s(h, "to"))
                || ids.iter().any(|i| i == s(h, "id"))
        })
        .map(|h| s(h, "id").to_string())
        .collect();
    let mut checks: BTreeSet<String> = BTreeSet::new();
    for c in &pl.checks {
        let pr = str_list(c, "proves");
        if ids.iter().any(|i| pr.contains(i) || i == s(c, "id"))
            || units.contains(s(c, "unit"))
            || pr.iter().any(|x| handoffs.contains(x))
        {
            checks.insert(s(c, "id").to_string());
        }
    }
    let mut cps: BTreeSet<String> = BTreeSet::new();
    for k in &pl.checkpoints {
        let pr = str_list(k, "proves");
        let after = str_list(k, "after");
        if ids.iter().any(|i| pr.contains(i) || i == s(k, "id"))
            || after.iter().any(|u| units.contains(u))
            || str_list(k, "checks").iter().any(|c| checks.contains(c))
        {
            cps.insert(s(k, "id").to_string());
        }
    }
    let rc: Vec<String> = receipts
        .iter()
        .filter(|r| checks.contains(r.check()))
        .map(|r| r.id().to_string())
        .collect();
    Ok(json!({
        "ids": ids,
        "units": units,
        "handoffs": handoffs,
        "checks": checks,
        "checkpoints": cps,
        "receipts": rc,
    }))
}
