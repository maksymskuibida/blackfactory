//! Argument parsing, dispatch and output for every `bf` command.
//!
//! Each command lives in a module of its own under `cli/`; this file keeps the argument type, the entry
//! point, `dispatch` and the helpers several commands share.

mod amend;
mod ask;
mod brief;
mod checkpoint;
mod cost;
mod doctor;
mod evidence;
mod gate;
mod group;
mod guard;
mod head;
mod help;
mod human_turn;
mod id;
mod init;
mod ledger;
mod log;
mod next;
mod page;
mod plan;
mod policy;
mod record;
mod repo;
mod report;
mod run;
mod status;
mod stuck;
mod test_mode;
mod unit;
mod view;

use crate::events;
use crate::refusal::{refuse, Fail, R};
use crate::schema;
use crate::store::Project;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const VALUE_FLAGS: &[&str] = &[
    "file",
    "kind",
    "unit",
    "result",
    "role",
    "note",
    "findings",
    "cost",
    "ref",
    "template",
    "out",
    "checkpoint",
    "tail",
    "name",
    "outcome",
    "says",
    "branch",
    "stage",
    "title",
    "context",
    "options",
    "reason",
    "scope",
    "quote",
    "token",
    "attempt",
    "judge",
    "accept",
    "decline",
    "fix-findings",
    "need-files",
    "verdict",
    "revert",
    "wave",
    "root",
    "C",
];

pub struct Args {
    pub pos: Vec<String>,
    pub flags: HashMap<String, String>,
    pub bools: HashSet<String>,
    pub rest: Vec<String>,
}

impl Args {
    pub fn parse(raw: &[String]) -> Args {
        let mut a = Args {
            pos: vec![],
            flags: HashMap::new(),
            bools: HashSet::new(),
            rest: vec![],
        };
        let mut i = 0;
        while i < raw.len() {
            let x = &raw[i];
            if x == "--" {
                a.rest = raw[i + 1..].to_vec();
                break;
            }
            if let Some(f) = x.strip_prefix("--") {
                if let Some((k, v)) = f.split_once('=') {
                    a.flags.insert(k.to_string(), v.to_string());
                } else if VALUE_FLAGS.contains(&f) && i + 1 < raw.len() {
                    a.flags.insert(f.to_string(), raw[i + 1].clone());
                    i += 1;
                } else {
                    a.bools.insert(f.to_string());
                }
            } else if x == "-C" && i + 1 < raw.len() {
                a.flags.insert("C".into(), raw[i + 1].clone());
                i += 1;
            } else {
                a.pos.push(x.clone());
            }
            i += 1;
        }
        a
    }
    pub(crate) fn flag(&self, k: &str) -> Option<&str> {
        self.flags.get(k).map(|x| x.as_str())
    }
    pub(crate) fn has(&self, k: &str) -> bool {
        self.bools.contains(k)
    }
    pub(crate) fn need(&self, k: &str, what: &str) -> R<&str> {
        self.flag(k).ok_or_else(|| {
            refuse(
                format!("run this command without --{k}"),
                format!("it needs {what}"),
                &[&format!("Add `--{k} <value>` and run the command again.")],
                "The command prints its result.",
            )
        })
    }
    pub(crate) fn pos_at(&self, i: usize, what: &str) -> R<&str> {
        self.pos.get(i).map(|x| x.as_str()).ok_or_else(|| {
            refuse(
                "run this command with a missing argument",
                format!("it needs {what}"),
                &["Add the argument and run the command again; docs/engine.md lists every command."],
                "The command prints its result.",
            )
        })
    }
}

pub(crate) fn read_stdin() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    s
}

pub(crate) fn read_data_in(p: &Project, path: &str) -> R<Value> {
    let safe = p.safe_input(Path::new(path))?;
    read_data(&safe.to_string_lossy())
}

pub(crate) fn read_data(path: &str) -> R<Value> {
    let text = fs::read_to_string(path).map_err(|e| {
        refuse(
            format!("read {path}"),
            format!("the file cannot be read: {e}"),
            &["Give the path of an existing file with `--file`."],
            "The command prints its result.",
        )
    })?;
    if path.ends_with(".json") {
        serde_json::from_str(&text).map_err(|e| {
            refuse(
                format!("read {path}"),
                format!("it is not valid JSON: {e}"),
                &["Fix the JSON syntax."],
                "The command prints its result.",
            )
        })
    } else {
        toml::from_str(&text).map_err(|e| {
            refuse(
                format!("read {path}"),
                format!("it is not valid TOML: {e}"),
                &["Fix the TOML syntax (or use a .json file)."],
                "The command prints its result.",
            )
        })
    }
}

pub(crate) fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Entry point used by the binary and by in-process tests. Returns the exit code.
pub fn main_with(raw: Vec<String>) -> i32 {
    let args = Args::parse(&raw);
    let json_mode = args.has("json");
    match dispatch(&args) {
        Ok(()) => 0,
        Err(Fail::Refused(r)) => {
            // The hook commands must never block the human's prompt with exit 2.
            let hook = args.pos.first().map(|x| x == "human-turn").unwrap_or(false);
            if json_mode {
                println!("{}", pretty(&r.to_json()));
            } else {
                eprint!("{}", r.text());
            }
            if hook {
                1
            } else {
                2
            }
        }
        Err(Fail::Error(e)) => {
            if json_mode {
                println!("{}", pretty(&json!({"ok": false, "error": e})));
            } else {
                eprintln!("error: {e}");
            }
            1
        }
    }
}

pub(crate) fn out(args: &Args, v: &Value, human: impl FnOnce() -> String) {
    if args.has("json") {
        println!("{}", pretty(v));
    } else {
        let t = human();
        if t.ends_with('\n') {
            print!("{t}");
        } else {
            println!("{t}");
        }
    }
}

pub(crate) fn start_dir(args: &Args) -> PathBuf {
    args.flag("C")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Where the process really runs: inside a unit's or a checkpoint's working copy, by its own current
/// directory (not by `-C`). ("unit" | "checkpoint", id)
pub(crate) fn real_zone(p: &Project) -> Option<(&'static str, String)> {
    let cwd = fs::canonicalize(std::env::current_dir().ok()?).ok()?;
    let rel = cwd.strip_prefix(p.work_dir()).ok()?;
    let id = rel.components().next()?.as_os_str().to_str()?.to_string();
    Some((
        if id.starts_with("K-") {
            "checkpoint"
        } else {
            "unit"
        },
        id,
    ))
}

/// A command run from inside a working copy is a worker's or a verifier's: it may report, run
/// evidence and read state, nothing else. This is the engine's own check; it does not rely on the hook.
fn enforce_zone(p: &Project, args: &Args) -> R<()> {
    let Some((zone, id)) = real_zone(p) else {
        return Ok(());
    };
    let cmd = args.pos.first().map(|x| x.as_str()).unwrap_or("");
    let sub = args.pos.get(1).map(|x| x.as_str()).unwrap_or("");
    let evidence_ok = ["run", "verify", "fail", "unreachable", "attach"];
    let ok = match (zone, cmd) {
        (_, "status") => true,
        (_, "evidence") => evidence_ok.contains(&sub),
        (_, "ask") => true,
        // A checkpoint's working copy reports on an open sync, or as a role with its token, and puts
        // records only with a role's token (the engine checks both).
        ("checkpoint", "report") => true,
        ("unit", "report") => true,
        (_, "record") => matches!(sub, "get" | "list" | "put"),
        _ => false,
    };
    if ok {
        return Ok(());
    }
    Err(refuse(
        format!("run `bf {cmd}` from the working copy of {id}"),
        "commands run from inside a working copy are a worker's or a verifier's: they may run evidence, read the status, and (a unit's worker, or an auditor with its token) report and file findings; planning, gates, briefs, landing, the log and the pages belong to the conductor and the human",
        &["Run the command from the project folder, as the conductor.", "From a working copy use `bf report`, `bf evidence run|verify|fail|unreachable|attach`, `bf record put finding`, `bf record get|list` or `bf status`."],
        "The command runs from the project folder.",
    ))
}

pub(crate) fn project_unchecked(args: &Args) -> R<Project> {
    let p = Project::discover(&start_dir(args))?;
    enforce_zone(&p, args)?;
    Ok(p)
}

/// The project, after checking that the log is intact and matches the state head: every command
/// that reads or changes state goes through this, so a rolled-back or damaged log is refused.
pub(crate) fn project(args: &Args) -> R<Project> {
    let p = project_unchecked(args)?;
    require_policy(&p)?;
    events::require_chain(&p)?;
    Ok(p)
}

/// Every command runs on the policy the human accepted. A file on disk that differs is refused.
fn require_policy(p: &Project) -> R<()> {
    use crate::store::PolicyState::*;
    let (because, steps): (&str, Vec<&str>) = match p.policy_state {
        Accepted => return Ok(()),
        NeverAccepted => (
            "no policy was ever accepted for this project on this machine",
            vec!["The human runs `bf init` in the project: it accepts the policy file that is there."],
        ),
        Changed => (
            "factory/policy.toml differs from the policy the human accepted (it was edited, or replaced, since)",
            vec![
                "Do not edit the policy yourself; it holds the safety settings (role tokens, the guard, the PATH checks run with).",
                "The human reads the change with `bf policy diff`.",
                "If they agree, they say so in chat and `bf policy accept --quote \"<their words>\"` takes it. If not, they put the old file back.",
            ],
        ),
        CopyDamaged => (
            "the accepted copy of the policy in the state folder does not match its hash in the log head",
            vec!["Tell the human. They read `bf policy diff` and accept the policy again with `bf policy accept --quote \"<their words>\"`."],
        ),
    };
    Err(refuse(
        "run with an unaccepted policy",
        because,
        &steps,
        "`bf status` prints the project.",
    ))
}

fn dispatch(args: &Args) -> R<()> {
    let cmd = args.pos.first().map(|x| x.as_str()).unwrap_or("");
    // `--help` and `-h` anywhere print the usage.
    if args.has("help") || args.pos.iter().any(|x| x == "-h") {
        println!("{}", help::HELP);
        return Ok(());
    }
    match cmd {
        "" | "help" | "--help" | "-h" => {
            println!("{}", help::HELP);
            Ok(())
        }
        "version" => {
            println!("bf {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "guard" => guard::cmd_guard(args),
        "init" => init::cmd_init(args),
        "human-turn" => human_turn::cmd_human_turn(args),
        "id" => id::cmd_id(args),
        "record" => record::cmd_record(args),
        "checks" | "scan" | "legal" | "preset" | "uiux" | "roles" => group::cmd_group(args, cmd),
        "ledger" => ledger::cmd_ledger(args),
        "plan" => plan::cmd_plan(args),
        "run" => run::cmd_run(args),
        "status" => status::cmd_status(args),
        "progress" => status::cmd_progress(args),
        "log" => log::cmd_log(args),
        "next" => next::cmd_next(args),
        "report" => report::cmd_report(args),
        "brief" => brief::cmd_brief(args),
        "unit" => unit::cmd_unit(args),
        "evidence" => evidence::cmd_evidence(args),
        "checkpoint" => checkpoint::cmd_checkpoint(args),
        "gate" => gate::cmd_gate(args),
        "test-mode" => test_mode::cmd_test_mode(args),
        "view" => view::cmd_view(args),
        "render" => view::cmd_render(args),
        "ask" => ask::cmd_ask(args),
        "answer" => ask::cmd_answer(args),
        "questions" => ask::cmd_questions(args),
        "amend" => amend::cmd_amend(args),
        "cost" => cost::cmd_cost(args),
        "repo" => repo::cmd_repo(args),
        "page" => page::cmd_page(args),
        "stuck" => stuck::cmd_stuck(args),
        "policy" => policy::cmd_policy(args),
        "head" => head::cmd_head(args),
        "doctor" => doctor::cmd_doctor(args),
        other => Err(refuse(
            format!("run `bf {other}`"),
            "there is no such command",
            &["Run `bf help` for the list of commands."],
            "`bf help` prints the usage.",
        )),
    }
}

pub(crate) fn kind_arg(name: &str) -> R<&'static schema::Kind> {
    schema::kind_by_name(name).ok_or_else(|| {
        refuse(
            format!("use the kind `{name}`"),
            "it is not a record kind",
            &["Use one of: requirement, component, screen, journey, target, threat, decision, unit, handoff, checkpoint, check, finding, question."],
            "The command prints its result.",
        )
    })
}
