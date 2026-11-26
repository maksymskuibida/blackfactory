//! The confirmation of answered findings (R-78, D-160). A fix opened under this engine leaves the findings it
//! was opened for answered, not fixed. Only the next round of the source that filed a finding, the checkpoint
//! review or the audit of a check, ends it as fixed (`confirmed`) or opens it again (`still_open`). This module
//! holds the fate that rule gives, the section of a round's brief that lists the answered findings, and the
//! check of the report's two lists.

use crate::ckreview::{self, Round};
use crate::events::{CkFixDone, State};
use crate::gitx;
use crate::plan::Plan;
use crate::refusal::{refuse_owned, R};
use crate::run;
use crate::store::{s, Project};
use crate::strings::t;
use serde_json::{json, Value};

/// The source (the check id of an audit, empty for the review) of a round, as its opening recorded it.
pub fn source_of(st: &State, k: &str, round: u32) -> String {
    st.ck_round_source
        .get(&(k.to_string(), round))
        .cloned()
        .unwrap_or_default()
}

/// The source as a human reads it.
pub fn source_text(source: &str) -> String {
    if source.is_empty() {
        t("ck_confirm.source.review", &[])
    } else {
        t("ck_confirm.source.audit", &[("check", source)])
    }
}

/// The kind of a source for the page: `review` for the checkpoint review, else the kind of the audit's named
/// check (`security_audit`, `legal_audit`, `uiux`) or `audit` for an agent check with no named check.
pub fn source_kind(pl: &Plan, source: &str) -> &'static str {
    if source.is_empty() {
        return "review";
    }
    match crate::ckaudit::named_of(pl, source) {
        "security-audit" => "security_audit",
        "legal-audit" => "legal_audit",
        "built-uiux-verification" => "uiux",
        _ => "audit",
    }
}

/// The source as the page and `bf status` read it: the text of its kind with the check id filled in.
pub fn source_label(pl: &Plan, source: &str) -> String {
    t(
        &format!("page.finding.source.{}", source_kind(pl, source)),
        &[("check", source)],
    )
}

/// Whether a fate holds the close: none (open) or answered.
pub fn holds_fate(fate: &Option<(String, String)>) -> bool {
    match fate {
        None => true,
        Some((x, _)) => x == "answered",
    }
}

/// Whether a finding holds the close: it is open, or answered and waiting for its source.
pub fn holds(st: &State, k: &str, round: u32, index: usize, f: &Value) -> bool {
    holds_fate(&ckreview::fate(st, k, round, index, f))
}

fn names(d: &CkFixDone, round: u32, index: usize, f: &Value) -> bool {
    if d.ids.is_empty() {
        // Recorded before the pairs were kept: matched by value, but only findings of the rounds no later
        // than the one the fix was opened from.
        round <= d.from_round && d.origin_findings.contains(f)
    } else {
        d.ids.contains(&(round, index as u32 + 1))
    }
}

/// The name a fix goes by in a fate: its units and its id.
fn by(d: &CkFixDone) -> String {
    if d.units.is_empty() {
        d.id.clone()
    } else {
        format!("{} ({})", d.units.join(", "), d.id)
    }
}

/// The fate a finished fix gives to a finding it was opened for: `fixed` as it always was for a fix with no
/// `confirm`, `answered` while the source that filed it has not judged it, `fixed` again when a later round
/// of that source confirmed it, and none (open again) when that round named it still open.
pub fn fix_fate(
    st: &State,
    k: &str,
    round: u32,
    index: usize,
    f: &Value,
) -> Option<(String, String)> {
    let matching: Vec<&CkFixDone> = st
        .ck_fixes
        .iter()
        .filter(|d| d.checkpoint == k && names(d, round, index, f))
        .collect();
    let d = matching
        .iter()
        .rev()
        .find(|d| d.confirm)
        .or(matching.first())?;
    let source = source_of(st, k, round);
    // The source is read as it stands now: one switched off for the run, or whose check the checkpoint no
    // longer lists, confirms nothing, so its answered finding is fixed.
    let live = if source.is_empty() {
        ckreview::required(st)
    } else {
        !st.ck_unconfirming
            .contains(&(k.to_string(), source.clone()))
    };
    let waits = d.confirm && live;
    if !waits {
        return Some(("fixed".to_string(), by(d)));
    }
    let id = format!("r{round}.{}", index + 1);
    for c in st
        .ck_confirms
        .iter()
        .rev()
        .filter(|c| c.checkpoint == k && c.source == source && c.seq > d.seq)
    {
        if c.confirmed.contains(&id) {
            return Some((
                "fixed".to_string(),
                t(
                    "ck_confirm.fate.by_confirmed",
                    &[("by", &by(d)), ("round", &c.round.to_string())],
                ),
            ));
        }
        if c.still_open.contains(&id) {
            return None;
        }
    }
    Some(("answered".to_string(), by(d)))
}

/// What an answered finding reads as (the fix that answered it and the source that confirms it), by its id
/// `r<round>.<number>`; none when the finding is not answered.
pub fn answered_text(p: &Project, st: &State, k: &str, id: &str) -> R<Option<String>> {
    let Some((r, n)) = id
        .strip_prefix('r')
        .and_then(|x| x.split_once('.'))
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<usize>().ok()?)))
    else {
        return Ok(None);
    };
    let rs = ckreview::rounds(p, k)?;
    let Some(f) = rs
        .iter()
        .find(|x| x.round == r)
        .and_then(|x| x.findings.get(n.saturating_sub(1)))
    else {
        return Ok(None);
    };
    Ok(match ckreview::fate(st, k, r, n.saturating_sub(1), f) {
        Some((x, _)) if x == "answered" => {
            Some(ckreview::fate_text(st, k, r, n.saturating_sub(1), f))
        }
        _ => None,
    })
}

/// One answered finding of a source.
pub struct Answered {
    pub id: String,
    pub finding: Value,
    pub fix: CkFixDone,
}

/// Every finding of `source` that is answered now, in round order.
pub fn answered(st: &State, rs: &[Round], k: &str, source: &str) -> Vec<Answered> {
    let mut out = Vec::new();
    for r in rs
        .iter()
        .filter(|r| r.source == source && ckreview::reported(r))
    {
        for (i, f) in r.findings.iter().enumerate() {
            let Some((x, _)) = ckreview::fate(st, k, r.round, i, f) else {
                continue;
            };
            if x != "answered" {
                continue;
            }
            let fix = st
                .ck_fixes
                .iter()
                .filter(|d| d.checkpoint == k && names(d, r.round, i, f))
                .rfind(|d| d.confirm)
                .cloned();
            if let Some(fix) = fix {
                out.push(Answered {
                    id: format!("r{}.{}", r.round, i + 1),
                    finding: f.clone(),
                    fix,
                });
            }
        }
    }
    out
}

/// The diff a fix made, per repository of the checkpoint, from the heads it opened at to the heads it ended at.
fn fix_diff(p: &Project, pl: &Plan, k: &str, fix: &CkFixDone) -> R<String> {
    let mut out = String::new();
    for r in run::checkpoint_repos(p, pl, k)? {
        let wt = run::checkpoint_wt(p, pl, k, r)?;
        let (Some((_, from)), Some((_, to))) = (
            fix.before.iter().find(|(n, _)| *n == r.name),
            fix.after.iter().find(|(n, _)| *n == r.name),
        ) else {
            continue;
        };
        out.push_str(&format!(
            "# repository {}\n{}\n",
            r.name,
            gitx::must(&wt, &["diff", from, to]).unwrap_or_default()
        ));
    }
    Ok(out)
}

/// The section of a round's brief that lists the answered findings of its source, each with what it asked, the
/// fix that answered it and that fix's diff, and says how the report names them. Empty when none is answered.
pub fn brief_section(
    p: &Project,
    st: &State,
    pl: &Plan,
    rs: &[Round],
    k: &str,
    source: &str,
) -> R<String> {
    let list = answered(st, rs, k, source);
    if list.is_empty() {
        return Ok(String::new());
    }
    let mut md = format!(
        "{}\n\n{}\n\n",
        t("ck_confirm.brief.title", &[]),
        t(
            "ck_confirm.brief.intro",
            &[("source", &source_text(source))]
        )
    );
    let mut written: Vec<String> = Vec::new();
    for a in &list {
        let path = p.briefs_dir().join(format!(
            "{k}-confirm-{}-{}.diff",
            if source.is_empty() { "review" } else { source },
            a.fix.id
        ));
        if !written.contains(&a.fix.id) {
            crate::util::atomic_write(&path, fix_diff(p, pl, k, &a.fix)?.as_bytes())?;
            written.push(a.fix.id.clone());
        }
        let place = if s(&a.finding, "where").is_empty() {
            s(&a.finding, "file")
        } else {
            s(&a.finding, "where")
        };
        md.push_str(&t(
            "ck_confirm.brief.item",
            &[
                ("id", &a.id),
                ("severity", s(&a.finding, "severity")),
                ("title", s(&a.finding, "title")),
                ("where", place),
                ("action", s(&a.finding, "action")),
                ("fix", &by(&a.fix)),
                ("path", &path.to_string_lossy()),
            ],
        ));
        md.push('\n');
    }
    md.push('\n');
    md.push_str(&t("ck_confirm.brief.form", &[]));
    md.push_str("\n\n");
    Ok(md)
}

/// What a report says of the answered findings: the ids it confirms and the `(id, why)` it keeps open.
#[derive(Default, Clone, Debug)]
pub struct Said {
    pub confirmed: Vec<String>,
    pub still_open: Vec<(String, String)>,
}

impl Said {
    /// The two lists as the round's done event stores them.
    pub fn stored(&self) -> (Value, Value) {
        (
            json!(self.confirmed),
            json!(self
                .still_open
                .iter()
                .map(|(id, why)| json!({"id": id, "why": why}))
                .collect::<Vec<_>>()),
        )
    }
}

fn refuse_with(k: &str, source: &str, why: String, step: String) -> crate::refusal::Fail {
    refuse_owned(
        t(
            "ck_confirm.what.report",
            &[("k", k), ("source", &source_text(source))],
        ),
        why,
        vec![step],
        t("ck_confirm.refuse.check", &[]),
    )
}

/// Read `confirmed` and `still_open` from a report and check them against the answered findings of its source:
/// each answered finding is named once, in one of the two lists, and no other finding is named.
pub fn check_report(
    st: &State,
    rs: &[Round],
    k: &str,
    source: &str,
    report: Option<&Value>,
) -> R<Said> {
    let bad_shape = |field: &str| {
        refuse_with(
            k,
            source,
            t("ck_confirm.refuse.shape.why", &[("field", field)]),
            t("ck_confirm.refuse.shape.step", &[]),
        )
    };
    let mut said = Said::default();
    if let Some(v) = report
        .and_then(|r| r.get("confirmed"))
        .filter(|v| !v.is_null())
    {
        let Some(a) = v.as_array() else {
            return Err(bad_shape("confirmed"));
        };
        for x in a {
            let Some(id) = x.as_str() else {
                return Err(bad_shape("confirmed"));
            };
            said.confirmed.push(id.trim().to_string());
        }
    }
    if let Some(v) = report
        .and_then(|r| r.get("still_open"))
        .filter(|v| !v.is_null())
    {
        let Some(a) = v.as_array() else {
            return Err(bad_shape("still_open"));
        };
        for x in a {
            let id = s(x, "id").trim().to_string();
            if !x.is_object() || id.is_empty() {
                return Err(bad_shape("still_open"));
            }
            let why = s(x, "why").trim().to_string();
            if why.is_empty() {
                return Err(refuse_with(
                    k,
                    source,
                    t("ck_confirm.refuse.no_why.why", &[("id", &id)]),
                    t("ck_confirm.refuse.no_why.step", &[("id", &id)]),
                ));
            }
            said.still_open.push((id, why));
        }
    }
    let list = answered(st, rs, k, source);
    let mut seen: Vec<&str> = Vec::new();
    for id in said
        .confirmed
        .iter()
        .map(|x| x.as_str())
        .chain(said.still_open.iter().map(|(x, _)| x.as_str()))
    {
        if seen.contains(&id) {
            return Err(refuse_with(
                k,
                source,
                t("ck_confirm.refuse.twice.why", &[("id", id)]),
                t("ck_confirm.refuse.twice.step", &[("id", id)]),
            ));
        }
        seen.push(id);
        if !list.iter().any(|a| a.id == id) {
            return Err(refuse_with(
                k,
                source,
                t(
                    "ck_confirm.refuse.foreign.why",
                    &[("id", id), ("source", &source_text(source))],
                ),
                t("ck_confirm.refuse.foreign.step", &[("id", id)]),
            ));
        }
    }
    for a in &list {
        if !seen.contains(&a.id.as_str()) {
            return Err(refuse_with(
                k,
                source,
                t("ck_confirm.refuse.missing.why", &[("id", &a.id)]),
                t("ck_confirm.refuse.missing.step", &[("id", &a.id)]),
            ));
        }
    }
    Ok(said)
}

/// Whether a report keeps a blocking finding open: one of its `still_open` ids is a high or medium finding.
pub fn reopens_blocking(rs: &[Round], said: &Said) -> bool {
    said.still_open.iter().any(|(id, _)| {
        let parsed = id
            .strip_prefix('r')
            .and_then(|x| x.split_once('.'))
            .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<usize>().ok()?)));
        parsed
            .and_then(|(round, n)| {
                rs.iter()
                    .find(|r| r.round == round)
                    .and_then(|r| r.findings.get(n.wrapping_sub(1)))
            })
            .is_some_and(ckreview::blocking)
    })
}

/// A fix review's report may not end a finding of the checkpoint: it names none as confirmed or fixed.
pub fn refuse_fix_review_ends(k: &str, report: Option<&Value>) -> R<()> {
    for field in ["confirmed", "fixed"] {
        let named = report
            .and_then(|r| r.get(field))
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()));
        if named {
            return Err(refuse_owned(
                t("ck_confirm.what.fix_review", &[("k", k)]),
                t("ck_confirm.refuse.fix_review.why", &[("field", field)]),
                vec![t(
                    "ck_confirm.refuse.fix_review.step",
                    &[("k", k), ("field", field)],
                )],
                t("ck_confirm.refuse.check", &[]),
            ));
        }
    }
    Ok(())
}
