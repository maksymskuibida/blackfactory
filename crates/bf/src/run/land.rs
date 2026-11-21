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
    // R-95: the unit lands only on the content its last clean review judged.
    if let Some(r) = st.units.get(unit).and_then(|u| u.review_rounds.last()) {
        let judged = s(r, "content").to_string();
        if s(r, "result") == "built" && !judged.is_empty() && judged != content {
            let short = |h: &str| h[..12.min(h.len())].to_string();
            let round = r["round"].as_u64().unwrap_or(0);
            events::append(
                p,
                "unit.report",
                json!({
                    "unit": unit, "result": "changed", "role": "engine", "from": "reviewed",
                    "to": "review", "note": format!("the content changed after review round {round}"),
                    "cost": 0.0, "findings": [],
                    "judged_content": judged, "changed_content": content,
                }),
            )?;
            brief::write_brief(p, unit, "reviewer");
            p.write_guard()?;
            return Err(refuse(
                format!("land {unit}"),
                format!(
                    "the content now in the working copy (hash {}) is not the content its last clean review judged (round {round}, hash {})",
                    short(&content),
                    short(&judged)
                ),
                &[
                    &format!("{unit} is back in review. `bf next` gives the reviewer's directive."),
                    "The reviewer reviews what changed since that round and reports with `bf report`.",
                    &format!("Run `bf unit land {unit}` again after a clean report."),
                ],
                "`bf status` shows the unit reviewed, and landing goes on.",
            ));
        }
    }
    let receipts = evidence::load_receipts(p)?;
    let missing: Vec<String> = evidence::unit_landing_needs(p, &pl, &st, &receipts, unit, &content)
        .into_iter()
        .filter(|n| n.met.is_none())
        .map(|n| n.line)
        .collect();
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
            let files: Vec<String> =
                gitx::must(&cpwt, &["diff", "--name-only", "--diff-filter=U"])
                    .unwrap_or_default()
                    .lines()
                    .map(String::from)
                    .collect();
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
            return back_to_builder(p, unit, &k, &cpb, files);
        }
    }
    crash_point("land-after-merge");
    let tip = gitx::head_short(&cpwt);
    events::append(
        p,
        "unit.land",
        json!({"unit": unit, "repo": repo.name, "checkpoint": k, "hash": content, "merge": tip, "run": run}),
    )?;
    crate::scope::wait::resume_waiting(p)?;
    p.write_guard()?;
    Ok(
        json!({"unit": unit, "landed": true, "checkpoint": k, "branch": cpb, "merge": tip, "content_hash": content}),
    )
}

/// The merge into the checkpoint branch conflicts: the unit goes from `reviewed` back to its builder, who
/// gets the checkpoint branch in the working copy with the conflict left there. No attempt is counted.
fn back_to_builder(p: &Project, unit: &str, k: &str, cpb: &str, files: Vec<String>) -> R<Value> {
    use crate::strings::t;
    let named = files.join(", ");
    let note = t(
        "land_conflict.note",
        &[("unit", unit), ("checkpoint", cpb), ("files", &named)],
    );
    events::append(
        p,
        "unit.report",
        json!({
            "unit": unit, "result": "conflict", "role": "engine", "from": "reviewed",
            "to": "fixing", "note": note, "cost": 0.0, "findings": [],
            "conflicts": files, "checkpoint": k,
        }),
    )?;
    let st = events::replay(p)?;
    let synced = unit_sync::take_in_for_land(p, &st, unit);
    p.write_guard()?;
    brief::write_brief(p, unit, "builder");
    let aborted = synced.as_ref().is_some_and(|e| s(e, "how") == "aborted");
    let (because, step) = if aborted {
        (
            "land_conflict.aborted.because",
            "land_conflict.aborted.step",
        )
    } else {
        ("land_conflict.because", "land_conflict.step.back")
    };
    let mut steps = vec![t(step, &[("unit", unit)])];
    if !aborted {
        steps.push(t("land_conflict.step.land", &[("unit", unit)]));
    }
    Err(refuse_owned(
        t("land_conflict.what", &[("unit", unit)]),
        t(because, &[("checkpoint", cpb), ("files", &named)]),
        steps,
        &t("land_conflict.worked", &[]),
    ))
}
