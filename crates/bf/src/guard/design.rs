//! The guard's rule of the design roles (U-97, D-111, D-113): a designer, a design reviewer and a UI/UX
//! designer design and judge only, so a shell command of theirs is refused when it builds or runs cargo,
//! runs a check with `bf evidence run`, or copies a repository of the workspace or a folder of
//! `.bf/work`. Reading stays free.
//!
//! Stated limit, as for the repair judge: the agent type is the one the harness puts in the hook
//! payload, and a designer started under another type is not caught.

use super::{basename, bf_rest, deny, resolve, under, Decision};
use crate::strings::t;
use serde_json::Value;

/// The agent types the rule binds.
const ROLES: &[&str] = &["designer", "design-reviewer", "uiux-designer"];

/// The cargo subcommands a design role may not run.
const CARGO_BARRED: &[&str] = &[
    "build", "check", "clippy", "test", "nextest", "run", "bench",
];

/// Does the rule bind this agent type?
pub fn binds(agent: &str) -> bool {
    ROLES.contains(&agent)
}

fn refuse(key: &str, values: &[(&str, &str)]) -> Decision {
    deny(
        &t(&format!("guard_design.{key}.refused"), values),
        &t(&format!("guard_design.{key}.because"), &[]),
        &[&t(&format!("guard_design.{key}.step"), &[])],
        &t(&format!("guard_design.{key}.worked"), &[]),
    )
}

/// The cargo subcommand of a call: cargo's own options and a `+toolchain` may come first.
fn cargo_sub(w: &[String]) -> Option<&str> {
    let mut i = 1;
    while i < w.len() {
        let a = w[i].as_str();
        match a {
            "-C" | "--manifest-path" | "--config" | "-Z" | "--color" | "--target-dir" => i += 2,
            _ if a.starts_with('-') || a.starts_with('+') => i += 1,
            _ => return Some(a),
        }
    }
    None
}

/// Is the path a repository of the workspace (or one that holds it), or a folder of `.bf/work`?
fn is_checkout(table: &Value, abs: &str) -> bool {
    let wp = table["work_prefix"].as_str().unwrap_or("");
    if !wp.is_empty() && under(abs, wp).is_some() {
        return true;
    }
    let root = table["root"].as_str().unwrap_or("");
    let repos = table["repos"].as_array().into_iter().flatten();
    std::iter::once(root)
        .chain(repos.filter_map(|r| r["path"].as_str()))
        .filter(|p| !p.is_empty())
        .any(|p| {
            let p = p.trim_end_matches('/');
            // The repository itself, or a folder that holds it.
            abs.trim_end_matches('/') == p || abs == "/" || under(p, abs).is_some()
        })
}

/// Does the command copy a source that is a checkout? The words are those after the program.
fn copies_checkout(prog: &str, w: &[String], cwd: &str, table: &Value) -> bool {
    let args = &w[1..];
    let flag = |a: &String| a.starts_with('-') && a.len() > 1;
    let hit = |p: &str, base: &str| is_checkout(table, &resolve(p, base));
    match prog {
        "cp" => {
            let recursive = args.iter().any(|a| {
                a == "--recursive"
                    || a == "--archive"
                    || (a.starts_with('-')
                        && !a.starts_with("--")
                        && a.chars().any(|c| matches!(c, 'R' | 'r' | 'a')))
            });
            let pos: Vec<&String> = args.iter().filter(|a| !flag(a)).collect();
            recursive && pos.len() >= 2 && pos[..pos.len() - 1].iter().any(|p| hit(p, cwd))
        }
        "rsync" | "ditto" => {
            let pos: Vec<&String> = args.iter().filter(|a| !flag(a)).collect();
            pos.len() >= 2 && pos[..pos.len() - 1].iter().any(|p| hit(p, cwd))
        }
        "tar" => {
            let mut base = cwd.to_string();
            let mut based = false;
            let mut i = 0;
            let mut words: Vec<&String> = Vec::new();
            while i < args.len() {
                let a = &args[i];
                if a == "-C" || a == "--directory" {
                    if let Some(d) = args.get(i + 1) {
                        base = resolve(d, cwd);
                        based = true;
                    }
                    i += 2;
                    continue;
                }
                if !flag(a) {
                    words.push(a);
                }
                i += 1;
            }
            let base = base.as_str();
            words.iter().any(|p| hit(p, base)) || (based && is_checkout(table, base))
        }
        "git" => {
            let mut base = cwd.to_string();
            let mut i = 0;
            while i < args.len() {
                match args[i].as_str() {
                    "-C" => {
                        if let Some(d) = args.get(i + 1) {
                            base = resolve(d, &base);
                        }
                        i += 2;
                    }
                    "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
                    x if x.starts_with('-') => i += 1,
                    _ => break,
                }
            }
            let rest: Vec<&String> = args[i.min(args.len())..].iter().collect();
            match rest.first().map(|s| s.as_str()) {
                Some("clone") => rest[1..]
                    .iter()
                    .filter(|a| !flag(a))
                    .take(1)
                    .any(|p| hit(p, &base)),
                Some("worktree") => {
                    rest.get(1).map(|s| s.as_str()) == Some("add") && is_checkout(table, &base)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// The refusal for a shell command of a design role, or `None` when it may run.
pub fn shell_denial(cmd: &str, cwd: &str, table: &Value) -> Option<Decision> {
    for c in crate::shellparse::engine_calls(cmd) {
        if c.cargo {
            continue;
        }
        let rest = bf_rest(&c.words);
        if rest.first().map(|x| x.as_str()) == Some("evidence")
            && rest.get(1).map(|x| x.as_str()) == Some("run")
        {
            return Some(refuse("evidence", &[]));
        }
    }
    let parsed = crate::shellparse::commands(cmd);
    // An alias a line sets for cargo (`CARGO_ALIAS_X=test cargo x`, `export CARGO_ALIAS_X=test; cargo x`,
    // `--config 'alias.x="test"'`) may be any barred subcommand: its words cannot be read here.
    let sets_alias = parsed.iter().any(|p| {
        p.words
            .iter()
            .any(|w| w.to_ascii_uppercase().starts_with("CARGO_ALIAS_"))
    });
    for p in &parsed {
        let is_cargo = crate::shellparse::program_index(&p.words)
            .is_some_and(|i| basename(&p.words[i]) == "cargo");
        if is_cargo && (sets_alias || crate::shellparse::cargo_alias_args(&p.words).is_some()) {
            return Some(refuse("cargo", &[("sub", "alias")]));
        }
    }
    for p in parsed {
        let Some(i) = crate::shellparse::program_index(&p.words) else {
            continue;
        };
        let w = &p.words[i..];
        let prog = basename(&w[0]);
        // A tool called by its own program name (`cargo-nextest`, `cargo-clippy`) is the same call.
        let direct = prog
            .strip_prefix("cargo-")
            .filter(|s| CARGO_BARRED.contains(s));
        if let Some(sub) = direct {
            return Some(refuse("cargo", &[("sub", sub)]));
        }
        if prog == "cargo" {
            // Cargo's built-in short forms name the same subcommands.
            if let Some(sub) = cargo_sub(w)
                .map(|s| match s {
                    "b" => "build",
                    "c" => "check",
                    "t" => "test",
                    "r" => "run",
                    x => x,
                })
                .filter(|s| CARGO_BARRED.contains(s))
            {
                return Some(refuse("cargo", &[("sub", sub)]));
            }
        } else if copies_checkout(prog, w, cwd, table) {
            let shown = w.join(" ");
            return Some(refuse("copy", &[("command", &shown)]));
        }
    }
    None
}
