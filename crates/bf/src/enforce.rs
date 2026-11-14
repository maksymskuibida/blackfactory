//! The enforcement part (C-3): gate opening, the plan check, checkpoint closing and run closing refuse
//! when a required check has no proof. It asks the catalogue and the settings what is required
//! (`checks`) and the receipts what is proven (`evidence`).
//!
//! A required named check is proven through a check record that names it (`named = "<name>"`), so it
//! appears in the plan like every other check. A design-stage check leaves a product before the plan
//! exists: a receipt for the UI/UX verification, the threat records, the markets statement. A built-stage
//! check is proven by a passing receipt at the content being closed.

use crate::checks::{self, NamedCheck, Stage};
use crate::events::State;
use crate::evidence::{self, Receipt};
use crate::plan::{self, Plan};
use crate::refusal::{refuse_owned, R};
use crate::schema;
use crate::store::{s, str_list, Project};
use serde_json::{json, Value};

/// The check records of the plan that name this named check.
pub fn records_naming<'a>(pl: &'a Plan, name: &str) -> Vec<&'a Value> {
    pl.checks.iter().filter(|c| s(c, "named") == name).collect()
}

/// The named checks demanded now. A product with no interface (its ui gate skipped) has no UI/UX
/// verification of its design to demand.
pub fn required(st: &State) -> Vec<&'static NamedCheck> {
    checks::required_now(st)
        .into_iter()
        .filter(|c| !(c.name == "design-uiux-verification" && st.skipped_words("ui").is_some()))
        .collect()
}

/// The checks the human switched off, with their reasons: what a summary says was off.
pub fn off_summary(st: &State) -> Vec<Value> {
    checks::off_now(st)
        .iter()
        .map(|(n, r)| json!({"name": n, "reason": r}))
        .collect()
}

pub fn off_text(off: &[Value]) -> String {
    off.iter()
        .map(|o| {
            format!(
                "{} was off ({})",
                o["name"].as_str().unwrap_or(""),
                o["reason"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Problems the plan has for want of a check record for a required named check.
pub fn plan_problems(pl: &Plan, st: &State) -> Vec<String> {
    if pl.units.is_empty() {
        return vec![];
    }
    required(st)
        .iter()
        .filter(|c| records_naming(pl, c.name).is_empty())
        .map(|c| {
            format!(
                "the required check `{}` has no check record that names it (add a check record with `named = \"{}\"`, or ask the human to switch the check off)",
                c.name, c.name
            )
        })
        .collect()
}

fn design_receipt(pl: &Plan, receipts: &[Receipt], name: &str) -> bool {
    records_naming(pl, name).iter().any(|c| {
        receipts
            .iter()
            .any(|r| r.check() == s(c, "id") && r.passes())
    })
}

fn count(p: &Project, kind: &str) -> R<usize> {
    Ok(p.list_records(schema::kind_by_name(kind))?.len())
}

/// R-5: why a design gate may not open for want of the design-stage product of a required check.
pub fn design_blocker(p: &Project, st: &State, kind: &str) -> R<()> {
    let req = checks::required_now(st);
    let on = |n: &str| req.iter().any(|c| c.name == n);
    let mut gaps: Vec<String> = Vec::new();
    match kind {
        "ui" if on("design-uiux-verification") => {
            let pl = plan::load(p)?;
            let receipts = evidence::load_receipts(p)?;
            if !design_receipt(&pl, &receipts, "design-uiux-verification") {
                gaps.push("the design-stage UI/UX verification has no receipt: put a check record with `named = \"design-uiux-verification\"` and `how = \"agent\"`, have the screens judged, and attach the result with `bf evidence attach <X> <file> --outcome met --says \"<what was looked at>\"`".into());
            }
        }
        "architecture" => {
            if on("threat-model") && count(p, "threat")? == 0 {
                gaps.push("no threat is recorded and the threat model check is on: put threat records (`bf record put threat`), each naming what is protected, how it is attacked and how the design answers it".into());
            }
            let legal_on = checks::in_family(checks::Family::Legal)
                .iter()
                .any(|c| on(c.name));
            if legal_on && count(p, "markets")? == 0 {
                gaps.push("the project has no markets statement and the legal checks are on: put a markets record (`bf record put markets`); a statement of no market is a markets record with an empty list".into());
            }
        }
        _ => {}
    }
    if gaps.is_empty() {
        return Ok(());
    }
    let mut steps = gaps.clone();
    steps.push(format!("Open the `{kind}` gate again."));
    Err(refuse_owned(
        format!("open the `{kind}` gate"),
        format!(
            "{} required check(s) have no design-stage product",
            gaps.len()
        ),
        steps,
        format!("`bf gate open {kind}` prints a gate ID."),
    ))
}

/// Whether a built-stage named check has a passing proof: at the content a unit landed on, or a
/// checkpoint closed on.
fn built_proof(pl: &Plan, receipts: &[Receipt], st: &State, name: &str) -> bool {
    records_naming(pl, name).iter().any(|c| {
        let cid = s(c, "id");
        let unit = s(c, "unit");
        let by_unit = !unit.is_empty()
            && st
                .units
                .get(unit)
                .filter(|u| u.state == "landed")
                .map(|u| evidence::satisfied_at(receipts, cid, &u.landed_hash).is_some())
                .unwrap_or(false);
        by_unit
            || pl.checkpoints.iter().any(|k| {
                let kid = s(k, "id");
                str_list(k, "checks").iter().any(|x| x == cid)
                    && st
                        .closed
                        .get(kid)
                        .map(|cl| {
                            if s(c, "how") == "human" {
                                st.gate_passed("checkpoint-step", Some(kid))
                            } else {
                                evidence::satisfied_at(receipts, cid, &cl.hash).is_some()
                            }
                        })
                        .unwrap_or(false)
            })
    })
}

fn no_record_or_proof(pl: &Plan, name: &str) -> String {
    let recs = records_naming(pl, name);
    if recs.is_empty() {
        format!("{name}: no check record names it")
    } else {
        format!(
            "{name}: no passing receipt at the content being closed from {}",
            recs.iter()
                .map(|c| s(c, "id"))
                .collect::<Vec<_>>()
                .join(" or ")
        )
    }
}

/// R-4 at the end of the run: every required check has a passing proof. Returns what was off.
pub fn refuse_if_unproven(p: &Project, pl: &Plan) -> R<Value> {
    let st = crate::events::require_chain(p)?;
    let receipts = evidence::load_receipts(p)?;
    let mut gaps: Vec<String> = Vec::new();
    for c in required(&st) {
        let ok = match c.stage {
            Stage::Design => match c.name {
                "design-uiux-verification" => design_receipt(pl, &receipts, c.name),
                "threat-model" => count(p, "threat")? > 0,
                "markets-statement" => count(p, "markets")? > 0,
                _ => true,
            },
            Stage::Built => built_proof(pl, &receipts, &st, c.name),
        };
        if !ok {
            gaps.push(match c.stage {
                Stage::Design => format!("{}: its design-stage product is missing", c.name),
                Stage::Built => no_record_or_proof(pl, c.name),
            });
        }
    }
    let marked = crate::legal::unacknowledged(p)?;
    for m in &marked {
        gaps.push(format!("{m}: a marked legal finding is not acknowledged"));
    }
    if !gaps.is_empty() {
        let mut steps: Vec<String> = gaps.iter().map(|g| format!("Fix: {g}.")).collect();
        steps.push("Run `bf ledger check` again.".into());
        return Err(refuse_owned(
            "close the run",
            format!("{} required check(s) have no proof", gaps.len()),
            steps,
            "`bf ledger check` prints `ledger closed` and exits 0.",
        ));
    }
    Ok(json!(off_summary(&st)))
}

/// R-4 at a checkpoint: each required built-stage check that the checkpoint lists needs a passing
/// receipt at the content now in the checkpoint, and no marked legal finding may be unacknowledged.
pub fn checkpoint_gaps(
    p: &Project,
    pl: &Plan,
    st: &State,
    receipts: &[Receipt],
    k: &Value,
    content: &str,
) -> R<Vec<String>> {
    let kid = s(k, "id");
    let listed = str_list(k, "checks");
    let mut gaps = Vec::new();
    for c in required(st).iter().filter(|c| c.stage == Stage::Built) {
        let recs: Vec<&Value> = records_naming(pl, c.name)
            .into_iter()
            .filter(|r| listed.iter().any(|x| x == s(r, "id")))
            .collect();
        if recs.is_empty() {
            continue;
        }
        let ok = recs.iter().any(|r| {
            if s(r, "how") == "human" {
                st.gate_passed("checkpoint-step", Some(kid))
            } else {
                evidence::satisfied_at(receipts, s(r, "id"), content).is_some()
            }
        });
        if !ok {
            gaps.push(format!(
                "the required check `{}` has no passing receipt at this content from {}",
                c.name,
                recs.iter()
                    .map(|r| s(r, "id"))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ));
        }
    }
    for m in crate::legal::unacknowledged(p)? {
        gaps.push(format!("{m}: a marked legal finding is not acknowledged"));
    }
    Ok(gaps)
}

/// A checkpoint stops for the human or has no stop. A record from an earlier build that says carry on
/// is read as a stop.
pub fn stops(k: &Value) -> bool {
    k.get("stop") == Some(&Value::Bool(true))
        || !s(k, "human_step").is_empty()
        || !s(k, "on_human_step").is_empty()
}
