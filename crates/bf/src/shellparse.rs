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
}

struct Pending {
    cmd: usize,
    delim: String,
    strip_tabs: bool,
    expands: bool,
}

fn basename(w: &str) -> &str {
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
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    let mut calls = Vec::new();
    collect(cmds, None, 0, &mut calls);
    calls
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

/// Does an interpreter (or `cargo`) get an argument that holds `phrase` (`python3 -c \"... bf human-turn ...\"`)?
/// The body of a heredoc is not an argument.
pub fn interpreter_arg_with_phrase(cmd: &str, phrase: &str) -> bool {
    let mut counter = 0usize;
    let mut cmds = Vec::new();
    parse(cmd, None, 0, &mut counter, &mut cmds);
    cmds.iter().any(|c| {
        resolve(&c.words)
            .map(|(i, _)| {
                matches!(
                    basename(&c.words[i]),
                    "cargo"
                        | "python"
                        | "python3"
                        | "perl"
                        | "node"
                        | "ruby"
                        | "sh"
                        | "bash"
                        | "zsh"
                ) && c.words[i + 1..].iter().any(|w| w.contains(phrase))
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
    /// The shell the command runs in: 0 for the command line itself, a new number for each `sh -c`
    /// string and each script fed to a shell. A `cd` holds only inside its own shell.
    pub scope: usize,
    /// The shell that started this one (its working directory is the starting point).
    pub parent: usize,
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
            scope,
            parent,
        });
        let Some(i) = idx else { continue };
        let prog = basename(&cmd.words[i]);
        if prog == "find" {
            // The command after an execute flag is a command of its own.
            for words in exec_commands(&cmd.words[i + 1..]) {
                let inner = vec![Cmd {
                    id: 0,
                    words,
                    via: None,
                    heredoc: None,
                    redirs: vec![],
                    pipe_from: None,
                }];
                flatten(inner, scope, parent, next_scope, depth + 1, out);
            }
        }
        if SHELLS.contains(&prog) {
            let mut j = i + 1;
            let mut found = false;
            while j < cmd.words.len() && cmd.words[j].starts_with('-') {
                if cmd.words[j].len() > 1 && cmd.words[j][1..].contains('c') {
                    if let Some(s) = cmd.words.get(j + 1) {
                        let mut c2 = 0usize;
                        let mut inner = Vec::new();
                        parse(s, None, depth + 1, &mut c2, &mut inner);
                        let id = *next_scope;
                        *next_scope += 1;
                        flatten(inner, id, scope, next_scope, depth + 1, out);
                        found = true;
                    }
                    break;
                }
                j += 1;
            }
            if !found {
                let script = cmd
                    .heredoc
                    .as_ref()
                    .map(|(b, _)| b.clone())
                    .or_else(|| cmd.pipe_from.and_then(|id| texts.get(&id).cloned()));
                if let Some(body) = &script {
                    let mut c2 = 0usize;
                    let mut inner = Vec::new();
                    parse(body, None, depth + 1, &mut c2, &mut inner);
                    let id = *next_scope;
                    *next_scope += 1;
                    flatten(inner, id, scope, next_scope, depth + 1, out);
                }
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
        let Some((i, wrap)) = resolve(&cmd.words) else {
            continue;
        };
        if basename(&cmd.words[i]) == "find" {
            for words in exec_commands(&cmd.words[i + 1..]) {
                let inner = vec![Cmd {
                    id: 0,
                    words,
                    via: None,
                    heredoc: None,
                    redirs: vec![],
                    pipe_from: None,
                }];
                collect(inner, Some("find -exec"), depth + 1, calls);
            }
        }
        let eff: Option<String> = cmd
            .via
            .clone()
            .or_else(|| via.map(|v| v.to_string()))
            .or(wrap);
        let prog = basename(&cmd.words[i]).to_string();
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
                        let inner = vec![Cmd {
                            id: 0,
                            words: rest,
                            via: None,
                            heredoc: None,
                            redirs: vec![],
                            pipe_from: None,
                        }];
                        collect(inner, None, depth + 1, calls);
                    }
                }
            }
            calls.push(Call { words, via: eff });
        } else if SHELLS.contains(&prog.as_str()) {
            // `sh -c '<string>'`: the string is a command line.
            let mut j = i + 1;
            let mut found = false;
            while j < cmd.words.len() && cmd.words[j].starts_with('-') {
                if cmd.words[j].len() > 1 && cmd.words[j][1..].contains('c') {
                    if let Some(s) = cmd.words.get(j + 1) {
                        let mut c2 = 0usize;
                        let mut inner = Vec::new();
                        parse(s, None, depth + 1, &mut c2, &mut inner);
                        collect(inner, Some("sh -c"), depth + 1, calls);
                        found = true;
                    }
                    break;
                }
                j += 1;
            }
            if !found {
                // A heredoc or a pipe fed to a shell is a script: its lines are commands.
                let script = cmd
                    .heredoc
                    .as_ref()
                    .map(|(b, _)| b.clone())
                    .or_else(|| cmd.pipe_from.and_then(|id| texts.get(&id).cloned()));
                if let Some(body) = &script {
                    let mut c2 = 0usize;
                    let mut inner = Vec::new();
                    parse(body, None, depth + 1, &mut c2, &mut inner);
                    collect(inner, None, depth + 1, calls);
                }
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
    let mut cmd_id = *counter;
    *counter += 1;
    let mut bodies: HashMap<usize, (String, bool)> = HashMap::new();

    macro_rules! end_word {
        () => {
            if in_word {
                let w = std::mem::take(&mut cur);
                match redir.take() {
                    Some(true) => redirs.push(w),
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
            }
        };
    }
    macro_rules! end_cmd {
        () => {
            end_word!();
            redir = None;
            if !words.is_empty() || !redirs.is_empty() {
                out.push(Cmd {
                    id: cmd_id,
                    words: std::mem::take(&mut words),
                    via: via.map(|v| v.to_string()),
                    heredoc: None,
                    redirs: std::mem::take(&mut redirs),
                    pipe_from: pipe_in.take(),
                });
            }
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
                    if b[i] == '$' && i + 1 < n && b[i + 1] == '(' {
                        let (inner, next) = balanced(&b, i + 2);
                        cur.push_str("$(..)");
                        parse(&inner, Some("$("), depth + 1, counter, out);
                        i = next;
                        continue;
                    }
                    if b[i] == '`' {
                        let mut j = i + 1;
                        let mut inner = String::new();
                        while j < n && b[j] != '`' {
                            inner.push(b[j]);
                            j += 1;
                        }
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
                let (inner, next) = balanced(&b, i + 2);
                cur.push_str("$(..)");
                parse(&inner, Some("$("), depth + 1, counter, out);
                i = next;
                continue;
            }
            '$' => {
                in_word = true;
                cur.push('$');
                if i + 1 < n && b[i + 1] == '{' {
                    while i + 1 < n && b[i] != '}' {
                        i += 1;
                        cur.push(b[i]);
                    }
                }
            }
            '`' => {
                in_word = true;
                let mut j = i + 1;
                let mut inner = String::new();
                while j < n && b[j] != '`' {
                    inner.push(b[j]);
                    j += 1;
                }
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
            '>' | '<' => {
                // A file descriptor before the operator (`2>`) belongs to it.
                if in_word && cur.chars().all(|d| d.is_ascii_digit()) && !cur.is_empty() {
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
                    while i + 1 < n && !" \t\n;|&()<>".contains(b[i + 1]) {
                        i += 1;
                        match b[i] {
                            '\'' | '"' => quoted = true,
                            '\\' => quoted = true,
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
                    // `>&2`, `<&3`, `>&-`: the target is part of the operator.
                    i += 1;
                    while i + 1 < n && (b[i + 1].is_ascii_digit() || b[i + 1] == '-') {
                        i += 1;
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

/// The commands a `find` runs: the words between an execute flag and its closing `;` or `+`.
fn exec_commands(args: &[String]) -> Vec<Vec<String>> {
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
            let mut j = i + 1;
            let mut inner = String::new();
            while j < b.len() && b[j] != '`' {
                inner.push(b[j]);
                j += 1;
            }
            out.push((inner, "`"));
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// The text up to the `)` that closes a `$(`, and the index after it. Quotes are skipped.
fn balanced(b: &[char], start: usize) -> (String, usize) {
    let mut depth = 1;
    let mut i = start;
    let mut s = String::new();
    while i < b.len() {
        match b[i] {
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
