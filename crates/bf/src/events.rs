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

/// Record where repositories' main branches stand in the head, under the log's lock, so an append
/// at the same moment cannot write an old value back. Each pair is a repository name and a commit.
pub fn set_commits(p: &Project, commits: &[(String, String)]) -> R<()> {
    update_head(p, |h| {
        for (repo, commit) in commits {
            h.set_commit(repo, Some(commit.clone()));
        }
        true
    })
}

/// Change the head under the log's lock: read it, let `f` change it, and write it back when `f`
/// answers true. Any head change that is not an append goes through here, so an append at the same
/// moment cannot put an old value back.
pub fn update_head(p: &Project, f: impl FnOnce(&mut Head) -> bool) -> R<()> {
    fs::create_dir_all(p.dot_bf())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join("log.lock"))?;
    lock.lock()
        .map_err(|e| err(format!("cannot lock the log: {e}")))?;
    let mut h = read_head(p).unwrap_or_default();
    let r = if f(&mut h) { write_head(p, &h) } else { Ok(()) };
    let _ = lock.unlock();
    r
}

/// Read the log and compare it with the head. A shared hold on the log lock keeps an append from
/// finishing between the two reads, so a reader never sees a head one event ahead of the log it read.
pub fn load_log(p: &Project) -> R<Log> {
    let guard = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join("log.lock"))
        .ok();
    if let Some(g) = &guard {
        let _ = g.lock_shared();
    }
    let r = load_log_unlocked(p);
    if let Some(g) = &guard {
        let _ = g.unlock();
    }
    r
}

fn load_log_unlocked(p: &Project) -> R<Log> {
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
    let log = load_log_unlocked(p)?;
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
    drop(lock);
    // After an event that can start or end a hold, the engine logs the change (never after a hold event).
    if can_change_hold(kind) {
        let _ = crate::run::observe_holds(p);
    }
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
    /// The checks switched off for this run, with the reason for each: the project default when the
    /// run started, then what the human confirmed for this run.
    pub checks_off: BTreeMap<String, String>,
    /// The wave checks this run uses, as the human confirmed them: `{source, checks}` where `source` is
    /// `default`, `override` or `none`. None until the human decides at the plan stage.
    pub wave: Option<Value>,
    /// The checks every unit must pass in this run: `{mode, checks}` where `mode` is `project`,
    /// `override` or `none`. None until the human decides at the plan stage.
    pub unit_checks: Option<Value>,
}

/// A proposal about the wave checks or the checks every unit must pass, shown to the human before they
/// are asked. `topic` is `wave` or `unitchecks`.
#[derive(Clone, Debug)]
pub struct SettingProposal {
    pub seq: u64,
    pub topic: String,
    /// `run`, `default` (wave checks only) or `both`.
    pub scope: String,
    pub value: Value,
    pub run: String,
    pub used: bool,
}

/// A wave check run: for a checkpoint and a wave, the content it ran on and how it ended.
#[derive(Clone, Debug, Default)]
pub struct WaveRun {
    pub checkpoint: String,
    pub wave: u32,
    pub content: String,
    /// `running`, `passed`, `failed` or `stale` (units landed while it ran, so it counts for nothing).
    pub state: String,
    pub failed: Vec<Value>,
    pub start_seq: u64,
    pub start_at: String,
    pub receipts: Vec<String>,
}

/// One change of the check settings: a check name and, when it is switched off, the reason.
/// `None` switches the check on.
pub type CheckChange = (String, Option<String>);

/// A proposal of check settings that was shown to the human before they were asked.
#[derive(Clone, Debug)]
pub struct CheckProposal {
    pub seq: u64,
    /// `run`, `default` or `both`.
    pub scope: String,
    pub changes: Vec<CheckChange>,
    /// The run that was active when the proposal was made; empty when none.
    pub run: String,
    /// True once a setting recorded it: a proposal is confirmed only once.
    pub used: bool,
}

pub fn changes_to_json(c: &[CheckChange]) -> Value {
    Value::Array(
        c.iter()
            .map(|(n, r)| json!({"check": n, "off": r}))
            .collect(),
    )
}

pub fn changes_from_json(v: &Value) -> Vec<CheckChange> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    (
                        x["check"].as_str().unwrap_or("").to_string(),
                        x["off"].as_str().map(|s| s.to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn apply_changes(off: &mut BTreeMap<String, String>, changes: &[CheckChange]) {
    for (n, r) in changes {
        if crate::checks::by_name(n).is_none() {
            continue;
        }
        match r {
            Some(r) if !r.trim().is_empty() => {
                off.insert(n.clone(), r.clone());
            }
            Some(_) => {}
            None => {
                off.remove(n);
            }
        }
    }
}

/// A grant of files beyond a unit's list, as the log has it (R-84).
#[derive(Clone, Debug, Default)]
pub struct ScopeGrant {
    pub n: u32,
    pub seq: u64,
    pub at: String,
    /// `{path, why, patch}` for each file.
    pub files: Vec<Value>,
    /// `{path, mark}` for each marked file.
    pub marks: Vec<Value>,
    /// The log position of the request this grant answers.
    pub request: u64,
    /// The human's words and the time, when the grant was withdrawn.
    pub withdrawn: Option<(String, String)>,
    /// Whether the unit was sent back to undo changes the grant allowed.
    pub undo: bool,
}

/// An open request of a builder for files beyond its unit's list.
#[derive(Clone, Debug, Default)]
pub struct ScopeReq {
    pub seq: u64,
    pub at: String,
    pub from: String,
    pub files: Vec<Value>,
}

/// Everything the log says about a unit's files beyond its list.
#[derive(Clone, Debug, Default)]
pub struct ScopeSt {
    pub open: Option<ScopeReq>,
    pub grants: Vec<ScopeGrant>,
    /// Requests refused as an extension or by a hard rule: `{seq, at, refusal, files}`.
    pub refused: Vec<Value>,
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
    /// Every review round: number, the content it judged, its commit and what it covered.
    pub review_rounds: Vec<Value>,
}

/// One repository of an open sync: where the checkpoint branch stood before, the main branch's commit
/// being taken in, and the files left in conflict (none when the merge was clean).
#[derive(Clone, Debug)]
pub struct SyncRepo {
    pub repo: String,
    pub main: String,
    pub before: String,
    pub files: Vec<String>,
    pub merge: String,
    /// Patterns of the files the resolver may change here: what either side changed since the merge
    /// base, and new test files beside them.
    pub allowed: Vec<String>,
}

/// A sync of the main branch into a checkpoint that stopped at a conflict, while agents resolve it.
#[derive(Clone, Debug)]
pub struct SyncOpen {
    pub checkpoint: String,
    pub repos: Vec<SyncRepo>,
    /// `resolve` (a builder works), `review` (a reviewer judges) or `parked` (the human decides).
    pub phase: String,
    pub rounds: u32,
    pub reason: String,
    pub findings: Vec<Value>,
    pub open_seq: u64,
    /// The newest receipt on file when the sync started: receipts after it count for the joined content.
    pub floor: u64,
    /// The checkpoint checks that failed on the joined content when the sync opened.
    pub failed: Vec<Value>,
}

/// A checkpoint fix that was aborted.
#[derive(Clone, Debug, Default)]
pub struct CkFixAbort {
    pub checkpoint: String,
    /// The (round, number) of each finding the aborted fix was opened for.
    pub ids: Vec<(u32, u32)>,
    pub seq: u64,
}

/// A fix of a checkpoint that failed a check or drew findings: a builder works in the checkpoint's own
/// working copy, a reviewer judges it, and the checkpoint's checks run again.
#[derive(Clone, Debug, Default)]
pub struct CkFixOpen {
    pub checkpoint: String,
    /// `check` (a check failed) or `review` (the checkpoint review filed findings).
    pub source: String,
    /// `fix` (a builder works), `review` (a reviewer judges), `parked` or `gap` (the human decides).
    pub phase: String,
    /// Review rounds that ended in findings.
    pub rounds: u32,
    /// Failing check runs and blocked reports since the fix opened or was released.
    pub attempts: u32,
    pub reason: String,
    /// What is missing, when the fix is a plan gap.
    pub missing: Vec<String>,
    /// The findings the builder has to fix now.
    pub findings: Vec<Value>,
    /// The failing checks the builder has to fix now: check, command, exit, tail.
    pub failed: Vec<Value>,
    /// Patterns of the files the fix may change, and the test folders where it may add new test files.
    pub allowed: Vec<String>,
    pub test_dirs: Vec<String>,
    pub units: Vec<String>,
    /// Where each repository's checkpoint branch stood when the fix opened: (repository, commit).
    pub before: Vec<(String, String)>,
    /// The content of the checkpoint when the fix opened.
    pub content: String,
    pub open_seq: u64,
    /// The seq of the latest event that parked the fix; 0 while it was never parked.
    pub park_seq: u64,
    /// What set the fix off, as the opening event recorded it: the failed checks, or the findings.
    pub origin_failed: Vec<Value>,
    pub origin_findings: Vec<Value>,
    /// The round and the number inside the round (from 1) of each finding the fix was opened for. Empty for
    /// a fix opened before the pairs were recorded, and for a fix that no review finding opened.
    pub origin_ids: Vec<(u32, u32)>,
    /// Every review round: number, the content it judged, the commits, what it covered, its outcome.
    pub rounds_log: Vec<Value>,
    /// The findings of every earlier round, with the round they were filed in.
    pub history: Vec<Value>,
    /// The wave a wave fix is for; 0 for every other fix.
    pub wave: u32,
    /// The opening event cites a human turn of its own (a pick with words).
    pub own_turn: bool,
}

/// A hold the engine logged for a checkpoint: `marked`, `question` or `wave`, what it names, and the seq
/// of the event that started it.
#[derive(Debug, Clone, Default)]
pub struct CkHold {
    pub cause: String,
    pub detail: String,
    pub seq: u64,
}

/// A reason the human is not being asked about a checkpoint's findings. Each carries what a refusal names.
#[derive(Debug, Clone, PartialEq)]
pub enum Hold {
    /// A sync of main into this checkpoint is open (any phase, parked included).
    Sync { checkpoint: String },
    /// A checkpoint fix of this checkpoint is at work (open and not parked, the gap phase included).
    Fix { checkpoint: String },
    /// These checks of the checkpoint are marked failed or unreachable at its content.
    Marked {
        checkpoint: String,
        checks: Vec<String>,
    },
    /// A worker's question on the checkpoint is unanswered.
    Question { checkpoint: String, id: String },
    /// A wave check holds the checkpoint's close.
    Wave { checkpoint: String, reason: String },
}

impl Hold {
    /// The checkpoint the hold is of (for a sync or a fix, that of the sync or the fix).
    pub fn checkpoint(&self) -> &str {
        match self {
            Hold::Sync { checkpoint }
            | Hold::Fix { checkpoint }
            | Hold::Marked { checkpoint, .. }
            | Hold::Question { checkpoint, .. }
            | Hold::Wave { checkpoint, .. } => checkpoint,
        }
    }
    /// The hold in words, for a refusal ("because ...").
    pub fn because(&self) -> String {
        match self {
            Hold::Sync { checkpoint } => {
                format!("a sync of main into {checkpoint} is open")
            }
            Hold::Fix { checkpoint } => {
                format!("a checkpoint fix is at work on {checkpoint}")
            }
            Hold::Marked { checkpoint, checks } => format!(
                "a check of {checkpoint} is marked failed or unreachable ({})",
                checks.join(", ")
            ),
            Hold::Question { checkpoint, id } => {
                format!("a worker's question on {checkpoint} ({id}) is unanswered")
            }
            Hold::Wave { checkpoint, reason } => {
                format!("a wave check holds the close of {checkpoint} ({reason})")
            }
        }
    }
    /// The hold in words of the past, for the note of an ignored event.
    pub fn during(&self) -> String {
        match self {
            Hold::Sync { checkpoint } => format!("a sync of main into {checkpoint} was open"),
            Hold::Fix { .. } => "a checkpoint fix was at work".to_string(),
            Hold::Marked { checkpoint, checks } => format!(
                "a check of {checkpoint} was marked failed or unreachable ({})",
                checks.join(", ")
            ),
            Hold::Question { checkpoint, id } => {
                format!("a worker's question on {checkpoint} ({id}) was unanswered")
            }
            Hold::Wave { checkpoint, reason } => {
                format!("a wave check held the close of {checkpoint} ({reason})")
            }
        }
    }
}

/// A finding of a checkpoint review that the human accepted, or declined with a reason.
#[derive(Clone, Debug, Default)]
pub struct CkSettled {
    pub checkpoint: String,
    pub round: u32,
    /// The finding's number inside its round, from 1.
    pub index: u32,
    /// `accepted` or `declined`.
    pub fate: String,
    pub reason: String,
}

/// A checkpoint fix that was reviewed clean and whose checks passed again.
#[derive(Clone, Debug, Default)]
pub struct CkFixDone {
    pub id: String,
    pub checkpoint: String,
    pub source: String,
    pub files: Vec<String>,
    pub units: Vec<String>,
    /// File -> the units whose file lists hold it.
    pub by_file: Value,
    pub rounds_log: Vec<Value>,
    pub origin_failed: Vec<Value>,
    pub origin_findings: Vec<Value>,
    /// The (round, number) pairs the fix was opened for; empty for a fix recorded before they were kept.
    pub ids: Vec<(u32, u32)>,
    /// The latest review round of the checkpoint that had reported when the fix opened.
    pub from_round: u32,
    pub seq: u64,
}

#[derive(Clone, Debug)]
pub struct Amend {
    pub id: String,
    pub ids: Vec<String>,
    pub impact: Value,
    pub keep: Vec<String>,
    pub waive: Vec<String>,
    pub reasons: Value,
    /// `open`, `narrowed`, `approved` or `withdrawn`.
    pub status: String,
    /// For a withdrawn amendment: why.
    pub withdrawn_why: String,
    /// The log position of its latest narrowing or revision: an approval needs a human message after it.
    pub changed_seq: u64,
}

/// A closed checkpoint: where it closed in the log, the content it closed on, and whether it was merged.
#[derive(Clone, Debug, Default)]
pub struct Closed {
    pub seq: u64,
    pub hash: String,
    pub merged: bool,
    /// Whether the close event recorded which checks were off and which IDs they left unproven.
    /// A close from before that was recorded is judged by the settings of the latest run.
    pub recorded: bool,
    /// The run the checkpoint closed in.
    pub run: String,
    /// The names of the checks that were switched off when the checkpoint closed.
    pub off: Vec<String>,
    /// The IDs left unproven at the close because every check that proves them was off, with those names.
    pub waived: BTreeMap<String, Vec<String>>,
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
    /// Legal records the human accepted: (record, the words, seq). Each used up one human message.
    pub legal_accepts: Vec<(String, String, u64)>,
    /// Where each checkpoint branch took the main branch in: (checkpoint, repository, main's commit).
    pub synced: Vec<(String, String, String)>,
    /// A sync that stopped at a conflict and is being resolved.
    pub sync_open: Option<SyncOpen>,
    /// A checkpoint fix being worked on.
    pub ck_fix: Option<CkFixOpen>,
    /// Every checkpoint fix that was finished, in log order.
    pub ck_fixes: Vec<CkFixDone>,
    /// `<checkpoint>@<content>` of fixes the human turned down: no fix opens for that content again.
    pub ck_declined: Vec<String>,
    /// The seq of the latest event that ended, aborted or parked a checkpoint fix, of any checkpoint; 0 when none.
    pub ck_end_seq: u64,
    /// Checkpoint -> the hold the engine logged for it (a marked check, an unanswered question, a wave
    /// check), with the seq it started at. Kept by `checkpoint.hold`.
    pub ck_holds: BTreeMap<String, CkHold>,
    /// Findings of checkpoint reviews that the human accepted or declined, in log order.
    pub ck_settled: Vec<CkSettled>,
    /// Every checkpoint review round that reported: (checkpoint, round, seq of the report).
    pub ck_round_done: Vec<(String, u32, u64)>,
    /// Every checkpoint fix that was aborted: (checkpoint, the findings it was opened for as (round, number),
    /// seq of the abort).
    pub ck_fix_aborts: Vec<CkFixAbort>,
    /// Checkpoint ->the review round the human released it at: reviews before it no longer count towards
    /// the number of reviews a checkpoint may have.
    pub ck_review_floor: BTreeMap<String, u32>,
    /// Events that were well-formed but not valid, e.g. a simulated approval outside a test run.
    pub ignored: Vec<String>,
    pub last_at: String,
    /// The project default: checks switched off, with the reason for each.
    pub checks_default: BTreeMap<String, String>,
    /// Proposals of check settings shown to the human, in log order.
    pub checks_proposals: Vec<CheckProposal>,
    /// The project's default wave checks, kept by the engine: `{enabled, checks}`. None when the project
    /// has no default.
    pub wave_default: Option<Value>,
    /// Proposals about the wave checks and the checks every unit must pass, in log order.
    pub setting_proposals: Vec<SettingProposal>,
    /// The latest wave check run of each checkpoint and wave.
    pub wave_runs: BTreeMap<(String, u32), WaveRun>,
    /// Unit -> its open request, its grants and its refused requests.
    pub scope: BTreeMap<String, ScopeSt>,
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

fn id_pairs(v: &Value) -> Vec<(u32, u32)> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            (
                x["round"].as_u64().unwrap_or(0) as u32,
                x["index"].as_u64().unwrap_or(0) as u32,
            )
        })
        .collect()
}

/// The seq of the report of a checkpoint's review round (the latest one, when it reported more than once);
/// 0 when none.
pub fn round_report_seq(st: &State, k: &str, round: u32) -> u64 {
    st.ck_round_done
        .iter()
        .filter(|(c, r, _)| c == k && *r == round)
        .map(|(_, _, q)| *q)
        .max()
        .unwrap_or(0)
}

/// The seq of the latest event that put these findings (round, number) of a checkpoint to the human: for
/// each, the later of the report of its round and the abort of the latest fix opened for it. A fix recorded
/// without finding ids is not matched to its abort. The latest over all the findings; 0 when none.
pub fn put_to_human_seq(st: &State, k: &str, ids: &[(u32, u32)]) -> u64 {
    ids.iter()
        .map(|id| {
            let abort = st
                .ck_fix_aborts
                .iter()
                .filter(|a| a.checkpoint == k && a.ids.contains(id))
                .map(|a| a.seq)
                .max()
                .unwrap_or(0);
            abort.max(round_report_seq(st, k, id.0))
        })
        .max()
        .unwrap_or(0)
}

/// The one bound of the human decisions on a checkpoint finding (accept, decline, a pick to fix, the release
/// of a parked review, the release of a parked fix). `None` while a checkpoint fix opened before
/// `opened_before` is at work (open and not parked, the gap phase counts as at work), whatever it was opened
/// for and whichever checkpoint it is of: then the human is not asked. Otherwise the later of `base` and the
/// latest end, abort or park of a checkpoint fix. Commands pass `u64::MAX` as `opened_before`; the replay
/// passes the seq of the event it judges. One rule for the command and for the replay.
pub fn decision_bound(st: &State, k: &str, base: u64, opened_before: u64) -> Result<u64, Hold> {
    match hold(st, k, opened_before) {
        Some(h) => Err(h),
        None => Ok(base.max(st.ck_end_seq)),
    }
}

/// The one predicate for "the human is not being asked about the findings of checkpoint `k`": a sync of main
/// into any checkpoint is open (every phase); any checkpoint fix opened before `opened_before` is at work
/// (open and not parked, the gap phase included); or the engine logged a hold of `k` (a check marked, a
/// worker's question unanswered, a wave check holding the close). `bf next` and the five decisions share it.
pub fn hold(st: &State, k: &str, opened_before: u64) -> Option<Hold> {
    if let Some(so) = st.sync_open.as_ref() {
        return Some(Hold::Sync {
            checkpoint: so.checkpoint.clone(),
        });
    }
    if let Some(f) = st.ck_fix.as_ref() {
        if f.open_seq < opened_before && f.phase != "parked" {
            return Some(Hold::Fix {
                checkpoint: f.checkpoint.clone(),
            });
        }
    }
    let h = st.ck_holds.get(k)?;
    let checkpoint = k.to_string();
    match h.cause.as_str() {
        "marked" => Some(Hold::Marked {
            checkpoint,
            checks: h
                .detail
                .split(',')
                .filter(|c| !c.is_empty())
                .map(|c| c.to_string())
                .collect(),
        }),
        "question" => Some(Hold::Question {
            checkpoint,
            id: h.detail.clone(),
        }),
        "wave" => Some(Hold::Wave {
            checkpoint,
            reason: h.detail.clone(),
        }),
        _ => None,
    }
}

/// The `opened_before` of a review release's replay: the fix the release itself opened is the event just
/// before it, for the same checkpoint, with source `review` and no human turn of its own; only that fix is
/// set aside. Any other open fix counts.
fn release_opened_before(st: &State, k: &str, seq: u64) -> u64 {
    match st.ck_fix.as_ref() {
        Some(f)
            if f.open_seq + 1 == seq
                && f.checkpoint == k
                && f.source == "review"
                && !f.own_turn =>
        {
            seq.saturating_sub(1)
        }
        _ => seq,
    }
}

/// Whether an event of this kind can start or end a hold: the engine looks at the holds after it.
pub fn can_change_hold(kind: &str) -> bool {
    matches!(
        kind,
        "evidence.run"
            | "evidence.outcome"
            | "question.asked"
            | "question.answered"
            | "wave.check.run"
            | "unit.land"
            | "checkpoint.fix.done"
            | "checkpoint.sync"
            | "amend.approve"
    )
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
                    checks_off: st.checks_default.clone(),
                    wave: None,
                    unit_checks: None,
                });
            }
            "wave.propose" | "unitchecks.propose" => {
                st.setting_proposals.push(SettingProposal {
                    seq,
                    topic: if ev["kind"] == "wave.propose" {
                        "wave"
                    } else {
                        "unitchecks"
                    }
                    .into(),
                    scope: sv("scope"),
                    value: ev["value"].clone(),
                    run: st
                        .runs
                        .iter()
                        .rev()
                        .find(|r| !r.ended)
                        .map(|r| r.id.clone())
                        .unwrap_or_default(),
                    used: false,
                });
            }
            "wave.set" | "unitchecks.set" => {
                let topic = if ev["kind"] == "wave.set" {
                    "wave"
                } else {
                    "unitchecks"
                };
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                let pseq = ev["proposal"].as_u64().unwrap_or(0);
                let active = st
                    .runs
                    .iter()
                    .rev()
                    .find(|r| !r.ended)
                    .map(|r| r.id.clone())
                    .unwrap_or_default();
                let prop = st.setting_proposals.iter().find(|x| x.seq == pseq).cloned();
                let ok = prop
                    .as_ref()
                    .map(|x| {
                        x.topic == topic
                            && x.scope == sv("scope")
                            && x.value == ev["value"]
                            && !x.used
                            && !st
                                .setting_proposals
                                .iter()
                                .any(|y| y.topic == topic && y.seq > x.seq && y.seq < tseq)
                            && (x.scope == "default" || (!active.is_empty() && x.run == active))
                    })
                    .unwrap_or(false)
                    && decision_turn_ok(
                        &st,
                        &turn_ix,
                        tseq,
                        pseq,
                        seq,
                        &sv("quote"),
                        Decide::Approve,
                    );
                if !ok {
                    st.ignored.push(format!(
                        "event {seq}: a {topic} setting cites no matching proposal or no valid human turn and quote after it"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                if let Some(x) = st.setting_proposals.iter_mut().find(|x| x.seq == pseq) {
                    x.used = true;
                }
                let scope = sv("scope");
                let value = ev["value"].clone();
                if topic == "wave" {
                    if scope == "default" || scope == "both" {
                        st.wave_default = Some(json!({
                            "enabled": value["source"] != "none",
                            "checks": value["checks"],
                        }));
                    }
                    if scope == "run" || scope == "both" {
                        if let Some(r) = st.runs.iter_mut().rev().find(|r| !r.ended) {
                            r.wave = Some(value);
                        }
                    }
                } else if let Some(r) = st.runs.iter_mut().rev().find(|r| !r.ended) {
                    r.unit_checks = Some(value);
                }
            }
            "wave.check.start" => {
                let key = (sv("checkpoint"), ev["wave"].as_u64().unwrap_or(0) as u32);
                st.wave_runs.insert(
                    key.clone(),
                    WaveRun {
                        checkpoint: key.0,
                        wave: key.1,
                        content: sv("content"),
                        state: "running".into(),
                        start_seq: seq,
                        start_at: at.clone(),
                        ..Default::default()
                    },
                );
            }
            "wave.check.run" => {
                let key = (sv("checkpoint"), ev["wave"].as_u64().unwrap_or(0) as u32);
                let w = st.wave_runs.entry(key.clone()).or_insert_with(|| WaveRun {
                    checkpoint: key.0.clone(),
                    wave: key.1,
                    ..Default::default()
                });
                w.content = sv("content");
                w.state = sv("state");
                w.failed = ev["failed"].as_array().cloned().unwrap_or_default();
                w.receipts = crate::store::str_list(ev, "receipts");
            }
            "checks.propose" => {
                st.checks_proposals.push(CheckProposal {
                    seq,
                    scope: sv("scope"),
                    changes: changes_from_json(&ev["changes"]),
                    run: st
                        .runs
                        .iter()
                        .rev()
                        .find(|r| !r.ended)
                        .map(|r| r.id.clone())
                        .unwrap_or_default(),
                    used: false,
                });
            }
            "checks.set" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                let pseq = ev["proposal"].as_u64().unwrap_or(0);
                let changes = changes_from_json(&ev["changes"]);
                let prop = st.checks_proposals.iter().find(|x| x.seq == pseq).cloned();
                let ok = prop
                    .as_ref()
                    .map(|x| {
                        let active = st
                            .runs
                            .iter()
                            .rev()
                            .find(|r| !r.ended)
                            .map(|r| r.id.clone())
                            .unwrap_or_default();
                        x.scope == sv("scope")
                            && x.changes == changes
                            && !x.used
                            && !st
                                .checks_proposals
                                .iter()
                                .any(|y| y.seq > x.seq && y.seq < tseq)
                            && (x.scope == "default" || (!active.is_empty() && x.run == active))
                    })
                    .unwrap_or(false)
                    && decision_turn_ok(
                        &st,
                        &turn_ix,
                        tseq,
                        pseq,
                        seq,
                        &sv("quote"),
                        Decide::Approve,
                    );
                if !ok {
                    st.ignored.push(format!(
                        "event {seq}: a check setting cites no matching proposal or no valid human turn and quote after it"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                if let Some(x) = st.checks_proposals.iter_mut().find(|x| x.seq == pseq) {
                    x.used = true;
                }
                let scope = sv("scope");
                if scope == "default" || scope == "both" {
                    apply_changes(&mut st.checks_default, &changes);
                }
                if scope == "run" || scope == "both" {
                    if let Some(r) = st.runs.iter_mut().rev().find(|r| !r.ended) {
                        apply_changes(&mut r.checks_off, &changes);
                    }
                }
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
                if ev.get("round").is_some() {
                    u.review_rounds.push(json!({
                        "round": ev["round"], "content": ev["content"], "head": ev["head"],
                        "covered": ev["covered"], "result": ev["result"],
                    }));
                }
                u.last_seq = seq;
                u.last_at = at;
            }
            "unit.scope.request" => {
                let u = st.units.entry(sv("unit")).or_default();
                u.attempts += 1;
                let c = ev["cost"].as_f64().unwrap_or(0.0);
                u.cost += c;
                u.last_seq = seq;
                u.last_at = at.clone();
                st.cost += c;
                st.scope.entry(sv("unit")).or_default().open = Some(ScopeReq {
                    seq,
                    at,
                    from: sv("from"),
                    files: ev["files"].as_array().cloned().unwrap_or_default(),
                });
            }
            "unit.scope.grant" => {
                let sc = st.scope.entry(sv("unit")).or_default();
                sc.open = None;
                let n = sc.grants.len() as u32 + 1;
                sc.grants.push(ScopeGrant {
                    n,
                    seq,
                    at: at.clone(),
                    files: ev["files"].as_array().cloned().unwrap_or_default(),
                    marks: ev["marks"].as_array().cloned().unwrap_or_default(),
                    request: ev["request"].as_u64().unwrap_or(0),
                    withdrawn: None,
                    undo: false,
                });
                let c = ev["cost"].as_f64().unwrap_or(0.0);
                st.cost += c;
                if let Some(u) = st.units.get_mut(&sv("unit")) {
                    u.cost += c;
                    u.last_seq = seq;
                    u.last_at = at;
                }
            }
            "unit.scope.refused" => {
                // A hard refusal never opened a request, so it counts here; an extension's request
                // was counted when it was made.
                let hard = sv("refusal") == "hard";
                let sc = st.scope.entry(sv("unit")).or_default();
                sc.open = None;
                sc.refused.push(json!({
                    "seq": seq, "at": at, "refusal": sv("refusal"), "files": ev["files"],
                }));
                let u = st.units.entry(sv("unit")).or_default();
                if u.state != "blocked" {
                    u.prev_state = u.state.clone();
                }
                u.state = "blocked".into();
                if hard {
                    u.attempts += 1;
                    let c = ev["cost"].as_f64().unwrap_or(0.0);
                    u.cost += c;
                    st.cost += c;
                }
                u.last_seq = seq;
                u.last_at = at;
            }
            "unit.scope.revert" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.ignored.push(format!(
                        "event {seq}: a withdrawal of a grant cites no valid human turn and quote"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                let n = ev["n"].as_u64().unwrap_or(0) as u32;
                let to = sv("to");
                if let Some(g) = st
                    .scope
                    .get_mut(&sv("unit"))
                    .and_then(|sc| sc.grants.iter_mut().find(|g| g.n == n))
                {
                    g.withdrawn = Some((sv("quote"), at.clone()));
                    g.undo = !to.is_empty();
                }
                if !to.is_empty() {
                    if let Some(u) = st.units.get_mut(&sv("unit")) {
                        if u.state != "landed" {
                            u.state = to;
                            u.last_seq = seq;
                            u.last_at = at;
                        }
                    }
                }
            }
            "unit.role-report" => {
                // The cost a role reports counts in the total, and for a unit also in the unit's.
                let c = ev["cost"].as_f64().unwrap_or(0.0);
                st.cost += c;
                if let Some(u) = st.units.get_mut(&sv("unit")) {
                    u.cost += c;
                }
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
            "checkpoint.sync.open" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.ignored.push(format!(
                        "event {seq}: a checkpoint sync cites no valid human turn and quote"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                st.sync_open = Some(SyncOpen {
                    checkpoint: sv("checkpoint"),
                    repos: ev["repos"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| SyncRepo {
                            repo: r["repo"].as_str().unwrap_or("").to_string(),
                            main: r["main"].as_str().unwrap_or("").to_string(),
                            before: r["before"].as_str().unwrap_or("").to_string(),
                            files: crate::store::str_list(r, "files"),
                            merge: r["merge"].as_str().unwrap_or("").to_string(),
                            allowed: crate::store::str_list(r, "allowed"),
                        })
                        .collect(),
                    phase: "resolve".into(),
                    rounds: 0,
                    reason: String::new(),
                    findings: vec![],
                    open_seq: seq,
                    floor: ev["receipt_floor"].as_u64().unwrap_or(0),
                    failed: ev["failed"].as_array().cloned().unwrap_or_default(),
                });
            }
            "checkpoint.sync.built" => {
                if let Some(o) = st.sync_open.as_mut() {
                    o.phase = "review".into();
                }
            }
            "checkpoint.sync.reviewed" => {
                if let Some(o) = st.sync_open.as_mut() {
                    o.rounds += 1;
                    o.phase = sv("to");
                    o.reason = sv("reason");
                    o.findings = ev["findings"].as_array().cloned().unwrap_or_default();
                }
            }
            "checkpoint.sync.park" => {
                if let Some(o) = st.sync_open.as_mut() {
                    o.phase = "parked".into();
                    o.reason = sv("reason");
                }
            }
            "checkpoint.sync.release" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve) {
                    st.ignored.push(format!(
                        "event {seq}: a sync release cites no valid human turn and quote"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                if let Some(o) = st.sync_open.as_mut() {
                    o.phase = "resolve".into();
                    o.rounds = 0;
                    o.reason.clear();
                }
            }
            "checkpoint.sync.abort" => {
                if st.sync_open.take().is_some() {
                    st.ck_end_seq = seq;
                }
            }
            "checkpoint.hold" => {
                let k = sv("checkpoint");
                let cause = sv("cause");
                if cause.is_empty() {
                    if st.ck_holds.remove(&k).is_some() {
                        st.ck_end_seq = seq;
                    }
                } else {
                    let started = st.ck_holds.get(&k).map(|h| h.seq).unwrap_or(seq);
                    st.ck_holds.insert(
                        k,
                        CkHold {
                            cause,
                            detail: sv("detail"),
                            seq: started,
                        },
                    );
                }
            }
            "checkpoint.sync" => {
                // A sync that agents resolved was asked for in the human's words when it opened.
                let via_open = ev["via_open"].as_bool().unwrap_or(false);
                if !via_open {
                    let tseq = ev["turn"].as_u64().unwrap_or(0);
                    if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve)
                    {
                        st.ignored.push(format!(
                            "event {seq}: a checkpoint sync cites no valid human turn and quote"
                        ));
                        continue;
                    }
                    st.turns[turn_ix[&tseq]].used = true;
                }
                if st
                    .sync_open
                    .as_ref()
                    .map(|o| o.checkpoint == sv("checkpoint"))
                    .unwrap_or(false)
                {
                    st.sync_open = None;
                    st.ck_end_seq = seq;
                }
                for r in ev["repos"].as_array().into_iter().flatten() {
                    st.synced.push((
                        sv("checkpoint"),
                        r["repo"].as_str().unwrap_or("").to_string(),
                        r["main"].as_str().unwrap_or("").to_string(),
                    ));
                }
                // The checkpoint's own content changed: its checks' earlier receipts no longer count.
                let floor = ev["receipt_floor"].as_u64().unwrap_or(0);
                for c in crate::store::str_list(ev, "reopen_checks") {
                    let e = st.reopen_floor.entry(c).or_insert(0);
                    *e = (*e).max(floor);
                }
            }
            "checkpoint.fix.open" => {
                if let Some(tseq) = ev["human"]["turn"].as_u64() {
                    let quote = ev["human"]["quote"].as_str().unwrap_or("").to_string();
                    let ids = id_pairs(&ev["finding_ids"]);
                    let base = put_to_human_seq(&st, &sv("checkpoint"), &ids);
                    let bound = match decision_bound(&st, &sv("checkpoint"), base, seq) {
                        Ok(b) => b,
                        Err(h) => {
                            st.ignored.push(format!(
                                "event {seq}: a fix was picked for findings while {}",
                                h.during()
                            ));
                            continue;
                        }
                    };
                    if !decision_turn_ok(&st, &turn_ix, tseq, bound, seq, &quote, Decide::Approve) {
                        st.ignored.push(format!(
                            "event {seq}: a fix opened for findings cites no valid human turn and quote, or words written before the bound"
                        ));
                        continue;
                    }
                    st.turns[turn_ix[&tseq]].used = true;
                }
                let heads: Vec<(String, String)> = ev["before"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        (
                            r["repo"].as_str().unwrap_or("").to_string(),
                            r["head"].as_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect();
                let findings = ev["findings"].as_array().cloned().unwrap_or_default();
                st.ck_fix = Some(CkFixOpen {
                    checkpoint: sv("checkpoint"),
                    source: sv("source"),
                    phase: "fix".into(),
                    history: if findings.is_empty() {
                        vec![]
                    } else {
                        vec![json!({"round": 0, "findings": findings})]
                    },
                    origin_findings: findings.clone(),
                    origin_ids: ev["finding_ids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|x| {
                            (
                                x["round"].as_u64().unwrap_or(0) as u32,
                                x["index"].as_u64().unwrap_or(0) as u32,
                            )
                        })
                        .collect(),
                    origin_failed: ev["failed"].as_array().cloned().unwrap_or_default(),
                    findings,
                    failed: ev["failed"].as_array().cloned().unwrap_or_default(),
                    allowed: crate::store::str_list(ev, "allowed"),
                    test_dirs: crate::store::str_list(ev, "test_dirs"),
                    units: crate::store::str_list(ev, "units"),
                    before: heads,
                    content: sv("content"),
                    open_seq: seq,
                    wave: ev["wave"].as_u64().unwrap_or(0) as u32,
                    own_turn: ev["human"]["turn"].as_u64().is_some(),
                    ..Default::default()
                });
            }
            "checkpoint.fix.built" => {
                if let Some(o) = st.ck_fix.as_mut() {
                    o.phase = "review".into();
                    o.failed.clear();
                }
            }
            "checkpoint.fix.attempt" => {
                if let Some(o) = st.ck_fix.as_mut() {
                    o.attempts += 1;
                    o.phase = sv("to");
                    if o.phase == "parked" {
                        o.park_seq = seq;
                        st.ck_end_seq = seq;
                    }
                    o.reason = sv("reason");
                    o.failed = ev["failed"].as_array().cloned().unwrap_or_default();
                    o.findings.clear();
                }
            }
            "checkpoint.fix.reviewed" => {
                if let Some(o) = st.ck_fix.as_mut() {
                    o.rounds_log.push(json!({
                        "round": ev["round"], "content": ev["content"], "heads": ev["heads"],
                        "covered": ev["covered"], "result": ev["result"],
                    }));
                    if sv("result") == "findings" {
                        o.rounds += 1;
                        o.phase = sv("to");
                        if o.phase == "parked" {
                            o.park_seq = seq;
                            st.ck_end_seq = seq;
                        }
                        o.reason = sv("reason");
                        o.findings = ev["findings"].as_array().cloned().unwrap_or_default();
                        o.history
                            .push(json!({"round": ev["round"], "findings": o.findings}));
                    }
                }
            }
            "checkpoint.fix.park" => {
                if let Some(o) = st.ck_fix.as_mut() {
                    o.phase = "parked".into();
                    o.park_seq = seq;
                    st.ck_end_seq = seq;
                    o.reason = sv("reason");
                }
            }
            "checkpoint.fix.gap" => {
                if let Some(o) = st.ck_fix.as_mut() {
                    o.phase = "gap".into();
                    o.reason = sv("reason");
                    o.missing = crate::store::str_list(ev, "missing");
                }
            }
            "checkpoint.fix.release" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                let Some(parked_at) = st
                    .ck_fix
                    .as_ref()
                    .filter(|f| f.phase == "parked")
                    .map(|f| f.park_seq)
                else {
                    st.ignored.push(format!(
                        "event {seq}: a checkpoint fix release of a fix that was not parked"
                    ));
                    continue;
                };
                let bound = match decision_bound(&st, &sv("checkpoint"), parked_at, seq) {
                    Ok(b) => b,
                    Err(h) => {
                        st.ignored.push(format!(
                            "event {seq}: a checkpoint fix release was written while {}",
                            h.during()
                        ));
                        continue;
                    }
                };
                if !decision_turn_ok(
                    &st,
                    &turn_ix,
                    tseq,
                    bound,
                    seq,
                    &sv("quote"),
                    Decide::Approve,
                ) {
                    st.ignored.push(format!(
                        "event {seq}: a checkpoint fix release cites no valid human turn and quote, or words written before the park"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                if let Some(o) = st.ck_fix.as_mut() {
                    o.phase = "fix".into();
                    o.rounds = 0;
                    o.attempts = 0;
                    o.reason.clear();
                    o.missing.clear();
                }
            }
            "checkpoint.finding.settle" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                let fate = sv("fate");
                let ids = id_pairs(&ev["ids"]);
                let base = put_to_human_seq(&st, &sv("checkpoint"), &ids);
                let bound = match decision_bound(&st, &sv("checkpoint"), base, seq) {
                    Ok(b) => b,
                    Err(h) => {
                        st.ignored.push(format!(
                            "event {seq}: a finding settled while {}",
                            h.during()
                        ));
                        continue;
                    }
                };
                if fate == "declined" && sv("reason").trim().is_empty() {
                    st.ignored
                        .push(format!("event {seq}: a finding declined with no reason"));
                    continue;
                }
                if !decision_turn_ok(
                    &st,
                    &turn_ix,
                    tseq,
                    bound,
                    seq,
                    &sv("quote"),
                    Decide::Approve,
                ) || !matches!(fate.as_str(), "accepted" | "declined")
                {
                    st.ignored.push(format!(
                        "event {seq}: a finding settled with no valid human turn and quote, or words written before the bound"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                for id in ev["ids"].as_array().into_iter().flatten() {
                    st.ck_settled.push(CkSettled {
                        checkpoint: sv("checkpoint"),
                        round: id["round"].as_u64().unwrap_or(0) as u32,
                        index: id["index"].as_u64().unwrap_or(0) as u32,
                        fate: fate.clone(),
                        reason: sv("reason"),
                    });
                }
            }
            "checkpoint.review.release" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                let base = round_report_seq(
                    &st,
                    &sv("checkpoint"),
                    ev["after_round"].as_u64().unwrap_or(0) as u32,
                );
                // Only the fix this release itself opened (the event just before it) is set aside.
                let before = release_opened_before(&st, &sv("checkpoint"), seq);
                let bound = match decision_bound(&st, &sv("checkpoint"), base, before) {
                    Ok(b) => b,
                    Err(h) => {
                        st.ignored.push(format!(
                            "event {seq}: a checkpoint review release was written while {}",
                            h.during()
                        ));
                        continue;
                    }
                };
                if !decision_turn_ok(
                    &st,
                    &turn_ix,
                    tseq,
                    bound,
                    seq,
                    &sv("quote"),
                    Decide::Approve,
                ) {
                    st.ignored.push(format!(
                        "event {seq}: a checkpoint review release cites no valid human turn and quote, or words written before the bound"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                st.ck_review_floor.insert(
                    sv("checkpoint"),
                    ev["after_round"].as_u64().unwrap_or(0) as u32,
                );
            }
            "checkpoint.review.done" => {
                st.ck_round_done.push((
                    sv("checkpoint"),
                    ev["round"].as_u64().unwrap_or(0) as u32,
                    seq,
                ));
            }
            "checkpoint.fix.abort" => {
                if let Some(o) = st.ck_fix.take() {
                    st.ck_end_seq = seq;
                    st.ck_fix_aborts.push(CkFixAbort {
                        checkpoint: o.checkpoint.clone(),
                        ids: o.origin_ids.clone(),
                        seq,
                    });
                    st.ck_declined
                        .push(format!("{}@{}", o.checkpoint, o.content));
                }
            }
            "checkpoint.fix.done" => {
                if let Some(o) = st.ck_fix.take() {
                    st.ck_end_seq = seq;
                    let n = st
                        .ck_fixes
                        .iter()
                        .filter(|f| f.checkpoint == o.checkpoint)
                        .count()
                        + 1;
                    st.ck_fixes.push(CkFixDone {
                        id: format!("{}-fix-{n}", o.checkpoint),
                        checkpoint: o.checkpoint.clone(),
                        source: o.source.clone(),
                        files: crate::store::str_list(ev, "files"),
                        units: crate::store::str_list(ev, "units"),
                        by_file: ev["by_file"].clone(),
                        rounds_log: o.rounds_log.clone(),
                        origin_failed: o.origin_failed.clone(),
                        origin_findings: o.origin_findings.clone(),
                        ids: o.origin_ids.clone(),
                        from_round: st
                            .ck_round_done
                            .iter()
                            .filter(|(c, _, q)| *c == o.checkpoint && *q < o.open_seq)
                            .map(|(_, r, _)| *r)
                            .max()
                            .unwrap_or(0),
                        seq,
                    });
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
            "legal.accept" => {
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                if !decision_turn_ok(&st, &turn_ix, tseq, 0, seq, &sv("quote"), Decide::Approve)
                    || sv("record").is_empty()
                {
                    st.ignored.push(format!(
                        "event {seq}: a legal acceptance cites no valid human turn and quote"
                    ));
                    continue;
                }
                st.turns[turn_ix[&tseq]].used = true;
                st.legal_accepts.push((sv("record"), sv("quote"), seq));
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
                    if let Some(un) = ev.get("unproven_off").and_then(|v| v.as_array()) {
                        e.recorded = true;
                        e.run = sv("run");
                        e.off = crate::store::str_list(ev, "off_names");
                        for o in un {
                            let id = o["id"].as_str().unwrap_or("").to_string();
                            let names = o["checks"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|x| x.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default();
                            e.waived.insert(id, names);
                        }
                    }
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
                withdrawn_why: String::new(),
                changed_seq: seq,
            }),
            "amend.withdraw" => {
                let id = sv("amend");
                if let Some(a) = st
                    .amends
                    .iter_mut()
                    .find(|a| a.id == id && matches!(a.status.as_str(), "open" | "narrowed"))
                {
                    a.status = "withdrawn".into();
                    a.withdrawn_why = sv("reason");
                }
            }
            "amend.revise" => {
                let id = sv("amend");
                if let Some(a) = st
                    .amends
                    .iter_mut()
                    .find(|a| a.id == id && matches!(a.status.as_str(), "open" | "narrowed"))
                {
                    a.ids = crate::store::str_list(ev, "ids");
                    a.impact = ev.get("impact").cloned().unwrap_or(Value::Null);
                    a.keep = crate::store::str_list(ev, "impact_ids");
                    a.waive = vec![];
                    a.reasons = Value::Null;
                    a.status = "open".into();
                    a.changed_seq = seq;
                }
            }
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
                    a.changed_seq = seq;
                }
            }
            "amend.approve" => {
                let id = sv("amend");
                let tseq = ev["turn"].as_u64().unwrap_or(0);
                let changed = st
                    .amends
                    .iter()
                    .find(|a| a.id == id)
                    .map(|a| a.changed_seq)
                    .unwrap_or(0);
                if !decision_turn_ok(
                    &st,
                    &turn_ix,
                    tseq,
                    changed,
                    seq,
                    &sv("quote"),
                    Decide::Approve,
                ) {
                    st.ignored.push(format!(
                        "event {seq}: amendment approval cites no valid human turn after its latest change"
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
    if !candidates.clone().any(|t| t.text.contains(q)) {
        if st
            .turns
            .iter()
            .any(|t| t.used && t.text.contains(q) && !crate::gates::recorded_text_is_host(&t.text))
        {
            return Err(refuse(
                "record a decision with this quote",
                "those words were already used for another decision: a message decides once",
                &[
                    "Ask the human in chat for their decision in a new message, in their own words.",
                    "Then run the command again with `--quote` set to the words of that new message.",
                ],
                "`bf gate status` shows the decision with its quote.",
            ));
        }
        if after_seq > 0
            && st
                .turns
                .iter()
                .any(|t| t.seq <= after_seq && !t.used && t.text.contains(q))
        {
            return Err(refuse(
                "record a decision with this quote",
                format!("no human turn written after this was put to the human can decide it: those words were written before the decision was put to the human (log event {after_seq})"),
                &[
                    "Ask the human in chat for their decision in a new message, written now, in their own words.",
                    "Then run the command again with `--quote` set to the words of that new message.",
                ],
                "`bf gate status` shows the decision with its quote.",
            ));
        }
    }
    if candidates.peek().is_none() {
        let cause = st.human_turns_note();
        let mut steps: Vec<String> = Vec::new();
        if let Some(c) = &cause {
            steps.push(c.clone());
        }
        steps.push("Ask the human in chat for their decision, in their own words.".to_string());
        steps.push("The harness hook records their message with `bf human-turn`; check with `bf log --tail 5`.".to_string());
        steps.push(
            "Then run the command again with `--quote` set to the sentence that decides it."
                .to_string(),
        );
        return Err(crate::refusal::refuse_owned(
            "record a decision",
            if cause.is_some() {
                "no human turn was recorded at all: the hook that records the human's messages is not loaded"
            } else {
                "no human turn was recorded after this opened that can decide it: the human has not written since, or the message was already used for another decision"
            },
            steps,
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
