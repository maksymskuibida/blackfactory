//! The human-absent flag (R-72, D-93, D-94): a set of events in the run's log that only the hook
//! writes, set and released only in a conductor session of the run, read from the run's state.

use crate::adapter::DATA;
use crate::events::{self, State};
use crate::refusal::R;
use crate::store::Project;
use crate::strings::{raw, t};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// A stay or a release: when, in which session, and how (`message` or `ask`).
#[derive(Clone, Debug, Default)]
pub struct Act {
    pub at: String,
    pub session: String,
    pub by: String,
}

/// The run's flag, as the replay of the log leaves it.
#[derive(Clone, Debug, Default)]
pub struct Flag {
    pub on: bool,
    pub since: String,
    pub since_seq: u64,
    /// The session the flag was set in.
    pub session: String,
    /// Session -> the log position of the first human turn that marked the flag returning there.
    pub returning: BTreeMap<String, u64>,
    pub last_stay: Option<Act>,
    pub last_release: Option<Act>,
}

/// Whether the run's flag is on.
pub fn is_on(st: &State) -> bool {
    st.absent.on
}

/// Whether the flag is returning in this session.
pub fn returning(st: &State, session: &str) -> bool {
    st.absent.on && st.absent.returning.contains_key(session)
}

fn is_command(text: &str) -> bool {
    let t = text.trim();
    DATA.absent_commands.contains(&t)
}

fn holds_command(text: &str) -> bool {
    DATA.absent_commands.iter().any(|c| text.contains(c))
}

/// Whether the whole text is the release word, in any case, with one final `.` or `!`.
fn is_release_word(text: &str) -> bool {
    let word = raw("absent.release_word").unwrap_or("");
    let t = text.trim();
    let t = t.strip_suffix(['.', '!']).unwrap_or(t).trim_end();
    !word.is_empty() && t.to_lowercase() == word.to_lowercase()
}

fn nested_form(prompt: &str) -> bool {
    let p = prompt.trim_start();
    DATA.nested_forms.iter().any(|f| p.starts_with(f))
}

/// The replay of a recorded human turn: while the flag is on, the first turn of a conductor session
/// that is no command form marks the flag returning there.
pub fn replay_turn(st: &mut State, seq: u64) {
    if !st.absent.on {
        return;
    }
    let Some(turn) = st.turns.last() else { return };
    if turn.used || turn.session.is_empty() || st.absent_nested.contains(&turn.session) {
        return;
    }
    if is_command(&turn.text) {
        return;
    }
    let session = turn.session.clone();
    st.absent.returning.entry(session).or_insert(seq);
}

/// The index of a human turn of the session that nothing used and that came before `seq`.
fn turn_of(
    st: &State,
    ix: &HashMap<u64, usize>,
    tseq: u64,
    session: &str,
    seq: u64,
) -> Option<usize> {
    let i = *ix.get(&tseq)?;
    let t = &st.turns[i];
    (tseq < seq && !t.used && !session.is_empty() && t.session == session).then_some(i)
}

/// The replay of the events of the flag.
pub fn replay(st: &mut State, ix: &HashMap<u64, usize>, ev: &Value, seq: u64, at: &str) {
    let sv = |k: &str| ev.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let session = sv("session");
    let tseq = ev["turn"].as_u64().unwrap_or(0);
    match ev["kind"].as_str().unwrap_or("") {
        "absent.session" => {
            // The first mark of a session stands; a later one never changes it.
            if !session.is_empty()
                && st.absent_known.insert(session.clone())
                && ev["nested"] == true
            {
                st.absent_nested.insert(session);
            }
        }
        "absent.set" => {
            let ok = !st.absent.on
                && !st.absent_nested.contains(&session)
                && turn_of(st, ix, tseq, &session, seq)
                    .filter(|&i| is_command(&st.turns[i].text))
                    .is_some();
            if !ok {
                st.ignored
                    .push(t("absent.ignored.set", &[("seq", &seq.to_string())]));
                return;
            }
            st.turns[ix[&tseq]].used = true;
            st.absent.on = true;
            st.absent.since = at.to_string();
            st.absent.since_seq = seq;
            st.absent.session = session;
            st.absent.returning.clear();
        }
        "absent.stay" => {
            let by = sv("by");
            let ok = st.absent.on
                && match by.as_str() {
                    "message" => {
                        !st.absent_nested.contains(&session)
                            && turn_of(st, ix, tseq, &session, seq)
                                .filter(|&i| is_command(&st.turns[i].text))
                                .is_some()
                    }
                    "ask" => st.absent.returning.contains_key(&session),
                    _ => false,
                };
            if !ok {
                st.ignored
                    .push(t("absent.ignored.stay", &[("seq", &seq.to_string())]));
                return;
            }
            if by == "message" {
                st.turns[ix[&tseq]].used = true;
            }
            st.absent.returning.clear();
            st.absent.last_stay = Some(Act {
                at: at.to_string(),
                session,
                by,
            });
        }
        "absent.release" => {
            let by = sv("by");
            let since = st.absent.returning.get(&session).copied();
            let ok = st.absent.on
                && since.is_some()
                && match by.as_str() {
                    "message" => turn_of(st, ix, tseq, &session, seq)
                        .filter(|&i| {
                            is_release_word(&st.turns[i].text) && since.is_some_and(|s| s < tseq)
                        })
                        .is_some(),
                    "ask" => true,
                    _ => false,
                };
            if !ok {
                st.ignored
                    .push(t("absent.ignored.release", &[("seq", &seq.to_string())]));
                return;
            }
            if by == "message" {
                st.turns[ix[&tseq]].used = true;
            }
            st.absent.on = false;
            st.absent.returning.clear();
            st.absent.last_release = Some(Act {
                at: at.to_string(),
                session,
                by,
            });
        }
        _ => {}
    }
}

/// What the hook says: to the human on screen, and to the conductor in its context.
#[derive(Default, Debug)]
pub struct Say {
    pub human: Vec<String>,
    pub conductor: Vec<String>,
}

impl Say {
    pub fn is_empty(&self) -> bool {
        self.human.is_empty() && self.conductor.is_empty()
    }
}

pub(crate) fn is_subagent(payload: &Value) -> bool {
    DATA.subagent_fields.iter().any(|k| {
        payload
            .get(*k)
            .map(|v| !v.is_null() && v != &json!(false) && v != &json!(""))
            .unwrap_or(false)
    })
}

pub(crate) fn release_question() -> String {
    t(
        "absent.release_question",
        &[
            ("yes", &t("absent.answer_yes", &[])),
            ("stay", &t("absent.answer_stay", &[])),
        ],
    )
}

fn asked(say: &mut Say) {
    say.human.push(release_question());
    say.conductor.push(t(
        "absent.ask.conductor",
        &[
            ("yes", &t("absent.answer_yes", &[])),
            ("stay", &t("absent.answer_stay", &[])),
            ("question", &release_question()),
        ],
    ));
}

fn set_say(p: &Project, st: &State, payload: &Value, say: &mut Say) {
    say.human.push(t(
        "absent.set.human",
        &[("yes", &t("absent.answer_yes", &[]))],
    ));
    if let Some(mode) = payload.get("permission_mode").and_then(|v| v.as_str()) {
        if DATA.ask_modes.contains(&mode) {
            say.human.push(t("absent.set.mode", &[("mode", mode)]));
        }
    }
    let open: Vec<String> = st
        .amends
        .iter()
        .filter(|a| matches!(a.status.as_str(), "open" | "narrowed"))
        .map(|a| a.id.clone())
        .collect();
    if !open.is_empty() {
        say.human
            .push(t("absent.set.amendments", &[("list", &open.join(", "))]));
    }
    let hand = crate::plan::load(p).ok().and_then(|pl| {
        pl.checkpoints
            .iter()
            .map(|k| crate::store::s(k, "id").to_string())
            .find(|k| !st.closed.contains_key(k))
    });
    match hand {
        Some(k) => say
            .human
            .push(t("absent.set.checkpoint", &[("checkpoint", &k)])),
        None => say.human.push(t("absent.set.no_checkpoint", &[])),
    }
    let unreached = crate::hooks::unreached_list(p);
    if !unreached.is_empty() {
        say.human
            .push(t("absent.set.unreached", &[("events", &unreached)]));
    }
    say.human.push(t("absent.set.waits", &[]));
    say.human.push(t("absent.set.limits", &[]));
    say.conductor.push(t("absent.set.conductor", &[]));
}

fn refuse_set(p: &Project, session: &str, reason: &str) -> R<()> {
    events::append(
        p,
        "absent.refused",
        json!({"session": session, "reason": reason}),
    )?;
    Ok(())
}

/// The turn's part of the flag, called by the hook after it recorded or rejected the payload.
pub fn on_turn(p: &Project, payload: &Value, recorded: bool, turn_ev: &Value) -> R<Say> {
    let mut say = Say::default();
    let session = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let prompt = payload
        .get("prompt")
        .or_else(|| payload.get("user_prompt"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut st = events::replay(p)?;
    let mut wrote = false;
    if !session.is_empty() && !st.absent_known.contains(&session) {
        events::append(
            p,
            "absent.session",
            json!({"session": session, "nested": nested_form(prompt)}),
        )?;
        wrote = true;
        st = events::replay(p)?;
    }
    let cmd = is_command(prompt);
    let nested = st.absent_nested.contains(&session);
    let enabled = p.policy.absent.enabled;
    let tseq = turn_ev["seq"].as_u64().unwrap_or(0);
    if !recorded || is_subagent(payload) {
        if holds_command(prompt) {
            let fallback = t("absent.reason.not_recorded", &[]);
            let why = turn_ev["reason"].as_str().unwrap_or(&fallback);
            refuse_set(p, &session, why)?;
            wrote = true;
        }
    } else if nested {
        if holds_command(prompt) {
            refuse_set(p, &session, &t("absent.reason.nested", &[]))?;
            wrote = true;
        }
        if cmd {
            say.human.push(t("absent.refused.nested", &[]));
        } else if is_on(&st) {
            say.human.push(t("absent.nested.human", &[]));
        }
    } else if cmd {
        if !enabled {
            refuse_set(p, &session, &t("absent.reason.disabled", &[]))?;
            wrote = true;
            say.human.push(t("absent.refused.disabled", &[]));
        } else if !is_on(&st) {
            events::append(p, "absent.set", json!({"session": session, "turn": tseq}))?;
            wrote = true;
            let st2 = events::replay(p)?;
            if is_on(&st2) {
                set_say(p, &st2, payload, &mut say);
            }
        } else {
            events::append(
                p,
                "absent.stay",
                json!({"session": session, "by": "message", "turn": tseq}),
            )?;
            wrote = true;
            say.human.push(t("absent.stay.human", &[]));
            say.conductor.push(t("absent.stay.conductor", &[]));
        }
    } else if holds_command(prompt) {
        refuse_set(p, &session, &t("absent.reason.longer", &[]))?;
        wrote = true;
    }
    if recorded && !nested && !cmd && is_on(&st) && !session.is_empty() {
        let before = st.absent.returning.get(&session).is_some_and(|s| *s < tseq);
        if before && is_release_word(prompt) {
            events::append(
                p,
                "absent.release",
                json!({"session": session, "by": "message", "turn": tseq}),
            )?;
            wrote = true;
            clear_briefs(p)?;
            say.human.push(t("absent.release.human", &[]));
            say.conductor.push(t("absent.release.conductor", &[]));
        } else {
            asked(&mut say);
        }
    }
    if wrote {
        p.write_guard()?;
    }
    Ok(say)
}

/// The engine's instruction for the briefs, as the marked block that goes on top of a brief.
pub fn brief_block() -> String {
    format!(
        "{}\n## {}\n\n{}\n{}\n\n",
        t("absent_briefs.begin", &[]),
        t("absent_briefs.title", &[]),
        t("absent_briefs.instruction", &[]),
        t("absent_briefs.end", &[])
    )
}

/// The instruction alone, for `bf next`.
pub fn instruction() -> String {
    t("absent_briefs.instruction", &[])
}

/// Puts the block at the top of each named brief that lacks it (R-74). Nothing else of a brief changes.
pub fn stamp_briefs(paths: &[std::path::PathBuf]) -> R<()> {
    let begin = t("absent_briefs.begin", &[]);
    let block = brief_block();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if text.starts_with(&begin) {
            continue;
        }
        crate::util::atomic_write(path, format!("{block}{text}").as_bytes())?;
    }
    Ok(())
}

/// Takes the block out of every brief of the run: the brief is as it was written.
pub fn clear_briefs(p: &Project) -> R<()> {
    let begin = t("absent_briefs.begin", &[]);
    let end = t("absent_briefs.end", &[]);
    let Ok(rd) = std::fs::read_dir(p.briefs_dir()) else {
        return Ok(());
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains(&begin) {
            continue;
        }
        // Every block goes, the one at the top and the ones a brief embeds from other briefs.
        let mut out = String::new();
        let mut rest = text.as_str();
        while let Some(s) = rest.find(&begin) {
            out.push_str(&rest[..s]);
            let after = &rest[s..];
            let Some(at) = after.find(&end) else {
                rest = after;
                break;
            };
            let tail = &after[at + end.len()..];
            rest = tail.strip_prefix("\n\n").unwrap_or(tail);
        }
        out.push_str(rest);
        crate::util::atomic_write(&path, out.as_bytes())?;
    }
    Ok(())
}

/// The flag as data, for `bf absent status`.
pub fn status_json(st: &State) -> Value {
    let act = |a: &Option<Act>| match a {
        Some(a) => json!({"at": a.at, "session": a.session, "by": a.by}),
        None => Value::Null,
    };
    json!({
        "on": st.absent.on,
        "since": if st.absent.on { json!(st.absent.since) } else { Value::Null },
        "session": if st.absent.on { json!(st.absent.session) } else { Value::Null },
        "returning": st.absent.returning.keys().collect::<Vec<_>>(),
        "last_stay": act(&st.absent.last_stay),
        "last_release": act(&st.absent.last_release),
    })
}

/// The flag as lines for the human.
pub fn status_text(st: &State) -> String {
    let a = &st.absent;
    let mut lines = Vec::new();
    if a.on {
        lines.push(t(
            "absent.status.on",
            &[("since", &a.since), ("session", &a.session)],
        ));
        if a.returning.is_empty() {
            lines.push(t("absent.status.not_returning", &[]));
        } else {
            let list = a.returning.keys().cloned().collect::<Vec<_>>().join(", ");
            lines.push(t("absent.status.returning", &[("sessions", &list)]));
        }
    } else {
        lines.push(t("absent.status.off", &[]));
    }
    if let Some(s) = &a.last_stay {
        lines.push(t(
            "absent.status.stay",
            &[("at", &s.at), ("session", &s.session)],
        ));
    }
    if let Some(r) = &a.last_release {
        lines.push(t(
            "absent.status.release",
            &[("at", &r.at), ("session", &r.session)],
        ));
    }
    lines.join("\n")
}
