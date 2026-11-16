//! `bf unit land`: merging a reviewed unit into its checkpoint.

use super::*;

pub fn unit_land(p: &Project, unit: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("land {unit}"))?;
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("land {unit}"))?;
    refuse_if_main_moved(p, &st)?;
    let run = need_run(&st)?;
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("land {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints the merge.",
        ));
    };
    let cur = st.unit_state(unit).to_string();
    if cur == "landed" {
        return Ok(json!({"unit": unit, "landed": true, "already": true}));
    }
    if cur != "reviewed" {
        return Err(refuse(
            format!("land {unit}"),
            format!(
                "it is `{}`; only a reviewed unit lands",
                if cur.is_empty() { "not started" } else { &cur }
            ),
            &[
                "Build it, have it reviewed clean, and report both with `bf report`.",
                "`bf next` shows what is waiting.",
            ],
            "`bf status` shows the unit `reviewed`.",
        ));
    }
    let open: Vec<String> = p
        .list_records(schema::kind_by_name("finding"))?
        .iter()
        .filter(|f| s(f, "unit") == unit && is_open(f))
        .map(|f| s(f, "id").to_string())
        .collect();
    if !open.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            format!("open findings: {}", open.join(", ")),
            &["Fix them and have the reviewer report clean."],
            "No open findings remain.",
        ));
    }
    if let Some(why) = crate::wavecheck::land_blocker(&pl, &st, unit) {
        return Err(refuse(
            format!("land {unit}"),
            why,
            &[
                "`bf next` offers the wave check step, or the fix of a failed one.",
                "The unit may be built and reviewed meanwhile; only its landing waits.",
            ],
            "`bf status` shows the wave check passed.",
        ));
    }
    let k = s(u, "checkpoint").to_string();
    let repo = p.repo_of_unit(u)?.clone();
    let cpb = p.checkpoint_branch(&k);
    let ub = p.unit_branch(unit);
    let wt = p.work_dir().join(unit);
    if !wt.join(".git").exists() {
        return Err(refuse(
            format!("land {unit}"),
            "its working copy is missing",
            &["Run `bf unit start` to recreate it."],
            "The working copy exists.",
        ));
    }
    let checks = pl.unit_checks(unit);
    if checks.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            "the unit has no check, so nothing could prove it",
            &["Add a check record with `unit = \"<id>\"` through the plan."],
            "`bf plan check` passes.",
        ));
    }
    let content = p.content_hash(&wt)?;
    let receipts = evidence::load_receipts(p)?;
    let mut missing: Vec<String> = Vec::new();
    for c in &checks {
        let cid = s(c, "id");
        if evidence::satisfied_unit_at(&receipts, cid, &content).is_none() {
            let stale = receipts.iter().any(|r| r.check() == cid && r.passes());
            missing.push(format!(
                "{cid}: {} - run `bf evidence run {cid} --unit {unit} -- <its command>` inside {}",
                if stale {
                    "the receipts on file are for other content"
                } else {
                    "no passing sealed receipt"
                },
                wt.display()
            ));
        }
    }
    // R-76: the checks every unit must pass apply beside the unit's own, and no plan repeats them.
    missing.extend(crate::checks::attached_missing(
        p, &pl, &st, &receipts, unit, &content, &wt,
    ));
    if !missing.is_empty() {
        let mut steps = missing;
        steps.push(format!("Run `bf unit land {unit}` again."));
        return Err(refuse_owned(
            format!("land {unit}"),
            format!(
                "receipts do not match the content now in the working copy (hash {})",
                &content[..12.min(content.len())]
            ),
            steps,
            "`bf evidence verify --unit` prints every check ok.",
        ));
    }
    if gitx::has_uncommitted(&wt) {
        gitx::add_all_without_junk(&wt)?;
        gitx::must(
            &wt,
            &["commit", "-q", "-m", &format!("{unit}: {}", s(u, "title"))],
        )?;
    }
    // Scope: every changed path must be inside the unit's files.
    let diff = gitx::must(
        &repo.path,
        &["diff", "--name-only", &format!("{cpb}...{ub}")],
    )?;
    let scope = crate::scope::unit_files(&st, u);
    let outside: Vec<&str> = diff
        .lines()
        .filter(|f| !scope.iter().any(|g| util::path_matches(g, f)))
        .collect();
    if !outside.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            format!("the branch changes files outside the unit's scope: {}", outside.join(", ")),
            &["Revert those changes in the working copy (`git checkout -- <file>` or remove them), commit, and run the checks again.", "If the scope is wrong, the human changes the plan."],
            "`git diff --name-only` lists only files inside the scope.",
        ));
    }
    crate::scope::landing_check(&st, unit, &repo.path, &format!("{cpb}...{ub}"))?;
    let cpwt = ensure_checkpoint_worktree(p, &k, &repo)?;
    if !gitx::is_ancestor(&repo.path, &ub, &cpb) {
        let before = gitx::must(&cpwt, &["rev-parse", "HEAD"])?;
        let out = gitx::run(
            &cpwt,
            &[
                "merge",
                "--no-ff",
                "--no-edit",
                "-m",
                &format!("Land {unit}: {}", s(u, "title")),
                &ub,
            ],
            &[],
        )?;
        if !out.ok {
            // Put the working copy back exactly as it was before the merge started.
            let _ = gitx::run(&cpwt, &["merge", "--abort"], &[]);
            let _ = gitx::run(&cpwt, &["reset", "--hard", &before], &[]);
            let first = out.stderr.lines().next().unwrap_or("").to_string();
            let conflict = out.stdout.contains("CONFLICT")
                || out.stdout.contains("Automatic merge failed")
                || out.stderr.contains("CONFLICT");
            if !conflict {
                return Err(refuse(
                    format!("land {unit}"),
                    format!("git could not merge into {cpb}: {first}"),
                    &["The checkpoint's working copy was put back as it was. Run `bf unit land` again."],
                    "`bf unit land` prints the merge.",
                ));
            }
            return Err(refuse(
                format!("land {unit}"),
                format!("the merge into {cpb} conflicts: {first}"),
                &["Rebase or merge the checkpoint branch into the unit's working copy and resolve the conflict there.", "Run the checks again, then `bf unit land` again."],
                "`bf unit land` prints the merge.",
            ));
        }
    }
    crash_point("land-after-merge");
    let tip = gitx::head_short(&cpwt);
    events::append(
        p,
        "unit.land",
        json!({"unit": unit, "repo": repo.name, "checkpoint": k, "hash": content, "merge": tip, "run": run}),
    )?;
    p.write_guard()?;
    Ok(
        json!({"unit": unit, "landed": true, "checkpoint": k, "branch": cpb, "merge": tip, "content_hash": content}),
    )
}
