//! The amendment part (C-14): the stage coverage of a design amendment, the refusal at approval, and
//! the data of the amendment page.
//!
//! A design amendment covers each design stage, in order, with its new or changed records or with one
//! sentence that says the stage is not affected. The sentence is a decision record made by the agent
//! (`maker = "agent"`) that names its stage (`stage`) and is named in the amendment's set when the
//! amendment is opened or revised. Approval is refused while a stage has neither, and while a
//! requirement of the amendment has no link to another design part.

use crate::events::Amend;
use crate::refusal::{refuse_owned, R};
use crate::store::{s, str_list, Project};
use serde_json::Value;

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
    for id in &a.ids {
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

/// A fingerprint of the records of the amendment's set as they are now. It is written into the
/// narrowing the human reads, and approval is refused when the records no longer match it (TH-12).
pub fn set_hash(p: &Project, a: &Amend) -> R<String> {
    let mut ids: Vec<&String> = a.ids.iter().collect();
    ids.sort();
    let mut parts = Vec::new();
    for id in ids {
        let h = match p.get_record(id)? {
            Some(r) => crate::evidence::record_hash(&r),
            None => "missing".to_string(),
        };
        parts.push(format!("{id}:{h}"));
    }
    Ok(crate::evidence::record_hash(&serde_json::json!(parts)))
}

/// Refuse approval when a record of the set changed after the narrowing the human read.
pub fn refuse_if_changed_since_narrowing(p: &Project, a: &Amend, narrowed: &Value) -> R<()> {
    let Some(was) = narrowed["set_hash"].as_str() else {
        return Ok(());
    };
    if was == set_hash(p, a)? {
        return Ok(());
    }
    let aid = &a.id;
    Err(refuse_owned(
        format!("approve {aid}"),
        "a record of the amendment was put or replaced after it was narrowed, so the human has not read it".to_string(),
        vec![
            format!("Narrow {aid} again with `bf amend narrow --file <f>`."),
            "Ask the human to read it and say yes after that.".to_string(),
        ],
        "`bf amend approve` prints the approved amendment.",
    ))
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
