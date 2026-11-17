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
fn all_naming<'a>(pl: &'a Plan, name: &str) -> Vec<&'a Value> {
    pl.checks.iter().filter(|c| s(c, "named") == name).collect()
}

/// The check records that name this check and are proven the way the catalogue says (a scan is a
/// command, so a record that says a human step or an agent's word settles it does not count).
pub fn records_naming<'a>(pl: &'a Plan, name: &str) -> Vec<&'a Value> {
    all_naming(pl, name)
        .into_iter()
        .filter(|c| {
            checks::by_name(name)
                .map(|n| proven_as_catalogued(n, c))
                .unwrap_or(true)
        })
        .collect()
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

/// The way out that every refusal for a missing proof names: the human may switch the check off for
/// this run.
pub fn switch_off_step(name: &str) -> String {
    format!(
        "Or ask the human to switch `{name}` off for the run: `bf checks propose --scope run {name}=off:<why>`, show them the result, then `bf checks set --quote \"<their words>\"`."
    )
}

/// The steps every refusal at a checkpoint close adds for a required check without proof.
pub fn way_out_steps(st: &State, unproven: &[String]) -> Vec<String> {
    let mut v: Vec<String> = undecided_step(st).into_iter().collect();
    v.extend(unproven.iter().map(|n| switch_off_step(n)));
    v
}

/// Whether the check settings are decided: the human recorded the project default, or the settings of
/// a run. A project whose run began before the engine knew the checks has neither.
pub fn decided(st: &State) -> bool {
    checks::decided(st)
}

/// When the settings are not decided yet, the step that says so (and names the way out for a run).
fn undecided_step(st: &State) -> Option<String> {
    if decided(st) {
        return None;
    }
    Some("No check settings are decided yet (a run begun before the engine knew the checks counts as undecided): ask the human to switch off, for the run, each check that does not apply (`bf checks propose --scope run <check>=off:<why>`, then `bf checks set --quote \"<their words>\"`), or to decide the project default (`--scope default`).".into())
}

/// A check record counts for a named check only when it is proven the way the catalogue says: a scan
/// is a command, so a human step or an agent's word cannot settle it.
fn proven_as_catalogued(c: &NamedCheck, rec: &Value) -> bool {
    s(rec, "how") == c.proof.name()
}

pub fn latest_run_id(st: &State) -> String {
    st.runs.last().map(|r| r.id.clone()).unwrap_or_default()
}

/// Whether a closed checkpoint is judged by the settings it closed under: once a later run is the
/// latest. In the run it closed in, a change of the settings still counts.
pub fn judged_by_close(st: &State, cl: &crate::events::Closed) -> bool {
    cl.recorded && cl.run != latest_run_id(st)
}

/// Whether a check record names a named check that the human switched off (for the last run, which
/// judges the close). Such a record is not demanded.
pub fn is_off(st: &State, rec: &Value) -> bool {
    let n = s(rec, "named");
    !n.is_empty() && checks::off_closing(st).contains_key(n)
}

/// Whether an ID of a checkpoint is proven by one of the checks that prove it: a passing receipt at
/// this content, the human's own gate for a human check, or the human having switched the check off.
/// An ID that only switched-off checks prove is not demanded.
pub fn id_proven(
    st: &State,
    receipts: &[Receipt],
    checks: &[&Value],
    k: &str,
    content: &str,
) -> bool {
    let on: Vec<&&Value> = checks.iter().filter(|c| !is_off(st, c)).collect();
    // Waived only when every check that proves the ID is switched off.
    if on.is_empty() {
        return !checks.is_empty();
    }
    on.iter().any(|c| {
        if s(c, "how") == "human" {
            st.gate_passed("checkpoint-step", Some(k))
        } else {
            evidence::satisfied_at(receipts, s(c, "id"), content).is_some()
        }
    })
}

/// The IDs of a checkpoint that went unproven because every check that proves them is switched off,
/// each with the names of those checks. The close, the run summary and the ledger all use this rule.
pub fn off_unproven(pl: &Plan, st: &State, k: &Value) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for id in str_list(k, "proves") {
        let cs: Vec<&Value> = pl
            .checks
            .iter()
            .filter(|c| {
                str_list(k, "checks").iter().any(|x| x == s(c, "id"))
                    && str_list(c, "proves").contains(&id)
            })
            .collect();
        if !cs.is_empty() && cs.iter().all(|c| is_off(st, c)) {
            let mut names: Vec<String> = cs.iter().map(|c| s(c, "named").to_string()).collect();
            names.sort();
            names.dedup();
            out.push((id, names));
        }
    }
    out
}

/// `off_unproven` for every checkpoint of the plan, as a summary value.
pub fn off_unproven_summary(pl: &Plan, st: &State) -> Vec<Value> {
    // A checkpoint closed in an earlier run is judged by the waiver its close recorded (as the ledger
    // does), and the entry names that run.
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let latest = latest_run_id(st);
    for k in &pl.checkpoints {
        let entries: Vec<(String, Vec<String>, String)> = match st.closed.get(s(k, "id")) {
            Some(cl) if judged_by_close(st, cl) => cl
                .waived
                .iter()
                .map(|(i, n)| (i.clone(), n.clone(), cl.run.clone()))
                .collect(),
            _ => off_unproven(pl, st, k)
                .into_iter()
                .map(|(i, n)| (i, n, latest.clone()))
                .collect(),
        };
        for (id, names, run) in entries {
            if seen.insert(id.clone()) {
                out.push(json!({"id": id, "checks": names, "run": run}));
            }
        }
    }
    out
}

/// What a summary says about IDs left unproven because their checks were off.
pub fn unproven_text(list: &[Value]) -> String {
    list.iter()
        .map(|o| {
            let names: Vec<&str> = o["checks"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            let run = o["run"].as_str().filter(|r| !r.is_empty());
            format!(
                "{} (not proven: {} off{})",
                o["id"].as_str().unwrap_or(""),
                names.join(", "),
                run.map(|r| format!(" in {r}")).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
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
        let mut m = "no check settings are decided yet: run `bf checks propose` (with `--scope default`, naming each check to switch off and why), show the human the result, and record their answer with `bf checks set --quote \"<their words>\"`, before the plan check".to_string();
        if st.active_run().is_some() {
            m.push_str(". A run is active that began before the engine knew the checks: ask the human to switch off, for the run, each check that does not apply (`bf checks propose --scope run <check>=off:<why>`, then `bf checks set --quote \"<their words>\"`)");
        }
        return vec![m];
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
        let all_recs = all_naming(pl, c.name);
        let mut wrong = false;
        for r in &all_recs {
            if !proven_as_catalogued(c, r) {
                wrong = true;
                out.push(format!(
                    "the check record {} names `{}`, which is proven by {} (`how = \"{}\"`), but its `how` is `{}`: a human step or an agent's word cannot settle it (change the record, or ask the human to switch the check off for the run with `bf checks propose --scope run {}=off:<why>`)",
                    s(r, "id"),
                    c.name,
                    c.proof.name(),
                    c.proof.name(),
                    s(r, "how"),
                    c.name
                ));
            }
        }
        let recs: Vec<&Value> = all_recs
            .into_iter()
            .filter(|r| proven_as_catalogued(c, r))
            .collect();
        match c.stage {
            Stage::Design if c.name != "design-uiux-verification" => {}
            Stage::Design => {
                if recs.is_empty() && !wrong {
                    out.push(format!(
                        "the required check `{}` has no check record that names it (add a check record with `named = \"{}\"`, or ask the human to switch the check off for the run with `bf checks propose --scope run {}=off:<why>`)",
                        c.name, c.name, c.name
                    ));
                }
            }
            Stage::Built => {
                if recs.is_empty() {
                    if !wrong {
                        out.push(format!(
                            "the required check `{}` has no check record that names it (add a check record with `named = \"{}\"`, or ask the human to switch the check off for the run with `bf checks propose --scope run {}=off:<why>`)",
                            c.name, c.name, c.name
                        ));
                    }
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

/// A hash of the threat records as they are now: the security expert's report binds to it, so a report
/// stays good while the records stay as they were, whatever the files' times say.
pub fn threats_hash(p: &Project) -> R<String> {
    let mut recs = p.list_records(schema::kind_by_name("threat"))?;
    recs.sort_by(|a, b| s(a, "id").cmp(s(b, "id")));
    let mut text = String::new();
    for r in &recs {
        text.push_str(s(r, "id"));
        text.push(' ');
        text.push_str(&evidence::record_hash(r));
        text.push('\n');
    }
    Ok(crate::util::sha256_hex(text.as_bytes()))
}

/// The threat model is proven when threats exist and the security expert reported at the design stage
/// on the threat records as they are now: the report's event holds a hash of them (`threats_hash`). A
/// report of an older log that holds no hash counts when it came after the last change to any threat
/// record (a report in the same second as a change counts as before it).
pub fn threat_model_proven(p: &Project) -> R<bool> {
    let recs = p.list_records(schema::kind_by_name("threat"))?;
    if recs.is_empty() {
        return Ok(false);
    }
    let now = threats_hash(p)?;
    let mut changed_ms = 0u64;
    for r in &recs {
        let ms = p
            .record_path(s(r, "id"))
            .ok()
            .and_then(|f| std::fs::metadata(f).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        changed_ms = changed_ms.max(ms);
    }
    let log = crate::events::load_log(p)?;
    Ok(log.events.iter().any(|e| {
        e["kind"] == "unit.role-report"
            && e["unit"] == crate::roles::DESIGN
            && e["role"] == "security-expert"
            && e["result"] == "built"
            && match e["threats_hash"].as_str() {
                Some(h) => h == now,
                None => {
                    crate::util::parse_rfc3339(e["at"].as_str().unwrap_or("")) * 1000 > changed_ms
                }
            }
    }))
}

/// Design-stage checks switched on after their gate was approved: the ones whose product is missing,
/// one line each. The gate cannot be opened again by the check alone, so the switch is refused.
pub fn design_product_gaps(p: &Project, st: &State, on: &[&'static NamedCheck]) -> R<Vec<String>> {
    let mut out = Vec::new();
    for c in on.iter().filter(|c| c.stage == Stage::Design) {
        let (gate, missing) = match c.name {
            "design-uiux-verification" => {
                if st.skipped_words("ui").is_some() {
                    continue;
                }
                let pl = plan::load(p)?;
                let receipts = evidence::load_receipts(p)?;
                ("ui", !design_receipt(p, &pl, &receipts, c.name)?)
            }
            "threat-model" => ("architecture", !threat_model_proven(p)?),
            "markets-statement" => ("architecture", count(p, "markets")? == 0),
            _ => continue,
        };
        if missing && st.gate_passed(gate, None) {
            out.push(format!(
                "the design product of `{}` is missing and the `{gate}` gate is already approved",
                c.name
            ));
        }
    }
    Ok(out)
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
            } else if on("threat-model") && !threat_model_proven(p)? {
                gaps.push("the security expert has not reported on the threat model since the last change to a threat record: the expert reads the threats and reports `bf report design --result built --role security-expert --token <token>`".into());
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
    // A check that was switched off when the checkpoint closed is judged by that close.
    if judged_by_close(st, cl) && cl.off.iter().any(|n| n == name) {
        return true;
    }
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
    let mut unproven: Vec<&str> = Vec::new();
    for c in required_closing(&st) {
        let ok = match c.stage {
            Stage::Design => match c.name {
                "design-uiux-verification" => design_receipt(p, pl, &receipts, c.name)?,
                "threat-model" => threat_model_proven(p)?,
                "markets-statement" => count(p, "markets")? > 0,
                _ => true,
            },
            Stage::Built => built_proof(pl, &receipts, &st, c.name),
        };
        if !ok {
            unproven.push(c.name);
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
        steps.extend(undecided_step(&st));
        steps.extend(unproven.iter().map(|n| switch_off_step(n)));
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
) -> R<(Vec<String>, Vec<String>)> {
    let kid = s(k, "id");
    let listed = str_list(k, "checks");
    let mut gaps = Vec::new();
    let mut unproven: Vec<String> = Vec::new();
    // A checkpoint record changed by an amendment the human has not approved is a draft: it closes nothing.
    if let Some(a) = crate::gates::unapproved_change(st, k) {
        gaps.push(format!(
            "{kid}: changed by {a}, which the human has not approved; a draft of the plan is not the record in force"
        ));
    }
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
                unproven.push(c.name.to_string());
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
            unproven.push(c.name.to_string());
            let waits = recs
                .iter()
                .any(|r| evidence::awaits_judgement(receipts, s(r, "id"), content));
            gaps.push(format!(
                "the required check `{}` has no passing receipt at this content from {}{}",
                c.name,
                recs.iter()
                    .map(|r| s(r, "id"))
                    .collect::<Vec<_>>()
                    .join(" or "),
                if waits {
                    " (it passed after a failure at this content: the checkpoint reviewer judges that first)"
                } else {
                    ""
                }
            ));
        }
    }
    for m in crate::legal::unacknowledged(p)? {
        gaps.push(format!("{m}: a marked legal finding is not acknowledged"));
    }
    Ok((gaps, unproven))
}

/// A checkpoint stops for the human or has no stop. A record from an earlier build that says carry on
/// is read as a stop.
pub fn stops(k: &Value) -> bool {
    k.get("stop") == Some(&Value::Bool(true))
        || !s(k, "human_step").is_empty()
        || !s(k, "on_human_step").is_empty()
}
