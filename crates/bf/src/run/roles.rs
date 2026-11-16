//! The reports of the new roles, their tokens and the brief of a checkpoint's verifier.

use super::*;

/// One of the five new roles reports with its own token; the unit's state does not move. The design
/// stage (`design`) needs no run and no plan; a unit or a checkpoint needs a run. A role reports only
/// on a subject at its stage.
pub(crate) fn role_report(
    p: &Project,
    subject: &str,
    result: &str,
    r: &str,
    note: &str,
    cost: f64,
    token: Option<&str>,
) -> R<Value> {
    let st = events::require_chain(p)?;
    if subject != crate::roles::DESIGN {
        need_run(&st)?;
    }
    if !["built", "blocked"].contains(&result) {
        return Err(refuse(
            format!("report {result} as {r}"),
            "a new role reports `built` (done) or `blocked`",
            &["Use --result built or --result blocked; findings and threats are filed with `bf record put`."],
            "The command prints the unit's state.",
        ));
    }
    crate::roles::check_subject(p, subject, r)?;
    if p.policy.roles.tokens {
        let key = check_token(p, subject, r, token)?;
        spend_token(p, &key)?;
    }
    events::append(
        p,
        "unit.role-report",
        json!({"unit": subject, "result": result, "role": r, "note": note, "cost": cost.max(0.0)}),
    )?;
    let state = if subject == crate::roles::DESIGN {
        String::new()
    } else {
        st.unit_state(subject).to_string()
    };
    Ok(json!({"unit": subject, "result": result, "state": state, "role": r}))
}

/// A role's token ends with the run it was issued in: starting or stopping a run drops every token
/// of the new roles (a design-stage token is dropped when the first run starts).
pub(crate) fn drop_role_tokens(p: &Project) -> R<()> {
    let _tl = crate::lock::tokens(p)?;
    let mut v = read_tokens(p);
    if let Some(o) = v.as_object_mut() {
        o.retain(|k, _| {
            !k.split_once(':')
                .map(|(_, r)| crate::roles::is_new_role(r))
                .unwrap_or(false)
        });
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(())
}

pub(crate) fn brief_checkpoint(p: &Project, k: &str, role: &str) -> R<PathBuf> {
    if matches!(role, "builder" | "reviewer") {
        if let Some(so) = events::replay(p)?.sync_open.filter(|o| o.checkpoint == k) {
            return brief_sync(p, &so, role);
        }
        if let Some(f) = events::replay(p)?.ck_fix.filter(|f| f.checkpoint == k) {
            return crate::ckfix::brief(p, &f, role);
        }
    }
    if role == "reviewer" {
        return crate::ckreview::brief(p, k);
    }
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
        "\n## Outcomes\n\nRun each check from inside `{wc}` so the receipt is for the checkpoint's content. Then, for each ID you judged:\n\n- met: the receipt from `bf evidence run` is the proof.\n- failed: the check ran and does not match the contract, or it fails without a receipt you can show: `bf evidence fail <check> --reason \"what you saw\"`.\n- unreachable: you could not run it: `bf evidence unreachable <check> --reason \"what blocked you\"`. This is not a pass. The checkpoint then waits for the human.\n\nGive your outcomes in your last message too.\n\n## Rules\n\n- Never edit anything under `factory/` or `.bf/` (the working copy named above is the only place under `.bf/` you may use).\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot judge without the human's decision, ask: `bf ask --title \"<the question>\"`. This checkpoint then waits for the answer.\n",
        wc = wc.display()
    ));
    let path = p.briefs_dir().join(format!("{k}-verifier.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}
