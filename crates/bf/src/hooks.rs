//! The engine's answers to the harness's other hooks (R-74, D-96, U-72): what would wait for the human
//! and is no tool call. A permission prompt and a tool server's request are answered no, an early stop
//! is turned back, the ask menu's answer to the release question releases or stays, a notice that the
//! session waits is journalled, and a session's start is told the flag. Each acts only while the run's
//! flag is on. The harness's names and the form of each answer are adapter data.

use crate::absent;
use crate::adapter::{HookEvent, DATA};
use crate::events;
use crate::refusal::R;
use crate::store::Project;
use crate::strings::t;
use crate::util;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;

/// How many heads of the log a stop remembers.
const KEEP_HEADS: usize = 50;

fn marks_path(p: &Project) -> std::path::PathBuf {
    p.state_dir().join("hooks.json")
}

fn journal_path(p: &Project) -> std::path::PathBuf {
    p.state_dir().join("waits.journal")
}

#[derive(Default)]
struct Marks {
    reached: BTreeMap<String, String>,
    heads: Vec<String>,
}

fn read_marks(p: &Project) -> Marks {
    let v: Value = std::fs::read_to_string(marks_path(p))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    Marks {
        reached: v["reached"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        heads: v["heads"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Changes the marks under a lock of their own, so two hooks at once keep both changes.
fn update_marks(p: &Project, f: impl FnOnce(&mut Marks)) -> R<()> {
    let dir = p.state_dir();
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("hooks.lock"))?;
    let _ = lock.lock();
    let mut m = read_marks(p);
    f(&mut m);
    let v = json!({"reached": m.reached, "heads": m.heads});
    util::atomic_write(&marks_path(p), serde_json::to_string(&v)?.as_bytes())?;
    Ok(())
}

/// The events of the adapter that the harness has never called for this project.
pub fn unreached(p: &Project) -> Vec<&'static HookEvent> {
    let m = read_marks(p);
    DATA.hook_events
        .iter()
        .filter(|e| !m.reached.contains_key(e.event))
        .collect()
}

/// The events never reached, as the words of a message: the engine's name and the harness's.
pub fn unreached_list(p: &Project) -> String {
    unreached(p)
        .iter()
        .map(|e| format!("{} ({})", e.event, e.harness))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The last lines of the journal of notices that a session waits for input.
pub fn journal_tail(p: &Project, n: usize) -> Vec<String> {
    let text = std::fs::read_to_string(journal_path(p)).unwrap_or_default();
    let lines: Vec<String> = text.lines().map(String::from).collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

fn append_journal(p: &Project, line: &str) -> R<()> {
    std::fs::create_dir_all(p.state_dir())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_path(p))?;
    writeln!(f, "{}", line.replace('\n', " "))?;
    Ok(())
}

/// The harness's form of an answer, with the engine's text and the line for the human put in.
fn answer(ev: &HookEvent, text: &str, human: &str) -> Option<Value> {
    fn fill(v: &mut Value, text: &str, human: &str) {
        match v {
            Value::String(s) => {
                if s == "<text>" {
                    *s = text.to_string();
                } else if s == "<human>" {
                    *s = human.to_string();
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| fill(x, text, human)),
            Value::Object(m) => m.values_mut().for_each(|x| fill(x, text, human)),
            _ => {}
        }
    }
    if ev.answer.is_empty() {
        return None;
    }
    let mut v: Value = serde_json::from_str(ev.answer).ok()?;
    fill(&mut v, text, human);
    // A line for the human that is empty is left out.
    if human.is_empty() {
        if let Some(m) = v.as_object_mut() {
            m.remove("systemMessage");
        }
    }
    Some(v)
}

/// The head of the log: its length and last hash.
fn head_key(p: &Project) -> String {
    events::read_head(p)
        .map(|h| format!("{}:{}", h.count, h.last_hash))
        .unwrap_or_default()
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim()
}

/// What `bf hook <event>` prints for the payload, or `None` for nothing. The call is marked as reached
/// first, whether the flag is on or not.
pub fn run(p: &Project, event: &str, payload: &Value) -> R<Option<Value>> {
    let Some(ev) = DATA.hook_events.iter().find(|e| e.event == event) else {
        return Ok(None);
    };
    let at = util::now();
    update_marks(p, |m| {
        m.reached.insert(ev.event.to_string(), at);
    })?;
    let st = events::replay(p)?;
    if !absent::is_on(&st) {
        return Ok(None);
    }
    let session = str_of(payload, "session_id");
    match event {
        "permission" => Ok(answer(ev, &t("hooks.permission.denied", &[]), "")),
        "elicitation" => Ok(answer(ev, "", "")),
        "stop" => stop(p, ev, &st, payload, session),
        "ask-answer" => ask_answer(p, ev, &st, payload, session),
        "idle" => {
            let msg = str_of(payload, "message");
            let msg = if msg.is_empty() {
                t("hooks.journal.no_message", &[])
            } else {
                msg.to_string()
            };
            append_journal(
                p,
                &t(
                    "hooks.journal.line",
                    &[
                        ("at", &util::now()),
                        ("session", session),
                        ("message", &msg),
                    ],
                ),
            )?;
            Ok(None)
        }
        "start" => {
            let line = t(
                "absent_briefs.next.line",
                &[
                    ("since", &st.absent.since),
                    ("session", &st.absent.session),
                    ("instruction", &absent::instruction().replace('\n', " ")),
                ],
            );
            Ok(answer(
                ev,
                &t("hooks.start.context", &[("line", &line)]),
                "",
            ))
        }
        _ => Ok(None),
    }
}

/// The end of the conductor's turn: turned back once for each head of the log while `bf next` has a
/// step of any role but the human's (findings F-231, F-236).
fn stop(
    p: &Project,
    ev: &HookEvent,
    st: &events::State,
    payload: &Value,
    session: &str,
) -> R<Option<Value>> {
    if !p.policy.absent.stop_guard
        || absent::is_subagent(payload)
        || st.absent_nested.contains(session)
        || absent::returning(st, session)
    {
        return Ok(None);
    }
    let before = head_key(p);
    if read_marks(p).heads.contains(&before) {
        return Ok(None);
    }
    let next = crate::run::next(p)?;
    let steps: Vec<String> = next["directives"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["role"] != "human")
        .map(|d| {
            let target = d["unit"]
                .as_str()
                .or(d["checkpoint"].as_str())
                .or(d["gate"].as_str())
                .or(d["gate_kind"].as_str())
                .unwrap_or("");
            t(
                "hooks.stop.step",
                &[
                    ("role", d["role"].as_str().unwrap_or("")),
                    ("action", d["action"].as_str().unwrap_or("")),
                    ("target", target),
                ],
            )
            .trim()
            .to_string()
        })
        .collect();
    if steps.is_empty() {
        return Ok(None);
    }
    // `bf next` may write to the log itself: both heads count as seen.
    let after = head_key(p);
    update_marks(p, |m| {
        for h in [before, after] {
            if !m.heads.contains(&h) {
                m.heads.push(h);
            }
        }
        let over = m.heads.len().saturating_sub(KEEP_HEADS);
        m.heads.drain(..over);
    })?;
    Ok(answer(
        ev,
        &t("hooks.stop.reason", &[("steps", &steps.join("; "))]),
        "",
    ))
}

/// The answers the ask menu's reply carries for a question, lowercased.
fn replies(response: &Value, question: &str) -> Vec<String> {
    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.trim().to_lowercase()),
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(m) => m.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }
    let parsed;
    let response = match response {
        Value::String(s) => {
            parsed = serde_json::from_str::<Value>(s).unwrap_or(Value::Null);
            &parsed
        }
        other => other,
    };
    let mut out = Vec::new();
    if let Some(a) = response.get("answers").and_then(|a| a.get(question)) {
        strings(a, &mut out);
    }
    out
}

/// The ask menu's answer to the release question (R-72 acceptance 11).
fn ask_answer(
    p: &Project,
    ev: &HookEvent,
    st: &events::State,
    payload: &Value,
    session: &str,
) -> Result<Option<Value>, crate::refusal::Fail> {
    let tool = str_of(payload, "tool_name");
    if absent::is_subagent(payload)
        || session.is_empty()
        || st.absent_nested.contains(session)
        || !absent::returning(st, session)
        || !DATA.question_tools.iter().any(|q| q.tool == tool)
    {
        return Ok(None);
    }
    let qs = payload["tool_input"]["questions"].as_array();
    let Some([q]) = qs.map(|v| v.as_slice()) else {
        return Ok(None);
    };
    let question = q["question"].as_str().unwrap_or("").trim().to_string();
    if question != absent::release_question() {
        return Ok(None);
    }
    let got = replies(
        payload.get("tool_response").unwrap_or(&Value::Null),
        &question,
    );
    let yes = t("absent.answer_yes", &[]).to_lowercase();
    let stay = t("absent.answer_stay", &[]).to_lowercase();
    if got.len() != 1 {
        return Ok(None);
    }
    if got[0] == yes {
        events::append(
            p,
            "absent.release",
            json!({"session": session, "by": "ask"}),
        )?;
        absent::clear_briefs(p)?;
        p.write_guard()?;
        if absent::is_on(&events::replay(p)?) {
            return Ok(None);
        }
        Ok(answer(
            ev,
            &t("absent.release.conductor", &[]),
            &t("absent.release.human", &[]),
        ))
    } else if got[0] == stay {
        events::append(p, "absent.stay", json!({"session": session, "by": "ask"}))?;
        p.write_guard()?;
        Ok(answer(
            ev,
            &t("absent.stay.conductor", &[]),
            &t("absent.stay.human", &[]),
        ))
    } else {
        Ok(None)
    }
}
