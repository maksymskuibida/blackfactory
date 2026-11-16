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

pub(crate) fn budget_check(p: &Project, st: &State) -> R<()> {
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

    // With the gates in order, the one to offer is the first that is not approved or skipped: decide it
    // if it is open, otherwise open it, or say which record is missing for it to open.
    if p.policy.gates.in_order && !st.gate_passed("plan", None) {
        let kind = gates::first_unpassed_design_gate(&st);
        if let Some(g) = st.latest_gate(kind, None).filter(|g| g.status == "open") {
            directives.push(
                json!({"role": "human", "action": "decide-gate", "gate": g.id, "gate_kind": kind,
                "note": format!("Ask the human to approve or reject the {kind} gate in chat.")}),
            );
        } else if let Some(why) = gates::open_blocker(p, &st, kind) {
            let mut d = json!({"role": "conductor", "action": "add-records", "gate_kind": kind, "missing": why,
                "note": format!("The {kind} gate cannot open yet: {why}. Write what it needs, then ask again.")});
            if kind == "ui" {
                // A product with no interface needs no screen: the human can skip the gate.
                d["skip_command"] = json!("bf gate skip ui --quote \"<the human's words>\"");
                d["note"] = json!(format!("The ui gate cannot open yet: {why}. Write a screen or journey, or, for a product with no interface, ask the human whether to skip the ui gate and record their words with the skip command."));
            }
            directives.push(d);
        } else {
            directives.push(json!({"role": "human", "action": "open-gate", "gate_kind": kind, "command": format!("bf gate open {kind}")}));
        }
        return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
    }
    match st.latest_gate("plan", None) {
        Some(g) if g.status == "approved" => {
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
        Some(g) if g.status == "open" => {
            directives.push(json!({"role": "human", "action": "decide-gate", "gate": g.id, "note": "Ask the human to approve or reject the plan in chat."}));
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
        }
        _ => {
            // The run's two plan questions come first: the plan gate does not open until both are answered.
            if let Some(r) = st.active_run().filter(|_| !pl.units.is_empty()) {
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
            return Ok(json!({"run": run, "directives": directives, "waiting": waiting}));
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
    for (id, state, why) in unit_states(p, &pl, &st, &receipts) {
        let u = pl.unit(&id).unwrap();
        if held(&id) && matches!(state.as_str(), "ready" | "running" | "fixing" | "review") {
            waiting.push(format!(
                "{id}: waiting for the human's answer to its question"
            ));
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
            "ready" if !unanswered_questions(&st).is_empty() => {
                waiting.push(format!("{id}: waiting for the human's answers to the run's plan questions"))
            }
            "ready" => directives.push(json!({"role": "builder", "action": "start-unit", "unit": id, "tier": tier_for(p, u, "builder"),
                "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                "commands": [format!("bf unit start {id}"), format!("bf brief {id} --role builder")]})),
            "running" | "fixing" if crate::scope::open_request(&st, &id).is_some() => {
                // A builder's request for files is with a cold reviewer: its directive replaces the
                // builder's until the verdict is in.
                directives.push(crate::scope::directive(p, &st, &id, limits(rounds)));
            }
            "running" | "fixing" => {
                let mut d = json!({"role": "builder", "action": if state == "fixing" { "fix" } else { "build" }, "unit": id,
                    "tier": tier_for(p, u, "builder"), "working_copy": wc, "brief": brief("builder"), "limits": limits(rounds),
                    "commands": [format!("bf brief {id} --role builder")]});
                if let Some(a) = st.units.get(&id).map(|x| x.reopened_by.clone()).filter(|a| !a.is_empty()) {
                    d["amendment"] = json!(a);
                }
                directives.push(d);
            }
            "review" => directives.push(json!({"role": "reviewer", "action": "review", "unit": id, "tier": tier_for(p, u, "reviewer"),
                "working_copy": wc, "brief": brief("reviewer"), "limits": limits(rounds),
                "commands": [format!("bf brief {id} --role reviewer")]})),
            "reviewed" => directives.push(json!({"role": "engine", "action": "land", "unit": id, "command": format!("bf unit land {id}")})),
            "blocked" | "parked" => {
                let mut note = format!("{id} is {state}; the human decides whether to re-plan or release it.");
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
                    note.push_str(&format!(" A request for files was refused ({}): {}. The plan repair is the human's.", s(r, "refusal"), files.join("; ")));
                }
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
                    "note": "A check failed or could not be run. The human decides: fix the cause and have the verifier run it again (a passing run replaces the mark), accept the gap by amendment, or change the plan."}));
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
            if !review_needed {
                if let Some(d) = crate::ckreview::low_directive(p, &st, kid)? {
                    directives.push(d);
                }
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
    let mut v = next_raw(p)?;
    if p.policy.roles.tokens {
        sync_tokens(p, &mut v)?;
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
    // A directive that waits on the human for a gate or for questions carries the page's link when one
    // was recorded.
    if let Some(ds) = v["directives"].as_array_mut() {
        for d in ds.iter_mut() {
            if d["role"] != "human" {
                continue;
            }
            let page = match d["action"].as_str().unwrap_or("") {
                "decide-gate" => d["gate"].as_str().map(|g| format!("gate-{g}")),
                "answer-question" => Some("questions".to_string()),
                _ => None,
            };
            if let Some((url, _)) = page.and_then(|n| st.pages.get(&n)) {
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
