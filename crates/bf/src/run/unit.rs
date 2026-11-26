//! A unit's working copy: start, release and parking a unit that keeps failing.

use super::*;

pub fn unit_start(p: &Project, unit: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("start {unit}"))?;
    let st = events::require_chain(p)?;
    need_run(&st)?;
    budget_check(p, &st)?;
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("start {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints a working copy path.",
        ));
    };
    if !st.gate_passed("plan", None) {
        return Err(refuse(
            format!("start {unit}"),
            "the plan gate has not been approved",
            &[
                "Open the gate with `bf gate open plan` and show the plan page.",
                "The human approves in chat; then run `bf gate approve plan`.",
            ],
            "`bf gate status` shows the plan gate approved.",
        ));
    }
    gates::refuse_if_drift(p, &st, &format!("start {unit}"))?;
    if st.unit_state(unit).is_empty() {
        if let Some((_, command, what)) = unanswered_questions(&st).first() {
            return Err(refuse(
                format!("start {unit}"),
                format!("the run has not answered the question {what}: no unit starts until both plan questions are answered"),
                &[&format!("Run `{command}`, ask the human, then `propose` and `set --quote \"<their words>\"`; `bf next` lists both questions."), "Then start the unit again."],
                "`bf status` shows both plan questions answered.",
            ));
        }
    }
    if let Some(so) = &st.sync_open {
        return Err(refuse(
            format!("start {unit}"),
            format!("a sync of the main branch into {} is open ({}): no unit starts until it is settled", so.checkpoint, so.phase),
            &["`bf next` gives the directive that settles it.", &format!("To go back to how it was: `bf checkpoint sync {} --abort`.", so.checkpoint)],
            "`bf status` shows no open sync.",
        ));
    }
    let cur = st.unit_state(unit).to_string();
    // Starting a unit that is already in flight only makes sure its working copy exists.
    if !cur.is_empty() && !matches!(cur.as_str(), "running" | "review" | "fixing" | "reviewed") {
        return Err(refuse(
            format!("start {unit}"),
            format!("it is already `{cur}`"),
            &["Use `bf next` to see what is waiting."],
            "`bf status` shows the unit in its state.",
        ));
    }
    if cur.is_empty() {
        if let Some(a) = amend_hold(&st, unit) {
            let vals = [("unit", unit), ("amend", a.as_str())];
            return Err(refuse_owned(
                crate::strings::t("unit.start.held.refused", &vals),
                crate::strings::t("unit.start.held.because", &vals),
                vec![crate::strings::t("unit.start.held.do_this", &vals)],
                crate::strings::t("unit.start.held.worked_when", &vals),
            ));
        }
        let receipts = evidence::load_receipts(p)?;
        if let Some(why) = plan::not_ready(p, &pl, &st, unit, &receipts) {
            return Err(refuse(
                format!("start {unit}"),
                why,
                &["Land the units it waits for first (`bf next` lists them)."],
                "`bf next` lists the unit as ready.",
            ));
        }
    }
    // A unit that has not started, on a disk with little free space: the clean-up runs first and the unit
    // starts all the same. The disk only warns (D-109).
    let mut disk_warning = None;
    if cur.is_empty() {
        let free = cleanup::disk::free(p);
        if cleanup::disk::is_low(p, free) {
            let swept = cleanup::disk::clean(p, false).ok();
            let (count, bytes) = swept
                .as_ref()
                .map(|c| (c.removed.len(), c.freed()))
                .unwrap_or((0, 0));
            disk_warning = Some(crate::strings::t(
                "clean.start.low",
                &[
                    ("limit", &p.policy.disk.low_free_gb.to_string()),
                    ("free", &format!("{:.1}", free as f64 / (1u64 << 30) as f64)),
                    ("count", &count.to_string()),
                    ("bytes", &bytes.to_string()),
                ],
            ));
        }
    }
    let k = s(u, "checkpoint").to_string();
    let repo = p.repo_of_unit(u)?.clone();
    let cpb = p.checkpoint_branch(&k);
    ensure_checkpoint_worktree(p, &k, &repo)?;
    let ub = p.unit_branch(unit);
    let path = p.work_dir().join(unit);
    let existed = gitx::branch_exists(&repo.path, &ub);
    let mut behind = 0;
    if existed {
        behind = cleanup::ensure_unit_copy(p, unit)?;
    } else {
        if !path.join(".git").exists() {
            gitx::must(
                &repo.path,
                &["worktree", "add", "-b", &ub, &path.to_string_lossy(), &cpb],
            )?;
        }
        crash_point("unit-start-after-git");
    }
    if cur.is_empty() {
        let base_commit = gitx::must(&repo.path, &["rev-parse", &ub])?;
        events::append(
            p,
            "unit.start",
            json!({"unit": unit, "checkpoint": k, "repo": repo.name, "branch": ub, "path": path.to_string_lossy(), "base": cpb, "base_commit": base_commit}),
        )?;
    }
    p.write_guard()?;
    let note = brief::write_brief(p, unit, "builder");
    let mut out = json!({"unit": unit, "repo": repo.name, "working_copy": path.to_string_lossy(), "branch": ub, "base": cpb, "resumed": !cur.is_empty()});
    if let Some(n) = note {
        out["brief_note"] = json!(n);
    }
    if behind > 0 {
        out["behind"] = json!(behind);
    }
    if let Some(w) = disk_warning {
        out["disk_warning"] = json!(w);
    }
    Ok(out)
}

/// `bf unit release <U> --quote ...`: the human puts a parked or blocked unit back to work. Its round and
/// attempt counts start again; the log keeps the whole history.
pub fn unit_release(p: &Project, unit: &str, quote: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let Some(u) = st.units.get(unit) else {
        return Err(refuse(
            format!("release {unit}"),
            "that unit has not been started",
            &["Use a unit ID from `bf status`."],
            "`bf status` shows the unit back at work.",
        ));
    };
    if !matches!(u.state.as_str(), "parked" | "blocked") {
        return Err(refuse(
            format!("release {unit}"),
            format!(
                "it is `{}`; only a parked or blocked unit is released",
                u.state
            ),
            &["`bf next` shows what each unit is waiting for."],
            "`bf status` shows the unit back at work.",
        ));
    }
    let to = match u.prev_state.as_str() {
        "review" if u.parked_reason.starts_with("more than") => "fixing",
        "review" => "review",
        "fixing" => "fixing",
        _ => "running",
    };
    let (seq, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    events::append(
        p,
        "unit.release",
        json!({"unit": unit, "turn": seq, "quote": quote.trim(), "from": u.state, "to": to, "reason": u.parked_reason}),
    )?;
    p.write_guard()?;
    // A blocked or parked unit with no commits of its own and a clean working copy is brought up to the
    // checkpoint branch (cause release); one with commits is left as it is.
    let mut out = json!({"unit": unit, "state": to, "released": true});
    let st = events::replay(p)?;
    let behind = unit_sync::auto_ff(p, &st, unit, "release");
    if behind > 0 {
        out["behind"] = json!(behind);
    }
    Ok(out)
}

/// A builder that keeps failing its checks is parked after `max_attempts` in a row.
pub fn park_if_looping(p: &Project, unit: &str) -> R<()> {
    let st = events::replay(p)?;
    let Some(u) = st.units.get(unit) else {
        return Ok(());
    };
    if matches!(u.state.as_str(), "running" | "fixing")
        && u.attempts >= p.policy.limits.max_attempts
    {
        events::append(
            p,
            "unit.park",
            json!({"unit": unit, "reason": format!("{} failing check runs or blocked reports in a row", u.attempts)}),
        )?;
        p.write_guard()?;
    }
    Ok(())
}
