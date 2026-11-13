//! Receipts. The engine runs a check's command itself and seals the result.
//!
//! A receipt is sealed with a key kept in the project's state folder, outside the project tree,
//! which is never committed and which the guard keeps agents away from. The seal covers every field that
//! matters, including the content hash the command ran on. A receipt written by
//! hand has no valid seal and is ignored everywhere.

use crate::events;
use crate::refusal::{err, refuse, R};
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Receipt {
    pub v: Value,
    pub seal_ok: bool,
    /// The check record is no longer the one the receipt was made for (its command or fields changed).
    pub stale: bool,
    /// How the check is proven: command, agent or human.
    pub how: String,
}

/// The hash of a record as the engine sees it. Receipts and gate snapshots both use it.
pub fn record_hash(rec: &Value) -> String {
    util::sha256_hex(serde_json::to_string(rec).unwrap_or_default().as_bytes())
}

impl Receipt {
    pub fn id(&self) -> &str {
        s(&self.v, "id")
    }
    pub fn check(&self) -> &str {
        s(&self.v, "check")
    }
    pub fn content(&self) -> &str {
        s(&self.v, "content_hash")
    }
    pub fn is_run(&self) -> bool {
        s(&self.v, "type") == "run"
    }
    /// A sealed receipt that records success: exit 0 for a run, any attachment.
    pub fn passes(&self) -> bool {
        match s(&self.v, "type") {
            "run" => self.seal_ok && !self.stale && self.v["exit"].as_i64() == Some(0),
            // A file proves only a check that an agent does; a human check is settled by the human's gate.
            // A met outcome with the sentence that says what was looked at; a bare file is supporting
            // material and settles nothing.
            "attach" => {
                self.seal_ok
                    && !self.stale
                    && self.how == "agent"
                    && !s(&self.v, "reason").is_empty()
            }
            // `fail` and `unreachable` outcomes are records, never proof.
            _ => false,
        }
    }
    /// A sealed verifier outcome: the check failed, or could not be run.
    pub fn outcome(&self) -> Option<&str> {
        match s(&self.v, "type") {
            t @ ("fail" | "unreachable") if self.seal_ok => Some(t),
            _ => None,
        }
    }
}

fn seal_input(v: &Value) -> String {
    let f = |k: &str| match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    };
    [
        "bf-receipt-v1",
        "id",
        "type",
        "check",
        "unit",
        "command",
        "exit",
        "content_hash",
        "output_hash",
        "file_hash",
        "at",
        "bytes",
        "reason",
        "check_hash",
        "program",
        "sandbox",
    ]
    .iter()
    .map(|k| {
        if *k == "bf-receipt-v1" {
            k.to_string()
        } else {
            format!("{k}={}", f(k))
        }
    })
    .collect::<Vec<_>>()
    .join("\x1f")
}

pub fn read_key(p: &Project) -> Option<String> {
    p.read_key()
}

pub fn seal(key: &str, v: &Value) -> String {
    util::sha256_hex(format!("{key}\x1e{}", seal_input(v)).as_bytes())
}

/// The number of the newest receipt on file (0 when there is none).
pub fn highest_receipt_number(p: &Project) -> u64 {
    fs::read_dir(p.receipts_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| receipt_number(&e.path().file_stem()?.to_string_lossy()))
        .max()
        .unwrap_or(0)
}

fn receipt_number(id: &str) -> Option<u64> {
    id.strip_prefix("rc-")?.parse().ok()
}

pub fn load_receipts(p: &Project) -> R<Vec<Receipt>> {
    let key = read_key(p);
    // Receipts that an approved amendment put out of count: for a re-opened check, those up to the
    // newest one on file when the amendment was approved.
    let floors = crate::events::replay(p)
        .map(|st| st.reopen_floor)
        .unwrap_or_default();
    let mut out = Vec::new();
    let rd = match fs::read_dir(p.receipts_dir()) {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut checks: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for path in paths {
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let Ok(v) = toml::from_str::<Value>(&text) else {
            continue;
        };
        // A receipt's id is its file name: a copy under another name is not a second receipt.
        if path.file_stem().and_then(|f| f.to_str()) != v.get("id").and_then(|x| x.as_str()) {
            continue;
        }
        let want = v.get("seal").and_then(|x| x.as_str()).unwrap_or("");
        let seal_ok = match &key {
            Some(k) => !want.is_empty() && seal(k, &v) == want,
            None => false,
        };
        let cid = s(&v, "check").to_string();
        let (hash, how) = checks
            .entry(cid.clone())
            .or_insert_with(|| match p.get_record(&cid) {
                Ok(Some(rec)) => (record_hash(&rec), s(&rec, "how").to_string()),
                _ => (String::new(), String::new()),
            })
            .clone();
        let stale = hash.is_empty()
            || s(&v, "check_hash") != hash
            || floors
                .get(&cid)
                .map(|f| receipt_number(s(&v, "id")).unwrap_or(0) <= *f)
                .unwrap_or(false);
        out.push(Receipt {
            v,
            seal_ok,
            stale,
            how,
        });
    }
    Ok(out)
}

fn next_receipt_id(p: &Project) -> String {
    let mut max = 0u32;
    if let Ok(rd) = fs::read_dir(p.receipts_dir()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(num) = n
                .strip_prefix("rc-")
                .and_then(|r| r.split('.').next())
                .and_then(|r| r.parse::<u32>().ok())
            {
                max = max.max(num);
            }
        }
    }
    format!("rc-{:05}", max + 1)
}

fn write_receipt(p: &Project, mut v: Value) -> R<Value> {
    let key = read_key(p).ok_or_else(|| {
        refuse(
            "write a receipt",
            "the sealing key in the project's state folder is missing",
            &["Run `bf init` again in this project; it creates the key if it is absent."],
            "`bf doctor` reports `key: ok`.",
        )
    })?;
    fs::create_dir_all(p.receipts_dir())?;
    if let Some(rec) = p.get_record(s(&v, "check"))? {
        v["check_hash"] = json!(record_hash(&rec));
    }
    loop {
        let id = next_receipt_id(p);
        v["id"] = json!(id);
        let sl = seal(&key, &v);
        v["seal"] = json!(sl);
        let path = p.receipts_dir().join(format!("{id}.toml"));
        let text = toml::to_string_pretty(&crate::store::strip_nulls(v.clone()))?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes())?;
                return Ok(v);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

fn command_words(check: &Value) -> Vec<String> {
    match check.get("command") {
        Some(Value::String(c)) => c.split_whitespace().map(|x| x.to_string()).collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    }
}

/// `bf evidence run <check> [--unit U] -- <command...>`
pub fn run(p: &Project, check_id: &str, unit: Option<&str>, argv: &[String]) -> R<Value> {
    if argv.is_empty() {
        return Err(refuse(
            "run evidence",
            "no command was given after `--`",
            &["Put the check's command after `--`, for example `bf evidence run X-1 -- cargo test`."],
            "The command prints a receipt with exit 0.",
        ));
    }
    let chk = p.require_record(check_id)?;
    if s(&chk, "kind") != "check" {
        return Err(refuse(
            format!("run evidence for {check_id}"),
            "that ID is not a check",
            &["Use the ID of a check record (X-<number>)."],
            "`bf record get <id>` shows kind = \"check\".",
        ));
    }
    if s(&chk, "how") != "command" {
        return Err(refuse(
            format!("run a command for {check_id}"),
            format!(
                "this check is done by an {}, not by a command",
                s(&chk, "how")
            ),
            &["Use `bf evidence attach` to store the file that proves it."],
            "`bf evidence verify` lists the receipt.",
        ));
    }
    let want = command_words(&chk);
    if want != argv {
        return Err(refuse(
            format!("run `{}` as evidence for {check_id}", argv.join(" ")),
            format!("the check defines its own command: `{}`; a receipt for another command would prove nothing about this check", want.join(" ")),
            &["Run exactly the command stored in the check record, after `--`.", "To change what the check runs, change the record (the human's step)."],
            "The receipt's command equals the check's command.",
        ));
    }
    if let Some(u) = unit {
        if p.get_record(u)?.is_none() {
            return Err(refuse(
                format!("run evidence for unit {u}"),
                "no such unit",
                &["Use a unit ID from `bf record list --kind unit`."],
                "`bf record get` prints the unit.",
            ));
        }
    }
    let top = p.top.clone();
    let before = p.content_hash(&top)?;
    let tail_bytes = p.policy.evidence.tail_bytes;
    let started = std::time::Instant::now();
    let program_note = crate::programs::verify(p, check_id, &argv[0])?;
    if let Some(n) = &program_note {
        eprintln!("note: {n}");
    }
    let (exit, tail, out_hash, total, sandboxed) = exec_capture(p, &top, argv, tail_bytes)?;
    let after = p.content_hash(&top)?;
    if before != after {
        return Err(refuse(
            format!("write a receipt for {check_id}"),
            "the command changed the files of the working copy while it ran, so the receipt would not describe the content it started on",
            &[
                "Find which files the command writes (see `git status`).",
                "Add generated files to `.gitignore`, or make the command leave tracked files alone.",
                "Run the same `bf evidence run` command again.",
            ],
            "The receipt is written and its content hash equals the one before the command.",
        ));
    }
    let v = json!({
        "type": "run",
        "check": check_id,
        "unit": unit,
        "command": argv.join(" "),
        "program": CHECK_SHELL,
        "sandbox": if sandboxed { "sandbox-exec" } else { "none" },
        "program_note": program_note.clone().unwrap_or_default(),
        "exit": exit,
        "content_hash": before,
        "output_hash": out_hash,
        "bytes": total,
        "output_tail": tail,
        "at": util::now(),
        "seconds": started.elapsed().as_secs_f64(),
        "working_copy": top.strip_prefix(&p.root).map(|r| r.to_string_lossy().to_string()).unwrap_or_default(),
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.run",
        json!({"receipt": v["id"], "check": check_id, "unit": unit, "exit": exit}),
    )?;
    if exit != 0 {
        if let Some(u) = unit {
            crate::run::park_if_looping(p, u)?;
        }
    }
    Ok(v)
}

/// Run a command with stdout and stderr merged. Keep a bounded tail and hash all of it.
fn shell_quote(a: &str) -> String {
    format!("'{}'", a.replace('\'', "'\\''"))
}

/// The environment a check runs in: nothing of the caller's except a short list, and the PATH from the
/// policy. A caller cannot put a fake `sh` or `cargo` first on the path, or point git elsewhere.
pub const CHECK_ENV: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "TMPDIR",
    "TZ",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "CARGO_BUILD_JOBS",
];
pub const CHECK_SHELL: &str = "/bin/sh";

fn exec_capture(
    p: &Project,
    dir: &Path,
    argv: &[String],
    tail_bytes: usize,
) -> R<(i64, String, String, u64, bool)> {
    let (mut reader, writer) = std::io::pipe().map_err(|e| err(format!("pipe: {e}")))?;
    let w2 = writer.try_clone().map_err(|e| err(format!("pipe: {e}")))?;
    let path = if p.policy.exec.path.is_empty() {
        "/usr/bin:/bin:/usr/sbin:/sbin".to_string()
    } else {
        p.policy.exec.path.clone()
    };
    let line = argv
        .iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    // Under the sandbox where there is one: the check reaches neither the state folder nor the network,
    // and writes only inside its working copy.
    let sandboxed = p.policy.exec.sandbox && crate::sandbox::available();
    let mut cmd = if sandboxed {
        crate::sandbox::command(
            &crate::sandbox::check_profile(
                &p.state_dir_resolved(),
                &p.root.to_string_lossy(),
                &dir.to_string_lossy(),
                p.policy.exec.network,
                &p.transcript_root(),
                &p.repos
                    .iter()
                    .map(|r| r.path.to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
            ),
            CHECK_SHELL,
        )
    } else {
        Command::new(CHECK_SHELL)
    };
    cmd.arg("-c")
        .arg(&line)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(w2)
        .stderr(writer)
        .env_clear()
        .env("PATH", &path)
        .env("BF_EVIDENCE", "1");
    for k in CHECK_ENV
        .iter()
        .copied()
        .chain(p.policy.exec.env.iter().map(|s| s.as_str()))
    {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    let mut child = cmd.spawn().map_err(|e| {
        refuse(
            format!("run `{}`", argv.join(" ")),
            format!("the command could not be started: {e}"),
            &["Check that the program exists and is on PATH."],
            "The command starts and a receipt is written.",
        )
    })?;
    drop(cmd);
    let mut hasher = sha2::Sha256::default();
    use sha2::Digest;
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = [0u8; 32_768];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > tail_bytes * 2 {
            let cut = tail.len() - tail_bytes;
            tail.drain(..cut);
        }
    }
    let status = child.wait()?;
    if tail.len() > tail_bytes {
        let cut = tail.len() - tail_bytes;
        tail.drain(..cut);
    }
    let exit = status.code().map(|c| c as i64).unwrap_or(-1);
    Ok((
        exit,
        String::from_utf8_lossy(&tail).to_string(),
        util::hex(&hasher.finalize()),
        total,
        sandboxed,
    ))
}

/// What is worth knowing about the folders on the check PATH, for `bf doctor`: ones inside the project
/// and ones in the user's home folder. Information only; nothing is refused for it.
pub fn path_notes(p: &Project) -> Vec<String> {
    let root = p.root.clone();
    let mut out = Vec::new();
    for dir in p.policy.exec.path.split(':').filter(|d| !d.is_empty()) {
        let Ok(canon) = fs::canonicalize(dir) else {
            continue;
        };
        if canon.starts_with(&root) {
            out.push(format!("{dir} lies inside the project"));
            continue;
        }
        let standard = ["/bin", "/usr/bin", "/sbin", "/usr/sbin"];
        if standard.iter().any(|s| canon == Path::new(s)) {
            continue;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() && canon.starts_with(&home) {
            out.push(format!("{dir} is in the user's home folder (the user, and so any program they run, can change what is in it)"));
        }
    }
    out
}

/// `bf evidence attach <check> <file>`
/// What a file attached to a check says: `None` keeps it as supporting material; `met`, `failed` and
/// `unreachable` are a judgment, with the sentence that says what was looked at.
pub struct Judgment<'a> {
    pub outcome: Option<&'a str>,
    pub says: &'a str,
    pub token: Option<&'a str>,
}

pub fn attach(
    p: &Project,
    check_id: &str,
    file: &Path,
    unit: Option<&str>,
    j: &Judgment,
) -> R<Value> {
    // Resolved again now: a link that leads into the state folder is refused.
    let resolved = p.safe_input(file)?;
    let file = resolved.as_path();
    let chk = p.require_record(check_id)?;
    if s(&chk, "kind") != "check" {
        return Err(refuse(
            format!("attach evidence to {check_id}"),
            "that ID is not a check",
            &["Use the ID of a check record (X-<number>)."],
            "`bf record get` shows kind = \"check\".",
        ));
    }
    if s(&chk, "how") == "command" {
        return Err(refuse(
            format!("attach a file to {check_id}"),
            "a command check is proven only by running its command; a stored file would prove nothing",
            &["Use `bf evidence run` for this check."],
            "`bf evidence verify` lists a run receipt.",
        ));
    }
    let meta = fs::metadata(file).map_err(|e| {
        refuse(
            format!("attach {}", file.display()),
            format!("the file cannot be read: {e}"),
            &["Give the path of an existing file."],
            "The command prints a receipt.",
        )
    })?;
    if meta.len() > p.policy.evidence.max_attach_bytes {
        return Err(refuse(
            format!("attach {}", file.display()),
            format!("the file is {} bytes; the policy limit is {}", meta.len(), p.policy.evidence.max_attach_bytes),
            &["Attach a smaller file, or ask the human to raise `evidence.max_attach_bytes` in factory/policy.toml."],
            "The command prints a receipt.",
        ));
    }
    let says = j.says.trim();
    let kind = match j.outcome {
        None => "support",
        Some("met") => "attach",
        Some("failed") => "fail",
        Some("unreachable") => "unreachable",
        Some(o) => {
            return Err(refuse(
                format!("attach with the outcome `{o}`"),
                "that is not an outcome",
                &["Use `--outcome met`, `--outcome failed` or `--outcome unreachable`."],
                "The command prints a receipt.",
            ))
        }
    };
    if j.outcome.is_some() {
        if s(&chk, "how") != "agent" {
            return Err(refuse(
                format!("give {check_id} an outcome"),
                "only a check judged by an agent takes an outcome; a human check is settled by the human's gate",
                &["Attach the file without an outcome as supporting material."],
                "`bf evidence verify` lists the receipt.",
            ));
        }
        if says.is_empty() || says.contains('\n') || says.chars().count() > 400 {
            return Err(refuse(
                format!("give {check_id} an outcome"),
                "an outcome needs one sentence saying what was looked at (`--says \"...\"`, one line)",
                &["Say what you looked at and what you saw: `--says \"I opened the page at 375px and read every label\"`."],
                "The command prints a receipt.",
            ));
        }
        if p.policy.roles.tokens {
            // The verifier's token for a checkpoint check; the reviewer's for a unit's own check. A
            // builder holds neither, so it cannot settle a check for its own unit.
            let pl = crate::plan::load(p)?;
            let mut keys: Vec<String> = pl
                .checkpoints
                .iter()
                .filter(|k| str_list(k, "checks").iter().any(|c| c == check_id))
                .map(|k| format!("{}:verifier", s(k, "id")))
                .collect();
            if !s(&chk, "unit").is_empty() {
                keys.push(format!("{}:reviewer", s(&chk, "unit")));
            }
            if !keys.iter().any(|k| crate::run::holds_token(p, k, j.token)) {
                return Err(refuse(
                    format!("give {check_id} an outcome"),
                    "an outcome on a check judged by an agent is accepted only with the verifier's token (the reviewer's, for a check of one unit); none, or the wrong one, was given",
                    &[
                        "The conductor runs `bf next --json`; the verifier's directive for the checkpoint carries its `token`.",
                        "The verifier passes it: `bf evidence attach <check> <file> --outcome met --says \"...\" --token <token>`. A builder cannot settle a check for its own unit.",
                    ],
                    "`bf evidence verify` lists the outcome.",
                ));
            }
        }
    }
    let h = util::sha256_file(file)?;
    fs::create_dir_all(p.evidence_dir())?;
    let dest = p.evidence_dir().join(&h);
    if !dest.exists() {
        fs::copy(file, &dest)?;
    }
    let content = p.content_hash(&p.top)?;
    let v = json!({
        "type": kind,
        "check": check_id,
        "unit": unit,
        "reason": says,
        "file_hash": h,
        "file_name": file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        "bytes": meta.len(),
        "content_hash": content,
        "at": util::now(),
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.attach",
        json!({"receipt": v["id"], "check": check_id, "unit": unit}),
    )?;
    Ok(v)
}

/// `bf evidence fail|unreachable <check> --reason ...`: the verifier records that a check ran and
/// does not match (`fail`) or could not be run (`unreachable`). A sealed record bound to the content,
/// never a pass.
pub fn outcome(
    p: &Project,
    check_id: &str,
    kind: &str,
    reason: &str,
    unit: Option<&str>,
) -> R<Value> {
    let chk = p.require_record(check_id)?;
    if s(&chk, "kind") != "check" {
        return Err(refuse(
            format!("record an outcome for {check_id}"),
            "that ID is not a check",
            &["Use the ID of a check record (X-<number>)."],
            "`bf evidence verify` lists the outcome.",
        ));
    }
    if reason.trim().is_empty() {
        return Err(refuse(
            format!("mark {check_id} as {kind}"),
            "no reason was given",
            &["Say what you saw or what blocked you: `--reason \"the staging service is down\"`."],
            "The command prints a receipt of type fail or unreachable.",
        ));
    }
    let content = p.content_hash(&p.top)?;
    let v = json!({
        "type": kind,
        "check": check_id,
        "unit": unit,
        "reason": reason,
        "content_hash": content,
        "at": util::now(),
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.outcome",
        json!({"receipt": v["id"], "check": check_id, "outcome": kind, "unit": unit}),
    )?;
    Ok(v)
}

/// The verifier's standing outcome for a check at this content: `fail` or `unreachable`, unless a
/// later sealed passing receipt for the same content supersedes it.
pub fn standing_outcome<'a>(
    receipts: &'a [Receipt],
    check_id: &str,
    content: &str,
) -> Option<&'a Receipt> {
    let mut cur: Option<&Receipt> = None;
    for r in receipts
        .iter()
        .filter(|r| r.check() == check_id && r.content() == content)
    {
        if r.passes() {
            cur = None;
        } else if r.outcome().is_some() {
            cur = Some(r);
        }
    }
    cur
}

/// A check is satisfied at `content` when a sealed, passing receipt for it carries that content hash.
pub fn satisfied_at<'a>(
    receipts: &'a [Receipt],
    check_id: &str,
    content: &str,
) -> Option<&'a Receipt> {
    receipts
        .iter()
        .rev()
        .find(|r| r.check() == check_id && r.passes() && r.content() == content)
}

pub struct VerifyRow {
    pub receipt: String,
    pub check: String,
    pub seal_ok: bool,
    pub content_ok: Option<bool>,
    pub exit: Option<i64>,
}

/// `bf evidence verify [--unit U] [--checkpoint K]`
pub fn verify(
    p: &Project,
    unit: Option<&str>,
    checkpoint: Option<&str>,
) -> R<(Vec<VerifyRow>, bool)> {
    let receipts = load_receipts(p)?;
    let mut current: Option<(String, Vec<String>)> = None; // content hash and the checks in scope
    let checks = p.list_records(crate::schema::kind_by_name("check"))?;
    if let Some(u) = unit {
        let dir = p.work_dir().join(u);
        let top = if dir.is_dir() { dir } else { p.top.clone() };
        let ids = checks
            .iter()
            .filter(|c| s(c, "unit") == u)
            .map(|c| s(c, "id").to_string())
            .collect();
        current = Some((p.content_hash(&top)?, ids));
    }
    if let Some(k) = checkpoint {
        let rec = p.require_record(k)?;
        let dir = p.work_dir().join(k);
        let top = if dir.is_dir() { dir } else { p.top.clone() };
        current = Some((
            p.content_hash(&top)?,
            crate::store::str_list(&rec, "checks"),
        ));
    }
    let mut rows = Vec::new();
    let mut all_ok = true;
    for r in &receipts {
        if let Some((_, ids)) = &current {
            if !ids.iter().any(|i| i == r.check()) {
                continue;
            }
        }
        let content_ok = current.as_ref().map(|(h, _)| h == r.content());
        // Scoped: an older receipt for other content is shown (OTHER) but only a bad seal fails it;
        // what counts is that every check in scope has a passing receipt at the current content (below).
        // Unscoped: every receipt must be sealed and passing.
        let ok = if current.is_some() {
            r.seal_ok
        } else {
            r.seal_ok && r.passes()
        };
        if !ok {
            all_ok = false;
        }
        rows.push(VerifyRow {
            receipt: r.id().to_string(),
            check: r.check().to_string(),
            seal_ok: r.seal_ok,
            content_ok,
            exit: r.v["exit"].as_i64(),
        });
    }
    // With a scope, every check in scope must have at least one passing receipt at the current content.
    if let Some((h, ids)) = &current {
        for id in ids {
            if satisfied_at(&receipts, id, h).is_none() {
                all_ok = false;
            }
        }
    }
    Ok((rows, all_ok))
}
