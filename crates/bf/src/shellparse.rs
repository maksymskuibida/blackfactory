//! A small reader of shell command lines, for the guard: where would a shell really run `bf`?
//!
//! A word counts as a call of the engine only in command position: the first word of a simple command
//! (after `&&`, `||`, `;`, `|`, `&`, a newline, `(`, inside `$( )` or backticks, after leading
//! `VAR=value` assignments and the wrappers `env`, `command`, `exec`, `time`, `nohup`, `xargs`, `sudo`,
//! ...), and in the string argument of `sh -c`, `bash -c`, `zsh -c` and `eval`, read the same way. An
//! argument of another program (`-p bf`, `crates/bf`), text in quotes that is data for another program,
//! and the body of a heredoc that feeds a program that is not a shell are never a call. Where the reading
//! is unsure about a word that is not in command position, the word passes.
#![allow(unused_assignments)]

use std::collections::HashMap;

/// A call of `bf` found in a command line.
#[derive(Debug, Clone)]
pub struct Call {
    /// The words of the call, `bf` first, quotes removed.
    pub words: Vec<String>,
    /// How it is reached when not directly: `eval`, `env`, `sh -c`, `$(`, a backtick ...
    pub via: Option<String>,
    /// The call holds an unquoted `(` after its first word (`x(N)`, `(check|zz)`) or a zsh number range
    /// (`x<->`): a pattern the shell may expand, drop or join, so its words cannot be read here.
    pub group: bool,
    /// The engine reached as `cargo run [...] -- <words>`: `words` are `bf` and what cargo hands on.
    pub cargo: bool,
}

struct Cmd {
    id: usize,
    words: Vec<String>,
    via: Option<String>,
    /// The body of a heredoc fed to this command, and whether the shell expands it.
    heredoc: Option<(String, bool)>,
    /// The targets of the output redirects (`>`, `>>`, `>|`, `&>`) of this command.
    redirs: Vec<String>,
    /// The command whose output is piped into this one (`a | this`).
    pipe_from: Option<usize>,
    /// The text each process substitution (`<( )`) of this command writes, when it is text.
    procs: Vec<String>,
    /// See `Call::group`.
    group: bool,
    /// The index of a word that is itself a pattern because a group touches it (`b(f)`, `x(N)`).
    group_word: Option<usize>,
    /// The command line evaluates text as code at run time here: arithmetic (`$(( ))`, `(( ))`,
    /// `$[ ]`), a zsh parameter flag (`${(e)V}`) or a bash parameter operator (`${V@P}`).
    evaluates: bool,
    /// The functions this command defines: an unquoted `name()` in command position, or the names
    /// after an unquoted keyword `function`.
    defines: Vec<String>,
    /// The command has an input redirect, a heredoc or a here string.
    reads_redirect: bool,
    /// A command of no words right after a `)` or `))`: its redirects belong to that subshell or
    /// arithmetic (`(cmd) 2>/dev/null`), not to a command of their own.
    attached: bool,
}

impl Cmd {
    /// A command made of `words` alone.
    fn of(words: Vec<String>) -> Cmd {
        Cmd {
            id: 0,
            words,
            via: None,
            heredoc: None,
            redirs: vec![],
            pipe_from: None,
            procs: vec![],
            group: false,
            group_word: None,
            evaluates: false,
            defines: vec![],
            reads_redirect: false,
            attached: false,
        }
    }
}

struct Pending {
    cmd: usize,
    delim: String,
    strip_tabs: bool,
    expands: bool,
}

pub fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

/// Does the program word name the engine? Case is ignored, since the default macOS volume opens
/// `/usr/local/bin/BF` as `bf`, and zsh's `=bf` (with `equals` set) is its path.
pub fn is_engine(w: &str) -> bool {
    basename(w.strip_prefix('=').unwrap_or(w)).eq_ignore_ascii_case("bf")
}

/// Text with each `$NAME`, `${NAME}` and zsh `$=NAME`, `$~NAME`, `${=NAME}` replaced by `value(NAME)`
/// where it gives one. `None` when nothing was replaced.
pub fn substitute(w: &str, value: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let b: Vec<char> = w.chars().collect();
    let mut out = String::new();
    let mut hit = false;
    let mut i = 0;
    while i < b.len() {
        if b[i] == '$' {
            let mut j = i + 1;
            let brace = b.get(j) == Some(&'{');
            if brace {
                j += 1;
            }
            while matches!(b.get(j), Some('=' | '~' | '^')) {
                j += 1;
            }
            let s = j;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == '_') {
                j += 1;
            }
            let name: String = b[s..j].iter().collect();
            let closed = !brace || b.get(j) == Some(&'}');
            if !name.is_empty() && closed {
                if let Some(v) = value(&name) {
                    out.push_str(&v);
                    hit = true;
                    i = if brace { j + 1 } else { j };
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    hit.then_some(out)
}

/// What `printf FORMAT ARGS...` writes, for the conversions `%s`, `%b`, `%q`, `%c`, `%d`, `%i`, `%u`
/// and `%%` and the escapes `\n`, `\t`, `\\`; the format is used again while arguments are left.
/// `None` for anything else.
fn printf_text(fmt: &str, args: &[String]) -> Option<String> {
    let f: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut next = 0usize;
    loop {
        let mut used = false;
        let mut k = 0;
        while k < f.len() {
            match f[k] {
                '\\' if k + 1 < f.len() => {
                    out.push(match f[k + 1] {
                        'n' => '\n',
                        't' => '\t',
                        '\\' => '\\',
                        _ => return None,
                    });
                    k += 2;
                }
                '%' => {
                    let mut j = k + 1;
                    while j < f.len() && "-+ #0123456789.".contains(f[j]) {
                        j += 1;
                    }
                    match f.get(j) {
                        Some('%') => out.push('%'),
                        Some('s' | 'b' | 'q' | 'c' | 'd' | 'i' | 'u') => {
                            let a = args.get(next).map(|s| s.as_str()).unwrap_or("");
                            next += 1;
                            used = true;
                            if f[j] == 'c' {
                                out.extend(a.chars().take(1));
                            } else {
                                out.push_str(a);
                            }
                        }
                        _ => return None,
                    }
                    k = j + 1;
                }
                c => {
                    out.push(c);
                    k += 1;
                }
            }
        }
        if !used || next >= args.len() {
            return Some(out);
        }
    }
}

/// The text of a `$'...'` quote that starts at `b[i]` (after the opening quote), whether it holds an
/// escape that is not decoded here, and the index of the closing quote (or the end).
fn ansi_c(b: &[char], mut i: usize) -> (String, bool, usize) {
    let mut text = String::new();
    let mut unknown = false;
    let n = b.len();
    let digits = |i: usize, max: usize, radix: u32| -> (u32, usize) {
        let mut v = 0u32;
        let mut k = 0;
        while k < max && i + k < n {
            match b[i + k].to_digit(radix) {
                Some(d) => v = v * radix + d,
                None => break,
            }
            k += 1;
        }
        (v, k)
    };
    while i < n && b[i] != '\'' {
        if b[i] == '\\' && i + 1 < n {
            let e = b[i + 1];
            i += 2;
            let simple = match e {
                'a' => Some('\u{7}'),
                'b' => Some('\u{8}'),
                'e' | 'E' => Some('\u{1b}'),
                'f' => Some('\u{c}'),
                'n' => Some('\n'),
                'r' => Some('\r'),
                't' => Some('\t'),
                'v' => Some('\u{b}'),
                '\\' | '\'' | '"' | '?' => Some(e),
                _ => None,
            };
            if let Some(c) = simple {
                text.push(c);
                continue;
            }
            let (v, k) = match e {
                'x' => digits(i, 2, 16),
                'u' => digits(i, 4, 16),
                'U' => digits(i, 8, 16),
                '0'..='7' => {
                    i -= 1;
                    digits(i, 3, 8)
                }
                _ => (0, 0),
            };
            match char::from_u32(v) {
                Some(c) if k > 0 => {
                    text.push(c);
                    i += k;
                }
                _ => {
                    unknown = true;
                    text.push(e);
                }
            }
            continue;
        }
        text.push(b[i]);
        i += 1;
    }
    (text, unknown, i)
}

/// The index of the `-c` string of the shell at `words[i]`, as `shell_command_string` reads it.
pub fn shell_string(words: &[String], i: usize) -> Option<usize> {
    shell_command_string(words, i)
}

/// Is the program word a shell the parse reads `-c` and scripts for?
pub fn is_shell(w: &str) -> bool {
    SHELLS.contains(&basename(w))
}

/// The text a via phrase stands for, from the strings files.
fn via_text(key: &str) -> String {
    crate::strings::t(&format!("guard.via.{key}"), &[])
}

fn is_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((k, _)) => {
            !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.chars().next().unwrap().is_ascii_digit()
        }
        None => false,
    }
}

const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "csh", "tcsh", "fish", "mksh", "yash", "posh",
];
const KEYWORDS: &[&str] = &[
    "{", "}", "!", "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "case",
    "esac",
];

/// Every call of `bf` a shell would make when it runs `cmd`.
pub fn bf_calls(cmd: &str) -> Vec<Call> {
    engine_calls(cmd).into_iter().filter(|c| !c.cargo).collect()
}

/// Every call of `bf` a shell would make when it runs `cmd`, and every `cargo run` whose words are read
/// as the engine's (`cargo run -- unit unblock U-1` runs the engine built from this project).
pub fn engine_calls(cmd: &str) -> Vec<Call> {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    let mut calls = Vec::new();
    collect(cmds, None, 0, &mut calls);
    // `export CARGO_ALIAS_X='run --'; cargo x ...`: an alias the line sets reaches every cargo call.
    let parsed = commands(cmd);
    let line_alias = parsed.iter().any(|p| {
        p.words
            .iter()
            .any(|w| w.to_ascii_uppercase().starts_with("CARGO_ALIAS_") && w.contains('='))
    });
    if line_alias {
        for p in &parsed {
            if let Some(i) = program_index(&p.words) {
                if basename(&p.words[i]) == "cargo" {
                    let mut words = vec!["bf".to_string()];
                    words.extend(p.words[i + 1..].iter().cloned());
                    calls.push(Call {
                        words,
                        via: None,
                        group: true,
                        cargo: true,
                    });
                }
            }
        }
    }
    calls
}

/// The functions the command line defines in its own shell: an unquoted `name()` in command position
/// or names after an unquoted keyword `function`. A quoted `"function"`, a `name()` in a comment or a
/// string, a body of a heredoc and the inside of a substitution define nothing here.
pub fn defined_functions(cmd: &str) -> Vec<String> {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    cmds.into_iter()
        .filter(|c| c.via.is_none())
        .flat_map(|c| c.defines)
        .collect()
}

/// Is there a command with no program, only assignments, that has a redirect, a heredoc or a here
/// string? zsh runs `$READNULLCMD` or `$NULLCMD` for it (`READNULLCMD=sh; <<< 'cmd'`).
pub fn null_command_with_redirect(cmd: &str) -> bool {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    cmds.iter().any(|c| {
        c.words.iter().all(|w| is_assignment(w))
            && !c.attached
            && (c.reads_redirect || !c.redirs.is_empty() || c.heredoc.is_some())
    })
}

/// Does the command line evaluate text as code at run time (see `Cmd::evaluates`)?
pub fn evaluates_text(cmd: &str) -> bool {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    cmds.iter().any(|c| c.evaluates)
}

/// Command lines a command hands to a shell or splits into a command of its own, read the same way:
/// `env -S '<line>'`, `script|flock|su|runuser|sg ... -c '<line>'`, a quoted program that holds a
/// whole line (`watch 'bf status'`), and the value of `alias name='<line>'`. With how each is reached.
fn handed_lines(words: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let pi = program_index(words);
    for (k, w) in words.iter().enumerate() {
        // Only the program and the wrappers before it.
        if pi.is_some_and(|p| k > p) {
            break;
        }
        let base = basename(w);
        let rest = &words[k + 1..];
        match base {
            "env" => {
                for (j, o) in rest.iter().enumerate() {
                    let tail = rest[j + 1..].join(" ");
                    let line = if o == "-S" || o == "--split-string" {
                        rest.get(j + 1)
                            .map(|v| format!("{v} {}", rest[(j + 2).min(rest.len())..].join(" ")))
                    } else if let Some(v) = o.strip_prefix("--split-string=") {
                        Some(format!("{v} {tail}"))
                    } else if o.starts_with("-S") && o.len() > 2 {
                        Some(format!("{} {tail}", &o[2..]))
                    } else {
                        None
                    };
                    if let Some(l) = line {
                        out.push((l, "env -S".to_string()));
                        break;
                    }
                }
            }
            "script" | "flock" | "su" | "runuser" | "sg" => {
                for (j, o) in rest.iter().enumerate() {
                    let line = if o == "-c" || o == "--command" {
                        rest.get(j + 1).cloned()
                    } else {
                        o.strip_prefix("--command=").map(|v| v.to_string())
                    };
                    if let Some(l) = line {
                        out.push((l, format!("{base} -c")));
                        break;
                    }
                }
            }
            "alias" if pi == Some(k) => {
                for a in rest {
                    if let Some((_, v)) = a.split_once('=') {
                        out.push((v.to_string(), "alias".to_string()));
                    }
                }
            }
            // zsh's `zpty NAME 'cmd ...'` runs its words after the name as a command line; BSD's
            // `apply 'cmd %1' args` runs its first word with the arguments put in.
            "zpty" if pi == Some(k) => {
                let plain: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
                if plain.len() > 1 {
                    let line: Vec<String> = plain[1..].iter().map(|s| s.to_string()).collect();
                    out.push((line.join(" "), "zpty".to_string()));
                }
            }
            "apply" if pi == Some(k) => {
                if let Some(first) = rest.iter().find(|a| !a.starts_with('-')) {
                    let mut line = first.clone();
                    for a in rest.iter().skip_while(|a| *a != first).skip(1) {
                        line.push(' ');
                        line.push_str(a);
                    }
                    out.push((line, "apply".to_string()));
                }
            }
            _ => {}
        }
    }
    // A program word that holds a whole command line is a line some wrapper hands to a shell.
    if let Some(i) = program_index(words) {
        let p = &words[i];
        if !is_engine(p) && p.contains([' ', '\t', '\n', ';', '|', '&']) {
            out.push((words[i..].join(" "), via_text("quoted_command")));
        }
    }
    out
}

/// The command of `bf evidence run <check> ... -- <command>`, when `words` (with `bf` first) is such a
/// call: the words after the first `--`, read as the engine's parser reads the subcommand.
pub fn evidence_command(words: &[String]) -> Option<Vec<String>> {
    let a = crate::cli::Args::parse(words.get(1..).unwrap_or(&[]));
    (a.pos.first().map(|s| s.as_str()) == Some("evidence")
        && a.pos.get(1).map(|s| s.as_str()) == Some("run")
        && !a.rest.is_empty())
    .then_some(a.rest)
}

/// Does a handed line come from a quoted program word (read for calls only, not by the path rules)?
fn is_quoted_command(how: &str) -> bool {
    how == via_text("quoted_command")
}

/// The words after cargo's own options when the line gives cargo an alias it cannot read here: a
/// `CARGO_ALIAS_*` assignment before it, or a `--config` value that names an alias
/// (`--config 'alias.x="run --"'`). Such a call may be `cargo run`.
pub fn cargo_alias_args(words: &[String]) -> Option<Vec<String>> {
    let i = program_index(words)?;
    if basename(&words[i]) != "cargo" {
        return None;
    }
    let assigned = words[..i]
        .iter()
        .any(|w| w.to_ascii_uppercase().starts_with("CARGO_ALIAS_"));
    let mut configured = false;
    let mut k = i + 1;
    while k < words.len() && (words[k].starts_with('-') || words[k].starts_with('+')) {
        let w = words[k].as_str();
        if w == "--config" {
            configured |= words.get(k + 1).is_some_and(|v| v.contains("alias"));
            k += 2;
        } else if let Some(v) = w.strip_prefix("--config=") {
            configured |= v.contains("alias");
            k += 1;
        } else if matches!(w, "--color" | "-Z" | "-C" | "--explain") {
            k += 2;
        } else {
            k += 1;
        }
    }
    (assigned || configured).then(|| words[k.min(words.len())..].to_vec())
}

/// The words `cargo run` hands to the program it builds, when `words` is such a call. Cargo's own
/// options and a `+toolchain` may come before `run` (`cargo -q run`, `cargo +stable run`).
pub fn cargo_run_args(words: &[String]) -> Option<Vec<String>> {
    let i = program_index(words)?;
    if basename(&words[i]) != "cargo" {
        return None;
    }
    const OWN_WITH_VALUE: &[&str] = &["--color", "--config", "-Z", "-C", "--explain"];
    let mut k = i + 1;
    while k < words.len() && (words[k].starts_with('-') || words[k].starts_with('+')) {
        k += if OWN_WITH_VALUE.contains(&words[k].as_str()) {
            2
        } else {
            1
        };
    }
    if !matches!(words.get(k)?.as_str(), "run" | "r") {
        return None;
    }
    let rest = &words[k + 1..];
    if let Some(d) = rest.iter().position(|w| w == "--") {
        return Some(rest[d + 1..].to_vec());
    }
    const WITH_VALUE: &[&str] = &[
        "--bin",
        "-p",
        "--package",
        "--features",
        "-F",
        "--manifest-path",
        "--target",
        "--target-dir",
        "--profile",
        "-j",
        "--jobs",
        "--example",
        "--color",
        "--config",
        "-Z",
        "--message-format",
        "--lockfile-path",
    ];
    let mut j = 0;
    while j < rest.len() && rest[j].starts_with('-') {
        j += if WITH_VALUE.contains(&rest[j].as_str()) {
            2
        } else {
            1
        };
    }
    Some(rest[j.min(rest.len())..].to_vec())
}

/// Split the words of one simple command where a zsh short form starts a command of its own: after the
/// `]]` of `if|elif|while|until [[ ... ]]`, and at a `{` after a command that starts with a keyword,
/// `for`, `select`, `foreach`, `coproc`, `always`, `function` (zsh's `function f g { ... }` names two)
/// or `time`. (A quoted `{` is split too; that only reads more.)
fn split_short_forms(words: Vec<String>) -> Vec<Vec<String>> {
    let mut parts = Vec::new();
    let mut rest = words;
    loop {
        let starts_kw = rest.first().is_some_and(|w| {
            KEYWORDS.contains(&w.as_str())
                || matches!(
                    w.as_str(),
                    "for" | "select" | "foreach" | "coproc" | "always" | "function" | "time"
                )
        });
        let mut at = None;
        if starts_kw {
            let s = rest
                .iter()
                .position(|w| !KEYWORDS.contains(&w.as_str()))
                .unwrap_or(rest.len());
            if rest.get(s).is_some_and(|w| w == "[[") {
                if let Some(e) = rest[s..].iter().position(|w| w == "]]") {
                    if s + e + 1 < rest.len() {
                        at = Some(s + e + 1);
                    }
                }
            }
            if at.is_none() {
                at = rest.iter().skip(1).position(|w| w == "{").map(|k| k + 1);
            }
        }
        match at {
            Some(k) if k > 0 && k < rest.len() => {
                let tail = rest.split_off(k);
                parts.push(rest);
                rest = tail;
            }
            _ => break,
        }
    }
    parts.push(rest);
    parts
}

/// The index of the command string of a shell run with `-c` (`words[i]` is the shell): the first word
/// after all the options, as bash and zsh read them (`bash -c -e 'cmd'`, `bash --norc -c 'cmd'`,
/// `bash -o posix -c 'cmd'`, `bash +x -c 'cmd'`). `None` when there is no `-c`.
fn shell_command_string(words: &[String], i: usize) -> Option<usize> {
    let mut j = i + 1;
    let mut dash_c = false;
    while j < words.len() {
        let w = words[j].as_str();
        if w == "--" || w == "-" {
            j += 1;
            break;
        }
        if w.starts_with("--") {
            j += if matches!(w, "--rcfile" | "--init-file" | "--emulate") {
                2
            } else {
                1
            };
            continue;
        }
        if (w.starts_with('-') || w.starts_with('+')) && w.len() > 1 {
            let flags = &w[1..];
            if w.starts_with('-') && flags.contains('c') {
                dash_c = true;
            }
            // `-o name`, `+o name`, `-O name`, `+O name` take the next word.
            j += if flags.contains(['o', 'O']) { 2 } else { 1 };
            continue;
        }
        break;
    }
    (dash_c && j < words.len()).then_some(j)
}

/// Is `w` the `{name}` of a redirect that puts a new descriptor in a variable (`{fd}>file`)?
fn is_fd_var(w: &str) -> bool {
    w.strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .is_some_and(|v| {
            !v.is_empty()
                && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !v.starts_with(|c: char| c.is_ascii_digit())
        })
}

/// Does the `$` at `b[i]` start an expansion other than a plain one? Plain are `$name`, `${name}`, a
/// positional or special parameter (`$1`, `$@`, `$?`) and a command substitution `$( )`. Every other
/// form may evaluate text as code or turn it into other words: arithmetic (`$((`, `$[`), a subscript
/// (`${a[V]}`, zsh's `$x[V]`), an operator (`${x:V}`, `${V@P}`, `${V:-w}`), a zsh flag or modifier
/// (`${(e)V}`, `$~V`, `$=V`, `$^V`). Read as plain text when the `$` stands alone.
fn evaluates_at(b: &[char], i: usize) -> bool {
    if b.get(i) != Some(&'$') {
        return false;
    }
    let name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    match b.get(i + 1) {
        Some('(') => b.get(i + 2) == Some(&'('),
        Some('{') => {
            // `${name}` with nothing after the name and no subscript after the brace.
            let mut j = i + 2;
            while j < b.len() && name(b[j]) {
                j += 1;
            }
            !(j > i + 2 && b.get(j) == Some(&'}') && b.get(j + 1) != Some(&'['))
        }
        Some(c) if name(*c) => {
            let mut j = i + 1;
            while j < b.len() && name(b[j]) {
                j += 1;
            }
            b.get(j) == Some(&'[')
        }
        Some('@' | '*' | '?' | '!' | '$' | '-') => false,
        Some('#') => b.get(i + 2).is_some_and(|c| name(*c) || *c == '{'),
        // zsh's `$~V`, `$=V`, `$^V`, `$+V`, and arithmetic `$[ ]`; any other character leaves the `$`
        // as text.
        Some(c) => matches!(c, '~' | '=' | '^' | '+' | '['),
        None => false,
    }
}

/// Is `s` a zsh number range glob at its start (`<->`, `<1-9>`), and how long is it?
fn number_range(b: &[char], i: usize) -> Option<usize> {
    if b.get(i) != Some(&'<') {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    if b.get(j) != Some(&'-') {
        return None;
    }
    j += 1;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    (b.get(j) == Some(&'>')).then_some(j + 1 - i)
}

/// Text with its shell quotes and backslashes removed, read as one word: no comment, no operator.
fn unquote(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '\\' if i + 1 < b.len() => {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            q @ ('\'' | '"') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if q == '"' && b[i] == '\\' && i + 1 < b.len() {
                        i += 1;
                    }
                    out.push(b[i]);
                    i += 1;
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// Text with its quoted parts and escaped characters left out: what the shell reads as special.
fn unquoted_only(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '\\' => i += 2,
            q @ ('\'' | '"') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The shell code of the zsh `e` glob qualifiers in the text of a `( )` group (`e:'cmd':`, `e{cmd}`,
/// `#qe:cmd:`), with quotes removed. The text is read loosely and fails closed: any `e` followed by
/// any character counts, that character taken as the delimiter, and a `#` is never a comment here.
fn qualifier_code(group: &str, _depth: usize) -> Vec<String> {
    let plain: Vec<char> = unquote(group).chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k + 1 < plain.len() {
        let d = plain[k + 1];
        if plain[k] == 'e' && !d.is_whitespace() && d != ')' {
            let close = match d {
                '(' => ')',
                '[' => ']',
                '{' => '}',
                '<' => '>',
                x => x,
            };
            let mut j = k + 2;
            let mut code = String::new();
            while j < plain.len() && plain[j] != close {
                code.push(plain[j]);
                j += 1;
            }
            // Every `e` is a candidate, so a false one hides no later qualifier.
            if !code.trim().is_empty() {
                out.push(code);
            }
        }
        k += 1;
    }
    out
}

/// Is there a command whose program is built at run time (`$B`, `` `x` ``) and which has the word
/// `word` among its arguments? (`B=bf; $B human-turn`.)
pub fn runtime_program_with_word(cmd: &str, word: &str) -> bool {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    cmds.iter().any(|c| {
        resolve(&c.words)
            .map(|(i, _)| c.words[i].contains('$') || c.words[i].contains('`'))
            .unwrap_or(false)
            && c.words.iter().skip(1).any(|w| w == word)
    })
}

/// Does `cargo` or a shell get an argument that holds `phrase`? The body of a heredoc is not an argument.
/// (Code given to a language interpreter is judged by the guard for a call, not for the phrase alone.)
pub fn interpreter_arg_with_phrase(cmd: &str, phrase: &str) -> bool {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    cmds.iter().any(|c| {
        resolve(&c.words)
            .map(|(i, _)| {
                matches!(basename(&c.words[i]), "cargo" | "sh" | "bash" | "zsh")
                    && c.words[i + 1..].iter().any(|w| w.contains(phrase))
            })
            .unwrap_or(false)
    })
}

/// One simple command, as the guard's path rules read it.
#[derive(Debug, Clone)]
pub struct Parsed {
    /// The words, quotes removed. Redirect operators and their targets are not among them.
    pub words: Vec<String>,
    /// The targets of the output redirects (`>`, `>>`, `>|`, `&>`).
    pub redirects: Vec<String>,
    /// The body of a heredoc fed to the command.
    pub heredoc: Option<String>,
    /// The text the command reads on standard input when the line says what it is: a heredoc, a here
    /// string, or what `echo` and `printf` pipe into it. `None` when the input is unknown.
    pub input: Option<String>,
    /// The shell the command runs in: 0 for the command line itself, a new number for each `sh -c`
    /// string and each script fed to a shell. A `cd` holds only inside its own shell.
    pub scope: usize,
    /// The shell that started this one (its working directory is the starting point).
    pub parent: usize,
    /// For the commands of a `sh -c` string: the words after the string (`$0`, `$1`, ...).
    pub bound: Option<Vec<String>>,
    /// The command reads a pipe (`a | this`), whether or not `input` knows its text.
    pub piped: bool,
    /// A shell, `source` or `.` that reads a script from a pipe or a process substitution whose text
    /// the parse cannot see (`... | rev | sh`, `sh <(curl ..)`).
    pub script_unknown: bool,
}

/// The index of the program word of a simple command (after assignments, keywords and wrappers).
pub fn program_index(words: &[String]) -> Option<usize> {
    resolve(words).map(|(i, _)| i)
}

/// Every simple command a shell would run for `cmd`, in order, with the commands inside `$( )`,
/// backticks, `sh -c '...'`, `eval` and a script fed to a shell read the same way.
pub fn commands(cmd: &str) -> Vec<Parsed> {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    let mut out = Vec::new();
    let mut next_scope = 1usize;
    flatten(cmds, 0, 0, &mut next_scope, 0, &mut out);
    out
}

fn flatten(
    cmds: Vec<Cmd>,
    scope: usize,
    parent: usize,
    next_scope: &mut usize,
    depth: usize,
    out: &mut Vec<Parsed>,
) {
    flatten_bound(cmds, scope, parent, next_scope, depth, out, None)
}

fn flatten_bound(
    cmds: Vec<Cmd>,
    scope: usize,
    parent: usize,
    next_scope: &mut usize,
    depth: usize,
    out: &mut Vec<Parsed>,
    bound: Option<Vec<String>>,
) {
    if depth > 8 {
        return;
    }
    let texts = producer_texts(&cmds);
    for cmd in cmds {
        let idx = resolve(&cmd.words).map(|(i, _)| i);
        out.push(Parsed {
            words: cmd.words.clone(),
            redirects: cmd.redirs.clone(),
            heredoc: cmd.heredoc.as_ref().map(|(b, _)| b.clone()),
            input: cmd
                .heredoc
                .as_ref()
                .map(|(b, _)| b.clone())
                .or_else(|| cmd.pipe_from.and_then(|id| texts.get(&id).cloned())),
            scope,
            parent,
            bound: bound.clone(),
            piped: cmd.pipe_from.is_some(),
            script_unknown: idx.is_some_and(|i| {
                let prog = basename(&cmd.words[i]);
                // A script file named by its path is not read from the input.
                let operand = cmd.words[i + 1..]
                    .iter()
                    .find(|a| !a.starts_with('-') || a.as_str() == "-");
                // With `-s` the script comes from the input whatever words follow.
                let dash_s = cmd.words[i + 1..]
                    .iter()
                    .take_while(|a| a.starts_with('-') || a.starts_with('+'))
                    .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('s'));
                let from_input = dash_s
                    || operand.is_none_or(|a| {
                        matches!(a.as_str(), "-" | "/dev/stdin" | "/dev/fd/0" | "<(..)")
                    });
                (SHELLS.contains(&prog) || matches!(prog, "source" | "."))
                    && from_input
                    && shell_command_string(&cmd.words, i).is_none()
                    && (cmd.pipe_from.is_some() || cmd.words[i + 1..].iter().any(|w| w == "<(..)"))
                    && script_text(&cmd, &texts).is_none()
            }),
        });
        // A quoted program word with spaces is read for calls of the engine only: as a path it is still
        // the one program the path rules judge.
        for (line, _) in handed_lines(&cmd.words)
            .into_iter()
            .filter(|(_, how)| !is_quoted_command(how))
        {
            let mut c2 = 0usize;
            let mut inner = Vec::new();
            parse(&line, None, depth + 1, &mut c2, &mut inner);
            let id = *next_scope;
            *next_scope += 1;
            flatten(inner, id, scope, next_scope, depth + 1, out);
        }
        let Some(i) = idx else { continue };
        let prog = basename(&cmd.words[i]);
        if prog == "find" {
            // The command after an execute flag is a command of its own.
            for words in exec_commands_at(&cmd.words[i + 1..]) {
                let inner = vec![Cmd::of(words)];
                flatten(inner, scope, parent, next_scope, depth + 1, out);
            }
        }
        if SHELLS.contains(&prog) {
            let mut found = false;
            if let Some(j) = shell_command_string(&cmd.words, i) {
                let s = &cmd.words[j];
                let mut c2 = 0usize;
                let mut inner = Vec::new();
                parse(s, None, depth + 1, &mut c2, &mut inner);
                let id = *next_scope;
                *next_scope += 1;
                let rest: Vec<String> = cmd.words[(j + 1).min(cmd.words.len())..].to_vec();
                flatten_bound(inner, id, scope, next_scope, depth + 1, out, Some(rest));
                found = true;
            }
            if !found {
                let script = script_text(&cmd, &texts);
                if let Some(body) = &script {
                    let mut c2 = 0usize;
                    let mut inner = Vec::new();
                    parse(body, None, depth + 1, &mut c2, &mut inner);
                    let id = *next_scope;
                    *next_scope += 1;
                    // `$0`, `$1`, ... of the script are the words after the script operand
                    // (`<(..)`, `/dev/stdin`, `-`), or after `-s`.
                    let args = &cmd.words[i + 1..];
                    let mut bound_words: Vec<String> = vec!["_".to_string()];
                    let mut k = 0;
                    while k < args.len() && args[k].starts_with('-') && args[k] != "-" {
                        k += 1;
                    }
                    if k < args.len() {
                        if args.iter().take(k).any(|a| a == "-s") {
                            bound_words.extend(args[k..].iter().cloned());
                        } else {
                            bound_words = args[k..].to_vec();
                        }
                    }
                    flatten_bound(
                        inner,
                        id,
                        scope,
                        next_scope,
                        depth + 1,
                        out,
                        Some(bound_words),
                    );
                }
            }
        } else if matches!(prog, "source" | ".") && reads_input(&cmd.words[i + 1..]) {
            // `source /dev/stdin <<< '...'`, `. <(echo '...')`: the text runs in this shell.
            if let Some(body) = script_text(&cmd, &texts) {
                let mut c2 = 0usize;
                let mut inner = Vec::new();
                parse(&body, None, depth + 1, &mut c2, &mut inner);
                flatten(inner, scope, parent, next_scope, depth + 1, out);
            }
        } else if prog == "trap" {
            if let Some(s) = trap_command(&cmd.words[i + 1..]) {
                let mut c2 = 0usize;
                let mut inner = Vec::new();
                parse(&s, None, depth + 1, &mut c2, &mut inner);
                flatten(inner, scope, parent, next_scope, depth + 1, out);
            }
        } else if prog == "eval" {
            let s = cmd.words[i + 1..].join(" ");
            let mut c2 = 0usize;
            let mut inner = Vec::new();
            parse(&s, None, depth + 1, &mut c2, &mut inner);
            flatten(inner, scope, parent, next_scope, depth + 1, out);
        }
    }
}

fn collect(cmds: Vec<Cmd>, via: Option<&str>, depth: usize, calls: &mut Vec<Call>) {
    if depth > 8 {
        return;
    }
    let texts = producer_texts(&cmds);
    for cmd in cmds {
        for (line, how) in handed_lines(&cmd.words) {
            let mut c2 = 0usize;
            let mut inner = Vec::new();
            parse(&line, None, depth + 1, &mut c2, &mut inner);
            collect(inner, Some(&how), depth + 1, calls);
        }
        let Some((i, wrap)) = resolve(&cmd.words) else {
            continue;
        };
        if basename(&cmd.words[i]) == "find" {
            for words in exec_commands(&cmd.words[i + 1..]) {
                let inner = vec![Cmd::of(words)];
                collect(inner, Some("find -exec"), depth + 1, calls);
            }
        }
        let eff: Option<String> = cmd
            .via
            .clone()
            .or_else(|| via.map(|v| v.to_string()))
            .or(wrap);
        // zsh's `=bf` (with `equals` set) is the path of `bf`.
        let prog = basename(cmd.words[i].strip_prefix('=').unwrap_or(&cmd.words[i])).to_string();
        let engine = is_engine(&cmd.words[i]);
        // A program that is a pattern or a brace group may become `bf` (`b?`, `b[f]`, `b(f)` where a
        // file `bf` exists, `{bf,x}`): it is a call that cannot be read. Case is ignored.
        let lower = prog.to_ascii_lowercase();
        let pattern_prog = cmd.group_word == Some(i)
            || (lower.contains(['*', '?', '[']) && crate::guard::pattern_matches(&lower, "bf"))
            || (lower.contains('{') && (lower.contains(',') || lower.contains("..")));
        if pattern_prog && !engine {
            let mut words = vec!["bf".to_string()];
            words.extend(cmd.words[i + 1..].iter().cloned());
            calls.push(Call {
                words,
                via: Some(via_text("pattern")),
                group: true,
                cargo: false,
            });
        }
        // `hash q=/path/bf` (zsh) or `hash -p /path/bf q` (bash) names the engine under another name;
        // a value built at run time may be it too. (`hash -d` names a folder.)
        if prog == "hash"
            && !cmd.words[i + 1..].iter().any(|a| a == "-d")
            && cmd.words[i + 1..].iter().any(|a| {
                let v = a.split_once('=').map(|(_, v)| v).unwrap_or(a);
                is_engine(v) || v.contains('$') || v.contains('`')
            })
        {
            calls.push(Call {
                words: vec!["bf".to_string()],
                via: Some("hash".to_string()),
                group: cmd.group,
                cargo: false,
            });
        }
        if let Some(args) = cargo_run_args(&cmd.words) {
            let mut words = vec!["bf".to_string()];
            words.extend(args);
            calls.push(Call {
                words,
                via: eff.clone(),
                group: cmd.group,
                cargo: true,
            });
        } else if let Some(args) = cargo_alias_args(&cmd.words) {
            // An alias may be `run`: its words cannot be read, as after a pattern group.
            let mut words = vec!["bf".to_string()];
            words.extend(args);
            calls.push(Call {
                words,
                via: eff.clone(),
                group: true,
                cargo: true,
            });
        }
        if engine {
            let words: Vec<String> = cmd.words[i..].to_vec();
            // `bf evidence run <check> ... -- <command>`: the command after `--` is a command line of
            // its own, read by the same rule. Flags may stand anywhere (`bf -C . evidence run`,
            // `bf evidence --unit U-1 run`), as the engine's own parser reads them.
            if let Some(rest) = evidence_command(&words) {
                let inner = vec![Cmd::of(rest)];
                collect(inner, None, depth + 1, calls);
            }
            calls.push(Call {
                words,
                via: eff,
                group: cmd.group,
                cargo: false,
            });
        } else if SHELLS.contains(&prog.as_str()) {
            // `sh -c '<string>'`: the string is a command line.
            let mut found = false;
            if let Some(j) = shell_command_string(&cmd.words, i) {
                let mut c2 = 0usize;
                let mut inner = Vec::new();
                parse(&cmd.words[j], None, depth + 1, &mut c2, &mut inner);
                collect(inner, Some("sh -c"), depth + 1, calls);
                found = true;
            }
            if !found {
                // A heredoc, a pipe or a process substitution fed to a shell is a script: its lines
                // are commands.
                let script = script_text(&cmd, &texts);
                if let Some(body) = &script {
                    let mut c2 = 0usize;
                    let mut inner = Vec::new();
                    parse(body, None, depth + 1, &mut c2, &mut inner);
                    collect(inner, None, depth + 1, calls);
                }
            }
        } else if matches!(prog.as_str(), "source" | ".") && reads_input(&cmd.words[i + 1..]) {
            if let Some(body) = script_text(&cmd, &texts) {
                let mut c2 = 0usize;
                let mut inner = Vec::new();
                parse(&body, None, depth + 1, &mut c2, &mut inner);
                collect(inner, Some("source"), depth + 1, calls);
            }
        } else if prog == "trap" {
            if let Some(s) = trap_command(&cmd.words[i + 1..]) {
                let mut c2 = 0usize;
                let mut inner = Vec::new();
                parse(&s, None, depth + 1, &mut c2, &mut inner);
                collect(inner, Some("trap"), depth + 1, calls);
            }
        } else if prog == "eval" {
            let s = cmd.words[i + 1..].join(" ");
            let mut c2 = 0usize;
            let mut inner = Vec::new();
            parse(&s, None, depth + 1, &mut c2, &mut inner);
            collect(inner, Some("eval"), depth + 1, calls);
        }
    }
}

/// The index of the program word of a simple command and the wrapper it was reached through, skipping
/// assignments, shell keywords and the wrappers the guard knows. `None` when the command is not a
/// call of anything (or the reading is unsure).
fn resolve(words: &[String]) -> Option<(usize, Option<String>)> {
    let mut i = 0;
    let mut via: Option<String> = None;
    loop {
        while i < words.len() && (is_assignment(&words[i]) || KEYWORDS.contains(&words[i].as_str()))
        {
            i += 1;
        }
        let w = words.get(i)?;
        let base = basename(w);
        let wrapper_at = i;
        // Options that take an argument, per wrapper.
        // The last number is how many plain words follow the options before the command (a time limit,
        // a lock file, a mask, a pseudo terminal's log file).
        let (kind, with_arg, skip_pos): (&str, &[&str], usize) = match base {
            "env" => ("env", &["-u", "-C", "-S", "--unset", "--chdir"], 0),
            "command" => ("command", &[], 0),
            "exec" => ("exec", &["-a"], 0),
            "builtin" => ("builtin", &[], 0),
            "xargs" => (
                "xargs",
                &["-n", "-I", "-P", "-L", "-s", "-d", "-E", "-a", "-l", "-i"],
                0,
            ),
            "time" => ("", &[], 0),
            "nohup" => ("", &[], 0),
            "nice" => ("", &["-n"], 0),
            "sudo" | "doas" => ("", &["-u", "-g", "-C", "-h", "-p", "-r", "-t", "-U"], 0),
            "timeout" => ("", &["-s", "-k", "--signal", "--kill-after"], 1),
            "gtimeout" => ("", &["-s", "-k", "--signal", "--kill-after"], 1),
            "stdbuf" => ("", &["-i", "-o", "-e"], 0),
            "flock" => ("", &["-w", "-E", "--timeout"], 1),
            "setsid" | "unbuffer" | "caffeinate" | "dtruss" | "strace" | "ltrace" | "arch"
            | "time-limit" | "winpty" => ("", &[], 0),
            "ionice" => ("", &["-c", "-n", "-p"], 0),
            "chrt" => ("", &[], 1),
            "taskset" => ("", &[], 1),
            "script" => ("", &["-t", "-F"], 1),
            "watch" => ("", &["-n", "-d"], 0),
            "chroot" => ("", &[], 1),
            // zsh's precommand modifiers and bash's and zsh's `coproc` run the words after them.
            "noglob" | "nocorrect" | "-" | "coproc" => ("", &[], 0),
            // zsh's `repeat N cmd` and `function name { ... }`: the count or the name comes first.
            "repeat" | "function" => ("", &[], 1),
            // `rustup run <toolchain> cmd`: the word `run` and the toolchain come first.
            // Only `run` runs a command; any other subcommand (`component add clippy`) is not a wrapper.
            "rustup" => {
                let sub = words[i + 1..]
                    .iter()
                    .find(|a| !a.starts_with('-') && !a.starts_with('+'));
                if sub.map(String::as_str) != Some("run") {
                    return Some((i, via));
                }
                ("", &[], 0)
            }
            "sandbox-exec" => ("", &["-f", "-n", "-p", "-D"], 0),
            "pkexec" => ("", &["--user"], 0),
            "runuser" => ("", &["-u", "-g", "-G", "-s", "-w"], 0),
            "unshare" | "fakeroot" => ("", &[], 0),
            "nsenter" => ("", &["-t", "--target"], 0),
            "systemd-run" => ("", &["-p", "--property", "-u", "--unit"], 0),
            "parallel" => (
                "parallel",
                &[
                    "-j",
                    "--jobs",
                    "-S",
                    "--sshlogin",
                    "-a",
                    "--arg-file",
                    "-d",
                    "--delimiter",
                    "--colsep",
                    "-I",
                    "--joblog",
                    "--results",
                    "--tmpdir",
                ],
                0,
            ),
            _ => return Some((i, via)),
        };
        i += 1;
        // `command -v bf` asks whether it exists; it does not run it.
        if base == "command"
            && words
                .get(i)
                .map(|s| s.starts_with("-v") || s.starts_with("-V"))
                == Some(true)
        {
            return None;
        }
        while i < words.len() {
            let o = &words[i];
            if with_arg.contains(&o.as_str()) {
                i += 2;
            } else if (o.starts_with('-') && o.len() > 1) || (kind == "env" && is_assignment(o)) {
                i += 1;
            } else {
                break;
            }
        }
        if base == "rustup" {
            // Find the word `run` itself (rustup's own options and a `+toolchain` may come before it),
            // then run's own options (`--install`, `-h`), the toolchain and an optional `--`.
            i = wrapper_at + 1;
            while i < words.len() && words[i] != "run" {
                i += 1;
            }
            i += 1;
            // Run's options, `--` and the toolchain may come in any order; the first other word after
            // the toolchain is the program.
            let mut toolchain = false;
            while i < words.len() {
                let w = words[i].as_str();
                if w.starts_with('-') {
                    i += 1;
                } else if !toolchain {
                    toolchain = true;
                    i += 1;
                } else {
                    break;
                }
            }
        }
        i += skip_pos;
        if !kind.is_empty() {
            via = Some(kind.to_string());
        }
    }
}

/// Read `text` into simple commands, pushing them (and the commands inside `$( )` and backticks) onto
/// `out`.
fn parse(text: &str, via: Option<&str>, depth: usize, counter: &mut usize, out: &mut Vec<Cmd>) {
    if depth > 8 {
        return;
    }
    let b: Vec<char> = text.chars().collect();
    let n = b.len();
    let mut i = 0;
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    // Set after a redirect operator: `Some(true)` for an output redirect, `Some(false)` for an input one.
    let mut redir: Option<bool> = None;
    let mut redirs: Vec<String> = Vec::new();
    // `<<<`: the next word is the text fed to the command.
    let mut here_string = false;
    // The command that pipes into the one being read.
    let mut pipe_in: Option<usize> = None;
    let mut pending: Vec<Pending> = Vec::new();
    // The text each `<( )` of the command being read writes.
    let mut procs: Vec<String> = Vec::new();
    let mut cmd_id = *counter;
    *counter += 1;
    let mut bodies: HashMap<usize, (String, bool)> = HashMap::new();
    // The command being read holds a pattern group (see `Call::group`).
    let mut group = false;
    let mut group_word: Option<usize> = None;
    // The word being read holds a quote or a backslash; the first word of the command did.
    let mut cur_quoted = false;
    let mut first_quoted = false;
    // Functions the command being read defines, and whether it reads an input redirect.
    let mut defines: Vec<String> = Vec::new();
    let mut reads_redirect = false;
    // The last thing read closed a subshell or arithmetic: a redirect now belongs to it.
    let mut after_close = false;
    // The groups opened and not yet closed: `true` for a subshell, `false` for the list of a
    // `case` pattern or a `for x (..)`.
    let mut sub_depth: Vec<bool> = Vec::new();
    // Heredoc bodies whose delimiter line was not found.
    let mut unclosed: Vec<String> = Vec::new();
    // The command being read evaluates text as code (see `Cmd::evaluates`).
    let mut evaluates = false;
    // After `>&` or `<&` and a space: a target of digits or `-` names a descriptor, not a file.
    let mut dup = false;

    macro_rules! end_word {
        () => {
            if in_word {
                let w = std::mem::take(&mut cur);
                match redir.take() {
                    Some(true) => {
                        // `>& 2`, `>&-` and `>&$fd` name a descriptor.
                        if !(dup
                            && (w == "-"
                                || w.starts_with('$')
                                || w.chars().all(|d| d.is_ascii_digit())))
                        {
                            redirs.push(w)
                        }
                    }
                    Some(false) => {
                        if here_string {
                            bodies.insert(cmd_id, (format!("{w}\n"), true));
                        }
                    }
                    None => {
                        if words.is_empty() {
                            first_quoted = cur_quoted;
                        }
                        after_close = false;
                        words.push(w)
                    }
                }
                cur.clear();
                in_word = false;
                here_string = false;
                dup = false;
            }
            cur_quoted = false;
        };
    }
    macro_rules! end_cmd {
        () => {
            end_word!();
            redir = None;
            dup = false;
            if !words.is_empty() || !redirs.is_empty() || evaluates || reads_redirect {
                let attached = words.is_empty() && after_close;
                // `function f [g] { ... }` with an unquoted keyword defines its names.
                if words.first().is_some_and(|w| w == "function") && !first_quoted {
                    defines.extend(
                        words[1..]
                            .iter()
                            .take_while(|w| !matches!(w.as_str(), "{" | "()"))
                            .cloned(),
                    );
                }
                // zsh's short forms run a command in the same simple command: `if [[ x ]] cmd`,
                // `while (( x )) { cmd }`, `coproc NAME { cmd; }`.
                let mut parts = split_short_forms(std::mem::take(&mut words));
                words = parts.pop().unwrap_or_default();
                let first_len = parts.iter().map(|p| p.len()).sum::<usize>();
                for p in parts {
                    out.push(Cmd {
                        id: *counter,
                        via: via.map(|v| v.to_string()),
                        group,
                        defines: std::mem::take(&mut defines),
                        ..Cmd::of(p)
                    });
                    *counter += 1;
                }
                // The pattern word's index in the last part.
                let gw = group_word.and_then(|k| k.checked_sub(first_len));
                out.push(Cmd {
                    id: cmd_id,
                    words: std::mem::take(&mut words),
                    via: via.map(|v| v.to_string()),
                    heredoc: None,
                    redirs: std::mem::take(&mut redirs),
                    pipe_from: pipe_in.take(),
                    procs: std::mem::take(&mut procs),
                    group,
                    group_word: gw,
                    evaluates,
                    defines: std::mem::take(&mut defines),
                    reads_redirect,
                    attached,
                });
            }
            after_close = false;
            group = false;
            group_word = None;
            evaluates = false;
            defines.clear();
            reads_redirect = false;
            first_quoted = false;
            cmd_id = *counter;
            *counter += 1;
        };
    }

    while i < n {
        let c = b[i];
        match c {
            '\\' => {
                if i + 1 < n {
                    if b[i + 1] != '\n' {
                        cur.push(b[i + 1]);
                        in_word = true;
                        cur_quoted = true;
                    }
                    i += 1;
                }
            }
            '\'' => {
                in_word = true;
                cur_quoted = true;
                i += 1;
                while i < n && b[i] != '\'' {
                    cur.push(b[i]);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                cur_quoted = true;
                i += 1;
                while i < n && b[i] != '"' {
                    if b[i] == '\\' && i + 1 < n {
                        if matches!(b[i + 1], '$' | '`' | '"' | '\\') {
                            cur.push(b[i + 1]);
                        } else if b[i + 1] != '\n' {
                            cur.push('\\');
                            cur.push(b[i + 1]);
                        }
                        i += 2;
                        continue;
                    }
                    if evaluates_at(&b, i) {
                        evaluates = true;
                    }
                    // bash 5.3's `${ cmd; }` and `${| cmd; }` inside double quotes.
                    if b[i] == '$'
                        && b.get(i + 1) == Some(&'{')
                        && matches!(b.get(i + 2), Some(' ' | '\t' | '\n' | '|'))
                    {
                        let (inner, next) = braced(&b, i + 2);
                        let inner = inner.strip_prefix('|').unwrap_or(&inner).to_string();
                        cur.push_str("$(..)");
                        parse(&inner, Some("$("), depth + 1, counter, out);
                        i = next;
                        continue;
                    }
                    if b[i] == '$' && i + 1 < n && b[i + 1] == '(' {
                        let (inner, next) = balanced(&b, i + 2);
                        cur.push_str("$(..)");
                        parse(&inner, Some("$("), depth + 1, counter, out);
                        i = next;
                        continue;
                    }
                    if b[i] == '`' {
                        let (inner, j) = backtick(&b, i);
                        cur.push_str("`..`");
                        parse(&inner, Some("`"), depth + 1, counter, out);
                        i = j + 1;
                        continue;
                    }
                    cur.push(b[i]);
                    i += 1;
                }
            }
            '$' if i + 1 < n && b[i + 1] == '(' => {
                in_word = true;
                evaluates |= evaluates_at(&b, i);
                let (inner, next) = balanced(&b, i + 2);
                cur.push_str("$(..)");
                parse(&inner, Some("$("), depth + 1, counter, out);
                i = next;
                continue;
            }
            '$' if b.get(i + 1) == Some(&'\'') => {
                // `$'...'`: a quote whose backslash escapes the shell decodes (`$'\''`, `$'\x72'` is `r`,
                // `$'\157'` is `o`). Its text is the word's; an escape the parse cannot decode leaves a
                // `$` in front, so the word counts as one the parse does not read.
                in_word = true;
                cur_quoted = true;
                i += 2;
                let (text, unknown, next) = ansi_c(&b, i);
                i = next;
                if unknown {
                    cur.push('$');
                }
                cur.push_str(&text);
            }
            '$' if b.get(i + 1) == Some(&'{')
                && matches!(b.get(i + 2), Some(' ' | '\t' | '\n' | '|')) =>
            {
                // bash 5.3's `${ cmd; }` and `${| cmd; }` run commands in this shell, like `$( )`.
                in_word = true;
                let (inner, next) = braced(&b, i + 2);
                let inner = inner.strip_prefix('|').unwrap_or(&inner).to_string();
                cur.push_str("$(..)");
                parse(&inner, Some("$("), depth + 1, counter, out);
                i = next;
                continue;
            }
            '$' if b.get(i + 1) == Some(&'[') => {
                // `$[ ... ]`: arithmetic. Its text is read on its own, so a `<<` in it takes no heredoc
                // body, and a substitution in it still runs.
                in_word = true;
                evaluates = true;
                let mut j = i + 2;
                let mut d = 1usize;
                while j < n {
                    match b[j] {
                        '[' => d += 1,
                        ']' => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let inner: String = b[(i + 2).min(n)..j.min(n)].iter().collect();
                parse(&inner, Some("$("), depth + 1, counter, out);
                cur.push_str("$[..]");
                i = j;
            }
            '$' if b.get(i + 1) == Some(&'{') => {
                // `${...}` up to its own `}`: quotes and nested braces inside it are skipped, and the
                // substitutions inside it run (`${V:-$(cmd)}`).
                in_word = true;
                evaluates |= evaluates_at(&b, i);
                cur.push('$');
                let mut depth_b = 0usize;
                i += 1;
                while i < n {
                    let ch = b[i];
                    if ch == '\\' && i + 1 < n {
                        cur.push(ch);
                        cur.push(b[i + 1]);
                        i += 2;
                        continue;
                    }
                    if ch == '\'' || ch == '"' {
                        cur.push(ch);
                        i += 1;
                        while i < n && b[i] != ch {
                            if ch == '"' && b[i] == '\\' && i + 1 < n {
                                cur.push(b[i]);
                                i += 1;
                            }
                            cur.push(b[i]);
                            i += 1;
                        }
                        if i < n {
                            cur.push(ch);
                        }
                        i += 1;
                        continue;
                    }
                    if ch == '$' && b.get(i + 1) == Some(&'(') {
                        let (inner, next) = balanced(&b, i + 2);
                        cur.push_str("$(..)");
                        parse(&inner, Some("$("), depth + 1, counter, out);
                        i = next;
                        continue;
                    }
                    if ch == '`' {
                        let (inner, j) = backtick(&b, i);
                        cur.push_str("`..`");
                        parse(&inner, Some("`"), depth + 1, counter, out);
                        i = j + 1;
                        continue;
                    }
                    cur.push(ch);
                    if ch == '{' {
                        depth_b += 1;
                    } else if ch == '}' {
                        depth_b -= 1;
                        if depth_b == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
            }
            '$' => {
                in_word = true;
                evaluates |= evaluates_at(&b, i);
                cur.push('$');
            }
            '`' => {
                in_word = true;
                let (inner, j) = backtick(&b, i);
                cur.push_str("`..`");
                parse(&inner, Some("`"), depth + 1, counter, out);
                i = j + 1;
                continue;
            }
            // A comment starts only where a word starts after a blank or an operator; a `#` right after a
            // `(` or `)` is inside a zsh pattern group (`(#)x(N)`, `/(#qe:..:)`) and is a word character.
            '#' if !in_word
                && (i == 0 || matches!(b[i - 1], ' ' | '\t' | '\r' | '\n' | ';' | '&' | '|')) =>
            {
                while i < n && b[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '\n' => {
                end_cmd!();
                // Heredocs declared on this line read their bodies now.
                for p in pending.drain(..) {
                    let mut body = String::new();
                    let mut found = false;
                    i += 1;
                    while i < n {
                        let mut j = i;
                        while j < n && b[j] != '\n' {
                            j += 1;
                        }
                        let line: String = b[i..j].iter().collect();
                        i = j;
                        let l = if p.strip_tabs {
                            line.trim_start_matches('\t')
                        } else {
                            line.as_str()
                        };
                        if l == p.delim {
                            found = true;
                            break;
                        }
                        body.push_str(&line);
                        body.push('\n');
                        i += 1;
                    }
                    // A body whose delimiter line never comes may be a misreading: its lines are also
                    // read as commands, so nothing the shell may run is hidden.
                    if !found {
                        unclosed.push(body.clone());
                    }
                    bodies.insert(p.cmd, (body, p.expands));
                    // `i` is at the newline that ends the delimiter line (or the end); the loop's own
                    // increment steps past it.
                }
            }
            ' ' | '\t' | '\r' => {
                end_word!();
            }
            '(' if b.get(i + 1) == Some(&'(')
                && matches!(balanced(&b, i + 2), (_, next) if b.get(next) == Some(&')'))
                && (!in_word
                    || (words.iter().all(|w| KEYWORDS.contains(&w.as_str()))
                        && !cur_quoted
                        && (KEYWORDS.contains(&cur.as_str())
                            || matches!(cur.as_str(), "for" | "time" | "select" | "foreach")))) =>
            {
                // `(( ... ))` wherever it stands (`time (( ))`, `{ (( )) }`, `for (( ))`, and glued to
                // any keyword: `for((`, `while((`, `then((`, `do((`, `!((`): arithmetic, which evaluates what it reads. A
                // `<<` in it is a shift, never a heredoc: its text is read on its own so nothing after it
                // is taken for a heredoc body; a substitution in it still runs. After a word that is not
                // a keyword or a wrapper it may also be a zsh pattern group, so the command is marked.
                end_word!();
                if !words.iter().all(|w| {
                    KEYWORDS.contains(&w.as_str())
                        || matches!(
                            w.as_str(),
                            "for" | "while" | "until" | "if" | "elif" | "time" | "repeat"
                        )
                }) {
                    group = true;
                }
                end_cmd!();
                let (inner, next) = balanced(&b, i + 2);
                evaluates = true;
                end_cmd!();
                parse(&inner, via, depth + 1, counter, out);
                // A redirect after `))` belongs to the arithmetic.
                after_close = true;
                i = next + 1;
                continue;
            }
            '(' if in_word && cur == "=" => {
                // zsh's `=( ... )`: its commands run, and what they print is a file, as for `<( )`.
                cur.clear();
                let (inner, next) = balanced(&b, i + 1);
                let mut local: Vec<Cmd> = Vec::new();
                parse(&inner, None, depth + 1, counter, &mut local);
                let texts = producer_texts(&local);
                for lc in &local {
                    if let Some(t) = texts.get(&lc.id) {
                        procs.push(t.clone());
                    }
                }
                out.extend(local);
                cur.push_str("<(..)");
                i = next;
                continue;
            }
            '(' if in_word || program_index(&words).is_some() => {
                // An unquoted `(` inside a word, or after the program, starts no subshell: zsh reads it
                // as a pattern group (`x(N)`, `(check|zz)`, `chec(k)`, `(a #b)`), bash's extglob too
                // (`@(x)`), and a pattern may vanish or join its neighbours. The group is part of the
                // word: its text holds no command, no blank ends it and no `#` starts a comment in it.
                // `name()`, `a=(..)` and the lists of `for x (..)`, `select`, `foreach` and `case`
                // patterns stay what they are.
                // An empty `()` (or `( )`) defines functions, glued or not: `name()`, `name () {`,
                // `function name () {`, zsh's `name1 name2 () { }`. Its body is read as commands.
                let empty = balanced(&b, i + 1).0.trim().is_empty();
                let fn_def = empty
                    && ((in_word && (words.is_empty() || words.iter().all(|w| w == "function")))
                        || (!in_word
                            && !words.is_empty()
                            && !words.first().is_some_and(|w| {
                                matches!(w.as_str(), "for" | "foreach" | "select" | "case")
                            })));
                // `name=(` and `name+=(` alone start an array; `/de[=v](..)` is a pattern.
                let array = in_word
                    && words.is_empty()
                    && (cur
                        .strip_suffix("+=")
                        .is_some_and(|k| is_assignment(&format!("{k}=")))
                        || (cur.ends_with('=') && is_assignment(&cur)));
                let list = !in_word
                    && words.first().is_some_and(|w| {
                        matches!(w.as_str(), "for" | "foreach" | "select" | "case")
                    });
                if fn_def && in_word && words.is_empty() && !cur_quoted {
                    // A real definition: an unquoted name in command position.
                    defines.push(cur.clone());
                }
                if fn_def && !in_word && !first_quoted {
                    // `name () {` and zsh's `name1 name2 () {`; `function name ()` is read by the
                    // keyword.
                    if words.first().is_some_and(|w| w != "function") {
                        defines.extend(words.iter().cloned());
                    }
                }
                if fn_def || array || list {
                    if !(fn_def || array) {
                        group = true;
                    }
                    end_cmd!();
                    // Its `)` closes no subshell.
                    sub_depth.push(false);
                } else {
                    group = true;
                    let (inner, next) = balanced(&b, i + 1);
                    // A substitution inside a pattern still runs (`x(|$(cmd))`).
                    for (code, v) in substitutions(&inner) {
                        parse(&code, Some(v), depth + 1, counter, out);
                    }
                    if in_word {
                        // The pattern is the word before the `(`: when that is the program, the
                        // program is a pattern too (`b(f) record put` runs `bf` where a file `bf`
                        // exists).
                        group_word = Some(words.len());
                    }
                    // zsh's `e` glob qualifier runs shell code: `x(e:'cmd':)`. Read fail-closed:
                    // the code of every group in a word, with or without characters before it,
                    // unless its unquoted text holds `|` (an alternation, `src/(a|b)/x`). zsh
                    // decides whether a group is the word's last at run time (an expansion after
                    // it may be empty: `/(e:..:)$E`), and a `~` is special only under
                    // `extended_glob` (measured: both run their code). A group that opens its
                    // word (`(e:'cmd':)`) runs nothing in zsh (measured), and is read anyway.
                    let bare = unquoted_only(&inner);
                    let explicit = bare.trim_start().starts_with("#q");
                    if explicit || !bare.contains('|') {
                        for code in qualifier_code(&inner, depth + 1) {
                            let how = via_text("glob_qualifier");
                            parse(&code, Some(&how), depth + 1, counter, out);
                        }
                    }
                    // The group stays part of the word; the word goes on after it.
                    in_word = in_word || next < n && !" \t\r\n;&|()<>".contains(b[next]);
                    i = next;
                    continue;
                }
            }
            ';' | '|' | '(' | ')' => {
                let ended = cmd_id;
                end_cmd!();
                // A redirect right after the `)` that closes a subshell belongs to it
                // (`(cmd) 2>/dev/null`); the `)` of a `case` pattern closes none.
                if c == '(' {
                    sub_depth.push(true);
                } else if c == ')' {
                    after_close = sub_depth.pop() == Some(true);
                }
                if (c == '|' || c == ';') && i + 1 < n && b[i + 1] == c {
                    i += 1;
                } else if c == '|' {
                    // `a | b`: `b` reads what `a` writes.
                    pipe_in = Some(ended);
                }
            }
            '&' => {
                if i + 1 < n && b[i + 1] == '>' {
                    end_word!();
                    i += 1;
                    while i + 1 < n && b[i + 1] == '>' {
                        i += 1;
                    }
                    redir = Some(true);
                    in_word = false;
                } else {
                    end_cmd!();
                    if i + 1 < n && b[i + 1] == '&' {
                        i += 1;
                    }
                }
            }
            '>' | '<' if i + 1 < n && b[i + 1] == '(' => {
                // A process substitution: its commands run, and what they print is a file for the
                // command it stands in.
                end_word!();
                let (inner, next) = balanced(&b, i + 2);
                let mut local: Vec<Cmd> = Vec::new();
                parse(&inner, None, depth + 1, counter, &mut local);
                if c == '<' {
                    let texts = producer_texts(&local);
                    for lc in &local {
                        if let Some(t) = texts.get(&lc.id) {
                            procs.push(t.clone());
                        }
                    }
                } else {
                    // `cmd > >(sh)`: the commands inside read what this command writes.
                    for lc in local.iter_mut() {
                        if lc.pipe_from.is_none() {
                            lc.pipe_from = Some(cmd_id);
                        }
                    }
                }
                out.extend(local);
                cur.push_str("<(..)");
                in_word = true;
                i = next;
                continue;
            }
            '>' | '<' => {
                // zsh reads `<->` and `<1-9>` as a number range pattern, bash as redirects.
                if number_range(&b, i).is_some() {
                    group = true;
                }
                // A file descriptor before the operator (`2>`), or a variable that gets one (`{fd}>`),
                // belongs to it.
                if in_word
                    && (cur.chars().all(|d| d.is_ascii_digit()) || is_fd_var(&cur))
                    && !cur.is_empty()
                {
                    cur.clear();
                    in_word = false;
                } else {
                    end_word!();
                }
                let op = c;
                if op == '<' {
                    reads_redirect = true;
                }
                let mut count = 1;
                while i + 1 < n && b[i + 1] == op && count < 3 {
                    i += 1;
                    count += 1;
                }
                if op == '<' && count == 2 {
                    // A heredoc: `<<`, `<<-`, `<<'EOF'`, `<<"EOF"`.
                    let mut strip = false;
                    if i + 1 < n && b[i + 1] == '-' {
                        strip = true;
                        i += 1;
                    }
                    while i + 1 < n && (b[i + 1] == ' ' || b[i + 1] == '\t') {
                        i += 1;
                    }
                    let mut delim = String::new();
                    let mut quoted = false;
                    // The delimiter is one shell word: a quoted part keeps its spaces (`<< "E F"`).
                    while i + 1 < n && !" \t\n;|&()<>".contains(b[i + 1]) {
                        i += 1;
                        match b[i] {
                            // `$'E'` and `$"E"` are the delimiter `E`.
                            '$' if matches!(b.get(i + 1), Some('\'' | '"')) => {}
                            // `$(x)` and a backtick stay in the delimiter as written.
                            '$' if b.get(i + 1) == Some(&'(') => {
                                let (inner, next) = balanced(&b, i + 2);
                                delim.push_str("$(");
                                delim.push_str(&inner);
                                delim.push(')');
                                i = next - 1;
                            }
                            '`' => {
                                let (_, j) = backtick(&b, i);
                                let end = j.min(n - 1);
                                delim.extend(b[i..=end].iter());
                                i = end;
                            }
                            q @ ('\'' | '"') => {
                                quoted = true;
                                while i + 1 < n && b[i + 1] != q {
                                    i += 1;
                                    delim.push(b[i]);
                                }
                                if i + 1 < n {
                                    i += 1;
                                }
                            }
                            '\\' => {
                                quoted = true;
                                if i + 1 < n {
                                    i += 1;
                                    delim.push(b[i]);
                                }
                            }
                            ch => delim.push(ch),
                        }
                    }
                    pending.push(Pending {
                        cmd: cmd_id,
                        delim,
                        strip_tabs: strip,
                        expands: !quoted,
                    });
                } else if op == '<' && count == 3 {
                    here_string = true;
                    redir = Some(false);
                } else if i + 1 < n && b[i + 1] == '&' {
                    // `>&2`, `<&3`, `>&-`: a word of digits or `-` alone names a descriptor. Any other
                    // word after it (`>&file`, `>& file`, `>&2x`) is the target: a file for `>&`, never
                    // a word of the command.
                    i += 1;
                    let mut j = i + 1;
                    while j < n && !" \t\r\n;|&()<>".contains(b[j]) {
                        j += 1;
                    }
                    let target: String = b[i + 1..j].iter().collect();
                    if !target.is_empty() && target.chars().all(|d| d.is_ascii_digit() || d == '-')
                    {
                        i = j - 1;
                    } else {
                        redir = Some(op == '>');
                        dup = true;
                    }
                } else {
                    if op == '>' && i + 1 < n && b[i + 1] == '|' {
                        i += 1;
                    }
                    redir = Some(op == '>');
                }
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
        i += 1;
    }
    end_cmd!();
    // Attach heredoc bodies and read the substitutions an expanding body would run.
    let mut extra: Vec<Cmd> = Vec::new();
    for cmd in out.iter_mut() {
        if let Some((body, expands)) = bodies.remove(&cmd.id) {
            if expands {
                for (inner, v) in substitutions(&body) {
                    parse(&inner, Some(v), depth + 1, counter, &mut extra);
                }
                let bc: Vec<char> = body.chars().collect();
                if (0..bc.len()).any(|k| evaluates_at(&bc, k) && (k == 0 || bc[k - 1] != '\\')) {
                    cmd.evaluates = true;
                }
            }
            cmd.heredoc = Some((body, expands));
        }
    }
    // Marked as read from a heredoc body, so nothing in it counts as a plain call or a definition.
    let how = via_text("heredoc_body");
    for body in unclosed {
        parse(&body, Some(&how), depth + 1, counter, &mut extra);
    }
    out.extend(extra);
}

/// What each command writes to a pipe, when it is text a shell could read as a script: the body of a
/// heredoc, or the arguments of `echo` and `printf`.
fn producer_texts(cmds: &[Cmd]) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    for c in cmds {
        let mut text = String::new();
        if let Some((b, _)) = &c.heredoc {
            text.push_str(b);
        }
        if let Some((i, _)) = resolve(&c.words) {
            // zsh's `print` writes its words like `echo`, one on each line with `-l`. `printf` puts its
            // arguments into its format (`printf 'bf unit unbl%s' ock`).
            let prog = basename(&c.words[i]);
            if prog == "printf" {
                let args: Vec<String> = c.words[i + 1..]
                    .iter()
                    .skip_while(|a| a.starts_with('-') && a.len() > 1)
                    .cloned()
                    .collect();
                match args
                    .split_first()
                    .and_then(|(f, rest)| printf_text(f, rest))
                {
                    Some(t) => text.push_str(&t),
                    // Read the words as written, as before; that only reads more.
                    None => text.push_str(&args.join(" ").replace("\\n", "\n")),
                }
            } else if matches!(prog, "echo" | "print") {
                let args: Vec<&str> = c.words[i + 1..]
                    .iter()
                    .filter(|a| !(a.starts_with('-') && a.len() > 1))
                    .map(|s| s.as_str())
                    .collect();
                let lines = prog == "print"
                    && c.words[i + 1..]
                        .iter()
                        .any(|a| a.starts_with('-') && a.contains('l'));
                let sep = if lines { "\n" } else { " " };
                // A newline written as `\n` is a line break to `echo -e`, `print` and `printf`.
                text.push_str(&args.join(sep).replace("\\n", "\n"));
            }
        }
        if !text.is_empty() {
            out.insert(c.id, text);
        }
    }
    out
}

/// The text a shell reads as a script from standard input: a heredoc or here string, what is piped in,
/// or what a process substitution prints.
fn script_text(cmd: &Cmd, texts: &HashMap<usize, String>) -> Option<String> {
    cmd.heredoc
        .as_ref()
        .map(|(b, _)| b.clone())
        .or_else(|| cmd.pipe_from.and_then(|id| texts.get(&id).cloned()))
        .or_else(|| (!cmd.procs.is_empty()).then(|| cmd.procs.join("\n")))
}

/// Does `source` (or `.`) read its script from standard input or a process substitution?
fn reads_input(args: &[String]) -> bool {
    match args
        .iter()
        .find(|a| !a.starts_with('-') || a.as_str() == "-")
    {
        Some(a) => a == "/dev/stdin" || a == "/dev/fd/0" || a == "-" || a.starts_with("<("),
        // `source <(...)` written with the substitution as a redirect: no word follows.
        None => true,
    }
}

/// The command string of `trap 'cmd' SIGNAL ...`; `None` for `trap -p`, `trap -l` and the reset form.
fn trap_command(args: &[String]) -> Option<String> {
    let mut it = args.iter().filter(|a| !a.starts_with('-') || a.len() == 1);
    if args.iter().any(|a| a == "-p" || a == "-l") {
        return None;
    }
    it.next().filter(|s| !s.is_empty()).cloned()
}

/// The starting points of a `find`: the words before its first expression, after the leading options
/// `-H`, `-L`, `-P`, `-D <x>` and `-O<n>`.
pub fn find_starts(args: &[String]) -> Vec<String> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if matches!(a, "-H" | "-L" | "-P") || a.starts_with("-O") {
            i += 1;
        } else if a == "-D" {
            i += 2;
        } else if a.starts_with("-D") {
            i += 1;
        } else {
            break;
        }
    }
    args[i.min(args.len())..]
        .iter()
        .take_while(|a| !(a.starts_with('-') || *a == "(" || *a == "!"))
        .cloned()
        .collect()
}

/// A filter of a `find` that every match must satisfy: `-name`, `-iname`, `-path` and their kin.
pub struct FindFilter {
    /// Matches the whole path (`-path`), not only the last name (`-name`).
    pub path: bool,
    pub pattern: String,
    /// Ignores case (`-iname`, `-ipath`).
    pub fold: bool,
}

/// The filters that bind every match of a `find`. None when the expression has an alternative or a
/// group (`-o`, `(`), where a filter may not bind the action; a negated filter does not count.
pub fn find_filters(args: &[String]) -> Vec<FindFilter> {
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "-o" | "-or" | "(" | "\\("))
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut negated = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "!" | "-not" => {
                negated = true;
                i += 1;
                continue;
            }
            "-name" | "-iname" | "-path" | "-ipath" | "-wholename" | "-iwholename" => {
                if let (false, Some(v)) = (negated, args.get(i + 1)) {
                    out.push(FindFilter {
                        path: a.contains("path") || a.contains("wholename"),
                        pattern: v.clone(),
                        fold: a.starts_with("-i"),
                    });
                }
                i += 2;
            }
            "-exec" | "-execdir" | "-ok" | "-okdir" => {
                while i < args.len() && args[i] != ";" && args[i] != "+" {
                    i += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
        negated = false;
    }
    out
}

/// Like `exec_commands`, with `{}` read as what a `find` matches, so that what the command would touch
/// is a path the guard can judge: with a name filter the starting point with the name (`./*.orig`),
/// otherwise the starting point itself.
fn exec_commands_at(args: &[String]) -> Vec<Vec<String>> {
    let starts: Vec<String> = {
        let s = find_starts(args);
        if s.is_empty() {
            vec![".".to_string()]
        } else {
            s
        }
    };
    let names: Vec<String> = find_filters(args)
        .into_iter()
        .filter(|f| !f.path)
        .map(|f| f.pattern)
        .collect();
    let mut stands: Vec<String> = Vec::new();
    for s in &starts {
        if names.is_empty() {
            stands.push(s.clone());
        } else {
            for n in &names {
                stands.push(format!("{}/{}", s.trim_end_matches('/'), n));
            }
        }
    }
    let mut out = Vec::new();
    for words in exec_commands(args) {
        if words.iter().any(|w| w.contains("{}")) {
            for s in &stands {
                out.push(words.iter().map(|w| w.replace("{}", s)).collect());
            }
        } else {
            out.push(words);
        }
    }
    out
}

/// The commands a `find` runs: the words between an execute flag and its closing `;` or `+`.
pub fn exec_commands(args: &[String]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if matches!(args[i].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            let mut j = i + 1;
            let mut words = Vec::new();
            while j < args.len() && args[j] != ";" && args[j] != "+" {
                words.push(args[j].clone());
                j += 1;
            }
            if !words.is_empty() {
                out.push(words);
            }
            i = j;
        }
        i += 1;
    }
    out
}

/// The text of the `$( )` and backtick substitutions in a body.
fn substitutions(body: &str) -> Vec<(String, &'static str)> {
    let b: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\\' {
            i += 2;
            continue;
        }
        if b[i] == '$' && i + 1 < b.len() && b[i + 1] == '(' {
            let (inner, next) = balanced(&b, i + 2);
            out.push((inner, "$("));
            i = next;
            continue;
        }
        // bash 5.3's `${ cmd; }`.
        if b[i] == '$'
            && b.get(i + 1) == Some(&'{')
            && matches!(b.get(i + 2), Some(' ' | '\t' | '\n' | '|'))
        {
            let (inner, next) = braced(&b, i + 2);
            out.push((inner.strip_prefix('|').unwrap_or(&inner).to_string(), "$("));
            i = next;
            continue;
        }
        if b[i] == '`' {
            let (inner, j) = backtick(&b, i);
            out.push((inner, "`"));
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// The command inside the backticks that open at `b[i]`, and the index of the closing backtick (or the
/// end). Inside, `` \` ``, `\\` and `\$` stand for the character itself, so `` `echo \`bf x\`` `` holds
/// the nested substitution `` `bf x` ``.
fn backtick(b: &[char], i: usize) -> (String, usize) {
    let mut j = i + 1;
    let mut inner = String::new();
    while j < b.len() && b[j] != '`' {
        if b[j] == '\\' && j + 1 < b.len() {
            if matches!(b[j + 1], '`' | '\\' | '$') {
                inner.push(b[j + 1]);
            } else {
                inner.push('\\');
                inner.push(b[j + 1]);
            }
            j += 2;
            continue;
        }
        inner.push(b[j]);
        j += 1;
    }
    (inner, j)
}

/// The text up to the `}` that closes a `${ `, and the index after it. Quotes and escapes are skipped.
fn braced(b: &[char], start: usize) -> (String, usize) {
    let mut depth = 1;
    let mut i = start;
    let mut s = String::new();
    while i < b.len() {
        match b[i] {
            '\\' if i + 1 < b.len() => {
                s.push('\\');
                s.push(b[i + 1]);
                i += 1;
            }
            q @ ('\'' | '"') => {
                s.push(q);
                i += 1;
                while i < b.len() && b[i] != q {
                    s.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    s.push(q);
                }
            }
            '{' => {
                depth += 1;
                s.push('{');
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (s, i + 1);
                }
                s.push('}');
            }
            c => s.push(c),
        }
        i += 1;
    }
    (s, i)
}

/// The text up to the `)` that closes a `$(`, and the index after it. Quotes are skipped.
fn balanced(b: &[char], start: usize) -> (String, usize) {
    let mut depth = 1;
    let mut i = start;
    let mut s = String::new();
    while i < b.len() {
        match b[i] {
            // An escaped character closes nothing and opens nothing (`\)`, `\'`).
            '\\' if i + 1 < b.len() => {
                s.push('\\');
                s.push(b[i + 1]);
                i += 1;
            }
            '\'' => {
                s.push('\'');
                i += 1;
                while i < b.len() && b[i] != '\'' {
                    s.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    s.push('\'');
                }
            }
            '"' => {
                s.push('"');
                i += 1;
                while i < b.len() && b[i] != '"' {
                    if b[i] == '\\' && i + 1 < b.len() {
                        s.push(b[i]);
                        i += 1;
                    }
                    s.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    s.push('"');
                }
            }
            '(' => {
                depth += 1;
                s.push('(');
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (s, i + 1);
                }
                s.push(')');
            }
            c => s.push(c),
        }
        i += 1;
    }
    (s, i)
}
