//! The check catalogue (C-1): the three families of named checks, each with its stage, how it is
//! proven and whether it is required by default. Compiled into the engine (D-16); every other part
//! reads the named checks from here.

use crate::events::{self, CheckChange, State};
use crate::refusal::{refuse, refuse_owned, R};
use crate::store::Project;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Uiux,
    Security,
    Legal,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Uiux => "uiux",
            Family::Security => "security",
            Family::Legal => "legal",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Design,
    Built,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Design => "design",
            Stage::Built => "built",
        }
    }
}

/// How a named check is proven; the same words as the `how` field of a check record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proof {
    Command,
    Agent,
    Human,
}

impl Proof {
    pub fn name(self) -> &'static str {
        match self {
            Proof::Command => "command",
            Proof::Agent => "agent",
            Proof::Human => "human",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NamedCheck {
    pub family: Family,
    pub name: &'static str,
    pub stage: Stage,
    pub proof: Proof,
    pub required: bool,
}

const fn c(family: Family, name: &'static str, stage: Stage, proof: Proof) -> NamedCheck {
    NamedCheck {
        family,
        name,
        stage,
        proof,
        required: true,
    }
}

pub static CATALOGUE: &[NamedCheck] = &[
    c(
        Family::Uiux,
        "design-uiux-verification",
        Stage::Design,
        Proof::Agent,
    ),
    c(
        Family::Uiux,
        "built-uiux-verification",
        Stage::Built,
        Proof::Agent,
    ),
    c(Family::Uiux, "accessibility", Stage::Built, Proof::Command),
    c(
        Family::Security,
        "threat-model",
        Stage::Design,
        Proof::Agent,
    ),
    c(
        Family::Security,
        "security-audit",
        Stage::Built,
        Proof::Agent,
    ),
    c(
        Family::Security,
        "secret-scan",
        Stage::Built,
        Proof::Command,
    ),
    c(
        Family::Legal,
        "dependency-licence-check",
        Stage::Built,
        Proof::Command,
    ),
    c(
        Family::Legal,
        "markets-statement",
        Stage::Design,
        Proof::Human,
    ),
    c(Family::Legal, "legal-audit", Stage::Built, Proof::Agent),
];

/// Handoff H-1: every named check with its family, stage and default.
pub fn all() -> &'static [NamedCheck] {
    CATALOGUE
}

pub fn by_name(name: &str) -> Option<&'static NamedCheck> {
    CATALOGUE.iter().find(|x| x.name == name)
}

pub fn in_family(family: Family) -> Vec<&'static NamedCheck> {
    CATALOGUE.iter().filter(|x| x.family == family).collect()
}

/// The checks required on a project that sets nothing.
pub fn required_by_default() -> Vec<&'static NamedCheck> {
    CATALOGUE.iter().filter(|x| x.required).collect()
}

pub fn to_json(x: &NamedCheck) -> Value {
    json!({
        "family": x.family.name(),
        "name": x.name,
        "stage": x.stage.name(),
        "how": x.proof.name(),
        "required": x.required,
    })
}

/// Entry point of the list command, which needs no project.
pub fn run(sub: &str, rest: &[String]) -> R<Value> {
    match sub {
        "" | "list" => Ok(json!({"checks": all().iter().map(to_json).collect::<Vec<_>>()})),
        _ => Err(unknown(sub, rest)),
    }
}

fn unknown(sub: &str, _rest: &[String]) -> crate::refusal::Fail {
    refuse(
        "run `bf checks`",
        format!("unknown subcommand `{sub}`"),
        &["Run `bf checks list`, `propose`, `set` or `state`."],
        "The command prints the checks.",
    )
}

/// Handoff H-2: the named checks a run requires. A run that is not known gets the project default.
pub fn required_in_run(st: &State, run: &str) -> Vec<&'static NamedCheck> {
    let off = st
        .runs
        .iter()
        .find(|r| r.id == run)
        .map(|r| &r.checks_off)
        .unwrap_or(&st.checks_default);
    CATALOGUE
        .iter()
        .filter(|x| x.required && !off.contains_key(x.name))
        .collect()
}

/// The checks required now: those of the active run, or the project default when none is active.
pub fn required_now(st: &State) -> Vec<&'static NamedCheck> {
    match st.active_run() {
        Some(r) => required_in_run(st, &r.id.clone()),
        None => required_in_run(st, ""),
    }
}

/// The checks required when the run is closed: the choice of the last run, also after it was stopped;
/// the project default when no run was ever started.
pub fn required_closing(st: &State) -> Vec<&'static NamedCheck> {
    match st.runs.last() {
        Some(r) => required_in_run(st, &r.id.clone()),
        None => required_in_run(st, ""),
    }
}

/// Whether the check settings are decided: a recorded choice for the project default, or for a run.
/// A run begun before the engine knew the checks has neither, so it counts as undecided.
pub fn decided(st: &State) -> bool {
    st.checks_proposals.iter().any(|x| x.used)
}

/// The switched-off checks of the last run (the close is judged by it), or of the project default.
pub fn off_closing(st: &State) -> &BTreeMap<String, String> {
    st.runs
        .last()
        .map(|r| &r.checks_off)
        .unwrap_or(&st.checks_default)
}

/// The switched-off checks of the active run, or of the project default when no run is active.
pub fn off_now(st: &State) -> &BTreeMap<String, String> {
    st.active_run()
        .map(|r| &r.checks_off)
        .unwrap_or(&st.checks_default)
}

fn rows(off: &BTreeMap<String, String>, changes: &[CheckChange]) -> Vec<Value> {
    all()
        .iter()
        .map(|x| {
            let mut v = to_json(x);
            let reason = off.get(x.name);
            v["required"] = json!(x.required && reason.is_none());
            v["reason"] = json!(reason);
            if let Some((_, c)) = changes.iter().find(|(n, _)| n == x.name) {
                v["change"] = json!(if c.is_some() { "off" } else { "on" });
            }
            v
        })
        .collect()
}

fn parse_changes(rest: &[String]) -> R<Vec<CheckChange>> {
    let mut out: Vec<CheckChange> = vec![];
    for a in rest {
        let bad = |why: String| {
            refuse_owned(
                "change the check settings",
                why,
                vec![
                    "Write each change as `<check>=on` or `<check>=off:<the reason>`.".into(),
                    "Run `bf checks list` to see the names of the checks.".into(),
                ],
                "`bf checks propose` prints the proposal with a reason for each check switched off.",
            )
        };
        let Some((name, what)) = a.split_once('=') else {
            return Err(bad(format!(
                "`{a}` is not `<check>=on` or `<check>=off:<reason>`"
            )));
        };
        if by_name(name).is_none() {
            return Err(bad(format!("`{name}` is not a named check")));
        }
        let change = if what == "on" {
            None
        } else if let Some(r) = what.strip_prefix("off:") {
            if r.trim().is_empty() {
                return Err(bad(format!("`{name}` is switched off with no reason")));
            }
            Some(r.trim().to_string())
        } else {
            return Err(bad(format!(
                "`{a}` is not `<check>=on` or `<check>=off:<reason>`"
            )));
        };
        out.retain(|(n, _)| n != name);
        out.push((name.to_string(), change));
    }
    Ok(out)
}

fn state_value(st: &State) -> Value {
    let run = st.active_run();
    json!({
        "default": rows(&st.checks_default, &[]),
        "run": run.map(|r| r.id.clone()),
        "run_checks": run.map(|r| rows(&r.checks_off, &[])),
        "checks": rows(off_now(st), &[]),
        "scope": if run.is_some() { "run" } else { "default" },
    })
}

fn write_default(p: &Project, off: &BTreeMap<String, String>) -> R<()> {
    let body = serde_json::to_string_pretty(&json!({"off": off}))?;
    crate::util::atomic_write(
        &p.factory().join("checks.json"),
        format!("{body}\n").as_bytes(),
    )?;
    Ok(())
}

/// Entry point of the check settings commands inside a project: list, propose, set, state.
pub fn run_in(
    p: &Project,
    sub: &str,
    rest: &[String],
    scope: Option<&str>,
    quote: Option<&str>,
) -> R<Value> {
    match sub {
        "" | "list" => run(sub, rest),
        "state" => Ok(state_value(&events::replay(p)?)),
        "propose" => {
            let st = events::require_chain(p)?;
            let changes = parse_changes(rest)?;
            let scope = pick_scope(&st, scope)?;
            refuse_if_plan_cannot_meet(p, &st, &changes, &scope)?;
            let ev = events::append(
                p,
                "checks.propose",
                json!({"scope": scope, "changes": events::changes_to_json(&changes),
                    "run": st.active_run().map(|r| r.id.clone())}),
            )?;
            let mut after = if scope == "default" {
                st.checks_default.clone()
            } else {
                off_now(&st).clone()
            };
            events::apply_changes(&mut after, &changes);
            let mut out = json!({
                "proposal": ev["seq"],
                "scope": scope,
                "run": st.active_run().map(|r| r.id.clone()),
                "changed": false,
                "checks": rows(&after, &changes),
            });
            if scope == "both" {
                let mut def = st.checks_default.clone();
                events::apply_changes(&mut def, &changes);
                out["default_checks"] = json!(rows(&def, &changes));
            }
            Ok(out)
        }
        "set" => {
            let st = events::require_chain(p)?;
            let Some(prop) = st.checks_proposals.last().cloned().filter(|x| !x.used) else {
                return Err(refuse(
                    "record the check settings",
                    "no proposal is waiting: none was shown to the human yet, or the last one was already recorded",
                    &[
                        "Run `bf checks propose <check>=off:<reason> ...` and show the human the result.",
                        "Ask the human in chat, then run `bf checks set` with `--quote`.",
                    ],
                    "`bf checks state` shows the checks that are on and off.",
                ));
            };
            let changes = parse_changes(rest)?;
            if (!rest.is_empty() && changes != prop.changes)
                || scope.map(|s| s != prop.scope).unwrap_or(false)
            {
                return Err(refuse(
                    "record check settings that differ from the proposal",
                    "the human saw the latest proposal; the command may record only that one",
                    &[
                        "Run `bf checks propose` again with the changes and the scope you mean.",
                        "Show the human the new proposal and ask again.",
                    ],
                    "`bf checks state` shows the checks that are on and off.",
                ));
            }
            if (prop.scope == "run" || prop.scope == "both") && st.active_run().is_none() {
                return Err(refuse(
                    "record check settings for a run",
                    "no run is active",
                    &["Start a run with `bf run start`, or propose the project default with `--scope default`."],
                    "`bf checks state` shows the checks that are on and off.",
                ));
            }
            let active = st.active_run().map(|r| r.id.clone()).unwrap_or_default();
            if prop.scope != "default" && prop.run != active {
                return Err(refuse(
                    "record check settings for another run",
                    "the proposal was made for a run that is no longer the active one",
                    &["Run `bf checks propose` again in the active run and show the human the result."],
                    "`bf checks state` shows the checks that are on and off.",
                ));
            }
            let q = quote.unwrap_or("");
            let (turn, _) = events::pick_turn(&st, prop.seq, events::Decide::Approve, q)?;
            events::append(
                p,
                "checks.set",
                json!({
                    "scope": prop.scope,
                    "proposal": prop.seq,
                    "turn": turn,
                    "quote": q.trim(),
                    "changes": events::changes_to_json(&prop.changes),
                    "run": st.active_run().map(|r| r.id.clone()),
                }),
            )?;
            let st = events::replay(p)?;
            if prop.scope != "run" {
                write_default(p, &st.checks_default)?;
            }
            Ok(state_value(&st))
        }
        _ => Err(unknown(sub, rest)),
    }
}

/// Switching a check on is refused when the frozen plan cannot meet it.
fn refuse_if_plan_cannot_meet(
    p: &Project,
    st: &State,
    changes: &[CheckChange],
    scope: &str,
) -> R<()> {
    let on: Vec<&'static NamedCheck> = changes
        .iter()
        .filter(|(_, r)| r.is_none())
        .filter_map(|(n, _)| by_name(n))
        .filter(|c| c.required)
        .collect();
    if on.is_empty() {
        return Ok(());
    }
    let mut problems = crate::enforce::design_product_gaps(p, st, &on)?;
    if st.gate_passed("plan", None) {
        let pl = crate::plan::load(p)?;
        problems.extend(crate::enforce::plan_problems_for(&pl, st, &on));
    }
    if problems.is_empty() {
        return Ok(());
    }
    let mut steps: Vec<String> = problems.iter().map(|x| format!("Fix: {x}.")).collect();
    steps.push(format!(
        "The plan is frozen: the human changes it by an amendment, or the check stays off (scope `{scope}`)."
    ));
    Err(refuse_owned(
        "switch a check on",
        format!(
            "the frozen plan cannot meet {} check(s) switched on",
            problems.len()
        ),
        steps,
        "`bf checks state` shows the checks that are on and off.",
    ))
}

fn pick_scope(st: &State, scope: Option<&str>) -> R<String> {
    let s = scope.map(|s| s.to_string()).unwrap_or_else(|| {
        if st.active_run().is_some() {
            "run"
        } else {
            "default"
        }
        .to_string()
    });
    if !matches!(s.as_str(), "run" | "default" | "both") {
        return Err(refuse(
            "propose check settings",
            format!("the scope `{s}` is not `run`, `default` or `both`"),
            &["Use `--scope run` for this run, `--scope default` for the project default, or `--scope both`."],
            "`bf checks propose` prints the proposal.",
        ));
    }
    if s != "default" && st.active_run().is_none() {
        return Err(refuse(
            "propose check settings for a run",
            "no run is active",
            &["Start a run with `bf run start`, or use `--scope default`."],
            "`bf checks propose` prints the proposal.",
        ));
    }
    Ok(s)
}

pub fn text(v: &Value) -> String {
    let head = match (v["proposal"].as_u64(), v["scope"].as_str()) {
        (Some(n), Some(sc)) if v.get("changed").is_some() => {
            format!("proposal {n} for scope {sc}; nothing is changed yet\n")
        }
        _ => String::new(),
    };
    let mut out = head;
    if v.get("default_checks").is_some() {
        out += "project default:\n";
        out += &rows_text(&v["default_checks"]);
        out += "\nthis run:\n";
    }
    out + &rows_text(&v["checks"])
}

fn rows_text(rows: &Value) -> String {
    rows.as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    format!(
                        "{:<9} {:<28} {:<7} {:<8} {}",
                        x["family"].as_str().unwrap_or(""),
                        x["name"].as_str().unwrap_or(""),
                        x["stage"].as_str().unwrap_or(""),
                        x["how"].as_str().unwrap_or(""),
                        match (x["required"] == true, x["reason"].as_str()) {
                            (true, _) => "required".to_string(),
                            (false, Some(r)) => format!("off: {r}"),
                            (false, None) => "optional".to_string(),
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        for (i, a) in CATALOGUE.iter().enumerate() {
            assert!(CATALOGUE[i + 1..].iter().all(|b| b.name != a.name));
        }
    }
}
