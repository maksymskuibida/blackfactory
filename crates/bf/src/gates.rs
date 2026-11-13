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
    "channel",
];

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
    if let Some(rest) = trimmed.strip_prefix('<') {
        let tag: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if MACHINE_TAGS.contains(&tag.as_str()) {
            return Err(format!("the text starts with a <{tag}> wrapper: it was written by a program or another session"));
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
        "architecture" => &["component", "decision", "target", "threat"],
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
        "component" | "decision" | "target" | "threat" => Some("architecture"),
        "unit" | "handoff" | "checkpoint" | "check" => Some("plan"),
        _ => None,
    }
}

/// An approved amendment covers this ID.
pub fn amended(st: &State, id: &str) -> bool {
    st.amends.iter().any(|a| {
        a.status == "approved" && (a.keep.iter().any(|k| k == id) || a.ids.iter().any(|k| k == id))
    })
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

// ---- amendments ----

pub fn amend_open(p: &Project, ids: &[String]) -> R<Value> {
    let st = events::require_chain(p)?;
    if ids.is_empty() {
        return Err(refuse(
            "open an amendment",
            "no IDs were given",
            &["Run `bf amend open R-1 C-2` with the IDs that change."],
            "The command prints the impact set.",
        ));
    }
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
    let id = format!("A-{}", st.amends.len() + 1);
    events::append(
        p,
        "amend.open",
        json!({"amend": id, "ids": ids, "impact": imp, "impact_ids": all}),
    )?;
    p.write_guard()?;
    Ok(json!({"amend": id, "ids": ids, "impact": imp, "impact_ids": all}))
}

pub fn amend_narrow(p: &Project, file: &Value) -> R<Value> {
    let st = events::require_chain(p)?;
    let aid = file
        .get("amend")
        .and_then(|v| v.as_str())
        .map(|x| x.to_string())
        .or_else(|| {
            st.amends
                .iter()
                .rev()
                .find(|a| a.status != "approved")
                .map(|a| a.id.clone())
        });
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
    events::append(
        p,
        "amend.narrow",
        json!({"amend": aid, "keep": keep, "waive": waive, "reasons": reasons}),
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
    if a.map(|a| a.status != "narrowed").unwrap_or(true) {
        return Err(refuse(
            "approve an amendment",
            format!("{aid} is not narrowed yet"),
            &["Run `bf amend narrow --file <f>` first."],
            "`bf amend approve` prints the approved set.",
        ));
    }
    let narrow_seq = log
        .events
        .iter()
        .rev()
        .find(|e| e["kind"] == "amend.narrow" && e["amend"] == aid.as_str())
        .and_then(|e| e["seq"].as_u64())
        .unwrap_or(0);
    let (seq, _) = events::pick_turn(&st, narrow_seq, events::Decide::Approve, quote)?;
    // What the amendment re-opens, worked out from the plan now and written into the event.
    let a = a.unwrap();
    let pl = plan::load(p)?;
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
    events::append(
        p,
        "amend.approve",
        json!({"amend": aid, "turn": seq, "quote": quote.trim(),
               "reopen_units": reopen_units, "reopen_checkpoints": reopen_checkpoints,
               "reopen_checks": reopen_checks, "receipt_floor": receipt_floor}),
    )?;
    p.write_guard()?;
    Ok(json!({"amend": aid, "approved": true, "turn": seq}))
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
