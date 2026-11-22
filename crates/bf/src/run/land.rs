//! `bf unit land`: one landing turn for each checkpoint (D-115, D-116, FU-47). The turn merges every
//! reviewed unit of the checkpoint that passes the rules of a landing, runs the run's checks once on the
//! merged content, and writes one `unit.land` event for each unit that passed.

use super::*;
use crate::checks::Attached;
use crate::lock;
use crate::strings::t;

/// A unit that passes every rule of a landing now.
struct Ready {
    unit: String,
    title: String,
    /// The content of the unit's own working copy, which its last clean review judged.
    content: String,
    wt: PathBuf,
    repo: Repo,
    ub: String,
}

/// A unit merged into the checkpoint's working copy: where the branch stood before and after.
struct Merged {
    r: Ready,
    before: String,
    tip: String,
}

enum Merge {
    Done { before: String, tip: String },
    Conflict,
    Failed(String),
}

enum Verdict {
    Pass,
    Fail { check: String, tests: Vec<String> },
}

fn because(f: &crate::refusal::Fail) -> String {
    match f {
        crate::refusal::Fail::Refused(r) => r.because.clone(),
        crate::refusal::Fail::Error(e) => e.clone(),
    }
}

fn waits_turn(unit: &str, k: &str) -> Value {
    json!({"unit": unit, "landed": false, "waiting": true, "checkpoint": k,
        "line": t("land_merge.waits.turn", &[("unit", unit), ("checkpoint", k)])})
}

/// The state of the unit as a landing needs it: landed already (the answer), reviewed (nothing), or refused.
fn check_state(st: &State, pl: &Plan, unit: &str) -> R<Option<Value>> {
    if pl.unit(unit).is_none() {
        return Err(refuse(
            format!("land {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints the merge.",
        ));
    }
    let cur = st.unit_state(unit).to_string();
    if cur == "landed" {
        return Ok(Some(json!({"unit": unit, "landed": true, "already": true})));
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
    Ok(None)
}

pub fn unit_land(p: &Project, unit: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("land {unit}"))?;
    refuse_if_main_moved(p, &st)?;
    need_run(&st)?;
    let pl = plan::load(p)?;
    if let Some(v) = check_state(&st, &pl, unit)? {
        return Ok(v);
    }
    let k = pl
        .unit(unit)
        .map(|u| s(u, "checkpoint").to_string())
        .unwrap_or_default();
    // One turn for each checkpoint: a second landing waits behind it and counts as no attempt.
    let Some(_turn) = lock::turn(p, &k)? else {
        return Ok(waits_turn(unit, &k));
    };
    let mut marked = false;
    let res = run_turn(p, unit, &k, &mut marked);
    if res.is_err() && marked {
        // Whatever stopped the turn, its own merges go back; a refusal about the branch keeps the mark.
        if let (Ok(_g), Ok(st), Ok(pl)) = (
            lock::engine(p, &format!("land {unit}")),
            events::replay(p),
            plan::load(p),
        ) {
            let _ = recover(p, &st, &pl, &k, unit);
        }
    }
    res
}

/// Why no turn opens now: a sync of the main branch, a fix or a review round of the checkpoint is open.
pub fn open_reason(p: &Project, st: &State, k: &str) -> Option<&'static str> {
    if st.sync_open.as_ref().is_some_and(|x| x.checkpoint == k) {
        Some("sync")
    } else if st.ck_fix.as_ref().is_some_and(|x| x.checkpoint == k) {
        Some("fix")
    } else if crate::ckreview::rounds(p, k)
        .ok()
        .and_then(|rs| rs.last().map(|r| r.result.is_none()))
        .unwrap_or(false)
    {
        Some("review")
    } else {
        None
    }
}

/// Test seam: hold the turn open at a named point until the process's input ends.
pub(crate) fn hold_point(name: &str, k: &str) {
    if std::env::var("BF_HOLD_AT").is_ok_and(|v| v == name) {
        eprintln!("{}", t("land_merge.hold.line", &[("checkpoint", k)]));
        let mut sink = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut sink);
    }
}

fn run_turn(p: &Project, unit: &str, k: &str, marked: &mut bool) -> R<Value> {
    let eng = lock::engine(p, &format!("land {unit}"))?;
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("land {unit}"))?;
    refuse_if_main_moved(p, &st)?;
    let run = need_run(&st)?;
    let pl = plan::load(p)?;
    if let Some(v) = check_state(&st, &pl, unit)? {
        return Ok(v);
    }
    // The unit's own rules come first: a refusal about the unit is more to the point than a wait.
    ready(p, &st, &pl, &evidence::load_receipts(p)?, unit, true)?;
    if let Some(why) = open_reason(p, &st, k) {
        let w = t(&format!("land_merge.waits.why.{why}"), &[]);
        return Ok(
            json!({"unit": unit, "landed": false, "waiting": true, "checkpoint": k,
            "line": t("land_merge.waits.open", &[("unit", unit), ("why", &w), ("checkpoint", k)])}),
        );
    }
    recover(p, &st, &pl, k, unit)?;

    // The units of the turn: every reviewed unit of the checkpoint that passes the rules now, in plan order.
    let receipts = evidence::load_receipts(p)?;
    let mut turn: Vec<Ready> = Vec::new();
    let mut left: Vec<(String, String)> = Vec::new();
    for cu in pl.units.iter().filter(|x| s(x, "checkpoint") == k) {
        let id = s(cu, "id");
        if st.unit_state(id) != "reviewed" {
            continue;
        }
        if st.units.get(id).is_some_and(|x| x.paused) && id != unit {
            left.push((id.to_string(), t("land_merge.left.paused", &[])));
            continue;
        }
        match ready(p, &st, &pl, &receipts, id, id == unit) {
            Ok(r) => turn.push(r),
            Err(f) if id == unit => return Err(f),
            Err(f) => left.push((id.to_string(), because(&f))),
        }
    }
    let cpb = p.checkpoint_branch(k);

    // The mark: where each checkpoint branch stands and which units the turn holds, before the first merge.
    let mut cps: Vec<(Repo, PathBuf)> = Vec::new();
    for r in &turn {
        if !cps.iter().any(|c| c.0.name == r.repo.name) {
            let wt = ensure_checkpoint_worktree(p, k, &r.repo)?;
            cps.push((r.repo.clone(), wt));
        }
    }
    let cpwt_of = |name: &str| -> PathBuf {
        cps.iter()
            .find(|c| c.0.name == name)
            .map(|c| c.1.clone())
            .unwrap_or_default()
    };
    let mut heads: Vec<(String, String)> = Vec::new();
    for (repo, wt) in &cps {
        heads.push((repo.name.clone(), gitx::must(wt, &["rev-parse", "HEAD"])?));
    }
    fs::write(
        lock::turn_mark(p, k),
        serde_json::to_string_pretty(&json!({
            "checkpoint": k,
            "units": turn.iter().map(|r| r.unit.clone()).collect::<Vec<_>>(),
            "heads": heads.iter().map(|(n, c)| json!({"repo": n, "commit": c})).collect::<Vec<_>>(),
        }))
        .unwrap_or_default(),
    )?;
    *marked = true;
    // The turn's merges sit on a detached head: the checkpoint branch itself stays where it was until the
    // turn passes, so no automatic sync of another unit can take in a merge that is later put back.
    for (_, wt) in &cps {
        gitx::must(wt, &["checkout", "-q", "--detach"])?;
    }

    // The merge: each unit into the checkpoint's own working copy, in order. No event yet.
    let mut merged: Vec<Merged> = Vec::new();
    let mut conflicted: Vec<Ready> = Vec::new();
    for r in turn {
        match merge_one(&cpwt_of(&r.repo.name), &r)? {
            Merge::Done { before, tip } => merged.push(Merged { r, before, tip }),
            Merge::Conflict => conflicted.push(r),
            Merge::Failed(why) => {
                let why = t(
                    "land_merge.left.merge_failed",
                    &[("checkpoint", &cpb), ("why", &why)],
                );
                if r.unit == unit {
                    return Err(refuse_owned(
                        t("land_merge.fail.what", &[("unit", unit)]),
                        why,
                        vec![t("land_merge.left.step", &[("unit", unit)])],
                        t("land_merge.left.worked", &[]),
                    ));
                }
                left.push((r.unit.clone(), why));
            }
        }
    }
    crash_point("land-detached");

    // The check, once, with only the turn's lock held.
    drop(eng);
    hold_point("land-turn", k);
    let top = p.work_dir().join(k);
    let mut landed: Vec<(Ready, String, String)> = Vec::new();
    let mut blamed: Vec<(Ready, String, Vec<String>)> = Vec::new();
    if !merged.is_empty() {
        let several = merged.len() > 1;
        let (content, verdict) = {
            let refs: Vec<&Ready> = merged.iter().map(|m| &m.r).collect();
            let suite = suite_for(p, &pl, &st, &refs);
            run_landing(p, &suite, &top, &refs, !several)?
        };
        match verdict {
            Verdict::Pass => {
                for m in merged {
                    landed.push((m.r, m.tip, content.clone()));
                }
            }
            Verdict::Fail { check, tests } if !several => {
                let m = merged.into_iter().next().unwrap();
                {
                    let _g = lock::engine(p, &format!("land {unit}"))?;
                    gitx::must(&cpwt_of(&m.r.repo.name), &["reset", "--hard", &m.before])?;
                }
                blamed.push((m.r, check, tests));
            }
            Verdict::Fail { .. } => {
                // Several units failed together: the branch goes back to the mark and the units land one at a
                // time in the same order, each with its own run.
                let again: Vec<Ready> = merged.into_iter().map(|m| m.r).collect();
                {
                    let _g = lock::engine(p, &format!("land {unit}"))?;
                    put_back(p, &pl, k, &heads)?;
                }
                for r in again {
                    let cpwt = cpwt_of(&r.repo.name);
                    let step = {
                        let _g = lock::engine(p, &format!("land {unit}"))?;
                        merge_one(&cpwt, &r)?
                    };
                    match step {
                        Merge::Conflict => conflicted.push(r),
                        Merge::Failed(why) => {
                            let why = t(
                                "land_merge.left.merge_failed",
                                &[("checkpoint", &cpb), ("why", &why)],
                            );
                            if r.unit == unit {
                                // The requested unit neither lands nor waits: it is refused, as at the first merge.
                                return Err(refuse_owned(
                                    t("land_merge.fail.what", &[("unit", unit)]),
                                    why,
                                    vec![t("land_merge.left.step", &[("unit", unit)])],
                                    t("land_merge.left.worked", &[]),
                                ));
                            }
                            left.push((r.unit.clone(), why));
                        }
                        Merge::Done { before, tip } => {
                            let (content, v) = {
                                let suite = suite_for(p, &pl, &st, &[&r]);
                                run_landing(p, &suite, &top, &[&r], true)?
                            };
                            match v {
                                Verdict::Pass => landed.push((r, tip, content)),
                                Verdict::Fail { check, tests } => {
                                    let _g = lock::engine(p, &format!("land {unit}"))?;
                                    gitx::must(&cpwt, &["reset", "--hard", &before])?;
                                    blamed.push((r, check, tests));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // The outcome: one `unit.land` event for each unit, in order; then the mark goes.
    let _eng = lock::engine(p, &format!("land {unit}"))?;
    // The log is read again: the suite ran for a long time without the engine lock.
    let st3 = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st3, &format!("land {unit}"))?;
    refuse_if_main_moved(p, &st3)?;
    for (r, _, _) in &landed {
        if let Some(why) = changed_since(p, &st3, &r.unit) {
            abandon(p, &pl, k, &cpb, &heads)?;
            let _ = fs::remove_file(lock::turn_mark(p, k));
            *marked = false;
            return Err(refuse_owned(
                t("land_merge.recheck.what", &[("unit", unit)]),
                t(
                    "land_merge.recheck.because",
                    &[("unit", &r.unit), ("why", &why)],
                ),
                vec![t("land_merge.recheck.step", &[("unit", unit)])],
                t("land_merge.recheck.worked", &[]),
            ));
        }
    }
    // Only now does each checkpoint branch move, to where the turn's merges end.
    for (_, wt) in &cps {
        gitx::must(wt, &["checkout", "-q", "-B", &cpb, "HEAD"])?;
    }
    crash_point("land-after-merge");
    for (r, tip, merged_hash) in &landed {
        events::append(
            p,
            "unit.land",
            json!({"unit": r.unit, "repo": r.repo.name, "checkpoint": k, "hash": r.content,
                "merged_hash": merged_hash, "merge": tip, "run": run}),
        )?;
    }
    let _ = fs::remove_file(lock::turn_mark(p, k));
    *marked = false;
    // Each landed unit's working copy goes; one that holds something stays and says why.
    let mut cleaned: Vec<(String, cleanup::Removal)> = Vec::new();
    for (r, _, _) in &landed {
        if let Ok(x) = cleanup::remove_unit_copy(p, &r.unit) {
            cleaned.push((r.unit.clone(), x));
        }
    }

    let mut requested_err: Option<crate::refusal::Fail> = None;
    let mut sent_back: Vec<(String, String)> = Vec::new();
    for (r, check, tests) in &blamed {
        let named = if tests.is_empty() {
            t("land_merge.fail.none", &[])
        } else {
            tests.join(", ")
        };
        let vals = [
            ("unit", r.unit.as_str()),
            ("checkpoint", cpb.as_str()),
            ("check", check.as_str()),
            ("tests", named.as_str()),
        ];
        events::append(
            p,
            "unit.report",
            json!({
                "unit": r.unit, "result": "suite", "role": "engine", "from": "reviewed",
                "to": "fixing", "note": t("land_merge.fail.note", &vals), "cost": 0.0,
                "findings": [], "checkpoint": k,
            }),
        )?;
        let st2 = events::replay(p)?;
        let _ = unit_sync::take_in_for_land(p, &st2, &r.unit);
        p.write_guard()?;
        brief::write_brief(p, &r.unit, "builder");
        if r.unit == unit {
            requested_err = Some(refuse_owned(
                t("land_merge.fail.what", &[("unit", unit)]),
                t("land_merge.fail.because", &vals),
                vec![t("land_merge.fail.step", &vals)],
                t("land_merge.fail.worked", &[]),
            ));
        } else {
            sent_back.push((r.unit.clone(), t("land_merge.fail.note", &vals)));
        }
    }
    // A merge that conflicted is tried again against the branch as the turn left it: only a conflict
    // that is still there sends the unit to its builder.
    for r in &conflicted {
        match conflict_files(&r.repo.path, &cpb, &r.ub) {
            Some(files) => {
                if let Err(f) = back_to_builder(p, &r.unit, k, &cpb, files) {
                    if r.unit == unit {
                        requested_err = Some(f);
                    } else if events::replay(p)?.unit_state(&r.unit) == "reviewed" {
                        // A conflict in a protected path is aborted and the unit stays reviewed.
                        left.push((r.unit.clone(), because(&f)));
                    } else {
                        sent_back.push((r.unit.clone(), because(&f)));
                    }
                }
            }
            None => {
                let why = t("land_merge.left.conflict_gone", &[]);
                if r.unit == unit {
                    requested_err = Some(refuse_owned(
                        t("land_merge.fail.what", &[("unit", unit)]),
                        why,
                        vec![t("land_merge.left.step", &[("unit", unit)])],
                        t("land_merge.left.worked", &[]),
                    ));
                } else {
                    left.push((r.unit.clone(), why));
                }
            }
        }
    }
    crate::scope::wait::resume_waiting(p)?;
    p.write_guard()?;
    if let Some(e) = requested_err {
        return Err(e);
    }
    let Some((me, tip, merged_hash)) = landed.iter().find(|l| l.0.unit == unit) else {
        return Ok(json!({"unit": unit, "landed": false}));
    };
    let others: Vec<&str> = landed
        .iter()
        .filter(|l| l.0.unit != unit)
        .map(|l| l.0.unit.as_str())
        .collect();
    let mut lines = vec![t(
        "land_merge.line.landed",
        &[("unit", unit), ("branch", &cpb)],
    )];
    if !others.is_empty() {
        lines.push(t(
            "land_merge.line.same_turn",
            &[("units", &others.join(", "))],
        ));
    }
    for (u, why) in &left {
        lines.push(t("land_merge.line.left", &[("unit", u), ("reason", why)]));
    }
    for (u, why) in &sent_back {
        lines.push(t(
            "land_merge.line.sent_back",
            &[("unit", u), ("reason", why)],
        ));
    }
    let mut removed = Value::Null;
    let mut kept = Value::Null;
    for (u, x) in &cleaned {
        let line = match x {
            cleanup::Removal::Removed(path) => {
                let at = path.to_string_lossy();
                if u == unit {
                    removed = json!(at);
                }
                t("cleanup.line.removed", &[("unit", u), ("path", &at)])
            }
            cleanup::Removal::Kept(why) => {
                if u == unit {
                    kept = json!(why);
                }
                t("cleanup.line.kept", &[("unit", u), ("reason", why)])
            }
        };
        lines.push(line);
    }
    Ok(json!({
        "removed": removed, "kept": kept,
        "unit": unit, "landed": true, "checkpoint": k, "branch": cpb, "merge": tip,
        "content_hash": me.content, "merged_hash": merged_hash,
        "landed_units": landed.iter().map(|l| l.0.unit.clone()).collect::<Vec<_>>(),
        "left": left.iter().map(|(u, w)| json!({"unit": u, "reason": w})).collect::<Vec<_>>(),
        "sent_back": sent_back.iter().map(|(u, w)| json!({"unit": u, "reason": w})).collect::<Vec<_>>(),
        "line": lines.join("\n"),
    }))
}

/// The rules a landing applies to one unit. For the unit asked for a refusal acts (R-95 sends it back to
/// review); for another unit it only says why the unit waits.
fn ready(
    p: &Project,
    st: &State,
    pl: &Plan,
    receipts: &[Receipt],
    unit: &str,
    requested: bool,
) -> R<Ready> {
    let u = pl.unit(unit).cloned().unwrap_or_default();
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
    if let Some(why) = crate::wavecheck::land_blocker(pl, st, unit) {
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
    let repo = p.repo_of_unit(&u)?.clone();
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
    if pl.unit_checks(unit).is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            "the unit has no check, so nothing could prove it",
            &["Add a check record with `unit = \"<id>\"` through the plan."],
            "`bf plan check` passes.",
        ));
    }
    let content = p.content_hash(&wt)?;
    // R-95: the unit lands only on the content its last clean review judged, or the engine's clean merge of it.
    if let Some(r) = st.units.get(unit).and_then(|u| u.review_rounds.last()) {
        let judged = s(r, "content").to_string();
        if s(r, "result") == "built"
            && !judged.is_empty()
            && judged != content
            && !clean_engine_merge(p, unit, &wt, s(r, "head"))
        {
            let short = |h: &str| h[..12.min(h.len())].to_string();
            let round = r["round"].as_u64().unwrap_or(0);
            let why = format!(
                "the content now in the working copy (hash {}) is not the content its last clean review judged (round {round}, hash {})",
                short(&content),
                short(&judged)
            );
            if !requested {
                return Err(refuse(
                    format!("land {unit}"),
                    why,
                    &[&t("land_merge.left.review_step", &[("unit", unit)])],
                    t("land_merge.left.review_worked", &[]),
                ));
            }
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
                why,
                &[
                    &format!("{unit} is back in review. `bf next` gives the reviewer's directive."),
                    "The reviewer reviews what changed since that round and reports with `bf report`.",
                    &format!("Run `bf unit land {unit}` again after a clean report."),
                ],
                "`bf status` shows the unit reviewed, and landing goes on.",
            ));
        }
    }
    let missing: Vec<String> = evidence::unit_landing_needs(p, pl, st, receipts, unit, &content)
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
            &["commit", "-q", "-m", &format!("{unit}: {}", s(&u, "title"))],
        )?;
    }
    let cpb = p.checkpoint_branch(s(&u, "checkpoint"));
    crate::scope::landing_check(st, unit, &repo.path, &format!("{cpb}...{ub}"))?;
    Ok(Ready {
        unit: unit.to_string(),
        title: s(&u, "title").to_string(),
        content,
        wt,
        repo,
        ub,
    })
}

/// The unit's head is reached from the head its review judged by merges of the checkpoint branch that
/// the engine made without a conflict (`unit.sync`), and nothing else.
pub fn clean_engine_merge(p: &Project, unit: &str, wt: &Path, judged_head: &str) -> bool {
    if judged_head.is_empty() || gitx::has_uncommitted(wt) {
        return false;
    }
    let Ok(log) = events::load_log(p) else {
        return false;
    };
    let made: Vec<String> = log
        .events
        .iter()
        .filter(|e| {
            e["kind"] == "unit.sync"
                && s(e, "unit") == unit
                && s(e, "how") == "merge"
                && str_list(e, "conflicts").is_empty()
        })
        .map(|e| s(e, "commit").to_string())
        .collect();
    let Ok(chain) = gitx::must(
        wt,
        &[
            "rev-list",
            "--first-parent",
            &format!("{judged_head}..HEAD"),
        ],
    ) else {
        return false;
    };
    let chain: Vec<&str> = chain.lines().collect();
    !chain.is_empty()
        && chain.iter().all(|c| made.iter().any(|m| m == c))
        && gitx::is_ancestor(wt, judged_head, "HEAD")
}

/// Merge one unit into the checkpoint's working copy with the `Land <U>` commit. A merge that does not
/// finish leaves the working copy exactly as it was.
fn merge_one(cpwt: &Path, r: &Ready) -> R<Merge> {
    let before = gitx::must(cpwt, &["rev-parse", "HEAD"])?;
    if gitx::is_ancestor(cpwt, &r.ub, "HEAD") {
        return Ok(Merge::Done {
            tip: gitx::head_short(cpwt),
            before,
        });
    }
    let out = gitx::run(
        cpwt,
        &[
            "merge",
            "--no-ff",
            "--no-edit",
            "-m",
            &format!("Land {}: {}", r.unit, r.title),
            &r.ub,
        ],
        &[],
    )?;
    if out.ok {
        return Ok(Merge::Done {
            tip: gitx::head_short(cpwt),
            before,
        });
    }
    let _ = gitx::run(cpwt, &["merge", "--abort"], &[]);
    let _ = gitx::run(cpwt, &["reset", "--hard", &before], &[]);
    let conflict = out.stdout.contains("CONFLICT")
        || out.stdout.contains("Automatic merge failed")
        || out.stderr.contains("CONFLICT");
    if conflict {
        Ok(Merge::Conflict)
    } else {
        Ok(Merge::Failed(
            out.stderr.lines().next().unwrap_or("").to_string(),
        ))
    }
}

/// The files that a merge of the unit's branch into the checkpoint branch would leave in conflict now,
/// or none when it merges cleanly. Nothing is changed.
fn conflict_files(repo: &Path, cpb: &str, ub: &str) -> Option<Vec<String>> {
    let o = gitx::run(
        repo,
        &["merge-tree", "--write-tree", "--name-only", cpb, ub],
        &[],
    )
    .ok()?;
    if o.ok {
        return None;
    }
    let mut lines = o.stdout.lines();
    let first = lines.next()?.trim();
    if first.len() < 40 || !first.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let files: Vec<String> = lines
        .take_while(|l| !l.trim().is_empty())
        .map(String::from)
        .collect();
    (!files.is_empty()).then_some(files)
}

/// The run's checks of the turn: the list that applies to a unit of it, and the unit the runs are written
/// for. A unit that names its tests has them in the suite's place, so a unit that names none is taken.
struct Suite {
    exec: String,
    list: Vec<Attached>,
}

fn suite_for(p: &Project, pl: &Plan, st: &State, units: &[&Ready]) -> Suite {
    // The runs are recorded under a unit of the turn, never another. The landing runs the full suite of the
    // list, not the tests a unit names in its place (R-62 is for a unit's own check).
    let exec = units.first().map(|r| r.unit.clone()).unwrap_or_default();
    let mut list = crate::checks::attached_for_unit(p, pl, st, &exec).0;
    // Each suite entry runs its own listed command.
    for a in list.iter_mut().filter(|a| a.kind == "suite") {
        if let Some(c) = evidence::listed_command(p, pl, st, &exec, &a.id()) {
            a.command = c;
        }
    }
    Suite { exec, list }
}

fn failing_names(r: &Value) -> Vec<String> {
    let mut v: Vec<String> = r["failing_tests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x["name"].as_str().map(String::from))
        .collect();
    if v.is_empty() {
        v = evidence::failing_tests(s(r, "output_tail"))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
    }
    v
}

/// A passing receipt of the same run (`evidence::find_run`) that may be used again at landing: its sandbox
/// names no compile cache, because landing decides what enters the checkpoint (D-104, TH-20).
pub fn reusable_run(receipts: &[Receipt], keys: &[evidence::RunKey]) -> bool {
    keys.iter()
        .filter_map(|k| evidence::find_run(receipts, k))
        .any(|r| r.passes() && !s(&r.v, "sandbox").contains("compile-cache"))
}

/// `run_checks` with the runner told that this is a landing run.
fn run_landing(
    p: &Project,
    suite: &Suite,
    top: &Path,
    units: &[&Ready],
    retry: bool,
) -> R<(String, Verdict)> {
    evidence::landing_run(true);
    let r = run_checks(p, suite, top, units, retry);
    evidence::landing_run(false);
    r
}

/// Each check of the suite on the content now in `top`, through the evidence runner. A passing receipt of the
/// same command on the same content is used again, unless its sandbox names a compile cache; the turn's own
/// run never uses the cache. With `retry`, a failing run is run once more on the same content before the
/// unit is blamed, and both receipts are kept.
fn run_checks(
    p: &Project,
    suite: &Suite,
    top: &Path,
    units: &[&Ready],
    retry: bool,
) -> R<(String, Verdict)> {
    let content = p.content_hash(top)?;
    let reason = t("land_merge.rerun.reason", &[]);
    for a in &suite.list {
        let (id, argv) = (a.id(), a.command.clone());
        let receipts = evidence::load_receipts(p)?;
        let mut keys = vec![evidence::run_key(p, &argv, &content, top)];
        keys.extend(
            units
                .iter()
                .map(|r| evidence::run_key(p, &argv, &content, &r.wt)),
        );
        if reusable_run(&receipts, &keys) {
            continue;
        }
        let mut tries = if retry { 2 } else { 1 };
        loop {
            let attempt = evidence::next_attempt(p, &id, &argv, top)?;
            let repeat = (attempt > 1).then_some((attempt, reason.as_str()));
            let r = evidence::run_with(p, &id, Some(&suite.exec), &argv, top, repeat)?;
            if r["exit"].as_i64() == Some(0) {
                break;
            }
            tries -= 1;
            if tries == 0 {
                return Ok((
                    content,
                    Verdict::Fail {
                        check: id,
                        tests: failing_names(&r),
                    },
                ));
            }
        }
    }
    Ok((content, Verdict::Pass))
}

/// Why a unit of the turn may no longer land: it is no longer reviewed, it has an open finding or it is paused.
fn changed_since(p: &Project, st: &State, unit: &str) -> Option<String> {
    if st.unit_state(unit) != "reviewed" {
        return Some(t("land_merge.recheck.state", &[]));
    }
    if st.units.get(unit).is_some_and(|x| x.paused) {
        return Some(t("land_merge.left.paused", &[]));
    }
    let open = p
        .list_records(schema::kind_by_name("finding"))
        .ok()?
        .iter()
        .filter(|f| s(f, "unit") == unit && is_open(f))
        .map(|f| s(f, "id").to_string())
        .collect::<Vec<_>>();
    (!open.is_empty()).then(|| t("land_merge.recheck.findings", &[("ids", &open.join(", "))]))
}

/// Give a turn up: every checkpoint branch is as it was and checked out again.
fn abandon(p: &Project, pl: &Plan, k: &str, cpb: &str, heads: &[(String, String)]) -> R<()> {
    put_back(p, pl, k, heads)?;
    for (name, _) in heads {
        if let Some(repo) = p.repos.iter().find(|r| &r.name == name) {
            let wt = checkpoint_wt(p, pl, k, repo)?;
            if wt.join(".git").exists() {
                gitx::must(&wt, &["checkout", "-q", "-f", cpb])?;
            }
        }
    }
    Ok(())
}

/// Put each repository's checkpoint branch back to the commit the mark names.
fn put_back(p: &Project, pl: &Plan, k: &str, heads: &[(String, String)]) -> R<()> {
    for (name, commit) in heads {
        let Some(repo) = p.repos.iter().find(|r| &r.name == name) else {
            continue;
        };
        let wt = checkpoint_wt(p, pl, k, repo)?;
        if !wt.join(".git").exists() {
            continue;
        }
        let _ = gitx::run(&wt, &["merge", "--abort"], &[]);
        gitx::must(&wt, &["reset", "--hard", commit])?;
    }
    Ok(())
}

/// A turn cut off (F-386): the next landing of the checkpoint looks for the mark. With none, nothing is
/// put back. With one, a branch is put back to the marked commit only when every commit above it is a
/// `Land <U>` merge of a unit the mark names; a unit whose landing the log already holds keeps its merge.
fn recover(p: &Project, st: &State, pl: &Plan, k: &str, unit: &str) -> R<()> {
    let path = lock::turn_mark(p, k);
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let mark = path.to_string_lossy().to_string();
    let cpb = p.checkpoint_branch(k);
    let Ok(m) = serde_json::from_str::<Value>(&text) else {
        return Err(refuse_owned(
            t("land_merge.mark.what", &[("unit", unit)]),
            t(
                "land_merge.mark.unreadable",
                &[("checkpoint", &cpb), ("mark", &mark)],
            ),
            vec![t(
                "land_merge.mark.step",
                &[("where", &p.work_dir().to_string_lossy()), ("mark", &mark)],
            )],
            t("land_merge.mark.worked", &[]),
        ));
    };
    let units = str_list(&m, "units");
    let mut resets: Vec<(PathBuf, String)> = Vec::new();
    for h in m["heads"].as_array().cloned().unwrap_or_default() {
        let Some(repo) = p.repos.iter().find(|r| r.name == s(&h, "repo")) else {
            continue;
        };
        let wt = checkpoint_wt(p, pl, k, repo)?;
        if !wt.join(".git").exists() {
            continue;
        }
        let base = s(&h, "commit").to_string();
        let log = gitx::must(
            &wt,
            &[
                "log",
                "--first-parent",
                "--format=%H%x09%P%x09%s",
                &format!("{base}..HEAD"),
            ],
        )?;
        let mut foreign: Vec<String> = Vec::new();
        let mut target = base.clone();
        let mut found = false;
        for row in log.lines() {
            let mut f = row.splitn(3, '\t');
            let (hash, parents, subject) = (
                f.next().unwrap_or(""),
                f.next().unwrap_or(""),
                f.next().unwrap_or(""),
            );
            let ours = parents.split_whitespace().count() >= 2
                && subject
                    .strip_prefix("Land ")
                    .and_then(|x| x.split(':').next())
                    .is_some_and(|u| units.iter().any(|y| y == u));
            if !ours {
                foreign.push(format!("{} {}", &hash[..7.min(hash.len())], subject));
            } else if !found {
                let u = subject
                    .strip_prefix("Land ")
                    .and_then(|x| x.split(':').next())
                    .unwrap_or("");
                if st.unit_state(u) == "landed" {
                    target = hash.to_string();
                    found = true;
                }
            }
        }
        if !gitx::is_ancestor(&wt, &base, "HEAD") {
            foreign.push(base[..7.min(base.len())].to_string());
        }
        if !foreign.is_empty() {
            return Err(refuse_owned(
                t("land_merge.mark.what", &[("unit", unit)]),
                t(
                    "land_merge.mark.because",
                    &[("checkpoint", &cpb), ("commits", &foreign.join("; "))],
                ),
                vec![t(
                    "land_merge.mark.step",
                    &[("where", &wt.to_string_lossy()), ("mark", &mark)],
                )],
                t("land_merge.mark.worked", &[]),
            ));
        }
        resets.push((wt, target));
    }
    for (wt, target) in resets {
        let _ = gitx::run(&wt, &["merge", "--abort"], &[]);
        // A turn cut off before it passed left the head detached and the branch where it was.
        if gitx::current_branch(&wt).is_none() {
            gitx::must(&wt, &["checkout", "-q", "-f", &cpb])?;
        }
        gitx::must(&wt, &["reset", "--hard", &target])?;
    }
    let _ = fs::remove_file(&path);
    Ok(())
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
    let protected = p.protected_paths();
    if files
        .iter()
        .any(|f| protected.iter().any(|g| crate::util::path_matches(g, f)))
    {
        return Err(refuse_owned(
            t("land_conflict.what", &[("unit", unit)]),
            t(
                "land_conflict.aborted.because",
                &[("checkpoint", cpb), ("files", &named)],
            ),
            vec![t("land_conflict.aborted.step", &[("unit", unit)])],
            t("land_conflict.worked_aborted", &[]),
        ));
    }
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
    let _ = unit_sync::take_in_for_land(p, &st, unit);
    p.write_guard()?;
    brief::write_brief(p, unit, "builder");
    let steps = vec![
        t("land_conflict.step.back", &[("unit", unit)]),
        t("land_conflict.step.land", &[("unit", unit)]),
    ];
    Err(refuse_owned(
        t("land_conflict.what", &[("unit", unit)]),
        t(
            "land_conflict.because",
            &[("checkpoint", cpb), ("files", &named)],
        ),
        steps,
        t("land_conflict.worked", &[]),
    ))
}
