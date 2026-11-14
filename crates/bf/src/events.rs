//! The append-only run log and the state replayed from it.
//!
//! One JSON object per line in `factory/log/events.jsonl`. Every event carries a
//! sequence number, the hash of the previous event, and its own hash, so any
//! edit or deletion of a past line breaks the chain and is reported.
//! State is never stored: it is always replayed from the log.

use crate::refusal::{err, refuse, R};
use crate::store::Project;
use crate::util;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;

/// The gates of the design phase: a test-mode grant may satisfy these before a run starts.
pub const DESIGN_GATES: &[&str] = &["idea", "requirements", "ui", "architecture", "plan"];

pub struct Log {
    pub events: Vec<Value>,
    pub chain_ok: bool,
    pub chain_error: Option<String>,
    /// Set when the log disagrees with the head record kept outside the project.
    pub head_error: Option<String>,
    pub last_hash: String,
    /// Byte length of the valid, newline-terminated part of the file.
    pub valid_len: usize,
    pub file_len: usize,
}

/// The keyed hash of one event: HMAC-SHA256 with the project's key over the previous hash and the
/// event's own fields. Without the key (it lives outside the project) no line can be made to verify.
pub fn event_hash(key: &str, prev: &str, ev: &Map<String, Value>) -> String {
    let mut m = ev.clone();
    m.remove("h");
    let body = serde_json::to_string(&Value::Object(m)).unwrap_or_default();
    util::hmac_sha256_hex(key.as_bytes(), format!("{prev}\n{body}").as_bytes())
}

/// The log's head, kept outside the project: how many events there are and the hash of the last.
/// A log that is shorter than the head, or whose event at that place differs, was rolled back.
#[derive(Clone, Debug, Default)]
pub struct Head {
    pub count: u64,
    pub last_hash: String,
    /// Per code repository: the main branch watched and the commit the engine last put there or the
    /// human accepted. (A project with one repository at `.` has one entry.)
    pub repos: std::collections::BTreeMap<String, RepoHead>,
    /// SHA-256 of the policy the human accepted, and when (seconds since the epoch).
    pub policy_hash: Option<String>,
    pub policy_at: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct RepoHead {
    /// The branch watched. It is kept here, outside the project, so an edit of `factory/config.toml` or
    /// `factory/workspace.toml` cannot move the watch; `bf head accept` takes the file's value.
    pub branch: Option<String>,
    pub commit: Option<String>,
}

impl Head {
    pub fn commit_of(&self, repo: &str) -> Option<String> {
        self.repos.get(repo).and_then(|r| r.commit.clone())
    }
    pub fn set_commit(&mut self, repo: &str, commit: Option<String>) {
        self.repos.entry(repo.to_string()).or_default().commit = commit;
    }
}

pub fn read_head(p: &Project) -> Option<Head> {
    let v: Value = serde_json::from_str(&fs::read_to_string(p.head_path()).ok()?).ok()?;
    Some(Head {
        count: v["count"].as_u64()?,
        last_hash: v["last_hash"].as_str()?.to_string(),
        repos: {
            let mut m = std::collections::BTreeMap::new();
            if let Some(o) = v["repos"].as_object() {
                for (k, r) in o {
                    m.insert(
                        k.clone(),
                        RepoHead {
                            branch: r["branch"].as_str().map(|s| s.to_string()),
                            commit: r["commit"].as_str().map(|s| s.to_string()),
                        },
                    );
                }
            } else if v["main_commit"].is_string() || v["main_branch"].is_string() {
                // The older shape: one repository.
                m.insert(
                    crate::store::CLASSIC_REPO.to_string(),
                    RepoHead {
                        branch: v["main_branch"].as_str().map(|s| s.to_string()),
                        commit: v["main_commit"].as_str().map(|s| s.to_string()),
                    },
                );
            }
            m
        },
        policy_hash: v["policy_hash"].as_str().map(|s| s.to_string()),
        policy_at: v["policy_at"].as_u64(),
    })
}

pub fn write_head(p: &Project, h: &Head) -> R<()> {
    let repos: serde_json::Map<String, Value> = h
        .repos
        .iter()
        .map(|(k, r)| (k.clone(), json!({"branch": r.branch, "commit": r.commit})))
        .collect();
    let v = json!({"count": h.count, "last_hash": h.last_hash, "repos": repos,
        "policy_hash": h.policy_hash, "policy_at": h.policy_at});
    util::atomic_write(&p.head_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(())
}

pub fn load_log(p: &Project) -> R<Log> {
    let text = match fs::read(p.log_path()) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let file_len = text.len();
    let valid_len = text
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let body = String::from_utf8_lossy(&text[..valid_len]).to_string();
    let mut events = Vec::new();
    let mut prev = String::new();
    let key = p.read_key();
    let mut chain_ok = true;
    let mut chain_error = None;
    if key.is_none() && !body.trim().is_empty() {
        chain_ok = false;
        chain_error = Some("the state key is missing, so no line can be checked".to_string());
    }
    let key_s = key.clone().unwrap_or_default();
    for (i, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(m)) => {
                let want_seq = events.len() as u64 + 1;
                let ok_seq = m.get("seq").and_then(|v| v.as_u64()) == Some(want_seq);
                let ok_prev = m.get("prev").and_then(|v| v.as_str()) == Some(prev.as_str());
                let h = m
                    .get("h")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let ok_hash = key.is_some() && event_hash(&key_s, &prev, &m) == h;
                if chain_ok && !(ok_seq && ok_prev && ok_hash) {
                    chain_ok = false;
                    chain_error = Some(format!("line {} breaks the hash chain", i + 1));
                }
                prev = h;
                events.push(Value::Object(m));
            }
            _ => {
                if chain_ok {
                    chain_ok = false;
                    chain_error = Some(format!("line {} is not a JSON object", i + 1));
                }
            }
        }
    }
    // Compare with the head kept outside the project.
    let mut head_error = None;
    match read_head(p) {
        Some(h) => {
            if (events.len() as u64) < h.count {
                head_error = Some(format!(
                    "the log is shorter than it was: it has {} events and the state head says {}",
                    events.len(),
                    h.count
                ));
            } else if h.count > 0
                && events[h.count as usize - 1]["h"].as_str() != Some(h.last_hash.as_str())
            {
                head_error = Some(format!(
                    "the log diverges from the state head at event {}: another history has been put in its place",
                    h.count
                ));
            }
        }
        None => {
            if !events.is_empty() {
                head_error = Some("the state head is missing, so this log has not been seen by this machine's state folder".to_string());
            }
        }
    }
    Ok(Log {
        events,
        chain_ok,
        chain_error,
        head_error,
        last_hash: prev,
        valid_len,
        file_len,
    })
}

/// Upgrade path for `bf init` on an existing project: write the head if the state folder has none yet
/// and the log verifies under the current key.
pub fn ensure_head(p: &Project) -> R<()> {
    if read_head(p).is_some() {
        return Ok(());
    }
    let log = load_log(p)?;
    if log.chain_ok {
        write_head(
            p,
            &Head {
                count: log.events.len() as u64,
                last_hash: log.last_hash,
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

/// Append one event. Takes a file lock so two processes never share a sequence number.
/// Returns the stored event.
pub fn append(p: &Project, kind: &str, data: Value) -> R<Value> {
    fs::create_dir_all(p.dot_bf())?;
    fs::create_dir_all(p.log_path().parent().unwrap())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join("log.lock"))?;
    lock.lock()
        .map_err(|e| err(format!("cannot lock the log: {e}")))?;
    let key = p.read_key().ok_or_else(|| {
        refuse(
            "write to the run log",
            "the state key is missing; it lives in the project's state folder, outside the project",
            &["Run `bf init` in the project: it creates a key if none exists.", "Then the human runs `bf head accept` to take the log under the new key (old receipts must be re-run)."],
            "`bf doctor` reports `key: ok`.",
        )
    })?;
    let log = load_log(p)?;
    // Heal a torn last line left by a killed process.
    if log.file_len != log.valid_len {
        let f = fs::OpenOptions::new().write(true).open(p.log_path())?;
        f.set_len(log.valid_len as u64)?;
    }
    let mut m = match data {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    m.insert("seq".into(), json!(log.events.len() as u64 + 1));
    m.insert("at".into(), json!(util::now()));
    m.insert("kind".into(), json!(kind));
    m.insert("prev".into(), json!(log.last_hash));
    let h = event_hash(&key, &log.last_hash, &m);
    m.insert("h".into(), json!(h));
    let mut line = serde_json::to_string(&Value::Object(m.clone()))?;
    line.push('\n');
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p.log_path())?;
    f.write_all(line.as_bytes())?;
    f.flush()?;
    // The head moves with the log, but only while the two agree: a log that has been rolled back
    // must not be able to drag the head along with it.
    if log.chain_ok && log.head_error.is_none() {
        let old = read_head(p).unwrap_or_default();
        write_head(
            p,
            &Head {
                count: log.events.len() as u64 + 1,
                last_hash: h,
                repos: old.repos,
                policy_hash: old.policy_hash,
                policy_at: old.policy_at,
            },
        )?;
    }
    let _ = lock.unlock();
    Ok(Value::Object(m))
}

#[derive(Clone, Debug)]
pub struct Turn {
    pub seq: u64,
    pub text: String,
    pub at: String,
    pub used: bool,
    /// What the harness's transcript said about this prompt when it was recorded:
    /// `match`, `differs`, `unparsed` or `absent`. Information only.
    pub transcript: String,
}

#[derive(Clone, Debug)]
pub struct Gate {
    pub id: String,
    pub kind: String,
    pub refr: Option<String>,
    pub round: u32,
    pub open_seq: u64,
    pub open_at: String,
    pub status: String,
    pub simulated: bool,
    pub decided_seq: Option<u64>,
    pub turn: Option<u64>,
    pub note: String,
    /// The words of the human's message that were taken as the decision.
    pub quote: String,
    /// The whole message of the human that the decision rests on.
    pub message: String,
    /// A visible caution about the message (see `caution`); empty when there is none.
    pub caution: String,
    /// What the transcript said about the human's message when it was recorded (information only).
    pub transcript: String,
    pub snapshot: Value,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub start_seq: u64,
    pub start_at: String,
    pub test_mode: bool,
    pub ended: bool,
    pub simulated_used: bool,
}

#[derive(Clone, Debug, Default)]
pub struct UnitSt {
    pub state: String,
    pub rounds: u32,
    pub started_at: String,
    pub last_seq: u64,
    pub last_at: String,
    pub cost: f64,
    pub parked_reason: String,
    /// Failing check runs and blocked reports since the unit last went to review or was released.
    pub attempts: u32,
    /// The state before it was blocked or parked, so a release can put it back to work.
    pub prev_state: String,
    pub landed_hash: String,
    pub landed_run: String,
    /// The amendment that last sent this unit back to work.
    pub reopened_by: String,
    /// The commit of the checkpoint branch the unit's branch was cut from.
    pub base_commit: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Amend {
    pub id: String,
    pub ids: Vec<String>,
    pub impact: Value,
    pub keep: Vec<String>,
    pub waive: Vec<String>,
    pub reasons: Value,
    pub status: String,
}

/// A closed checkpoint: where it closed in the log, the content it closed on, and whether it was merged.
#[derive(Clone, Debug, Default)]
pub struct Closed {
    pub seq: u64,
    pub hash: String,
    pub merged: bool,
}

#[derive(Default)]
pub struct State {
    pub chain_ok: bool,
    pub chain_error: Option<String>,
    pub head_error: Option<String>,
    pub words_approve: Vec<String>,
    pub words_refuse: Vec<String>,
    pub event_count: usize,
    pub turns: Vec<Turn>,
    pub gates: Vec<Gate>,
    pub runs: Vec<Run>,
    pub units: BTreeMap<String, UnitSt>,
    pub amends: Vec<Amend>,
    pub pending_grants: Vec<u64>,
    pub closed: BTreeMap<String, Closed>,
    pub cost: f64,
    /// Check ID -> highest receipt number that no longer counts (an amendment re-opened the work).
    pub reopen_floor: BTreeMap<String, u64>,
    /// Events of the human's messages, recorded or rejected (`human.turn`, `human.turn.rejected`).
    pub human_events: usize,
    /// Where each page was published: name -> (url, when). The latest event for a name wins.
    pub pages: BTreeMap<String, (String, String)>,
    /// Events from a conductor at work: anything but the project's start, the human's messages and
    /// the acceptance of the policy or the log.
    pub work_events: usize,
    /// Design gates the human skipped: (kind, the words, seq). Only `ui` can be skipped.
    pub skipped: Vec<(String, String, u64)>,
    /// Events that were well-formed but not valid, e.g. a simulated approval outside a test run.
    pub ignored: Vec<String>,
    pub last_at: String,
}

impl State {
    pub fn active_run(&self) -> Option<&Run> {
        self.runs.iter().rev().find(|r| !r.ended)
    }
    pub fn gate(&self, id: &str) -> Option<&Gate> {
        self.gates.iter().find(|g| g.id == id)
    }
    pub fn latest_gate(&self, kind: &str, refr: Option<&str>) -> Option<&Gate> {
        self.gates
            .iter()
            .rev()
            .find(|g| g.kind == kind && g.refr.as_deref() == refr)
    }
    /// A gate kind counts as passed when its latest round is approved.
    /// The words the human gave to skip this gate, if they did.
    /// One line of information when the factory has been at work and not one message of the human has
    /// been recorded or rejected since the project started: the hook that records them is not loaded.
    pub fn human_turns_note(&self) -> Option<String> {
        if self.work_events > 0 && self.human_events == 0 {
            Some("no message of the human has been recorded since the project started, although the factory is at work: the hook that records them is not loaded. Start Claude Code in the folder that holds factory/ and .claude/settings.json (for a workspace, the workspace folder itself), then type a message.".to_string())
        } else {
            None
        }
    }
    pub fn skipped_words(&self, kind: &str) -> Option<&str> {
        self.skipped
            .iter()
            .find(|(k, _, _)| k == kind)
            .map(|(_, q, _)| q.as_str())
    }
    pub fn gate_passed(&self, kind: &str, refr: Option<&str>) -> bool {
        self.latest_gate(kind, refr)
            .map(|g| g.status == "approved")
            .unwrap_or(false)
    }
    pub fn unit_state(&self, id: &str) -> &str {
        self.units.get(id).map(|u| u.state.as_str()).unwrap_or("")
    }
    pub fn run_has_simulated(&self) -> bool {
        self.runs.iter().any(|r| r.simulated_used)
    }
    /// The newest human turn after `after_seq` that nothing has used yet.
    pub fn unused_turn_after(&self, after_seq: u64) -> Option<&Turn> {
        self.turns
            .iter()
            .rev()
            .find(|t| t.seq > after_seq && !t.used)
    }
}

pub fn replay(p: &Project) -> R<State> {
    let log = load_log(p)?;
    Ok(replay_log_with(
        &log,
        &Words {
            approve: p.policy.human.approval_words.clone(),
            refuse: p.policy.human.refusal_words.clone(),
        },
    ))
}

pub fn replay_log(log: &Log) -> State {
    let h = crate::store::HumanPol::default();
    replay_log_with(
        log,
        &Words {
            approve: h.approval_words,
            refuse: h.refusal_words,
        },
    )
}

/// A decision is valid only if its turn exists, is unused, comes in the right place, and the quote
/// and the message pass the decision rules.
fn decision_turn_ok(
    st: &State,
    ix: &HashMap<u64, usize>,
    tseq: u64,
    after: u64,
    before: u64,
    quote: &str,
    kind: Decide,
) -> bool {
    let Some(&i) = ix.get(&tseq) else {
        return false;
    };
    let t = &st.turns[i];
    let words = Words {
        approve: st.words_approve.clone(),
        refuse: st.words_refuse.clone(),
    };
    !t.used
        && t.seq > after
        && t.seq < before
        && check_decision(&t.text, quote, kind, &words).is_ok()
}

pub fn replay_log_with(log: &Log, words: &Words) -> State {
    let mut st = State {
        chain_ok: log.chain_ok,
        chain_error: log.chain_error.clone(),
        head_error: log.head_error.clone(),
        words_approve: words.approve.clone(),
        words_refuse: words.refuse.clone(),
        ..Default::default()
    };
    st.event_count = log.events.len();
    let mut turn_ix: HashMap<u64, usize> = HashMap::new();
    for ev in &log.events {
        let seq = ev["seq"].as_u64().unwrap_or(0);
        let at = ev["at"].as_str().unwrap_or("").to_string();
        let sv = |k: &str| ev.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        st.last_at = at.clone();
        if let Some(u) = ev.get("unit").and_then(|v| v.as_str()) {
            if let Some(us) = st.units.get_mut(u) {
                us.last_seq = seq;
                us.last_at = at.clone();
            }
        }
        match ev["kind"].as_str().unwrap_or("") {
            "human.turn" | "human.turn.rejected" => st.human_events += 1,
            "project.init" | "head.accept" | "policy.accept" => {}
            _ => st.work_events += 1,
        }
        match ev["kind"].as_str().unwrap_or("") {
            "human.turn" => {
                turn_ix.insert(seq, st.turns.len());
                // A turn recorded with a wrapper that marks text written by a program or another session
                // (an older build recorded some) can never decide anything.
                let machine = crate::gates::recorded_text_is_host(&sv("text"));
                st.turns.push(Turn {
                    seq,
                    text: sv("text"),
                    at,
                    used: machine,
                    transcript: sv("transcript"),
                });
            }
            "gate.open" => {
                st.gates.push(Gate {
                    id: sv("gate"),
                    kind: sv("gate_kind"),
                    refr: ev
                        .get("ref")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    round: ev["round"].as_u64().unwrap_or(1) as u32,
                    open_seq: seq,
                    open_at: at,
                    status: "open".into(),
                    simulated: false,
                    decided_seq: None,
                    turn: None,
                    note: String::new(),
                    quote: String::new(),
                    message: String::new(),
                    caution: String::new(),
                    transcript: String::new(),
                    snapshot: ev.get("snapshot").cloned().unwrap_or(Value::Null),
                });
            }
            "gate.approve" | "gate.reject" => {
                let approve = ev["kind"] == "gate.approve";
                let gid = sv("gate");
                let simulated = ev["simulated"].as_bool().unwrap_or(false);
                let Some(gi) = st.gates.iter().position(|g| g.id == gid) else {
                    st.ignored.push(format!("event {seq}: unknown gate {gid}"));
                    continue;
                };
                if st.gates[gi].status != "open" {
                    st.ignored
                        .push(format!("event {seq}: gate {gid} was already decided"));
                    continue;
                }
                if simulated {
                    // Valid only inside a granted test run that is still active, or, for the design
                    // gates, under a test-mode grant that is waiting for its run to start.
                    let rid = sv("run");
                    let design = DESIGN_GATES.contains(&st.gates[gi].kind.as_str());
                    let by_run = st
                        .runs
                        .iter()
                        .any(|r| r.id == rid && r.test_mode && !r.ended && r.start_seq < seq);
                    let by_grant = rid.is_empty()
                        && design
                        && ev
                            .get("grant")
                            .and_then(|v| v.as_u64())
                            .map(|g| st.pending_grants.contains(&g))
                            .unwrap_or(false);
                    if !(approve && (by_run || by_grant)) {
                        st.ignored.push(format!(
                            "event {seq}: simulated approval of {gid} outside a granted test run"
                        ));
                        continue;
                    }
                    if let Some(r) = st.runs.iter_mut().find(|r| r.id == rid) {
                        r.simulated_used = true;
                    }
                    let g = &mut st.gates[gi];
                    g.status = "approved".into();
                    g.simulated = true;
                    g.decided_seq = Some(seq);
                } else {
                    let tseq = ev["turn"].as_u64().unwrap_or(0);
                    let open_seq = st.gates[gi].open_seq;
                    if !decision_turn_ok(
                        &st,
                        &turn_ix,
                        tseq,
                        open_seq,
                        seq,
                        &sv("quote"),
                        if approve {
                            Decide::Approve
                        } else {
                            Decide::Reject
                        },
                    ) {
                        st.ignored.push(format!(
                            "event {seq}: {gid} decision cites no valid human turn and quote"
                        ));
                        continue;
                    }
                    st.turns[turn_ix[&tseq]].used = true;
                    let message = st.turns[turn_ix[&tseq]].text.clone();
                    let tcheck = st.turns[turn_ix[&tseq]].transcript.clone();
                    let g = &mut st.gates[gi];
                    g.message = message;
                    g.transcript = tcheck;
                    g.status = if approve {
                        "approved".into()
                    } else {
                        "rejected".into()
                    };
                    g.decided_seq = Some(seq);
                    g.turn = Some(tseq);
                    g.note = sv("note");
                    g.quote = sv("quote");
                    g.caution = caution(
                        &g.message,
                        if approve {
                            Decide::Approve
                        } else {
                            Decide::Reject
                        },
                        &Words {
                            approve: st.words_approve.clone(),
                            refuse: st.words_refuse.clone(),
                        },
                    );
                }
            }
            "testmode.grant" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Grant) {
                    st.ignored.push(format!(
                        "event {seq}: test-mode grant cites no valid human turn"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                match ev.get("run").and_then(|v| v.as_str()) {
                    Some(rid) => {
                        if let Some(r) = st.runs.iter_mut().find(|r| r.id == rid && !r.ended) {
                            r.test_mode = true;
                        }
                    }
                    None => st.pending_grants.push(seq),
                }
            }
            "run.start" => {
                let mut test_mode = false;
                if let Some(g) = ev.get("grant").and_then(|v| v.as_u64()) {
                    if let Some(i) = st.pending_grants.iter().position(|x| *x == g) {
                        st.pending_grants.remove(i);
                        test_mode = true;
                    }
                }
                st.runs.push(Run {
                    id: sv("run"),
                    start_seq: seq,
                    start_at: at,
                    test_mode,
                    ended: false,
                    simulated_used: false,
                });
            }
            "run.stop" => {
                let rid = sv("run");
                if let Some(r) = st.runs.iter_mut().find(|r| r.id == rid) {
                    r.ended = true;
                }
            }
            "unit.start" => {
                let u = st.units.entry(sv("unit")).or_default();
                u.state = "running".into();
                u.base_commit = sv("base_commit");
                u.started_at = at.clone();
                u.last_seq = seq;
                u.last_at = at;
            }
            "unit.report" => {
                let u = st.units.entry(sv("unit")).or_default();
                let to = sv("to");
                if matches!(to.as_str(), "blocked" | "parked") && u.state != to {
                    u.prev_state = u.state.clone();
                }
                u.state = to.clone();
                if ev["result"] == "findings" {
                    u.rounds += 1;
                }
                if ev["result"] == "blocked" {
                    u.attempts += 1;
                }
                if matches!(to.as_str(), "review" | "reviewed") {
                    u.attempts = 0;
                }
                let c = ev["cost"].as_f64().unwrap_or(0.0);
                u.cost += c;
                st.cost += c;
                let note = sv("note");
                if !note.is_empty() {
                    u.notes.push(note);
                }
                u.last_seq = seq;
                u.last_at = at;
            }
            "unit.land" => {
                let u = st.units.entry(sv("unit")).or_default();
                u.state = "landed".into();
                u.landed_hash = sv("hash");
                u.landed_run = sv("run");
                u.last_seq = seq;
                u.last_at = at;
            }
            "evidence.run" => {
                if let Some(u) = st.units.get_mut(&sv("unit")) {
                    if ev["exit"].as_i64().unwrap_or(0) != 0
                        && matches!(u.state.as_str(), "running" | "fixing")
                    {
                        u.attempts += 1;
                    }
                }
            }
            "page.published" => {
                if !sv("name").is_empty() && !sv("url").is_empty() {
                    st.pages.insert(sv("name"), (sv("url"), at.clone()));
                }
            }
            "gate.skip" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve)
                    || sv("gate_kind") != "ui"
                {
                    st.ignored.push(format!(
                        "event {seq}: a gate skip cites no valid human turn and quote"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                st.skipped.push((sv("gate_kind"), sv("quote"), seq));
            }
            "unit.release" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.ignored.push(format!(
                        "event {seq}: unit release cites no valid human turn and quote"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                if let Some(u) = st.units.get_mut(&sv("unit")) {
                    if matches!(u.state.as_str(), "parked" | "blocked") {
                        u.state = sv("to");
                        u.rounds = 0;
                        u.attempts = 0;
                        u.parked_reason.clear();
                        u.last_seq = seq;
                        u.last_at = at;
                    }
                }
            }
            "unit.park" => {
                let u = st.units.entry(sv("unit")).or_default();
                if u.state != "parked" {
                    u.prev_state = u.state.clone();
                }
                u.state = "parked".into();
                u.parked_reason = sv("reason");
                u.last_seq = seq;
                u.last_at = at;
            }
            "checkpoint.close" => {
                let k = sv("checkpoint");
                let merged = ev["merged"].as_bool().unwrap_or(false);
                let e = st.closed.entry(k).or_default();
                if e.seq == 0 {
                    e.seq = seq;
                    e.hash = sv("hash");
                }
                e.merged = e.merged || merged;
            }
            "amend.open" => st.amends.push(Amend {
                id: sv("amend"),
                ids: crate::store::str_list(ev, "ids"),
                impact: ev.get("impact").cloned().unwrap_or(Value::Null),
                keep: crate::store::str_list(ev, "impact_ids"),
                waive: vec![],
                reasons: Value::Null,
                status: "open".into(),
            }),
            "amend.narrow" => {
                let id = sv("amend");
                if let Some(a) = st
                    .amends
                    .iter_mut()
                    .find(|a| a.id == id && a.status != "approved")
                {
                    a.keep = crate::store::str_list(ev, "keep");
                    a.waive = crate::store::str_list(ev, "waive");
                    a.reasons = ev.get("reasons").cloned().unwrap_or(Value::Null);
                    a.status = "narrowed".into();
                }
            }
            "amend.approve" => {
                let id = sv("amend");
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.ignored.push(format!(
                        "event {seq}: amendment approval cites no valid human turn"
                    ));
                    continue;
                }
                if let Some(a) = st
                    .amends
                    .iter_mut()
                    .find(|a| a.id == id && a.status == "narrowed")
                {
                    a.status = "approved".into();
                    st.turns[turn_ix[&tseq]].used = true;
                    // The approved amendment re-opens what it touches: the units go back to work (what
                    // they landed stays in the log), the checkpoints open again, and the receipts for
                    // the changed checks that exist now stop counting.
                    for uid in crate::store::str_list(ev, "reopen_units") {
                        if let Some(u) = st.units.get_mut(&uid) {
                            if matches!(u.state.as_str(), "landed" | "reviewed" | "review") {
                                u.state = "running".into();
                            }
                            u.rounds = 0;
                            u.attempts = 0;
                            u.reopened_by = id.clone();
                            u.last_seq = seq;
                            u.last_at = at.clone();
                        }
                    }
                    for k in crate::store::str_list(ev, "reopen_checkpoints") {
                        st.closed.remove(&k);
                    }
                    let floor = ev["receipt_floor"].as_u64().unwrap_or(0);
                    for c in crate::store::str_list(ev, "reopen_checks") {
                        let e = st.reopen_floor.entry(c).or_insert(0);
                        *e = (*e).max(floor);
                    }
                }
            }
            "tokens.reset" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.turns[turn_ix[&tseq]].used = true;
                }
            }
            "policy.accept" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.turns[turn_ix[&tseq]].used = true;
                }
            }
            "head.accept" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.turns[turn_ix[&tseq]].used = true;
                }
            }
            _ => {}
        }
    }
    st
}

/// Refuse when the log's hash chain is broken.
pub fn require_chain(p: &Project) -> R<State> {
    let st = replay(p)?;
    if !st.chain_ok {
        return Err(refuse(
            "continue with a damaged run log",
            format!(
                "the hash chain of factory/log/events.jsonl is broken ({})",
                st.chain_error.clone().unwrap_or_default()
            ),
            &[
                "Do not edit, restore, stash or reset the log: that drops events. Copy the file aside first so nothing is lost.",
                "Tell the human what you saw. Only they decide what to do with a log that does not verify.",
                "If they say in chat to accept the log as it is, run `bf head accept --quote \"<their words>\"`; it takes the whole log under the current key.",
            ],
            "`bf log --verify` prints `chain ok`.",
        ));
    }
    if let Some(e) = &st.head_error {
        return Err(refuse(
            "continue with a log that does not match the state head",
            format!("{e}. A rollback (a stash, a checkout, an older copy put back) would undo gates, findings and parked units."),
            &[
                "Do not edit, restore, stash or reset the log. Copy `factory/log/events.jsonl` aside so nothing is lost.",
                "Tell the human what happened. If the older log is what they want, they say so in chat and you run `bf head accept --quote \"<their words>\"`.",
                "If it was not on purpose, the human puts the newer copy of the file back; the head then matches again.",
            ],
            "`bf status` prints the project and the log has the events it had before.",
        ));
    }
    Ok(st)
}

/// `bf head accept`: the human takes the log as it now stands. The whole log is chained again under
/// the current key and the head is set to its end. Receipts sealed with a key that is gone must be run again.
pub fn accept_head(p: &Project, quote: &str) -> R<Value> {
    let log = load_log(p)?;
    let st = replay_log(&log);
    let t = pick_turn(&st, 0, Decide::Approve, quote)?;
    let key = p.read_key().ok_or_else(|| {
        refuse(
            "accept the log",
            "the state key is missing",
            &[
                "Run `bf init`: it creates a key if none exists.",
                "Then run `bf head accept` again.",
            ],
            "`bf log --verify` prints `chain ok`.",
        )
    })?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join("log.lock"))?;
    lock.lock()
        .map_err(|e| err(format!("cannot lock the log: {e}")))?;
    let mut prev = String::new();
    let mut out = String::new();
    for (i, e) in log.events.iter().enumerate() {
        let mut m = e.as_object().cloned().unwrap_or_default();
        m.insert("seq".into(), json!(i as u64 + 1));
        m.insert("prev".into(), json!(prev));
        let h = event_hash(&key, &prev, &m);
        m.insert("h".into(), json!(h));
        prev = h;
        out.push_str(&serde_json::to_string(&Value::Object(m))?);
        out.push('\n');
    }
    util::atomic_write(&p.log_path(), out.as_bytes())?;
    let old = read_head(p).unwrap_or_default();
    // The human also accepts where each repository's main branch stands now, under the branch name
    // the config files give.
    let mut repos = old.repos.clone();
    for r in &p.repos {
        let e = repos.entry(r.name.clone()).or_default();
        e.branch = Some(r.configured.clone());
        if crate::gitx::branch_exists(&r.path, &r.configured) {
            if let Ok(c) = crate::gitx::must(&r.path, &["rev-parse", &r.configured]) {
                e.commit = Some(c);
            }
        }
    }
    write_head(
        p,
        &Head {
            count: log.events.len() as u64,
            last_hash: prev,
            repos,
            policy_hash: old.policy_hash.clone(),
            policy_at: old.policy_at,
        },
    )?;
    let _ = lock.unlock();
    let ev = append(
        p,
        "head.accept",
        json!({"turn": t.0, "quote": quote, "events": log.events.len(), "head_was": old.count, "problem": log.head_error.or(log.chain_error)}),
    )?;
    Ok(json!({"accepted": true, "events": log.events.len() + 1, "event": ev["seq"]}))
}

/// What a human act decides, which sets the words it must hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decide {
    Approve,
    Reject,
    /// A test-mode grant: an approval that also has to speak of a test run.
    Grant,
}

/// The policy's word lists, as the engine reads a message.
#[derive(Clone, Debug, Default)]
pub struct Words {
    pub approve: Vec<String>,
    pub refuse: Vec<String>,
}

fn tokens_of(text: &str) -> Vec<String> {
    text.replace('\u{2019}', "'")
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// The first listed word (or phrase) found in the text.
fn first_hit(text: &str, list: &[String]) -> Option<String> {
    let lower = text.replace('\u{2019}', "'").to_lowercase();
    let toks = tokens_of(text);
    for w in list {
        let w = w.to_lowercase();
        if w.contains(' ') {
            if lower.contains(&w) {
                return Some(w);
            }
        } else if toks.contains(&w) {
            return Some(w);
        }
    }
    toks.iter().find(|t| t.ends_with("n't")).cloned()
}

/// The sentences of a message, each trimmed, with its closing punctuation.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        cur.push(c);
        if matches!(c, '.' | '!' | '?' | '\n') {
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// A message that is only questions asks; it does not decide.
fn only_questions(text: &str) -> bool {
    let ss = sentences(text);
    !ss.is_empty() && ss.iter().all(|s| s.trim_end().ends_with('?'))
}

/// A visible caution, never a refusal: the message also holds a word that points the other way
/// ("the message also contains: not"). Shown beside the decision in `bf gate status` and on the gate
/// page, so the human can see what the conductor read as a decision.
pub fn caution(text: &str, kind: Decide, w: &Words) -> String {
    let (list, what) = match kind {
        Decide::Approve | Decide::Grant => (&w.refuse, "a word that can mean no"),
        Decide::Reject => (&w.approve, "a word that can mean yes"),
    };
    match first_hit(text, list) {
        Some(h) => format!("the message also contains {what}: \"{h}\""),
        None => String::new(),
    }
}

/// Is this message, with this quote, a decision of the given kind? The conductor names the words it
/// took as the decision; the engine checks only what it can check: the quote is in the human's
/// message, an approval is not a question, and a test-mode grant speaks of a test run. The quote and
/// the whole message are stored and shown, with a caution when the message holds a word pointing the
/// other way (see `caution`). The engine does not decide what the human meant.
pub fn check_decision(text: &str, quote: &str, kind: Decide, _w: &Words) -> Result<(), String> {
    let q = quote.trim();
    if q.is_empty() || !text.contains(q) {
        return Err("the quote is not in the human's message".into());
    }
    match kind {
        Decide::Approve | Decide::Grant => {
            if q.ends_with('?') {
                return Err("a question is not an approval".into());
            }
            if only_questions(text) {
                return Err("a message that only asks questions is not an approval".into());
            }
            if kind == Decide::Grant {
                let lower = text.to_lowercase();
                if !(lower.contains("test") || lower.contains("simulat")) {
                    return Err(
                        "a test-mode grant must speak of a test run: the message does not".into(),
                    );
                }
            }
            Ok(())
        }
        Decide::Reject => Ok(()),
    }
}

/// Choose the human turn that decides something. It must come after `after_seq`, be unused and hold
/// the quote; the decision rules above apply. Returns (seq, text).
pub fn pick_turn(st: &State, after_seq: u64, kind: Decide, quote: &str) -> R<(u64, String)> {
    let q = quote.trim();
    if q.is_empty() {
        return Err(refuse(
            "record a decision without the words that decide it",
            "no quote was given; the engine cannot understand a sentence, so the words taken as the decision are stored and shown for the human to audit",
            &["Run the command again with `--quote \"<the words of the human's message that you took as the decision>\"`."],
            "`bf gate status` shows the quote beside the decision.",
        ));
    }
    let mut candidates = st
        .turns
        .iter()
        .rev()
        .filter(|t| t.seq > after_seq && !t.used)
        .peekable();
    if candidates.peek().is_none() {
        let cause = st.human_turns_note();
        let first = cause.clone().unwrap_or_else(|| {
            "Ask the human in chat for their decision, in their own words.".to_string()
        });
        return Err(refuse(
            "record a decision",
            if cause.is_some() {
                "no human turn was recorded at all: the hook that records the human's messages is not loaded"
            } else {
                "no human turn was recorded after this opened that can decide it: the human has not written since, or the message was already used for another decision"
            },
            &[
                &first,
                "Then ask the human in chat for their decision, in their own words.",
                "The harness hook records their message with `bf human-turn`; check with `bf log --tail 5`.",
                "Then run the command again with `--quote` set to the sentence that decides it.",
            ],
            "`bf gate status` shows the decision with its quote.",
        ));
    }
    let Some(t) = candidates.find(|t| t.text.contains(q)) else {
        let last = st
            .turns
            .iter()
            .rev()
            .find(|t| t.seq > after_seq && !t.used)
            .map(|t| t.text.clone())
            .unwrap_or_default();
        return Err(refuse(
            "record a decision with this quote",
            format!("the quote is not in the human's message, which says: \"{}\"", last.chars().take(300).collect::<String>()),
            &["Copy the words that decide it from that message exactly, and run the command again with `--quote`."],
            "`bf gate status` shows the decision with its quote.",
        ));
    };
    let words = Words {
        approve: st.words_approve.clone(),
        refuse: st.words_refuse.clone(),
    };
    if let Err(why) = check_decision(&t.text, q, kind, &words) {
        return Err(refuse(
            "record a decision from this message",
            format!("{why}. The human's message says: \"{}\"", t.text.chars().take(300).collect::<String>()),
            &[
                "Do not guess what the human meant. Ask them in chat for their decision in their own words.",
                "Then run the command again with `--quote` set to the words that decide it.",
            ],
            "`bf gate status` shows the decision with its quote and the human's whole message.",
        ));
    }
    Ok((t.seq, t.text.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, prev: &str, kind: &str, extra: Value) -> Value {
        let mut m = extra.as_object().cloned().unwrap_or_default();
        m.insert("seq".into(), json!(seq));
        m.insert("at".into(), json!("2026-01-01T00:00:00Z"));
        m.insert("kind".into(), json!(kind));
        m.insert("prev".into(), json!(prev));
        let h = event_hash("k", prev, &m);
        m.insert("h".into(), json!(h));
        Value::Object(m)
    }

    fn log_of(items: &[(&str, Value)]) -> Log {
        let mut events = Vec::new();
        let mut prev = String::new();
        for (i, (k, d)) in items.iter().enumerate() {
            let e = ev(i as u64 + 1, &prev, k, d.clone());
            prev = e["h"].as_str().unwrap().to_string();
            events.push(e);
        }
        Log {
            events,
            chain_ok: true,
            chain_error: None,
            head_error: None,
            last_hash: prev,
            valid_len: 0,
            file_len: 0,
        }
    }

    #[test]
    fn simulated_approval_outside_test_run_counts_for_nothing() {
        let log = log_of(&[
            (
                "gate.open",
                json!({"gate":"plan-r1","gate_kind":"plan","round":1}),
            ),
            ("run.start", json!({"run":"run-1"})),
            (
                "gate.approve",
                json!({"gate":"plan-r1","simulated":true,"run":"run-1"}),
            ),
        ]);
        let st = replay_log(&log);
        assert_eq!(st.gate("plan-r1").unwrap().status, "open");
        assert_eq!(st.ignored.len(), 1);
        assert!(!st.gate_passed("plan", None));
    }

    #[test]
    fn real_approval_needs_a_later_unused_turn() {
        let log = log_of(&[
            ("human.turn", json!({"text":"early"})),
            (
                "gate.open",
                json!({"gate":"plan-r1","gate_kind":"plan","round":1}),
            ),
            (
                "gate.approve",
                json!({"gate":"plan-r1","turn":1,"quote":"early"}),
            ),
            ("human.turn", json!({"text":"approve"})),
            (
                "gate.approve",
                json!({"gate":"plan-r1","turn":4,"quote":"approve"}),
            ),
        ]);
        let st = replay_log(&log);
        assert_eq!(st.gate("plan-r1").unwrap().status, "approved");
        assert_eq!(st.ignored.len(), 1, "{:?}", st.ignored);
        assert_eq!(st.gate("plan-r1").unwrap().turn, Some(4));
    }

    #[test]
    fn a_turn_approves_only_once() {
        let log = log_of(&[
            (
                "gate.open",
                json!({"gate":"a-r1","gate_kind":"a","round":1}),
            ),
            (
                "gate.open",
                json!({"gate":"b-r1","gate_kind":"b","round":1}),
            ),
            ("human.turn", json!({"text":"approve"})),
            (
                "gate.approve",
                json!({"gate":"a-r1","turn":3,"quote":"approve"}),
            ),
            (
                "gate.approve",
                json!({"gate":"b-r1","turn":3,"quote":"approve"}),
            ),
        ]);
        let st = replay_log(&log);
        assert!(st.gate_passed("a", None) && !st.gate_passed("b", None));
    }

    #[test]
    fn grant_marks_run_as_test_mode() {
        let log = log_of(&[
            ("human.turn", json!({"text":"yes, test run"})),
            ("testmode.grant", json!({"turn":1,"quote":"yes, test run"})),
            ("run.start", json!({"run":"run-1","grant":2})),
            (
                "gate.open",
                json!({"gate":"plan-r1","gate_kind":"plan","round":1}),
            ),
            (
                "gate.approve",
                json!({"gate":"plan-r1","simulated":true,"run":"run-1"}),
            ),
        ]);
        let st = replay_log(&log);
        assert!(st.gate_passed("plan", None));
        assert!(st.run_has_simulated());
    }
}
