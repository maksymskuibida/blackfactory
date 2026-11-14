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

/// Whether the review ran clean on exactly this content.
pub fn clean_at(p: &Project, k: &str, content: &str) -> R<bool> {
    Ok(rounds(p, k)?
        .iter()
        .any(|r| r.result.as_deref() == Some("clean") && r.content == content))
}

/// Whether the checkpoint still needs its review before it can close.
pub fn needed(p: &Project, st: &State, k: &str, content: &str) -> R<bool> {
    Ok(required(st) && !clean_at(p, k, content)?)
}

/// The rounds that ended in findings: the limit of rounds counts these.
fn spent(rs: &[Round]) -> u32 {
    rs.iter()
        .filter(|r| r.result.as_deref() == Some("findings"))
        .count() as u32
}

/// Whether the review used up its rounds without a clean result at this content.
pub fn exhausted(p: &Project, k: &str) -> R<bool> {
    Ok(spent(&rounds(p, k)?) >= p.policy.limits.max_rounds)
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
    if spent(&rs) >= p.policy.limits.max_rounds {
        return Ok(
            json!({"role": "human", "action": "decide-checkpoint-review", "checkpoint": k,
            "note": format!("The review of {k} filed findings in {} rounds and did not come back clean. The human decides: an amendment, or switching the checkpoint review off for the run with a reason (`bf checks propose --scope run {CHECK}=off:<why>`, then `bf checks set --quote \"<their words>\"`).", spent(&rs))}),
        );
    }
    let round = open_round(&rs)
        .map(|r| r.round)
        .unwrap_or(rs.len() as u32 + 1);
    Ok(
        json!({"role": "reviewer", "action": "review-checkpoint", "checkpoint": k,
        "tier": p.policy.tiers.reviewer, "round": round,
        "working_copy": copy_path(p, k).to_string_lossy(),
        "brief": p.briefs_dir().join(format!("{k}-reviewer.md")).to_string_lossy(),
        "limits": {"max_rounds": p.policy.limits.max_rounds, "round": spent(&rs), "idle_minutes": p.policy.limits.idle_minutes,
                   "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }},
        "commands": [format!("bf brief {k} --role reviewer")]}),
    )
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

fn outcome_of(st: &State, k: &str, r: &Round) -> &'static str {
    if st
        .ck_fixes
        .iter()
        .any(|f| f.checkpoint == k && f.origin_findings == r.findings)
    {
        "fixed (the fix was reviewed clean)"
    } else if st
        .ck_fix
        .as_ref()
        .map(|f| f.checkpoint == k && f.origin_findings == r.findings)
        .unwrap_or(false)
    {
        "open (a fix is under way)"
    } else {
        "open"
    }
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
    if clean_at(p, k, &content)? {
        return Err(refuse_not_ready(
            k,
            "the review already ran clean on this content",
        ));
    }
    let mut rs = rounds(p, k)?;
    if spent(&rs) >= p.policy.limits.max_rounds {
        return Err(refuse_not_ready(
            k,
            "the review used up its rounds; the human decides",
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
            let earlier = rs
                .iter()
                .rev()
                .find(|r| r.result.as_deref() == Some("findings"));
            let covered = match earlier {
                None => "the whole checkpoint, cold".to_string(),
                Some(e) => format!(
                    "the whole checkpoint, the earlier findings with their outcomes, and what changed since round {}",
                    e.round
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
    md.push_str(&format!(
        "You review the whole checkpoint cold, on the strong tier. Round {n} covers: {}.\n\n",
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
    md.push_str("\n## The brief of every unit on this checkpoint\n\n");
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let id = s(u, "id");
        md.push_str(&format!(
            "<details><summary>{id}</summary>\n\n{}\n</details>\n\n",
            read_unit_brief(p, id)
        ));
    }
    // Earlier rounds: the findings with their outcomes, and what changed since the round before.
    let earlier: Vec<&Round> = rs
        .iter()
        .filter(|r| r.round < n && r.result.as_deref() == Some("findings"))
        .collect();
    if !earlier.is_empty() {
        md.push_str("## Earlier rounds\n\nCheck that each finding below is fixed. File a new finding for anything a fix broke or that you come across. The whole checkpoint stays in scope.\n\n");
        for r in &earlier {
            md.push_str(&format!(
                "Round {} judged content {}:\n",
                r.round,
                &r.content[..12.min(r.content.len())]
            ));
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
                    outcome_of(&st, k, r)
                ));
            }
        }
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
        "## Report\n\nFindings carry a title, a severity (critical, high, medium or low), `where` (the file and line) and `action`:\n\n```\nbf report {k} --result findings --role reviewer --token <token> --findings - <<'EOF'\n[{{\"title\": \"...\", \"severity\": \"medium\", \"where\": \"src/a.rs:10\", \"action\": \"...\"}}]\nEOF\n```\n\nThey open the checkpoint fix. With nothing to file, run the same command without `--findings`: it needs a passing receipt for every command check of the checkpoint at this content.\n\n## Rules\n\n- Never edit anything under `factory/` or `.bf/`, apart from your own copy.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot judge without the human's decision, ask: `bf ask --title \"<the question>\"`.\n"
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
    {
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
        if list.is_empty() {
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
            events::append(
                p,
                "checkpoint.review.done",
                json!({"checkpoint": k, "round": round.round, "content": content, "result": "clean",
                       "findings": [], "note": note}),
            )?;
            // The review settles the named check at the content it judged.
            let says = format!(
                "The checkpoint reviewer judged the whole of {k} in round {} and found nothing to fix.",
                round.round
            );
            for c in pl.checks.iter().filter(|c| s(c, "named") == CHECK) {
                evidence::review_receipt(p, s(c, "id"), &content, &says)?;
            }
            remove_copy(p, &pl, k);
            if p.policy.roles.tokens {
                run::spend_token(p, &format!("{k}:reviewer"))?;
            }
            p.write_guard()?;
            return Ok(json!({"checkpoint": k, "round": round.round, "result": "clean"}));
        }
        check_findings(k, &list)?;
        events::append(
            p,
            "checkpoint.review.done",
            json!({"checkpoint": k, "round": round.round, "content": content, "result": "findings",
                   "findings": list, "note": note}),
        )?;
        remove_copy(p, &pl, k);
    }
    let v = crate::ckfix::report(
        p,
        k,
        "findings",
        Some("reviewer"),
        note,
        Some(&json!(list)),
        token,
    )?;
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
        out.push(json!({"checkpoint": k, "rounds": rs.iter().map(|r| json!({
            "round": r.round, "content": r.content, "covered": r.covered,
            "result": r.result, "findings": r.findings.len(),
            "outcomes": if r.findings.is_empty() { Value::Null } else { json!(outcome_of(st, k, r)) },
        })).collect::<Vec<_>>()}));
    }
    json!(out)
}
