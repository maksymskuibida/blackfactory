//! `bf guard`: the per-tool-call hook.
//!
//! It reads the tool call from stdin and the project's guard table, `.bf/guard.json`, and, to check the
//! table's signature, the key in the project's state folder. It starts no process and calls no git.
//! Paths are resolved against the disk: each symbolic link is read and followed, one whose target does
//! not exist included, and the part that exists is then put in its canonical form.
//!
//! The guard stops mistakes, drift and casual cheating and makes deliberate cheating visible. It cannot
//! stop an agent that has an unrestricted shell under the same account: see "What the engine does not
//! stop" in docs/engine.md.

use crate::refusal::Refusal;
use crate::util;
use serde_json::{json, Value};
use std::collections::HashMap;
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

/// How many links in a row are followed before the path is judged where it stands.
const LINK_BOUND: usize = 40;

/// Follow symbolic links in an absolute, normalised path by reading each one, from the outermost folder
/// inwards, so a link whose target does not exist is followed too. A relative target is taken from the
/// folder that holds the link. Stops after `LINK_BOUND` links.
fn follow_links(lex: &str) -> String {
    let mut cur = lex.to_string();
    for _ in 0..LINK_BOUND {
        let parts: Vec<&str> = cur.split('/').filter(|s| !s.is_empty()).collect();
        let mut prefix = String::new();
        let mut hit: Option<(usize, String, String)> = None;
        for (i, seg) in parts.iter().enumerate() {
            let folder = prefix.clone();
            prefix.push('/');
            prefix.push_str(seg);
            let is_link = fs::symlink_metadata(&prefix)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if is_link {
                if let Ok(t) = fs::read_link(&prefix) {
                    hit = Some((i, folder, t.to_string_lossy().to_string()));
                    break;
                }
            }
        }
        let Some((i, folder, target)) = hit else {
            return cur;
        };
        let tail = parts[i + 1..].join("/");
        let base = if target.starts_with('/') {
            target
        } else {
            format!("{folder}/{target}")
        };
        cur = normalize(&format!("{base}/{tail}"));
    }
    cur
}

/// Absolute, normalised, and with symlinks followed, an unresolved one included.
fn resolve(p: &str, cwd: &str) -> String {
    let lex = follow_links(&absolutize(p, cwd));
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
    /// Paths that existed when a checkpoint fix opened: a fix may not overwrite them unless a unit lists them.
    existing: Vec<String>,
    /// A sync of the main branch into this checkpoint is waiting for its resolver or reviewer.
    sync: bool,
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
            existing: w["existing"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            sync: w["sync"].as_bool().unwrap_or(false),
        },
        None => {
            // A working copy of a unit that has landed is no longer in the table. It is still a unit's
            // working copy (nothing may be written in it), not the main checkout.
            let wp = table["work_prefix"].as_str().unwrap_or("");
            if !wp.is_empty() {
                if let Some(rel) = under(abs, wp) {
                    if let Some(name) = rel.split('/').next().filter(|n| !n.is_empty()) {
                        if name.starts_with("K-") && name.ends_with("-review") {
                            // A checkpoint reviewer's own copy: it builds, runs and probes there.
                            return Zone {
                                kind: "review",
                                id: name,
                                base: format!("{}/{}", wp.trim_end_matches('/'), name),
                                files: vec![],
                                existing: vec![],
                                sync: false,
                            };
                        }
                        if !name.starts_with("K-") {
                            return Zone {
                                kind: "unit",
                                id: name,
                                base: format!("{}/{}", wp.trim_end_matches('/'), name),
                                files: vec![],
                                existing: vec![],
                                sync: false,
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
                existing: vec![],
                sync: false,
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

/// Protection inside a unit's or checkpoint's working copy. In a project folder the working copy holds a
/// copy of the factory's own files, which are protected; in a workspace it is a copy of a code
/// repository, whose own `factory/` or `.claude/` folders are its source like any other.
fn is_protected_in_wc(table: &Value, rel: &str) -> bool {
    if table["workspace"].as_bool() == Some(true) {
        return false;
    }
    is_protected_rel(table, rel)
}

/// Where the conductor and planners may write inside the protected folders.
fn is_open_rel(rel: &str) -> bool {
    rel.starts_with(".bf/drafts/")
        || rel == ".bf/drafts"
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
    if is_protected_in_wc(table, rel) || rel == ".git" || rel.starts_with(".git/") {
        return deny(
            &format!("write {rel} in {}", zone.id),
            "that path belongs to the engine, the human or git, not to a unit's work",
            &["Change only files inside the unit's scope."],
            "The path is inside the unit's `files` list.",
        );
    }
    if zone.kind == "review" {
        // Its check runs become the checkpoint's receipts, so only the reviewer working in it writes there.
        if here.kind == "review" && here.id == zone.id {
            return allow();
        }
        return deny(
            &format!("write {rel} in {}", zone.id),
            "that is the checkpoint reviewer's own copy; only the reviewer working inside it writes there",
            &["Write inside your own working copy.", "A change for the checkpoint goes through a unit's working copy and `bf unit land`."],
            "The path starts with your own working copy's folder.",
        );
    }
    if zone.kind == "checkpoint"
        && zone.files.iter().any(|g| util::path_matches(g, rel))
        && !zone.existing.iter().any(|e| e == rel)
    {
        // An open sync or checkpoint fix: the builder writes the files the engine named, and only those.
        return allow();
    }
    if zone.kind == "checkpoint" {
        return deny(
            &format!("write {rel} in the checkpoint working copy {}", zone.id),
            "a checkpoint's content changes only by landing units, by the resolver of an open sync, or by the fix of a failed checkpoint inside the files of its units",
            &["Make the change in a unit's working copy and land the unit.", "A checkpoint fix may change only the files of the checkpoint's units and new test files beside them; anything more is a plan gap the human decides by amendment."],
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
//
// The shell rules work on the parsed command (see `shellparse`): the programs, their arguments, the
// redirects and the working directory after each `cd`. They ask where a command would write, delete or
// run, each resolved to a path, and never whether a word occurs in the text. Heredoc bodies and quoted
// strings are data.

fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
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

/// The main checkouts of the code repositories that are not the project folder.
fn other_repos(table: &Value) -> Vec<String> {
    let root = table["root"].as_str().unwrap_or("");
    table["repos"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["path"].as_str())
                .filter(|p| *p != root && !p.is_empty())
                .map(|p| p.trim_end_matches('/').to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Is the path inside the git folder of a code repository (its hooks, its config and the rest of it)?
fn in_repo_git(table: &Value, abs: &str) -> bool {
    other_repos(table)
        .iter()
        .any(|p| under(abs, p).is_some_and(|rel| rel == ".git" || rel.starts_with(".git/")))
}

/// Does a path that is protected (in the project root or in any working copy) get named?
fn is_protected_abs(table: &Value, abs: &str) -> bool {
    if in_repo_git(table, abs) {
        return true;
    }
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
                if is_protected_in_wc(table, rel) {
                    return true;
                }
            }
        }
    }
    false
}

/// The state folder or the folder above it.
fn in_state_or_base(table: &Value, abs: &str) -> bool {
    in_state(table, abs)
        || state_base(table)
            .map(|b| under(abs, b).is_some())
            .unwrap_or(false)
}

/// Programs that run code given as text: a script on standard input, after `-c` or `-e`, or a file.
fn is_interpreter(b: &str) -> bool {
    b.starts_with("python")
        || matches!(
            b,
            "node"
                | "nodejs"
                | "deno"
                | "bun"
                | "ruby"
                | "perl"
                | "php"
                | "lua"
                | "awk"
                | "gawk"
                | "mawk"
                | "Rscript"
        )
}

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];

/// Programs that write or delete every path they are given.
const WRITERS: &[&str] = &[
    "rm", "tee", "chmod", "chown", "truncate", "touch", "mkdir", "rmdir", "unlink", "shred",
    "patch", "mkfifo", "mknod", "mktemp", "sponge",
];
/// The compressors write a file next to the one they are given (or replace it), unless they print to
/// standard output.
const COMPRESSORS: &[&str] = &[
    "gzip",
    "bzip2",
    "xz",
    "zstd",
    "lzma",
    "compress",
    "gunzip",
    "bunzip2",
    "unxz",
    "unzstd",
    "uncompress",
    "lz4",
    "brotli",
];
/// The line editors and full screen editors write the files they are given.
const EDITORS: &[&str] = &[
    "ed", "ex", "vi", "vim", "nvim", "view", "nano", "pico", "emacs", "red",
];
/// Programs whose last argument (or `-t dir`) is where they write.
const COPIERS: &[&str] = &["cp", "ln", "install", "rsync"];

/// The words of a text that look like a real path: absolute, from home, or climbing out of the folder.
/// Plain words, relative names and source text are not paths.
fn text_paths(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || "\"'`()<>|&;=,{}[]:".contains(c))
        .filter(|t| {
            t.starts_with('/')
                || *t == "~"
                || t.starts_with("~/")
                || t.starts_with("$HOME/")
                || (t.starts_with("../") && !t.trim_end_matches('/').ends_with(".."))
        })
        .map(|t| t.to_string())
        .collect()
}

/// Does ruby code delete a file by what it is handed (a file object a read call returned): `File.delete(f)`,
/// `File.unlink(f)`, `FileUtils.rm(f)`, `rm_f`, `rm_r`, `rm_rf`? Only as a call on those owners: a hash key
/// `'delete'` or `list.delete(x)` is not a file delete.
///
/// Quoted text is data and is not read (`puts 'use File.delete(f)'`), except the code of a `#{ }` in a
/// double quoted string. A call is a call on its owner by `.` or `::`; `File.send(:delete, f)` and
/// `File.method(:delete)` name the same call; after `include FileUtils` (or `extend`) a bare `rm(f)` is one.
fn ruby_file_delete(code: &str) -> bool {
    const FILE_CALLS: &[&str] = &["delete", "unlink", "remove_file"];
    const UTIL_CALLS: &[&str] = &[
        "rm",
        "rm_f",
        "rm_r",
        "rm_rf",
        "remove",
        "remove_file",
        "remove_dir",
        "remove_entry",
        "remove_entry_secure",
        "safe_unlink",
    ];
    const INDIRECT: &[&str] = &[
        "send",
        "public_send",
        "__send__",
        "method",
        "public_method",
        "instance_method",
    ];
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let raw: Vec<char> = code.chars().collect();
    // The same text with the content of quoted strings blanked, one character for one.
    let mut chars = raw.clone();
    let mut i = 0;
    while i < chars.len() {
        let q = chars[i];
        if q != '"' && q != '\'' {
            i += 1;
            continue;
        }
        i += 1;
        while i < chars.len() && chars[i] != q {
            if chars[i] == '\\' {
                chars[i] = ' ';
                i += 1;
                if i < chars.len() {
                    chars[i] = ' ';
                    i += 1;
                }
                continue;
            }
            if q == '"' && chars[i] == '#' && chars.get(i + 1) == Some(&'{') {
                // Code inside an interpolation stays code.
                let mut depth = 0;
                while i < chars.len() {
                    match chars[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                continue;
            }
            chars[i] = ' ';
            i += 1;
        }
        i += 1;
    }
    let owner_of = |end: usize| -> String {
        chars[..end]
            .iter()
            .rev()
            .take_while(|c| is_word(**c))
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    };
    // `include FileUtils`: its methods are then called bare.
    let mut bare_utils = false;
    let mut i = 0;
    while i < chars.len() {
        if !is_word(chars[i]) || (i > 0 && is_word(chars[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        if matches!(name.as_str(), "include" | "extend") {
            let mut k = i;
            while k < chars.len()
                && (chars[k].is_whitespace() || chars[k] == ':' || chars[k] == '(')
            {
                k += 1;
            }
            let w: String = chars[k..].iter().take_while(|c| is_word(**c)).collect();
            if w == "FileUtils" {
                bare_utils = true;
            }
        }
    }
    let mut i = 0;
    while i < chars.len() {
        if !is_word(chars[i]) || (i > 0 && is_word(chars[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        let dot = start >= 1 && chars[start - 1] == '.';
        let colons = start >= 2 && chars[start - 1] == ':' && chars[start - 2] == ':';
        let called = match chars.get(i) {
            Some('(') => true,
            Some(c) => c.is_whitespace(),
            None => false,
        };
        if !dot && !colons {
            // A bare call of a `FileUtils` method after it was included.
            if bare_utils && called && UTIL_CALLS.contains(&name.as_str()) {
                return true;
            }
            continue;
        }
        let owner = owner_of(start - if dot { 1 } else { 2 });
        let file = owner == "File";
        let utils = owner == "FileUtils";
        if !file && !utils {
            continue;
        }
        let names: &[&str] = if file { FILE_CALLS } else { UTIL_CALLS };
        if called && names.contains(&name.as_str()) {
            return true;
        }
        // `File.send(:delete, f)`, `File.method('delete')`: the name is read from the raw text.
        if INDIRECT.contains(&name.as_str()) {
            let mut k = i;
            while k < raw.len()
                && (raw[k].is_whitespace()
                    || raw[k] == '('
                    || raw[k] == ':'
                    || raw[k] == '"'
                    || raw[k] == '\'')
            {
                k += 1;
            }
            let w: String = raw[k..].iter().take_while(|c| is_word(**c)).collect();
            if names.contains(&w.as_str()) {
                return true;
            }
        }
    }
    false
}

/// A string literal of interpreter code that can stand for a path held in a variable: a word with no
/// space that is not a mode or a module name. It is judged as a write when the code deletes or changes a
/// file by whatever it is handed (`f = File.open('a'); File.delete(f)`, `fs.rmSync(p)`).
fn literals_for_unnamed_writes(code: &str, dir: &Option<String>) -> Vec<String> {
    // The word before the open call that the position `before` ends inside.
    fn enclosing_call(before: &str) -> Option<String> {
        let c: Vec<char> = before.chars().collect();
        let mut depth = 0i32;
        for k in (0..c.len()).rev() {
            match c[k] {
                ')' => depth += 1,
                '(' if depth > 0 => depth -= 1,
                '(' => {
                    let mut w: Vec<char> = c[..k]
                        .iter()
                        .rev()
                        .take_while(|ch| ch.is_alphanumeric() || **ch == '_')
                        .copied()
                        .collect();
                    w.reverse();
                    return Some(w.into_iter().collect());
                }
                _ => {}
            }
        }
        None
    }
    let keep = |lit: &str, before: &str| -> bool {
        // A printed word (`'done'`, `'ok.'`) is not a path: it holds a `/`, a name after a dot (a file
        // extension), or names an entry on disk.
        let ext = lit.rsplit_once('.').map_or("", |x| x.1);
        // A name after a dot counts as an extension when it holds a letter (`v1.2` and `0.2.1` are
        // version numbers), or when the literal is handed to a file call.
        const FILE_CALLS: &[&str] = &[
            "open", "opensync", "remove", "unlink", "unlinksync", "rename", "renamesync", "replace",
            "rmsync", "rmtree", "rmdir", "truncate", "truncatesync", "writefile", "writefilesync",
            "appendfile", "appendfilesync", "readfile", "readfilesync", "delete", "copy", "copy2",
            "copyfile", "copyfilesync", "move", "chmod", "chown", "file", "path", "symlink", "link",
        ];
        let to_file_call = enclosing_call(before)
            .is_some_and(|w| FILE_CALLS.contains(&w.to_lowercase().as_str()));
        let path_like = lit.contains('/')
            || (!ext.is_empty()
                && ext.chars().all(|c| c.is_alphanumeric())
                && (ext.chars().any(|c| c.is_alphabetic()) || to_file_call))
            || target_abs(lit, dir).is_some_and(|a| std::path::Path::new(&a).exists());
        path_like && {
            let b = before.trim_end().trim_end_matches('(').trim_end();
            let loads = ["require", "require_relative", "import", "from", "load"]
                .iter()
                .any(|w| b.ends_with(w));
            !lit.is_empty()
                && lit.len() <= 512
                && !lit
                    .chars()
                    .any(|c| c.is_whitespace() || c == '#' || c == '%')
                && !lit.starts_with('/')
                && !lit.starts_with('~')
                && !lit.starts_with("$HOME")
                && !lit.starts_with("../")
                && !(lit.len() <= 3 && lit.chars().all(|c| "rwabxt+U".contains(c)))
                && !loads
        }
    };
    let mut out: Vec<String> = string_literals(code)
        .into_iter()
        .filter(|(lit, before, _)| {
            // A component of a path-joining call is not a path by itself; the joined path is judged below.
            enclosing_call(before).is_none_or(|w| w != "join") && keep(lit, before)
        })
        .map(|l| l.0)
        .collect();
    // `os.path.join('src', 'a.rs')`, `path.join(...)`, `File.join(...)`: arguments that are all literals
    // make one path.
    let mut from = 0;
    while let Some(at) = code[from..].find("join(") {
        let open = from + at + 5;
        from = open;
        let Some(len) = code[open..].find(')') else {
            break;
        };
        let parts: Vec<&str> = code[open..open + len].split(',').map(str::trim).collect();
        let lits: Vec<&str> = parts
            .iter()
            .filter_map(|p| {
                let q = p.chars().next()?;
                (p.len() >= 2 && (q == '\'' || q == '"') && p.ends_with(q))
                    .then(|| &p[1..p.len() - 1])
            })
            .collect();
        if !lits.is_empty() && lits.len() == parts.len() {
            let joined = lits.join("/");
            if keep(&joined, "") {
                out.push(joined);
            }
        }
    }
    out
}

/// The code with each delete call that is given a string literal first (`os.remove('a')`,
/// `fs.rmSync('a')`) renamed, so the call is not read as one that names no path: the literal is its path.
fn mask_literal_deletes(code: &str) -> String {
    const NAMES: &[&str] = &[
        "remove",
        "unlink",
        "unlinksync",
        "rmsync",
        "rm",
        "rmdir",
        "rmdirsync",
        "rmtree",
    ];
    let c: Vec<char> = code.chars().collect();
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if !is_word(c[i]) {
            out.push(c[i]);
            i += 1;
            continue;
        }
        let s = i;
        while i < c.len() && is_word(c[i]) {
            i += 1;
        }
        let w: String = c[s..i].iter().collect();
        let mut k = i;
        while k < c.len() && c[k].is_whitespace() {
            k += 1;
        }
        let literal_call = c.get(k) == Some(&'(') && {
            let mut m = k + 1;
            while m < c.len() && c[m].is_whitespace() {
                m += 1;
            }
            matches!(c.get(m), Some('\'' | '"'))
        };
        if literal_call && NAMES.contains(&w.to_lowercase().as_str()) {
            out.push_str("named_literal_delete");
        } else {
            out.push_str(&w);
        }
    }
    out
}

/// A write call whose path is a variable: `open(p, 'w')`, `fs.writeFileSync(p, x)`, `File.write(p, x)`.
/// The path is a literal held elsewhere in the code, so every path-like literal is judged as written.
fn write_through_variable(code: &str) -> bool {
    const NODE: &[&str] = &[
        "writefilesync",
        "appendfilesync",
        "writefile",
        "appendfile",
        "createwritestream",
        "truncatesync",
    ];
    let c: Vec<char> = code.chars().collect();
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let mut i = 0;
    while i < c.len() {
        if !is_word(c[i]) {
            i += 1;
            continue;
        }
        let s = i;
        while i < c.len() && is_word(c[i]) {
            i += 1;
        }
        let w: String = c[s..i].iter().collect::<String>().to_lowercase();
        if c.get(i) != Some(&'(') {
            continue;
        }
        let mut m = i + 1;
        while m < c.len() && c[m].is_whitespace() {
            m += 1;
        }
        let before: String = c[..s].iter().collect();
        // A copy or a move writes its destination: the second argument, given as a name.
        let on_shutil = before.ends_with("shutil.");
        let on_utils = before.ends_with("FileUtils.") || before.ends_with("FileUtils::");
        let copies = matches!(w.as_str(), "copyfilesync" | "copyfile" | "cpsync" | "copy2")
            || (on_shutil && matches!(w.as_str(), "copy" | "move" | "copytree"))
            || (on_utils && matches!(w.as_str(), "cp" | "mv" | "copy" | "move" | "cp_r"));
        if copies {
            let end = c[i + 1..]
                .iter()
                .position(|ch| *ch == ')')
                .map_or(c.len(), |e| i + 1 + e);
            let args: String = c[i + 1..end].iter().collect();
            let dest = args.split(',').nth(1).map(str::trim).unwrap_or("");
            if dest
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
            {
                return true;
            }
        }
        // `os.replace(src, dst)` renames like `os.rename`: either argument given as a name is a path held
        // in a variable.
        if w == "replace" && before.ends_with("os.") {
            let end = c[i + 1..]
                .iter()
                .position(|ch| *ch == ')')
                .map_or(c.len(), |e| i + 1 + e);
            let args: String = c[i + 1..end].iter().collect();
            if args.split(',').any(|a| {
                a.trim()
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
            }) {
                return true;
            }
        }
        // The first argument is a name, not a literal.
        if !c.get(m).is_some_and(|ch| ch.is_alphabetic() || *ch == '_') {
            continue;
        }
        let owner_file = before.ends_with("File.") || before.ends_with("IO.");
        let hit = if NODE.contains(&w.as_str()) {
            true
        } else if matches!(w.as_str(), "write" | "binwrite") {
            owner_file
        } else if w == "open" || w == "opensync" {
            // A mode given as a string after the path: `open(p, 'w')`.
            let end = c[m..]
                .iter()
                .position(|ch| *ch == ')')
                .map_or(c.len(), |e| m + e);
            let args: String = c[m..end].iter().collect();
            string_literals(&args).iter().any(|(l, _, _)| {
                !l.is_empty()
                    && l.len() <= 3
                    && l.chars().all(|ch| "rwabxt+U".contains(ch))
                    && l.chars().any(|ch| matches!(ch, 'w' | 'a' | 'x' | '+'))
            })
        } else {
            false
        };
        if hit {
            return true;
        }
    }
    false
}

/// Can this code write, delete or change a file? A call that does, a mode that opens for writing, or a
/// redirect string. Code that only reads a path it names is not a write of it.
fn code_may_write(code: &str) -> bool {
    if ruby_file_delete(code) || write_through_variable(code) {
        return true;
    }
    // Names are matched as whole words: `json.dumps` reads, `json.dump` writes to a file.
    const PREFIXES: &[&str] = &[
        "write", "append", "exec", "spawn", "popen", "unlink", "rmtree", "rmdir", "rmsync",
        "mkdir", "makedirs", "truncate", "chmod", "chown", "rename", "remove", "copy", "symlink",
        "hardlink", "extract", "unpack", "sysopen", "touch", "o_wronly", "o_rdwr", "o_trunc",
        "o_creat", "o_append",
    ];
    const EXACT: &[&str] = &[
        "move",
        "link",
        "rm",
        "linksync",
        "dump",
        "system",
        "subprocess",
        "shell",
        "connect",
        "replacesync",
    ];
    // Output streams are not files: a call on them, or a dump handed one, writes nothing to disk.
    const STREAMS: &[&str] = &[
        "sys.stdout",
        "sys.stderr",
        "process.stdout",
        "process.stderr",
    ];
    let to_stream = STREAMS.iter().any(|s| code.contains(s));
    let mut prev = "";
    let mut copy_module = false;
    for tok in code.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')) {
        let before = prev;
        prev = tok;
        // `os.replace` renames a file; `text.replace` does not.
        if tok == "os.replace" || tok.ends_with(".os.replace") {
            return true;
        }
        if STREAMS.iter().any(|s| tok.starts_with(s)) {
            continue;
        }
        // The `copy` module copies objects in memory; `shutil.copy` copies files.
        if (tok == "copy" && before == "import") || (copy_module && tok.starts_with("copy.")) {
            copy_module |= tok == "copy";
            continue;
        }
        for w in tok.split('.') {
            let l = w.to_lowercase();
            if l.starts_with("removeprefix") || l.starts_with("removesuffix") {
                continue;
            }
            // `import copy` names a module; a database cursor's `execute` runs a query.
            if l.starts_with("execute") {
                continue;
            }
            if l == "dump" && to_stream {
                continue;
            }
            if EXACT.contains(&l.as_str()) || PREFIXES.iter().any(|p| l.starts_with(p)) {
                return true;
            }
        }
    }
    // The quoted pieces of the code, one scan, no copies of the text around them.
    let c: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '"' || c[i] == '\'' {
            let q = c[i];
            if let Some(e) = c[i + 1..].iter().position(|x| *x == q) {
                let lit: String = c[i + 1..i + 1 + e].iter().take(8).collect();
                let t = lit.trim();
                // A mode follows the path, after a comma (`open(p, 'a')`), not the open parenthesis
                // (`s.replace('a', 'b')`).
                let after_comma = c[..i].iter().rev().find(|x| !x.is_whitespace()) == Some(&',');
                if t.starts_with('>')
                    || (after_comma
                        && !t.is_empty()
                        && e <= 3
                        && t.chars().all(|c| "rwabxt+U".contains(c))
                        && t.chars().any(|c| matches!(c, 'w' | 'a' | 'x' | '+')))
                {
                    return true;
                }
                i += e + 2;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// Can the code change a file by a means that names no path literal: a call that deletes, renames or
/// changes a file by whatever it is handed (it may be the object an `open` returned, `f.name`), a shell-out,
/// or a path taken from an object? Such code keeps every path it mentions judged as a write.
fn writes_without_literal(code: &str) -> bool {
    if ruby_file_delete(code) || write_through_variable(code) {
        return true;
    }
    const RISKY: &[&str] = &[
        "unlink",
        "unlinksync",
        "remove",
        "rm",
        "rmtree",
        "rmdir",
        "rmdirsync",
        "rmsync",
        "chmod",
        "chown",
        "chgrp",
        "lchmod",
        "fchmod",
        "rename",
        "renamesync",
        "truncate",
        "ftruncate",
        "utime",
        "symlink",
        "link",
        "hardlink",
        "move",
        "system",
        "popen",
        "subprocess",
        "shell",
        "spawn",
        "spawnsync",
        "run",
        "call",
        "check_call",
        "check_output",
        "name",
        "sysopen",
        "fdopen",
        "fileno",
    ];
    // An attribute `.path` of an opened object (a node stream, a ruby file) returns the path it was
    // opened with, like `.name`. `os.path.x`, `sys.path` and the `path` module are not that.
    let mut from = 0;
    while let Some(k) = code[from..].find(".path") {
        let at = from + k;
        let end = at + 5;
        from = end;
        let next = code[end..].chars().next();
        if next.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let owner: String = code[..at]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if matches!(
            owner.as_str(),
            "os" | "sys" | "posixpath" | "ntpath" | "process"
        ) {
            continue;
        }
        return true;
    }
    // Names that are also everyday data keys or list methods: they count only as a call, and
    // `name` and `fileno` only as an attribute (`f.name`), never as a dict key or a bare word.
    const AMBIGUOUS: &[&str] = &[
        "remove", "rm", "move", "link", "call", "run", "shell", "rename", "truncate",
    ];
    // Owners whose `remove(` deletes a file; any other owner is a list or a set.
    const FILE_OWNERS: &[&str] = &[
        "os",
        "shutil",
        "fs",
        "fsp",
        "promises",
        "file",
        "fileutils",
        "dir",
        "io",
        "kernel",
        "pathlib",
        "path",
        "unlink",
    ];
    let chars: Vec<char> = code.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = 0;
    while i < chars.len() {
        if !is_word(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        let l: String = chars[start..i].iter().collect::<String>().to_lowercase();
        let prev = chars[..start].iter().rev().find(|c| !c.is_whitespace());
        let prev_adjacent = if start > 0 {
            Some(chars[start - 1])
        } else {
            None
        };
        let next = chars[i..].iter().find(|c| !c.is_whitespace()).copied();
        let next_adjacent = chars.get(i).copied();
        let is_call = next == Some('(');
        let is_attr = prev_adjacent == Some('.');
        if l == "name" || l == "fileno" {
            if is_attr {
                return true;
            }
            continue;
        }
        if matches!(prev_adjacent, Some('\'' | '"' | '[')) {
            continue;
        }
        if l.starts_with("exec") && !l.starts_with("execute") {
            return true;
        }
        if !RISKY.contains(&l.as_str()) {
            continue;
        }
        if AMBIGUOUS.contains(&l.as_str()) {
            if !is_call || (l == "rm" && !is_attr) {
                continue;
            }
            if (l == "remove" || l == "rm") && is_attr {
                let owner: String = chars[..start - 1]
                    .iter()
                    .rev()
                    .take_while(|c| is_word(**c))
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
                    .to_lowercase();
                let after_call = chars[..start - 1].last().is_some_and(|c| *c == ')');
                if !owner.is_empty() && !FILE_OWNERS.contains(&owner.as_str()) && !after_call {
                    continue;
                }
            }
            return true;
        }
        // Not ambiguous: a call, an attribute, or a ruby style call with no parenthesis. A word
        // followed by `=`, `:`, a closing bracket or a quote is a key or an argument name.
        let _ = prev;
        if is_call
            || is_attr
            || next_adjacent.is_none()
            || next_adjacent.is_some_and(|c| c.is_whitespace() || matches!(c, ';' | ',' | '.'))
        {
            return true;
        }
    }
    false
}

/// Can the code that a shell or an interpreter is handed in these arguments write? Code with a call or
/// flag that writes, a shell string with a redirect or a program that writes, an awk program with a
/// redirect. A formatter, a compile check or a reader is not.
fn handed_code_may_write(prog: &str, rest: &[String]) -> bool {
    let joined = rest.join(" ");
    if code_may_write(&joined) {
        return true;
    }
    let shell = SHELLS.contains(&prog) || prog == "eval";
    if (shell || matches!(prog, "awk" | "gawk" | "mawk")) && joined.contains('>') {
        return true;
    }
    if shell {
        let toks: Vec<&str> = joined
            .split(|c: char| c.is_whitespace() || "\"'`;|&(){}$".contains(c))
            .filter(|t| !t.is_empty())
            .collect();
        return toks.iter().any(|t| {
            let b = basename(t);
            WRITERS.contains(&b)
                || COPIERS.contains(&b)
                || matches!(b, "mv" | "dd" | "tar" | "unzip" | "tee")
                || COMPRESSORS.contains(&b)
                || EDITORS.contains(&b)
                || (b == "sed"
                    && toks
                        .iter()
                        .any(|x| x.starts_with("-i") || *x == "--in-place"))
        });
    }
    false
}

/// The paths an interpreter's command holds as text: its arguments and the script fed to it.
/// Only real paths count (absolute, from home, or climbing out of the folder). A relative word such as
/// a record's name is a file a tool only reads or a string of data; code that writes one is caught by
/// `written_literals`, which resolves the path against the working directory.
fn interpreter_paths(p: &crate::shellparse::Parsed, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut texts: Vec<&String> = args.iter().collect();
    if let Some(h) = &p.input {
        texts.push(h);
    }
    for a in texts {
        out.extend(text_paths(a));
    }
    out
}

/// Does code given to an interpreter call the command that records a human message? A call is a
/// system call, a process start or a shell-out that holds the engine's name and `human-turn`; the
/// phrase in a string that is only printed is not.
fn code_calls_human_turn(code: &str) -> bool {
    // A comment line is not code.
    let code: String = code
        .lines()
        .filter(|l| {
            let l = l.trim_start();
            !(l.starts_with('#') || l.starts_with("//"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !code.contains("human-turn") || !code.contains("bf") {
        return false;
    }
    const CALLS: &[&str] = &[
        "system",
        "popen",
        "exec",
        "spawn",
        "subprocess",
        "check_call",
        "check_output",
        "qx",
        "shell_exec",
        "passthru",
        "Open3",
        "`",
        "%x",
        "Deno.run",
        "Deno.Command",
        "child_process",
        "Process",
        "getoutput",
        "getstatusoutput",
        "pty.",
    ];
    if CALLS.iter().any(|c| code.contains(c)) || code.contains("proc_open") {
        return true;
    }
    pipe_open_calls(&code)
}

/// The string literals of code, without their quotes, with the text just before and after each.
fn string_literals(code: &str) -> Vec<(String, String, String)> {
    let c: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '"' || c[i] == '\'' {
            let q = c[i];
            if let Some(e) = c[i + 1..].iter().position(|x| *x == q) {
                let lit: String = c[i + 1..i + 1 + e].iter().collect();
                let before: String = c[..i].iter().collect();
                let after: String = c[i + 2 + e..].iter().collect();
                out.push((lit, before, after));
                i += e + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The first string literal inside the call that ends just before `.method`: `Path('a')` in
/// `Path('a').write_text('x')`. `before` ends with the closing parenthesis.
fn receiver_literal(before: &[char]) -> Option<String> {
    let mut depth = 0i32;
    let mut start = None;
    for k in (0..before.len()).rev() {
        match before[k] {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    start = Some(k);
                    break;
                }
            }
            _ => {}
        }
    }
    let s: String = before[start? + 1..before.len() - 1].iter().collect();
    string_literals(&s).into_iter().map(|l| l.0).next()
}

/// What an extra call does with the paths it is given.
#[derive(Clone, Copy)]
enum Extra {
    /// Writes the destination, deletes the source: both are judged.
    Move,
    /// Makes a link: the path it names and the one it points at are both judged.
    Link,
    /// Writes only its destination.
    Copy,
    /// Writes its first path.
    Write,
    /// A path object opened with a write mode: the receiver is the path.
    OpenMode,
}

fn is_fs_module(r: &str) -> bool {
    matches!(
        r,
        "fs" | "node:fs" | "fs/promises" | "node:fs/promises" | "fs-extra" | "os" | "shutil"
    )
}

/// The word before the dot that precedes the name at `i`: `os` in `os.replace`.
fn qualifier(b: &[char], i: usize) -> Option<String> {
    if i < 2 || b[i - 1] != '.' {
        return None;
    }
    let mut s = i - 1;
    while s > 0 && (b[s - 1].is_alphanumeric() || b[s - 1] == '_') {
        s -= 1;
    }
    let q: String = b[s..i - 1].iter().collect();
    (!q.is_empty()).then_some(q)
}

/// Is the receiver (ending with its closing parenthesis) a path object: `Path('a')`, `Pathname.new(...)`?
fn receiver_is_path(before: &[char]) -> bool {
    let mut depth = 0i32;
    let mut open = None;
    for k in (0..before.len()).rev() {
        match before[k] {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(k);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(o) = open else { return false };
    let mut s = o;
    while s > 0 && (before[s - 1].is_alphanumeric() || before[s - 1] == '_') {
        s -= 1;
    }
    let ctor: String = before[s..o].iter().collect();
    matches!(
        ctor.as_str(),
        "Path" | "PurePath" | "PosixPath" | "WindowsPath" | "new" | "Pathname"
    )
}

/// Calls beside the plain ones: a move, copy or link of a module, a path object's method, and a path
/// object opened with a mode. The ambiguous names (`replace`, `move`, `copy`, `link`) count only on a
/// module or a path object, so `'abc'.replace('a','b')` is not a write.
fn extra_kind(name: &str, qual: &str, recv_mod: bool, recv_path: bool) -> Option<Extra> {
    let fs = recv_mod || matches!(qual, "os" | "shutil" | "fs" | "fse" | "fsp" | "promises");
    let ruby_file = matches!(qual, "File" | "IO");
    let utils = qual == "FileUtils";
    match name {
        "open" if recv_path => Some(Extra::OpenMode),
        "replace" | "move" if fs || recv_path => Some(Extra::Move),
        "copy" if fs => Some(Extra::Copy),
        "copy" if recv_path => Some(Extra::Copy),
        "cpSync" | "cp" if fs => Some(Extra::Copy),
        "symlink" | "link" | "symlinkSync" | "linkSync" if fs => Some(Extra::Link),
        "symlink_to" | "hardlink_to" | "link_to" if recv_path => Some(Extra::Link),
        "write" | "binwrite" | "delete" | "unlink" if ruby_file => Some(Extra::Write),
        "cp" | "cp_r" | "copy" if utils => Some(Extra::Copy),
        "mv" | "move" if utils => Some(Extra::Move),
        "ln" | "ln_s" | "ln_sf" if utils => Some(Extra::Link),
        "rm" | "rm_f" | "rm_r" | "rm_rf" | "remove" | "touch" | "mkdir" | "mkdir_p" if utils => {
            Some(Extra::Write)
        }
        _ => None,
    }
}

/// The text of a call's arguments, up to the parenthesis that closes it: inner parentheses (`open(('a'),
/// 'w')`) are part of it and quoted text is skipped over.
fn call_args(rest: &str) -> String {
    let mut depth = 1i32;
    let mut out = String::new();
    let mut quote: Option<char> = None;
    for ch in rest.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            },
        }
        out.push(ch);
    }
    out
}

/// The paths that code written for an interpreter opens for writing or deletes, where the path is a
/// string literal in the call (`open('a', 'w')`, `Path('a').write_text`, `writeFileSync('a', ...)`,
/// `os.remove('a')`, perl's `open(F, '>a')` and `open(F, '>', 'a')`). Only these calls count: a path that
/// is read, or that is only text, is not a write.
fn written_literals(code: &str) -> Vec<String> {
    const CALLS: &[&str] = &[
        "write_text",
        "write_bytes",
        "writeFileSync",
        "writeFile",
        "appendFileSync",
        "appendFile",
        "createWriteStream",
        "unlinkSync",
        "rmSync",
        "mkdirSync",
        "renameSync",
        "copyFileSync",
        "unlink",
        "remove",
        "rmtree",
        "rename",
        "copyfile",
        "copy2",
        "copytree",
        "truncate",
        "mkdir",
        "makedirs",
        "mkdir_p",
        "touch",
        "rmdir",
    ];
    let mut out = Vec::new();
    let b: Vec<char> = code.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = 0;
    while i < b.len() {
        if !is_word(b[i]) || (i > 0 && is_word(b[i - 1])) {
            i += 1;
            continue;
        }
        let mut e = i;
        while e < b.len() && is_word(b[e]) {
            e += 1;
        }
        let name: String = b[i..e].iter().collect();
        let mut j = e;
        while j < b.len() && b[j] == ' ' {
            j += 1;
        }
        let has_recv = i >= 2 && b[i - 1] == '.' && b[i - 2] == ')';
        let qual = qualifier(&b, i);
        let recv_lit = if has_recv {
            receiver_literal(&b[..i - 1])
        } else {
            None
        };
        let recv_mod = recv_lit.as_deref().is_some_and(is_fs_module);
        let recv_path = has_recv && !recv_mod && receiver_is_path(&b[..i - 1]);
        let extra = extra_kind(&name, qual.as_deref().unwrap_or(""), recv_mod, recv_path);
        // A name that lists, sets and files all use (`remove`, `rename`, `unlink`, `truncate`, `mkdir`,
        // `touch`, `rmdir`) is a file write only on a file module, a path object or by itself, never
        // on any other receiver (`items.remove('a')`, `classList.remove('hidden')`).
        let ambiguous = matches!(
            name.as_str(),
            "remove" | "rename" | "unlink" | "truncate" | "mkdir" | "touch" | "rmdir"
        );
        let on_file = recv_mod
            || recv_path
            || !(i > 0 && b[i - 1] == '.')
            || matches!(
                qual.as_deref().unwrap_or(""),
                "os" | "shutil"
                    | "fs"
                    | "fse"
                    | "fsp"
                    | "promises"
                    | "File"
                    | "FileUtils"
                    | "IO"
                    | "Dir"
            );
        if j < b.len()
            && b[j] == '('
            && (!ambiguous || extra.is_some() || on_file)
            && (name == "open" || extra.is_some() || CALLS.contains(&name.as_str()))
        {
            let rest: String = b[j + 1..].iter().collect();
            let args = call_args(&rest);
            let lits: Vec<String> = string_literals(&args).into_iter().map(|l| l.0).collect();
            if let Some(kind) = extra {
                let recv = if recv_path { recv_lit.clone() } else { None };
                match kind {
                    Extra::Move | Extra::Link => {
                        let n = if recv.is_some() { 1 } else { 2 };
                        out.extend(recv);
                        out.extend(lits.iter().take(n).cloned());
                    }
                    Extra::Copy => {
                        // Only the destination is written; a source is read.
                        if recv.is_some() {
                            out.extend(lits.first().cloned());
                        } else {
                            out.extend(lits.get(1).cloned());
                        }
                    }
                    Extra::Write => match recv {
                        Some(r) => out.push(r),
                        None => out.extend(lits.first().cloned()),
                    },
                    Extra::OpenMode => {
                        let writes =
                            |m: &str| m.chars().any(|c| matches!(c, 'w' | 'a' | 'x' | '+'));
                        if lits.first().is_some_and(|m| writes(m)) {
                            out.extend(recv);
                        }
                    }
                }
            } else if name == "open" {
                let writes = |m: &str| m.chars().any(|c| matches!(c, 'w' | 'a' | 'x' | '+'));
                match lits.as_slice() {
                    [m, p, ..] if m.trim_start().starts_with('>') || m.starts_with("+<") => {
                        if m.trim_start_matches(['>', '+', '<']).trim().is_empty() {
                            out.push(p.clone());
                        }
                    }
                    [p] if p.trim_start().starts_with('>') => {
                        out.push(p.trim_start_matches(['>', '+']).trim().to_string())
                    }
                    [p, m, ..]
                        if m.len() <= 3
                            && m.chars().all(|c| "rwabxt+U".contains(c))
                            && writes(m) =>
                    {
                        out.push(p.clone())
                    }
                    _ => {}
                }
            } else {
                // A method on a path object (`Path('a').write_text('x')`) names its path in the
                // receiver, not in the arguments.
                // A module (`require('fs').writeFileSync('a', x)`) is not a path: the arguments are.
                let receiver = if i >= 2 && b[i - 1] == '.' && b[i - 2] == ')' {
                    receiver_literal(&b[..i - 1]).filter(|r| {
                        !matches!(
                            r.as_str(),
                            "fs" | "node:fs"
                                | "fs/promises"
                                | "node:fs/promises"
                                | "fs-extra"
                                | "os"
                                | "shutil"
                        )
                    })
                } else {
                    None
                };
                let method_only = matches!(name.as_str(), "write_text" | "write_bytes");
                // A rename writes the destination and deletes the source; a copy only writes the
                // destination, so reading a file by copying it is not a write of the source.
                let moves = matches!(name.as_str(), "rename" | "renameSync");
                let copies = matches!(
                    name.as_str(),
                    "copyfile" | "copy2" | "copytree" | "copyFileSync"
                );
                if let Some(r) = receiver {
                    // `Path('a').rename('b')` also writes `b`; a copy leaves its receiver alone.
                    if !copies {
                        out.push(r);
                    }
                    if moves {
                        out.extend(lits.iter().take(2).cloned());
                    } else if copies {
                        // `require('fs').copyFileSync(a, b)` has a module as its receiver and
                        // names both paths; `Path('a').copy('b')` names the destination only.
                        if let Some(p) = lits.get(1).or(lits.first()) {
                            out.push(p.clone());
                        }
                    }
                } else if !method_only {
                    if moves {
                        out.extend(lits.iter().take(2).cloned());
                    } else if copies {
                        if let Some(p) = lits.get(1) {
                            out.push(p.clone());
                        }
                    } else if let Some(p) = lits.first() {
                        out.push(p.clone());
                    }
                }
            }
        }
        i = e;
    }
    out.retain(|p| !p.is_empty() && !p.starts_with("/dev/"));
    out
}

/// The path literals that code only reads: the first path of a call that opens for reading (`open('a')`,
/// `open('a', 'rb')`, `readFileSync('a')`, `read_text`), and the source of a copy. Each comes with the
/// number of times it is read that way.
fn read_literals(code: &str) -> HashMap<String, usize> {
    const READS: &[&str] = &[
        "read",
        "binread",
        "readlines",
        "foreach",
        "readFileSync",
        "readFile",
        "readdirSync",
        "readdir",
        "createReadStream",
        "statSync",
        "existsSync",
        "read_text",
        "read_bytes",
        "listdir",
        "isfile",
        "isdir",
        "exists",
        "stat",
        "load",
        "copyfile",
        "copy2",
        "copytree",
        "copy",
        "copyFileSync",
        "cpSync",
        "cp",
    ];
    let mut out: HashMap<String, usize> = HashMap::new();
    let b: Vec<char> = code.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = 0;
    while i < b.len() {
        if !is_word(b[i]) || (i > 0 && is_word(b[i - 1])) {
            i += 1;
            continue;
        }
        let mut e = i;
        while e < b.len() && is_word(b[e]) {
            e += 1;
        }
        let name: String = b[i..e].iter().collect();
        let mut j = e;
        while j < b.len() && b[j] == ' ' {
            j += 1;
        }
        if j < b.len() && b[j] == '(' && (name == "open" || READS.contains(&name.as_str())) {
            let rest: String = b[j + 1..].iter().collect();
            let args_text = call_args(&rest);
            let lits: Vec<String> = string_literals(&args_text)
                .into_iter()
                .map(|l| l.0)
                .collect();
            let reads =
                |m: &str| m.len() <= 3 && m.chars().all(|c| "rbtU".contains(c)) && !m.is_empty();
            let first_is_read = if name == "open" {
                match lits.as_slice() {
                    // `os.open(p, os.O_WRONLY)` has one literal and a flag: only a bare path is a read.
                    [_] => !args_text.contains(','),
                    [_, m, ..] => reads(m),
                    _ => false,
                }
            } else {
                true
            };
            if first_is_read {
                if let Some(p) = lits.first() {
                    *out.entry(p.clone()).or_insert(0) += 1;
                }
            }
        }
        i = e;
    }
    out
}

/// The files an awk program prints to: `print > "a"`, `print >> "a"` and `printf(...) > "a"`, where the
/// path is a string literal. A `>` that is a comparison (no `print` before it in its statement) is not a
/// redirect.
fn awk_written(code: &str) -> Vec<String> {
    let c: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    for (i, ch) in c.iter().enumerate() {
        if *ch != '>' {
            continue;
        }
        let stmt_start = c[..i]
            .iter()
            .rposition(|x| matches!(x, '{' | ';' | '\n'))
            .map(|k| k + 1)
            .unwrap_or(0);
        let stmt: String = c[stmt_start..i].iter().collect();
        if !stmt.contains("print") {
            continue;
        }
        let mut j = i + 1;
        if c.get(j) == Some(&'>') {
            j += 1;
        }
        while c.get(j).is_some_and(|x| x.is_whitespace()) {
            j += 1;
        }
        if c.get(j) == Some(&'"') {
            let lit: String = c[j + 1..].iter().take_while(|x| **x != '"').collect();
            if !lit.is_empty() {
                out.push(lit);
            }
        }
    }
    out
}

/// The folders that code makes with a call such as `os.makedirs('a')` or `fs.mkdirSync('a')`.
fn made_literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for name in ["makedirs", "mkdirSync", "mkdir", "mkdir_p"] {
        let mut from = 0;
        while let Some(k) = code[from..].find(&format!("{name}(")).map(|k| k + from) {
            let after = &code[k + name.len() + 1..];
            let args = after.split(')').next().unwrap_or("");
            // A path object names its folder in the receiver: `Path('a').mkdir(parents=True)`.
            let before: Vec<char> = code[..k].chars().collect();
            let recv =
                if before.ends_with(&['.']) && before.len() >= 2 && before[before.len() - 2] == ')'
                {
                    receiver_literal(&before[..before.len() - 1])
                } else {
                    None
                };
            // A folder is made only by a file module, a path object or a bare call, not by any other
            // receiver (`dirs.mkdir('a')` on an object of the code's own).
            if recv.is_none() && before.last() == Some(&'.') {
                let q = qualifier(&before, before.len()).unwrap_or_default();
                if !matches!(
                    q.as_str(),
                    "os" | "shutil" | "fs" | "fse" | "fsp" | "promises" | "FileUtils" | "Dir"
                ) {
                    from = k + name.len() + 1;
                    continue;
                }
            }
            if let Some(r) = recv {
                out.push(r);
            } else if let Some(l) = string_literals(args).into_iter().map(|l| l.0).next() {
                out.push(l);
            }
            from = k + name.len() + 1;
        }
    }
    out
}

/// A program started without a call word: a pipe open (a string that starts or ends with `|` given
/// to `open`, the `-|` and `|-` modes) and the pipe forms of awk (`"cmd" | getline`, `print | "cmd"`).
fn pipe_open_calls(code: &str) -> bool {
    let lits = string_literals(code);
    if code.contains("open")
        && lits
            .iter()
            .any(|(l, _, _)| l.trim() == "-|" || l.trim() == "|-")
    {
        return true;
    }
    lits.iter().any(|(l, before, after)| {
        let t = l.trim();
        if !(l.contains("human-turn") && l.contains("bf")) {
            return false;
        }
        t.starts_with('|')
            || t.ends_with('|')
            || before.trim_end().ends_with('|')
            || after.trim_start().starts_with('|')
    })
}

/// Does this word name the state folder or the variables that locate it?
fn names_state(table: &Value, w: &str) -> bool {
    if w.contains("$BF_STATE_HOME")
        || w.contains("${BF_STATE_HOME")
        || w.contains("$XDG_STATE_HOME")
        || w.contains("${XDG_STATE_HOME")
        || w.starts_with("BF_STATE_HOME=")
        || w.starts_with("XDG_STATE_HOME=")
    {
        return true;
    }
    let sd = state_dir(table);
    let pid = table["project_id"].as_str().unwrap_or("");
    !sd.is_empty()
        && (w.contains(sd)
            || (pid.len() >= 8 && w.contains(pid))
            || state_base(table)
                .map(|b| b.len() > 8 && w.contains(b))
                .unwrap_or(false))
}

fn is_home_form(t: &str) -> bool {
    t == "~" || t.starts_with("~/") || t == "$HOME" || t.starts_with("$HOME/")
}

/// A path as a command gives it, resolved from the folder it runs in. `None` when it cannot be known:
/// the path or the folder is built at run time.
fn target_abs(t: &str, dir: &Option<String>) -> Option<String> {
    let dynamic = (t.contains('$') && !is_home_form(t)) || t.contains('`');
    if dynamic {
        return None;
    }
    let relative = !(t.starts_with('/') || is_home_form(t));
    match dir {
        Some(d) => Some(resolve(t, d)),
        None if !relative => Some(resolve(t, "/")),
        None => None,
    }
}

/// Does the text of a path that cannot be resolved still name a protected folder as a segment?
fn names_protected_segment(t: &str) -> bool {
    t.split('/')
        .any(|s| matches!(s, "factory" | ".bf" | ".claude"))
}

fn is_flag(a: &str) -> bool {
    a.starts_with('-') && a.len() > 1
}

/// Options of the copiers and movers that take the next word as their value.
fn takes_value(prog: &str, a: &str) -> bool {
    match prog {
        "cp" | "mv" | "ln" => matches!(a, "-S" | "--suffix"),
        "install" => matches!(
            a,
            "-m" | "-o" | "-g" | "-S" | "--mode" | "--owner" | "--group" | "--suffix"
        ),
        "rsync" => matches!(
            a,
            "-e" | "--rsh"
                | "--exclude"
                | "--include"
                | "--filter"
                | "-f"
                | "--exclude-from"
                | "--include-from"
                | "--files-from"
                | "--rsync-path"
                | "--chmod"
                | "--log-file"
                | "--partial-dir"
                | "--backup-dir"
                | "--suffix"
                | "--compare-dest"
                | "--copy-dest"
                | "--link-dest"
                | "-T"
                | "--temp-dir"
                | "--port"
                | "--bwlimit"
                | "--timeout"
                | "--max-size"
                | "--min-size"
                | "--password-file"
                | "--usermap"
                | "--groupmap"
                | "--chown"
        ),
        _ => false,
    }
}

/// The parts of a copy, move, link, install or sync: what it reads, and where it puts it.
struct Transfer {
    sources: Vec<String>,
    /// The last word, or the folder given with `-t` / `--target-directory`.
    dest: Option<String>,
    /// The destination is a folder because `-t` said so.
    by_option: bool,
    /// `-T` / `--no-target-directory`: the destination is the thing itself, never a folder to go into.
    plain: bool,
}

fn transfer_parts(prog: &str, args: &[String]) -> Transfer {
    let mut pos: Vec<String> = Vec::new();
    let mut tdir: Option<String> = None;
    let mut plain = false;
    let mut rest_are_words = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if rest_are_words {
            pos.push(a.clone());
        } else if a == "--" {
            rest_are_words = true;
        } else if a == "-t" || a == "--target-directory" {
            tdir = it.next().cloned();
        } else if let Some(d) = a.strip_prefix("--target-directory=") {
            tdir = Some(d.to_string());
        } else if a == "--no-target-directory" || (prog != "rsync" && a == "-T") {
            plain = true;
        } else if takes_value(prog, a) {
            it.next();
        } else if is_flag(a) {
            // A bundle of short options ends with `t` when the folder follows (`cp -at dir file`), and
            // holds `T` for the no-target-folder form (`cp -rT a b`). Not for rsync, where `-t` keeps times.
            let short = a.starts_with('-') && !a.starts_with("--");
            if prog != "rsync" && short && a.len() > 2 {
                if a.ends_with('t') {
                    tdir = it.next().cloned();
                } else if a[1..].contains('T') {
                    plain = true;
                }
            }
            // `ln -n` treats a link to a folder as a file: the link itself is replaced.
            if prog == "ln" && (a == "--no-dereference" || (short && a[1..].contains('n'))) {
                plain = true;
            }
        } else {
            pos.push(a.clone());
        }
    }
    match tdir {
        Some(d) => Transfer {
            sources: pos,
            dest: Some(d),
            by_option: true,
            plain,
        },
        None => {
            // `ln` with one word makes the link `./<name>` in the working directory.
            let dest = if pos.len() > 1 || prog != "ln" {
                pos.pop()
            } else if pos.len() == 1 {
                Some(".".to_string())
            } else {
                None
            };
            Transfer {
                sources: pos,
                dest,
                by_option: false,
                plain,
            }
        }
    }
}

/// Is the destination of a transfer a folder the sources go into? By option, by a trailing slash or a
/// dot, because several sources need one, or because it is one on disk.
fn goes_into_folder(tp: &Transfer, dest: &str, dir: &Option<String>) -> bool {
    if tp.by_option {
        return true;
    }
    if tp.plain {
        return false;
    }
    let dot = dest == "." || dest == ".." || dest.ends_with("/.") || dest.ends_with("/..");
    dest.ends_with('/')
        || dot
        || tp.sources.len() > 1
        || target_abs(dest, dir).is_some_and(|a| Path::new(&a).is_dir())
}

/// Where a source ends up when it goes into a folder: the folder with the source's own name. A source
/// that stands for its contents (a trailing slash, `.`, a variable) or for several names lands as a
/// wildcard.
fn landing(dest: &str, src: &str) -> String {
    let trimmed = src.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or("");
    let contents = src.ends_with('/')
        || base.is_empty()
        || base == "."
        || base == ".."
        || base.contains('$')
        || base.contains('`')
        // Input that `xargs` hands over and the guard cannot read.
        || base.contains('\u{1}');
    format!(
        "{}/{}",
        dest.trim_end_matches('/'),
        if contents { "*" } else { base }
    )
}

/// The paths a transfer writes: where each source lands, or the destination itself when it is a file.
/// A mover also deletes its sources, and a linker names the path it points at, so those count too.
fn transfer_targets(prog: &str, args: &[String], dir: &Option<String>) -> Vec<String> {
    let tp = transfer_parts(prog, args);
    let mut out = Vec::new();
    if prog == "mv" {
        out.extend(tp.sources.iter().cloned());
    } else if prog == "ln" {
        // The target of a symbolic link, when it is relative, is read from the folder the link is made
        // in, not from the working directory.
        let symbolic = args.iter().any(|a| {
            a == "--symbolic"
                || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('s'))
        });
        // With `-r` GNU ln reads the target from the working directory and writes the relative link
        // itself, so the target is judged from there.
        let from_cwd = args.iter().any(|a| {
            a == "--relative"
                || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('r'))
        });
        let link_dir = tp.dest.as_ref().map(|d| {
            if goes_into_folder(&tp, d, dir) {
                d.trim_end_matches('/').to_string()
            } else {
                match d.trim_end_matches('/').rsplit_once('/') {
                    Some(("", _)) => "/".to_string(),
                    Some((parent, _)) => parent.to_string(),
                    None => ".".to_string(),
                }
            }
        });
        for s in &tp.sources {
            let relative =
                !(s.starts_with('/') || is_home_form(s) || s.contains('$') || s.contains('`'));
            match (&link_dir, symbolic && relative && !from_cwd) {
                (Some(l), true) if !l.is_empty() => out.push(format!("{l}/{s}")),
                _ => out.push(s.clone()),
            }
        }
    }
    if let Some(d) = &tp.dest {
        if goes_into_folder(&tp, d, dir) {
            out.extend(tp.sources.iter().map(|s| landing(d, s)));
        } else {
            out.push(d.clone());
        }
    }
    out
}

/// The paths of the arguments of `chmod`, `chown` and the like, without the words that are not paths: the
/// mode or owner, and the values of options.
fn writer_paths(prog: &str, args: &[String]) -> Vec<String> {
    if prog == "mktemp" {
        // With `-p DIR` the file is made in that folder and the template is only a name; with `-t` it is
        // made in the temporary folder. Otherwise the template is a path in the working directory.
        let mut it = args.iter();
        let mut templates = Vec::new();
        while let Some(a) = it.next() {
            if a == "-p" || a == "--tmpdir" {
                return it.next().cloned().into_iter().collect();
            } else if let Some(d) = a.strip_prefix("--tmpdir=") {
                return vec![d.to_string()];
            } else if a == "-t" {
                return Vec::new();
            } else if a == "--suffix" {
                it.next();
            } else if !is_flag(a) {
                templates.push(a.clone());
            }
        }
        return templates;
    }
    let by_reference = args
        .iter()
        .any(|a| a == "--reference" || a.starts_with("--reference="));
    let valued: &[&str] = match prog {
        "truncate" => &["-s", "--size", "-r", "--reference"],
        "mkdir" => &["-m", "--mode"],
        "touch" => &["-t", "-d", "--date", "-r", "--reference"],
        _ => &[],
    };
    let mut out = Vec::new();
    // The first word of `chmod` and `chown` is the mode or the owner, not a path.
    let mut mode_given = !matches!(prog, "chmod" | "chown") || by_reference;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if valued.contains(&a.as_str()) {
            it.next();
        } else if is_flag(a) {
            // A mode that removes permission (`-w`, `-x`) looks like an option; no option of `chmod`
            // is spelled with only these letters.
            if prog == "chmod" && !mode_given && a[1..].chars().all(|c| "rwxXstugoa".contains(c)) {
                mode_given = true;
            }
        } else if !mode_given {
            mode_given = true;
        } else {
            out.push(a.clone());
        }
    }
    out
}

/// Is the mode word of `chmod` one that only takes permissions away (`go-w`, `a-x,o-r`)?
fn chmod_only_removes(args: &[String]) -> bool {
    let Some(mode) = args.iter().find(|a| !is_flag(a)) else {
        return false;
    };
    !mode.is_empty()
        && mode.split(',').all(|clause| {
            clause.split_once('-').is_some_and(|(who, what)| {
                who.chars().all(|c| "ugoa".contains(c))
                    && !what.is_empty()
                    && what.chars().all(|c| "rwxXst".contains(c))
            })
        })
}

/// Is `arg` (without a value after `=`) the long option `full`, written whole or shortened to at least
/// `min` characters? Git accepts a shortened long option when it is unambiguous.
fn long_option(arg: &str, full: &str, min: usize) -> bool {
    let name = arg.split('=').next().unwrap_or(arg);
    name.len() >= min && full.starts_with(name)
}

/// The `(mode, path)` entries `git update-index --cacheinfo` is given: `mode,object,path` or three words.
fn cacheinfo_entries(rest: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if long_option(&rest[i], "--cacheinfo", 4) {
            if let Some(v) = rest.get(i + 1) {
                if v.contains(',') {
                    let mut it = v.splitn(3, ',');
                    let mode = it.next().unwrap_or("").to_string();
                    let _object = it.next();
                    out.push((mode, it.next().unwrap_or("").to_string()));
                    i += 2;
                    continue;
                }
                if let (Some(_), Some(p)) = (rest.get(i + 2), rest.get(i + 3)) {
                    out.push((v.clone(), p.clone()));
                    i += 4;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Does `git` with these arguments only change the index? `restore --staged` without `--worktree`, and
/// `rm --cached`, leave the files in the working tree alone.
fn git_index_only(sub: &str, rest: &[String]) -> bool {
    let has = |long: &str, short: &str| {
        rest.iter().any(|a| {
            a == long
                || (!short.is_empty()
                    && a.starts_with('-')
                    && !a.starts_with("--")
                    && a[1..].contains(short))
        })
    };
    match sub {
        "restore" => has("--staged", "S") && !has("--worktree", "W"),
        "rm" => has("--cached", ""),
        _ => false,
    }
}

/// The paths a program writes or deletes, from its arguments. Programs that only read give none. `dir` is
/// the folder the command runs in: a copy into a folder lands on the file inside it, so whether the
/// destination is a folder is read from disk.
fn writer_args(prog: &str, args: &[String], dir: &Option<String>) -> Vec<String> {
    let mut out = Vec::new();
    if prog == "mv" || COPIERS.contains(&prog) {
        if prog == "install" && args.iter().any(|a| a == "-d" || a == "--directory") {
            out.extend(args.iter().filter(|a| !is_flag(a)).cloned());
        } else {
            out.extend(transfer_targets(prog, args, dir));
        }
    } else if WRITERS.contains(&prog) {
        out.extend(writer_paths(prog, args));
    } else if prog == "dd" {
        out.extend(
            args.iter()
                .filter_map(|a| a.strip_prefix("of=").map(|s| s.to_string())),
        );
    } else if prog == "sed" || prog == "perl" || prog == "ruby" {
        let in_place = args.iter().any(|a| {
            a == "--in-place"
                || a.starts_with("--in-place=")
                || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('i'))
        });
        if in_place {
            let mut script_given = false;
            let mut skip = false;
            let mut it = args.iter().peekable();
            while let Some(a) = it.next() {
                if skip {
                    skip = false;
                    script_given = true;
                    continue;
                }
                if a.starts_with("--") {
                    if a == "--expression" || a == "--file" {
                        skip = true;
                    }
                    continue;
                }
                if is_flag(a) {
                    // This machine's sed takes the backup suffix as the next word, and an empty one
                    // (`-i ''`) is not the script.
                    if prog == "sed"
                        && a.ends_with('i')
                        && it.peek().map(|n| n.is_empty()) == Some(true)
                    {
                        it.next();
                        continue;
                    }
                    if a.ends_with('e')
                        || (prog == "sed" && a.ends_with('f'))
                        || (prog == "perl" && a.ends_with('E'))
                    {
                        skip = true;
                    }
                    continue;
                }
                if !script_given {
                    // The first word that is not an option is the script.
                    script_given = true;
                    continue;
                }
                out.push(a.clone());
            }
        }
    } else if prog == "find" {
        // Only `-delete` removes what the starting point holds; the command after `-exec` is judged
        // by itself, with `{}` read as each starting point.
        // A shell or an interpreter given `{}` as an argument (`-exec sh -c 'rm -f $1' _ {} \;`,
        // `-exec python3 -c '...argv[1]...' {} \;`) reaches it only as `$1`, `$@` or `argv`, which the
        // inner code is not read for, so the starting points stay write targets there.
        let mut hands_start_to_code = false;
        // The handed code can write what it is given (an `os.remove`, a redirect, an `rm`).
        let mut code_writes = false;
        let mut k = 0;
        while k < args.len() {
            if matches!(args[k].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                let mut j = k + 1;
                let mut words: Vec<&String> = Vec::new();
                while j < args.len() && args[j] != ";" && args[j] != "+" {
                    words.push(&args[j]);
                    j += 1;
                }
                // A wrapper (`env`, `nice`, `timeout`, ...) before the program is read through.
                let owned: Vec<String> = words.iter().map(|w| (*w).clone()).collect();
                if let Some(pi) = crate::shellparse::program_index(&owned) {
                    let b = basename(&owned[pi]);
                    if (SHELLS.contains(&b) || b == "eval" || is_interpreter(b))
                        && owned.iter().skip(pi + 1).any(|w| w.contains("{}"))
                    {
                        hands_start_to_code = true;
                        if handed_code_may_write(b, &owned[pi + 1..]) {
                            code_writes = true;
                        }
                    }
                }
                k = j;
            }
            k += 1;
        }
        // A name filter binds what is deleted or handed to the code: the matches are named files below the
        // starting point, not the starting point. The code of the command is judged with `{}` read as those
        // matches (see `shellparse`), and a delete against the protected places by `find_filter_hit`.
        let filters = crate::shellparse::find_filters(args);
        let names: Vec<&String> = filters
            .iter()
            .filter(|f| !f.path)
            .map(|f| &f.pattern)
            .collect();
        let delete = args.iter().any(|x| x == "-delete");
        let mut starts = crate::shellparse::find_starts(args);
        if starts.is_empty() {
            starts.push(".".to_string());
        }
        if (delete || code_writes) && !names.is_empty() {
            // Known gap (named in docs/engine.md): a match deeper than the starting point's own folder
            // is judged as if it were in it; the landing step holds the whole branch to the unit's list.
            for s in &starts {
                for n in &names {
                    out.push(format!("{}/{}", s.trim_end_matches('/'), n));
                }
            }
        } else if (hands_start_to_code && (filters.is_empty() || code_writes)) || delete {
            out.extend(starts);
        }
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if matches!(a.as_str(), "-fprint" | "-fprintf" | "-fprint0" | "-fls") {
                out.extend(it.next().cloned());
            }
        }
    } else if prog == "git" {
        let mut full = vec!["git".to_string()];
        full.extend(args.iter().cloned());
        if let Some((i, sub)) = git_sub(&full) {
            if matches!(sub, "checkout" | "restore" | "rm" | "mv" | "clean")
                && !git_index_only(sub, &full[i + 1..])
            {
                out.extend(full[i + 1..].iter().filter(|a| !is_flag(a)).cloned());
            }
            if sub == "checkout-index" {
                out.extend(full[i + 1..].iter().filter(|a| !is_flag(a)).cloned());
            }
            if sub == "update-index" {
                out.extend(cacheinfo_entries(&full[i + 1..]).into_iter().map(|(_, p)| p));
            }
        }
    } else if prog == "curl" || prog == "wget" {
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if matches!(
                a.as_str(),
                "-o" | "--output" | "-P" | "--directory-prefix" | "--output-dir"
            ) || (prog == "wget" && a == "-O")
            {
                out.extend(it.next().cloned());
            } else if let Some(f) = a.strip_prefix("--output=") {
                out.push(f.to_string());
            } else if (prog == "curl" && matches!(a.as_str(), "-O" | "--remote-name"))
                || prog == "wget"
            {
                // Saved under the name the server gives, in the working directory.
                out.push(".".to_string());
            }
        }
    } else if prog == "tar" {
        out.extend(tar_targets(args));
    } else if prog == "unzip" || prog == "7z" || prog == "7za" {
        out.extend(unzip_targets(prog, args));
    } else if COMPRESSORS.contains(&prog) {
        // `-c`, `--stdout`, `-t`, `-l`: nothing is written.
        let quiet = args.iter().any(|a| {
            matches!(
                a.as_str(),
                "--stdout" | "--to-stdout" | "--test" | "--list" | "--help" | "--version"
            ) || (a.starts_with('-')
                && !a.starts_with("--")
                && a[1..]
                    .chars()
                    .any(|c| matches!(c, 'c' | 't' | 'l' | 'h' | 'V')))
        });
        if !quiet {
            // The value of an option that takes one (`-T 4`, `-b 9`) is not a file; `-o out` names one.
            let mut it = args.iter();
            while let Some(a) = it.next() {
                if matches!(
                    a.as_str(),
                    "-T" | "--threads"
                        | "-b"
                        | "-B"
                        | "-M"
                        | "--memory"
                        | "-S"
                        | "--suffix"
                        | "-j"
                        | "-p"
                        | "--block-size"
                        | "-w"
                        | "--window"
                ) {
                    it.next();
                } else if matches!(a.as_str(), "-o" | "--output") {
                    out.extend(it.next().cloned());
                } else if !is_flag(a) {
                    out.push(a.clone());
                }
            }
        }
    } else if EDITORS.contains(&prog) {
        out.extend(
            args.iter()
                .filter(|a| !is_flag(a) && !a.starts_with('+'))
                .cloned(),
        );
    } else if prog == "split" || prog == "csplit" {
        // `split [options] [input [prefix]]` writes `prefix` followed by a suffix, `x` by default, in the
        // working directory.
        let mut positional: Vec<&String> = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if matches!(
                a.as_str(),
                "-l" | "-b"
                    | "-n"
                    | "-a"
                    | "-C"
                    | "--lines"
                    | "--bytes"
                    | "--number"
                    | "--suffix-length"
                    | "--line-bytes"
                    | "--additional-suffix"
                    | "-f"
                    | "--filter"
                    | "-t"
                    | "--separator"
            ) {
                it.next();
            } else if !is_flag(a) {
                positional.push(a);
            }
        }
        if prog == "csplit" {
            out.push("xx".to_string());
        } else {
            out.push(
                positional
                    .get(1)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "x".to_string()),
            );
        }
    } else if prog == "sort" {
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "-o" || a == "--output" {
                out.extend(it.next().cloned());
            } else if let Some(f) = a.strip_prefix("--output=") {
                out.push(f.to_string());
            } else if a.starts_with("-o") && !a.starts_with("--") && a.len() > 2 {
                out.push(a[2..].to_string());
            }
        }
    } else if matches!(
        prog,
        "uniq" | "xxd" | "zip" | "ditto" | "iconv" | "base64" | "openssl" | "gpg"
    ) {
        out.extend(filter_output(prog, args));
    }
    out
}

/// Does this program, with these arguments, delete files or edit them in place?
fn deletes_or_edits(prog: &str, args: &[String]) -> bool {
    if matches!(
        prog,
        "rm" | "rmdir" | "unlink" | "shred" | "truncate" | "mv" | "chmod" | "chown" | "tee"
    ) {
        return true;
    }
    if prog == "find" {
        return args.iter().any(|a| a == "-delete");
    }
    if prog == "sed" || prog == "perl" || prog == "ruby" {
        return args.iter().any(|a| {
            a == "--in-place"
                || a.starts_with("--in-place=")
                || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('i'))
        });
    }
    if prog == "git" {
        let mut full = vec!["git".to_string()];
        full.extend(args.iter().cloned());
        return matches!(
            git_sub(&full),
            Some((_, "rm" | "clean" | "checkout" | "restore"))
        );
    }
    false
}

/// The file a filter writes besides standard output: the second file of `uniq` and `xxd`, the output
/// option of `base64`, `openssl`, `gpg` and `iconv`, the archive of `zip`, the destination of `ditto`.
fn filter_output(prog: &str, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let valued: &[&str] = match prog {
        "uniq" => &[
            "-f",
            "-s",
            "-w",
            "--skip-fields",
            "--skip-chars",
            "--check-chars",
        ],
        "xxd" => &["-c", "-g", "-l", "-s", "-o", "-n", "-i"],
        _ => &[],
    };
    let mut positional: Vec<&String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if valued.contains(&a.as_str()) {
            it.next();
            continue;
        }
        match prog {
            "base64" | "gpg" | "iconv" | "openssl" => {
                if matches!(a.as_str(), "-o" | "--output" | "-out" | "--out") {
                    out.extend(it.next().cloned());
                    continue;
                }
                if let Some(f) = a.strip_prefix("--output=") {
                    out.push(f.to_string());
                    continue;
                }
            }
            _ => {}
        }
        if !is_flag(a) {
            positional.push(a);
        }
    }
    match prog {
        "uniq" | "xxd" => out.extend(positional.get(1).map(|s| s.to_string())),
        "zip" => out.extend(positional.first().map(|s| s.to_string())),
        "ditto" => out.extend(
            positional
                .last()
                .filter(|_| positional.len() > 1)
                .map(|s| s.to_string()),
        ),
        _ => {}
    }
    out
}

/// The paths `tar` writes: the folder an archive is unpacked into (the working directory unless `-C`
/// says otherwise) and each member named, or the archive it creates.
fn tar_targets(args: &[String]) -> Vec<String> {
    let (mode, dir, archive, members) = tar_parse(args);
    let mut out = Vec::new();
    match mode {
        Some('x') => {
            let base = dir.unwrap_or_else(|| ".".to_string());
            for m in members {
                out.push(format!("{}/{}", base.trim_end_matches('/'), m));
            }
            out.push(base);
        }
        Some(_) => {
            if let Some(a) = archive.filter(|a| a != "-") {
                out.push(a);
            }
        }
        None => out.extend(dir),
    }
    out
}

/// Does this command unpack an archive and name no member? Then what lands is whatever the archive
/// holds, and the guard cannot judge it against a unit's file scope.
fn unpacks_unnamed(prog: &str, args: &[String]) -> bool {
    match prog {
        "tar" | "bsdtar" | "gtar" => {
            let (mode, _, _, members) = tar_parse(args);
            mode == Some('x') && members.is_empty()
        }
        "unzip" => {
            let listing = args.iter().any(|a| {
                is_flag(a)
                    && a[1..]
                        .chars()
                        .all(|c| matches!(c, 'l' | 'p' | 't' | 'v' | 'z'))
            });
            let mut positional = 0;
            let mut skip = false;
            for a in args {
                if skip {
                    skip = false;
                } else if a == "-d" {
                    skip = true;
                } else if a == "-x" {
                    break;
                } else if !is_flag(a) {
                    positional += 1;
                }
            }
            !listing && positional <= 1
        }
        _ => false,
    }
}

/// The folder an unpack names for itself (`-C`, `-d`); `None` when it unpacks where it stands.
fn unpack_dir(prog: &str, args: &[String]) -> Option<String> {
    match prog {
        "unzip" => args.windows(2).find(|w| w[0] == "-d").map(|w| w[1].clone()),
        _ => tar_parse(args).1,
    }
}

/// Does the pattern match the whole text? `*` is any run of characters (a slash too), `?` is one
/// character, `[...]` is a class, a backslash quotes the next character.
fn glob_match(p: &[char], s: &[char]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some(('*', rest)) => (0..=s.len()).any(|k| glob_match(rest, &s[k..])),
        Some(('?', rest)) => !s.is_empty() && glob_match(rest, &s[1..]),
        Some(('\\', rest)) if !rest.is_empty() => {
            s.first() == Some(&rest[0]) && glob_match(&rest[1..], &s[1..])
        }
        Some(('[', rest)) => match rest.iter().skip(1).position(|c| *c == ']') {
            Some(e) => {
                let class = &rest[..e + 1];
                let Some(c) = s.first() else { return false };
                let (neg, class) = match class.first() {
                    Some('!') | Some('^') => (true, &class[1..]),
                    _ => (false, class),
                };
                let mut hit = false;
                let mut k = 0;
                while k < class.len() {
                    if k + 2 < class.len() && class[k + 1] == '-' {
                        hit |= class[k] <= *c && *c <= class[k + 2];
                        k += 3;
                    } else {
                        hit |= class[k] == *c;
                        k += 1;
                    }
                }
                hit != neg && glob_match(&rest[e + 2..], &s[1..])
            }
            None => s.first() == Some(&'[') && glob_match(rest, &s[1..]),
        },
        Some((c, rest)) => s.first() == Some(c) && glob_match(rest, &s[1..]),
    }
}

fn filter_holds(f: &crate::shellparse::FindFilter, text: &str) -> bool {
    let (p, s) = if f.fold {
        (f.pattern.to_lowercase(), text.to_lowercase())
    } else {
        (f.pattern.clone(), text.to_string())
    };
    glob_match(
        &p.chars().collect::<Vec<_>>(),
        &s.chars().collect::<Vec<_>>(),
    )
}

/// The first entry at or below `path` that every filter lets through. `shown` is how `find` would print
/// it. Open folders and the working copies are not walked. A walk that grows too large counts as a hit.
fn walk_for_match(
    table: &Value,
    path: &Path,
    shown: &str,
    filters: &[crate::shellparse::FindFilter],
    budget: &mut usize,
) -> Option<String> {
    let text = path.to_string_lossy().to_string();
    let root = table["root"].as_str().unwrap_or("");
    if let Some(rel) = under(&text, root) {
        if rel == ".bf/work" || rel.starts_with(".bf/work/") || is_open_rel(rel) {
            return None;
        }
    }
    if *budget == 0 {
        return Some(text);
    }
    *budget -= 1;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if filters
        .iter()
        .all(|f| filter_holds(f, if f.path { shown } else { &name }))
    {
        return Some(text);
    }
    let md = fs::symlink_metadata(path).ok()?;
    if md.is_dir() {
        for e in fs::read_dir(path).ok()?.flatten() {
            let child = format!(
                "{}/{}",
                shown.trim_end_matches('/'),
                e.file_name().to_string_lossy()
            );
            if let Some(h) = walk_for_match(table, &e.path(), &child, filters, budget) {
                return Some(h);
            }
        }
    }
    None
}

/// A `find` whose name filter lets a protected file through, and that deletes it or hands it to a
/// program that writes. A starting point that is only a folder above the protected places is not a
/// target: what the filters can match is.
fn find_filter_denial(table: &Value, args: &[String], dir: &Option<String>) -> Option<Decision> {
    const MATCH: &str = "\u{1}match";
    let filters = crate::shellparse::find_filters(args);
    if filters.is_empty() {
        return None;
    }
    let deletes = args.iter().any(|a| a == "-delete");
    let exec_writes = crate::shellparse::exec_commands(args).iter().any(|words| {
        let owned: Vec<String> = words.iter().map(|w| w.replace("{}", MATCH)).collect();
        let Some(pi) = crate::shellparse::program_index(&owned) else {
            return false;
        };
        let prog = basename(&owned[pi]);
        let rest = &owned[pi + 1..];
        // A shell or an interpreter given the match may write it, and the guard cannot read how. That
        // counts only for a starting point that is itself in a protected place.
        let mut handed_writes = false;
        if (SHELLS.contains(&prog) || prog == "eval" || is_interpreter(prog))
            && rest.iter().any(|w| w.contains(MATCH))
        {
            // Code that can write what it is handed is a delete of the matches.
            handed_writes = handed_code_may_write(prog, rest);
        }
        handed_writes
            || writer_args(prog, rest, dir)
                .iter()
                .any(|t| t.contains(MATCH))
            || (deletes_or_edits(prog, rest) && rest.iter().any(|w| w.contains(MATCH)))
    });
    // Matches handed to code that only reads them are not a write, at the folder or below it.
    if !deletes && !exec_writes {
        return None;
    }
    let mut starts = crate::shellparse::find_starts(args);
    if starts.is_empty() {
        starts.push(".".to_string());
    }
    let root = table["root"].as_str().unwrap_or("").trim_end_matches('/');
    let places: Vec<String> = table["protected"]
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str())
        .map(|e| format!("{root}/{}", e.trim_end_matches('/')))
        .collect();
    for s in &starts {
        let Some(abs) = target_abs(s, dir) else {
            continue;
        };
        for place in &places {
            let (walk, shown) = if let Some(rel) = under(place, &abs) {
                let shown = if rel.is_empty() {
                    s.clone()
                } else {
                    format!("{}/{rel}", s.trim_end_matches('/'))
                };
                (place.clone(), shown)
            } else if under(&abs, place).is_some() {
                (abs.clone(), s.clone())
            } else {
                continue;
            };
            let mut budget = 50_000usize;
            if let Some(hit) =
                walk_for_match(table, Path::new(&walk), &shown, &filters, &mut budget)
            {
                let f = &filters[0];
                return Some(path_denial(
                    &hit,
                    &format!("`find` with the filter `{}`", f.pattern),
                    Reach::Delete,
                ));
            }
        }
    }
    None
}

fn tar_parse(args: &[String]) -> (Option<char>, Option<String>, Option<String>, Vec<String>) {
    let mut dir: Option<String> = None;
    let mut archive: Option<String> = None;
    let mut mode: Option<char> = None;
    let mut members: Vec<String> = Vec::new();
    let mut i = 0;
    let mut first = true;
    let set_mode = |m: &mut Option<char>, letters: &str| {
        for c in letters.chars() {
            if m.is_none() && matches!(c, 'x' | 'c' | 'r' | 'u') {
                *m = Some(c);
            }
        }
    };
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-C" || a == "--directory" {
            dir = args.get(i + 1).cloned();
            i += 2;
        } else if let Some(d) = a.strip_prefix("--directory=") {
            dir = Some(d.to_string());
            i += 1;
        } else if a == "--file" {
            archive = args.get(i + 1).cloned();
            i += 2;
        } else if let Some(f) = a.strip_prefix("--file=") {
            archive = Some(f.to_string());
            i += 1;
        } else if a.starts_with("--") {
            match a {
                "--extract" | "--get" => set_mode(&mut mode, "x"),
                "--create" => set_mode(&mut mode, "c"),
                "--append" | "--update" => set_mode(&mut mode, "r"),
                _ => {}
            }
            i += 1;
        } else if (a.starts_with('-') && a.len() > 1) || (first && !a.is_empty()) {
            // A bundle of letters (`-xzf`, or `xzf` without the dash as the first word).
            let letters = a.trim_start_matches('-');
            set_mode(&mut mode, letters);
            if letters.contains('f') {
                archive = args.get(i + 1).cloned();
                i += 2;
            } else if letters.ends_with('C') {
                dir = args.get(i + 1).cloned();
                i += 2;
            } else {
                i += 1;
            }
        } else {
            members.push(a.to_string());
            i += 1;
        }
        first = false;
    }
    (mode, dir, archive, members)
}

/// The paths `unzip` writes: the folder given with `-d`, or the working directory, and each member named.
fn unzip_targets(prog: &str, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut dir: Option<String> = None;
    let mut positional: Vec<&String> = Vec::new();
    let mut listing = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if prog == "unzip" && a == "-d" {
            dir = it.next().cloned();
        } else if let Some(d) = a
            .strip_prefix("-o")
            .filter(|_| prog != "unzip" && a.len() > 2)
        {
            dir = Some(d.to_string());
        } else if prog == "unzip" && a == "-x" {
            break;
        } else if is_flag(a) {
            if a[1..]
                .chars()
                .all(|c| matches!(c, 'l' | 'p' | 't' | 'v' | 'z'))
            {
                listing = true;
            }
        } else {
            positional.push(a);
        }
    }
    if listing || (prog != "unzip" && positional.first().map(|s| s.as_str()) != Some("x")) {
        return out;
    }
    let base = dir.unwrap_or_else(|| ".".to_string());
    if prog == "unzip" {
        for m in positional.iter().skip(1) {
            out.push(format!("{}/{}", base.trim_end_matches('/'), m));
        }
    }
    out.push(base);
    out
}

/// Does a folder lead to a path the unit may write? True when the folder is the directory part of a
/// file entry, a prefix of the literal part of a pattern, or an entry that is a folder. A folder that
/// is the entry's own file name does not count.
fn leads_to_scope(files: &[String], rel: &str) -> bool {
    let r: Vec<&str> = rel
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if r.is_empty() {
        return false;
    }
    files.iter().any(|g| {
        let g = g.trim_start_matches("./");
        let is_dir = g.ends_with('/');
        let s: Vec<&str> = g.split('/').filter(|x| !x.is_empty()).collect();
        for (k, seg) in r.iter().enumerate() {
            let Some(want) = s.get(k) else {
                return false;
            };
            if want.contains('*') {
                return true;
            }
            if want != seg {
                return false;
            }
        }
        r.len() < s.len() || is_dir
    })
}

/// How a command reached a path.
#[derive(Clone, Copy)]
enum Reach {
    /// Making a folder: refused like a write, except where it leads to a file the unit may write.
    Make,
    Write,
    /// A recursive change of permissions or owner: judged like a write, and like a delete of the whole
    /// working copy when it names the copy's root.
    Perm,
    Delete,
    Run,
    Text,
}

fn path_denial(abs: &str, part: &str, reach: Reach) -> Decision {
    let (verb, steps): (&str, Vec<&str>) = match reach {
        Reach::Write | Reach::Make | Reach::Perm => (
            "write",
            vec![
                "Change a record, a finding or a report with an engine command: `bf record put`, `bf report`, `bf evidence run`.",
                "To write a file of your own, give a path inside your working copy.",
                "If the policy, a gate or the hook settings must change, ask the human.",
            ],
        ),
        Reach::Delete => (
            "delete",
            vec![
                "Remove only files inside your working copy, by a path that lies there.",
                "Records are removed by engine commands only; ask the human if one must go.",
            ],
        ),
        Reach::Run => (
            "run",
            vec![
                "Run a program or script that lives in your working copy.",
                "Reading a protected file with `cat` or `ls` is fine.",
            ],
        ),
        Reach::Text => (
            "open",
            vec![
                "Open files from a path inside your working copy, or read the protected file with `cat`.",
                "A record changes through an engine command: `bf record put`, `bf report`.",
            ],
        ),
    };
    deny(
        &format!("{verb} {abs}"),
        &format!(
            "{part} would {verb} {abs}, a protected place: factory records, the policy, gates, the guard table, the hook settings and the git folder of a code repository are changed by the engine and the human only"
        ),
        &steps,
        "The path the command writes, deletes, runs or opens lies outside those places.",
    )
}

/// The protected place of the project that lies below `abs`, when `abs` is a folder above it. A target
/// that is such a folder takes the protected place with it.
fn covers_protected(table: &Value, abs: &str) -> Option<String> {
    let root = table["root"].as_str().unwrap_or("").trim_end_matches('/');
    if root.is_empty() {
        return None;
    }
    let base = abs.trim_end_matches('/');
    let mut places = vec![root.to_string()];
    places.extend(other_repos(table).into_iter().map(|r| format!("{r}/.git")));
    for e in table["protected"]
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str())
    {
        places.push(format!("{root}/{}", e.trim_end_matches('/')));
    }
    // A place strictly below the target (the root itself counts as a place, for targets above it).
    places
        .into_iter()
        .find(|p| p.len() > base.len() && p.starts_with(base) && p[base.len()..].starts_with('/'))
}

fn cover_denial(abs: &str, hit: &str, prog: &str, t: &str) -> Decision {
    deny(
        &format!("run `{prog}` on {abs}"),
        &format!("`{prog}` with the argument `{t}` would remove, move, re-permission, link, check out or fill {abs}, a folder that holds the protected place {hit}: factory records, the policy, gates, the guard table, the git hooks and the hook settings are changed by the engine and the human only"),
        &[
            "Name the files you mean, inside your working copy, for example `rm src/old.rs`.",
            "To discard your own changes use a path in your working copy; records change through engine commands.",
            "If the project itself must be moved or cleaned, ask the human.",
        ],
        "The command names only paths that hold no protected place.",
    )
}

fn unresolved_denial(t: &str, part: &str, why: &str) -> Decision {
    deny(
        &format!("write `{t}`"),
        &format!("{part} writes a path that cannot be resolved ({why}), so the guard cannot tell where it lands"),
        &[
            "Write the path out in full, for example `src/a.rs`.",
            "Change into a folder with a plain `cd <folder>` first, not a folder built from a variable.",
            "Or assign the variable in the same command line with a value that lies in your working copy, for example `D=out; echo x > $D/a.txt`.",
        ],
        "The command names every path it writes and every folder it changes into.",
    )
}

/// Judge a path a command writes or deletes: where it lands decides, never the words. Gives the
/// resolved path when it is allowed and known.
///
/// A target that holds a variable assigned in the same line is read with the value; one that holds a
/// brace is read as each of its alternatives; one that holds a glob or a command substitution is read
/// as a pattern and refused when it could land in a protected place. Each path that is allowed comes
/// back with `None` when it is exact, or with the whole pattern when it is one (the path then is the
/// folder before its first wildcard).
fn judge_target(
    table: &Value,
    t: &str,
    dir: &Option<String>,
    part: &str,
    reach: Reach,
    vars: &[HashMap<String, String>],
) -> Result<Vec<(String, Option<String>)>, Decision> {
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for v in vars {
        let t = subst_vars(t, v);
        if seen.contains(&t) {
            continue;
        }
        for alt in brace_expand(&t) {
            out.extend(judge_alt(table, &alt, dir, part, reach)?);
        }
        seen.push(t);
    }
    Ok(out)
}

fn judge_alt(
    table: &Value,
    t: &str,
    dir: &Option<String>,
    part: &str,
    reach: Reach,
) -> Result<Vec<(String, Option<String>)>, Decision> {
    let t = &expand_home(t);
    // A folder reached by `cd fac*` is a pattern too: what is written relative to it is a pattern.
    let dir_glob = !t.starts_with('/')
        && dir
            .as_deref()
            .is_some_and(|d| d.chars().any(|c| matches!(c, '*' | '?' | '[')));
    let has_glob = dir_glob || t.chars().any(|c| matches!(c, '*' | '?' | '['));
    let has_subst = t.contains("$(..)") || t.contains("`..`");
    if has_glob || has_subst {
        let relative = !t.starts_with('/');
        let abs_pattern = match (relative, dir) {
            (false, _) => t.to_string(),
            (true, Some(d)) => format!("{d}/{t}"),
            (true, None) => {
                return Err(unresolved_denial(
                    t,
                    part,
                    "the folder it is relative to was built at run time",
                ))
            }
        };
        let segs = pattern_segments(&abs_pattern);
        // A word that is nothing but a substitution or a variable is a path built at run time; it
        // passes unless it names a protected folder, as before.
        if !t.contains('/') && !has_glob && seg_tokens(t).iter().all(|k| matches!(k, Tok::Dyn)) {
            return Ok(vec![]);
        }
        // A command substitution that is the whole first segment stands for a path of its own.
        let first_free = relative
            && t.split('/')
                .next()
                .map(|s| seg_tokens(s).iter().all(|k| matches!(k, Tok::Dyn)) && !s.is_empty())
                .unwrap_or(false);
        let segs_for_match: Vec<String> = if first_free {
            let d_len = pattern_segments(dir.as_deref().unwrap_or("")).len();
            segs[d_len..].to_vec()
        } else {
            segs.clone()
        };
        // A substitution can also stand for one folder name under the working directory; a variable the
        // line does not assign is the environment's and is taken for an absolute path.
        let first_is_subst = first_free
            && t.contains('/')
            && t.split('/')
                .next()
                .map(|s| s.contains("$(..)") || s.contains("`..`"))
                .unwrap_or(false);
        let hit = could_land(table, &segs_for_match, first_free).or_else(|| {
            if first_is_subst {
                could_land(table, &segs, false)
            } else {
                None
            }
        });
        if let Some(hit) = hit {
            return Err(pattern_denial(t, part, &hit));
        }
        let mut prefix = String::new();
        for s in &segs {
            if seg_tokens(s).iter().any(|k| !matches!(k, Tok::Lit(_))) {
                break;
            }
            prefix.push('/');
            prefix.push_str(s);
        }
        let prefix = if first_free { String::new() } else { prefix };
        return Ok(if prefix.is_empty() {
            vec![]
        } else {
            vec![(resolve(&prefix, "/"), Some(abs_pattern.clone()))]
        });
    }
    match target_abs(t, dir) {
        Some(abs) => {
            if in_state_or_base(table, &abs) {
                return Err(state_denial());
            }
            if in_transcripts(table, &abs) {
                return Err(transcript_denial());
            }
            if is_protected_abs(table, &abs) {
                return Err(path_denial(&abs, part, reach));
            }
            Ok(vec![(abs, None)])
        }
        None => {
            if names_protected_segment(t) {
                return Err(unresolved_denial(
                    t,
                    part,
                    "part of it is built at run time",
                ));
            }
            if !(t.contains('$') || t.contains('`')) {
                return Err(unresolved_denial(
                    t,
                    part,
                    "the folder it is relative to was built at run time",
                ));
            }
            // A path that starts with a variable the command line does not assign (`$OUT`, `$TMPDIR/x`)
            // is whatever the environment holds, and that can be any path, a protected one included.
            // The same holds one step later (`./$X/p`, `src/../$X/p`, `a$X/p`): a variable in the first
            // segment or in any folder segment can be `..` or an absolute path. A variable only in
            // the last segment after known folders (`src/out-$N.txt`) is not judged.
            let segs: Vec<&str> = t.split('/').collect();
            // A last segment that is nothing but a variable (`./$OUT`, `src/$OUT`) is judged like a
            // folder segment: its value can hold slashes or `..`.
            let starts_with_var = segs.iter().enumerate().any(|(k, s)| {
                (k == 0 || k + 1 < segs.len() || is_whole_variable(s)) && holds_variable(s)
            });
            if starts_with_var {
                return Err(unresolved_denial(
                    t,
                    part,
                    "it holds a variable that this command does not assign, and the environment's value could be a protected place",
                ));
            }
            Ok(vec![])
        }
    }
}

/// The paths a write target stands for when `$0`, `$1`, `$@` and `$*` are bound to `args`
/// (`$0` is `args[0]`, `$1` is `args[1]`). A parameter with no word is replaced by a plain word.
fn bind_positional(t: &str, args: &[String]) -> Vec<String> {
    let c: Vec<char> = t.chars().collect();
    let all = |out: &mut Vec<String>| {
        let mut o = Vec::new();
        for a in args.iter().skip(1) {
            o.push(a.clone());
        }
        if o.is_empty() {
            o.push("_".to_string());
        }
        *out = o;
    };
    let mut variants: Vec<String> = vec![String::new()];
    let mut i = 0;
    while i < c.len() {
        if c[i] == '$'
            && i + 1 < c.len()
            && (c[i + 1].is_ascii_digit() || matches!(c[i + 1], '@' | '*'))
        {
            let vals: Vec<String> = if c[i + 1].is_ascii_digit() {
                let n = c[i + 1].to_digit(10).unwrap_or(0) as usize;
                vec![args.get(n).cloned().unwrap_or_else(|| "_".to_string())]
            } else {
                let mut v = Vec::new();
                all(&mut v);
                v
            };
            let mut next = Vec::new();
            for base in &variants {
                for v in &vals {
                    next.push(format!("{base}{v}"));
                }
            }
            variants = next;
            i += 2;
        } else {
            for v in variants.iter_mut() {
                v.push(c[i]);
            }
            i += 1;
        }
    }
    variants
}

/// A path with each positional parameter (`$1`, `$@`, `$*`) replaced by a plain word.
fn neutral_positional(t: &str) -> String {
    let c: Vec<char> = t.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '$'
            && i + 1 < c.len()
            && (c[i + 1].is_ascii_digit() || matches!(c[i + 1], '@' | '*'))
        {
            out.push('_');
            i += 2;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// Is a path segment nothing but one variable (`$X`, `${X}`)?
fn is_whole_variable(seg: &str) -> bool {
    let name = match seg.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
        Some(n) => n,
        None => match seg.strip_prefix('$') {
            Some(n) => n,
            None => return false,
        },
    };
    matches!(name, "@" | "*")
        || (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// Does a path segment hold a named variable (`$X`, `${X}`)?
fn holds_variable(seg: &str) -> bool {
    let c: Vec<char> = seg.chars().collect();
    c.windows(2).any(|w| {
        // Positional parameters (`$1`, `$@`, `$*`) are set by `set --` or the caller, which the
        // guard does not follow, so they count as variables the line does not assign.
        w[0] == '$' && (w[1] == '{' || w[1].is_alphanumeric() || matches!(w[1], '_' | '@' | '*'))
    })
}

fn pattern_denial(t: &str, part: &str, hit: &str) -> Decision {
    deny(
        &format!("write `{t}`"),
        &format!("{part} writes a path with a wildcard, a brace or a substitution that could land in {hit}, a protected place"),
        &[
            "Write the path out in full, for example `src/a.rs`, so that the guard can see where it lands.",
            "If you meant a file in your working copy, name it without a wildcard.",
        ],
        "The command names every path it writes and none of them can reach a protected place.",
    )
}

/// One character of a path segment that may stand for more than itself.
enum Tok {
    Lit(char),
    /// `*`, `?`, `[...]`: a glob.
    Glob(bool),
    /// A command substitution, a variable or a brace group: unknown text without a slash.
    Dyn,
}

fn seg_tokens(seg: &str) -> Vec<Tok> {
    let c: Vec<char> = seg.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '*' => out.push(Tok::Glob(true)),
            '?' => out.push(Tok::Glob(false)),
            '[' => {
                while i < c.len() && c[i] != ']' {
                    i += 1;
                }
                out.push(Tok::Glob(false));
            }
            '$' => {
                if i + 1 < c.len() && c[i + 1] == '(' {
                    // `$(..)`
                    i += 4;
                } else if i + 1 < c.len() && c[i + 1] == '{' {
                    while i < c.len() && c[i] != '}' {
                        i += 1;
                    }
                } else {
                    while i + 1 < c.len() && (c[i + 1].is_ascii_alphanumeric() || c[i + 1] == '_') {
                        i += 1;
                    }
                }
                out.push(Tok::Dyn);
            }
            '`' => {
                // "`..`"
                i += 3;
                out.push(Tok::Dyn);
            }
            ch => out.push(Tok::Lit(ch)),
        }
        i += 1;
    }
    out
}

fn tok_match(toks: &[Tok], s: &[char]) -> bool {
    match toks.split_first() {
        None => s.is_empty(),
        Some((Tok::Lit(c), rest)) => s.first() == Some(c) && tok_match(rest, &s[1..]),
        Some((Tok::Glob(false), rest)) => !s.is_empty() && tok_match(rest, &s[1..]),
        Some((_, rest)) => (0..=s.len()).any(|k| tok_match(rest, &s[k..])),
    }
}

fn seg_matches(pat: &str, lit: &str) -> bool {
    let chars: Vec<char> = lit.chars().collect();
    tok_match(&seg_tokens(pat), &chars)
}

/// What a segment would be if every wildcard were an `x`.
fn literalize(seg: &str) -> String {
    seg_tokens(seg)
        .iter()
        .map(|k| match k {
            Tok::Lit(c) => *c,
            _ => 'x',
        })
        .collect()
}

/// The segments of an absolute pattern, with the `.` and `..` that are written out folded away.
fn pattern_segments(p: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in p.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s.to_string()),
        }
    }
    out
}

/// Every place the guard protects, as absolute paths: the protected paths of the project and of each
/// working copy, the state folder and its parent, and the transcript root.
fn protected_roots(table: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let entries: Vec<String> = table["protected"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|p| p.trim_end_matches('/').to_string())
                .collect()
        })
        .unwrap_or_default();
    let root = table["root"].as_str().unwrap_or("");
    if !root.is_empty() {
        for e in &entries {
            out.push(format!("{}/{e}", root.trim_end_matches('/')));
        }
    }
    if table["workspace"].as_bool() != Some(true) {
        if let Some(ws) = table["worktrees"].as_array() {
            for w in ws {
                if let Some(base) = w["path"].as_str() {
                    for e in &entries {
                        out.push(format!("{}/{e}", base.trim_end_matches('/')));
                    }
                }
            }
        }
    }
    for r in other_repos(table) {
        out.push(format!("{r}/.git"));
    }
    let sd = state_dir(table);
    if !sd.is_empty() {
        out.push(sd.to_string());
    }
    if let Some(b) = state_base(table) {
        out.push(b.to_string());
    }
    let tr = transcript_root(table);
    if !tr.is_empty() {
        out.push(tr.to_string());
    }
    out
}

fn forbidden_abs(table: &Value, abs: &str) -> bool {
    in_state_or_base(table, abs) || in_transcripts(table, abs) || is_protected_abs(table, abs)
}

/// Could the pattern (the segments of an absolute path, or of a path that starts with an unknown
/// absolute prefix) name a path in a protected place? Gives the protected place.
fn could_land(table: &Value, segs: &[String], free_prefix: bool) -> Option<String> {
    for root in protected_roots(table) {
        let r = pattern_segments(&root);
        let n = r.len();
        if n == 0 {
            continue;
        }
        if free_prefix {
            // The first segment stands for any absolute prefix; the literal segments after it must
            // meet the protected path.
            let rest = &segs[1.min(segs.len())..];
            for j in 0..n {
                let overlap = rest.len().min(n - j);
                if overlap == 0 || j + rest.len() < n {
                    continue;
                }
                if (0..overlap).all(|k| seg_matches(&rest[k], &r[j + k])) {
                    let mut concrete = r.clone();
                    concrete.extend(rest[overlap..].iter().map(|s| literalize(s)));
                    if forbidden_abs(table, &format!("/{}", concrete.join("/"))) {
                        return Some(root);
                    }
                }
            }
        } else if segs.len() >= n && (0..n).all(|k| seg_matches(&segs[k], &r[k])) {
            let mut concrete = r.clone();
            concrete.extend(segs[n..].iter().map(|s| literalize(s)));
            if forbidden_abs(table, &format!("/{}", concrete.join("/"))) {
                return Some(root);
            }
        }
    }
    None
}

/// The alternatives of the brace groups in a word (`{a,b}/x` gives `a/x` and `b/x`). A group without a
/// comma (`{}`) is text. At most 64 alternatives; a larger set stays as it is.
fn brace_expand(t: &str) -> Vec<String> {
    let c: Vec<char> = t.chars().collect();
    let mut depth = 0;
    let mut start = None;
    for (i, ch) in c.iter().enumerate() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let s = start.unwrap();
                    let inner: Vec<char> = c[s + 1..i].to_vec();
                    // Split the inside at top-level commas.
                    let mut parts: Vec<String> = Vec::new();
                    let mut cur = String::new();
                    let mut d = 0;
                    for ch in &inner {
                        match ch {
                            '{' => {
                                d += 1;
                                cur.push(*ch);
                            }
                            '}' => {
                                d -= 1;
                                cur.push(*ch);
                            }
                            ',' if d == 0 => parts.push(std::mem::take(&mut cur)),
                            _ => cur.push(*ch),
                        }
                    }
                    parts.push(cur);
                    if parts.len() < 2 {
                        continue;
                    }
                    let pre: String = c[..s].iter().collect();
                    let post: String = c[i + 1..].iter().collect();
                    let mut out = Vec::new();
                    for p in parts {
                        out.extend(brace_expand(&format!("{pre}{p}{post}")));
                        if out.len() > 64 {
                            return vec![braces_to_star(t)];
                        }
                    }
                    return out;
                }
            }
            _ => {}
        }
    }
    vec![t.to_string()]
}

/// A word with every brace group read as a wildcard.
fn braces_to_star(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for ch in t.chars() {
        match ch {
            '{' => {
                if depth == 0 {
                    out.push('*');
                }
                depth += 1;
            }
            '}' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// A word with the variables that the same command line assigned replaced by their values.
fn subst_vars(t: &str, vars: &HashMap<String, String>) -> String {
    if !t.contains('$') {
        return t.to_string();
    }
    let c: Vec<char> = t.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '$' && i + 1 < c.len() {
            let mut default: Option<String> = None;
            let (name, next) = if c[i + 1] == '{' {
                match c[i + 2..].iter().position(|x| *x == '}') {
                    Some(e) => {
                        let inner: String = c[i + 2..i + 2 + e].iter().collect();
                        // `${NAME:-value}`, `${NAME-value}`, `${NAME:=value}`, `${NAME=value}`: the
                        // value stands in when the name is not set.
                        let n = inner
                            .chars()
                            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                            .count();
                        let (nm, rest) = inner.split_at(n);
                        for op in [":-", ":=", "-", "="] {
                            if n > 0 {
                                if let Some(d) = rest.strip_prefix(op) {
                                    default = Some(d.to_string());
                                    break;
                                }
                            }
                        }
                        let name = if default.is_some() {
                            nm.to_string()
                        } else {
                            inner
                        };
                        (name, i + 3 + e)
                    }
                    None => (String::new(), i + 1),
                }
            } else {
                let mut j = i + 1;
                while j < c.len() && (c[j].is_ascii_alphanumeric() || c[j] == '_') {
                    j += 1;
                }
                (c[i + 1..j].iter().collect::<String>(), j)
            };
            if let Some(v) = vars.get(&name) {
                out.push_str(v);
                i = next;
                continue;
            }
            if let Some(d) = default {
                out.push_str(&d);
                i = next;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

fn is_assign_word(w: &str) -> bool {
    match w.split_once('=') {
        Some((k, _)) => {
            !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.chars().next().unwrap().is_ascii_digit()
        }
        None => false,
    }
}

/// The names a `read` command sets.
fn read_names(rest: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut it = rest[1..].iter();
    while let Some(a) = it.next() {
        if matches!(a.as_str(), "-p" | "-n" | "-N" | "-t" | "-u" | "-d") {
            it.next();
        } else if a == "-a" {
            names.extend(it.next().cloned());
        } else if !is_flag(a) {
            names.push(a.clone());
        }
    }
    if names.is_empty() {
        names.push("REPLY".to_string());
    }
    names
}

/// The names `read` sets from input the guard cannot see, and that the line gives no value.
fn unread_vars(
    parsed: &[crate::shellparse::Parsed],
    known: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for p in parsed {
        let rest: Vec<String> = p
            .words
            .iter()
            .skip_while(|w| {
                [
                    "do", "then", "else", "elif", "if", "while", "until", "{", "!", "time",
                ]
                .contains(&w.as_str())
            })
            .cloned()
            .collect();
        if rest.first().map(|w| w.as_str()) == Some("read") && p.input.is_none() {
            for n in read_names(&rest) {
                if !known.contains_key(&n) {
                    out.push(n);
                }
            }
        }
    }
    out
}

/// The first of these names a word reads (`$f`, `${f}`, `${f:-x}`).
fn mentions_var(w: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .find(|n| {
            let plain = format!("${n}");
            let braced = format!("${{{n}");
            w.match_indices(&plain).any(|(i, _)| {
                !w[i + plain.len()..]
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphanumeric() || c == '_')
                    .unwrap_or(false)
            }) || w.contains(&braced)
        })
        .cloned()
}

/// The variables a command line assigns (`D=factory`, `export D=factory`), every value of each, in order.
fn assigned_vars(parsed: &[crate::shellparse::Parsed]) -> HashMap<String, Vec<String>> {
    let mut vars: HashMap<String, Vec<String>> = HashMap::new();
    const LEAD: &[&str] = &[
        "do", "then", "else", "elif", "if", "while", "until", "{", "!", "time",
    ];
    for p in parsed {
        let lead = p
            .words
            .iter()
            .take_while(|w| LEAD.contains(&w.as_str()))
            .count();
        let rest = &p.words[lead..];
        // `for d in a b c`: the variable takes each word in turn.
        if rest.first().map(|w| w.as_str()) == Some("for")
            && rest.get(2).map(|w| w.as_str()) == Some("in")
        {
            if let Some(name) = rest.get(1).filter(|n| is_assign_word(&format!("{n}="))) {
                for v in &rest[3..] {
                    vars.entry(name.to_string()).or_default().push(v.clone());
                }
            }
        }
        // `read d`: the variable takes the words of each line the command is given.
        if rest.first().map(|w| w.as_str()) == Some("read") {
            let names = read_names(rest);
            if let Some(text) = &p.input {
                for line in text.lines() {
                    let mut toks = line.split_whitespace();
                    for (n, name) in names.iter().enumerate() {
                        let v = if n + 1 == names.len() {
                            toks.by_ref().collect::<Vec<_>>().join(" ")
                        } else {
                            toks.next().unwrap_or("").to_string()
                        };
                        vars.entry(name.to_string()).or_default().push(v);
                    }
                }
            }
        }
        // `${D:=value}` and `${D=value}` assign D for the commands that follow.
        for w in &p.words {
            let mut from = 0;
            while let Some(o) = w[from..].find("${") {
                let start = from + o + 2;
                let Some(e) = w[start..].find('}') else { break };
                let inner = &w[start..start + e];
                let n = inner
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                    .count();
                let (nm, rest2) = inner.split_at(n);
                if n > 0 {
                    if let Some(v) = rest2.strip_prefix(":=").or_else(|| rest2.strip_prefix('=')) {
                        vars.entry(nm.to_string()).or_default().push(v.to_string());
                    }
                }
                from = start + e;
            }
        }
        let mut declared = false;
        for (k, w) in p.words.iter().enumerate() {
            if matches!(
                w.as_str(),
                "export" | "declare" | "typeset" | "readonly" | "local" | "set" | "env"
            ) {
                declared = true;
                continue;
            }
            if is_assign_word(w) && (declared || p.words[..k].iter().all(|x| is_assign_word(x))) {
                if let Some((n, v)) = w.split_once('=') {
                    vars.entry(n.to_string()).or_default().push(v.to_string());
                }
            } else if !declared {
                break;
            }
        }
    }
    vars
}

/// The readings of the line's variables a target is judged under: the last value of each, each
/// position of the assignments for all names together, and each value of one name with the others at
/// their last. A name assigned again later cannot hide an earlier value.
fn var_readings(vars: &HashMap<String, Vec<String>>) -> Vec<HashMap<String, String>> {
    let last: HashMap<String, String> = vars
        .iter()
        .filter_map(|(k, v)| v.last().map(|x| (k.clone(), x.clone())))
        .collect();
    let mut out = vec![last.clone()];
    let longest = vars.values().map(|v| v.len()).max().unwrap_or(0);
    for i in 0..longest {
        out.push(
            vars.iter()
                .filter_map(|(k, v)| v.get(i.min(v.len() - 1)).map(|x| (k.clone(), x.clone())))
                .collect(),
        );
    }
    for (k, v) in vars {
        for x in v {
            let mut m = last.clone();
            m.insert(k.clone(), x.clone());
            out.push(m);
        }
    }
    out
}

/// Judge what a path in an interpreter's text or a script name would open or run.
fn judge_text_path(
    table: &Value,
    t: &str,
    dir: &Option<String>,
    part: &str,
    reach: Reach,
) -> Option<Decision> {
    let abs = target_abs(t, dir)?;
    if in_state_or_base(table, &abs) {
        return Some(state_denial());
    }
    if is_protected_abs(table, &abs) {
        return Some(path_denial(&abs, part, reach));
    }
    None
}

/// The refusal of a git command, or `allow()`.
fn judge_git(table: &Value, dir: &str, w: &[String]) -> Decision {
    let main_branch = table["main_branch"].as_str().unwrap_or("main");
    let prefix = table["branch_prefix"].as_str().unwrap_or("bf/");
    let Some((i, sub)) = git_sub(w) else {
        return allow();
    };
    let target = {
        let mut d = dir.to_string();
        let mut j = 1;
        while j + 1 < w.len() && j < i {
            if w[j] == "-C" {
                d = resolve(&w[j + 1], dir);
            }
            j += 1;
        }
        zone_of(table, &d).kind.to_string()
    };
    let in_unit = target == "unit";
    let in_wc = in_unit || target == "checkpoint";
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
        return why("pointing git at another repository would hide what is being changed");
    }
    if w.windows(2).any(|p| {
        p[0] == "-c"
            && (p[1].starts_with("core.")
                || p[1].starts_with("alias.")
                || p[1].starts_with("filter."))
    }) {
        return why(
            "git config given on the command line can run programs or change what git sees",
        );
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
        "push" => {
            return why("nothing is pushed until the human decides; no engine command touches the network")
        }
        "update-ref" | "pull" | "worktree" | "reflog" | "gc" | "replace" | "filter-branch"
        | "symbolic-ref" => {
            return why("branch pointers and working copies are managed by the engine only")
        }
        "fetch" if rest.iter().any(|a| a.contains(':') || a.starts_with('+')) => {
            return why("a fetch with a refspec can move the main branch or a checkpoint branch")
        }
        // Taking in another branch's commits is the engine's step even in a unit's own working copy: it
        // would bring in files that the unit's scope never covered, and the guard cannot judge them.
        "merge" | "rebase" | "cherry-pick" | "am" if in_unit => {
            return why("taking in another branch's commits (merge, rebase, cherry-pick, am) brings in files outside the unit's scope; the engine joins the branches (`bf unit land`), and a change of yours is made with the editing tools");
        }
        // Moving the working tree onto another branch's files brings in what the unit's scope never
        // covered: `git switch`, `git read-tree`, and a reset that names the main branch or an engine
        // branch, or a hard reset onto a revision.
        "switch" | "read-tree" if in_unit => {
            return why("switching branches or reading another tree into the working copy brings in files outside the unit's scope; the engine joins the branches (`bf unit land`)");
        }
        "reset" if in_unit => {
            let before_paths: Vec<&str> = rest.iter().copied().take_while(|a| *a != "--").collect();
            let hard = before_paths
                .iter()
                .any(|a| matches!(*a, "--hard" | "--merge" | "--keep"));
            let revs: Vec<&str> = before_paths
                .iter()
                .copied()
                .filter(|a| !a.starts_with('-') && !a.starts_with("HEAD"))
                .collect();
            let names_branch = revs
                .iter()
                .any(|a| *a == main_branch || a.starts_with(prefix) || a.starts_with("origin/"));
            if names_branch || (hard && !revs.is_empty()) {
                return why("a reset onto the main branch, an engine branch or a revision brings in files outside the unit's scope; `git reset --hard` with no revision, and `git reset HEAD <file>`, stay open");
            }
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
                && !rest.contains(&"--")
                && rest
                    .iter()
                    .any(|a| a.starts_with(prefix) || a.contains(&format!(":{main_branch}"))) =>
        {
            return why("switching to or committing onto the engine's branches would change them behind its back")
        }
        // Git plumbing that writes the index or the working tree without a name the scope can judge, or
        // that turns a file into a symbolic link.
        "update-index" | "checkout-index" if in_wc => {
            let owned: Vec<String> = rest.iter().map(|s| s.to_string()).collect();
            let link = |s: &str| {
                deny(
                    &format!("run `git {sub}` that {s}"),
                    "a file of a working copy made into a symbolic link, or written without a name, can lead a later write to a place outside the unit's scope or to a protected place",
                    &[
                        "Change files with the editing tools, inside the unit's files.",
                        "Name each path on the command line; `git hash-object -w` and `git add <file>` stay open.",
                    ],
                    "The command names only paths inside your unit's files and makes no link.",
                )
            };
            if sub == "update-index" {
                if owned.iter().any(|a| long_option(a, "--index-info", 5)) {
                    return link("reads index entries from the input");
                }
                for (mode, _) in cacheinfo_entries(&owned) {
                    if u32::from_str_radix(mode.trim(), 8).map_or(true, |m| m & 0o170000 == 0o120000) {
                        return link("records a symbolic link (mode 120000)");
                    }
                }
            } else if owned.iter().any(|a| {
                (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('a'))
                    || long_option(a, "--all", 4)
                    || long_option(a, "--stdin", 5)
                    || long_option(a, "--prefix", 4)
                    || long_option(a, "--temp", 4)
            }) {
                return link("writes files that it does not name");
            }
        }
        "branch" => {
            let force = rest.iter().any(|x| {
                matches!(
                    *x,
                    "-d" | "-D" | "-f" | "--force" | "-m" | "-M" | "--delete" | "-c" | "-C" | "--move" | "--copy"
                )
            });
            let names = rest
                .iter()
                .any(|x| *x == main_branch || x.starts_with(prefix));
            if force && names {
                return why("the main branch and the engine's branches move only through the engine");
            }
        }
        _ => {}
    }
    allow()
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
    "page",
    "version",
    "help",
    "human-turn",
    "roles",
    "checks",
    "progress",
    "scan",
    "legal",
    "preset",
    "uiux",
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
/// The record kinds a working copy may put: a finding (builders' reviewers and the security and legal
/// auditors), a threat (the security expert) and a legal record (the lawyer and the legal auditor).
const PUT_KINDS: &[&str] = &["finding", "threat", "legal"];
const CP_BF: &[(&str, &[&str])] = &[
    ("ask", &[]),
    ("status", &[]),
    (
        "evidence",
        &["run", "verify", "fail", "unreachable", "attach"],
    ),
    ("record", &["get", "list", "put"]),
];

fn human_turn_denial() -> Decision {
    deny(
        "run `bf human-turn`",
        "only the harness hook records what the human typed; an agent that could record a turn could approve its own gate",
        &["Ask the human in chat for their decision.", "The hook records their message by itself."],
        "`bf log --tail 3` shows a human turn that you did not write.",
    )
}

/// Programs whose arguments are text to print, search or list, never a command to run. `bf human-turn`
/// as separate words after one of these is data.
const DATA_PROGRAMS: &[&str] = &[
    "echo", "printf", "grep", "egrep", "fgrep", "rg", "ag", "cat", "head", "tail", "less", "more",
    "ls", "wc", "which", "type", "whereis", "man", "stat", "diff", "test", "[", "true", "false",
    "cd", "git", "sed", "awk", "gawk", "mawk",
];

/// Does a command hold the engine's name followed later by the word `human-turn`, as separate words,
/// where the program is not one that only prints text? The engine's name may be a variable.
fn names_human_turn_call(words: &[String]) -> bool {
    let Some(i) = crate::shellparse::program_index(words) else {
        return false;
    };
    if DATA_PROGRAMS.contains(&basename(&words[i])) {
        return false;
    }
    let Some(h) = words.iter().rposition(|w| w == "human-turn") else {
        return false;
    };
    words[i..h]
        .iter()
        .any(|w| basename(w) == "bf" || w.contains('$') || w.contains('`'))
}

/// A call of `bf` that cannot be read, or that only the harness may make.
fn judge_call(c: &crate::shellparse::Call) -> Option<Decision> {
    let misuse = |why: String| {
        Some(deny(
            "run `bf` this way",
            &format!("{why}; the engine's commands are written out in full so that the guard can read them"),
            &["Write the command plainly, for example `bf status` or `bf report U-1 --result built`."],
            "The command starts with `bf` and a subcommand from `bf help`.",
        ))
    };
    if let Some(v) = &c.via {
        return misuse(format!(
            "`bf` is run through `{v}`, which hides what it does"
        ));
    }
    let w = &c.words;
    let mut j = 1;
    while j < w.len() && w[j].starts_with('-') {
        j += if w[j] == "-C" { 2 } else { 1 };
    }
    let sub = w.get(j)?;
    if sub == "human-turn" {
        return Some(human_turn_denial());
    }
    if sub.contains('$') || sub.contains('`') {
        return misuse("the `bf` subcommand is built at run time".into());
    }
    if !sub.chars().all(|c| c.is_ascii_lowercase() || c == '-') || !BF_KNOWN.contains(&sub.as_str())
    {
        return misuse(format!("`bf {sub}` is not a plain subcommand of the list"));
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

/// Is this an option that gives a program its code as text (`-c`, `-e`, `-m`)?
fn is_inline_flag(a: &str) -> bool {
    matches!(a, "-m" | "--eval" | "--command" | "--print")
        || (a.starts_with('-')
            && !a.starts_with("--")
            && (a.ends_with('c') || a.ends_with('e') || a.ends_with('E')))
}

fn check_bash(table: &Value, cwd: &str, cmd: &str) -> Decision {
    // Where would a shell really run `bf`? Only a word in command position counts; the word `bf` as an
    // argument, in a pattern or in a heredoc for another program is data.
    let calls = crate::shellparse::bf_calls(cmd);
    if crate::shellparse::runtime_program_with_word(cmd, "human-turn")
        || crate::shellparse::interpreter_arg_with_phrase(cmd, "bf human-turn")
    {
        return human_turn_denial();
    }
    for c in &calls {
        if let Some(d) = judge_call(c) {
            return d;
        }
    }
    let parsed = crate::shellparse::commands(cmd);
    // The engine's name followed by `human-turn` in any argument position, behind any wrapper program the
    // parse does not know (`stdbuf -o0 bf human-turn`, `find . -exec bf human-turn ;`), is the same call.
    if parsed.iter().any(|p| names_human_turn_call(&p.words)) {
        return human_turn_denial();
    }
    // `cargo run -- human-turn` runs the engine's own binary with that subcommand.
    if parsed.iter().any(|p| {
        crate::shellparse::program_index(&p.words).is_some_and(|i| {
            let w = &p.words[i..];
            basename(&w[0]) == "cargo"
                && w.get(1).is_some_and(|a| a == "run" || a == "r")
                // Cargo hands the words after the program's name to it, with or without `--`.
                && w.iter().skip(2).any(|a| a == "human-turn")
        })
    }) {
        return human_turn_denial();
    }
    // The state folder, however it is spelled: no command word or redirect target may name it. The body
    // of a heredoc is not a word.
    for p in &parsed {
        for w in p.words.iter().chain(p.redirects.iter()) {
            if names_state(table, w) {
                return state_denial();
            }
        }
    }
    let here = zone_of(table, cwd);
    let in_worker = here.kind == "unit";
    let in_cp = here.kind == "checkpoint";

    // -- bf subcommands, for each real call
    for call in &calls {
        let w = &call.words;
        {
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
                    // The resolver of an open sync reports from the checkpoint's working copy.
                    || (in_cp && sub == "report" && here.sync)
                    // A built-stage role (an auditor) reports on a checkpoint with its own token.
                    || (in_cp
                        && sub == "report"
                        && w.windows(2)
                            .any(|x| x[0] == "--role" && crate::roles::is_new_role(&x[1])))
                    || list.iter().any(|(c, subs)| *c == sub && (subs.is_empty() || subs.contains(&second)))
                    // Of `record put`, only a finding.
                    && !(sub == "record"
                        && second == "put"
                        && !PUT_KINDS.contains(&w.get(i + 2).map(|x| x.as_str()).unwrap_or("")));
                if !ok {
                    return deny(
                        &format!("run `bf {sub} {second}` from a working copy").replace("  ", " "),
                        if in_worker {
                            "a worker may report, file a finding, run evidence and read the status, and put a threat or a legal record with the token of a role that writes it; nothing else"
                        } else {
                            "a checkpoint's working copy is for running its checks, reading state, and an auditor's report and records with its token"
                        },
                        &["Use `bf report`, `bf evidence run|verify|fail|unreachable`, `bf record put finding|threat|legal`, `bf record get|list` or `bf status`.", "Ask the conductor for anything else."],
                        "The command you need is on that list.",
                    );
                }
            }
        }
    }

    // -- every command: where it runs, what it would execute, where it would write or delete
    let root = table["root"].as_str().unwrap_or("");
    let all_vars = assigned_vars(&parsed);
    let vars: HashMap<String, String> = all_vars
        .iter()
        .filter_map(|(k, v)| v.last().map(|x| (k.clone(), x.clone())))
        .collect();
    let readings = var_readings(&all_vars);
    let unread = unread_vars(&parsed, &all_vars);
    let mut dirs: HashMap<usize, Option<String>> = HashMap::new();
    dirs.insert(0, Some(cwd.to_string()));
    // The shells whose code sets its own positional parameters with `set --`.
    let sets_positional: std::collections::HashSet<usize> = parsed
        .iter()
        .filter(|q| {
            crate::shellparse::program_index(&q.words).is_some_and(|i| {
                crate::shellparse::basename(&q.words[i]) == "set"
                    && q.words.get(i + 1).is_some_and(|a| a == "--")
            })
        })
        .map(|q| q.scope)
        .collect();
    for p in &parsed {
        let start: Option<String> = match dirs.get(&p.scope) {
            Some(d) => d.clone(),
            None => dirs
                .get(&p.parent)
                .cloned()
                .unwrap_or_else(|| Some(cwd.to_string())),
        };
        let dir_str = start.clone().unwrap_or_else(|| cwd.to_string());
        let mut after = start.clone();

        // A path in the state folder, whatever the program and however it is spelled.
        for w in p.words.iter().chain(p.redirects.iter()) {
            for t in tokens(w) {
                if (t.contains('/') || t == "~") && in_state_or_base(table, &resolve(&t, &dir_str))
                {
                    return state_denial();
                }
            }
        }

        let mut writes: Vec<(String, String, Reach)> = Vec::new();
        for t in &p.redirects {
            if t.starts_with("/dev/") {
                continue;
            }
            writes.push((t.clone(), format!("the redirect `> {t}`"), Reach::Write));
        }
        if let Some(i) = crate::shellparse::program_index(&p.words) {
            let prog = basename(&p.words[i]).to_string();
            let args = &p.words[i + 1..];
            if prog != "bf" {
                // -- leaving the working copy, and the folder later commands run in
                if matches!(prog.as_str(), "cd" | "pushd" | "popd") {
                    let target_text = args
                        .iter()
                        .find(|a| !is_flag(a))
                        .map(|s| subst_vars(s, &vars))
                        .unwrap_or_else(|| "~".to_string());
                    let target = target_text.as_str();
                    if in_worker || in_cp {
                        let abs = resolve(target, &dir_str);
                        if under(&abs, &here.base).is_none() {
                            return deny(
                                "leave your working copy",
                                "from a working copy, commands run in that working copy; changing folder would let later commands run as if from the project",
                                &[&format!("Stay inside {}.", here.base)],
                                "Every command runs in your working copy.",
                            );
                        }
                    }
                    after = if target == "-" || prog == "popd" {
                        None
                    } else {
                        target_abs(target, &start)
                    };
                }

                // -- git
                if prog == "git" {
                    let d = judge_git(table, &dir_str, &p.words[i..]);
                    if !d.allow {
                        return d;
                    }
                }

                // -- what it would execute
                let mut runs: Vec<(String, String)> = Vec::new();
                if p.words[i].contains('/') {
                    runs.push((p.words[i].clone(), format!("the program `{}`", p.words[i])));
                }
                if matches!(prog.as_str(), "source" | ".") {
                    if let Some(a) = args.iter().find(|a| !is_flag(a)) {
                        runs.push((a.clone(), format!("`{prog} {a}`")));
                    }
                }
                let shell = SHELLS.contains(&prog.as_str());
                let interp = is_interpreter(&prog);
                if (shell || (interp && prog != "awk" && prog != "gawk" && prog != "mawk"))
                    && !args.iter().any(|a| is_inline_flag(a))
                {
                    if let Some(a) = args.iter().find(|a| !is_flag(a) && a.as_str() != "-") {
                        runs.push((a.clone(), format!("the script `{a}` given to `{prog}`")));
                    }
                }
                for (t, part) in runs {
                    if let Some(d) = judge_text_path(table, &t, &start, &part, Reach::Run) {
                        return d;
                    }
                }

                // -- a makefile read from standard input: its recipe lines are shell commands that run
                if matches!(prog.as_str(), "make" | "gmake") {
                    let from_stdin = args.windows(2).any(|w| {
                        (w[0] == "-f" || w[0] == "--file" || w[0] == "--makefile")
                            && (w[1] == "/dev/stdin" || w[1] == "-")
                    }) || args.iter().any(|a| {
                        a == "-f/dev/stdin"
                            || a == "-f-"
                            || a == "--file=/dev/stdin"
                            || a == "--file=-"
                    });
                    if let (true, Some(text)) = (from_stdin, &p.input) {
                        let recipe: String = text
                            .lines()
                            .filter(|l| l.starts_with('\t'))
                            .map(|l| l.trim_start_matches(['\t', '@', '-', '+']))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !recipe.trim().is_empty() {
                            let d = check_bash(table, &dir_str, &recipe);
                            if !d.allow {
                                return d;
                            }
                        }
                    }
                }

                // -- text fed to an interpreter: only paths in it count, never words
                if interp {
                    // Code on standard input and code after an inline flag are judged alike: a call of
                    // the command that records a human message is refused, the phrase alone is not.
                    let mut code = args.join("\n");
                    if let Some(h) = &p.input {
                        code.push('\n');
                        code.push_str(h);
                    }
                    if code_calls_human_turn(&code) {
                        return human_turn_denial();
                    }
                    // A path the code opens for writing is a write like any other: the protected places
                    // and, in a unit's working copy, the unit's file scope judge it.
                    let made = made_literals(&code);
                    let mut written = written_literals(&code);
                    if matches!(prog.as_str(), "awk" | "gawk" | "mawk") {
                        written.extend(awk_written(&code));
                    }
                    // Code that deletes or changes a file by whatever it is handed (a file object, a
                    // variable, `f.name`) names its path outside the call: every path-like literal it
                    // holds is judged as a write, as an absolute one already is.
                    // `sys.path` is a list of folders to import from: `sys.path.remove('src')` is no delete.
                    // A delete that is given a string literal first names its path in the call.
                    let unnamed = mask_literal_deletes(&code.replace("sys.path", "sys_path"));
                    if writes_without_literal(&unnamed) && code_may_write(&code) {
                        for t in literals_for_unnamed_writes(&code, &start) {
                            if !written.contains(&t) {
                                written.push(t);
                            }
                        }
                    }
                    for t in written {
                        let reach = if made.contains(&t) {
                            Reach::Make
                        } else {
                            Reach::Write
                        };
                        writes.push((
                            t.clone(),
                            format!("the code given to `{prog}` (it writes the path `{t}`)"),
                            reach,
                        ));
                    }
                    // A real path in the code is only a read where the code has nothing that writes: a
                    // record opened by its absolute path is judged as it is by a relative one. The state
                    // folder is closed to reads too.
                    let may_write = code_may_write(&code);
                    let reads = read_literals(&code);
                    for t in interpreter_paths(p, args) {
                        let part = format!("the code given to `{prog}` (it holds the path `{t}`)");
                        if let Some(abs) = target_abs(&t, &start) {
                            if in_state_or_base(table, &abs) {
                                return state_denial();
                            }
                        }
                        // A path that every mention reads (opened for reading, or the source of a copy)
                        // is not a write; the paths the code writes are judged above.
                        let only_read = !writes_without_literal(&unnamed)
                            && reads
                                .get(&t)
                                .is_some_and(|n| *n >= code.matches(&t).count());
                        if may_write && !only_read {
                            if let Some(d) = judge_text_path(table, &t, &start, &part, Reach::Text)
                            {
                                return d;
                            }
                        }
                    }
                }

                // -- what it would write or delete
                let deleting = matches!(prog.as_str(), "rm" | "rmdir" | "unlink" | "shred")
                    || (prog == "find" && args.iter().any(|a| a == "-delete"));
                let mut writer_in: Vec<String> = args.to_vec();
                let via_xargs = p.words[..i].iter().any(|w| basename(w) == "xargs");
                if via_xargs {
                    // xargs adds its input to the writer's arguments (or puts it where the
                    // replacement string stands).
                    let mut repl: Option<String> = None;
                    for (k, w) in p.words[..i].iter().enumerate() {
                        if w == "-I" {
                            repl = p.words.get(k + 1).cloned();
                        } else if let Some(r) = w.strip_prefix("-I").filter(|r| !r.is_empty()) {
                            repl = Some(r.to_string());
                        } else if w == "-i" || w == "--replace" {
                            repl = Some("{}".to_string());
                        } else if let Some(r) = w
                            .strip_prefix("--replace=")
                            .or_else(|| w.strip_prefix("-i"))
                            .filter(|r| !r.is_empty())
                        {
                            repl = Some(r.to_string());
                        }
                    }
                    // `-a FILE` and `--arg-file=FILE` read the input from a file, not the pipe.
                    let arg_file = p.words[..i]
                        .iter()
                        .any(|w| w == "-a" || w == "--arg-file" || w.starts_with("--arg-file="));
                    let known_input = if arg_file { &None } else { &p.input };
                    match known_input {
                        Some(text) => {
                            let toks: Vec<String> = text
                                .replace("\\0", "\n")
                                .split_whitespace()
                                .map(|s| s.to_string())
                                .collect();
                            match repl {
                                Some(r) => {
                                    writer_in = Vec::new();
                                    for t in &toks {
                                        writer_in.extend(args.iter().map(|a| a.replace(&r, t)));
                                    }
                                }
                                None => writer_in.extend(toks),
                            }
                        }
                        None => {
                            // Does the unknown input become a write target? Without a replacement
                            // string xargs appends it to the arguments; with one it lands where the
                            // string stands.
                            const SENTINEL: &str = "\u{1}xargs-input";
                            let mut probe = args.to_vec();
                            probe.push(SENTINEL.to_string());
                            let becomes_target = match &repl {
                                Some(r) => writer_args(&prog, args, &start)
                                    .iter()
                                    .any(|t| t.contains(r.as_str())),
                                None => writer_args(&prog, &probe, &start)
                                    .iter()
                                    .any(|t| t == SENTINEL),
                            };
                            if becomes_target && !deletes_or_edits(&prog, args) {
                                return deny(
                                    &format!("run `{prog}` through `xargs` on input the guard cannot read"),
                                    &format!("`{prog}` would write the files `xargs` hands it, and the guard cannot tell which files those are"),
                                    &[
                                        "Name the files in the command, for example `cp src/a.txt out/a.txt`.",
                                        "Or feed `xargs` from `echo` or `printf` with the paths written out.",
                                    ],
                                    "The command names every path it writes.",
                                );
                            }
                            // Unknown names handed to a copy into a folder land in that folder as a
                            // wildcard: judged like `dir/*`.
                            if prog == "mv" || COPIERS.contains(&prog.as_str()) {
                                writer_in = probe.clone();
                            }
                            if deletes_or_edits(&prog, args) {
                                return deny(
                                    &format!("run `{prog}` through `xargs` on input the guard cannot read"),
                                    &format!("`{prog}` would delete or edit in place the files `xargs` hands it, and the guard cannot tell which files those are"),
                                    &[
                                        "Name the files in the command, for example `rm src/a.o src/b.o`.",
                                        "Or feed `xargs` from `echo` or `printf` with the paths written out.",
                                    ],
                                    "The command names every path it deletes or edits.",
                                );
                            }
                        }
                    }
                }
                // A variable set by `read` from input the guard cannot see stands for unknown paths.
                if !via_xargs && !unread.is_empty() {
                    let destructive = deletes_or_edits(&prog, args);
                    let hit = if destructive {
                        args.iter()
                            .filter(|a| !is_flag(a))
                            .find_map(|a| mentions_var(a, &unread))
                    } else {
                        writer_args(&prog, args, &start)
                            .iter()
                            .find_map(|a| mentions_var(a, &unread))
                    };
                    if let Some(n) = hit {
                        if destructive {
                            return deny(
                                &format!("run `{prog}` on `${n}`, set by `read` from input the guard cannot read"),
                                &format!("`{prog}` would delete or edit in place the files `${n}` names, and the guard cannot tell which files those are"),
                                &[
                                    "Name the files in the command, for example `rm src/a.o src/b.o`.",
                                    "Or loop over paths written out: `for f in src/a.o src/b.o; do rm \"$f\"; done`.",
                                ],
                                "The command names every path it deletes or edits.",
                            );
                        }
                        return deny(
                            &format!("run `{prog}` on `${n}`, set by `read` from input the guard cannot read"),
                            &format!("`{prog}` would write the files `${n}` names (a copy or compressed file lands on them), and the guard cannot tell which files those are"),
                            &[
                                "Name the destination in the command, for example `cp /tmp/new src/a.txt`.",
                                "Or loop over destinations written out: `for f in src/a.txt src/b.txt; do cp /tmp/new \"$f\"; done`.",
                            ],
                            "The command names every path it writes.",
                        );
                    }
                }
                // Unpacking an archive into the working copy with no member named: the members that land
                // are the archive's own, so the unit's file scope cannot be applied to them.
                if (in_worker || in_cp) && unpacks_unnamed(&prog, &writer_in) {
                    if let Some(base) = writer_args(&prog, &writer_in, &start).last() {
                        if let Some(abs) = target_abs(base, &start) {
                            if under(&abs, &here.base).is_some() {
                                return deny(
                                    &format!("unpack an archive into {abs} without naming its members"),
                                    &format!("`{prog}` would write every member of the archive into your working copy, and the guard cannot tell whether each one is a file your unit may touch"),
                                    &[
                                        "Name the members to unpack, for example `tar xf a.tar src/hello.txt`.",
                                        "Or unpack into a folder outside the project, such as `tar xf a.tar -C /tmp/x`, and copy the files you need with `cp`.",
                                    ],
                                    "Every file the command writes is named and lies in your unit's files.",
                                );
                            }
                        }
                    }
                }
                // A target that is a folder holding a protected place (the project's root, `.claude`,
                // `.git`) is deleted, moved, re-permissioned, linked or checked out as a whole: refused
                // like a target inside the place.
                {
                    let all = writer_args(&prog, &writer_in, &start);
                    let tp = transfer_parts(&prog, &writer_in);
                    let covering: Vec<&String> = match prog.as_str() {
                        // Taking permissions away (`chmod -R go-w .`) gives no access to anything.
                        "chmod" if chmod_only_removes(&writer_in) => Vec::new(),
                        "rm" | "rmdir" | "unlink" | "shred" | "chmod" | "chown" | "git" => {
                            all.iter().collect()
                        }
                        // Only `-delete` removes what the starting point holds; `-exec` runs a
                        // command that is judged by itself.
                        "find" if args.iter().any(|a| a == "-delete") => all.iter().collect(),
                        // Every source of a move goes, with or without `-t`.
                        "mv" => tp.sources.iter().collect(),
                        // A link that replaces a folder (`ln -sfn x .claude`) takes it with it; a link
                        // made inside a folder does not.
                        "ln" => tp
                            .dest
                            .iter()
                            .filter(|d| !goes_into_folder(&tp, d, &start))
                            .collect(),
                        _ => Vec::new(),
                    };
                    for t in covering {
                        if let Some(abs) = target_abs(t, &start) {
                            if let Some(hit) = covers_protected(table, &abs) {
                                return cover_denial(&abs, &hit, &prog, t);
                            }
                        }
                    }
                    // A folder copied or synchronised into a folder that holds a protected place, from a
                    // source that is, or holds, a protected folder's name or its whole contents. (Unpacking
                    // an archive there is not judged: see docs/engine.md.)
                    if matches!(prog.as_str(), "cp" | "rsync") {
                        let recursive = prog == "rsync"
                            || args.iter().any(|a| {
                                a == "--recursive"
                                    || a == "--archive"
                                    || (a.starts_with('-')
                                        && !a.starts_with("--")
                                        && a[1..].contains(['r', 'R', 'a']))
                            });
                        let dest = tp.dest.clone();
                        // `cp -rT a b` puts what `a` holds into `b`, whatever `a` is called.
                        if let (true, Some(dest)) = (recursive || tp.plain, dest) {
                            let lands = target_abs(&dest, &start)
                                .and_then(|abs| covers_protected(table, &abs).map(|h| (abs, h)));
                            if let Some((abs, hit)) = lands {
                                for s in &tp.sources {
                                    let trimmed = s.trim_end_matches('/');
                                    let last = trimmed.rsplit('/').next().unwrap_or("");
                                    let whole = tp.plain
                                        || s.ends_with('/')
                                        || s.as_str() == "."
                                        || s.as_str() == ".."
                                        || last == "."
                                        || last == ".."
                                        // A pattern in the last segment stands for the entries
                                        // of a folder, any of which may be a protected name.
                                        || last.contains(['*', '?', '[']);
                                    if whole
                                        || matches!(last, "factory" | ".bf" | ".claude" | ".git")
                                    {
                                        return cover_denial(&abs, &hit, &prog, s);
                                    }
                                }
                            }
                        }
                    }
                }
                // Unpacking, with no member named, into a folder that holds a protected place
                // (`tar xf a.tar -C .claude`): whatever the archive holds lands there.
                if matches!(prog.as_str(), "tar" | "bsdtar" | "gtar" | "unzip")
                    && unpacks_unnamed(&prog, &writer_in)
                {
                    if let Some(d) = unpack_dir(&prog, &writer_in) {
                        if let Some(abs) = target_abs(&d, &start) {
                            if abs.trim_end_matches('/') != root.trim_end_matches('/') {
                                if let Some(hit) = covers_protected(table, &abs) {
                                    return cover_denial(&abs, &hit, &prog, &d);
                                }
                            }
                        }
                    }
                }
                // A `find` with a name filter that deletes, or hands its matches to a program that
                // writes: what counts is whether a protected file can be one of the matches.
                if prog == "find" && !in_worker && !in_cp {
                    if let Some(d) = find_filter_denial(table, args, &start) {
                        return d;
                    }
                }
                let git_removes = prog == "git"
                    && git_sub(&p.words[i..]).is_some_and(|(k, s)| {
                        s == "rm" && !git_index_only("rm", &p.words[i + k + 1..])
                    });
                for t in writer_args(&prog, &writer_in, &start) {
                    let part = if via_xargs {
                        format!("`{prog}` started by `xargs`, with the path `{t}`")
                    } else {
                        format!("`{prog}` with the argument `{t}`")
                    };
                    writes.push((
                        t,
                        part,
                        if deleting
                            || git_removes
                            || (prog == "find"
                                && !args.iter().any(|a| a.starts_with("-fprint") || a == "-fls"))
                        {
                            Reach::Delete
                        } else if matches!(prog.as_str(), "chmod" | "chown" | "chgrp")
                            && !(prog == "chmod" && chmod_only_removes(&writer_in))
                            && writer_in.iter().any(|a| {
                                a == "--recursive"
                                    || (a.starts_with('-')
                                        && !a.starts_with("--")
                                        && a[1..].contains('R'))
                            })
                        {
                            Reach::Perm
                        } else if prog == "mkdir"
                            || (prog == "install"
                                && args.iter().any(|a| a == "-d" || a == "--directory"))
                        {
                            Reach::Make
                        } else {
                            Reach::Write
                        },
                    ));
                }
            }
        }

        let mut expanded: Vec<(String, String, Reach)> = Vec::new();
        for (t, part, reach) in writes {
            // In a `sh -c` string, `$0`, `$1` and `$@` are the words given after the string: they
            // are judged as the paths they stand for. A string that sets its own parameters with
            // `set --` keeps them as variables the line does not assign. In a script fed to a
            // shell the parameters are unknown and stand for a path the caller named.
            if p.scope != 0 && !sets_positional.contains(&p.scope) {
                match &p.bound {
                    Some(args) => {
                        for v in bind_positional(&t, args) {
                            expanded.push((v, part.clone(), reach));
                        }
                    }
                    None => expanded.push((neutral_positional(&t), part, reach)),
                }
            } else {
                expanded.push((t, part, reach));
            }
        }
        for (t, part, reach) in expanded {
            match judge_target(table, &t, &start, &part, reach, &readings) {
                Err(d) => return d,
                Ok(found) => {
                    for (abs, pattern) in found {
                        // -- writes from a working copy stay in it
                        let inside_root =
                            under(&abs, root).is_some() || repo_containing(table, &abs).is_some();
                        let own = under(&abs, &here.base).is_some();
                        if (in_cp || in_worker) && inside_root && !own {
                            return deny(
                            "write outside your working copy",
                            &format!(
                                "{part} would write {abs}, which is somewhere else in the project; you work in {}",
                                here.base
                            ),
                            &["Write only inside your working copy, in the files your unit may touch."],
                            "Every path you write is inside your working copy.",
                        );
                        }
                        // -- inside the unit's own copy, the unit's file scope governs, as for any write tool
                        // (A folder given as a whole, and the folder before a wildcard, are judged for where
                        // they are, not for the scope of the files in them.)
                        // A pattern is held to the scope as written: it must match a scope entry itself.
                        // (Unpacking or downloading into the copy's root names no member, so it is a known
                        // gap of the scope rule; docs/engine.md says so.)
                        // In a checkpoint's copy the files of an open sync are the only ones to write.
                        if (in_worker || in_cp) && own {
                            // The root of the working copy is a folder like any other: deleting it
                            // deletes files outside the scope too.
                            if abs == here.base
                                && pattern.is_none()
                                && matches!(reach, Reach::Delete | Reach::Perm)
                            {
                                let verb = if matches!(reach, Reach::Perm) {
                                    "change the permissions of"
                                } else {
                                    "delete"
                                };
                                return deny(
                                    &format!("{verb} the whole working copy"),
                                    &format!("{part} would {verb} {abs}, the whole working copy, and with it files outside the file scope of {}", here.id),
                                    &[
                                        "Name the files you mean, inside the scope, for example `rm src/old.rs`.",
                                        "To discard your own changes use `git restore <file>` on a file in scope.",
                                    ],
                                    "The command names only paths inside your unit's files.",
                                );
                            }
                            let checked = match &pattern {
                                None if abs != here.base => Some(abs.clone()),
                                Some(pat) => Some(pat.clone()),
                                None => None,
                            };
                            if let Some(c) = checked {
                                let d = check_write(table, cwd, &c);
                                // Position: removing (never creating) a backup copy of an allowed file,
                                // the leftover of an editor or of `sed -i.bak`, is housekeeping on a file
                                // the unit may touch. Only a delete of an exact `.bak`, `.orig` or `~`
                                // name beside an allowed file passes; any other delete stays in scope.
                                let backup_of_allowed = matches!(reach, Reach::Delete)
                                    && pattern.is_none()
                                    && ["~", ".bak", ".orig"].iter().any(|suf| {
                                        c.strip_suffix(suf)
                                            .filter(|stem| !stem.is_empty() && !stem.ends_with('/'))
                                            .is_some_and(|stem| check_write(table, cwd, stem).allow)
                                    });
                                // Making a folder that leads to a path the unit may write is allowed.
                                let leads = !d.allow
                                    && matches!(reach, Reach::Make)
                                    && pattern.is_none()
                                    && under(&c, &here.base).is_some_and(|rel| {
                                        let z = zone_of(table, &c);
                                        z.kind == "unit"
                                            && !is_protected_in_wc(table, rel)
                                            && rel != ".git"
                                            && !rel.starts_with(".git/")
                                            && leads_to_scope(&z.files, rel)
                                    });
                                if !d.allow && !backup_of_allowed && !leads {
                                    return d;
                                }
                            }
                        }
                    }
                }
            }
        }
        dirs.insert(p.scope, after);
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
    // While a checkpoint fix is open, the engine hands the guard the files that fix may change.
    crate::ckfix::overlay(&mut table);
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

/// The absolute paths a command names as words, redirect targets or the paths in code given to an
/// interpreter: the places whose project may hold the table that judges the call.
fn absolute_words(cmd: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in crate::shellparse::commands(cmd) {
        for w in p.words.iter().chain(p.redirects.iter()) {
            out.extend(tokens(w));
        }
        if let Some(i) = crate::shellparse::program_index(&p.words) {
            if is_interpreter(basename(&p.words[i])) {
                out.extend(interpreter_paths(&p, &p.words[i + 1..]));
            }
        }
    }
    out.into_iter()
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
            for a in absolute_words(c) {
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

    #[test]
    fn a_folder_that_leads_to_a_path_the_unit_may_write_may_be_made() {
        let files: Vec<String> = ["src/hello.txt", "docs/", "lib/deep/x.rs", "gen/**"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for ok in ["src", "lib", "lib/deep", "gen", "gen/a/b", "docs"] {
            assert!(leads_to_scope(&files, ok), "{ok}");
        }
        for no in [
            "other",
            "src/hello.txt",
            "lib/deep/x.rs",
            "lib/other",
            "srcx",
            "",
        ] {
            assert!(!leads_to_scope(&files, no), "{no}");
        }
    }

    #[test]
    fn a_folder_above_a_protected_place_covers_it() {
        let t = table();
        for above in ["/p", "/", "/p/.claude", "/p/.claude/"] {
            assert!(covers_protected(&t, above).is_some(), "{above}");
        }
        for not in ["/p/src", "/p/.bf/work/U-1", "/q", "/p/factory"] {
            assert!(covers_protected(&t, not).is_none(), "{not}");
        }
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
    fn the_git_folder_of_a_nested_or_a_sibling_repository_is_closed_to_the_shell() {
        let mut t = table();
        t["workspace"] = json!(true);
        t["repos"] = json!([{"name":"a","path":"/p/app"},{"name":"b","path":"/other/lib"}]);
        let run = |cmd: &str| {
            decide(
                &json!({"tool_name":"Bash","tool_input":{"command":cmd},"cwd":"/p"}),
                &t,
            )
            .allow
        };
        for cmd in [
            "echo x > app/.git/hooks/post-merge",
            "cp /tmp/h app/.git/hooks/",
            "printf a >> /other/lib/.git/config",
            "rm -rf app/.git",
            "mv /other/lib/.git /tmp/x",
            "chmod +x app/.git/hooks/pre-commit",
            "cd app && echo x > .git/config",
        ] {
            assert!(!run(cmd), "{cmd}");
        }
        for cmd in [
            "git -C app log -1",
            "cat app/.git/config",
            "cp /other/lib/.git/config /tmp/c",
            "echo x > app/notes.txt",
            "ls /other/lib/.git/hooks",
        ] {
            assert!(run(cmd), "{cmd}");
        }
    }

    #[test]
    fn the_word_bf_as_data_is_not_a_call_of_the_engine() {
        for cmd in [
            "cargo test -p bf --test checks_catalogue",
            "cargo test --package=bf --test x -- --nocapture",
            "bf evidence run X-1 --unit U-1 -- cargo test -p bf --test checks_catalogue",
            "git status --short | head; ls; ls crates/bf crates/bf/src crates/bf/tests; wc -l crates/bf/src/*.rs",
            "grep -n \"checks\\|\\\"human-turn\\\"\\|fn cmd_\" src/cli.rs",
            "cat > checks.rs <<'EOF'\n//! The checks, listed by `bf checks propose`.\n/// Run `bf status` first.\nfn main() {}\nEOF",
            "python3 - <<'EOF'\n# docs say `bf 2` then `bf human-turn`\nprint(1)\nEOF",
            "sed -i 's|old|the `bf checks propose` command prints|' notes.md",
            "python3 -I /tmp/s/edit.py /tmp/d/crates/bf /tmp/s/new.rs && cd /tmp/d",
            "echo bf human-turn is a command of the engine",
            "command -v bf",
            "which bf; ls ~/bin/bf",
            "# bf human-turn\nls",
        ] {
            assert!(bash(cmd, "/p"), "should pass: {cmd}");
        }
        // The same in a unit's working copy.
        for cmd in [
            "cargo test -p bf --test checks_catalogue",
            "bf evidence run X-1 --unit U-1 -- cargo test -p bf --test checks_catalogue",
            "ls crates/bf/src && wc -l crates/bf/src/*.rs",
        ] {
            assert!(
                bash(cmd, "/p/.bf/work/U-1"),
                "should pass in a working copy: {cmd}"
            );
        }
    }

    #[test]
    fn a_call_in_command_position_is_judged_as_before() {
        for cmd in [
            "bf human-turn",
            "ls && bf human-turn < p.json",
            "X=1 bf human-turn",
            "time bf human-turn",
            "nohup bf human-turn &",
            "B=bf; $B human-turn < p.json",
            "sh -c \"printf x | bf human-turn\"",
            "echo $(bf human-turn)",
            "bf evidence run X-1 -- bf human-turn",
            "env bf status",
            "sh -c 'bf status'",
            "bash -c \"cd x && bf status\"",
            "eval \"bf status\"",
            "echo $(bf status)",
            "echo `bf status`",
            "ls | xargs bf status",
            "bf $(echo human)-turn",
            "bf \"$(printf x)\"",
            "bf nonsense",
            "cat <<EOF\n$(bf human-turn)\nEOF",
            "sh <<'EOF'\nbf human-turn\nEOF",
        ] {
            assert!(!bash(cmd, "/p"), "should be refused: {cmd}");
        }
        // Plain calls still pass, and a worker keeps its short list.
        for cmd in [
            "bf status",
            "cd /p && bf status --json",
            "X=1 bf status",
            "sudo bf status",
        ] {
            assert!(bash(cmd, "/p"), "{cmd}");
        }
        assert!(bash("bf report U-1 --result built", "/p/.bf/work/U-1"));
        assert!(!bash("bf gate open idea", "/p/.bf/work/U-1"));
        assert!(!bash("X=1 bf next", "/p/.bf/work/U-1"));
        assert!(!bash("sudo bf next", "/p/.bf/work/U-1"));
        assert!(!bash("cd x && bf unit land U-1", "/p/.bf/work/U-1"));
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
        assert!(
            !bash("git merge origin/x", "/p/.bf/work/U-1"),
            "a unit's working copy takes in no other branch's commits"
        );
        assert!(bash("git revert HEAD", "/p/.bf/work/U-1"));
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
        assert!(bash(
            "bf record put threat --file f.toml",
            "/p/.bf/work/U-1"
        ));
        assert!(bash("bf record put legal --file f.toml", "/p/.bf/work/U-1"));
        // A checkpoint's copy may put a record too; the engine asks for the role's token.
        assert!(bash(
            "bf record put threat --file f.toml",
            "/p/.bf/work/K-1"
        ));
        assert!(!bash(
            "bf record put requirement --file f.toml",
            "/p/.bf/work/K-1"
        ));
        assert!(
            bash("bf roles token U-1 lawyer", "/p"),
            "the conductor may hand out a role token"
        );
        assert!(!bash("bf roles token U-1 lawyer", "/p/.bf/work/U-1"));
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
        assert!(bash("find . -name '*.toml' -path './factory/*'", "/p"));
        // Code that only reads a protected file by a relative path is a read, from the main checkout
        // as from a working copy; code that writes one is judged by `written_literals`.
        assert!(bash(
            "python3 -c \"import tomllib; print(tomllib.load(open('factory/records/R-1.toml','rb')))\"",
            "/p"
        ));
        assert!(!bash(
            "python3 -c \"open('factory/records/R-1.toml','w').write('x')\"",
            "/p"
        ));
        assert!(bash(
            "python3 -c \"import tomllib; print(tomllib.load(open('factory/records/R-1.toml','rb')))\"",
            "/p/.bf/work/U-1"
        ));
        assert!(bash("git add factory/ && git status", "/p"));
        assert!(bash("cp a.txt /p/.bf/work/U-1/a.txt", "/p"));
        assert!(
            bash("echo x > docs/factory/mod.rs", "/p/.bf/work/U-1"),
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
