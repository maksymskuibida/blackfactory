//! `bf next`: the directives, the tier of each role and the budget check.

use super::*;

pub(crate) fn tier_for(p: &Project, unit: &Value, role: &str) -> String {
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

/// The tier, the effort and any model named for a kind of work, as the policy in force has them.
pub fn work_entry(p: &Project, kind: &str) -> Value {
    let mut e = json!({"tier": p.policy.tiers.get(kind), "effort": p.policy.efforts.get(kind)});
    if let Some(m) = p.policy.models.get(kind).filter(|m| !m.is_empty()) {
        e["model"] = json!(m);
    }
    e
}

/// Every kind of work with its entry: what `bf policy` prints beside the rest of the policy.
pub fn work_policy(p: &Project) -> Value {
    let mut m = serde_json::Map::new();
    for k in crate::store::WORK_KINDS {
        m.insert(k.to_string(), work_entry(p, k));
    }
    Value::Object(m)
}

/// The kind of work a directive asks for.
fn directive_kind(d: &Value) -> &'static str {
    match d["work"].as_str() {
        Some("designer") => return "designer",
        Some("amender") => return "amender",
        _ => {}
    }
    match d["role"].as_str().unwrap_or("") {
        "builder" => "builder",
        "reviewer" if !d["checkpoint"].is_null() => "checkpoint_reviewer",
        "reviewer" => "reviewer",
        "verifier" => "verifier",
        "security-auditor" => "reviewer",
        _ => "conductor",
    }
}

/// Put the tier and the effort of its kind of work (and any model) on a directive. A unit's own tier
/// on a builder's directive stays.
fn add_work(p: &Project, d: &mut Value) {
    let e = work_entry(p, directive_kind(d));
    if d["tier"].is_null() || d["role"] != "builder" {
        d["tier"] = e["tier"].clone();
    }
    d["effort"] = e["effort"].clone();
    if !e["model"].is_null() {
        d["model"] = e["model"].clone();
    }
}

/// Whether the budget cap is reached: `bf next` refuses, and the checkpoints' findings are held.
pub fn budget_reached(p: &Project, st: &State) -> bool {
    let cap = p.policy.budget.cap;
    cap > 0.0 && st.cost >= cap
}

pub(crate) fn budget_check(p: &Project, st: &State) -> R<()> {
    let cap = p.policy.budget.cap;
    if budget_reached(p, st) {
        return Err(refuse(
            "start new work",
            format!("the budget cap is reached: spent {:.2} of {:.2} {}", st.cost, cap, p.policy.budget.unit),
            &["Ask the human whether to raise `budget.cap` in factory/policy.toml (only they edit it).", "Run `bf cost status` to see where it went."],
            "`bf next` returns directives again.",
        ));
    }
    Ok(())
}

/// The kind of work the records of a design step are: a designer's, or an amender's while an amendment
/// is open (R-46).
fn add_records_work(st: &State) -> &'static str {
    if st
        .amends
        .iter()
        .any(|a| !crate::amend::is_repair(a) && matches!(a.status.as_str(), "open" | "narrowed"))
    {
        "amender"
    } else {
        "designer"
    }
}

/// The open questions of one design stage, by ID.
fn open_stage_questions(p: &Project, stage: &str) -> Vec<String> {
    let mut ids: Vec<String> = p
        .list_records(schema::kind_by_name("question"))
        .unwrap_or_default()
        .iter()
        .filter(|q| s(q, "stage") == stage && q.get("answer").is_none())
        .map(|q| s(q, "id").to_string())
        .collect();
    ids.sort();
    ids
}

/// The directives of a design stage, while the plan gate is not approved. The first design gate that is
/// neither approved nor skipped is the one to offer: decide it if it is open; answer the stage's open
/// questions, which come before its gate; add the records it lacks; or open it (R-46).
pub(crate) fn design_directives(p: &Project, st: &State) -> Vec<Value> {
    let kind = gates::first_unpassed_design_gate(st);
    if let Some(g) = st.latest_gate(kind, None).filter(|g| g.status == "open") {
        return vec![
            json!({"role": "human", "action": "decide-gate", "gate": g.id, "gate_kind": kind,
            "note": crate::strings::t("cli.design.next.decide_gate", &[("kind", kind)])}),
        ];
    }
    let open_q = open_stage_questions(p, kind);
    if !open_q.is_empty() {
        let ids = open_q.join(", ");
        return vec![
            json!({"role": "human", "action": "answer-question", "stage": kind,
            "question": open_q[0], "questions": open_q,
            "note": crate::strings::t("cli.design.next.answer_questions", &[("stage", kind), ("ids", &ids)])}),
        ];
    }
    if let Some(why) = gates::open_blocker(p, st, kind) {
        let mut d = json!({"role": "conductor", "action": "add-records", "work": add_records_work(st), "gate_kind": kind, "missing": why,
            "note": crate::strings::t("cli.design.next.add_records", &[("kind", kind), ("why", &why)])});
        if kind == "ui" {
            // A product with no interface needs no screen: the human can skip the gate.
            d["skip_command"] = json!("bf gate skip ui --quote \"<the human's words>\"");
            d["note"] = json!(crate::strings::t(
                "cli.design.next.add_records_ui",
                &[("why", &why)]
            ));
        }
        return vec![d];
    }
    vec![
        json!({"role": "human", "action": "open-gate", "gate_kind": kind, "command": format!("bf gate open {kind}")}),
    ]
}

/// `bf next` before a run, while the plan gate is not approved: the design directives, no run.
fn design_raw(p: &Project, st: &State) -> Value {
    json!({"run": Value::Null, "directives": design_directives(p, st), "waiting": Vec::<String>::new()})
}

/// The directives while the latest plan gate of the run is not approved (`open_gate` is its ID when it is
/// open). With the gates in order, the one to offer is the first that is not approved or skipped: decide it
/// if it is open, otherwise open it, or say which record is missing for it to open.
fn run_hold_directives(p: &Project, st: &State, open_gate: Option<&str>) -> Vec<Value> {
    let mut directives: Vec<Value> = Vec::new();
    if p.policy.gates.in_order {
        return design_directives(p, st);
    }
    if let Some(g) = open_gate {
        directives.push(json!({"role": "human", "action": "decide-gate", "gate": g, "note": crate::strings::t("cli.design.next.decide_gate", &[("kind", "plan")])}));
        return directives;
    }
    // The run's two plan questions come first: the plan gate does not open until both are answered.
    if let Some(r) = st.active_run().filter(|_| {
        plan::load(p)
            .map(|pl| !pl.units.is_empty())
            .unwrap_or(false)
    }) {
        if r.wave.is_none() {
            directives.push(json!({"role": "human", "action": "answer-wave-checks", "command": "bf checks wave question",
                "note": "Ask the human which wave checks this run uses (the project default, an override or none), then record the answer with `bf checks wave propose` and `bf checks wave set --quote`."}));
        }
        if r.unit_checks.is_none() {
            directives.push(json!({"role": "human", "action": "answer-unit-checks", "command": "bf checks unit question",
                "note": "Ask the human which checks every unit of this run must pass (the project's list, an override or none), then record the answer with `bf checks unit propose` and `bf checks unit set --quote`."}));
        }
    }
    directives.push(json!({"role": "human", "action": "open-gate", "gate_kind": "plan", "command": "bf gate open plan"}));
    directives
}

pub(crate) fn next_raw(p: &Project) -> R<Value> {
    // The holds are logged before the directives are worked out, so the one predicate and the log agree.
    observe_holds(p)?;
    let st = events::require_chain(p)?;
    let run = need_run(&st)?;
    budget_check(p, &st)?;
    let pl = plan::load(p)?;
    let receipts = evidence::load_receipts(p)?;
    let mut directives: Vec<Value> = Vec::new();
    let mut waiting: Vec<String> = Vec::new();

    // The latest plan gate not approved: `bf next` returns before its checkpoints, and the predicate holds.
    match events::run_hold(&st) {
        Some(events::RunHold::Open { gate }) => {
            return Ok(
                json!({"run": run, "directives": run_hold_directives(p, &st, Some(&gate)), "waiting": waiting}),
            );
        }
        Some(events::RunHold::NotOpen { .. }) => {
            return Ok(
                json!({"run": run, "directives": run_hold_directives(p, &st, None), "waiting": waiting}),
            );
        }
        None => {
            // A run that has not answered a plan question is asked it now; no unit starts until then.
            if !pl.units.is_empty() {
                for (action, command, _) in unanswered_questions(&st) {
                    let (what, set) = if action == "answer-wave-checks" {
                        ("which wave checks this run uses (the project default, an override or none)", "bf checks wave")
                    } else {
                        ("which checks every unit of this run must pass (the project's list, an override or none)", "bf checks unit")
                    };
                    directives.push(json!({"role": "human", "action": action, "command": command,
                        "note": format!("Ask the human {what}, then record the answer with `{set} propose` and `{set} set --quote`. No unit starts until both questions are answered.")}));
                }
            }
        }
    }
    // An open sync of the main branch into a checkpoint: nothing else starts until it is settled.
    if let Some(so) = &st.sync_open {
        let k = &so.checkpoint;
        let wc = p.work_dir().join(k).to_string_lossy().to_string();
        let lim = json!({"max_rounds": p.policy.limits.max_rounds, "round": so.rounds, "idle_minutes": p.policy.limits.idle_minutes,
               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }});
        let brief = |role: &str| {
            p.briefs_dir()
                .join(format!("{k}-{role}.md"))
                .to_string_lossy()
                .to_string()
        };
        let mut d = match so.phase.as_str() {
            "resolve" => json!({"role": "builder", "action": "resolve-sync", "checkpoint": k,
                "tier": p.policy.tiers.builder, "working_copy": wc, "brief": brief("builder"), "limits": lim,
                "files": sync_files(so),
                "may_change": sync_allowed(so),
                "failing_checks": so.failed.iter().filter_map(|f| f["check"].as_str()).collect::<Vec<_>>(),
                "commands": [format!("bf brief {k} --role builder")],
                "abort_command": format!("bf checkpoint sync {k} --abort")}),
            "review" => json!({"role": "reviewer", "action": "review-sync", "checkpoint": k,
                "tier": p.policy.tiers.reviewer, "working_copy": wc, "brief": brief("reviewer"), "limits": lim,
                "files": sync_files(so),
                "commands": [format!("bf brief {k} --role reviewer")],
                "abort_command": format!("bf checkpoint sync {k} --abort")}),
            _ => json!({"role": "human", "action": "decide-sync", "checkpoint": k,
                "note": format!("The sync of {k} is parked ({}). The human says whether to try again (`bf checkpoint sync {k} --release --quote \"<their words>\"`) or to go back (`bf checkpoint sync {k} --abort`).", so.reason),
                "release_command": format!("bf checkpoint sync {k} --release --quote \"<the human's words>\""),
                "abort_command": format!("bf checkpoint sync {k} --abort")}),
        };
        d["note_sync"] = json!(format!(
            "A sync of the main branch into {k} is open; no unit starts until it is settled."
        ));
        directives.push(d);
        return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
    }
    // An open fix of a checkpoint: it is settled before anything else starts.
    if let Some(f) = &st.ck_fix {
        let mut d = crate::ckfix::directive(p, &st, f);
        if f.source == "wave" {
            // A fix of a wave check does not hold the next wave back: its units are built without
            // waiting, and only their landing waits for the wave check to pass.
            d["note_fix"] = json!(format!(
                "A fix of the wave check of wave {} of {} is open. The units of the next wave are offered below and are built without waiting; none of them lands until the wave check has passed.",
                f.wave, f.checkpoint
            ));
            directives.push(d);
        } else {
            d["note_fix"] = json!(format!(
                "A fix of {} is open; nothing else starts until it is settled.",
                f.checkpoint
            ));
            directives.push(d);
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
        }
    }
    let limits = |us_rounds: u32| {
        json!({"max_rounds": p.policy.limits.max_rounds, "round": us_rounds, "idle_minutes": p.policy.limits.idle_minutes,
               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }})
    };
    // Questions a worker put to the human hold that unit (or checkpoint) only; the others go on.
    let open_unit_questions: Vec<Value> = p
        .list_records(schema::kind_by_name("question"))?
        .into_iter()
        .filter(|q| !s(q, "unit").is_empty() && q.get("answer").is_none())
        .collect();
    let held = |id: &str| open_unit_questions.iter().any(|q| s(q, "unit") == id);
    for q in &open_unit_questions {
        directives.push(json!({"role": "human", "action": "answer-question", "question": s(q, "id"), "unit": s(q, "unit"),
            "title": s(q, "title"),
            "note": format!("{} asks: {}. Show the human the question; when they have answered, `bf answer {} --file <answer>`. Only {} waits.", s(q, "unit"), s(q, "title"), s(q, "id"), s(q, "unit"))}));
    }
    judge_flow(p, &st, &mut directives, &mut waiting);
    for (id, state, why) in unit_states(p, &pl, &st, &receipts) {
        let u = pl.unit(&id).unwrap();
        if held(&id) && matches!(state.as_str(), "ready" | "running" | "fixing" | "review") {
            waiting.push(format!(
                "{id}: waiting for the human's answer to its question"
            ));
            continue;
        }
        if let Some(line) = super::pause::waiting_line(&st, &id) {
            waiting.push(line);
            continue;
        }
        let wc = p.work_dir().join(&id).to_string_lossy().to_string();
        let rounds = st.units.get(&id).map(|x| x.rounds).unwrap_or(0);
        let brief = |role: &str| {
            p.briefs_dir()
                .join(format!("{id}-{role}.md"))
                .to_string_lossy()
                .to_string()
        };
        match state.as_str() {
            "ready" if amend_hold(&st, &id).is_some() => {
                waiting.push(crate::strings::t(
                    "next.unit_held.line",
                    &[("unit", &id), ("amend", &amend_hold(&st, &id).unwrap_or_default())],
                ));
            }
            "ready" if !unanswered_questions(&st).is_empty() => {
                waiting.push(format!("{id}: waiting for the human's answers to the run's plan questions"))
            }
            "ready" => directives.push(json!({"role": "builder", "action": "start-unit", "unit": id, "tier": tier_for(p, u, "builder"),
                "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                "commands": [format!("bf unit start {id}"), format!("bf brief {id} --role builder")]})),
            "running" | "fixing" => {
                let mut d = json!({"role": "builder", "action": if state == "fixing" { "fix" } else { "build" }, "unit": id,
                    "tier": tier_for(p, u, "builder"), "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                    "commands": [format!("bf brief {id} --role builder")]});
                if let Some(a) = st.units.get(&id).map(|x| x.reopened_by.clone()).filter(|a| !a.is_empty()) {
                    d["amendment"] = json!(a);
                }
                super::unblock::decorate(p, &st, &id, &mut d);
                super::unit_sync::decorate(p, &id, &mut d);
                directives.push(d);
            }
            "review" if super::expert::asked(p, &st, &id)?.is_some() => {
                let ask = super::expert::asked(p, &st, &id)?.unwrap_or_default();
                directives.push(super::expert::directive(&id, &ask, &wc, &brief(super::expert::ROLE),
                    limits(rounds), &p.policy.tiers.reviewer));
            }
            "review" => directives.push(json!({"role": "reviewer", "action": "review", "unit": id, "tier": tier_for(p, u, "reviewer"),
                "working_copy": wc, "brief": brief("reviewer"), "limits": limits(rounds),
                "commands": [format!("bf brief {id} --role reviewer")]})),
            "reviewed" => {
                // No land is offered while no turn of its checkpoint can open: a sync, a fix or a review
                // round of the checkpoint is open, or another process holds the turn.
                let k = s(u, "checkpoint");
                if let Some(why) = super::land::open_reason(p, &st, k) {
                    waiting.push(format!(
                        "{id}: {}",
                        super::land::wait_line(p, &st, &pl, &id, k, why)
                    ));
                } else if crate::lock::turn_open(p, k) {
                    waiting.push(format!(
                        "{id}: {}",
                        crate::strings::t("land_merge.waits.turn", &[("unit", &id), ("checkpoint", k)])
                    ));
                } else {
                    directives.push(json!({"role": "engine", "action": "land", "unit": id, "command": format!("bf unit land {id}")}))
                }
            }
            "blocked" | "parked" => {
                use crate::scope::NextEntry;
                let entry = if state == "blocked" {
                    super::unblock::next_entry(p, &pl, &st, &id)
                } else {
                    None
                };
                match entry {
                    Some(NextEntry::Waiting(line)) => {
                        waiting.push(line);
                        continue;
                    }
                    Some(NextEntry::Directive(d)) => {
                        directives.push(d);
                        continue;
                    }
                    None => {}
                }
                let mut note = crate::strings::t("next.unit_parked.note", &[("id", &id), ("state", &state)]);
                if let Some(r) = st.scope.get(&id).and_then(|sc| sc.refused.last()).filter(|_| state == "blocked") {
                    let files: Vec<String> = r["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|f| {
                            format!(
                                "{}: {}",
                                s(f, "path"),
                                if s(f, "rule").is_empty() { s(f, "why") } else { s(f, "rule") }
                            )
                        })
                        .collect();
                    // The reason of the refusal comes from a key of its own, never the raw word (FU-36).
                    let word = s(r, "refusal");
                    let key = format!("check_accept.refusal.{word}");
                    let reason = match crate::strings::raw(&key) {
                        Some(_) => crate::strings::t(&key, &[]),
                        None => crate::strings::t("check_accept.refusal.other", &[]),
                    };
                    note.push_str(&crate::strings::t(
                        "next.unit_parked.refused",
                        &[("refusal", &reason), ("files", &files.join("; "))],
                    ));
                }
                note.push_str(&super::unblock::human_note(p, &pl, &st, &id));
                directives.push(json!({"role": "human", "action": "decide-unit", "unit": id, "state": state, "note": note}))
            }
            "planned" => waiting.push(format!("{id}: {}", why.unwrap_or_default())),
            _ => {}
        }
    }
    // The step after a wave has landed: each wave check runs on a copy of the content, beside the units of
    // the next wave. The engine runs it; it needs no worker's token.
    for (k, w) in crate::wavecheck::due(p, &pl, &st) {
        directives.push(crate::wavecheck::directive(&k, w, &pl));
    }
    for (k, w) in crate::wavecheck::running(p, &pl, &st) {
        waiting.push(format!("{k}: the wave check of wave {w} is running"));
    }
    // Checkpoints whose units have all landed.
    for k in &pl.checkpoints {
        let kid = s(k, "id");
        let after = str_list(k, "after");
        if st.closed.contains_key(kid) {
            continue;
        }
        // The last wave's check counts for the checkpoint's own checks when it ran on the content the
        // checkpoint closes on, so the checkpoint waits for it.
        let landed = !after.is_empty() && after.iter().all(|u| st.unit_state(u) == "landed");
        // Every withholding of the checkpoint's findings is a case of the one predicate.
        let hold = if landed {
            events::hold(&st, kid, u64::MAX)
        } else {
            None
        };
        match hold {
            Some(Hold::Wave { reason, .. }) => {
                waiting.push(format!("{kid}: {reason}"));
                if st.ck_fix.is_none()
                    && st
                        .wave_runs
                        .iter()
                        .any(|((c, _), r)| c == kid && r.state == "failed")
                {
                    directives.push(json!({"role": "human", "action": "decide-wave-check", "checkpoint": kid,
                        "note": format!("A wave check of {kid} failed and its fix was turned down. The human decides: change the plan by amendment, or have the fix tried again.")}));
                }
                continue;
            }
            Some(Hold::Question { .. }) => {
                waiting.push(format!(
                    "{kid}: waiting for the human's answer to its question"
                ));
                continue;
            }
            Some(Hold::Marked { checks, .. }) => {
                let content = p.content_hash(&p.work_dir().join(kid)).unwrap_or_default();
                let marked: Vec<Value> = checks
                    .iter()
                    .map(|c| match evidence::standing_outcome(&receipts, c, &content) {
                        Some(r) => json!({"check": c, "outcome": r.outcome().unwrap_or(""), "reason": s(&r.v, "reason")}),
                        None => json!({"check": c, "outcome": "", "reason": ""}),
                    })
                    .collect();
                directives.push(json!({"role": "human", "action": "decide-check", "checkpoint": kid, "checks": marked,
                    "accept_command": format!("bf checkpoint check {kid} --accept <X-ids> --quote \"<the human's words>\""),
                    "note": crate::strings::t("check_accept.decide.note", &[("k", kid)])}));
                // D-110: a hold made only by `unreachable` marks is offered to the verifier again as an
                // ordinary step; no agent judges the load.
                if marked.iter().all(|m| m["outcome"] == "unreachable") {
                    let (load, cores) = evidence::machine_load();
                    directives.push(json!({"role": "verifier", "action": "run-checkpoint-checks", "checkpoint": kid, "checks": checks,
                        "tier": p.policy.tiers.verifier, "working_copy": p.work_dir().join(kid).to_string_lossy(),
                        "brief": p.briefs_dir().join(format!("{kid}-verifier.md")).to_string_lossy(),
                        "load": load, "cores": cores,
                        "limits": {"max_rounds": p.policy.limits.max_rounds, "idle_minutes": p.policy.limits.idle_minutes,
                                   "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }},
                        "commands": [format!("bf brief {kid} --role verifier")],
                        "note": crate::strings::t("check_accept.run_again.note", &[("k", kid)])}));
                }
                continue;
            }
            // A sync or a fix is at work: their early returns above end the directives, except for the
            // fix of a wave check, which lets the next wave's units go on and holds the checkpoint here.
            Some(h @ (Hold::Sync { .. } | Hold::Fix { .. })) => {
                let wave_held = crate::wavecheck::close_blockers(&pl, &st, kid);
                if wave_held.is_empty() {
                    waiting.push(format!("{kid}: {}", h.because()));
                } else {
                    waiting.push(format!("{kid}: {}", wave_held.join("; ")));
                }
                continue;
            }
            Some(Hold::Run { .. } | Hold::Budget { .. }) => {
                // Both end `bf next` before this loop; a logged hold is shown for what it is.
                if let Some(h) = hold {
                    waiting.push(format!("{kid}: {}", h.because()));
                }
                continue;
            }
            None => {}
        }
        if landed {
            let wc = p.work_dir().join(kid).to_string_lossy().to_string();
            let content = p.content_hash(&p.work_dir().join(kid)).unwrap_or_default();
            // A check the human switched off is not asked for.
            let missing: Vec<String> = str_list(k, "checks")
                .into_iter()
                .filter(|c| {
                    !pl.checks
                        .iter()
                        .any(|r| s(r, "id") == c && crate::enforce::is_off(&st, r))
                })
                .filter(|c| evidence::satisfied_at(&receipts, c, &content).is_none())
                .filter(|c| evidence::accepted_at(&st, kid, c, &content).is_none())
                // The checkpoint review is settled by its reviewer, never by the verifier.
                .filter(|c| {
                    !pl.checks.iter().any(|r| {
                        s(r, "id") == c && s(r, "named") == crate::checks::CHECKPOINT_REVIEW
                    })
                })
                .collect();
            let review_needed = crate::ckreview::needed(p, &st, kid, &content)?;
            let human_needed =
                crate::enforce::stops(k) && !st.gate_passed("checkpoint-step", Some(kid));
            if !missing.is_empty() {
                directives.push(json!({"role": "verifier", "action": "run-checkpoint-checks", "checkpoint": kid, "checks": missing,
                    "tier": p.policy.tiers.verifier, "working_copy": wc,
                    "brief": p.briefs_dir().join(format!("{kid}-verifier.md")).to_string_lossy(),
                    "limits": {"max_rounds": p.policy.limits.max_rounds, "idle_minutes": p.policy.limits.idle_minutes,
                               "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }},
                    "commands": [format!("bf brief {kid} --role verifier")]}));
                if review_needed {
                    directives.push(crate::ckreview::directive(p, &st, kid)?);
                }
            } else if review_needed {
                directives.push(crate::ckreview::directive(p, &st, kid)?);
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
                    "on_human_step": "stop",
                    "stop_why": s(k, "stop_why"),
                    "note": s(k, "human_step")}));
            } else {
                // Offer the close only when the engine would accept it.
                let waived = gates::waived_ids(&st);
                let mut gaps: Vec<String> = str_list(k, "proves")
                    .into_iter()
                    .filter(|id| !waived.contains_key(id))
                    .filter(|id| {
                        let cs: Vec<&Value> = pl
                            .checks
                            .iter()
                            .filter(|c| {
                                str_list(k, "checks").iter().any(|x| x == s(c, "id"))
                                    && str_list(c, "proves").contains(id)
                            })
                            .collect();
                        !crate::enforce::id_proven(&st, &receipts, &cs, kid, &content)
                    })
                    .map(|id| format!("{id}: no passing receipt at this content from a check the checkpoint lists"))
                    .collect();
                let (check_gaps, unproven) =
                    crate::enforce::checkpoint_gaps(p, &pl, &st, &receipts, k, &content)?;
                gaps.extend(check_gaps);
                if gaps.is_empty() {
                    directives.push(json!({"role": "engine", "action": "close-checkpoint", "checkpoint": kid, "command": format!("bf checkpoint close {kid}")}));
                } else {
                    directives.push(json!({"role": "conductor", "action": "fix-checkpoint-gaps", "checkpoint": kid, "gaps": gaps,
                        "steps": crate::enforce::way_out_steps(&st, &unproven),
                        "note": format!("`bf checkpoint close {kid}` would refuse: {}. Settle each gap, or ask the human to switch the check off for the run.", gaps.join("; "))}));
                }
            }
            // Offered whenever a low finding is open and nothing holds.
            if let Some(d) = crate::ckreview::low_directive(p, &st, kid)? {
                directives.push(d);
            }
        } else {
            waiting.push(format!("{kid}: waiting for its units to land"));
        }
    }
    Ok(json!({"run": run, "directives": directives, "waiting": waiting}))
}

/// The next directives, plus a plain statement of whether anything can start and, if not, why.
pub fn next(p: &Project) -> R<Value> {
    // The idle check runs here, so nobody has to remember `bf stuck`.
    if events::replay(p)?.active_run().is_some() {
        stuck(p, false)?;
    }
    // A checkpoint whose check failed gets a fix for a builder, not a question to the human.
    if events::replay(p)?.active_run().is_some() {
        crate::ckfix::maybe_open(p)?;
        // A wave check that failed gets a fix too, held to the units that landed since the last pass.
        crate::wavecheck::maybe_open(p)?;
    }
    let st0 = events::require_chain(p)?;
    let design_only = st0.active_run().is_none() && !st0.gate_passed("plan", None);
    let mut v = if design_only {
        design_raw(p, &st0)
    } else {
        next_raw(p)?
    };
    if p.policy.roles.tokens && !design_only {
        sync_tokens(p, &mut v)?;
    }
    if let Some(ds) = v["directives"].as_array_mut() {
        for d in ds.iter_mut() {
            add_work(p, d);
        }
    }
    let st = events::replay(p)?;
    let pl = plan::load(p)?;
    let dirs = v["directives"].as_array().cloned().unwrap_or_default();
    let can_start = dirs.iter().any(|d| d["role"] != "human");
    let human_needed = dirs
        .iter()
        .any(|d| d["role"] == "human" && d["info"] != true);
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
    // A directive that waits on the human for a gate or for questions carries the page's link when one
    // was recorded.
    if let Some(ds) = v["directives"].as_array_mut() {
        for d in ds.iter_mut() {
            if d["role"] != "human" {
                continue;
            }
            // The names a page may be recorded under, best first. A question of a stage has the page of
            // its stage (or the page of the old name); a worker's question has the page of its unit.
            let names: Vec<String> = match d["action"].as_str().unwrap_or("") {
                "decide-gate" => d["gate"]
                    .as_str()
                    .map(|g| format!("gate-{g}"))
                    .into_iter()
                    .collect(),
                "answer-question" => match (d["stage"].as_str(), d["unit"].as_str()) {
                    (Some(stage), _) => vec![format!("questions-{stage}"), "questions".to_string()],
                    (None, Some(unit)) if !unit.is_empty() => vec![format!("questions-{unit}")],
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            if let Some((url, _)) = names.iter().find_map(|n| st.pages.get(n)) {
                d["page_url"] = json!(url);
            }
        }
    }
    v["main_ahead"] = json!(main_ahead(p, &st, &pl));
    v["human_turns_note"] = json!(st.human_turns_note().unwrap_or_default());
    if let Some(n) = st.human_turns_note() {
        eprintln!("note: {n}");
    }
    v["can_start"] = json!(can_start);
    v["human_needed"] = json!(human_needed);
    v["idle_reason"] = reason;
    Ok(v)
}

/// The open amendment, neither approved nor withdrawn, that holds a unit not yet started: it adds the
/// unit as a draft, or the unit is in its set or its kept impact (R-69 acceptance 7, F-197, F-211).
pub(crate) fn amend_hold(st: &State, unit: &str) -> Option<String> {
    if !st.unit_state(unit).is_empty() {
        return None;
    }
    st.amends
        .iter()
        .find(|a| {
            matches!(a.status.as_str(), "open" | "narrowed")
                && (a.ids.iter().any(|i| i == unit)
                    || a.keep.iter().any(|i| i == unit)
                    || a.touched.iter().any(|i| i == unit)
                    || a.changes
                        .iter()
                        .any(|c| c["id"] == unit && c["how"] == "new"))
        })
        .map(|a| a.id.clone())
}

fn reasons_text(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Array(x) => x
            .iter()
            .filter_map(|y| y.as_str())
            .collect::<Vec<_>>()
            .join("; "),
        _ => String::new(),
    }
}

/// Whether the judge is due for a narrowed plan repair: the same conditions as the judge's token, so the
/// conductor is never sent for a token the engine refuses.
fn judge_due(p: &Project, st: &State, a: &events::Amend) -> bool {
    let own = |v: &Option<Value>| {
        v.as_ref()
            .is_some_and(|v| v["fingerprint"].as_str() == Some(a.fingerprint.as_str()))
    };
    p.policy.review.repair_judge
        && a.status == "narrowed"
        && crate::amend::is_repair(a)
        && crate::amend::designed_in(&a.ids).is_empty()
        && !a.human_only
        && !a.from_design
        && !own(&a.verdict)
        && a.failed.is_none()
        && a.judge_turns.is_empty()
        && !a
            .changes
            .iter()
            .filter_map(|c| c["id"].as_str())
            .any(|id| st.asked.contains_key(id))
        && crate::judge::refused_records(st, a).is_empty()
}

/// The minutes since the judge's token of the latest narrowing was issued, when it was issued more than
/// the policy's idle limit ago and no verdict or failure ended its turn.
fn judge_idle(idle_minutes: u64, a: &events::Amend) -> Option<u64> {
    if a.status != "narrowed" || a.verdict.is_some() || a.failed.is_some() {
        return None;
    }
    let t = a
        .judge_turns
        .last()
        .filter(|t| t["fingerprint"].as_str() == Some(a.fingerprint.as_str()))?;
    let at = crate::util::parse_rfc3339(t["at"].as_str().unwrap_or(""));
    let age = crate::util::now_secs().saturating_sub(at) / 60;
    (at > 0 && age > idle_minutes).then_some(age)
}

/// The judge's part of `bf next`: the directive for the conductor, the human's step for an `ask`, a
/// failed judge or a judge that cannot report, and the repairs the human has not yet seen (F-212, F-213).
fn judge_flow(p: &Project, st: &State, directives: &mut Vec<Value>, waiting: &mut Vec<String>) {
    use crate::strings::t;
    for a in &st.amends {
        let open = matches!(a.status.as_str(), "open" | "narrowed");
        let verdict = a.verdict.as_ref();
        let asked = verdict.is_some_and(|v| v["verdict"] == "ask");
        let reasons = verdict
            .map(|v| reasons_text(&v["reasons"]))
            .unwrap_or_default();
        let failed = a.failed.as_ref().map(|f| s(f, "reason").to_string());
        if a.status == "narrowed" && crate::amend::is_repair(a) && judge_due(p, st, a) {
            directives.push(
                json!({"role": "conductor", "action": "judge-repair", "work": "amender",
                "amendment": a.id, "commands": [format!("bf roles token {} judge", a.id)],
                "note": t("next.judge_repair.note", &[("amend", &a.id)])}),
            );
            continue;
        }
        if open && a.judge_turns.last().is_some() && verdict.is_none() && failed.is_none() {
            let stale = a
                .judge_turns
                .last()
                .is_some_and(|t| t["fingerprint"].as_str() != Some(a.fingerprint.as_str()));
            let idle = judge_idle(p.policy.limits.idle_minutes, a);
            if stale || idle.is_some() {
                let why = if stale {
                    t("next.judge_stale.reason", &[])
                } else {
                    t(
                        "next.judge_idle.reason",
                        &[("minutes", &idle.unwrap_or(0).to_string())],
                    )
                };
                directives.push(
                    json!({"role": "human", "action": "decide-repair", "amendment": a.id,
                    "judge": "failed", "reason": why,
                    "note": t("next.judge_failed.note", &[("amend", &a.id), ("reason", &why)])}),
                );
            } else {
                waiting.push(t("next.judge_running.line", &[("amend", &a.id)]));
            }
            continue;
        }
        if a.status == "narrowed" && asked && !a.seen {
            directives.push(
                json!({"role": "human", "action": "decide-repair", "amendment": a.id,
                "judge": "ask", "reasons": reasons,
                "note": t("next.judge_ask.note", &[("amend", &a.id), ("reasons", &reasons)])}),
            );
            continue;
        }
        if a.status == "narrowed" && failed.is_some() && !a.seen {
            let why = failed.clone().unwrap_or_default();
            directives.push(
                json!({"role": "human", "action": "decide-repair", "amendment": a.id,
                "judge": "failed", "reason": why,
                "note": t("next.judge_failed.note", &[("amend", &a.id), ("reason", &why)])}),
            );
            continue;
        }
        // The information the human keeps seeing until they say they saw it.
        if a.seen {
            continue;
        }
        let state = if a.status == "withdrawn" {
            t("next.judged_seen.state_withdrawn", &[])
        } else {
            a.status.clone()
        };
        let note = if a.judged && st.judged_unseen.contains(&a.id) {
            Some(t(
                "next.judged_seen.approved",
                &[("amend", &a.id), ("reasons", &reasons)],
            ))
        } else if let Some(why) = &failed {
            Some(t(
                "next.judged_seen.failed",
                &[("amend", &a.id), ("state", &state), ("reason", why)],
            ))
        } else if asked {
            Some(t(
                "next.judged_seen.ask",
                &[("amend", &a.id), ("state", &state), ("reasons", &reasons)],
            ))
        } else {
            None
        };
        if let Some(note) = note {
            let kind = if a.judged {
                "approved"
            } else if failed.is_some() {
                "failed"
            } else {
                "ask"
            };
            directives.push(
                json!({"role": "human", "action": "see-judged-repair", "info": true,
                "amendment": a.id, "judge": kind, "reasons": reasons, "note": note}),
            );
        }
    }
}

#[cfg(test)]
mod next_checks {
    use super::*;

    fn amend_with_turn(at: &str) -> events::Amend {
        events::Amend {
            status: "narrowed".into(),
            fingerprint: "fp".into(),
            judge_turns: vec![json!({"fingerprint": "fp", "seq": 5, "at": at})],
            ..Default::default()
        }
    }

    #[test]
    fn next_checks_a_judge_token_past_the_idle_limit_with_no_verdict_is_idle() {
        let p = 60u64;
        let old = crate::util::rfc3339(crate::util::now_secs() - 3 * 3600);
        let fresh = crate::util::now();
        assert!(judge_idle(p, &amend_with_turn(&old)).is_some());
        assert!(judge_idle(p, &amend_with_turn(&fresh)).is_none());
        let mut a = amend_with_turn(&old);
        a.failed = Some(json!({"reason": "x"}));
        assert!(judge_idle(p, &a).is_none());
    }
}
