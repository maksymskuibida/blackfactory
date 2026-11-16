//! `bf checkpoint sync`: the checkpoint takes in the main branch.

use super::*;

/// Where the main branch holds commits the open stack of checkpoints lacks: for each repository, the
/// newest open checkpoint that has a branch there and how many commits of the main branch it lacks.
/// Information only; `bf checkpoint sync` takes them in.
pub fn main_ahead(p: &Project, st: &State, pl: &Plan) -> Vec<Value> {
    let mut out = Vec::new();
    for r in &p.repos {
        if !gitx::branch_exists(&r.path, &r.branch) {
            continue;
        }
        for k in pl.checkpoints.iter().rev() {
            let kid = s(k, "id");
            if st.closed.contains_key(kid) {
                continue;
            }
            let cpb = p.checkpoint_branch(kid);
            if !gitx::branch_exists(&r.path, &cpb) {
                continue;
            }
            let n = gitx::must(
                &r.path,
                &["rev-list", "--count", &format!("{cpb}..{}", r.branch)],
            )
            .unwrap_or_default();
            let n: u64 = n.trim().parse().unwrap_or(0);
            if n > 0 {
                out.push(json!({
                    "repo": r.name, "checkpoint": kid, "commits": n,
                    "command": format!("bf checkpoint sync {kid} --quote \"<the human's words>\""),
                }));
            }
            break;
        }
    }
    out
}

/// `bf checkpoint sync <K> --quote "..." [--resolved]`: the human has the checkpoint branch take in the
/// repository's local main branch (no network: pulling into main is an ordinary git command done before).
/// Only at a quiet point; a conflict leaves the merge in progress in the checkpoint's working copy
/// (`--abort` puts everything back). Checks the human switched off are not run, and the answer names them.
/// What `bf checkpoint sync` was asked to do.
#[derive(Clone, Copy, Default)]
pub struct SyncMode {
    /// The human merged by hand: record it.
    pub resolved: bool,
    /// Put the checkpoint back as it was before an open sync started (no human turn needed).
    pub abort: bool,
    /// The human releases a parked sync back to its builder.
    pub release: bool,
}

pub fn checkpoint_sync(p: &Project, k: &str, quote: &str, mode: SyncMode) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("sync {k}"))?;
    let resolved = mode.resolved;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    if mode.abort {
        return sync_abort(p, &st, k);
    }
    if mode.release {
        return sync_release(p, &st, k, quote);
    }
    let pl = plan::load(p)?;
    if let Some(so) = st.sync_open.clone() {
        if so.checkpoint == k && resolved {
            return sync_hand_resolved(p, &st, &pl, &so, quote);
        }
        return Err(refuse(
            format!("sync {k}"),
            format!("a sync of {} is open ({}): its conflict is being resolved", so.checkpoint, so.phase),
            &[
                "`bf next` gives the directive that settles it (`resolve-sync`, `review-sync` or, when parked, the human's decision).",
                &format!("To go back to how the checkpoint was: `bf checkpoint sync {} --abort`.", so.checkpoint),
            ],
            "`bf status` shows no open sync.",
        ));
    }
    if let Some(f) = st.ck_fix.as_ref() {
        return Err(refuse(
            format!("sync {k}"),
            format!(
                "a checkpoint fix of {} is open ({}): a sync during it would be dropped by the fix's abort or refused at its report",
                f.checkpoint, f.phase
            ),
            &[
                "`bf next` shows what the fix waits for; the sync can start when it has ended.",
                &format!(
                    "To end the fix now: `bf checkpoint fix {} --abort`, then ask for the sync again.",
                    f.checkpoint
                ),
            ],
            "`bf status` shows no open fix.",
        ));
    }
    refuse_if_main_moved(p, &st)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(refuse(
            format!("sync {k}"),
            "no such checkpoint",
            &["Use a checkpoint ID from `bf record list --kind checkpoint`."],
            "The command prints the sync.",
        ));
    };
    if st.closed.contains_key(k) {
        return Err(refuse(
            format!("sync {k}"),
            "the checkpoint is closed; the main branch is taken in by the checkpoint at the tip of the stack",
            &["Sync the newest open checkpoint instead (`bf status` says which)."],
            "`bf status` shows the checkpoint open.",
        ));
    }
    let repos: Vec<Repo> = checkpoint_repos(p, &pl, k)?.into_iter().cloned().collect();
    // The tip of the stack: no later checkpoint has started.
    let after: Vec<&Value> = pl
        .checkpoints
        .iter()
        .skip_while(|c| s(c, "id") != k)
        .skip(1)
        .collect();
    for c in after {
        let cid = s(c, "id");
        let started = p
            .repos
            .iter()
            .any(|r| gitx::branch_exists(&r.path, &p.checkpoint_branch(cid)));
        if started {
            return Err(refuse(
                format!("sync {k}"),
                format!("{cid} already builds on it: only the checkpoint at the tip of the stack takes the main branch in"),
                &[&format!("Sync {cid}, or the newest open checkpoint, instead.")],
                "`bf checkpoint sync` prints the sync.",
            ));
        }
    }
    // A quiet point: no unit between its start and its landing.
    let busy: Vec<String> = st
        .units
        .iter()
        .filter(|(_, u)| {
            matches!(
                u.state.as_str(),
                "running" | "review" | "fixing" | "reviewed" | "blocked" | "parked"
            )
        })
        .map(|(id, u)| format!("{id} ({})", u.state))
        .collect();
    if !busy.is_empty() {
        return Err(refuse(
            format!("sync {k}"),
            format!("units are between start and landing: {}", busy.join(", ")),
            &[
                "Finish and land them (`bf next` shows where each one is).",
                "A parked or blocked unit is released by the human (`bf unit release <U> --quote \"...\"`) and then finished.",
                "Run the command again at the quiet point.",
            ],
            "`bf status` shows no unit running, in review, fixing, blocked or parked.",
        ));
    }
    // What each repository's checkpoint branch lacks.
    struct Step {
        repo: Repo,
        wt: PathBuf,
        main_tip: String,
        old_tip: String,
        lacks: bool,
        hand_merged: bool,
    }
    let cpb = p.checkpoint_branch(k);
    let mut steps: Vec<Step> = Vec::new();
    for r in &repos {
        let main_tip = gitx::must(&r.path, &["rev-parse", &r.branch]).map_err(|_| {
            refuse(
                format!("sync {k}"),
                format!(
                    "the main branch `{}` of `{}` does not exist",
                    r.branch, r.name
                ),
                &["Check the branch the project files name."],
                "`bf status` shows the repository.",
            )
        })?;
        let wt = ensure_checkpoint_worktree(p, k, r)?;
        if gitx::has_uncommitted(&wt) {
            return Err(refuse(
                format!("sync {k}"),
                "the checkpoint's working copy has uncommitted changes",
                &[&format!(
                    "Commit or remove them (`git -C {} status`), then run the command again.",
                    wt.display()
                )],
                "The working copy is clean.",
            ));
        }
        let lacks = !gitx::is_ancestor(&r.path, &main_tip, &cpb);
        let parents =
            gitx::must(&wt, &["rev-list", "--parents", "-n", "1", "HEAD"]).unwrap_or_default();
        let is_merge = parents.split_whitespace().count() >= 3;
        let known = st
            .synced
            .iter()
            .any(|(kk, rr, mm)| kk == k && rr == &r.name && mm == &main_tip);
        let old_tip = gitx::must(&wt, &["rev-parse", "HEAD"]).unwrap_or_default();
        steps.push(Step {
            repo: r.clone(),
            wt,
            main_tip,
            old_tip,
            lacks,
            hand_merged: resolved && !lacks && !known && is_merge,
        });
    }
    if resolved {
        if let Some(s0) = steps.iter().find(|x| x.lacks) {
            return Err(refuse(
                format!("sync {k}"),
                format!("the checkpoint branch does not contain the main branch of `{}` yet", s0.repo.name),
                &[
                    &format!("In {}: `git merge {}`, resolve the conflicts, `git add`, `git commit`.", s0.wt.display(), s0.repo.branch),
                    &format!("Then run `bf checkpoint sync {k} --resolved --quote \"<the human's words>\"` again."),
                ],
                "`bf checkpoint sync` records the merge.",
            ));
        }
    }
    let todo: Vec<&Step> = steps.iter().filter(|x| x.lacks || x.hand_merged).collect();
    if todo.is_empty() {
        return Ok(
            json!({"checkpoint": k, "synced": false, "nothing_to_take_in": true,
            "message": "the checkpoint already holds everything on the main branch; nothing changed"}),
        );
    }
    // The human's words first: nothing is merged for a sync that could not be recorded.
    let (turn, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    // Merge, one repository after another. A conflict is left in progress in the checkpoint's working
    // copy for the factory's own agents to resolve; the human's part is the sentence that asked.
    let floor = evidence::highest_receipt_number(p);
    let mut done: Vec<(String, String, String)> = Vec::new();
    let mut open_repos: Vec<Value> = Vec::new();
    let mut conflicted: Vec<String> = Vec::new();
    let several = steps.len() > 1;
    for step in steps.iter() {
        if !step.lacks {
            if step.hand_merged {
                done.push((
                    step.repo.name.clone(),
                    step.main_tip.clone(),
                    gitx::head_short(&step.wt),
                ));
            }
            continue;
        }
        let out = gitx::run(
            &step.wt,
            &[
                "merge",
                "--no-ff",
                "--no-edit",
                "-m",
                &format!("Take {} into {k}", step.repo.branch),
                &step.main_tip,
            ],
            &[],
        )?;
        let allowed = allowed_set(&step.wt, &step.old_tip, &step.main_tip);
        if out.ok {
            let c = gitx::head_short(&step.wt);
            done.push((step.repo.name.clone(), step.main_tip.clone(), c.clone()));
            open_repos.push(json!({"repo": step.repo.name, "main": step.main_tip, "before": step.old_tip, "files": [], "merge": c, "allowed": allowed}));
        } else {
            let files: Vec<String> =
                gitx::must(&step.wt, &["diff", "--name-only", "--diff-filter=U"])
                    .unwrap_or_default()
                    .lines()
                    .map(|l| l.to_string())
                    .collect();
            for f in &files {
                conflicted.push(if several {
                    format!("{}/{f}", step.repo.name)
                } else {
                    f.clone()
                });
            }
            open_repos.push(json!({"repo": step.repo.name, "main": step.main_tip, "before": step.old_tip, "files": files, "merge": "", "allowed": allowed}));
        }
    }
    if !conflicted.is_empty() {
        events::append(
            p,
            "checkpoint.sync.open",
            json!({"checkpoint": k, "repos": open_repos, "turn": turn, "quote": quote.trim(), "receipt_floor": floor, "failed": []}),
        )?;
        p.write_guard()?;
        return Ok(
            json!({"checkpoint": k, "synced": false, "sync_open": true, "conflicted": conflicted,
            "message": "the main branch conflicts with the checkpoint; the merge is left in progress in the checkpoint's working copy and `bf next` gives the directive that resolves it (`bf checkpoint sync <K> --abort` puts everything back)"}),
        );
    }
    // The joined content is proven before the sync counts: the sync's checks (the command checks every
    // unit of the checkpoint is held to) run on it. The checkpoint's own checks run at verification.
    let failed = run_sync_checks(p, &pl, &st, k, floor)?;
    let skipped = skipped_k_checks(p, &pl, k)?;
    if !failed.is_empty() {
        events::append(
            p,
            "checkpoint.sync.open",
            json!({"checkpoint": k, "repos": open_repos, "turn": turn, "quote": quote.trim(), "receipt_floor": floor, "failed": failed}),
        )?;
        p.write_guard()?;
        return Ok(
            json!({"checkpoint": k, "synced": false, "sync_open": true, "conflicted": [],
            "failed_checks": failed, "skipped_checks": skipped,
            "message": "the main branch merged without a conflict, but the sync's checks fail on the joined content; the merge is kept and `bf next` gives the directive that repairs it (`bf checkpoint sync <K> --abort` puts everything back)"}),
        );
    }
    let reopen_checks = str_list(kr, "checks");
    events::append(
        p,
        "checkpoint.sync",
        json!({
            "checkpoint": k,
            "repos": done.iter().map(|(r, m, c)| json!({"repo": r, "main": m, "merge": c})).collect::<Vec<_>>(),
            "turn": turn, "quote": quote.trim(),
            "reopen_checks": reopen_checks, "receipt_floor": floor,
        }),
    )?;
    // The repository's main branch is where the engine now counts from: the stack holds it.
    let moved: Vec<(String, String)> = done
        .iter()
        .map(|(name, main, _)| (name.clone(), main.clone()))
        .collect();
    events::set_commits(p, &moved)?;
    p.write_guard()?;
    Ok(
        json!({"checkpoint": k, "synced": true, "skipped_checks": skipped,
        "repos": done.iter().map(|(r, m, c)| json!({"repo": r, "main": m, "merge": c})).collect::<Vec<_>>()}),
    )
}

fn sync_wt(p: &Project, k: &str, repo: &str) -> R<PathBuf> {
    let pl = plan::load(p)?;
    let r = p.repo(repo)?;
    checkpoint_wt(p, &pl, k, r)
}

/// The files the resolver of a sync may change in a repository: those either side changed since the merge
/// base, and new test files beside them (a file with `test` in its name in the same folder, or any file
/// in a folder named for tests).
fn allowed_set(wt: &Path, before: &str, main: &str) -> Vec<String> {
    let base = gitx::must(wt, &["merge-base", before, main]).unwrap_or_default();
    let mut files: Vec<String> = Vec::new();
    for side in [before, main] {
        if base.is_empty() {
            break;
        }
        let d = gitx::must(wt, &["diff", "--name-only", &base, side]).unwrap_or_default();
        for f in d.lines() {
            if !files.iter().any(|x| x == f) {
                files.push(f.to_string());
            }
        }
    }
    let mut dirs: Vec<String> = Vec::new();
    for f in &files {
        let d = f
            .rsplit_once('/')
            .map(|x| x.0.to_string())
            .unwrap_or_default();
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    files.truncate(3000);
    for d in dirs {
        let last = d.rsplit('/').next().unwrap_or("").to_lowercase();
        if last.contains("test") {
            files.push(format!("{d}/"));
        } else if d.is_empty() {
            files.push("*test*".into());
        } else {
            files.push(format!("{d}/*test*"));
        }
    }
    files
}

/// A check a sync runs: the label it is shown under (the unit check's ID, or the name of a check every
/// unit must pass) and its command.
#[derive(Clone, Debug)]
pub(crate) struct SyncCheck {
    pub label: String,
    pub argv: Vec<String>,
}

/// The command checks every unit of the checkpoint is held to, each distinct command once: those of
/// its unit check records and of the list of checks every unit must pass, that all its units share.
/// `None` when the units share no command (or there is no unit): the sync then runs the checkpoint's own
/// command checks, as it always did.
pub(crate) fn sync_checks(p: &Project, pl: &Plan, st: &State, k: &str) -> Option<Vec<SyncCheck>> {
    let mut per_unit: Vec<Vec<SyncCheck>> = Vec::new();
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let uid = s(u, "id");
        let mut list: Vec<SyncCheck> = Vec::new();
        for c in pl.unit_checks(uid) {
            if s(c, "how") != "command" || crate::enforce::is_off(st, c) {
                continue;
            }
            list.push(SyncCheck {
                label: s(c, "id").to_string(),
                argv: evidence::command_words(c),
            });
        }
        for a in crate::checks::attached_for_unit(p, pl, st, uid).0 {
            list.push(SyncCheck {
                label: a.name.clone(),
                argv: a.command.clone(),
            });
        }
        per_unit.push(list);
    }
    let first = per_unit.first()?;
    let mut shared: Vec<SyncCheck> = Vec::new();
    for c in first {
        if c.argv.is_empty()
            || shared.iter().any(|x| x.argv == c.argv)
            || !per_unit[1..]
                .iter()
                .all(|l| l.iter().any(|x| x.argv == c.argv))
        {
            continue;
        }
        shared.push(c.clone());
    }
    (!shared.is_empty()).then_some(shared)
}

/// Run the sync's checks on the content now in the checkpoint's working copy. Returns the failures:
/// check, command, exit, output tail. The runs of the units' shared checks are recorded as sync runs, which
/// settle nothing; a checkpoint whose units share no command runs its own command checks (real receipts).
pub(crate) fn run_sync_checks(
    p: &Project,
    pl: &Plan,
    st: &State,
    k: &str,
    sync: u64,
) -> R<Vec<Value>> {
    let Some(list) = sync_checks(p, pl, st, k) else {
        return run_k_checks(p, pl, k);
    };
    let top = p.work_dir().join(k);
    let mut failed = Vec::new();
    for c in list {
        match evidence::sync_run(p, k, &c.label, &c.argv, &top, sync) {
            Ok(r) if r["exit"].as_i64() == Some(0) => {}
            Ok(r) => failed.push(
                json!({"check": c.label, "command": c.argv.join(" "), "exit": r["exit"], "tail": s(&r, "output_tail")}),
            ),
            Err(e) => failed.push(
                json!({"check": c.label, "command": c.argv.join(" "), "exit": null, "tail": format!("{e:?}")}),
            ),
        }
    }
    Ok(failed)
}

/// What the resolver may change, as patterns inside the checkpoint's working copy folder (prefixed by the
/// repository name when several are touched).
pub fn sync_allowed(so: &events::SyncOpen) -> Vec<String> {
    let several = so.repos.len() > 1;
    so.repos
        .iter()
        .flat_map(|r| {
            r.allowed.iter().map(move |f| {
                if several {
                    format!("{}/{f}", r.repo)
                } else {
                    f.clone()
                }
            })
        })
        .collect()
}

/// The conflicted files of an open sync, as paths inside the checkpoint's working copy folder.
pub fn sync_files(so: &events::SyncOpen) -> Vec<String> {
    let several = so.repos.len() > 1;
    so.repos
        .iter()
        .flat_map(|r| {
            r.files.iter().map(move |f| {
                if several {
                    format!("{}/{f}", r.repo)
                } else {
                    f.clone()
                }
            })
        })
        .collect()
}

/// Finish an open sync: the checkpoint holds the main branch. Records `checkpoint.sync`.
fn finish_sync(
    p: &Project,
    pl: &Plan,
    so: &events::SyncOpen,
    human: Option<(u64, &str)>,
) -> R<Value> {
    let kr = pl.checkpoint(&so.checkpoint);
    let reopen_checks = kr.map(|k| str_list(k, "checks")).unwrap_or_default();
    let mut repos: Vec<Value> = Vec::new();
    for r in &so.repos {
        let wt = sync_wt(p, &so.checkpoint, &r.repo)?;
        repos.push(json!({"repo": r.repo, "main": r.main, "merge": gitx::head_short(&wt)}));
    }
    let mut ev = json!({
        "checkpoint": so.checkpoint, "repos": repos,
        "reopen_checks": reopen_checks, "receipt_floor": so.floor,
    });
    match human {
        Some((turn, quote)) => {
            ev["turn"] = json!(turn);
            ev["quote"] = json!(quote.trim());
        }
        None => ev["via_open"] = json!(true),
    }
    events::append(p, "checkpoint.sync", ev)?;
    let moved: Vec<(String, String)> = so
        .repos
        .iter()
        .map(|r| (r.repo.clone(), r.main.clone()))
        .collect();
    events::set_commits(p, &moved)?;
    p.write_guard()?;
    let skipped = skipped_k_checks(p, pl, &so.checkpoint)?;
    Ok(
        json!({"checkpoint": so.checkpoint, "synced": true, "skipped_checks": skipped, "repos": repos}),
    )
}

/// What is still wrong with a resolution: unmerged paths, conflict markers, a merge not concluded or
/// not holding the main branch, or files changed beyond the conflicted ones (compared with git's own
/// automatic merge).
fn sync_problems(p: &Project, so: &events::SyncOpen) -> R<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for r in &so.repos {
        let wt = sync_wt(p, &so.checkpoint, &r.repo)?;
        let unmerged =
            gitx::must(&wt, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        for f in unmerged.lines() {
            out.push(format!(
                "{}: {f} is still unmerged (resolve it, then `git add {f}`)",
                r.repo
            ));
        }
        for f in &r.files {
            let text = fs::read_to_string(wt.join(f)).unwrap_or_default();
            if text
                .lines()
                .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "=======")
            {
                out.push(format!("{}: {f} still has conflict markers", r.repo));
            }
        }
        let merging = gitx::run(&wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[])
            .map(|o| o.ok)
            .unwrap_or(false);
        if merging {
            out.push(format!(
                "{}: the merge is not concluded (`git commit --no-edit` in {})",
                r.repo,
                wt.display()
            ));
        } else if gitx::has_uncommitted(&wt) {
            out.push(format!(
                "{}: the working copy has uncommitted changes",
                r.repo
            ));
        }
        if !merging && !gitx::is_ancestor(&wt, &r.main, "HEAD") {
            out.push(format!(
                "{}: the checkpoint does not contain the main branch's commit {}",
                r.repo,
                &r.main[..r.main.len().min(10)]
            ));
        }
        if !merging && !r.before.is_empty() {
            // Only the conflicted files may differ from what git's own merge produced.
            if let Ok(o) = gitx::run(
                &wt,
                &["merge-tree", "--write-tree", &r.before, &r.main],
                &[],
            ) {
                if let Some(tree) = o
                    .stdout
                    .lines()
                    .next()
                    .filter(|t| t.len() >= 40 && t.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    let diff =
                        gitx::must(&wt, &["diff", "--name-only", tree, "HEAD"]).unwrap_or_default();
                    for f in diff.lines() {
                        if !r.allowed.iter().any(|g| util::path_matches(g, f)) {
                            out.push(format!(
                                "{}: {f} was changed but is not a file either side changed (nor a new test file beside one)",
                                r.repo
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn sync_abort(p: &Project, st: &State, k: &str) -> R<Value> {
    let Some(so) = st.sync_open.clone().filter(|o| o.checkpoint == k) else {
        return Err(refuse(
            format!("abort the sync of {k}"),
            "no sync is open for it",
            &["`bf status` shows an open sync when there is one."],
            "`bf status` shows no open sync.",
        ));
    };
    for r in &so.repos {
        let wt = sync_wt(p, k, &r.repo)?;
        let _ = gitx::run(&wt, &["merge", "--abort"], &[]);
        if !r.before.is_empty() {
            gitx::must(&wt, &["reset", "--hard", "-q", &r.before])?;
        }
    }
    events::append(p, "checkpoint.sync.abort", json!({"checkpoint": k}))?;
    p.write_guard()?;
    Ok(
        json!({"checkpoint": k, "aborted": true, "message": "the checkpoint is as it was before the sync started"}),
    )
}

fn sync_release(p: &Project, st: &State, k: &str, quote: &str) -> R<Value> {
    let Some(so) = st.sync_open.clone().filter(|o| o.checkpoint == k) else {
        return Err(refuse(
            format!("release the sync of {k}"),
            "no sync is open for it",
            &["`bf status` shows an open sync when there is one."],
            "`bf status` shows no open sync.",
        ));
    };
    if so.phase != "parked" {
        return Err(refuse(
            format!("release the sync of {k}"),
            format!("it is `{}`, not parked", so.phase),
            &["`bf next` shows what it waits for."],
            "`bf status` shows the sync back at work.",
        ));
    }
    let (turn, _) = events::pick_turn(st, 0, events::Decide::Approve, quote)?;
    events::append(
        p,
        "checkpoint.sync.release",
        json!({"checkpoint": k, "turn": turn, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    Ok(json!({"checkpoint": k, "released": true}))
}

/// The human resolved the merge by hand in the checkpoint's working copy while a sync was open.
fn sync_hand_resolved(
    p: &Project,
    st: &State,
    pl: &Plan,
    so: &events::SyncOpen,
    quote: &str,
) -> R<Value> {
    let problems = sync_problems(p, so)?;
    if !problems.is_empty() {
        let mut steps = problems.clone();
        steps.push(format!(
            "Then run `bf checkpoint sync {} --resolved --quote \"<the human's words>\"` again.",
            so.checkpoint
        ));
        return Err(refuse_owned(
            format!("sync {}", so.checkpoint),
            format!("the merge is not finished: {}", problems.len()),
            steps,
            "`bf checkpoint sync` records the merge.",
        ));
    }
    // The joined content must be proven too.
    let failed = run_sync_checks(p, pl, st, &so.checkpoint, so.floor)?;
    if !failed.is_empty() {
        let mut steps: Vec<String> = failed
            .iter()
            .map(|f| {
                format!(
                    "{} fails ({})",
                    f["check"].as_str().unwrap_or(""),
                    f["command"].as_str().unwrap_or("")
                )
            })
            .collect();
        steps.push("Repair the joined content, commit, then run the command again.".into());
        return Err(refuse_owned(
            format!("sync {}", so.checkpoint),
            format!(
                "the sync's checks fail on the joined content: {}",
                failed.len()
            ),
            steps,
            "`bf checkpoint sync` records the merge.",
        ));
    }
    let (turn, _) = events::pick_turn(st, 0, events::Decide::Approve, quote)?;
    finish_sync(p, pl, so, Some((turn, quote)))
}

/// `bf report <K>` while a sync is open: the resolver says built or blocked; the reviewer says findings
/// or clean.
pub fn sync_report(
    p: &Project,
    k: &str,
    result: &str,
    role: Option<&str>,
    note: &str,
    findings: Option<&Value>,
    token: Option<&str>,
) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("report on the sync of {k}"))?;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let Some(so) = st.sync_open.clone().filter(|o| o.checkpoint == k) else {
        return Err(refuse(
            format!("report on {k}"),
            "no sync is open for it; a checkpoint is not reported on",
            &["Units are reported on with `bf report <U>`."],
            "`bf status` shows the unit's new state.",
        ));
    };
    let expected = match so.phase.as_str() {
        "resolve" => "builder",
        "review" => "reviewer",
        _ => {
            return Err(refuse(
                format!("report on the sync of {k}"),
                "the sync is parked: the human decides",
                &["The human releases it (`bf checkpoint sync <K> --release --quote \"...\"`) or the conductor aborts it (`--abort`)."],
                "`bf status` shows the sync back at work.",
            ))
        }
    };
    if role.map(|r| r != expected).unwrap_or(false) {
        return Err(refuse(
            format!("report as {}", role.unwrap_or("")),
            format!("the sync of {k} waits for its {expected}"),
            &["Report with the role the directive named."],
            "`bf status` shows the sync in its new phase.",
        ));
    }
    if p.policy.roles.tokens && !holds_token(p, &format!("{k}:{expected}"), token) {
        return Err(refuse(
            format!("report on the sync of {k} as {expected}"),
            format!("a report needs the token of the {expected}'s directive; none, or the wrong one, was given"),
            &["`bf next --json` shows the directive with its `token`; pass it with `--token`."],
            "`bf status` shows the sync in its new phase.",
        ));
    }
    let pl = plan::load(p)?;
    match (so.phase.as_str(), result) {
        ("resolve", "blocked") => {
            events::append(p, "checkpoint.sync.park", json!({"checkpoint": k, "reason": if note.is_empty() { "the resolver is blocked" } else { note }}))?;
            p.write_guard()?;
            Ok(json!({"checkpoint": k, "phase": "parked"}))
        }
        ("resolve", "built" | "fixed") => {
            let problems = sync_problems(p, &so)?;
            if !problems.is_empty() {
                let mut steps = problems.clone();
                steps.push("Fix them, then report again.".into());
                return Err(refuse_owned(
                    format!("report the resolution of {k}"),
                    format!("the resolution is not finished: {} problem(s)", problems.len()),
                    steps,
                    "`bf status` shows the sync in review.",
                ));
            }
            // The joined content must be proven: the sync's checks run on it, through the engine, and
            // the runs are recorded as sync runs.
            let failed = run_sync_checks(p, &pl, &st, k, so.floor)?;
            if !failed.is_empty() {
                let mut steps: Vec<String> = failed
                    .iter()
                    .map(|f| {
                        format!(
                            "{} fails ({}): {}",
                            f["check"].as_str().unwrap_or(""),
                            f["command"].as_str().unwrap_or(""),
                            f["tail"].as_str().unwrap_or("").lines().last().unwrap_or("")
                        )
                    })
                    .collect();
                steps.push("Repair it (only files either side changed, or new test files beside them), commit, run only the tests you change, then report again: the engine runs the sync's checks when you report.".into());
                return Err(refuse_owned(
                    format!("report the resolution of {k}"),
                    format!("the sync's checks fail on the joined content: {}", failed.len()),
                    steps,
                    "`bf status` shows the sync in review.",
                ));
            }
            // Untouched since a clean merge: nothing was resolved, so there is nothing to review.
            let untouched = so.repos.iter().all(|r| {
                r.files.is_empty()
                    && sync_wt(p, k, &r.repo)
                        .map(|wt| gitx::head_short(&wt) == r.merge)
                        .unwrap_or(false)
            });
            if untouched {
                return finish_sync(p, &pl, &so, None);
            }
            events::append(p, "checkpoint.sync.built", json!({"checkpoint": k}))?;
            p.write_guard()?;
            let skipped = skipped_k_checks(p, &pl, k)?;
            Ok(json!({"checkpoint": k, "phase": "review", "skipped_checks": skipped}))
        }
        ("review", "findings") => {
            let listed: Vec<Value> = findings
                .and_then(|f| f.as_array().cloned().or_else(|| f.get("findings").and_then(|x| x.as_array().cloned())))
                .unwrap_or_default();
            if listed.is_empty() {
                return Err(refuse(
                    format!("report findings on the sync of {k}"),
                    "no finding was given",
                    &["Give the findings (`--findings -` with a JSON list), or report `built` when the resolution is right."],
                    "`bf status` shows the sync in its new phase.",
                ));
            }
            let rounds = so.rounds + 1;
            let over = rounds > p.policy.limits.max_rounds;
            events::append(
                p,
                "checkpoint.sync.reviewed",
                json!({"checkpoint": k, "to": if over { "parked" } else { "resolve" }, "findings": listed,
                       "reason": if over { format!("more than {} review rounds", p.policy.limits.max_rounds) } else { String::new() }}),
            )?;
            p.write_guard()?;
            Ok(json!({"checkpoint": k, "phase": if over { "parked" } else { "resolve" }}))
        }
        ("review", "built") => {
            // The sync was asked for in the human's words when it opened.
            let v = finish_sync(p, &pl, &so, None)?;
            Ok(v)
        }
        (ph, r) => Err(refuse(
            format!("report {r} on the sync of {k}"),
            format!("a `{r}` report does not fit a sync in `{ph}`"),
            &["The resolver reports built or blocked; the reviewer reports findings or built (clean)."],
            "`bf status` shows the sync in its new phase.",
        )),
    }
}

/// The brief for the resolver or the reviewer of an open sync.
pub(crate) fn brief_sync(p: &Project, so: &events::SyncOpen, role: &str) -> R<PathBuf> {
    let k = &so.checkpoint;
    let mut md = format!("# Brief: the sync of {k} as {role}\n\n");
    md.push_str(&format!(
        "The human asked for the main branch to be taken into checkpoint {k}. It conflicts in the files below; the merge is in progress in the checkpoint's working copy `{}`.\n\n",
        p.work_dir().join(k).display()
    ));
    let limited = |t: String| -> String {
        let lines: Vec<&str> = t.lines().collect();
        if lines.len() > 200 {
            format!(
                "{}\n... ({} more lines)\n",
                lines[..200].join("\n"),
                lines.len() - 200
            )
        } else {
            t
        }
    };
    for r in &so.repos {
        if r.files.is_empty() {
            continue;
        }
        let wt = sync_wt(p, k, &r.repo)?;
        let base = gitx::must(&wt, &["merge-base", &r.before, &r.main]).unwrap_or_default();
        md.push_str(&format!("## Repository `{}`\n\n", r.repo));
        for f in &r.files {
            md.push_str(&format!("### `{f}`\n\n"));
            if role == "reviewer" {
                md.push_str("The resolution compared with the stack's side:\n\n```diff\n");
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &r.before, "HEAD", "--", f]).unwrap_or_default(),
                ));
                md.push_str(
                    "\n```\n\nThe resolution compared with the main branch's side:\n\n```diff\n",
                );
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &r.main, "HEAD", "--", f]).unwrap_or_default(),
                ));
                md.push_str("\n```\n\n");
            } else {
                md.push_str("What the stack changed:\n\n```diff\n");
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &base, &r.before, "--", f]).unwrap_or_default(),
                ));
                md.push_str("\n```\n\nWhat the main branch changed:\n\n```diff\n");
                md.push_str(&limited(
                    gitx::must(&wt, &["diff", &base, &r.main, "--", f]).unwrap_or_default(),
                ));
                md.push_str("\n```\n\n");
            }
        }
    }
    // The sync's checks on the joined content: the command checks every unit of the checkpoint is held
    // to; a checkpoint whose units share no command is checked by its own command checks.
    let pl = plan::load(p)?;
    if let Some(kr) = pl.checkpoint(k) {
        let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
        let receipts = evidence::load_receipts(p)?;
        let st = events::require_chain(p)?;
        let shared = sync_checks(p, &pl, &st, k);
        let mut entries: Vec<(String, String)> = Vec::new();
        match &shared {
            Some(list) => {
                for c in list {
                    entries.push((c.label.clone(), c.argv.join(" ")));
                }
            }
            None => {
                for cid in str_list(kr, "checks") {
                    let Some(c) = pl
                        .checks
                        .iter()
                        .find(|c| s(c, "id") == cid && s(c, "how") == "command")
                    else {
                        continue;
                    };
                    // A check the human switched off is not run by the sync, so the brief does not ask for it.
                    if crate::enforce::is_off(&st, c) {
                        continue;
                    }
                    entries.push((cid, evidence::command_words(c).join(" ")));
                }
            }
        }
        let shown = !entries.is_empty();
        if shown {
            md.push_str("## The sync's checks on the joined content\n\n");
        }
        for (cid, argv) in &entries {
            let last = receipts.iter().rev().find(|r| {
                r.content() == content
                    && if shared.is_some() {
                        s(&r.v, "type") == "sync"
                            && r.check() == k
                            && s(&r.v, "reason") == cid
                            && s(&r.v, "command") == argv
                    } else {
                        r.check() == cid && r.is_run()
                    }
            });
            let passes = |r: &Receipt| {
                if shared.is_some() {
                    r.seal_ok && r.v["exit"].as_i64() == Some(0)
                } else {
                    r.passes()
                }
            };
            match last {
                Some(r) if passes(r) => {
                    md.push_str(&format!("- {cid} (`{argv}`): passes on this content.\n"))
                }
                Some(r) => {
                    let tail = s(&r.v, "output_tail");
                    let lines: Vec<&str> = tail.lines().collect();
                    let from = lines.len().saturating_sub(40);
                    md.push_str(&format!(
                        "- {cid} (`{argv}`): FAILS (exit {}). Output tail:\n\n```\n{}\n```\n\n",
                        r.v["exit"],
                        lines[from..].join("\n")
                    ));
                }
                None => md.push_str(&format!(
                    "- {cid} (`{argv}`): not run on this content yet.\n"
                )),
            }
        }
        if shown {
            md.push_str(&format!(
                "\nThe engine runs these checks when the resolver reports, on the content in `{}`. The checkpoint's own checks run only at verification, never in a sync. Run only the tests you change, selected by name; never the checkpoint's own checks or the extended suite.\n\n",
                p.work_dir().join(k).display()
            ));
        }
    }
    if role == "builder" {
        md.push_str("## What you may change\n\nOnly files either side changed since the merge base, and new test files beside them:\n\n");
        for g in sync_allowed(so).iter().take(60) {
            md.push_str(&format!("- `{g}`\n"));
        }
        md.push('\n');
    }
    if role == "reviewer" {
        // The whole fix (resolution and repairs) against git's own automatic merge.
        for r in &so.repos {
            let wt = sync_wt(p, k, &r.repo)?;
            if let Ok(o) = gitx::run(
                &wt,
                &["merge-tree", "--write-tree", &r.before, &r.main],
                &[],
            ) {
                if let Some(tree) = o
                    .stdout
                    .lines()
                    .next()
                    .filter(|t| t.len() >= 40 && t.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    md.push_str(&format!("## Everything changed compared with git's automatic merge (`{}`)\n\n```diff\n", r.repo));
                    md.push_str(&limited(
                        gitx::must(&wt, &["diff", tree, "HEAD"]).unwrap_or_default(),
                    ));
                    md.push_str("\n```\n\n");
                }
            }
        }
    }
    if role == "reviewer" {
        md.push_str("## What to judge\n\nFor each file: does the resolution keep what the stack added AND what the main branch added, with no line lost or doubled by mistake? Report with `bf report <K> --role reviewer --result findings --findings -` (a JSON list of `{title, where, action}`), or `--result built` when every file is right. Pass the token of your directive with `--token`.\n");
    } else {
        md.push_str(&format!(
            "## Rules\n\n- Change only the conflicted files named above, in the checkpoint's working copy; nothing else. Keep both sides' intent.\n- Remove every conflict marker (`<<<<<<<`, `=======`, `>>>>>>>`).\n- Then `git add` those files and run `git commit --no-edit` there to conclude the merge. Do not abort, reset or check out anything: the engine aborts.\n- Report with `bf report {k} --role builder --result built --token <token>`. If you cannot resolve it: `bf report {k} --role builder --result blocked --note \"why\"`.\n"
        ));
        if !so.findings.is_empty() {
            md.push_str("\n## Findings of the review to fix\n\n");
            for f in &so.findings {
                md.push_str(&format!(
                    "- {} at {}: {}\n",
                    f["title"].as_str().unwrap_or(""),
                    f["where"].as_str().unwrap_or(""),
                    f["action"].as_str().unwrap_or("")
                ));
            }
        }
    }
    let path = p.briefs_dir().join(format!("{k}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}
