//! `bf human-turn`: the hook that records a message of the human.

use super::{pretty, project, read_stdin, start_dir, Args};
use crate::gates;
use crate::refusal::{err, R};
use crate::statusline::{self, SessionState};
use crate::store::Project;
use crate::util;
use serde_json::{json, Value};
use std::path::PathBuf;

pub(crate) fn cmd_human_turn(args: &Args) -> R<()> {
    let payload: Value = serde_json::from_str(&read_stdin())
        .map_err(|e| err(format!("the hook payload is not JSON: {e}")))?;
    let dir = payload
        .get("cwd")
        .and_then(|c| c.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| start_dir(args));
    let p = Project::discover(&dir).or_else(|_| project(args))?;
    let session = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let prompt = payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    // A message that names a required model is refused, before anything is recorded, when the session
    // runs on another. The refusal exits 0: exit code 2 would erase the prompt.
    if let Some(req) = required_line(prompt) {
        let state = statusline::session_state(&p, session);
        if let Some(reason) = check_required(&req, state.as_ref(), prompt) {
            let v = json!({"decision": "block", "reason": reason, "recorded": false});
            println!("{}", pretty(&v));
            return Ok(());
        }
    }
    let (recorded, ev) = gates::human_turn(&p, &payload)?;
    if args.has("json") {
        println!("{}", pretty(&json!({"recorded": recorded, "event": ev})));
    }
    // The flag's part of the turn. A failure here never erases the prompt: the hook still exits 0.
    let say = crate::absent::on_turn(&p, &payload, recorded, &ev).unwrap_or_default();
    // Stdout of this hook is added to the conductor's context: the note is the only thing printed,
    // unless the flag has something to say, which goes as the harness's JSON form.
    let note = statusline::context_note(&p, session, util::now_secs());
    if say.is_empty() {
        if let Some(n) = note {
            println!("{n}");
        }
    } else {
        let mut context = say.conductor.clone();
        context.extend(note);
        let mut out = json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": context.join("\n\n"),
            }
        });
        if !say.human.is_empty() {
            out["systemMessage"] = json!(say.human.join("\n\n"));
        }
        println!("{}", pretty(&out));
    }
    Ok(())
}

/// What a first line `Required model: <model> [<effort>]` asks for.
#[derive(Debug, PartialEq)]
pub(crate) struct Required {
    model: String,
    effort: Option<String>,
}

const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub(crate) fn required_line(prompt: &str) -> Option<Required> {
    let first = prompt.trim_start().lines().next()?.trim();
    let head = first.get(..15)?;
    if !head.eq_ignore_ascii_case("required model:")
        && !head.eq_ignore_ascii_case("required model ")
    {
        return None;
    }
    let mut words: Vec<&str> = first[15..].split_whitespace().collect();
    let effort = match words.last() {
        Some(w) if EFFORTS.contains(&w.to_lowercase().as_str()) && words.len() > 1 => {
            Some(words.pop().unwrap().to_lowercase())
        }
        _ => None,
    };
    if words.is_empty() {
        return None;
    }
    Some(Required {
        model: words.join(" "),
        effort,
    })
}

fn tokens(t: &str) -> Vec<String> {
    t.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

/// Whether `have` (a display name or a model id) is the whole name `want`, ignoring case and
/// punctuation. An id may carry a leading `claude` and a trailing date of eight digits; nothing else
/// may be added, so "Opus 5" is not "Opus 5.5" and "Opus" is not "Opus 5.5".
fn names(want: &[String], have: &str) -> bool {
    let mut h = tokens(have);
    if want.is_empty() {
        return false;
    }
    if h == want {
        return true;
    }
    if h.first().map(|x| x == "claude").unwrap_or(false) {
        h.remove(0);
    }
    if h.last()
        .map(|x| x.len() == 8 && x.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false)
    {
        h.pop();
    }
    h == want
}

/// `None` when the session runs on what the line asks for; else the reason to refuse.
fn check_required(req: &Required, st: Option<&SessionState>, prompt: &str) -> Option<String> {
    let want = match &req.effort {
        Some(e) => format!("{} {}", req.model, e),
        None => req.model.clone(),
    };
    let back = format!("Your message, to send again after switching:\n\n{prompt}");
    let Some(st) = st else {
        return Some(format!(
            "Refused: this message requires {want}, and the engine could not read the model and the effort of this session (the status line has saved nothing for it yet). Switch to {want} with /model, wait for one reply so the status line runs, then send the message again.\n\n{back}"
        ));
    };
    let want_tokens = tokens(&req.model);
    let model_ok = (!st.model_id.is_empty() || !st.model_name.is_empty())
        && (names(&want_tokens, &st.model_name) || names(&want_tokens, &st.model_id));
    if st.model_id.is_empty() && st.model_name.is_empty() {
        return Some(format!(
            "Refused: this message requires {want}, and the engine could not read the model of this session from the status line. Switch to {want} and send the message again.\n\n{back}"
        ));
    }
    if let Some(e) = &req.effort {
        if st.effort.is_empty() {
            return Some(format!(
                "Refused: this message requires {want}, and the engine could not read the effort of this session from the status line. Switch to {want} with /model and send the message again.\n\n{back}"
            ));
        }
        if !st.effort.eq_ignore_ascii_case(e) || !model_ok {
            return Some(mismatch(&want, st, &back));
        }
        return None;
    }
    if model_ok {
        None
    } else {
        Some(mismatch(&want, st, &back))
    }
}

fn mismatch(want: &str, st: &SessionState, back: &str) -> String {
    let on = if st.model_name.is_empty() {
        st.model_id.clone()
    } else {
        st.model_name.clone()
    };
    let eff = if st.effort.is_empty() {
        String::new()
    } else {
        format!(" {}", st.effort)
    };
    format!(
        "Refused: this message requires {want}, and this session runs on {on}{eff}. Switch to {want} with /model, then send the message again.\n\n{back}"
    )
}
