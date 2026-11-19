//! The check catalogue (C-1): the three families of named checks, each with its stage, how it is
//! proven and whether it is required by default. Compiled into the engine (D-16); every other part
//! reads the named checks from here.

use crate::events::{self, CheckChange, State};
use crate::refusal::{refuse, refuse_owned, R};
use crate::store::Project;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub mod review;
pub use review::{kind_records, records_hash, ReviewKind, REVIEW_KINDS};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Uiux,
    Security,
    Legal,
    /// The cold review of a whole checkpoint, settled by the checkpoint reviewer.
    Review,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Uiux => "uiux",
            Family::Security => "security",
            Family::Legal => "legal",
            Family::Review => "review",
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
    c(
        Family::Review,
        CHECKPOINT_REVIEW,
        Stage::Built,
        Proof::Agent,
    ),
    c(Family::Review, IDEA_REVIEW, Stage::Design, Proof::Agent),
    c(
        Family::Review,
        REQUIREMENTS_REVIEW,
        Stage::Design,
        Proof::Agent,
    ),
    c(
        Family::Review,
        ARCHITECTURE_REVIEW,
        Stage::Design,
        Proof::Agent,
    ),
    c(Family::Review, PLAN_REVIEW, Stage::Design, Proof::Agent),
    c(
        Family::Review,
        AMENDMENT_REVIEW,
        Stage::Design,
        Proof::Agent,
    ),
];

/// The named check of the cold review of the idea records before the idea gate opens (D-74).
pub const IDEA_REVIEW: &str = "idea-review";
/// The named check of the cold review of the requirements and journeys.
pub const REQUIREMENTS_REVIEW: &str = "requirements-review";
/// The named check of the cold review of the architecture records.
pub const ARCHITECTURE_REVIEW: &str = "architecture-review";
/// The named check of the cold review of the plan before its gate opens.
pub const PLAN_REVIEW: &str = "plan-review";
/// The named check of the independent review of an amendment (R-98).
pub const AMENDMENT_REVIEW: &str = "amendment-review";

/// The names the catalogue held before the review checks were added. A check setting decided before
/// that holds exactly these: a decision made now records the names it holds in its changes (as `on`
/// for each check added later), and one without them is an old decision. The brief says nine;
/// the catalogue held ten, with the checkpoint review, and the ten are what is listed here.
pub const HELD_BEFORE_REVIEW: [&str; 10] = [
    "design-uiux-verification",
    "built-uiux-verification",
    "accessibility",
    "threat-model",
    "security-audit",
    "secret-scan",
    "dependency-licence-check",
    "markets-statement",
    "legal-audit",
    CHECKPOINT_REVIEW,
];

/// Why a check the catalogue gained after the settings were decided is off.
pub const LATE_REASON: &str =
    "added after the check settings were decided; off until the human switches it on";

/// The named check of the cold review of a whole checkpoint (D-29).
pub const CHECKPOINT_REVIEW: &str = "checkpoint-review";

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

/// A check every unit must pass, as it applies to one unit (R-76).
#[derive(Clone, Debug, PartialEq)]
pub struct Attached {
    pub name: String,
    pub command: Vec<String>,
    /// Where it comes from: `unit`, `checkpoint`, `run` or `project`.
    pub source: &'static str,
    /// The reason of the override that set the list; empty for the project's own list.
    pub reason: String,
    /// `format`, `lint` or `suite` (D-59); empty when the entry does not say.
    pub kind: String,
    /// The entry says it starts devices, containers or a build of the whole product.
    pub heavy: bool,
}

impl Attached {
    /// The check ID receipts are written under.
    pub fn id(&self) -> String {
        format!("{ATTACHED_PREFIX}{}", self.name)
    }
    pub fn command_line(&self) -> String {
        self.command.join(" ")
    }
}

/// The ID of a check that the engine attached to a unit starts with this.
pub const ATTACHED_PREFIX: &str = "attached:";

/// Split a command line into words the way a shell does: single quotes, double quotes and a backslash
/// keep a word together, so `sh -c "a && b"` is three words.
pub fn shell_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => match chars.next() {
                Some(n @ ('"' | '\\' | '$' | '`')) => cur.push(n),
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                }
                None => cur.push('\\'),
            },
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') => {
                started = true;
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => {
                started = true;
                cur.push(c);
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// One entry of a list: name, command words, kind and whether it is heavy.
type Def = (String, Vec<String>, String, bool);

fn defs_from_value(v: Option<&Value>) -> Vec<Def> {
    v.and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .map(|x| {
                    (
                        x["name"].as_str().unwrap_or("").trim().to_string(),
                        shell_words(x["command"].as_str().unwrap_or("")),
                        x["kind"].as_str().unwrap_or("").to_string(),
                        x["heavy"].as_bool() == Some(true),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The kind of an entry of the project's own list, whose policy table carries no `kind`: read from its name.
fn kind_from_name(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("suite") || n == "test" || n == "tests" {
        "suite"
    } else if n.contains("lint") || n.contains("clippy") {
        "lint"
    } else if n.contains("fmt") || n.contains("format") {
        "format"
    } else {
        ""
    }
}

fn attach_list(list: Vec<Def>, source: &'static str, reason: &str) -> Vec<Attached> {
    list.into_iter()
        .map(|(name, command, kind, heavy)| Attached {
            kind: if kind.is_empty() {
                kind_from_name(&name).to_string()
            } else {
                kind
            },
            name,
            command,
            source,
            reason: reason.to_string(),
            heavy,
        })
        .collect()
}

/// The checks that apply to a unit and where the list comes from. The most specific wins: the unit's own
/// list, then its checkpoint's, then the run's, then the project's. Returns the list, its source and the
/// reason of the override (empty for the project's list).
pub fn attached_for_unit(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &State,
    unit: &str,
) -> (Vec<Attached>, &'static str, String) {
    let (mut list, source, reason) = attached_listed(p, pl, st, unit);
    // R-62: a unit that names its tests runs them in the suite's place, and the suite's place in the
    // order stays where it is.
    if let Some(t) = pl
        .unit(unit)
        .map(|u| crate::store::s(u, "tests").trim().to_string())
        .filter(|t| !t.is_empty())
    {
        for a in list.iter_mut().filter(|a| a.kind == "suite") {
            a.command = shell_words(&t);
        }
    }
    (list, source, reason)
}

/// The command of the suite among the checks every unit must pass for this unit, as the list gives it
/// (before a unit's own tests take its place).
pub fn suite_command(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &State,
    unit: &str,
) -> Option<Vec<String>> {
    attached_listed(p, pl, st, unit)
        .0
        .into_iter()
        .find(|a| a.kind == "suite")
        .map(|a| a.command)
}

fn attached_listed(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &State,
    unit: &str,
) -> (Vec<Attached>, &'static str, String) {
    let Some(u) = pl.unit(unit) else {
        return (vec![], "project", String::new());
    };
    if u.get("own_checks").is_some() {
        let reason = crate::store::s(u, "own_checks_reason").to_string();
        return (
            attach_list(defs_from_value(u.get("own_checks")), "unit", &reason),
            "unit",
            reason,
        );
    }
    if let Some(k) = pl.checkpoint(crate::store::s(u, "checkpoint")) {
        if k.get("unit_checks").is_some() {
            let reason = crate::store::s(k, "unit_checks_reason").to_string();
            return (
                attach_list(defs_from_value(k.get("unit_checks")), "checkpoint", &reason),
                "checkpoint",
                reason,
            );
        }
    }
    if let Some(c) = st.runs.last().and_then(|r| r.unit_checks.as_ref()) {
        match c["mode"].as_str().unwrap_or("") {
            "override" => {
                return (
                    attach_list(defs_from_value(c.get("checks")), "run", ""),
                    "run",
                    String::new(),
                );
            }
            "none" => return (vec![], "run", String::new()),
            _ => {}
        }
    }
    let list: Vec<Def> = p
        .policy
        .unit_checks
        .list
        .iter()
        .map(|d| {
            (
                d.name.trim().to_string(),
                shell_words(&d.command),
                kind_from_name(&d.name).to_string(),
                false,
            )
        })
        .collect();
    (attach_list(list, "project", ""), "project", String::new())
}

/// The passing receipt of an attached check at `content`: sealed, for the same command, on that content.
pub fn attached_receipt<'a>(
    receipts: &'a [crate::evidence::Receipt],
    a: &Attached,
    content: &str,
) -> Option<&'a crate::evidence::Receipt> {
    let id = a.id();
    receipts.iter().rev().find(|r| {
        r.check() == id
            && r.content() == content
            && r.passes()
            && crate::store::s(&r.v, "command") == a.command_line()
    })
}

/// What a unit still lacks of the attached checks: one line each, with the command that supplies it.
pub fn attached_missing(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &State,
    receipts: &[crate::evidence::Receipt],
    unit: &str,
    content: &str,
    wt: &std::path::Path,
) -> Vec<String> {
    attached_for_unit(p, pl, st, unit)
        .0
        .iter()
        .filter(|a| attached_receipt(receipts, a, content).is_none())
        .map(|a| {
            format!(
                "{}: {} - run `bf evidence run {} --unit {unit} -- {}` inside {}",
                a.id(),
                if receipts.iter().any(|r| r.check() == a.id() && r.passes()) {
                    "the receipts on file are for other content or another command"
                } else {
                    "no passing sealed receipt"
                },
                a.id(),
                a.command_line(),
                wt.display()
            )
        })
        .collect()
}

/// The attached checks that come before `id` in the unit's list and have no passing receipt on `content`
/// (R-33): the first of them refuses a later check.
pub fn earlier_unpassed(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &State,
    receipts: &[crate::evidence::Receipt],
    unit: &str,
    id: &str,
    content: &str,
) -> Option<Attached> {
    let list = attached_for_unit(p, pl, st, unit).0;
    let at = list.iter().position(|a| a.id() == id)?;
    list.into_iter()
        .take(at)
        .find(|a| attached_receipt(receipts, a, content).is_none())
}

/// Resolve an attached check by its ID for a unit: the command the receipt must carry.
pub fn attached_by_id(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &State,
    unit: &str,
    id: &str,
) -> Option<Attached> {
    attached_for_unit(p, pl, st, unit)
        .0
        .into_iter()
        .find(|a| a.id() == id)
}

/// Lines for a brief or a page: each attached check with its source.
pub fn attached_lines(p: &Project, pl: &crate::plan::Plan, st: &State, unit: &str) -> Vec<String> {
    let (list, source, reason) = attached_for_unit(p, pl, st, unit);
    list.iter()
        .map(|a| {
            format!(
                "{} ({}): `{}`, from {}{}",
                a.id(),
                a.name,
                a.command_line(),
                match source {
                    "unit" => "the unit's own list",
                    "checkpoint" => "the checkpoint's list",
                    "run" => "the run's list",
                    _ => "the project's list",
                },
                if reason.is_empty() {
                    String::new()
                } else {
                    format!(" (override: {reason})")
                }
            )
        })
        .collect()
}

/// Entry point of the list command, which needs no project.
pub fn run(sub: &str, rest: &[String]) -> R<Value> {
    match sub {
        "" | "list" => {
            // Inside a project the list shows what is off and why; elsewhere, the catalogue as it is.
            let settled = std::env::current_dir()
                .ok()
                .and_then(|d| Project::discover(&d).ok())
                .and_then(|p| events::replay(&p).ok());
            Ok(match settled {
                Some(st) => json!({"checks": rows(&off_now(&st), &[])}),
                None => json!({"checks": all().iter().map(to_json).collect::<Vec<_>>()}),
            })
        }
        _ => Err(unknown(sub, rest)),
    }
}

fn unknown(sub: &str, _rest: &[String]) -> crate::refusal::Fail {
    refuse(
        "run `bf checks`",
        format!("unknown subcommand `{sub}`"),
        &["Run `bf checks list`, `propose`, `set`, `state` or `mirror`."],
        "The command prints the checks.",
    )
}

/// The checks the catalogue gained after the settings were decided, which the human has not switched
/// on, with the reason each is off. `run` is the run the settings are read for; empty for the project
/// default. Nothing is late while no setting was decided.
fn late_off(st: &State, run: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if !decided(st) {
        return out;
    }
    let start = st
        .runs
        .iter()
        .find(|r| r.id == run)
        .map(|r| r.start_seq)
        .unwrap_or(0);
    for x in CATALOGUE {
        if HELD_BEFORE_REVIEW.contains(&x.name) {
            continue;
        }
        // The last decision that touches this check for these settings says whether it is on.
        let switched_on = st
            .checks_proposals
            .iter()
            .filter(|p| p.used)
            .filter(|p| {
                let default_part = matches!(p.scope.as_str(), "default" | "both")
                    && (run.is_empty() || p.seq < start || p.run == run);
                let run_part =
                    matches!(p.scope.as_str(), "run" | "both") && !run.is_empty() && p.run == run;
                default_part || run_part
            })
            .filter_map(|p| p.changes.iter().find(|(n, _)| n == x.name))
            .next_back()
            .map(|(_, c)| c.is_none())
            .unwrap_or(false);
        if !switched_on {
            out.insert(x.name.to_string(), LATE_REASON.to_string());
        }
    }
    out
}

/// The switched-off checks of a run (the project default when the run is not known): what the human
/// switched off, and the checks added after the settings were decided that are not switched on yet.
fn effective_off(st: &State, run: &str) -> BTreeMap<String, String> {
    let mut off = late_off(st, run);
    let human = st
        .runs
        .iter()
        .find(|r| r.id == run)
        .map(|r| &r.checks_off)
        .unwrap_or(&st.checks_default);
    for (n, r) in human {
        off.insert(n.clone(), r.clone());
    }
    off
}

/// The switched-off checks of a run, or of the project default for an empty or unknown run ID.
pub fn off_in_run(st: &State, run: &str) -> BTreeMap<String, String> {
    effective_off(st, run)
}

/// Handoff H-2: the named checks a run requires. A run that is not known gets the project default.
pub fn required_in_run(st: &State, run: &str) -> Vec<&'static NamedCheck> {
    let off = effective_off(st, run);
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
pub fn off_closing(st: &State) -> BTreeMap<String, String> {
    effective_off(st, st.runs.last().map(|r| r.id.as_str()).unwrap_or(""))
}

/// The switched-off checks of the active run, or of the project default when no run is active.
pub fn off_now(st: &State) -> BTreeMap<String, String> {
    effective_off(st, st.active_run().map(|r| r.id.as_str()).unwrap_or(""))
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

/// A decision records the names the catalogue holds when it is made: every named check the catalogue
/// gained after the review checks' time and the change does not mention is written as `on`. A decision
/// recorded before that carries none of these and holds only the names of `HELD_BEFORE_REVIEW`, so the
/// checks added later are off for it until the human switches them on.
fn with_catalogue_names(mut changes: Vec<CheckChange>) -> Vec<CheckChange> {
    for x in CATALOGUE {
        if !HELD_BEFORE_REVIEW.contains(&x.name) && !changes.iter().any(|(n, _)| n == x.name) {
            changes.push((x.name.to_string(), None));
        }
    }
    changes
}

fn state_value(st: &State) -> Value {
    let run = st.active_run();
    json!({
        "default": rows(&effective_off(st, ""), &[]),
        "run": run.map(|r| r.id.clone()),
        "run_checks": run.map(|r| rows(&effective_off(st, &r.id), &[])),
        "checks": rows(&off_now(st), &[]),
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
        "" | "list" => match events::replay(p) {
            Ok(st) => Ok(json!({"checks": rows(&off_now(&st), &[])})),
            Err(_) => run(sub, rest),
        },
        // (`run` above serves the same list outside a project.)
        "state" => Ok(state_value(&events::replay(p)?)),
        // The wave checks and the checks every unit must pass: decided with the human at the plan stage.
        "wave" => crate::wavecheck::settings(p, rest, scope, quote),
        "unit" => unit_settings(p, rest, scope, quote),
        // The mirror file holds the project default as the log has it; this writes it again from the
        // log, for a file that is missing or was changed by hand. It records nothing and decides nothing.
        "mirror" => {
            let st = events::require_chain(p)?;
            write_default(p, &st.checks_default)?;
            let mut v = state_value(&st);
            v["mirror"] = json!("factory/checks.json");
            Ok(v)
        }
        "propose" => {
            let st = events::require_chain(p)?;
            let changes = parse_changes(rest)?;
            let scope = pick_scope(&st, scope)?;
            refuse_if_plan_cannot_meet(p, &st, &changes, &scope)?;
            // The decision names every check the catalogue holds now, so none of them counts as added later.
            let changes = with_catalogue_names(changes);
            let ev = events::append(
                p,
                "checks.propose",
                json!({"scope": scope, "changes": events::changes_to_json(&changes),
                    "run": st.active_run().map(|r| r.id.clone())}),
            )?;
            let mut after = if scope == "default" {
                effective_off(&st, "")
            } else {
                off_now(&st)
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
                let mut def = effective_off(&st, "");
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
            let changes = with_catalogue_names(parse_changes(rest)?);
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
                    &[
                        "Start a run with `bf run start`, or propose the project default with `--scope default`.",
                    ],
                    "`bf checks state` shows the checks that are on and off.",
                ));
            }
            let active = st.active_run().map(|r| r.id.clone()).unwrap_or_default();
            if prop.scope != "default" && prop.run != active {
                return Err(refuse(
                    "record check settings for another run",
                    "the proposal was made for a run that is no longer the active one",
                    &[
                        "Run `bf checks propose` again in the active run and show the human the result.",
                    ],
                    "`bf checks state` shows the checks that are on and off.",
                ));
            }
            let q = quote.unwrap_or("");
            // Switching the checkpoint review off ends the hold of every open finding: its words are bound
            // like the other decisions (a hold refuses; otherwise no words before the latest hold's end).
            let mut st = st;
            let mut bound = prop.seq;
            if events::ckreview_required_now(&st)
                && !events::review_required_after(&mut st, &prop.scope, &prop.changes)
            {
                crate::run::observe_holds(p)?;
                st = events::require_chain(p)?;
                let pl = crate::plan::load(p)?;
                let mut open: Vec<String> = Vec::new();
                for k in &pl.checkpoints {
                    let kid = crate::store::s(k, "id");
                    if !st.closed.contains_key(kid)
                        && !crate::ckreview::open_blocking_named(p, &st, kid)?.is_empty()
                    {
                        open.push(kid.to_string());
                    }
                }
                bound = match events::review_off_bound(&st, &open, prop.seq, u64::MAX) {
                    Ok(b) => b,
                    Err((k, h)) => {
                        return Err(crate::ckreview::hold_refusal(
                            "switch the checkpoint review off",
                            &k,
                            &h,
                        ));
                    }
                };
            }
            let (turn, _) = events::pick_turn(&st, bound, events::Decide::Approve, q)?;
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

/// The list the engine proposes for every unit: format and lint before the suite, and a suite command that
/// runs to its end, so one run names every failing test (R-31, R-33, R-62). It is the same whether or not a
/// unit names its tests. The commands are the Rust toolchain's when the project has a manifest, else
/// empty for the human to fill.
fn default_unit_proposal(p: &Project) -> Value {
    let manifest = p.root.join(["Cargo", "toml"].join(".")).is_file();
    let cmd = |c: &'static str| if manifest { c } else { "" };
    json!([
        {"name": "format", "kind": "format", "command": cmd("cargo fmt --check")},
        {"name": "lint", "kind": "lint", "command": cmd("cargo clippy --all-targets -- -D warnings")},
        {"name": "suite", "kind": "suite", "command": cmd("cargo nextest run --no-fail-fast")},
    ])
}

/// `bf checks unit <state|question|propose|set>`: the run's choice about the checks every unit must pass
/// (R-76). The project's own list lives in the policy file, which only the human edits.
fn unit_settings(
    p: &Project,
    rest: &[String],
    scope: Option<&str>,
    quote: Option<&str>,
) -> R<Value> {
    let sub = rest.first().map(|x| x.as_str()).unwrap_or("state");
    match sub {
        "state" | "question" => {
            let st = events::replay(p)?;
            let mut v = json!({
                "project_list": p.policy.unit_checks.list.iter().map(|d| json!({"name": d.name, "command": d.command})).collect::<Vec<_>>(),
                "run": st.runs.last().map(|r| r.id.clone()),
                "run_choice": st.runs.last().and_then(|r| r.unit_checks.clone()),
            });
            if sub == "question" {
                v["question"] = json!({
                    "ask": "Which checks must every unit of this run pass?",
                    "options": [
                        {"choice": "project", "text": "Use the project's list", "shows": v["project_list"]},
                        {"choice": "override", "text": "Override it for this run: give each check as name=command"},
                        {"choice": "none", "text": "No checks for every unit in this run"},
                    ],
                    "note": format!(
                        "{} {}",
                        crate::strings::t("cli.checks.unit.question.note", &[]),
                        crate::strings::t("cli.checks.unit.question.note_tests", &[])
                    ),
                    "proposal": default_unit_proposal(p),
                });
            }
            Ok(v)
        }
        "propose" => {
            let st = events::require_chain(p)?;
            if scope.map(|s| s != "run").unwrap_or(false) {
                return Err(refuse(
                    "propose the checks every unit must pass for another scope",
                    "only the run's choice is made here: the project's list is in factory/policy.toml, which only the human edits",
                    &["Use `--scope run`, or leave the scope out."],
                    "`bf checks unit state` shows the list.",
                ));
            }
            if st.active_run().is_none() {
                return Err(refuse(
                    "propose the checks every unit must pass",
                    "no run is active",
                    &["Start a run with `bf run start`."],
                    "`bf checks unit state` shows the list.",
                ));
            }
            crate::wavecheck::refuse_after_plan_gate(
                &st,
                "the checks every unit must pass",
                "unitchecks",
            )?;
            let mode = rest.get(1).map(|x| x.as_str()).unwrap_or("");
            let value = match mode {
                "project" => json!({"mode": "project", "checks": []}),
                "none" => json!({"mode": "none", "checks": []}),
                "override" => {
                    let mut checks = Vec::new();
                    for a in rest.iter().skip(2) {
                        let Some((n, c)) = a.split_once('=') else {
                            return Err(refuse(
                                "propose the checks every unit must pass",
                                format!("`{a}` is not `<name>=<command>`"),
                                &["Write each check as `lint=cargo clippy -- -D warnings`."],
                                "The command prints the proposal.",
                            ));
                        };
                        if n.trim().is_empty()
                            || c.trim().is_empty()
                            || !n.trim().chars().all(|ch| {
                                ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')
                            })
                        {
                            return Err(refuse(
                                "propose the checks every unit must pass",
                                format!(
                                    "`{a}` needs a name of letters, digits, `-`, `_` or `.` and a command"
                                ),
                                &["Write each check as `lint=cargo clippy -- -D warnings`."],
                                "The command prints the proposal.",
                            ));
                        }
                        if let Some(w) =
                            crate::evidence::unquoted_operator_word(c.trim()).or_else(|| {
                                crate::evidence::shell_operator_word(&shell_words(c.trim()), false)
                            })
                        {
                            return Err(refuse(
                                "propose the checks every unit must pass",
                                crate::evidence::operator_message(
                                    &format!("check `{}`", n.trim()),
                                    &w,
                                ),
                                &[
                                    "Propose one check per program.",
                                    "Or keep an argument with a space whole by quoting it: the engine passes the quoted words as they are.",
                                ],
                                "The command prints the proposal.",
                            ));
                        }
                        checks.push(json!({"name": n.trim(), "command": c.trim()}));
                    }
                    if checks.is_empty() {
                        return Err(refuse(
                            "propose the checks every unit must pass",
                            "an override needs at least one check; use `none` for no checks",
                            &["Write each check as `<name>=<command>`."],
                            "The command prints the proposal.",
                        ));
                    }
                    json!({"mode": "override", "checks": checks})
                }
                _ => {
                    return Err(refuse(
                        "propose the checks every unit must pass",
                        format!("`{mode}` is not `project`, `override` or `none`"),
                        &["Run `bf checks unit question` for the question to ask the human."],
                        "The command prints the proposal.",
                    ));
                }
            };
            crate::wavecheck::propose(p, "unitchecks", "run", value)
        }
        "set" => {
            let _ = crate::wavecheck::confirm(p, "unitchecks", scope, quote)?;
            unit_settings(p, &["state".to_string()], None, None)
        }
        other => Err(refuse(
            "run `bf checks unit`",
            format!("unknown subcommand `{other}`"),
            &["Run `bf checks unit question`, `propose`, `set` or `state`."],
            "The command prints the state.",
        )),
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
            &[
                "Use `--scope run` for this run, `--scope default` for the project default, or `--scope both`.",
            ],
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
    fn a_quoted_argument_stays_one_word() {
        assert_eq!(shell_words("sh -c \"a && b\""), vec!["sh", "-c", "a && b"]);
        assert_eq!(
            shell_words("echo 'x  y' z\\ w"),
            vec!["echo", "x  y", "z w"]
        );
        assert_eq!(shell_words("  cargo   test "), vec!["cargo", "test"]);
        assert_eq!(shell_words("a \"\" b"), vec!["a", "", "b"]);
    }

    #[test]
    fn names_are_unique() {
        for (i, a) in CATALOGUE.iter().enumerate() {
            assert!(CATALOGUE[i + 1..].iter().all(|b| b.name != a.name));
        }
    }
}
