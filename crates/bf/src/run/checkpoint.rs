//! Checkpoints: their working copies, where the main branch stands, closing and the checks the engine runs.

use super::*;

/// The checkpoint that comes just before `k` in plan order, if any.
pub fn previous_checkpoint(pl: &Plan, k: &str) -> Option<String> {
    let i = pl.checkpoints.iter().position(|c| s(c, "id") == k)?;
    if i == 0 {
        None
    } else {
        Some(s(&pl.checkpoints[i - 1], "id").to_string())
    }
}

/// The repositories a checkpoint's units change, in the workspace's order.
pub fn checkpoint_repos<'a>(p: &'a Project, pl: &Plan, k: &str) -> R<Vec<&'a Repo>> {
    let mut out: Vec<&Repo> = Vec::new();
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let r = p.repo_of_unit(u)?;
        if !out.iter().any(|x| x.name == r.name) {
            out.push(r);
        }
    }
    if out.is_empty() {
        if let Some(r) = p.repos.first() {
            out.push(r);
        }
    }
    out.sort_by_key(|r| p.repos.iter().position(|x| x.name == r.name));
    Ok(out)
}

/// Where a checkpoint's working copy of a repository lives: the checkpoint's folder itself when its
/// units change one repository, otherwise a subfolder named for each repository.
pub fn checkpoint_wt(p: &Project, pl: &Plan, k: &str, repo: &Repo) -> R<PathBuf> {
    let root = p.work_dir().join(k);
    Ok(if checkpoint_repos(p, pl, k)?.len() <= 1 {
        root
    } else {
        root.join(&repo.name)
    })
}

pub(crate) fn ensure_checkpoint_worktree(p: &Project, k: &str, repo: &Repo) -> R<PathBuf> {
    let main = &repo.branch;
    // Each checkpoint builds on the one before it in plan order (the first on the main branch), so the
    // last checkpoint's content holds everything and the final verification runs over the whole result.
    let pl = plan::load(p)?;
    let st = events::replay(p)?;
    let prev = previous_checkpoint(&pl, k).filter(|c| st.closed.contains_key(c));
    let base = match &prev {
        Some(c) if gitx::branch_exists(&repo.path, &p.checkpoint_branch(c)) => {
            p.checkpoint_branch(c)
        }
        _ => main.clone(),
    };
    if !gitx::branch_exists(&repo.path, main) {
        return Err(refuse(
            "create a working copy",
            format!("the main branch `{main}` of the repository `{}` does not exist or has no commit", repo.name),
            &[
                "Make an initial commit on the main branch.",
                "Or set the branch the project files name (`main_branch` in factory/config.toml or factory/workspace.toml) to the branch you use.",
            ],
            "`git rev-parse <branch>` prints a commit.",
        ));
    }
    let cpb = p.checkpoint_branch(k);
    if !gitx::branch_exists(&repo.path, &cpb) {
        gitx::must(&repo.path, &["branch", &cpb, &base])?;
    }
    let wt = checkpoint_wt(p, &pl, k, repo)?;
    if !wt.join(".git").exists() {
        if let Some(parent) = wt.parent() {
            fs::create_dir_all(parent)?;
        }
        gitx::must(
            &repo.path,
            &["worktree", "add", &wt.to_string_lossy(), &cpb],
        )?;
    }
    // Work that landed in the earlier checkpoint after this one was cut (an amendment re-opened it) is
    // brought in before more lands here.
    if let Some(c) = &prev {
        let pb = p.checkpoint_branch(c);
        if gitx::branch_exists(&repo.path, &pb) && !gitx::is_ancestor(&repo.path, &pb, &cpb) {
            let out = gitx::run(
                &wt,
                &[
                    "merge",
                    "--no-ff",
                    "--no-edit",
                    "-m",
                    &format!("Bring in {c}"),
                    &pb,
                ],
                &[],
            )?;
            if !out.ok {
                let _ = gitx::run(&wt, &["merge", "--abort"], &[]);
                return Err(refuse(
                    format!("bring {c} into {k}"),
                    format!("the merge conflicts: {}", out.stderr.lines().next().unwrap_or("")),
                    &["The human resolves the conflict between the two checkpoint branches by hand."],
                    "`bf unit land` merges.",
                ));
            }
        }
    }
    Ok(wt)
}

/// Where the main branch stands against what the engine recorded in the log head.
pub struct MainPos {
    /// The repository this is about.
    pub repo: String,
    /// The branch is at a commit the engine did not put there and the human has not accepted.
    pub moved: bool,
    /// Why, in words: the work it holds that should not be there.
    pub detail: String,
    pub tip: String,
    /// A quiet line when the human's own commits moved the branch; it blocks nothing.
    pub note: String,
}

/// Compare the main branch with the commit the engine last put there (or the baseline set at `bf init`
/// or accepted by the human). A move by the human that only touches `factory/` and `.gitignore` is
/// taken as theirs (they commit their records). Anything else is a move to account for. Independently,
/// work of a unit that never landed, or landed in a test run, on the main branch is always one.
pub fn repo_main_position(p: &Project, st: &State, repo: &Repo) -> R<MainPos> {
    repo_main_position_in(p, st, repo, &gitx::heads(&repo.path))
}

/// The same, with the repository's branches already read (`gitx::heads`).
pub fn repo_main_position_in(
    p: &Project,
    st: &State,
    repo: &Repo,
    heads: &gitx::Heads,
) -> R<MainPos> {
    let main = &repo.branch;
    let rname = repo.name.clone();
    let ok = |tip: &str| MainPos {
        repo: rname.clone(),
        moved: false,
        detail: String::new(),
        tip: tip.to_string(),
        note: String::new(),
    };
    if !heads.has(main) {
        // A branch that was there and is gone is a change to account for.
        if events::read_head(p)
            .and_then(|h| h.commit_of(&repo.name))
            .is_some()
        {
            return Ok(MainPos {
                repo: repo.name.clone(),
                moved: true,
                detail: format!("the main branch `{main}` does not exist any more"),
                tip: String::new(),
                note: String::new(),
            });
        }
        return Ok(ok(""));
    }
    let tip = match heads.tip(main) {
        Some(t) => t,
        None => gitx::must(&repo.path, &["rev-parse", main])?,
    };
    let recorded = events::read_head(p).and_then(|h| h.commit_of(&repo.name));
    if recorded.as_deref() == Some(tip.as_str()) {
        return Ok(ok(&tip));
    }
    // Work that should never be on main, by whatever way it got there.
    for (id, u) in &st.units {
        let test_landed =
            u.state == "landed" && st.runs.iter().any(|r| r.id == u.landed_run && r.test_mode);
        let unlanded = u.state != "landed";
        if !(test_landed || unlanded) || u.base_commit.is_empty() {
            continue;
        }
        let ub = p.unit_branch(id);
        if !heads.has(&ub) {
            continue;
        }
        let work = gitx::must(
            &repo.path,
            &["rev-list", "--count", &format!("{}..{}", u.base_commit, ub)],
        )
        .unwrap_or_default();
        if work.trim() == "0" || work.is_empty() {
            continue;
        }
        if gitx::is_ancestor(&repo.path, &ub, main) {
            let why = if test_landed {
                "landed in a test run"
            } else {
                "never landed"
            };
            return Ok(MainPos {
                repo: repo.name.clone(),
                moved: true,
                detail: format!(
                    "the main branch (now at {}) contains the commits of {id}, which {why}",
                    short(&tip)
                ),
                tip,
                note: String::new(),
            });
        }
    }
    // Anything else is the human's own commits (the project's scaffolding, their records, their code):
    // noted in one quiet line, never a reason to stop.
    let Some(rec) = recorded else {
        return Ok(ok(&tip));
    };
    let count = gitx::must(
        &repo.path,
        &["rev-list", "--count", &format!("{rec}..{tip}")],
    )
    .unwrap_or_default();
    let mut m = ok(&tip);
    m.note = format!(
        "the main branch has moved on from {} to {} by commits the engine did not make ({} new); they hold no unlanded or test-run work",
        short(&rec),
        short(&tip),
        if count.trim().is_empty() { "some".to_string() } else { count.trim().to_string() }
    );
    Ok(m)
}

/// Every repository's main branch against what the engine recorded.
pub fn main_positions(p: &Project, st: &State) -> R<Vec<MainPos>> {
    let heads: Vec<gitx::Heads> = p.repos.iter().map(|r| gitx::heads(&r.path)).collect();
    main_positions_in(p, st, &heads)
}

/// The same, with each repository's branches already read, in the order of `p.repos`.
pub fn main_positions_in(p: &Project, st: &State, heads: &[gitx::Heads]) -> R<Vec<MainPos>> {
    p.repos
        .iter()
        .zip(heads)
        .map(|(r, h)| repo_main_position_in(p, st, r, h))
        .collect()
}

/// All repositories together: moved if any is; the details and notes of each, named by repository when
/// there are several.
pub fn main_position(p: &Project, st: &State) -> R<MainPos> {
    Ok(combine_positions(&main_positions(p, st)?))
}

pub(crate) fn combine_positions(all: &[MainPos]) -> MainPos {
    let several = all.len() > 1;
    let label = |m: &MainPos, t: &str| {
        if t.is_empty() {
            String::new()
        } else if several {
            format!("[{}] {t}", m.repo)
        } else {
            t.to_string()
        }
    };
    MainPos {
        repo: all.first().map(|m| m.repo.clone()).unwrap_or_default(),
        moved: all.iter().any(|m| m.moved),
        detail: all
            .iter()
            .map(|m| label(m, &m.detail))
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        tip: all.first().map(|m| m.tip.clone()).unwrap_or_default(),
        note: all
            .iter()
            .map(|m| label(m, &m.note))
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn short(c: &str) -> &str {
    &c[..c.len().min(10)]
}

/// Refuse while the main branch is somewhere the engine does not account for.
pub fn refuse_if_main_moved(p: &Project, st: &State) -> R<()> {
    let m = main_position(p, st)?;
    if !m.moved {
        return Ok(());
    }
    Err(refuse(
        "continue while the main branch is where the engine did not put it",
        m.detail,
        &[
            "Do not push anything. Tell the human what happened: the main branch holds work of a unit that never landed, or of a test run.",
            "They move the branch back with git to the last good commit, or have the work reviewed and landed through the engine.",
        ],
        "`bf status` shows `main: ok`.",
    ))
}

fn was_closed_already(st: &events::State, k: &str) -> bool {
    st.closed.contains_key(k)
}

pub fn checkpoint_close(p: &Project, k: &str, merge: bool) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("close {k}"))?;
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("close {k}"))?;
    refuse_if_main_moved(p, &st)?;
    let pl = plan::load(p)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(refuse(
            format!("close {k}"),
            "no such checkpoint",
            &["Use a checkpoint ID from `bf record list --kind checkpoint`."],
            "The command prints the closure.",
        ));
    };
    if let Some(so) = st.sync_open.as_ref().filter(|o| o.checkpoint == k) {
        return Err(refuse(
            format!("close {k}"),
            format!("a sync of the main branch into it is open ({})", so.phase),
            &[
                "`bf next` gives the directive that settles it.",
                &format!("Or go back: `bf checkpoint sync {k} --abort`."),
            ],
            "`bf status` shows no open sync.",
        ));
    }
    if let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == k) {
        let n = st.ck_fixes.iter().filter(|d| d.checkpoint == k).count() + 1;
        return Err(refuse(
            format!("close {k}"),
            format!(
                "a fix of it is open ({k}-fix-{n}, opened by {}, now in `{}`)",
                match f.source.as_str() {
                    "check" => "a failed check",
                    "wave" => "a failed wave check",
                    _ => "findings",
                },
                f.phase
            ),
            &[
                "`bf next` gives the directive that settles the fix.",
                &format!("Or put the checkpoint back as it was: `bf checkpoint fix {k} --abort`."),
            ],
            "`bf status` shows no open fix.",
        ));
    }
    let after = str_list(kr, "after");
    let unlanded: Vec<&String> = after
        .iter()
        .filter(|u| st.unit_state(u) != "landed")
        .collect();
    if !unlanded.is_empty() {
        return Err(refuse(
            format!("close {k}"),
            format!(
                "units not landed: {}",
                unlanded
                    .iter()
                    .map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            &["Land them with `bf unit land`; `bf next` shows where each one is."],
            "`bf status` shows them landed.",
        ));
    }
    let held = crate::wavecheck::close_blockers(&pl, &st, k);
    if !held.is_empty() {
        return Err(refuse_owned(
            format!("close {k}"),
            "a wave check has not passed",
            held.iter()
                .map(|h| format!("{h}."))
                .chain([
                    "`bf next` offers the wave check step, or the fix of a failed one.".to_string(),
                ])
                .collect(),
            "`bf status` shows every wave check of the checkpoint passed.",
        ));
    }
    // The checkpoint's working copy: its folder, which holds one working copy per repository when its
    // units change several.
    let cpwt = p.work_dir().join(k);
    let cp_repos: Vec<Repo> = checkpoint_repos(p, &pl, k)?.into_iter().cloned().collect();
    for r in &cp_repos {
        let wt = checkpoint_wt(p, &pl, k, r)?;
        if !wt.join(".git").exists() {
            return Err(refuse(
                format!("close {k}"),
                format!(
                    "the checkpoint's working copy in the repository `{}` does not exist",
                    r.name
                ),
                &["Land a unit of this checkpoint in that repository first."],
                "`bf checkpoint close` succeeds.",
            ));
        }
        if gitx::has_uncommitted(&wt) {
            return Err(refuse(
                format!("close {k}"),
                "the checkpoint's working copy has uncommitted changes, so no receipt describes the branch",
                &[&format!("Inspect `git -C {} status`.", wt.display()), "Remove the changes; checkpoint content only changes by landing units."],
                "The working copy is clean.",
            ));
        }
    }
    let content = p.content_hash(&cpwt)?;
    let receipts = evidence::load_receipts(p)?;
    let waived = gates::waived_ids(&st);
    let mut gaps: Vec<String> = Vec::new();
    let mut via_amend: Vec<Value> = Vec::new();
    for id in str_list(kr, "proves") {
        if let Some(a) = waived.get(&id) {
            via_amend.push(json!({"id": id, "amendment": a}));
            continue;
        }
        let checks: Vec<&Value> = pl
            .checks
            .iter()
            .filter(|c| {
                str_list(kr, "checks").iter().any(|x| x == s(c, "id"))
                    && str_list(c, "proves").contains(&id)
            })
            .collect();
        // A command or agent check is settled by its receipt at this content; a human check only by the
        // human's own gate. An attached file proves nothing about a human check.
        let ok = crate::enforce::id_proven(&st, &receipts, &checks, k, &content);
        let marked: Vec<String> = checks
            .iter()
            .filter_map(|c| {
                evidence::standing_outcome(&receipts, s(c, "id"), &content).map(|r| {
                    format!(
                        "{} is marked {} ({})",
                        s(c, "id"),
                        r.outcome().unwrap_or(""),
                        s(&r.v, "reason")
                    )
                })
            })
            .collect();
        if !ok && !marked.is_empty() {
            gaps.push(format!(
                "{id}: {}; the human decides whether to fix the cause and run it again, to accept the gap by amendment, or to change the plan",
                marked.join("; ")
            ));
        } else if !ok {
            let names: Vec<&str> = checks.iter().map(|c| s(c, "id")).collect();
            gaps.push(format!(
                "{id}: no sealed passing receipt at this content (hash {}) from {}; run `bf evidence run <check> -- <its command>` inside {}, or ask the human to approve an amendment",
                &content[..12.min(content.len())],
                if names.is_empty() { "any check".to_string() } else { names.join(" or ") },
                cpwt.display()
            ));
        }
    }
    if crate::enforce::stops(kr) && !st.gate_passed("checkpoint-step", Some(k)) {
        gaps.push(format!("the stop `{}` has no approved checkpoint-step gate: run `bf gate open checkpoint-step --ref {k}`", if s(kr, "human_step").is_empty() { s(kr, "stop_why") } else { s(kr, "human_step") }));
    }
    let (mut check_gaps, mut unproven_names) =
        crate::enforce::checkpoint_gaps(p, &pl, &st, &receipts, kr, &content)?;
    // R-49: no checkpoint closes before its review ran on the content being closed.
    let held_by = if crate::ckreview::required(&st) {
        crate::ckreview::open_blocking_named(p, &st, k)?
    } else {
        vec![]
    };
    if !was_closed_already(&st, k) && !held_by.is_empty() {
        let (id, sev, title) = &held_by[0];
        check_gaps.push(format!(
            "the finding {id} ({sev}: {title}) of the checkpoint review is still open and holds the close: `bf next` gives the way on"
        ));
        unproven_names.push(crate::checks::CHECKPOINT_REVIEW.to_string());
    } else if !was_closed_already(&st, k) && crate::ckreview::needed(p, &st, k, &content)? {
        check_gaps.push(format!(
            "the checkpoint review has not run on this content (hash {}): `bf next` gives the reviewer's directive",
            &content[..12.min(content.len())]
        ));
        unproven_names.push(crate::checks::CHECKPOINT_REVIEW.to_string());
    }
    let has_check_gaps = !check_gaps.is_empty();
    gaps.extend(check_gaps);
    if !gaps.is_empty() {
        let mut steps = gaps.clone();
        if has_check_gaps {
            steps.extend(crate::enforce::way_out_steps(&st, &unproven_names));
        }
        steps.push(format!("Run `bf checkpoint close {k}` again."));
        let because = match held_by.first() {
            Some((id, sev, title)) if !was_closed_already(&st, k) => {
                let head = format!(
                    "the finding {id} ({sev}: {title}) of the checkpoint review is still open and holds the close"
                );
                if gaps.len() > 1 {
                    format!("{head}, and {} other gap(s) have no proof", gaps.len() - 1)
                } else {
                    head
                }
            }
            _ => format!("{} ID(s) have no proof", gaps.len()),
        };
        return Err(refuse_owned(
            format!("close {k}"),
            because,
            steps,
            "`bf checkpoint close` prints the closure.",
        ));
    }
    let was_closed = st.closed.contains_key(k);
    let mut merged = false;
    if merge {
        // Main moves only with a human act after the closure: an approved checkpoint-step gate opened
        // after the close event. (D-16: main moves when a human merges.)
        let closed_seq = st.closed.get(k).map(|c| c.seq);
        let Some(closed_seq) = closed_seq else {
            return Err(refuse(
                format!("merge {k} onto the main branch"),
                "the checkpoint is not closed yet; merging follows the closure and a human act",
                &[&format!("Run `bf checkpoint close {k}` first.")],
                "`bf status` lists the checkpoint as closed.",
            ));
        };
        // The last checkpoint holds everything: it merges only when every required check has proof.
        if crate::enforce::last_checkpoint(&pl)
            .map(|l| s(l, "id") == k)
            .unwrap_or(false)
        {
            crate::enforce::refuse_if_unproven_for(
                p,
                &pl,
                &format!("merge {k} onto the main branch"),
                &format!("bf checkpoint close {k} --merge"),
            )?;
        }
        // A run with simulated approvals can never reach the main branch.
        let mut owned: Vec<String> = after.clone();
        for c in &pl.checkpoints {
            if s(c, "id") == k {
                break;
            }
            owned.extend(str_list(c, "after"));
        }
        let tainted_unit = owned.iter().any(|u| {
            let lr = st
                .units
                .get(u)
                .map(|x| x.landed_run.clone())
                .unwrap_or_default();
            st.runs.iter().any(|r| r.id == lr && r.test_mode)
        });
        let tainted_gate = ["idea", "requirements", "ui", "architecture", "plan"]
            .iter()
            .any(|g| {
                st.latest_gate(g, None)
                    .map(|x| x.simulated)
                    .unwrap_or(false)
            })
            || st
                .latest_gate("checkpoint-step", Some(k))
                .map(|x| x.simulated)
                .unwrap_or(false);
        if tainted_unit || tainted_gate {
            return Err(refuse(
                format!("merge {k} onto the main branch"),
                "the run contains a simulated approval or test-mode work; such a run can never land on the main branch",
                &["Leave the checkpoint branch as it is for inspection.", "Redo the work in a real run: the human approves each gate in their own words."],
                "`bf checkpoint close` without --merge still records the closure.",
            ));
        }
        let approved_after = st.gates.iter().any(|g| {
            g.kind == "checkpoint-step"
                && g.refr.as_deref() == Some(k)
                && g.status == "approved"
                && !g.simulated
                && g.open_seq > closed_seq
        });
        if !approved_after {
            return Err(refuse(
                format!("merge {k} onto the main branch"),
                "merging onto the main branch needs the human's go-ahead after the checkpoint closed",
                &[
                    &format!("Open the gate: `bf gate open checkpoint-step --ref {k}` and show the human the checkpoint page."),
                    "When the human says in chat to merge it, `bf gate approve <gate> --quote \"<their words>\"`.",
                    &format!("Then run `bf checkpoint close {k} --merge` again."),
                ],
                "The command prints the merge.",
            ));
        }
        // Every repository the checkpoint touches: its main checkout must be on its main branch first.
        for r in &cp_repos {
            let main = &r.branch;
            if gitx::current_branch(&r.path).as_deref() != Some(main.as_str()) {
                return Err(refuse(
                    format!("merge {k} onto {main}"),
                    format!("the main checkout of `{}` is not on `{main}`", r.name),
                    &[
                        &format!(
                            "Switch the main checkout ({}) to `{main}` (the human's step).",
                            r.path.display()
                        ),
                        "Run the command again.",
                    ],
                    "The command prints the merge commit.",
                ));
            }
        }
        let cpb = p.checkpoint_branch(k);
        for r in &cp_repos {
            let main = r.branch.clone();
            if !gitx::is_ancestor(&r.path, &cpb, &main) {
                let out = gitx::run(
                    &r.path,
                    &[
                        "merge",
                        "--no-ff",
                        "--no-edit",
                        "-m",
                        &format!("Close {k}: {}", s(kr, "title")),
                        &cpb,
                    ],
                    &[],
                )?;
                if !out.ok {
                    let _ = gitx::run(&r.path, &["merge", "--abort"], &[]);
                    return Err(refuse(
                        format!("merge {k} onto {main}"),
                        format!("git refused the merge in `{}`: {}", r.name, out.stderr.lines().next().unwrap_or("")),
                        &["Commit or stash local changes that touch the same files, or resolve the conflict by hand (the human's step).", "Run the command again."],
                        "The command prints the merge commit.",
                    ));
                }
            }
            merged = true;
            // Remember the commit the engine put on the main branch.
            if let Ok(tip) = gitx::must(&r.path, &["rev-parse", &main]) {
                events::set_commits(p, &[(r.name.clone(), tip)])?;
            }
        }
    }
    let unproven_off: Vec<Value> = crate::enforce::off_unproven(&pl, &st, kr)
        .into_iter()
        .map(|(id, c)| json!({"id": id, "checks": c}))
        .collect();
    if !was_closed || merged {
        // The waiver is fixed here: the ledger reads it from the log, not from a later run's settings.
        let off_names: Vec<String> = crate::checks::off_closing(&st).keys().cloned().collect();
        events::append(
            p,
            "checkpoint.close",
            json!({"checkpoint": k, "merged": merged, "hash": content, "unproven_off": unproven_off, "off_names": off_names, "run": crate::enforce::latest_run_id(&st)}),
        )?;
        p.write_guard()?;
    }
    Ok(
        json!({"checkpoint": k, "closed": true, "already": was_closed && !merged, "merged": merged, "content_hash": content, "amendments": via_amend, "checks_off": crate::enforce::off_summary(&st),
            "unproven_off": unproven_off}),
    )
}

/// The command checks of a checkpoint that the human switched off: a sync leaves them out, as the close does.
pub(crate) fn skipped_k_checks(p: &Project, pl: &Plan, k: &str) -> R<Vec<String>> {
    let Some(kr) = pl.checkpoint(k) else {
        return Ok(vec![]);
    };
    let st = events::require_chain(p)?;
    Ok(str_list(kr, "checks")
        .into_iter()
        .filter(|cid| {
            pl.checks.iter().any(|c| {
                s(c, "id") == *cid && s(c, "how") == "command" && crate::enforce::is_off(&st, c)
            })
        })
        .collect())
}

/// The id of a receipt that shows the command passed for the check on the content now in `top`.
fn passing_receipt(p: &Project, check: &str, argv: &[String], top: &Path) -> R<Option<String>> {
    let content = p.content_hash(top)?;
    let command = argv.join(" ");
    Ok(evidence::load_receipts(p)?
        .iter()
        .find(|r| {
            r.is_run()
                && r.seal_ok
                && r.check() == check
                && r.content() == content
                && s(&r.v, "command") == command
                && r.passes()
        })
        .map(|r| r.id().to_string()))
}

/// Run the checkpoint's own command checks that are switched on, on the content now in its working copy,
/// through the engine (real receipts). Returns the failures: check, command, exit, output tail. Agent-judged checks and a
/// checkpoint with none skip this.
pub(crate) fn run_k_checks(p: &Project, pl: &Plan, k: &str) -> R<Vec<Value>> {
    let Some(kr) = pl.checkpoint(k) else {
        return Ok(vec![]);
    };
    let off = skipped_k_checks(p, pl, k)?;
    let top = p.work_dir().join(k);
    let mut failed = Vec::new();
    for cid in str_list(kr, "checks") {
        let Some(c) = pl.checks.iter().find(|c| s(c, "id") == cid) else {
            continue;
        };
        if s(c, "how") != "command" || off.contains(&cid) {
            continue;
        }
        let argv = evidence::command_words(c);
        // A check that already passed on this very content (the fix builder ran it through the engine)
        // is not run again: its receipt stands (R-32).
        // Each skip is logged with the receipt it reused (R-71).
        if let Some(receipt) = passing_receipt(p, &cid, &argv, &top)? {
            events::append(
                p,
                "checkpoint.check.skip",
                json!({"checkpoint": k, "check": cid, "receipt": receipt,
                       "command": argv.join(" "),
                       "note": crate::strings::t("accept.checks.skip", &[("k", k), ("check", &cid), ("receipt", &receipt)])}),
            )?;
            continue;
        }
        // The engine runs the checks again on content they may already have run on: that is a deliberate
        // repeat, stated as one.
        let attempt = evidence::next_attempt(p, &cid, &argv, &top)?;
        let repeat = (attempt > 1).then_some((
            attempt,
            "the engine runs the checkpoint's checks again after a fix or a sync",
        ));
        match evidence::run_with(p, &cid, None, &argv, &top, repeat) {
            Ok(r) if r["exit"].as_i64() == Some(0) => {}
            Ok(r) => failed.push(
                json!({"check": cid, "command": argv.join(" "), "exit": r["exit"], "tail": s(&r, "output_tail")}),
            ),
            Err(e) => failed.push(
                json!({"check": cid, "command": argv.join(" "), "exit": null, "tail": format!("{e:?}")}),
            ),
        }
    }
    Ok(failed)
}
