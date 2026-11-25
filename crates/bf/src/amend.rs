//! The amendment part (C-14): the stage coverage of a design amendment, the refusal at approval, and
//! the data of the amendment page.
//!
//! A design amendment covers each design stage, in order, with its new or changed records or with one
//! sentence that says the stage is not affected. The sentence is a decision record made by the agent
//! (`maker = "agent"`) that names its stage (`stage`) and is named in the amendment's set when the
//! amendment is opened or revised. Approval is refused while a stage has neither, and while a
//! requirement of the amendment has no link to another design part.

use crate::events::{Amend, State};
use crate::refusal::{from_key, refuse_owned, R};
use crate::schema::{self, parse_id};
use crate::store::{s, str_list, Project};
use crate::strings;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

/// The design stages in the order they are filled.
pub const STAGES: &[&str] = &["requirements", "ui", "architecture", "plan"];

/// One stage of an amendment: its records, or the decision that says it is not affected.
#[derive(Clone, Debug)]
pub struct StageCover {
    pub stage: &'static str,
    /// The records of the amendment that belong to this stage (as read; a stub when not yet written).
    pub records: Vec<Value>,
    /// The agent's decision that the stage is not affected, when the amendment holds one.
    pub sentence: Option<Value>,
}

impl StageCover {
    pub fn covered(&self) -> bool {
        !self.records.is_empty() || self.sentence.is_some()
    }
}

/// The stage a record of a kind belongs to; None for kinds that are no part of the design stages.
fn stage_of_kind(id: &str) -> Option<&'static str> {
    match crate::schema::parse_id(id)?.0 {
        "R" | "T" => Some("requirements"),
        "S" | "J" => Some("ui"),
        "C" | "D" | "TH" => Some("architecture"),
        "U" | "H" | "K" | "X" => Some("plan"),
        _ => None,
    }
}

/// The stage a decision speaks for when it is the agent's not affected sentence.
fn sentence_stage(rec: &Value) -> Option<&'static str> {
    if s(rec, "maker") != "agent" {
        return None;
    }
    STAGES.iter().copied().find(|x| *x == s(rec, "stage"))
}

/// Work out the four stages of an amendment from its set and the records on disk.
pub fn coverage(p: &Project, a: &Amend) -> R<Vec<StageCover>> {
    let mut out: Vec<StageCover> = STAGES
        .iter()
        .map(|st| StageCover {
            stage: st,
            records: vec![],
            sentence: None,
        })
        .collect();
    // The records its checkpoint operations change or create belong to the plan stage too.
    let extra: Vec<&String> = a.touched.iter().filter(|t| !a.ids.contains(t)).collect();
    for id in a.ids.iter().chain(extra) {
        let rec = p.get_record(id)?;
        if let Some(r) = rec.as_ref().filter(|r| s(r, "kind") == "decision") {
            if let Some(st) = sentence_stage(r) {
                if let Some(c) = out.iter_mut().find(|c| c.stage == st) {
                    c.sentence = Some(r.clone());
                }
                continue;
            }
        }
        if let Some(st) = stage_of_kind(id) {
            let rec = rec.unwrap_or_else(|| serde_json::json!({"id": id, "title": ""}));
            if let Some(c) = out.iter_mut().find(|c| c.stage == st) {
                c.records.push(rec);
            }
        }
    }
    Ok(out)
}

/// Everything of the amendment's set that differs from the content last approved, and the
/// fingerprint of exactly that. The set is the amendment's own records, the records its operations
/// touch and the kept impact set (`keep`, the one of the narrowing being written or the one in force).
/// A record differs when it is new, when its content is not the approved content, or when the approval
/// has it and it is gone. Each entry holds the record as read now and the approved content it differs from, so the page shows what the
/// fingerprint covers. The fingerprint also covers every record of the set and the operations, so a
/// change of any of them after the narrowing, by any route, voids the human's reading (TH-12).
pub fn narrowing(p: &Project, st: &State, a: &Amend, keep: &[String]) -> R<(Vec<Value>, String)> {
    let mut ids: Vec<&String> = a
        .ids
        .iter()
        .chain(a.touched.iter())
        .chain(keep.iter())
        .collect();
    ids.sort();
    ids.dedup();
    let mut parts = Vec::new();
    for (i, op) in a.ops.iter().enumerate() {
        parts.push(format!("op{i}:{}", crate::evidence::record_hash(op)));
    }
    let mut changes = Vec::new();
    // The records the checkpoint operations remove at the approval: read as gone before it, so the
    // fingerprint the human approves covers the deletion.
    let removed: Vec<String> = if a.ops.is_empty() {
        vec![]
    } else {
        edit_of(p, st, &a.ops)
            .map(|e| e.removed)
            .unwrap_or_default()
    };
    for id in ids {
        let rec = if removed.contains(id) {
            None
        } else {
            p.get_record(id)?
        };
        let h = match &rec {
            Some(r) => crate::evidence::record_hash(r),
            None => "missing".to_string(),
        };
        parts.push(format!("{id}:{h}"));
        let stage = rec
            .as_ref()
            .map(|r| s(r, "kind").to_string())
            .or_else(|| schema::kind_of_id(id).map(|k| k.name.to_string()))
            .and_then(|k| crate::gates::stage_of_kind(&k));
        let approved = stage
            .and_then(|stage| crate::gates::approved_entry(st, stage, id))
            .map(|(h, _)| h);
        let was = stage
            .and_then(|stage| crate::gates::approved_record(st, stage, id))
            .unwrap_or(Value::Null);
        let how = match (&rec, &approved) {
            (Some(_), Some(ah)) if *ah == h => continue,
            (Some(_), Some(ah)) if ah != "missing" => "changed",
            (Some(_), _) => "new",
            (None, Some(ah)) if ah != "missing" => "removed",
            (None, _) => continue,
        };
        let kept = keep.contains(id) && !a.ids.contains(id) && !a.touched.contains(id);
        parts.push(format!(
            "change:{id}:{how}:{h}:{}",
            crate::evidence::record_hash(&was)
        ));
        changes.push(json!({"id": id, "how": how, "kept": kept,
            "record": rec.unwrap_or(Value::Null), "was": was}));
    }
    Ok((changes, crate::evidence::record_hash(&json!(parts))))
}

/// Refuse approval when a record of the set changed after the narrowing the human read.
pub fn refuse_if_changed_since_narrowing(p: &Project, a: &Amend, narrowed: &Value) -> R<()> {
    let Some(was) = narrowed["set_hash"].as_str() else {
        return Ok(());
    };
    let st = crate::events::replay(p)?;
    if was == narrowing(p, &st, a, &a.keep)?.1 {
        return Ok(());
    }
    let aid = &a.id;
    Err(refuse_owned(
        format!("approve {aid}"),
        "a record of the amendment was put or replaced after it was narrowed, so the human has not read it".to_string(),
        vec![
            format!("Run `bf amend revise <the IDs it names now> plus every record that changed` on {aid} (revise replaces the whole set; a kept record that must change is named there)."),
            format!("Narrow {aid} again with `bf amend narrow --file <f>`."),
            "Ask the human to read it and say yes after that.".to_string(),
        ],
        "`bf amend approve` prints the approved amendment.",
    ))
}

/// Refuse while an ID the amendment names has no record. A new record is put while the amendment waits
/// for the human, so the human reads it before the approval (R-43). `stem` is the key stem of the
/// refusal of the command that asks.
pub fn refuse_if_names_missing(p: &Project, a: &Amend, stem: &str) -> R<()> {
    let mut missing: Vec<String> = Vec::new();
    let mut st: Option<State> = None;
    for id in &a.ids {
        if p.get_record(id)?.is_none() {
            // A unit that was in force and is gone is a removal (`bf record remove`): the narrowing
            // lists it as removed with its old content, and the approval records it as gone (R-73).
            if schema::kind_of_id(id).is_some_and(|k| k.name == "unit") {
                if st.is_none() {
                    st = Some(crate::events::replay(p)?);
                }
                let in_force = st
                    .as_ref()
                    .and_then(|st| crate::gates::approved_entry(st, "plan", id))
                    .is_some_and(|(h, _)| h != "missing");
                if in_force {
                    continue;
                }
            }
            missing.push(id.clone());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(from_key(
        stem,
        &[("amend", &a.id), ("ids", &missing.join(", "))],
    ))
}

/// What a withdrawal does to one record.
#[derive(Clone, Debug)]
pub enum Back {
    /// Write this content, the content last approved.
    Restore(Value),
    /// Take the record out of the records in force: the amendment added it.
    Remove,
}

/// The records a withdrawal puts back, each with what is done to it. The content comes from the log by
/// the hash it holds for the last approval of the record. Nothing is written here: when the content of
/// any record cannot be found, the withdrawal is refused naming each such record (R-44).
pub fn withdraw_plan(p: &Project, st: &State, a: &Amend) -> R<Vec<(String, Back)>> {
    let mut ids: Vec<&String> = a.ids.iter().chain(a.touched.iter()).collect();
    ids.sort();
    ids.dedup();
    let mut out = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    for id in ids {
        let cur = p.get_record(id)?;
        let stage = cur
            .as_ref()
            .map(|r| s(r, "kind").to_string())
            .or_else(|| schema::kind_of_id(id).map(|k| k.name.to_string()))
            .and_then(|k| crate::gates::stage_of_kind(&k));
        let Some(stage) = stage else { continue };
        if !st
            .latest_gate(stage, None)
            .map(|g| g.status == "approved")
            .unwrap_or(false)
        {
            continue;
        }
        match crate::gates::approved_entry(st, stage, id) {
            Some((h, _)) if h != "missing" => {
                let found = crate::gates::approved_record(st, stage, id)
                    .filter(|r| crate::evidence::record_hash(r) == h);
                match (found, &cur) {
                    (None, _) => lost.push(id.clone()),
                    (Some(r), Some(c)) if *c == r => {}
                    (Some(r), _) => out.push((id.clone(), Back::Restore(r))),
                }
            }
            _ => {
                if cur.is_some() {
                    out.push((id.clone(), Back::Remove));
                }
            }
        }
    }
    if !lost.is_empty() {
        return Err(from_key(
            "cli.amend.withdraw_lost",
            &[("amend", &a.id), ("ids", &lost.join(", "))],
        ));
    }
    Ok(out)
}

/// Do the plan of `withdraw_plan`, all or none. Returns what was on disk before, for `undo_writes`.
pub fn apply_withdraw(
    p: &Project,
    plan: &[(String, Back)],
) -> R<Vec<(std::path::PathBuf, Option<Vec<u8>>)>> {
    let mut before: Vec<(std::path::PathBuf, Option<Vec<u8>>)> = vec![];
    for (id, _) in plan {
        let path = p.record_path(id)?;
        before.push((path.clone(), std::fs::read(&path).ok()));
    }
    for (id, how) in plan {
        let done = match how {
            Back::Restore(r) => p.write_record(r),
            Back::Remove => p
                .record_path(id)
                .and_then(|f| std::fs::remove_file(f).map_err(Into::into)),
        };
        if let Err(e) = done {
            undo_writes(&before);
            return Err(e);
        }
    }
    Ok(before)
}

/// The text of a finished withdrawal: its line, then each record put back.
pub fn withdraw_text(v: &Value) -> String {
    let aid = s(v, "amend");
    let mut lines = vec![strings::t(
        "cli.amend.withdraw.line",
        &[("amend", aid), ("reason", s(v, "reason"))],
    )];
    let back = v["put_back"].as_array().cloned().unwrap_or_default();
    if back.is_empty() {
        lines.push(strings::t("cli.amend.withdraw.nothing", &[("amend", aid)]));
    }
    for b in &back {
        let key = if s(b, "how") == "removed" {
            "cli.amend.withdraw.removed"
        } else {
            "cli.amend.withdraw.restored"
        };
        lines.push(strings::t(key, &[("id", s(b, "id")), ("amend", aid)]));
    }
    lines.join("\n")
}

/// The longest line the log view prints for one event (R-52).
pub const EVENT_LINE_MAX: usize = 2000;

/// An event as the log view prints it: an amendment's narrowing and approval events carry the IDs of
/// their records, under the label of `cli.log.records`, where they hold the records' contents. The bool
/// says whether contents were left out. The log itself is never changed.
pub fn shorten_event(e: &Value) -> (Value, bool) {
    // The judge's verdict shows its verdict, model and reasons and the record IDs it names, and nothing
    // else: not the measures, the fingerprint, the session or the agent (S-26).
    if s(e, "kind") == "amend.judge" {
        let mut m = serde_json::Map::new();
        for k in [
            "seq", "at", "kind", "amend", "verdict", "model", "reasons", "records",
        ] {
            if let Some(v) = e.get(k) {
                m.insert(k.to_string(), v.clone());
            }
        }
        return (Value::Object(m), true);
    }
    let drop: &[&str] = match s(e, "kind") {
        "amend.narrow" => &["changes"],
        "amend.approve" => &["content"],
        "amend.withdraw" => &["taken_away"],
        _ => return (e.clone(), false),
    };
    let mut m = e.clone();
    let mut ids: Vec<String> = Vec::new();
    if let Some(o) = m.as_object_mut() {
        for k in drop {
            match o.remove(*k) {
                Some(Value::Object(c)) => ids.extend(c.keys().cloned()),
                Some(Value::Array(c)) => {
                    ids.extend(c.iter().map(|x| s(x, "id").to_string()));
                }
                _ => {}
            }
        }
        let label = strings::t("cli.log.records", &[]);
        o.insert(label, json!(ids));
    }
    (m, true)
}

/// The line the log view and the recent events view print for an event: its JSON on one line. A
/// shortened event ends with the mark of `cli.log.contents_left_out`, and the whole line, mark included,
/// is at most `EVENT_LINE_MAX` bytes: the longest fields go to their count, then the line is cut (R-52).
pub fn event_line(m: &Value, cut: bool) -> String {
    let mark = if cut {
        format!(" {}", strings::t("cli.log.contents_left_out", &[]))
    } else {
        String::new()
    };
    let line = serde_json::to_string(m).unwrap_or_default();
    if !cut || line.len() + mark.len() <= EVENT_LINE_MAX {
        return format!("{line}{mark}");
    }
    let room = EVENT_LINE_MAX - mark.len();
    let mut m = m.clone();
    if let Some(o) = m.as_object_mut() {
        let mut keys: Vec<(usize, String)> = o
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "seq" | "at" | "kind" | "amend"))
            .map(|(k, v)| (v.to_string().len(), k.clone()))
            .collect();
        keys.sort();
        keys.reverse();
        for (_, k) in keys {
            if serde_json::to_string(&o).map(|x| x.len()).unwrap_or(0) <= room {
                break;
            }
            let n = match &o[&k] {
                Value::Array(a) => a.len(),
                Value::Object(c) => c.len(),
                Value::String(x) => x.chars().count(),
                _ => continue,
            };
            o.insert(k, json!(n));
        }
    }
    let mut line = serde_json::to_string(&m).unwrap_or_default();
    if line.len() > room {
        let mut end = room;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    format!("{line}{mark}")
}

/// The requirements of the amendment that have no link to another design part (their own `links`,
/// or a design part that links to them).
pub fn unlinked_requirements(p: &Project, a: &Amend) -> R<Vec<String>> {
    let all = p.list_records(None)?;
    let mut bad = Vec::new();
    for id in a.ids.iter().filter(|i| i.starts_with("R-")) {
        let linked = all.iter().any(|r| {
            let rid = s(r, "id");
            let designed = crate::schema::is_designed_id(rid);
            if rid == id {
                !str_list(r, "links").is_empty()
            } else {
                designed && str_list(r, "links").contains(id)
            }
        });
        if !linked {
            bad.push(id.clone());
        }
    }
    Ok(bad)
}

/// Refuse approval while a stage is uncovered or a requirement has no links. The refusal names each.
pub fn refuse_if_uncovered(p: &Project, a: &Amend) -> R<()> {
    refuse_if_unreviewed(p, a)?;
    let missing: Vec<&'static str> = coverage(p, a)?
        .iter()
        .filter(|c| !c.covered())
        .map(|c| c.stage)
        .collect();
    let unlinked = unlinked_requirements(p, a)?;
    if missing.is_empty() && unlinked.is_empty() {
        return Ok(());
    }
    let aid = &a.id;
    let mut because = Vec::new();
    let mut steps = Vec::new();
    if !missing.is_empty() {
        because.push(format!(
            "the stage(s) {} have neither records nor a not affected sentence",
            missing.join(", ")
        ));
        for st in &missing {
            steps.push(format!(
                "For {st}: put its new or changed records and run `bf amend revise` with them, or put a decision record with maker = \"agent\" and stage = \"{st}\" that says in one sentence why it is not affected, and name it in `bf amend revise`."
            ));
        }
    }
    for r in &unlinked {
        because.push(format!(
            "the requirement {r} has no link to another design part"
        ));
        steps.push(format!(
            "Give {r} a link in `links`, or link a design part to it."
        ));
    }
    steps.push(format!(
        "Narrow {aid} again with `bf amend narrow --file <f>`; the human reads it after its last change."
    ));
    Err(refuse_owned(
        format!("approve {aid}"),
        because.join("; "),
        steps,
        "`bf amend approve` prints the approved amendment.",
    ))
}

/// Refuse approval while the amendment review holds it (R-98): a review the shape needs is missing
/// at the current records, or an open critical, high or medium finding remains. A plan repair and an
/// amendment whose review check is off are not held. The refusal names each.
pub fn refuse_if_unreviewed(p: &Project, a: &Amend) -> R<()> {
    if is_repair(a) {
        return Ok(());
    }
    let st = crate::events::replay(p)?;
    let lines = crate::checks::review::amendment_blocker(p, &st, &a.id)?;
    if lines.is_empty() {
        return Ok(());
    }
    let aid = &a.id;
    Err(refuse_owned(
        format!("approve {aid}"),
        format!("the amendment review holds it: {}", lines.join("; ")),
        vec![
            format!("Have the missing reviews run on the records of {aid} as they are now (`bf review token amendment --subject {aid}`)."),
            "Fix each open finding, or ask the human to accept it in their own words: `bf review finding <A-n> --accept <F-ids> --quote \"<their words>\"`.".to_string(),
        ],
        "`bf amend approve` prints the approved amendment.",
    ))
}

// ---- the plan repair (R-41) ----

/// The IDs of a set that name a design part: requirement, screen, journey, component, threat,
/// target or decision.
pub fn designed_in(ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|i| schema::is_designed_id(i))
        .cloned()
        .collect()
}

/// An amendment is a plan repair when it was opened with the mark, or when it holds checkpoint
/// operations and no design part (R-64).
pub fn is_repair(a: &Amend) -> bool {
    a.repair || (!a.ops.is_empty() && designed_in(&a.ids).is_empty())
}

/// Refuse the repair mark when the set holds a design part. The refusal names each one.
pub fn refuse_repair_mark(what: &str, ids: &[String]) -> R<()> {
    let bad = designed_in(ids);
    if bad.is_empty() {
        return Ok(());
    }
    let named: Vec<String> = bad
        .iter()
        .map(|i| {
            let k = schema::kind_of_id(i).map(|k| k.name).unwrap_or("part");
            format!("{i} ({k})")
        })
        .collect();
    Err(refuse_owned(
        what.to_string(),
        format!(
            "a plan repair covers only a check's command, a unit's file list or order, or moving a unit between checkpoints, and the set holds {}",
            named.join(", ")
        ),
        vec![
            format!("Leave {} out of the set, or open it without `--repair` as a design amendment that covers each stage.", bad.join(", ")),
        ],
        "`bf status` lists the amendment as a repair.",
    ))
}

// ---- checkpoint insertion and split (R-64) ----

fn num(id: &str) -> u32 {
    parse_id(id).map(|x| x.1).unwrap_or(0)
}

/// The plan as the checkpoint operations of an amendment would leave it, in memory. Nothing is written
/// until the human approves the amendment.
pub struct Edit {
    pub units: Vec<Value>,
    pub handoffs: Vec<Value>,
    pub cps: Vec<Value>,
    pub checks: Vec<Value>,
    /// Each renamed checkpoint: its number on disk and its number after the amendment.
    pub renames: Vec<(String, String)>,
    /// The checkpoints made by the operations.
    pub created: Vec<String>,
    /// The checkpoint records the operations leave no place for: approval removes their files.
    pub removed: Vec<String>,
    disk: BTreeMap<String, Value>,
}

impl Edit {
    pub fn load(p: &Project) -> R<Edit> {
        let get = |n: &str| p.list_records(schema::kind_by_name(n));
        let (units, handoffs, cps, checks) = (
            get("unit")?,
            get("handoff")?,
            get("checkpoint")?,
            get("check")?,
        );
        let disk = units
            .iter()
            .chain(&cps)
            .chain(&checks)
            .map(|r| (s(r, "id").to_string(), r.clone()))
            .collect();
        Ok(Edit {
            units,
            handoffs,
            cps,
            checks,
            renames: vec![],
            created: vec![],
            removed: vec![],
            disk,
        })
    }

    /// The records that differ from the files now, or have none: the ones approval writes.
    pub fn changed(&self) -> Vec<Value> {
        self.units
            .iter()
            .chain(&self.cps)
            .chain(&self.checks)
            .filter(|r| self.disk.get(s(r, "id")) != Some(*r))
            .cloned()
            .collect()
    }

    pub fn touched(&self) -> Vec<String> {
        self.changed()
            .iter()
            .map(|r| s(r, "id").to_string())
            .chain(self.removed.iter().cloned())
            .collect()
    }

    /// The number a checkpoint has on disk now; None for one the operations made.
    fn orig_of(&self, cur: &str) -> Option<String> {
        if self.created.iter().any(|c| c == cur) {
            return None;
        }
        Some(
            self.renames
                .iter()
                .find(|(_, n)| n == cur)
                .map(|(o, _)| o.clone())
                .unwrap_or_else(|| cur.to_string()),
        )
    }
}

/// Whether `from` depends on `to`, directly or through other units.
fn reaches(units: &[Value], from: &str, to: &str) -> bool {
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack = vec![from.to_string()];
    while let Some(x) = stack.pop() {
        if !seen.insert(x.clone()) {
            continue;
        }
        if let Some(u) = units.iter().find(|u| s(u, "id") == x) {
            for d in str_list(u, "depends_on") {
                if d == to {
                    return true;
                }
                stack.push(d);
            }
        }
    }
    false
}

/// The request of a checkpoint operation, as it is kept in the log.
pub fn op_from_args(pos: &[String], title: Option<&str>) -> R<Value> {
    let how = pos.first().map(|x| x.as_str()).unwrap_or("");
    let ok = match how {
        "before" | "after" | "split" => pos.len() >= 3,
        "merge" => pos.len() == 3,
        _ => false,
    };
    if !ok {
        return Err(from_key("cli.amend_merge.usage", &[]));
    }
    let mut op = if how == "merge" {
        json!({"how": how, "at": pos[1], "with": pos[2]})
    } else {
        let mut units: Vec<String> = vec![];
        for u in &pos[2..] {
            if !units.contains(u) {
                units.push(u.clone());
            }
        }
        json!({"how": how, "at": pos[1], "units": units})
    };
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        op["title"] = json!(t.trim());
    }
    Ok(op)
}

/// Merge the checkpoint right after `at` into it: the units and the proven parts of both, the later
/// checkpoints one number lower, and the record of the highest number left without a place.
fn apply_merge(st: &State, e: &mut Edit, op: &Value) -> R<()> {
    let (a, b) = (s(op, "at").to_string(), s(op, "with").to_string());
    let vals = [("a", a.as_str()), ("b", b.as_str())];
    let find = |e: &Edit, id: &str| e.cps.iter().find(|k| s(k, "id") == id).cloned();
    let (Some(ka), Some(kb)) = (find(e, &a), find(e, &b)) else {
        return Err(from_key("cli.amend_merge.unknown", &vals));
    };
    if num(&b) != num(&a) + 1 {
        return Err(from_key("cli.amend_merge.not_next", &vals));
    }
    let units_of = |e: &Edit, k: &Value| -> Vec<String> {
        let mut v = str_list(k, "after");
        for u in e.units.iter().filter(|u| s(u, "checkpoint") == s(k, "id")) {
            let id = s(u, "id").to_string();
            if !v.contains(&id) {
                v.push(id);
            }
        }
        v
    };
    let (ua, ub) = (units_of(e, &ka), units_of(e, &kb));
    let started = |e: &Edit, id: &str, us: &[String]| {
        us.iter().any(|u| st.units.contains_key(u))
            || e.orig_of(id)
                .is_some_and(|o| st.closed.contains_key(&o) || st.synced.iter().any(|x| x.0 == o))
    };
    if started(e, &b, &ub) {
        return Err(from_key("cli.amend_merge.started", &vals));
    }
    if e.orig_of(&a).is_some_and(|o| st.closed.contains_key(&o)) {
        return Err(from_key("cli.amend_merge.closed", &vals));
    }
    let (ca, cb) = (ka.get("unit_checks"), kb.get("unit_checks"));
    if let (Some(x), Some(y)) = (ca, cb) {
        if x != y {
            return Err(from_key(
                "cli.amend_merge.unit_checks",
                &[
                    ("a", &a),
                    ("b", &b),
                    ("list_a", &x.to_string()),
                    ("list_b", &y.to_string()),
                ],
            ));
        }
    }
    for k in e.cps.iter().filter(|k| num(s(k, "id")) > num(&b)) {
        let id = s(k, "id");
        let us = units_of(e, k);
        if started(e, id, &us) {
            return Err(from_key(
                "cli.amend_merge.later_started",
                &[("a", &a), ("b", &b), ("id", id)],
            ));
        }
    }
    // The merged checkpoint.
    let union = |f: &str| {
        let mut v = str_list(&ka, f);
        for x in str_list(&kb, f) {
            if !v.contains(&x) {
                v.push(x);
            }
        }
        v
    };
    let mut k = ka.clone();
    let mut after = ua.clone();
    after.extend(ub.iter().filter(|u| !ua.contains(u)).cloned());
    k["after"] = json!(after);
    k["proves"] = json!(union("proves"));
    k["checks"] = json!(union("checks"));
    for f in ["stop", "pull_request"] {
        let v = ka[f].as_bool().unwrap_or(false) || kb[f].as_bool().unwrap_or(false);
        if v || ka.get(f).is_some() {
            k[f] = json!(v);
        }
    }
    for f in ["stop_why", "human_step"] {
        let (x, y) = (s(&ka, f).to_string(), s(&kb, f).to_string());
        let v = if x.is_empty() {
            y
        } else if y.is_empty() || x == y {
            x
        } else {
            format!("{x} {y}")
        };
        if !v.is_empty() {
            k[f] = json!(v);
        }
    }
    for f in ["on_human_step", "unit_checks", "unit_checks_reason"] {
        if ka.get(f).is_none() {
            if let Some(v) = kb.get(f) {
                k[f] = v.clone();
            }
        }
    }
    if !s(op, "title").is_empty() {
        k["title"] = json!(s(op, "title"));
    }
    let (x, y) = (s(&ka, "text").to_string(), s(&kb, "text").to_string());
    if !y.is_empty() && x != y {
        k["text"] = json!(if x.is_empty() {
            y
        } else {
            format!("{x}\n\n{y}")
        });
    }
    // The units of the later checkpoint go onto this one, keeping their order among themselves.
    for u in e
        .units
        .iter_mut()
        .filter(|u| ub.iter().any(|x| x == s(u, "id")))
    {
        u["checkpoint"] = json!(a);
    }
    // Two units that now share a checkpoint and a file, with no order between them, go one after
    // the other: the later checkpoint's unit after the earlier one's.
    for ub1 in &ub {
        for ua1 in &ua {
            let find = |id: &String| e.units.iter().find(|u| s(u, "id") == id).cloned();
            let (Some(x), Some(y)) = (find(ua1), find(ub1)) else {
                continue;
            };
            let share = str_list(&x, "files").iter().any(|fa| {
                str_list(&y, "files").iter().any(|fb| {
                    crate::util::norm_overlap(
                        &crate::util::norm_scope(fa),
                        &crate::util::norm_scope(fb),
                    )
                })
            });
            if !share || reaches(&e.units, ub1, ua1) || reaches(&e.units, ua1, ub1) {
                continue;
            }
            if let Some(u) = e.units.iter_mut().find(|u| s(u, "id") == ub1) {
                let mut d = str_list(u, "depends_on");
                d.push(ua1.clone());
                u["depends_on"] = json!(d);
            }
        }
    }
    e.cps.retain(|c| s(c, "id") != b);
    if let Some(c) = e.cps.iter_mut().find(|c| s(c, "id") == a) {
        *c = k;
    }
    // Every checkpoint after the merged one takes the number one lower.
    let map: BTreeMap<String, String> = e
        .cps
        .iter()
        .map(|k| s(k, "id").to_string())
        .filter(|id| num(id) > num(&b))
        .map(|id| {
            let to = format!("K-{}", num(&id) - 1);
            (id, to)
        })
        .collect();
    let follow = |x: &str| map.get(x).cloned().unwrap_or_else(|| x.to_string());
    let fresh: Vec<(String, String)> = map
        .iter()
        .filter(|(cur, _)| {
            !e.created.iter().any(|c| c == *cur) && !e.renames.iter().any(|(_, n)| n == *cur)
        })
        .map(|(c, n)| (c.clone(), n.clone()))
        .collect();
    e.renames = e
        .renames
        .iter()
        .filter(|(_, n)| *n != b)
        .map(|(o, n)| (o.clone(), follow(n)))
        .chain(fresh)
        .collect();
    e.created = e
        .created
        .iter()
        .filter(|c| **c != b)
        .map(|c| follow(c))
        .collect();
    for k in e.cps.iter_mut() {
        let id = follow(s(k, "id"));
        k["id"] = json!(id);
    }
    for u in e.units.iter_mut() {
        let id = follow(s(u, "checkpoint"));
        u["checkpoint"] = json!(id);
    }
    e.cps.sort_by_key(|k| num(s(k, "id")));
    Ok(())
}

fn apply_op(p: &Project, st: &State, e: &mut Edit, op: &Value) -> R<()> {
    if s(op, "how") == "merge" {
        return apply_merge(st, e, op);
    }
    let how = s(op, "how").to_string();
    let at = s(op, "at").to_string();
    let what = if how == "split" {
        "split a checkpoint"
    } else {
        "insert a checkpoint"
    };
    let fail = |because: String, steps: Vec<String>| {
        refuse_owned(
            what,
            because,
            steps,
            "`bf amend checkpoint` prints the renamed numbers and the plan check on the result.",
        )
    };
    if !e.cps.iter().any(|k| s(k, "id") == at) {
        return Err(fail(
            format!("{at} is not a checkpoint of the plan"),
            vec!["Name a checkpoint that `bf plan check` lists.".to_string()],
        ));
    }
    let named = str_list(op, "units");
    let mut unknown: Vec<String> = vec![];
    for u in &named {
        if !e.units.iter().any(|x| s(x, "id") == u) {
            unknown.push(u.clone());
        }
    }
    if named.is_empty() || !unknown.is_empty() {
        return Err(fail(
            if named.is_empty() {
                "no units were named".to_string()
            } else {
                format!("{} is not a unit", unknown.join(", "))
            },
            vec!["Name units of the plan.".to_string()],
        ));
    }
    let at_num = num(&at);
    let new_num = if how == "before" { at_num } else { at_num + 1 };
    let new_id = format!("K-{new_num}");
    let unit_cp = |e: &Edit, u: &str| -> String {
        e.units
            .iter()
            .find(|x| s(x, "id") == u)
            .map(|x| s(x, "checkpoint").to_string())
            .unwrap_or_default()
    };
    // The units that go onto the new checkpoint.
    let moved: Vec<String> = if how == "split" {
        if let Some(u) = named.iter().find(|u| unit_cp(e, u) != at) {
            return Err(fail(
                format!("{u} is not on {at}, so it cannot stay there"),
                vec![format!("Name only units that {at} lists in `after`.")],
            ));
        }
        let rest: Vec<String> = e
            .cps
            .iter()
            .find(|k| s(k, "id") == at)
            .map(|k| str_list(k, "after"))
            .unwrap_or_default()
            .into_iter()
            .filter(|u| !named.contains(u))
            .collect();
        if rest.is_empty() {
            return Err(fail(
                format!("every unit of {at} was named, so nothing goes onto the new checkpoint"),
                vec![format!("Name only the units that stay on {at}.")],
            ));
        }
        rest
    } else {
        named.clone()
    };
    // Only a checkpoint that has not started can be renamed or lose a unit.
    let mut refusals: Vec<String> = vec![];
    for u in &moved {
        if st.units.contains_key(u) {
            refusals.push(format!(
                "{u} has started, so it cannot move to another checkpoint"
            ));
        }
        let from = unit_cp(e, u);
        if e.orig_of(&from)
            .map(|o| st.closed.contains_key(&o))
            .unwrap_or(false)
        {
            refusals.push(format!("{from} is closed, so {u} cannot leave it"));
        }
    }
    if how == "split"
        && e.orig_of(&at)
            .map(|o| st.closed.contains_key(&o))
            .unwrap_or(false)
    {
        refusals.push(format!("{at} is closed, so it cannot be split"));
    }
    for k in e.cps.iter().filter(|k| num(s(k, "id")) >= new_num) {
        let id = s(k, "id");
        if e.orig_of(id)
            .map(|o| st.closed.contains_key(&o))
            .unwrap_or(false)
        {
            refusals.push(format!(
                "{id} is closed, so it cannot be renamed to K-{}",
                num(id) + 1
            ));
        }
        for u in e.units.iter().filter(|u| s(u, "checkpoint") == id) {
            if st.units.contains_key(s(u, "id")) {
                refusals.push(format!(
                    "{id} holds {}, which has started, so it cannot be renamed to K-{}",
                    s(u, "id"),
                    num(id) + 1
                ));
            }
        }
    }
    refusals.dedup();
    if !refusals.is_empty() {
        return Err(fail(
            refusals.join("; "),
            vec![
                "Only a checkpoint that has not started can be renamed: insert after the last checkpoint that has started, or pick units that have not started.".to_string(),
            ],
        ));
    }
    // Rename every checkpoint from the new number on, one number forward.
    let map: BTreeMap<String, String> = e
        .cps
        .iter()
        .map(|k| s(k, "id").to_string())
        .filter(|id| num(id) >= new_num)
        .map(|id| {
            let to = format!("K-{}", num(&id) + 1);
            (id, to)
        })
        .collect();
    let follow = |x: &str| map.get(x).cloned().unwrap_or_else(|| x.to_string());
    let fresh: Vec<(String, String)> = map
        .iter()
        .filter(|(cur, _)| {
            !e.created.iter().any(|c| c == *cur) && !e.renames.iter().any(|(_, n)| n == *cur)
        })
        .map(|(c, n)| (c.clone(), n.clone()))
        .collect();
    e.renames = e
        .renames
        .iter()
        .map(|(o, n)| (o.clone(), follow(n)))
        .chain(fresh)
        .collect();
    e.created = e.created.iter().map(|c| follow(c)).collect();
    for k in e.cps.iter_mut() {
        let id = follow(s(k, "id"));
        k["id"] = json!(id);
    }
    for u in e.units.iter_mut() {
        let id = follow(s(u, "checkpoint"));
        u["checkpoint"] = json!(id);
    }
    // The check of the whole checkpoint: each command check of its units, run over the checkpoint.
    let at = follow(&at);
    let mut xn = e.checks.iter().map(|c| num(s(c, "id"))).max().unwrap_or(0) + 1;
    if let Some(k) = schema::kind_by_name("check") {
        if let Ok(id) = p.next_id(k) {
            xn = xn.max(num(&id));
        }
    }
    let mut copies: Vec<Value> = vec![];
    let mut seen: Vec<Value> = vec![];
    for u in &moved {
        for c in e
            .checks
            .iter()
            .filter(|c| s(c, "unit") == u && s(c, "how") == "command" && s(c, "scope") == "unit")
        {
            let cmd = c.get("command").cloned().unwrap_or(Value::Null);
            if seen.contains(&cmd) {
                continue;
            }
            seen.push(cmd);
            let mut n = c.clone();
            n["id"] = json!(format!("X-{xn}"));
            xn += 1;
            n["scope"] = json!("checkpoint");
            if let Some(o) = n.as_object_mut() {
                o.remove("unit");
                o.remove("named");
            }
            copies.push(n);
        }
    }
    let proved: HashSet<String> = copies.iter().flat_map(|c| str_list(c, "proves")).collect();
    // What each checkpoint the units leave stops proving, and the new one proves instead.
    let mut sources: Vec<String> = vec![];
    for u in &moved {
        let from = unit_cp(e, u);
        if !sources.contains(&from) {
            sources.push(from);
        }
    }
    let delivers = |e: &Edit, u: &str| -> Vec<String> {
        e.units
            .iter()
            .find(|x| s(x, "id") == u)
            .map(|x| str_list(x, "delivers"))
            .unwrap_or_default()
    };
    let mut new_proves: Vec<String> = vec![];
    let mut leftovers: Vec<String> = vec![];
    let mut cut: Vec<(String, Vec<String>, Vec<String>)> = vec![];
    for sid in &sources {
        // Named by the number it has on disk, which is the one the human knows.
        let was = e.orig_of(sid).unwrap_or_else(|| sid.clone());
        let after: Vec<String> = e
            .cps
            .iter()
            .find(|k| s(k, "id") == sid)
            .map(|k| str_list(k, "after"))
            .unwrap_or_default();
        let (gone, stay): (Vec<String>, Vec<String>) =
            after.into_iter().partition(|u| moved.contains(u));
        if stay.is_empty() {
            leftovers.push(format!("{was} would be left with no unit"));
        }
        let proves: Vec<String> = e
            .cps
            .iter()
            .find(|k| s(k, "id") == sid)
            .map(|k| str_list(k, "proves"))
            .unwrap_or_default();
        let by_gone: HashSet<String> = gone.iter().flat_map(|u| delivers(e, u)).collect();
        let by_stay: HashSet<String> = stay.iter().flat_map(|u| delivers(e, u)).collect();
        let handoff_to_gone = |t: &str| {
            e.handoffs
                .iter()
                .any(|h| s(h, "id") == t && gone.iter().any(|u| u == s(h, "to")))
        };
        let (take, keep): (Vec<String>, Vec<String>) = proves.into_iter().partition(|t| {
            proved.contains(t)
                && ((by_gone.contains(t) && !by_stay.contains(t)) || handoff_to_gone(t))
        });
        if keep.is_empty() {
            leftovers.push(format!(
                "{was} would prove nothing after the units leave it"
            ));
        }
        for t in &take {
            if !new_proves.contains(t) {
                new_proves.push(t.clone());
            }
        }
        cut.push((sid.clone(), stay, keep));
    }
    if new_proves.is_empty() {
        leftovers.push(
            "none of the units that go onto it has a command check that proves a part it delivers"
                .to_string(),
        );
    }
    if !leftovers.is_empty() {
        return Err(fail(
            leftovers.join("; "),
            vec!["Pick units so that every checkpoint keeps a unit and a part it proves, and each unit that moves has a check with `how = \"command\"` and `scope = \"unit\"` that proves what it delivers.".to_string()],
        ));
    }
    // The stop for the human and the checks for the units follow the units to the new checkpoint.
    let donor = if how == "split" {
        e.cps.iter().find(|k| s(k, "id") == at).cloned()
    } else {
        sources
            .iter()
            .filter_map(|sid| e.cps.iter().find(|k| s(k, "id") == sid))
            .find(|k| k.get("stop").and_then(|v| v.as_bool()).unwrap_or(false))
            .cloned()
            .or_else(|| {
                sources
                    .first()
                    .and_then(|sid| e.cps.iter().find(|k| s(k, "id") == sid).cloned())
            })
    };
    let title = if s(op, "title").is_empty() {
        moved
            .iter()
            .filter_map(|u| {
                e.units
                    .iter()
                    .find(|x| s(x, "id") == u)
                    .map(|x| s(x, "title").to_string())
            })
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        s(op, "title").to_string()
    };
    let mut k = json!({
        "id": new_id, "kind": "checkpoint", "title": title,
        "after": moved, "proves": new_proves,
        "checks": copies.iter().map(|c| s(c, "id").to_string()).collect::<Vec<_>>(),
    });
    if let Some(d) = &donor {
        for f in [
            "stop",
            "stop_why",
            "human_step",
            "on_human_step",
            "pull_request",
            "unit_checks",
            "unit_checks_reason",
        ] {
            if let Some(v) = d.get(f) {
                k[f] = v.clone();
            }
        }
    }
    for (sid, stay, keep) in cut {
        if let Some(c) = e.cps.iter_mut().find(|c| s(c, "id") == sid) {
            c["after"] = json!(stay);
            c["proves"] = json!(keep);
        }
    }
    for u in e.units.iter_mut() {
        if moved.iter().any(|m| m == s(u, "id")) {
            u["checkpoint"] = json!(new_id);
        }
    }
    e.cps.push(k);
    e.cps.sort_by_key(|k| num(s(k, "id")));
    e.checks.extend(copies);
    e.created.push(new_id);
    // Units of the new checkpoint that share files go one after the other, in the order they were named.
    let repo_of = |u: &Value| {
        p.repo_of_unit(u)
            .map(|r| r.name.clone())
            .unwrap_or_default()
    };
    for (j, b) in moved.iter().enumerate() {
        for a in &moved[..j] {
            let find = |id: &String| e.units.iter().find(|u| s(u, "id") == id).cloned();
            let (Some(ua), Some(ub)) = (find(a), find(b)) else {
                continue;
            };
            if repo_of(&ua) != repo_of(&ub) {
                continue;
            }
            let share = str_list(&ua, "files").iter().any(|fa| {
                str_list(&ub, "files").iter().any(|fb| {
                    crate::util::norm_overlap(
                        &crate::util::norm_scope(fa),
                        &crate::util::norm_scope(fb),
                    )
                })
            });
            if !share || reaches(&e.units, b, a) || reaches(&e.units, a, b) {
                continue;
            }
            if let Some(u) = e.units.iter_mut().find(|u| s(u, "id") == b) {
                let mut d = str_list(u, "depends_on");
                d.push(a.clone());
                u["depends_on"] = json!(d);
            }
        }
    }
    Ok(())
}

/// The plan after all the operations of an amendment, in memory.
pub fn edit_of(p: &Project, st: &State, ops: &[Value]) -> R<Edit> {
    let mut e = Edit::load(p)?;
    for op in ops {
        apply_op(p, st, &mut e, op)?;
        // A checkpoint record on disk that the plan now has no place for is removed at the approval.
        e.removed = e
            .disk
            .keys()
            .filter(|id| id.starts_with("K-") && !e.cps.iter().any(|k| s(k, "id") == id.as_str()))
            .cloned()
            .collect();
    }
    Ok(e)
}

/// The plan problems the result has that the plan on disk does not.
/// The units the approval of `a` would send back to work count as not landed on the result's side.
pub fn result_problems(p: &Project, a: &Amend, e: &Edit) -> R<Vec<String>> {
    // The plan in force before the narrowing is the one to compare with: a check the amendment changed
    // counts as new for the test-target judge, and a problem it adds is the amendment's own (FU-35).
    let mut before = crate::gates::before_plan(p, a)?;
    let base = crate::plan::problems_of_edited(
        p,
        &before,
        before.units.clone(),
        before.checkpoints.clone(),
        before.checks.clone(),
        &[],
    )?;
    before.handoffs = crate::plan::load(p)?.handoffs;
    let st = crate::events::replay(p)?;
    let reopened = crate::gates::reopen_units_of(&e.units, &st, a);
    let after = crate::plan::problems_of_edited(
        p,
        &before,
        e.units.clone(),
        e.cps.clone(),
        e.checks.clone(),
        &reopened,
    )?;
    Ok(after.into_iter().filter(|x| !base.contains(x)).collect())
}

/// Refuse approval when the plan check finds a problem in what the operations would leave.
pub fn refuse_if_result_breaks_plan(p: &Project, a: &Amend, e: &Edit) -> R<()> {
    let problems = result_problems(p, a, e)?;
    if problems.is_empty() {
        return Ok(());
    }
    let aid = &a.id;
    let mut steps: Vec<String> = problems
        .iter()
        .take(20)
        .map(|x| format!("Fix: {x}."))
        .collect();
    steps.push(format!(
        "Change the records or the operations of {aid}, and narrow it again with `bf amend narrow --file <f>`."
    ));
    Err(refuse_owned(
        format!("approve {aid}"),
        format!(
            "the plan check finds {} problem(s) in the plan its checkpoint operations would leave",
            problems.len()
        ),
        steps,
        "`bf amend approve` prints the approved amendment.",
    ))
}

/// Write the records the operations change. All or none: when a write fails, the records already
/// written are put back as they were, so approval can be tried again on an untouched plan.
pub fn write_edit(p: &Project, e: &Edit) -> R<Vec<(std::path::PathBuf, Option<Vec<u8>>)>> {
    let changed = e.changed();
    let mut before: Vec<(std::path::PathBuf, Option<Vec<u8>>)> = vec![];
    for id in changed
        .iter()
        .map(|r| s(r, "id").to_string())
        .chain(e.removed.iter().cloned())
    {
        let path = p.record_path(&id)?;
        before.push((path.clone(), std::fs::read(&path).ok()));
    }
    for r in &changed {
        if let Err(err) = p.write_record(r) {
            undo_writes(&before);
            return Err(err);
        }
    }
    for id in &e.removed {
        let gone = p
            .record_path(id)
            .and_then(|f| std::fs::remove_file(f).map_err(Into::into));
        if let Err(err) = gone {
            undo_writes(&before);
            return Err(err);
        }
    }
    Ok(before)
}

/// Put back the files `write_edit` read before it wrote: their old bytes, or no file.
pub fn undo_writes(before: &[(std::path::PathBuf, Option<Vec<u8>>)]) {
    for (path, old) in before {
        let _ = match old {
            Some(b) => crate::util::atomic_write(path, b),
            None => std::fs::remove_file(path),
        };
    }
}

/// The rows of the plan stage of an amendment page for its checkpoint operations: a renamed checkpoint
/// under its old number, marked as moved, every other changed record as changed or new, and each record
/// an operation removes with its old content, marked gone. The marks are texts of `amend_merge.toml`.
pub fn plan_rows(e: &Edit) -> Vec<(String, Value, String)> {
    let mut out = vec![];
    for r in e.changed() {
        let id = s(&r, "id").to_string();
        if let Some((old, _)) = e.renames.iter().find(|(_, n)| *n == id) {
            let how = strings::t("cli.amend_merge.row.moved", &[("id", &id)]);
            out.push((old.clone(), r.clone(), how));
        } else if e.created.contains(&id) || !e.disk.contains_key(&id) {
            // The freed number belongs to the new checkpoint, and its old holder's row keeps that id.
            let how = strings::t("cli.amend_merge.row.new", &[]);
            out.push((format!("new-{id}"), r.clone(), how));
        } else {
            let how = strings::t("cli.amend_merge.row.changed", &[]);
            out.push((id, r.clone(), how));
        }
    }
    for id in &e.removed {
        if let Some(old) = e.disk.get(id) {
            let how = strings::t("cli.amend_merge.row.gone", &[]);
            out.push((format!("gone-{id}"), old.clone(), how));
        }
    }
    out
}
