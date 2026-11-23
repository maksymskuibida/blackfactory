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
    /// The group touched the last word (`x(N)`): that word is a pattern.
    group_last: bool,
    /// The command line evaluates text as code at run time here: arithmetic (`$(( ))`, `(( ))`,
    /// `$[ ]`), a zsh parameter flag (`${(e)V}`) or a bash parameter operator (`${V@P}`).
    evaluates: bool,
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
            group_last: false,
            evaluates: false,
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

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];
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
    calls
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
            _ => {}
        }
    }
    // A program word that holds a whole command line is a line some wrapper hands to a shell.
    if let Some(i) = program_index(words) {
        let p = &words[i];
        if basename(p) != "bf" && p.contains([' ', '\t', '\n', ';', '|', '&']) {
            out.push((words[i..].join(" "), "a quoted command".to_string()));
        }
    }
    out
}

/// The words `cargo run` hands to the program it builds, when `words` is such a call.
fn cargo_run_args(words: &[String]) -> Option<Vec<String>> {
    let i = program_index(words)?;
    if basename(&words[i]) != "cargo" || !matches!(words.get(i + 1)?.as_str(), "run" | "r") {
        return None;
    }
    let rest = &words[i + 2..];
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
/// `for`, `select`, `foreach`, `coproc` or `always`. (A quoted `{` is split too; that only reads more.)
fn split_short_forms(words: Vec<String>) -> Vec<Vec<String>> {
    let mut parts = Vec::new();
    let mut rest = words;
    loop {
        let starts_kw = rest.first().is_some_and(|w| {
            KEYWORDS.contains(&w.as_str())
                || matches!(
                    w.as_str(),
                    "for" | "select" | "foreach" | "coproc" | "always"
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
            j += if matches!(w, "--rcfile" | "--init-file") {
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

/// Does the `$` at `b[i]` start an expansion that evaluates text as code: arithmetic (`$((`, `$[`),
/// a zsh parameter flag (`${(e)V}`), a bash parameter operator (`${V@P}`) or an array subscript, which
/// bash reads as arithmetic (`${a[V]}`)?
fn evaluates_at(b: &[char], i: usize) -> bool {
    if b.get(i) != Some(&'$') {
        return false;
    }
    match (b.get(i + 1), b.get(i + 2)) {
        (Some('('), Some('(')) | (Some('['), _) | (Some('{'), Some('(')) => true,
        (Some('{'), _) => b[i + 2..]
            .iter()
            .take_while(|c| **c != '}')
            .any(|c| matches!(c, '@' | '[')),
        _ => false,
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

/// The shell code of the zsh `e` glob qualifiers in the text of a `( )` group (`e:'cmd':`, `e{cmd}`),
/// with quotes removed. The text is read loosely: any `e` followed by a delimiter counts.
fn qualifier_code(group: &str, depth: usize) -> Vec<String> {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(group, None, depth, &mut counter, &mut cmds);
    let plain: Vec<char> = cmds
        .iter()
        .flat_map(|c| c.words.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k + 1 < plain.len() {
        let d = plain[k + 1];
        if plain[k] == 'e' && !d.is_alphanumeric() && !d.is_whitespace() && d != ')' {
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
            if !code.trim().is_empty() {
                out.push(code);
            }
            k = j;
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
        });
        // A quoted program word with spaces is read for calls of the engine only: as a path it is still
        // the one program the path rules judge.
        for (line, _) in handed_lines(&cmd.words)
            .into_iter()
            .filter(|(_, how)| how != "a quoted command")
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
        // A program that is a pattern or a brace group may become `bf` (`b?`, `b[f]`, `b(f)` where a
        // file `bf` exists, `{bf,x}`): it is a call that cannot be read.
        let pattern_prog = (cmd.group_last && i + 1 == cmd.words.len())
            || (prog.contains(['*', '?', '[']) && crate::guard::pattern_matches(&prog, "bf"))
            || (prog.contains('{') && (prog.contains(',') || prog.contains("..")));
        if pattern_prog && prog != "bf" {
            let mut words = vec!["bf".to_string()];
            words.extend(cmd.words[i + 1..].iter().cloned());
            calls.push(Call {
                words,
                via: Some("a pattern".to_string()),
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
                basename(v) == "bf" || v.contains('$') || v.contains('`')
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
        }
        if prog == "bf" {
            let words: Vec<String> = cmd.words[i..].to_vec();
            // `bf evidence run <check> ... -- <command>`: the command after `--` is a command line of
            // its own, read by the same rule.
            if words.get(1).map(|s| s.as_str()) == Some("evidence")
                && words.get(2).map(|s| s.as_str()) == Some("run")
            {
                if let Some(d) = words.iter().position(|w| w == "--") {
                    let rest: Vec<String> = words[d + 1..].to_vec();
                    if !rest.is_empty() {
                        let inner = vec![Cmd::of(rest)];
                        collect(inner, None, depth + 1, calls);
                    }
                }
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
    let mut group_last = false;
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
                    None => words.push(w),
                }
                cur.clear();
                in_word = false;
                here_string = false;
                dup = false;
            }
        };
    }
    macro_rules! end_cmd {
        () => {
            end_word!();
            redir = None;
            dup = false;
            if !words.is_empty() || !redirs.is_empty() || evaluates {
                // zsh's short forms run a command in the same simple command: `if [[ x ]] cmd`,
                // `while (( x )) { cmd }`, `coproc NAME { cmd; }`.
                let mut parts = split_short_forms(std::mem::take(&mut words));
                words = parts.pop().unwrap_or_default();
                for p in parts {
                    out.push(Cmd {
                        id: *counter,
                        via: via.map(|v| v.to_string()),
                        group,
                        ..Cmd::of(p)
                    });
                    *counter += 1;
                }
                out.push(Cmd {
                    id: cmd_id,
                    words: std::mem::take(&mut words),
                    via: via.map(|v| v.to_string()),
                    heredoc: None,
                    redirs: std::mem::take(&mut redirs),
                    pipe_from: pipe_in.take(),
                    procs: std::mem::take(&mut procs),
                    group,
                    group_last,
                    evaluates,
                });
            }
            group = false;
            group_last = false;
            evaluates = false;
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
                    }
                    i += 1;
                }
            }
            '\'' => {
                in_word = true;
                i += 1;
                while i < n && b[i] != '\'' {
                    cur.push(b[i]);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
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
                // `$'...'`: a quote in which a backslash escapes the next character (`$'\''`). Its text
                // is the word's when it holds no backslash; otherwise the `$` stays in front of it, so
                // the word counts as one the parse does not read (`$'\x72'` is `r`).
                in_word = true;
                i += 2;
                let mut text = String::new();
                let mut escaped = false;
                while i < n && b[i] != '\'' {
                    if b[i] == '\\' && i + 1 < n {
                        escaped = true;
                        text.push(b[i + 1]);
                        i += 2;
                        continue;
                    }
                    text.push(b[i]);
                    i += 1;
                }
                if escaped {
                    cur.push('$');
                }
                cur.push_str(&text);
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
            '#' if !in_word => {
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
                            break;
                        }
                        body.push_str(&line);
                        body.push('\n');
                        i += 1;
                    }
                    bodies.insert(p.cmd, (body, p.expands));
                    // `i` is at the newline that ends the delimiter line (or the end); the loop's own
                    // increment steps past it.
                }
            }
            ' ' | '\t' | '\r' => {
                end_word!();
            }
            '(' if !in_word
                && words
                    .iter()
                    .all(|w| KEYWORDS.contains(&w.as_str()) || w == "for")
                && b.get(i + 1) == Some(&'(')
                && matches!(balanced(&b, i + 2), (_, next) if b.get(next) == Some(&')')) =>
            {
                // `(( ... ))` and `for (( ... ))`: arithmetic, which evaluates what it reads. A `<<` in it
                // is a shift, never a heredoc: its text is read on its own so nothing after it is taken
                // for a heredoc body; a substitution in it still runs.
                end_cmd!();
                let (inner, next) = balanced(&b, i + 2);
                evaluates = true;
                end_cmd!();
                parse(&inner, via, depth + 1, counter, out);
                i = next + 1;
                continue;
            }
            '(' if in_word || !words.iter().all(|w| KEYWORDS.contains(&w.as_str())) => {
                // An unquoted `(` after the first word, or inside a word, starts no subshell: zsh reads it
                // as a pattern group (`x(N)`, `(check|zz)`, `chec(k)`), bash's extglob too (`@(x)`), and a
                // pattern may vanish or join its neighbours. `name()` and `a=(..)` stay what they are.
                let fn_def = in_word
                    && (words.is_empty() || words.iter().all(|w| w == "function"))
                    && b.get(i + 1) == Some(&')');
                let array = in_word && words.is_empty() && cur.contains('=');
                if !(fn_def || array) {
                    group = true;
                }
                if !(fn_def || array) && in_word {
                    // The pattern is the word before the `(`: when that is the program, the program
                    // is a pattern too (`b(f) record put` runs `bf` where a file `bf` exists).
                    group_last = true;
                    // zsh's `e` glob qualifier, after a pattern, runs shell code: `x(e:'cmd':)`.
                    let (inner, _) = balanced(&b, i + 1);
                    for code in qualifier_code(&inner, depth + 1) {
                        parse(&code, Some("a glob qualifier"), depth + 1, counter, out);
                    }
                }
                end_cmd!();
            }
            ';' | '|' | '(' | ')' => {
                let ended = cmd_id;
                end_cmd!();
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
                    // `>&2`, `<&3`, `>&-`: the target is part of the operator. Any other word after it
                    // (`>&file`, `>& file`) is the target: a file for `>&`, never a word of the command.
                    i += 1;
                    if i + 1 < n && (b[i + 1].is_ascii_digit() || b[i + 1] == '-') {
                        while i + 1 < n && (b[i + 1].is_ascii_digit() || b[i + 1] == '-') {
                            i += 1;
                        }
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
            if matches!(basename(&c.words[i]), "echo" | "printf") {
                let args: Vec<&str> = c.words[i + 1..]
                    .iter()
                    .filter(|a| !(a.starts_with('-') && a.len() > 1))
                    .map(|s| s.as_str())
                    .collect();
                // A newline written as `\n` is a line break to `echo -e` and `printf`.
                text.push_str(&args.join(" ").replace("\\n", "\n"));
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
