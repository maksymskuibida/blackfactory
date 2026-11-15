//! The checkpoint review part (C-18). A checkpoint cannot close until a cold review of the whole
//! checkpoint has run on the content being closed, by one reviewer on the strong tier. The engine writes
//! the reviewer's brief from the checkpoint's whole diff and the brief of every unit on it, gives the
//! reviewer a copy of its own to build, run and probe in (a detached worktree that is thrown away and whose
//! branches it cannot move), takes its findings and hands them to the checkpoint fix.
//!
//! What the reviewer runs goes into the evidence: a run of one of the checkpoint's own checks is a normal
//! receipt, any other command is a probe run that settles nothing, a repeat of the same run needs its
//! attempt number and a reason (`evidence`), and a pass after a failure counts only with the reviewer's
//! recorded judgement. The review is the named check `checkpoint-review`; the human can switch it off with
//! a reason, like any named check. The review rounds are kept in the run log.

use crate::checks::CHECKPOINT_REVIEW as CHECK;
use crate::events::{self, State};
use crate::evidence::{self, Receipt};
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, refuse_owned, R};
use crate::run;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One round of the review of a checkpoint, as the log has it.
#[derive(Clone, Debug, Default)]
pub struct Round {
    pub checkpoint: String,
    pub round: u32,
    /// The content the round judges.
    pub content: String,
    /// The commit of each repository's checkpoint branch when the round opened.
    pub heads: Vec<(String, String)>,
    pub covered: String,
    /// `clean`, `findings` or `void` (the content changed before the reviewer reported); none while open.
    pub result: Option<String>,
    pub findings: Vec<Value>,
    /// The areas the reviewer probed, each with how: read, built and run, or probed with a test of its own.
    pub areas: Vec<Value>,
    /// The probe tests the reviewer wrote: name, what it was meant to show, what it showed, where it is kept.
    pub probes: Vec<Value>,
}

/// Every review round of `k`, in log order.
pub fn rounds(p: &Project, k: &str) -> R<Vec<Round>> {
    let log = events::load_log(p)?;
    let mut out: Vec<Round> = Vec::new();
    for ev in &log.events {
        if ev["checkpoint"].as_str() != Some(k) {
            continue;
        }
        match ev["kind"].as_str() {
            Some("checkpoint.review.open") => out.push(Round {
                checkpoint: k.to_string(),
                round: ev["round"].as_u64().unwrap_or(0) as u32,
                content: s(ev, "content").to_string(),
                heads: ev["heads"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|h| (s(h, "repo").to_string(), s(h, "head").to_string()))
                    .collect(),
                covered: s(ev, "covered").to_string(),
                ..Default::default()
            }),
            Some("checkpoint.review.done") => {
                let n = ev["round"].as_u64().unwrap_or(0) as u32;
                if let Some(r) = out.iter_mut().rev().find(|r| r.round == n) {
                    r.result = Some(s(ev, "result").to_string());
                    r.findings = ev["findings"].as_array().cloned().unwrap_or_default();
                    r.areas = ev["areas"].as_array().cloned().unwrap_or_default();
                    r.probes = ev["probes"].as_array().cloned().unwrap_or_default();
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Whether the closing run requires the checkpoint review (the human can switch it off).
pub fn required(st: &State) -> bool {
    crate::enforce::required_closing(st)
        .iter()
        .any(|c| c.name == CHECK)
}

/// Whether a finding stops the checkpoint from closing while it is open.
fn blocking(f: &Value) -> bool {
    matches!(s(f, "severity"), "critical" | "high" | "medium")
}

/// What became of a finding: `fixed` (with the unit or units that fixed it), `accepted` by the human, or
/// `declined` with a reason. None while it is open.
pub fn fate(st: &State, k: &str, round: u32, index: usize, f: &Value) -> Option<(String, String)> {
    if let Some(x) = st
        .ck_settled
        .iter()
        .rev()
        .find(|x| x.checkpoint == k && x.round == round && x.index as usize == index + 1)
    {
        return Some((x.fate.clone(), x.reason.clone()));
    }
    st.ck_fixes
        .iter()
        .find(|d| {
            d.checkpoint == k
                && if d.ids.is_empty() {
                    // Recorded before the pairs were kept: matched by value, but only findings of the
                    // rounds no later than the one the fix was opened from.
                    round <= d.from_round && d.origin_findings.contains(f)
                } else {
                    d.ids.contains(&(round, index as u32 + 1))
                }
        })
        .map(|d| {
            let by = if d.units.is_empty() {
                d.id.clone()
            } else {
                format!("{} ({})", d.units.join(", "), d.id)
            };
            ("fixed".to_string(), by)
        })
}

/// The fate of a finding as a sentence: `open` while it has none.
pub fn fate_text(st: &State, k: &str, round: u32, index: usize, f: &Value) -> String {
    match fate(st, k, round, index, f) {
        None => "open".to_string(),
        Some((x, why)) if x == "fixed" => format!("fixed by {why}"),
        Some((x, why)) if x == "accepted" => {
            if why.is_empty() {
                "accepted by the human".to_string()
            } else {
                format!("accepted by the human: {why}")
            }
        }
        Some((_, why)) => format!("declined: {why}"),
    }
}

/// The findings of a round that are open: (number inside the round from 1, the finding).
fn open_findings<'a>(st: &State, k: &str, r: &'a Round) -> Vec<(usize, &'a Value)> {
    r.findings
        .iter()
        .enumerate()
        .filter(|(i, f)| fate(st, k, r.round, *i, f).is_none())
        .map(|(i, f)| (i + 1, f))
        .collect()
}

/// Whether a round finished with a report (not void, not still open).
fn reported(r: &Round) -> bool {
    matches!(r.result.as_deref(), Some("clean") | Some("findings"))
}

/// The reviews the checkpoint has had since the human last released it.
fn reviews(st: &State, k: &str, rs: &[Round]) -> u32 {
    let floor = st.ck_review_floor.get(k).copied().unwrap_or(0);
    rs.iter().filter(|r| reported(r) && r.round > floor).count() as u32
}

/// The high and medium findings still open in any review of the checkpoint: (round, number inside the
/// round, the finding, the content the round judged). A release moves the floor only for counting reviews;
/// the findings it sent to a fix keep holding the close until that fix passes its review or the human
/// settles them.
fn open_blocking(st: &State, k: &str, rs: &[Round]) -> Vec<(u32, usize, Value, String)> {
    rs.iter()
        .filter(|r| reported(r))
        .flat_map(|r| {
            open_findings(st, k, r)
                .into_iter()
                .filter(|(_, f)| blocking(f))
                .map(|(i, f)| (r.round, i, f.clone(), r.content.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether the review at exactly this content let the checkpoint go on: it filed no high and no medium
/// finding, or each such finding has since been fixed, accepted or declined, and no high or medium finding
/// of any review since the human last released the checkpoint is open.
pub fn passed_at(p: &Project, st: &State, k: &str, content: &str) -> R<bool> {
    let rs = rounds(p, k)?;
    Ok(rs
        .iter()
        .filter(|r| reported(r) && r.content == content)
        .any(|r| !open_findings(st, k, r).iter().any(|(_, f)| blocking(f)))
        && open_blocking(st, k, &rs).is_empty())
}

/// Whether the checkpoint still needs its review before it can close.
pub fn needed(p: &Project, st: &State, k: &str, content: &str) -> R<bool> {
    Ok(required(st) && !passed_at(p, st, k, content)?)
}

/// Whether the checkpoint is parked: it has had the number of reviews the policy allows, and a high or
/// medium finding of a review since the human last released it is still open. Only the human releases it.
pub fn parked(p: &Project, st: &State, k: &str) -> R<bool> {
    let rs = rounds(p, k)?;
    Ok(reviews(st, k, &rs) >= p.policy.limits.checkpoint_reviews
        && !open_blocking(st, k, &rs).is_empty())
}

/// The low findings of the latest review that nobody has settled yet, for the human.
pub fn open_low(p: &Project, st: &State, k: &str) -> R<Vec<Value>> {
    let rs = rounds(p, k)?;
    let Some(last) = rs.iter().rev().find(|r| reported(r)) else {
        return Ok(vec![]);
    };
    Ok(open_findings(st, k, last)
        .into_iter()
        .filter(|(_, f)| !blocking(f))
        .map(|(i, f)| json!({"id": format!("r{}.{}", last.round, i), "title": s(f, "title"), "where": s(f, "where"), "action": s(f, "action")}))
        .collect())
}

pub fn copy_path(p: &Project, k: &str) -> PathBuf {
    p.work_dir().join(format!("{k}-review"))
}

fn open_round(rs: &[Round]) -> Option<&Round> {
    rs.last().filter(|r| r.result.is_none())
}

/// The directive for the reviewer of a checkpoint, or for the human when the rounds are used up.
pub fn directive(p: &Project, st: &State, k: &str) -> R<Value> {
    let rs = rounds(p, k)?;
    if parked(p, st, k)? {
        return Ok(
            json!({"role": "human", "action": "decide-checkpoint-review", "checkpoint": k,
            "note": format!("{k} is parked: it had {} reviews and a high or medium finding is still open. The human decides: release it in their own words (`bf checkpoint review {k} --release --quote \"<their words>\"`) so the findings go to a fix, accept or decline the findings (`bf checkpoint finding {k} --accept <ids> --quote \"<their words>\"`), or switch the checkpoint review off for the run with a reason (`bf checks propose --scope run {CHECK}=off:<why>`, then `bf checks set --quote \"<their words>\"`).", reviews(st, k, &rs))}),
        );
    }
    // A fix that ended without passing its review leaves its findings open. No new review is offered at
    // the content they were filed on while no fix is open: the human says what becomes of them.
    if st.ck_fix.is_none() && open_round(&rs).is_none() {
        let content = cp_content(p, k)?;
        let open: Vec<Value> = open_blocking(st, k, &rs)
            .into_iter()
            .filter(|(_, _, _, c)| *c == content)
            .map(|(r, i, f, _)| json!({"id": format!("r{r}.{i}"), "severity": s(&f, "severity"), "title": s(&f, "title"), "where": s(&f, "where"), "action": s(&f, "action")}))
            .collect();
        if !open.is_empty() {
            return Ok(
                json!({"role": "human", "action": "decide-checkpoint-fix", "checkpoint": k, "findings": open,
                "note": format!("The fix of a high or medium finding of {k} ended without passing its review, so the finding is still open and holds the close. No new review is offered at this content. The human decides: open the fix again (`bf checkpoint finding {k} --fix-findings <ids>`), or accept (`bf checkpoint finding {k} --accept <ids> --quote \"<their words>\"`) or decline (`bf checkpoint finding {k} --decline <ids> --reason \"<why>\" --quote \"<their words>\"`) each finding in their own words."),
                "commands": [format!("bf checkpoint finding {k} --fix-findings <ids>")]}),
            );
        }
    }
    let round = open_round(&rs)
        .map(|r| r.round)
        .unwrap_or(rs.len() as u32 + 1);
    Ok(
        json!({"role": "reviewer", "action": "review-checkpoint", "checkpoint": k,
        "tier": p.policy.tiers.reviewer, "round": round,
        "working_copy": copy_path(p, k).to_string_lossy(),
        "brief": p.briefs_dir().join(format!("{k}-reviewer.md")).to_string_lossy(),
        "limits": {"max_rounds": p.policy.limits.checkpoint_reviews, "round": reviews(st, k, &rs), "idle_minutes": p.policy.limits.idle_minutes,
                   "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }},
        "commands": [format!("bf brief {k} --role reviewer")]}),
    )
}

/// The directive that lists the low findings of the latest review for the human: accept them together, or
/// pick some to be fixed. None when there are none. They never hold the checkpoint back.
pub fn low_directive(p: &Project, st: &State, k: &str) -> R<Option<Value>> {
    let low = open_low(p, st, k)?;
    if low.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        json!({"role": "human", "action": "decide-low-findings", "checkpoint": k, "findings": low,
        "note": format!("The latest review of {k} filed low findings only (they do not hold the checkpoint back). The human accepts them together (`bf checkpoint finding {k} --accept low --quote \"<their words>\"`), declines some with a reason, or picks some to be fixed (`bf checkpoint finding {k} --fix-findings <ids>`)."),
        "commands": [format!("bf checkpoint finding {k} --accept low --quote \"<the human's words>\"")]}),
    ))
}

fn heads(p: &Project, pl: &Plan, k: &str) -> R<Vec<(String, String)>> {
    let mut out = Vec::new();
    for r in run::checkpoint_repos(p, pl, k)? {
        let wt = run::checkpoint_wt(p, pl, k, r)?;
        out.push((
            r.name.clone(),
            gitx::must(&wt, &["rev-parse", "HEAD"]).unwrap_or_default(),
        ));
    }
    Ok(out)
}

fn heads_json(h: &[(String, String)]) -> Value {
    json!(h
        .iter()
        .map(|(r, c)| json!({"repo": r, "head": c}))
        .collect::<Vec<_>>())
}

/// Throw the reviewer's copy away.
pub fn remove_copy(p: &Project, pl: &Plan, k: &str) {
    let root = copy_path(p, k);
    if !root.exists() {
        return;
    }
    if let Ok(repos) = run::checkpoint_repos(p, pl, k) {
        let several = repos.len() > 1;
        for r in repos {
            let dest = if several {
                root.join(&r.name)
            } else {
                root.clone()
            };
            let _ = gitx::run(
                &r.path,
                &["worktree", "remove", "--force", &dest.to_string_lossy()],
                &[],
            );
            let _ = gitx::run(&r.path, &["worktree", "prune"], &[]);
        }
    }
    let _ = fs::remove_dir_all(&root);
}

/// A fresh copy of the checkpoint's content: a detached worktree of each repository at the commit the
/// checkpoint branch stands on, so nothing done in it reaches the branch.
fn make_copy(p: &Project, pl: &Plan, k: &str) -> R<PathBuf> {
    remove_copy(p, pl, k);
    let root = copy_path(p, k);
    let repos = run::checkpoint_repos(p, pl, k)?;
    let several = repos.len() > 1;
    for r in repos {
        let wt = run::checkpoint_wt(p, pl, k, r)?;
        let head = gitx::must(&wt, &["rev-parse", "HEAD"])?;
        let dest = if several {
            root.join(&r.name)
        } else {
            root.clone()
        };
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        gitx::must(
            &r.path,
            &[
                "worktree",
                "add",
                "--detach",
                &dest.to_string_lossy(),
                &head,
            ],
        )?;
    }
    Ok(root)
}

fn cp_content(p: &Project, k: &str) -> R<String> {
    p.content_hash(&p.work_dir().join(k))
}

fn refuse_not_ready(k: &str, because: &str) -> crate::refusal::Fail {
    refuse(
        format!("review {k}"),
        because.to_string(),
        &["`bf next` shows what the checkpoint waits for."],
        "`bf next` gives the reviewer's directive.",
    )
}

/// The fate of every finding of a round, for the status and the page.
fn outcomes_json(st: &State, k: &str, r: &Round) -> Value {
    json!(r
        .findings
        .iter()
        .enumerate()
        .map(
            |(i, f)| json!({"id": format!("r{}.{}", r.round, i + 1), "title": s(f, "title"),
            "severity": s(f, "severity"), "fate": fate_text(st, k, r.round, i, f)})
        )
        .collect::<Vec<_>>())
}

/// Lines that say, for each round, the areas it probed and how, and the probes it kept.
fn report_lines(r: &Round) -> String {
    let mut t = String::new();
    for a in &r.areas {
        t.push_str(&format!("  - Area: {} ({})\n", s(a, "area"), s(a, "how")));
    }
    for x in &r.probes {
        t.push_str(&format!(
            "  - Probe {}: meant to show: {}. Showed: {}. Kept in `{}`.\n",
            s(x, "name"),
            s(x, "meant"),
            s(x, "showed"),
            s(x, "stored")
        ));
    }
    t
}

/// The areas every review before round `n` probed, in order, without repeats: the angles already taken.
fn areas_taken(rs: &[Round], n: u32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in rs.iter().filter(|r| r.round < n && reported(r)) {
        for a in &r.areas {
            let x = s(a, "area").to_string();
            if !out.contains(&x) {
                out.push(x);
            }
        }
    }
    out
}

fn read_unit_brief(p: &Project, u: &str) -> String {
    let path = p.briefs_dir().join(format!("{u}-builder.md"));
    if let Ok(t) = fs::read_to_string(&path) {
        return t;
    }
    match run::brief(p, u, "builder").and_then(|f| Ok(fs::read_to_string(f)?)) {
        Ok(t) => t,
        Err(_) => String::from("(the brief of this unit could not be written)\n"),
    }
}

/// `bf brief <K> --role reviewer` for the review of a whole checkpoint: opens the round, makes the
/// reviewer's copy and writes the brief.
pub fn brief(p: &Project, k: &str) -> R<PathBuf> {
    let _lock = crate::lock::engine(p, &format!("brief the review of {k}"))?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let pl = plan::load(p)?;
    let Some(kr) = pl.checkpoint(k).cloned() else {
        return Err(refuse_not_ready(k, "no such checkpoint"));
    };
    let after = str_list(&kr, "after");
    if st.closed.contains_key(k) {
        return Err(refuse_not_ready(k, "the checkpoint is closed"));
    }
    if after.is_empty() || !after.iter().all(|u| st.unit_state(u) == "landed") {
        return Err(refuse_not_ready(k, "its units have not all landed"));
    }
    if !required(&st) {
        return Err(refuse_not_ready(
            k,
            "the checkpoint review is switched off for this run",
        ));
    }
    let wc = p.work_dir().join(k);
    let content = p.content_hash(&wc)?;
    if passed_at(p, &st, k, &content)? {
        return Err(refuse_not_ready(
            k,
            "the review already let the checkpoint go on at this content",
        ));
    }
    let mut rs = rounds(p, k)?;
    if parked(p, &st, k)? {
        return Err(refuse_not_ready(
            k,
            "the checkpoint is parked after the reviews the policy allows; the human decides",
        ));
    }
    // A round still open on other content is void: the reviewer judged what is no longer there.
    if let Some(o) = open_round(&rs).filter(|o| o.content != content).cloned() {
        events::append(
            p,
            "checkpoint.review.done",
            json!({"checkpoint": k, "round": o.round, "content": o.content, "result": "void", "findings": []}),
        )?;
        rs = rounds(p, k)?;
    }
    let n = match open_round(&rs) {
        Some(o) => o.round,
        None => {
            let n = rs.last().map(|r| r.round).unwrap_or(0) + 1;
            let earlier = rs.iter().rev().find(|r| reported(r));
            let covered = match earlier {
                None => "the whole checkpoint, cold".to_string(),
                Some(e) => format!(
                    "the whole checkpoint, the report of round {} with what became of each finding, the probes kept, and what changed since round {}",
                    e.round, e.round
                ),
            };
            events::append(
                p,
                "checkpoint.review.open",
                json!({"checkpoint": k, "round": n, "content": content,
                       "heads": heads_json(&heads(p, &pl, k)?), "covered": covered}),
            )?;
            n
        }
    };
    rs = rounds(p, k)?;
    let copy = make_copy(p, &pl, k)?;
    fs::create_dir_all(p.briefs_dir())?;

    // The whole diff of the checkpoint: against the checkpoint before it, or the main branch.
    let prev = run::previous_checkpoint(&pl, k).filter(|c| st.closed.contains_key(c));
    let mut stat = String::new();
    let mut diff = String::new();
    for r in run::checkpoint_repos(p, &pl, k)? {
        let wt = run::checkpoint_wt(p, &pl, k, r)?;
        let base = match &prev {
            Some(c) if gitx::branch_exists(&r.path, &p.checkpoint_branch(c)) => {
                p.checkpoint_branch(c)
            }
            _ => r.branch.clone(),
        };
        stat.push_str(&format!(
            "### {}\n\n{}\n\n",
            r.name,
            gitx::must(&wt, &["diff", "--stat", &base, "HEAD"]).unwrap_or_default()
        ));
        diff.push_str(&format!(
            "# repository {}\n{}\n",
            r.name,
            gitx::must(&wt, &["diff", &base, "HEAD"]).unwrap_or_default()
        ));
    }
    let diff_path = p.briefs_dir().join(format!("{k}-review.diff"));
    util::atomic_write(&diff_path, diff.as_bytes())?;

    let mut md = format!(
        "# Brief: {k} as reviewer (checkpoint review, round {n})\n\nCheckpoint: {}\n\n",
        s(&kr, "title")
    );
    let is_repeat = rs.iter().any(|r| r.round < n && reported(r));
    md.push_str(&format!(
        "You review the whole checkpoint{}, on the strong tier. Round {n} covers: {}.\n\n",
        if is_repeat { "" } else { " cold" },
        rs.iter()
            .find(|r| r.round == n)
            .map(|r| r.covered.as_str())
            .unwrap_or("")
    ));
    md.push_str(&format!(
        "## Your copy\n\nBuild, run and probe in `{}`. It is a throwaway copy of the checkpoint's content: nothing in it reaches the checkpoint, its branch or the project. Work only there.\n\n",
        copy.display()
    ));
    md.push_str(&format!(
        "## The whole diff\n\nThe full diff is in `{}`. Read it all, and read beyond it whenever you need to.\n\n{stat}",
        diff_path.display()
    ));
    md.push_str("## What the checkpoint proves\n\n");
    for id in str_list(&kr, "proves") {
        if let Some(r) = p.get_record(&id)? {
            md.push_str(&format!("- {} {}\n", id, s(&r, "title")));
            for a in str_list(&r, "acceptance") {
                md.push_str(&format!("  - Acceptance: {a}\n"));
            }
        }
    }
    md.push('\n');
    md.push_str(&crate::scope::grants_md_checkpoint(&st, &pl, k));
    md.push_str("## The brief of every unit on this checkpoint\n\n");
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let id = s(u, "id");
        md.push_str(&format!(
            "<details><summary>{id}</summary>\n\n{}\n</details>\n\n",
            read_unit_brief(p, id)
        ));
    }
    // Earlier rounds: the findings with their outcomes, and what changed since the round before.
    let earlier: Vec<&Round> = rs.iter().filter(|r| r.round < n && reported(r)).collect();
    if !earlier.is_empty() {
        md.push_str("## Earlier rounds\n\nThe scope of this round is the whole checkpoint, not only what changed. Work in this order: first confirm that each finding marked fixed below is fixed; then review the code that changed; then take angles the earlier reviews did not take (the areas below tell you which they took). File only new problems: do not file again a finding the human accepted or declined, or one that is open and listed here. A finding that is listed here as open stays open and holds the close until a fix opened for it passes its review or the human settles it, so it is not filed again; a finding filed again word for word is a new finding of its own and stays open on its own. Anything a fix broke is new.\n\n");
        for r in &earlier {
            md.push_str(&format!(
                "Round {} judged content {}:\n",
                r.round,
                &r.content[..12.min(r.content.len())]
            ));
            md.push_str(&report_lines(r));
            if r.findings.is_empty() {
                md.push_str("- It filed no finding.\n");
            }
            for (i, f) in r.findings.iter().enumerate() {
                md.push_str(&format!(
                    "- r{}.{} [{}] {} ({}): {}. Outcome: {}\n",
                    r.round,
                    i + 1,
                    s(f, "severity"),
                    s(f, "title"),
                    if s(f, "where").is_empty() {
                        s(f, "file")
                    } else {
                        s(f, "where")
                    },
                    s(f, "action"),
                    fate_text(&st, k, r.round, i, f)
                ));
            }
        }
        let taken = areas_taken(&rs, n);
        md.push_str(&format!(
            "\nAreas the reviews before you probed: {}.\n",
            if taken.is_empty() {
                "none listed".to_string()
            } else {
                taken.join("; ")
            }
        ));
        if let Some(last) = rs.iter().rev().find(|r| r.round < n && !r.heads.is_empty()) {
            let mut since = String::new();
            for r in run::checkpoint_repos(p, &pl, k)? {
                let wt = run::checkpoint_wt(p, &pl, k, r)?;
                if let Some((_, old)) = last.heads.iter().find(|(n, _)| *n == r.name) {
                    since.push_str(&format!(
                        "# repository {}\n{}\n",
                        r.name,
                        gitx::must(&wt, &["diff", old, "HEAD"]).unwrap_or_default()
                    ));
                }
            }
            let since_path = p.briefs_dir().join(format!("{k}-review-since.diff"));
            util::atomic_write(&since_path, since.as_bytes())?;
            md.push_str(&format!(
                "\nThe changes since the content round {} judged are in `{}`.\n",
                last.round,
                since_path.display()
            ));
        }
        md.push('\n');
    }
    // The verification commands.
    md.push_str("## Verification commands\n\nRun these through the engine, from inside your copy, and never on your own. Run each at most once: a check that already has a passing receipt at this content is not run again. A repeat needs `--attempt <n> --reason \"<why>\"`.\n\n");
    let receipts = evidence::load_receipts(p)?;
    for cid in str_list(&kr, "checks") {
        let Some(c) = pl.checks.iter().find(|c| s(c, "id") == cid) else {
            continue;
        };
        if crate::enforce::is_off(&st, c) || s(c, "how") != "command" {
            continue;
        }
        let cmd = evidence::command_words(c).join(" ");
        if evidence::satisfied_at(&receipts, &cid, &content).is_some() {
            md.push_str(&format!(
                "- {cid} ({}): already has a passing receipt at this content; do not run it again\n",
                s(c, "title")
            ));
        } else {
            md.push_str(&format!(
                "- {cid} ({}): `bf evidence run {cid} -- {cmd}`\n",
                s(c, "title")
            ));
        }
    }
    md.push_str(&format!("\nAnything else you run (a probe test, a build to look at) goes through `bf evidence run --probe {k} -- <command>`. It is recorded and settles nothing. A check that failed and then passed on a deliberate repeat at the same content needs your judgement before it counts: `bf evidence run <check> --judge flaky|failure|other --reason \"<what you saw>\" --token <token>`. A check judged by an agent stays with its own verifier: you do not settle it.\n\n"));
    md.push_str("## Try to get around the rules\n\nFor each new rule this checkpoint adds (the requirements above), try to get around it: find the input, the order of steps, the file or the command that the rule does not hold against, and show it with a probe. A rule you could not get around is worth saying so.\n\n");
    md.push_str(&format!(
        "## Report\n\nA report says which areas you probed and how, and not only what you found. Each area carries `how`: `read`, `built and run`, or `probed with a test of its own`. A report with no areas is refused. Findings carry a title, a severity (critical, high, medium or low), `where` (the file and line) and `action`. A finding that a probe exposed names it in `probe`. Every probe test you wrote goes in `probes` with `name`, `meant` (what it was meant to show), `showed` (what it showed), `file` (its path inside your copy), `command` (how to run it: give a command that fails when it runs no test, such as `cargo nextest run` with a filter) and, for a probe that exposed a fault, `expect` (the right answer, which the fix must make the test give; `expect_exit` is the exit code that means so, 0 by default). A probe holds no name of a person or a private project, no home path and no private link.\n\n```\nbf report {k} --result findings --role reviewer --token <token> --findings - <<'EOF'\n{{\"areas\": [{{\"area\": \"the close rule\", \"how\": \"read\"}}],\n \"probes\": [{{\"name\": \"p1\", \"meant\": \"...\", \"showed\": \"...\", \"file\": \"tests/p1.rs\", \"command\": \"cargo test p1\", \"expect\": \"...\"}}],\n \"findings\": [{{\"title\": \"...\", \"severity\": \"medium\", \"where\": \"src/a.rs:10\", \"action\": \"...\", \"probe\": \"p1\"}}]}}\nEOF\n```\n\nA high or medium finding opens the checkpoint fix, and the fix adds each probe that exposed it to the suite as a permanent test. Low findings do not hold the checkpoint back: the human accepts them together or picks some to be fixed. With nothing to file, give an empty `findings` list: the report needs a passing receipt for every command check of the checkpoint at this content.\n\n## Rules\n\n- Never edit anything under `factory/` or `.bf/`, apart from your own copy.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot judge without the human's decision, ask: `bf ask --title \"<the question>\"`.\n"
    ));
    let path = p.briefs_dir().join(format!("{k}-reviewer.md"));
    util::atomic_write(&path, md.as_bytes())?;
    p.write_guard()?;
    Ok(path)
}

/// Whether `bf report <K>` is the checkpoint reviewer's report on an open review round.
pub fn wants(p: &Project, st: &State, k: &str, role: Option<&str>) -> bool {
    st.ck_fix.is_none()
        && st.sync_open.is_none()
        && role == Some("reviewer")
        && rounds(p, k)
            .map(|rs| open_round(&rs).is_some())
            .unwrap_or(false)
}

const SEVERITIES: [&str; 4] = ["critical", "high", "medium", "low"];

fn finding_list(findings: Option<&Value>) -> Vec<Value> {
    findings
        .and_then(|f| {
            f.as_array()
                .cloned()
                .or_else(|| f.get("findings").and_then(|x| x.as_array().cloned()))
        })
        .unwrap_or_default()
}

fn object_list(findings: Option<&Value>, key: &str) -> Vec<Value> {
    findings
        .and_then(|f| f.get(key))
        .and_then(|x| x.as_array().cloned())
        .unwrap_or_default()
}

/// How an area was probed, in one of the three words the report may use.
fn canonical_how(how: &str) -> Option<&'static str> {
    let h = how
        .trim()
        .to_lowercase()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match h.as_str() {
        "read" => Some("read"),
        "built and run" | "built" | "run" => Some("built and run"),
        "probed with a test of its own" | "probed with a test" | "probed" | "probe" => {
            Some("probed with a test of its own")
        }
        _ => None,
    }
}

/// The areas of a report, each with its `how` in canonical form. A report with none is refused.
fn check_areas(k: &str, raw: Vec<Value>) -> R<Vec<Value>> {
    let mut out = Vec::new();
    for (i, a) in raw.iter().enumerate() {
        let name = s(a, "area").trim().to_string();
        match (name.is_empty(), canonical_how(s(a, "how"))) {
            (false, Some(how)) => out.push(json!({"area": name, "how": how})),
            _ => {
                return Err(refuse(
                    format!("report on the review of {k}"),
                    format!("area {} is not complete: it needs `area` and a `how` of read, built and run, or probed with a test of its own", i + 1),
                    &["Give each area as `{\"area\": \"what you looked at\", \"how\": \"read\"}`."],
                    "The command records the review.",
                ));
            }
        }
    }
    if out.is_empty() {
        return Err(refuse(
            format!("report on the review of {k}"),
            "the report does not say which areas the reviewer probed and how",
            &["Give `areas` beside the findings: a JSON object `{\"areas\": [{\"area\": \"...\", \"how\": \"read\"}], \"findings\": [...]}` through `--findings -`. `how` is read, built and run, or probed with a test of its own."],
            "The command records the review with its areas.",
        ));
    }
    Ok(out)
}

/// A word of `text` that is `name`, whole and in any case.
fn has_word(text: &str, name: &str) -> bool {
    let (t, n) = (text.to_lowercase(), name.to_lowercase());
    t.match_indices(&n).any(|(i, m)| {
        let before = t[..i].chars().next_back();
        let after = t[i + m.len()..].chars().next();
        !before.map(|c| c.is_alphanumeric()).unwrap_or(false)
            && !after.map(|c| c.is_alphanumeric()).unwrap_or(false)
    })
}

/// What a stored probe must not hold: a name of a person, a home path, an e-mail address or a link.
fn leak_in(p: &Project, text: &str) -> Option<&'static str> {
    let low = text.to_lowercase();
    if ["/users/", "/home/", "c:\\users", "~/"]
        .iter()
        .any(|x| low.contains(x))
    {
        return Some("a home path");
    }
    if ["http", "https", "file", "ssh"]
        .iter()
        .any(|x| low.contains(&format!("{x}:{}", "//")))
        || low.contains("git@")
    {
        return Some("a link");
    }
    let bytes: Vec<char> = text.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if *c == '@'
            && i > 0
            && bytes[i - 1].is_alphanumeric()
            && bytes
                .get(i + 1..)
                .map(|r| {
                    let w: String = r.iter().take_while(|x| !x.is_whitespace()).collect();
                    w.contains('.')
                        && w.chars()
                            .next()
                            .map(|x| x.is_alphanumeric())
                            .unwrap_or(false)
                })
                .unwrap_or(false)
        {
            return Some("an e-mail address");
        }
    }
    let mut names: Vec<String> = ["USER", "LOGNAME"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .collect();
    if let Ok(o) = gitx::run(&p.top, &["config", "--global", "user.name"], &[]) {
        if o.ok {
            names.push(o.stdout.trim().to_string());
        }
    }
    for n in names.iter().filter(|n| n.chars().count() >= 4) {
        if has_word(text, n) {
            return Some("a person's name");
        }
    }
    None
}

fn refuse_probe(k: &str, name: &str, because: String) -> crate::refusal::Fail {
    refuse(
        format!("report on the review of {k}"),
        format!("probe `{name}`: {because}"),
        &["A probe holds no name of a person or a private project, no home path and no private link. Give each probe `name`, `meant`, `showed`, and `file` and `command` when a finding names it."],
        "The command records the review with its probes.",
    )
}

/// The probes of a report, checked, with their files read from the reviewer's copy and stored as evidence
/// files of the round. Returns the probes as stored with the review.
fn store_probes(p: &Project, k: &str, round: u32, raw: &[Value], list: &[Value]) -> R<Vec<Value>> {
    let copy = copy_path(p, k);
    let dir = p
        .evidence_dir()
        .join("probes")
        .join(format!("{k}-review-{round}"));
    let mut out: Vec<Value> = Vec::new();
    for x in raw {
        let name = s(x, "name").trim().to_string();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            return Err(refuse_probe(
                k,
                &name,
                "its name is empty or has characters beyond letters, digits, `_`, `-` and `.`"
                    .into(),
            ));
        }
        if out.iter().any(|o| s(o, "name") == name) {
            return Err(refuse_probe(k, &name, "two probes have this name".into()));
        }
        for key in ["meant", "showed"] {
            if s(x, key).trim().is_empty() {
                return Err(refuse_probe(k, &name, format!("it has no `{key}`")));
            }
        }
        let named_by_finding = list.iter().any(|f| s(f, "probe") == name);
        let file = s(x, "file").trim().to_string();
        let command = s(x, "command").trim().to_string();
        let expect = s(x, "expect").trim().to_string();
        let words: Vec<String> = command.split_whitespace().map(|w| w.to_string()).collect();
        if let Some(w) = crate::evidence::shell_operator_word(&words, true) {
            return Err(refuse_probe(
                k,
                &name,
                crate::evidence::operator_message("its command", &w),
            ));
        }
        if named_by_finding && (file.is_empty() || command.is_empty() || expect.is_empty()) {
            return Err(refuse_probe(k, &name, "a finding names it, so it needs `file`, `command` and `expect` for the fix to keep it as a permanent test".into()));
        }
        let mut hold = vec![name.clone()];
        for key in ["meant", "showed", "file", "command", "expect"] {
            hold.push(s(x, key).to_string());
        }
        let mut stored = String::new();
        if !file.is_empty() {
            let rel = std::path::Path::new(&file);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(refuse_probe(
                    k,
                    &name,
                    "its file is not a path inside the reviewer's copy".into(),
                ));
            }
            let body = fs::read_to_string(copy.join(rel)).map_err(|_| {
                refuse_probe(
                    k,
                    &name,
                    format!("its file `{file}` is not in the reviewer's copy"),
                )
            })?;
            hold.push(body.clone());
            if let Some(why) = hold.iter().find_map(|t| leak_in(p, t)) {
                return Err(refuse_probe(k, &name, format!("it holds {why}")));
            }
            fs::create_dir_all(&dir)?;
            let base = rel.file_name().and_then(|b| b.to_str()).unwrap_or("probe");
            let dest = dir.join(format!("{name}-{base}"));
            util::atomic_write(&dest, body.as_bytes())?;
            stored = format!("factory/evidence/probes/{k}-review-{round}/{name}-{base}");
        } else if let Some(why) = hold.iter().find_map(|t| leak_in(p, t)) {
            return Err(refuse_probe(k, &name, format!("it holds {why}")));
        }
        out.push(
            json!({"name": name, "meant": s(x, "meant").trim(), "showed": s(x, "showed").trim(),
            "file": file, "command": command, "command_words": words, "expect": expect,
            "expect_exit": x.get("expect_exit").and_then(|e| e.as_i64()).unwrap_or(0),
            "stored": stored}),
        );
    }
    Ok(out)
}

/// The findings, each naming a probe checked against the stored ones: a finding that a probe exposed
/// carries what the fix needs to keep it. A finding the human already accepted or declined is not filed again.
fn prepare_findings(
    p: &Project,
    st: &State,
    k: &str,
    list: &[Value],
    probes: &[Value],
) -> R<Vec<Value>> {
    let settled: Vec<(String, String)> = rounds(p, k)?
        .iter()
        .flat_map(|r| {
            r.findings
                .iter()
                .enumerate()
                .filter(|(i, f)| {
                    matches!(
                        fate(st, k, r.round, *i, f).map(|x| x.0).as_deref(),
                        Some("accepted") | Some("declined")
                    )
                })
                .map(|(_, f)| (norm(s(f, "title")), norm(finding_where(f))))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut out = Vec::new();
    for f in list {
        if settled.contains(&(norm(s(f, "title")), norm(finding_where(f)))) {
            return Err(refuse(
                format!("report findings on {k}"),
                format!("`{}` was accepted by the human or declined with a reason; a repeat review files only new problems", s(f, "title")),
                &["Leave it out of the report."],
                "The command records the review.",
            ));
        }
        let mut f = f.clone();
        let name = s(&f, "probe").to_string();
        if !name.is_empty() {
            let Some(pr) = probes.iter().find(|x| s(x, "name") == name) else {
                return Err(refuse(
                    format!("report findings on {k}"),
                    format!("`{}` names the probe `{name}`, which the report does not list in `probes`", s(&f, "title")),
                    &["List the probe with its name, what it was meant to show and what it showed."],
                    "The command records the review.",
                ));
            };
            f["probe_file"] = pr["file"].clone();
            f["probe_command"] = pr["command"].clone();
            f["probe_words"] = pr["command_words"].clone();
            f["probe_expect"] = pr["expect"].clone();
            f["probe_expect_exit"] = pr["expect_exit"].clone();
            f["probe_stored"] = pr["stored"].clone();
        }
        out.push(f);
    }
    Ok(out)
}

fn norm(t: &str) -> String {
    t.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn finding_where(f: &Value) -> &str {
    if s(f, "where").is_empty() {
        s(f, "file")
    } else {
        s(f, "where")
    }
}

fn check_findings(k: &str, list: &[Value]) -> R<()> {
    for (i, f) in list.iter().enumerate() {
        let sev = s(f, "severity");
        let file = if s(f, "where").is_empty() {
            s(f, "file")
        } else {
            s(f, "where")
        };
        let why = if s(f, "title").trim().is_empty() {
            Some("it has no title")
        } else if !SEVERITIES.contains(&sev) {
            Some("its severity is not critical, high, medium or low")
        } else if file.trim().is_empty() {
            Some("it names no file")
        } else {
            None
        };
        if let Some(w) = why {
            return Err(refuse(
                format!("report findings on {k}"),
                format!("finding {} is not complete: {w}", i + 1),
                &["Each finding carries `title`, `severity` (critical, high, medium or low), `where` (the file and line) and `action`."],
                "The command opens the checkpoint fix.",
            ));
        }
    }
    Ok(())
}

/// `bf report <K>` by the checkpoint reviewer on an open round: findings open the checkpoint fix, no
/// findings closes the round clean when every command check of the checkpoint has its receipt.
pub fn report(
    p: &Project,
    k: &str,
    result: &str,
    note: &str,
    findings: Option<&Value>,
    token: Option<&str>,
) -> R<Value> {
    let list = finding_list(findings);
    let raw_areas = object_list(findings, "areas");
    let raw_probes = object_list(findings, "probes");
    if !matches!(result, "findings" | "built") {
        return Err(refuse(
            format!("report `{result}` on the review of {k}"),
            "the checkpoint reviewer reports `findings` (with the findings, or none for a clean review)",
            &["Use `bf report <K> --result findings --role reviewer --token <token> [--findings -]`."],
            "`bf status` shows the review.",
        ));
    }
    if p.policy.roles.tokens && !run::holds_token(p, &format!("{k}:reviewer"), token) {
        return Err(refuse(
            format!("report on the review of {k}"),
            "a report needs the token of the reviewer's directive; none, or the wrong one, was given",
            &["Pass the token of the reviewer's directive for the checkpoint with `--token`."],
            "`bf status` shows the review.",
        ));
    }
    let list = {
        let _lock = crate::lock::engine(p, &format!("report on the review of {k}"))?;
        let st = events::require_chain(p)?;
        run::need_run(&st)?;
        let pl = plan::load(p)?;
        let rs = rounds(p, k)?;
        let Some(round) = open_round(&rs).cloned() else {
            return Err(refuse(
                format!("report on the review of {k}"),
                "no review round is open",
                &["`bf next` gives the reviewer's directive; `bf brief <K> --role reviewer` opens the round."],
                "`bf status` shows the review.",
            ));
        };
        let content = cp_content(p, k)?;
        if content != round.content {
            return Err(refuse(
                format!("report on the review of {k}"),
                "the checkpoint's content changed since the round opened, so the round judged other content",
                &["Run `bf brief <K> --role reviewer` again: it opens a new round on the content now in the checkpoint."],
                "`bf status` shows the review.",
            ));
        }
        let areas = check_areas(k, raw_areas)?;
        check_findings(k, &list)?;
        let probes = store_probes(p, k, round.round, &raw_probes, &list)?;
        let list = prepare_findings(p, &st, k, &list, &probes)?;
        let held_back = list.iter().any(blocking);
        if !held_back {
            let receipts = evidence::load_receipts(p)?;
            let missing = unrun_checks(&pl, &st, &receipts, k, &content);
            if !missing.is_empty() {
                return Err(refuse_owned(
                    format!("report a clean review of {k}"),
                    format!(
                        "{} command check(s) have no passing receipt at this content: the reviewer runs them itself",
                        missing.len()
                    ),
                    missing
                        .iter()
                        .map(|m| format!("Run `{m}` from inside your copy, then report again."))
                        .collect(),
                    "`bf evidence verify --checkpoint <K>` shows a passing receipt for each.",
                ));
            }
            let result = if list.is_empty() { "clean" } else { "findings" };
            events::append(
                p,
                "checkpoint.review.done",
                json!({"checkpoint": k, "round": round.round, "content": content, "result": result,
                       "findings": list, "areas": areas, "probes": probes, "note": note}),
            )?;
            // The review settles the named check at the content it judged.
            let says = if list.is_empty() {
                format!(
                    "The checkpoint reviewer judged the whole of {k} in round {} and found nothing to fix.",
                    round.round
                )
            } else {
                format!(
                    "The checkpoint reviewer judged the whole of {k} in round {} and filed no high and no medium finding; {} low finding(s) go to the human.",
                    round.round,
                    list.len()
                )
            };
            for c in pl.checks.iter().filter(|c| s(c, "named") == CHECK) {
                evidence::review_receipt(p, s(c, "id"), &content, &says)?;
            }
            remove_copy(p, &pl, k);
            if p.policy.roles.tokens {
                run::spend_token(p, &format!("{k}:reviewer"))?;
            }
            p.write_guard()?;
            return Ok(
                json!({"checkpoint": k, "round": round.round, "result": result,
                "low_findings": list.len(), "goes_on": true}),
            );
        }
        events::append(
            p,
            "checkpoint.review.done",
            json!({"checkpoint": k, "round": round.round, "content": content, "result": "findings",
                   "findings": list, "areas": areas, "probes": probes, "note": note}),
        )?;
        remove_copy(p, &pl, k);
        // The number of reviews the policy allows is reached with a high or medium finding open: the
        // checkpoint parks and the human is asked. No fix opens.
        if reviews(&st, k, &rs) + 1 >= p.policy.limits.checkpoint_reviews {
            if p.policy.roles.tokens {
                run::spend_token(p, &format!("{k}:reviewer"))?;
            }
            p.write_guard()?;
            return Ok(
                json!({"checkpoint": k, "round": round.round, "result": "findings",
                "parked": true, "note": "the checkpoint is parked: the human decides"}),
            );
        }
        list
    };
    let v = crate::ckfix::report_from_round(p, k, note, Some(&json!(list)), token)?;
    if p.policy.roles.tokens {
        run::spend_token(p, &format!("{k}:reviewer"))?;
    }
    Ok(v)
}

/// The commands of the checkpoint's own command checks that have no passing receipt at `content`.
fn unrun_checks(
    pl: &Plan,
    st: &State,
    receipts: &[Receipt],
    k: &str,
    content: &str,
) -> Vec<String> {
    let Some(kr) = pl.checkpoint(k) else {
        return vec![];
    };
    str_list(kr, "checks")
        .into_iter()
        .filter_map(|cid| {
            let c = pl.checks.iter().find(|c| s(c, "id") == cid)?;
            if crate::enforce::is_off(st, c)
                || s(c, "how") != "command"
                || evidence::satisfied_at(receipts, &cid, content).is_some()
            {
                return None;
            }
            Some(format!(
                "bf evidence run {cid} -- {}",
                evidence::command_words(c).join(" ")
            ))
        })
        .collect()
}

/// The checkpoint a reviewer's copy belongs to, when the command runs inside one with a round open.
fn copy_of_open_round(p: &Project, what: &str) -> R<(String, PathBuf)> {
    let top = p.top.clone();
    let name = top
        .strip_prefix(p.work_dir())
        .ok()
        .and_then(|rel| rel.components().next())
        .and_then(|c| c.as_os_str().to_str().map(|x| x.to_string()))
        .unwrap_or_default();
    let k = name.strip_suffix("-review").unwrap_or("").to_string();
    if k.is_empty() || !crate::sandbox::is_review_copy(&p.work_dir(), &top) {
        return Err(refuse(
            what.to_string(),
            "it runs only inside the checkpoint reviewer's copy (`.bf/work/<K>-review`)",
            &["Run `bf brief <K> --role reviewer`, then work inside the copy it makes."],
            "The command prints a receipt.",
        ));
    }
    let rs = rounds(p, &k)?;
    if open_round(&rs).is_none() {
        return Err(refuse(
            what.to_string(),
            "no review round is open for this checkpoint",
            &["`bf next` gives the reviewer's directive."],
            "The command prints a receipt.",
        ));
    }
    Ok((k, top))
}

/// `bf evidence run --probe <K> -- <command>`.
pub fn probe(p: &Project, k: &str, argv: &[String]) -> R<Value> {
    let (own, top) = copy_of_open_round(p, "run a probe")?;
    if own != k {
        return Err(refuse(
            format!("run a probe for {k}"),
            format!("this copy belongs to the review of {own}"),
            &["Name the checkpoint of your copy."],
            "The command prints a receipt.",
        ));
    }
    evidence::probe(p, k, argv, &top)
}

/// `bf evidence run <check> --judge flaky|failure|other --reason ... --token ...`.
pub fn judge(
    p: &Project,
    check: &str,
    judgement: &str,
    reason: &str,
    token: Option<&str>,
) -> R<Value> {
    let (k, top) = copy_of_open_round(p, "judge a check")?;
    let pl = plan::load(p)?;
    let listed = pl
        .checkpoint(&k)
        .map(|kr| str_list(kr, "checks").iter().any(|c| c == check))
        .unwrap_or(false);
    if !listed {
        return Err(refuse(
            format!("judge {check}"),
            format!("{k} does not list it"),
            &["Judge a check of the checkpoint you review."],
            "The command prints a receipt.",
        ));
    }
    if p.policy.roles.tokens && !run::holds_token(p, &format!("{k}:reviewer"), token) {
        return Err(refuse(
            format!("judge {check}"),
            "a judgement needs the token of the checkpoint reviewer's directive; none, or the wrong one, was given",
            &["Pass the token of the reviewer's directive with `--token`."],
            "The command prints a receipt.",
        ));
    }
    if p.content_hash(&top)? != cp_content(p, &k)? {
        return Err(refuse(
            format!("judge {check}"),
            "your copy no longer holds the checkpoint's content, so the runs it judges are not the checkpoint's",
            &["Roll back your probe files, or ask for a fresh copy with `bf brief <K> --role reviewer`."],
            "The command prints a receipt.",
        ));
    }
    // The runs judged are the checkpoint's: those at its content.
    evidence::judge(p, check, judgement, reason, &top)
}

/// The review rounds of every checkpoint, for `bf status`.
pub fn status_json(p: &Project, st: &State) -> Value {
    let Ok(pl) = plan::load(p) else {
        return json!([]);
    };
    let mut out = Vec::new();
    for kr in &pl.checkpoints {
        let k = s(kr, "id");
        let Ok(rs) = rounds(p, k) else { continue };
        if rs.is_empty() {
            continue;
        }
        out.push(
            json!({"checkpoint": k, "parked": parked(p, st, k).unwrap_or(false),
            "rounds": rs.iter().map(|r| json!({
            "round": r.round, "content": r.content, "covered": r.covered,
            "result": r.result, "findings": r.findings.len(),
            "areas": r.areas, "probes": r.probes,
            "outcomes": if r.findings.is_empty() { Value::Null } else { outcomes_json(st, k, r) },
        })).collect::<Vec<_>>()}),
        );
    }
    json!(out)
}

/// The findings a human names: `low` for every low finding of the latest review that is still open, or ids
/// such as `r1.2,r1.3`. Each must exist and still be open. Returns (round, number inside the round, finding).
fn pick_findings(p: &Project, st: &State, k: &str, spec: &str) -> R<Vec<(u32, usize, Value)>> {
    let rs = rounds(p, k)?;
    let bad = |because: String| {
        refuse(
            format!("name findings of {k}"),
            because,
            &["Name findings as `r<round>.<number>`, separated by commas, or `low` for every open low finding of the latest review. `bf status` lists them."],
            "`bf status` shows what became of each finding.",
        )
    };
    let mut out: Vec<(u32, usize, Value)> = Vec::new();
    for word in spec.split(',').map(|w| w.trim()).filter(|w| !w.is_empty()) {
        if word == "low" {
            let Some(last) = rs.iter().rev().find(|r| reported(r)) else {
                return Err(bad(format!("{k} has no review report yet")));
            };
            for (i, f) in open_findings(st, k, last)
                .into_iter()
                .filter(|(_, f)| !blocking(f))
            {
                out.push((last.round, i, f.clone()));
            }
            continue;
        }
        let parsed = word
            .strip_prefix('r')
            .and_then(|x| x.split_once('.'))
            .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<usize>().ok()?)));
        let Some((round, idx)) = parsed else {
            return Err(bad(format!("`{word}` is not a finding id")));
        };
        let Some(f) = rs
            .iter()
            .find(|r| r.round == round)
            .and_then(|r| r.findings.get(idx.wrapping_sub(1)))
        else {
            return Err(bad(format!("`{word}` is not a finding of {k}")));
        };
        if fate(st, k, round, idx - 1, f).is_some() {
            return Err(bad(format!(
                "`{word}` already has an end: {}",
                fate_text(st, k, round, idx - 1, f)
            )));
        }
        out.push((round, idx, f.clone()));
    }
    out.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    if out.is_empty() {
        return Err(bad("no open finding is named".to_string()));
    }
    Ok(out)
}

/// `bf checkpoint finding <K> --accept <ids> --quote ...` or `--decline <ids> --reason ... --quote ...`: the
/// human ends findings in their own words. A decline needs a reason.
pub fn settle(
    p: &Project,
    k: &str,
    spec: &str,
    fate_name: &str,
    reason: &str,
    quote: &str,
) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("settle findings of {k}"))?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    if fate_name == "declined" && reason.trim().is_empty() {
        return Err(refuse(
            format!("decline findings of {k}"),
            "a declined finding needs its reason",
            &["Add `--reason \"<why it is declined>\"`."],
            "`bf status` shows the reason with the finding.",
        ));
    }
    let picked = pick_findings(p, &st, k, spec)?;
    let (turn, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    let ids: Vec<Value> = picked
        .iter()
        .map(|(r, i, _)| json!({"round": r, "index": i}))
        .collect();
    events::append(
        p,
        "checkpoint.finding.settle",
        json!({"checkpoint": k, "ids": ids, "fate": fate_name, "reason": reason.trim(),
               "turn": turn, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    Ok(json!({"checkpoint": k, "settled": picked.len(), "fate": fate_name}))
}

/// `bf checkpoint finding <K> --fix-findings <ids>`: findings picked to be fixed open the checkpoint fix.
pub fn fix_chosen(p: &Project, k: &str, spec: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("fix findings of {k}"))?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let picked = pick_findings(p, &st, k, spec)?;
    let pl = plan::load(p)?;
    if st.ck_fix.is_some() || st.sync_open.is_some() {
        return Err(refuse(
            format!("fix findings of {k}"),
            "a fix or a sync is open already",
            &["`bf next` shows what it waits for."],
            "`bf status` shows the fix.",
        ));
    }
    let content = cp_content(p, k)?;
    let ids: Vec<(u32, u32)> = picked.iter().map(|x| (x.0, x.1 as u32)).collect();
    crate::ckfix::open_chosen(
        p,
        &pl,
        k,
        picked.into_iter().map(|x| x.2).collect(),
        ids,
        &content,
    )?;
    Ok(json!({"checkpoint": k, "phase": "fix", "fix_opened": true}))
}

/// `bf checkpoint review <K> --release --quote ...`: only the human releases a parked checkpoint, in their
/// own words. The reviews had so far stop counting, and the open high and medium findings go to a fix.
pub fn release(p: &Project, k: &str, quote: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("release the review of {k}"))?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    if !parked(p, &st, k)? {
        return Err(refuse(
            format!("release the review of {k}"),
            "the checkpoint is not parked",
            &["`bf next` shows what the checkpoint waits for."],
            "`bf status` shows the review.",
        ));
    }
    let (turn, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    let rs = rounds(p, k)?;
    let Some(last) = rs.iter().rev().find(|r| reported(r)).cloned() else {
        return Err(refuse(
            format!("release the review of {k}"),
            "no review report",
            &["`bf status` shows the review."],
            "`bf status` shows the review.",
        ));
    };
    let pl = plan::load(p)?;
    let held = open_blocking(&st, k, &rs);
    let ids: Vec<(u32, u32)> = held.iter().map(|x| (x.0, x.1 as u32)).collect();
    let open: Vec<Value> = held.into_iter().map(|x| x.2).collect();
    let content = cp_content(p, k)?;
    crate::ckfix::open_chosen(p, &pl, k, open, ids, &content)?;
    events::append(
        p,
        "checkpoint.review.release",
        json!({"checkpoint": k, "after_round": last.round, "turn": turn, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    Ok(json!({"checkpoint": k, "released": true, "fix_opened": true}))
}
