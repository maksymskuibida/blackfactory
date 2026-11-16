//! U-104 (A-73): `bf unit format <U> <path>...` accepts a file outside a unit's list as format only
//! when rustfmt, run on the file's content at the landing diff's base with that base's edition and
//! settings, gives exactly the working file; landing checks the two contents again by hash.

use crate::events::{self, State};
use crate::refusal::{refuse_owned, R};
use crate::store::{s, Project};
use crate::{plan, run, scope, util};
use serde_json::{json, Value};
use std::path::Path;

mod exec;
mod git;
mod prove;

use exec::locate_rustfmt;
use git::{at, blob_id, merge_base};
use prove::{prove, Ctx};

pub const MARK_FORMAT: &str = "format only";

fn no(what: String, because: String, steps: Vec<String>) -> crate::refusal::Fail {
    refuse_owned(
        what,
        because,
        steps,
        "`bf status` shows the files in the unit's scope and landing accepts them.",
    )
}

/// `bf unit format <U> <path>...`: run by the conductor, with no token and no quote.
pub fn format(p: &Project, unit: &str, paths: &[String]) -> R<Value> {
    let what = format!("accept files of {unit} as format only");
    let st = events::require_chain(p)?;
    run::need_run(&st)?;
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(no(
            what,
            "no such unit".into(),
            vec!["Use a unit ID from `bf status`.".into()],
        ));
    };
    let cur = st.unit_state(unit).to_string();
    if !matches!(cur.as_str(), "running" | "fixing" | "review") || paths.is_empty() {
        let why = format!("{unit} is `{cur}`, or no file is named; the command takes a unit at work or in review and the files");
        return Err(no(what, why, vec!["Run `bf unit format <U> <path>...` while the unit is running, fixing or in review.".into()]));
    }
    let norm: Vec<String> = paths
        .iter()
        .map(|x| scope::normalize_path(x).unwrap_or_else(|| x.clone()))
        .collect();
    let mut st = st;
    if scope::order::place(p, &pl, &st, unit, &norm)? {
        st = events::replay(p)?;
    }
    let repo = p.repo_of_unit(u)?.clone();
    let wc = p.work_dir().join(unit);
    let g = if p.policy.exec.git.is_empty() {
        "git".to_string()
    } else {
        p.policy.exec.git.clone()
    };
    let base = merge_base(
        &g,
        &repo.path,
        &p.checkpoint_branch(s(u, "checkpoint")),
        &p.unit_branch(unit),
    )
    .ok_or_else(|| {
        no(
            what.clone(),
            "the merge base of the unit's branch and its checkpoint branch is not found".into(),
            vec!["Run `bf unit start` to recreate the working copy.".into()],
        )
    })?;
    let fmt = norm
        .iter()
        .any(|x| x.ends_with(".rs"))
        .then(|| locate_rustfmt(p, &repo.path));
    let c = Ctx {
        p,
        g,
        repo: &repo.path,
        wc: &wc,
        base,
        fmt,
    };
    let files = scope::unit_files(&st, u);
    let (mut ok, mut bad): (Vec<Value>, Vec<String>) = (vec![], vec![]);
    for path in &norm {
        let r = if files.iter().any(|f| crate::util::path_matches(f, path)) {
            Err("inside the unit's files".to_string())
        } else if let Some(h) = scope::hard_rule(p, &pl, &st, unit, path) {
            Err(h)
        } else {
            prove(&c, path)
        };
        match r {
            Ok(v) => ok.push(v),
            Err(e) => bad.push(format!("{path}: {e}")),
        }
    }
    if !bad.is_empty() {
        return Err(no(what, bad.join("; "), vec![
            format!("Put each named file back to its base with `bf unit restore {unit} <path>... --token <token>`, run by the builder."),
            format!("Run `bf unit format {unit} <path>...` again with the files that are formatting only."),
        ]));
    }
    let n = st.scope.get(unit).map_or(0, |sc| sc.grants.len()) + 1;
    let marks: Vec<Value> = ok
        .iter()
        .map(|f| json!({"path": s(f, "path"), "mark": MARK_FORMAT}))
        .collect();
    events::append(
        p,
        "unit.scope.grant",
        json!({"unit": unit, "n": n, "files": ok, "marks": marks, "request": 0, "verdicts": [], "note": "", "cost": 0.0}),
    )?;
    p.write_guard()?;
    let names: Vec<&str> = ok.iter().map(|f| s(f, "path")).collect();
    let mut out = json!({"unit": unit, "grant": n, "files": names, "line": format!("{unit}: {} accepted as format only", names.join(", "))});
    if cur == "review" {
        if let Some(note) = run::write_brief(p, unit, "reviewer") {
            out["brief_note"] = json!(note);
        }
    }
    Ok(out)
}

/// The result for one granted file at landing, when this rule decides it; None leaves the file to
/// the removed-lines rule.
pub fn landing(_st: &State, unit: &str, repo: &Path, range: &str, file: &Value) -> Option<R<()>> {
    let f = file.get("format").filter(|f| f.is_object())?;
    let path = s(file, "path");
    let g = "git";
    let blob = |rev: &str| blob_id(g, repo, rev, path).unwrap_or_default();
    let (a, b) = range.split_once("...").unwrap_or((range, "HEAD"));
    let why = match merge_base(g, repo, a, b) {
        None => Some("the merge base of the branch is not found"),
        Some(mb) if blob(&mb) != s(f, "base_blob") => {
            Some("its content at the base changed since it was accepted as format only")
        }
        _ if at(g, repo, b, path)
            .map(|x| util::sha256_hex(&x))
            .as_deref()
            != Some(s(f, "result_sha256")) =>
        {
            Some("its content differs from the content accepted as format only")
        }
        _ => None,
    }?;
    Some(Err(no(
        format!("land {unit}"),
        format!("{path}, accepted as format only: {why}"),
        vec![
            format!("Put the file back with `bf unit restore {unit} {path} --token <token>`, or have the conductor run `bf unit format {unit} {path}` again."),
            "Commit and run the checks again.".into(),
        ],
    )))
}
