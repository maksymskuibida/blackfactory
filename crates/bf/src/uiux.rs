//! `bf uiux`: the UI/UX evidence. This file belongs to this command group alone; the command line
//! calls its entry point.
//!
//! The two UI/UX verifications (`design-uiux-verification`, `built-uiux-verification`) take their
//! evidence step by step. A verifier attaches one file and one outcome to one journey step (or one
//! screen state) with the step evidence command (H-4):
//!
//! `bf evidence attach <check> <file> --outcome met|failed|unreachable --says "<J-id> step <n>: <what was seen>" --token <token>`
//!
//! A screen state is named `S-<n> state <label>: ...`. The engine seals each as a `step` receipt bound to
//! the content it was taken on. The verification's own `met` is refused until every recorded journey
//! step has a `met` step receipt at the current content: a missing, failed or unreachable step never
//! counts as passed.

use crate::evidence::{self, Receipt};
use crate::refusal::{refuse, R};
use crate::store::{s, Project};
use serde_json::{json, Value};
use std::path::Path;

/// The WCAG level the accessibility result is held to when the design records name none. An
/// assumption of the draft.
pub const WCAG_LEVEL: &str = "WCAG 2.2 level AA";

/// The title of the decision record in which the human sets the accessibility standard at the UI
/// gate; its `chosen` field holds the standard (for example `WCAG 2.2 level AAA`).
pub const STANDARD_DECISION: &str = "accessibility standard";

/// The subject of the accessibility result's own evidence: `accessibility: <what was checked>`.
pub const A11Y: &str = "accessibility";

/// The accessibility standard in force: the `chosen` of the decision titled "Accessibility
/// standard" if there is one, else WCAG 2.2 level AA.
pub fn standard(p: &Project) -> R<String> {
    let ds = p.list_records(crate::schema::kind_by_name("decision"))?;
    let mut ds: Vec<&Value> = ds
        .iter()
        .filter(|d| s(d, "title").trim().eq_ignore_ascii_case(STANDARD_DECISION))
        .filter(|d| !s(d, "chosen").trim().is_empty())
        .collect();
    ds.sort_by(|a, b| s(a, "id").cmp(s(b, "id")));
    Ok(ds
        .last()
        .map(|d| s(d, "chosen").trim().to_string())
        .unwrap_or_else(|| WCAG_LEVEL.to_string()))
}

/// Split a sentence of the form `<ID> step <n>: <text>` (journey) or `<ID> state <label>: <text>`
/// (screen). Returns the subject ID, the step number or state label, and the text.
pub fn parse_step(says: &str) -> Option<(String, String, String)> {
    let (head, text) = says.split_once(':')?;
    if head.trim() == A11Y {
        if text.trim().is_empty() {
            return None;
        }
        return Some((
            A11Y.to_string(),
            "check".to_string(),
            text.trim().to_string(),
        ));
    }
    let words: Vec<&str> = head.split_whitespace().collect();
    if words.len() < 3 || text.trim().is_empty() {
        return None;
    }
    let (id, word) = (words[0], words[1]);
    let label = words[2..].join(" ");
    match word {
        "step" if id.starts_with("J-") && words.len() == 3 && label.parse::<usize>().ok()? >= 1 => {
        }
        "state" if id.starts_with("S-") => {}
        _ => return None,
    }
    Some((id.to_string(), label, text.trim().to_string()))
}

/// Refuse a step that names a journey or screen that is not recorded, or a step past the last one.
pub fn require_step_exists(p: &Project, subject: &str, n: &str) -> R<()> {
    if subject == A11Y {
        return Ok(());
    }
    let rec = p.get_record(subject)?;
    let bad = |why: String| {
        refuse(
            format!("attach evidence for {subject} step {n}"),
            why,
            &["Name a recorded journey step (`J-<n> step <k>`, k from 1) or screen state (`S-<n> state <label>`); `bf record get <ID>` shows the steps."],
            "The command prints a receipt.",
        )
    };
    let Some(rec) = rec else {
        return Err(bad(format!("no record {subject} exists")));
    };
    if subject.starts_with("J-") {
        let count = crate::store::str_list(&rec, "steps").len();
        let k: usize = n.parse().unwrap_or(0);
        if k == 0 || k > count {
            return Err(bad(format!(
                "{subject} has {count} step(s); there is no step {n}"
            )));
        }
    } else if s(&rec, "kind") != "screen" {
        return Err(bad(format!("{subject} is not a screen")));
    }
    Ok(())
}

/// TH-6: the evidence of the built-result verification must not come from the built result. A file
/// that lies inside the checked-out product could have been written by the product itself.
pub fn refuse_evidence_from_built_result(p: &Project, file: &Path) -> R<()> {
    let canon = std::fs::canonicalize(file)?;
    let top = std::fs::canonicalize(&p.top).unwrap_or_else(|_| p.top.clone());
    if canon.starts_with(&top) {
        return Err(refuse(
            format!("attach {} as evidence of the built result", file.display()),
            "the file lies inside the built result, so the product itself could have written it",
            &["Take the screenshot or transcript with your own tools and save it outside the project folder, then attach that file."],
            "The command prints a receipt of type step.",
        ));
    }
    Ok(())
}

/// A hash of the screens and journeys as they are now: what a design-stage verification looked at.
/// The same hash binds the step receipts and the verification's own `met`.
pub fn design_hash(p: &Project) -> R<String> {
    evidence::screens_hash(p)
}

/// What the step receipts of one verification say now.
pub struct Coverage {
    /// One row per journey step: journey, step, text, status (`met`, `failed`, `unreachable`, `missing`),
    /// receipt.
    pub rows: Vec<Value>,
    /// Screens with the status of their states.
    pub screens: Vec<Value>,
    /// One line per step that is not `met`.
    pub problems: Vec<String>,
    /// Each WCAG criterion named in a failed step, with where.
    pub failed_criteria: Vec<Value>,
    /// The accessibility result of this verification: standard, status (`met`, `failed`,
    /// `unreachable`, `missing`), the receipt that proves it and the failed criteria with where.
    pub accessibility: Value,
}

/// WCAG criteria (`1.4.3`) named in a sentence.
pub fn criteria(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in text.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let parts: Vec<&str> = w.trim_matches('.').split('.').collect();
        if parts.len() == 3
            && parts[0].len() == 1
            && parts
                .iter()
                .all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))
            && !out.contains(&parts.join("."))
        {
            out.push(parts.join("."));
        }
    }
    out
}

pub fn coverage_of(p: &Project, check_id: &str, named: &str) -> R<Coverage> {
    let receipts = evidence::load_receipts(p)?;
    let bound = if named == "design-uiux-verification" {
        design_hash(p)?
    } else {
        p.content_hash(&p.top)?
    };
    let steps: Vec<&Receipt> = receipts
        .iter()
        .filter(|r| {
            r.seal_ok
                && !r.stale
                && s(&r.v, "type") == "step"
                && r.check() == check_id
                && if named == "design-uiux-verification" {
                    s(&r.v, "screens_hash") == bound
                } else {
                    r.content() == bound
                }
        })
        .collect();
    let latest = |subject: &str, n: &str| {
        steps
            .iter()
            .rev()
            .find(|r| s(&r.v, "journey") == subject && s(&r.v, "step") == n)
    };
    let mut cov = Coverage {
        rows: vec![],
        screens: vec![],
        problems: vec![],
        failed_criteria: vec![],
        accessibility: Value::Null,
    };
    let mut js = p.list_records(crate::schema::kind_by_name("journey"))?;
    js.sort_by(|a, b| s(a, "id").cmp(s(b, "id")));
    for j in &js {
        let id = s(j, "id");
        for (i, text) in crate::store::str_list(j, "steps").iter().enumerate() {
            let n = (i + 1).to_string();
            let r = latest(id, &n);
            let status = r.map(|r| s(&r.v, "step_outcome")).unwrap_or("missing");
            if status != "met" {
                cov.problems.push(match status {
                    "missing" => format!("{id} step {n} has no evidence"),
                    "unreachable" => {
                        format!("{id} step {n} could not be reached; it is not passed")
                    }
                    _ => format!("{id} step {n} failed"),
                });
            }
            if let Some(r) = r.filter(|_| status == "failed" || status == "unreachable") {
                for c in criteria(s(&r.v, "reason")) {
                    cov.failed_criteria.push(json!({"criterion": c, "where": format!("{id} step {n}"), "receipt": r.id()}));
                }
            }
            cov.rows.push(json!({"journey": id, "step": i + 1, "text": text, "status": status, "receipt": r.map(|r| r.id())}));
        }
    }
    let mut ss = p.list_records(crate::schema::kind_by_name("screen"))?;
    ss.sort_by(|a, b| s(a, "id").cmp(s(b, "id")));
    for sc in &ss {
        let id = s(sc, "id");
        let mine: Vec<&&Receipt> = steps.iter().filter(|r| s(&r.v, "journey") == id).collect();
        // The latest receipt of each named state decides that state.
        let mut states: Vec<(String, &str, String)> = vec![];
        for r in &mine {
            let st = s(&r.v, "step").to_string();
            let o = s(&r.v, "step_outcome");
            states.retain(|x| x.0 != st);
            states.push((st, o, r.id().to_string()));
        }
        for (st, o, rid) in &states {
            if *o != "met" {
                cov.problems.push(format!("{id} state {st} is {o}"));
            }
            if *o == "failed" || *o == "unreachable" {
                if let Some(r) = mine.iter().rev().find(|r| s(&r.v, "step") == st) {
                    for c in criteria(s(&r.v, "reason")) {
                        cov.failed_criteria.push(json!({"criterion": c, "where": format!("{id} state {st}"), "receipt": rid}));
                    }
                }
            }
        }
        if states.is_empty() {
            cov.problems.push(format!("{id} has no evidence"));
        }
        cov.screens.push(json!({"screen": id, "states": states.iter().map(|(a, b, c)| json!({"state": a, "status": b, "receipt": c})).collect::<Vec<_>>()}));
    }
    // The accessibility result has its own proof: one `accessibility: ...` receipt.
    let std = standard(p)?;
    let ar = latest(A11Y, "check");
    let mut astatus = ar.map(|r| s(&r.v, "step_outcome")).unwrap_or("missing");
    if let Some(r) = ar.filter(|_| astatus != "met") {
        for c in criteria(s(&r.v, "reason")) {
            cov.failed_criteria
                .push(json!({"criterion": c, "where": "accessibility check", "receipt": r.id()}));
        }
    }
    if astatus == "met" && !cov.failed_criteria.is_empty() {
        astatus = "failed";
    }
    match astatus {
        "met" => {}
        "missing" => cov
            .problems
            .push(format!("accessibility has no evidence against {std}")),
        "unreachable" => cov
            .problems
            .push("accessibility could not be checked; it is not passed".to_string()),
        _ => cov
            .problems
            .push(format!("accessibility failed against {std}")),
    }
    cov.accessibility = json!({
        "standard": std,
        "status": astatus,
        "receipt": ar.map(|r| r.id()),
        "failed_criteria": cov.failed_criteria.clone(),
    });
    Ok(cov)
}

/// The verification's own `met` needs every recorded journey step met at the current content.
pub fn require_covered(p: &Project, check_id: &str, named: &str) -> R<()> {
    let cov = coverage_of(p, check_id, named)?;
    if cov.problems.is_empty() {
        return Ok(());
    }
    let shown: Vec<String> = cov.problems.iter().take(8).cloned().collect();
    Err(refuse(
        format!("settle {check_id} as met"),
        format!(
            "not every step is met at the content as it is now: {}{}",
            shown.join("; "),
            if cov.problems.len() > shown.len() { "; and more" } else { "" }
        ),
        &[
            "Attach evidence for each step, each screen state and the accessibility result (`--says \"accessibility: <what was checked>\"`): `bf evidence attach <check> <file> --outcome met|failed|unreachable --says \"J-<n> step <k>: <what was seen>\" --token <token>`.",
            "If a step failed or cannot be reached, file it with `bf record put finding` and settle the check with `bf evidence fail` or `bf evidence unreachable`, not as met.",
        ],
        "`bf uiux coverage <check>` lists every step as met.",
    ))
}

/// Entry point of `bf uiux <sub> ...`: `coverage <check>` prints the state of every step.
pub fn run(sub: &str, rest: &[String]) -> R<Value> {
    match (sub, rest.first()) {
        ("coverage", Some(check)) => {
            let cwd = std::env::current_dir()?;
            let p = Project::discover(&cwd)?;
            let chk = p.require_record(check)?;
            let named = s(&chk, "named").to_string();
            if !matches!(
                named.as_str(),
                "design-uiux-verification" | "built-uiux-verification"
            ) {
                return Err(refuse(
                    format!("show the coverage of {check}"),
                    "that check is not a UI/UX verification",
                    &["Give the ID of the check named design-uiux-verification or built-uiux-verification."],
                    "The command prints its result.",
                ));
            }
            let cov = coverage_of(&p, check, &named)?;
            Ok(json!({
                "ok": cov.problems.is_empty(),
                "check": check,
                "standard": standard(&p)?,
                "accessibility": cov.accessibility,
                "steps": cov.rows,
                "screens": cov.screens,
                "problems": cov.problems,
                "failed_criteria": cov.failed_criteria,
            }))
        }
        _ => Err(refuse(
            format!("run `bf uiux {sub}`"),
            "only `bf uiux coverage <check>` exists",
            &["Run `bf uiux coverage <check>`."],
            "The command prints its result.",
        )),
    }
}
