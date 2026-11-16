//! The one-line progress report: a sign per state, the unit IDs in that state, and the review round.
//! The conductor shows this line as the whole progress report; it does not compose it.

use crate::refusal::{err, R};
use crate::store::{s, str_list, Project};
use crate::{events, evidence, plan, run, util};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

/// The sign and the words for a state. `None` for states that are not work in progress (planned, ready).
pub fn sign(state: &str) -> Option<(&'static str, &'static str)> {
    Some(match state {
        "running" => ("▶", "building"),
        "review" => ("◐", "in review"),
        "fixing" => ("↻", "fixing"),
        "reviewed" => ("●", "reviewed, ready to land"),
        "landed" => ("✔", "landed"),
        "blocked" => ("⚠", "blocked"),
        "parked" => ("⚠", "parked"),
        "verifying" => ("◎", "checkpoint being verified"),
        "fix-review" => ("◐", "checkpoint fix in review"),
        "fix-fixing" => ("↻", "checkpoint being fixed"),
        "closed" => ("■", "checkpoint closed"),
        "wave-running" => ("◌", "wave check running"),
        "wave-passed" => ("✓", "wave check passed"),
        "wave-failed" => ("✗", "wave check failed"),
        "wave-fix-fixing" => ("↻", "wave check being fixed"),
        "wave-fix-review" => ("◐", "wave check fix in review"),
        _ => return None,
    })
}

/// One item: an ID, its state and its review round (0 when it has none to show).
pub type Item = (String, String, u32);

/// Build the line. Items in the same state (and round) share one segment.
pub fn render(items: &[Item]) -> String {
    let order = [
        "running",
        "review",
        "fixing",
        "reviewed",
        "landed",
        "blocked",
        "fix-review",
        "fix-fixing",
        "verifying",
        "wave-running",
        "wave-failed",
        "wave-fix-fixing",
        "wave-fix-review",
        "wave-passed",
        "closed",
    ];
    // Blocked and parked units share one sign and so one segment.
    let mut groups: BTreeMap<usize, Vec<(&str, u32)>> = BTreeMap::new();
    let mut stuck: Vec<(&str, &str)> = Vec::new();
    for (id, state, round) in items {
        let key = if state == "parked" { "blocked" } else { state };
        if let Some(ix) = order.iter().position(|o| *o == key) {
            groups.entry(ix).or_default().push((id, *round));
            if key == "blocked" {
                stuck.push((id, state));
            }
        }
    }
    let mut parts = Vec::new();
    for (ix, units) in groups {
        let (sg, words) = sign(order[ix]).unwrap();
        if order[ix] == "blocked" {
            let all_same = stuck.iter().all(|(_, s)| *s == stuck[0].1);
            let t = if all_same {
                let ids: Vec<&str> = stuck.iter().map(|(i, _)| *i).collect();
                format!("{sg} {} {}", ids.join(", "), stuck[0].1)
            } else {
                let each: Vec<String> = stuck.iter().map(|(i, s)| format!("{i} {s}")).collect();
                format!("{sg} {}", each.join(", "))
            };
            parts.push(t);
            continue;
        }
        // One segment per state. When every unit is in the same round it is said once at the end;
        // otherwise each unit carries its own round.
        let same = units.iter().all(|(_, r)| *r == units[0].1);
        let ids: Vec<String> = units
            .iter()
            .map(|(id, r)| {
                if !same && *r > 0 {
                    format!("{id} (round {r})")
                } else {
                    id.to_string()
                }
            })
            .collect();
        let mut t = format!("{sg} {} {words}", ids.join(", "));
        if same && units[0].1 > 0 {
            t.push_str(&format!(", round {}", units[0].1));
        }
        parts.push(t);
    }
    if parts.is_empty() {
        "no unit has started".to_string()
    } else {
        parts.join(" · ")
    }
}

/// The items of the run now: units with their state, checkpoints being verified or closed.
pub fn items(p: &Project) -> R<Vec<Item>> {
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let mut out: Vec<Item> = Vec::new();
    for (id, state, _) in run::unit_states(p, &pl, &st, &receipts) {
        let rounds = st.units.get(&id).map(|u| u.rounds).unwrap_or(0);
        // The round under review is the one after the findings so far; a fixing unit is in the round
        // whose findings it fixes.
        let round = match state.as_str() {
            "review" => rounds + 1,
            "fixing" => rounds,
            _ => 0,
        };
        out.push((id, state, round));
    }
    for k in &pl.checkpoints {
        let kid = s(k, "id").to_string();
        let after = str_list(k, "after");
        // The latest wave check of a checkpoint that is not closed: running, passed or failed. A failed
        // one with its fix open is shown as the fix.
        let wave_fix_open = st
            .ck_fix
            .as_ref()
            .map(|f| f.checkpoint == kid && f.source == "wave")
            .unwrap_or(false);
        if !st.closed.contains_key(&kid) && !wave_fix_open {
            if let Some((w, r)) = st
                .wave_runs
                .iter()
                .filter(|((c, _), _)| *c == kid)
                .map(|((_, w), r)| (*w, r))
                .max_by_key(|(w, _)| *w)
            {
                let state = match r.state.as_str() {
                    "running" => "wave-running",
                    "passed" => "wave-passed",
                    "failed" => "wave-failed",
                    _ => "",
                };
                if !state.is_empty() {
                    out.push((format!("{kid} wave {w}"), state.into(), 0));
                }
            }
        }
        if st.closed.contains_key(&kid) {
            out.push((kid, "closed".into(), 0));
        } else if let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == kid) {
            // A checkpoint with a fix open: fixing (round of the findings it fixes), in review (the round
            // under review), or waiting for the human like a parked unit. A wave fix says so.
            let judged = f.rounds_log.len() as u32;
            let label = if f.source == "wave" {
                format!("{kid} wave {}", f.wave)
            } else {
                kid.clone()
            };
            let wave = f.source == "wave";
            match f.phase.as_str() {
                "review" if wave => out.push((label, "wave-fix-review".into(), judged + 1)),
                "fix" if wave => out.push((label, "wave-fix-fixing".into(), judged)),
                "review" => out.push((kid, "fix-review".into(), judged + 1)),
                "fix" => out.push((kid, "fix-fixing".into(), judged)),
                "gap" => out.push((kid, "blocked".into(), 0)),
                _ => out.push((kid, "parked".into(), 0)),
            }
        } else if !after.is_empty() && after.iter().all(|u| st.unit_state(u) == "landed") {
            out.push((kid, "verifying".into(), 0));
        }
    }
    Ok(out)
}

pub fn line(p: &Project) -> R<String> {
    // The harness hands its status line command a JSON object on stdin. Saving it must never spoil
    // the line, and a caller with no input (a terminal, an open pipe nobody writes to) must not hang.
    if let Some(text) = stdin_within(Duration::from_millis(300)) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            let _ = save(p, &v);
        }
    }
    Ok(render(&items(p)?))
}

// What the harness gives the status line command, and what it cannot do (checked against the Claude
// Code documentation of the status line and of hooks; this build had no network, so the names marked
// "not confirmed here" come from that documentation as last read and are to be checked on a live
// session by the human before the limits below are relied on):
//
// - Status line input (JSON on stdin, refreshed after each assistant message and when the model or
//   the permission mode changes, at most every 300 ms):
//     `session_id`                      the session this line belongs to;
//     `model.id`, `model.display_name`  the model the session runs on (for example "claude-opus-5-5"
//                                       and "Opus 5.5");
//     `context_window.used_percentage`  the share of the context window in use, 0 to 100, and null
//                                       before the first message of a session;
//     `effort.level`                    the effort the session runs on ("low", "medium", "high",
//                                       "xhigh", "max"); not confirmed here, and absent for a model
//                                       with no effort setting. A plain string `effort` is read too.
//   The engine saves these, with the time, per session in `.bf/session-state.json` of the project.
// - A hook learns the model and the effort only from that file: the payload of the
//   `UserPromptSubmit` hook carries `session_id`, `transcript_path`, `cwd`, `permission_mode`,
//   `hook_event_name` and `prompt`, and no model and no effort. The transcript names the model of
//   past replies but never the effort, and it is empty before the first reply. So the hook cannot
//   know the model of a session whose status line has not run yet (a new session before its first
//   reply), and it then refuses a required-model message and says what it could not read.
// - A refused message: the hook prints `{"decision":"block","reason":"..."}` on stdout and exits 0.
//   The harness does not send the prompt to the model and shows the reason to the human. Whether the
//   harness puts the text back into the input box depends on its version and is not confirmed here:
//   the engine therefore repeats the whole text of the message in the reason, so the human can paste
//   it. Exit code 2 would erase the prompt, and the hook never uses it.
// - Limits: the status line runs only in a session with a status line command in its settings; a
//   saved share is the share at the last refresh, so a session that has not replied lately is judged
//   on an old number (older than the policy's `max_age_seconds` it is ignored, and no note is made);
//   with no share given nothing is claimed; two status lines written at the same moment can lose one
//   save, which the next refresh repairs.

const STATE_FILE: &str = "session-state.json";
const KEEP_SECONDS: u64 = 2 * 24 * 3600;

/// What the status line saved about one session.
#[derive(Debug, Clone, Default)]
pub struct SessionState {
    pub share: Option<f64>,
    pub model_id: String,
    pub model_name: String,
    pub effort: String,
    pub saved_at: u64,
}

fn stdin_within(limit: Duration) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut t = String::new();
        let _ = std::io::stdin().read_to_string(&mut t);
        let _ = tx.send(t);
    });
    rx.recv_timeout(limit).ok().filter(|t| !t.trim().is_empty())
}

fn state_path(p: &Project) -> std::path::PathBuf {
    p.dot_bf().join(STATE_FILE)
}

fn read_all(p: &Project) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(state_path(p))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Save the share, the model and the effort of the session from a status line input.
pub fn save(p: &Project, input: &Value) -> R<()> {
    let sid = input
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if sid.is_empty() {
        return Ok(());
    }
    let share = input
        .pointer("/context_window/used_percentage")
        .and_then(|v| v.as_f64());
    let text = |ptr: &str| input.pointer(ptr).and_then(|v| v.as_str()).unwrap_or("");
    let effort = if text("/effort/level").is_empty() {
        text("/effort")
    } else {
        text("/effort/level")
    };
    let now = util::now_secs();
    let mut all = read_all(p);
    all.retain(|_, v| v["saved_at"].as_u64().unwrap_or(0) + KEEP_SECONDS > now);
    all.insert(
        sid.to_string(),
        json!({
            "share": share,
            "model_id": text("/model/id"),
            "model_name": text("/model/display_name"),
            "effort": effort,
            "saved_at": now,
        }),
    );
    util::atomic_write(
        &state_path(p),
        serde_json::to_string_pretty(&Value::Object(all))
            .unwrap_or_default()
            .as_bytes(),
    )
    .map_err(|e| err(format!("cannot save the session state: {e}")))
}

/// What was saved for a session, if anything.
pub fn session_state(p: &Project, session: &str) -> Option<SessionState> {
    let all = read_all(p);
    let v = all.get(session)?;
    let st = |k: &str| v[k].as_str().unwrap_or("").to_string();
    Some(SessionState {
        share: v["share"].as_f64(),
        model_id: st("model_id"),
        model_name: st("model_name"),
        effort: st("effort"),
        saved_at: v["saved_at"].as_u64().unwrap_or(0),
    })
}

/// The note for the conductor when the saved share of the session is above the policy's setting and
/// not old. With no saved share, or an old one, nothing: no note and no claim.
pub fn context_note(p: &Project, session: &str, now: u64) -> Option<String> {
    let st = session_state(p, session)?;
    let share = st.share?;
    let pol = &p.policy.context;
    if now.saturating_sub(st.saved_at) > pol.max_age_seconds || share <= pol.max_share {
        return None;
    }
    Some(format!(
        "Note from the engine: this session's context is {share:.0}% full, above the {:.0}% the policy allows. \
Finish the step in hand, then ask the human which they want: a new session, compact, or continue here. \
Do not compact yourself: if the human answers compact, the human runs it. On new session, start it as the model policy says.",
        pol.max_share
    ))
}
