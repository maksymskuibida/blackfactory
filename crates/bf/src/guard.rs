//! `bf guard`: the per-tool-call hook.
//!
//! It reads the tool call from stdin and the project's guard table, `.bf/guard.json`, and, to check the
//! table's signature, the key in the project's state folder. It starts no process and calls no git.
//! Paths are resolved against the disk only with `canonicalize`, so a symlink cannot hide a target.
//!
//! The guard stops mistakes, drift and casual cheating and makes deliberate cheating visible. It cannot
//! stop an agent that has an unrestricted shell under the same account: see "What the engine does not
//! stop" in docs/engine.md.

use crate::refusal::Refusal;
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Decision {
    /// True when nothing in the table forbids the call.
    pub allow: bool,
    pub reason: String,
}

fn deny(refused: &str, because: &str, steps: &[&str], worked: &str) -> Decision {
    let r = Refusal {
        refused: refused.into(),
        because: because.into(),
        do_this: steps.iter().map(|s| s.to_string()).collect(),
        worked_when: worked.into(),
    };
    Decision {
        allow: false,
        reason: r.text(),
    }
}

fn allow() -> Decision {
    Decision {
        allow: true,
        reason: "bf guard: nothing in the guard table forbids this call".into(),
    }
}

/// Lexically normalise a path: resolve `.` and `..` without touching the disk.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

fn expand_home(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if p == "~" || p == "$HOME" {
        home
    } else if let Some(r) = p.strip_prefix("~/").or_else(|| p.strip_prefix("$HOME/")) {
        format!("{home}/{r}")
    } else {
        p.to_string()
    }
}

fn absolutize(p: &str, cwd: &str) -> String {
    let p = expand_home(p);
    if p.starts_with('/') {
        normalize(&p)
    } else {
        normalize(&format!("{cwd}/{p}"))
    }
}

/// Absolute, normalised, and with symlinks resolved for the part of the path that exists.
fn resolve(p: &str, cwd: &str) -> String {
    let lex = absolutize(p, cwd);
    let mut base = lex.clone();
    let mut rest: Vec<String> = Vec::new();
    loop {
        if let Ok(c) = fs::canonicalize(&base) {
            let mut out = c.to_string_lossy().to_string();
            for r in rest.iter().rev() {
                out.push('/');
                out.push_str(r);
            }
            return normalize(&out);
        }
        match base.rfind('/') {
            Some(0) | None => return lex,
            Some(i) => {
                rest.push(base[i + 1..].to_string());
                base.truncate(i);
            }
        }
    }
}

fn under<'a>(path: &'a str, base: &str) -> Option<&'a str> {
    let base = base.trim_end_matches('/');
    if path == base {
        return Some("");
    }
    path.strip_prefix(base).and_then(|r| r.strip_prefix('/'))
}

struct Zone<'a> {
    kind: &'a str, // "unit", "checkpoint" or "main"
    id: &'a str,
    base: String,
    files: Vec<String>,
}

fn zone_of<'a>(table: &'a Value, abs: &'a str) -> Zone<'a> {
    let mut best: Option<(&Value, usize)> = None;
    if let Some(ws) = table["worktrees"].as_array() {
        for w in ws {
            let path = w["path"].as_str().unwrap_or("");
            if !path.is_empty() && under(abs, path).is_some() {
                let l = path.len();
                if best.map(|b| l > b.1).unwrap_or(true) {
                    best = Some((w, l));
                }
            }
        }
    }
    match best {
        Some((w, _)) => Zone {
            kind: w["kind"].as_str().unwrap_or("unit"),
            id: w["id"].as_str().unwrap_or(""),
            base: w["path"].as_str().unwrap_or("").to_string(),
            files: w["files"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
        },
        None => {
            // A working copy of a unit that has landed is no longer in the table. It is still a unit's
            // working copy (nothing may be written in it), not the main checkout.
            let wp = table["work_prefix"].as_str().unwrap_or("");
            if !wp.is_empty() {
                if let Some(rel) = under(abs, wp) {
                    if let Some(name) = rel.split('/').next().filter(|n| !n.is_empty()) {
                        if !name.starts_with("K-") {
                            return Zone {
                                kind: "unit",
                                id: name,
                                base: format!("{}/{}", wp.trim_end_matches('/'), name),
                                files: vec![],
                            };
                        }
                    }
                }
            }
            Zone {
                kind: "main",
                id: "",
                base: table["root"].as_str().unwrap_or("").to_string(),
                files: vec![],
            }
        }
    }
}

fn is_protected_rel(table: &Value, rel: &str) -> bool {
    if let Some(a) = table["protected"].as_array() {
        for p in a.iter().filter_map(|x| x.as_str()) {
            let p = p.trim_end_matches('/');
            if rel == p || rel.starts_with(&format!("{p}/")) {
                return true;
            }
        }
    }
    false
}

/// Where the conductor and planners may write inside the protected folders.
fn is_open_rel(rel: &str) -> bool {
    rel.starts_with(".bf/drafts/")
        || rel.starts_with(".bf/pages/")
        || rel == ".bf/pages"
        || rel == "factory/idea.md"
}

fn transcript_root(table: &Value) -> &str {
    table["transcript_root"].as_str().unwrap_or("")
}

fn in_transcripts(table: &Value, abs: &str) -> bool {
    let t = transcript_root(table);
    !t.is_empty() && under(abs, t).is_some()
}

fn state_dir(table: &Value) -> &str {
    table["state_dir"].as_str().unwrap_or("")
}

fn state_base(table: &Value) -> Option<&str> {
    let sd = state_dir(table);
    sd.rsplit_once('/').map(|(b, _)| b)
}

fn in_state(table: &Value, abs: &str) -> bool {
    let sd = state_dir(table);
    !sd.is_empty() && under(abs, sd).is_some()
}

fn state_denial() -> Decision {
    deny(
        "touch the project's state folder",
        "it holds the key that seals receipts and the log, the head of the log and the role tokens; an agent that could read or write it could forge evidence and approvals",
        &["Use the engine's own commands: `bf evidence run`, `bf report`, `bf status`."],
        "`bf evidence verify` lists your receipt as sealed.",
    )
}

fn transcript_denial() -> Decision {
    deny(
        "write under the harness's transcript root",
        "the transcripts are how the engine tells the human's words from an agent's; a file written there could stand in for the human's message",
        &["Do not write under the transcript root."],
        "The path is somewhere else.",
    )
}

/// The main checkout of a code repository (other than the project folder) that holds `abs`.
fn repo_containing(table: &Value, abs: &str) -> Option<String> {
    let root = table["root"].as_str().unwrap_or("");
    table["repos"]
        .as_array()?
        .iter()
        .filter_map(|r| r["path"].as_str())
        .find(|p| *p != root && under(abs, p).is_some())
        .map(|p| p.to_string())
}

fn check_write(table: &Value, cwd: &str, raw: &str) -> Decision {
    let abs = resolve(raw, cwd);
    if in_state(table, &abs) {
        return state_denial();
    }
    if in_transcripts(table, &abs) {
        return transcript_denial();
    }
    let root = table["root"].as_str().unwrap_or("");
    let zone = zone_of(table, &abs);
    let here = zone_of(table, cwd);
    if zone.kind == "main" {
        // A code repository that lies outside the workspace: nobody writes git's hooks or config there,
        // and a worker does not write into the repository's own checkout (the human edits it freely).
        if let Some(repo) = repo_containing(table, &abs) {
            let rel = under(&abs, &repo).unwrap_or("");
            if rel == ".git/config" || rel == ".git/hooks" || rel.starts_with(".git/hooks/") {
                return deny(
                    &format!("write {rel} in a code repository"),
                    "git hooks and git config belong to git and the human",
                    &["Change only files inside your unit's working copy."],
                    "The path is inside your unit's `files` list.",
                );
            }
            if here.kind == "unit" || here.kind == "checkpoint" {
                return deny(
                    &format!("write {raw}"),
                    &format!(
                        "that is the checkout of a code repository ({repo}); you work in {}",
                        here.base
                    ),
                    &["Write inside your unit's working copy; landing merges it into the repository."],
                    "The path starts with your working copy's folder.",
                );
            }
            return allow();
        }
        let Some(rel) = under(&abs, root) else {
            // Outside the project. A worker still stays out of the other working copies' parent folders.
            return allow();
        };
        if rel == ".bf/work" || rel.starts_with(".bf/work/") {
            return deny(
                &format!("write {raw}"),
                "that folder holds working copies and is not a unit's or checkpoint's own working copy",
                &["Work inside the working copy that `bf unit start` created for your unit."],
                "The write goes to a path inside your unit's working copy.",
            );
        }
        if here.kind != "unit" && is_open_rel(rel) {
            return allow();
        }
        if is_protected_rel(table, rel) {
            return deny(
                &format!("write {rel}"),
                "factory records, the policy, gates, the guard table and the hook settings are changed by the engine and the human only",
                &["Use an engine command: `bf record put`, `bf report`, `bf evidence run`.", "If the policy or a gate must change, ask the human."],
                "`bf record get <id>` shows the change you asked the engine to make.",
            );
        }
        if here.kind == "unit" {
            return deny(
                &format!("write {rel}"),
                &format!(
                    "you work in the working copy of {}; that path is outside it",
                    here.id
                ),
                &[&format!("Write the file inside {}.", here.base)],
                "The path starts with your working copy's folder.",
            );
        }
        return allow();
    }
    let rel = under(&abs, &zone.base).unwrap_or("");
    if is_protected_rel(table, rel) || rel == ".git" || rel.starts_with(".git/") {
        return deny(
            &format!("write {rel} in {}", zone.id),
            "that path belongs to the engine, the human or git, not to a unit's work",
            &["Change only files inside the unit's scope."],
            "The path is inside the unit's `files` list.",
        );
    }
    if zone.kind == "checkpoint" {
        return deny(
            &format!("write {rel} in the checkpoint working copy {}", zone.id),
            "a checkpoint's content changes only by landing units",
            &["Make the change in a unit's working copy and land the unit."],
            "`bf unit land` merges your change.",
        );
    }
    if here.kind == "unit" && here.id != zone.id {
        return deny(
            &format!("write {rel} in {}", zone.id),
            &format!("you work in the working copy of {}", here.id),
            &[&format!("Write inside {}.", here.base)],
            "The path starts with your working copy's folder.",
        );
    }
    if !zone.files.iter().any(|g| util::path_matches(g, rel)) {
        return deny(
            &format!("write {rel}"),
            &format!(
                "it is outside the file scope of {}: {}",
                zone.id,
                zone.files.join(", ")
            ),
            &[
                "Keep to the files the unit lists.",
                "If the scope is too small, tell the conductor; only the human changes the plan.",
            ],
            "The path matches one of the unit's `files` entries.",
        );
    }
    allow()
}

// ---------------------------------------------------------------- shell commands

/// Strip quotes and backslashes: `g"it"` and `b\f` name the programs they look like.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '"' | '\'' | '\\'))
        .collect()
}

fn words(seg: &str) -> Vec<String> {
    seg.split_whitespace().map(|w| w.to_string()).collect()
}

fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

fn split_segments(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let b: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let two = if i + 1 < b.len() {
            Some(b[i + 1])
        } else {
            None
        };
        match (c, two) {
            ('&', Some('&')) | ('|', Some('|')) => {
                out.push(std::mem::take(&mut cur));
                i += 2;
                continue;
            }
            (';', _) | ('\n', _) | ('|', _) | ('`', _) => {
                out.push(std::mem::take(&mut cur));
            }
            ('$', Some('(')) => {
                out.push(std::mem::take(&mut cur));
                i += 2;
                continue;
            }
            (')', _) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
        i += 1;
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Strip leading environment assignments and common wrappers.
fn command_words(seg: &str) -> Vec<String> {
    let mut w = words(seg);
    loop {
        match w.first().map(|s| s.as_str()) {
            Some(x) if x.contains('=') && !x.starts_with('-') && !x.contains('/') => {
                w.remove(0);
            }
            Some("sudo") | Some("env") | Some("command") | Some("nohup") | Some("time")
            | Some("exec") | Some("nice") | Some("xargs") | Some("builtin") => {
                w.remove(0);
            }
            _ => break,
        }
    }
    w
}

fn git_sub(w: &[String]) -> Option<(usize, &str)> {
    let mut i = 1;
    while i < w.len() {
        match w[i].as_str() {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
            x if x.starts_with('-') => i += 1,
            x => return Some((i, x)),
        }
    }
    None
}

/// Words of a command with their quotes kept apart: split on spaces, quotes and shell punctuation.
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || "\"'`()<>|&;=,{}[]".contains(c))
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// Does a path that is protected (in the project root or in any working copy) get named?
fn is_protected_abs(table: &Value, abs: &str) -> bool {
    let root = table["root"].as_str().unwrap_or("");
    if let Some(rel) = under(abs, root) {
        if rel == ".bf/work" || rel.starts_with(".bf/work/") {
            // A working copy: its own factory/ copy is protected, its source is not.
        } else if !is_open_rel(rel) && is_protected_rel(table, rel) {
            return true;
        }
    }
    if let Some(ws) = table["worktrees"].as_array() {
        for w in ws {
            let Some(base) = w["path"].as_str() else {
                continue;
            };
            if let Some(rel) = under(abs, base) {
                if is_protected_rel(table, rel) {
                    return true;
                }
            }
        }
    }
    false
}

/// Does the shell text name a path under the harness's transcript root?
fn names_transcripts(table: &Value, text: &str, cwd: &str) -> bool {
    if transcript_root(table).is_empty() {
        return false;
    }
    tokens(text)
        .into_iter()
        .filter(|t| t.contains('/') || t == "~")
        .any(|t| in_transcripts(table, &resolve(&t, cwd)))
}

/// Does the shell text name a protected path or the state folder, however it is spelled (`./factory`,
/// `x/../factory`, an absolute path, a symlink, `~`)?
fn names_protected(table: &Value, text: &str, cwd: &str) -> bool {
    for t in tokens(text) {
        if t.starts_with('-') && !t.contains('/') {
            continue;
        }
        let path_like =
            t.contains('/') || t == "~" || matches!(t.as_str(), "factory" | ".bf" | ".claude");
        if !path_like {
            continue;
        }
        let abs = resolve(&t, cwd);
        if in_state(table, &abs) || is_protected_abs(table, &abs) {
            return true;
        }
    }
    false
}

const READ_ONLY: &[&str] = &[
    "cat",
    "ls",
    "head",
    "tail",
    "wc",
    "less",
    "more",
    "grep",
    "rg",
    "file",
    "stat",
    "diff",
    "echo",
    "printf",
    "true",
    "pwd",
    "test",
    "[",
    "jq",
    "sort",
    "uniq",
    "cut",
    "tr",
    "nl",
    "tac",
    "column",
    "od",
    "xxd",
    "sha256sum",
    "shasum",
    "md5",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "which",
];
const GIT_READ_ONLY: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "ls-files",
    "add",
    "blame",
    "check-ignore",
    "rev-parse",
    "branch",
];
const WRITERS: &[&str] = &[
    "rm", "mv", "cp", "tee", "chmod", "chown", "truncate", "dd", "ln", "install", "touch", "mkdir",
    "rmdir", "unlink", "shred", "rsync", "patch",
];

/// Paths a segment writes to: redirect targets and the arguments of writer programs.
fn write_targets(seg: &str, w: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = seg;
    while let Some(i) = rest.find('>') {
        let after = rest[i + 1..]
            .trim_start_matches(['>', '&', '|'])
            .trim_start();
        let t: String = after
            .chars()
            .take_while(|c| !c.is_whitespace() && !";&|)".contains(*c))
            .collect();
        if !t.is_empty() && !t.starts_with('&') && t != "/dev/null" {
            out.push(t);
        }
        rest = &rest[i + 1..];
    }
    if let Some(first) = w.first() {
        let b = basename(first);
        if WRITERS.contains(&b) {
            out.extend(w[1..].iter().filter(|a| !a.starts_with('-')).cloned());
        } else if (b == "sed" || b == "perl") && w.iter().any(|x| x.starts_with("-i") || x == "-pi")
        {
            out.extend(
                w[1..]
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .skip(1)
                    .cloned(),
            );
        }
    }
    out
}

/// Every subcommand of `bf`. A call whose subcommand is not one of these, written literally, is denied.
const BF_KNOWN: &[&str] = &[
    "init",
    "record",
    "id",
    "ledger",
    "plan",
    "run",
    "status",
    "log",
    "next",
    "report",
    "brief",
    "unit",
    "evidence",
    "checkpoint",
    "gate",
    "test-mode",
    "view",
    "render",
    "ask",
    "answer",
    "questions",
    "amend",
    "cost",
    "stuck",
    "doctor",
    "guard",
    "head",
    "policy",
    "repo",
    "version",
    "help",
    "human-turn",
];

/// What a working copy may run: (subcommand, the second words allowed; empty = any). A unit's
/// working copy also reports and files findings; a checkpoint's only runs checks and reads.
const UNIT_BF: &[(&str, &[&str])] = &[
    ("report", &[]),
    ("ask", &[]),
    ("status", &[]),
    (
        "evidence",
        &["run", "verify", "fail", "unreachable", "attach"],
    ),
    ("record", &["get", "list", "put"]),
];
const CP_BF: &[(&str, &[&str])] = &[
    ("ask", &[]),
    ("status", &[]),
    (
        "evidence",
        &["run", "verify", "fail", "unreachable", "attach"],
    ),
    ("record", &["get", "list"]),
];

/// Tokens of the command with shell punctuation and the start of a substitution kept apart.
fn shell_tokens(cleaned: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let b: Vec<char> = cleaned.chars().collect();
    let mut i = 0;
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            flush(&mut cur, &mut out);
        } else if c == '$' && i + 1 < b.len() && (b[i + 1] == '(' || b[i + 1] == '{') {
            flush(&mut cur, &mut out);
            out.push(format!("${}", b[i + 1]));
            i += 1;
        } else if c == '`' || ";&|()<>".contains(c) {
            flush(&mut cur, &mut out);
            out.push(c.to_string());
        } else {
            cur.push(c);
        }
        i += 1;
    }
    flush(&mut cur, &mut out);
    out
}

/// A call of `bf` that cannot be read: after `eval`, `sh -c`, `env`, `xargs`, `command`, a backtick
/// or `$(`; or with a subcommand that is not a plain word from the list.
fn bf_misuse(cleaned: &str) -> Option<String> {
    let t = shell_tokens(cleaned);
    for i in 0..t.len() {
        if basename(&t[i]) != "bf" {
            continue;
        }
        let prev = if i > 0 { t[i - 1].as_str() } else { "" };
        if matches!(
            prev,
            "eval"
                | "env"
                | "xargs"
                | "command"
                | "exec"
                | "-c"
                | "`"
                | "$("
                | "${"
                | "sh"
                | "bash"
                | "zsh"
                | "dash"
                | "builtin"
        ) {
            return Some(format!(
                "`bf` is run through `{prev}`, which hides what it does"
            ));
        }
        let mut j = i + 1;
        while j < t.len() && t[j].starts_with('-') {
            j += if t[j] == "-C" { 2 } else { 1 };
        }
        let Some(sub) = t.get(j) else { continue };
        if matches!(sub.as_str(), ";" | "&" | "|" | ")" | "<" | ">") {
            continue;
        }
        if sub.contains('$') || sub.contains('`') || sub == "(" {
            return Some("the `bf` subcommand is built at run time".into());
        }
        if !sub.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            || !BF_KNOWN.contains(&sub.as_str())
        {
            return Some(format!("`bf {sub}` is not a plain subcommand of the list"));
        }
    }
    None
}

/// Does the text redirect somewhere other than `/dev/null` or another descriptor?
fn real_redirect(seg: &str) -> bool {
    let mut t = seg.to_string();
    for n in [
        "&>/dev/null",
        "2>/dev/null",
        "1>/dev/null",
        ">/dev/null",
        "2>&1",
        "1>&2",
        ">&2",
        ">&1",
        "&>",
        "2>>/dev/null",
    ] {
        t = t.replace(n, " ");
    }
    t.contains('>')
}

fn check_bash(table: &Value, cwd: &str, cmd: &str) -> Decision {
    let cleaned = clean(cmd);
    if cleaned.contains("human-turn") {
        return deny(
            "run `bf human-turn`",
            "only the harness hook records what the human typed; an agent that could record a turn could approve its own gate",
            &["Ask the human in chat for their decision.", "The hook records their message by itself."],
            "`bf log --tail 3` shows a human turn that you did not write.",
        );
    }
    if let Some(why) = bf_misuse(&cleaned) {
        return deny(
            "run `bf` this way",
            &format!("{why}; the engine's commands are written out in full so that the guard can read them"),
            &["Write the command plainly, for example `bf status` or `bf report U-1 --result built`."],
            "The command starts with `bf` and a subcommand from `bf help`.",
        );
    }
    if cleaned.contains("BF_STATE_HOME") || cleaned.contains("XDG_STATE_HOME") {
        return state_denial();
    }
    // The state folder, however it is spelled: nothing may name it, whatever the program.
    let sd = state_dir(table);
    let pid = table["project_id"].as_str().unwrap_or("");
    if !sd.is_empty()
        && (cleaned.contains(sd)
            || (pid.len() >= 8 && cleaned.contains(pid))
            || state_base(table)
                .map(|b| b.len() > 8 && cleaned.contains(b))
                .unwrap_or(false))
    {
        return state_denial();
    }
    for t in tokens(cmd) {
        if (t.contains('/') || t == "~") && in_state(table, &resolve(&t, cwd)) {
            return state_denial();
        }
    }
    let here = zone_of(table, cwd);
    let in_worker = here.kind == "unit";
    let in_cp = here.kind == "checkpoint";
    let main_branch = table["main_branch"].as_str().unwrap_or("main");
    let prefix = table["branch_prefix"].as_str().unwrap_or("bf/");

    for seg in split_segments(cmd) {
        let segc = clean(&seg);
        let w = command_words(&segc);
        let Some(first) = w.first() else { continue };
        let b = basename(first);

        // -- bf subcommands
        if b == "bf" {
            let mut i = 1;
            let mut dash_c = false;
            while i < w.len() && w[i].starts_with('-') {
                if w[i] == "-C" {
                    dash_c = true;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let sub = w.get(i).map(|s| s.as_str()).unwrap_or("");
            if (in_worker || in_cp) && dash_c {
                return deny(
                    "run `bf -C` from a working copy",
                    "workers and verifiers act in their own working copy; `-C` would point the engine somewhere else",
                    &["Run the command without -C, from your working copy."],
                    "The command runs in your working copy.",
                );
            }
            if in_worker || in_cp {
                let list: &[(&str, &[&str])] = if in_worker { UNIT_BF } else { CP_BF };
                let second = w.get(i + 1).map(|x| x.as_str()).unwrap_or("");
                let ok = sub.is_empty()
                    || matches!(sub, "help" | "version")
                    || list.iter().any(|(c, subs)| *c == sub && (subs.is_empty() || subs.contains(&second)))
                    // Of `record put`, only a finding.
                    && !(sub == "record" && second == "put" && w.get(i + 2).map(|x| x.as_str()) != Some("finding"));
                if !ok {
                    return deny(
                        &format!("run `bf {sub} {second}` from a working copy").replace("  ", " "),
                        if in_worker {
                            "a worker may report, file a finding, run evidence and read the status; nothing else"
                        } else {
                            "a checkpoint's working copy is for running its checks and reading state"
                        },
                        &["Use `bf report`, `bf evidence run|verify|fail|unreachable`, `bf record put finding`, `bf record get|list` or `bf status`.", "Ask the conductor for anything else."],
                        "The command you need is on that list.",
                    );
                }
            }
            continue;
        }
        if (b == "cargo" || b == "python" || b == "python3" || b == "sh" || b == "bash")
            && cleaned.contains("bf human-turn")
        {
            return deny(
                "run `human-turn`",
                "only the harness hook records what the human typed",
                &["Ask the human in chat."],
                "`bf log --tail 3` shows a turn you did not write.",
            );
        }

        // -- leaving the working copy
        if (in_worker || in_cp) && matches!(b, "cd" | "pushd") {
            let target = w.get(1).map(|s| s.as_str()).unwrap_or("~");
            let abs = resolve(target, cwd);
            if under(&abs, &here.base).is_none() {
                return deny(
                    "leave your working copy",
                    "from a working copy, commands run in that working copy; changing folder would let later commands run as if from the project",
                    &[&format!("Stay inside {}.", here.base)],
                    "Every command runs in your working copy.",
                );
            }
        }

        // -- git
        if b == "git" {
            if let Some((i, sub)) = git_sub(&w) {
                let target = {
                    let mut dir = cwd.to_string();
                    let mut j = 1;
                    while j + 1 < w.len() && j < i {
                        if w[j] == "-C" {
                            dir = resolve(&w[j + 1], cwd);
                        }
                        j += 1;
                    }
                    zone_of(table, &dir).kind.to_string()
                };
                let in_unit = target == "unit";
                let rest: Vec<&str> = w[i + 1..].iter().map(|s| s.as_str()).collect();
                let why = |s: &str| {
                    deny(
                        &format!("run `git {sub}`"),
                        s,
                        &[
                            "Use `bf unit land` / `bf checkpoint close` for merges.",
                            "Ask the human to do it with git.",
                        ],
                        "The branch you need is updated by the engine.",
                    )
                };
                if w.iter()
                    .any(|x| matches!(x.as_str(), "--git-dir" | "--work-tree"))
                    || w.iter()
                        .any(|x| x.starts_with("--git-dir=") || x.starts_with("--work-tree="))
                {
                    return why(
                        "pointing git at another repository would hide what is being changed",
                    );
                }
                if w.windows(2).any(|p| {
                    p[0] == "-c"
                        && (p[1].starts_with("core.")
                            || p[1].starts_with("alias.")
                            || p[1].starts_with("filter."))
                }) {
                    return why("git config given on the command line can run programs or change what git sees");
                }
                // An unknown subcommand is usually the user's own alias; setting one up is refused below
                // (`git config alias.*`, `git -c alias.*`), and moving a ref is refused by name.
                if sub == "config"
                    && rest.iter().any(|a| {
                        let a = a.to_lowercase();
                        a.starts_with("alias.")
                            || a.starts_with("filter.")
                            || a.starts_with("core.")
                            || a.starts_with("include")
                            || a.starts_with("url.")
                    })
                {
                    return why("setting an alias, a filter or a core option in git config can make git run programs or move branches");
                }
                match sub {
                    "stash" if !in_unit && matches!(rest.first(), Some(&"list") | Some(&"show")) => {}
                    "push" => return why("nothing is pushed until the human decides; no engine command touches the network"),
                    "update-ref" | "pull" | "worktree" | "reflog" | "gc" | "replace" | "filter-branch" | "symbolic-ref" => {
                        return why("branch pointers and working copies are managed by the engine only")
                    }
                    "fetch" if rest.iter().any(|a| a.contains(':') || a.starts_with('+')) => {
                        return why("a fetch with a refspec can move the main branch or a checkpoint branch")
                    }
                    "merge" | "rebase" | "cherry-pick" | "revert" | "am" | "stash" | "clean" | "apply"
                        if !in_unit =>
                    {
                        return why("merging, rebasing, stashing or cleaning in the main checkout or a checkpoint working copy can move the main branch or lose records; `bf unit land` and `bf checkpoint close` do the merging, and the human does the rest with git");
                    }
                    "reset" if !in_unit && rest.iter().any(|a| matches!(*a, "--hard" | "--merge" | "--keep")) => {
                        return why("a hard reset in the main checkout or a checkpoint working copy can move the main branch or lose records")
                    }
                    "checkout" | "switch" | "restore" | "commit"
                        if !in_unit
                            && !rest.contains(&"--") && rest.iter().any(|a| a.starts_with(prefix) || a.contains(&format!(":{main_branch}"))) =>
                    {
                        return why("switching to or committing onto the engine's branches would change them behind its back")
                    }
                    "branch" => {
                        let force = rest.iter().any(|x| matches!(*x, "-d" | "-D" | "-f" | "--force" | "-m" | "-M" | "--delete" | "-c" | "-C" | "--move" | "--copy"));
                        let names = rest.iter().any(|x| *x == main_branch || x.starts_with(prefix));
                        if force && names {
                            return why("the main branch and the engine's branches move only through the engine");
                        }
                    }
                    _ => {}
                }
            }
        }

        // -- protected paths and the state folder, named by anything but the engine
        // Interpreters and `find` are left to the harness's own permission checks: the engine cannot
        // tell a script that reads a record from one that writes it, and refusing the reading is the
        // worse error.
        let prog_ro = READ_ONLY.contains(&b)
            || matches!(
                b,
                "python" | "python3" | "node" | "ruby" | "perl" | "tree" | "du"
            )
            || (b == "find"
                && !w.iter().any(|x| {
                    matches!(
                        x.as_str(),
                        "-delete"
                            | "-exec"
                            | "-execdir"
                            | "-ok"
                            | "-okdir"
                            | "-fprint"
                            | "-fprintf"
                    )
                }))
            || (b == "sed" && !w.iter().any(|x| x.starts_with("-i") || x == "-pi"))
            || (b == "awk" && !segc.contains("system"))
            || (b == "git"
                && git_sub(&w)
                    .map(|(i, s)| {
                        GIT_READ_ONLY.contains(&s)
                            || (s == "stash"
                                && matches!(
                                    w.get(i + 1).map(|x| x.as_str()),
                                    Some("list") | Some("show")
                                ))
                    })
                    .unwrap_or(false));
        // The transcripts are read by the engine to tell the human's words from an agent's: reading
        // them is fine, writing them is not.
        if names_transcripts(table, &segc, cwd) && (!prog_ro || real_redirect(&segc)) {
            return transcript_denial();
        }
        // A command that is one interpreter call (`python3 -c "a; b"`) is judged as a whole: the text
        // inside its quotes is not a list of shell commands.
        let whole_interpreter = words(&cleaned)
            .first()
            .map(|f| matches!(basename(f), "python" | "python3" | "node" | "ruby" | "perl"))
            .unwrap_or(false)
            && !real_redirect(&cleaned);
        if !whole_interpreter
            && names_protected(table, &segc, cwd)
            && (!prog_ro || real_redirect(&segc))
        {
            {
                return deny(
                    "change or script a protected path from the shell",
                    "factory records, the policy, gates, the guard table and the hook settings are changed by the engine and the human only; a script, an interpreter or a redirect that names them is refused",
                    &["Use an engine command instead: `bf record put`, `bf report`, `bf evidence run`.", "Reading a record with `cat` or `ls` is fine."],
                    "The shell command no longer names `factory/`, `.bf/` or the hook settings.",
                );
            }
        }

        // -- writes from a working copy stay in it
        if in_worker || in_cp {
            let root = table["root"].as_str().unwrap_or("");
            for t in write_targets(&segc, &w) {
                let abs = resolve(&t, cwd);
                let inside_root =
                    under(&abs, root).is_some() || repo_containing(table, &abs).is_some();
                let own = under(&abs, &here.base).is_some();
                if (in_cp && inside_root) || (in_worker && inside_root && !own) {
                    return deny(
                        "write outside your working copy",
                        &format!("you work in {}; `{t}` is somewhere else in the project", here.base),
                        &["Write only inside your working copy, in the files your unit may touch."],
                        "Every path you write is inside your working copy.",
                    );
                }
            }
        }
    }
    allow()
}

/// Decide one tool call. Pure apart from resolving symlinks on disk: input JSON and table JSON in,
/// decision out.
pub fn decide(input: &Value, table: &Value) -> Decision {
    let tool = input["tool_name"].as_str().unwrap_or("");
    let ti = &input["tool_input"];
    let cwd_raw = input["cwd"].as_str().unwrap_or("");
    let cwd = if cwd_raw.is_empty() {
        table["root"].as_str().unwrap_or("/").to_string()
    } else {
        resolve(cwd_raw, "/")
    };
    match tool {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let p = ti["file_path"]
                .as_str()
                .or_else(|| ti["notebook_path"].as_str())
                .unwrap_or("");
            if p.is_empty() {
                return allow();
            }
            check_write(table, &cwd, p)
        }
        "Read" | "Grep" | "Glob" | "NotebookRead" => {
            let p = ti["file_path"]
                .as_str()
                .or_else(|| ti["path"].as_str())
                .or_else(|| ti["notebook_path"].as_str())
                .unwrap_or("");
            let pat = ti["pattern"].as_str().unwrap_or("");
            let sd = state_dir(table);
            let at_base = !p.is_empty() && state_base(table) == Some(resolve(p, &cwd).as_str());
            if at_base
                || (!p.is_empty() && in_state(table, &resolve(p, &cwd)))
                || (!sd.is_empty() && pat.contains(sd))
                || (tool == "Glob"
                    && !p.is_empty()
                    && under(&resolve(p, &cwd), table["root"].as_str().unwrap_or("")).is_none()
                    && pat.contains("blackfactory"))
            {
                return state_denial();
            }
            allow()
        }
        "Bash" => {
            let cmd = ti["command"].as_str().unwrap_or("");
            check_bash(table, &cwd, cmd)
        }
        _ => allow(),
    }
}

// ---------------------------------------------------------------- finding the table

struct Loc {
    dir: PathBuf,
    table_text: Option<String>,
    has_factory: bool,
}

/// The project that owns `start`: the outermost folder above it that has a guard table or a `factory/`
/// folder. The outermost wins, so a table planted deeper (for example inside a working copy) is ignored.
fn find_exact(dir: &Path) -> Option<Loc> {
    let text = fs::read_to_string(dir.join(".bf/guard.json")).ok();
    let has_factory = dir.join("factory/log/events.jsonl").is_file();
    if text.is_some() || has_factory {
        Some(Loc {
            dir: dir.to_path_buf(),
            table_text: text,
            has_factory,
        })
    } else {
        None
    }
}

fn find_project(start: &Path) -> Option<Loc> {
    let mut found: Option<Loc> = None;
    let mut dir = Some(start);
    while let Some(d) = dir {
        let text = fs::read_to_string(d.join(".bf/guard.json")).ok();
        let has_factory = d.join("factory/log/events.jsonl").is_file();
        if text.is_some() || has_factory {
            found = Some(Loc {
                dir: d.to_path_buf(),
                table_text: text,
                has_factory,
            });
        }
        dir = d.parent();
    }
    found
}

/// The table, if it is whole and signed. A folder with a `factory/` must have one.
fn load_table(loc: &Loc) -> Result<Value, String> {
    let Some(text) = &loc.table_text else {
        return Err(if loc.has_factory {
            "the project has a factory/ folder but no guard table (.bf/guard.json)".into()
        } else {
            "no guard table".into()
        });
    };
    let mut table: Value =
        serde_json::from_str(text).map_err(|_| "the guard table is not valid JSON".to_string())?;
    let canon = fs::canonicalize(&loc.dir).unwrap_or_else(|_| loc.dir.clone());
    if table["root"].as_str().map(Path::new) != Some(canon.as_path()) {
        return Err("the guard table is empty or belongs to another folder (its `root` is not this project)".into());
    }
    let sig = table.as_object_mut().and_then(|o| o.remove("sig"));
    match sig.as_ref().and_then(|s| s.as_str()) {
        Some(sig) => {
            let key_path =
                Path::new(table["state_dir"].as_str().unwrap_or("/nonexistent")).join("key");
            let key = fs::read_to_string(key_path).map_err(|_| {
                "the guard table cannot be checked: the key in the state folder cannot be read"
                    .to_string()
            })?;
            let want = util::hmac_sha256_hex(
                key.trim().as_bytes(),
                serde_json::to_string(&table).unwrap_or_default().as_bytes(),
            );
            if want != sig {
                return Err(
                    "the guard table's signature does not match: it was changed by hand".into(),
                );
            }
        }
        None => {
            if loc.has_factory {
                return Err("the guard table is not signed".into());
            }
        }
    }
    // A table older than the newest one the engine wrote is a copy put back.
    let counter = fs::read_to_string(
        Path::new(table["state_dir"].as_str().unwrap_or("/nonexistent")).join("guard.gen"),
    )
    .ok()
    .and_then(|t| t.trim().parse::<u64>().ok())
    .unwrap_or(0);
    if table["gen"].as_u64().unwrap_or(0) < counter {
        return Err("the guard table is older than the newest one the engine wrote: an old copy was put back".into());
    }
    Ok(table)
}

pub fn render(d: &Decision, explicit_allow: bool) -> String {
    if !d.allow {
        return json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": d.reason,
        }})
        .to_string();
    }
    if !explicit_allow {
        // Say nothing: the harness's own permission prompts stay on.
        return "{}".into();
    }
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": d.reason,
    }})
    .to_string()
}

fn absolute_words(cmd: &str) -> Vec<String> {
    tokens(cmd)
        .into_iter()
        .filter(|t| t.starts_with('/') || t.starts_with("~/"))
        .take(8)
        .collect()
}

/// The whole hook: stdin text in, stdout text out. A damaged or unsigned table in a project denies.
/// `root` (from `--root` or `$BF_ROOT`) names the project when the harness knows it.
pub fn run_hook(stdin: &str, process_cwd: &Path, root: Option<&str>) -> String {
    let input: Value = match serde_json::from_str(stdin) {
        Ok(v) => v,
        Err(_) => {
            return render(
                &deny("run this tool call", "the guard could not read the hook payload as JSON", &["Check that the hook is configured as a PreToolUse command hook for `bf guard`."], "The payload on stdin is the hook's JSON."),
                false,
            )
        }
    };
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| process_cwd.to_path_buf());
    let cwd = fs::canonicalize(&cwd).unwrap_or(cwd);
    // Every place the call touches may belong to a project: the working directory, the file written,
    // and the absolute paths a shell command names.
    let mut starts: Vec<PathBuf> = Vec::new();
    // When the harness names the project, that folder is the project: a `factory/` folder further up
    // cannot take its place.
    if let Some(r) = root {
        starts.push(PathBuf::from(r));
    } else {
        starts.push(cwd.clone());
    }
    let ti = &input["tool_input"];
    if root.is_none() {
        for k in ["file_path", "notebook_path", "path"] {
            if let Some(p) = ti[k].as_str() {
                let abs = resolve(p, &cwd.to_string_lossy());
                starts.push(
                    Path::new(&abs)
                        .parent()
                        .map(|x| x.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("/")),
                );
            }
        }
        if let Some(c) = ti["command"].as_str() {
            for a in absolute_words(&clean(c)) {
                let abs = resolve(&a, &cwd.to_string_lossy());
                starts.push(
                    Path::new(&abs)
                        .parent()
                        .map(|x| x.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("/")),
                );
            }
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut explicit = false;
    let mut any = false;
    for s in starts {
        let found = if root.is_some() {
            find_exact(&fs::canonicalize(&s).unwrap_or(s.clone()))
        } else {
            find_project(&s)
        };
        let Some(loc) = found else {
            continue;
        };
        if seen.contains(&loc.dir) {
            continue;
        }
        seen.push(loc.dir.clone());
        any = true;
        let table = match load_table(&loc) {
            Ok(t) => t,
            Err(why) => {
                return render(
                    &deny("run this tool call", &why, &["Ask the human to run any state-changing `bf` command in the project, which writes a fresh signed table.", "Or run `bf doctor` to see what is wrong."], "`bf doctor` reports `guard table: ok`."),
                    false,
                )
            }
        };
        let d = decide(&input, &table);
        if !d.allow {
            return render(&d, false);
        }
        explicit |= table["allow"].as_str() == Some("allow");
    }
    if !any {
        // Not a factory project: nothing to guard.
        return "{}".into();
    }
    render(&allow(), explicit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Value {
        json!({
            "v": 1, "root": "/p", "allow": "abstain",
            "protected": ["factory/", ".bf/", ".claude/settings.json"],
            "state_dir": "/state/abc123def4567890", "project_id": "abc123def4567890",
            "worker_bf_allow": ["report", "evidence", "status"],
            "checkpoint_bf_allow": ["evidence", "status"],
            "worktrees": [
                {"kind":"unit","id":"U-1","path":"/p/.bf/work/U-1","files":["src/a.rs","docs/"]},
                {"kind":"checkpoint","id":"K-1","path":"/p/.bf/work/K-1","files":[]}
            ],
            "main_branch": "main", "branch_prefix": "bf/"
        })
    }

    fn call(tool: &str, input: Value, cwd: &str) -> Decision {
        decide(
            &json!({"tool_name": tool, "tool_input": input, "cwd": cwd}),
            &table(),
        )
    }

    fn bash(cmd: &str, cwd: &str) -> bool {
        call("Bash", json!({"command": cmd}), cwd).allow
    }

    #[test]
    fn writes_to_records_are_denied() {
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/factory/records/R-1.toml"}),
                "/p"
            )
            .allow
        );
        assert!(!call("Edit", json!({"file_path": "factory/policy.toml"}), "/p").allow);
        assert!(!call("Write", json!({"file_path": "./factory/policy.toml"}), "/p").allow);
        assert!(
            !call(
                "Write",
                json!({"file_path": "src/../factory/policy.toml"}),
                "/p"
            )
            .allow
        );
        assert!(!call("Write", json!({"file_path": "/p/.bf/guard.json"}), "/p").allow);
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/.claude/settings.json"}),
                "/p"
            )
            .allow
        );
        assert!(
            !call(
                "Write",
                json!({"file_path": "/state/abc123def4567890/head.json"}),
                "/p"
            )
            .allow
        );
        assert!(call("Write", json!({"file_path": "/p/src/main.rs"}), "/p").allow);
        assert!(call("Write", json!({"file_path": "/tmp/x"}), "/p").allow);
    }

    #[test]
    fn dotdot_cannot_escape_a_scope() {
        let d = call(
            "Write",
            json!({"file_path": "/p/.bf/work/U-1/src/../../../../factory/x"}),
            "/p/.bf/work/U-1",
        );
        assert!(!d.allow);
    }

    #[test]
    fn unit_scope() {
        let cwd = "/p/.bf/work/U-1";
        assert!(
            call(
                "Write",
                json!({"file_path": "/p/.bf/work/U-1/src/a.rs"}),
                cwd
            )
            .allow
        );
        assert!(call("Write", json!({"file_path": "docs/x/y.md"}), cwd).allow);
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/.bf/work/U-1/src/b.rs"}),
                cwd
            )
            .allow
        );
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/.bf/work/U-1/factory/records/R-1.toml"}),
                cwd
            )
            .allow
        );
        assert!(!call("Write", json!({"file_path": "/p/src/a.rs"}), cwd).allow);
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/.bf/work/K-1/src/a.rs"}),
                cwd
            )
            .allow
        );
    }

    #[test]
    fn commands() {
        assert!(!bash("bf human-turn", "/p"));
        assert!(!bash("echo hi && ./target/debug/bf human-turn < x", "/p"));
        assert!(!bash("sh -c \"printf x | bf human-turn\"", "/p"));
        assert!(!bash("B=bf; $B human-turn < p.json", "/p"));
        assert!(!bash("b\\f human-turn < p.json", "/p"));
        assert!(!bash("git push origin main", "/p"));
        assert!(!bash("g\"it\" push", "/p"));
        assert!(!bash("git -C /p push", "/p"));
        assert!(!bash("git merge bf/K-1", "/p"));
        assert!(
            !bash("git -C /p merge bf/U-1", "/p/.bf/work/U-1"),
            "judged by the target, not the cwd"
        );
        assert!(bash("git merge origin/x", "/p/.bf/work/U-1"));
        assert!(!bash("git fetch . bf/K-1:main", "/p"));
        assert!(!bash("git pull", "/p"));
        assert!(!bash("git reset --hard", "/p"));
        assert!(bash("git reset HEAD notes.txt", "/p"));
        assert!(
            bash("git commit -m x", "/p"),
            "committing is the user's business; the main-branch watch looks at what lands"
        );
        assert!(!bash("git checkout bf/K-1", "/p"));
        assert!(!bash("git stash", "/p"));
        assert!(!bash("git clean -fd", "/p"));
        assert!(!bash("git checkout -- factory", "/p"));
        assert!(!bash("git rebase main", "/p"));
        assert!(!bash("git cherry-pick abc", "/p"));
        assert!(!bash("git update-ref refs/heads/main abc", "/p"));
        assert!(!bash("git branch -D bf/K-1", "/p"));
        assert!(!bash("git branch -f main abc", "/p"));
        assert!(bash("git status && cargo test", "/p"));
        assert!(bash("git fetch origin", "/p"));
        assert!(!bash("bf gate approve plan", "/p/.bf/work/U-1"));
        assert!(!bash("bf brief U-1 --role builder", "/p/.bf/work/U-1"));
        assert!(!bash("bf -C /p status", "/p/.bf/work/U-1"));
        assert!(
            bash("bf gate approve plan", "/p"),
            "the conductor may approve once a human turn exists"
        );
        assert!(bash("bf report U-1 --result built", "/p/.bf/work/U-1"));
        assert!(bash(
            "bf record put finding --file f.toml",
            "/p/.bf/work/U-1"
        ));
        assert!(!bash(
            "bf record put requirement --file f.toml",
            "/p/.bf/work/U-1"
        ));
        assert!(
            !bash("bf report U-1 --result built", "/p/.bf/work/K-1"),
            "a checkpoint working copy does not report units"
        );
        assert!(bash("bf evidence run X-3 -- ls", "/p/.bf/work/K-1"));
        assert!(!bash("cd /p && bf next", "/p/.bf/work/U-1"));
    }

    #[test]
    fn shell_writes_to_protected_paths() {
        for cmd in [
            "echo x > factory/records/R-1.toml",
            "echo x > ./factory/policy.toml",
            "echo x > a/../factory/policy.toml",
            "echo x > /p/factory/policy.toml",
            "rm -rf .bf",
            "rm -rf ./factory",
            "sed -i s/a/b/ factory/policy.toml",
            "sh -c \"echo x > factory/policy.toml\"",
            "find . -name events.jsonl -delete ; rm -r factory/log",
            "cat <<EOF > factory/policy.toml\nx\nEOF",
            "cat /state/abc123def4567890/key",
            "cd /state/abc123def4567890 && cat key",
            "cat .bf/guard.json > /tmp/x ; echo y > .bf/guard.json",
        ] {
            assert!(!bash(cmd, "/p"), "should be denied: {cmd}");
        }
        assert!(
            bash("cat factory/records/R-1.toml", "/p"),
            "reading records is fine"
        );
        assert!(bash("ls factory/records", "/p"));
        // Interpreters and `find` are left to the harness's own permission checks.
        assert!(bash("find . -name '*.toml' -path './factory/*'", "/p"));
        assert!(bash(
            "python3 -c \"import tomllib; print(tomllib.load(open('factory/records/R-1.toml','rb')))\"",
            "/p"
        ));
        assert!(bash("git add factory/ && git status", "/p"));
        assert!(bash("cp a.txt /p/.bf/work/U-1/a.txt", "/p"));
        assert!(
            bash("echo x > src/factory/mod.rs", "/p/.bf/work/U-1"),
            "a source folder called factory is not protected"
        );
        assert!(
            !call(
                "Read",
                json!({"file_path": "/state/abc123def4567890/key"}),
                "/p"
            )
            .allow
        );
        assert!(
            !call(
                "Grep",
                json!({"path": "/state/abc123def4567890", "pattern": "."}),
                "/p"
            )
            .allow
        );
        assert!(
            call(
                "Read",
                json!({"file_path": "/p/.bf/briefs/U-1-builder.md"}),
                "/p"
            )
            .allow
        );
    }

    #[test]
    fn a_worker_writes_only_in_its_own_working_copy() {
        let w = "/p/.bf/work/U-1";
        assert!(!bash("echo x > /p/src/a.txt", w));
        assert!(!bash("echo x > ../../../src/a.txt", w));
        assert!(!bash("cp a.txt /p/.bf/work/U-2/a.txt", w));
        assert!(bash("echo x > /tmp/elsewhere.txt", w));
        assert!(bash("echo x > src/a.rs", w));
        assert!(
            !bash("echo x > src/a.rs", "/p/.bf/work/K-1"),
            "a checkpoint working copy is not written to"
        );
    }

    #[test]
    fn drafts_and_the_idea_brief_are_writable() {
        assert!(call("Write", json!({"file_path": "/p/.bf/drafts/r1.toml"}), "/p").allow);
        assert!(call("Write", json!({"file_path": "/p/factory/idea.md"}), "/p").allow);
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/factory/idea.md.bak"}),
                "/p"
            )
            .allow
        );
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/factory/policy.toml"}),
                "/p"
            )
            .allow
        );
        assert!(
            !call(
                "Write",
                json!({"file_path": "/p/.bf/drafts/x"}),
                "/p/.bf/work/U-1"
            )
            .allow
        );
        assert!(bash("cat > .bf/drafts/r1.toml", "/p"));
        assert!(bash("echo hi > factory/idea.md", "/p"));
        assert!(!bash("echo hi > factory/policy.toml", "/p"));
    }

    #[test]
    fn hook_output_shape() {
        let d = deny("a", "b", &["c"], "d");
        let out: Value = serde_json::from_str(&render(&d, false)).unwrap();
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        // What is not denied says nothing, unless the policy asks for an explicit allow.
        assert_eq!(render(&allow(), false), "{}");
        let a: Value = serde_json::from_str(&render(&allow(), true)).unwrap();
        assert_eq!(a["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn bad_payload_is_denied_not_allowed() {
        let out = run_hook("not json", Path::new("/"), None);
        assert!(out.contains("deny"));
    }
}
