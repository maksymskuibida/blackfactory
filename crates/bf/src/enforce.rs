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
    without_skipped_ui(st, checks::required_now(st))
}

/// The named checks demanded when the run is closed: the choice of the last run, also after it was
/// stopped (the project default when no run was ever started).
pub fn required_closing(st: &State) -> Vec<&'static NamedCheck> {
    without_skipped_ui(st, checks::required_closing(st))
}

fn without_skipped_ui(st: &State, v: Vec<&'static NamedCheck>) -> Vec<&'static NamedCheck> {
    v.into_iter()
        .filter(|c| !(c.name == "design-uiux-verification" && st.skipped_words("ui").is_some()))
        .collect()
}

/// The last checkpoint of the plan: its content holds everything, so a built-stage check counts only there.
pub fn last_checkpoint(pl: &Plan) -> Option<&Value> {
    pl.checkpoints.last()
}

fn listed_by_last(pl: &Plan, record: &Value) -> bool {
    last_checkpoint(pl)
        .map(|k| str_list(k, "checks").iter().any(|x| x == s(record, "id")))
        .unwrap_or(false)
}

/// The checks the human switched off, with their reasons: what a summary says was off.
pub fn off_summary(st: &State) -> Vec<Value> {
    checks::off_closing(st)
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
pub fn plan_problems(pl: &Plan, st: &State, decided: bool) -> Vec<String> {
    if pl.units.is_empty() {
        return vec![];
    }
    if !decided {
        return vec!["no check settings are decided yet: run `bf checks propose` (with `--scope default`, naming each check to switch off and why), show the human the result, and record their answer with `bf checks set --quote \"<their words>\"`, before the plan check".into()];
    }
    let mut out = plan_problems_for(pl, st, &required(st));
    // A named check record belongs to a unit or is listed by a checkpoint, or nothing runs it.
    for c in &pl.checks {
        let named = s(c, "named");
        let design = checks::by_name(named)
            .map(|n| n.stage == Stage::Design)
            .unwrap_or(true);
        if named.is_empty() || design {
            continue;
        }
        let in_unit = pl.units.iter().any(|u| s(u, "id") == s(c, "unit"));
        let listed = pl
            .checkpoints
            .iter()
            .any(|k| str_list(k, "checks").iter().any(|x| x == s(c, "id")));
        if !in_unit && !listed {
            out.push(format!(
                "the check record {} names `{named}` but belongs to no unit and is listed by no checkpoint, so nothing runs it",
                s(c, "id")
            ));
        }
    }
    out
}

/// The same for a given list of required checks. The threat model and the markets statement are
/// proven by their records, so they need no check record; a built-stage check is listed by the last
/// checkpoint, because it counts only at that content.
pub fn plan_problems_for(pl: &Plan, st: &State, req: &[&'static NamedCheck]) -> Vec<String> {
    let _ = st;
    if pl.units.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    for c in req {
        let recs = records_naming(pl, c.name);
        match c.stage {
            Stage::Design if c.name != "design-uiux-verification" => {}
            Stage::Design => {
                if recs.is_empty() {
                    out.push(format!(
                        "the required check `{}` has no check record that names it (add a check record with `named = \"{}\"`, or ask the human to switch the check off)",
                        c.name, c.name
                    ));
                }
            }
            Stage::Built => {
                if recs.is_empty() {
                    out.push(format!(
                        "the required check `{}` has no check record that names it (add a check record with `named = \"{}\"`, or ask the human to switch the check off)",
                        c.name, c.name
                    ));
                } else if let Some(k) = last_checkpoint(pl) {
                    if !recs.iter().any(|r| listed_by_last(pl, r)) {
                        out.push(format!(
                            "the required check `{}` is not listed by the last checkpoint {}: a built-stage check counts only at the content of the last checkpoint (add one of {} to its `checks`)",
                            c.name,
                            s(k, "id"),
                            recs.iter().map(|r| s(r, "id")).collect::<Vec<_>>().join(" or ")
                        ));
                    }
                }
            }
        }
    }
    out
}

/// A passing receipt for a design-stage check. The UI/UX verification counts only for the screens it
/// judged: the receipt carries a hash of the screen records, and it must match them now.
fn design_receipt(p: &Project, pl: &Plan, receipts: &[Receipt], name: &str) -> R<bool> {
    let screens = if name == "design-uiux-verification" {
        Some(evidence::screens_hash(p)?)
    } else {
        None
    };
    Ok(records_naming(pl, name).iter().any(|c| {
        receipts.iter().any(|r| {
            r.check() == s(c, "id")
                && r.passes()
                && screens
                    .as_ref()
                    .map(|h| !h.is_empty() && s(&r.v, "screens_hash") == *h)
                    .unwrap_or(true)
        })
    }))
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
            if !design_receipt(p, &pl, &receipts, "design-uiux-verification")? {
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

/// Whether a built-stage named check has a passing proof: at the content the last checkpoint closed on.
fn built_proof(pl: &Plan, receipts: &[Receipt], st: &State, name: &str) -> bool {
    let Some(k) = last_checkpoint(pl) else {
        return false;
    };
    let kid = s(k, "id");
    let Some(cl) = st.closed.get(kid) else {
        return false;
    };
    records_naming(pl, name)
        .iter()
        .filter(|c| listed_by_last(pl, c))
        .any(|c| {
            if s(c, "how") == "human" {
                st.gate_passed("checkpoint-step", Some(kid))
            } else {
                evidence::satisfied_at(receipts, s(c, "id"), &cl.hash).is_some()
            }
        })
}

fn no_record_or_proof(pl: &Plan, name: &str) -> String {
    let recs = records_naming(pl, name);
    let last = last_checkpoint(pl).map(|k| s(k, "id").to_string());
    if recs.is_empty() {
        format!("{name}: no check record names it")
    } else if !recs.iter().any(|c| listed_by_last(pl, c)) {
        format!(
            "{name}: no check record that names it is listed by the last checkpoint {}",
            last.unwrap_or_default()
        )
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
    refuse_if_unproven_for(p, pl, "close the run", "bf ledger check")
}

/// The same, naming the act that was refused and the command to run again.
pub fn refuse_if_unproven_for(p: &Project, pl: &Plan, act: &str, retry: &str) -> R<Value> {
    let st = crate::events::require_chain(p)?;
    let receipts = evidence::load_receipts(p)?;
    let mut gaps: Vec<String> = Vec::new();
    for c in required_closing(&st) {
        let ok = match c.stage {
            Stage::Design => match c.name {
                "design-uiux-verification" => design_receipt(p, pl, &receipts, c.name)?,
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
        steps.push(format!("Run `{retry}` again."));
        return Err(refuse_owned(
            act.to_string(),
            format!("{} required check(s) have no proof", gaps.len()),
            steps,
            format!("`{retry}` no longer refuses for a missing proof."),
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
    let is_last = last_checkpoint(pl)
        .map(|l| s(l, "id") == kid)
        .unwrap_or(false);
    for c in required_closing(st)
        .iter()
        .filter(|c| c.stage == Stage::Built)
    {
        let recs: Vec<&Value> = records_naming(pl, c.name)
            .into_iter()
            .filter(|r| listed.iter().any(|x| x == s(r, "id")))
            .collect();
        if recs.is_empty() {
            if is_last {
                gaps.push(format!(
                    "the required check `{}` is not listed by this last checkpoint: a built-stage check counts only at its content",
                    c.name
                ));
            }
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
