//! Page data in the view contract (version 1) and the template renderer.

pub mod workspace;

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
        "markets" => "Markets".into(),
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

/// What a record's row says about who made it: the designer agent and the model it reported, or the
/// human's words that lifted the designer rule.
fn mark_made(rows: &mut [Value], made: &std::collections::BTreeMap<String, Value>) {
    for row in rows.iter_mut() {
        let Some(m) = row["id"].as_str().and_then(|id| made.get(id)) else {
            continue;
        };
        let model = m["model"].as_str().unwrap_or("");
        let lifted = m["lifted"].as_str().unwrap_or("");
        let (by, line) = if lifted.is_empty() {
            (
                "designer agent",
                format!(
                    "Made by the designer agent ({}) on the model it reported: {model}.",
                    m["role"].as_str().unwrap_or("designer")
                ),
            )
        } else {
            (
                "conductor",
                format!("Made without a designer: the human lifted the rule in their words: \"{lifted}\"."),
            )
        };
        row["made_by"] = json!({"by": by, "role": m["role"], "model": model, "subject": m["subject"], "lifted": lifted});
        if let Some(b) = row["body"].as_array_mut() {
            b.push(text(&line));
        }
    }
}

/// The policy in force for the run: the tier, the effort and any model of each kind of work.
fn policy_section(p: &Project) -> Value {
    let work = run::work_policy(p);
    let rows: Vec<Value> = crate::store::WORK_KINDS
        .iter()
        .map(|k| {
            let e = &work[*k];
            let model = e["model"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(h, v)| format!("{h}: {}", v.as_str().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            json!({"id": k, "title": format!("{}: tier {}, effort {}", k.replace('_', " "), e["tier"].as_str().unwrap_or(""), e["effort"].as_str().unwrap_or("")),
                "tier": e["tier"], "effort": e["effort"], "model": e.get("model").cloned().unwrap_or(Value::Null),
                "handled": {"tone": "accepted", "text": if model.is_empty() { "tier of the harness".to_string() } else { model }},
                "body": []})
        })
        .collect();
    json!({"id": "policy", "title": "Policy in force", "hint": "The tier and effort for each kind of work; only the human changes it", "open": false, "blocks": [], "rows": rows})
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
        ("protects", "Protects"),
        ("attack", "Attacked by"),
        ("mitigation", "Answered by"),
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
    let made = crate::gates::made_by(p)?;
    let mut records: Vec<Value> = Vec::new();
    let mut sections = Vec::new();
    let (mut changed, mut new, mut removed) = (0, 0, 0);
    let kinds: Vec<&str> = match kind.as_str() {
        "idea" => vec!["idea"],
        "requirements" => vec!["requirement"],
        "ui" => vec!["screen", "journey"],
        "architecture" => vec!["component", "decision", "target", "threat", "markets"],
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
        mark_made(&mut rows, &made);
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
        // R-61 and R-76: the wave checks the run uses (or the plain statement that there are none) and the
        // checks every unit must pass, with each override and its reason, are approved with the plan.
        sections.push(crate::wavecheck::plan_section(&pl, st));
        sections.push(crate::wavecheck::unit_checks_section(p, &pl, st));
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
    if crate::checks::review::GATES.contains(&kind.as_str()) {
        v["review"] = crate::checks::review::page_section(p, st, &kind)?;
    }
    sections.push(policy_section(p));
    v["policy"] = run::work_policy(p);
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
            // Every checkpoint either stops for the human or has no stop; the page says which is
            // proposed, and why when a reason is given.
            let stop = crate::enforce::stops(r);
            body.push(json!({"type": "text", "text": format!("{}{}",
                if stop { "Proposed: stops and waits for you." } else { "Proposed: no stop." },
                if s(r, "stop_why").is_empty() { String::new() } else { format!(" {}", s(r, "stop_why")) })}));
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
            body.push(evidence_row(r));
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

/// The design stages a questions page is built for, in their order.
fn is_stage(name: &str) -> bool {
    crate::gates::DESIGN_ORDER.contains(&name)
}

/// The page of the questions of one stage (or of every question when no stage is named, the old page).
/// A time is shown only where the engine recorded the answer's (R-53): the page data holds no time of
/// its own, not even a build time.
fn questions_view(p: &Project, arg: Option<&str>) -> R<Value> {
    use crate::strings::t;
    let qs = p.list_records(schema::kind_by_name("question"))?;
    let stage = arg.filter(|a| is_stage(a));
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
        // The time the engine recorded for the answer, and no other.
        if q.get("answer").is_some() && !s(q, "answered_at").is_empty() {
            item["answered_at"] = json!(s(q, "answered_at"));
        }
        out.push(item);
    }
    let open = out.iter().filter(|q| q["answer"].is_null()).count();
    let intent = p.config.project.clone();
    let mut crumbs = vec![
        t("cli.design.questions.crumb.all", &[]),
        p.config.project.clone(),
        t("cli.design.questions.crumb.questions", &[]),
    ];
    if let Some(st) = stage {
        crumbs.push(st.to_string());
    }
    let status = if out.is_empty() && stage.is_some() {
        (t("cli.design.questions.none", &[]), "ok")
    } else if open > 0 {
        (t("cli.design.questions.status.open", &[]), "wait")
    } else {
        (t("cli.design.questions.status.answered", &[]), "ok")
    };
    let (title, lede) = match stage {
        Some(st) => (
            t(
                "cli.design.questions.title",
                &[("stage", st), ("intent", &intent)],
            ),
            t("cli.design.questions.lede", &[]),
        ),
        None => (
            t("cli.design.questions.title_all", &[]),
            t("cli.design.questions.lede_all", &[]),
        ),
    };
    let id = match stage {
        Some(st) => format!("questions-{st}"),
        None => arg.unwrap_or("all").to_string(),
    };
    let mut v = base(
        p,
        "questions",
        &id,
        crumbs,
        (&status.0, status.1),
        &title,
        &lede,
        vec![
            fact(
                &t("cli.design.questions.fact.open", &[]),
                open,
                &t("cli.design.questions.fact.open_sub", &[]),
            ),
            fact(
                &t("cli.design.questions.fact.answered", &[]),
                out.len() - open,
                &t("cli.design.questions.fact.answered_sub", &[]),
            ),
        ],
    );
    if let Some(o) = v.as_object_mut() {
        o.remove("built_at");
    }
    if stage.is_some() {
        v["intent"] = json!(intent);
        v["stage"] = json!(stage);
        v["back"] =
            json!({"text": t("cli.design.questions.other_stages", &[]), "href": "index.html"});
    }
    v["questions"] = json!(out);
    v["answer"] = json!({"how": "form", "prompt": t("cli.design.questions.answer_prompt", &[])});
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

/// An evidence row of a page. A deliberate repeat (R-57) shows its attempt number and its reason.
pub fn evidence_row(r: &evidence::Receipt) -> Value {
    let mut row = json!({"type": "evidence", "check": r.check(), "command": s(&r.v, "command"), "exit": r.v["exit"].as_i64().unwrap_or(0),
        "commit": &r.content()[..r.content().len().min(12)], "at": s(&r.v, "at"), "output_tail": s(&r.v, "output_tail")});
    if let Some(a) = r.v["attempt"].as_u64() {
        let why = s(&r.v, "reason").trim();
        row["reason"] = json!(if why.is_empty() {
            format!("attempt {a}")
        } else {
            format!("attempt {a}, {why}")
        });
    }
    row
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
    sections.push(crate::wavecheck::plan_section(&pl, st));
    sections.push(crate::wavecheck::unit_checks_section(p, &pl, st));
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

/// A unit's row names its grants: number, files and whether it was withdrawn.
fn grants_line(st: &State, unit: &str) -> Option<String> {
    let sc = st.scope.get(unit).filter(|sc| !sc.grants.is_empty())?;
    Some(format!(
        "Files granted beyond the list: {}.",
        sc.grants
            .iter()
            .map(|g| format!(
                "grant {}{} ({})",
                g.n,
                if g.withdrawn.is_some() {
                    ", withdrawn"
                } else {
                    ""
                },
                g.files
                    .iter()
                    .map(|f| s(f, "path"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .collect::<Vec<_>>()
            .join("; ")
    ))
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
            body.extend(round_lines(&us.review_rounds));
        }
        let fx = crate::ckfix::fixes_of_unit(st, id);
        if !fx.is_empty() {
            body.push(text(&format!(
                "Checkpoint fixes attributed: {}.",
                fx.join(", ")
            )));
        }
        if let Some(l) = grants_line(st, id) {
            body.push(text(&l));
        }
        for c in pl.unit_checks(id) {
            if let Some(r) = rc.iter().rfind(|r| r.check() == s(c, "id") && r.is_run()) {
                body.push(evidence_row(r));
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
    let check_rows = |off: &std::collections::BTreeMap<String, String>| -> Vec<Value> {
        crate::checks::all().iter().map(|c| {
            let why = off.get(c.name);
            let on = c.required && why.is_none();
            json!({"id": c.name, "title": c.name,
                "handled": {"tone": if on { "ok" } else { "open" }, "text": if on { "required" } else { "off" }},
                "body": [text(&format!("Family: {}. Stage: {}. Proof: {}.{}", c.family.name(), c.stage.name(), c.proof.name(),
                    why.map(|r| format!(" Switched off: {r}")).unwrap_or_default()))]})
        }).collect()
    };
    let mut secs = vec![
        json!({"id": "units", "title": "Units", "hint": "State of each", "open": true, "blocks": blocks, "rows": rows}),
        crate::scope::section(st, &pl, None),
        json!({"id": "checks-default", "title": "Checks: project default", "hint": "Each switched-off check shows its reason", "open": false, "blocks": [], "rows": check_rows(&st.checks_default)}),
    ];
    if let Some(r) = run {
        secs.push(json!({"id": "checks-run", "title": format!("Checks: {}", r.id), "hint": "The choice in force for this run", "open": false, "blocks": [], "rows": check_rows(&r.checks_off)}));
    }
    secs.push(policy_section(p));
    v["policy"] = run::work_policy(p);
    let made = crate::gates::made_by(p)?;
    let amends = crate::gates::amendments_json(st);
    if !amends.is_empty() {
        secs.push(json!({"id": "amendments", "title": "Amendments", "hint": "Open, approved or withdrawn", "open": true, "blocks": [],
            "rows": amends.iter().map(|a| json!({"id": a["id"], "title": a["ids"].as_array().map(|x| x.iter().filter_map(|i| i.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                "handled": {"tone": match a["status"].as_str() { Some("approved") => "ok", Some("withdrawn") => "accepted", _ => "open" },
                    "text": if a["repair"] == true { format!("{}, plan repair", a["status"].as_str().unwrap_or("")) } else { a["status"].as_str().unwrap_or("").to_string() }},
                "body": if a["withdrawn_why"].as_str().unwrap_or("").is_empty() { vec![] } else { vec![text(a["withdrawn_why"].as_str().unwrap_or(""))] }})).collect::<Vec<_>>()}));
    }
    // The amendment page: for each amendment that is not withdrawn, the four stages as sections,
    // each with its records or its not affected sentence.
    let mut reviews = serde_json::Map::new();
    for a in st.amends.iter().filter(|a| a.status != "withdrawn") {
        let repair = crate::amend::is_repair(a);
        if !repair {
            reviews.insert(
                a.id.clone(),
                crate::checks::review::page_section(p, st, &a.id)?,
            );
        }
        // What its checkpoint operations would leave: a renamed checkpoint shows under its old number,
        // marked as moved, so the human's read mark follows it.
        let edit = if a.ops.is_empty() {
            None
        } else {
            crate::amend::edit_of(p, st, &a.ops).ok()
        };
        for c in crate::amend::coverage(p, a)? {
            let mut rows: Vec<Value> = if let Some(d) = &c.sentence {
                vec![json!({"id": s(d, "id"), "title": s(d, "title"),
                    "handled": {"tone": "accepted", "text": "not affected"},
                    "body": [text(if s(d, "text").is_empty() { s(d, "chosen") } else { s(d, "text") })]})]
            } else {
                c.records.iter().map(|r| design_row(r, None)).collect()
            };
            mark_made(&mut rows, &made);
            if let (true, "plan") = (edit.is_some(), c.stage) {
                if let Some(e) = &edit {
                    let touched = e.touched();
                    rows.retain(|r| !touched.iter().any(|t| *t == r["id"]));
                    for (id, rec, how) in crate::amend::plan_rows(e) {
                        let mut r = design_row(&rec, None);
                        r["id"] = json!(id);
                        r["handled"] = json!({"tone": "accepted", "text": how});
                        rows.push(r);
                    }
                }
            }
            let tail = if c.covered() || repair {
                ""
            } else {
                " (neither records nor a not affected sentence yet)"
            };
            let mark = if repair { " (plan repair)" } else { "" };
            secs.push(json!({"id": format!("{}-{}", a.id, c.stage),
                "title": format!("{}: {}{}{}", a.id, c.stage, mark, tail),
                "hint": "Its records, or the sentence that says it is not affected",
                "open": true, "blocks": [], "rows": rows}));
        }
        // Everything the narrowing recorded as differing from the approved content, kept records
        // included: the list the fingerprint covers, read from the narrowing, not recomputed.
        if !a.changes.is_empty() {
            let rows: Vec<Value> = a.changes.iter().map(change_row).collect();
            secs.push(json!({"id": format!("{}-changes", a.id),
                "title": format!("{}: everything that differs from the approved content", a.id),
                "hint": "What the narrowing recorded; approval binds exactly this",
                "open": true, "blocks": [], "rows": rows}));
        }
    }
    secs.push(json!({"id": "events", "title": "Recent events", "hint": "Newest first", "open": false, "blocks": [], "rows": recent}));
    v["amendment_reviews"] = Value::Object(reviews);
    v["sections"] = json!(secs);
    Ok(v)
}

/// One value as a line: a string as it is, a list of strings joined, anything else as compact JSON.
fn field_text(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Array(a) if a.iter().all(|i| i.is_string()) => format!(
            "[{}]",
            a.iter()
                .filter_map(|i| i.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

/// Every field that differs between the approved content and the content now, as lines: the field,
/// then the old value, then the new one. A field on one side only shows as added or removed. Walks
/// the records, so it covers every kind and every field.
fn diff_lines(was: &Value, now: &Value) -> Vec<String> {
    let empty = serde_json::Map::new();
    let (w, n) = (
        was.as_object().unwrap_or(&empty),
        now.as_object().unwrap_or(&empty),
    );
    let mut keys: Vec<&String> = w.keys().chain(n.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut out = Vec::new();
    for k in keys {
        match (w.get(k), n.get(k)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => {
                out.push(format!("{k}: was {}, now {}", field_text(a), field_text(b)))
            }
            (None, Some(b)) => out.push(format!("{k}: added {}", field_text(b))),
            (Some(a), None) => out.push(format!("{k}: removed {}", field_text(a))),
            (None, None) => {}
        }
    }
    out
}

/// One row of the list a narrowing stored: the record as it was approved and as it is, with every
/// field that differs, marked as changed, new or removed.
fn change_row(c: &Value) -> Value {
    let (rec, was) = (&c["record"], &c["was"]);
    let how = c["how"].as_str().unwrap_or("changed");
    let title = if s(rec, "title").is_empty() {
        s(was, "title")
    } else {
        s(rec, "title")
    };
    let body: Vec<Value> = diff_lines(was, rec).iter().map(|l| text(l)).collect();
    json!({"id": c["id"], "title": title, "body": body,
        "handled": {"tone": if c["kept"] == true { "open" } else { "accepted" }, "text": how}})
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

/// One line per review round: its number, the content it judged, what it covered and its result.
fn round_lines(rounds: &[Value]) -> Vec<Value> {
    rounds
        .iter()
        .map(|r| {
            let content = r["content"].as_str().unwrap_or("");
            let result = r["result"].as_str().unwrap_or("");
            text(&format!(
                "Review round {} judged content {}; covered: {}.{}",
                r["round"],
                &content[..content.len().min(12)],
                r["covered"].as_str().unwrap_or(""),
                if result.is_empty() {
                    String::new()
                } else {
                    format!(" Result: {result}.")
                }
            ))
        })
        .collect()
}

fn failed_line(x: &Value) -> String {
    let tail = x["tail"]
        .as_str()
        .unwrap_or("")
        .lines()
        .last()
        .unwrap_or("");
    format!(
        "failed check {} (`{}`), exit {}, output ends: {}",
        x["check"].as_str().unwrap_or(""),
        x["command"].as_str().unwrap_or(""),
        if x["exit"].is_null() {
            "none".to_string()
        } else {
            x["exit"].to_string()
        },
        tail
    )
}

fn finding_line(x: &Value) -> String {
    format!(
        "[{}] {} at {}",
        x["severity"].as_str().unwrap_or(""),
        x["title"].as_str().unwrap_or(""),
        x["where"].as_str().unwrap_or("")
    )
}

#[allow(clippy::too_many_arguments)]
fn fix_row(
    id: &str,
    checkpoint: &str,
    source: &str,
    phase: &str,
    note: &str,
    failed: &[Value],
    findings: &[Value],
    files: Option<&[String]>,
    units: &[String],
    by_file: &Value,
    rounds: &[Value],
) -> Value {
    let mut body = vec![text(&format!(
        "Set off by: {}.",
        if source == "check" {
            "a failed checkpoint check"
        } else {
            "findings of the checkpoint review"
        }
    ))];
    for x in failed {
        body.push(text(&format!("- {}", failed_line(x))));
    }
    for x in findings {
        body.push(text(&format!("- finding {}", finding_line(x))));
    }
    body.push(text(&format!(
        "Phase: {phase}.{}",
        if note.is_empty() {
            String::new()
        } else {
            format!(" {note}")
        }
    )));
    body.push(text(&match files {
        Some(f) if !f.is_empty() => format!("Files changed: {}.", f.join(", ")),
        Some(_) => "Files changed: none.".to_string(),
        None => "Files changed: recorded when the fix finishes.".to_string(),
    }));
    if units.is_empty() {
        body.push(text("Units: none attributed yet."));
    } else {
        let per: Vec<String> = by_file
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(f, us)| {
                        let us: Vec<&str> = us
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|u| u.as_str())
                            .collect();
                        format!("{f} -> {}", us.join(", "))
                    })
                    .collect()
            })
            .unwrap_or_default();
        body.push(text(&format!(
            "Units: {}.{}",
            units.join(", "),
            if per.is_empty() {
                String::new()
            } else {
                format!(" By file: {}.", per.join("; "))
            }
        )));
    }
    if rounds.is_empty() {
        body.push(text("Review rounds: none yet."));
    }
    body.extend(round_lines(rounds));
    json!({"id": id, "title": format!("Fix of {checkpoint} ({})", if source == "check" { "check failed" } else { "review findings" }),
        "needs_you": matches!(phase, "parked" | "gap"),
        "handled": {"tone": if phase == "done" { "ok" } else { "open" }, "text": phase},
        "body": body})
}

/// The fixes of a checkpoint: every finished one in order, then the open one.
fn fix_rows(st: &State, kid: &str) -> Vec<Value> {
    let mut rows: Vec<Value> = st
        .ck_fixes
        .iter()
        .filter(|d| d.checkpoint == kid)
        .map(|d| {
            fix_row(
                &d.id,
                kid,
                &d.source,
                "done",
                "Reviewed clean and the checkpoint's checks passed again.",
                &d.origin_failed,
                &d.origin_findings,
                Some(&d.files),
                &d.units,
                &d.by_file,
                &d.rounds_log,
            )
        })
        .collect();
    if let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == kid) {
        let n = rows.len() + 1;
        let mut note = String::new();
        if !f.reason.is_empty() {
            note.push_str(&format!("Reason: {}.", f.reason.trim_end_matches('.')));
        }
        if !f.missing.is_empty() {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(&format!("Missing: {}.", f.missing.join("; ")));
        }
        rows.push(fix_row(
            &format!("{kid}-fix-{n}"),
            kid,
            &f.source,
            &f.phase,
            &note,
            &f.origin_failed,
            &f.origin_findings,
            None,
            &[],
            &Value::Null,
            &f.rounds_log,
        ));
    }
    rows
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
    let fixes = fix_rows(st, &kid);
    let mut rows = rows;
    let mut rbody = vec![];
    let review_rounds = crate::ckreview::rounds(p, &kid)?;
    if review_rounds.is_empty() {
        rbody.push(text("Review rounds: none yet."));
    }
    for r in &review_rounds {
        rbody.push(text(&format!(
            "Review round {} judged content {}; covered: {}.{}",
            r.round,
            &r.content[..r.content.len().min(12)],
            r.covered,
            match r.result.as_deref() {
                Some(x) if !x.is_empty() => format!(" Result: {x}."),
                _ => String::new(),
            }
        )));
        for a in &r.areas {
            rbody.push(text(&format!(
                "Round {} probed: {} ({}).",
                r.round,
                a["area"].as_str().unwrap_or(""),
                a["how"].as_str().unwrap_or("")
            )));
        }
        for x in &r.probes {
            rbody.push(text(&format!(
                "Round {} probe {}: meant to show {}; showed {}.",
                r.round,
                x["name"].as_str().unwrap_or(""),
                x["meant"].as_str().unwrap_or(""),
                x["showed"].as_str().unwrap_or("")
            )));
        }
    }
    let flaky: Vec<String> = rc
        .iter()
        .filter(|r| {
            r.v["flag"].as_str() == Some("flaky") && Some(r.content()) == content.as_deref()
        })
        .map(|r| r.check().to_string())
        .collect();
    rbody.push(text(&if flaky.is_empty() {
        "Flaky: none flagged.".to_string()
    } else {
        format!(
            "Flaky: {} passed on a repeat and was judged acceptable flakiness.",
            flaky.join(", ")
        )
    }));
    rows.push(json!({"id": format!("{kid}-review"), "title": "Checkpoint review", "needs_you": false, "body": rbody}));
    let unit_rows: Vec<Value> = pl
        .units
        .iter()
        .filter(|u| s(u, "checkpoint") == kid)
        .map(|u| {
            let id = s(u, "id");
            let fx = crate::ckfix::fixes_of_unit(st, id);
            let mut body = vec![text(&if fx.is_empty() {
                "Fixes attributed: none.".to_string()
            } else {
                format!("Fixes attributed: {}.", fx.join(", "))
            })];
            if let Some(l) = grants_line(st, id) {
                body.push(text(&l));
            }
            json!({"id": id, "title": s(u, "title"), "body": body})
        })
        .collect();
    let unit_ids: Vec<String> = pl
        .units
        .iter()
        .filter(|u| s(u, "checkpoint") == kid)
        .map(|u| s(u, "id").to_string())
        .collect();
    v["sections"] = json!([
        {"id": "proofs", "title": "Proofs", "hint": "One row per ID", "open": true, "blocks": [], "rows": rows},
        {"id": "fixes", "title": "Fixes", "hint": "What set each fix off, its files, units and review rounds", "open": !fixes.is_empty(), "blocks": [], "rows": fixes},
        crate::scope::section(st, &pl, Some(&unit_ids)),
        {"id": "units", "title": "Units", "hint": "The fixes attributed to each unit", "open": false, "blocks": [], "rows": unit_rows},
    ]);
    v["answer"] = json!({"how": "chat", "prompt": "Close with `bf checkpoint close` once every row is proven."});
    Ok(v)
}

fn when(at: &str) -> String {
    at.get(..16).unwrap_or(at).replace('T', " ")
}

fn page_row(id: &str, title: &str, href: &str, meta: &str, note: &str) -> Value {
    json!({"id": id, "title": title, "href": href, "meta": meta, "body": [text(note)]})
}

/// The list of pages. File names follow one convention: `gate-<gate id>.html`, `questions-<stage>.html` (one for each stage that has questions; `questions.html`
/// only for a page recorded under the old name),
/// `plan.html`, `run.html`, `ledger.html`, `coverage.html` and `checkpoint-<K>.html`.
fn index_view(p: &Project, st: &State) -> R<Value> {
    use crate::strings::t;
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
    // One row for each stage that has questions, in the order of the stages (R-29, D-36). A project
    // whose only questions page is under the old name has one row for it, without a stage.
    let mut stages: Vec<String> = crate::gates::DESIGN_ORDER
        .iter()
        .map(|x| x.to_string())
        .collect();
    for q in &all_q {
        let sg = s(q, "stage");
        if !sg.is_empty() && !stages.iter().any(|x| x == sg) {
            stages.push(sg.to_string());
        }
    }
    let counts = |sg: &str| -> (usize, usize) {
        let mine: Vec<&Value> = all_q.iter().filter(|q| s(q, "stage") == sg).collect();
        let open = mine.iter().filter(|q| q.get("answer").is_none()).count();
        (open, mine.len() - open)
    };
    // A unit's page `questions-U-n` is no stage page: only the design stages' pages count.
    let old_only = st.pages.contains_key("questions")
        && !crate::gates::DESIGN_ORDER
            .iter()
            .any(|sg| st.pages.contains_key(&format!("questions-{sg}")));
    let mut question_rows = Vec::new();
    if old_only {
        let open = open_q;
        if !all_q.is_empty() {
            let mut r = page_row(
                "questions",
                &t("cli.design.index.row.old", &[]),
                "questions.html",
                &t(
                    "cli.design.index.row.old_counts",
                    &[
                        ("open", &open.to_string()),
                        ("total", &all_q.len().to_string()),
                    ],
                ),
                "",
            );
            let tag = if open > 0 { "open" } else { "answered" };
            r["handled"] = json!({"tone": if open > 0 { "open" } else { "ok" }, "text": t(&format!("cli.design.index.tag.{tag}"), &[])});
            if open > 0 {
                let mut w = r.clone();
                w["title"] = json!(t(
                    "cli.design.index.waiting.old",
                    &[("open", &open.to_string())]
                ));
                w["needs_you"] = json!(true);
                waiting.push(w);
            }
            question_rows.push(r);
        }
    } else {
        for sg in &stages {
            let (open, answered) = counts(sg);
            if open + answered == 0 {
                continue;
            }
            let href = format!("questions-{sg}.html");
            let mut r = page_row(
                &format!("questions-{sg}"),
                &t("cli.design.index.row.title", &[("stage", sg)]),
                &href,
                &t(
                    "cli.design.index.row.counts",
                    &[
                        ("open", &open.to_string()),
                        ("answered", &answered.to_string()),
                    ],
                ),
                "",
            );
            let tag = if open > 0 { "open" } else { "answered" };
            r["handled"] = json!({"tone": if open > 0 { "open" } else { "ok" }, "text": t(&format!("cli.design.index.tag.{tag}"), &[])});
            if open > 0 {
                let mut w = page_row(
                    &format!("questions-{sg}"),
                    &t(
                        "cli.design.index.waiting.title",
                        &[("open", &open.to_string()), ("stage", sg)],
                    ),
                    &href,
                    "",
                    &t("cli.design.index.waiting.note", &[]),
                );
                w["needs_you"] = json!(true);
                waiting.push(w);
            }
            question_rows.push(r);
        }
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
    let n_pages = waiting.len() + state_rows.len() + history.len() + question_rows.len();
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
    if !question_rows.is_empty() {
        sections.push(json!({"id": "questions", "title": t("cli.design.index.section.title", &[]), "hint": t("cli.design.index.section.hint", &[]), "open": true, "rows": question_rows}));
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
