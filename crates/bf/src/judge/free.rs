//! The script rules of a free reshuffle (R-73, D-95): pure functions over the plan in force and the plan a
//! narrowing would leave. Nothing here calls them yet; the judge's mode `absent` (U-70) switches them on.
//!
//! The rules go by record IDs and never read a title or a text (Q-11, Q-19): the judge reads those.

use super::{cp_order, longest_run, protected_hit, waves};
use crate::plan::targets;
use crate::store::{s, str_list};
use crate::strings;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The plan records of one side of a reshuffle.
#[derive(Clone, Copy)]
pub struct Slice<'a> {
    pub units: &'a [Value],
    pub handoffs: &'a [Value],
    pub checkpoints: &'a [Value],
    pub checks: &'a [Value],
}

/// Everything the rules read.
pub struct Facts<'a> {
    /// The plan in force.
    pub before: Slice<'a>,
    /// The plan the narrowing would leave.
    pub after: Slice<'a>,
    /// The records the narrowing lists as changed, new or removed, kept ones included.
    pub changes: &'a [Value],
    /// The checkpoint operations of the amendment.
    pub ops: usize,
    /// The records the amendment waives.
    pub waive: &'a [String],
    /// The landed units and the closed checkpoints of the state.
    pub landed: &'a BTreeSet<String>,
    pub closed: &'a BTreeSet<String>,
    /// The IDs of the design records in force: the items a unit may deliver.
    pub design: &'a BTreeSet<String>,
    /// The plan problems the plan after has and the plan in force did not.
    pub new_problems: &'a [String],
}

/// The fields of a unit that decide which checks it runs (finding F-227).
const OWN_FIELDS: [&str; 5] = [
    "tests",
    "full_suite",
    "own_checks",
    "own_checks_reason",
    "repo",
];

fn by_id(list: &[Value]) -> BTreeMap<String, &Value> {
    list.iter().map(|r| (s(r, "id").to_string(), r)).collect()
}

/// An item of a check's `proves` that a unit delivers: a designed part, never a handoff.
fn items_of(rec: &Value, field: &str) -> BTreeSet<String> {
    str_list(rec, field)
        .into_iter()
        .filter(|x| !x.starts_with("H-"))
        .collect()
}

/// The fields in which two records differ, sorted.
fn differing(a: &Value, b: &Value) -> Vec<String> {
    let mut keys: Vec<String> = a
        .as_object()
        .into_iter()
        .chain(b.as_object())
        .flat_map(|o| o.keys().cloned())
        .collect();
    keys.sort();
    keys.dedup();
    keys.into_iter().filter(|k| a.get(k) != b.get(k)).collect()
}

fn join(items: &BTreeSet<String>) -> String {
    items.iter().cloned().collect::<Vec<_>>().join(", ")
}

fn is_unit_check(c: &Value) -> bool {
    s(c, "how") == "command" && s(c, "scope") == "unit"
}

/// What the commands of a new check or a new unit are held against.
struct Ctx {
    /// The shapes of the commands of the unit checks in force.
    shapes: Vec<Vec<String>>,
    /// The test names that a check in force already names.
    named: BTreeSet<String>,
    /// Every path that a unit lists in the plan in force.
    old_paths: BTreeSet<String>,
}

fn context(before: &Slice) -> Ctx {
    let mut shapes = vec![];
    let mut named = BTreeSet::new();
    for c in before.checks {
        if let Some((shape, names)) = targets::command_shape(&crate::evidence::command_words(c)) {
            named.extend(names);
            if is_unit_check(c) {
                shapes.push(shape);
            }
        }
    }
    let old_paths = before
        .units
        .iter()
        .flat_map(|u| str_list(u, "files"))
        .collect();
    Ctx {
        shapes,
        named,
        old_paths,
    }
}

/// Why a command of a new unit check, or the `tests` of a new unit, is refused; None when it passes.
fn command_why(words: &[String], files: &[String], who: &str, ctx: &Ctx) -> Option<String> {
    let Some((shape, names)) = targets::command_shape(words) else {
        return Some(strings::t("judge.free.command.program", &[("who", who)]));
    };
    if names.is_empty() {
        return Some(strings::t("judge.free.command.no_names", &[("who", who)]));
    }
    if !ctx.shapes.contains(&shape) {
        return Some(strings::t("judge.free.command.shape", &[("who", who)]));
    }
    let new_names: BTreeSet<String> = files
        .iter()
        .filter(|f| !ctx.old_paths.contains(*f))
        .filter_map(|f| targets::test_name_of_path(f))
        .collect();
    for n in &names {
        if !new_names.contains(n) && !ctx.named.contains(n) {
            return Some(strings::t(
                "judge.free.command.name",
                &[("who", who), ("name", n)],
            ));
        }
    }
    if !names.iter().any(|n| new_names.contains(n)) {
        return Some(strings::t("judge.free.command.new_file", &[("who", who)]));
    }
    None
}

/// Every refusal of a free reshuffle, by name. Empty when the scripts pass it.
pub fn refusals(f: &Facts) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let (b, a) = (&f.before, &f.after);
    let (bu, au) = (by_id(b.units), by_id(a.units));
    let (bk, ak) = (by_id(b.checkpoints), by_id(a.checkpoints));
    let (bc, ac) = (by_id(b.checks), by_id(a.checks));
    let (bh, ah) = (by_id(b.handoffs), by_id(a.handoffs));
    let ctx = context(b);

    // (a) only units, handoffs, checks and checkpoints change.
    let allowed = |kind: &str| matches!(kind, "unit" | "handoff" | "check" | "checkpoint");
    for c in f.changes {
        let rec = if c["record"].is_null() {
            &c["was"]
        } else {
            &c["record"]
        };
        let kind = s(rec, "kind");
        if !allowed(kind) {
            out.push(strings::t(
                "judge.free.kind",
                &[("id", s(c, "id")), ("kind", kind)],
            ));
        }
    }
    for w in f.waive {
        let kind = crate::schema::kind_of_id(w).map(|k| k.name).unwrap_or("");
        if !allowed(kind) {
            out.push(strings::t("judge.free.waive", &[("id", w)]));
        }
    }

    // (b) the checks in force.
    for (id, was) in &bc {
        let Some(now) = ac.get(id) else {
            out.push(strings::t("judge.free.check_removed", &[("id", id)]));
            continue;
        };
        let unit_check = s(was, "scope") == "unit";
        for field in differing(was, now) {
            if field == "unit" && unit_check {
                continue;
            }
            out.push(strings::t(
                "judge.free.check_field",
                &[("id", id), ("field", &field)],
            ));
        }
        if !unit_check || was.get("unit") == now.get("unit") {
            continue;
        }
        let old_unit = s(was, "unit");
        if au.contains_key(old_unit) {
            out.push(strings::t(
                "judge.free.check_unit_stays",
                &[("id", id), ("unit", old_unit)],
            ));
            continue;
        }
        let new_unit = s(now, "unit");
        let Some(nu) = au.get(new_unit) else {
            out.push(strings::t(
                "judge.free.check_unit_missing",
                &[("id", id), ("unit", new_unit)],
            ));
            continue;
        };
        let lacking: BTreeSet<String> = items_of(was, "proves")
            .difference(&items_of(nu, "delivers"))
            .cloned()
            .collect();
        if !lacking.is_empty() {
            out.push(strings::t(
                "judge.free.check_unit_delivers",
                &[("id", id), ("unit", new_unit), ("items", &join(&lacking))],
            ));
        }
        let was_cp = bu.get(old_unit).map(|u| s(u, "checkpoint")).unwrap_or("");
        if cp_order(s(nu, "checkpoint")) > cp_order(was_cp) {
            out.push(strings::t(
                "judge.free.check_unit_later",
                &[
                    ("id", id),
                    ("unit", new_unit),
                    ("was", was_cp),
                    ("now", s(nu, "checkpoint")),
                ],
            ));
        }
    }
    let new_units: BTreeSet<&String> = au.keys().filter(|k| !bu.contains_key(*k)).collect();
    let gone_units: BTreeSet<&String> = bu.keys().filter(|k| !au.contains_key(*k)).collect();

    // (c) the checks the repair adds.
    for (id, c) in ac.iter().filter(|(id, _)| !bc.contains_key(*id)) {
        let unit = s(c, "unit");
        if !is_unit_check(c) {
            out.push(strings::t("judge.free.check_added_kind", &[("id", id)]));
            continue;
        }
        let Some(u) = au
            .get(unit)
            .filter(|_| new_units.contains(&unit.to_string()))
        else {
            out.push(strings::t(
                "judge.free.check_added_unit",
                &[("id", id), ("unit", unit)],
            ));
            continue;
        };
        let extra: BTreeSet<String> = items_of(c, "proves")
            .difference(&items_of(u, "delivers"))
            .cloned()
            .collect();
        if !extra.is_empty() {
            out.push(strings::t(
                "judge.free.check_added_proves",
                &[("id", id), ("unit", unit), ("items", &join(&extra))],
            ));
        }
        if let Some(why) = command_why(
            &crate::evidence::command_words(c),
            &str_list(u, "files"),
            id,
            &ctx,
        ) {
            out.push(why);
        }
    }

    // (d) the checkpoints.
    if f.ops > 0 {
        out.push(strings::t("judge.free.checkpoint_operation", &[]));
    }
    for id in ak.keys().filter(|k| !bk.contains_key(*k)) {
        out.push(strings::t("judge.free.checkpoint_added", &[("id", id)]));
    }
    for (id, was) in &bk {
        let Some(now) = ak.get(id) else {
            out.push(strings::t("judge.free.checkpoint_removed", &[("id", id)]));
            continue;
        };
        let fields = differing(was, now);
        if f.closed.contains(id) {
            if !fields.is_empty() {
                out.push(strings::t("judge.free.closed_checkpoint", &[("id", id)]));
            }
            continue;
        }
        for field in fields.iter().filter(|x| *x != "after") {
            out.push(strings::t(
                "judge.free.checkpoint_field",
                &[("id", id), ("field", field)],
            ));
        }
        if str_list(now, "after").is_empty() {
            out.push(strings::t("judge.free.checkpoint_empty", &[("id", id)]));
        }
    }

    // (e) a landed unit.
    for id in f.landed {
        if let Some(was) = bu.get(id) {
            if au.get(id) != Some(was) {
                out.push(strings::t("judge.free.landed_unit", &[("id", id)]));
            }
        }
    }

    // (f) the handoffs.
    for id in ah.keys().filter(|k| !bh.contains_key(*k)) {
        out.push(strings::t("judge.free.handoff_added", &[("id", id)]));
    }
    for (id, was) in &bh {
        match ah.get(id) {
            None => out.push(strings::t("judge.free.handoff_removed", &[("id", id)])),
            Some(now) => {
                for field in differing(was, now) {
                    if field != "from" && field != "to" {
                        out.push(strings::t(
                            "judge.free.handoff_field",
                            &[("id", id), ("field", &field)],
                        ));
                    }
                }
            }
        }
    }

    // (g) no new plan problem.
    for x in f.new_problems {
        out.push(strings::t("judge.free.plan_problem", &[("problem", x)]));
    }

    // (h) no unit that depends on a unit of a later checkpoint, unless the plan in force had the pair.
    let later = |units: &BTreeMap<String, &Value>, uid: &str, dep: &str| -> bool {
        match (units.get(uid), units.get(dep)) {
            (Some(u), Some(d)) => cp_order(s(d, "checkpoint")) > cp_order(s(u, "checkpoint")),
            _ => false,
        }
    };
    for (uid, u) in &au {
        for d in str_list(u, "depends_on") {
            if later(&au, uid, &d)
                && !(str_list(bu.get(uid).unwrap_or(&&Value::Null), "depends_on").contains(&d)
                    && later(&bu, uid, &d))
            {
                let (uk, dk) = (
                    s(u, "checkpoint").to_string(),
                    au.get(&d).map(|x| s(x, "checkpoint")).unwrap_or(""),
                );
                out.push(strings::t(
                    "judge.free.later_checkpoint",
                    &[
                        ("unit", uid),
                        ("dep", &d),
                        ("checkpoint", &uk),
                        ("later", dk),
                    ],
                ));
            }
        }
    }

    // (3) the cover.
    let delivered = |units: &BTreeMap<String, &Value>, k: &str| -> BTreeSet<String> {
        units
            .values()
            .filter(|u| s(u, "checkpoint") == k)
            .flat_map(|u| str_list(u, "delivers"))
            .collect()
    };
    for k in bk.keys() {
        let had = delivered(&bu, k);
        let own = delivered(&au, k);
        for item in &had {
            if own.contains(item) {
                continue;
            }
            let brought = au.values().any(|u| {
                let e = s(u, "checkpoint");
                cp_order(e) < cp_order(k)
                    && str_list(u, "delivers").contains(item)
                    && !delivered(&bu, e).contains(item)
            });
            if !brought {
                out.push(strings::t(
                    "judge.free.cover_lost",
                    &[("checkpoint", k), ("item", item)],
                ));
            }
        }
    }
    for (id, now) in &au {
        let Some(was) = bu.get(id) else { continue };
        if cp_order(s(now, "checkpoint")) > cp_order(s(was, "checkpoint")) {
            out.push(strings::t(
                "judge.free.unit_later",
                &[
                    ("unit", id),
                    ("was", s(was, "checkpoint")),
                    ("now", s(now, "checkpoint")),
                ],
            ));
        }
        let now_items = items_of(now, "delivers");
        for c in b
            .checks
            .iter()
            .filter(|c| is_unit_check(c) && s(c, "unit") == id)
        {
            for item in items_of(c, "proves").difference(&now_items) {
                out.push(strings::t(
                    "judge.free.unit_check_item",
                    &[("unit", id), ("item", item), ("check", s(c, "id"))],
                ));
            }
        }
    }
    for (id, u) in &au {
        let changed = bu
            .get(id)
            .is_none_or(|was| was.get("delivers") != u.get("delivers"));
        if !changed {
            continue;
        }
        let items = str_list(u, "delivers");
        if items.is_empty() {
            out.push(strings::t("judge.free.no_delivery", &[("unit", id)]));
        }
        for item in items.iter().filter(|i| !f.design.contains(*i)) {
            out.push(strings::t(
                "judge.free.unknown_item",
                &[("unit", id), ("item", item)],
            ));
        }
    }

    // (4) a unit's own fields and files.
    for (id, now) in &au {
        match bu.get(id) {
            Some(was) => {
                for field in OWN_FIELDS.iter().filter(|x| was.get(**x) != now.get(**x)) {
                    out.push(strings::t(
                        "judge.free.own_field",
                        &[("unit", id), ("field", field)],
                    ));
                }
            }
            None => {
                for field in ["own_checks", "own_checks_reason"] {
                    if now.get(field).is_some() {
                        out.push(strings::t(
                            "judge.free.new_own_field",
                            &[("unit", id), ("field", field)],
                        ));
                    }
                }
                if let Some(t) = now.get("tests").and_then(|t| t.as_str()) {
                    let words: Vec<String> = t.split_whitespace().map(|x| x.to_string()).collect();
                    let who = format!("{id} tests");
                    if let Some(why) = command_why(&words, &str_list(now, "files"), &who, &ctx) {
                        out.push(why);
                    }
                }
            }
        }
    }
    let files_of = |units: &BTreeMap<String, &Value>, id: &str| -> BTreeSet<String> {
        units
            .get(id)
            .map(|u| str_list(u, "files").into_iter().collect())
            .unwrap_or_default()
    };
    let all_ids: BTreeSet<&String> = bu.keys().chain(au.keys()).collect();
    for id in &all_ids {
        let (old, new) = (files_of(&bu, id), files_of(&au, id));
        for path in new.difference(&old) {
            if let Some(q) = protected_hit(path) {
                out.push(strings::t(
                    "judge.free.protected_added",
                    &[("unit", id), ("path", path), ("protected", q)],
                ));
            }
        }
        for path in old.difference(&new) {
            if let Some(q) = protected_hit(path) {
                let key = if gone_units.contains(id) {
                    "judge.free.protected_removed"
                } else {
                    "judge.free.protected_taken"
                };
                out.push(strings::t(
                    key,
                    &[("unit", id), ("path", path), ("protected", q)],
                ));
            }
        }
    }
    // A test file that a check in force names stays with its unit, but for a removed unit's.
    let paths: BTreeSet<String> = bu
        .keys()
        .chain(au.keys())
        .flat_map(|id| files_of(&bu, id).into_iter().chain(files_of(&au, id)))
        .collect();
    for path in &paths {
        let Some(name) = targets::test_name_of_path(path) else {
            continue;
        };
        let named_by: Vec<&Value> = b
            .checks
            .iter()
            .filter(|c| is_unit_check(c))
            .filter(|c| {
                targets::command_shape(&crate::evidence::command_words(c))
                    .is_some_and(|(_, n)| n.contains(&name))
            })
            .collect();
        if named_by.is_empty() {
            continue;
        }
        let holders = |units: &BTreeMap<String, &Value>| -> BTreeSet<String> {
            units
                .keys()
                .filter(|id| files_of(units, id).contains(path))
                .cloned()
                .collect()
        };
        let (was, now) = (holders(&bu), holders(&au));
        for id in now.difference(&was) {
            let from_removed = was.iter().any(|g| {
                gone_units.contains(g)
                    && named_by.iter().any(|c| {
                        s(c, "unit") == g && ac.get(s(c, "id")).is_some_and(|x| s(x, "unit") == id)
                    })
            });
            if !from_removed {
                out.push(strings::t(
                    "judge.free.test_added",
                    &[("unit", id), ("path", path)],
                ));
            }
        }
        for id in was.difference(&now) {
            let with_check = gone_units.contains(id)
                && named_by.iter().any(|c| {
                    s(c, "unit") == id
                        && ac
                            .get(s(c, "id"))
                            .is_some_and(|x| now.contains(s(x, "unit")))
                });
            if !with_check {
                out.push(strings::t(
                    "judge.free.test_taken",
                    &[("unit", id), ("path", path)],
                ));
            }
        }
    }
    out.dedup();
    out
}

/// The measures of a free go, logged and refusing nothing: for each checkpoint the number of waves, the
/// widest wave and the longest run of one-unit waves, before and after; the units whose title or text
/// changed (a new unit counts), and the units removed.
pub fn measures(before: &[Value], after: &[Value], checkpoints: &[String]) -> Value {
    let shape = |units: &[Value], k: &str| -> Value {
        let wave = waves(units);
        let mut per: BTreeMap<u32, u32> = BTreeMap::new();
        for u in units.iter().filter(|u| s(u, "checkpoint") == k) {
            if let Some(w) = wave.get(s(u, "id")) {
                *per.entry(*w).or_default() += 1;
            }
        }
        json!({"waves": per.len(), "widest": per.values().max().copied().unwrap_or(0),
            "chain": longest_run(units, k)})
    };
    let mut cps = serde_json::Map::new();
    for k in checkpoints {
        cps.insert(
            k.clone(),
            json!({"before": shape(before, k), "after": shape(after, k)}),
        );
    }
    let (bu, au) = (by_id(before), by_id(after));
    let reworded: Vec<&String> = au
        .iter()
        .filter(|(id, u)| {
            bu.get(*id)
                .is_none_or(|w| w.get("title") != u.get("title") || w.get("text") != u.get("text"))
        })
        .map(|(id, _)| id)
        .collect();
    let removed: Vec<&String> = bu.keys().filter(|k| !au.contains_key(*k)).collect();
    json!({"checkpoints": cps, "reworded": reworded, "removed": removed})
}
