//! U-109 (A-73, D-87): `bf unit sync <U>`, the conductor's: bring the checkpoint branch into a waiting
//! unit's branch. U-82 (R-74): also the automatic sync at `bf unit start`, `bf unit release` and
//! `bf unit unblock`: the builder's changes are committed, then the checkpoint branch is taken in by
//! fast-forward or merge, and a conflict is left for the builder or aborted.

use super::*;
use crate::refusal::Fail;

/// Replay of `unit.sync`: the unit's base moves to the checkpoint commit taken in; nothing else changes.
pub(crate) fn replay(st: &mut State, ev: &Value, _seq: u64, _at: &str) {
    if let Some(u) = st.units.get_mut(s(ev, "unit")) {
        let nb = s(ev, "new_base");
        if !nb.is_empty() {
            u.base_commit = nb.to_string();
        }
    }
}

struct Ctx {
    wt: PathBuf,
    repo: PathBuf,
    cpb: String,
    ub: String,
}

fn ctx(p: &Project, unit: &str) -> Option<Ctx> {
    let pl = plan::load(p).ok()?;
    let u = pl.unit(unit)?;
    let repo = p.repo_of_unit(u).ok()?.path.clone();
    let wt = p.work_dir().join(unit);
    wt.join(".git").exists().then(|| Ctx {
        wt,
        repo,
        cpb: p.checkpoint_branch(s(u, "checkpoint")),
        ub: p.unit_branch(unit),
    })
}

fn count(c: &Ctx, range: &str) -> u64 {
    gitx::must(&c.repo, &["rev-list", "--count", range])
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn status(wt: &Path) -> R<String> {
    gitx::must(wt, &["status", "--porcelain"])
}

/// What the automatic sync knows when it takes the checkpoint branch in: the commit of the builder's
/// changes it made first (or empty).
struct Auto {
    wip: String,
}

/// Take the checkpoint branch in and append `unit.sync`. Any conflict is left in the working copy with
/// its marks, except one in a protected path, which is aborted and recorded.
fn take_in(
    p: &Project,
    unit: &str,
    c: &Ctx,
    cause: &str,
    ff: bool,
    auto: &Auto,
) -> R<(Value, Vec<String>)> {
    let old_base = gitx::must(&c.repo, &["merge-base", &c.cpb, &c.ub])?;
    let new_base = gitx::must(&c.repo, &["rev-parse", &c.cpb])?;
    let head = gitx::must(&c.wt, &["rev-parse", "HEAD"])?;
    let args: &[&str] = if ff {
        &["merge", "--ff-only", "-q", &c.cpb]
    } else {
        &["merge", "--no-edit", "-q", &c.cpb]
    };
    let o = gitx::run(&c.wt, args, &[])?;
    if !o.ok {
        let files =
            gitx::must(&c.wt, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        let files: Vec<String> = files.lines().map(String::from).collect();
        let protected = p.protected_paths();
        let inside = !files.is_empty()
            && !files
                .iter()
                .any(|f| protected.iter().any(|g| util::path_matches(g, f)));
        let mut ev = if inside {
            json!({"unit": unit, "cause": cause, "old_base": old_base, "new_base": new_base, "how": "merge", "commit": head, "wip_commit": auto.wip, "conflicts": files})
        } else {
            let _ = gitx::run(&c.wt, &["merge", "--abort"], &[]);
            json!({"unit": unit, "cause": cause, "old_base": old_base, "new_base": "", "how": "aborted", "checkpoint_commit": new_base, "commit": head, "wip_commit": auto.wip, "conflicts": [], "files": files})
        };
        events::append(p, "unit.sync", ev.clone())?;
        if !inside {
            ev["why"] = json!(o.stderr.trim());
        }
        return Ok((ev, Vec::new()));
    }
    let commit = gitx::must(&c.wt, &["rev-parse", "HEAD"])?;
    let changed = gitx::must(&c.repo, &["diff", "--name-only", &old_base, &new_base])?;
    let changed: Vec<String> = changed.lines().map(String::from).collect();
    let how = if ff { "fast-forward" } else { "merge" };
    let ev = json!({"unit": unit, "cause": cause, "old_base": old_base, "new_base": new_base, "how": how, "commit": commit, "wip_commit": auto.wip, "conflicts": []});
    events::append(p, "unit.sync", ev.clone())?;
    Ok((ev, changed))
}

fn no(unit: &str, because: String, step: String) -> Fail {
    use crate::strings::t;
    refuse_owned(
        t("unit_sync.what", &[("unit", unit)]),
        because,
        vec![step],
        t("unit_sync.worked", &[]),
    )
}

pub(crate) fn unit_sync(p: &Project, unit: &str) -> R<Value> {
    use crate::strings::t;
    let _lock = crate::lock::engine(p, &format!("sync {unit}"))?;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let cur = st.unit_state(unit).to_string();
    if !matches!(
        cur.as_str(),
        "" | "running" | "fixing" | "blocked" | "parked"
    ) {
        let key = if matches!(cur.as_str(), "review" | "reviewed") {
            "unit_sync.no.state_review"
        } else {
            "unit_sync.no.state_other"
        };
        return Err(no(
            unit,
            t(key, &[("state", &cur)]),
            t("unit_sync.no.state_step", &[]),
        ));
    }
    let Some(c) = ctx(p, unit) else {
        return Err(no(
            unit,
            t("unit_sync.no.copy", &[]),
            t("unit_sync.no.copy_step", &[]),
        ));
    };
    if crate::scope::open_request(&st, unit).is_some() {
        return Err(no(
            unit,
            t("unit_sync.no.request", &[]),
            t("unit_sync.no.request_step", &[]),
        ));
    }
    if let Some(so) = &st.sync_open {
        return Err(no(
            unit,
            t("unit_sync.no.sync_open", &[("checkpoint", &so.checkpoint)]),
            t("unit_sync.no.sync_open_step", &[]),
        ));
    }
    if merge_in_progress(&c.wt) {
        let mut files = marked_files(p, unit);
        if files.is_empty() {
            files = unmerged(&c.wt);
        }
        return Err(no(
            unit,
            t("unit_sync.no.merging", &[("files", &files.join(", "))]),
            t("unit_sync.no.merging_step", &[]),
        ));
    }
    if !status(&c.wt)?.is_empty() {
        return Err(no(
            unit,
            t("unit_sync.no.dirty", &[]),
            t("unit_sync.no.dirty_step", &[]),
        ));
    }
    if gitx::is_ancestor(&c.repo, &c.cpb, &c.ub) {
        return Err(no(
            unit,
            t("unit_sync.no.nothing", &[]),
            t("unit_sync.no.nothing_step", &[]),
        ));
    }
    let ff = count(&c, &format!("{}..{}", c.cpb, c.ub)) == 0;
    let auto = Auto { wip: String::new() };
    let (ev, changed) = take_in(p, unit, &c, "sync", ff, &auto)?;
    p.write_guard()?;
    let mut out = ev;
    if s(&out, "how") == "aborted" {
        let files = str_list(&out, "files");
        let stuck = gitx::must(&c.wt, &["rev-parse", "HEAD"]).ok().as_deref()
            != Some(s(&out, "commit"))
            || merge_in_progress(&c.wt);
        let because = if files.is_empty() {
            t(
                "unit_sync.aborted.because_none",
                &[("checkpoint", &c.cpb), ("why", s(&out, "why"))],
            )
        } else {
            t(
                "unit_sync.aborted.because",
                &[("checkpoint", &c.cpb), ("files", &files.join(", "))],
            )
        };
        let wt = c.wt.display().to_string();
        let step = if stuck {
            t("unit_sync.aborted.step_stuck", &[("path", &wt)])
        } else {
            t("unit_sync.aborted.step", &[])
        };
        return Err(no_with(
            unit,
            because,
            step,
            t("unit_sync.aborted.worked", &[]),
        ));
    }
    let conflicts = str_list(&out, "conflicts");
    if !conflicts.is_empty() {
        let _ = super::brief::write_brief(p, unit, "builder");
        out["line"] = json!(format!(
            "{}\n{}",
            t(
                "unit_sync.line.conflicts",
                &[("unit", unit), ("files", &conflicts.join(", "))]
            ),
            t("unit_sync.step.conflicts", &[("unit", unit)])
        ));
        out["step"] = json!(t("unit_sync.step.conflicts", &[("unit", unit)]));
        return Ok(out);
    }
    let granted = crate::scope::granted_files(&st, unit);
    let files: Vec<&String> = changed
        .iter()
        .filter(|f| granted.iter().any(|g| util::path_matches(g, f)))
        .collect();
    let list = if files.is_empty() {
        t("unit_sync.line.none", &[])
    } else {
        files
            .iter()
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let line = t(
        "unit_sync.line.merged",
        &[
            ("unit", unit),
            ("how", s(&out, "how")),
            (
                "base",
                &s(&out, "new_base")[..12.min(s(&out, "new_base").len())],
            ),
            ("files", &list),
        ],
    );
    out["files"] = json!(files);
    out["line"] = json!(line);
    Ok(out)
}

fn no_with(unit: &str, because: String, step: String, worked: String) -> Fail {
    refuse_owned(
        crate::strings::t("unit_sync.what", &[("unit", unit)]),
        because,
        vec![step],
        worked,
    )
}

fn merge_in_progress(wt: &Path) -> bool {
    gitx::run(wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[])
        .map(|o| o.ok)
        .unwrap_or(false)
}

/// The files a merge in progress still holds unmerged, read from git.
pub(crate) fn unmerged(wt: &Path) -> Vec<String> {
    if !wt.join(".git").exists() || !merge_in_progress(wt) {
        return Vec::new();
    }
    gitx::must(wt, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

/// The latest `unit.sync` of the unit, of any cause, from the log.
fn latest_auto(p: &Project, unit: &str) -> Option<Value> {
    let log = events::load_log(p).ok()?;
    latest_auto_in(&log.events, unit)
}

fn latest_auto_in(evs: &[Value], unit: &str) -> Option<Value> {
    evs.iter()
        .rev()
        .find(|e| e["kind"] == "unit.sync" && s(e, "unit") == unit)
        .cloned()
}

/// The files of the unit's latest `unit.sync` (of any cause) when it was aborted and its checkpoint
/// commit is still not in the unit's branch, else none.
pub(crate) fn aborted_files(p: &Project, unit: &str) -> Vec<String> {
    aborted_in(
        p,
        &events::load_log(p).map(|l| l.events).unwrap_or_default(),
        unit,
    )
}

/// The same from events already loaded (`bf status` loads the log once).
pub(crate) fn aborted_in(p: &Project, evs: &[Value], unit: &str) -> Vec<String> {
    let Some(e) = evs
        .iter()
        .rev()
        .find(|e| e["kind"] == "unit.sync" && s(e, "unit") == unit)
    else {
        return Vec::new();
    };
    if s(e, "how") != "aborted" {
        return Vec::new();
    }
    let Some(c) = ctx(p, unit) else {
        return Vec::new();
    };
    let at = s(e, "checkpoint_commit");
    if !at.is_empty() && gitx::is_ancestor(&c.repo, at, &c.ub) {
        return Vec::new();
    }
    str_list(e, "files")
}

/// The directive's data of a builder's unit: `sync_conflicts` and `sync_aborted`.
pub(crate) fn decorate(p: &Project, unit: &str, d: &mut Value) {
    let wt = p.work_dir().join(unit);
    let conflicts = unmerged(&wt);
    if !conflicts.is_empty() {
        d["sync_conflicts"] = json!(conflicts);
    }
    let aborted = aborted_files(p, unit);
    if !aborted.is_empty() {
        d["sync_aborted"] = json!(aborted);
    }
}

/// The builder's brief sections for a merge left in progress and for an aborted sync.
pub(crate) fn brief_md(p: &Project, unit: &str) -> String {
    use crate::strings::t;
    let wt = p.work_dir().join(unit);
    let mut md = String::new();
    let conflicts = unmerged(&wt);
    if !conflicts.is_empty() {
        md.push_str(&format!(
            "## {}\n\n{}\n\n",
            t("autosync.brief.conflicts.title", &[]),
            t("autosync.brief.conflicts.body", &[])
        ));
        for f in &conflicts {
            md.push_str(&format!("{}\n", t("autosync.brief.file", &[("path", f)])));
        }
        md.push('\n');
    }
    if let Some(e) = sent_back(p, unit) {
        md.push_str(&format!(
            "## {}\n\n{}\n\n{}\n\n",
            t("land_conflict.brief.title", &[]),
            t("land_conflict.brief.reason", &[("note", s(&e, "note"))]),
            if s(&e, "result") == "suite" {
                t("land_merge.brief.after", &[])
            } else {
                t("land_conflict.brief.after", &[])
            }
        ));
    }
    let aborted = aborted_files(p, unit);
    if !aborted.is_empty() {
        md.push_str(&format!(
            "## {}\n\n{}\n\n",
            t("autosync.brief.aborted.title", &[]),
            t("autosync.brief.aborted.body", &[])
        ));
        for f in &aborted {
            md.push_str(&format!("{}\n", t("autosync.brief.file", &[("path", f)])));
        }
        md.push('\n');
    }
    md
}

/// The files that still hold a conflict mark: the unmerged ones and those the sync named.
pub(crate) fn marked_files(p: &Project, unit: &str) -> Vec<String> {
    let wt = p.work_dir().join(unit);
    let mut files = unmerged(&wt);
    if let Some(e) = latest_auto(p, unit) {
        for f in str_list(&e, "conflicts") {
            if !files.contains(&f) {
                files.push(f);
            }
        }
    }
    files
        .into_iter()
        .filter(|f| {
            // The marks are as long as the file's `conflict-marker-size` attribute (7 when unset).
            let size = gitx::run(&wt, &["check-attr", "conflict-marker-size", "--", f], &[])
                .ok()
                .filter(|o| o.ok)
                .and_then(|o| o.stdout.trim().rsplit(": ").next()?.parse::<usize>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(7);
            let (open, sep, close) = ("<".repeat(size), "=".repeat(size), ">".repeat(size));
            std::fs::read_to_string(wt.join(f))
                .map(|x| {
                    x.lines().any(|l| {
                        // The separator is a line of its own: a setext heading underline is longer or
                        // carries text after the signs.
                        let mark = |m: &str| {
                            l.strip_prefix(m)
                                .is_some_and(|r| r.is_empty() || r.starts_with(' '))
                        };
                        mark(&open) || l == sep || mark(&close)
                    })
                })
                .unwrap_or(false)
        })
        .collect()
}

/// A `built` or `fixed` report of a unit whose working copy has a merge in progress: refused naming the
/// files that still hold marks; with none left the merge is finished with one commit.
pub(crate) fn finish_merge(p: &Project, unit: &str) -> R<()> {
    use crate::strings::t;
    let wt = p.work_dir().join(unit);
    if !merge_in_progress(&wt) {
        return Ok(());
    }
    let marked = marked_files(p, unit);
    if !marked.is_empty() {
        return Err(refuse_owned(
            t("autosync.report.refused", &[("unit", unit)]),
            t("autosync.report.because", &[("files", &marked.join(", "))]),
            vec![t("autosync.report.step", &[("unit", unit)])],
            t("autosync.report.worked", &[]),
        ));
    }
    let mut files = unmerged(&wt);
    if let Some(e) = latest_auto(p, unit) {
        for f in str_list(&e, "conflicts") {
            if !files.contains(&f) {
                files.push(f);
            }
        }
    }
    for f in &files {
        gitx::must(&wt, &["add", "-A", "--", f])?;
    }
    gitx::add_all_without_junk(&wt)?;
    let msg = t(
        "autosync.merge_commit",
        &[("unit", unit), ("files", &files.join(", "))],
    );
    gitx::must(&wt, &["commit", "-q", "-m", &msg])?;
    Ok(())
}

/// At start, release and unblock, and at each new round: bring the unit's branch up to the checkpoint
/// branch. The builder's uncommitted changes are committed first; the checkpoint branch is taken in as
/// a fast-forward for a unit with no commits of its own and as a merge otherwise. A conflict is left in
/// the working copy for the builder, except one in a protected path, which is aborted. Returns how far
/// the unit's branch is still behind the checkpoint branch.
pub(crate) fn auto_ff(p: &Project, st: &State, unit: &str, cause: &str) -> u64 {
    auto_inner(p, st, unit, cause).0
}

/// A landing whose merge conflicts: take the checkpoint branch into the unit's working copy, which
/// leaves the conflict there. Returns the sync event, or none when nothing was taken in.
pub(crate) fn take_in_for_land(p: &Project, st: &State, unit: &str) -> Option<Value> {
    auto_inner(p, st, unit, "land").1
}

fn auto_inner(p: &Project, st: &State, unit: &str, cause: &str) -> (u64, Option<Value>) {
    let Some(c) = ctx(p, unit) else {
        return (0, None);
    };
    if !gitx::branch_exists(&c.repo, &c.ub) || !gitx::branch_exists(&c.repo, &c.cpb) {
        return (0, None);
    }
    let behind = |c: &Ctx| count(c, &format!("{}..{}", c.ub, c.cpb));
    let free = crate::scope::open_request(st, unit).is_none()
        && st.sync_open.is_none()
        && !matches!(st.unit_state(unit), "review" | "reviewed")
        && !merge_in_progress(&c.wt)
        && !gitx::is_ancestor(&c.repo, &c.cpb, &c.ub);
    if !free {
        return (behind(&c), None);
    }
    let mut wip = String::new();
    if gitx::has_uncommitted(&c.wt) {
        let msg = crate::strings::t("autosync.commit", &[("unit", unit), ("cause", cause)]);
        let made = gitx::add_all_without_junk(&c.wt)
            .and_then(|_| gitx::must(&c.wt, &["commit", "-q", "-m", &msg]))
            .and_then(|_| gitx::must(&c.wt, &["rev-parse", "HEAD"]));
        match made {
            Ok(h) => wip = h,
            Err(_) => return (behind(&c), None),
        }
    }
    let ff = count(&c, &format!("{}..{}", c.cpb, c.ub)) == 0;
    let auto = Auto { wip };
    let mut ev = None;
    if let Ok((e, _)) = take_in(p, unit, &c, cause, ff, &auto) {
        if s(&e, "how") == "aborted" || !str_list(&e, "conflicts").is_empty() {
            let _ = p.write_guard();
            let _ = super::brief::write_brief(p, unit, "builder");
        }
        ev = Some(e);
    }
    (behind(&c), ev)
}

/// The note of the engine's latest report that sent the unit from `reviewed` back to `fixing`, while no
/// later report of the unit has moved it on.
fn sent_back(p: &Project, unit: &str) -> Option<Value> {
    let log = events::load_log(p).ok()?;
    let last = log
        .events
        .iter()
        .rev()
        .find(|e| e["kind"] == "unit.report" && s(e, "unit") == unit)?;
    (s(last, "role") == "engine" && s(last, "from") == "reviewed" && s(last, "to") == "fixing")
        .then(|| last.clone())
}

/// The index in the log after which a reviewer's round looks for syncs: the unit's latest report of a
/// reviewer, else the unit's latest start.
fn round_start(evs: &[Value], unit: &str) -> usize {
    let at =
        |pred: &dyn Fn(&Value) -> bool| evs.iter().rposition(|e| s(e, "unit") == unit && pred(e));
    at(&|e| e["kind"] == "unit.report" && s(e, "role") == "reviewer")
        .or_else(|| at(&|e| e["kind"] == "unit.start"))
        .unwrap_or(0)
}

/// The files each `unit.sync` of the unit since the reviewer's latest round (for the first round, since
/// the unit's start) took in from the checkpoint branch.
pub(crate) fn taken_in(p: &Project, unit: &str) -> Vec<String> {
    let Ok(log) = events::load_log(p) else {
        return Vec::new();
    };
    let Some(c) = ctx(p, unit) else {
        return Vec::new();
    };
    let from = round_start(&log.events, unit);
    let mut out: Vec<String> = Vec::new();
    for e in log.events.iter().skip(from + 1) {
        if e["kind"] != "unit.sync" || s(e, "unit") != unit || s(e, "new_base").is_empty() {
            continue;
        }
        let d = gitx::must(
            &c.repo,
            &["diff", "--name-only", s(e, "old_base"), s(e, "new_base")],
        )
        .unwrap_or_default();
        for f in d.lines() {
            if !out.iter().any(|o| o == f) {
                out.push(f.to_string());
            }
        }
    }
    out
}

/// The files of the unit's own diff against its checkpoint branch.
pub(crate) fn own_files(p: &Project, unit: &str) -> Vec<String> {
    let Some(c) = ctx(p, unit) else {
        return Vec::new();
    };
    gitx::must(
        &c.wt,
        &["diff", "--name-only", &format!("{}...HEAD", c.cpb)],
    )
    .unwrap_or_default()
    .lines()
    .map(String::from)
    .collect()
}

/// The reviewer's view of a resolution round: the conflicted files of the landing or the sync that left a
/// conflict for the builder and what the builder changed while resolving them, as a diff against what git
/// merges by itself.
pub(crate) fn resolution_diff(p: &Project, unit: &str) -> Option<(Vec<String>, String)> {
    let log = events::load_log(p).ok()?;
    let mine = |e: &&Value| e["kind"] == "unit.report" && s(e, "unit") == unit;
    let sent = log.events.iter().rposition(|e| {
        mine(&e) && s(e, "role") == "engine" && s(e, "from") == "reviewed" && s(e, "to") == "fixing"
    });
    let found = log
        .events
        .iter()
        .rposition(|e| mine(&e) && s(e, "result") == "findings");
    let sent = sent.filter(|i| !found.is_some_and(|j| j > *i));
    let from = round_start(&log.events, unit);
    let synced = log.events.iter().rposition(|e| {
        e["kind"] == "unit.sync" && s(e, "unit") == unit && !str_list(e, "conflicts").is_empty()
    });
    let synced = synced.filter(|i| *i > from);
    let pick = match (sent, synced) {
        (Some(a), Some(b)) => a.max(b),
        (a, b) => a.or(b)?,
    };
    let files = str_list(&log.events[pick], "conflicts");
    if files.is_empty() {
        return None;
    }
    let wt = p.work_dir().join(unit);
    let m = gitx::must(&wt, &["rev-list", "--merges", "-n1", "HEAD"]).ok()?;
    if m.is_empty() {
        return None;
    }
    let o = gitx::run(
        &wt,
        &[
            "merge-tree",
            "--write-tree",
            &format!("{m}^1"),
            &format!("{m}^2"),
        ],
        &[],
    )
    .ok()?;
    let tree = o.stdout.lines().next()?.trim().to_string();
    if tree.len() < 40 {
        return None;
    }
    let mut args = vec!["diff", tree.as_str(), "--"];
    args.extend(files.iter().map(|f| f.as_str()));
    let d = gitx::must(&wt, &args).ok()?;
    Some((files, d))
}
