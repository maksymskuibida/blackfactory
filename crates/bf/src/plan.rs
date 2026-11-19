//! The delivery plan: units, handoffs, checkpoints, waves and readiness.

use crate::events::{self, State};
use crate::evidence;
use crate::gitx;
use crate::refusal::{refuse_owned, R};
use crate::schema;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Plan {
    pub units: Vec<Value>,
    pub handoffs: Vec<Value>,
    pub checkpoints: Vec<Value>,
    pub checks: Vec<Value>,
    /// The plan settings record `PL-1`, when the plan holds one (D-59).
    pub settings: Option<Value>,
    /// True when the plan was approved by an engine that did not yet mark its plan gates with the tests
    /// rule (R-62): such a plan reads as today, every unit running the full suite.
    pub approved_before_tests_rule: bool,
    /// Units of an old plan that count as new for the tests rule: added, or changed in `files`, `text` or
    /// `delivers`, by an amendment the human approved with the marker (R-62, D-59).
    pub new_units: HashSet<String>,
    pub designed: Vec<Value>,
    pub wave: HashMap<String, u32>,
    pub problems: Vec<String>,
    /// Notes that refuse nothing: the plan page shows them.
    pub notes: Vec<String>,
    /// Record files that cannot be read.
    pub bad: Vec<(String, String)>,
}

impl Plan {
    pub fn unit(&self, id: &str) -> Option<&Value> {
        self.units.iter().find(|u| s(u, "id") == id)
    }
    pub fn checkpoint(&self, id: &str) -> Option<&Value> {
        self.checkpoints.iter().find(|u| s(u, "id") == id)
    }
    pub fn waves(&self) -> BTreeMap<u32, Vec<String>> {
        let mut m: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for u in &self.units {
            let id = s(u, "id").to_string();
            if let Some(w) = self.wave.get(&id) {
                m.entry(*w).or_default().push(id);
            }
        }
        m
    }
    /// The limit of heavy checks at once, when the plan states one (D-59).
    pub fn heavy_limit(&self) -> Option<u64> {
        self.settings
            .as_ref()
            .and_then(|v| v.get("heavy_limit"))
            .and_then(|v| v.as_u64())
            .filter(|n| *n >= 1)
    }
    /// Whether this unit must name its `tests` or say why it runs the full suite: the plan is new, or the
    /// unit is new by an approval that carries the marker. Every other unit reads as today.
    pub fn unit_knows_tests(&self, id: &str) -> bool {
        !self.approved_before_tests_rule || self.new_units.contains(id)
    }
    /// Checks attached to a unit by `unit = U`.
    pub fn unit_checks(&self, unit: &str) -> Vec<&Value> {
        self.checks
            .iter()
            .filter(|c| s(c, "unit") == unit)
            .collect()
    }
}

pub fn load(p: &Project) -> R<Plan> {
    let (recs, bad) = p.scan_records(None)?;
    let mut by: HashMap<&str, Vec<Value>> = HashMap::new();
    let mut ids: HashSet<String> = HashSet::new();
    for r in &recs {
        ids.insert(s(r, "id").to_string());
    }
    let ledger_kinds: HashSet<&str> = p.policy.ledger.kinds.iter().map(|x| x.as_str()).collect();
    let mut designed = Vec::new();
    for r in recs.iter() {
        let k = s(r, "kind");
        if ledger_kinds.contains(k) {
            designed.push(r.clone());
        }
    }
    let settings = recs
        .iter()
        .find(|r| s(r, "kind") == "plan_settings")
        .cloned();
    for r in recs {
        let k = match s(&r, "kind") {
            "unit" => "unit",
            "handoff" => "handoff",
            "checkpoint" => "checkpoint",
            "check" => "check",
            _ => continue,
        };
        by.entry(k).or_default().push(r);
    }
    let mut plan = Plan {
        units: by.remove("unit").unwrap_or_default(),
        handoffs: by.remove("handoff").unwrap_or_default(),
        checkpoints: by.remove("checkpoint").unwrap_or_default(),
        checks: by.remove("check").unwrap_or_default(),
        settings,
        approved_before_tests_rule: approved_before_tests_rule(p),
        new_units: new_units_by_marked_approvals(p),
        designed,
        wave: HashMap::new(),
        problems: vec![],
        notes: vec![],
        bad,
    };
    finish(p, &mut plan, &ids);
    Ok(plan)
}

/// The plan gate was approved before gates carried the tests-rule marker, read from the log (F-151).
fn approved_before_tests_rule(p: &Project) -> bool {
    events::replay(p)
        .map(|st| {
            st.gates
                .iter()
                .rfind(|g| g.kind == "plan" && g.status == "approved")
                .map(|g| !g.tests_rule)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// The units that a human-approved amendment with the tests-rule marker added, or changed in `files`,
/// `text` or `delivers`, read from the log: the narrowing's `changes` and the marker of the approval. A
/// change of `depends_on`, `checkpoint` or `estimate` alone never makes a unit new.
fn new_units_by_marked_approvals(p: &Project) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(log) = events::load_log(p) else {
        return out;
    };
    let mut narrowed: HashMap<String, &Value> = HashMap::new();
    for e in &log.events {
        match e["kind"].as_str() {
            Some("amend.narrow") => {
                if let Some(a) = e["amend"].as_str() {
                    narrowed.insert(a.to_string(), e);
                }
            }
            Some("amend.approve") if e["tests_rule"].as_bool() == Some(true) => {
                let Some(n) = e["amend"].as_str().and_then(|a| narrowed.get(a)) else {
                    continue;
                };
                for c in n["changes"].as_array().into_iter().flatten() {
                    let rec = &c["record"];
                    if s(rec, "kind") != "unit" {
                        continue;
                    }
                    let was = &c["was"];
                    let new = match c["how"].as_str() {
                        Some("new") => true,
                        Some("changed") => ["files", "text", "delivers"]
                            .iter()
                            .any(|f| rec.get(*f) != was.get(*f)),
                        _ => false,
                    };
                    if new {
                        out.insert(s(rec, "id").to_string());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The plan problems of a plan the amendment's checkpoint operations would leave: the same check as
/// `load`, on the edited units, handoffs, checkpoints and checks instead of the files (R-64).
pub fn problems_of_edited(
    p: &Project,
    base: &Plan,
    units: Vec<Value>,
    checkpoints: Vec<Value>,
    checks: Vec<Value>,
) -> R<Vec<String>> {
    let mut ids: HashSet<String> = HashSet::new();
    for r in p.list_records(None)? {
        ids.insert(s(&r, "id").to_string());
    }
    for r in units.iter().chain(&checkpoints).chain(&checks) {
        ids.insert(s(r, "id").to_string());
    }
    let mut plan = Plan {
        units,
        handoffs: base.handoffs.clone(),
        checkpoints,
        checks,
        settings: base.settings.clone(),
        approved_before_tests_rule: base.approved_before_tests_rule,
        new_units: base.new_units.clone(),
        designed: base.designed.clone(),
        wave: HashMap::new(),
        problems: vec![],
        notes: vec![],
        bad: base.bad.clone(),
    };
    finish(p, &mut plan, &ids);
    Ok(plan.problems)
}

fn finish(p: &Project, plan: &mut Plan, ids: &HashSet<String>) {
    analyse(p, plan, ids);
    // A required named check appears in the plan as a check record that names it.
    if let Ok(st) = crate::events::replay(p) {
        // Decided means the log says so: a recorded setting for the project default or for a run. A
        // run begun before the engine knew the checks does not decide them.
        let decided = crate::checks::decided(&st);
        let more = crate::enforce::plan_problems(plan, &st, decided);
        plan.problems.extend(more);
        let q = question_problems(plan, &st);
        plan.problems.extend(q);
        let c = close_suite_problems(p, plan, &st);
        plan.problems.extend(c);
    }
}

/// R-62: the full suite runs when the checkpoint closes. A checkpoint that holds a unit naming `tests`
/// lists a command check that runs the suite's command, or it has a problem.
fn close_suite_problems(p: &Project, pl: &Plan, st: &State) -> Vec<String> {
    let mut out = vec![];
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        let Some(u) = pl
            .units
            .iter()
            .find(|u| s(u, "checkpoint") == kid && !s(u, "tests").trim().is_empty())
        else {
            continue;
        };
        let Some(suite) = crate::checks::suite_command(p, pl, st, s(u, "id")) else {
            continue;
        };
        let runs = str_list(k, "checks").iter().any(|cid| {
            pl.checks.iter().any(|c| {
                s(c, "id") == cid && s(c, "how") == "command" && evidence::command_words(c) == suite
            })
        });
        if !runs {
            out.push(crate::strings::t(
                "cli.evidence.plan.close_without_suite",
                &[
                    ("checkpoint", kid),
                    ("unit", s(u, "id")),
                    ("suite", &suite.join(" ")),
                ],
            ));
        }
    }
    out
}

/// The questions every run puts to the human at its plan stage (R-61, R-76): which wave checks it uses,
/// and which checks every unit must pass. An active run that has not answered one has a plan problem, so
/// the plan gate does not open. Before a run starts nothing can be answered, so nothing is asked then.
fn question_problems(pl: &Plan, st: &State) -> Vec<String> {
    let Some(run) = st.active_run() else {
        return vec![];
    };
    if pl.units.is_empty() || st.gate_passed("plan", None) {
        return vec![];
    }
    let mut out = vec![];
    if run.wave.is_none() {
        out.push("the human has not answered the wave-check question for this run: run `bf checks wave question`, ask the human, then `bf checks wave propose` and `bf checks wave set --quote \"<their words>\"`".to_string());
    }
    if run.unit_checks.is_none() {
        out.push("the human has not answered the question about the checks every unit must pass for this run: run `bf checks unit question`, ask the human, then `bf checks unit propose` and `bf checks unit set --quote \"<their words>\"`".to_string());
    }
    out
}

fn analyse(p: &Project, pl: &mut Plan, ids: &HashSet<String>) {
    let mut bad: Vec<String> = Vec::new();
    let unit_ids: HashSet<String> = pl.units.iter().map(|u| s(u, "id").to_string()).collect();
    let cp_ids: HashSet<String> = pl
        .checkpoints
        .iter()
        .map(|u| s(u, "id").to_string())
        .collect();
    let check_ids: HashSet<String> = pl.checks.iter().map(|u| s(u, "id").to_string()).collect();

    // Units: references exist.
    for u in &pl.units {
        let id = s(u, "id");
        for d in str_list(u, "delivers") {
            if !ids.contains(&d) {
                bad.push(format!("{id} delivers {d}, which has no record"));
            }
        }
        for d in str_list(u, "depends_on") {
            if !unit_ids.contains(&d) {
                bad.push(format!("{id} depends on {d}, which is not a unit"));
            }
            if d == id {
                bad.push(format!("{id} depends on itself"));
            }
        }
        if !cp_ids.contains(s(u, "checkpoint")) {
            bad.push(format!(
                "{id} names checkpoint {}, which has no record",
                s(u, "checkpoint")
            ));
        }
    }
    // Cycle detection and waves (Kahn's algorithm on levels).
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    for u in &pl.units {
        let d: Vec<String> = str_list(u, "depends_on")
            .into_iter()
            .filter(|x| unit_ids.contains(x) && x != s(u, "id"))
            .collect();
        deps.insert(s(u, "id").to_string(), d);
    }
    let mut wave: HashMap<String, u32> = HashMap::new();
    let mut remaining: Vec<String> = deps.keys().cloned().collect();
    remaining.sort();
    loop {
        let before = remaining.len();
        let mut next = Vec::new();
        for id in &remaining {
            if deps[id].iter().all(|d| wave.contains_key(d)) {
                let w = deps[id].iter().map(|d| wave[d]).max().unwrap_or(0) + 1;
                wave.insert(id.clone(), w);
            } else {
                next.push(id.clone());
            }
        }
        remaining = next;
        if remaining.is_empty() || remaining.len() == before {
            break;
        }
    }
    if !remaining.is_empty() {
        bad.push(format!(
            "the unit graph has a cycle through {}",
            remaining.join(", ")
        ));
    }
    pl.wave = wave;

    // Handoffs.
    for h in &pl.handoffs {
        let id = s(h, "id");
        let (from, to) = (s(h, "from"), s(h, "to"));
        if !unit_ids.contains(from) {
            bad.push(format!("{id} has no sender: {from} is not a unit"));
        }
        if !unit_ids.contains(to) {
            bad.push(format!("{id} has no receiver: {to} is not a unit"));
        }
        if s(h, "thing").trim().is_empty() {
            bad.push(format!("{id} does not name the thing handed over"));
        }
        if from == to {
            bad.push(format!("{id} hands over from a unit to itself"));
        } else if unit_ids.contains(from)
            && unit_ids.contains(to)
            && !depends_transitively(&deps, to, from)
        {
            bad.push(format!(
                "{id}: {to} receives from {from} but does not depend on it"
            ));
        }
    }
    // Every handoff is proved by some check: what one unit hands to another must get done.
    for h in &pl.handoffs {
        let hid = s(h, "id");
        if !pl
            .checks
            .iter()
            .any(|c| str_list(c, "proves").iter().any(|x| x == hid))
        {
            bad.push(format!(
                "handoff {hid} is proved by no check (add it to the `proves` of a check)"
            ));
        } else if !pl.checks.iter().any(|c| {
            str_list(c, "proves").iter().any(|x| x == hid)
                && (unit_ids.contains(s(c, "unit"))
                    || pl
                        .checkpoints
                        .iter()
                        .any(|k| str_list(k, "checks").iter().any(|x| x == s(c, "id"))))
        }) {
            bad.push(format!(
                "handoff {hid} is proved only by checks that nothing runs: none belongs to a unit (`unit = \"<id>\"`) or is listed by a checkpoint (`checks`)"
            ));
        }
    }
    // Checks.
    for c in &pl.checks {
        let id = s(c, "id");
        for t in str_list(c, "proves") {
            if !ids.contains(&t) {
                bad.push(format!("{id} proves {t}, which has no record"));
            }
        }
        let u = s(c, "unit");
        if !u.is_empty() && !unit_ids.contains(u) {
            bad.push(format!("{id} belongs to {u}, which is not a unit"));
        }
        if s(c, "how") == "command" {
            if let Some(w) = crate::evidence::record_operator_word(c) {
                bad.push(crate::evidence::operator_message(id, &w));
            }
        }
    }
    // Checkpoints.
    let check_proves: HashMap<String, HashSet<String>> = pl
        .checks
        .iter()
        .map(|c| {
            (
                s(c, "id").to_string(),
                str_list(c, "proves").into_iter().collect(),
            )
        })
        .collect();
    let mut proved_by_cp: HashSet<String> = HashSet::new();
    for k in &pl.checkpoints {
        let id = s(k, "id");
        let proves = str_list(k, "proves");
        let cp_checks = str_list(k, "checks");
        if proves.is_empty() {
            bad.push(format!("{id} names no IDs it proves"));
        }
        for t in &proves {
            if !ids.contains(t) {
                bad.push(format!("{id} proves {t}, which has no record"));
            }
            proved_by_cp.insert(t.clone());
            let covered = cp_checks.iter().any(|cid| {
                check_proves
                    .get(cid)
                    .map(|p| p.contains(t))
                    .unwrap_or(false)
            });
            if !covered {
                bad.push(format!("{id} proves {t} but none of its checks proves {t}"));
            }
        }
        for cid in str_list(k, "checks") {
            if !check_ids.contains(&cid) {
                bad.push(format!("{id} lists check {cid}, which has no record"));
            }
        }
        for u in str_list(k, "after") {
            match pl.units.iter().find(|x| s(x, "id") == u) {
                None => bad.push(format!("{id} comes after {u}, which is not a unit")),
                Some(unit) if s(unit, "checkpoint") != id => bad.push(format!(
                    "{id} lists {u}, but {u} belongs to {}",
                    s(unit, "checkpoint")
                )),
                _ => {}
            }
        }
    }
    // Every unit is listed by its checkpoint, and has a check.
    for u in &pl.units {
        let id = s(u, "id");
        if let Some(k) = pl
            .checkpoints
            .iter()
            .find(|k| s(k, "id") == s(u, "checkpoint"))
        {
            if !str_list(k, "after").iter().any(|x| x == id) {
                bad.push(format!("{} does not list {id} in `after`", s(k, "id")));
            }
        }
        if pl.unit_checks(id).is_empty() {
            bad.push(format!(
                "{id} has no check (a check with `unit = \"{id}\"`), so it could not land"
            ));
        }
    }
    let mut notes: Vec<String> = Vec::new();
    heavy_problems(pl, &mut bad, &mut notes);
    pl.notes = notes;
    // Every designed ID belongs to a unit and to a checkpoint.
    let delivered: HashSet<String> = pl
        .units
        .iter()
        .flat_map(|u| str_list(u, "delivers"))
        .collect();
    for d in &pl.designed {
        let id = s(d, "id");
        if !delivered.contains(id) {
            bad.push(format!("{id} belongs to no unit"));
        }
        if !proved_by_cp.contains(id) {
            bad.push(format!("{id} belongs to no checkpoint"));
        }
    }
    // Each unit changes one repository: the one it names, or the only one the workspace has.
    for u in &pl.units {
        let named = s(u, "repo");
        if named.is_empty() && p.repos.len() > 1 {
            bad.push(format!(
                "{} names no repository, and the workspace lists several ({}): add `repo = \"<name>\"`",
                s(u, "id"),
                p.repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ")
            ));
        } else if !named.is_empty() && !p.repos.iter().any(|r| r.name == named) {
            bad.push(format!(
                "{} names the repository `{named}`, which the workspace does not list (`bf repo list`)",
                s(u, "id")
            ));
        } else if p.repos.is_empty() {
            bad.push(format!(
                "{} has no repository: the workspace lists none (`bf repo add <name> <path>`)",
                s(u, "id")
            ));
        }
    }
    // Units in one wave share no files of the same repository.
    let scopes: HashMap<String, Vec<(String, String)>> = pl
        .units
        .iter()
        .map(|u| {
            (
                s(u, "id").to_string(),
                str_list(u, "files")
                    .into_iter()
                    .map(|f| (util::norm_scope(&f), f))
                    .collect(),
            )
        })
        .collect();
    let repo_of = |id: &str| -> String {
        pl.units
            .iter()
            .find(|u| s(u, "id") == id)
            .and_then(|u| p.repo_of_unit(u).ok().map(|r| r.name.clone()))
            .unwrap_or_default()
    };
    for (w, members) in &pl.waves() {
        for i in 0..members.len() {
            for j in i + 1..members.len() {
                if repo_of(&members[i]) != repo_of(&members[j]) {
                    continue;
                }
                for (na, fa) in &scopes[&members[i]] {
                    for (nb, fb) in &scopes[&members[j]] {
                        if util::norm_overlap(na, nb) {
                            bad.push(format!(
                                "wave {w}: {} and {} both touch `{fa}` / `{fb}`",
                                members[i], members[j]
                            ));
                        }
                    }
                }
            }
        }
    }
    for (name, e) in &pl.bad {
        bad.push(format!(
            "record file {name} cannot be read ({e}), so what it holds is not in the plan"
        ));
    }
    pl.problems = bad;
}

fn depends_transitively(deps: &HashMap<String, Vec<String>>, unit: &str, on: &str) -> bool {
    let mut stack = vec![unit.to_string()];
    let mut seen = HashSet::new();
    while let Some(u) = stack.pop() {
        if !seen.insert(u.clone()) {
            continue;
        }
        for d in deps.get(&u).into_iter().flatten() {
            if d == on {
                return true;
            }
            stack.push(d.clone());
        }
    }
    false
}

/// `bf plan check`: refuse while the plan has problems.
pub fn check_or_refuse(pl: &Plan) -> R<()> {
    if pl.problems.is_empty() {
        return Ok(());
    }
    let mut steps: Vec<String> = pl
        .problems
        .iter()
        .take(40)
        .map(|x| format!("Fix: {x}."))
        .collect();
    if pl.problems.len() > 40 {
        steps.push(format!(
            "...and {} more; run `bf plan check --json` for all of them.",
            pl.problems.len() - 40
        ));
    }
    steps.push("Run `bf plan check` again.".into());
    Err(refuse_owned(
        "accept the delivery plan",
        format!("the plan has {} problem(s)", pl.problems.len()),
        steps,
        "`bf plan check` prints `plan ok` and exits 0.",
    ))
}

/// Why a unit cannot start yet, or None when it is ready.
pub fn not_ready(
    p: &Project,
    pl: &Plan,
    st: &State,
    unit: &str,
    receipts: &[evidence::Receipt],
) -> Option<String> {
    let u = pl.unit(unit)?;
    for d in str_list(u, "depends_on") {
        if st.unit_state(&d) != "landed" {
            return Some(format!("it depends on {d}, which has not landed"));
        }
    }
    // An order of the run log (A-73) holds a unit back like a dependency: the asker lands first.
    for o in st.orders.iter().filter(|o| s(o, "unit") == unit) {
        let asker = s(o, "after");
        // `order::after` leaves out an order the records overrule, so no cycle can hold a unit back.
        if st.unit_state(asker) != "landed" && crate::scope::order::after(pl, st, unit, asker) {
            return Some(format!(
                "it comes after {asker}, which has not landed: {asker} asked for {} and the engine put it after (event {})",
                s(o, "path"),
                o["seq"].as_u64().unwrap_or(0)
            ));
        }
    }
    for h in &pl.handoffs {
        if s(h, "to") != unit {
            continue;
        }
        let hid = s(h, "id");
        if st.unit_state(s(h, "from")) != "landed" {
            return Some(format!(
                "handoff {hid} comes from {}, which has not landed",
                s(h, "from")
            ));
        }
        // The sender's own checks that prove the handoff need a sealed passing receipt before the
        // receiver starts. (A checkpoint check runs after the receiver, so it cannot be waited for.)
        for c in pl.checks.iter().filter(|c| {
            str_list(c, "proves").iter().any(|x| x == hid)
                && s(c, "scope") == "unit"
                && s(c, "unit") == s(h, "from")
        }) {
            if !receipts
                .iter()
                .any(|r| r.check() == s(c, "id") && r.passes())
            {
                return Some(format!(
                    "handoff {hid} has no receipt for check {}",
                    s(c, "id")
                ));
            }
        }
    }
    // A checkpoint builds on the one before it in plan order: its first unit waits until that one is
    // closed, so the branch can be cut from it.
    let k = s(u, "checkpoint");
    if !k.is_empty() {
        if let Some(prev) = crate::run::previous_checkpoint(pl, k) {
            if !st.closed.contains_key(&prev)
                && p.repo_of_unit(u)
                    .map(|r| !gitx::branch_exists(&r.path, &p.checkpoint_branch(k)))
                    .unwrap_or(false)
            {
                return Some(format!(
                    "its checkpoint {k} builds on {prev}, which is not closed yet"
                ));
            }
        }
    }
    None
}

/// D-59: a unit that is new by the plan or by a marked approval names its tests or says why it runs the
/// full suite, and a plan that holds a heavy check states the limit. A plan of an older engine is read as today.
fn heavy_problems(pl: &Plan, bad: &mut Vec<String>, notes: &mut Vec<String>) {
    for u in &pl.units {
        if pl.unit_knows_tests(s(u, "id"))
            && u.get("tests").is_none()
            && u.get("full_suite").is_none()
        {
            bad.push(crate::strings::t(
                "cli.evidence.plan.unit_without_tests",
                &[("unit", s(u, "id"))],
            ));
        }
    }
    // A later check of a unit that names no `needs` is told, not refused.
    for u in &pl.units {
        let id = s(u, "id");
        if !pl.unit_knows_tests(id) {
            continue;
        }
        for c in pl.unit_checks(id).into_iter().skip(1) {
            if str_list(c, "needs").is_empty() {
                notes.push(crate::strings::t(
                    "cli.evidence.plan.later_without_needs",
                    &[("check", s(c, "id")), ("unit", id)],
                ));
            }
        }
    }
    // A unit's `tests` is a command in the form a check's command has: no shell operator.
    for u in &pl.units {
        let t = s(u, "tests").trim();
        if !t.is_empty() {
            let probe = serde_json::json!({"command": t});
            if let Some(w) = evidence::record_operator_word(&probe) {
                bad.push(crate::strings::t(
                    "cli.evidence.plan.tests_operator",
                    &[("unit", s(u, "id")), ("word", &w)],
                ));
            }
        }
    }
    let heavy_entry = |v: &Value, list: &str| {
        v.get(list)
            .and_then(|a| a.as_array())
            .map(|a| a.iter().any(|x| x["heavy"].as_bool() == Some(true)))
            .unwrap_or(false)
    };
    let mut heavy: Vec<String> = pl
        .checks
        .iter()
        .filter(|c| c["heavy"].as_bool() == Some(true))
        .map(|c| s(c, "id").to_string())
        .collect();
    heavy.extend(
        pl.units
            .iter()
            .filter(|u| heavy_entry(u, "own_checks"))
            .map(|u| s(u, "id").to_string()),
    );
    heavy.extend(
        pl.checkpoints
            .iter()
            .filter(|k| heavy_entry(k, "unit_checks"))
            .map(|k| s(k, "id").to_string()),
    );
    if !heavy.is_empty() && pl.heavy_limit().is_none() {
        bad.push(crate::strings::t(
            "cli.evidence.plan.heavy_without_limit",
            &[("checks", &heavy.join(", "))],
        ));
    }
}

pub fn designed_kinds() -> Vec<&'static str> {
    schema::KINDS
        .iter()
        .filter(|k| k.designed)
        .map(|k| k.name)
        .collect()
}
