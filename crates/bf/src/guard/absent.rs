//! The guard's rules of the human-absent flag (R-74, D-96): while the run's flag is on, the tools that
//! ask the human are refused with the step to take in their place; and, flag or not, the hook commands
//! and the harness's own program are kept from every agent, in the shell and in the listed tools that
//! run a command line or type (R-72 acceptance 4, finding F-220).
//!
//! The tool lists are adapter data (`harness/claude/adapter.toml`); the policy adds to the command and
//! input lists, and names the tools refused under the flag, by name or by prefix. A tool that no list
//! names passes unseen: D-94 states it as a limit.

use super::{deny, Decision};
use crate::adapter::{ToolField, DATA};
use crate::strings::t;
use serde_json::Value;
use std::collections::HashMap;

/// Does the tool name match a policy entry, by name or by prefix (a trailing `*` is dropped)?
fn named_by(entry: &str, tool: &str) -> bool {
    let e = entry.trim().trim_end_matches('*');
    !e.is_empty() && tool.starts_with(e)
}

fn policy_list(v: &Value) -> impl Iterator<Item = &str> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
}

/// The field of a tool in an adapter list, when the tool is one of its entries.
fn adapter_entry<'a>(list: &'a [ToolField], tool: &str) -> Option<&'a ToolField> {
    list.iter().find(|e| e.tool == tool)
}

/// Every string in a value, keys left out.
fn texts_of(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| texts_of(x, out)),
        Value::Object(m) => m.values().for_each(|x| texts_of(x, out)),
        _ => {}
    }
}

/// The texts a tool's input carries in a field, or every text of the input when the field is empty.
fn input_texts(ti: &Value, field: &str) -> Vec<String> {
    let mut out = Vec::new();
    if field.is_empty() {
        texts_of(ti, &mut out);
    } else if let Some(v) = ti.get(field) {
        texts_of(v, &mut out);
    }
    out
}

/// How a tool is listed for a rule: by the adapter with its field, or by the policy (every text).
fn listed(adapter: &[ToolField], policy: &Value, tool: &str) -> Option<String> {
    if let Some(e) = adapter_entry(adapter, tool) {
        return Some(e.field.to_string());
    }
    policy_list(policy)
        .any(|e| named_by(e, tool))
        .then(String::new)
}

fn step_lines(key: &str) -> Vec<String> {
    t(key, &[])
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn denial(group: &str, tool: &str) -> Decision {
    let v = [("tool", tool)];
    let steps = step_lines(&format!("guard_absent.{group}.steps"));
    let steps: Vec<&str> = steps.iter().map(|s| s.as_str()).collect();
    deny(
        &t(&format!("guard_absent.{group}.refused"), &v),
        &t(&format!("guard_absent.{group}.because"), &v),
        &steps,
        &t(&format!("guard_absent.{group}.worked"), &v),
    )
}

/// The release question of U-67's key, as the engine asks it.
fn release_question() -> String {
    t(
        "absent.release_question",
        &[
            ("yes", &t("absent.answer_yes", &[])),
            ("stay", &t("absent.answer_stay", &[])),
        ],
    )
}

fn label_of(v: &Value) -> Option<String> {
    v.as_str()
        .or_else(|| v.get("label").and_then(|l| l.as_str()))
        .map(|s| s.trim().to_lowercase())
}

/// Is the call exactly the release question, with the yes answer and at most the stay-absent answer?
/// The ask menu holds one question with its options; the next-step box holds the question in its field
/// and the answers in `actions`.
fn is_release_question(ti: &Value, field: &str) -> bool {
    let (question, answers): (String, Vec<Value>) =
        match ti.get("questions").and_then(|q| q.as_array()) {
            Some(qs) => {
                if qs.len() != 1 {
                    return false;
                }
                (
                    qs[0]["question"].as_str().unwrap_or("").to_string(),
                    qs[0]["options"].as_array().cloned().unwrap_or_default(),
                )
            }
            None => {
                if field.is_empty() {
                    return false;
                }
                (
                    ti.get(field)
                        .and_then(|q| q.as_str())
                        .unwrap_or("")
                        .to_string(),
                    ti["actions"].as_array().cloned().unwrap_or_default(),
                )
            }
        };
    if question.trim() != release_question() {
        return false;
    }
    let yes = t("absent.answer_yes", &[]).to_lowercase();
    let stay = t("absent.answer_stay", &[]).to_lowercase();
    let labels: Vec<Option<String>> = answers.iter().map(label_of).collect();
    labels.iter().any(|l| l.as_deref() == Some(yes.as_str()))
        && labels
            .iter()
            .all(|l| l.as_deref() == Some(yes.as_str()) || l.as_deref() == Some(stay.as_str()))
}

/// Does the question tool's input carry a question? A tool with a field asks only when the field is set.
fn asks(ti: &Value, field: &str) -> bool {
    if field.is_empty() {
        return true;
    }
    match ti.get(field) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
        Some(_) => true,
    }
}

/// The rules of the tool hook for the tools of the adapter's and the policy's lists. `None` lets the
/// call go on to the rules every tool has.
pub(super) fn judge(input: &Value, table: &Value) -> Option<Decision> {
    let tool = input["tool_name"].as_str().unwrap_or("");
    let ti = &input["tool_input"];
    if tool.is_empty() || tool == "Bash" {
        return None;
    }
    let entry = &table["absent"];
    if entry["on"].as_bool() == Some(true) {
        let session = input["session_id"].as_str().unwrap_or("");
        if let Some(e) = adapter_entry(DATA.question_tools, tool) {
            if asks(ti, e.field) {
                let returning =
                    !session.is_empty() && policy_list(&entry["returning"]).any(|s| s == session);
                if returning && is_release_question(ti, e.field) {
                    return None;
                }
                return Some(denial("question", tool));
            }
        }
        if DATA.plan_tools.contains(&tool) {
            return Some(denial("plan", tool));
        }
        if DATA.access_tools.contains(&tool) {
            return Some(denial("access", tool));
        }
        if policy_list(&entry["deny_tools"]).any(|e| named_by(e, tool)) {
            return Some(denial("deny_tools", tool));
        }
    }
    if let Some(field) = listed(DATA.command_tools, &table["command_tools"], tool) {
        for text in input_texts(ti, &field) {
            if let Some(d) = command_text_denial(&text) {
                return Some(d);
            }
        }
    }
    if let Some(field) = listed(DATA.input_tools, &table["input_tools"], tool) {
        for text in input_texts(ti, &field) {
            let low = text.to_lowercase();
            if DATA
                .absent_commands
                .iter()
                .any(|c| low.contains(&c.to_lowercase()))
            {
                return Some(denial("typed", tool));
            }
        }
    }
    None
}

/// The text of a command tool, judged as a shell command line and as code handed to an interpreter.
fn command_text_denial(text: &str) -> Option<Decision> {
    if shell_calls_human_turn(text) || super::code_calls_human_turn(text) {
        return Some(super::human_turn_denial());
    }
    if let Some(d) = shell_denial(text).or_else(|| code_denial(text)) {
        return Some(d);
    }
    None
}

/// Does a shell command line call the command that records a human message? The same readings as the
/// shell tool's.
fn shell_calls_human_turn(cmd: &str) -> bool {
    use crate::shellparse as sp;
    if sp::runtime_program_with_word(cmd, "human-turn")
        || sp::interpreter_arg_with_phrase(cmd, "bf human-turn")
    {
        return true;
    }
    if sp::bf_calls(cmd)
        .iter()
        .any(|c| super::bf_rest(&c.words).first().map(|s| s.as_str()) == Some("human-turn"))
    {
        return true;
    }
    let parsed = sp::commands(cmd);
    if parsed
        .iter()
        .any(|p| super::names_human_turn_call(&p.words))
    {
        return true;
    }
    parsed.iter().any(|p| {
        sp::program_index(&p.words).is_some_and(|i| {
            let w = &p.words[i..];
            super::basename(&w[0]) == "cargo"
                && w.get(1).is_some_and(|a| a == "run" || a == "r")
                && w.iter().skip(2).any(|a| a == "human-turn")
        }) || sp::cargo_run_args(&p.words)
            .is_some_and(|args| args.iter().any(|a| a == "human-turn"))
    }) || sp::engine_calls(cmd)
        .iter()
        .any(|c| c.cargo && c.words.iter().any(|w| w == "human-turn"))
}

/// Is a program word one of the harness's own programs (`claude`)? Case is ignored, as for the engine's
/// name, since the default macOS volume opens `/usr/local/bin/Claude` as `claude`.
fn is_harness_program(w: &str) -> bool {
    let b = super::basename(w.strip_prefix('=').unwrap_or(w));
    DATA.programs.iter().any(|p| b.eq_ignore_ascii_case(p)) || is_harness_package(w)
}

/// Does the word name the harness's package, with or without a version (`pkg@latest`)?
fn is_harness_package(w: &str) -> bool {
    DATA.packages
        .iter()
        .any(|p| w == *p || w.strip_prefix(p).is_some_and(|r| r.starts_with('@')))
}

/// Does a word of a script path run inside the harness's package (`.../node_modules/pkg/cli.js`, `$(npm root -g)/pkg/cli.js`)?
fn in_harness_package(w: &str) -> bool {
    DATA.packages.iter().any(|p| w.contains(&format!("/{p}/")))
}

/// The shell lines an `osascript` call hands to a shell: the quoted text after `do shell script`.
fn osascript_lines(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        let mut rest = a.as_str();
        while let Some(i) = rest.find("do shell script") {
            rest = &rest[i + "do shell script".len()..];
            let r = rest.trim_start();
            if let Some(q) = r.strip_prefix('"') {
                let mut line = String::new();
                let mut it = q.chars();
                while let Some(c) = it.next() {
                    match c {
                        '\\' => line.extend(it.next()),
                        '"' => break,
                        c => line.push(c),
                    }
                }
                out.push(line);
            }
        }
    }
    out
}

/// Programs that run the program they are given: the first plain word after their own options.
const RUNNERS: &[&str] = &["npx", "bunx", "pnpx", "xargs", "nohup", "doas"];
/// Package tools whose `exec` or `dlx` runs a program.
const PACKAGE_RUNNERS: &[&str] = &["npm", "pnpm", "yarn", "bun"];

fn first_plain(args: &[String]) -> Option<&String> {
    args.iter().find(|a| !a.starts_with('-'))
}

/// Does one simple command start the harness's own program?
fn starts_harness(words: &[String], vars: &[HashMap<String, String>]) -> bool {
    let Some(i) = crate::shellparse::program_index(words) else {
        return false;
    };
    let prog = &words[i];
    let args = &words[i + 1..];
    let named = |w: &str| {
        is_harness_program(w)
            || (w.contains('$')
                && vars
                    .iter()
                    .any(|v| is_harness_program(&super::subst_vars(w, v))))
    };
    if named(prog) {
        return true;
    }
    let base = super::basename(prog);
    if matches!(base, "node" | "nodejs" | "bun" | "deno")
        && args.iter().any(|a| in_harness_package(a))
    {
        return true;
    }
    if base == "osascript" {
        let hit = osascript_lines(args).iter().any(|l| {
            let parsed = crate::shellparse::commands(l);
            let v = super::var_readings(&super::assigned_vars(&parsed));
            parsed.iter().any(|p| starts_harness(&p.words, &v))
        });
        if hit {
            return true;
        }
    }
    if RUNNERS.contains(&base) && first_plain(args).is_some_and(|a| named(a)) {
        return true;
    }
    if PACKAGE_RUNNERS.contains(&base) {
        if let Some(k) = args
            .iter()
            .position(|a| matches!(a.as_str(), "exec" | "dlx" | "x"))
        {
            if first_plain(&args[k + 1..]).is_some_and(|a| named(a)) {
                return true;
            }
        }
    }
    if base == "find" {
        return crate::shellparse::exec_commands(args)
            .iter()
            .any(|c| starts_harness(c, vars));
    }
    false
}

/// A shell command that runs the harness's own program or the engine's `hook` command is refused to
/// every agent and session, flag or not.
pub(super) fn shell_denial(cmd: &str) -> Option<Decision> {
    use crate::shellparse as sp;
    let parsed = sp::commands(cmd);
    let vars = super::var_readings(&super::assigned_vars(&parsed));
    if parsed.iter().any(|p| starts_harness(&p.words, &vars)) {
        return Some(denial("harness", ""));
    }
    if sp::runtime_program_with_word(cmd, "hook")
        || sp::interpreter_arg_with_phrase(cmd, "bf hook")
        || sp::bf_calls(cmd)
            .iter()
            .any(|c| super::bf_rest(&c.words).first().map(|s| s.as_str()) == Some("hook"))
        || parsed
            .iter()
            .any(|p| super::names_bf_word_call(&p.words, "hook"))
    {
        return Some(denial("hook", ""));
    }
    None
}

/// Does a string literal of the code start with the harness's program? A program stands first in an
/// argument list (`["claude", "-p"]`) or first in the line a shell is handed (`"claude -p x"`); a path
/// that only ends in the name, or a word further in, does not start it.
fn literal_starts_harness(code: &str) -> bool {
    let c: Vec<char> = code.chars().collect();
    (0..c.len()).any(|i| {
        if !matches!(c[i], '"' | '\'' | '`') {
            return false;
        }
        let first: String = c[i + 1..]
            .iter()
            .skip_while(|x| x.is_whitespace())
            .take_while(|x| !x.is_whitespace() && !matches!(x, '"' | '\'' | '`' | ',' | ']' | ')'))
            .collect();
        !first.is_empty() && is_harness_program(&first)
    })
}

/// Code handed to an interpreter or a script that shells out to the harness's own program or to the
/// engine's `hook` command.
pub(super) fn code_denial(code: &str) -> Option<Decision> {
    let code = super::without_comment_lines(code);
    if !super::code_has_call(&code) {
        return None;
    }
    if literal_starts_harness(&code) {
        return Some(denial("harness", ""));
    }
    // Quotes, commas and brackets between the words of an argument list read as spaces.
    let flat: String = code
        .chars()
        .map(|c| if "\"'`,[](){};=".contains(c) { ' ' } else { c })
        .collect();
    let tokens: Vec<&str> = flat.split_whitespace().collect();
    if tokens
        .windows(2)
        .any(|w| super::basename(w[0]).eq_ignore_ascii_case("bf") && w[1] == "hook")
    {
        return Some(denial("hook", ""));
    }
    None
}
