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

/// Take the checkpoint branch in and append `unit.sync`. With `auto`, any conflict is left in the working
/// copy with its marks, except one in a protected path, which is aborted and recorded.
fn take_in(
    p: &Project,
    unit: &str,
    c: &Ctx,
    cause: &str,
    ff: bool,
    auto: Option<&Auto>,
) -> R<(Value, Vec<String>)> {
    let old_base = gitx::must(&c.repo, &["merge-base", &c.cpb, &c.ub])?;
    let new_base = gitx::must(&c.repo, &["rev-parse", &c.cpb])?;
    let head = gitx::must(&c.wt, &["rev-parse", "HEAD"])?;
    let before = status(&c.wt)?;
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
        if let Some(a) = auto {
            let protected = p.protected_paths();
            let inside = !files.is_empty()
                && !files
                    .iter()
                    .any(|f| protected.iter().any(|g| util::path_matches(g, f)));
            let ev = if inside {
                json!({"unit": unit, "cause": cause, "old_base": old_base, "new_base": new_base, "how": "merge", "commit": head, "wip_commit": a.wip, "conflicts": files})
            } else {
                let _ = gitx::run(&c.wt, &["merge", "--abort"], &[]);
                json!({"unit": unit, "cause": cause, "old_base": old_base, "new_base": "", "how": "aborted", "checkpoint_commit": new_base, "commit": head, "wip_commit": a.wip, "conflicts": [], "files": files})
            };
            events::append(p, "unit.sync", ev.clone())?;
            return Ok((ev, Vec::new()));
        }
        let _ = gitx::run(&c.wt, &["merge", "--abort"], &[]);
        let same = gitx::must(&c.wt, &["rev-parse", "HEAD"])? == head && status(&c.wt)? == before;
        let named = if files.is_empty() {
            o.stderr.clone()
        } else {
            files.join(", ")
        };
        return Err(refuse_owned(
            format!("sync {unit}"),
            format!("the merge of {} into the unit's branch conflicts in: {named}{}", c.cpb, if same { "" } else { " (the working copy could not be restored; check `git status`)" }),
            vec![
                format!("The human resolves it in {} (merge, fix the files, commit), or lands the unit first.", c.wt.display()),
                if same { "The working copy is as it was; nothing is recorded.".to_string() } else { format!("Nothing is recorded. Run `git status` in {}, then restore it (`git merge --abort` or `git reset --merge`) before anything else.", c.wt.display()) },
            ],
            "`bf unit sync` is taken again and prints the files the checkpoint changed.",
        ));
    }
    let commit = gitx::must(&c.wt, &["rev-parse", "HEAD"])?;
    let changed = gitx::must(&c.repo, &["diff", "--name-only", &old_base, &new_base])?;
    let changed: Vec<String> = changed.lines().map(String::from).collect();
    let how = if ff { "fast-forward" } else { "merge" };
    let wip = auto.map(|a| a.wip.as_str()).unwrap_or("");
    let ev = json!({"unit": unit, "cause": cause, "old_base": old_base, "new_base": new_base, "how": how, "commit": commit, "wip_commit": wip, "conflicts": []});
    events::append(p, "unit.sync", ev.clone())?;
    Ok((ev, changed))
}

fn no(unit: &str, because: String, step: &str) -> Fail {
    refuse_owned(
        format!("sync {unit}"),
        because,
        vec![step.into()],
        "`bf unit sync` brings the unit's branch up to the checkpoint branch.",
    )
}

pub(crate) fn unit_sync(p: &Project, unit: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("sync {unit}"))?;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let cur = st.unit_state(unit).to_string();
    if !matches!(
        cur.as_str(),
        "" | "running" | "fixing" | "blocked" | "parked"
    ) {
        let why = if matches!(cur.as_str(), "review" | "reviewed") {
            format!("it is in `{cur}`: the content the reviewer judges would change")
        } else {
            format!("it is `{cur}`")
        };
        return Err(no(
            unit,
            why,
            "Let the review end, or sync after findings send the unit back to work.",
        ));
    }
    let Some(c) = ctx(p, unit) else {
        return Err(no(
            unit,
            "the unit has no working copy".into(),
            "Use `bf unit start` first, or a unit ID from `bf status`.",
        ));
    };
    if crate::scope::open_request(&st, unit).is_some() {
        return Err(no(
            unit,
            "a request for files is open".into(),
            "Let the request be answered (`bf next`), then sync.",
        ));
    }
    if let Some(so) = &st.sync_open {
        return Err(no(
            unit,
            format!("a sync of the main branch into {} is open", so.checkpoint),
            "Settle that sync (`bf next`), then sync the unit.",
        ));
    }
    if !status(&c.wt)?.is_empty() {
        return Err(no(
            unit,
            "the working copy has changes `git status` shows".into(),
            "The builder commits them or puts them back; then sync.",
        ));
    }
    if gitx::is_ancestor(&c.repo, &c.cpb, &c.ub) {
        return Err(no(
            unit,
            "nothing to do: the checkpoint branch is already in the unit's branch".into(),
            "Go on with the unit.",
        ));
    }
    let ff = count(&c, &format!("{}..{}", c.cpb, c.ub)) == 0;
    let (ev, changed) = take_in(p, unit, &c, "sync", ff, None)?;
    p.write_guard()?;
    let granted = crate::scope::granted_files(&st, unit);
    let files: Vec<&String> = changed
        .iter()
        .filter(|f| granted.iter().any(|g| util::path_matches(g, f)))
        .collect();
    let mut out = ev;
    let line = format!(
        "{unit}: {} ({}); granted files the checkpoint changed: {}",
        s(&out, "how"),
        &s(&out, "new_base")[..12.min(s(&out, "new_base").len())],
        if files.is_empty() {
            "none".into()
        } else {
            files
                .iter()
                .map(|f| f.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    out["files"] = json!(files);
    out["line"] = json!(line);
    Ok(out)
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

/// The latest automatic sync of the unit, from the log.
fn latest_auto(p: &Project, unit: &str) -> Option<Value> {
    let log = events::load_log(p).ok()?;
    latest_auto_in(&log.events, unit)
}

fn latest_auto_in(evs: &[Value], unit: &str) -> Option<Value> {
    evs.iter()
        .rev()
        .find(|e| e["kind"] == "unit.sync" && s(e, "unit") == unit && s(e, "cause") != "sync")
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
    if let Ok((e, _)) = take_in(p, unit, &c, cause, ff, Some(&auto)) {
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

/// The reviewer's view of a resolution round: the conflicted files of the landing that sent the unit
/// back and what the builder changed while resolving them, as a diff against what git merges by itself.
pub(crate) fn resolution_diff(p: &Project, unit: &str) -> Option<(Vec<String>, String)> {
    let log = events::load_log(p).ok()?;
    let mine = |e: &&Value| e["kind"] == "unit.report" && s(e, "unit") == unit;
    let sent = log.events.iter().rposition(|e| {
        mine(&e) && s(e, "role") == "engine" && s(e, "from") == "reviewed" && s(e, "to") == "fixing"
    })?;
    let found = log
        .events
        .iter()
        .rposition(|e| mine(&e) && s(e, "result") == "findings");
    if found.is_some_and(|j| j > sent) {
        return None;
    }
    let files = str_list(&log.events[sent], "conflicts");
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
