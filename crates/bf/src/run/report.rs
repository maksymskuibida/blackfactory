//! `bf report`: what a worker says about its unit.

use super::*;

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
    if let Some(r) = role.filter(|r| crate::roles::is_new_role(r)) {
        return role_report(p, unit, result, r, note, cost, token);
    }
    if result == "scope" && !unit.starts_with("K-") {
        return crate::scope::verdict(p, unit, role, note, findings, cost, token);
    }
    if unit.starts_with("K-") {
        let st = events::require_chain(p)?;
        if crate::ckreview::wants(p, &st, unit, role) {
            return crate::ckreview::report(p, unit, result, note, findings, token);
        }
        if crate::ckfix::wants(&st, unit, result, role) {
            return crate::ckfix::report(p, unit, result, role, note, findings, token);
        }
        return sync_report(p, unit, result, role, note, findings, token);
    }
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
    if crate::scope::open_request(&st, unit).is_some() {
        return Err(refuse(
            format!("report {result} on {unit}"),
            "a request of this unit for files is with the scope reviewer; the unit goes on when the verdict is in",
            &["`bf next` shows the review-scope directive; its reviewer reports the verdict."],
            "`bf status` shows the unit without an open request.",
        ));
    }
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
            let id = p.write_numbered(kind, &mut rec)?;
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
    let mut ev = json!({"unit": unit, "result": result, "role": expected_role, "from": cur, "to": to, "note": note, "cost": cost_total, "findings": finding_ids});
    if cur == "review" {
        // The round this report closes: its number, the content it judged and what it covered.
        let round = st.units.get(unit).map(|u| u.rounds).unwrap_or(0) + 1;
        let wc = p.work_dir().join(unit);
        ev["round"] = json!(round);
        ev["content"] = json!(p.content_hash(&wc).unwrap_or_default());
        ev["head"] = json!(gitx::must(&wc, &["rev-parse", "HEAD"]).unwrap_or_default());
        ev["covered"] = json!(if st
            .units
            .get(unit)
            .map(|u| u.review_rounds.is_empty())
            .unwrap_or(true)
        {
            "everything in its scope".to_string()
        } else {
            "the earlier findings and what changed since the round before".to_string()
        });
    }
    events::append(p, ev_kind, ev)?;
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
