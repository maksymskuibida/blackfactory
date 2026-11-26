//! `bf legal`: the markets statement and the legal audit with its legal findings. This file belongs
//! to this command group alone; the command line calls its entry point.
//!
//! A legal finding is the record `L-<n>` (the fields the record kind has) plus a detail file in
//! `factory/legal/L-<n>.json` that holds what the record kind has no place for: the markets in free
//! words, the kind of risk in free words, whether research was done, the sources with their grades
//! and the lawyer mark. The engine only validates what it is given (D-18): the lawyer agent does the
//! research with the harness's tools.
//!
//! A finding that carries the lawyer mark becomes a question of the closing stage (`bf legal
//! questions`). It is acknowledged only by that question being answered `A` after a human message
//! that came after the question was asked (D-22, TH-7).

use crate::events;
use crate::refusal::{err, refuse, R};
use crate::schema;
use crate::store::{s, Project};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// The reliability grades of a source.
pub const GRADES: &[&str] = &["official", "reliable", "uncertain", "unreliable"];
/// The stage the acknowledgement questions are tagged with.
pub const CLOSE_STAGE: &str = "close";
/// The answer key that acknowledges a finding.
pub const ACK_KEY: &str = "A";

fn detail_dir(p: &Project) -> PathBuf {
    p.factory().join("legal")
}

fn detail_path(p: &Project, id: &str) -> PathBuf {
    detail_dir(p).join(format!("{id}.json"))
}

fn read_detail(p: &Project, id: &str) -> Option<Value> {
    fs::read_to_string(detail_path(p, id))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

fn write_detail(p: &Project, id: &str, v: &Value) -> R<()> {
    fs::create_dir_all(detail_dir(p))?;
    util::atomic_write(
        &detail_path(p, id),
        serde_json::to_string_pretty(v)?.as_bytes(),
    )?;
    Ok(())
}

fn text_of(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn bad(because: impl Into<String>) -> crate::refusal::Fail {
    refuse(
        "file this legal finding",
        because,
        &[
            "Give the markets and the kind of risk in your own words; say whether research was done (`researched = true` or `false`).",
            "With research, list at least one source: `url`, `title`, `read_on` (YYYY-MM-DD), `grade` (official, reliable, uncertain or unreliable) and `why` (one sentence).",
            "Run `bf legal finding <file>` again.",
        ],
        "The command prints the finding's ID.",
    )
}

fn valid_date(d: &str) -> bool {
    let b: Vec<&str> = d.split('-').collect();
    if b.len() != 3 || b[0].len() != 4 || b[1].len() != 2 || b[2].len() != 2 {
        return false;
    }
    let n: Vec<Option<u32>> = b.iter().map(|x| x.parse::<u32>().ok()).collect();
    match (n[0], n[1], n[2]) {
        (Some(_), Some(m), Some(day)) => (1..=12).contains(&m) && (1..=31).contains(&day),
        _ => false,
    }
}

/// Validate the sources of a finding. Returns what is wrong, one line each.
fn source_problems(sources: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, src) in sources.iter().enumerate() {
        let n = i + 1;
        let url = text_of(src, "url");
        let linked = url.split_once(':').is_some_and(|(scheme, rest)| {
            matches!(scheme, "http" | "https") && rest.starts_with("//") && rest.len() > 2
        });
        if !linked || url.contains(' ') {
            out.push(format!("source {n}: `url` must be a link (http or https)"));
        }
        if text_of(src, "title").is_empty() {
            out.push(format!("source {n}: `title` is empty"));
        }
        if !valid_date(&text_of(src, "read_on")) {
            out.push(format!(
                "source {n}: `read_on` must be the date it was read, YYYY-MM-DD"
            ));
        }
        let g = text_of(src, "grade");
        if !GRADES.contains(&g.as_str()) {
            out.push(format!(
                "source {n}: `grade` must be one of {}",
                GRADES.join(", ")
            ));
        }
        if text_of(src, "why").is_empty() {
            out.push(format!(
                "source {n}: `why` is empty: say in one sentence why it got that grade"
            ));
        }
    }
    out
}

/// Validate a legal finding as given. The markets and the kind of risk are free words: any market and
/// any kind of risk is accepted, as long as they are named.
pub fn validate_finding(f: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if text_of(f, "title").is_empty() {
        out.push("`title` is empty".into());
    }
    match f.get("markets").and_then(|m| m.as_array()) {
        Some(a)
            if !a.is_empty()
                && a.iter()
                    .all(|x| x.as_str().is_some_and(|t| !t.trim().is_empty())) => {}
        _ => out.push(
            "`markets` must list at least one market by name (any country, region or word)".into(),
        ),
    }
    if text_of(f, "risk").is_empty() {
        out.push("`risk` is empty: name the kind of risk in free words".into());
    }
    if f.get("lawyer_check").and_then(|v| v.as_bool()).is_none() {
        out.push("`lawyer_check` must be true or false: the mark that says whether a lawyer must check it".into());
    }
    let sources: Vec<Value> = match f.get("sources") {
        None => vec![],
        Some(Value::Array(a)) => a.clone(),
        Some(_) => {
            out.push("`sources` must be a list of tables".into());
            vec![]
        }
    };
    match f.get("researched").and_then(|v| v.as_bool()) {
        None => out.push(
            "`researched` must be true or false: a finding made without research says so".into(),
        ),
        Some(true) => {
            if sources.is_empty() {
                out.push("the finding claims research and lists no source".into());
            }
        }
        Some(false) => {
            if !sources.is_empty() {
                out.push(
                    "the finding says it was made without research but lists sources: set `researched = true`, or drop the sources".into(),
                );
            }
        }
    }
    out.extend(source_problems(&sources));
    if matches!(text_of(f, "outcome").as_str(), "accepted") {
        out.push("`outcome = \"accepted\"` is the human's alone".into());
    }
    if !matches!(
        text_of(f, "outcome").as_str(),
        "" | "open" | "fixed" | "accepted"
    ) {
        out.push("`outcome` must be open or fixed".into());
    }
    if !matches!(
        text_of(f, "severity").as_str(),
        "" | "critical" | "high" | "medium" | "low"
    ) {
        out.push("`severity` must be critical, high, medium or low".into());
    }
    out
}

/// A hash of what the human reads in a finding: the record's title, text, severity, where, action,
/// market and links, and the detail's risk, markets, research mark and sources. The closing question
/// is bound to it when it is asked, so a finding changed afterwards (by any put) is no longer the one
/// the human acknowledged. The outcome is left out: it moves without changing what was read.
fn content_hash(f: &Value, d: &Value) -> String {
    let mut v = serde_json::Map::new();
    for k in [
        "title", "text", "severity", "where", "action", "market", "links",
    ] {
        v.insert(k.to_string(), f.get(k).cloned().unwrap_or(Value::Null));
    }
    for k in ["risk", "markets", "researched", "sources"] {
        v.insert(format!("d_{k}"), d.get(k).cloned().unwrap_or(Value::Null));
    }
    let v = Value::Object(v);
    util::sha256_hex(v.to_string().as_bytes())
}

fn is_marked(d: &Value) -> bool {
    d.get("lawyer_check").and_then(|v| v.as_bool()) == Some(true)
}

/// The sequence number of the event that asked the question, if any.
fn asked_seq(log: &events::Log, q: &str) -> Option<u64> {
    log.events
        .iter()
        .find(|e| e["kind"] == "question.asked" && e["question"] == q)
        .and_then(|e| e["seq"].as_u64())
}

/// Whether the question holds a real acknowledgement: answered with the acknowledging key, through
/// the answer command (an answered event in the log), after a human message that came after the
/// question was asked.
fn question_acknowledges(p: &Project, log: &events::Log, st: &events::State, qid: &str) -> R<bool> {
    let Some(q) = p.get_record(qid)? else {
        return Ok(false);
    };
    if s(&q, "kind") != "question" || s(&q, "stage") != CLOSE_STAGE {
        return Ok(false);
    }
    let Some(asked) = asked_seq(log, qid) else {
        return Ok(false);
    };
    let key = q
        .get("answer")
        .and_then(|a| a.get("choice"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    if key != ACK_KEY {
        return Ok(false);
    }
    let turn = q.get("answered_turn").and_then(|v| v.as_u64()).unwrap_or(0);
    let logged = log.events.iter().any(|e| {
        e["kind"] == "question.answered" && e["question"] == qid && e["turn"].as_u64() == Some(turn)
    });
    let human = st.turns.iter().any(|t| t.seq == turn);
    Ok(logged && human && turn > asked)
}

/// Whether a question is still waiting, or was answered with an acknowledgement.
fn question_is_current(p: &Project, log: &events::Log, st: &events::State, qid: &str) -> R<bool> {
    let Some(q) = p.get_record(qid)? else {
        return Ok(false);
    };
    if s(&q, "kind") != "question" {
        return Ok(false);
    }
    // Answered: it counts only as a real acknowledgement. Any other answer, forged or not, means the
    // finding needs asking again.
    if q.get("answer").is_some() {
        return question_acknowledges(p, log, st, qid);
    }
    Ok(true)
}

/// Handoff H-3: the legal findings that are marked and not yet acknowledged, one line each. The close
/// of a checkpoint and of the run asks for them and refuses while there is one. The engine's
/// enforcement part calls it.
pub fn unacknowledged(p: &Project) -> R<Vec<String>> {
    let mut out = Vec::new();
    let findings = p.list_records(schema::kind_by_name("legal"))?;
    let mut ctx: Option<(events::Log, events::State)> = None;
    for f in &findings {
        let id = s(f, "id").to_string();
        let Some(d) = read_detail(p, &id) else {
            continue;
        };
        if !is_marked(&d) {
            continue;
        }
        let title = s(f, "title");
        let qid = text_of(&d, "question");
        if qid.is_empty() {
            out.push(format!(
                "{id} \"{title}\" (no closing question yet: run `bf legal questions`)"
            ));
            continue;
        }
        if text_of(&d, "asked_hash") != content_hash(f, &d) {
            out.push(format!(
                "{id} \"{title}\" (changed since its question was asked: run `bf legal questions`)"
            ));
            continue;
        }
        if ctx.is_none() {
            ctx = Some((events::load_log(p)?, events::replay(p)?));
        }
        let (log, st) = ctx.as_ref().unwrap();
        if !question_acknowledges(p, log, st, &qid)? {
            out.push(format!(
                "{id} \"{title}\" (question {qid} is not answered with an acknowledgement)"
            ));
        }
    }
    Ok(out)
}

fn read_file(p: &Project, path: &str) -> R<Value> {
    let safe = p.safe_input(std::path::Path::new(path))?;
    let text = fs::read_to_string(&safe).map_err(|e| {
        refuse(
            format!("read {path}"),
            format!("the file cannot be read: {e}"),
            &["Give the path of an existing finding file (TOML or JSON)."],
            "The command prints its result.",
        )
    })?;
    let parsed: Result<Value, String> = if path.ends_with(".json") {
        serde_json::from_str(&text).map_err(|e| e.to_string())
    } else {
        toml::from_str(&text).map_err(|e| e.to_string())
    };
    parsed.map_err(|e| {
        refuse(
            format!("read {path}"),
            format!("it is not valid: {}", e.replace('\n', " ")),
            &["Fix the syntax of the file."],
            "The command prints its result.",
        )
    })
}

/// File a legal finding: validate it, write the record and its detail. A finding put again by its ID
/// is replaced, and its closing question is dropped: what the human acknowledged is no longer it.
pub fn file_finding(p: &Project, f: &Value) -> R<Value> {
    let problems = validate_finding(f);
    if !problems.is_empty() {
        return Err(bad(problems.join("; ")));
    }
    let kind = schema::kind_by_name("legal").ok_or_else(|| err("no legal record kind"))?;
    let markets: Vec<String> = f["markets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|x| x.trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut rec = json!({
        "kind": "legal",
        "title": text_of(f, "title"),
        "market": markets.join(", "),
        "outcome": if text_of(f, "outcome").is_empty() { "open".to_string() } else { text_of(f, "outcome") },
    });
    for k in ["text", "severity", "where", "action"] {
        let v = text_of(f, k);
        if !v.is_empty() {
            rec[k] = json!(v);
        }
    }
    if let Some(l) = f.get("links") {
        rec["links"] = l.clone();
    }
    let given = text_of(f, "id");
    let id = if given.is_empty() {
        let mut probe = rec.clone();
        probe["id"] = json!("L-0");
        schema::validate_or_refuse(kind, &probe)?;
        p.write_numbered(kind, &mut rec)?
    } else {
        match schema::parse_id(&given) {
            Some((pre, _)) if pre == kind.prefix => {}
            _ => return Err(bad(format!("`id` {given} is not an L-<number> ID"))),
        }
        if let Some(old) = p.get_record(&given)? {
            if s(&old, "outcome") == "accepted" {
                return Err(bad(format!(
                    "{given} was accepted by the human; file a new finding instead"
                )));
            }
        }
        if f.get("lawyer_check").and_then(|v| v.as_bool()) == Some(false)
            && read_detail(p, &given).is_some_and(|d| is_marked(&d))
        {
            return Err(bad(format!(
                "{given} carries the lawyer mark; only the human removes it, so the mark cannot be cleared by filing it again"
            )));
        }
        rec["id"] = json!(given);
        schema::validate_or_refuse(kind, &rec)?;
        p.write_record(&rec)?;
        given.clone()
    };
    let sources = f.get("sources").cloned().unwrap_or_else(|| json!([]));
    let detail = json!({
        "id": id,
        "markets": markets,
        "risk": text_of(f, "risk"),
        "researched": f["researched"],
        "sources": sources,
        "lawyer_check": f["lawyer_check"],
        "filed_at": util::now(),
    });
    write_detail(p, &id, &detail)?;
    p.write_guard()?;
    Ok(
        json!({"ok": true, "id": id, "researched": detail["researched"], "lawyer_check": detail["lawyer_check"]}),
    )
}

fn question_for(p: &Project, f: &Value, d: &Value) -> Value {
    let id = s(f, "id");
    let markets = d["markets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let mut ctx = format!(
        "Legal finding {id}: {}\nMarkets: {markets}\nKind of risk: {}\n",
        s(f, "title"),
        text_of(d, "risk")
    );
    let _ = p;
    for k in ["severity", "text", "action", "where"] {
        if !s(f, k).is_empty() {
            ctx.push_str(&format!("{k}: {}\n", s(f, k)));
        }
    }
    if d["researched"] == json!(true) {
        ctx.push_str("Sources:\n");
        if let Some(a) = d["sources"].as_array() {
            for src in a {
                ctx.push_str(&format!(
                    "- {} ({}), read {}, graded {}: {}\n",
                    text_of(src, "title"),
                    text_of(src, "url"),
                    text_of(src, "read_on"),
                    text_of(src, "grade"),
                    text_of(src, "why")
                ));
            }
        }
    } else {
        ctx.push_str("This finding was made without research.\n");
    }
    json!({
        "kind": "question",
        "title": format!("Do you acknowledge the legal finding {id} and its lawyer mark?"),
        "context": ctx,
        "answer_kind": "single",
        "stage": CLOSE_STAGE,
        "other": false,
        "options": [
            {"key": ACK_KEY, "text": "I have read it and I acknowledge it: a lawyer should check it", "proposed": false, "why": "The run can close once every marked finding is acknowledged."},
            {"key": "B", "text": "Not yet", "proposed": false, "why": "The run stays open."}
        ],
    })
}

/// Ask the closing question of each marked finding that has none that is current. Returns the new
/// question IDs by finding.
pub fn ask_questions(p: &Project) -> R<Value> {
    let qkind = schema::kind_by_name("question").ok_or_else(|| err("no question kind"))?;
    let findings = p.list_records(schema::kind_by_name("legal"))?;
    let mut made = Vec::new();
    let mut ctx: Option<(events::Log, events::State)> = None;
    for f in &findings {
        let id = s(f, "id").to_string();
        let Some(mut d) = read_detail(p, &id) else {
            continue;
        };
        if !is_marked(&d) {
            continue;
        }
        let cur = text_of(&d, "question");
        if !cur.is_empty() {
            if ctx.is_none() {
                ctx = Some((events::load_log(p)?, events::replay(p)?));
            }
            let (log, st) = ctx.as_ref().unwrap();
            if text_of(&d, "asked_hash") == content_hash(f, &d)
                && question_is_current(p, log, st, &cur)?
            {
                continue;
            }
        }
        let mut q = question_for(p, f, &d);
        schema::validate_or_refuse(qkind, &{
            let mut t = q.clone();
            t["id"] = json!("Q-0");
            t
        })?;
        let qid = p.write_numbered(qkind, &mut q)?;
        events::append(p, "question.asked", json!({"question": qid}))?;
        d["question"] = json!(qid);
        d["asked_hash"] = json!(content_hash(f, &d));
        write_detail(p, &id, &d)?;
        made.push(json!({"finding": id, "question": qid}));
    }
    p.write_guard()?;
    Ok(json!({"ok": true, "asked": made}))
}

/// Every legal finding with its mark and whether it is acknowledged.
pub fn list(p: &Project) -> R<Value> {
    let open: Vec<String> = unacknowledged(p)?;
    let mut rows = Vec::new();
    for f in p.list_records(schema::kind_by_name("legal"))? {
        let id = s(&f, "id").to_string();
        let d = read_detail(p, &id).unwrap_or_else(|| json!({}));
        let marked = is_marked(&d);
        let pending = open.iter().any(|l| l.starts_with(&format!("{id} ")));
        rows.push(json!({
            "id": id,
            "title": s(&f, "title"),
            "markets": d.get("markets").cloned().unwrap_or(json!([])),
            "risk": d.get("risk").cloned().unwrap_or(Value::Null),
            "researched": d.get("researched").cloned().unwrap_or(Value::Null),
            "sources": d.get("sources").cloned().unwrap_or(json!([])),
            "lawyer_check": marked,
            "question": d.get("question").cloned().unwrap_or(Value::Null),
            "acknowledged": marked && !pending,
        }));
    }
    Ok(json!({"findings": rows, "unacknowledged": open}))
}

fn project_here() -> R<Project> {
    let raw: Vec<String> = std::env::args().collect();
    let dir = match raw.iter().position(|a| a == "-C") {
        Some(i) if i + 1 < raw.len() => PathBuf::from(&raw[i + 1]),
        _ => std::env::current_dir()?,
    };
    Project::discover(&dir)
}

/// The refusal of `bf legal finding` from a checkpoint's working copy: the audit round's report is named (D-164).
fn refuse_from_checkpoint(p: &Project, k: &str) -> crate::refusal::Fail {
    crate::refusal::from_key(
        "ck_audit_records.refuse.legal_finding",
        &[
            ("k", k),
            (
                "check",
                &crate::ckaudit::checks_of_role(p, k, "legal-auditor"),
            ),
        ],
    )
}

/// Entry point of `bf legal <sub> ...`: `finding <file>`, `questions`, `list`.
pub fn run(sub: &str, rest: &[String]) -> R<Value> {
    match sub {
        "finding" => {
            let p = project_here()?;
            let Some(path) = rest.first() else {
                return Err(bad("no finding file was given"));
            };
            if let Some((_, zone_id)) =
                crate::cli::real_zone(&p).filter(|(z, _)| *z == "checkpoint")
            {
                return Err(refuse_from_checkpoint(&p, &zone_id));
            }
            let f = read_file(&p, path)?;
            file_finding(&p, &f)
        }
        "questions" => ask_questions(&project_here()?),
        "list" => list(&project_here()?),
        _ => Err(refuse(
            format!("run `bf legal {sub}`"),
            "that is not a legal command",
            &["Use `bf legal finding <file>`, `bf legal questions` or `bf legal list`."],
            "The command prints its result.",
        )),
    }
}
