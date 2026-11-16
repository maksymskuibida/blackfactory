//! `bf scan`: the secret scan and the dependency licence check. This file belongs to this command
//! group alone; the command line calls its entry point, so a later unit changes only this file.
//!
//! Two commands: `bf scan secrets` and `bf scan licences`. Each starts an outside tool found on the
//! policy's check PATH (never the caller's), under the sandbox with no network where there is one.
//! A missing tool is a refusal that names the tool to install, never a pass. The receipt records where
//! the tool was found and a hash of it. Secret values are removed from everything the runner stores or
//! prints: only the rule, the file, the line and a short commit are kept.
//!
//! The licence rule is an assumption of this draft (the human can change it at the gate): permissive
//! licences pass, weak copyleft passes with a note, strong copyleft and unknown licences become legal
//! findings. A project states its own rule in `factory/licences.toml`: `allow = [...]` adds licences to
//! the allowed ones and `refuse = [...]` names licences that are always a finding (refuse wins).

use crate::refusal::{refuse, refuse_owned, R};
use crate::store::{s, Project};
use crate::util;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The licences that pass by default.
pub const PERMISSIVE: &[&str] = &[
    "MIT",
    "MIT-0",
    "APACHE-2.0",
    "BSD",
    "BSD-2-CLAUSE",
    "BSD-3-CLAUSE",
    "ISC",
    "0BSD",
    "UNLICENSE",
    "CC0-1.0",
    "ZLIB",
    "BSL-1.0",
    "PYTHON-2.0",
    "UNICODE-3.0",
    "UNICODE-DFS-2016",
    "WTFPL",
    "CC-BY-4.0",
    "BLUEOAK-1.0.0",
];
/// Weak copyleft: passes, and is named in the receipt.
pub const WEAK: &[&str] = &[
    "MPL-2.0", "LGPL-2.1", "LGPL-3.0", "LGPL-2.0", "EPL-1.0", "EPL-2.0", "CDDL-1.0", "CDDL-1.1",
];
/// Strong copyleft: a legal finding by default.
pub const STRONG: &[&str] = &[
    "GPL-1.0", "GPL-2.0", "GPL-3.0", "AGPL-1.0", "AGPL-3.0", "SSPL-1.0", "EUPL-1.1", "EUPL-1.2",
    "OSL-3.0", "CPAL-1.0", "RPL-1.5",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fmt {
    Json,
    Csv,
}

/// One row of the table that maps a kind of project to its tool.
pub struct Eco {
    pub name: &'static str,
    /// Lockfiles that mark the kind of project.
    pub markers: &'static [&'static str],
    pub tool: &'static str,
    pub args: &'static [&'static str],
    pub install: &'static str,
    fmt: Fmt,
    /// A file, relative to the project folder, where the tool writes its report instead of printing it.
    /// The runner removes it before the run and reads it after.
    pub report: Option<&'static str>,
    /// Whether the arguments were checked against the real tool. When not, a run that gives no report
    /// is refused with the tool's name and that doubt stated plainly.
    pub verified: bool,
}

/// The table: a kind of project, its lockfiles, its tool.
pub const ECOSYSTEMS: &[Eco] = &[
    Eco {
        name: "rust",
        markers: &["Cargo.lock"],
        tool: "cargo-license",
        args: &["--json"],
        install: "cargo install cargo-license",
        fmt: Fmt::Json,
        report: None,
        verified: true,
    },
    Eco {
        name: "javascript-typescript",
        markers: &[
            "package-lock.json",
            "npm-shrinkwrap.json",
            "yarn.lock",
            "pnpm-lock.yaml",
        ],
        tool: "license-checker",
        args: &["--json", "--production"],
        install: "npm install --global license-checker",
        fmt: Fmt::Json,
        report: None,
        verified: true,
    },
    Eco {
        name: "python",
        markers: &["poetry.lock", "uv.lock", "Pipfile.lock", "requirements.txt"],
        tool: "pip-licenses",
        args: &["--format=json"],
        install: "pipx install pip-licenses",
        fmt: Fmt::Json,
        report: None,
        verified: true,
    },
    Eco {
        name: "go",
        markers: &["go.sum"],
        tool: "go-licenses",
        args: &["csv", "./..."],
        install: "go install github.com/google/go-licenses@latest",
        fmt: Fmt::Csv,
        report: None,
        verified: true,
    },
    Eco {
        name: "swift-cocoapods",
        markers: &["Package.resolved", "Podfile.lock"],
        tool: "license-plist",
        args: &["--output-path", "build/bf-licences", "--format", "json"],
        install: "brew install licenseplist",
        fmt: Fmt::Json,
        report: Some("build/bf-licences/license_plist.json"),
        verified: false,
    },
    Eco {
        name: "gradle",
        markers: &["gradle.lockfile"],
        tool: "gradle",
        args: &["--offline", "--quiet", "generateLicenseReport"],
        install: "install Gradle with the license report plugin",
        fmt: Fmt::Json,
        report: Some("build/reports/dependency-license/index.json"),
        verified: false,
    },
];

const SECRET_TOOL: &str = "gitleaks";
const SECRET_INSTALL: &str = "brew install gitleaks";
/// The history: every commit.
const SECRET_ARGS_HISTORY: &[&str] = &[
    "detect",
    "--source",
    ".",
    "--report-format",
    "json",
    "--report-path",
    "-",
    "--no-banner",
    "--redact",
];
/// The files as they are now, committed or not (`--no-git` reads the folder, not the history).
const SECRET_ARGS_FILES: &[&str] = &[
    "detect",
    "--no-git",
    "--source",
    ".",
    "--report-format",
    "json",
    "--report-path",
    "-",
    "--no-banner",
    "--redact",
];
const MARK: &str = "[redacted]";

// ---- redaction ---------------------------------------------------------------------------------

/// Remove every secret value from a text.
fn scrub(text: &str, secrets: &BTreeSet<String>) -> String {
    let mut t = text.to_string();
    // Longest first, so a secret that holds a shorter one goes whole.
    let mut v: Vec<&String> = secrets.iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.len()));
    for sec in v {
        t = t.replace(sec.as_str(), MARK);
    }
    t
}

fn scrub_value(v: &mut Value, secrets: &BTreeSet<String>) {
    match v {
        Value::String(x) => *x = scrub(x, secrets),
        Value::Array(a) => a.iter_mut().for_each(|y| scrub_value(y, secrets)),
        Value::Object(o) => o.iter_mut().for_each(|(_, y)| scrub_value(y, secrets)),
        _ => {}
    }
}

// ---- running a tool ----------------------------------------------------------------------------

struct ToolRun {
    exit: i64,
    stdout: String,
    stderr: String,
    sandboxed: bool,
    found: Value,
    note: Option<String>,
}

fn quote(a: &str) -> String {
    format!("'{}'", a.replace('\'', "'\\''"))
}

fn missing(tool: &str, install: &str, what: &str) -> crate::refusal::Fail {
    refuse_owned(
        format!("run the {what}"),
        format!("the tool `{tool}` is not on the check PATH, so nothing was scanned; a missing tool is never a pass"),
        vec![
            format!("Install `{tool}` ({install})."),
            "Make sure its folder is on `exec.path` in factory/policy.toml (the human edits that file).".to_string(),
            "Run the `bf scan` command again.".to_string(),
        ],
        format!("The command finds `{tool}` and prints a result."),
    )
}

fn run_tool(
    p: &Project,
    check_key: &str,
    dir: &Path,
    tool: &str,
    args: &[&str],
    what: &str,
    install: &str,
) -> R<ToolRun> {
    let Some(found) = crate::programs::locate(p, tool) else {
        return Err(missing(tool, install, what));
    };
    let note = crate::programs::verify(p, check_key, tool)?;
    let path = if p.policy.exec.path.is_empty() {
        "/usr/bin:/bin:/usr/sbin:/sbin".to_string()
    } else {
        p.policy.exec.path.clone()
    };
    // The file that was found is the one that runs.
    let program = s(&found, "path").to_string();
    let line = std::iter::once(program.as_str())
        .chain(args.iter().copied())
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ");
    let sandboxed = p.policy.exec.sandbox && crate::sandbox::available();
    let mut cmd = if sandboxed {
        let repo_paths = p
            .repos
            .iter()
            .map(|r| r.path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let profile = crate::sandbox::check_profile(
            &p.state_dir_resolved(),
            &p.root.to_string_lossy(),
            &dir.to_string_lossy(),
            false,
            &p.transcript_root(),
            &repo_paths,
        );
        crate::sandbox::command(&profile, crate::evidence::CHECK_SHELL)
    } else {
        Command::new(crate::evidence::CHECK_SHELL)
    };
    cmd.arg("-c")
        .arg(&line)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", &path)
        .env("BF_SCAN", "1")
        .env("CARGO_NET_OFFLINE", "true")
        .env("npm_config_offline", "true")
        .env("PIP_NO_INDEX", "1")
        .env("GOPROXY", "off")
        .env("GOFLAGS", "-mod=mod")
        .env("GRADLE_OPTS", "-Dorg.gradle.offline=true");
    for k in crate::evidence::CHECK_ENV
        .iter()
        .copied()
        .chain(p.policy.exec.env.iter().map(|x| x.as_str()))
    {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    let o = cmd.output().map_err(|e| {
        refuse(
            format!("run `{tool}`"),
            format!("the tool could not be started: {e}"),
            &["Check that the tool is installed and runs."],
            "The command starts and prints a result.",
        )
    })?;
    Ok(ToolRun {
        exit: o.status.code().map(|c| c as i64).unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        sandboxed,
        found,
        note,
    })
}

// ---- common: receipt and flags -----------------------------------------------------------------

fn project_here() -> R<Project> {
    let raw: Vec<String> = std::env::args().collect();
    let dir = match raw.iter().position(|a| a == "-C") {
        Some(i) if i + 1 < raw.len() => PathBuf::from(&raw[i + 1]),
        _ => std::env::current_dir()?,
    };
    Project::discover(&dir)
}

fn flag(name: &str) -> Option<String> {
    let raw: Vec<String> = std::env::args().collect();
    let long = format!("--{name}");
    let i = raw.iter().position(|a| *a == long)?;
    raw.get(i + 1).cloned()
}

/// The check record the scan is the proof of: the command check whose command is `bf scan <sub>`,
/// or the one named with `--check`. Falls back to the named check's name.
fn check_for(p: &Project, sub: &str, fallback: &str) -> R<String> {
    if let Some(c) = flag("check") {
        return Ok(c);
    }
    for c in p.list_records(crate::schema::kind_by_name("check"))? {
        if s(&c, "how") != "command" {
            continue;
        }
        let w = crate::evidence::command_words(&c);
        if w.len() >= 3 && w[1] == "scan" && w[2] == sub {
            return Ok(s(&c, "id").to_string());
        }
    }
    Err(refuse_owned(
        format!("write the receipt of `bf scan {sub}`"),
        format!("no command check record has the command `bf scan {sub}`, so the receipt has no check to belong to ({fallback})"),
        vec![
            format!("Record a check with how = \"command\" and the command `bf scan {sub}`, or name one with `--check <ID>`."),
            "Run the `bf scan` command again.".to_string(),
        ],
        "The command prints a result and a receipt ID.",
    ))
}

#[allow(clippy::too_many_arguments)]
fn write_scan_receipt(
    p: &Project,
    check: &str,
    command: &str,
    exit: i64,
    tool: &ToolRun,
    summary: &str,
    secrets: &BTreeSet<String>,
    extra: Value,
) -> R<Value> {
    let top = p.top.clone();
    let content = p.content_hash(&top)?;
    let mut v = json!({
        "type": "run",
        "check": check,
        "unit": flag("unit"),
        "command": command,
        "program": s(&tool.found, "path"),
        "sandbox": if tool.sandboxed { "sandbox-exec" } else { "none" },
        "program_note": tool.note.clone().unwrap_or_default(),
        "tool_path": s(&tool.found, "path"),
        "tool_hash": s(&tool.found, "hash"),
        "exit": exit,
        "content_hash": content,
        "output_hash": util::sha256_hex(summary.as_bytes()),
        "bytes": summary.len(),
        "output_tail": summary,
        "at": util::now(),
        "working_copy": top.strip_prefix(&p.root).map(|r| r.to_string_lossy().to_string()).unwrap_or_default(),
        "scan": extra,
    });
    scrub_value(&mut v, secrets);
    let v = crate::evidence::write_receipt(p, v)?;
    crate::events::append(
        p,
        "scan.run",
        json!({"receipt": v["id"], "check": check, "exit": exit}),
    )?;
    Ok(v)
}

// ---- secrets -----------------------------------------------------------------------------------

fn text_of(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(x) = v.get(*k) {
            match x {
                Value::String(t) if !t.is_empty() => return t.clone(),
                Value::Number(n) => return n.to_string(),
                _ => {}
            }
        }
    }
    String::new()
}

pub fn scan_secrets(p: &Project) -> R<Value> {
    let mut secrets: BTreeSet<String> = BTreeSet::new();
    let mut leaks: Vec<Value> = Vec::new();
    let mut runs: Vec<ToolRun> = Vec::new();
    // Two runs: the files as they are now, and the commit history.
    for args in [SECRET_ARGS_FILES, SECRET_ARGS_HISTORY] {
        let r = run_tool(
            p,
            "scan:secrets",
            &p.top.clone(),
            SECRET_TOOL,
            args,
            "secret scan",
            SECRET_INSTALL,
        )?;
        let parsed: Option<Vec<Value>> = serde_json::from_str::<Value>(r.stdout.trim())
            .ok()
            .and_then(|v| match v {
                Value::Array(a) => Some(a),
                Value::Null => Some(vec![]),
                _ => None,
            });
        let Some(a) = parsed else {
            // Not a report: the scan did not happen, whatever the exit said. Nothing the tool printed
            // is quoted, because there is no report to tell which part of it is a secret.
            return Err(refuse_owned(
                "accept the secret scan",
                format!(
                    "`{SECRET_TOOL}` exited {} and printed no report the engine can read, so it is not known that the files and history are clean (its own output is not shown, in case it holds a secret)",
                    r.exit
                ),
                vec![
                    format!("Run `{SECRET_TOOL}` by hand in the project and fix what it reports."),
                    "Run `bf scan secrets` again.".to_string(),
                ],
                "The tool prints a JSON report and the command writes a receipt.",
            ));
        };
        // The values in the tool's own report are the ones to remove, wherever they would show.
        for l in &a {
            for k in ["Secret", "Match", "secret", "match"] {
                if let Some(t) = l.get(k).and_then(|x| x.as_str()) {
                    if t.len() >= 3 {
                        secrets.insert(t.to_string());
                    }
                }
            }
        }
        if r.exit != 0 && r.exit != 1 {
            return Err(refuse(
                "accept the secret scan",
                format!("`{SECRET_TOOL}` failed with exit {}", r.exit),
                &["Run the tool by hand to see why, then run `bf scan secrets` again."],
                "The tool exits 0 or 1 and the command writes a receipt.",
            ));
        }
        leaks.extend(a);
        runs.push(r);
    }
    let all_zero = runs.iter().all(|x| x.exit == 0);
    let mut r = runs.remove(0);
    if r.note.is_none() {
        r.note = runs.iter().find_map(|x| x.note.clone());
    }
    let mut findings: Vec<Value> = Vec::new();
    for l in &leaks {
        let commit: String = text_of(l, &["Commit", "commit"]).chars().take(12).collect();
        findings.push(json!({
            "rule": text_of(l, &["RuleID", "rule", "Rule"]),
            "file": text_of(l, &["File", "file"]),
            "line": text_of(l, &["StartLine", "line", "Line"]),
            "commit": commit,
        }));
    }
    // An exit of 1 with an empty report is still a failure: the tool said it found something.
    let pass = findings.is_empty() && all_zero;
    let summary = if pass {
        "secret scan: no secret found in the files and the history".to_string()
    } else {
        let mut t = format!(
            "secret scan: {} finding(s) (values removed)\n",
            findings.len()
        );
        for f in &findings {
            t.push_str(&format!(
                "- rule {} at {}:{} commit {}\n",
                s(f, "rule"),
                s(f, "file"),
                s(f, "line"),
                s(f, "commit")
            ));
        }
        t
    };
    let summary = scrub(&summary, &secrets);
    let check = check_for(p, "secrets", "secret-scan")?;
    let exit = if pass { 0 } else { 1 };
    let rc = write_scan_receipt(
        p,
        &check,
        "bf scan secrets",
        exit,
        &r,
        &summary,
        &secrets,
        json!({"findings": findings, "count": findings.len()}),
    )?;
    let mut out = json!({
        "ok": pass,
        "check": check,
        "receipt": rc["id"],
        "tool": SECRET_TOOL,
        "tool_path": rc["tool_path"],
        "tool_hash": rc["tool_hash"],
        "tool_note": r.note.clone().unwrap_or_default(),
        "count": findings.len(),
        "findings": findings,
    });
    scrub_value(&mut out, &secrets);
    Ok(out)
}

// ---- licences ----------------------------------------------------------------------------------

#[derive(Default)]
struct Rule {
    allow: BTreeSet<String>,
    refuse: BTreeSet<String>,
}

fn norm(id: &str) -> String {
    let mut t = id.trim().to_ascii_uppercase();
    for suf in ["-ONLY", "-OR-LATER", "+"] {
        if let Some(x) = t.strip_suffix(suf) {
            t = x.to_string();
        }
    }
    t
}

fn load_rule(p: &Project) -> R<Rule> {
    let path = p.factory().join("licences.toml");
    let mut r = Rule::default();
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(r);
    };
    let v: toml::Value = toml::from_str(&text).map_err(|e| {
        refuse(
            "read factory/licences.toml",
            format!("it is not valid: {}", e.to_string().replace('\n', " ")),
            &["Fix the file: `allow = [\"MIT\"]` and `refuse = [\"GPL-3.0\"]` are lists of licence names."],
            "`bf scan licences` reads the file.",
        )
    })?;
    for (k, set) in [("allow", &mut r.allow), ("refuse", &mut r.refuse)] {
        if let Some(a) = v.get(k).and_then(|x| x.as_array()) {
            for x in a.iter().filter_map(|x| x.as_str()) {
                set.insert(norm(x));
            }
        }
    }
    Ok(r)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Class {
    Ok,
    Weak,
    Unknown,
    Refused,
}

fn class_of(id: &str, rule: &Rule) -> Class {
    let n = norm(id);
    if rule.refuse.contains(&n) {
        return Class::Refused;
    }
    if rule.allow.contains(&n) || PERMISSIVE.contains(&n.as_str()) {
        return Class::Ok;
    }
    if WEAK.contains(&n.as_str()) {
        return Class::Weak;
    }
    if STRONG.contains(&n.as_str()) || n.starts_with("GPL-") || n.starts_with("AGPL-") {
        return Class::Refused;
    }
    Class::Unknown
}

fn tokens(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in expr.chars() {
        match c {
            '(' | ')' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(c.to_string());
            }
            c if c.is_whitespace() || c == '/' || c == ',' || c == ';' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Evaluate an SPDX-like expression: OR takes the better side, AND the worse.
fn eval(toks: &[String], i: &mut usize, rule: &Rule) -> Class {
    // or-expr := and-expr (OR and-expr)*
    let mut best = eval_and(toks, i, rule);
    while *i < toks.len() && toks[*i].eq_ignore_ascii_case("OR") {
        *i += 1;
        let r = eval_and(toks, i, rule);
        best = best.min(r);
    }
    best
}

fn eval_and(toks: &[String], i: &mut usize, rule: &Rule) -> Class {
    let mut worst = eval_atom(toks, i, rule);
    while *i < toks.len() && toks[*i].eq_ignore_ascii_case("AND") {
        *i += 1;
        let r = eval_atom(toks, i, rule);
        worst = worst.max(r);
    }
    worst
}

fn eval_atom(toks: &[String], i: &mut usize, rule: &Rule) -> Class {
    if *i >= toks.len() {
        return Class::Unknown;
    }
    if toks[*i] == "(" {
        *i += 1;
        let c = eval(toks, i, rule);
        if *i < toks.len() && toks[*i] == ")" {
            *i += 1;
        }
        return c;
    }
    let mut name = toks[*i].clone();
    *i += 1;
    // "WITH exception" keeps the base licence.
    while *i + 1 < toks.len() && toks[*i].eq_ignore_ascii_case("WITH") {
        *i += 2;
    }
    if name.eq_ignore_ascii_case("UNKNOWN") || name.eq_ignore_ascii_case("NONE") {
        name = String::new();
    }
    if name.is_empty() {
        Class::Unknown
    } else {
        class_of(&name, rule)
    }
}

fn class_expr(expr: &str, rule: &Rule) -> Class {
    let toks = tokens(expr);
    if toks.is_empty() {
        return Class::Unknown;
    }
    let mut i = 0;
    let c = eval(&toks, &mut i, rule);
    if i < toks.len() {
        // Something the reader did not understand: judge it by the worst of its words.
        return c.max(Class::Unknown);
    }
    c
}

struct Dep {
    name: String,
    version: String,
    licence: String,
}

fn lic_text(v: &Value) -> String {
    match v {
        Value::String(t) => t.trim().to_string(),
        Value::Array(a) => a
            .iter()
            .map(lic_text)
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" OR "),
        Value::Object(o) => o
            .get("name")
            .or_else(|| o.get("moduleLicense"))
            .map(lic_text)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn dep_from_obj(o: &Value, key: Option<&str>) -> Option<Dep> {
    let get = |names: &[&str]| -> Option<&Value> {
        o.as_object()?
            .iter()
            .find(|(k, _)| names.iter().any(|n| k.eq_ignore_ascii_case(n)))
            .map(|(_, v)| v)
    };
    let mut name = get(&["name", "package", "module", "id", "moduleName"])
        .map(lic_text)
        .unwrap_or_default();
    let mut version = get(&["version", "moduleVersion"])
        .map(lic_text)
        .unwrap_or_default();
    if name.is_empty() {
        let k = key?;
        match k.rfind('@').filter(|i| *i > 0) {
            Some(i) => {
                name = k[..i].to_string();
                version = k[i + 1..].to_string();
            }
            None => name = k.to_string(),
        }
    }
    let licence = get(&[
        "license",
        "licence",
        "licenses",
        "licences",
        "moduleLicenses",
    ])
    .map(lic_text)
    .unwrap_or_default();
    Some(Dep {
        name,
        version,
        licence,
    })
}

fn parse_json(text: &str) -> Option<Vec<Dep>> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    match &v {
        Value::Array(a) => a.iter().map(|x| dep_from_obj(x, None)).collect(),
        Value::Object(o) => {
            for k in ["dependencies", "packages", "licenses", "libraries"] {
                if let Some(Value::Array(a)) = o.get(k) {
                    return a.iter().map(|x| dep_from_obj(x, None)).collect();
                }
            }
            o.iter()
                .map(|(k, x)| dep_from_obj(x, Some(k)))
                .collect::<Option<Vec<_>>>()
        }
        _ => None,
    }
}

fn parse_csv(text: &str) -> Option<Vec<Dep>> {
    let mut out = Vec::new();
    for l in text.lines().filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = l.split(',').collect();
        if f.len() < 3 {
            return None;
        }
        out.push(Dep {
            name: f[0].trim().to_string(),
            version: String::new(),
            licence: f[f.len() - 1].trim().to_string(),
        });
    }
    Some(out)
}

fn skip_dir(n: &str) -> bool {
    matches!(
        n,
        ".git"
            | "node_modules"
            | "target"
            | "vendor"
            | ".bf"
            | "factory"
            | "Pods"
            | "build"
            | ".gradle"
    )
}

fn find_markers(dir: &Path, depth: u32, out: &mut Vec<(usize, PathBuf, String)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        if path.is_dir() {
            if depth > 0 && !skip_dir(&name) && !path.is_symlink() {
                find_markers(&path, depth - 1, out);
            }
        } else {
            for (i, eco) in ECOSYSTEMS.iter().enumerate() {
                if eco.markers.contains(&name.as_str()) {
                    out.push((i, dir.to_path_buf(), name.clone()));
                }
            }
        }
    }
}

fn existing_finding(p: &Project, title: &str) -> R<Option<String>> {
    for r in p.list_records(crate::schema::kind_by_name("legal"))? {
        if s(&r, "title") == title {
            return Ok(Some(s(&r, "id").to_string()));
        }
    }
    Ok(None)
}

pub fn scan_licences(p: &Project) -> R<Value> {
    let rule = load_rule(p)?;
    let mut marks = Vec::new();
    find_markers(&p.top, 4, &mut marks);
    // One run per kind of project and folder; a folder with several lockfiles of one kind runs once.
    let mut jobs: BTreeMap<(usize, PathBuf), Vec<String>> = BTreeMap::new();
    for (i, d, m) in marks {
        jobs.entry((i, d)).or_default().push(m);
    }
    // Every tool must be there before any runs: a part-scan must not read as a whole one.
    let mut gone: Vec<(&str, &str)> = Vec::new();
    for (i, _) in jobs.keys() {
        let e = &ECOSYSTEMS[*i];
        if crate::programs::locate(p, e.tool).is_none() && !gone.iter().any(|g| g.0 == e.tool) {
            gone.push((e.tool, e.install));
        }
    }
    if !gone.is_empty() {
        let names: Vec<String> = gone.iter().map(|g| format!("`{}`", g.0)).collect();
        let mut steps: Vec<String> = gone
            .iter()
            .map(|g| format!("Install `{}` ({}).", g.0, g.1))
            .collect();
        steps.push(
            "Make sure its folder is on `exec.path` in factory/policy.toml (the human edits that file).".into(),
        );
        steps.push("Run `bf scan licences` again.".into());
        return Err(refuse_owned(
            "run the dependency licence check",
            format!(
                "the tool {} is not on the check PATH, so the licences were not read; a missing tool is never a pass",
                names.join(", ")
            ),
            steps,
            "The command finds every tool and prints a result.",
        ));
    }
    let mut last: Option<ToolRun> = None;
    let mut deps: Vec<(String, Dep)> = Vec::new();
    let mut ran: Vec<Value> = Vec::new();
    for ((i, dir), files) in &jobs {
        let e = &ECOSYSTEMS[*i];
        // A tool that writes its report to a file: remove an old one first, so it is never read as new.
        let report_file = e.report.map(|f| dir.join(f));
        if let Some(f) = &report_file {
            let _ = fs::remove_file(f);
            if let Some(par) = f.parent() {
                let _ = fs::create_dir_all(par);
            }
        }
        let r = run_tool(
            p,
            &format!("scan:licences:{}", e.name),
            dir,
            e.tool,
            e.args,
            "dependency licence check",
            e.install,
        )?;
        let text = match &report_file {
            Some(f) => fs::read_to_string(f).unwrap_or_default(),
            None => r.stdout.clone(),
        };
        let parsed = match e.fmt {
            Fmt::Json => parse_json(&text),
            Fmt::Csv => parse_csv(&text),
        };
        let Some(list) = parsed.filter(|_| r.exit == 0) else {
            let said: String = r.stderr.trim().chars().take(300).collect();
            let where_report = match e.report {
                Some(f) => format!(" in the file {f}"),
                None => String::new(),
            };
            let doubt = if e.verified {
                String::new()
            } else {
                format!(
                    "; the arguments of `{}` in this version's table are not checked against the real tool",
                    e.tool
                )
            };
            return Err(refuse_owned(
                "accept the dependency licence check",
                format!(
                    "`{}` exited {} and gave no list of licences{} the engine can read, so the licences of the {} project are not known{}{}",
                    e.tool,
                    r.exit,
                    where_report,
                    e.name,
                    doubt,
                    if said.is_empty() { String::new() } else { format!(" (it said: {said})") }
                ),
                vec![
                    format!("Run `{}` by hand in {} and fix what it reports (it must work without the network).", e.tool, dir.display()),
                    "Run `bf scan licences` again.".to_string(),
                ],
                "The tool prints its licences and the command writes a receipt.",
            ));
        };
        let rel = dir
            .strip_prefix(&p.top)
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        let where_ = if rel.is_empty() {
            files.join(", ")
        } else {
            files
                .iter()
                .map(|f| format!("{rel}/{f}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        ran.push(json!({"ecosystem": e.name, "tool": e.tool, "lockfiles": where_,
            "tool_path": s(&r.found, "path"), "tool_hash": s(&r.found, "hash"), "tool_note": r.note.clone().unwrap_or_default(), "packages": list.len()}));
        for d in list {
            deps.push((where_.clone(), d));
        }
        last = Some(r);
    }
    // Group by licence text and class.
    let mut weak: BTreeSet<String> = BTreeSet::new();
    let mut bad: BTreeMap<(String, Class), (String, Vec<String>)> = BTreeMap::new();
    for (where_, d) in &deps {
        let lic = if d.licence.is_empty() {
            "UNKNOWN".to_string()
        } else {
            d.licence.clone()
        };
        match class_expr(&lic, &rule) {
            Class::Ok => {}
            Class::Weak => {
                weak.insert(lic);
            }
            c => {
                let e = bad.entry((lic, c)).or_insert((where_.clone(), vec![]));
                let n = if d.version.is_empty() {
                    d.name.clone()
                } else {
                    format!("{} {}", d.name, d.version)
                };
                e.1.push(n);
            }
        }
    }
    let mut findings: Vec<Value> = Vec::new();
    for ((lic, c), (where_, pk)) in &bad {
        let title = format!("Dependency licence: {lic}");
        let refused = *c == Class::Refused;
        let id = match existing_finding(p, &title)? {
            Some(id) => id,
            None => {
                let shown: Vec<&String> = pk.iter().take(20).collect();
                let more = if pk.len() > 20 {
                    format!(" and {} more", pk.len() - 20)
                } else {
                    String::new()
                };
                let mut f = json!({
                    "title": title,
                    "markets": ["every market the project ships to"],
                    "risk": if refused { "strong copyleft or refused dependency licence" } else { "unknown dependency licence" },
                    "researched": false,
                    "lawyer_check": true,
                    "severity": if refused { "high" } else { "medium" },
                    "where": where_,
                    "text": format!("Packages under {lic}: {}{more}.", shown.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ")),
                });
                f["action"] = Value::String(
                    if refused {
                        "Replace the dependency, or ask the human to accept the licence in writing."
                    } else {
                        "Find out the licence of these packages and state it in factory/licences.toml."
                    }
                    .to_string(),
                );
                let r = crate::legal::file_finding(p, &f)?;
                s(&r, "id").to_string()
            }
        };
        findings.push(
            json!({"id": id, "licence": lic, "class": if refused {"refused"} else {"unknown"},
            "packages": pk.len(), "lockfiles": where_}),
        );
    }
    let pass = findings.is_empty();
    let mut summary = format!(
        "licence check: {} package(s) in {} project(s), {} licence finding(s)",
        deps.len(),
        ran.len(),
        findings.len()
    );
    if ran.is_empty() {
        summary.push_str("; no lockfile was found, so there was nothing to read");
    }
    if !weak.is_empty() {
        summary.push_str(&format!(
            "\nweak copyleft, allowed: {}",
            weak.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    for f in &findings {
        summary.push_str(&format!(
            "\n- {} ({}) in {}: {} package(s), {}",
            s(f, "licence"),
            s(f, "class"),
            s(f, "lockfiles"),
            f["packages"],
            s(f, "id")
        ));
    }
    let check = check_for(p, "licences", "dependency-licence-check")?;
    let tool = match last {
        Some(t) => t,
        // Nothing ran: no tool was needed, and no tool is claimed.
        None => ToolRun {
            exit: 0,
            stdout: String::new(),
            stderr: String::new(),
            sandboxed: false,
            found: json!({}),
            note: None,
        },
    };
    let rc = write_scan_receipt(
        p,
        &check,
        "bf scan licences",
        if pass { 0 } else { 1 },
        &tool,
        &summary,
        &BTreeSet::new(),
        json!({"projects": ran, "findings": findings,
               "weak_copyleft": weak.iter().collect::<Vec<_>>(),
               "rule": "permissive passes; weak copyleft passes with a note; strong copyleft and unknown become legal findings; factory/licences.toml adds `allow` and `refuse`"}),
    )?;
    Ok(json!({
        "ok": pass,
        "check": check,
        "receipt": rc["id"],
        "projects": ran,
        "tool_note": tool.note.clone().unwrap_or_default(),
        "packages": deps.len(),
        "findings": findings,
        "weak_copyleft": weak.iter().collect::<Vec<_>>(),
    }))
}

/// Entry point of `bf scan <sub> ...`: `secrets` and `licences`.
pub fn run(sub: &str, _rest: &[String]) -> R<Value> {
    match sub {
        "secrets" => scan_secrets(&project_here()?),
        "licences" | "licenses" => scan_licences(&project_here()?),
        _ => Err(refuse(
            format!("run `bf scan {sub}`"),
            "that is not a scan command",
            &["Use `bf scan secrets` or `bf scan licences`."],
            "The command prints its result.",
        )),
    }
}
