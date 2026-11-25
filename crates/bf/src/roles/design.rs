//! The design-review part of the roles (C-24): what the design reviewer reads for each kind of review
//! and how its brief is written. The role itself is in the table of `roles.rs`.
//!
//! This file also holds the tokens and briefs of the agents that work on an amendment (R-98, R-99): the
//! design reviewer of one kind, the stage agents, and the general designer's split of an amendment
//! into stages. `cli/review.rs` calls it.

use crate::checks::{kind_records, ReviewKind};
use crate::events::{self, Amend};
use crate::refusal::{refuse, Fail, R};
use crate::run;
use crate::schema;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The role file of the design reviewer: its rules per kind are read from it, so the brief and the
/// file never differ.
const ROLE_FILE: &str = include_str!("../../../../harness/claude/agents/design-reviewer.md");

/// The stages an amendment can be split into, in the order they are filled.
pub const STAGES: [&str; 6] = [
    "requirements",
    "ui",
    "architecture",
    "threats",
    "legal",
    "plan",
];

/// The roles that draft a stage, and the one reviewer that is a stage agent of the UI/UX stage.
fn stage_roles(stage: &str) -> &'static [&'static str] {
    match stage {
        "requirements" | "architecture" | "plan" => &["designer"],
        "ui" => &["uiux-designer", "uiux-verifier"],
        "legal" => &["lawyer"],
        "threats" => &["security-expert"],
        _ => &[],
    }
}

/// Whether the role judges a stage instead of drafting it.
fn is_reviewing_role(role: &str) -> bool {
    role == "uiux-verifier"
}

fn what_to_draft(stage: &str) -> &'static str {
    match stage {
        "requirements" => "Draft the requirements and journeys of the change, each with acceptance that can be tested. Keep them consistent with the screens, the components and the plan; link each requirement to another design part.",
        "ui" => "Draft the screens and journeys of the change. Keep them consistent with the requirements they serve and the components that supply their data.",
        "architecture" => "Draft the components, decisions and targets of the change. Every decision names the options it rejected. Keep them consistent with the requirements they serve and with the threats.",
        "threats" => "Draft the threats of the architecture stage: what is protected, the attack, the mitigation. Keep them consistent with the components and decisions they cover.",
        "legal" => "Draft the legal records and the markets record the change needs, each with a source. Keep them consistent with the markets the product serves.",
        "plan" => "Draft the units, handoffs, checks and checkpoints of the change. Every requirement is delivered by a unit and proved by a check; units are at most 8 points and ordered.",
        _ => "",
    }
}

fn bad_amend(what: &str, because: String, step: &str) -> Fail {
    refuse(
        what.to_string(),
        because,
        &[step],
        "The command prints its result.",
    )
}

/// The amendment `id`, when it is open (not approved, not withdrawn) and not a plan repair.
fn open_amend(p: &Project, id: &str) -> R<Amend> {
    let st = events::require_chain(p)?;
    let what = format!("issue a token on {id}");
    let Some(a) = st.amends.iter().find(|a| a.id == id) else {
        return Err(bad_amend(
            &what,
            "there is no such amendment".into(),
            "Use the ID of the open amendment from `bf status`.",
        ));
    };
    if !matches!(a.status.as_str(), "open" | "narrowed") {
        return Err(bad_amend(
            &what,
            format!("{id} is {}, and no amendment is open", a.status),
            "A token on an amendment is issued while it is open; open one with `bf amend open <ID>...`.",
        ));
    }
    if crate::amend::is_repair(a) {
        return Err(bad_amend(
            &what,
            format!("{id} is a plan repair, which needs no designer and no review"),
            "A plan repair is judged by the human alone.",
        ));
    }
    Ok(a.clone())
}

fn subject_id(subject: &str) -> R<()> {
    match schema::parse_id(subject) {
        Some(("A", _)) => Ok(()),
        _ => Err(bad_amend(
            &format!("use the subject `{subject}`"),
            "a subject is an amendment ID like A-1".into(),
            "Give `--subject A-n`, or leave it out for a review of a design stage.",
        )),
    }
}

/// The kind a design reviewer judges: a kind of a design gate, or of an amendment when `amend`.
fn review_kind(kind: &str, amend: bool) -> R<&'static str> {
    let r = super::get("design-reviewer").expect("the design reviewer is in the table");
    let found = if amend {
        r.amendment_kinds
            .iter()
            .copied()
            .chain(["agreement"])
            .find(|k| *k == kind)
    } else {
        r.kinds.iter().copied().find(|k| *k == kind)
    };
    found.ok_or_else(|| {
        let list: Vec<&str> = if amend {
            r.amendment_kinds
                .iter()
                .copied()
                .chain(["agreement"])
                .collect()
        } else {
            r.kinds.to_vec()
        };
        bad_amend(
            &format!("use the review kind `{kind}`"),
            format!(
                "the design reviewer judges {} of {}",
                list.join(", "),
                if amend { "an amendment" } else { "a design stage" }
            ),
            if amend {
                "Use one of the kinds of an amendment with `--subject A-n`."
            } else {
                "Use one of the kinds of a design stage; a kind of an amendment needs `--subject A-n`."
            },
        )
    })
}

fn key_of_kind(subject: &str, kind: &str) -> String {
    format!("{subject}:design-reviewer:{kind}")
}

/// `bf review token <kind> [--subject A-n]`: the token of the design reviewer of one kind.
pub fn issue_kind_token(p: &Project, kind: &str, subject: Option<&str>) -> R<Value> {
    let subj = match subject {
        Some(a) => {
            subject_id(a)?;
            open_amend(p, a)?;
            a
        }
        None => super::DESIGN,
    };
    let kind = review_kind(kind, subject.is_some())?;
    let token = run::issue_token(p, &key_of_kind(subj, kind))?;
    Ok(json!({"subject": subj, "role": "design-reviewer", "kind": kind, "token": token}))
}

/// The splits the log holds: amendment -> stage -> IDs.
fn splits(p: &Project, amend: &str) -> R<BTreeMap<String, Vec<String>>> {
    let log = events::load_log(p)?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ev in &log.events {
        if ev["kind"] == "amend.split" && ev["amendment"] == amend {
            if let Some(o) = ev["stages"].as_object() {
                for (st, ids) in o {
                    out.insert(st.clone(), str_list(&json!({"x": ids}), "x"));
                }
            }
        }
    }
    Ok(out)
}

/// Whether the split of `amend` gave `id` to `stage`: what a stage agent may put (U-70's acceptance of
/// design records calls it). False when there is no split, the stage was not named or the ID is not its.
pub fn stage_may_put(p: &Project, amend: &str, stage: &str, id: &str) -> bool {
    splits(p, amend)
        .map(|m| {
            m.get(stage)
                .map(|ids| ids.iter().any(|i| i == id))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// `bf review token --subject A-n --stage <stage> --role <role>`: the token of a stage agent.
pub fn issue_stage_token(p: &Project, amend: &str, stage: &str, role: &str) -> R<Value> {
    subject_id(amend)?;
    if !STAGES.contains(&stage) {
        return Err(bad_amend(
            &format!("issue a token for the stage `{stage}`"),
            format!("a stage is one of {}", STAGES.join(", ")),
            "Use `--stage` with one of them.",
        ));
    }
    if !stage_roles(stage).contains(&role) {
        return Err(bad_amend(
            &format!("issue a token for `{role}` at the stage `{stage}`"),
            format!(
                "the stage `{stage}` is worked by {}",
                stage_roles(stage).join(" or ")
            ),
            "Use `--role` with the role of the stage.",
        ));
    }
    open_amend(p, amend)?;
    if !is_reviewing_role(role) {
        let split = splits(p, amend)?;
        if !split.contains_key(stage) {
            return Err(bad_amend(
                &format!("issue a token for {role} at the stage `{stage}` of {amend}"),
                if split.is_empty() {
                    "the general designer has not split the amendment; no stage designer exists before the split names its stage".to_string()
                } else {
                    format!("the split of {amend} did not name the stage `{stage}`")
                },
                "The general designer splits with `bf review split <A-n> --stage <stage>=<ID,ID> --token <token>`.",
            ));
        }
        let log = events::load_log(p)?;
        if log.events.iter().any(|e| {
            e["kind"] == "review.stage-token" && e["amend"] == amend && e["stage"] == stage
        }) {
            return Err(bad_amend(
                &format!("issue a second token for the stage `{stage}` of {amend}"),
                "a stage has at most one designer for each amendment, and its token was issued already".into(),
                "Use the token that was handed out; a stage designer cannot ask for another agent.",
            ));
        }
        events::append(
            p,
            "review.stage-token",
            json!({"amend": amend, "stage": stage, "role": role}),
        )?;
        p.write_guard()?;
    }
    let token = run::issue_token(p, &format!("{amend}:{role}:{stage}"))?;
    Ok(json!({"subject": amend, "role": role, "stage": stage, "token": token}))
}

/// `bf review split <A-n> --stage <stage>=<ID,ID>... --token <t>`: the general designer's split.
pub fn split(p: &Project, amend: &str, stages: &[String], token: Option<&str>) -> R<Value> {
    subject_id(amend)?;
    let a = open_amend(p, amend)?;
    if !run::holds_token(p, &format!("{amend}:designer"), token) {
        return Err(bad_amend(
            &format!("split {amend}"),
            "only the general designer of the amendment splits it, with its own token; none, or another token, was given (a stage designer cannot split)".into(),
            "Give `--token` with the token of the general designer.",
        ));
    }
    if stages.is_empty() {
        return Err(bad_amend(
            &format!("split {amend}"),
            "no stage was named".into(),
            "Give `--stage <stage>=<ID,ID>` once for each stage handed out.",
        ));
    }
    let have = splits(p, amend)?;
    let mut new: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut given: Vec<String> = have.values().flatten().cloned().collect();
    for spec in stages {
        let what = format!("split {amend} by `{spec}`");
        let Some((stage, ids)) = spec.split_once('=') else {
            return Err(bad_amend(
                &what,
                "a stage is given as `<stage>=<ID,ID>`".into(),
                "Write `--stage requirements=R-3,R-4`.",
            ));
        };
        if !STAGES.contains(&stage) {
            return Err(bad_amend(
                &what,
                format!(
                    "`{stage}` is not a stage; the stages are {}",
                    STAGES.join(", ")
                ),
                "Use one of the stages.",
            ));
        }
        if have.contains_key(stage) || new.contains_key(stage) {
            return Err(bad_amend(
                &what,
                format!("the stage `{stage}` is named already"),
                "Name each stage once.",
            ));
        }
        let ids: Vec<String> = ids
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        if ids.is_empty() {
            return Err(bad_amend(
                &what,
                "the stage got no record ID".into(),
                "Name the IDs of the amendment's set the stage drafts.",
            ));
        }
        for id in &ids {
            if !a.ids.contains(id) {
                return Err(bad_amend(
                    &what,
                    format!("{id} is not in the set of {amend} ({})", a.ids.join(", ")),
                    "A stage may be given only IDs of the amendment's set.",
                ));
            }
            if given.contains(id) {
                return Err(bad_amend(
                    &what,
                    format!("{id} is given to another stage already"),
                    "Each ID goes to one stage.",
                ));
            }
            given.push(id.clone());
        }
        new.insert(stage.to_string(), ids);
    }
    events::append(p, "amend.split", json!({"amendment": amend, "stages": new}))?;
    p.write_guard()?;
    Ok(json!({"amend": amend, "split": new}))
}

// ---- briefs ----

fn pretty_record(content: &str) -> String {
    match serde_json::from_str::<Value>(content) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| content.to_string()),
        Err(_) => content.to_string(),
    }
}

/// The rules of one kind, as the role file lists them.
fn kind_rule(kind: &str) -> String {
    let head = format!("- **{kind}**");
    let mut out = String::new();
    let mut on = false;
    for line in ROLE_FILE.lines() {
        if line.starts_with(&head) {
            on = true;
        } else if on
            && (line.starts_with("- **") || line.starts_with('#') || line.trim().is_empty())
        {
            break;
        }
        if on {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The section of the role file about asking for a stage review.
fn asking_rule() -> String {
    let mut out = String::new();
    let mut on = false;
    for line in ROLE_FILE.lines() {
        if line.starts_with("## Asking for a stage review") {
            on = true;
            continue;
        }
        if on && line.starts_with("## ") {
            break;
        }
        if on {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

fn limits_and_tier(p: &Project) -> String {
    format!(
        "## Tier and effort\n\nTier: {}. Effort: {} (the policy's entry for a design stage).\n\n## Limits\n\n- Review rounds: up to {}.\n- Idle limit: {} minutes.\n",
        p.policy.tiers.designer, p.policy.efforts.designer, p.policy.limits.max_rounds, p.policy.limits.idle_minutes
    )
}

/// The record as committed before the amendment, if it was; the text of the file now.
fn before_after(p: &Project, id: &str) -> (Option<String>, Option<String>) {
    let rel = format!("factory/records/{id}.toml");
    let now = std::fs::read_to_string(p.root.join(&rel)).ok();
    let before = crate::gitx::run(&p.root, &["show", &format!("HEAD:{rel}")], &[])
        .ok()
        .filter(|o| o.ok)
        .map(|o| o.stdout);
    (before, now)
}

fn stages_of_kind(kind: &str) -> &'static [&'static str] {
    match kind {
        "idea" => &["idea"],
        "requirements" => &["requirements"],
        "architecture" => &["architecture"],
        "legal" => &["architecture", "legal"],
        "plan" => &["plan"],
        _ => &["requirements", "ui", "architecture", "legal", "plan"],
    }
}

fn is_open_finding(f: &Value) -> bool {
    matches!(f["outcome"].as_str(), None | Some("open"))
}

/// `bf review brief <kind> [--subject A-n]`: the brief of the design reviewer of one kind.
pub fn brief_kind(p: &Project, kind: &str, subject: Option<&str>) -> R<PathBuf> {
    let amend = match subject {
        Some(a) => {
            subject_id(a)?;
            Some(open_amend(p, a)?)
        }
        None => None,
    };
    let kind = review_kind(kind, amend.is_some())?;
    let subj = subject.unwrap_or(super::DESIGN);
    let rk = ReviewKind::parse(kind).unwrap_or(ReviewKind::Amendment);
    let set: Option<Vec<String>> = amend.as_ref().map(|a| {
        let mut v = a.ids.clone();
        v.extend(a.touched.iter().cloned());
        v.sort();
        v.dedup();
        v
    });
    let recs = kind_records(p, rk, set.as_deref())?;
    let names: Vec<&str> = recs.iter().map(|(n, _)| n.as_str()).collect();
    let r = super::get("design-reviewer").expect("the design reviewer is in the table");
    let mut md = format!("# Brief: {subj} as design reviewer, {kind}\n\n");
    md.push_str(&format!(
        "## Your role\n\n{} Kind: {kind}. Stage: design.\n\n",
        r.summary
    ));
    md.push_str("## The records you judge\n\n");
    if recs.is_empty() {
        md.push_str("None yet.\n\n");
    }
    for (name, content) in &recs {
        md.push_str(&format!(
            "### {name}\n\n```\n{}\n```\n\n",
            pretty_record(content)
        ));
    }
    if let (Some(a), Some(set)) = (&amend, &set) {
        md.push_str(&format!(
            "## The amendment {}\n\nIts set: {}\n\n",
            a.id,
            set.join(", ")
        ));
        md.push_str("### Each changed record, before and after\n\n");
        for id in set {
            let (before, now) = before_after(p, id);
            match (&before, &now) {
                (Some(b), Some(n)) if b == n => {
                    md.push_str(&format!("- {id}: unchanged since the last commit\n"))
                }
                (b, n) => {
                    md.push_str(&format!(
                        "#### {id}\n\nBefore:\n\n```\n{}\n```\n\nAfter:\n\n```\n{}\n```\n\n",
                        b.as_deref().unwrap_or("(a new record)").trim_end(),
                        n.as_deref().unwrap_or("(no record yet)").trim_end()
                    ));
                }
            }
        }
        md.push_str("\n### The stage decisions\n\n");
        let mut any = false;
        for (name, content) in &recs {
            if let Ok(v) = serde_json::from_str::<Value>(content) {
                if v["kind"] == "decision" && v.get("stage").is_some() {
                    md.push_str(&format!(
                        "- {name} (stage {}): {}\n",
                        s(&v, "stage"),
                        s(&v, "title")
                    ));
                    any = true;
                }
            }
        }
        if !any {
            md.push_str("- none\n");
        }
        md.push_str("\n### The impact lines of the ledger\n\n");
        let mut lines = 0;
        if let Some(o) = a.impact.as_object() {
            for (k, v) in o {
                let ids = str_list(&json!({"x": v}), "x");
                if !ids.is_empty() {
                    md.push_str(&format!("- {k}: {}\n", ids.join(", ")));
                    lines += 1;
                }
            }
        }
        if lines == 0 {
            md.push_str("- none\n");
        }
        md.push_str("\n### The split\n\n");
        let sp = splits(p, &a.id)?;
        if sp.is_empty() {
            md.push_str("Not split: one designer drafted the amendment.\n");
        }
        for (st, ids) in &sp {
            md.push_str(&format!("- {st}: {}\n", ids.join(", ")));
        }
        md.push('\n');
    }
    // The records they link to that the review does not judge.
    md.push_str("## The approved records of earlier stages they link to\n\n");
    let mut linked: Vec<String> = Vec::new();
    for (_, content) in &recs {
        if let Ok(v) = serde_json::from_str::<Value>(content) {
            for l in str_list(&v, "links") {
                if !names.contains(&l.as_str()) && !linked.contains(&l) {
                    linked.push(l);
                }
            }
        }
    }
    linked.sort();
    let mut any = false;
    for id in &linked {
        if let Some(rec) = p.get_record(id)? {
            any = true;
            md.push_str(&format!(
                "### {id}\n\n```\n{}\n```\n\n",
                serde_json::to_string_pretty(&rec).unwrap_or_default()
            ));
        }
    }
    if !any {
        md.push_str("None.\n\n");
    }
    md.push_str("## The questions of the stage with the human's answers\n\n");
    let stages = stages_of_kind(kind);
    let mut any = false;
    for q in p.list_records(schema::kind_by_name("question"))? {
        if stages.contains(&s(&q, "stage")) {
            any = true;
            let ans = if q.get("answer").is_some() {
                crate::store::answer_text(&q)
            } else {
                "(not answered)".to_string()
            };
            md.push_str(&format!("- {} {}: {}\n", s(&q, "id"), s(&q, "title"), ans));
        }
    }
    if !any {
        md.push_str("None.\n");
    }
    // The open and the earlier findings of the kind.
    let mine = |f: &Value| -> bool {
        match subject {
            Some(a) => {
                s(f, "amendment") == a
                    && (matches!(kind, "amendment" | "agreement")
                        || s(f, "asks") == kind
                        || s(f, "gate") == kind)
            }
            None => s(f, "gate") == kind,
        }
    };
    let findings: Vec<Value> = p
        .list_records(schema::kind_by_name("finding"))?
        .into_iter()
        .filter(mine)
        .collect();
    for (title, open) in [("The open findings", true), ("The earlier findings", false)] {
        md.push_str(&format!("\n## {title}\n\n"));
        let mut any = false;
        for f in findings.iter().filter(|f| is_open_finding(f) == open) {
            any = true;
            md.push_str(&format!(
                "- {} [{}] {} ({})\n",
                s(f, "id"),
                s(f, "severity"),
                s(f, "title"),
                if s(f, "outcome").is_empty() {
                    "open"
                } else {
                    s(f, "outcome")
                }
            ));
        }
        if !any {
            md.push_str("None.\n");
        }
    }
    md.push_str("\n## You may\n\nRead only what this brief names or links to.\n\nWrite:\n");
    for x in r.writes {
        md.push_str(&format!("- {x}\n"));
    }
    md.push_str("\nRun:\n");
    for x in r.runs {
        md.push_str(&format!("- `{x}`\n"));
    }
    md.push_str("\n## Your rules\n\n");
    md.push_str(&kind_rule(kind));
    if subject.is_some() {
        if kind == "amendment" {
            md.push_str(&format!("\nAsking for a stage review: {}\n", asking_rule()));
        } else {
            md.push_str("\nYou are a stage reviewer: you cannot request further reviews.\n");
        }
    }
    md.push_str(&design_only_section(true));
    md.push('\n');
    md.push_str(&limits_and_tier(p));
    md.push_str(
        "\n## Rules\n\n- File your findings with `bf record put finding`.\n- You decide nothing for the human and approve nothing; the human accepts a finding only in their own words.\n- Never edit anything under the factory or engine folders, except your own findings.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot go on without the human's decision, ask: `bf ask --title \"<the question>\"`.\n"
    );
    let path = p
        .briefs_dir()
        .join(format!("{subj}-design-reviewer-{kind}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// `bf review brief <stage> --subject A-n --role <role>`: the brief of a stage agent.
pub fn brief_stage(p: &Project, stage: &str, amend: &str, role: Option<&str>) -> R<PathBuf> {
    subject_id(amend)?;
    let a = open_amend(p, amend)?;
    let role = match role.or_else(|| stage_roles(stage).first().copied()) {
        Some(r) if stage_roles(stage).contains(&r) => r,
        _ => {
            return Err(bad_amend(
                &format!("brief the stage `{stage}`"),
                format!(
                    "the stage is one of {}, worked by its own role",
                    STAGES.join(", ")
                ),
                "Give `--role` with the role of the stage.",
            ))
        }
    };
    let r = super::get(role).expect("a stage role is in the table");
    let split = splits(p, amend)?;
    let ids: Vec<String> = if is_reviewing_role(role) {
        vec![]
    } else {
        match split.get(stage) {
            Some(ids) => ids.clone(),
            None => {
                return Err(bad_amend(
                    &format!("brief {role} for the stage `{stage}` of {amend}"),
                    "the split of the amendment did not name that stage".into(),
                    "The general designer splits first: `bf review split <A-n> --stage <stage>=<ID,ID> --token <token>`.",
                ))
            }
        }
    };
    let mut md = format!("# Brief: {amend} as {role}, stage {stage}\n\n");
    md.push_str(&format!(
        "## Your role\n\n{} Stage: {stage}. You work on one stage of the amendment {amend} and no other.\n\n",
        r.summary
    ));
    md.push_str(&format!("## What to do\n\n{}\n\n", what_to_draft(stage)));
    md.push_str("## The records you may write\n\n");
    if ids.is_empty() {
        md.push_str("- none: you judge the stage and write no record\n");
    }
    for id in &ids {
        md.push_str(&format!("- {id}\n"));
    }
    md.push_str(&format!(
        "\nYou may put only these IDs; the engine refuses any other. The amendment's set is: {}.\n\n## The records now\n\n",
        a.ids.join(", ")
    ));
    for id in ids.iter().chain(if ids.is_empty() {
        a.ids.iter()
    } else {
        [].iter()
    }) {
        match p.get_record(id)? {
            Some(rec) => md.push_str(&format!(
                "### {id}\n\n```\n{}\n```\n\n",
                serde_json::to_string_pretty(&rec).unwrap_or_default()
            )),
            None => md.push_str(&format!("### {id}\n\nNot written yet.\n\n")),
        }
    }
    md.push_str("## You may\n\nRead:\n");
    for x in r.reads {
        md.push_str(&format!("- {x}\n"));
    }
    md.push_str("\nWrite:\n");
    if r.writes.is_empty() || is_reviewing_role(role) {
        md.push_str("- nothing\n");
    } else {
        for x in r.writes {
            md.push_str(&format!("- {x}\n"));
        }
    }
    md.push_str("\nRun:\n");
    for x in r.runs {
        md.push_str(&format!("- `{x}`\n"));
    }
    md.push_str(&design_only_section(is_reviewing_role(role)));
    md.push('\n');
    md.push_str(&limits_and_tier(p));
    md.push_str(
        "\n## Rules\n\n- Write your records with `bf record put`.\n- You cannot split the amendment or ask for further agents or reviews.\n- Write no record outside the IDs above.\n- Never edit anything under the factory or engine folders.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot go on without the human's decision, ask: `bf ask --title \"<the question>\"`.\n"
    );
    let path = p.briefs_dir().join(format!("{amend}-{role}-{stage}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// The brief's section that tells a designer or a design reviewer to design and judge only (D-111).
pub fn design_only_section(reviewer: bool) -> String {
    let key = if reviewer {
        "guard_design.brief.reviewer"
    } else {
        "guard_design.brief.designer"
    };
    format!(
        "\n## {}\n\n{}\n",
        crate::strings::t("guard_design.brief.title", &[]),
        crate::strings::t(key, &[]).trim()
    )
}

/// Whether `name` is a stage of a split that no kind of design review carries (`ui`, `threats`).
pub fn stage_only(name: &str) -> bool {
    STAGES.contains(&name) && review_kind(name, true).is_err()
}
