//! `bf init [--workspace]`.

use super::{out, start_dir, Args};
use crate::refusal::{refuse, R};
use crate::store::{default_policy_text, Project};
use crate::{events, gitx, util};
use serde_json::json;
use std::fs;
use std::path::Path;

pub(crate) fn cmd_init(args: &Args) -> R<()> {
    let dir = fs::canonicalize(start_dir(args))?;
    let workspace = args.has("workspace");
    // A workspace is a folder of its own that holds the factory and points at code repositories; it is
    // normally a private git repository, and gets one here if it is not one yet.
    if workspace && !dir.join(".git").exists() {
        let o = gitx::run(&dir, &["init", "-q", "-b", "main"], &[])?;
        if !o.ok {
            return Err(refuse(
                "create the workspace's git repository",
                format!("git init failed: {}", o.stderr),
                &["Run `git init` in the folder yourself, then `bf init --workspace` again."],
                "`bf status` prints the project name.",
            ));
        }
    }
    if !workspace && !dir.join(".git").exists() {
        return Err(refuse(
            "initialise a project here",
            "this folder is not the top of a git repository",
            &[
                "Run `git init` (and make a first commit) or change into the repository root.",
                "Run `bf init` again.",
            ],
            "`bf status` prints the project name.",
        ));
    }
    let existed = dir.join("factory/config.toml").is_file();
    for d in [
        "factory/records",
        "factory/receipts",
        "factory/evidence",
        "factory/log",
        ".bf/work",
        ".bf/briefs",
        ".bf/drafts",
    ] {
        fs::create_dir_all(dir.join(d))?;
    }
    let project_id = util::random_hex(8)?;
    if !existed {
        let name = args.flag("name").map(|x| x.to_string()).unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "project".into())
        });
        let main = if workspace {
            "main".to_string()
        } else {
            gitx::current_branch(&dir).unwrap_or_else(|| "main".into())
        };
        let cfg = format!(
            "# Black Factory project config.\nproject = {}\nproject_id = \"{project_id}\"\nmain_branch = {}\nbranch_prefix = \"bf/\"\n",
            toml::Value::String(name),
            toml::Value::String(main)
        );
        fs::write(dir.join("factory/config.toml"), cfg)?;
        if workspace && !dir.join("factory/workspace.toml").exists() {
            fs::write(
                dir.join("factory/workspace.toml"),
                "# The code repositories this workspace works on. `bf repo add <name> <path>` adds one.\n",
            )?;
        }
        // The PATH and the git program that checks and git calls will use are captured from this shell.
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
        let git = find_in_path("git", &path).unwrap_or_else(|| "/usr/bin/git".into());
        // A policy that is already there (a cloned project) is kept; the human accepts it below.
        if !dir.join("factory/policy.toml").exists() {
            fs::write(
                dir.join("factory/policy.toml"),
                default_policy_text(&path, &git),
            )?;
        }
    } else {
        // A project from before the state folder gets its id.
        let cfg_path = dir.join("factory/config.toml");
        let cfg = fs::read_to_string(&cfg_path)?;
        if !cfg
            .lines()
            .any(|l| l.trim_start().starts_with("project_id"))
        {
            fs::write(&cfg_path, format!("{cfg}project_id = \"{project_id}\"\n"))?;
        }
    }
    let gi = dir.join(".gitignore");
    let cur = fs::read_to_string(&gi).unwrap_or_default();
    if !cur.lines().any(|l| l.trim() == ".bf/" || l.trim() == ".bf") {
        let mut n = cur.clone();
        if !n.is_empty() && !n.ends_with('\n') {
            n.push('\n');
        }
        n.push_str(".bf/\n");
        fs::write(&gi, n)?;
    }
    let p = Project::discover(&dir)?;
    // The state folder, outside the project tree, owner only: the key and the log head.
    p.ensure_state()?;
    if !existed {
        events::append(&p, "project.init", json!({"project": p.config.project}))?;
    } else {
        events::ensure_head(&p)?;
    }
    // The first policy of a project on this machine is accepted by `bf init`; a later change on disk is
    // the human's to accept with `bf policy accept`.
    // The main branch as it stands now is where the engine starts counting.
    events::update_head(&p, |h| record_repo_heads(&p, h))?;
    let mut p = p;
    if p.policy_state == crate::store::PolicyState::NeverAccepted {
        p.store_accepted_policy()?;
        p.reload_policy()?;
    }
    p.write_guard()?;
    // A workspace whose code repositories live inside it keeps them out of its own git repository.
    let _ = p.ensure_excludes()?;
    out(
        args,
        &json!({"initialised": !existed, "root": dir.to_string_lossy(), "project": p.config.project}),
        || {
            if existed {
                "already initialised; key, log head and guard table refreshed".to_string()
            } else {
                format!("initialised {}", p.config.project)
            }
        },
    );
    Ok(())
}

/// Note, for every repository that has none yet, the branch to watch and where it stands now.
fn record_repo_heads(p: &Project, h: &mut events::Head) -> bool {
    let mut changed = false;
    for r in &p.repos {
        let e = h.repos.entry(r.name.clone()).or_default();
        if e.branch.is_none() {
            e.branch = Some(r.configured.clone());
            changed = true;
        }
        let main = e.branch.clone().unwrap_or_default();
        if e.commit.is_none() && gitx::branch_exists(&r.path, &main) {
            e.commit = gitx::must(&r.path, &["rev-parse", &main]).ok();
            changed = true;
        }
    }
    changed
}

/// The first executable `name` in a PATH string, as an absolute path.
fn find_in_path(name: &str, path: &str) -> Option<String> {
    path.split(':')
        .map(|d| Path::new(d).join(name))
        .find(|c| c.is_file())
        .map(|c| c.to_string_lossy().to_string())
}
