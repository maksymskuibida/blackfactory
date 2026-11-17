//! U-109 (A-73, D-87): `bf unit sync <U>`, the conductor's: bring the checkpoint branch into a waiting
//! unit's branch. Also the fast-forward `bf unit start` and `bf unit release` make for a unit with no
//! commits of its own.

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

/// Take the checkpoint branch in and append `unit.sync`.
fn take_in(p: &Project, unit: &str, c: &Ctx, cause: &str, ff: bool) -> R<(Value, Vec<String>)> {
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
    let ev = json!({"unit": unit, "cause": cause, "old_base": old_base, "new_base": new_base, "how": how, "commit": commit});
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
    let (ev, changed) = take_in(p, unit, &c, "sync", ff)?;
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

/// At start and release: fast-forward a clean unit with no commits of its own. Anything else is left as
/// it is. Returns how far the unit's branch is still behind the checkpoint branch.
pub(crate) fn auto_ff(p: &Project, st: &State, unit: &str, cause: &str) -> u64 {
    let Some(c) = ctx(p, unit) else { return 0 };
    if !gitx::branch_exists(&c.repo, &c.ub) || !gitx::branch_exists(&c.repo, &c.cpb) {
        return 0;
    }
    let behind = |c: &Ctx| count(c, &format!("{}..{}", c.ub, c.cpb));
    let free = crate::scope::open_request(st, unit).is_none()
        && st.sync_open.is_none()
        && !matches!(st.unit_state(unit), "review" | "reviewed")
        && count(&c, &format!("{}..{}", c.cpb, c.ub)) == 0
        && status(&c.wt).map(|x| x.is_empty()).unwrap_or(false);
    if free && behind(&c) > 0 && take_in(p, unit, &c, cause, true).is_err() {
        return behind(&c);
    }
    behind(&c)
}
