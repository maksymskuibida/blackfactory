//! `bf repo add|list`: the code repositories of a workspace.

use super::{out, project, Args};
use crate::refusal::{refuse, R};
use crate::store::Project;
use crate::{events, gitx, run, util};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

/// `bf repo add <name> <path> [--branch B]` and `bf repo list`: the code repositories of a workspace.
pub(crate) fn cmd_repo(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`add` or `list`")? {
        "add" => {
            if !p.workspace {
                return Err(refuse(
                    "add a repository",
                    "this project is not a workspace: its one repository is the project folder",
                    &["Run `bf init --workspace` in a new folder to make a workspace that points at code repositories."],
                    "`bf repo list` shows the repositories.",
                ));
            }
            let name = args.pos_at(2, "a short name")?;
            let path = args.pos_at(3, "the path of the repository")?;
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(refuse(
                    "add a repository",
                    "the name must be letters, digits, `-` or `_`",
                    &["Pick a short name such as `app` or `api`."],
                    "`bf repo list` shows it.",
                ));
            }
            if p.repos.iter().any(|r| r.name == name) {
                return Err(refuse(
                    "add a repository",
                    format!("the workspace already lists `{name}`"),
                    &["Pick another name, or edit factory/workspace.toml by hand."],
                    "`bf repo list` shows it once.",
                ));
            }
            let given = Path::new(path);
            let joined = if given.is_absolute() {
                given.to_path_buf()
            } else {
                std::env::current_dir()?.join(given)
            };
            let abs = fs::canonicalize(&joined).map_err(|e| {
                refuse(
                    "add a repository",
                    format!("{} cannot be opened: {e}", joined.display()),
                    &["Give the path of an existing git repository."],
                    "`bf repo list` shows it.",
                )
            })?;
            check_repo_path(&p, &abs)?;
            let branch = match args.flag("branch") {
                Some(b) => b.to_string(),
                None => gitx::current_branch(&abs).unwrap_or_else(|| "main".into()),
            };
            if !gitx::branch_exists(&abs, &branch) {
                return Err(refuse(
                    "add a repository",
                    format!(
                        "the branch `{branch}` does not exist or has no commit in {}",
                        abs.display()
                    ),
                    &["Make an initial commit, or name the main branch with `--branch`."],
                    "`bf repo list` shows it.",
                ));
            }
            // Relative to the workspace when it lies beside it, otherwise absolute: the file is tracked.
            let shown = shown_path(&p, &abs);
            let file = p.root.join("factory/workspace.toml");
            let mut text = fs::read_to_string(&file).unwrap_or_default();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&format!(
                "\n[[repo]]\nname = {}\npath = {}\nmain_branch = {}\n",
                toml::Value::String(name.to_string()),
                toml::Value::String(shown),
                toml::Value::String(branch.clone()),
            ));
            util::atomic_write(&file, text.as_bytes())?;
            let mut p2 = Project::discover(&p.root)?;
            let tip = gitx::must(&abs, &["rev-parse", &branch]).ok();
            events::update_head(&p2, |h| {
                let e = h.repos.entry(name.to_string()).or_default();
                e.branch = Some(branch.clone());
                e.commit = tip;
                true
            })?;
            p2.reload_policy()?;
            p2.write_guard()?;
            let added = p2.ensure_excludes()?;
            let v = json!({"name": name, "path": abs.to_string_lossy(), "branch": branch, "excluded": added});
            out(args, &v, || {
                let mut t = format!("added {name}: {}", abs.display());
                if !added.is_empty() {
                    t.push_str(&format!(
                        "\nadded {} to the workspace's local ignore file (.git/info/exclude): nothing of the code repository is committed into the workspace",
                        added.join(", ")
                    ));
                }
                t
            });
        }
        "list" => {
            let st = events::replay(&p)?;
            let pos = run::main_positions(&p, &st)?;
            let v = json!(p
                .repos
                .iter()
                .zip(pos.iter())
                .map(|(r, m)| json!({"name": r.name, "path": r.path.to_string_lossy(), "branch": r.branch, "tip": m.tip, "ok": !m.moved}))
                .collect::<Vec<_>>());
            out(args, &v, || {
                p.repos
                    .iter()
                    .zip(pos.iter())
                    .map(|(r, m)| {
                        format!(
                            "{} {} {} at {}",
                            r.name,
                            r.path.display(),
                            r.branch,
                            m.tip.chars().take(10).collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        _ => {
            return Err(refuse(
                "run `bf repo`",
                "unknown subcommand",
                &["Use `bf repo add <name> <path> [--branch B]` or `bf repo list`."],
                "The command prints the result.",
            ))
        }
    }
    Ok(())
}

/// How a repository path is written in `factory/workspace.toml`: relative to the workspace (inside it, or
/// a sibling), otherwise absolute.
fn shown_path(p: &Project, abs: &Path) -> String {
    if let Ok(rel) = abs.strip_prefix(&p.root) {
        return rel.to_string_lossy().to_string();
    }
    pathdiff(abs, &p.root).unwrap_or_else(|| abs.to_string_lossy().to_string())
}

/// A code repository may be apart from the workspace or a folder inside it; it must be the top of its own
/// git repository, and not the workspace itself or a place the factory keeps its own files.
fn check_repo_path(p: &Project, abs: &Path) -> R<()> {
    let bad = |why: String, step: &str| {
        refuse("add a repository", why, &[step], "`bf repo list` shows it.")
    };
    if abs == p.root || p.root.starts_with(abs) {
        return Err(bad(
            "that is the workspace itself (or a folder above it), not a code repository".into(),
            "Give the path of a code repository: a folder of its own with its own `.git`, beside the workspace or inside it.",
        ));
    }
    if let Ok(rel) = abs.strip_prefix(&p.root) {
        let first = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_default();
        if matches!(first.as_str(), "factory" | ".bf" | ".claude" | ".git") {
            return Err(bad(
                format!(
                    "{} is inside `{first}/`, where the factory keeps its own files",
                    abs.display()
                ),
                "Put the code repository in a folder of its own, for example `./app`.",
            ));
        }
    }
    if !gitx::is_repo(abs) || !abs.join(".git").exists() {
        return Err(bad(
            format!("{} is not the top of a git repository", abs.display()),
            "Give the main checkout of the repository (the folder that holds `.git`).",
        ));
    }
    Ok(())
}

/// `path` relative to `base` when both are absolute and `path` is not below `base` (a sibling folder).
fn pathdiff(path: &Path, base: &Path) -> Option<String> {
    let pc: Vec<_> = path.components().collect();
    let bc: Vec<_> = base.components().collect();
    let common = pc.iter().zip(bc.iter()).take_while(|(a, b)| a == b).count();
    if common == 0 || common < 2 {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..bc.len() {
        out.push("..");
    }
    for c in &pc[common..] {
        out.push(c.as_os_str());
    }
    Some(out.to_string_lossy().to_string())
}
