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
        // The screens a design-stage judgment looked at; sealed only when present, so older
        // receipts keep their seals.
        + &match f("screens_hash") {
            h if h.is_empty() => String::new(),
            h => format!("\x1fscreens_hash={h}"),
        }
        // A deliberate repeat's attempt number and a judgement are sealed only when present.
        + &match f("attempt") {
            h if h.is_empty() => String::new(),
            h => format!("\x1fattempt={h}"),
        }
        + &match f("judgement") {
            h if h.is_empty() => String::new(),
            h => format!("\x1fjudgement={h}"),
        }
        // A UI/UX journey step (or screen state) receipt seals what it is about and what was seen.
        + &["journey", "step", "step_outcome"]
            .iter()
            .map(|k| match f(k) {
                h if h.is_empty() => String::new(),
                h => format!("\x1f{k}={h}"),
            })
            .collect::<String>()
        // A design review receipt seals the kind of review, the amendment it is for and the hash of
        // the records it judged, each only when present.
        + &["review_kind", "review_amendment", "records_hash", "review_round"]
            .iter()
            .map(|k| match f(k) {
                h if h.is_empty() => String::new(),
                h => format!("\x1f{k}={h}"),
            })
            .collect::<String>()
        // R-32: what makes two runs the same run, and the receipt a counted one stands for.
        + &["repos", "folder", "counted_from", "load", "cores"]
            .iter()
            .map(|k| match f(k) {
                h if h.is_empty() => String::new(),
                h => format!("\x1f{k}={h}"),
            })
            .collect::<String>()
}

/// A hash of the screen records as they are now: what a design-stage UI/UX judgment looked at.
pub fn screens_hash(p: &Project) -> R<String> {
    let mut recs = p.list_records(crate::schema::kind_by_name("screen"))?;
    recs.sort_by(|a, b| s(a, "id").cmp(s(b, "id")));
    let mut text: Vec<String> = recs
        .iter()
        .map(|r| format!("{}:{}", s(r, "id"), record_hash(r)))
        .collect();
    // The journeys too: a changed journey makes a design-stage UI/UX judgment stale.
    let mut js = p.list_records(crate::schema::kind_by_name("journey"))?;
    js.sort_by(|a, b| s(a, "id").cmp(s(b, "id")));
    text.extend(
        js.iter()
            .map(|j| format!("{}:{}", s(j, "id"), record_hash(j))),
    );
    Ok(util::sha256_hex(text.join("\n").as_bytes()))
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
        let attached = cid.starts_with(crate::checks::ATTACHED_PREFIX);
        let (hash, how) = checks
            .entry(cid.clone())
            .or_insert_with(|| match p.get_record(&cid) {
                Ok(Some(rec)) => (record_hash(&rec), s(&rec, "how").to_string()),
                _ if attached => ("attached".to_string(), "command".to_string()),
                _ => (String::new(), String::new()),
            })
            .clone();
        let stale = hash.is_empty()
            || (!attached && s(&v, "check_hash") != hash)
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

pub(crate) fn write_receipt(p: &Project, mut v: Value) -> R<Value> {
    let key = read_key(p).ok_or_else(|| {
        refuse(
            "write a receipt",
            "the sealing key in the project's state folder is missing",
            &["Run `bf init` again in this project; it creates the key if it is absent."],
            "`bf doctor` reports `key: ok`.",
        )
    })?;
    fs::create_dir_all(p.receipts_dir())?;
    // A check the engine attached to a unit has no record; the receipt's sealed command says what ran.
    if !s(&v, "check").starts_with(crate::checks::ATTACHED_PREFIX) {
        if let Some(rec) = p.get_record(s(&v, "check"))? {
            v["check_hash"] = json!(record_hash(&rec));
        }
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

pub fn command_words(check: &Value) -> Vec<String> {
    match check.get("command") {
        Some(Value::String(c)) => c.split_whitespace().map(|x| x.to_string()).collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    }
}

/// The first word of a command that is a shell operator or redirection (`&&`, `||`, `;`, `|`, `&`, `>`,
/// `>>`, `<`, `<<`, `2>`, `2>&1`, `&>`, `>|`, a word that ends in `;` or begins with `|`, `<` or `>`).
/// A command written as one string (`one_string`) is split on spaces, so a quote, a backtick, a backslash
/// or `$(` in it would reach the program as it is: such a word is found too. The list form keeps an
/// argument with a space.
pub fn shell_operator_word(words: &[String], one_string: bool) -> Option<String> {
    const OPS: [&str; 13] = [
        "&&", "||", ";", "|", "&", ">", ">>", "<", "<<", "2>", "2>&1", "&>", ">|",
    ];
    words
        .iter()
        .find(|w| {
            OPS.contains(&w.as_str())
                || w.ends_with(';')
                || w.starts_with(['|', '<', '>'])
                || (one_string
                    && (w.contains(['\'', '"', '`', '\\', '&', '|', ';', '<', '>'])
                        || w.contains("$(")))
        })
        .cloned()
}

/// The first word of an override's text (words as the shell would split them, quotes keeping a word whole)
/// that holds, outside quotes, an operator or redirection character (`&`, `|`, `;`, `<`, `>`), a backtick
/// or `$(`, whether it stands alone or is joined to a word (`a&&b`, `2>/dev/null`). A quoted argument
/// such as `sh -c 'a && b'` is one word and passes.
pub fn unquoted_operator_word(text: &str) -> Option<String> {
    let mut cur = String::new();
    let mut bad = false;
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                } else if q == '"' && c == '\\' {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                    }
                }
            }
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                cur.push(c);
            }
            None if c == '\\' => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            None if c.is_whitespace() => {
                if bad {
                    return Some(cur);
                }
                cur.clear();
            }
            None => {
                if matches!(c, '&' | '|' | ';' | '<' | '>' | '`') || (c == '(' && prev == '$') {
                    bad = true;
                }
                cur.push(c);
            }
        }
        prev = c;
    }
    bad.then_some(cur)
}

/// The operator word of a check record's command, if it holds one.
pub fn record_operator_word(check: &Value) -> Option<String> {
    let one = matches!(check.get("command"), Some(Value::String(_)));
    shell_operator_word(&command_words(check), one)
}

/// The refusal text for a command with an operator word: the check, the word and the two ways out.
pub fn operator_message(check: &str, word: &str) -> String {
    format!(
        "{check}: the command holds `{word}`, which a check cannot use: a check's command is one program and its arguments, run without a shell. Split it into one check per program, or write the command as a list (`command = [...]`) when an argument holds a space"
    )
}

/// The command the list gives for the attached check `check_id` of `unit`, before the unit's named tests take
/// the suite's place: the unit's own list, then its checkpoint's, the run's, the project's (the order of
/// `checks::attached_for_unit`).
pub(crate) fn listed_command(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &crate::events::State,
    unit: &str,
    check_id: &str,
) -> Option<Vec<String>> {
    let name = check_id.strip_prefix(crate::checks::ATTACHED_PREFIX)?;
    let u = pl.unit(unit)?;
    let from_value = |v: &Value| -> Vec<(String, String)> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|x| (s(x, "name").trim().to_string(), s(x, "command").to_string()))
            .collect()
    };
    let list: Vec<(String, String)> = if let Some(v) = u.get("own_checks") {
        from_value(v)
    } else if let Some(v) = pl
        .checkpoint(s(u, "checkpoint"))
        .and_then(|k| k.get("unit_checks"))
    {
        from_value(v)
    } else {
        match st.runs.last().and_then(|r| r.unit_checks.as_ref()) {
            Some(c) if c["mode"] == "override" => from_value(&c["checks"]),
            Some(c) if c["mode"] == "none" => vec![],
            _ => p
                .policy
                .unit_checks
                .list
                .iter()
                .map(|d| (d.name.trim().to_string(), d.command.clone()))
                .collect(),
        }
    };
    list.into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, c)| crate::checks::shell_words(&c))
}

/// Set while a landing turn runs the run's checks in this process: only then is the list's own suite command
/// accepted for the attached suite check of a unit that names its tests (R-62 holds for a unit's own runs).
static LANDING_RUN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn landing_run(on: bool) {
    LANDING_RUN.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// `bf evidence run <check> [--unit U] -- <command...>`
pub fn run(p: &Project, check_id: &str, unit: Option<&str>, argv: &[String]) -> R<Value> {
    run_at(p, check_id, unit, argv, &p.top.clone())
}

/// The same, in the working copy (or folder of working copies) `top`.
pub fn run_at(
    p: &Project,
    check_id: &str,
    unit: Option<&str>,
    argv: &[String],
    top: &Path,
) -> R<Value> {
    run_with(p, check_id, unit, argv, top, None)
}

/// The check the engine attached to a unit (R-76), as a record-shaped value: it has no record file, the
/// list that applies to the unit says what it is.
fn attached_record(p: &Project, check_id: &str, unit: Option<&str>) -> R<Value> {
    let Some(u) = unit else {
        return Err(refuse(
            format!("run evidence for {check_id}"),
            "a check every unit must pass is run for one unit: name it with `--unit`",
            &[
                "Run `bf evidence run <check> --unit <U> -- <its command>`; the unit's brief gives the exact line.",
            ],
            "The command prints a receipt with exit 0.",
        ));
    };
    let st = events::replay(p)?;
    let pl = crate::plan::load(p)?;
    match crate::checks::attached_by_id(p, &pl, &st, u, check_id) {
        Some(a) => Ok(json!({"id": check_id, "kind": "check", "how": "command",
            "command": a.command, "heavy": a.heavy})),
        None => Err(refuse(
            format!("run evidence for {check_id}"),
            format!(
                "no check of that name applies to {u}: the list that applies to it does not hold it"
            ),
            &["The unit's brief lists the checks that apply to it and where they come from."],
            "`bf brief <U> --role builder` lists the check.",
        )),
    }
}

/// A deliberate repeat: the number of the attempt and the reason for it.
pub type Repeat<'a> = Option<(u64, &'a str)>;

/// The sealed run receipts of a check for one command at one content: the runs already recorded.
fn earlier_runs<'a>(
    receipts: &'a [Receipt],
    check_id: &str,
    command: &str,
    content: &str,
) -> Vec<&'a Receipt> {
    receipts
        .iter()
        .filter(|r| {
            r.is_run()
                && r.seal_ok
                && r.check() == check_id
                && r.content() == content
                && s(&r.v, "command") == command
        })
        .collect()
}

/// True when the command already ran for the check on the content now in `top` and passed: the engine
/// need not run it again (R-32).
pub fn passed_on_content(p: &Project, check_id: &str, argv: &[String], top: &Path) -> R<bool> {
    let content = p.content_hash(top)?;
    let receipts = load_receipts(p)?;
    Ok(earlier_runs(&receipts, check_id, &argv.join(" "), &content)
        .iter()
        .any(|r| r.passes()))
}

/// The number the next run of this command has on the content now in `top`: the runs recorded plus one.
pub fn next_attempt(p: &Project, check_id: &str, argv: &[String], top: &Path) -> R<u64> {
    let content = p.content_hash(top)?;
    let receipts = load_receipts(p)?;
    Ok(earlier_runs(&receipts, check_id, &argv.join(" "), &content).len() as u64 + 1)
}

/// R-57: the same command on the same content as a recorded run is refused, unless it is stated as
/// a deliberate repeat with the number of the attempt (one more than the runs recorded) and a reason.
fn refuse_repeat(
    p: &Project,
    check_id: &str,
    argv: &[String],
    content: &str,
    repeat: Repeat,
    unit: Option<&str>,
) -> R<()> {
    let receipts = load_receipts(p)?;
    let command = argv.join(" ");
    let n = earlier_runs(&receipts, check_id, &command, content).len() as u64;
    let unit_flag = unit.map(|u| format!(" --unit {u}")).unwrap_or_default();
    let how = format!(
        "bf evidence run {check_id}{unit_flag} --attempt {} --reason \"<why it is run again>\" -- {command}",
        n + 1
    );
    match repeat {
        None if n == 0 => Ok(()),
        None => Err(refuse(
            format!("run `{command}` for {check_id} again"),
            format!(
                "it already ran {n} time(s) on this same content, and a run again on the same content is a repeat that must be stated"
            ),
            &[
                "A run on changed content is not a repeat; if nothing changed, the earlier receipt already says what happened.",
                &format!(
                    "To repeat on purpose, give the number of the attempt and a reason: `{how}`."
                ),
            ],
            "The command writes a receipt that records the attempt number and the reason.",
        )),
        Some((a, reason)) => {
            if a != n + 1 {
                return Err(refuse(
                    format!("run `{command}` for {check_id} as attempt {a}"),
                    format!(
                        "it ran {n} time(s) on this content, so the next attempt is {}",
                        n + 1
                    ),
                    &[&format!("Run it as `{how}`.")],
                    "The command writes a receipt that records the attempt number and the reason.",
                ));
            }
            if n > 0 && reason.trim().is_empty() {
                return Err(refuse(
                    format!("repeat `{command}` for {check_id}"),
                    "a deliberate repeat needs a reason",
                    &[&format!("Run it as `{how}`.")],
                    "The command writes a receipt that records the attempt number and the reason.",
                ));
            }
            Ok(())
        }
    }
}

pub fn run_with(
    p: &Project,
    check_id: &str,
    unit: Option<&str>,
    argv: &[String],
    top: &Path,
    repeat: Repeat,
) -> R<Value> {
    if argv.is_empty() {
        return Err(refuse(
            "run evidence",
            "no command was given after `--`",
            &[
                "Put the check's command after `--`, for example `bf evidence run X-1 -- cargo test`.",
            ],
            "The command prints a receipt with exit 0.",
        ));
    }
    let chk = if check_id.starts_with(crate::checks::ATTACHED_PREFIX) {
        attached_record(p, check_id, unit)?
    } else {
        p.require_record(check_id)?
    };
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
    // R-62: a unit that names its tests has them in the suite's place. The landing runs the list's own suite
    // command for a unit of its turn, so that command is accepted for that unit's attached suite check too.
    let full_suite = LANDING_RUN.load(std::sync::atomic::Ordering::SeqCst)
        && check_id.starts_with(crate::checks::ATTACHED_PREFIX)
        && unit.is_some_and(|u| {
            let (Ok(st), Ok(pl)) = (events::replay(p), crate::plan::load(p)) else {
                return false;
            };
            crate::checks::attached_by_id(p, &pl, &st, u, check_id)
                .is_some_and(|a| a.kind == "suite")
                && listed_command(p, &pl, &st, u, check_id).as_deref() == Some(argv)
        });
    // Under the landing acceptance the command that runs is the list's, so that is the one tested.
    let operator = if full_suite {
        shell_operator_word(argv, false)
    } else {
        record_operator_word(&chk)
    };
    if let Some(w) = operator {
        return Err(refuse(
            format!("run evidence for {check_id}"),
            operator_message(check_id, &w),
            &[
                "Split the check into one check per program, or use the list form (`command = [...]`) for an argument with a space.",
                "To change what the check runs, change the record (the human's step).",
            ],
            "The check's command holds no shell operator, and `bf evidence run` writes a receipt.",
        ));
    }
    {
        let st = events::replay(p)?;
        let mut drafts = vec![chk.clone()];
        if let Some(u) = unit {
            drafts.extend(p.get_record(u)?);
        }
        for d in &drafts {
            if let Some(a) = crate::gates::unapproved_change(&st, d) {
                return Err(crate::gates::draft_refusal(
                    &st,
                    &crate::strings::t("cli.amend.draft_what.run_evidence", &[("check", check_id)]),
                    s(d, "id"),
                    &a,
                ));
            }
        }
    }
    if want != argv && !full_suite {
        return Err(refuse(
            format!("run `{}` as evidence for {check_id}", argv.join(" ")),
            format!(
                "the check defines its own command: `{}`; a receipt for another command would prove nothing about this check",
                want.join(" ")
            ),
            &[
                "Run exactly the command stored in the check record, after `--`.",
                "To change what the check runs, change the record (the human's step).",
            ],
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
    let top = top.to_path_buf();
    let before = p.content_hash(&top)?;
    refuse_out_of_order(p, &chk, check_id, unit, &before)?;
    refuse_repeat(p, check_id, argv, &before, repeat, unit)?;
    // A command check whose command is exactly a scan runs in the engine's process: the sandbox keeps
    // the sealing key from the command a worker passes, so the scan could not write its own receipt there.
    if let Some(sub) = crate::scan::scan_sub(argv) {
        let v = crate::scan::run_as_check(p, sub, check_id, unit, argv, &top)?;
        if v["exit"].as_i64() != Some(0) {
            if let Some(u) = unit {
                crate::run::park_if_looping(p, u)?;
            }
        }
        return Ok(v);
    }
    // R-32: one command runs once on one content. A run of the same command in the same repositories,
    // folder and sandbox, for any check of this intent, counts here too, a failing one as well.
    let key = run_key(p, argv, &before, &top);
    if repeat.is_none() {
        let receipts = load_receipts(p)?;
        if let Some(src) = find_run(&receipts, &key) {
            let src = src.v.clone();
            return counted_receipt(p, check_id, unit, &src, &key);
        }
    }
    let tail_bytes = p.policy.evidence.tail_bytes;
    let program_note = crate::programs::verify(p, check_id, &argv[0])?;
    if let Some(n) = &program_note {
        eprintln!("note: {n}");
    }
    // R-63: a heavy check takes a place first and waits its turn; the wait is no attempt.
    let _place = if chk["heavy"].as_bool() == Some(true) {
        take_place(p, check_id, unit)?
    } else {
        None
    };
    // D-110: the machine's load at the start of the run is written into the receipt; a busy machine
    // is a warning only, and the check runs all the same.
    let (load, cores) = machine_load();
    if load_is_high(load, cores) {
        eprintln!(
            "{}",
            crate::strings::t(
                "cli.evidence.load_warning",
                &[
                    ("load", &format!("{load:.2}")),
                    ("cores", &cores.to_string()),
                    ("check", check_id),
                ]
            )
        );
    }
    let started = std::time::Instant::now();
    // TH-11: a heavy check runs under the time limit of a check; its place is freed when it ends.
    let limit = (chk["heavy"].as_bool() == Some(true)).then(|| heavy_time_limit(p));
    fs::create_dir_all(p.receipts_dir())?;
    // R-31: the ignore file exists before the pending output does, so an interrupted run leaves nothing
    // that a `git add -A` would commit.
    write_output_ignore(p);
    let pending = p
        .receipts_dir()
        .join(format!(".output-{}.txt", std::process::id()));
    let (exit, tail, out_hash, total, sandboxed) =
        exec_capture_keep(p, &top, argv, tail_bytes, Some(&pending), limit)?;
    drop(_place);
    let after = p.content_hash(&top)?;
    if before != after {
        let _ = fs::remove_file(&pending);
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
    // R-31: a failed run keeps its whole output and names every failing test.
    let (mut failing, mut output_file) = (Vec::new(), String::new());
    if exit != 0 {
        let whole = fs::read(&pending).unwrap_or_default();
        failing = failing_tests(&String::from_utf8_lossy(&whole));
        let name = format!("output-{out_hash}.txt");
        if fs::rename(&pending, p.receipts_dir().join(&name)).is_ok() {
            // Stored relative to the project root, so a committed receipt names no home folder.
            output_file = format!("factory/receipts/{name}");
            keep_outputs_bounded(p);
        }
    } else {
        let _ = fs::remove_file(&pending);
    }
    let (attempt, repeat_reason) = match repeat {
        Some((a, r)) => (json!(a), r.trim().to_string()),
        None => (Value::Null, String::new()),
    };
    let v = json!({
        "type": "run",
        "check": check_id,
        "unit": unit,
        "attempt": attempt,
        "reason": repeat_reason,
        "command": argv.join(" "),
        "program": CHECK_SHELL,
        "sandbox": if sandboxed { "sandbox-exec" } else { "none" },
        "program_note": program_note.clone().unwrap_or_default(),
        "exit": exit,
        "content_hash": before,
        "output_hash": out_hash,
        "bytes": total,
        "output_tail": tail,
        "output_file": output_file,
        "failing_tests": failing.iter().map(|(n, h)| json!({"name": n, "head": h})).collect::<Vec<_>>(),
        "at": util::now(),
        "seconds": started.elapsed().as_secs_f64(),
        "load": (load * 100.0).round() / 100.0,
        "cores": cores,
        "working_copy": key.folder,
        "repos": key.repos,
        "folder": key.folder,
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.run",
        json!({"receipt": v["id"], "check": check_id, "unit": unit, "exit": exit}),
    )?;
    if exit != 0 {
        print_failure(p, check_id, &v);
        if let Some(u) = unit {
            crate::run::park_if_looping(p, u)?;
        }
    }
    Ok(v)
}

/// D-110: a run warns when the one-minute load is at or above this share of the cores (one half).
/// There is no policy key and no refusal for load.
const LOAD_WARN_NUMERATOR: f64 = 1.0;
const LOAD_WARN_DENOMINATOR: f64 = 2.0;

/// The one-minute load average and the number of cores. A test build reads the load from
/// `BF_TEST_LOAD` when it is set (C-3).
fn machine_load() -> (f64, u64) {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(1);
    #[cfg(feature = "test_seam")]
    if let Some(l) = std::env::var("BF_TEST_LOAD")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
    {
        return (l, cores);
    }
    let mut avg = [0f64; 1];
    // SAFETY: the call writes at most one value into the array it is given.
    let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 1) };
    (if n >= 1 { avg[0] } else { 0.0 }, cores)
}

/// Whether the load is at or above half the cores.
fn load_is_high(load: f64, cores: u64) -> bool {
    load * LOAD_WARN_DENOMINATOR >= cores as f64 * LOAD_WARN_NUMERATOR
}

/// The line `verify` and a printed receipt show for the load of a run; empty for a receipt without one.
pub fn load_line(v: &Value) -> String {
    match (v["load"].as_f64(), v["cores"].as_u64()) {
        (Some(l), Some(c)) => crate::strings::t(
            "cli.evidence.load_line",
            &[("load", &format!("{l:.2}")), ("cores", &c.to_string())],
        ),
        _ => String::new(),
    }
}

/// What makes two runs the same run (TH-13): the command text, the content, the repositories, the
/// folder the command ran in and whether it ran in the sandbox.
#[derive(Clone, Debug, PartialEq)]
pub struct RunKey {
    pub command: String,
    pub content: String,
    pub repos: String,
    pub folder: String,
    pub sandbox: String,
}

/// The key a run of `argv` in `top` at `content` has now.
pub fn run_key(p: &Project, argv: &[String], content: &str, top: &Path) -> RunKey {
    let sandboxed = p.policy.exec.sandbox && crate::sandbox::available();
    RunKey {
        command: argv.join(" "),
        content: content.to_string(),
        repos: p
            .repos
            .iter()
            .map(|r| match r.path.strip_prefix(&p.root) {
                Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
                Ok(rel) => rel.to_string_lossy().to_string(),
                // A repository outside the project root is named, never given by its absolute path.
                Err(_) => r.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(","),
        folder: top
            .strip_prefix(&p.root)
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or_default(),
        sandbox: if sandboxed { "sandbox-exec" } else { "none" }.to_string(),
    }
}

/// H-15: the newest sealed run receipt of a command on a content in the same repositories, folder and
/// sandbox, passing or failing, whatever check it was written under.
pub fn find_run<'a>(receipts: &'a [Receipt], key: &RunKey) -> Option<&'a Receipt> {
    receipts.iter().rev().find(|r| {
        r.is_run()
            && r.seal_ok
            && !r.stale
            && r.v.get("repos").is_some()
            && r.content() == key.content
            && s(&r.v, "command") == key.command
            && s(&r.v, "repos") == key.repos
            && s(&r.v, "folder") == key.folder
            && s(&r.v, "sandbox") == key.sandbox
    })
}

/// A receipt for a check that stands for the run of another check with the same command on the same
/// content: no command runs, and no attempt is counted.
fn counted_receipt(
    p: &Project,
    check_id: &str,
    unit: Option<&str>,
    src: &Value,
    key: &RunKey,
) -> R<Value> {
    let v = json!({
        "type": "run",
        "check": check_id,
        "unit": unit,
        "command": key.command,
        "program": CHECK_SHELL,
        "sandbox": key.sandbox,
        "program_note": "",
        "exit": src["exit"],
        "content_hash": key.content,
        "output_hash": src["output_hash"],
        "bytes": src["bytes"],
        "output_tail": src["output_tail"],
        "output_file": src["output_file"],
        "failing_tests": src["failing_tests"],
        "at": util::now(),
        "seconds": 0.0,
        "working_copy": key.folder,
        "repos": key.repos,
        "folder": key.folder,
        "counted_from": src["id"],
        "load": src["load"],
        "cores": src["cores"],
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.counted",
        json!({"receipt": v["id"], "check": check_id, "unit": unit, "from": src["id"], "exit": src["exit"]}),
    )?;
    eprintln!(
        "{}",
        crate::strings::t(
            "cli.evidence.counted",
            &[
                ("check", check_id),
                ("from", s(src, "id")),
                ("source", s(src, "check")),
            ]
        )
    );
    if v["exit"].as_i64() != Some(0) {
        print_failure(p, check_id, &v);
    }
    Ok(v)
}

/// R-33: a check every unit must pass is refused while one before it in the list has no passing
/// receipt on the content; a check that `needs` others is refused while one of them has none.
fn refuse_out_of_order(
    p: &Project,
    chk: &Value,
    check_id: &str,
    unit: Option<&str>,
    content: &str,
) -> R<()> {
    let needs = str_list(chk, "needs");
    let attached = check_id.starts_with(crate::checks::ATTACHED_PREFIX);
    if needs.is_empty() && !attached {
        return Ok(());
    }
    let receipts = load_receipts(p)?;
    let earlier: Option<String> = if attached {
        match unit {
            Some(u) => {
                let st = events::replay(p)?;
                let pl = crate::plan::load(p)?;
                if LANDING_RUN.load(std::sync::atomic::Ordering::SeqCst) {
                    // A landing run puts the list's own suite command where a unit's named tests stand.
                    let list = crate::checks::attached_for_unit(p, &pl, &st, u).0;
                    let at = list.iter().position(|a| a.id() == check_id);
                    at.and_then(|at| {
                        list.iter().take(at).find(|a| {
                            let want = listed_command(p, &pl, &st, u, &a.id())
                                .unwrap_or_else(|| a.command.clone())
                                .join(" ");
                            !receipts.iter().any(|r| {
                                r.check() == a.id()
                                    && r.content() == content
                                    && r.passes()
                                    && s(&r.v, "command") == want
                            })
                        })
                    })
                    .map(|a| a.id())
                } else {
                    crate::checks::earlier_unpassed(p, &pl, &st, &receipts, u, check_id, content)
                        .map(|a| a.id())
                }
            }
            None => None,
        }
    } else {
        None
    };
    let earlier = earlier.or_else(|| {
        needs
            .into_iter()
            .find(|n| satisfied_unit_at(&receipts, n, content).is_none())
    });
    match earlier {
        None => Ok(()),
        Some(e) => Err(crate::refusal::from_key(
            "cli.evidence.order",
            &[("check", check_id), ("earlier", &e)],
        )),
    }
}

/// The failing tests in the whole output of a run, each with the first lines of its own output. Reads
/// the lines of `cargo nextest` and of `cargo test`.
pub fn failing_tests(output: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = output.lines().collect();
    let mut names: Vec<String> = Vec::new();
    for l in &lines {
        let t = l.trim();
        let name = if t.starts_with("FAIL [")
            || t.starts_with("TIMEOUT [")
            || t.starts_with("SIGABRT [")
        {
            t.split_once("] ")
                .and_then(|(_, rest)| rest.split_whitespace().last())
                .map(|n| n.to_string())
        } else if t.starts_with("test ") && t.ends_with("... FAILED") {
            t.strip_prefix("test ")
                .and_then(|r| r.strip_suffix("... FAILED"))
                .map(|n| n.trim().to_string())
        } else {
            None
        };
        if let Some(n) = name {
            if !n.is_empty() && !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names
        .into_iter()
        .map(|n| {
            let at = lines.iter().position(|l| {
                let t = l.trim();
                t.starts_with("---")
                    && t.contains(&n)
                    && (t.contains("STDERR") || t.contains("STDOUT") || t.contains("stdout"))
            });
            let head = at
                .map(|i| {
                    lines[i + 1..]
                        .iter()
                        .take_while(|l| !l.trim().starts_with("---") && !l.trim().is_empty())
                        .take(6)
                        .copied()
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            (n, head)
        })
        .collect()
}

/// The time limit of a heavy check: the idle limit of the policy, so a hung run frees its place. The
/// seconds in `BF_HEAVY_LIMIT_SECONDS` can only shorten it (for a test), never lengthen or lift it.
fn heavy_time_limit(p: &Project) -> std::time::Duration {
    let policy = p.policy.limits.idle_minutes.max(1) * 60;
    let env = std::env::var("BF_HEAVY_LIMIT_SECONDS").ok();
    std::time::Duration::from_secs(shortened_limit(policy, env.as_deref()))
}

/// The policy's limit, or the shorter one a test asked for; a longer or unusable value changes nothing.
fn shortened_limit(policy: u64, asked: Option<&str>) -> u64 {
    asked
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n >= 1)
        .map_or(policy, |n| n.min(policy))
}

/// How many whole-output files of failed runs stay in the receipts folder.
const KEPT_OUTPUTS: usize = 40;

/// The whole outputs are local files, not part of a commit: the folder ignores them.
fn write_output_ignore(p: &Project) {
    let ignore = p.receipts_dir().join(".gitignore");
    if !ignore.exists() {
        let _ = fs::write(&ignore, "output-*.txt\n.output-*.txt\n");
    }
}

/// Only the newest whole outputs stay.
fn keep_outputs_bounded(p: &Project) {
    let dir = p.receipts_dir();
    write_output_ignore(p);
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("output-") && n.ends_with(".txt")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort();
    let extra = files.len().saturating_sub(KEPT_OUTPUTS);
    for (_, f) in files.into_iter().take(extra) {
        let _ = fs::remove_file(f);
    }
}

/// What a failed run prints: every failing test with the first lines of its output, and the path of the
/// whole output (R-31).
fn print_failure(p: &Project, check_id: &str, v: &Value) {
    let tests = v["failing_tests"].as_array().cloned().unwrap_or_default();
    if !tests.is_empty() {
        eprintln!(
            "{}",
            crate::strings::t(
                "cli.evidence.failed.tests",
                &[("check", check_id), ("count", &tests.len().to_string())]
            )
        );
        for t in &tests {
            eprintln!(
                "{}",
                crate::strings::t("cli.evidence.failed.test", &[("name", s(t, "name"))])
            );
            for l in s(t, "head").lines() {
                eprintln!("      {l}");
            }
        }
    }
    let file = s(v, "output_file");
    if !file.is_empty() {
        eprintln!(
            "{}",
            crate::strings::t(
                "cli.evidence.failed.output",
                &[("path", &p.root.join(file).to_string_lossy())]
            )
        );
    }
}

// R-63, D-59: the places of heavy checks.

/// A place held by this process for a heavy check; freed when dropped, or when the process dies.
pub struct Place {
    _file: fs::File,
    path: std::path::PathBuf,
}

// The place file is not unlinked when the place is freed: unlocking frees it. A path unlinked while
// another process under the gate has opened it would let that process lock a file nobody counts.
impl Place {
    /// The file that names the holder.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// The folder of the places, counted for the whole workspace on the machine: next to the state folders
/// of every project.
pub fn places_dir(p: &Project) -> std::path::PathBuf {
    p.state_dir()
        .parent()
        .map(|d| d.join("places"))
        .unwrap_or_else(|| p.state_dir().join("places"))
}

fn holder_line(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    crate::strings::t(
        "cli.evidence.heavy.holder",
        &[
            ("pid", &v["pid"].to_string()),
            ("intent", s(&v, "intent")),
            ("check", s(&v, "check")),
        ],
    )
}

fn held(path: &Path) -> bool {
    match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => matches!(f.try_lock(), Err(fs::TryLockError::WouldBlock)),
        Err(_) => false,
    }
}

fn write_holder(file: &mut fs::File, p: &Project, check_id: &str, extra: Value) {
    use std::io::{Seek, Write};
    let mut v =
        json!({"pid": std::process::id(), "intent": p.config.project_id, "check": check_id});
    if let Value::Object(m) = extra {
        for (k, x) in m {
            v[k] = x;
        }
    }
    let _ = file.set_len(0);
    let _ = file.rewind();
    let _ = file.write_all(v.to_string().as_bytes());
}

fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Take a place for a heavy check, waiting in turn when the plan's limit is reached. None when the plan
/// states no limit. A waiting check is served before one that came later.
pub fn take_place(p: &Project, check_id: &str, unit: Option<&str>) -> R<Option<Place>> {
    let Some(limit) = crate::plan::load(p)?.heavy_limit() else {
        return Ok(None);
    };
    let dir = places_dir(p);
    fs::create_dir_all(&dir)?;
    let gate_path = dir.join(".gate");
    // Our turn: a ticket named by the time we came, held until we are served.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ticket_name = format!("wait-{nanos:030}-{}", std::process::id());
    let ticket_path = dir.join(&ticket_name);
    let mut ticket = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&ticket_path)?;
    let _ = ticket.try_lock();
    write_holder(&mut ticket, p, check_id, json!({"since": epoch()}));
    let mut told = false;
    let mut told_long = false;
    let began = std::time::Instant::now();
    // The time spent waiting is no idle time: a waiting event now and then is a sign of life of the unit.
    let beat = std::time::Duration::from_secs((p.policy.limits.idle_minutes * 60 / 4).max(1));
    let mut last_beat: Option<std::time::Instant> = None;
    loop {
        // A short lock around the count and the take.
        let gate = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&gate_path)?;
        while !matches!(gate.try_lock(), Ok(())) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut places: Vec<std::path::PathBuf> = vec![];
        let mut tickets: Vec<std::path::PathBuf> = vec![];
        for e in fs::read_dir(&dir)?.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("place-") {
                places.push(e.path());
            } else if n.starts_with("wait-") && n < ticket_name {
                tickets.push(e.path());
            }
        }
        let busy: Vec<&std::path::PathBuf> = places.iter().filter(|x| held(x)).collect();
        let earlier_waiting = tickets.iter().any(|t| held(t));
        if (busy.len() as u64) < limit && !earlier_waiting {
            let mut i = 1;
            loop {
                let path = dir.join(format!("place-{i}"));
                let mut f = fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(&path)?;
                if f.try_lock().is_ok() {
                    write_holder(&mut f, p, check_id, json!({"since": epoch()}));
                    drop(gate);
                    drop(ticket);
                    let _ = fs::remove_file(&ticket_path);
                    return Ok(Some(Place { _file: f, path }));
                }
                i += 1;
            }
        }
        let holder = busy.first().map(|h| holder_line(h)).unwrap_or_else(|| {
            tickets
                .iter()
                .find(|t| held(t))
                .map(|t| holder_line(t))
                .unwrap_or_default()
        });
        drop(gate);
        if !told {
            eprintln!(
                "{}",
                crate::strings::t(
                    "cli.evidence.heavy.waits",
                    &[
                        ("check", check_id),
                        ("limit", &limit.to_string()),
                        ("holder", &holder)
                    ]
                )
            );
            told = true;
        }
        if !told_long && began.elapsed().as_secs() >= LONG_WAIT_SECONDS {
            eprintln!(
                "{}",
                crate::strings::info(&crate::strings::t(
                    "cli.evidence.heavy.waits_long",
                    &[("check", check_id), ("holder", &holder)]
                ))
            );
            told_long = true;
        }
        if let Some(u) = unit {
            if last_beat.is_none_or(|t| t.elapsed() >= beat) {
                events::append(
                    p,
                    "evidence.wait",
                    json!({"unit": u, "check": check_id, "limit": limit}),
                )?;
                last_beat = Some(std::time::Instant::now());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A heavy check that has waited this long is told as an information line (D-59).
pub const LONG_WAIT_SECONDS: u64 = 600;

/// The information lines for the heavy checks that have waited longer than ten minutes, read live from
/// the ticket files, with the holder named. Given as data for `bf status`, `bf waiting` and `bf doctor`.
pub fn long_waits(p: &Project) -> Vec<String> {
    let dir = places_dir(p);
    let now = epoch();
    let mut holders: Vec<String> = vec![];
    let mut waiting: Vec<(String, u64)> = vec![];
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if !held(&e.path()) {
            continue;
        }
        let v: Value = serde_json::from_str(&fs::read_to_string(e.path()).unwrap_or_default())
            .unwrap_or(Value::Null);
        if n.starts_with("place-") {
            holders.push(holder_line(&e.path()));
        } else if n.starts_with("wait-") {
            waiting.push((
                s(&v, "check").to_string(),
                v["since"].as_u64().unwrap_or(now),
            ));
        }
    }
    let holder = holders.join(", ");
    waiting
        .into_iter()
        .filter(|(_, since)| now.saturating_sub(*since) >= LONG_WAIT_SECONDS)
        .map(|(check, _)| {
            crate::strings::info(&crate::strings::t(
                "cli.evidence.heavy.waits_long",
                &[("check", &check), ("holder", &holder)],
            ))
        })
        .collect()
}

/// Run a command with stdout and stderr merged. Keep a bounded tail and hash all of it.
fn shell_quote(a: &str) -> String {
    format!("'{}'", a.replace('\'', "'\\''"))
}

/// The environment a check runs in: nothing of the caller's except a short list, and the PATH from the
/// policy. A caller cannot put a fake `sh` or `cargo` first on the path, or point git elsewhere.
pub const CHECK_ENV: &[&str] = &[
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

pub(crate) fn exec_capture(
    p: &Project,
    dir: &Path,
    argv: &[String],
    tail_bytes: usize,
) -> R<(i64, String, String, u64, bool)> {
    exec_capture_keep(p, dir, argv, tail_bytes, None, None)
}

/// The same, and the whole output is also written to `keep` when one is given (R-31).
pub(crate) fn exec_capture_keep(
    p: &Project,
    dir: &Path,
    argv: &[String],
    tail_bytes: usize,
    keep: Option<&Path>,
    limit: Option<std::time::Duration>,
) -> R<(i64, String, String, u64, bool)> {
    let mut keep_file = match keep {
        Some(k) => Some(fs::File::create(k)?),
        None => None,
    };
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
        let repo_paths = p
            .repos
            .iter()
            .map(|r| r.path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let mut profile = crate::sandbox::check_profile(
            &p.state_dir_resolved(),
            &p.root.to_string_lossy(),
            &dir.to_string_lossy(),
            p.policy.exec.network,
            &p.transcript_root(),
            &repo_paths,
        );
        if crate::sandbox::is_review_copy(&p.work_dir(), dir) {
            profile = crate::sandbox::review_profile(&profile, &repo_paths);
        }
        crate::sandbox::command(&profile, CHECK_SHELL)
    } else {
        Command::new(CHECK_SHELL)
    };
    if limit.is_some() {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
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
    // After the loop, so whatever the policy lists, the account's home is what the check sees.
    crate::session::home::set_env(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        refuse(
            format!("run `{}`", argv.join(" ")),
            format!("the command could not be started: {e}"),
            &["Check that the program exists and is on PATH."],
            "The command starts and a receipt is written.",
        )
    })?;
    drop(cmd);
    // The limit: a watcher stops the whole process group of the command when the time is up.
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = limit.map(|lim| {
        let (done, timed_out, pid) = (done.clone(), timed_out.clone(), child.id());
        std::thread::spawn(move || {
            let began = std::time::Instant::now();
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                if began.elapsed() >= lim {
                    timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                    let _ = Command::new("/bin/kill")
                        .args(["-KILL", "--", &format!("-{pid}")])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        })
    });
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
        if let Some(f) = keep_file.as_mut() {
            use std::io::Write;
            f.write_all(&buf[..n])?;
        }
        total += n as u64;
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > tail_bytes * 2 {
            let cut = tail.len() - tail_bytes;
            tail.drain(..cut);
        }
    }
    let status = child.wait()?;
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(w) = watcher {
        let _ = w.join();
    }
    if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
        let note = crate::strings::t(
            "cli.evidence.heavy.timeout",
            &[(
                "seconds",
                &limit
                    .map(|l| l.as_secs_f64().ceil() as u64)
                    .unwrap_or(0)
                    .to_string(),
            )],
        );
        tail.extend_from_slice(format!("\n{note}\n").as_bytes());
    }
    if tail.len() > tail_bytes {
        let cut = tail.len() - tail_bytes;
        tail.drain(..cut);
    }
    let exit = if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
        124
    } else {
        status.code().map(|c| c as i64).unwrap_or(-1)
    };
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
        let home = crate::session::home::account_home()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default();
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
    /// The amendment and the kind of review an outcome on a design review is for, when no token says.
    pub amendment: Option<&'a str>,
    pub kind: Option<&'a str>,
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
            format!(
                "the file is {} bytes; the policy limit is {}",
                meta.len(),
                p.policy.evidence.max_attach_bytes
            ),
            &[
                "Attach a smaller file, or ask the human to raise `evidence.max_attach_bytes` in factory/policy.toml.",
            ],
            "The command prints a receipt.",
        ));
    }
    let says = j.says.trim();
    let mut review: Option<crate::checks::review::Attach> = None;
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
            ));
        }
    };
    if j.outcome.is_some() {
        refuse_review_check(&chk, "give an outcome")?;
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
                &[
                    "Say what you looked at and what you saw: `--says \"I opened the page at 375px and read every label\"`.",
                ],
                "The command prints a receipt.",
            ));
        }
        let named = crate::checks::by_name(s(&chk, "named"));
        let design_stage = named
            .map(|c| c.stage == crate::checks::Stage::Design)
            .unwrap_or(false);
        // A design review has its own rules for the token, the hold of open findings and the park.
        let st = events::replay(p)?;
        review = crate::checks::review::on_attach(
            p,
            &st,
            &chk,
            j.outcome.unwrap_or(""),
            j.token,
            j.amendment,
            j.kind,
        )?;
        if p.policy.roles.tokens && review.is_none() {
            // A design-stage named check is settled with the token of a design-stage role of its
            // family (`design:<role>`), issued by `bf roles token design <role>`. Any other check:
            // the verifier's token for a checkpoint check; the reviewer's for a unit's own check; the
            // token of a built-stage role of the check's family on that checkpoint or unit. A builder
            // holds none of them, so it cannot settle a check for its own unit.
            let pl = crate::plan::load(p)?;
            let family_roles = |design: bool| -> Vec<&'static str> {
                crate::roles::ROLES
                    .iter()
                    .filter(|r| {
                        named.map(|c| c.family.name() == r.family).unwrap_or(false)
                            && if design {
                                r.at_design()
                            } else {
                                r.stage == "built"
                            }
                    })
                    .map(|r| r.name)
                    .collect()
            };
            let mut keys: Vec<String> = Vec::new();
            if design_stage {
                for r in family_roles(true) {
                    keys.push(format!("{}:{r}", crate::roles::DESIGN));
                }
            } else {
                for k in pl
                    .checkpoints
                    .iter()
                    .filter(|k| str_list(k, "checks").iter().any(|c| c == check_id))
                {
                    keys.push(format!("{}:verifier", s(k, "id")));
                    for r in family_roles(false) {
                        keys.push(format!("{}:{r}", s(k, "id")));
                    }
                }
                if !s(&chk, "unit").is_empty() {
                    keys.push(format!("{}:reviewer", s(&chk, "unit")));
                    for r in family_roles(false) {
                        keys.push(format!("{}:{r}", s(&chk, "unit")));
                    }
                }
            }
            if !keys.iter().any(|k| crate::run::holds_token(p, k, j.token)) {
                return Err(refuse(
                    format!("give {check_id} an outcome"),
                    "an outcome on a check judged by an agent is accepted only with the verifier's token (the reviewer's, for a check of one unit; the token of the role of the check's family; for a design-stage check, the design-stage role's token); none, or the wrong one, was given",
                    &[
                        "The conductor runs `bf next --json`; the verifier's directive for the checkpoint carries its `token`. A design-stage check takes the token from `bf roles token design <role>`.",
                        "The verifier passes it: `bf evidence attach <check> <file> --outcome met --says \"...\" --token <token>`. A builder cannot settle a check for its own unit.",
                    ],
                    "`bf evidence verify` lists the outcome.",
                ));
            }
        }
    }
    // A UI/UX verification takes its evidence step by step: a sentence that names a journey step (or a
    // screen state) makes a step receipt; any other `met` is the verification's own and needs every
    // step covered first.
    let uiux_name = matches!(
        s(&chk, "named"),
        "design-uiux-verification" | "built-uiux-verification"
    );
    let mut step_fields: Option<(String, String)> = None;
    // TH-6: no outcome of the built-result verification rests on a file from the built result.
    if s(&chk, "named") == "built-uiux-verification" && j.outcome.is_some() {
        crate::uiux::refuse_evidence_from_built_result(p, file)?;
    }
    if let (true, Some(o)) = (uiux_name, j.outcome) {
        if let Some((subject, n, _)) = crate::uiux::parse_step(says) {
            crate::uiux::require_step_exists(p, &subject, &n)?;
            step_fields = Some((subject, n));
        } else if o == "met" {
            crate::uiux::require_covered(p, check_id, s(&chk, "named"))?;
        }
    }
    let h = util::sha256_file(file)?;
    fs::create_dir_all(p.evidence_dir())?;
    let dest = p.evidence_dir().join(&h);
    if !dest.exists() {
        fs::copy(file, &dest)?;
    }
    let content = p.content_hash(&p.top)?;
    let screens = if j.outcome.is_some() && s(&chk, "named") == "design-uiux-verification" {
        crate::uiux::design_hash(p)?
    } else {
        String::new()
    };
    let kind = if step_fields.is_some() { "step" } else { kind };
    let (step_journey, step_n, step_outcome) = match (&step_fields, j.outcome) {
        (Some((a, b)), Some(o)) => (a.as_str(), b.as_str(), o),
        _ => ("", "", ""),
    };
    let v = json!({
        "journey": step_journey,
        "step": step_n,
        "step_outcome": step_outcome,
        "type": kind,
        "check": check_id,
        "unit": unit,
        "screens_hash": screens,
        "reason": says,
        "file_hash": h,
        "file_name": file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        "bytes": meta.len(),
        "content_hash": content,
        "at": util::now(),
    });
    let mut v = v;
    if let Some(a) = &review {
        for (k, x) in a.fields().as_object().into_iter().flatten() {
            v[k] = x.clone();
        }
    }
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.attach",
        json!({"receipt": v["id"], "check": check_id, "unit": unit}),
    )?;
    if let Some(a) = review.as_ref().filter(|a| a.park) {
        events::append(
            p,
            crate::checks::review::PARK_EVENT,
            json!({"subject": a.subject, "round": a.round}),
        )?;
    }
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
    refuse_review_check(&chk, &format!("mark it as {kind}"))?;
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
///
/// R-58: a pass that follows a failing run at the same content counts only after the checkpoint
/// reviewer recorded a judgement of acceptable flakiness or another cause; a judgement that the
/// failure stands, or none at all, leaves the check unproven.
pub fn satisfied_at<'a>(
    receipts: &'a [Receipt],
    check_id: &str,
    content: &str,
) -> Option<&'a Receipt> {
    let here = || {
        receipts
            .iter()
            .filter(|r| r.check() == check_id && r.content() == content)
    };
    let pass = here().rev().find(|r| r.passes())?;
    let pass_n = receipt_number(pass.id()).unwrap_or(0);
    // The judgement must come after the latest failing run that precedes the pass that counts.
    let failed_first = here()
        .filter(|r| r.is_run() && r.seal_ok && r.v["exit"].as_i64() != Some(0))
        .map(|r| receipt_number(r.id()).unwrap_or(0))
        .filter(|n| *n < pass_n)
        .max();
    let last_judgement = here().rfind(|r| r.seal_ok && !s(&r.v, "judgement").is_empty());
    if let Some(j) = last_judgement {
        if s(&j.v, "judgement") == "failure" {
            return None;
        }
    }
    if let Some(f) = failed_first {
        let judged = last_judgement
            .map(|j| receipt_number(j.id()).unwrap_or(0) > f)
            .unwrap_or(false);
        if !judged {
            return None;
        }
    }
    Some(pass)
}

/// A unit's check is satisfied at `content` by a sealed passing receipt at that content. The judgement
/// rule of R-58 belongs to the checkpoint's checks, where a reviewer can record it, so it is not asked
/// here: a unit check that failed and then passed on a deliberate repeat counts.
pub fn satisfied_unit_at<'a>(
    receipts: &'a [Receipt],
    check_id: &str,
    content: &str,
) -> Option<&'a Receipt> {
    receipts
        .iter()
        .rev()
        .find(|r| r.check() == check_id && r.content() == content && r.passes())
}

/// Whether the check has a pass at this content that waits for the reviewer's judgement of an earlier
/// failure: the one case where a passing receipt does not count yet.
pub fn awaits_judgement(receipts: &[Receipt], check_id: &str, content: &str) -> bool {
    satisfied_at(receipts, check_id, content).is_none()
        && receipts
            .iter()
            .any(|r| r.check() == check_id && r.content() == content && r.passes())
        && !receipts.iter().any(|r| {
            r.check() == check_id && r.content() == content && s(&r.v, "judgement") == "failure"
        })
}

/// The judgements the checkpoint reviewer can record (R-58).
pub const JUDGEMENTS: [&str; 3] = ["flaky", "failure", "other"];

/// `bf evidence run <check> --judge flaky|failure|other --reason ...`: the checkpoint reviewer judges a
/// check that failed and then passed on a deliberate repeat at the same content. Flaky and other count
/// the pass (flaky is flagged on the receipt); a failure stands as a `fail` receipt, so the check goes
/// to the checkpoint fix.
pub fn judge(p: &Project, check_id: &str, judgement: &str, reason: &str, top: &Path) -> R<Value> {
    if !JUDGEMENTS.contains(&judgement) {
        return Err(refuse(
            format!("judge {check_id} as `{judgement}`"),
            "that is not a judgement",
            &[
                "Use `flaky` (acceptable flakiness), `failure` (the failure stands) or `other` (another cause, such as an interrupted run).",
            ],
            "The command prints a receipt.",
        ));
    }
    if reason.trim().is_empty() {
        return Err(refuse(
            format!("judge {check_id}"),
            "a judgement needs a reason",
            &["Add `--reason \"<what you saw>\"`."],
            "The command prints a receipt.",
        ));
    }
    let chk = p.require_record(check_id)?;
    if s(&chk, "kind") != "check" || s(&chk, "how") != "command" {
        return Err(refuse(
            format!("judge {check_id}"),
            "only a command check that failed and then passed is judged",
            &["Use the ID of a command check."],
            "The command prints a receipt.",
        ));
    }
    let content = p.content_hash(top)?;
    let receipts = load_receipts(p)?;
    let here: Vec<&Receipt> = receipts
        .iter()
        .filter(|r| r.check() == check_id && r.content() == content)
        .collect();
    let failed = here
        .iter()
        .any(|r| r.is_run() && r.seal_ok && r.v["exit"].as_i64() != Some(0));
    let passed = here.iter().any(|r| r.passes());
    if !(failed && passed) {
        return Err(refuse(
            format!("judge {check_id}"),
            "it has no failing run followed by a passing run at this content, so there is nothing to judge",
            &[
                "Judge only a check that failed and then passed on a deliberate repeat at the same content (`--attempt <n> --reason ...`).",
            ],
            "The command prints a receipt.",
        ));
    }
    let (kind, flag) = match judgement {
        "failure" => ("fail", ""),
        "flaky" => ("judge", "flaky"),
        _ => ("judge", ""),
    };
    let v = json!({
        "type": kind,
        "check": check_id,
        "judgement": judgement,
        "flag": flag,
        "reason": format!("{judgement}: {}", reason.trim()),
        "content_hash": content,
        "at": util::now(),
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.judged",
        json!({"receipt": v["id"], "check": check_id, "judgement": judgement}),
    )?;
    Ok(v)
}

/// `bf evidence run --probe <K> -- <command...>`: a command the checkpoint reviewer runs in its own copy
/// that is not one of the checkpoint's checks, such as a probe test. It is recorded with the command, the
/// content it ran on, the exit code and the end of its output, and it settles nothing: no check, gate or
/// close counts it as proof.
pub fn probe(p: &Project, checkpoint: &str, argv: &[String], top: &Path) -> R<Value> {
    if argv.is_empty() {
        return Err(refuse(
            "run a probe",
            "no command was given after `--`",
            &[
                "Put the command after `--`, for example `bf evidence run --probe K-1 -- cargo test probe_`.",
            ],
            "The command prints a probe receipt.",
        ));
    }
    let content = p.content_hash(top)?;
    let (exit, tail, out_hash, total, sandboxed) =
        exec_capture(p, top, argv, p.policy.evidence.tail_bytes)?;
    let after = p.content_hash(top)?;
    let v = json!({
        "type": "probe",
        "check": checkpoint,
        "command": argv.join(" "),
        "program": CHECK_SHELL,
        "sandbox": if sandboxed { "sandbox-exec" } else { "none" },
        "exit": exit,
        "content_hash": content,
        "content_after": after,
        "output_hash": out_hash,
        "bytes": total,
        "output_tail": tail,
        "at": util::now(),
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.probe",
        json!({"receipt": v["id"], "checkpoint": checkpoint, "exit": exit}),
    )?;
    Ok(v)
}

/// A run of a sync check: the engine runs one of the checks every unit of the checkpoint is held to on the
/// joined content, as part of the sync `sync` (the newest receipt number when the sync started). It is
/// recorded with its command, exit and content, and settles nothing: no unit's check and no checkpoint's
/// check counts it. The receipt's `reason` is the label of the sync check, sealed with the rest.
pub fn sync_run(
    p: &Project,
    checkpoint: &str,
    label: &str,
    argv: &[String],
    top: &Path,
    sync: u64,
) -> R<Value> {
    let content = p.content_hash(top)?;
    let (exit, tail, out_hash, total, sandboxed) =
        exec_capture(p, top, argv, p.policy.evidence.tail_bytes)?;
    let after = p.content_hash(top)?;
    let v = json!({
        "type": "sync",
        "check": checkpoint,
        "reason": label,
        "sync": sync,
        "command": argv.join(" "),
        "program": CHECK_SHELL,
        "sandbox": if sandboxed { "sandbox-exec" } else { "none" },
        "exit": exit,
        "content_hash": content,
        "content_after": after,
        "output_hash": out_hash,
        "bytes": total,
        "output_tail": tail,
        "at": util::now(),
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.sync",
        json!({"receipt": v["id"], "checkpoint": checkpoint, "check": label, "exit": exit}),
    )?;
    Ok(v)
}

/// The engine's own receipt for the checkpoint review, written when the reviewer reports a clean review:
/// the review settles the check that names it, at the content it judged.
pub fn review_receipt(p: &Project, check_id: &str, content: &str, says: &str) -> R<Value> {
    let v = json!({
        "type": "attach",
        "check": check_id,
        "reason": says,
        "content_hash": content,
        "at": util::now(),
    });
    let v = write_receipt(p, v)?;
    events::append(
        p,
        "evidence.attach",
        json!({"receipt": v["id"], "check": check_id, "unit": null}),
    )?;
    Ok(v)
}

fn refuse_review_check(chk: &Value, what: &str) -> R<()> {
    if s(chk, "named") == crate::checks::CHECKPOINT_REVIEW {
        return Err(refuse(
            format!("{what} for {}", s(chk, "id")),
            "the checkpoint review is settled only by the checkpoint reviewer's own report; no verifier or auditor settles it, and the reviewer settles no other check",
            &[
                "The checkpoint reviewer reports with `bf report <K> --result findings --role reviewer --token <token>`.",
            ],
            "`bf evidence verify` lists the review's receipt after a clean report.",
        ));
    }
    Ok(())
}

/// One check a unit's landing requires: its own check records and the checks every unit must pass.
pub struct UnitNeed<'a> {
    pub id: String,
    /// One of the checks every unit must pass, not a check record of the unit.
    pub attached: bool,
    /// The passing receipt at the content asked for, when there is one.
    pub met: Option<&'a Receipt>,
    /// The line landing prints when the check is not met.
    pub line: String,
}

/// Every check the landing of `unit` requires at `content`, each with whether it is met: the unit's own
/// check records, then the checks every unit must pass (R-76). Landing and `bf evidence verify --unit`
/// both take their list from here.
pub fn unit_landing_needs<'a>(
    p: &Project,
    pl: &crate::plan::Plan,
    st: &crate::events::State,
    receipts: &'a [Receipt],
    unit: &str,
    content: &str,
) -> Vec<UnitNeed<'a>> {
    let wt = p.work_dir().join(unit);
    let mut out = Vec::new();
    // A unit or a check changed by an amendment the human has not approved is a draft: landing waits.
    for d in pl.unit(unit).into_iter().chain(pl.unit_checks(unit)) {
        if let Some(a) = crate::gates::unapproved_change(st, d) {
            out.push(UnitNeed {
                id: s(d, "id").to_string(),
                attached: false,
                met: None,
                line: crate::gates::draft_line(st, s(d, "id"), &a),
            });
        }
    }
    // A check of the approved plan that belonged to this unit and that a draft moved away, changed or
    // deleted still holds the landing: the plan in force is what the human approved, not the draft.
    for (cid, a) in crate::gates::drafts_off_unit(st, &pl.checks, unit) {
        if out
            .iter()
            .any(|n| n.id == cid && !n.attached && n.met.is_none())
        {
            continue;
        }
        out.push(UnitNeed {
            id: cid.clone(),
            attached: false,
            met: None,
            line: crate::gates::draft_line(st, &cid, &a),
        });
    }
    for c in pl.unit_checks(unit) {
        let cid = s(c, "id");
        let met = satisfied_unit_at(receipts, cid, content);
        let stale = receipts.iter().any(|r| r.check() == cid && r.passes());
        out.push(UnitNeed {
            id: cid.to_string(),
            attached: false,
            met,
            line: format!(
                "{cid}: {} - run `bf evidence run {cid} --unit {unit} -- <its command>` inside {}",
                if stale {
                    "the receipts on file are for other content"
                } else {
                    "no passing sealed receipt"
                },
                wt.display()
            ),
        });
    }
    for a in crate::checks::attached_for_unit(p, pl, st, unit).0 {
        let met = crate::checks::attached_receipt(receipts, &a, content);
        let stale = receipts.iter().any(|r| r.check() == a.id() && r.passes());
        out.push(UnitNeed {
            id: a.id(),
            attached: true,
            met,
            line: format!(
                "{}: {} - run `bf evidence run {} --unit {unit} -- {}` inside {}",
                a.id(),
                if stale {
                    "the receipts on file are for other content or another command"
                } else {
                    "no passing sealed receipt"
                },
                a.id(),
                a.command_line(),
                wt.display()
            ),
        });
    }
    out
}

/// A check in scope whose counting pass followed a failing run of the same check at the same content.
pub struct PassAfterFailure {
    pub check: String,
    pub failing: String,
    pub passing: String,
}

/// The mark for a check met by `pass`: the latest sealed failing run at that content numbered before it.
fn pass_after_failure(
    receipts: &[Receipt],
    check: &str,
    pass: &Receipt,
) -> Option<PassAfterFailure> {
    let pass_n = receipt_number(pass.id())?;
    receipts
        .iter()
        .filter(|r| {
            r.check() == check
                && r.content() == pass.content()
                && r.is_run()
                && r.seal_ok
                && r.v["exit"].as_i64() != Some(0)
        })
        .filter_map(|r| Some((receipt_number(r.id())?, r)))
        .filter(|(n, _)| *n < pass_n)
        .max_by_key(|(n, _)| *n)
        .map(|(_, r)| PassAfterFailure {
            check: check.to_string(),
            failing: r.id().to_string(),
            passing: pass.id().to_string(),
        })
}

/// What `verify` found beyond the rows.
pub struct Verified {
    pub rows: Vec<VerifyRow>,
    pub ok: bool,
    /// With `--unit`: the checks whose pass followed a failure at the same content.
    pub pass_after_failure: Vec<PassAfterFailure>,
    /// With `--unit`: the lines of the checks landing would refuse.
    pub unmet: Vec<(String, String)>,
}

pub struct VerifyRow {
    pub receipt: String,
    pub check: String,
    pub seal_ok: bool,
    pub content_ok: Option<bool>,
    pub exit: Option<i64>,
    /// The load line of the receipt; empty for one written before the load was kept.
    pub load: String,
}

/// `bf evidence verify [--unit U] [--checkpoint K]`
pub fn verify(
    p: &Project,
    unit: Option<&str>,
    checkpoint: Option<&str>,
) -> R<(Vec<VerifyRow>, bool)> {
    let v = verify_full(p, unit, checkpoint)?;
    Ok((v.rows, v.ok))
}

pub fn verify_full(p: &Project, unit: Option<&str>, checkpoint: Option<&str>) -> R<Verified> {
    // A checkpoint's scope wins when both are given.
    let unit = if checkpoint.is_some() { None } else { unit };
    let receipts = load_receipts(p)?;
    let mut current: Option<(String, Vec<String>)> = None; // content hash and the checks in scope
    let mut pass_marks: Vec<PassAfterFailure> = Vec::new();
    let mut unmet: Vec<(String, String)> = Vec::new();
    if let Some(u) = unit {
        let dir = p.work_dir().join(u);
        let st = crate::events::replay(p)?;
        // A landed unit's working copy is removed: its content is what its `unit.land` event recorded.
        let landed = st
            .units
            .get(u)
            .filter(|x| x.state == "landed" && !x.landed_hash.is_empty() && !dir.is_dir())
            .map(|x| x.landed_hash.clone());
        let content = match landed {
            Some(h) => h,
            None => {
                let top = if dir.is_dir() { dir } else { p.top.clone() };
                p.content_hash(&top)?
            }
        };
        let pl = crate::plan::load(p)?;
        let needs = unit_landing_needs(p, &pl, &st, &receipts, u, &content);
        for n in &needs {
            match n.met {
                Some(pass) => pass_marks.extend(pass_after_failure(&receipts, &n.id, pass)),
                None => unmet.push((n.id.clone(), n.line.clone())),
            }
        }
        current = Some((content, needs.iter().map(|n| n.id.clone()).collect()));
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
        // A probe run settles nothing; it is in the evidence, never in a verification.
        // A sync run settles nothing either; it is listed apart, below.
        if matches!(s(&r.v, "type"), "probe" | "judge" | "sync") {
            continue;
        }
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
            load: load_line(&r.v),
        });
    }
    // The sync's runs of this checkpoint, apart from its own receipts: each row's check reads
    // `sync:<the sync check>`. They are shown, never counted: only a bad seal fails the verification.
    if let (Some(k), Some((h, _))) = (checkpoint, &current) {
        for r in receipts
            .iter()
            .filter(|r| s(&r.v, "type") == "sync" && r.check() == k)
        {
            if !r.seal_ok {
                all_ok = false;
            }
            rows.push(VerifyRow {
                receipt: r.id().to_string(),
                check: format!("sync:{}", s(&r.v, "reason")),
                seal_ok: r.seal_ok,
                content_ok: Some(h == r.content()),
                exit: r.v["exit"].as_i64(),
                load: load_line(&r.v),
            });
        }
    }
    // With a scope, every check in scope must have at least one passing receipt at the current content.
    if unit.is_some() {
        // Landing's own list: a check it would refuse fails the verification too.
        if !unmet.is_empty() {
            all_ok = false;
        }
    } else if let Some((h, ids)) = &current {
        for id in ids {
            // A checkpoint's checks also need the judgement of a pass that followed a failure.
            if satisfied_at(&receipts, id, h).is_none() {
                all_ok = false;
            }
        }
    }
    Ok(Verified {
        rows,
        ok: all_ok,
        pass_after_failure: pass_marks,
        unmet,
    })
}

#[cfg(test)]
mod heavy_limit_tests {
    use super::shortened_limit;

    #[test]
    fn the_variable_only_shortens_the_limit() {
        assert_eq!(shortened_limit(3600, None), 3600);
        assert_eq!(shortened_limit(3600, Some("5")), 5);
        assert_eq!(shortened_limit(3600, Some("999999")), 3600);
        assert_eq!(shortened_limit(3600, Some("0")), 3600);
        assert_eq!(shortened_limit(3600, Some("x")), 3600);
    }
}
