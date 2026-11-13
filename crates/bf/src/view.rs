//! Page data in the view contract (version 1) and the template renderer.

use crate::events::{self, Gate, State};
use crate::evidence::{self, Receipt};
use crate::gates::GATE_KINDS;
use crate::ledger;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, R};
use crate::run;
use crate::schema;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub const KINDS: &[&str] = &[
    "index",
    "gate",
    "questions",
    "ledger",
    "plan",
    "run",
    "checkpoint",
    "coverage",
];

fn text(t: &str) -> Value {
    json!({"type": "text", "text": t})
}

fn plural(kind: &str) -> String {
    match kind {
        "decision" => "Decisions".into(),
        "threat" => "Threats".into(),
        k => {
            let mut c = k.chars();
            let first = c
                .next()
                .map(|x| x.to_uppercase().to_string())
                .unwrap_or_default();
            format!("{first}{}s", c.as_str())
        }
    }
}

fn tone_of_state(state: &str) -> &'static str {
    match state {
        "landed" | "reviewed" => "ok",
        "parked" | "blocked" => "open",
        _ => "accepted",
    }
}

fn block_state(state: &str) -> &'static str {
    match state {
        "landed" => "landed",
        "parked" | "blocked" => "parked",
        "ready" => "ready",
        "planned" => "planned",
        _ => "running",
    }
}

fn wave_block(pl: &Plan, states: &HashMap<String, String>) -> Value {
    let mut waves = Vec::new();
    for (n, ids) in pl.waves() {
        let units: Vec<Value> = ids
            .iter()
            .map(|id| {
                let u = pl.unit(id).unwrap();
                let mut unit = json!({"id": id, "title": s(u, "title"), "state": block_state(states.get(id).map(|x| x.as_str()).unwrap_or("planned"))});
                if let Some(points) = u.get("estimate") {
                    unit["points"] = points.clone();
                }
                unit
            })
            .collect();
        waves.push(json!({"n": n, "units": units}));
    }
    let mut edges = Vec::new();
    for u in &pl.units {
        for d in str_list(u, "depends_on") {
            edges.push(json!([d, s(u, "id")]));
        }
    }
    json!({"type": "waves", "waves": waves, "edges": edges})
}

fn record_toml(rec: &Value) -> String {
    toml::to_string_pretty(&crate::store::strip_nulls(rec.clone())).unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn base(
    p: &Project,
    kind: &str,
    id: &str,
    crumbs: Vec<String>,
    status: (&str, &str),
    title: &str,
    lede: &str,
    facts: Vec<Value>,
) -> Value {
    json!({
        "contract": 1,
        "kind": kind,
        "id": id,
        "project": p.config.project,
        "built_at": util::now(),
        "page": {
            "crumbs": crumbs,
            "status": {"text": status.0, "tone": status.1},
            "title": title,
            "lede": lede,
            "facts": facts,
        },
        "sections": [],
    })
}

fn fact(label: &str, value: impl ToString, sub: &str) -> Value {
    json!({"label": label, "value": value.to_string(), "sub": sub})
}

fn state_map(p: &Project, pl: &Plan, st: &State, rc: &[Receipt]) -> HashMap<String, String> {
    run::unit_states(p, pl, st, rc)
        .into_iter()
        .map(|(a, b, _)| (a, b))
        .collect()
}

fn design_row(rec: &Value, round: Option<(&str, String, Option<String>)>) -> Value {
    let mut body = Vec::new();
    if !s(rec, "text").is_empty() {
        body.push(text(s(rec, "text")));
    }
    let acc = str_list(rec, "acceptance");
    if !acc.is_empty() {
        body.push(json!({"type": "list", "items": acc}));
    }
    for (k, label) in [
        ("chosen", "Chosen"),
        ("rejected", "Rejected"),
        ("mitigation", "Mitigation"),
        ("measure", "Measure"),
    ] {
        if !s(rec, k).is_empty() {
            body.push(text(&format!("{label}: {}", s(rec, k))));
        }
    }
    let mut row = json!({
        "id": s(rec, "id"),
        "title": s(rec, "title"),
        "label": rec.get("label").cloned().unwrap_or(Value::Null),
        "body": body,
        "refs": str_list(rec, "links").iter().map(|l| json!({"label": l, "tip": "", "target": null})).collect::<Vec<_>>(),
    });
    if let Some((r, note, was)) = round {
        row["round"] = json!(r);
        row["round_note"] = json!(note);
        if let Some(w) = was {
            row["body"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type": "was", "text": w}));
        }
    }
    row
}

/// Page for a gate. `arg` is a gate ID, a gate kind, or None for the latest gate.
fn gate_view(p: &Project, st: &State, arg: Option<&str>) -> R<Value> {
    let gate: Option<&Gate> = match arg {
        Some(a) => st
            .gate(a)
            .or_else(|| st.gates.iter().rev().find(|g| g.kind == a)),
        None => st.gates.last(),
    };
    let (kind, refr, round, status, gid, snapshot) = match gate {
        Some(g) => (
            g.kind.clone(),
            g.refr.clone(),
            g.round,
            g.status.clone(),
            g.id.clone(),
            g.snapshot.clone(),
        ),
        None => match arg {
            Some(a) if GATE_KINDS.contains(&a) => (
                a.to_string(),
                None,
                1,
                "draft".into(),
                format!("{a}-draft"),
                Value::Null,
            ),
            _ => return Err(refuse(
                "build a gate page",
                "no gate matches and none is open",
                &["Open one with `bf gate open <kind>`, or name a gate kind to preview its page."],
                "`bf view gate --json` prints the page data.",
            )),
        },
    };
    let prev: Option<&Gate> = st
        .gates
        .iter()
        .rev()
        .find(|g| g.kind == kind && g.refr == refr && g.round + 1 == round);
    let prev_snap = prev.map(|g| g.snapshot.clone()).unwrap_or(Value::Null);
    let pl = plan::load(p)?;
    let mut records: Vec<Value> = Vec::new();
    let mut sections = Vec::new();
    let (mut changed, mut new, mut removed) = (0, 0, 0);
    let kinds: Vec<&str> = match kind.as_str() {
        "idea" => vec!["idea"],
        "requirements" => vec!["requirement"],
        "ui" => vec!["screen", "journey"],
        "architecture" => vec!["component", "decision", "target", "threat"],
        "plan" => vec!["unit", "handoff", "checkpoint", "check"],
        _ => vec![],
    };
    let cur_snapshot = if gate.is_some() {
        snapshot.clone()
    } else {
        crate::gates::snapshot(p, &kind, refr.as_deref()).unwrap_or(Value::Null)
    };
    for k in &kinds {
        let recs = p.list_records(schema::kind_by_name(k))?;
        let mut rows = Vec::new();
        for r in &recs {
            let id = s(r, "id");
            let mark = if prev.is_some() {
                match prev_snap.get(id) {
                    None => {
                        new += 1;
                        Some(("new", "New in this round.".to_string(), None))
                    }
                    Some(old) => {
                        let canon = serde_json::to_string(r).unwrap_or_default();
                        if old["h"].as_str() != Some(&util::sha256_hex(canon.as_bytes())) {
                            changed += 1;
                            let was = old["title"]
                                .as_str()
                                .filter(|t| *t != s(r, "title"))
                                .map(|t| t.to_string());
                            Some(("changed", "Changed since the last round.".to_string(), was))
                        } else {
                            None
                        }
                    }
                }
            } else {
                None
            };
            let mut row = if *k == "unit" || *k == "handoff" || *k == "checkpoint" || *k == "check"
            {
                plan_row(p, r, &pl)
            } else {
                design_row(r, mark.clone())
            };
            if let Some((rd, note, _)) = &mark {
                row["round"] = json!(rd);
                row["round_note"] = json!(note);
            }
            row["needs_you"] =
                json!(status == "open" && (mark.is_some() || prev.is_none()) && *k != "check");
            rows.push(row);
            records.push(
                json!({"title": format!("{id}.toml"), "format": "toml", "text": record_toml(r)}),
            );
        }
        // Rows that were in the last round and are gone now.
        if let Some(obj) = prev_snap.as_object() {
            for (id, old) in obj {
                if schema::kind_of_id(id)
                    .map(|kk| kk.name == *k)
                    .unwrap_or(false)
                    && !recs.iter().any(|r| s(r, "id") == id)
                {
                    removed += 1;
                    rows.push(json!({"id": id, "title": old["title"], "round": "removed", "round_note": "Removed in this round.", "body": []}));
                }
            }
        }
        sections.push(json!({"id": *k, "title": plural(k), "hint": format!("{} in this gate", plural(k).to_lowercase()), "open": true, "blocks": [], "rows": rows}));
    }
    if kind == "idea" {
        if let Some((title, text)) = crate::gates::idea_brief(p) {
            let mark = if prev.is_some() {
                match prev_snap.get("idea.md") {
                    None => {
                        new += 1;
                        Some(("new", "New in this round."))
                    }
                    Some(old) if old["h"].as_str() != Some(&util::sha256_hex(text.as_bytes())) => {
                        changed += 1;
                        Some(("changed", "Changed since the last round."))
                    }
                    _ => None,
                }
            } else {
                None
            };
            let mut row = json!({"id": "idea", "title": title, "needs_you": status == "open", "body": [{"type": "text", "text": text}]});
            if let Some((r, note)) = mark {
                row["round"] = json!(r);
                row["round_note"] = json!(note);
            }
            if sections.is_empty() {
                sections.push(json!({"id": "idea", "title": "The idea", "hint": "The brief the human wrote", "open": true, "blocks": [], "rows": [row]}));
            } else {
                sections[0]["rows"].as_array_mut().unwrap().insert(0, row);
            }
            records.push(json!({"title": "idea.md", "format": "markdown", "text": text}));
        }
    }
    if kind == "plan" {
        let rc = evidence::load_receipts(p)?;
        let states = state_map(p, &pl, st, &rc);
        if let Some(sec) = sections.first_mut() {
            // What is being approved: the waves, and every designed ID with its unit, checkpoint,
            // checks and state (gaps marked).
            let cov = ledger::rows(&pl, &[], false, st);
            sec["blocks"] = json!([wave_block(&pl, &states), coverage_block(&cov)]);
        }
        if !pl.problems.is_empty() {
            sections.insert(0, json!({"id": "problems", "title": "Plan problems", "hint": "Fix these before the gate opens", "open": true, "blocks": [],
                "rows": pl.problems.iter().enumerate().map(|(i, x)| json!({"id": format!("P-{}", i + 1), "title": x, "needs_you": false, "label": "high", "body": []})).collect::<Vec<_>>()}));
        }
    }
    if kind == "checkpoint-step" {
        if let Some(k) = &refr {
            let rec = p.require_record(k)?;
            let rc = evidence::load_receipts(p)?;
            let mut rows = Vec::new();
            for id in str_list(&rec, "proves") {
                let mut row = proof_row(p, &pl, &rc, &id, None);
                row["needs_you"] = json!(status == "open");
                rows.push(row);
            }
            sections.push(json!({"id": "proofs", "title": "What this checkpoint proves", "hint": s(&rec, "human_step"), "open": true, "blocks": [], "rows": rows}));
            records.push(
                json!({"title": format!("{k}.toml"), "format": "toml", "text": record_toml(&rec)}),
            );
        }
    }
    let _ = cur_snapshot;
    let (stext, tone) = match status.as_str() {
        "approved" => ("Approved", "ok"),
        "rejected" => ("Changes requested", "info"),
        "open" => ("Open · answer in chat", "wait"),
        _ => ("Draft · not opened yet", "info"),
    };
    let title = format!(
        "{} the {}",
        if status == "approved" {
            "Approved:"
        } else {
            "Approve"
        },
        kind.replace('-', " ")
    );
    let mut v = base(
        p,
        "gate",
        &gid,
        vec![
            "All pages".into(),
            p.config.project.clone(),
            "Gates".into(),
            kind.clone(),
        ],
        (&format!("{} gate · {stext}", kind), tone),
        &title,
        &format!(
            "Round {round}. Say approve or ask for changes in chat; the page only helps you read."
        ),
        vec![
            fact(
                "Round",
                round,
                if prev.is_some() {
                    "after your changes"
                } else {
                    "first look"
                },
            ),
            fact("Records", records.len(), "shown on this page"),
        ],
    );
    if kind == "architecture" {
        if let Some(q) = st.skipped_words("ui") {
            v["page"]["facts"].as_array_mut().unwrap().push(fact(
                "UI gate skipped",
                q,
                "the human skipped the ui gate: this product has no interface",
            ));
        }
        if p.list_records(schema::kind_by_name("threat"))?.is_empty() {
            v["page"]["facts"].as_array_mut().unwrap().push(fact(
                "Caution",
                "No threat was recorded",
                "the architecture has no threat record: nobody wrote down what could go wrong",
            ));
        }
    }
    if let Some(g) = gate.filter(|g| !g.quote.is_empty()) {
        v["page"]["facts"].as_array_mut().unwrap().push(fact(
            "Decided on the words",
            &g.quote,
            "taken from the human's message; check that they say what you meant",
        ));
        v["page"]["facts"].as_array_mut().unwrap().push(fact(
            "The human's whole message",
            &g.message,
            "so you can see what the quote was cut from",
        ));
        if !g.transcript.is_empty() {
            v["page"]["facts"].as_array_mut().unwrap().push(fact(
                "Transcript",
                &g.transcript,
                "what the harness's transcript said about this message (information only)",
            ));
        }
        if !g.caution.is_empty() {
            v["page"]["facts"].as_array_mut().unwrap().push(fact(
                "Caution",
                &g.caution,
                "not a refusal: check that the words above say what you meant",
            ));
        }
    }
    v["page"]["round"] = json!({"n": round, "prev": if prev.is_some() { json!(round - 1) } else { Value::Null }, "changed": changed, "new": new, "removed": removed});
    v["sections"] = json!(sections);
    v["records"] = json!(records);
    v["answer"] = json!({"how": "chat", "prompt": "Approve in chat in your own words, or ask for changes. Nothing on this page records a decision."});
    Ok(v)
}

/// A table of the IDs a check or checkpoint proves, each with what it is.
fn proved_table(pl: &Plan, ids: &[String]) -> Value {
    let title_of = |id: &str| -> String {
        if let Some(d) = pl.designed.iter().find(|d| s(d, "id") == id) {
            return s(d, "title").to_string();
        }
        if let Some(h) = pl.handoffs.iter().find(|h| s(h, "id") == id) {
            return format!(
                "Handoff: {} from {} to {}",
                s(h, "thing"),
                s(h, "from"),
                s(h, "to")
            );
        }
        String::new()
    };
    json!({"type": "table", "head": ["Proves", "What it is"],
        "rows": ids.iter().map(|i| vec![i.clone(), title_of(i)]).collect::<Vec<_>>()})
}

fn proves_refs(pl: &Plan, ids: &[String]) -> Vec<Value> {
    let tbl = proved_table(pl, ids);
    ids.iter()
        .enumerate()
        .map(|(i, id)| json!({"label": id, "tip": tbl["rows"][i][1], "target": null}))
        .collect()
}

fn plan_row(p: &Project, r: &Value, pl: &Plan) -> Value {
    let id = s(r, "id");
    let mut body = Vec::new();
    match s(r, "kind") {
        "unit" => {
            body.push(text(&format!(
                "Delivers {}. Checkpoint {}.",
                str_list(r, "delivers").join(", "),
                s(r, "checkpoint")
            )));
            // Where a row names a file, it says which repository the file is in (in a workspace).
            let repo = if p.workspace {
                p.repo_of_unit(r)
                    .map(|x| format!(" in {}", x.name))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            body.push(json!({"type": "list", "items": str_list(r, "files").iter().map(|f| format!("May touch {f}{repo}")).collect::<Vec<_>>()}));
            let deps = str_list(r, "depends_on");
            if !deps.is_empty() {
                body.push(text(&format!("Waits for {}.", deps.join(", "))));
            }
        }
        "handoff" => {
            body.push(text(&format!(
                "{} hands {} to {}.",
                s(r, "from"),
                s(r, "thing"),
                s(r, "to")
            )));
            if !s(r, "contract").is_empty() {
                body.push(text(s(r, "contract")));
            }
        }
        "checkpoint" => {
            body.push(proved_table(pl, &str_list(r, "proves")));
            body.push(text(&format!(
                "Its checks: {}.",
                str_list(r, "checks").join(", ")
            )));
            if !s(r, "human_step").is_empty() {
                body.push(text(&format!("Human step: {}", s(r, "human_step"))));
            }
        }
        _ => {
            body.push(proved_table(pl, &str_list(r, "proves")));
            if let Some(c) = r.get("command") {
                body.push(json!({"type": "code", "text": match c { Value::String(x) => x.clone(), o => o.to_string() }}));
            }
        }
    }
    let title = if s(r, "title").is_empty() {
        format!("{} {} to {}", id, s(r, "from"), s(r, "to"))
    } else {
        s(r, "title").to_string()
    };
    let refs = if matches!(s(r, "kind"), "check" | "checkpoint") {
        proves_refs(pl, &str_list(r, "proves"))
    } else {
        vec![]
    };
    json!({"id": id, "title": title, "body": body, "refs": refs})
}

fn proof_row(p: &Project, pl: &Plan, rc: &[Receipt], id: &str, content: Option<&str>) -> Value {
    let rec = p.get_record(id).ok().flatten().unwrap_or(Value::Null);
    let checks: Vec<&Value> = pl
        .checks
        .iter()
        .filter(|c| str_list(c, "proves").iter().any(|x| x == id))
        .collect();
    let mut body = Vec::new();
    let mut proven = false;
    for c in &checks {
        for r in rc.iter().filter(|r| r.check() == s(c, "id")) {
            let current = content.map(|h| h == r.content()).unwrap_or(true);
            if r.passes() && current {
                proven = true;
            }
            if let Some(t) = judgment_text(r) {
                body.push(text(&t));
                continue;
            }
            body.push(json!({"type": "evidence", "check": r.check(), "command": s(&r.v, "command"), "exit": r.v["exit"].as_i64().unwrap_or(0),
                "commit": &r.content()[..r.content().len().min(12)], "at": s(&r.v, "at"), "output_tail": s(&r.v, "output_tail")}));
        }
    }
    // A verifier's standing outcome (failed or unreachable) means the human has to decide.
    let mut marked: Vec<String> = Vec::new();
    if let Some(h) = content {
        for c in &checks {
            if let Some(r) = evidence::standing_outcome(rc, s(c, "id"), h) {
                marked.push(format!(
                    "{} is {}: {}",
                    s(c, "id"),
                    r.outcome().unwrap_or(""),
                    s(&r.v, "reason")
                ));
            }
        }
    }
    for m in &marked {
        body.push(text(m));
    }
    if body.is_empty() {
        body.push(text("No receipt yet."));
    }
    let title = if rec.is_null() {
        id.to_string()
    } else {
        s(&rec, "title").to_string()
    };
    json!({"id": id, "title": title, "needs_you": !proven && !marked.is_empty(), "handled": {"tone": if proven { "ok" } else { "open" }, "text": if proven { "Proven".to_string() } else if marked.is_empty() { "No proof at this content".to_string() } else { "A check failed or is unreachable".to_string() }}, "body": body})
}

fn questions_view(p: &Project, arg: Option<&str>) -> R<Value> {
    let qs = p.list_records(schema::kind_by_name("question"))?;
    let mut out = Vec::new();
    for q in qs.iter().filter(|q| {
        arg.map(|a| s(q, "stage") == a || s(q, "id") == a)
            .unwrap_or(true)
    }) {
        let opts: Vec<Value> = q
            .get("options")
            .and_then(|o| o.as_array())
            .cloned()
            .unwrap_or_default();
        // An answer with an Other choice carries the human's text in `other`; an answer to a choice
        // question always has the field (empty when there is none).
        let mut answer = q.get("answer").cloned().unwrap_or(Value::Null);
        if let Value::Object(o) = &mut answer {
            if s(q, "answer_kind") != "text" {
                o.entry("other").or_insert_with(|| json!(""));
            }
        }
        let mut item = json!({
            "id": s(q, "id"), "title": s(q, "title"), "context": s(q, "context"), "kind": s(q, "answer_kind"),
            "options": opts, "answer": answer,
        });
        if s(q, "answer_kind") != "text" {
            item["other"] = json!(crate::store::question_takes_other(q));
        }
        out.push(item);
    }
    let open = out.iter().filter(|q| q["answer"].is_null()).count();
    let mut v = base(
        p,
        "questions",
        arg.unwrap_or("all"),
        vec!["All pages".into(), p.config.project.clone(), "Questions".into()],
        (if open > 0 { "Questions · answer in the form" } else { "All answered" }, if open > 0 { "wait" } else { "ok" }),
        "Questions before the approval",
        "Answer these first. The approval page is built after the answers and holds no open question.",
        vec![fact("Open", open, "still to answer"), fact("Answered", out.len() - open, "so far")],
    );
    v["questions"] = json!(out);
    v["answer"] = json!({"how": "form", "prompt": "Fill in the form; the answers are copied into the records."});
    Ok(v)
}

fn ledger_rows_view(p: &Project, with_evidence: bool) -> R<(Plan, Vec<ledger::Row>)> {
    let pl = plan::load(p)?;
    let rc = evidence::load_receipts(p)?;
    let st = events::replay(p)?;
    let rows = ledger::rows(&pl, &rc, with_evidence, &st);
    Ok((pl, rows))
}

fn coverage_state(r: &ledger::Row) -> &'static str {
    if !r.missing.is_empty() && (r.units.is_empty() || r.checks.is_empty()) {
        "gap"
    } else if r.evidence.is_empty() {
        "planned"
    } else {
        "proven"
    }
}

/// What a check judged by an agent says: the outcome, the sentence about what was looked at, and the
/// file kept with it. `None` for a receipt that is a command run.
fn judgment_text(r: &evidence::Receipt) -> Option<String> {
    let kind = match s(&r.v, "type") {
        "attach" => "met",
        "fail" if !s(&r.v, "file_hash").is_empty() => "failed",
        "unreachable" if !s(&r.v, "file_hash").is_empty() => "unreachable",
        "support" => "supporting file only (settles nothing)",
        _ => return None,
    };
    let file = format!(
        "{} ({})",
        s(&r.v, "file_name"),
        &s(&r.v, "file_hash")[..s(&r.v, "file_hash").len().min(12)]
    );
    Some(if s(&r.v, "reason").is_empty() {
        format!("{}: {kind}. File: {file}.", r.check())
    } else {
        format!("{}: {kind}. {} File: {file}.", r.check(), s(&r.v, "reason"))
    })
}

fn coverage_block(rows: &[ledger::Row]) -> Value {
    json!({"type": "coverage", "rows": rows.iter().map(|r| json!({
        "id": r.id, "title": r.title, "unit": r.units.first().cloned().unwrap_or_default(),
        "checkpoint": r.checkpoints.first().cloned().unwrap_or_default(), "checks": r.checks.len(), "state": coverage_state(r)})).collect::<Vec<_>>()})
}

fn ledger_view(p: &Project) -> R<Value> {
    let (_, rows) = ledger_rows_view(p, true)?;
    let gaps = rows.iter().filter(|r| !r.missing.is_empty()).count();
    let out_rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id, "title": r.title, "needs_you": false,
                "handled": {"tone": if r.missing.is_empty() { "ok" } else { "open" }, "text": if r.missing.is_empty() { "Linked".to_string() } else { format!("Missing {}", r.missing.join(", ")) }},
                "body": [
                    {"type": "table", "head": ["Link", "IDs"], "rows": [
                        ["Units", r.units.join(", ")], ["Checkpoints", r.checkpoints.join(", ")],
                        ["Checks", r.checks.join(", ")], ["Receipts", r.evidence.join(", ")]]}
                ],
            })
        })
        .collect();
    let mut v = base(
        p,
        "ledger",
        "ledger",
        vec![
            "All pages".into(),
            p.config.project.clone(),
            "Ledger".into(),
        ],
        (
            if gaps == 0 {
                "Ledger closed"
            } else {
                "Ledger open · gaps remain"
            },
            if gaps == 0 { "ok" } else { "wait" },
        ),
        "Traceability ledger",
        "Every designed part needs a unit, a check and evidence.",
        vec![
            fact("Parts", rows.len(), "designed"),
            fact("Gaps", gaps, "unlinked"),
        ],
    );
    v["sections"] = json!([{"id": "parts", "title": "Designed parts", "hint": "Each with its links", "open": true, "blocks": [], "rows": out_rows}]);
    Ok(v)
}

fn coverage_view(p: &Project) -> R<Value> {
    let (_, rows) = ledger_rows_view(p, true)?;
    let proven = rows
        .iter()
        .filter(|r| coverage_state(r) == "proven")
        .count();
    let mut v = base(
        p,
        "coverage",
        "coverage",
        vec![
            "All pages".into(),
            p.config.project.clone(),
            "Coverage".into(),
        ],
        (
            &format!("{proven} of {} proven", rows.len()),
            if proven == rows.len() && !rows.is_empty() {
                "ok"
            } else {
                "info"
            },
        ),
        "Coverage",
        "Which designed parts are proven, planned or missing a link.",
        vec![
            fact("Proven", proven, "with a passing receipt"),
            fact("Total", rows.len(), "designed parts"),
        ],
    );
    v["sections"] = json!([{"id": "coverage", "title": "Coverage", "hint": "One row per designed part", "open": true,
        "blocks": [coverage_block(&rows), {"type": "meter", "label": "Proven", "value": proven, "max": rows.len(), "unit": "parts"}], "rows": []}]);
    // Checks judged by an agent: the outcome, what was looked at, and the file kept with it.
    let judged: Vec<Value> = evidence::load_receipts(p)?
        .iter()
        .filter_map(|r| {
            judgment_text(r).map(|t| json!({"id": r.id(), "title": r.check(), "needs_you": false, "body": [text(&t)]}))
        })
        .collect();
    if !judged.is_empty() {
        v["sections"].as_array_mut().unwrap().push(json!({"id": "judged", "title": "Judged by an agent", "hint": "Outcome, what was looked at, the file", "open": true, "blocks": [], "rows": judged}));
    }
    Ok(v)
}

fn plan_view(p: &Project, st: &State) -> R<Value> {
    let pl = plan::load(p)?;
    let rc = evidence::load_receipts(p)?;
    let states = state_map(p, &pl, st, &rc);
    let mut unit_rows = Vec::new();
    for u in &pl.units {
        let id = s(u, "id");
        let stt = states.get(id).cloned().unwrap_or_default();
        let mut row = plan_row(p, u, &pl);
        row["handled"] = json!({"tone": tone_of_state(&stt), "text": stt});
        unit_rows.push(row);
    }
    let mut sections = vec![
        json!({"id": "units", "title": "Units", "hint": "In waves; units in one wave share no files", "open": true, "blocks": [wave_block(&pl, &states)], "rows": unit_rows}),
    ];
    sections.push(json!({"id": "handoffs", "title": "Handoffs", "hint": "What passes between units", "open": false, "blocks": [],
        "rows": pl.handoffs.iter().map(|h| plan_row(p, h, &pl)).collect::<Vec<_>>()}));
    sections.push(json!({"id": "checkpoints", "title": "Checkpoints", "hint": "What each one proves", "open": false, "blocks": [],
        "rows": pl.checkpoints.iter().map(|h| plan_row(p, h, &pl)).collect::<Vec<_>>()}));
    if !pl.problems.is_empty() {
        sections.insert(0, json!({"id": "problems", "title": "Plan problems", "hint": "`bf plan check` refuses until these are fixed", "open": true, "blocks": [],
            "rows": pl.problems.iter().enumerate().map(|(i, x)| json!({"id": format!("P-{}", i + 1), "title": x, "needs_you": false, "label": "high", "body": []})).collect::<Vec<_>>()}));
    }
    let mut v = base(
        p,
        "plan",
        "plan",
        vec!["All pages".into(), p.config.project.clone(), "Plan".into()],
        (
            if pl.problems.is_empty() {
                "Plan ok"
            } else {
                "Plan has problems"
            },
            if pl.problems.is_empty() { "ok" } else { "wait" },
        ),
        "Delivery plan",
        "Units, handoffs and checkpoints, grouped in waves.",
        vec![
            fact("Units", pl.units.len(), "in the plan"),
            fact("Waves", pl.waves().len(), "in order"),
            fact("Checkpoints", pl.checkpoints.len(), "to close"),
        ],
    );
    v["sections"] = json!(sections);
    Ok(v)
}

fn run_view(p: &Project, st: &State) -> R<Value> {
    let pl = plan::load(p)?;
    let rc = evidence::load_receipts(p)?;
    let states = run::unit_states(p, &pl, st, &rc);
    let map: HashMap<String, String> = states
        .iter()
        .map(|(a, b, _)| (a.clone(), b.clone()))
        .collect();
    let mut rows = Vec::new();
    for (id, state, why) in &states {
        let u = pl.unit(id).unwrap();
        let mut body = vec![text(&format!(
            "State: {state}.{}",
            why.as_ref()
                .map(|w| format!(" Waiting: {w}."))
                .unwrap_or_default()
        ))];
        if let Some(us) = st.units.get(id) {
            body.push(text(&format!(
                "Review rounds: {}. Spent: {:.2}.",
                us.rounds, us.cost
            )));
            for n in &us.notes {
                body.push(text(n));
            }
        }
        for c in pl.unit_checks(id) {
            if let Some(r) = rc.iter().rfind(|r| r.check() == s(c, "id") && r.is_run()) {
                body.push(json!({"type": "evidence", "check": r.check(), "command": s(&r.v, "command"), "exit": r.v["exit"].as_i64().unwrap_or(0),
                    "commit": &r.content()[..r.content().len().min(12)], "at": s(&r.v, "at"), "output_tail": s(&r.v, "output_tail")}));
            }
        }
        rows.push(json!({"id": id, "title": s(u, "title"), "needs_you": matches!(state.as_str(), "blocked" | "parked"),
            "handled": {"tone": tone_of_state(state), "text": state}, "body": body}));
    }
    let log = events::load_log(p)?;
    let recent: Vec<Value> = log.events.iter().rev().take(15).map(|e| {
        json!({"id": format!("E-{}", e["seq"]), "title": format!("{} {}", s(e, "at"), s(e, "kind")), "body": [{"type": "code", "text": serde_json::to_string(&strip_event(e)).unwrap_or_default()}]})
    }).collect();
    let run = st.active_run();
    let mut blocks = vec![wave_block(&pl, &map)];
    if p.policy.budget.cap > 0.0 {
        blocks.push(json!({"type": "meter", "label": "Budget", "value": st.cost, "max": p.policy.budget.cap, "unit": p.policy.budget.unit}));
    }
    let landed = map.values().filter(|x| x.as_str() == "landed").count();
    let mut v = base(
        p,
        "run",
        run.map(|r| r.id.as_str()).unwrap_or("no-run"),
        vec!["All pages".into(), p.config.project.clone(), "Run".into()],
        (
            &match run {
                Some(r) => format!(
                    "{} · {}{}",
                    r.id,
                    if r.test_mode { "test run" } else { "real run" },
                    ""
                ),
                None => "No active run".into(),
            },
            if run.is_some() { "info" } else { "wait" },
        ),
        "Run",
        "Where each unit is, and what happened last.",
        vec![
            fact("Landed", format!("{landed} of {}", pl.units.len()), "units"),
            fact("Events", st.event_count, "in the log"),
            fact("Spent", format!("{:.2}", st.cost), &p.policy.budget.unit),
        ],
    );
    v["sections"] = json!([
        {"id": "units", "title": "Units", "hint": "State of each", "open": true, "blocks": blocks, "rows": rows},
        {"id": "events", "title": "Recent events", "hint": "Newest first", "open": false, "blocks": [], "rows": recent}
    ]);
    Ok(v)
}

fn strip_event(e: &Value) -> Value {
    let mut m = e.clone();
    if let Some(o) = m.as_object_mut() {
        for k in ["prev", "h", "text", "snapshot", "impact"] {
            o.remove(k);
        }
    }
    m
}

fn checkpoint_view(p: &Project, st: &State, arg: Option<&str>) -> R<Value> {
    let pl = plan::load(p)?;
    let kid = match arg {
        Some(a) => a.to_string(),
        None => pl
            .checkpoints
            .first()
            .map(|k| s(k, "id").to_string())
            .ok_or_else(|| {
                refuse(
                    "build a checkpoint page",
                    "the plan has no checkpoint",
                    &["Add a checkpoint record."],
                    "`bf view checkpoint K-1 --json` prints the page data.",
                )
            })?,
    };
    let k = pl.checkpoint(&kid).ok_or_else(|| {
        refuse(
            "build a checkpoint page",
            format!("no checkpoint {kid}"),
            &["Use an ID from `bf record list --kind checkpoint`."],
            "The page data prints.",
        )
    })?;
    let rc = evidence::load_receipts(p)?;
    let wt = p.work_dir().join(&kid);
    let content = if wt.join(".git").exists() {
        p.content_hash(&wt).ok()
    } else {
        None
    };
    let rows: Vec<Value> = str_list(k, "proves")
        .iter()
        .map(|id| proof_row(p, &pl, &rc, id, content.as_deref()))
        .collect();
    let closed = st.closed.contains_key(&kid);
    let proven = rows.iter().filter(|r| r["handled"]["tone"] == "ok").count();
    let mut v = base(
        p,
        "checkpoint",
        &kid,
        vec![
            "All pages".into(),
            p.config.project.clone(),
            "Checkpoints".into(),
            kid.clone(),
        ],
        (
            if closed { "Closed" } else { "Open" },
            if closed { "ok" } else { "wait" },
        ),
        &format!("Checkpoint {kid}: {}", s(k, "title")),
        "Each ID this checkpoint names needs a receipt at the content it ships.",
        vec![
            fact("Proven", format!("{proven} of {}", rows.len()), "IDs"),
            fact("Units", str_list(k, "after").len(), "before it"),
        ],
    );
    v["sections"] = json!([{"id": "proofs", "title": "Proofs", "hint": "One row per ID", "open": true, "blocks": [], "rows": rows}]);
    v["answer"] = json!({"how": "chat", "prompt": "Close with `bf checkpoint close` once every row is proven."});
    Ok(v)
}

fn when(at: &str) -> String {
    at.get(..16).unwrap_or(at).replace('T', " ")
}

fn page_row(id: &str, title: &str, href: &str, meta: &str, note: &str) -> Value {
    json!({"id": id, "title": title, "href": href, "meta": meta, "body": [text(note)]})
}

/// The list of pages. File names follow one convention: `gate-<gate id>.html`, `questions.html`,
/// `plan.html`, `run.html`, `ledger.html`, `coverage.html` and `checkpoint-<K>.html`.
fn index_view(p: &Project, st: &State) -> R<Value> {
    let pl = plan::load(p)?;
    let rc = evidence::load_receipts(p)?;
    let states = state_map(p, &pl, st, &rc);
    let landed = states.values().filter(|x| x.as_str() == "landed").count();
    let now = util::now();
    let mut waiting = Vec::new();
    for g in st.gates.iter().filter(|g| g.status == "open") {
        let mut r = page_row(
            &g.id,
            &format!("Approve the {}", g.kind.replace('-', " ")),
            &format!("gate-{}.html", g.id),
            &when(&g.open_at),
            &format!("{} gate \u{b7} answer in chat", g.kind),
        );
        r["handled"] = json!({"tone": "open", "text": format!("Round {}", g.round)});
        r["needs_you"] = json!(true);
        waiting.push(r);
    }
    let all_q = p.list_records(schema::kind_by_name("question"))?;
    let open_q = all_q.iter().filter(|q| q.get("answer").is_none()).count();
    if open_q > 0 {
        let mut r = page_row(
            "questions",
            &format!("{open_q} question(s) to answer before the next approval"),
            "questions.html",
            &when(&now),
            "Answer them in the form",
        );
        r["needs_you"] = json!(true);
        waiting.push(r);
    }
    for (id, state) in states
        .iter()
        .filter(|(_, s_)| matches!(s_.as_str(), "parked" | "blocked"))
    {
        let mut r = page_row(
            id,
            &format!("{id} is {state}: the human decides"),
            "run.html",
            &when(&now),
            "See the run page",
        );
        r["needs_you"] = json!(true);
        waiting.push(r);
    }
    let run = st.active_run();
    let mut state_rows = vec![
        page_row(
            "run",
            &match run {
                Some(r) => format!("{}: {landed} of {} units landed", r.id, pl.units.len()),
                None => "No active run".into(),
            },
            "run.html",
            &when(&st.last_at),
            "Where each unit is",
        ),
        page_row(
            "plan",
            &format!(
                "Delivery plan: {} units in {} waves",
                pl.units.len(),
                pl.waves().len()
            ),
            "plan.html",
            &when(&st.last_at),
            if pl.problems.is_empty() {
                "Plan ok"
            } else {
                "The plan has problems"
            },
        ),
        page_row(
            "ledger",
            "Every designed part and what is linked to it",
            "ledger.html",
            &when(&st.last_at),
            "Units, checks and receipts per part",
        ),
        page_row(
            "coverage",
            "Coverage of the designed parts",
            "coverage.html",
            &when(&st.last_at),
            "Proven, planned or missing a link",
        ),
    ];
    if let Some(q) = st.skipped_words("ui") {
        state_rows.push(page_row(
            "ui-skipped",
            &format!("The ui gate was skipped: \"{q}\""),
            "run.html",
            &when(&st.last_at),
            "No interface in this product, by the human's word",
        ));
    }
    if !all_q.is_empty() {
        state_rows.push(page_row(
            "questions-all",
            &format!("{} question(s) asked, {} still open", all_q.len(), open_q),
            "questions.html",
            &when(&st.last_at),
            "The form and its answers",
        ));
    }
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        state_rows.push(page_row(
            kid,
            &format!("Checkpoint {kid}: {}", s(k, "title")),
            &format!("checkpoint-{kid}.html"),
            &when(&st.last_at),
            if st.closed.contains_key(kid) {
                "Closed"
            } else {
                "Open"
            },
        ));
    }
    let history: Vec<Value> = st
        .gates
        .iter()
        .rev()
        .filter(|g| g.status != "open")
        .map(|g| {
            let mut r = page_row(
                &g.id,
                &format!("The {} gate, round {}", g.kind.replace('-', " "), g.round),
                &format!("gate-{}.html", g.id),
                &when(&g.open_at),
                &format!("{}{}", g.status, if g.simulated { " (simulated, in a test run)" } else { "" }),
            );
            r["handled"] = json!({"tone": if g.status == "approved" { "ok" } else { "accepted" }, "text": g.status});
            r
        })
        .collect();
    let n_pages = waiting.len() + state_rows.len() + history.len();
    let mut v = base(
        p,
        "index",
        "index",
        vec!["All pages".into(), p.config.project.clone()],
        (
            &if waiting.is_empty() {
                "Nothing waits for you".to_string()
            } else {
                format!("{} page(s) need you", waiting.len())
            },
            if waiting.is_empty() { "ok" } else { "wait" },
        ),
        "All pages",
        "Every page the factory has built for this project.",
        vec![
            fact("Pages", n_pages, ""),
            fact("Need you", waiting.len(), ""),
            fact(
                "Units landed",
                format!("{landed} of {}", pl.units.len()),
                "",
            ),
        ],
    );
    let mut sections = Vec::new();
    if !waiting.is_empty() {
        sections.push(json!({"id": "open", "title": "Waiting for you", "hint": "Open these first", "open": true, "rows": waiting}));
    }
    sections.push(json!({"id": "state", "title": "Project state", "hint": "Live views", "open": true, "rows": state_rows}));
    if !history.is_empty() {
        sections.push(json!({"id": "gates", "title": "Gates so far", "hint": "Newest first", "open": false, "rows": history}));
    }
    v["sections"] = json!(sections);
    Ok(v)
}

pub fn build(p: &Project, kind: &str, arg: Option<&str>) -> R<Value> {
    let st = events::replay(p)?;
    match kind {
        "index" => index_view(p, &st),
        "gate" => gate_view(p, &st, arg),
        "questions" => questions_view(p, arg),
        "ledger" => ledger_view(p),
        "plan" => plan_view(p, &st),
        "run" => run_view(p, &st),
        "checkpoint" => checkpoint_view(p, &st, arg),
        "coverage" => coverage_view(p),
        _ => Err(refuse(
            format!("build a `{kind}` page"),
            "that is not a page kind",
            &["Use one of: index, gate, questions, ledger, plan, run, checkpoint, coverage."],
            "`bf view <kind> --json` prints the page data.",
        )),
    }
}

/// JSON safe to put inside an HTML script element.
pub fn script_safe(json: &str) -> String {
    json.replace("</", "<\\/")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .replace("<!--", "<\\!--")
}

pub const MARKER: &str = "/*BF_DATA*/";

pub fn render(data: &Value, template: &Path) -> R<String> {
    let t = fs::read_to_string(template).map_err(|e| {
        refuse(
            "render a page",
            format!("the template cannot be read: {e}"),
            &["Give the path of an HTML template with `--template`."],
            "The command writes the HTML file.",
        )
    })?;
    if !t.contains(MARKER) {
        return Err(refuse(
            "render a page",
            format!("the template has no `{MARKER}` marker"),
            &[&format!("Put {MARKER} where the page data should go, for example `const BF_DATA = {MARKER} null;`.")],
            "The command writes the HTML file.",
        ));
    }
    let json = script_safe(&serde_json::to_string(data)?);
    let at = t.find(MARKER).unwrap();
    let (head, tail) = (&t[..at], &t[at + MARKER.len()..]);
    // A literal placeholder right after the marker (`null`, `{}`, `[]`, `undefined`) is replaced too,
    // so `const D = /*BF_DATA*/ null;` and `const D = /*BF_DATA*/;` both work.
    let trimmed = tail.trim_start();
    let mut rest = tail;
    for ph in ["null", "undefined", "{}", "[]"] {
        if let Some(after) = trimmed.strip_prefix(ph) {
            if !after.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                rest = after;
                break;
            }
        }
    }
    let html = format!("{head}{json}{rest}");
    // The page title: a `/*BF_TITLE*/` marker, or the template's default title, takes the page's own.
    let title = data["page"]["title"].as_str().unwrap_or("");
    let esc = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let html = html.replacen("/*BF_TITLE*/", &esc, 1);
    Ok(if title.is_empty() {
        html
    } else {
        html.replacen(
            "<title>Black Factory</title>",
            &format!("<title>{esc} \u{b7} Black Factory</title>"),
            1,
        )
    })
}
