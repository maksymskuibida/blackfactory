//! The repair judge (R-69, D-88, D-90): a plan repair that changes no design part is judged by a strong
//! model. A `go` inside the engine's limits is approved at once with no human; any other go is refused
//! naming each failed limit, and an `ask` waits for the human. The events and their replay live in
//! `events.rs` (H-21); the guard's pass, the role file and the seen list are the guard unit's (U-63).
//!
//! The limits are one function, `limits`, which takes the mode of the judging, so the free rules of the
//! human-absent flag (U-68) are added beside them and not inside the approval.

use crate::events::{self, Amend, State};
use crate::refusal::{refuse_owned, Fail, R};
use crate::schema;
use crate::store::{s, str_list, Project};
use crate::strings;
use crate::util;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub mod free;

/// The role of the judge's token, `<A-n>:judge`.
pub const ROLE: &str = "judge";

/// The agent type the guard accepts for the judge (U-63).
pub const AGENT_TYPE: &str = "repair-judge";

/// The paths of the engine's approval, judge, guard, roles, plan and log code and of the harness's role
/// files, worker rules, skills, hooks and installer: no unit's `files` may gain an entry that overlaps
/// one of them by a judged go (review findings M3, L-2).
pub const PROTECTED: &[&str] = &[
    "crates/bf/src/judge.rs",
    "crates/bf/src/gates.rs",
    "crates/bf/src/amend.rs",
    "crates/bf/src/cli/amend.rs",
    "crates/bf/src/events.rs",
    "crates/bf/src/events",
    "crates/bf/src/guard.rs",
    "crates/bf/src/guardtable.rs",
    "crates/bf/src/shellparse.rs",
    "crates/bf/src/roles.rs",
    "crates/bf/src/roles",
    "crates/bf/src/store.rs",
    "crates/bf/src/policy.rs",
    "crates/bf/src/decisions.rs",
    "crates/bf/src/plan.rs",
    "crates/bf/src/plan",
    "crates/bf/src/run/tokens.rs",
    "crates/bf/src/run/roles.rs",
    "crates/bf/src/cli/guard.rs",
    "crates/bf/src/run/next.rs",
    "crates/bf/src/ckfix/design.rs",
    "harness/claude/agents",
    "harness/claude/worker-rules.toml",
    "harness/claude/skills",
    "harness/claude/settings.fragment.json",
    "harness/claude/install.sh",
    // The paths of the free reshuffle's own feature (R-73).
    "crates/bf/src/judge",
    "crates/bf/src/absent.rs",
    "crates/bf/src/hooks.rs",
    "crates/bf/src/guard",
    "crates/bf/src/plan/targets.rs",
    "crates/bf/src/cli/human_turn.rs",
    "crates/bf/src/cli/hook.rs",
    "crates/bf/src/cli/record.rs",
    "harness/claude/commands",
    "harness/claude/adapter.toml",
];

/// The name of the free mode in the log: the judge's token and its events carry it.
pub const MODE_ABSENT: &str = "absent";

/// The mode of the judging: the usual one with the human present, or the free one while the run's
/// human-absent flag is on (R-73, D-95). The mode is fixed when the token is issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Present,
    Absent,
}

impl Mode {
    /// The mode as the log writes it: empty for the usual one.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Present => "",
            Mode::Absent => MODE_ABSENT,
        }
    }
    /// The mode a token asked for now would have: the run's flag, read from its state.
    pub fn now(st: &State) -> Mode {
        if crate::absent::is_on(st) {
            Mode::Absent
        } else {
            Mode::Present
        }
    }
}

/// The turn the token of `fingerprint` opened, if there is one.
fn turn_for<'a>(a: &'a Amend, fp: &str) -> Option<&'a Value> {
    a.judge_turns
        .iter()
        .rev()
        .find(|t| t["fingerprint"].as_str() == Some(fp))
}

fn is_void(t: &Value) -> bool {
    t["void"].as_bool().unwrap_or(false)
}

fn is_absent_turn(t: &Value) -> bool {
    t["mode"].as_str() == Some(MODE_ABSENT)
}

/// The turns that count: a turn of mode `absent` that the release of the flag voided does not.
pub fn live_turns(a: &Amend) -> Vec<&Value> {
    a.judge_turns.iter().filter(|t| !is_void(t)).collect()
}

/// The turns of mode `absent` counted since the run's flag was set.
fn absent_turns(a: &Amend, since_seq: u64) -> Vec<&Value> {
    live_turns(a)
        .into_iter()
        .filter(|t| is_absent_turn(t) && t["seq"].as_u64().unwrap_or(0) > since_seq)
        .collect()
}

/// Whether the turn of `fp` may speak: a turn of mode `absent` only while the run's flag is on.
pub fn turn_mode_ok(st: &State, a: &Amend, fp: &str) -> bool {
    match turn_for(a, fp) {
        Some(t) if is_absent_turn(t) => st.absent.on && !is_void(t),
        _ => true,
    }
}

/// The replay of the end of a turn (a verdict or a failed judge). In the usual mode a turn that ended
/// with no go makes the repair the human's. In mode `absent` it does so only at the last turn of the
/// policy, and then the records the narrowings of those turns changed are bound until the human saw them
/// (R-73 acceptances 9 and 10).
pub fn end_turn(st: &mut State, id: &str, fp: &str, go: bool) {
    let since = st.absent.since_seq;
    let Some(a) = st.amends.iter_mut().find(|a| a.id == id) else {
        return;
    };
    let absent = turn_for(a, fp).is_some_and(is_absent_turn);
    if let Some(t) = a
        .judge_turns
        .iter_mut()
        .rev()
        .find(|t| t["fingerprint"].as_str() == Some(fp))
    {
        t["ended"] = json!(true);
    }
    if go {
        return;
    }
    if !absent {
        a.human_only = true;
        events::keep_asked(st, id);
        return;
    }
    let turns = absent_turns(a, since);
    let limit = turns
        .last()
        .and_then(|t| t["limit"].as_u64())
        .filter(|l| *l > 0)
        .unwrap_or(3) as usize;
    if turns.len() < limit {
        return;
    }
    let mut bound: Vec<String> = turns
        .iter()
        .flat_map(|t| {
            t["ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(String::from))
        })
        .collect();
    bound.sort();
    bound.dedup();
    a.human_only = true;
    for b in bound {
        st.asked.insert(b, id.to_string());
    }
}

/// The replay of the release of the run's flag: a token of mode `absent` that no verdict ended is void
/// and counts no turn; a repair whose turns of that mode ended with no go is the human's, as in the usual
/// mode.
pub fn void_absent_turns(st: &mut State) {
    let ids: Vec<String> = st.amends.iter().map(|a| a.id.clone()).collect();
    for id in ids {
        let mut hold = false;
        if let Some(a) = st.amends.iter_mut().find(|a| a.id == id) {
            for t in a.judge_turns.iter_mut() {
                if is_absent_turn(t) && !is_void(t) && !t["ended"].as_bool().unwrap_or(false) {
                    t["void"] = json!(true);
                }
            }
            let open = matches!(a.status.as_str(), "open" | "narrowed");
            let ended = a
                .judge_turns
                .iter()
                .any(|t| is_absent_turn(t) && !is_void(t));
            if open && ended && !a.human_only {
                a.human_only = true;
                hold = true;
            }
        }
        if hold {
            events::keep_asked(st, &id);
        }
    }
}

/// Why no judge token can be issued for the narrowed repair `a` however it is revised, when that is so:
/// the switch is off, it changes a record that an unseen `ask` binds, it holds records a fix design
/// marked, or it had all its turns. The conductor withdraws such a repair while the human is away.
pub fn token_blocked(p: &Project, st: &State, a: &Amend) -> Option<String> {
    if a.status != "narrowed" {
        return None;
    }
    if !p.policy.review.repair_judge {
        return Some(strings::t("judge.token.because.switch_off", &[]));
    }
    let asked: Vec<String> = a
        .changes
        .iter()
        .filter_map(|c| c["id"].as_str())
        .filter(|id| st.asked.contains_key(*id))
        .map(|id| format!("{id} ({})", st.asked[id]))
        .collect();
    if !asked.is_empty() {
        return Some(strings::t(
            "judge.token.because.asked",
            &[("amend", &a.id), ("records", &asked.join(", "))],
        ));
    }
    let marked = refused_records(st, a);
    if !marked.is_empty() {
        return Some(strings::t(
            "judge.token.because.marked",
            &[("amend", &a.id), ("records", &marked.join(", "))],
        ));
    }
    let limit = p.policy.absent.judge_turns.max(1) as usize;
    // A turn counts as used only once it has ended: a judge that is still running may give a go.
    let ended = absent_turns(a, st.absent.since_seq)
        .iter()
        .filter(|t| t["ended"].as_bool().unwrap_or(false))
        .count();
    if ended >= limit {
        return Some(strings::t(
            "absent_judge.token.turns_used",
            &[("amend", &a.id)],
        ));
    }
    None
}

/// The judge's turns an amendment may still have in `mode`, as the token counts them.
pub fn turn_room(p: &Project, st: &State, a: &Amend, mode: Mode) -> bool {
    match mode {
        Mode::Present => live_turns(a).is_empty(),
        Mode::Absent => {
            let limit = p.policy.absent.judge_turns.max(1) as usize;
            absent_turns(a, st.absent.since_seq).len() < limit
                && !live_turns(a)
                    .iter()
                    .any(|t| t["fingerprint"].as_str() == Some(a.fingerprint.as_str()))
        }
    }
}

/// Whether an `amend.approve` written by the judge (`by = "judge"`, with `judge_seq`) counts on replay:
/// only when `judge_seq` names the `amend.judge` of the same amendment with `go`, written after its
/// latest narrowing, holding that narrowing's fingerprint, with a judge token issued before it.
pub fn approve_counts(st: &State, ev: &Value) -> bool {
    let id = ev["amend"].as_str().unwrap_or("");
    let Some(js) = ev["judge_seq"].as_u64() else {
        return false;
    };
    let Some(a) = st.amends.iter().find(|a| a.id == id) else {
        return false;
    };
    let Some(v) = &a.verdict else {
        return false;
    };
    a.status == "narrowed"
        && !a.human_only
        && !a.from_design
        && v["seq"].as_u64() == Some(js)
        && v["verdict"].as_str() == Some("go")
        && js > a.changed_seq
        && v["fingerprint"].as_str() == Some(a.fingerprint.as_str())
        && turn_mode_ok(st, a, &a.fingerprint)
        && ev["mode"].as_str().unwrap_or("")
            == turn_for(a, &a.fingerprint).map_or("", |t| t["mode"].as_str().unwrap_or(""))
        && a.judge_turns.iter().any(|t| {
            t["fingerprint"].as_str() == Some(a.fingerprint.as_str())
                && t["seq"].as_u64().is_some_and(|x| x < js)
        })
}

/// The records of an amendment that the fix design's marked items name (`ckfix::design::marked_records`):
/// a repair that holds one gets no judge token. Every record the amendment names or changes counts.
pub fn refused_records(st: &State, a: &Amend) -> Vec<String> {
    let marked = crate::ckfix::design::marked_records(st);
    let mut named: Vec<String> = a.ids.clone();
    named.extend(
        a.changes
            .iter()
            .filter_map(|c| c["id"].as_str().map(|x| x.to_string())),
    );
    named.sort();
    named.dedup();
    named.into_iter().filter(|x| marked.contains(x)).collect()
}

/// The pass the guard wrote for the judge's command (U-63): who spoke.
pub struct Pass {
    pub session: String,
    pub agent: String,
}

/// The file of the state folder that holds the pass: one pass at a time, used once.
pub const PASS_FILE: &str = "judge-pass.json";

/// How long a pass is fresh, in seconds: the judge runs its command right after the guard saw it.
pub const PASS_SECS: u64 = 600;

fn pass_sig(key: &str, body: &Value) -> String {
    util::hmac_sha256_hex(
        key.trim().as_bytes(),
        serde_json::to_string(body).unwrap_or_default().as_bytes(),
    )
}

/// The guard's side: write the signed one-use pass for `command`, a call of `bf amend judge <amend>` that
/// came with the agent type `repair-judge` (session, agent id, agent type, time and the command).
pub fn write_pass(
    state_dir: &std::path::Path,
    session: &str,
    agent: &str,
    amend: &str,
    command: &str,
) -> std::io::Result<()> {
    let key = std::fs::read_to_string(state_dir.join("key"))?;
    let mut body = json!({"v": 1, "amend": amend, "session": session, "agent": agent,
        "agent_type": AGENT_TYPE, "at": util::now_secs(), "command": command});
    let sig = pass_sig(&key, &body);
    body["sig"] = Value::String(sig);
    std::fs::write(state_dir.join(PASS_FILE), body.to_string())
}

/// The value of `--token` in the command a pass was written for.
fn command_token(command: &str) -> Option<String> {
    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || "\"'".contains(c))
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().enumerate().find_map(|(i, w)| {
        if *w == "--token" {
            words.get(i + 1).map(|t| t.to_string())
        } else {
            w.strip_prefix("--token=").map(|t| t.to_string())
        }
    })
}

/// Take the one-use pass for `bf amend judge <amend>`: it exists only when the guard saw the agent type
/// `repair-judge` in the hook payload, signed it with the project's key, wrote it for this amendment, and
/// it is fresh. The reason is the text of a refusal when there is none. A pass written for another
/// amendment stays where it is and is not taken.
fn take_pass(p: &Project, amend: &str, token: Option<&str>) -> Result<Pass, String> {
    let none = || strings::t("judge.failed.no_pass", &[("amend", amend)]);
    let path = p.state_dir().join(PASS_FILE);
    let text = std::fs::read_to_string(&path).map_err(|_| none())?;
    let mut v: Value = serde_json::from_str(&text).map_err(|_| none())?;
    let sig = v
        .as_object_mut()
        .and_then(|o| o.remove("sig"))
        .and_then(|x| x.as_str().map(|x| x.to_string()))
        .unwrap_or_default();
    let key = p.read_key().ok_or_else(none)?;
    if sig.is_empty() || pass_sig(&key, &v) != sig {
        return Err(strings::t(
            "judge_guard.pass.bad_signature",
            &[("amend", amend)],
        ));
    }
    if s(&v, "agent_type") != AGENT_TYPE {
        return Err(none());
    }
    if s(&v, "amend") != amend {
        return Err(strings::t(
            "judge_guard.pass.other_amend",
            &[("amend", amend), ("other", s(&v, "amend"))],
        ));
    }
    if command_token(s(&v, "command")).as_deref() != token {
        return Err(strings::t(
            "judge_guard.pass.other_token",
            &[("amend", amend)],
        ));
    }
    let at = v["at"].as_u64().unwrap_or(0);
    let now = util::now_secs();
    if at > now + 60 || now.saturating_sub(at) > PASS_SECS {
        let _ = std::fs::remove_file(&path);
        return Err(strings::t("judge_guard.pass.stale", &[("amend", amend)]));
    }
    let _ = std::fs::remove_file(&path);
    Ok(Pass {
        session: s(&v, "session").to_string(),
        agent: s(&v, "agent").to_string(),
    })
}

/// The harness copy of the role file, compiled in.
const ROLE_FILE: &[u8] = include_bytes!("../../../harness/claude/agents/repair-judge.md");

/// The part of a role file above its worker-rules section: the installer writes the section from the
/// data, so only what is above it must match.
fn above_rules(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches('\n') == "## Worker rules" {
            break;
        }
        out.push_str(line);
    }
    out
}

/// Why the installed role file of the judge differs from the harness copy (U-63), when it does.
fn role_file_problem(p: &Project) -> Option<String> {
    let path = p.root.join(".claude/agents/repair-judge.md");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Some(strings::t("judge_guard.role_file.missing", &[]));
    };
    let harness = String::from_utf8_lossy(ROLE_FILE);
    if above_rules(&text) != above_rules(&harness) {
        return Some(strings::t("judge_guard.role_file.differs", &[]));
    }
    None
}

fn token_key(amend: &str) -> String {
    format!("{amend}:{ROLE}")
}

fn deny(
    what_key: &str,
    vals: &[(&str, &str)],
    because_key: &str,
    because: &[(&str, &str)],
) -> Fail {
    refuse_owned(
        strings::t(what_key, vals),
        strings::t(because_key, because),
        vec![strings::t("judge.refusal.do_this", &[])],
        strings::t("judge.refusal.worked_when", &[]),
    )
}

// ---- the token ----

/// `bf roles token <A-n> judge`: the one-use token of the judge, for a narrowed plan repair that has no
/// design part, is not the human's for good, has had no verdict and no judge turn for its latest
/// narrowing and none that ended with no verdict, changes no record of an unseen `ask`, names none of
/// the records the guard unit refuses, while the switch is on. An issued token writes
/// `amend.judge.token`.
pub fn token(p: &Project, amend: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    let refused = |key: &str, vals: &[(&str, &str)]| {
        let mut v = vec![("amend", amend)];
        v.extend_from_slice(vals);
        deny("judge.token.refused", &[("amend", amend)], key, &v)
    };
    if !p.policy.review.repair_judge {
        return Err(refused("judge.token.because.switch_off", &[]));
    }
    let Some(a) = st.amends.iter().find(|a| a.id == amend) else {
        return Err(refused("judge.token.because.no_amendment", &[]));
    };
    if a.status != "narrowed" {
        return Err(refused(
            "judge.token.because.not_narrowed",
            &[("status", a.status.as_str())],
        ));
    }
    if !crate::amend::is_repair(a) {
        return Err(refused("judge.token.because.not_repair", &[]));
    }
    let designed = crate::amend::designed_in(&a.ids);
    if !designed.is_empty() {
        let named: Vec<String> = designed
            .iter()
            .map(|i| {
                let k = schema::kind_of_id(i).map(|k| k.name).unwrap_or("part");
                format!("{i} ({k})")
            })
            .collect();
        return Err(refused(
            "judge.token.because.design_part",
            &[("parts", &named.join(", "))],
        ));
    }
    if a.human_only || a.from_design {
        return Err(refused("judge.token.because.human_only", &[]));
    }
    let mode = Mode::now(&st);
    let own = |t: &&Value| t["fingerprint"].as_str() == Some(a.fingerprint.as_str());
    if a.verdict
        .as_ref()
        .is_some_and(|v| v["fingerprint"].as_str() == Some(a.fingerprint.as_str()))
    {
        return Err(refused("judge.token.because.has_verdict", &[]));
    }
    if live_turns(a).iter().any(own) {
        return Err(refused("judge.token.because.has_token", &[]));
    }
    if !turn_room(p, &st, a, mode) {
        let key = if mode == Mode::Absent {
            "absent_judge.token.turns_used"
        } else {
            "judge.token.because.earlier_turn"
        };
        return Err(refused(key, &[]));
    }
    let asked: Vec<String> = a
        .changes
        .iter()
        .filter_map(|c| c["id"].as_str())
        .filter(|id| st.asked.contains_key(*id))
        .map(|id| format!("{id} ({})", st.asked[id]))
        .collect();
    if !asked.is_empty() {
        return Err(refused(
            "judge.token.because.asked",
            &[("records", &asked.join(", "))],
        ));
    }
    let marked = refused_records(&st, a);
    if !marked.is_empty() {
        return Err(refused(
            "judge.token.because.marked",
            &[("records", &marked.join(", "))],
        ));
    }
    events::append(
        p,
        "amend.judge.token",
        json!({"amend": amend, "fingerprint": a.fingerprint, "mode": mode.name(),
            "limit": p.policy.absent.judge_turns.max(1)}),
    )?;
    let tok = crate::run::issue_token(p, &token_key(amend))?;
    p.write_guard()?;
    Ok(json!({"amend": amend, "role": ROLE, "token": tok,
        "mode": if mode == Mode::Absent { MODE_ABSENT } else { "present" }}))
}

/// `bf amend open --repair --from-design <K>`: the mark needs the repair mark and a checkpoint.
pub fn refuse_bad_from_design(_p: &Project, k: &str, repair: bool) -> R<()> {
    let is_cp = schema::parse_id(k).map(|(pre, _)| pre) == Some("K");
    if repair && is_cp {
        return Ok(());
    }
    Err(refuse_owned(
        strings::t("judge.from_design.refused", &[]),
        if repair {
            strings::t("judge.from_design.because.not_checkpoint", &[("value", k)])
        } else {
            strings::t("judge.from_design.because.not_repair", &[])
        },
        vec![strings::t("judge.from_design.do_this", &[])],
        strings::t("judge.from_design.worked_when", &[]),
    ))
}

// ---- the verdict ----

/// The verdict of the file: `go` or `ask` with reasons in plain words.
fn parse_verdict(text: &str) -> Result<(String, String), String> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| strings::t("judge.failed.unreadable_json", &[("error", &e.to_string())]))?;
    let verdict = v["verdict"].as_str().unwrap_or("").trim().to_string();
    if !matches!(verdict.as_str(), "go" | "ask") {
        return Err(strings::t("judge.failed.unreadable_verdict", &[]));
    }
    let reasons = v["reasons"].as_str().unwrap_or("").trim().to_string();
    if reasons.is_empty() {
        return Err(strings::t("judge.failed.unreadable_reasons", &[]));
    }
    Ok((verdict, reasons))
}

fn read_verdict_text(p: &Project, source: &str) -> Result<String, String> {
    use std::io::Read;
    if source == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).map_err(|e| {
            strings::t("judge.failed.unreadable_read", &[("error", &e.to_string())])
        })?;
        return Ok(text);
    }
    let safe = p
        .safe_input(std::path::Path::new(source))
        .map_err(|e| failure_text(&e))?;
    std::fs::read_to_string(safe)
        .map_err(|e| strings::t("judge.failed.unreadable_read", &[("error", &e.to_string())]))
}

/// The reason of a refusal, as the words of a failed judge.
fn failure_text(f: &Fail) -> String {
    match f {
        Fail::Refused(r) => r.because.clone(),
        Fail::Error(e) => e.clone(),
    }
}

/// `bf amend judge <A-n> --file <f|-> --token T --model M`: the judge's verdict. It puts no record and
/// does not narrow. Every refusal after the token of the latest narrowing was taken writes
/// `amend.judge.failed` first, so the repair is the human's and no second judge is started (F-212).
pub fn judge(p: &Project, amend: &str, source: &str, token: Option<&str>, model: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    let plain = |key: &str, vals: &[(&str, &str)]| {
        let mut v = vec![("amend", amend)];
        v.extend_from_slice(vals);
        deny("judge.refused", &[("amend", amend)], key, &v)
    };
    let Some(a) = st.amends.iter().find(|a| a.id == amend) else {
        return Err(plain("judge.because.no_amendment", &[]));
    };
    if !crate::run::holds_token(p, &token_key(amend), token) {
        return Err(plain("judge.because.token", &[]));
    }
    // The mode is the token's. A token of mode `absent` is void once the run's flag is released: no
    // verdict is taken, no turn is counted, and the repair is judged again in the usual mode.
    let mode = if a.judge_turns.last().is_some_and(is_absent_turn) {
        Mode::Absent
    } else {
        Mode::Present
    };
    // A turn the release voided stays void when the flag is set again: it takes no verdict.
    let voided = a.judge_turns.last().is_some_and(is_void);
    if mode == Mode::Absent && (!st.absent.on || voided) {
        let _ = crate::run::spend_token(p, &token_key(amend));
        return Err(plain("absent_judge.released", &[]));
    }
    // The token is for the latest narrowing, and that narrowing has not had its turn.
    let turn_is_current = a
        .judge_turns
        .last()
        .is_some_and(|t| t["fingerprint"].as_str() == Some(a.fingerprint.as_str()));
    if a.status != "narrowed" || !turn_is_current {
        let _ = crate::run::spend_token(p, &token_key(amend));
        return Err(plain("judge.because.old_token", &[]));
    }
    let ended = |v: &Option<Value>| {
        v.as_ref()
            .is_some_and(|v| v["fingerprint"].as_str() == Some(a.fingerprint.as_str()))
    };
    if ended(&a.verdict) || ended(&a.failed) || a.human_only {
        let _ = crate::run::spend_token(p, &token_key(amend));
        return Err(plain("judge.because.had_turn", &[]));
    }
    // From here on the turn is the judge's: every refusal writes the failure first.
    let finish_failed = |reason: String| -> R<Value> {
        events::append(
            p,
            "amend.judge.failed",
            json!({"amend": amend, "fingerprint": a.fingerprint, "reason": reason,
                "mode": mode.name()}),
        )?;
        let _ = crate::run::spend_token(p, &token_key(amend));
        p.write_guard()?;
        Err(deny(
            "judge.refused",
            &[("amend", amend)],
            "judge.because.failed",
            &[("amend", amend), ("reason", &reason)],
        ))
    };
    let pass = match take_pass(p, amend, token) {
        Ok(x) => x,
        Err(reason) => return finish_failed(reason),
    };
    if let Some(reason) = role_file_problem(p) {
        return finish_failed(reason);
    }
    let text = match read_verdict_text(p, source) {
        Ok(t) => t,
        Err(reason) => return finish_failed(reason),
    };
    let (verdict, reasons) = match parse_verdict(&text) {
        Ok(x) => x,
        Err(reason) => return finish_failed(reason),
    };
    let (failures, measures) = match limits(p, &st, a, mode) {
        Ok(x) => x,
        Err(e) => return finish_failed(failure_text(&e)),
    };
    if verdict == "go" {
        if !failures.is_empty() {
            let list = failures.join("; ");
            return finish_failed(strings::t(
                "judge.failed.limits",
                &[("count", &failures.len().to_string()), ("limits", &list)],
            ));
        }
        // The checks every approval passes, before anything is written.
        let log = events::load_log(p)?;
        let last_change = log.events.iter().rev().find(|e| {
            matches!(e["kind"].as_str(), Some("amend.narrow" | "amend.revise"))
                && e["amend"] == amend
        });
        let shared = crate::gates::refuse_if_names_missing_for(p, a)
            .and_then(|_| crate::gates::approval_checks(p, &st, a, last_change, false));
        if let Err(e) = shared {
            return finish_failed(failure_text(&e));
        }
        let edit = match crate::gates::approval_edit_of(p, &st, a) {
            Ok(x) => x,
            Err(e) => return finish_failed(failure_text(&e)),
        };
        let ev = events::append(
            p,
            "amend.judge",
            json!({"amend": amend, "verdict": verdict, "reasons": reasons, "model": model,
                "fingerprint": a.fingerprint, "measures": measures, "mode": mode.name(),
                "session": pass.session, "agent": pass.agent}),
        )?;
        let judge_seq = ev["seq"].as_u64().unwrap_or(0);
        let _ = crate::run::spend_token(p, &token_key(amend));
        let unblocked = crate::gates::finish_approval(
            p,
            &st,
            a,
            edit,
            json!({"by": "judge", "judge_seq": judge_seq, "mode": mode.name(),
                "measures": measures}),
        )?;
        return Ok(
            json!({"amend": amend, "verdict": "go", "approved": true, "mode": mode.name(),
            "judge_seq": judge_seq, "unblocked": unblocked, "measures": measures}),
        );
    }
    events::append(
        p,
        "amend.judge",
        json!({"amend": amend, "verdict": verdict, "reasons": reasons, "model": model,
            "fingerprint": a.fingerprint, "measures": measures, "mode": mode.name(),
            "session": pass.session, "agent": pass.agent}),
    )?;
    let _ = crate::run::spend_token(p, &token_key(amend));
    p.write_guard()?;
    Ok(
        json!({"amend": amend, "verdict": "ask", "approved": false, "mode": mode.name(),
        "reasons": reasons}),
    )
}

// ---- the human's sight ----

/// `bf amend seen <A-n>... --quote "<the human's words>"`, or `--all`: the human says they saw judged
/// repairs, `ask`s or failed judges, all on one message of theirs. It writes one `amend.judge.seen`; on
/// replay it counts only with a human turn after the latest verdict of them whose words carry the quote.
pub fn seen(p: &Project, amends: &[String], all: bool, quote: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    let turn_of = |a: &Amend| {
        [
            a.verdict.as_ref().and_then(|v| v["seq"].as_u64()),
            a.failed.as_ref().and_then(|v| v["seq"].as_u64()),
        ]
        .into_iter()
        .flatten()
        .max()
    };
    let name = amends.join(", ");
    let refused = |key: &str, id: &str| {
        refuse_owned(
            strings::t("judge_guard.seen.refused", &[("amend", &name)]),
            strings::t(key, &[("amend", id)]),
            vec![strings::t("judge_guard.seen.do_this", &[])],
            strings::t("judge_guard.seen.worked_when", &[]),
        )
    };
    let ids: Vec<String> = if all {
        st.amends
            .iter()
            .filter(|a| !a.seen && turn_of(a).is_some())
            .map(|a| a.id.clone())
            .collect()
    } else {
        amends.to_vec()
    };
    if ids.is_empty() {
        return Err(refused("absent_judge.seen.none", ""));
    }
    let mut after = 0u64;
    for id in &ids {
        let Some(a) = st.amends.iter().find(|a| a.id == *id) else {
            return Err(refused("judge_guard.seen.because.no_amendment", id));
        };
        let Some(t) = turn_of(a) else {
            return Err(refused("judge_guard.seen.because.no_turn", id));
        };
        if a.seen {
            return Err(refused("judge_guard.seen.because.already", id));
        }
        after = after.max(t);
    }
    let (turn, _) = events::pick_turn(&st, after, events::Decide::Approve, quote)?;
    let mut data = json!({"amend": ids[0], "amends": ids, "turn": turn, "quote": quote.trim()});
    if ids.len() == 1 {
        data.as_object_mut().map(|o| o.remove("amends"));
    }
    events::append(p, "amend.judge.seen", data)?;
    Ok(json!({"amend": ids[0], "amends": ids, "seen": true, "turn": turn}))
}

// ---- the limits ----

fn norm(path: &str) -> String {
    util::norm_scope(path.trim())
}

/// The protected path that `path` overlaps, if any.
fn protected_hit(path: &str) -> Option<&'static str> {
    let n = norm(path);
    PROTECTED
        .iter()
        .find(|q| util::norm_overlap(&n, &norm(q)))
        .copied()
}

/// Whether `path` is a new path in a tests folder: one of its folders is named `tests`.
fn in_tests_folder(path: &str) -> bool {
    let t = path.trim();
    // One named file: never a glob and never a folder (R-69).
    if t.ends_with('/') || t.contains(['*', '?', '[', '{']) {
        return false;
    }
    let n = norm(path);
    let parts: Vec<&str> = n.split('/').collect();
    parts.len() > 1 && parts[..parts.len() - 1].contains(&"tests")
}

/// Whether `path` exists already in the unit's repository, at the checkpoint's branch when it has one, else at
/// the main branch. Without a repository to ask, every repository of the workspace is asked.
fn exists_in_repo(p: &Project, unit: &Value, path: &str) -> bool {
    let n = norm(path);
    let repos: Vec<&crate::store::Repo> = match p.repo_of_unit(unit) {
        Ok(r) => vec![r],
        Err(_) => p.repos.iter().collect(),
    };
    let cp = p.checkpoint_branch(s(unit, "checkpoint"));
    repos.iter().any(|r| {
        let branch = if crate::gitx::branch_exists(&r.path, &cp) {
            cp.clone()
        } else {
            r.branch.clone()
        };
        crate::gitx::run(&r.path, &["cat-file", "-e", &format!("{branch}:{n}")], &[])
            .map(|o| o.ok)
            .unwrap_or(false)
    })
}

/// The wave of each unit: one after the highest wave of what it depends on, as `bf plan check` counts.
fn waves(units: &[Value]) -> HashMap<String, u32> {
    let ids: Vec<String> = units.iter().map(|u| s(u, "id").to_string()).collect();
    let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for u in units {
        let id = s(u, "id").to_string();
        let d: Vec<String> = str_list(u, "depends_on")
            .into_iter()
            .filter(|x| ids.contains(x) && *x != id)
            .collect();
        deps.insert(id, d);
    }
    let mut wave: HashMap<String, u32> = HashMap::new();
    let mut remaining: Vec<String> = deps.keys().cloned().collect();
    loop {
        let before = remaining.len();
        let mut next = Vec::new();
        for id in &remaining {
            if deps[id].iter().all(|d| wave.contains_key(d)) {
                let w = deps[id].iter().map(|d| wave[d]).max().unwrap_or(0) + 1;
                wave.insert(id.clone(), w);
            } else {
                next.push(id.clone());
            }
        }
        remaining = next;
        if remaining.is_empty() || remaining.len() == before {
            break;
        }
    }
    wave
}

/// The longest run of waves in a row that hold exactly one unit of checkpoint `k`.
fn longest_run(units: &[Value], k: &str) -> u32 {
    let wave = waves(units);
    let mut per: BTreeMap<u32, u32> = BTreeMap::new();
    for u in units.iter().filter(|u| s(u, "checkpoint") == k) {
        if let Some(w) = wave.get(s(u, "id")) {
            *per.entry(*w).or_default() += 1;
        }
    }
    let (mut best, mut run, mut prev) = (0u32, 0u32, 0u32);
    for (w, n) in per {
        if n == 1 {
            run = if run > 0 && w == prev + 1 { run + 1 } else { 1 };
            prev = w;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// The units as they were before a narrowing's changes: new ones gone, changed and removed ones as they
/// were approved.
fn revert_units(units: &mut Vec<Value>, changes: &[Value]) {
    for c in changes {
        let rec = if c["record"].is_null() {
            &c["was"]
        } else {
            &c["record"]
        };
        if s(rec, "kind") != "unit" {
            continue;
        }
        let id = s(c, "id").to_string();
        units.retain(|u| s(u, "id") != id);
        if s(c, "how") != "new" && !c["was"].is_null() {
            units.push(c["was"].clone());
        }
    }
}

/// The estimate of a unit: no estimate counts 1.
fn estimate(u: &Value) -> u64 {
    u.get("estimate").and_then(|e| e.as_u64()).unwrap_or(1)
}

fn unit_changes(a: &Amend) -> impl Iterator<Item = &Value> {
    a.changes.iter().filter(|c| {
        let rec = if c["record"].is_null() {
            &c["was"]
        } else {
            &c["record"]
        };
        s(rec, "kind") == "unit"
    })
}

fn scope_changed(c: &Value) -> bool {
    s(c, "how") == "changed"
        && (c["record"].get("files") != c["was"].get("files")
            || c["record"].get("estimate") != c["was"].get("estimate"))
}

/// The units of a checkpoint over the plan before the first of the repairs counted, and the units that
/// a repair counted changed in `files` or `estimate`, each with the larger estimate it had.
struct Scope {
    before_total: BTreeMap<String, u64>,
    changed: BTreeMap<String, u64>,
}

fn scope_of(st: &State, a: &Amend, now: &[Value]) -> Scope {
    // The repairs counted: the judged repairs the human has not seen, in log order, then this one.
    let mut repairs: Vec<&Amend> = st
        .judged_unseen
        .iter()
        .filter(|id| **id != a.id)
        .filter_map(|id| st.amends.iter().find(|x| x.id == *id))
        .collect();
    repairs.push(a);
    let mut before: Vec<Value> = now.to_vec();
    for r in repairs.iter().rev() {
        revert_units(&mut before, &r.changes);
    }
    // Each changed unit with the largest estimate it had in any of the repairs.
    let mut unit_value: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for r in &repairs {
        for c in unit_changes(r).filter(|c| scope_changed(c)) {
            let id = s(c, "id").to_string();
            let k = s(&c["record"], "checkpoint").to_string();
            let v = estimate(&c["record"]).max(estimate(&c["was"]));
            let e = unit_value.entry(id).or_insert((k, 0));
            e.1 = e.1.max(v);
        }
    }
    let mut changed: BTreeMap<String, u64> = BTreeMap::new();
    for (k, v) in unit_value.into_values() {
        *changed.entry(k).or_default() += v;
    }
    let mut before_total: BTreeMap<String, u64> = BTreeMap::new();
    for u in &before {
        *before_total
            .entry(s(u, "checkpoint").to_string())
            .or_default() += estimate(u);
    }
    Scope {
        before_total,
        changed,
    }
}

/// The index of a checkpoint in the plan's order: its number.
fn cp_order(id: &str) -> u32 {
    schema::parse_id(id).map(|x| x.1).unwrap_or(0)
}

/// The free rules of mode `absent` (R-73, D-95), in place of the field list, the scope limit and the
/// chain limit: the scripts of U-68 over the plan in force and the plan the narrowing leaves, and the
/// measures, which refuse nothing.
fn absent_limits(p: &Project, st: &State, a: &Amend) -> R<(Vec<String>, Value)> {
    use std::collections::BTreeSet;
    let before = crate::gates::before_plan(p, a)?;
    let after = crate::plan::load(p)?;
    let ids =
        |l: &[Value]| -> BTreeSet<String> { l.iter().map(|u| s(u, "id").to_string()).collect() };
    let mut landed = ids(&before.units);
    landed.extend(ids(&after.units));
    landed.retain(|u| st.unit_state(u) == "landed");
    let closed: BTreeSet<String> = st.closed.keys().cloned().collect();
    let design = ids(&after.designed);
    let new_problems = crate::gates::new_plan_problems(p, a, false)?;
    let facts = free::Facts {
        before: free::Slice {
            units: &before.units,
            handoffs: &before.handoffs,
            checkpoints: &before.checkpoints,
            checks: &before.checks,
        },
        after: free::Slice {
            units: &after.units,
            handoffs: &after.handoffs,
            checkpoints: &after.checkpoints,
            checks: &after.checks,
        },
        changes: &a.changes,
        ops: a.ops.len(),
        waive: &a.waive,
        landed: &landed,
        closed: &closed,
        design: &design,
        new_problems: &new_problems,
    };
    let mut out = free::refusals(&facts);
    out.dedup();
    let mut cp_ids: Vec<String> = ids(&after.checkpoints).into_iter().collect();
    cp_ids.sort_by_key(|k| cp_order(k));
    let measures = free::measures(&before.units, &after.units, &cp_ids);
    Ok((out, measures))
}

/// The engine's limits of a judged go, each failure named, and the measures (for each checkpoint the
/// share of its scope changed and the longest run of one-unit waves, before and after). The limits are
/// checked on every record the narrowing lists as changed, kept records included.
pub fn limits(p: &Project, st: &State, a: &Amend, mode: Mode) -> R<(Vec<String>, Value)> {
    if mode == Mode::Absent {
        return absent_limits(p, st, a);
    }
    let mut out: Vec<String> = Vec::new();
    let f = |key: &str, vals: &[(&str, &str)]| strings::t(key, vals);
    let ledger: Vec<&str> = p.policy.ledger.kinds.iter().map(|x| x.as_str()).collect();
    for w in &a.waive {
        out.push(f("judge.limit.waive", &[("id", w)]));
    }
    if !a.ops.is_empty() {
        out.push(f("judge.limit.checkpoint_operation", &[]));
    }
    let mut removed: Vec<(String, String)> = vec![];
    let mut added: Vec<(String, String)> = vec![];
    for c in &a.changes {
        let id = s(c, "id");
        let rec = if c["record"].is_null() {
            &c["was"]
        } else {
            &c["record"]
        };
        let kind = s(rec, "kind");
        if schema::is_designed_id(id) || ledger.contains(&kind) {
            out.push(f("judge.limit.design_part", &[("id", id), ("kind", kind)]));
            continue;
        }
        match s(c, "how") {
            "new" => {
                out.push(f("judge.limit.added", &[("id", id)]));
                continue;
            }
            "removed" => {
                out.push(f("judge.limit.deleted", &[("id", id)]));
                continue;
            }
            _ => {}
        }
        if kind == "checkpoint" && st.closed.contains_key(id) {
            out.push(f("judge.limit.closed_checkpoint", &[("id", id)]));
            continue;
        }
        if kind != "unit" {
            out.push(f("judge.limit.not_unit", &[("id", id), ("kind", kind)]));
            continue;
        }
        if st.unit_state(id) == "landed" {
            out.push(f("judge.limit.landed_unit", &[("id", id)]));
            continue;
        }
        let was = &c["was"];
        let mut keys: Vec<&String> = rec
            .as_object()
            .into_iter()
            .chain(was.as_object())
            .flat_map(|o| o.keys())
            .collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            if rec.get(k.as_str()) == was.get(k.as_str()) {
                continue;
            }
            if !matches!(k.as_str(), "depends_on" | "estimate" | "files") {
                out.push(f("judge.limit.field", &[("id", id), ("field", k)]));
            }
        }
        let old_deps = str_list(was, "depends_on");
        for d in old_deps
            .iter()
            .filter(|d| !str_list(rec, "depends_on").contains(d))
        {
            out.push(f("judge.limit.depends_removed", &[("id", id), ("dep", d)]));
        }
        let (old, new) = (str_list(was, "files"), str_list(rec, "files"));
        for x in old.iter().filter(|x| !new.contains(x)) {
            removed.push((id.to_string(), x.clone()));
        }
        for x in new.iter().filter(|x| !old.contains(x)) {
            added.push((id.to_string(), x.clone()));
        }
    }
    for (u, path) in &added {
        let moved = removed
            .iter()
            .any(|(u2, p2)| u2 != u && norm(p2) == norm(path));
        let existing = !moved
            && in_tests_folder(path)
            && a.changes
                .iter()
                .find(|c| s(c, "id") == u)
                .map(|c| {
                    let rec = if c["record"].is_null() {
                        &c["was"]
                    } else {
                        &c["record"]
                    };
                    exists_in_repo(p, rec, path)
                })
                .unwrap_or(false);
        if !moved && (!in_tests_folder(path) || existing) {
            out.push(f("judge.limit.files_added", &[("id", u), ("path", path)]));
        }
        if let Some(q) = protected_hit(path) {
            out.push(f(
                "judge.limit.files_protected",
                &[("id", u), ("path", path), ("protected", q)],
            ));
        }
    }
    for (u, path) in &removed {
        let moved = added
            .iter()
            .any(|(u2, p2)| u2 != u && norm(p2) == norm(path));
        if !moved {
            out.push(f("judge.limit.files_removed", &[("id", u), ("path", path)]));
        }
    }
    // The plan it leaves.
    let now_units = p.list_records(schema::kind_by_name("unit"))?;
    let mut before_units = now_units.clone();
    revert_units(&mut before_units, &a.changes);
    let cps = p.list_records(schema::kind_by_name("checkpoint"))?;
    let mut cp_ids: Vec<String> = cps.iter().map(|k| s(k, "id").to_string()).collect();
    cp_ids.sort_by_key(|k| cp_order(k));
    // A unit left depending on a unit of a later checkpoint, unless the plan in force had that pair.
    for u in &now_units {
        let uid = s(u, "id");
        let uk = cp_order(s(u, "checkpoint"));
        for d in str_list(u, "depends_on") {
            let Some(dep) = now_units.iter().find(|x| s(x, "id") == d) else {
                continue;
            };
            if cp_order(s(dep, "checkpoint")) <= uk {
                continue;
            }
            let had = before_units
                .iter()
                .any(|b| s(b, "id") == uid && str_list(b, "depends_on").contains(&d));
            if !had {
                out.push(f(
                    "judge.limit.later_checkpoint",
                    &[
                        ("unit", uid),
                        ("dep", &d),
                        ("checkpoint", s(u, "checkpoint")),
                        ("later", s(dep, "checkpoint")),
                    ],
                ));
            }
        }
    }
    // The approval adds no plan problem.
    for x in crate::gates::new_plan_problems(p, a, false)? {
        out.push(f("judge.limit.plan_problem", &[("problem", &x)]));
    }
    // The human's bound (D-88): scope and chain, measured as D-90 says.
    let scope = scope_of(st, a, &now_units);
    let (pct, chain) = (
        p.policy.review.judge_scope_percent as u64,
        p.policy.review.judge_chain_waves,
    );
    let mut measures = serde_json::Map::new();
    for k in &cp_ids {
        let total = scope.before_total.get(k).copied().unwrap_or(0);
        let changed = scope.changed.get(k).copied().unwrap_or(0);
        let share = if total == 0 {
            if changed > 0 {
                100
            } else {
                0
            }
        } else {
            (changed * 100).div_ceil(total)
        };
        let (rb, ra) = (longest_run(&before_units, k), longest_run(&now_units, k));
        measures.insert(
            k.clone(),
            json!({"scope_changed": changed, "scope_total": total, "scope_percent": share,
                "chain_before": rb, "chain_after": ra}),
        );
        if changed * 100 > pct * total.max(1) || (total == 0 && changed > 0) {
            out.push(f(
                "judge.limit.scope",
                &[
                    ("checkpoint", k),
                    ("percent", &share.to_string()),
                    ("limit", &pct.to_string()),
                ],
            ));
        }
        if ra > chain && ra > rb {
            out.push(f(
                "judge.limit.chain",
                &[
                    ("checkpoint", k),
                    ("run", &ra.to_string()),
                    ("limit", &chain.to_string()),
                    ("before", &rb.to_string()),
                ],
            ));
        }
    }
    out.dedup();
    Ok((out, Value::Object(measures)))
}
