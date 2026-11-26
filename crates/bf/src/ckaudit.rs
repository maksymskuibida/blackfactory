//! The audits of a checkpoint (R-77, D-159). An agent check of a checkpoint that judges the built result is
//! one more source of the checkpoint's review rounds: its auditor reports like a reviewer, each entry
//! becomes the finding `r<round>.<number>`, and the engine writes the check's receipt from a round with no
//! blocking finding open. The rounds themselves live in `ckreview`; this module holds what only an audit
//! needs: which checks are audits, the auditor's brief and report, and the receipt after the human's words.

use crate::checks::{self, Family, Stage};
use crate::ckreview::{self, Round};
use crate::events::{self, State};
use crate::evidence;
use crate::gitx;
use crate::plan::{self, Plan};
use crate::refusal::{refuse_owned, R};
use crate::run;
use crate::store::{s, str_list, Project};
use crate::strings::t;
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// The named check of a check record, empty when it has none.
pub(crate) fn named_of<'a>(pl: &'a Plan, check: &str) -> &'a str {
    pl.checks
        .iter()
        .find(|c| s(c, "id") == check)
        .map(|c| s(c, "named"))
        .unwrap_or("")
}

/// The audit sources of `k`: the agent checks it lists that judge the built result (R-77).
pub fn sources(pl: &Plan, k: &str) -> Vec<String> {
    let Some(kr) = pl.checkpoint(k) else {
        return vec![];
    };
    str_list(kr, "checks")
        .into_iter()
        .filter(|id| is_source(pl, k, id))
        .collect()
}

/// Whether `check` is an audit source of `k`.
pub fn is_source(pl: &Plan, k: &str, check: &str) -> bool {
    let Some(kr) = pl.checkpoint(k) else {
        return false;
    };
    if !str_list(kr, "checks").iter().any(|c| c == check) {
        return false;
    }
    let Some(c) = pl.checks.iter().find(|c| s(c, "id") == check) else {
        return false;
    };
    if s(c, "how") != "agent" {
        return false;
    }
    match s(c, "named") {
        "" => true,
        name => checks::by_name(name)
            .map(|x| x.stage == Stage::Built && x.family != Family::Review)
            .unwrap_or(false),
    }
}

/// The role that audits `check`.
pub fn role_of(pl: &Plan, check: &str) -> &'static str {
    match named_of(pl, check) {
        "security-audit" => "security-auditor",
        "legal-audit" => "legal-auditor",
        "built-uiux-verification" => "uiux-verifier",
        _ => "verifier",
    }
}

/// The audit checks of `k` whose auditor is `role`, joined for a refusal; `<X>` when the plan names none.
pub fn checks_of_role(p: &Project, k: &str, role: &str) -> String {
    let found: Vec<String> = plan::load(p)
        .map(|pl| {
            sources(&pl, k)
                .into_iter()
                .filter(|c| role_of(&pl, c) == role)
                .collect()
        })
        .unwrap_or_default();
    if found.is_empty() {
        "<X>".to_string()
    } else {
        found.join(" or ")
    }
}

fn is_uiux(pl: &Plan, check: &str) -> bool {
    named_of(pl, check) == "built-uiux-verification"
}

pub fn token_key(k: &str, check: &str) -> String {
    format!("{k}:audit:{check}")
}

fn refuse_with(what: String, why: String, step: String, check: &str) -> crate::refusal::Fail {
    refuse_owned(what, why, vec![step], t(check, &[]))
}

/// The step names a report may use for a step of the coverage: `<journey> step <n>`, `<journey>:<n>`.
fn failed_steps(
    p: &Project,
    pl: &Plan,
    k: &str,
    check: &str,
    content: &str,
) -> R<(Vec<String>, bool)> {
    let mut q = p.clone();
    q.top = p.work_dir().join(k);
    if q.content_hash(&q.top)? != content {
        return Ok((vec![], true));
    }
    let cov = crate::uiux::coverage_of(&q, check, named_of(pl, check))?;
    let mut failed = Vec::new();
    let mut missing = false;
    for r in &cov.rows {
        match s(r, "status") {
            "met" => {}
            "failed" => failed.push(format!(
                "{} step {}",
                s(r, "journey"),
                r["step"].as_u64().unwrap_or(0)
            )),
            _ => missing = true,
        }
    }
    for sc in &cov.screens {
        for st in sc["states"].as_array().into_iter().flatten() {
            match s(st, "status") {
                "met" => {}
                "failed" => failed.push(format!("{} state {}", s(sc, "screen"), s(st, "state"))),
                _ => missing = true,
            }
        }
    }
    Ok((failed, missing))
}

fn names_step(f: &Value, step: &str) -> bool {
    let norm = |x: &str| {
        x.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let given = norm(s(f, "step"));
    if given.is_empty() {
        return false;
    }
    let want = norm(step);
    let colon = want.replacen(" step ", ":", 1);
    let dot = want.replacen(" step ", ".", 1);
    given == want || given == colon || given == dot
}

/// `bf brief <K> --role <role> --check <X>`: opens the audit round and writes the auditor's brief.
pub fn brief(p: &Project, k: &str, role: &str, check: &str) -> R<PathBuf> {
    let what = t("ck_audit.what.brief", &[("k", k), ("check", check)]);
    let _lock = crate::lock::engine(p, &what)?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let pl = plan::load(p)?;
    if !is_source(&pl, k, check) {
        return Err(refuse_with(
            what,
            t(
                "ck_audit.refuse.no_source.why",
                &[("k", k), ("check", check)],
            ),
            t("ck_audit.refuse.no_source.step", &[("k", k)]),
            "ck_audit.refuse.check",
        ));
    }
    let want = role_of(&pl, check);
    if role != want {
        return Err(refuse_with(
            what,
            t(
                "ck_audit.refuse.role.why",
                &[("k", k), ("check", check), ("want", want), ("role", role)],
            ),
            t(
                "ck_audit.refuse.role.step",
                &[("k", k), ("check", check), ("want", want)],
            ),
            "ck_audit.refuse.check",
        ));
    }
    let kr = pl.checkpoint(k).cloned().unwrap_or_default();
    let landed = {
        let after = str_list(&kr, "after");
        !after.is_empty() && after.iter().all(|u| st.unit_state(u) == "landed")
    };
    if st.closed.contains_key(k) || !landed {
        return Err(refuse_with(
            what,
            t("ck_audit.refuse.closed.why", &[("k", k)]),
            t("ck_audit.refuse.closed.step", &[]),
            "ck_audit.refuse.check",
        ));
    }
    if st.sync_open.as_ref().is_some_and(|o| o.checkpoint == k)
        || st.ck_fix.as_ref().is_some_and(|f| f.checkpoint == k)
    {
        return Err(refuse_with(
            what,
            t("ck_audit.refuse.busy.why", &[("k", k)]),
            t("ck_audit.refuse.busy.step", &[]),
            "ck_audit.refuse.check",
        ));
    }
    let content = ckreview::cp_content(p, k)?;
    let open_here =
        ckreview::open_round(&ckreview::rounds(p, k)?, check).is_some_and(|o| o.content == content);
    if !open_here && ckreview::required(&st) && !ckreview::review_passed_at(p, &st, k, &content)? {
        return Err(refuse_with(
            what,
            t("ck_audit.refuse.review.why", &[("k", k)]),
            t("ck_audit.refuse.review.step", &[("k", k)]),
            "ck_audit.refuse.check",
        ));
    }
    let mut rs = ckreview::rounds(p, k)?;
    // A round of this audit still open on other content is void: the auditor judged what is no longer there.
    if let Some(o) = ckreview::open_round(&rs, check)
        .filter(|o| o.content != content)
        .cloned()
    {
        events::append(
            p,
            "checkpoint.review.done",
            json!({"checkpoint": k, "source": check, "round": o.round, "content": o.content,
                   "result": "void", "findings": []}),
        )?;
        rs = ckreview::rounds(p, k)?;
    }
    let earlier = rs
        .iter()
        .rev()
        .find(|r| r.source == check && ckreview::reported(r))
        .cloned();
    let n = match ckreview::open_round(&rs, check) {
        Some(o) => o.round,
        None => {
            let n = rs.iter().map(|r| r.round).max().unwrap_or(0) + 1;
            let covered = match &earlier {
                None => t("ck_audit.covered.cold", &[]),
                Some(e) => t(
                    "ck_audit.covered.repeat",
                    &[("round", &e.round.to_string())],
                ),
            };
            events::append(
                p,
                "checkpoint.review.open",
                json!({"checkpoint": k, "source": check, "round": n, "content": content,
                       "heads": ckreview::heads_json(&ckreview::heads(p, &pl, k)?), "covered": covered}),
            )?;
            n
        }
    };
    rs = ckreview::rounds(p, k)?;
    fs::create_dir_all(p.briefs_dir())?;

    // The whole diff of the checkpoint: against the checkpoint before it, or the main branch.
    let prev = run::previous_checkpoint(&pl, k).filter(|c| st.closed.contains_key(c));
    let mut diff = String::new();
    for r in run::checkpoint_repos(p, &pl, k)? {
        let wt = run::checkpoint_wt(p, &pl, k, r)?;
        let base = match &prev {
            Some(c) if gitx::branch_exists(&r.path, &p.checkpoint_branch(c)) => {
                p.checkpoint_branch(c)
            }
            _ => r.branch.clone(),
        };
        diff.push_str(&format!(
            "# repository {}\n{}\n",
            r.name,
            gitx::must(&wt, &["diff", &base, "HEAD"]).unwrap_or_default()
        ));
    }
    let diff_path = p.briefs_dir().join(format!("{k}-audit-{check}.diff"));
    util::atomic_write(&diff_path, diff.as_bytes())?;

    let record = pl
        .checks
        .iter()
        .find(|c| s(c, "id") == check)
        .cloned()
        .unwrap_or_default();
    let covered = rs
        .iter()
        .find(|r| r.round == n)
        .map(|r| r.covered.clone())
        .unwrap_or_default();
    let mut md = format!(
        "{}\n\n{}\n\n{}\n\n",
        t(
            "ck_audit.brief.title",
            &[
                ("k", k),
                ("role", role),
                ("check", check),
                ("n", &n.to_string())
            ]
        ),
        t("ck_audit.brief.checkpoint", &[("title", s(&kr, "title"))]),
        t(
            "ck_audit.brief.intro",
            &[
                ("check", check),
                ("check_title", s(&record, "title")),
                ("n", &n.to_string()),
                ("covered", &covered),
            ]
        )
    );
    md.push_str(&t(
        "ck_audit.brief.diff",
        &[
            ("path", &diff_path.to_string_lossy()),
            ("wc", &p.work_dir().join(k).to_string_lossy()),
        ],
    ));
    md.push_str("\n\n");
    md.push_str(&t(
        "ck_audit.brief.check",
        &[
            ("id", check),
            ("title", s(&record, "title")),
            ("proves", &str_list(&record, "proves").join(", ")),
        ],
    ));
    md.push_str("\n\n");
    md.push_str(&t("ck_audit.brief.proves", &[]));
    md.push_str("\n\n");
    let mut ids = str_list(&record, "proves");
    for id in str_list(&kr, "proves") {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    for id in ids {
        if let Some(r) = p.get_record(&id)? {
            md.push_str(&format!("- {} {}\n", id, s(&r, "title")));
            for a in str_list(&r, "acceptance") {
                md.push_str(&t("ck_audit.brief.acceptance", &[("text", &a)]));
                md.push('\n');
            }
        }
    }
    md.push('\n');
    md.push_str(&t("ck_audit.brief.units", &[]));
    md.push_str("\n\n");
    for u in pl.units.iter().filter(|u| s(u, "checkpoint") == k) {
        let id = s(u, "id");
        md.push_str(&format!(
            "<details><summary>{id}</summary>\n\n{}\n</details>\n\n",
            ckreview::read_unit_brief(p, id)
        ));
    }
    if let Some(e) = &earlier {
        md.push_str(&t(
            "ck_audit.brief.earlier",
            &[
                ("round", &e.round.to_string()),
                ("content", &e.content[..12.min(e.content.len())]),
            ],
        ));
        md.push_str("\n\n");
        md.push_str(&ckreview::report_lines(e));
        if e.findings.is_empty() {
            md.push_str(&t("ck_audit.brief.earlier_none", &[]));
            md.push('\n');
        }
        for (i, f) in e.findings.iter().enumerate() {
            let place = if s(f, "where").is_empty() {
                s(f, "file")
            } else {
                s(f, "where")
            };
            md.push_str(&t(
                "ck_audit.brief.earlier_finding",
                &[
                    ("round", &e.round.to_string()),
                    ("number", &(i + 1).to_string()),
                    ("severity", s(f, "severity")),
                    ("title", s(f, "title")),
                    ("where", place),
                    ("action", s(f, "action")),
                    ("outcome", &ckreview::fate_text(&st, k, e.round, i, f)),
                ],
            ));
            md.push('\n');
        }
        if !e.heads.is_empty() {
            let mut since = String::new();
            for r in run::checkpoint_repos(p, &pl, k)? {
                let wt = run::checkpoint_wt(p, &pl, k, r)?;
                if let Some((_, old)) = e.heads.iter().find(|(n, _)| *n == r.name) {
                    since.push_str(&format!(
                        "# repository {}\n{}\n",
                        r.name,
                        gitx::must(&wt, &["diff", old, "HEAD"]).unwrap_or_default()
                    ));
                }
            }
            let since_path = p.briefs_dir().join(format!("{k}-audit-{check}-since.diff"));
            util::atomic_write(&since_path, since.as_bytes())?;
            md.push('\n');
            md.push_str(&t(
                "ck_audit.brief.since",
                &[
                    ("round", &e.round.to_string()),
                    ("path", &since_path.to_string_lossy()),
                ],
            ));
            md.push('\n');
        }
        md.push('\n');
    }
    md.push_str(&crate::ckconfirm::brief_section(
        p, &st, &pl, &rs, k, check,
    )?);
    let command = format!(
        "bf report {k} --role {role} --check {check} --token <token> --result findings --findings -"
    );
    let step_field = if is_uiux(&pl, check) {
        t("ck_audit.brief.step_field", &[])
    } else {
        String::new()
    };
    md.push_str(&t(
        "ck_audit.brief.report",
        &[
            ("command", &command),
            ("step_field", &step_field),
            ("n", &n.to_string()),
        ],
    ));
    md.push_str("\n\n");
    md.push_str(&t("ck_audit.brief.rules", &[]));
    md.push('\n');
    let path = p.briefs_dir().join(format!("{k}-audit-{check}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    p.write_guard()?;
    Ok(path)
}

fn needs_lawyer(f: &Value) -> bool {
    f.get("lawyer_check").and_then(|v| v.as_bool()) == Some(true)
}

/// An entry that needs a lawyer belongs to a legal audit and names its market (D-164).
fn check_lawyer_entries(pl: &Plan, k: &str, check: &str, list: &[Value]) -> R<()> {
    for (i, f) in list.iter().enumerate() {
        if !needs_lawyer(f) {
            continue;
        }
        let n = (i + 1).to_string();
        let vals = [("k", k), ("check", check), ("number", n.as_str())];
        if named_of(pl, check) != "legal-audit" {
            return Err(crate::refusal::from_key(
                "ck_audit_records.refuse.lawyer_source",
                &vals,
            ));
        }
        if s(f, "market").trim().is_empty() {
            return Err(crate::refusal::from_key(
                "ck_audit_records.refuse.lawyer_market",
                &vals,
            ));
        }
    }
    Ok(())
}

/// Files each entry that needs a lawyer as a marked legal record and stores its id on the entry.
/// An entry with the same title, place and market that an earlier round of this source filed keeps its legal record,
/// so a repeat round files it again under the same id (a changed one is asked again, FU-15).
fn earlier_legal_record(
    p: &Project,
    rs: &[ckreview::Round],
    check: &str,
    f: &Value,
) -> Option<String> {
    let market = s(f, "market").trim().to_string();
    rs.iter()
        .rev()
        .filter(|r| r.source == check)
        .flat_map(|r| r.findings.iter().rev())
        .filter(|o| {
            s(o, "title") == s(f, "title")
                && s(o, "where") == s(f, "where")
                && s(o, "market").trim() == market
        })
        .filter_map(|o| o.get("legal_record").and_then(|v| v.as_str()))
        .find(|id| {
            p.get_record(id)
                .ok()
                .flatten()
                .is_some_and(|r| s(&r, "outcome") != "accepted")
        })
        .map(|id| id.to_string())
}

fn file_lawyer_entries(
    p: &Project,
    rs: &[ckreview::Round],
    k: &str,
    check: &str,
    list: Vec<Value>,
) -> R<Vec<Value>> {
    let mut out = Vec::new();
    for (i, mut f) in list.into_iter().enumerate() {
        if needs_lawyer(&f) {
            let mut legal = json!({
                "title": s(&f, "title"), "markets": [s(&f, "market").trim()],
                "risk": s(&f, "title"), "researched": false, "lawyer_check": true,
            });
            for key in ["text", "severity", "where", "action"] {
                if !s(&f, key).is_empty() {
                    legal[key] = json!(s(&f, key));
                }
            }
            if let Some(id) = earlier_legal_record(p, rs, check, &f) {
                legal["id"] = json!(id);
            }
            let filed = crate::legal::file_finding(p, &legal).map_err(|e| {
                let n = (i + 1).to_string();
                crate::refusal::from_key(
                    "ck_audit_records.refuse.lawyer_filed",
                    &[
                        ("k", k),
                        ("check", check),
                        ("number", &n),
                        ("why", &format!("{e}")),
                    ],
                )
            })?;
            f["legal_record"] = filed["id"].clone();
        }
        out.push(f);
    }
    Ok(out)
}

/// `bf report <K> --role <role> --check <X> --token <token> --result findings --findings -`.
#[allow(clippy::too_many_arguments)]
pub fn report(
    p: &Project,
    k: &str,
    role: &str,
    check: &str,
    result: &str,
    note: &str,
    findings: Option<&Value>,
    token: Option<&str>,
) -> R<Value> {
    let what = t("ck_audit.what.report", &[("k", k), ("check", check)]);
    let sub = [("k", k), ("check", check), ("role", role)];
    if result != "findings" && result != "built" {
        return Err(refuse_with(
            what,
            t("ck_audit.refuse.result.why", &[]),
            t("ck_audit.refuse.result.step", &sub),
            "ck_audit.refuse.check",
        ));
    }
    let key = token_key(k, check);
    if p.policy.roles.tokens && !run::holds_token(p, &key, token) {
        return Err(refuse_with(
            what,
            t("ck_audit.refuse.token.why", &[("check", check)]),
            t("ck_audit.refuse.token.step", &sub),
            "ck_audit.refuse.check",
        ));
    }
    let list = ckreview::finding_list(findings);
    let raw_areas = ckreview::object_list(findings, "areas");
    let raw_probes = ckreview::object_list(findings, "probes");
    let _lock = crate::lock::engine(p, &what)?;
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let pl = plan::load(p)?;
    if !is_source(&pl, k, check) || role_of(&pl, check) != role {
        return Err(refuse_with(
            what,
            t(
                "ck_audit.refuse.no_source.why",
                &[("k", k), ("check", check)],
            ),
            t("ck_audit.refuse.no_source.step", &[("k", k)]),
            "ck_audit.refuse.check",
        ));
    }
    let rs = ckreview::rounds(p, k)?;
    let Some(round) = ckreview::open_round(&rs, check).cloned() else {
        return Err(refuse_with(
            what,
            t("ck_audit.refuse.no_round.why", &sub),
            t("ck_audit.refuse.no_round.step", &sub),
            "ck_audit.refuse.check",
        ));
    };
    let content = ckreview::cp_content(p, k)?;
    if content != round.content {
        return Err(refuse_with(
            what,
            t("ck_audit.refuse.content.why", &sub),
            t("ck_audit.refuse.content.step", &sub),
            "ck_audit.refuse.check",
        ));
    }
    let areas = ckreview::check_areas(k, raw_areas)?;
    ckreview::check_findings(k, &list)?;
    check_lawyer_entries(&pl, k, check, &list)?;
    let said = crate::ckconfirm::check_report(&st, &rs, k, check, findings)?;
    let (confirmed, still_open) = said.stored();
    let reopened = crate::ckconfirm::reopens_blocking(&rs, &said);
    let label = format!("{k}-audit-{check}");
    let probes = ckreview::store_probes_in(
        p,
        k,
        round.round,
        &raw_probes,
        &list,
        &p.work_dir().join(k),
        &label,
    )?;
    let list = ckreview::prepare_findings(p, &st, k, &list, &probes)?;
    let uiux = is_uiux(&pl, check);
    let mut covered_ok = true;
    if uiux {
        let (failed, missing) = failed_steps(p, &pl, k, check, &content)?;
        if let Some(step) = failed
            .iter()
            .find(|x| !list.iter().any(|f| names_step(f, x)))
        {
            return Err(refuse_with(
                what,
                t("ck_audit.refuse.step.why", &[("step", step)]),
                t("ck_audit.refuse.step.step", &[("step", step)]),
                "ck_audit.refuse.check",
            ));
        }
        covered_ok = failed.is_empty() && !missing;
    }
    let list = file_lawyer_entries(p, &rs, k, check, list)?;
    let held_back = list.iter().any(ckreview::blocking) || reopened;
    let result_word = if list.is_empty() && said.still_open.is_empty() {
        "clean"
    } else {
        "findings"
    };
    let mut done = json!({"checkpoint": k, "source": check, "round": round.round, "content": content,
        "result": result_word,
        "findings": list, "areas": areas, "probes": probes, "note": note,
        "confirmed": confirmed, "still_open": still_open});
    let mut receipt = false;
    if held_back {
        if ckreview::reviews(&st, k, &rs) >= p.policy.limits.checkpoint_reviews {
            done["parked"] = json!(true);
        }
    } else if covered_ok {
        receipt = true;
    }
    let parked = done["parked"].as_bool().unwrap_or(false);
    events::append(p, "checkpoint.review.done", done)?;
    if receipt {
        let r = round.round.to_string();
        let says = if list.is_empty() {
            t(
                "ck_audit.receipt.clean",
                &[("check", check), ("k", k), ("round", &r)],
            )
        } else {
            t(
                "ck_audit.receipt.low",
                &[
                    ("check", check),
                    ("k", k),
                    ("round", &r),
                    ("count", &list.len().to_string()),
                ],
            )
        };
        evidence::review_receipt(p, check, &content, &says)?;
    }
    if p.policy.roles.tokens {
        run::spend_token(p, &key)?;
    }
    p.write_guard()?;
    let mut out = json!({"checkpoint": k, "source": check, "round": round.round,
        "result": result_word,
        "findings": list.len(), "receipt": receipt});
    if parked {
        out["parked"] = json!(true);
        out["note"] = json!(t("ck_audit.parked.note", &[]));
    }
    Ok(out)
}

/// Whether the human has ended each blocking finding of `r`.
fn ended_by_human(st: &State, k: &str, r: &Round) -> bool {
    let blocking: Vec<(usize, &Value)> = r
        .findings
        .iter()
        .enumerate()
        .filter(|(_, f)| ckreview::blocking(f))
        .collect();
    !blocking.is_empty()
        && blocking.iter().all(|(i, f)| {
            matches!(
                ckreview::fate(st, k, r.round, *i, f)
                    .map(|x| x.0)
                    .as_deref(),
                Some("accepted") | Some("declined")
            )
        })
}

/// When the human has ended each blocking finding of an audit round on the content it judged, the engine
/// writes the check's receipt naming them (R-37 for audits). Called after the human's words are logged.
pub fn receipt_on_decisions(p: &Project, k: &str) -> R<()> {
    let pl = plan::load(p)?;
    let srcs = sources(&pl, k);
    if srcs.is_empty() {
        return Ok(());
    }
    let st = events::require_chain(p)?;
    let rs = ckreview::rounds(p, k)?;
    for check in srcs {
        let Some(r) = rs
            .iter()
            .rev()
            .find(|r| r.source == check && ckreview::reported(r))
        else {
            continue;
        };
        if !ended_by_human(&st, k, r) {
            continue;
        }
        let receipts = evidence::load_receipts(p)?;
        if evidence::satisfied_at(&receipts, &check, &r.content).is_some() {
            continue;
        }
        if is_uiux(&pl, &check) {
            let (failed, missing) = failed_steps(p, &pl, k, &check, &r.content)?;
            if !failed.is_empty() || missing {
                continue;
            }
        }
        let ids: Vec<String> = r
            .findings
            .iter()
            .enumerate()
            .filter(|(_, f)| ckreview::blocking(f))
            .map(|(i, _)| format!("r{}.{}", r.round, i + 1))
            .collect();
        let says = t(
            "ck_audit.receipt.decided",
            &[
                ("check", &check),
                ("k", k),
                ("round", &r.round.to_string()),
                ("ids", &ids.join(", ")),
            ],
        );
        evidence::review_receipt(p, &check, &r.content, &says)?;
    }
    Ok(())
}

/// Whether `check` is an audit source of some checkpoint of the plan (the refusals of a mark by a worker).
pub fn is_source_anywhere(pl: &Plan, check: &str) -> bool {
    pl.checkpoints
        .iter()
        .any(|k| is_source(pl, s(k, "id"), check))
}

/// The audits of `k` due at `content`: sources the human has not switched off, with no passing receipt and no
/// accepted check at that content and no round that reported at it. Empty while the checkpoint review
/// still has to pass, since an audit follows it (D-159).
pub fn due(
    p: &Project,
    st: &State,
    pl: &Plan,
    k: &str,
    content: &str,
    receipts: &[evidence::Receipt],
) -> R<Vec<String>> {
    if ckreview::needed(p, st, k, content)? {
        return Ok(vec![]);
    }
    let rs = ckreview::rounds(p, k)?;
    Ok(sources(pl, k)
        .into_iter()
        .filter(|c| {
            !pl.checks
                .iter()
                .any(|r| s(r, "id") == c && crate::enforce::is_off(st, r))
        })
        .filter(|c| evidence::satisfied_at(receipts, c, content).is_none())
        .filter(|c| evidence::accepted_at(st, k, c, content).is_none())
        .filter(|c| {
            !rs.iter()
                .any(|r| &r.source == c && r.content == content && ckreview::reported(r))
        })
        .collect())
}

/// Whether an audit round of `k` is open.
pub fn round_open(p: &Project, k: &str) -> R<bool> {
    Ok(ckreview::rounds(p, k)?
        .iter()
        .any(|r| !r.source.is_empty() && r.result.is_none()))
}

/// The directive `audit-checkpoint` for one due audit of `k`.
pub fn directive(p: &Project, st: &State, pl: &Plan, k: &str, check: &str) -> Value {
    let role = role_of(pl, check);
    let kind = crate::roles::get(role).map_or("reviewer", |r| r.tier);
    json!({"role": role, "action": "audit-checkpoint", "checkpoint": k, "check": check,
        "tier": p.policy.tiers.get(kind),
        "working_copy": p.work_dir().join(k).to_string_lossy(),
        "brief": p.briefs_dir().join(format!("{k}-audit-{check}.md")).to_string_lossy(),
        "limits": {"max_rounds": p.policy.limits.max_rounds, "idle_minutes": p.policy.limits.idle_minutes,
                   "budget_left": if p.policy.budget.cap > 0.0 { json!(p.policy.budget.cap - st.cost) } else { Value::Null }},
        "commands": [format!("bf brief {k} --role {role} --check {check}")]})
}

/// The open critical, high and medium findings of the audit rounds of `k` that judged `content`, with their
/// round and number, for the one fix that follows the audits.
pub fn blocking_to_fix(
    p: &Project,
    st: &State,
    k: &str,
    content: &str,
) -> R<(Vec<Value>, Vec<(u32, u32)>)> {
    let mut findings = Vec::new();
    let mut ids = Vec::new();
    for r in ckreview::rounds(p, k)?
        .iter()
        .filter(|r| !r.source.is_empty() && ckreview::reported(r) && r.content == content)
    {
        for (n, f) in ckreview::open_findings(st, k, r) {
            if ckreview::blocking(f) {
                findings.push(f.clone());
                ids.push((r.round, n as u32));
            }
        }
    }
    Ok((findings, ids))
}
