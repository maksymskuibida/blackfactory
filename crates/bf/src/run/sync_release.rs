//! U-103 (D-117): the conductor puts the resolver of a blocked sync of the main branch back to work, at
//! most `LIMIT` times, and the sync is parked for the human only after that.
//!
//! A resolver's `blocked` report with a stated reason leaves the sync in the phase `blocked`; `bf next`
//! gives the conductor `judge-sync-block`, and `bf checkpoint sync <K> --unblock --judgment fixed|wait
//! --reason "<why>"` answers it. The fourth blocked report, a report with no reason and a blocked report
//! of the sync's reviewer park the sync as before. An old log's `checkpoint.sync.park` still parks.

use super::*;
use crate::refusal::refuse_owned;
use crate::strings::t;

/// How many times the conductor may put one sync's resolver back to work, counted since the human's own
/// latest act on the sync (its release).
pub const LIMIT: u32 = 3;

/// Park the sync for the human with the words given, or the default reason when there are none.
pub(crate) fn park(p: &Project, k: &str, note: &str) -> R<Value> {
    let note = note.trim();
    let reason = if note.is_empty() {
        t("sync_release.park.default", &[])
    } else {
        note.to_string()
    };
    events::append(
        p,
        "checkpoint.sync.park",
        json!({"checkpoint": k, "reason": reason}),
    )?;
    p.write_guard()?;
    Ok(json!({"checkpoint": k, "phase": "parked"}))
}

/// The resolver's blocked report: the conductor judges it when it has a reason and a release is left.
pub(crate) fn resolver_blocked(p: &Project, so: &events::SyncOpen, note: &str) -> R<Value> {
    let k = &so.checkpoint;
    let note = note.trim();
    if note.is_empty() || so.unblocks >= LIMIT {
        return park(p, k, note);
    }
    events::append(
        p,
        "checkpoint.sync.block",
        json!({"checkpoint": k, "reason": note}),
    )?;
    p.write_guard()?;
    Ok(json!({"checkpoint": k, "phase": "blocked"}))
}

/// Whether a landing or an approved amendment came after the seq.
fn wait_over(events: &[Value], seq: u64) -> bool {
    events.iter().any(|e| {
        matches!(s(e, "kind"), "unit.land" | "amend.approve")
            && e["seq"].as_u64().unwrap_or(0) > seq
    })
}

fn why_of(events: &[Value], seq: u64) -> String {
    events
        .iter()
        .find(|e| e["seq"].as_u64() == Some(seq))
        .map(|e| s(e, "why").to_string())
        .unwrap_or_default()
}

/// Whether a blocked sync waits for a landing that cannot come: the conductor judged `wait` and no unit is
/// left that could land. The human then decides, and may release it (F-589).
pub(crate) fn wait_stalled(p: &Project, st: &State, so: &events::SyncOpen) -> bool {
    if so.phase != "blocked" || so.wait_seq == 0 {
        return false;
    }
    let Ok(log) = events::load_log(p) else {
        return false;
    };
    !wait_over(&log.events, so.wait_seq) && !in_flight(st)
}

fn in_flight(st: &State) -> bool {
    st.units.values().any(|u| {
        matches!(
            u.state.as_str(),
            "running" | "review" | "fixing" | "reviewed"
        )
    })
}

/// The directive `bf next` gives for a sync in the phase `blocked`; `None` when the sync only waits (its
/// line is pushed to `waiting`).
pub(crate) fn directive(
    p: &Project,
    st: &State,
    so: &events::SyncOpen,
    waiting: &mut Vec<String>,
) -> Option<Value> {
    let k = &so.checkpoint;
    let left = LIMIT.saturating_sub(so.unblocks);
    if so.wait_seq > 0 {
        let log = events::load_log(p).ok()?;
        if !wait_over(&log.events, so.wait_seq) {
            let why = why_of(&log.events, so.wait_seq);
            let in_flight = in_flight(st);
            if in_flight {
                waiting.push(t(
                    "sync_release.next.wait.line",
                    &[("k", k), ("reason", &so.reason), ("why", &why)],
                ));
                return None;
            }
            // Nothing is left that could end the wait: the human decides.
            return Some(
                json!({"role": "human", "action": "decide-sync", "checkpoint": k,
                "note": t("sync_release.next.stalled.note", &[("k", k), ("reason", &so.reason), ("why", &why)]),
                "release_command": format!("bf checkpoint sync {k} --release --quote \"<the human's words>\""),
                "abort_command": format!("bf checkpoint sync {k} --abort")}),
            );
        }
    }
    let v = [("k", k.as_str())];
    Some(
        json!({"role": "conductor", "action": "judge-sync-block", "checkpoint": k,
        "reason": so.reason, "releases_left": left,
        "note": t("sync_release.next.note", &[("k", k), ("reason", &so.reason), ("left", &left.to_string())]),
        "commands": [t("sync_release.next.cmd.fixed", &v), t("sync_release.next.cmd.wait", &v)],
        "abort_command": format!("bf checkpoint sync {k} --abort")}),
    )
}

fn no(k: &str, because: String, step: String) -> crate::refusal::Fail {
    refuse_owned(
        t("sync_release.unblock.refused", &[("k", k)]),
        because,
        vec![step],
        t("sync_release.unblock.worked", &[("k", k)]),
    )
}

/// `bf checkpoint sync <K> --unblock --judgment fixed|wait --reason "<why>"`: the conductor's judgment of
/// a resolver's blocked report.
pub(crate) fn unblock(p: &Project, st: &State, k: &str, judgment: &str, reason: &str) -> R<Value> {
    let v = [("k", k)];
    let Some(so) = st.sync_open.clone().filter(|o| o.checkpoint == k) else {
        return Err(no(
            k,
            t("sync_release.unblock.none.because", &v),
            t("sync_release.unblock.none.step", &v),
        ));
    };
    if so.phase != "blocked" {
        return Err(no(
            k,
            t(
                "sync_release.unblock.phase.because",
                &[("k", k), ("phase", &so.phase)],
            ),
            t("sync_release.unblock.phase.step", &v),
        ));
    }
    let why = reason.trim();
    if why.is_empty() {
        return Err(no(
            k,
            t("sync_release.unblock.reason.because", &v),
            t("sync_release.unblock.reason.step", &v),
        ));
    }
    if !matches!(judgment, "fixed" | "wait") {
        return Err(no(
            k,
            t(
                "sync_release.unblock.judgment.because",
                &[("k", k), ("judgment", judgment)],
            ),
            t("sync_release.unblock.judgment.step", &v),
        ));
    }
    let release = so.unblocks + 1;
    let mut ev = json!({"checkpoint": k, "judgment": judgment, "why": why, "reason": so.reason,
        "block_seq": so.block_seq});
    if judgment == "fixed" {
        ev["release"] = json!(release);
    }
    events::append(p, "checkpoint.sync.unblock", ev)?;
    let line = if judgment == "fixed" {
        // The resolver's next turn has its own token.
        spend_token(p, &format!("{k}:builder"))?;
        t(
            "sync_release.out.fixed",
            &[
                ("k", k),
                ("n", &release.to_string()),
                ("limit", &LIMIT.to_string()),
            ],
        )
    } else {
        t("sync_release.out.wait", &v)
    };
    p.write_guard()?;
    Ok(
        json!({"checkpoint": k, "judgment": judgment, "why": why, "reason": so.reason,
        "phase": if judgment == "fixed" { "resolve" } else { "blocked" },
        "release": release, "line": line}),
    )
}
