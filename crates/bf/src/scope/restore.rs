//! U-103 (A-73): `bf unit restore <U> <path>... --token <T>`. The builder puts a file it changed by
//! mistake outside its scope back to its content at the landing diff's base, or removes it when the
//! base has no such file. The event changes no state, round or attempt.

use crate::events;
use crate::refusal::{refuse_owned, R};
use crate::store::{s, Project};
use crate::{plan, run, scope, util};
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};

fn no(what: String, because: String, steps: Vec<String>) -> crate::refusal::Fail {
    refuse_owned(
        what,
        because,
        steps,
        "`git status` in the working copy shows the file as the base has it.",
    )
}

/// Raw git output (a byte compare cannot use the trimmed text of the shared helper).
fn git(prog: &str, dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut c = Command::new(prog);
    c.arg("-C").arg(dir).args(args).stdin(Stdio::null());
    for (k, _) in std::env::vars_os() {
        if k.to_str().is_some_and(|k| k.starts_with("GIT_")) {
            c.env_remove(k);
        }
    }
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    c.output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
}

/// What the base holds at a path: None when nothing, else (mode, blob id, content).
type Held = Option<(String, String, Vec<u8>)>;

fn held(g: &str, repo: &Path, base: &str, path: &str) -> Result<Held, String> {
    let Some(t) = git(g, repo, &["ls-tree", base, "--", path]) else {
        return Err("the base cannot be read".into());
    };
    let t = String::from_utf8_lossy(&t).to_string();
    let Some(line) = t.lines().next() else {
        return Ok(None);
    };
    let (meta, name) = line.split_once('\t').unwrap_or((line, ""));
    let m: Vec<&str> = meta.split(' ').collect();
    if name != path || m.len() < 3 {
        return Err("the base holds a folder under that name".into());
    }
    if m[1] != "blob" || m[0] == "120000" {
        return Err("the base holds a link or a submodule under that name".into());
    }
    let Some(bytes) = git(g, repo, &["cat-file", "blob", m[2]]) else {
        return Err("the base's content cannot be read".into());
    };
    Ok(Some((m[0].to_string(), m[2].to_string(), bytes)))
}

fn write_fail(path: &str, e: std::io::Error) -> crate::refusal::Fail {
    no(
        format!("put {path} back to the base"),
        format!("it could not be written: {e}"),
        vec!["Check the working copy with `git status`, then run the command again.".into()],
    )
}

pub fn restore(p: &Project, unit: &str, paths: &[String], token: Option<&str>) -> R<Value> {
    let what = format!("put files of {unit} back to the base");
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
    if !matches!(cur.as_str(), "running" | "fixing") || paths.is_empty() {
        let why = format!("{unit} is `{cur}`, or no file is named; the command takes a builder at work (running or fixing) and the files");
        return Err(no(what, why, vec!["Run `bf unit restore <U> <path>... --token <token>` from the working copy while the unit is running or fixing.".into()]));
    }
    if p.policy.roles.tokens {
        // Checked, not spent: the builder goes on in the same turn.
        run::check_token(p, unit, "builder", token)?;
    }
    let repo = p.repo_of_unit(u)?.clone();
    let wc = p.work_dir().join(unit);
    let g = if p.policy.exec.git.is_empty() {
        "git".to_string()
    } else {
        p.policy.exec.git.clone()
    };
    let branches = [p.checkpoint_branch(s(u, "checkpoint")), p.unit_branch(unit)];
    let base = git(&g, &repo.path, &["merge-base", &branches[0], &branches[1]])
        .map(|o| String::from_utf8_lossy(&o).trim().to_string())
        .filter(|b| !b.is_empty())
        .ok_or_else(|| {
            no(
                what.clone(),
                "the merge base of the unit's branch and its checkpoint branch is not found".into(),
                vec!["Ask the conductor to run `bf unit start` again.".into()],
            )
        })?;
    let files = scope::unit_files(&st, u);
    let (mut todo, mut bad): (Vec<(String, Held)>, Vec<String>) = (vec![], vec![]);
    for raw in paths {
        let Some(path) = scope::normalize_path(raw) else {
            bad.push(format!("{raw}: not a path inside the repository"));
            continue;
        };
        let lower = path.to_lowercase();
        let side = scope::holder_beside(p, &pl, &st, unit, &path);
        // A restore only narrows the change: the rule about a unit beside is not asked.
        let hard = scope::hard_rule(p, &pl, &st, unit, raw).filter(|r| {
            !side
                .as_ref()
                .is_some_and(|v| r.starts_with(&format!("{v} holds it")))
        });
        let r: Result<Held, String> = if files
            .iter()
            .any(|f| util::path_matches(&f.to_lowercase(), &lower))
        {
            Err("it is inside the unit's files: edit it yourself".into())
        } else if let Some(h) = hard {
            Err(h)
        } else if wc.join(&path).is_dir() {
            Err("it is a folder: name each file".into())
        } else {
            held(&g, &repo.path, &base, &path)
        };
        match r {
            Ok(h) => todo.push((path, h)),
            Err(e) => bad.push(format!("{path}: {e}")),
        }
    }
    if !bad.is_empty() {
        return Err(no(
            what,
            format!("{}; no file was written", bad.join("; ")),
            vec![
                "Name only files outside the unit's files and grants, each a single file inside the repository.".into(),
                "For a file the unit needs, ask for it with a scope request.".into(),
            ],
        ));
    }
    let (mut done, mut put, mut gone): (Vec<Value>, Vec<&str>, Vec<&str>) =
        (vec![], vec![], vec![]);
    for (path, h) in &todo {
        let at = wc.join(path);
        if let Some((mode, blob, bytes)) = h {
            util::atomic_write(&at, bytes)
                .and_then(|_| {
                    use std::os::unix::fs::PermissionsExt;
                    let m = if mode == "100755" { 0o755 } else { 0o644 };
                    std::fs::set_permissions(&at, std::fs::Permissions::from_mode(m))
                })
                .map_err(|e| write_fail(path, e))?;
            done.push(json!({"path": path, "base": base, "blob": blob}));
            put.push(path);
        } else {
            if at.symlink_metadata().is_ok() {
                std::fs::remove_file(&at).map_err(|e| write_fail(path, e))?;
            }
            done.push(json!({"path": path, "base": base, "absent": true}));
            gone.push(path);
        }
    }
    events::append(
        p,
        "unit.scope.restore",
        json!({"unit": unit, "files": done}),
    )?;
    let mut parts: Vec<String> = vec![];
    if !put.is_empty() {
        parts.push(format!("{} back to the base", put.join(", ")));
    }
    if !gone.is_empty() {
        parts.push(format!("{} removed (the base has none)", gone.join(", ")));
    }
    Ok(
        json!({"unit": unit, "base": base, "restored": put, "removed": gone, "files": done,
        "line": format!("{unit}: {}", parts.join("; "))}),
    )
}
