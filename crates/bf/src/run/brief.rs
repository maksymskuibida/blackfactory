//! `bf brief`: the brief a worker is given.

use super::*;

/// The engine writes a unit's brief itself at the state change that needs it. When it cannot, the
/// command still succeeds, an older brief of that role is removed, and the answer says so.
pub(crate) fn write_brief(p: &Project, unit: &str, role: &str) -> Option<String> {
    match brief(p, unit, role) {
        Ok(_) => None,
        Err(_) => {
            let _ = std::fs::remove_file(p.briefs_dir().join(format!("{unit}-{role}.md")));
            let note = format!(
                "the {role}'s brief of {unit} could not be written; run `bf brief {unit} --role {role}` and read what it says"
            );
            eprintln!("note: {note}");
            Some(note)
        }
    }
}

pub fn brief(p: &Project, unit: &str, role: &str) -> R<PathBuf> {
    if unit.starts_with("K-") {
        return brief_checkpoint(p, unit, role);
    }
    if crate::roles::is_new_role(role) {
        return crate::roles::brief(p, unit, role);
    }
    if !["builder", "reviewer", "verifier"].contains(&role) {
        return Err(refuse(
            format!("write a brief for role `{role}`"),
            "that is not a role the engine briefs",
            &["Use --role builder, reviewer, verifier, uiux-verifier, security-expert, security-auditor, lawyer or legal-auditor."],
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
    if role == "reviewer" && crate::scope::open_request(&st, unit).is_some() {
        return crate::scope::brief_review(p, unit);
    }
    let mut md = String::new();
    md.push_str(&format!("# Brief: {} as {}\n\n", unit, role));
    if let Some(r) = crate::scope::reconsider::open_retry(&st, unit).filter(|_| role == "builder") {
        md.push_str(&retry_md(p, unit, &r));
    } else if role == "builder" && crate::scope::reconsider::request_from(&st, unit) {
        // The note of the blocked report itself, not the last note of any kind.
        let note = events::load_log(p)
            .ok()
            .and_then(|l| {
                l.events
                    .iter()
                    .rev()
                    .find(|e| e["kind"] == "unit.report" && e["unit"] == unit)
                    .map(|e| s(e, "note").trim().to_string())
            })
            .unwrap_or_default();
        let said = if note.is_empty() {
            String::new()
        } else {
            format!(" and said: {note}")
        };
        md.push_str(&format!("## Look again\n\nYou reported this unit blocked{said}.\n\nLook again before the human is asked. Pick the way that fits:\n\n- A file outside your list that your work needs: ask for it with `bf report {unit} --result blocked --token <token> --need-files -`.\n- A file you changed by mistake outside your list: put it back with `bf unit restore {unit} <path> --token <token>`, then go on.\n- A file whose change is formatting only: name it in your report's note as format only, and report built; the engine proves it.\n- A gap in the design (a requirement, decision or contract that is missing or contradicts this brief): report blocked again. It then goes to the human.\n\n"));
    }
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
    // The checks every unit must pass: the engine attaches them, and the unit cannot land without them.
    let (attached, _, _) = crate::checks::attached_for_unit(p, &pl, &st, unit);
    for (a, line) in attached
        .iter()
        .zip(crate::checks::attached_lines(p, &pl, &st, unit))
    {
        md.push_str(&format!(
            "- {line}. Run it with `bf evidence run {} --unit {unit} -- {}`\n",
            a.id(),
            a.command_line()
        ));
    }
    md.push('\n');
    if role != "verifier" {
        md.push_str("## Where you work\n\n");
        if let Some(sec) = outside_md(p, &st, u, unit, role) {
            md.push_str(&sec);
        }
        md.push_str(&format!(
            "Working copy: `{}`\n\n",
            p.work_dir().join(unit).display()
        ));
        if p.workspace {
            if let Ok(r) = p.repo_of_unit(u) {
                md.push_str(&format!(
                    "Repository: `{}` (the working copy is a git worktree of {}; the paths below are relative to it)\n\n",
                    r.name,
                    r.path.display()
                ));
            }
        }
        md.push_str("Files you may change:\n");
        for f in crate::scope::unit_files(&st, u) {
            md.push_str(&format!("- `{f}`\n"));
        }
        md.push('\n');
        md.push_str(&crate::scope::undo_md(&st, unit));
        if role == "reviewer" {
            md.push_str(&crate::scope::grants_md_unit(&st, unit));
        }
    } else {
        md.push_str("## Where you work\n\nYou are a blind verifier: judge the built result against the contract above. Do not read the source.\n\n");
    }
    if let Some(a) = st
        .units
        .get(unit)
        .map(|x| x.reopened_by.clone())
        .filter(|a| !a.is_empty())
    {
        md.push_str(&format!("## Amendment {a}\n\nThis unit was sent back to work because the human approved amendment {a}. The records it delivers changed: read them again (`bf record get <ID>`) and change what no longer fits. The receipts for its checks from before the amendment no longer count; run the checks again.\n\n"));
    }
    let qs = p.list_records(schema::kind_by_name("question"))?;
    let asked: Vec<&Value> = qs.iter().filter(|q| s(q, "unit") == unit).collect();
    if !asked.is_empty() {
        md.push_str("## Questions you asked the human\n\n");
        for q in asked {
            match q.get("answer") {
                Some(_) => md.push_str(&format!(
                    "- {} {}: answered: {}\n",
                    s(q, "id"),
                    s(q, "title"),
                    crate::store::answer_text(q)
                )),
                None => md.push_str(&format!(
                    "- {} {}: not answered yet; do not guess, wait for the answer\n",
                    s(q, "id"),
                    s(q, "title")
                )),
            }
        }
        md.push('\n');
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
            if pl.unit_checks(unit).iter().any(|c| s(c, "id") == r.check())
                || (r.check().starts_with(crate::checks::ATTACHED_PREFIX)
                    && s(&r.v, "unit") == unit)
            {
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
    if role == "reviewer" {
        md.push_str(&builder_claim_md(p, unit));
    }
    if role == "reviewer" {
        let wc = p.work_dir().join(unit);
        let prior: Vec<Value> = st
            .units
            .get(unit)
            .map(|x| {
                // The round an accept adds is not a review round the reviewer filed.
                x.review_rounds
                    .iter()
                    .filter(|r| r["accepted"] != true)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        md.push_str(&format!("## Review round {}\n\n", prior.len() + 1));
        match prior.last() {
            None => md.push_str("This is the first round and it is cold: no earlier report is given. Read everything in the unit's scope.\n\n"),
            Some(l) => {
                md.push_str("This is a later round. The findings above are every earlier finding with its outcome: check that each open one is fixed, and start from the changes made since the content the round before judged. Do not review the whole scope again when nothing points there. You may file a new finding on what changed, on something a fix broke, or on anything you came across while following a change.\n\n");
                let head = l["head"].as_str().unwrap_or("");
                if let Some((files, d)) = unit_sync::resolution_diff(p, unit) {
                    let lines: Vec<&str> = d.lines().collect();
                    let shown = if lines.len() > 400 {
                        format!(
                            "{}\n{}",
                            lines[..400].join("\n"),
                            crate::strings::t("land_conflict.review.more", &[("count", &(lines.len() - 400).to_string())])
                        )
                    } else {
                        d.clone()
                    };
                    md.push_str(&format!(
                        "### {}\n\n{}\n\n```diff\n{}\n```\n\n",
                        crate::strings::t("land_conflict.review.title", &[]),
                        crate::strings::t("land_conflict.review.intro", &[("files", &files.join(", "))]),
                        shown
                    ));
                } else if !head.is_empty() {
                    let d = gitx::must(&wc, &["diff", head]).unwrap_or_default();
                    let lines: Vec<&str> = d.lines().collect();
                    md.push_str(&format!(
                        "### Changes since round {}\n\n```diff\n{}\n```\n\n",
                        l["round"],
                        if lines.len() > 400 { format!(
                            "{}\n{}",
                            lines[..400].join("\n"),
                            crate::strings::t("land_conflict.review.more", &[("count", &(lines.len() - 400).to_string())])
                        ) } else { d.clone() }
                    ));
                }
            }
        }
        md.push_str("In any round you may read outside the changes, anywhere in the working copy, when you need to in order to judge them: what a change touches, calls or could break, and the code it relies on.\n\n");
    }
    if role == "builder" {
        md.push_str(&unblock::brief_md(p, &st, unit));
        md.push_str(&unit_sync::brief_md(p, unit));
    }
    md.push_str("## Limits\n\n");
    md.push_str(&format!(
        "- Review rounds: up to {} (so far {}).\n- Idle limit: {} minutes.\n",
        p.policy.limits.max_rounds,
        st.units.get(unit).map(|x| x.rounds).unwrap_or(0),
        p.policy.limits.idle_minutes
    ));
    md.push_str(&format!(
        "\n## Rules\n\n- Report with `bf report`.\n- Work only inside your working copy, `{}`. It lives under the engine's `.bf/work/` folder on purpose; do not touch anything else under `.bf/`, and leave the `factory/` folder inside your working copy alone: records change through the engine only.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot go on without the human's decision, ask: `bf ask --title \"<the question>\" --context \"<why>\" [--options \"A=first;B=second\"]`. Your unit then waits for the answer and the others go on; the answer is in your next brief.\n- A file outside your list that your work needs, most often a test your own change breaks, is a request with `--need-files`, never a plain blocked report.\n- A file you changed by mistake outside your list goes back with `bf unit restore {unit} <path> --token <token>`; if its change is formatting only, name it in your report's note as format only and the engine proves it.\n- A plain blocked report is for a gap in the design only: a requirement, decision or contract that is missing or contradicts the brief.\n- A check counts only when `bf evidence run` ran it.\n",
        p.work_dir().join(unit).display(),
        unit = unit
    ));
    md.push_str(&format!(
        "- {}\n",
        crate::strings::t("cli.evidence.brief.rule.checks_engine", &[])
    ));
    if role == "reviewer" {
        md.push_str(&format!(
            "- {}\n",
            crate::strings::t("cli.evidence.brief.rule.reviewer_receipts", &[])
        ));
    }
    let path = p.briefs_dir().join(format!("{unit}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// The section of a builder's brief for an open retry: each file to correct with the reviewer's
/// words, its round and the rounds left, the needed files to ask for again and the command.
fn retry_md(p: &Project, unit: &str, r: &Value) -> String {
    use crate::strings::t;
    let lim = crate::scope::retry::limit(p);
    let mut md = format!(
        "## {}\n\n{}\n\n",
        t("scope.retry.brief.title", &[]),
        t("scope.retry.brief.intro", &[])
    );
    let mut needed = String::new();
    for f in r["files"].as_array().into_iter().flatten() {
        let path = s(f, "path");
        if s(f, "verdict") == "needed" {
            needed.push_str(&format!(
                "{}\n",
                t("scope.retry.brief.needed_file", &[("path", path)])
            ));
            continue;
        }
        let round = f["round"].as_u64().unwrap_or(1) as u32;
        md.push_str(&format!(
            "{}\n",
            t(
                "scope.retry.brief.file",
                &[
                    ("path", path),
                    (
                        "why",
                        &s(f, "why").split_whitespace().collect::<Vec<_>>().join(" ")
                    ),
                    (
                        "must",
                        &s(f, "must")
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    ("round", &round.to_string()),
                    ("limit", &lim.to_string()),
                    ("left", &lim.saturating_sub(round).to_string()),
                ]
            )
        ));
    }
    md.push('\n');
    if !needed.is_empty() {
        md.push_str(&format!(
            "{}\n\n{needed}\n",
            t("scope.retry.brief.needed_title", &[])
        ));
    }
    md.push_str(&format!(
        "{}\n\n{}\n\n",
        t("scope.retry.brief.command", &[("unit", unit)]),
        t("scope.retry.brief.plain", &[])
    ));
    md
}

/// The files changed outside the unit's list, for the reviewer's and the builder's brief.
fn outside_md(
    p: &Project,
    st: &events::State,
    u: &Value,
    unit: &str,
    role: &str,
) -> Option<String> {
    if !["builder", "reviewer"].contains(&role) {
        return None;
    }
    let wc = p.work_dir().join(unit);
    if !wc.exists() {
        return None;
    }
    let ck = p.checkpoint_branch(s(u, "checkpoint"));
    let base = gitx::must(&wc, &["merge-base", &ck, "HEAD"])
        .ok()
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .or_else(|| {
            st.units
                .get(unit)
                .map(|x| x.base_commit.clone())
                .filter(|b| !b.is_empty())
        })?;
    let items = crate::outside::changed(&wc, &base, "", &crate::scope::unit_files(st, u));
    Some(crate::outside::section(&items, role))
}

/// The builder's latest report of the unit, as a claim to check.
fn builder_claim_md(p: &Project, unit: &str) -> String {
    let Ok(l) = events::load_log(p) else {
        return String::new();
    };
    l.events
        .iter()
        .rev()
        .find(|e| e["kind"] == "unit.report" && e["unit"] == unit && e["role"] == "builder")
        .map(|e| crate::outside::claim(s(e, "result"), s(e, "note")))
        .unwrap_or_default()
}
