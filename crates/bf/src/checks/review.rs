//! The review checks' view of the records (C-24): which records each kind of review judges, and the
//! hash of them that a review's receipt is bound to. A change to any record of a kind needs that
//! review again; a change to a record of another kind does not.

use crate::refusal::R;
use crate::store::Project;
use crate::util;
use serde_json::Value;

/// What a design review judges: one stage, or the whole of an amendment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewKind {
    Idea,
    Requirements,
    Architecture,
    /// The legal and markets records, a stage of an amendment.
    Legal,
    Plan,
    /// The whole of an amendment: the records its set names and its stage decisions.
    Amendment,
}

/// Every kind of review, in the order of the gates.
pub const REVIEW_KINDS: [ReviewKind; 6] = [
    ReviewKind::Idea,
    ReviewKind::Requirements,
    ReviewKind::Architecture,
    ReviewKind::Legal,
    ReviewKind::Plan,
    ReviewKind::Amendment,
];

impl ReviewKind {
    pub fn name(self) -> &'static str {
        match self {
            ReviewKind::Idea => "idea",
            ReviewKind::Requirements => "requirements",
            ReviewKind::Architecture => "architecture",
            ReviewKind::Legal => "legal",
            ReviewKind::Plan => "plan",
            ReviewKind::Amendment => "amendment",
        }
    }

    pub fn parse(name: &str) -> Option<ReviewKind> {
        REVIEW_KINDS.iter().copied().find(|k| k.name() == name)
    }

    /// The stage a review of this kind belongs to, as a decision names it.
    fn stage(self) -> &'static str {
        match self {
            ReviewKind::Idea => "idea",
            ReviewKind::Requirements => "requirements",
            ReviewKind::Architecture | ReviewKind::Legal => "architecture",
            ReviewKind::Plan => "plan",
            ReviewKind::Amendment => "",
        }
    }

    /// The record kinds this review judges. At a gate the architecture holds the markets and legal
    /// records too; inside an amendment the legal stage is its own.
    fn record_kinds(self, scoped: bool) -> &'static [&'static str] {
        match (self, scoped) {
            (ReviewKind::Idea, _) => &["idea"],
            (ReviewKind::Requirements, _) => &["requirement", "journey"],
            (ReviewKind::Architecture, false) => {
                &["component", "decision", "target", "threat", "markets", "legal"]
            }
            (ReviewKind::Architecture, true) => &["component", "decision", "target", "threat"],
            (ReviewKind::Legal, _) => &["markets", "legal"],
            (ReviewKind::Plan, _) => &["unit", "handoff", "check", "checkpoint"],
            (ReviewKind::Amendment, _) => &[],
        }
    }
}

/// The stage a record kind belongs to.
fn stage_of_kind(kind: &str) -> &'static str {
    match kind {
        "idea" => "idea",
        "requirement" | "journey" => "requirements",
        "screen" => "ui",
        "component" | "decision" | "target" | "threat" | "markets" | "legal" => "architecture",
        _ => "plan",
    }
}

fn is_review_check(rec: &Value) -> bool {
    let n = rec["named"].as_str().unwrap_or("");
    super::in_family(super::Family::Review)
        .iter()
        .any(|c| c.name == n)
}

/// The records a review of `kind` judges, as `(name, content)` sorted by name. `set` is the list of IDs
/// an amendment names: for the amendment kind it picks the records, for a stage kind it narrows the
/// stage to the records of the set. Without a set, a stage kind covers the whole stage.
/// The idea also holds the text of `factory/idea.md`.
pub fn kind_records(
    p: &Project,
    kind: ReviewKind,
    set: Option<&[String]>,
) -> R<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    let in_set = |id: &str| set.map(|s| s.iter().any(|x| x == id)).unwrap_or(true);
    let mut push = |id: &str, rec: &Value| {
        out.push((id.to_string(), serde_json::to_string(rec).unwrap_or_default()));
    };
    let all = p.list_records(None)?;
    match kind {
        ReviewKind::Amendment => {
            let ids = set.unwrap_or(&[]);
            let mut stages: Vec<&str> = Vec::new();
            for rec in &all {
                let id = rec["id"].as_str().unwrap_or("");
                if ids.iter().any(|x| x == id) {
                    stages.push(stage_of_kind(rec["kind"].as_str().unwrap_or("")));
                    push(id, rec);
                }
            }
            // The stage decisions of the stages the amendment touches.
            for rec in &all {
                let id = rec["id"].as_str().unwrap_or("");
                if rec["kind"] == "decision"
                    && !ids.iter().any(|x| x == id)
                    && stages.contains(&rec["stage"].as_str().unwrap_or(""))
                {
                    push(id, rec);
                }
            }
        }
        _ => {
            let kinds = kind.record_kinds(set.is_some());
            for rec in &all {
                let id = rec["id"].as_str().unwrap_or("");
                let k = rec["kind"].as_str().unwrap_or("");
                if !kinds.contains(&k) || is_review_check(rec) {
                    continue;
                }
                // A decision belongs to the stage it names; one that names none is architecture's.
                if k == "decision" && set.is_some() {
                    let st = rec["stage"].as_str().unwrap_or("");
                    if !in_set(id) && st != kind.stage() {
                        continue;
                    }
                } else if !in_set(id) {
                    continue;
                }
                push(id, rec);
            }
        }
    }
    if kind == ReviewKind::Idea {
        if let Ok(text) = std::fs::read_to_string(p.factory().join("idea.md")) {
            out.push(("factory/idea.md".to_string(), text));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The hash a review's receipt carries: of the records the review of `kind` judges. It changes with
/// any of them and with no record of another kind.
pub fn records_hash(p: &Project, kind: ReviewKind, set: Option<&[String]>) -> R<String> {
    let mut buf = String::new();
    buf.push_str(kind.name());
    buf.push('\n');
    for (name, content) in kind_records(p, kind, set)? {
        buf.push_str(&name);
        buf.push('\0');
        buf.push_str(&content);
        buf.push('\n');
    }
    Ok(util::sha256_hex(buf.as_bytes()))
}
