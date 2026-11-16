//! Human turns, gates, test mode and amendments: everything that depends on the human.

use crate::events::{self, Gate, State};
use crate::ledger;
use crate::plan;
use crate::refusal::{refuse, refuse_owned, R};
use crate::schema;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub const GATE_KINDS: &[&str] = &[
    "idea",
    "requirements",
    "ui",
    "architecture",
    "plan",
    "checkpoint-step",
];

/// Host-written elements that are a prompt only when they are the WHOLE prompt (a message of the human
/// that quotes one inside other words is still theirs): the tag and the names of its closing forms.
const HOST_ELEMENTS: &[(&str, &str)] = &[
    ("task-notification", "a background task's notification"),
    (
        "agent-message",
        "an agent's message (a subagent's hand-back)",
    ),
    ("cross-session-message", "another session's message"),
    ("side-sessions-event", "a side-session event"),
    ("ci-monitor-event", "a CI monitor event"),
];

const QUIT_SENTENCE: &str =
    "The app was quit while you were working. Please continue from where you left off.";

/// The prompt without the `<system-reminder>` blocks the host puts in front of it.
fn without_reminders(text: &str) -> (&str, bool) {
    let mut t = text.trim();
    let mut stripped = false;
    while let Some(rest) = t.strip_prefix("<system-reminder>") {
        match rest.find("</system-reminder>") {
            Some(end) => {
                t = rest[end + "</system-reminder>".len()..].trim();
                stripped = true;
            }
            None => break,
        }
    }
    (t, stripped)
}

/// Is the whole prompt one of the host's own forms? Names the form. Anchored at both ends for the
/// element forms, so a human message that contains or quotes a block is not matched.
pub fn host_form(text: &str) -> Option<String> {
    let (t, stripped) = without_reminders(text);
    if t.is_empty() {
        return if stripped {
            Some("only host reminders".into())
        } else {
            None
        };
    }
    if t == QUIT_SENTENCE {
        return Some("the host's note that the app was quit".into());
    }
    for lead in ["Background", "Task"] {
        let head = format!("{lead} (from the session that started this one):");
        if let Some(rest) = t.strip_prefix(&head) {
            if rest.starts_with('\n') || rest.starts_with("\r\n") {
                return Some("the brief a session is started with".into());
            }
        }
    }
    for (name, what) in HOST_ELEMENTS {
        // `<name ...>` with an optional `\` and `~` before the name, and a matching close tag.
        let Some(rest) = t.strip_prefix('<') else {
            continue;
        };
        let rest = rest.trim_start_matches(['\\', '~']);
        let Some(after) = rest.strip_prefix(name) else {
            continue;
        };
        if !(after.starts_with('>') || after.starts_with(char::is_whitespace)) {
            continue;
        }
        let closes = [
            format!("</{name}>"),
            format!("<~/{name}>"),
            format!("<\\~/{name}>"),
        ];
        // The first close tag must be the end of the prompt: one element, not an element and more.
        let first = closes
            .iter()
            .filter_map(|c| after.find(c.as_str()).map(|p| (p, c.len())))
            .min();
        if let Some((p, len)) = first {
            if after[p + len..].trim().is_empty() {
                return Some(format!("{what}, a <{name}> element"));
            }
        }
    }
    None
}

/// What the transcript row for this prompt says about who wrote it.
enum Origin {
    Host(String),
    Human,
}

/// The row's own marks, when the row can be found (same prompt id and exactly this text). Host:
/// `promptSource` = `system`; `turnOrigin` = `task_notification` or `peer`; `origin.kind` =
/// `task-notification`, `coordinator`, `peer` or `auto-continuation`. Human: `origin.kind` or
/// `turnOrigin` = `human`, or `promptSource` = `typed`. A row with neither mark decides nothing, and
/// neither does a missing row (the transcript may be written after the hook runs).
fn transcript_origin(payload: &Value, prompt: &str) -> Option<Origin> {
    let tp = payload.get("transcript_path").and_then(|v| v.as_str())?;
    let pid = payload
        .get("prompt_id")
        .or_else(|| payload.get("promptId"))
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty())?;
    let text = std::fs::read_to_string(tp).ok()?;
    for line in text.lines().rev() {
        if !line.contains(pid) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let id = v
            .get("promptId")
            .or_else(|| v.get("prompt_id"))
            .and_then(|x| x.as_str());
        if id != Some(pid) {
            continue;
        }
        let said = match &v["message"]["content"] {
            Value::String(x) => x.clone(),
            Value::Array(a) => a
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        if said.trim() != prompt.trim() {
            continue;
        }
        let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let kind = v["origin"]["kind"].as_str().unwrap_or("").to_string();
        let (source, turn) = (g("promptSource"), g("turnOrigin"));
        if kind == "human" || turn == "human" || source == "typed" {
            return Some(Origin::Human);
        }
        if source == "system"
            || matches!(turn.as_str(), "task_notification" | "peer")
            || matches!(
                kind.as_str(),
                "task-notification" | "coordinator" | "peer" | "auto-continuation"
            )
        {
            let mark = if source == "system" {
                "promptSource = system".to_string()
            } else if !turn.is_empty() && turn != "human" {
                format!("turnOrigin = {turn}")
            } else {
                format!("origin.kind = {kind}")
            };
            return Some(Origin::Host(mark));
        }
        return None;
    }
    None
}

/// Wrapper tags that mark text written by a program or another session, not typed by the human.
const MACHINE_TAGS: &[&str] = &[
    "system-reminder",
    "task-notification",
    "side-sessions-event",
    "ci-monitor-event",
    "local-command-stdout",
    "local-command-caveat",
    "user-prompt-submit-hook",
    "teammate-message",
    "cross-session-message",
    "agent-message",
    "channel",
];

/// Wrappers that mark a prompt as the host's when it STARTS with one (no closing form is checked).
const PREFIX_TAGS: &[&str] = &[
    "local-command-stdout",
    "local-command-caveat",
    "user-prompt-submit-hook",
    "teammate-message",
    "channel",
];

fn machine_tag_prefix(text: &str) -> Option<String> {
    let (t, _) = without_reminders(text);
    machine_tag(t).filter(|tag| PREFIX_TAGS.contains(&tag.as_str()))
}

/// Can this recorded turn never be the human's? An older build recorded some host messages (a
/// subagent's hand-back, another session's message) as human turns; replay treats them as used.
pub fn recorded_text_is_host(text: &str) -> bool {
    host_form(text).is_some()
        || machine_tag_prefix(text).is_some()
        || machine_tag(text)
            .map(|t| matches!(t.as_str(), "agent-message" | "cross-session-message"))
            .unwrap_or(false)
}

/// The wrapper tag a program or another session puts around its text, when the text starts with one
/// (after leading whitespace; a `\` or `~` between `<` and the name is allowed). The human's own pasted
/// text arrives as `<pasted_content` and is not one.
pub fn machine_tag(text: &str) -> Option<String> {
    let rest = text.trim_start().strip_prefix('<')?;
    let rest = rest.trim_start_matches(['\\', '~']);
    let tag: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if MACHINE_TAGS.contains(&tag.as_str()) {
        Some(tag)
    } else {
        None
    }
}

/// Decide whether a UserPromptSubmit payload is a message the human typed.
/// Returns Ok(text) or Err(reason).
pub fn classify_prompt(
    payload: &Value,
    project_root: &str,
    pol: &crate::store::HumanPol,
) -> Result<(String, &'static str), String> {
    let policy_sessions = &pol.sessions;
    match payload.get("hook_event_name").and_then(|v| v.as_str()) {
        Some("UserPromptSubmit") => {}
        Some(h) => {
            return Err(format!(
                "the payload is for the `{h}` hook, not a prompt the human submitted"
            ))
        }
        None => {
            return Err("the payload does not say which hook it is from (`hook_event_name`)".into())
        }
    }
    if payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|x| x.trim().is_empty())
        .unwrap_or(true)
    {
        return Err("the payload has no `session_id`".into());
    }
    for key in [
        "agent_id",
        "agent_type",
        "parent_tool_use_id",
        "subagent",
        "agent_name",
    ] {
        if payload
            .get(key)
            .map(|v| !v.is_null() && v != &json!(false) && v != &json!(""))
            .unwrap_or(false)
        {
            return Err(format!(
                "the payload carries `{key}`: it came from a subagent"
            ));
        }
    }
    if let Some(src) = payload.get("source").and_then(|v| v.as_str()) {
        if !matches!(src, "user" | "keyboard" | "typed" | "human") {
            return Err(format!("the payload source is `{src}`, not the human"));
        }
    }
    let text = payload
        .get("prompt")
        .or_else(|| payload.get("user_prompt"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "the payload has no `prompt` text".to_string())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("the prompt is empty".into());
    }
    // Who wrote this prompt? The transcript row for it, when it can be found, says so by its own marks;
    // without a decisive mark the words decide: the whole prompt must be one of the host's own forms.
    match transcript_origin(payload, text) {
        Some(Origin::Host(why)) => {
            return Err(format!(
                "the transcript marks this prompt as the host's, not the human's ({why})"
            ))
        }
        Some(Origin::Human) => {}
        None => {
            if let Some(form) = host_form(text) {
                return Err(format!(
                    "the prompt is {form}: the host's own message, not one the human typed"
                ));
            }
            if let Some(tag) = machine_tag_prefix(text) {
                return Err(format!("the text starts with a <{tag}> wrapper: it was written by a program or another session"));
            }
        }
    }
    if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
        if !project_root.is_empty() {
            if !Path::new(cwd).starts_with(project_root) {
                return Err("the session's working directory is outside this project".into());
            }
            if Path::new(cwd).starts_with(Path::new(project_root).join(".bf/work")) {
                return Err("the session works inside a unit working copy: it is a worker, not the human's session".into());
            }
        }
    }
    if !policy_sessions.is_empty() {
        let sid = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !policy_sessions.iter().any(|x| x == sid) {
            return Err("the session is not on the policy's list of the human's sessions".into());
        }
    }
    let strict = pol.transcript_match == "strict";
    let check = transcript_status(payload, &pol.transcript_root);
    if strict && check != "match" {
        return Err(format!(
            "strict mode: the transcript's last human message is not this prompt (check: {check}); a turn is recorded only when the harness's own transcript already ends with it"
        ));
    }
    Ok((text.to_string(), check))
}

fn expand_home(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(r) => format!("{}/{r}", std::env::var("HOME").unwrap_or_default()),
        None => p.to_string(),
    }
}

/// What the transcript says about this prompt, for information: `absent` when the payload names no
/// transcript, or the file does not exist or is not under the harness's transcript root; `unparsed`
/// when no human message could be read in it; `match` when the last human message is the prompt;
/// `differs` when it is another message (the harness may write the file only after the hook runs).
pub fn transcript_status(payload: &Value, root: &str) -> &'static str {
    let tp = payload
        .get("transcript_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if tp.is_empty() {
        return "absent";
    }
    let (Ok(canon_tp), Ok(canon_root)) = (
        std::fs::canonicalize(tp),
        std::fs::canonicalize(expand_home(root)),
    ) else {
        return "absent";
    };
    if !canon_tp.is_file() || !canon_tp.starts_with(&canon_root) {
        return "absent";
    }
    transcript_check(payload)
}

/// The human's lines in a transcript: type `user`, not a tool result, not a meta line.
pub fn transcript_says(path: &str, prompt: &str) -> &'static str {
    let Ok(text) = std::fs::read_to_string(path) else {
        return "unparsed";
    };
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["type"] != "user" || v["isMeta"] == true {
            continue;
        }
        let c = &v["message"]["content"];
        let said = match c {
            Value::String(x) => x.clone(),
            Value::Array(a) => {
                if a.iter().any(|b| b["type"] == "tool_result") {
                    continue;
                }
                a.iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => continue,
        };
        return if said.trim() == prompt.trim() {
            "match"
        } else {
            "differs"
        };
    }
    "unparsed"
}

pub fn transcript_check(payload: &Value) -> &'static str {
    match (
        payload.get("transcript_path").and_then(|v| v.as_str()),
        payload.get("prompt").and_then(|v| v.as_str()),
    ) {
        (Some(tp), Some(prompt)) => transcript_says(tp, prompt),
        _ => "unparsed",
    }
}

/// Record a human turn from the hook payload. Returns Ok(Some(event)) or Ok(None) when rejected.
pub fn human_turn(p: &Project, payload: &Value) -> R<(bool, Value)> {
    match classify_prompt(payload, &p.root.to_string_lossy(), &p.policy.human) {
        Ok((text, transcript)) => {
            let ev = events::append(
                p,
                "human.turn",
                json!({
                    "text": text,
                    "session": payload.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "cwd": payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(""),
                    "transcript": transcript,
                }),
            )?;
            p.write_guard()?;
            Ok((true, ev))
        }
        Err(reason) => {
            let text = payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
            let ev = events::append(
                p,
                "human.turn.rejected",
                json!({"reason": reason, "text": text}),
            )?;
            Ok((false, ev))
        }
    }
}

fn snapshot_kinds(kind: &str) -> &'static [&'static str] {
    match kind {
        "idea" => &["idea"],
        "requirements" => &["requirement"],
        "ui" => &["screen", "journey"],
        "architecture" => &["component", "decision", "target", "threat", "markets"],
        "plan" => &["unit", "handoff", "checkpoint", "check"],
        _ => &[],
    }
}

/// The records a gate page shows, as {id: {h, title}}.
pub fn snapshot(p: &Project, kind: &str, refr: Option<&str>) -> R<Value> {
    let mut m = serde_json::Map::new();
    let mut add = |rec: &Value| {
        let canon = serde_json::to_string(rec).unwrap_or_default();
        m.insert(
            s(rec, "id").to_string(),
            json!({"h": util::sha256_hex(canon.as_bytes()), "title": s(rec, "title")}),
        );
    };
    for k in snapshot_kinds(kind) {
        for rec in p.list_records(schema::kind_by_name(k))? {
            add(&rec);
        }
    }
    if kind == "checkpoint-step" {
        if let Some(k) = refr {
            let rec = p.require_record(k)?;
            add(&rec);
            for id in str_list(&rec, "proves") {
                if let Some(r) = p.get_record(&id)? {
                    add(&r);
                }
            }
        }
    }
    if kind == "idea" {
        if let Some((title, text)) = idea_brief(p) {
            m.insert(
                "idea.md".into(),
                json!({"h": util::sha256_hex(text.as_bytes()), "title": title}),
            );
        }
    }
    Ok(Value::Object(m))
}

/// The idea brief, `factory/idea.md`, when it exists: its first heading (or line) and its text.
pub fn idea_brief(p: &Project) -> Option<(String, String)> {
    let text = std::fs::read_to_string(p.factory().join("idea.md")).ok()?;
    let title = text
        .lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("The idea")
        .to_string();
    Some((title, text))
}

/// Which stage's gate holds a record of this kind still.
pub fn stage_of_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "idea" => Some("idea"),
        "requirement" => Some("requirements"),
        "screen" | "journey" => Some("ui"),
        "component" | "decision" | "target" | "threat" | "markets" => Some("architecture"),
        "unit" | "handoff" | "checkpoint" | "check" => Some("plan"),
        _ => None,
    }
}

/// An approved amendment covers this ID. So does the amendment that is open now, for the IDs of its
/// own set: its new or changed records and the agent's not affected decisions are put while it waits
/// for the human (R-39), and the one approval covers them. A change after the human read it needs
/// the human's reading again, because the amendment is revised or narrowed again.
pub fn amended(st: &State, id: &str) -> bool {
    st.amends.iter().any(|a| {
        (a.status == "approved" && a.keep.iter().any(|k| k == id))
            || (matches!(a.status.as_str(), "approved" | "open" | "narrowed")
                && (a.ids.iter().any(|k| k == id) || a.touched.iter().any(|k| k == id)))
    })
}

/// The amendment that holds this plan record as a draft: a unit or a check of the approved plan that an
/// amendment still waiting for the human names, and whose content is not the content the plan gate
/// approved. A draft is not the record in force: no evidence run, no scope and no landing rest on it
/// until the human approves the amendment (R-39).
pub fn unapproved_change(st: &State, rec: &Value) -> Option<String> {
    let kind = s(rec, "kind");
    if !matches!(kind, "unit" | "check") {
        return None;
    }
    let id = s(rec, "id");
    let a = st.amends.iter().find(|a| {
        matches!(a.status.as_str(), "open" | "narrowed")
            && (a.ids.iter().any(|k| k == id) || a.touched.iter().any(|k| k == id))
    })?;
    let g = st
        .latest_gate("plan", None)
        .filter(|g| g.status == "approved")?;
    let snap = g.snapshot.as_object()?;
    match snap.get(id) {
        Some(old) if old["h"].as_str() == Some(&crate::evidence::record_hash(rec)) => None,
        _ => Some(a.id.clone()),
    }
}

/// Records of a gate that was approved and that no longer match what the human approved: changed,
/// removed, or new since. An approved amendment covers its IDs. This is how a plan stays frozen, and
/// how a check record edited by hand after the plan gate is found.
pub fn drift(p: &Project, st: &State) -> R<Vec<String>> {
    let mut out = Vec::new();
    for stage in events::DESIGN_GATES {
        let Some(g) = st
            .latest_gate(stage, None)
            .filter(|g| g.status == "approved")
        else {
            continue;
        };
        let Some(snap) = g.snapshot.as_object() else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for k in snapshot_kinds(stage) {
            for rec in p.list_records(schema::kind_by_name(k))? {
                let id = s(&rec, "id").to_string();
                seen.insert(id.clone());
                if amended(st, &id) {
                    continue;
                }
                match snap.get(&id) {
                    None => out.push(format!(
                        "{id} is new since the {stage} gate ({}) was approved",
                        g.id
                    )),
                    Some(old) => {
                        if old["h"].as_str() != Some(&crate::evidence::record_hash(&rec)) {
                            out.push(format!(
                                "{id} was changed after the {stage} gate ({}) was approved",
                                g.id
                            ));
                        }
                    }
                }
            }
        }
        for id in snap.keys() {
            if !seen.contains(id) && !amended(st, id) && id != "idea.md" {
                out.push(format!(
                    "{id} was removed after the {stage} gate ({}) was approved",
                    g.id
                ));
            }
        }
    }
    Ok(out)
}

pub fn refuse_if_drift(p: &Project, st: &State, what: &str) -> R<()> {
    let d = drift(p, st)?;
    if d.is_empty() {
        return Ok(());
    }
    let mut steps: Vec<String> = d.iter().take(20).map(|x| format!("{x}.")).collect();
    steps.push("The plan and the design are frozen once their gate is approved. Open an amendment (`bf amend open <ID>...`), or ask the human to reopen the gate with `bf gate open <kind>`.".into());
    Err(crate::refusal::refuse_owned(
        what.to_string(),
        format!("{} record(s) differ from what the human approved", d.len()),
        steps,
        "`bf doctor` reports `frozen records: ok`.",
    ))
}

pub fn gate_id(kind: &str, refr: Option<&str>, round: u32) -> String {
    match refr {
        Some(r) => format!("{kind}-{r}-r{round}"),
        None => format!("{kind}-r{round}"),
    }
}

pub fn open(p: &Project, kind: &str, refr: Option<&str>) -> R<Gate> {
    if !GATE_KINDS.contains(&kind) {
        return Err(refuse(
            format!("open a `{kind}` gate"),
            "that is not a gate kind",
            &["Use one of: idea, requirements, ui, architecture, plan, checkpoint-step."],
            "`bf gate status` lists the new gate as open.",
        ));
    }
    if kind == "checkpoint-step" && refr.is_none() {
        return Err(refuse(
            "open a checkpoint-step gate",
            "it needs the checkpoint it belongs to",
            &["Run `bf gate open checkpoint-step --ref K-<number>`."],
            "`bf gate status` lists the gate with its checkpoint.",
        ));
    }
    if kind != "checkpoint-step" && refr.is_some() {
        return Err(refuse(
            format!("open a `{kind}` gate with --ref"),
            "only checkpoint-step gates take a reference",
            &["Run the command again without --ref."],
            "`bf gate status` lists the gate.",
        ));
    }
    if let Some(r) = refr {
        p.require_record(r)?;
    }
    let st = events::require_chain(p)?;
    if let Some(g) = st.latest_gate(kind, refr) {
        if g.status == "open" {
            return Err(refuse(
                format!("open a new `{kind}` gate"),
                format!("gate {} is still open", g.id),
                &[&format!("Decide it first: the human says approve or reject in chat, then `bf gate approve {}` or `bf gate reject {}`.", g.id, g.id)],
                "`bf gate status` shows no open gate of this kind.",
            ));
        }
    }
    if p.policy.gates.in_order && DESIGN_ORDER.contains(&kind) {
        check_order_and_content(p, &st, kind)?;
    }
    // A required check's design-stage product comes before its gate.
    crate::enforce::design_blocker(p, &st, kind)?;
    // A design gate with its review check on opens after the review of its records as they are now.
    let held = crate::checks::review::gate_blocker(p, &st, kind)?;
    if !held.is_empty() {
        return Err(refuse_owned(
            format!("open the `{kind}` gate"),
            format!("its review holds it: {}", held.join("; ")),
            vec![
                "Have the design reviewer judge the records: `bf review token {kind}`, then its findings and receipt.".replace("{kind}", kind),
                "Open the gate again once the review is clean.".to_string(),
            ],
            format!("`bf gate status` lists the {kind} gate as open."),
        ));
    }
    // Decision D-19: a stage's questions come before its gate.
    let open_q: Vec<String> = p
        .list_records(schema::kind_by_name("question"))?
        .iter()
        .filter(|q| s(q, "stage") == kind && q.get("answer").is_none())
        .map(|q| s(q, "id").to_string())
        .collect();
    if !open_q.is_empty() {
        return Err(refuse(
            format!("open the `{kind}` gate"),
            format!("these questions for the stage are unanswered: {}", open_q.join(", ")),
            &["Show the questions to the human and record each answer with `bf answer <Q> --file <answer.json>`.", "Open the gate again."],
            "`bf questions --open` lists none for this stage.",
        ));
    }
    if kind == "plan" {
        let pl = plan::load(p)?;
        plan::check_or_refuse(&pl)?;
    }
    let round = st
        .gates
        .iter()
        .filter(|g| g.kind == kind && g.refr.as_deref() == refr)
        .count() as u32
        + 1;
    let id = gate_id(kind, refr, round);
    let snap = snapshot(p, kind, refr)?;
    events::append(
        p,
        "gate.open",
        json!({"gate": id, "gate_kind": kind, "ref": refr, "round": round, "snapshot": snap}),
    )?;
    p.write_guard()?;
    let st = events::replay(p)?;
    Ok(st.gate(&id).cloned().unwrap())
}

/// Why `bf gate open <kind>` would refuse right now for want of records or an earlier gate, in words,
/// or `None` when it would open. `bf next` uses it so that no directive carries a command the engine
/// would refuse.
pub fn open_blocker(p: &Project, st: &State, kind: &str) -> Option<String> {
    let product = crate::enforce::design_blocker(p, st, kind).and_then(|_| {
        let held = crate::checks::review::gate_blocker(p, st, kind)?;
        if held.is_empty() {
            Ok(())
        } else {
            Err(refuse(
                format!("open the `{kind}` gate"),
                format!("its review holds it: {}", held.join("; ")),
                &["Have the design reviewer judge the records."],
                "`bf gate status` lists the gate as open.",
            ))
        }
    });
    match check_order_and_content(p, st, kind).and(product) {
        Ok(()) => None,
        Err(crate::refusal::Fail::Refused(r)) => {
            let mut why = r.because.clone();
            if kind == "plan" {
                if let Ok(pl) = plan::load(p) {
                    if !pl.problems.is_empty() {
                        why = format!(
                            "the plan has problems: {}",
                            pl.problems
                                .iter()
                                .take(5)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("; ")
                        );
                    }
                }
            }
            Some(why)
        }
        Err(crate::refusal::Fail::Error(e)) => Some(e),
    }
}

/// The first design gate, in order, that is neither approved nor skipped.
pub fn first_unpassed_design_gate(st: &State) -> &'static str {
    DESIGN_ORDER
        .iter()
        .copied()
        .find(|k| !(st.gate_passed(k, None) || (*k == "ui" && st.skipped_words("ui").is_some())))
        .unwrap_or("plan")
}

/// The design gates, in the order they are approved.
const DESIGN_ORDER: &[&str] = &["idea", "requirements", "ui", "architecture", "plan"];

/// A design gate opens when the earlier ones are approved (a rejected gate's next round is not held by
/// that), and when the stage has something to approve.
fn check_order_and_content(p: &Project, st: &State, kind: &str) -> R<()> {
    let i = DESIGN_ORDER.iter().position(|k| *k == kind).unwrap_or(0);
    let retry = st
        .latest_gate(kind, None)
        .map(|g| g.status == "rejected")
        .unwrap_or(false);
    if !retry {
        for e in &DESIGN_ORDER[..i] {
            if st.gate_passed(e, None) || (*e == "ui" && st.skipped_words("ui").is_some()) {
                continue;
            }
            return Err(refuse(
                format!("open the `{kind}` gate"),
                format!("the `{e}` gate comes first and is not approved"),
                &[
                    &format!("Open the `{e}` gate (`bf gate open {e}`), show the human its page and get their decision in chat."),
                    if *e == "ui" {
                        "A product with no interface: the human can skip the ui gate once, in their words: `bf gate skip ui --quote \"<their words>\"`."
                    } else {
                        "Gates are approved in this order: idea, requirements, ui, architecture, plan."
                    },
                ],
                format!("`bf gate status` shows `{e}` approved, then `bf gate open {kind}` works."),
            ));
        }
    }
    let count = |k: &str| -> R<usize> { Ok(p.list_records(schema::kind_by_name(k))?.len()) };
    let (empty, what, how): (bool, &str, &str) = match kind {
        "idea" => {
            let file = std::fs::read_to_string(p.root.join("factory/idea.md"))
                .map(|t| !t.trim().is_empty())
                .unwrap_or(false);
            (
                count("idea")? == 0 && !file,
                "an idea record or the idea file (factory/idea.md)",
                "Write the idea in `factory/idea.md` or put an idea record (`bf id new idea`).",
            )
        }
        "requirements" => (
            count("requirement")? == 0,
            "at least one requirement",
            "Put requirement records (`bf id new requirement`, then `bf record put requirement --file <f>`).",
        ),
        "ui" => (
            count("screen")? + count("journey")? == 0,
            "at least one screen or journey",
            "Put screen or journey records. A product with no interface skips this gate: `bf gate skip ui --quote \"<the human's words>\"`.",
        ),
        "architecture" => (
            count("component")? == 0 || count("decision")? == 0,
            "at least one component and one decision",
            "Put component and decision records (threat records are welcome too).",
        ),
        "plan" => {
            let pl = plan::load(p)?;
            plan::check_or_refuse(&pl)?;
            (
                pl.units.is_empty() || pl.checkpoints.is_empty(),
                "a plan with at least one unit and one checkpoint",
                "Put unit, check and checkpoint records, then `bf plan check`.",
            )
        }
        _ => (false, "", ""),
    };
    if empty {
        return Err(refuse(
            format!("open the `{kind}` gate"),
            format!("there is nothing to approve yet: the stage needs {what}"),
            &[how, "Then open the gate again."],
            format!("`bf gate open {kind}` prints a gate ID."),
        ));
    }
    Ok(())
}

/// `bf gate skip ui --quote "..."`: the human skips the ui gate once (a product with no interface). The
/// words are recorded; the skipped gate counts as passed for the order rule, and the index and the
/// architecture page say so.
pub fn skip(p: &Project, kind: &str, quote: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    if kind != "ui" {
        return Err(refuse(
            format!("skip the `{kind}` gate"),
            "only the ui gate can be skipped, for a product with no interface",
            &["Open the gate and have it decided like any other."],
            "`bf gate status` lists the gate.",
        ));
    }
    if st.skipped_words("ui").is_some() {
        return Err(refuse(
            "skip the ui gate",
            "it was skipped already",
            &["Nothing more to do: the architecture gate can open once the requirements are approved."],
            "`bf gate status` shows the earlier gates.",
        ));
    }
    if st.gate_passed("ui", None)
        || st
            .latest_gate("ui", None)
            .map(|g| g.status == "open")
            .unwrap_or(false)
    {
        return Err(refuse(
            "skip the ui gate",
            "the ui gate has been opened or approved; it is decided there",
            &["Decide the open gate, or leave the approved one as it is."],
            "`bf gate status` shows the ui gate.",
        ));
    }
    let (seq, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    events::append(
        p,
        "gate.skip",
        json!({"gate_kind": "ui", "turn": seq, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    Ok(json!({"skipped": "ui", "quote": quote.trim()}))
}

pub fn find_gate<'a>(st: &'a State, arg: &str) -> R<&'a Gate> {
    if let Some(g) = st.gate(arg) {
        return Ok(g);
    }
    // A bare kind means its latest round.
    if let Some(g) = st.gates.iter().rev().find(|g| g.kind == arg) {
        return Ok(g);
    }
    Err(refuse(
        format!("find gate `{arg}`"),
        "no gate has that ID or kind",
        &["Run `bf gate status` to list the gates."],
        "`bf gate status` shows the gate you named.",
    ))
}

pub fn approve(p: &Project, arg: &str, simulated: bool, quote: &str) -> R<Gate> {
    let st = events::require_chain(p)?;
    let g = find_gate(&st, arg)?.clone();
    if g.status != "open" {
        return Err(refuse(
            format!("approve gate {}", g.id),
            format!("it is already {}", g.status),
            &["Open a new round with `bf gate open` if the design changed."],
            "`bf gate status` shows the new round open.",
        ));
    }
    if simulated {
        let design = events::DESIGN_GATES.contains(&g.kind.as_str());
        if let Some(run) = st.active_run().filter(|r| r.test_mode) {
            let run = run.id.clone();
            events::append(
                p,
                "gate.approve",
                json!({"gate": g.id, "simulated": true, "run": run}),
            )?;
        } else if let (true, Some(grant)) = (design, st.pending_grants.last().copied()) {
            // A grant made before `bf run start` covers the design gates, so a test run can
            // go through them in order.
            events::append(
                p,
                "gate.approve",
                json!({"gate": g.id, "simulated": true, "grant": grant}),
            )?;
        } else {
            return Err(refuse(
                format!("approve gate {} as simulated", g.id),
                "there is no granted test run; a simulated approval outside one satisfies nothing",
                &[
                    "Ask the human whether this should be a test run. If they say yes in chat, run `bf test-mode grant`.",
                    "Then approve again: the grant covers the design gates before `bf run start`, and every gate inside the run.",
                    "For a real approval, the human says approve in chat and you run `bf gate approve <gate>` without --simulated.",
                ],
                "`bf status` shows `test mode: granted` or `pending`.",
            ));
        }
    } else {
        let (seq, _) = events::pick_turn(&st, g.open_seq, events::Decide::Approve, quote)?;
        events::append(
            p,
            "gate.approve",
            json!({"gate": g.id, "turn": seq, "simulated": false, "quote": quote.trim()}),
        )?;
    }
    // The programs the plan's checks start are recorded as they are when the plan is approved.
    if g.kind == "plan" {
        let _ = crate::programs::record_all(p);
    }
    p.write_guard()?;
    let st = events::replay(p)?;
    let ng = st.gate(&g.id).cloned().unwrap();
    if ng.status != "approved" {
        return Err(refuse(
            format!("approve gate {}", g.id),
            "the approval was written but does not count; check `bf doctor`",
            &["Run `bf doctor` and read the ignored events."],
            "`bf gate status` shows the gate approved.",
        ));
    }
    Ok(ng)
}

pub fn reject(p: &Project, arg: &str, note: &str, quote: &str) -> R<Gate> {
    let st = events::require_chain(p)?;
    let g = find_gate(&st, arg)?.clone();
    if g.status != "open" {
        return Err(refuse(
            format!("reject gate {}", g.id),
            format!("it is already {}", g.status),
            &["Open a new round with `bf gate open`."],
            "`bf gate status` shows the new round open.",
        ));
    }
    let (seq, _) = events::pick_turn(&st, g.open_seq, events::Decide::Reject, quote)?;
    events::append(
        p,
        "gate.reject",
        json!({"gate": g.id, "turn": seq, "note": note, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    let st = events::replay(p)?;
    Ok(st.gate(&g.id).cloned().unwrap())
}

pub fn gate_json(g: &Gate) -> Value {
    json!({
        "id": g.id, "kind": g.kind, "ref": g.refr, "round": g.round, "status": g.status,
        "opened_at": g.open_at, "simulated": g.simulated, "human_turn": g.turn, "note": g.note, "quote": g.quote, "human_message": g.message, "caution": g.caution, "transcript_check": g.transcript,
    })
}

/// `bf test-mode grant`
pub fn grant(p: &Project, quote: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    let (seq, _) = events::pick_turn(&st, 0, events::Decide::Grant, quote)?;
    if st.turns.last().map(|t| t.seq) != Some(seq) {
        return Err(refuse(
            "grant test mode",
            "a grant must come from the human's latest message",
            &[
                "Ask the human in chat: \"Should this be a test run, where approvals are simulated and nothing can land on main?\"",
                "When they say yes in their own words, run `bf test-mode grant --quote \"<their words>\"` straight away.",
            ],
            "`bf status` shows `test mode: granted` or `pending`.",
        ));
    }
    let run = st.active_run().map(|r| r.id.clone());
    let ev = events::append(
        p,
        "testmode.grant",
        json!({"turn": seq, "run": run, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    Ok(
        json!({"granted": true, "turn": seq, "run": run, "event": ev["seq"], "applies_to": run.clone().unwrap_or_else(|| "the next run".into())}),
    )
}

// ---- the designer's token (R-67) ----

/// The log event of a record the engine accepted from a designer (or, lifted, from the conductor).
pub const MADE_EVENT: &str = "design.made";
/// The log event of the human's words that lift the designer rule for one stage or one amendment.
pub const LIFT_EVENT: &str = "designer.lift";

/// The roles whose tokens put a record of this kind at the design stage: the designer for every kind,
/// and each role that writes the kind.
fn design_writers(kind: &str) -> Vec<&'static str> {
    let needle = format!("`bf record put {kind}`");
    crate::roles::ROLES
        .iter()
        .filter(|r| {
            r.at_design()
                && r.stage == "design"
                && (r.name == "designer" || r.writes.iter().any(|w| w.contains(&needle)))
        })
        .map(|r| r.name)
        .collect()
}

/// What the designer rule says about a record put now.
#[derive(Clone, Debug)]
pub struct Made {
    /// The stage (`requirements`...) or the amendment ID the record belongs to.
    pub subject: String,
    /// The role whose token was given; empty when the human lifted the rule.
    pub role: String,
    pub model: String,
    /// The human's words, when the rule was lifted.
    pub lifted: String,
}

/// The subject a design record belongs to: the open amendment that holds it when it is not a plan
/// repair, else the stage while its gate is not approved. `None` when the record is no design record, or belongs to a plan
/// repair or to an approved amendment, which need no designer.
fn designer_subject(st: &State, kind: &str, id: &str) -> Option<(String, bool)> {
    let stage = stage_of_kind(kind)?;
    let open = st.amends.iter().find(|a| {
        matches!(a.status.as_str(), "open" | "narrowed")
            && (a.ids.iter().any(|k| k == id) || a.touched.iter().any(|k| k == id))
    });
    if let Some(a) = open {
        if crate::amend::is_repair(a) {
            return None;
        }
        return Some((a.id.clone(), true));
    }
    if st.gate_passed(stage, None) {
        return None;
    }
    Some((stage.to_string(), false))
}

/// The words with which the human lifted the rule for `subject`, if they did.
fn lifted_words(p: &Project, subject: &str) -> R<Option<String>> {
    let log = events::load_log(p)?;
    Ok(log
        .events
        .iter()
        .rev()
        .find(|e| e["kind"] == LIFT_EVENT && e["subject"] == subject)
        .map(|e| e["quote"].as_str().unwrap_or("").to_string()))
}

/// Accept or refuse a design record put with `token` and the `model` the agent reports. `Ok(None)`
/// when the rule does not apply: role tokens are off, the record is no design record, or it belongs to
/// a plan repair. The caller writes the record and then calls [`design_made`] with the result.
pub fn designer_accept(
    p: &Project,
    st: &State,
    kind: &str,
    id: &str,
    token: Option<&str>,
    model: Option<&str>,
) -> R<Option<Made>> {
    if !p.policy.roles.tokens {
        return Ok(None);
    }
    let Some((subject, amendment)) = designer_subject(st, kind, id) else {
        return Ok(None);
    };
    let model = model.map(|m| m.trim()).unwrap_or("");
    let mut role = String::new();
    if amendment {
        if crate::run::holds_token(p, &format!("{subject}:designer"), token) {
            role = "designer".into();
        } else {
            for stage in crate::roles::design::STAGES {
                for r in crate::roles::ROLES
                    .iter()
                    .filter(|r| r.at_design() && !r.writes.is_empty())
                {
                    if crate::run::holds_token(p, &format!("{subject}:{}:{stage}", r.name), token)
                        && crate::roles::design::stage_may_put(p, &subject, stage, id)
                    {
                        role = r.name.to_string();
                    }
                }
            }
        }
    } else {
        for r in design_writers(kind) {
            if crate::run::holds_token(p, &format!("{}:{r}", crate::roles::DESIGN), token) {
                role = r.to_string();
                break;
            }
        }
    }
    if role.is_empty() {
        if let Some(words) = lifted_words(p, &subject)? {
            return Ok(Some(Made {
                subject,
                role,
                model: model.to_string(),
                lifted: words,
            }));
        }
        let what = if amendment {
            format!("the amendment {subject}")
        } else {
            format!("the {subject} stage")
        };
        return Err(refuse(
            format!("put the {kind} {id}"),
            format!(
                "a record of {what} is accepted only with the designer's token that the engine issued for it; none, or another one, was given. The conductor drafts no design record itself"
            ),
            &[
                &format!(
                    "Start a designer agent on the model the policy names (`bf policy` prints it) and issue its token: `bf roles token {} designer`.",
                    if amendment { subject.as_str() } else { crate::roles::DESIGN }
                ),
                "The designer puts the record with `--token <token> --model <the model it runs on>`.",
                &format!("Only the human lifts this, for {what} alone: `bf roles lift {subject} --quote \"<their words>\"`."),
            ],
            format!("`bf record get {id}` shows the record, made by the designer agent."),
        ));
    }
    if model.is_empty() {
        return Err(refuse(
            format!("put the {kind} {id}"),
            "the designer reports the model it ran on with every record, and none was given",
            &["Put the record again with `--model <the model you run on>`; it is your own report, and the engine keeps it with the record."],
            format!("`bf record get {id}` shows the record."),
        ));
    }
    Ok(Some(Made {
        subject,
        role,
        model: model.to_string(),
        lifted: String::new(),
    }))
}

/// Keep with the record who made it: the log event the pages read.
pub fn design_made(p: &Project, id: &str, kind: &str, m: &Made) -> R<()> {
    events::append(
        p,
        MADE_EVENT,
        json!({"record": id, "record_kind": kind, "subject": m.subject, "role": m.role,
            "model": m.model, "lifted": m.lifted}),
    )?;
    p.write_guard()?;
    Ok(())
}

/// Who made each record, by ID: the latest `design.made` event of each.
pub fn made_by(p: &Project) -> R<BTreeMap<String, Value>> {
    let log = events::load_log(p)?;
    let mut out = BTreeMap::new();
    for e in log.events.iter().filter(|e| e["kind"] == MADE_EVENT) {
        if let Some(id) = e["record"].as_str() {
            out.insert(id.to_string(), e.clone());
        }
    }
    Ok(out)
}

/// `bf roles lift <stage|A-n> --quote "..."`: the human lifts the designer rule for one stage or one
/// amendment. The words are recorded with the quote; a message lifts once.
pub fn designer_lift(p: &Project, subject: &str, quote: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    let what = format!("lift the designer rule for {subject}");
    let is_stage = DESIGN_ORDER.contains(&subject);
    let amend = st.amends.iter().find(|a| a.id == subject);
    if !is_stage && amend.is_none() {
        return Err(refuse(
            what,
            "the rule is lifted for one design stage (idea, requirements, ui, architecture or plan) or one amendment (A-n)",
            &["Name the stage or the amendment: `bf roles lift requirements --quote \"<their words>\"`."],
            "The command prints the words recorded.",
        ));
    }
    if let Some(a) = amend {
        if !matches!(a.status.as_str(), "open" | "narrowed") || crate::amend::is_repair(a) {
            return Err(refuse(
                what,
                format!("{subject} is {}, and only an open amendment that is not a plan repair has the rule", if crate::amend::is_repair(a) { "a plan repair".to_string() } else { a.status.clone() }),
                &["A plan repair needs no designer; nothing to lift."],
                "The command prints the words recorded.",
            ));
        }
    }
    if lifted_words(p, subject)?.is_some() {
        return Err(refuse(
            what,
            "it was lifted already",
            &["Nothing more to do: the records of it are accepted without the designer's token."],
            "`bf view run --json` shows the words.",
        ));
    }
    // Only words written after the work they lift: after the amendment was opened, or after the gate
    // before the stage was approved.
    let after = match amend {
        Some(a) => a.opened_seq,
        None => {
            let i = DESIGN_ORDER.iter().position(|k| *k == subject).unwrap_or(0);
            DESIGN_ORDER[..i]
                .iter()
                .filter_map(|k| st.latest_gate(k, None))
                .filter(|g| g.status == "approved")
                .filter_map(|g| g.decided_seq)
                .max()
                .unwrap_or(0)
        }
    };
    let mut st = st;
    for e in events::load_log(p)?
        .events
        .iter()
        .filter(|e| e["kind"] == LIFT_EVENT)
    {
        if let Some(t) = e["turn"].as_u64() {
            if let Some(x) = st.turns.iter_mut().find(|x| x.seq == t) {
                x.used = true;
            }
        }
    }
    let (seq, message) = events::pick_turn(&st, after, events::Decide::Approve, quote)?;
    events::append(
        p,
        LIFT_EVENT,
        json!({"subject": subject, "turn": seq, "message": message, "quote": quote.trim()}),
    )?;
    p.write_guard()?;
    Ok(json!({"lifted": subject, "quote": quote.trim()}))
}

// ---- amendments ----

/// The amendment that is open now (not approved, not withdrawn), if any.
fn open_amendment(st: &State) -> Option<&events::Amend> {
    st.amends
        .iter()
        .find(|a| matches!(a.status.as_str(), "open" | "narrowed"))
}

/// Check the IDs of an amendment and work out its impact set.
fn amend_impact(p: &Project, ids: &[String]) -> R<(Value, Vec<String>)> {
    for id in ids {
        // A new record can be amended in too (the plan is frozen after its gate); it just has no links yet.
        if schema::kind_of_id(id).is_none() {
            p.require_record(id)?;
        }
    }
    let pl = plan::load(p)?;
    let imp = ledger::impact(p, &pl, ids)?;
    let mut all: Vec<String> = ids.to_vec();
    for k in ["units", "handoffs", "checks", "checkpoints"] {
        all.extend(str_list(&imp, k));
    }
    all.sort();
    all.dedup();
    Ok((imp, all))
}

pub fn amend_open(p: &Project, ids: &[String]) -> R<Value> {
    amend_open_marked(p, ids, false)
}

/// Open an amendment; `repair` marks it as a plan repair, which the engine refuses when the set holds
/// a design part (R-41).
pub fn amend_open_marked(p: &Project, ids: &[String], repair: bool) -> R<Value> {
    let st = events::require_chain(p)?;
    if ids.is_empty() {
        return Err(refuse(
            "open an amendment",
            "no IDs were given",
            &["Run `bf amend open R-1 C-2` with the IDs that change."],
            "The command prints the impact set.",
        ));
    }
    if let Some(open) = open_amendment(&st) {
        let o = open.id.clone();
        return Err(refuse_owned(
            "open an amendment",
            format!("{o} is still open; only one amendment is open at a time"),
            vec![
                format!("Run `bf amend revise <ID>...` to change the records {o} covers."),
                format!("Or run `bf amend withdraw {o} --reason \"<why>\"` and open a new one."),
            ],
            "The status lists one open amendment.",
        ));
    }
    if repair {
        crate::amend::refuse_repair_mark("open an amendment as a plan repair", ids)?;
    }
    let (imp, all) = amend_impact(p, ids)?;
    let id = format!("A-{}", st.amends.len() + 1);
    events::append(
        p,
        "amend.open",
        json!({"amend": id, "ids": ids, "impact": imp, "impact_ids": all, "repair": repair}),
    )?;
    p.write_guard()?;
    Ok(json!({"amend": id, "ids": ids, "impact": imp, "impact_ids": all, "repair": repair}))
}

/// Inside the open amendment: insert a checkpoint before or after a named one, or split one. The
/// operation is kept in the amendment and takes effect only when the human approves it (R-64).
pub fn amend_checkpoint(p: &Project, pos: &[String], title: Option<&str>) -> R<Value> {
    let st = events::require_chain(p)?;
    let Some(open) = open_amendment(&st) else {
        return Err(refuse(
            "insert or split a checkpoint",
            "no amendment is open",
            &["Run `bf amend open <ID>...` (or `bf amend open --repair <ID>...` for a plan repair) first."],
            "`bf amend open` prints an amendment ID.",
        ));
    };
    let aid = open.id.clone();
    let op = crate::amend::op_from_args(pos, title)?;
    let mut ops = open.ops.clone();
    ops.push(op.clone());
    let edit = crate::amend::edit_of(p, &st, &ops)?;
    let touched = edit.touched();
    let problems = crate::amend::result_problems(p, &edit)?;
    events::append(
        p,
        "amend.checkpoint",
        json!({"amend": aid, "op": op, "touched": touched, "renamed": edit.renames}),
    )?;
    p.write_guard()?;
    Ok(json!({"amend": aid, "op": op,
        "renamed": edit.renames.iter().map(|(o, n)| json!({"from": o, "to": n})).collect::<Vec<_>>(),
        "created": edit.created, "touched": touched, "plan_problems": problems,
        "note": "Nothing is written until the human approves the amendment: narrow it again with `bf amend narrow --file <f>`."}))
}

/// Change the records the open amendment covers. It keeps its ID, goes back to open (the narrowing
/// is void) and needs a narrowing and a human message after this revision before it can be approved.
pub fn amend_revise(p: &Project, ids: &[String]) -> R<Value> {
    let st = events::require_chain(p)?;
    let Some(open) = open_amendment(&st) else {
        return Err(refuse(
            "revise an amendment",
            "no amendment is open",
            &["Run `bf amend open <ID>...` to open one."],
            "`bf amend open` prints an amendment ID.",
        ));
    };
    let aid = open.id.clone();
    if ids.is_empty() {
        return Err(refuse(
            "revise an amendment",
            "no IDs were given",
            &["Run `bf amend revise R-1 C-2` with the IDs the amendment should cover."],
            "The command prints the impact set.",
        ));
    }
    if open.repair {
        crate::amend::refuse_repair_mark("revise a plan repair", ids)?;
    }
    let (imp, all) = amend_impact(p, ids)?;
    events::append(
        p,
        "amend.revise",
        json!({"amend": aid, "ids": ids, "impact": imp, "impact_ids": all}),
    )?;
    p.write_guard()?;
    Ok(
        json!({"amend": aid, "revised": true, "ids": ids, "impact": imp, "impact_ids": all,
        "note": "Narrow it again with `bf amend narrow --file <f>`; the human approves after that."}),
    )
}

/// Withdraw an amendment the human has not approved. The log keeps that it existed and that it was
/// withdrawn; it can no longer be narrowed or approved.
pub fn amend_withdraw(p: &Project, arg: Option<&str>, reason: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    let Some(aid) = arg else {
        return Err(refuse(
            "withdraw an amendment",
            "no amendment ID was given",
            &["Run the withdraw command with the ID that the status lists as open."],
            "The status lists the amendment as withdrawn.",
        ));
    };
    let Some(a) = st.amends.iter().find(|a| a.id == aid) else {
        return Err(refuse(
            "withdraw an amendment",
            format!("{aid} does not exist"),
            &["Use an ID that the status lists under amendments."],
            "The status lists the amendment as withdrawn.",
        ));
    };
    if a.status == "approved" || a.status == "withdrawn" {
        return Err(refuse(
            format!("withdraw {aid}"),
            format!("it is already {}", a.status),
            &["Only an amendment that is not approved can be withdrawn; open a new one to change an approved one."],
            "The status lists the amendment as withdrawn.",
        ));
    }
    if reason.trim().is_empty() {
        return Err(refuse(
            format!("withdraw {aid}"),
            "no reason was given",
            &[&format!(
                "Run `bf amend withdraw {aid} --reason \"<why>\"`."
            )],
            "The status lists the amendment as withdrawn, with its reason.",
        ));
    }
    events::append(
        p,
        "amend.withdraw",
        json!({"amend": aid, "reason": reason.trim()}),
    )?;
    p.write_guard()?;
    Ok(json!({"amend": aid, "withdrawn": true, "reason": reason.trim()}))
}

/// The status of an amendment as the human sees it: open, approved or withdrawn.
pub fn amend_status(a: &events::Amend) -> &'static str {
    match a.status.as_str() {
        "approved" => "approved",
        "withdrawn" => "withdrawn",
        _ => "open",
    }
}

pub fn amendments_json(st: &State) -> Vec<Value> {
    st.amends
        .iter()
        .map(|a| {
            json!({"id": a.id, "ids": a.ids, "status": amend_status(a),
                "repair": crate::amend::is_repair(a),
                "withdrawn_why": a.withdrawn_why})
        })
        .collect()
}

pub fn amend_narrow(p: &Project, file: &Value) -> R<Value> {
    let st = events::require_chain(p)?;
    let aid = file
        .get("amend")
        .and_then(|v| v.as_str())
        .map(|x| x.to_string())
        .or_else(|| open_amendment(&st).map(|a| a.id.clone()));
    let Some(aid) = aid else {
        return Err(refuse(
            "narrow an amendment",
            "no amendment is open",
            &["Run `bf amend open <ID>...` first."],
            "`bf amend open` prints an amendment ID.",
        ));
    };
    let Some(a) = st.amends.iter().find(|a| a.id == aid) else {
        return Err(refuse(
            "narrow an amendment",
            format!("{aid} does not exist"),
            &["Use the ID that `bf amend open` printed."],
            "The command prints the narrowed set.",
        ));
    };
    if a.status == "withdrawn" {
        return Err(refuse(
            "narrow an amendment",
            format!("{aid} was withdrawn"),
            &["Open a new amendment over the same records."],
            "A new amendment ID is printed.",
        ));
    }
    if a.status == "approved" {
        return Err(refuse(
            "narrow an amendment",
            format!("{aid} is already approved"),
            &["Open a new amendment."],
            "`bf amend open` prints a new ID.",
        ));
    }
    let keep = str_list(file, "keep");
    let waive = str_list(file, "waive");
    let reasons = file.get("reasons").cloned().unwrap_or(json!({}));
    let mut problems = Vec::new();
    let full: Vec<String> = a.keep.clone();
    for k in &keep {
        if !full.contains(k) {
            problems.push(format!(
                "{k} is not in the impact set; narrowing can only remove IDs, never add them"
            ));
        }
    }
    for dropped in full.iter().filter(|x| !keep.contains(x)) {
        if reasons
            .get(dropped)
            .and_then(|v| v.as_str())
            .map(|r| r.trim().is_empty())
            .unwrap_or(true)
        {
            problems.push(format!(
                "{dropped} is dropped from the set without a reason"
            ));
        }
    }
    for w in &waive {
        if !keep.contains(w) && !a.ids.contains(w) {
            problems.push(format!("{w} is waived but not part of the amendment"));
        }
        if reasons
            .get(w)
            .and_then(|v| v.as_str())
            .map(|r| r.trim().is_empty())
            .unwrap_or(true)
        {
            problems.push(format!("{w} is waived without a reason"));
        }
    }
    if !problems.is_empty() {
        let mut steps: Vec<String> = problems.iter().map(|x| format!("Fix: {x}.")).collect();
        steps.push("Run `bf amend narrow --file <f>` again.".into());
        return Err(refuse_owned(
            "narrow the amendment",
            format!("{} problem(s) in the narrowing", problems.len()),
            steps,
            "The command prints the narrowed set.",
        ));
    }
    // The records of the set as the human reads them now: approval is refused if they change later.
    let set_hash = match st.amends.iter().find(|a| a.id == aid) {
        Some(a) => crate::amend::set_hash(p, a)?,
        None => String::new(),
    };
    events::append(
        p,
        "amend.narrow",
        json!({"amend": aid, "keep": keep, "waive": waive, "reasons": reasons, "set_hash": set_hash}),
    )?;
    p.write_guard()?;
    Ok(json!({"amend": aid, "keep": keep, "waive": waive}))
}

pub fn amend_approve(p: &Project, arg: Option<&str>, quote: &str) -> R<Value> {
    let log = events::load_log(p)?;
    let st = events::require_chain(p)?;
    let aid = arg.map(|x| x.to_string()).or_else(|| {
        st.amends
            .iter()
            .rev()
            .find(|a| a.status == "narrowed")
            .map(|a| a.id.clone())
    });
    let Some(aid) = aid else {
        return Err(refuse(
            "approve an amendment",
            "no narrowed amendment exists",
            &["Run `bf amend open` and `bf amend narrow --file <f>` first."],
            "`bf amend approve` prints the approved set.",
        ));
    };
    let a = st.amends.iter().find(|a| a.id == aid);
    if a.map(|a| a.status == "withdrawn").unwrap_or(false) {
        return Err(refuse(
            "approve an amendment",
            format!("{aid} was withdrawn"),
            &["A withdrawn amendment cannot be approved: open a new one over the same records."],
            "The approved set is printed.",
        ));
    }
    if a.map(|a| a.status != "narrowed").unwrap_or(true) {
        return Err(refuse(
            "approve an amendment",
            format!("{aid} is not narrowed yet"),
            &["Run `bf amend narrow --file <f>` first."],
            "`bf amend approve` prints the approved set.",
        ));
    }
    let last_change = log.events.iter().rev().find(|e| {
        matches!(e["kind"].as_str(), Some("amend.narrow" | "amend.revise"))
            && e["amend"] == aid.as_str()
    });
    let narrow_seq = last_change.and_then(|e| e["seq"].as_u64()).unwrap_or(0);
    let (seq, _) = events::pick_turn(&st, narrow_seq, events::Decide::Approve, quote)?;
    let a = a.unwrap();
    // Every stage covered and every requirement linked, before the approval is written (R-40).
    // A plan repair needs no stage sentences and no links, but it may not hold a design part (R-41).
    if a.repair {
        crate::amend::refuse_repair_mark(&format!("approve {aid} as a plan repair"), &a.ids)?;
    }
    if !crate::amend::is_repair(a) {
        crate::amend::refuse_if_uncovered(p, a)?;
    }
    // And no record of the set changed after the narrowing the human read (TH-12).
    if let Some(e) = last_change {
        crate::amend::refuse_if_changed_since_narrowing(p, a, e)?;
    }
    // The checkpoint operations: the plan they leave must pass the plan check before approval (R-64).
    let edit = if a.ops.is_empty() {
        None
    } else {
        let e = crate::amend::edit_of(p, &st, &a.ops)?;
        crate::amend::refuse_if_result_breaks_plan(p, a, &e)?;
        Some(e)
    };
    // What the amendment re-opens, worked out from the plan now (as the operations leave it) and
    // written into the event.
    let mut pl = plan::load(p)?;
    if let Some(e) = &edit {
        pl.units = e.units.clone();
        pl.checkpoints = e.cps.clone();
        pl.checks = e.checks.clone();
    }
    // What changed is the amendment's IDs and the kept impact set, less what the human waived.
    let changed: Vec<String> = a
        .keep
        .iter()
        .chain(a.ids.iter())
        .filter(|i| !a.waive.contains(i))
        .cloned()
        .collect();
    let reopen_units: Vec<String> = pl
        .units
        .iter()
        .filter(|u| {
            let uid = s(u, "id").to_string();
            let del = str_list(u, "delivers");
            a.keep.contains(&uid)
                && st.units.contains_key(&uid)
                && (del.is_empty() || del.iter().any(|d| !a.waive.contains(d)))
        })
        .map(|u| s(u, "id").to_string())
        .collect();
    let reopen_checkpoints: Vec<String> = pl
        .checkpoints
        .iter()
        .filter(|k| {
            str_list(k, "proves").iter().any(|i| changed.contains(i))
                || str_list(k, "after")
                    .iter()
                    .any(|u| reopen_units.contains(u))
        })
        .map(|k| s(k, "id").to_string())
        .collect();
    let mut reopen_checks: Vec<String> = pl
        .checks
        .iter()
        .filter(|c| {
            let cid = s(c, "id").to_string();
            (a.keep.contains(&cid) && str_list(c, "proves").iter().any(|i| changed.contains(i)))
                || reopen_units.contains(&s(c, "unit").to_string())
                || pl.checkpoints.iter().any(|k| {
                    reopen_checkpoints.contains(&s(k, "id").to_string())
                        && str_list(k, "checks").contains(&cid)
                })
        })
        .map(|c| s(c, "id").to_string())
        .collect();
    reopen_checks.sort();
    reopen_checks.dedup();
    let receipt_floor = crate::evidence::highest_receipt_number(p);
    // The records take their new numbers first, all or none; the approval is recorded last, so a
    // failure leaves the amendment narrowed and the plan as it was, and approving again works.
    let written = match &edit {
        Some(e) => crate::amend::write_edit(p, e)?,
        None => vec![],
    };
    let extra = crate::scope::unblock::approve_fields(p, &st, a, &reopen_units);
    let mut data = json!({"amend": aid, "turn": seq, "quote": quote.trim(),
               "reopen_units": reopen_units, "reopen_checkpoints": reopen_checkpoints,
               "reopen_checks": reopen_checks, "receipt_floor": receipt_floor,
               "renamed": edit.as_ref().map(|e| e.renames.clone()).unwrap_or_default()});
    let unblocked = extra.get("unblock_units").cloned().unwrap_or(json!([]));
    crate::scope::merge_fields(&mut data, extra);
    if let Err(err) = events::append(p, "amend.approve", data) {
        crate::amend::undo_writes(&written);
        return Err(err);
    }
    crate::scope::wait::resume_waiting(p)?;
    p.write_guard()?;
    Ok(json!({"amend": aid, "approved": true, "turn": seq, "unblocked": unblocked}))
}

/// IDs waived by approved amendments.
pub fn waived_ids(st: &State) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for a in st.amends.iter().filter(|a| a.status == "approved") {
        for w in &a.waive {
            m.insert(w.clone(), a.id.clone());
        }
    }
    m
}

// ---- the policy ----

/// The policy the human accepted is the policy in force. This is how a change on disk becomes one.
pub fn accept_policy(p: &Project, quote: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    let (seq, _) = events::pick_turn(&st, 0, events::Decide::Approve, quote)?;
    let disk = std::fs::read(p.root.join("factory/policy.toml"))?;
    if let Err(e) = toml::from_str::<crate::store::Policy>(&String::from_utf8_lossy(&disk)) {
        return Err(refuse(
            "accept this policy",
            format!("the file on disk is not a valid policy: {e}"),
            &["Fix factory/policy.toml, or put the accepted one back."],
            "`bf policy accept` prints the new hash.",
        ));
    }
    let old = std::fs::read(p.accepted_policy_path())
        .map(|b| util::sha256_hex(&b))
        .unwrap_or_default();
    let new = util::sha256_hex(&disk);
    events::append(
        p,
        "policy.accept",
        json!({"turn": seq, "quote": quote.trim(), "old": old, "new": new}),
    )?;
    p.store_accepted_policy()?;
    p.write_guard()?;
    Ok(json!({"accepted": true, "old": old, "new": new}))
}

/// The lines that differ between the accepted policy and the file on disk, for the human to read.
pub fn policy_diff(p: &Project) -> String {
    let acc = std::fs::read_to_string(p.accepted_policy_path()).unwrap_or_default();
    let disk = std::fs::read_to_string(p.root.join("factory/policy.toml")).unwrap_or_default();
    let (a, d): (
        std::collections::HashSet<&str>,
        std::collections::HashSet<&str>,
    ) = (acc.lines().collect(), disk.lines().collect());
    let mut out = String::new();
    for l in acc.lines().filter(|l| !d.contains(l)) {
        out.push_str(&format!("- {l}\n"));
    }
    for l in disk.lines().filter(|l| !a.contains(l)) {
        out.push_str(&format!("+ {l}\n"));
    }
    if out.is_empty() {
        out.push_str("no difference\n");
    }
    out
}
