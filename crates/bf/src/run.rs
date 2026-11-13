//! Runs: start, status, next directives, reports, briefs, unit working copies,
//! landing, checkpoint closing, stuck units and cost.

use crate::events::{self, State};
use crate::evidence::{self, Receipt};
use crate::gates;
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse, refuse_owned, R};
use crate::schema;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// Test hook: exit abruptly at a named point when `BF_CRASH_AT` names it.
pub fn crash_point(name: &str) {
    if std::env::var("BF_CRASH_AT")
        .map(|v| v == name)
        .unwrap_or(false)
    {
        std::process::exit(137);
    }
}

/// A finding is open until its outcome says fixed or accepted.
pub fn is_open(f: &Value) -> bool {
    !matches!(s(f, "outcome"), "fixed" | "accepted")
}

fn need_run(st: &State) -> R<String> {
    st.active_run().map(|r| r.id.clone()).ok_or_else(|| {
        refuse(
            "continue without a run",
            "no run is active",
            &["Run `bf run start`."],
            "`bf status` shows an active run.",
        )
    })
}

pub fn run_start(p: &Project) -> R<Value> {
    let st = events::require_chain(p)?;
    if let Some(r) = st.active_run() {
        return Err(refuse(
            "start a run",
            format!("run {} is already active", r.id),
            &["Continue it with `bf next`, or end it with `bf run stop`."],
            "`bf status` shows one active run.",
        ));
    }
    let pl = plan::load(p)?;
    plan::check_or_refuse(&pl)?;
    let id = format!("run-{}", st.runs.len() + 1);
    let grant = st.pending_grants.last().copied();
    events::append(p, "run.start", json!({"run": id, "grant": grant}))?;
    p.write_guard()?;
    Ok(json!({"run": id, "test_mode": grant.is_some()}))
}

pub fn run_stop(p: &Project) -> R<Value> {
    let st = events::require_chain(p)?;
    let id = need_run(&st)?;
    events::append(p, "run.stop", json!({"run": id}))?;
    p.write_guard()?;
    Ok(json!({"run": id, "stopped": true}))
}

/// Computed state of every unit: planned, ready, or the logged state.
pub fn unit_states(
    p: &Project,
    pl: &Plan,
    st: &State,
    receipts: &[Receipt],
) -> Vec<(String, String, Option<String>)> {
    let mut out = Vec::new();
    for u in &pl.units {
        let id = s(u, "id").to_string();
        let logged = st.unit_state(&id);
        if !logged.is_empty() {
            out.push((id, logged.to_string(), None));
        } else if let Some(why) = plan::not_ready(p, pl, st, &id, receipts) {
            out.push((id, "planned".into(), Some(why)));
        } else {
            out.push((id, "ready".into(), None));
        }
    }
    out
}

pub fn status(p: &Project) -> R<Value> {
    let st = events::replay(p)?;
    let main = main_position(p, &st)?;
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let states = unit_states(p, &pl, &st, &receipts);
    let mut counts = serde_json::Map::new();
    for (_, s_, _) in &states {
        let c = counts.entry(s_.clone()).or_insert(json!(0));
        *c = json!(c.as_u64().unwrap_or(0) + 1);
    }
    let run = st.active_run();
    let test = match (run.map(|r| r.test_mode), st.pending_grants.is_empty()) {
        (Some(true), _) => "granted",
        (_, false) => "pending",
        _ => "none",
    };
    Ok(json!({
        "project": p.config.project,
        "run": run.map(|r| r.id.clone()),
        "test_mode": test,
        "chain_ok": st.chain_ok,
        "events": st.event_count,
        "units": states.iter().map(|(i, s_, why)| json!({"id": i, "state": s_, "waiting_for": why})).collect::<Vec<_>>(),
        "unit_counts": counts,
        "gates": st.gates.iter().map(gates::gate_json).collect::<Vec<_>>(),
        "checkpoints_closed": st.closed.keys().collect::<Vec<_>>(),
        "spent": st.cost,
        "budget_cap": p.policy.budget.cap,
        "ignored_events": st.ignored,
        "frozen_drift": gates::drift(p, &st)?,
        "main": {"ok": !main.moved, "detail": main.detail, "note": main.note, "tip": main.tip},
        "policy": format!("{:?}", p.policy_state),
        "bad_records": pl.bad.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>(),
    }))
}

fn tier_for(p: &Project, unit: &Value, role: &str) -> String {
    if let Some(t) = unit.get("tier").and_then(|v| v.as_str()) {
        if role == "builder" {
            return t.to_string();
        }
    }
    match role {
        "reviewer" => p.policy.tiers.reviewer.clone(),
        "verifier" => p.policy.tiers.verifier.clone(),
        _ => p.policy.tiers.builder.clone(),
    }
}

fn budget_check(p: &Project, st: &State) -> R<()> {
    let cap = p.policy.budget.cap;
    if cap > 0.0 && st.cost >= cap {
        return Err(refuse(
            "start new work",
            format!("the budget cap is reached: spent {:.2} of {:.2} {}", st.cost, cap, p.policy.budget.unit),
            &["Ask the human whether to raise `budget.cap` in factory/policy.toml (only they edit it).", "Run `bf cost status` to see where it went."],
            "`bf next` returns directives again.",
        ));
    }
    Ok(())
}

fn next_raw(p: &Project) -> R<Value> {
    let st = events::require_chain(p)?;
    let run = need_run(&st)?;
    budget_check(p, &st)?;
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let mut directives: Vec<Value> = Vec::new();
    let mut waiting: Vec<String> = Vec::new();

    match st.latest_gate("plan", None) {
        Some(g) if g.status == "approved" => {}
        Some(g) if g.status == "open" => {
            directives.push(json!({"role": "human", "action": "decide-gate", "gate": g.id, "note": "Ask the human to approve or reject the plan in chat."}));
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
        }
        _ => {
            directives.push(json!({"role": "human", "action": "open-gate", "gate_kind": "plan", "command": "bf gate open plan"}));
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
        }
    }
    let limits = |us_rounds: u32| {
        json!({"max_rounds": p.policy.limits.max_rounds, "round": us_rounds, "idle_minutes": p.policy.limits.idle_minutes,
               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }})
    };
    for (id, state, why) in unit_states(p, &pl, &st, &receipts) {
        let u = pl.unit(&id).unwrap();
        let wc = p.work_dir().join(&id).to_string_lossy().to_string();
        let rounds = st.units.get(&id).map(|x| x.rounds).unwrap_or(0);
        let brief = |role: &str| {
            p.briefs_dir()
                .join(format!("{id}-{role}.md"))
                .to_string_lossy()
                .to_string()
        };
        match state.as_str() {
            "ready" => directives.push(json!({"role": "builder", "action": "start-unit", "unit": id, "tier": tier_for(p, u, "builder"),
                "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                "commands": [format!("bf unit start {id}"), format!("bf brief {id} --role builder")]})),
            "running" | "fixing" => directives.push(json!({"role": "builder", "action": if state == "fixing" { "fix" } else { "build" }, "unit": id,
                "tier": tier_for(p, u, "builder"), "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                "commands": [format!("bf brief {id} --role builder")]})),
            "review" => directives.push(json!({"role": "reviewer", "action": "review", "unit": id, "tier": tier_for(p, u, "reviewer"),
                "working_copy": wc, "brief": brief("reviewer"), "limits": limits(rounds),
                "commands": [format!("bf brief {id} --role reviewer")]})),
            "reviewed" => directives.push(json!({"role": "engine", "action": "land", "unit": id, "command": format!("bf unit land {id}")})),
            "blocked" | "parked" => directives.push(json!({"role": "human", "action": "decide-unit", "unit": id, "state": state,
                "note": format!("{id} is {state}; the human decides whether to re-plan or release it.")})),
            "planned" => waiting.push(format!("{id}: {}", why.unwrap_or_default())),
            _ => {}
        }
    }
    // Checkpoints whose units have all landed.
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        let after = str_list(k, "after");
        if st.closed.contains_key(kid) {
            continue;
        }
        if !after.is_empty() && after.iter().all(|u| st.unit_state(u) == "landed") {
            let wc = p.work_dir().join(kid).to_string_lossy().to_string();
            let content = gitx::content_hash(&p.work_dir().join(kid)).unwrap_or_default();
            let missing: Vec<String> = str_list(k, "checks")
                .into_iter()
                .filter(|c| evidence::satisfied_at(&receipts, c, &content).is_none())
                .collect();
            let human_needed =
                !s(k, "human_step").is_empty() && !st.gate_passed("checkpoint-step", Some(kid));
            let marked: Vec<Value> = str_list(k, "checks")
                .iter()
                .filter_map(|c| {
                    evidence::standing_outcome(&receipts, c, &content).map(|r| {
                        json!({"check": c, "outcome": r.outcome().unwrap_or(""), "reason": s(&r.v, "reason")})
                    })
                })
                .collect();
            if !marked.is_empty() {
                directives.push(json!({"role": "human", "action": "decide-check", "checkpoint": kid, "checks": marked,
                    "note": "A check failed or could not be run. The human decides: fix the cause and have the verifier run it again (a passing run replaces the mark), accept the gap by amendment, or change the plan."}));
            } else if !missing.is_empty() {
                directives.push(json!({"role": "verifier", "action": "run-checkpoint-checks", "checkpoint": kid, "checks": missing,
                    "tier": p.policy.tiers.verifier, "working_copy": wc,
                    "brief": p.briefs_dir().join(format!("{kid}-verifier.md")).to_string_lossy(),
                    "limits": {"max_rounds": p.policy.limits.max_rounds, "idle_minutes": p.policy.limits.idle_minutes,
                               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }},
                    "commands": [format!("bf brief {kid} --role verifier")]}));
            } else if human_needed {
                let open = st
                    .latest_gate("checkpoint-step", Some(kid))
                    .map(|g| g.status == "open")
                    .unwrap_or(false);
                let step_gate = st
                    .latest_gate("checkpoint-step", Some(kid))
                    .filter(|g| g.status == "open")
                    .map(|g| g.id.clone());
                directives.push(json!({"role": "human", "action": if open { "decide-gate" } else { "open-gate" }, "gate_kind": "checkpoint-step", "checkpoint": kid, "gate": step_gate,
                    "on_human_step": s(k, "on_human_step"),
                    "note": s(k, "human_step")}));
            } else {
                directives.push(json!({"role": "engine", "action": "close-checkpoint", "checkpoint": kid, "command": format!("bf checkpoint close {kid}")}));
            }
        } else {
            waiting.push(format!("{kid}: waiting for its units to land"));
        }
    }
    Ok(json!({"run": run, "directives": directives, "waiting": waiting}))
}

fn read_tokens(p: &Project) -> Value {
    std::fs::read_to_string(p.tokens_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn read_offers(p: &Project) -> Value {
    std::fs::read_to_string(p.offers_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

/// `bf token take <unit> --role R`: the one-use token for a directive that the latest `bf next` gave.
/// It is handed out once. A second request fails loudly, so that whoever got the token first, if it
/// was not the conductor, is noticed.
pub fn take_token(p: &Project, unit: &str, role: &str) -> R<String> {
    let mut offers = read_offers(p);
    let key = format!("{unit}:{role}");
    let Some(o) = offers.get(&key).cloned() else {
        return Err(refuse(
            format!("take the {role} token of {unit}"),
            "the latest `bf next` did not give that directive, and a token is handed out only for a directive just given",
            &["Run `bf next --json` and take the token of a directive it lists (its `token_command`)."],
            "`bf token take` prints a token.",
        ));
    };
    let token = util::random_hex(12)?;
    let mut v = read_tokens(p);
    if o["taken"] == true {
        // Taken before (a conductor that lost its context asks again, or someone else asked first).
        // The earlier token is void and a new one is issued; the note says so, so whoever held the
        // earlier token finds out when its report fails.
        eprintln!(
            "note: the {role} token of {unit} was already taken (at {}); that token is void now. If it was not you who took it, tell the human.",
            o["at"].as_str().unwrap_or("an earlier time")
        );
        if let Some(m) = v.as_object_mut() {
            m.retain(|_, e| !(e["unit"] == unit && e["role"] == role));
        }
    }
    if let Some(m) = v.as_object_mut() {
        m.insert(
            util::sha256_hex(token.as_bytes()),
            json!({"unit": unit, "role": role}),
        );
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    offers[&key]["taken"] = json!(true);
    offers[&key]["at"] = json!(util::now());
    util::atomic_write(&p.offers_path(), serde_json::to_string(&offers)?.as_bytes())?;
    Ok(token)
}

/// `bf tokens reset`: the human clears every unspent role token, so the next `bf next` hands out new ones.
pub fn reset_tokens(p: &Project) -> R<()> {
    util::atomic_write(&p.tokens_path(), b"{}")?;
    util::atomic_write(&p.offers_path(), b"{}")?;
    Ok(())
}

fn check_token(p: &Project, unit: &str, role: &str, token: Option<&str>) -> R<String> {
    let hash = token
        .map(|t| util::sha256_hex(t.trim().as_bytes()))
        .unwrap_or_default();
    let v = read_tokens(p);
    let ok = !hash.is_empty()
        && v.get(&hash)
            .map(|e| e["unit"] == unit && e["role"] == role)
            .unwrap_or(false);
    if ok {
        return Ok(hash);
    }
    Err(refuse(
        format!("report on {unit} as {role}"),
        format!("a report needs the one-use token that `bf next` handed out for the {role} of {unit}; none, or the wrong one, was given"),
        &[
            "The conductor runs `bf next --json` and passes each worker the `token` of its directive.",
            "The worker reports with `--token <token>`. A builder cannot report as the reviewer: it never holds the reviewer's token.",
        ],
        "`bf status` shows the unit in its new state.",
    ))
}

fn spend_token(p: &Project, hash: &str) -> R<()> {
    let mut v = read_tokens(p);
    if let Some(o) = v.as_object_mut() {
        o.remove(hash);
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(())
}

/// The next directives, plus a plain statement of whether anything can start and, if not, why.
pub fn next(p: &Project) -> R<Value> {
    let mut v = next_raw(p)?;
    // A worker directive can be given a token, once, to whoever asks for it with `bf token take`. The
    // offers are exactly the directives this call returned.
    if p.policy.roles.tokens {
        let mut offers = read_offers(p);
        let tokens = read_tokens(p);
        let mut fresh = json!({});
        if let Some(ds) = v["directives"].as_array_mut() {
            for d in ds.iter_mut() {
                let (role, unit) = (
                    d["role"].as_str().unwrap_or("").to_string(),
                    d["unit"].as_str().unwrap_or("").to_string(),
                );
                if matches!(role.as_str(), "builder" | "reviewer") && !unit.is_empty() {
                    let key = format!("{unit}:{role}");
                    // A directive whose token was taken and is still unspent stays "taken"; otherwise it is new.
                    let unspent = tokens
                        .as_object()
                        .map(|o| {
                            o.values()
                                .any(|e| e["unit"] == unit.as_str() && e["role"] == role.as_str())
                        })
                        .unwrap_or(false);
                    let taken = offers
                        .get(&key)
                        .map(|o| o["taken"] == true)
                        .unwrap_or(false)
                        && unspent;
                    fresh[&key] = json!({"unit": unit, "role": role, "taken": taken, "at": offers.get(&key).and_then(|o| o.get("at")).cloned().unwrap_or(Value::Null)});
                    d["token_command"] = json!(format!("bf token take {unit} --role {role}"));
                }
            }
        }
        offers = fresh;
        util::atomic_write(&p.offers_path(), serde_json::to_string(&offers)?.as_bytes())?;
    }
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let dirs = v["directives"].as_array().cloned().unwrap_or_default();
    let can_start = dirs.iter().any(|d| d["role"] != "human");
    let human_needed = dirs.iter().any(|d| d["role"] == "human");
    let all_landed = !pl.units.is_empty()
        && pl
            .units
            .iter()
            .all(|u| st.unit_state(s(u, "id")) == "landed");
    let all_closed = pl
        .checkpoints
        .iter()
        .all(|k| st.closed.contains_key(s(k, "id")));
    let reason = if can_start {
        Value::Null
    } else if all_landed && all_closed {
        json!("run done: every unit has landed and every checkpoint is closed")
    } else if human_needed {
        let parked: Vec<String> = st
            .units
            .iter()
            .filter(|(_, u)| matches!(u.state.as_str(), "parked" | "blocked"))
            .map(|(id, u)| format!("{id} ({})", u.state))
            .collect();
        if let Some(d) = dirs.iter().find(|d| d["action"] == "decide-check") {
            json!(format!(
                "a check of {} failed or is unreachable; the human decides",
                d["checkpoint"].as_str().unwrap_or("a checkpoint")
            ))
        } else if parked.is_empty() {
            json!("waiting on a human step; see the human directives")
        } else {
            json!(format!(
                "units parked or blocked: {}; the human decides",
                parked.join(", ")
            ))
        }
    } else {
        let w: Vec<String> = v["waiting"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        json!(format!(
            "nothing is ready: {}",
            if w.is_empty() {
                "no unit is waiting either".into()
            } else {
                w.join("; ")
            }
        ))
    };
    v["can_start"] = json!(can_start);
    v["human_needed"] = json!(human_needed);
    v["idle_reason"] = reason;
    Ok(v)
}

#[allow(clippy::too_many_arguments)]
pub fn report(
    p: &Project,
    unit: &str,
    result: &str,
    role: Option<&str>,
    note: &str,
    findings: Option<&Value>,
    cost: f64,
    token: Option<&str>,
) -> R<Value> {
    let st = events::require_chain(p)?;
    need_run(&st)?;
    let pl = plan::load(p)?;
    if pl.unit(unit).is_none() {
        return Err(refuse(
            format!("report on {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "`bf status` lists the unit.",
        ));
    }
    if !["built", "findings", "fixed", "blocked"].contains(&result) {
        return Err(refuse(
            format!("report `{result}`"),
            "that is not a result the engine knows",
            &["Use one of: built, findings, fixed, blocked."],
            "The command prints the unit's new state.",
        ));
    }
    let cur = st.unit_state(unit).to_string();
    let expected_role = if cur == "review" {
        "reviewer"
    } else {
        "builder"
    };
    // A unit in review is judged by the reviewer, who must say so: a builder's repeated `built`
    // must never read as a clean review.
    if cur == "review" && role != Some("reviewer") {
        return Err(refuse(
            format!("report {result} on {unit}"),
            format!("{unit} is waiting for its reviewer; reports on it need `--role reviewer`"),
            &["The cold reviewer reports with `bf report <unit> --result findings|built --role reviewer`.", "A builder waits until the unit comes back as `fixing`."],
            "`bf status` shows the unit in its new state.",
        ));
    }
    if let Some(r) = role {
        if r != expected_role && !cur.is_empty() {
            return Err(refuse(
                format!("report as {r}"),
                format!("{unit} is `{cur}`, so the next report comes from the {expected_role}"),
                &["Report with the role the unit is waiting for."],
                "The command prints the unit's new state.",
            ));
        }
    }
    // Findings given in a file, and findings already filed with `bf record put finding`.
    let listed: Vec<Value> = findings
        .and_then(|f| {
            f.as_array()
                .cloned()
                .or_else(|| f.get("findings").and_then(|x| x.as_array().cloned()))
        })
        .unwrap_or_default();
    let open_filed = p
        .list_records(schema::kind_by_name("finding"))?
        .iter()
        .filter(|f| s(f, "unit") == unit && is_open(f))
        .count();
    // A reviewer with nothing to report says `findings` with no finding on file: the unit moves on.
    let result = if result == "findings" && cur == "review" && listed.is_empty() && open_filed == 0
    {
        "built"
    } else {
        result
    };
    let token_hash = if p.policy.roles.tokens && !cur.is_empty() {
        Some(check_token(p, unit, expected_role, token)?)
    } else {
        None
    };
    let review_required = p.policy.review.required;
    let mut event_kind = "unit.report";
    let to: String = match (cur.as_str(), result) {
        ("running", "built") | ("fixing", "built") | ("fixing", "fixed") => {
            if review_required {
                "review".into()
            } else {
                "reviewed".into()
            }
        }
        ("running" | "fixing", "blocked") => "blocked".into(),
        ("review", "findings") => {
            let rounds = st.units.get(unit).map(|u| u.rounds).unwrap_or(0) + 1;
            if rounds > p.policy.limits.max_rounds {
                event_kind = "unit.report+park";
                "parked".into()
            } else {
                "fixing".into()
            }
        }
        ("review", "built") => "reviewed".into(),
        (c, r) => {
            let why = match c {
                "" => format!("{unit} has not been started"),
                "reviewed" | "landed" => format!("{unit} is already `{c}`"),
                "blocked" | "parked" => format!("{unit} is `{c}` and waits for the human"),
                _ => format!("a `{r}` report does not fit a unit that is `{c}`"),
            };
            return Err(refuse(
                format!("report {r} on {unit}"),
                why,
                &["Check the state with `bf status`.", "Builders report built, fixed or blocked; reviewers report findings or built (clean)."],
                "`bf status` shows the unit in its new state.",
            ));
        }
    };
    // Findings.
    let mut finding_ids: Vec<String> = Vec::new();
    if result == "findings" {
        let kind = schema::kind_by_name("finding").unwrap();
        for f in listed {
            let id = p.next_id(kind)?;
            let mut rec = json!({"id": id, "kind": "finding", "unit": unit, "outcome": "open"});
            for k in ["title", "severity", "where", "action", "text"] {
                if let Some(v) = f.get(k) {
                    rec[k] = v.clone();
                }
            }
            schema::validate_or_refuse(kind, &rec)?;
            p.write_record(&rec)?;
            finding_ids.push(id);
        }
    }
    let findings_all = p.list_records(schema::kind_by_name("finding"))?;
    let mine: Vec<&Value> = findings_all
        .iter()
        .filter(|f| s(f, "unit") == unit)
        .collect();
    if cur == "review" && result == "built" {
        let open: Vec<&str> = mine
            .iter()
            .filter(|f| is_open(f))
            .map(|f| s(f, "id"))
            .collect();
        if !open.is_empty() {
            return Err(refuse(
                format!("report {unit} clean"),
                format!("findings are still open: {}", open.join(", ")),
                &[
                    "Have the builder fix them and report `fixed`.",
                    "Or have the human accept them (outcome accepted).",
                ],
                "`bf report` is accepted and the unit is `reviewed`.",
            ));
        }
    }
    if cur == "fixing" {
        for f in mine.iter().filter(|f| is_open(f)) {
            let mut r = (*f).clone();
            r["outcome"] = json!("fixed");
            p.write_record(&r)?;
        }
    }
    let cost_total = cost.max(0.0);
    if let Some(h) = &token_hash {
        spend_token(p, h)?;
    }
    let ev_kind = "unit.report";
    let _ = event_kind;
    events::append(
        p,
        ev_kind,
        json!({"unit": unit, "result": result, "role": expected_role, "from": cur, "to": to, "note": note, "cost": cost_total, "findings": finding_ids}),
    )?;
    if to == "parked" {
        events::append(
            p,
            "unit.park",
            json!({"unit": unit, "reason": format!("more than {} review rounds", p.policy.limits.max_rounds)}),
        )?;
    }
    p.write_guard()?;
    Ok(json!({"unit": unit, "result": result, "state": to, "findings": finding_ids}))
}

fn brief_checkpoint(p: &Project, k: &str, role: &str) -> R<PathBuf> {
    if role != "verifier" {
        return Err(refuse(
            format!("write a brief for {k} as {role}"),
            "a checkpoint is briefed to its verifier only",
            &["Use --role verifier, or give a unit ID."],
            "The command prints the path of the brief.",
        ));
    }
    let pl = plan::load(p)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(refuse(
            format!("brief {k}"),
            "no such checkpoint",
            &["Use a K- ID from `bf record list --kind checkpoint`."],
            "The command prints a path.",
        ));
    };
    let mut md = format!(
        "# Brief: {k} as verifier\n\nCheckpoint: {}\n\n",
        s(kr, "title")
    );
    md.push_str("You are a blind verifier: judge the built result against the contract below. Do not read the source.\n\n## What this checkpoint proves\n\n");
    for id in str_list(kr, "proves") {
        if let Some(r) = p.get_record(&id)? {
            md.push_str(&format!("### {} {}\n\n", id, s(&r, "title")));
            if !s(&r, "text").is_empty() {
                md.push_str(&format!("{}\n\n", s(&r, "text")));
            }
            for a in str_list(&r, "acceptance") {
                md.push_str(&format!("- Acceptance: {a}\n"));
            }
            md.push('\n');
        }
    }
    md.push_str("## Checks\n\n");
    let wc = p.work_dir().join(k);
    for cid in str_list(kr, "checks") {
        if let Some(c) = pl.checks.iter().find(|c| s(c, "id") == cid) {
            let cmd = match c.get("command") {
                Some(Value::String(x)) => x.clone(),
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            md.push_str(&format!(
                "- {} ({}), proves {}: ",
                cid,
                s(c, "title"),
                str_list(c, "proves").join(", ")
            ));
            if s(c, "how") == "command" {
                md.push_str(&format!(
                    "run `bf evidence run {cid} -- {cmd}` inside `{}`\n",
                    wc.display()
                ));
            } else {
                md.push_str(&format!("{} check: store the proof with `bf evidence attach {cid} <file>` inside `{}`\n", s(c, "how"), wc.display()));
            }
        }
    }
    if !s(kr, "human_step").is_empty() {
        md.push_str(&format!("\n## Human step\n\n{}\n", s(kr, "human_step")));
    }
    md.push_str(&format!(
        "\n## Outcomes\n\nRun each check from inside `{wc}` so the receipt is for the checkpoint's content. Then, for each ID you judged:\n\n- met: the receipt from `bf evidence run` is the proof.\n- failed: the check ran and does not match the contract, or it fails without a receipt you can show: `bf evidence fail <check> --reason \"what you saw\"`.\n- unreachable: you could not run it: `bf evidence unreachable <check> --reason \"what blocked you\"`. This is not a pass. The checkpoint then waits for the human.\n\nGive your outcomes in your last message too.\n\n## Rules\n\n- Never edit anything under `factory/` or `.bf/` (the working copy named above is the only place under `.bf/` you may use).\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n",
        wc = wc.display()
    ));
    let path = p.briefs_dir().join(format!("{k}-verifier.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

pub fn brief(p: &Project, unit: &str, role: &str) -> R<PathBuf> {
    if unit.starts_with("K-") {
        return brief_checkpoint(p, unit, role);
    }
    if !["builder", "reviewer", "verifier"].contains(&role) {
        return Err(refuse(
            format!("write a brief for role `{role}`"),
            "that is not a role the engine briefs",
            &["Use --role builder, reviewer or verifier."],
            "The command prints the path of the brief.",
        ));
    }
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("brief {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints a path.",
        ));
    };
    let st = events::replay(p)?;
    let mut md = String::new();
    md.push_str(&format!("# Brief: {} as {}\n\n", unit, role));
    md.push_str(&format!("Unit: {}\n\n", s(u, "title")));
    if !s(u, "text").is_empty() {
        md.push_str(&format!("{}\n\n", s(u, "text")));
    }
    md.push_str("## What this unit delivers\n\n");
    for id in str_list(u, "delivers") {
        if let Some(r) = p.get_record(&id)? {
            md.push_str(&format!("### {} {}\n\n", id, s(&r, "title")));
            if !s(&r, "text").is_empty() {
                md.push_str(&format!("{}\n\n", s(&r, "text")));
            }
            for a in str_list(&r, "acceptance") {
                md.push_str(&format!("- Acceptance: {a}\n"));
            }
            md.push('\n');
        }
    }
    let received: Vec<&Value> = pl.handoffs.iter().filter(|h| s(h, "to") == unit).collect();
    if !received.is_empty() {
        md.push_str("## Contracts you receive\n\n");
        for h in received {
            md.push_str(&format!(
                "- {}: {} from {} ({})\n",
                s(h, "id"),
                s(h, "thing"),
                s(h, "from"),
                s(h, "contract")
            ));
        }
        md.push('\n');
    }
    let given: Vec<&Value> = pl
        .handoffs
        .iter()
        .filter(|h| s(h, "from") == unit)
        .collect();
    if !given.is_empty() {
        md.push_str("## Contracts you hand over\n\n");
        for h in given {
            md.push_str(&format!(
                "- {}: {} to {} ({})\n",
                s(h, "id"),
                s(h, "thing"),
                s(h, "to"),
                s(h, "contract")
            ));
        }
        md.push('\n');
    }
    md.push_str("## Checks\n\n");
    for c in pl.unit_checks(unit) {
        let cmd = match c.get("command") {
            Some(Value::String(x)) => x.clone(),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        md.push_str(&format!(
            "- {} ({}): `{}`. Run it with `bf evidence run {} --unit {} -- {}`\n",
            s(c, "id"),
            s(c, "title"),
            cmd,
            s(c, "id"),
            unit,
            cmd
        ));
    }
    md.push('\n');
    if role != "verifier" {
        md.push_str("## Where you work\n\n");
        md.push_str(&format!(
            "Working copy: `{}`\n\nFiles you may change:\n",
            p.work_dir().join(unit).display()
        ));
        for f in str_list(u, "files") {
            md.push_str(&format!("- `{f}`\n"));
        }
        md.push('\n');
    } else {
        md.push_str("## Where you work\n\nYou are a blind verifier: judge the built result against the contract above. Do not read the source.\n\n");
    }
    let fs_all = p.list_records(schema::kind_by_name("finding"))?;
    let mine: Vec<&Value> = fs_all.iter().filter(|f| s(f, "unit") == unit).collect();
    if !mine.is_empty() {
        md.push_str("## Findings\n\n");
        for f in mine {
            md.push_str(&format!(
                "- {} [{}] {} at {}: {} ({})\n",
                s(f, "id"),
                s(f, "severity"),
                s(f, "title"),
                s(f, "where"),
                s(f, "action"),
                s(f, "outcome")
            ));
        }
        md.push('\n');
    }
    if role == "reviewer" {
        md.push_str("## Receipts\n\nVerify these instead of re-running the checks:\n\n");
        for r in evidence::load_receipts(p)? {
            if pl.unit_checks(unit).iter().any(|c| s(c, "id") == r.check()) {
                md.push_str(&format!(
                    "- {} for {}: sealed={}, exit={}, content {}\n",
                    r.id(),
                    r.check(),
                    r.seal_ok,
                    r.v["exit"],
                    r.content()
                ));
            }
        }
        md.push('\n');
    }
    md.push_str("## Limits\n\n");
    md.push_str(&format!(
        "- Review rounds: up to {} (so far {}).\n- Idle limit: {} minutes.\n",
        p.policy.limits.max_rounds,
        st.units.get(unit).map(|x| x.rounds).unwrap_or(0),
        p.policy.limits.idle_minutes
    ));
    md.push_str(&format!(
        "\n## Rules\n\n- Report with `bf report`.\n- Work only inside your working copy, `{}`. It lives under the engine's `.bf/work/` folder on purpose; do not touch anything else under `.bf/`, and leave the `factory/` folder inside your working copy alone: records change through the engine only.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- A check counts only when `bf evidence run` ran it.\n",
        p.work_dir().join(unit).display()
    ));
    let path = p.briefs_dir().join(format!("{unit}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

fn ensure_checkpoint_worktree(p: &Project, k: &str) -> R<PathBuf> {
    let main = &p.config.main_branch;
    if !gitx::branch_exists(&p.root, main) {
        return Err(refuse(
            "create a working copy",
            format!("the main branch `{main}` does not exist or has no commit"),
            &[
                "Make an initial commit on the main branch.",
                "Or set `main_branch` in factory/config.toml to the branch you use.",
            ],
            "`git rev-parse main` prints a commit.",
        ));
    }
    let cpb = p.checkpoint_branch(k);
    if !gitx::branch_exists(&p.root, &cpb) {
        gitx::must(&p.root, &["branch", &cpb, main])?;
    }
    let wt = p.work_dir().join(k);
    if !wt.join(".git").exists() {
        fs::create_dir_all(p.work_dir())?;
        gitx::must(&p.root, &["worktree", "add", &wt.to_string_lossy(), &cpb])?;
    }
    Ok(wt)
}

pub fn unit_start(p: &Project, unit: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    need_run(&st)?;
    budget_check(p, &st)?;
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("start {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints a working copy path.",
        ));
    };
    if !st.gate_passed("plan", None) {
        return Err(refuse(
            format!("start {unit}"),
            "the plan gate has not been approved",
            &[
                "Open the gate with `bf gate open plan` and show the plan page.",
                "The human approves in chat; then run `bf gate approve plan`.",
            ],
            "`bf gate status` shows the plan gate approved.",
        ));
    }
    gates::refuse_if_drift(p, &st, &format!("start {unit}"))?;
    let cur = st.unit_state(unit).to_string();
    // Starting a unit that is already in flight only makes sure its working copy exists.
    if !cur.is_empty() && !matches!(cur.as_str(), "running" | "review" | "fixing" | "reviewed") {
        return Err(refuse(
            format!("start {unit}"),
            format!("it is already `{cur}`"),
            &["Use `bf next` to see what is waiting."],
            "`bf status` shows the unit in its state.",
        ));
    }
    if cur.is_empty() {
        let receipts = evidence::load_receipts(p)?;
        if let Some(why) = plan::not_ready(p, &pl, &st, unit, &receipts) {
            return Err(refuse(
                format!("start {unit}"),
                why,
                &["Land the units it waits for first (`bf next` lists them)."],
                "`bf next` lists the unit as ready.",
            ));
        }
    }
    let k = s(u, "checkpoint").to_string();
    let cpb = p.checkpoint_branch(&k);
    ensure_checkpoint_worktree(p, &k)?;
    let ub = p.unit_branch(unit);
    let path = p.work_dir().join(unit);
    if !path.join(".git").exists() {
        if gitx::branch_exists(&p.root, &ub) {
            gitx::must(&p.root, &["worktree", "add", &path.to_string_lossy(), &ub])?;
        } else {
            gitx::must(
                &p.root,
                &["worktree", "add", "-b", &ub, &path.to_string_lossy(), &cpb],
            )?;
        }
    }
    crash_point("unit-start-after-git");
    if cur.is_empty() {
        let base_commit = gitx::must(&p.root, &["rev-parse", &ub])?;
        events::append(
            p,
            "unit.start",
            json!({"unit": unit, "branch": ub, "path": path.to_string_lossy(), "base": cpb, "base_commit": base_commit}),
        )?;
    }
    p.write_guard()?;
    Ok(
        json!({"unit": unit, "working_copy": path.to_string_lossy(), "branch": ub, "base": cpb, "resumed": !cur.is_empty()}),
    )
}

pub fn unit_land(p: &Project, unit: &str) -> R<Value> {
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("land {unit}"))?;
    refuse_if_main_moved(p, &st)?;
    let run = need_run(&st)?;
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("land {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints the merge.",
        ));
    };
    let cur = st.unit_state(unit).to_string();
    if cur == "landed" {
        return Ok(json!({"unit": unit, "landed": true, "already": true}));
    }
    if cur != "reviewed" {
        return Err(refuse(
            format!("land {unit}"),
            format!(
                "it is `{}`; only a reviewed unit lands",
                if cur.is_empty() { "not started" } else { &cur }
            ),
            &[
                "Build it, have it reviewed clean, and report both with `bf report`.",
                "`bf next` shows what is waiting.",
            ],
            "`bf status` shows the unit `reviewed`.",
        ));
    }
    let open: Vec<String> = p
        .list_records(schema::kind_by_name("finding"))?
        .iter()
        .filter(|f| s(f, "unit") == unit && is_open(f))
        .map(|f| s(f, "id").to_string())
        .collect();
    if !open.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            format!("open findings: {}", open.join(", ")),
            &["Fix them and have the reviewer report clean."],
            "No open findings remain.",
        ));
    }
    let k = s(u, "checkpoint").to_string();
    let cpb = p.checkpoint_branch(&k);
    let ub = p.unit_branch(unit);
    let wt = p.work_dir().join(unit);
    if !wt.join(".git").exists() {
        return Err(refuse(
            format!("land {unit}"),
            "its working copy is missing",
            &["Run `bf unit start` to recreate it."],
            "The working copy exists.",
        ));
    }
    let checks = pl.unit_checks(unit);
    if checks.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            "the unit has no check, so nothing could prove it",
            &["Add a check record with `unit = \"<id>\"` through the plan."],
            "`bf plan check` passes.",
        ));
    }
    let content = gitx::content_hash(&wt)?;
    let receipts = evidence::load_receipts(p)?;
    let mut missing: Vec<String> = Vec::new();
    for c in &checks {
        let cid = s(c, "id");
        if evidence::satisfied_at(&receipts, cid, &content).is_none() {
            let stale = receipts.iter().any(|r| r.check() == cid && r.passes());
            missing.push(format!(
                "{cid}: {} - run `bf evidence run {cid} --unit {unit} -- <its command>` inside {}",
                if stale {
                    "the receipts on file are for other content"
                } else {
                    "no passing sealed receipt"
                },
                wt.display()
            ));
        }
    }
    if !missing.is_empty() {
        let mut steps = missing;
        steps.push(format!("Run `bf unit land {unit}` again."));
        return Err(refuse_owned(
            format!("land {unit}"),
            format!(
                "receipts do not match the content now in the working copy (hash {})",
                &content[..12.min(content.len())]
            ),
            steps,
            "`bf evidence verify --unit` prints every check ok.",
        ));
    }
    if gitx::has_uncommitted(&wt) {
        gitx::must(&wt, &["add", "-A"])?;
        gitx::must(
            &wt,
            &["commit", "-q", "-m", &format!("{unit}: {}", s(u, "title"))],
        )?;
    }
    // Scope: every changed path must be inside the unit's files.
    let diff = gitx::must(&p.root, &["diff", "--name-only", &format!("{cpb}...{ub}")])?;
    let scope = str_list(u, "files");
    let outside: Vec<&str> = diff
        .lines()
        .filter(|f| !scope.iter().any(|g| util::path_matches(g, f)))
        .collect();
    if !outside.is_empty() {
        return Err(refuse(
            format!("land {unit}"),
            format!("the branch changes files outside the unit's scope: {}", outside.join(", ")),
            &["Revert those changes in the working copy (`git checkout -- <file>` or remove them), commit, and run the checks again.", "If the scope is wrong, the human changes the plan."],
            "`git diff --name-only` lists only files inside the scope.",
        ));
    }
    let cpwt = ensure_checkpoint_worktree(p, &k)?;
    if !gitx::is_ancestor(&p.root, &ub, &cpb) {
        let out = gitx::run(
            &cpwt,
            &[
                "merge",
                "--no-ff",
                "--no-edit",
                "-m",
                &format!("Land {unit}: {}", s(u, "title")),
                &ub,
            ],
            &[],
        )?;
        if !out.ok {
            let _ = gitx::run(&cpwt, &["merge", "--abort"], &[]);
            return Err(refuse(
                format!("land {unit}"),
                format!("the merge into {cpb} conflicts: {}", out.stderr.lines().next().unwrap_or("")),
                &["Rebase or merge the checkpoint branch into the unit's working copy and resolve the conflict there.", "Run the checks again, then `bf unit land` again."],
                "`bf unit land` prints the merge.",
            ));
        }
    }
    crash_point("land-after-merge");
    let tip = gitx::head_short(&cpwt);
    events::append(
        p,
        "unit.land",
        json!({"unit": unit, "checkpoint": k, "hash": content, "merge": tip, "run": run}),
    )?;
    p.write_guard()?;
    Ok(
        json!({"unit": unit, "landed": true, "checkpoint": k, "branch": cpb, "merge": tip, "content_hash": content}),
    )
}

/// Where the main branch stands against what the engine recorded in the log head.
pub struct MainPos {
    /// The branch is at a commit the engine did not put there and the human has not accepted.
    pub moved: bool,
    /// Why, in words: the work it holds that should not be there.
    pub detail: String,
    pub tip: String,
    /// A quiet line when the human's own commits moved the branch; it blocks nothing.
    pub note: String,
}

/// Compare the main branch with the commit the engine last put there (or the baseline set at `bf init`
/// or accepted by the human). A move by the human that only touches `factory/` and `.gitignore` is
/// taken as theirs (they commit their records). Anything else is a move to account for. Independently,
/// work of a unit that never landed, or landed in a test run, on the main branch is always one.
pub fn main_position(p: &Project, st: &State) -> R<MainPos> {
    let main = &p.config.main_branch;
    let ok = |tip: &str| MainPos {
        moved: false,
        detail: String::new(),
        tip: tip.to_string(),
        note: String::new(),
    };
    if !gitx::branch_exists(&p.root, main) {
        // A branch that was there and is gone is a change to account for.
        if events::read_head(p).and_then(|h| h.main_commit).is_some() {
            return Ok(MainPos {
                moved: true,
                detail: format!("the main branch `{main}` does not exist any more"),
                tip: String::new(),
                note: String::new(),
            });
        }
        return Ok(ok(""));
    }
    let tip = gitx::must(&p.root, &["rev-parse", main])?;
    let recorded = events::read_head(p).and_then(|h| h.main_commit);
    if recorded.as_deref() == Some(tip.as_str()) {
        return Ok(ok(&tip));
    }
    // Work that should never be on main, by whatever way it got there.
    for (id, u) in &st.units {
        let test_landed =
            u.state == "landed" && st.runs.iter().any(|r| r.id == u.landed_run && r.test_mode);
        let unlanded = u.state != "landed";
        if !(test_landed || unlanded) || u.base_commit.is_empty() {
            continue;
        }
        let ub = p.unit_branch(id);
        if !gitx::branch_exists(&p.root, &ub) {
            continue;
        }
        let work = gitx::must(
            &p.root,
            &["rev-list", "--count", &format!("{}..{}", u.base_commit, ub)],
        )
        .unwrap_or_default();
        if work.trim() == "0" || work.is_empty() {
            continue;
        }
        if gitx::is_ancestor(&p.root, &ub, main) {
            let why = if test_landed {
                "landed in a test run"
            } else {
                "never landed"
            };
            return Ok(MainPos {
                moved: true,
                detail: format!(
                    "the main branch (now at {}) contains the commits of {id}, which {why}",
                    short(&tip)
                ),
                tip,
                note: String::new(),
            });
        }
    }
    // Anything else is the human's own commits (the project's scaffolding, their records, their code):
    // noted in one quiet line, never a reason to stop.
    let Some(rec) = recorded else {
        return Ok(ok(&tip));
    };
    let count =
        gitx::must(&p.root, &["rev-list", "--count", &format!("{rec}..{tip}")]).unwrap_or_default();
    let mut m = ok(&tip);
    m.note = format!(
        "the main branch has moved on from {} to {} by commits the engine did not make ({} new); they hold no unlanded or test-run work",
        short(&rec),
        short(&tip),
        if count.trim().is_empty() { "some".to_string() } else { count.trim().to_string() }
    );
    Ok(m)
}

fn short(c: &str) -> &str {
    &c[..c.len().min(10)]
}

/// Refuse while the main branch is somewhere the engine does not account for.
pub fn refuse_if_main_moved(p: &Project, st: &State) -> R<()> {
    let m = main_position(p, st)?;
    if !m.moved {
        return Ok(());
    }
    Err(refuse(
        "continue while the main branch is where the engine did not put it",
        m.detail,
        &[
            "Do not push anything. Tell the human what happened: the main branch holds work of a unit that never landed, or of a test run.",
            "They move the branch back with git to the last good commit, or have the work reviewed and landed through the engine.",
        ],
        "`bf status` shows `main: ok`.",
    ))
}

pub fn checkpoint_close(p: &Project, k: &str, merge: bool) -> R<Value> {
    let st = events::require_chain(p)?;
    gates::refuse_if_drift(p, &st, &format!("close {k}"))?;
    refuse_if_main_moved(p, &st)?;
    let pl = plan::load(p)?;
    let Some(kr) = pl.checkpoint(k) else {
        return Err(refuse(
            format!("close {k}"),
            "no such checkpoint",
            &["Use a checkpoint ID from `bf record list --kind checkpoint`."],
            "The command prints the closure.",
        ));
    };
    let after = str_list(kr, "after");
    let unlanded: Vec<&String> = after
        .iter()
        .filter(|u| st.unit_state(u) != "landed")
        .collect();
    if !unlanded.is_empty() {
        return Err(refuse(
            format!("close {k}"),
            format!(
                "units not landed: {}",
                unlanded
                    .iter()
                    .map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            &["Land them with `bf unit land`; `bf next` shows where each one is."],
            "`bf status` shows them landed.",
        ));
    }
    let cpwt = p.work_dir().join(k);
    if !cpwt.join(".git").exists() {
        return Err(refuse(
            format!("close {k}"),
            "the checkpoint's working copy does not exist",
            &["Land a unit of this checkpoint first."],
            "`bf checkpoint close` succeeds.",
        ));
    }
    if gitx::has_uncommitted(&cpwt) {
        return Err(refuse(
            format!("close {k}"),
            "the checkpoint's working copy has uncommitted changes, so no receipt describes the branch",
            &[&format!("Inspect `git -C {} status`.", cpwt.display()), "Remove the changes; checkpoint content only changes by landing units."],
            "The working copy is clean.",
        ));
    }
    let content = gitx::content_hash(&cpwt)?;
    let receipts = evidence::load_receipts(p)?;
    let waived = gates::waived_ids(&st);
    let mut gaps: Vec<String> = Vec::new();
    let mut via_amend: Vec<Value> = Vec::new();
    for id in str_list(kr, "proves") {
        if let Some(a) = waived.get(&id) {
            via_amend.push(json!({"id": id, "amendment": a}));
            continue;
        }
        let checks: Vec<&Value> = pl
            .checks
            .iter()
            .filter(|c| {
                str_list(kr, "checks").iter().any(|x| x == s(c, "id"))
                    && str_list(c, "proves").contains(&id)
            })
            .collect();
        // A command or agent check is settled by its receipt at this content; a human check only by the
        // human's own gate. An attached file proves nothing about a human check.
        let ok = checks.iter().any(|c| {
            if s(c, "how") == "human" {
                st.gate_passed("checkpoint-step", Some(k))
            } else {
                evidence::satisfied_at(&receipts, s(c, "id"), &content).is_some()
            }
        });
        let marked: Vec<String> = checks
            .iter()
            .filter_map(|c| {
                evidence::standing_outcome(&receipts, s(c, "id"), &content).map(|r| {
                    format!(
                        "{} is marked {} ({})",
                        s(c, "id"),
                        r.outcome().unwrap_or(""),
                        s(&r.v, "reason")
                    )
                })
            })
            .collect();
        if !ok && !marked.is_empty() {
            gaps.push(format!(
                "{id}: {}; the human decides whether to fix the cause and run it again, to accept the gap by amendment, or to change the plan",
                marked.join("; ")
            ));
        } else if !ok {
            let names: Vec<&str> = checks.iter().map(|c| s(c, "id")).collect();
            gaps.push(format!(
                "{id}: no sealed passing receipt at this content (hash {}) from {}; run `bf evidence run <check> -- <its command>` inside {}, or ask the human to approve an amendment",
                &content[..12.min(content.len())],
                if names.is_empty() { "any check".to_string() } else { names.join(" or ") },
                cpwt.display()
            ));
        }
    }
    if !s(kr, "human_step").is_empty() && !st.gate_passed("checkpoint-step", Some(k)) {
        gaps.push(format!("the human step `{}` has no approved checkpoint-step gate: run `bf gate open checkpoint-step --ref {k}`", s(kr, "human_step")));
    }
    if !gaps.is_empty() {
        let mut steps = gaps.clone();
        steps.push(format!("Run `bf checkpoint close {k}` again."));
        return Err(refuse_owned(
            format!("close {k}"),
            format!("{} ID(s) have no proof", gaps.len()),
            steps,
            "`bf checkpoint close` prints the closure.",
        ));
    }
    let was_closed = st.closed.contains_key(k);
    let mut merged = false;
    if merge {
        // Main moves only with a human act after the closure: an approved checkpoint-step gate opened
        // after the close event. (D-16: main moves when a human merges.)
        let closed_seq = st.closed.get(k).map(|c| c.seq);
        let Some(closed_seq) = closed_seq else {
            return Err(refuse(
                format!("merge {k} onto {}", p.config.main_branch),
                "the checkpoint is not closed yet; merging follows the closure and a human act",
                &[&format!("Run `bf checkpoint close {k}` first.")],
                "`bf status` lists the checkpoint as closed.",
            ));
        };
        // A run with simulated approvals can never reach the main branch.
        let tainted_unit = after.iter().any(|u| {
            let lr = st
                .units
                .get(u)
                .map(|x| x.landed_run.clone())
                .unwrap_or_default();
            st.runs.iter().any(|r| r.id == lr && r.test_mode)
        });
        let tainted_gate = ["idea", "requirements", "ui", "architecture", "plan"]
            .iter()
            .any(|g| {
                st.latest_gate(g, None)
                    .map(|x| x.simulated)
                    .unwrap_or(false)
            })
            || st
                .latest_gate("checkpoint-step", Some(k))
                .map(|x| x.simulated)
                .unwrap_or(false);
        if tainted_unit || tainted_gate {
            return Err(refuse(
                format!("merge {k} onto {}", p.config.main_branch),
                "the run contains a simulated approval or test-mode work; such a run can never land on the main branch",
                &["Leave the checkpoint branch as it is for inspection.", "Redo the work in a real run: the human approves each gate in their own words."],
                "`bf checkpoint close` without --merge still records the closure.",
            ));
        }
        let approved_after = st.gates.iter().any(|g| {
            g.kind == "checkpoint-step"
                && g.refr.as_deref() == Some(k)
                && g.status == "approved"
                && !g.simulated
                && g.open_seq > closed_seq
        });
        if !approved_after {
            return Err(refuse(
                format!("merge {k} onto {}", p.config.main_branch),
                "merging onto the main branch needs the human's go-ahead after the checkpoint closed",
                &[
                    &format!("Open the gate: `bf gate open checkpoint-step --ref {k}` and show the human the checkpoint page."),
                    "When the human says in chat to merge it, `bf gate approve <gate> --quote \"<their words>\"`.",
                    &format!("Then run `bf checkpoint close {k} --merge` again."),
                ],
                "The command prints the merge.",
            ));
        }
        let main = p.config.main_branch.clone();
        if gitx::current_branch(&p.root).as_deref() != Some(main.as_str()) {
            return Err(refuse(
                format!("merge {k} onto {main}"),
                format!("the main checkout is not on `{main}`"),
                &[
                    &format!("Switch the main checkout to `{main}` (the human's step)."),
                    "Run the command again.",
                ],
                "The command prints the merge commit.",
            ));
        }
        let cpb = p.checkpoint_branch(k);
        if !gitx::is_ancestor(&p.root, &cpb, &main) {
            let out = gitx::run(
                &p.root,
                &[
                    "merge",
                    "--no-ff",
                    "--no-edit",
                    "-m",
                    &format!("Close {k}: {}", s(kr, "title")),
                    &cpb,
                ],
                &[],
            )?;
            if !out.ok {
                let _ = gitx::run(&p.root, &["merge", "--abort"], &[]);
                return Err(refuse(
                    format!("merge {k} onto {main}"),
                    format!("git refused the merge: {}", out.stderr.lines().next().unwrap_or("")),
                    &["Commit or stash local changes that touch the same files, or resolve the conflict by hand (the human's step).", "Run the command again."],
                    "The command prints the merge commit.",
                ));
            }
        }
        merged = true;
        // Remember the commit the engine put on the main branch.
        if let Ok(tip) = gitx::must(&p.root, &["rev-parse", &main]) {
            let mut h = events::read_head(p).unwrap_or_default();
            h.main_commit = Some(tip);
            events::write_head(p, &h)?;
        }
    }
    if !was_closed || merged {
        events::append(
            p,
            "checkpoint.close",
            json!({"checkpoint": k, "merged": merged, "hash": content}),
        )?;
        p.write_guard()?;
    }
    Ok(
        json!({"checkpoint": k, "closed": true, "already": was_closed && !merged, "merged": merged, "content_hash": content, "amendments": via_amend}),
    )
}

pub fn stuck(p: &Project, dry: bool) -> R<Value> {
    let st = events::require_chain(p)?;
    let now = util::now_secs();
    let limit = p.policy.limits.idle_minutes * 60;
    let mut parked = Vec::new();
    let mut listed = Vec::new();
    for (id, u) in &st.units {
        if u.state == "parked" {
            listed.push(json!({"unit": id, "reason": u.parked_reason}));
        } else if matches!(u.state.as_str(), "running" | "review" | "fixing") {
            let idle = now.saturating_sub(util::parse_rfc3339(&u.last_at));
            if idle > limit {
                parked
                    .push(json!({"unit": id, "reason": format!("idle for {} minutes", idle / 60)}));
            }
        }
    }
    if !dry {
        for x in &parked {
            events::append(
                p,
                "unit.park",
                json!({"unit": x["unit"], "reason": x["reason"]}),
            )?;
        }
        if !parked.is_empty() {
            p.write_guard()?;
        }
    }
    Ok(json!({"parked_now": parked, "already_parked": listed, "dry_run": dry}))
}

pub fn cost_status(p: &Project) -> R<Value> {
    let st = events::replay(p)?;
    let cap = p.policy.budget.cap;
    Ok(json!({
        "spent": st.cost, "cap": cap, "unit": p.policy.budget.unit,
        "left": if cap > 0.0 { json!(cap - st.cost) } else { Value::Null },
        "tiers": {"builder": p.policy.tiers.builder, "reviewer": p.policy.tiers.reviewer, "verifier": p.policy.tiers.verifier},
        "per_unit": st.units.iter().map(|(k, v)| json!({"unit": k, "cost": v.cost, "rounds": v.rounds})).collect::<Vec<_>>(),
    }))
}
