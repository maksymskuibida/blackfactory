//! Argument parsing, dispatch and output for every `bf` command.

use crate::refusal::{err, refuse, Fail, R};
use crate::store::{default_policy_text, s, Project};
use crate::{doctor, events, evidence, gates, gitx, guard, ledger, plan, run, schema, util, view};
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
    "quote",
    "token",
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
    fn flag(&self, k: &str) -> Option<&str> {
        self.flags.get(k).map(|x| x.as_str())
    }
    fn has(&self, k: &str) -> bool {
        self.bools.contains(k)
    }
    fn need(&self, k: &str, what: &str) -> R<&str> {
        self.flag(k).ok_or_else(|| {
            refuse(
                format!("run this command without --{k}"),
                format!("it needs {what}"),
                &[&format!("Add `--{k} <value>` and run the command again.")],
                "The command prints its result.",
            )
        })
    }
    fn pos_at(&self, i: usize, what: &str) -> R<&str> {
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

fn read_stdin() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    s
}

fn read_data_in(p: &Project, path: &str) -> R<Value> {
    let safe = p.safe_input(Path::new(path))?;
    read_data(&safe.to_string_lossy())
}

fn read_data(path: &str) -> R<Value> {
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

fn pretty(v: &Value) -> String {
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

fn out(args: &Args, v: &Value, human: impl FnOnce() -> String) {
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

fn start_dir(args: &Args) -> PathBuf {
    args.flag("C")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Where the process really runs: inside a unit's or a checkpoint's working copy, by its own current
/// directory (not by `-C`). ("unit" | "checkpoint", id)
fn real_zone(p: &Project) -> Option<(&'static str, String)> {
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
        ("unit", "report") => true,
        (_, "record") => matches!(sub, "get" | "list") || (zone == "unit" && sub == "put"),
        _ => false,
    };
    if ok {
        return Ok(());
    }
    Err(refuse(
        format!("run `bf {cmd}` from the working copy of {id}"),
        "commands run from inside a working copy are a worker's or a verifier's: they may run evidence, read the status, and (a unit's worker) report and file findings; planning, gates, briefs, landing, the log and the pages belong to the conductor and the human",
        &["Run the command from the project folder, as the conductor.", "From a working copy use `bf report`, `bf evidence run|verify|fail|unreachable|attach`, `bf record put finding`, `bf record get|list` or `bf status`."],
        "The command runs from the project folder.",
    ))
}

fn project_unchecked(args: &Args) -> R<Project> {
    let p = Project::discover(&start_dir(args))?;
    enforce_zone(&p, args)?;
    Ok(p)
}

/// The project, after checking that the log is intact and matches the state head: every command
/// that reads or changes state goes through this, so a rolled-back or damaged log is refused.
fn project(args: &Args) -> R<Project> {
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
        println!("{}", HELP);
        return Ok(());
    }
    match cmd {
        "" | "help" | "--help" | "-h" => {
            println!("{}", HELP);
            Ok(())
        }
        "version" => {
            println!("bf {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "guard" => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            let root = args
                .flag("root")
                .map(|x| x.to_string())
                .or_else(|| std::env::var("BF_ROOT").ok());
            let o = guard::run_hook(&read_stdin(), &cwd, root.as_deref());
            println!("{o}");
            Ok(())
        }
        "init" => cmd_init(args),
        "human-turn" => {
            let payload: Value = serde_json::from_str(&read_stdin())
                .map_err(|e| err(format!("the hook payload is not JSON: {e}")))?;
            let dir = payload
                .get("cwd")
                .and_then(|c| c.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| start_dir(args));
            let p = Project::discover(&dir).or_else(|_| project(args))?;
            let (recorded, ev) = gates::human_turn(&p, &payload)?;
            if args.has("json") {
                println!("{}", pretty(&json!({"recorded": recorded, "event": ev})));
            }
            Ok(())
        }
        "id" => {
            let p = project(args)?;
            let sub = args.pos_at(1, "`new`")?;
            if sub != "new" {
                return Err(refuse(
                    "run `bf id`",
                    format!("unknown subcommand `{sub}`"),
                    &["Use `bf id new <kind>`."],
                    "The command prints an ID.",
                ));
            }
            let k = kind_arg(args.pos_at(2, "a record kind")?)?;
            let id = p.reserve_id(k)?;
            out(args, &json!({"id": id}), || id.clone());
            Ok(())
        }
        "record" => cmd_record(args),
        "ledger" => cmd_ledger(args),
        "plan" => {
            let p = project(args)?;
            if args.pos.get(1).map(|x| x.as_str()) != Some("check") {
                return Err(refuse(
                    "run `bf plan`",
                    "only `bf plan check` exists",
                    &["Run `bf plan check`."],
                    "The command prints `plan ok`.",
                ));
            }
            let pl = plan::load(&p)?;
            let waves = pl.waves();
            let v = json!({"ok": pl.problems.is_empty(), "problems": pl.problems, "waves": waves});
            if args.has("json") && !pl.problems.is_empty() {
                println!("{}", pretty(&v));
                std::process::exit(2);
            }
            plan::check_or_refuse(&pl)?;
            out(args, &v, || {
                format!(
                    "plan ok: {} units in {} waves, {} checkpoints",
                    pl.units.len(),
                    waves.len(),
                    pl.checkpoints.len()
                )
            });
            Ok(())
        }
        "run" => {
            let p = project(args)?;
            let v = match args.pos.get(1).map(|x| x.as_str()) {
                Some("start") => run::run_start(&p)?,
                Some("stop") => run::run_stop(&p)?,
                _ => {
                    return Err(refuse(
                        "run `bf run`",
                        "unknown subcommand",
                        &["Use `bf run start` or `bf run stop`."],
                        "The command prints the run.",
                    ))
                }
            };
            out(args, &v, || {
                format!(
                    "run {} {}",
                    v["run"].as_str().unwrap_or(""),
                    if v["test_mode"] == true {
                        "(test mode)"
                    } else {
                        ""
                    }
                )
            });
            Ok(())
        }
        "status" => {
            let p = project(args)?;
            let v = run::status(&p)?;
            out(args, &v, || status_text(&v));
            Ok(())
        }
        "log" => cmd_log(args),
        "next" => {
            let p = project(args)?;
            let v = run::next(&p)?;
            out(args, &v, || {
                let mut t = format!("run {}\n", v["run"].as_str().unwrap_or(""));
                for d in v["directives"].as_array().into_iter().flatten() {
                    t.push_str(&format!(
                        "  {} {} {} {}\n",
                        d["role"].as_str().unwrap_or(""),
                        d["action"].as_str().unwrap_or(""),
                        d["unit"]
                            .as_str()
                            .or(d["checkpoint"].as_str())
                            .or(d["gate"].as_str())
                            .unwrap_or(""),
                        d["tier"].as_str().unwrap_or("")
                    ));
                }
                for w in v["waiting"].as_array().into_iter().flatten() {
                    t.push_str(&format!("  waiting: {}\n", w.as_str().unwrap_or("")));
                }
                t
            });
            Ok(())
        }
        "report" => {
            let p = project(args)?;
            let unit = args.pos_at(1, "a unit ID")?;
            let result = args.need("result", "a result: built, findings, fixed or blocked")?;
            let findings = match args.flag("findings") {
                Some("-") => Some(serde_json::from_str::<Value>(&read_stdin()).map_err(|e| {
                    refuse(
                        "read findings from stdin",
                        format!("the input is not valid JSON: {e}"),
                        &["Pipe a JSON list such as [{\"title\": \"...\", \"severity\": \"high\"}] into the command."],
                        "The command prints the new state of the unit.",
                    )
                })?),
                Some(f) => Some(read_data_in(&p, f)?),
                None => None,
            };
            // The spend is the conductor's to report, from the session's real usage. With a budget cap
            // a report without it is refused, and text that is not a number never counts as zero.
            let cost = match args.flag("cost") {
                Some(c) => c.trim().parse::<f64>().ok().filter(|x| x.is_finite() && *x >= 0.0).ok_or_else(|| {
                    refuse(
                        "report with this cost",
                        format!("`{c}` is not a number of at least 0"),
                        &["Pass the spend as a plain number, for example `--cost 0.42`."],
                        "The command prints the unit's new state.",
                    )
                })?,
                None if p.policy.budget.cap > 0.0 => {
                    return Err(refuse(
                        "report without a cost",
                        "the policy has a budget cap, so every report must say what the work cost",
                        &["The conductor reads the spend of the session and adds `--cost <amount>` to the report."],
                        "The command prints the unit's new state.",
                    ))
                }
                None => 0.0,
            };
            let v = run::report(
                &p,
                unit,
                result,
                args.flag("role"),
                args.flag("note").unwrap_or(""),
                findings.as_ref(),
                cost,
                args.flag("token"),
            )?;
            out(args, &v, || {
                format!("{unit} is now {}", v["state"].as_str().unwrap_or(""))
            });
            Ok(())
        }
        "brief" => {
            let p = project(args)?;
            let unit = args.pos_at(1, "a unit ID")?;
            let role = args.need("role", "a role: builder, reviewer or verifier")?;
            let path = run::brief(&p, unit, role)?;
            out(args, &json!({"path": path.to_string_lossy()}), || {
                path.to_string_lossy().to_string()
            });
            Ok(())
        }
        "unit" => {
            let p = project(args)?;
            let sub = args.pos_at(1, "`start`, `land` or `release`")?;
            let unit = args.pos_at(2, "a unit ID")?;
            let v = match sub {
                "start" => run::unit_start(&p, unit)?,
                "land" => run::unit_land(&p, unit)?,
                "release" => run::unit_release(&p, unit, args.flag("quote").unwrap_or(""))?,
                _ => {
                    return Err(refuse(
                        "run `bf unit`",
                        "unknown subcommand",
                        &["Use `bf unit start <U>`, `bf unit land <U>` or `bf unit release <U> --quote \"...\"`."],
                        "The command prints the result.",
                    ))
                }
            };
            out(args, &v, || match sub {
                "start" => format!(
                    "{unit} working copy: {}",
                    v["working_copy"].as_str().unwrap_or("")
                ),
                "release" => format!(
                    "{unit} is back at work ({})",
                    v["state"].as_str().unwrap_or("")
                ),
                _ => format!(
                    "{unit} landed on {}",
                    v["branch"].as_str().unwrap_or("its checkpoint branch")
                ),
            });
            Ok(())
        }
        "evidence" => cmd_evidence(args),
        "checkpoint" => {
            let p = project(args)?;
            if args.pos.get(1).map(|x| x.as_str()) == Some("sync") {
                let k = args.pos_at(2, "a checkpoint ID")?;
                let v = run::checkpoint_sync(
                    &p,
                    k,
                    args.flag("quote").unwrap_or(""),
                    args.has("resolved"),
                )?;
                out(args, &v, || {
                    if v["synced"] == true {
                        format!("{k} took in the main branch")
                    } else {
                        "nothing to take in".to_string()
                    }
                });
                return Ok(());
            }
            if args.pos.get(1).map(|x| x.as_str()) != Some("close") {
                return Err(refuse(
                    "run `bf checkpoint`",
                    "only `bf checkpoint close` and `bf checkpoint sync` exist",
                    &["Run `bf checkpoint close <K>` or `bf checkpoint sync <K> --quote \"...\"`."],
                    "The command prints the result.",
                ));
            }
            let k = args.pos_at(2, "a checkpoint ID")?;
            let v = run::checkpoint_close(&p, k, args.has("merge"))?;
            out(args, &v, || {
                format!(
                    "{k} closed{}",
                    if v["merged"] == true {
                        " and merged onto the main branch"
                    } else {
                        ""
                    }
                )
            });
            Ok(())
        }
        "gate" => cmd_gate(args),
        "test-mode" => {
            let p = project(args)?;
            if args.pos.get(1).map(|x| x.as_str()) != Some("grant") {
                return Err(refuse(
                    "run `bf test-mode`",
                    "only `grant` exists",
                    &["Run `bf test-mode grant`."],
                    "The command prints the grant.",
                ));
            }
            let v = gates::grant(&p, args.flag("quote").unwrap_or(""))?;
            out(args, &v, || {
                format!(
                    "test mode granted for {}",
                    v["applies_to"].as_str().unwrap_or("")
                )
            });
            Ok(())
        }
        "view" => {
            let p = project(args)?;
            let kind = args.pos_at(1, "a page kind")?;
            let v = view::build(&p, kind, args.pos.get(2).map(|x| x.as_str()))?;
            println!("{}", pretty(&v));
            Ok(())
        }
        "render" => {
            let p = project(args)?;
            let kind = args.pos_at(1, "a page kind")?;
            let tpl = args.need("template", "an HTML template")?;
            let outp = args.need("out", "an output file")?;
            let v = view::build(&p, kind, args.pos.get(2).map(|x| x.as_str()))?;
            let html = view::render(&v, &p.safe_input(Path::new(tpl))?)?;
            let target = p.safe_output(Path::new(outp))?;
            util::atomic_write(&target, html.as_bytes())?;
            out(args, &json!({"out": outp, "bytes": html.len()}), || {
                format!("wrote {outp} ({} bytes)", html.len())
            });
            Ok(())
        }
        "ask" => {
            let p = project(args)?;
            let f = match (args.flag("file"), args.flag("title")) {
                (Some(file), _) => read_data_in(&p, file)?,
                (None, Some(title)) => {
                    // A short question without a file: `--title`, `--context`, `--kind`, `--options "A=text;B=text"`.
                    let mut q = json!({"title": title});
                    if let Some(c) = args.flag("context") {
                        q["context"] = json!(c);
                    }
                    if let Some(k) = args.flag("kind") {
                        q["kind"] = json!(k);
                    }
                    if let Some(o) = args.flag("options") {
                        q["options"] = json!(o
                            .split(';')
                            .filter(|x| !x.trim().is_empty())
                            .map(|x| {
                                let (k, t) = x.split_once('=').unwrap_or(("", x));
                                json!({"key": k.trim(), "text": t.trim()})
                            })
                            .collect::<Vec<_>>());
                        if q.get("kind").is_none() {
                            q["kind"] = json!("single");
                        }
                    } else if q.get("kind").is_none() {
                        q["kind"] = json!("text");
                    }
                    q
                }
                (None, None) => {
                    return Err(refuse(
                        "ask a question",
                        "it needs a question file (`--file`) or a title (`--title`)",
                        &["Write the question as `bf ask --title \"...\" [--context \"...\"] [--options \"A=first;B=second\"]`, or give a JSON file with `--file`."],
                        "The command prints the new question ID.",
                    ))
                }
            };
            // A question belongs to a stage (the gate waits for it) or to a unit (a worker's question to
            // the human; that unit waits for the answer, the others go on).
            let zone = real_zone(&p);
            let unit_tag: Option<String> = match (&zone, args.flag("unit")) {
                (Some((_, id)), _) => Some(id.clone()),
                (None, Some(u)) => Some(u.to_string()),
                _ => None,
            };
            let items: Vec<Value> = match &f {
                Value::Array(a) => a.clone(),
                o => match o.get("questions").and_then(|q| q.as_array()) {
                    Some(a) => a.clone(),
                    None => vec![o.clone()],
                },
            };
            if items.is_empty() {
                return Err(refuse(
                    "ask questions",
                    "the file holds no question",
                    &["Put a list of questions in the file: [{\"title\": \"...\", \"kind\": \"single\", \"options\": [...]}]."],
                    "The command prints the new question IDs.",
                ));
            }
            let kind = schema::kind_by_name("question").unwrap();
            let mut made: Vec<Value> = Vec::new();
            let mut recs: Vec<Value> = Vec::new();
            for f in &items {
                let mut rec = json!({
                    "id": f.get("id").cloned().unwrap_or_else(|| json!(format!("{}-{}", kind.prefix, next_q(&p, &recs)))),
                    "kind": "question",
                    "title": f.get("title").cloned().unwrap_or(Value::Null),
                    "answer_kind": f.get("kind").or_else(|| f.get("answer_kind")).cloned().unwrap_or(json!("single")),
                });
                for k in ["context", "options", "stage"] {
                    if let Some(v) = f.get(k) {
                        rec[k] = v.clone();
                    }
                }
                // `other` is the reserved key of the Other answer the engine adds to every choice question.
                if let Some(opts) = rec.get("options").and_then(|o| o.as_array()) {
                    if opts
                        .iter()
                        .any(|o| s(o, "key").eq_ignore_ascii_case("other"))
                    {
                        return Err(refuse(
                            format!("ask {}", f.get("title").and_then(|t| t.as_str()).unwrap_or("a question")),
                            "an option uses the key `other`, which is reserved: every choice question already takes an Other answer in the human's own words",
                            &["Give that option another key (A, B, C ...), or drop it.", "To switch the Other answer off for this question, put `\"other\": false` in it."],
                            "`bf questions` lists the question.",
                        ));
                    }
                }
                if let Some(b) = f.get("other").and_then(|v| v.as_bool()) {
                    rec["other"] = json!(b);
                }
                if rec.get("stage").is_none() {
                    if let Some(st) = args.flag("stage") {
                        rec["stage"] = json!(st);
                    }
                }
                if let Some(u) = &unit_tag {
                    rec["unit"] = json!(u);
                    rec.as_object_mut().map(|o| o.remove("stage"));
                }
                if rec.get("stage").is_none() && rec.get("unit").is_none() {
                    return Err(refuse(
                        "ask a question that blocks nothing",
                        "the question is not tagged: it names no stage and no unit, so no gate and no unit would wait for the answer",
                        &[
                            "A design question: add `--stage idea|requirements|ui|architecture|plan` (or `stage` in the file); that gate will not open until it is answered.",
                            "A question about a unit's work: add `--unit U-<n>` (a worker asks from its working copy and is tagged by itself); that unit waits for the answer while the others go on.",
                        ],
                        "`bf questions --open` lists the question with its stage or unit.",
                    ));
                }
                schema::validate_or_refuse(kind, &rec)?;
                recs.push(rec);
            }
            // Validate everything first, then write, so a bad list writes nothing.
            for rec in &recs {
                p.write_record(rec)?;
                events::append(&p, "question.asked", json!({"question": rec["id"]}))?;
                made.push(rec.clone());
            }
            p.write_guard()?;
            if let Some(n) = events::replay(&p)?.human_turns_note() {
                eprintln!("note: {n}");
            }
            out(args, &json!(made), || {
                made.iter()
                    .map(|r| s(r, "id").to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            Ok(())
        }
        "answer" => {
            let p = project(args)?;
            let id = args.pos_at(1, "a question ID")?;
            let f = read_data_in(&p, args.need("file", "an answer file (JSON or TOML)")?)?;
            let mut q = p.require_record(id)?;
            if s(&q, "kind") != "question" {
                return Err(refuse(
                    format!("answer {id}"),
                    "that ID is not a question",
                    &["Use a Q-<number> ID."],
                    "`bf questions` lists it answered.",
                ));
            }
            // An answer is the human's. It needs a human turn recorded after the question was asked: the
            // engine cannot read the answer's source, but it can see that the human wrote since.
            let log = events::load_log(&p)?;
            let asked_seq = log
                .events
                .iter()
                .find(|e| e["kind"] == "question.asked" && e["question"] == id)
                .and_then(|e| e["seq"].as_u64())
                .unwrap_or(0);
            let st = events::replay(&p)?;
            let Some(turn) = st.turns.iter().rev().find(|t| t.seq > asked_seq) else {
                return Err(refuse(
                    format!("answer {id}"),
                    "no human turn was recorded after the question was asked, so nothing shows that the human answered",
                    &[
                        "Show the human the question form and wait for them to say they are done.",
                        "The hook records their message; then run `bf answer` with what they chose.",
                    ],
                    "`bf questions` lists the question as answered.",
                ));
            };
            let turn_seq = turn.seq;
            let mut ans = f.get("answer").cloned().unwrap_or(f);
            // An Other answer is the human's own words: the key `other` chosen needs the text.
            if let Value::Object(o) = &mut ans {
                let chose_other = o.get("choice").and_then(|v| v.as_str()) == Some("other")
                    || o.get("choices")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().any(|c| c.as_str() == Some("other")))
                        .unwrap_or(false);
                if chose_other {
                    if !crate::store::question_takes_other(&q) {
                        return Err(refuse(
                            format!("answer {id} with Other"),
                            "this question does not take an Other answer",
                            &["Answer with one of its options."],
                            "`bf questions` lists the question as answered.",
                        ));
                    }
                    let text = o
                        .get("other")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if text.is_empty() {
                        return Err(refuse(
                            format!("answer {id} with Other"),
                            "the answer chooses `other` but gives no text; Other means the human's own answer",
                            &["Put the human's own words in `other`: `{\"choice\": \"other\", \"other\": \"<their answer>\"}` (for a multi question `{\"choices\": [\"A\", \"other\"], \"other\": \"<text>\"}`).", "Run `bf answer` again."],
                            "`bf questions` lists the question as answered, with their text.",
                        ));
                    }
                    o.insert("other".into(), json!(text));
                }
            }
            q["answer"] = ans;
            q["answered_at"] = json!(util::now());
            q["answered_turn"] = json!(turn_seq);
            p.write_record(&q)?;
            events::append(
                &p,
                "question.answered",
                json!({"question": id, "turn": turn_seq}),
            )?;
            p.write_guard()?;
            out(args, &q, || format!("answered {id}"));
            Ok(())
        }
        "questions" => {
            let p = project(args)?;
            let qs = p.list_records(schema::kind_by_name("question"))?;
            let sel: Vec<&Value> = qs
                .iter()
                .filter(|q| !args.has("open") || q.get("answer").is_none())
                .collect();
            out(args, &json!(sel), || {
                sel.iter()
                    .map(|q| {
                        format!(
                            "{} {} [{}]",
                            s(q, "id"),
                            s(q, "title"),
                            if q.get("answer").is_some() {
                                format!("answered: {}", crate::store::answer_text(q))
                            } else {
                                "open".to_string()
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            Ok(())
        }
        "amend" => {
            let p = project(args)?;
            let v = match args.pos.get(1).map(|x| x.as_str()) {
                Some("open") => gates::amend_open(&p, &args.pos[2..])?,
                Some("narrow") => gates::amend_narrow(
                    &p,
                    &read_data_in(&p, args.need("file", "a narrowing file")?)?,
                )?,
                Some("approve") => gates::amend_approve(
                    &p,
                    args.pos.get(2).map(|x| x.as_str()),
                    args.flag("quote").unwrap_or(""),
                )?,
                _ => {
                    return Err(refuse(
                        "run `bf amend`",
                        "unknown subcommand",
                        &["Use `bf amend open <ID>...`, `narrow --file <f>` or `approve`."],
                        "The command prints the amendment.",
                    ))
                }
            };
            out(args, &v, || pretty(&v));
            Ok(())
        }
        "cost" => {
            let p = project(args)?;
            let v = run::cost_status(&p)?;
            out(args, &v, || {
                format!(
                    "spent {:.2} of {} {}",
                    v["spent"].as_f64().unwrap_or(0.0),
                    v["cap"],
                    v["unit"].as_str().unwrap_or("")
                )
            });
            Ok(())
        }
        "repo" => cmd_repo(args),
        "page" => {
            let p = project(args)?;
            match args.pos_at(1, "`set` or `list`")? {
                "set" => {
                    let v = run::page_set(
                        &p,
                        args.pos_at(2, "a page name")?,
                        args.pos_at(3, "the page's secure web address")?,
                    )?;
                    out(args, &v, || {
                        format!("{} is at {}", s(&v, "name"), s(&v, "url"))
                    });
                }
                "list" => {
                    let st = events::replay(&p)?;
                    let v = json!(run::pages_json(&st));
                    out(args, &v, || {
                        run::pages_json(&st)
                            .iter()
                            .map(|r| format!("{} {}", s(r, "name"), s(r, "url")))
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                }
                _ => {
                    return Err(refuse(
                        "run `bf page`",
                        "unknown subcommand",
                        &["Use `bf page set <name> <url>` or `bf page list`."],
                        "The command prints the result.",
                    ))
                }
            }
            Ok(())
        }
        "stuck" => {
            let p = project(args)?;
            let v = run::stuck(&p, args.has("dry-run"))?;
            out(args, &v, || pretty(&v));
            Ok(())
        }
        "policy" => {
            let p = project_unchecked(args)?;
            match args.pos.get(1).map(|x| x.as_str()) {
                Some("accept") => {
                    let v = gates::accept_policy(&p, args.flag("quote").unwrap_or(""))?;
                    out(args, &v, || "policy accepted".to_string());
                }
                Some("diff") => {
                    let d = gates::policy_diff(&p);
                    out(args, &json!({"diff": d}), || d.clone());
                }
                _ => {
                    return Err(refuse(
                        "run `bf policy`",
                        "only `accept` and `diff` exist",
                        &["`bf policy diff` shows what changed; `bf policy accept --quote \"<the human's words>\"` takes it."],
                        "`bf status` runs again.",
                    ))
                }
            }
            Ok(())
        }
        "head" => {
            let p = project_unchecked(args)?;
            if args.pos.get(1).map(|x| x.as_str()) != Some("accept") {
                return Err(refuse(
                    "run `bf head`",
                    "only `bf head accept` exists",
                    &["After the human says in chat to accept the log as it stands: `bf head accept --quote \"<their words>\"`."],
                    "`bf log --verify` prints `chain ok`.",
                ));
            }
            let v = events::accept_head(&p, args.flag("quote").unwrap_or(""))?;
            p.write_guard()?;
            out(args, &v, || {
                format!("accepted the log ({} events); the head is set", v["events"])
            });
            Ok(())
        }
        "doctor" => {
            let p = project_unchecked(args)?;
            if args.has("perf") {
                let exe =
                    std::env::current_exe().map_err(|e| err(format!("cannot find myself: {e}")))?;
                let v = doctor::perf(&p, &exe)?;
                out(args, &v, || {
                    v["rows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| {
                            format!(
                                "{:<24} median {:>8.2} ms   p99 {:>8.2} ms",
                                r["command"].as_str().unwrap_or(""),
                                r["median_ms"].as_f64().unwrap_or(0.0),
                                r["p99_ms"].as_f64().unwrap_or(0.0)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            } else {
                let v = doctor::doctor(&p)?;
                out(args, &v, || {
                    v["checks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|c| {
                            format!(
                                "{}: {} ({})",
                                c["check"].as_str().unwrap_or(""),
                                if c["ok"] == true { "ok" } else { "PROBLEM" },
                                c["note"].as_str().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                if v["ok"] != true {
                    std::process::exit(2);
                }
            }
            Ok(())
        }
        other => Err(refuse(
            format!("run `bf {other}`"),
            "there is no such command",
            &["Run `bf help` for the list of commands."],
            "`bf help` prints the usage.",
        )),
    }
}

/// The next free question number, counting the questions not yet written.
fn next_q(p: &Project, pending: &[Value]) -> u32 {
    let base = p
        .next_id(schema::kind_by_name("question").unwrap())
        .ok()
        .and_then(|id| schema::parse_id(&id).map(|x| x.1))
        .unwrap_or(1);
    let extra = pending
        .iter()
        .filter_map(|r| schema::parse_id(s(r, "id")).map(|x| x.1))
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);
    base.max(extra)
}

fn kind_arg(name: &str) -> R<&'static schema::Kind> {
    schema::kind_by_name(name).ok_or_else(|| {
        refuse(
            format!("use the kind `{name}`"),
            "it is not a record kind",
            &["Use one of: requirement, component, screen, journey, target, threat, decision, unit, handoff, checkpoint, check, finding, question."],
            "The command prints its result.",
        )
    })
}

fn status_text(v: &Value) -> String {
    let mut t = format!(
        "project: {}\nrun: {}\ntest mode: {}\nlog: {} events, chain {}\n",
        v["project"].as_str().unwrap_or(""),
        v["run"].as_str().unwrap_or("none"),
        v["test_mode"].as_str().unwrap_or(""),
        v["events"],
        if v["chain_ok"] == true {
            "ok"
        } else {
            "BROKEN"
        }
    );
    if let Some(n) = v["main"]["note"].as_str().filter(|n| !n.is_empty()) {
        t.push_str(&format!("note: {n}\n"));
    }
    if let Some(n) = v["human_turns_note"].as_str().filter(|n| !n.is_empty()) {
        t.push_str(&format!("note: {n}\n"));
    }
    for m in v["main_ahead"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "note: the main branch of {} holds {} commit(s) that {} lacks; `bf checkpoint sync {} --quote \"<the human's words>\"` takes them in\n",
            m["repo"].as_str().unwrap_or(""),
            m["commits"],
            m["checkpoint"].as_str().unwrap_or(""),
            m["checkpoint"].as_str().unwrap_or(""),
        ));
    }
    for pg in v["pages"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "page {}: {}\n",
            pg["name"].as_str().unwrap_or(""),
            pg["url"].as_str().unwrap_or("")
        ));
    }
    for b in v["bad_records"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "record file {} cannot be read: {}\n",
            b["file"].as_str().unwrap_or(""),
            b["error"].as_str().unwrap_or("")
        ));
    }
    if v["workspace"] == true {
        for r in v["repos"].as_array().into_iter().flatten() {
            t.push_str(&format!(
                "repository {}: {} at {} ({}){}\n",
                r["name"].as_str().unwrap_or(""),
                r["branch"].as_str().unwrap_or(""),
                r["tip"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(10)
                    .collect::<String>(),
                r["path"].as_str().unwrap_or(""),
                if r["ok"] == false { " - MOVED" } else { "" }
            ));
        }
    }
    if v["main"]["ok"] == false {
        t.push_str(&format!(
            "main: MOVED - {}\n",
            v["main"]["detail"].as_str().unwrap_or("")
        ));
    }
    for u in v["units"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "  {} {}\n",
            u["id"].as_str().unwrap_or(""),
            u["state"].as_str().unwrap_or("")
        ));
    }
    for g in v["gates"].as_array().into_iter().flatten() {
        t.push_str(&format!(
            "  gate {} {}\n",
            g["id"].as_str().unwrap_or(""),
            g["status"].as_str().unwrap_or("")
        ));
    }
    t
}

/// `bf repo add <name> <path> [--branch B]` and `bf repo list`: the code repositories of a workspace.
fn cmd_repo(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`add` or `list`")? {
        "add" => {
            if !p.workspace {
                return Err(refuse(
                    "add a repository",
                    "this project is not a workspace: its one repository is the project folder",
                    &["Run `bf init --workspace` in a new folder to make a workspace that points at code repositories."],
                    "`bf repo list` shows the repositories.",
                ));
            }
            let name = args.pos_at(2, "a short name")?;
            let path = args.pos_at(3, "the path of the repository")?;
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(refuse(
                    "add a repository",
                    "the name must be letters, digits, `-` or `_`",
                    &["Pick a short name such as `app` or `api`."],
                    "`bf repo list` shows it.",
                ));
            }
            if p.repos.iter().any(|r| r.name == name) {
                return Err(refuse(
                    "add a repository",
                    format!("the workspace already lists `{name}`"),
                    &["Pick another name, or edit factory/workspace.toml by hand."],
                    "`bf repo list` shows it once.",
                ));
            }
            let given = Path::new(path);
            let joined = if given.is_absolute() {
                given.to_path_buf()
            } else {
                std::env::current_dir()?.join(given)
            };
            let abs = fs::canonicalize(&joined).map_err(|e| {
                refuse(
                    "add a repository",
                    format!("{} cannot be opened: {e}", joined.display()),
                    &["Give the path of an existing git repository."],
                    "`bf repo list` shows it.",
                )
            })?;
            check_repo_path(&p, &abs)?;
            let branch = match args.flag("branch") {
                Some(b) => b.to_string(),
                None => gitx::current_branch(&abs).unwrap_or_else(|| "main".into()),
            };
            if !gitx::branch_exists(&abs, &branch) {
                return Err(refuse(
                    "add a repository",
                    format!(
                        "the branch `{branch}` does not exist or has no commit in {}",
                        abs.display()
                    ),
                    &["Make an initial commit, or name the main branch with `--branch`."],
                    "`bf repo list` shows it.",
                ));
            }
            // Relative to the workspace when it lies beside it, otherwise absolute: the file is tracked.
            let shown = shown_path(&p, &abs);
            let file = p.root.join("factory/workspace.toml");
            let mut text = fs::read_to_string(&file).unwrap_or_default();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&format!(
                "\n[[repo]]\nname = {}\npath = {}\nmain_branch = {}\n",
                toml::Value::String(name.to_string()),
                toml::Value::String(shown),
                toml::Value::String(branch.clone()),
            ));
            util::atomic_write(&file, text.as_bytes())?;
            let mut p2 = Project::discover(&p.root)?;
            let mut h = events::read_head(&p2).unwrap_or_default();
            let tip = gitx::must(&abs, &["rev-parse", &branch]).ok();
            let e = h.repos.entry(name.to_string()).or_default();
            e.branch = Some(branch.clone());
            e.commit = tip;
            events::write_head(&p2, &h)?;
            p2.reload_policy()?;
            p2.write_guard()?;
            let added = p2.ensure_excludes()?;
            let v = json!({"name": name, "path": abs.to_string_lossy(), "branch": branch, "excluded": added});
            out(args, &v, || {
                let mut t = format!("added {name}: {}", abs.display());
                if !added.is_empty() {
                    t.push_str(&format!(
                        "\nadded {} to the workspace's local ignore file (.git/info/exclude): nothing of the code repository is committed into the workspace",
                        added.join(", ")
                    ));
                }
                t
            });
        }
        "list" => {
            let st = events::replay(&p)?;
            let pos = run::main_positions(&p, &st)?;
            let v = json!(p
                .repos
                .iter()
                .zip(pos.iter())
                .map(|(r, m)| json!({"name": r.name, "path": r.path.to_string_lossy(), "branch": r.branch, "tip": m.tip, "ok": !m.moved}))
                .collect::<Vec<_>>());
            out(args, &v, || {
                p.repos
                    .iter()
                    .zip(pos.iter())
                    .map(|(r, m)| {
                        format!(
                            "{} {} {} at {}",
                            r.name,
                            r.path.display(),
                            r.branch,
                            m.tip.chars().take(10).collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        _ => {
            return Err(refuse(
                "run `bf repo`",
                "unknown subcommand",
                &["Use `bf repo add <name> <path> [--branch B]` or `bf repo list`."],
                "The command prints the result.",
            ))
        }
    }
    Ok(())
}

/// How a repository path is written in `factory/workspace.toml`: relative to the workspace (inside it, or
/// a sibling), otherwise absolute.
fn shown_path(p: &Project, abs: &Path) -> String {
    if let Ok(rel) = abs.strip_prefix(&p.root) {
        return rel.to_string_lossy().to_string();
    }
    pathdiff(abs, &p.root).unwrap_or_else(|| abs.to_string_lossy().to_string())
}

/// A code repository may be apart from the workspace or a folder inside it; it must be the top of its own
/// git repository, and not the workspace itself or a place the factory keeps its own files.
fn check_repo_path(p: &Project, abs: &Path) -> R<()> {
    let bad = |why: String, step: &str| {
        refuse("add a repository", why, &[step], "`bf repo list` shows it.")
    };
    if abs == p.root || p.root.starts_with(abs) {
        return Err(bad(
            "that is the workspace itself (or a folder above it), not a code repository".into(),
            "Give the path of a code repository: a folder of its own with its own `.git`, beside the workspace or inside it.",
        ));
    }
    if let Ok(rel) = abs.strip_prefix(&p.root) {
        let first = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_default();
        if matches!(first.as_str(), "factory" | ".bf" | ".claude" | ".git") {
            return Err(bad(
                format!(
                    "{} is inside `{first}/`, where the factory keeps its own files",
                    abs.display()
                ),
                "Put the code repository in a folder of its own, for example `./app`.",
            ));
        }
    }
    if !gitx::is_repo(abs) || !abs.join(".git").exists() {
        return Err(bad(
            format!("{} is not the top of a git repository", abs.display()),
            "Give the main checkout of the repository (the folder that holds `.git`).",
        ));
    }
    Ok(())
}

/// `path` relative to `base` when both are absolute and `path` is not below `base` (a sibling folder).
fn pathdiff(path: &Path, base: &Path) -> Option<String> {
    let pc: Vec<_> = path.components().collect();
    let bc: Vec<_> = base.components().collect();
    let common = pc.iter().zip(bc.iter()).take_while(|(a, b)| a == b).count();
    if common == 0 || common < 2 {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..bc.len() {
        out.push("..");
    }
    for c in &pc[common..] {
        out.push(c.as_os_str());
    }
    Some(out.to_string_lossy().to_string())
}

fn cmd_init(args: &Args) -> R<()> {
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
    {
        let mut h = events::read_head(&p).unwrap_or_default();
        if record_repo_heads(&p, &mut h) {
            events::write_head(&p, &h)?;
        }
    }
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

fn cmd_record(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`put`, `get` or `list`")? {
        "put" => cmd_record_put(args, &p)?,
        "get" => {
            let id = args.pos_at(2, "a record ID")?;
            let rec = p.require_record(id)?;
            if args.has("json") {
                println!("{}", pretty(&rec));
            } else {
                print!("{}", toml::to_string_pretty(&rec)?);
            }
        }
        "list" => {
            let kind = match args.flag("kind") {
                Some(k) => Some(kind_arg(k)?),
                None => None,
            };
            let recs = p.list_records(kind)?;
            out(args, &json!(recs), || {
                recs.iter()
                    .map(|r| format!("{} {}", s(r, "id"), s(r, "title")))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        _ => {
            return Err(refuse(
                "run `bf record`",
                "unknown subcommand",
                &["Use `bf record put <kind> --file <f>`, `get <id>` or `list [--kind k]`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}

/// Take the lock that serialises ID allocation and record writes (two puts at once must not collide).
fn lock_records(p: &Project) -> R<fs::File> {
    fs::create_dir_all(p.dot_bf())?;
    let f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join("records.lock"))?;
    f.lock()
        .map_err(|e| err(format!("cannot lock the records: {e}")))?;
    Ok(f)
}

fn cmd_record_put(args: &Args, p: &Project) -> R<()> {
    let kind = kind_arg(args.pos_at(2, "a record kind")?)?;
    let mut rec = read_data_in(p, args.need("file", "a record file (TOML or JSON)")?)?;
    let _lock = lock_records(p)?;
    // A worker's only record is a new, open finding for a unit that is in review. Marking a finding
    // fixed or accepted is not a worker's to do.
    if real_zone(p).is_some() {
        let st = events::replay(p)?;
        let unit = s(&rec, "unit").to_string();
        let outcome = s(&rec, "outcome").to_string();
        if kind.name != "finding"
            || st.unit_state(&unit) != "review"
            || !(outcome.is_empty() || outcome == "open")
        {
            return Err(refuse(
                format!("put a {} from a working copy", kind.name),
                "a worker may file a new, open finding for a unit that is in review; every other record belongs to the planner and the human, and fixed and accepted are set by the engine and the human",
                &["Report with `bf report <unit> --result findings --role reviewer` while the unit is in review.", "To accept a finding, ask the human; they put it from the main checkout."],
                "`bf record get` shows the finding as open.",
            ));
        }
    }
    if rec.get("kind").is_none() {
        rec["kind"] = json!(kind.name);
    }
    let mut assigned = false;
    if rec.get("id").is_none() {
        rec["id"] = json!(p.next_id(kind)?);
        assigned = true;
    }
    schema::validate_or_refuse(kind, &rec)?;
    let id = s(&rec, "id").to_string();
    let st = events::require_chain(p)?;
    let old = p.get_record(&id)?;
    // Once a stage's gate is approved, its records change, and new ones appear, only by amendment.
    if let Some(stage) = gates::stage_of_kind(kind.name) {
        let changes = old.as_ref().map(|o| o != &rec).unwrap_or(true);
        if st.gate_passed(stage, None) && changes && !gates::amended(&st, &id) {
            return Err(refuse(
                format!("{} {id}", if old.is_some() { "replace" } else { "add" }),
                format!("the {stage} gate is approved; an approved stage changes only by amendment, and that includes new records"),
                &[
                    &format!("Run `bf amend open {id}` to see the impact."),
                    "Narrow it with `bf amend narrow --file <f>`; the human approves it.",
                    "Put the record again.",
                ],
                format!("`bf record get {id}` shows the new content."),
            ));
        }
    }
    if let Some(old) = &old {
        if old != &rec && !args.has("replace") && kind.name != "finding" {
            return Err(refuse(
                format!("put {id}"),
                format!("{id} is already a record (\"{}\") and this file is different; putting it would overwrite that record", s(old, "title")),
                &[
                    &format!("If you meant a new record, put a fresh ID in the file: `bf id new {}` prints the next free one.", kind.name),
                    &format!("If you meant to revise {id}, run the same command again with `--replace`."),
                ],
                format!("`bf record get {id}` shows the record you intended."),
            ));
        }
    }
    p.write_record(&rec)?;
    p.write_guard()?;
    out(args, &json!({"put": id, "assigned": assigned}), || {
        if assigned {
            format!("put {id} (the file had no id, so the engine assigned {id}; use this id when you refer to the record)")
        } else {
            format!("put {id}")
        }
    });
    Ok(())
}

fn cmd_ledger(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`check` or `impact`")? {
        "check" => {
            let pl = plan::load(&p)?;
            let plan_only = args.has("plan");
            let rows = ledger::check(&p, &pl, plan_only)?;
            let gaps: Vec<&ledger::Row> = rows.iter().filter(|r| !r.missing.is_empty()).collect();
            if args.has("json") && !gaps.is_empty() {
                println!(
                    "{}",
                    pretty(
                        &json!({"ok": false, "gaps": gaps.iter().map(|g| json!({"id": g.id, "title": g.title, "missing": g.missing})).collect::<Vec<_>>()})
                    )
                );
                std::process::exit(2);
            }
            ledger::refuse_if_gaps(&rows, plan_only)?;
            out(args, &json!({"ok": true, "parts": rows.len()}), || {
                format!("ledger closed: {} designed parts linked", rows.len())
            });
        }
        "impact" => {
            if args.pos.len() < 3 {
                return Err(refuse(
                    "compute an impact set",
                    "no IDs were given",
                    &["Run `bf ledger impact R-1 C-2`."],
                    "The command prints the linked IDs.",
                ));
            }
            let pl = plan::load(&p)?;
            let v = ledger::impact(&p, &pl, &args.pos[2..])?;
            out(args, &v, || pretty(&v));
        }
        _ => {
            return Err(refuse(
                "run `bf ledger`",
                "unknown subcommand",
                &["Use `bf ledger check` or `bf ledger impact <ID>...`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}

fn cmd_log(args: &Args) -> R<()> {
    let p = project_unchecked(args)?;
    let log = events::load_log(&p)?;
    if args.has("verify") {
        let v =
            json!({"chain_ok": log.chain_ok, "events": log.events.len(), "error": log.chain_error});
        out(args, &v, || {
            if log.chain_ok {
                format!("chain ok ({} events)", log.events.len())
            } else {
                format!(
                    "chain BROKEN: {}",
                    log.chain_error.clone().unwrap_or_default()
                )
            }
        });
        if !log.chain_ok {
            std::process::exit(2);
        }
        return Ok(());
    }
    let n = args
        .flag("tail")
        .and_then(|x| x.parse::<usize>().ok())
        .unwrap_or(log.events.len());
    let skip = log.events.len().saturating_sub(n);
    let evs: Vec<&Value> = log.events.iter().skip(skip).collect();
    if args.has("json") {
        println!("{}", pretty(&json!(evs)));
    } else {
        for e in evs {
            let mut m = e.clone();
            if let Some(o) = m.as_object_mut() {
                for k in ["prev", "h", "snapshot", "impact"] {
                    o.remove(k);
                }
            }
            println!("{}", serde_json::to_string(&m)?);
        }
    }
    Ok(())
}

fn cmd_evidence(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`run`, `attach` or `verify`")? {
        "run" => {
            let check = args.pos_at(2, "a check ID")?;
            let v = evidence::run(&p, check, args.flag("unit"), &args.rest)?;
            let ok = v["exit"].as_i64() == Some(0);
            out(args, &v, || {
                format!(
                    "{} {}: exit {}, content {}",
                    s(&v, "id"),
                    check,
                    v["exit"],
                    &s(&v, "content_hash")[..12.min(s(&v, "content_hash").len())]
                )
            });
            if !ok {
                std::process::exit(1);
            }
        }
        "attach" => {
            let check = args.pos_at(2, "a check ID")?;
            let file = args.pos_at(3, "a file")?;
            let v = evidence::attach(
                &p,
                check,
                Path::new(file),
                args.flag("unit"),
                &evidence::Judgment {
                    outcome: args.flag("outcome"),
                    says: args.flag("says").unwrap_or(""),
                    token: args.flag("token"),
                },
            )?;
            out(args, &v, || {
                format!("{} attached {} to {}", s(&v, "id"), file, check)
            });
        }
        kind @ ("fail" | "unreachable") => {
            let check = args.pos_at(2, "a check ID")?;
            let reason = args.flag("reason").unwrap_or("");
            let v = evidence::outcome(&p, check, kind, reason, args.flag("unit"))?;
            out(args, &v, || {
                format!("{} recorded: {check} is {kind} ({reason})", s(&v, "id"))
            });
        }
        "verify" => {
            let (rows, ok) = evidence::verify(&p, args.flag("unit"), args.flag("checkpoint"))?;
            let v = json!({"ok": ok, "receipts": rows.iter().map(|r| json!({"receipt": r.receipt, "check": r.check, "seal_ok": r.seal_ok, "content_ok": r.content_ok, "exit": r.exit})).collect::<Vec<_>>()});
            if args.has("json") {
                println!("{}", pretty(&v));
            } else {
                for r in &rows {
                    println!(
                        "{} {} seal={} content={} exit={}",
                        r.receipt,
                        r.check,
                        if r.seal_ok { "ok" } else { "BAD" },
                        r.content_ok
                            .map(|c| if c { "ok" } else { "OTHER" })
                            .unwrap_or("-"),
                        r.exit.map(|e| e.to_string()).unwrap_or("-".into())
                    );
                }
                println!("{}", if ok { "evidence ok" } else { "evidence NOT ok" });
            }
            if !ok {
                std::process::exit(2);
            }
        }
        _ => {
            return Err(refuse(
                "run `bf evidence`",
                "unknown subcommand",
                &["Use `run`, `attach` or `verify`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}

fn cmd_gate(args: &Args) -> R<()> {
    let p = project(args)?;
    match args.pos_at(1, "`open`, `approve`, `reject` or `status`")? {
        "open" => {
            let kind = args.pos_at(2, "a gate kind")?;
            let g = gates::open(&p, kind, args.flag("ref"))?;
            out(args, &gates::gate_json(&g), || g.id.clone());
        }
        "skip" => {
            let kind = args.pos_at(2, "a gate kind (ui)")?;
            let v = gates::skip(&p, kind, args.flag("quote").unwrap_or(""))?;
            out(args, &v, || format!("the {kind} gate is skipped"));
        }
        "approve" => {
            let g = gates::approve(
                &p,
                args.pos_at(2, "a gate ID")?,
                args.has("simulated"),
                args.flag("quote").unwrap_or(""),
            )?;
            out(args, &gates::gate_json(&g), || {
                format!(
                    "approved {}{}",
                    g.id,
                    if g.simulated { " (simulated)" } else { "" }
                )
            });
        }
        "reject" => {
            let g = gates::reject(
                &p,
                args.pos_at(2, "a gate ID")?,
                args.flag("note").unwrap_or(""),
                args.flag("quote").unwrap_or(""),
            )?;
            out(args, &gates::gate_json(&g), || format!("rejected {}", g.id));
        }
        "status" => {
            let st = events::replay(&p)?;
            let v = json!(st.gates.iter().map(gates::gate_json).collect::<Vec<_>>());
            out(args, &v, || {
                if st.gates.is_empty() {
                    "no gates yet".into()
                } else {
                    st.gates
                        .iter()
                        .map(|g| {
                            format!(
                                "{} {}{}{}",
                                g.id,
                                g.status,
                                if g.simulated { " (simulated)" } else { "" },
                                if g.quote.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        " - decided on the words: \"{}\" (the human's whole message: \"{}\"){}",
                                        g.quote,
                                        g.message,
                                        if g.caution.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" - caution: {}", g.caution)
                                        } + &if g.transcript.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" - transcript: {}", g.transcript)
                                        }
                                    )
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            });
        }
        _ => {
            return Err(refuse(
                "run `bf gate`",
                "unknown subcommand",
                &["Use `open`, `approve`, `reject` or `status`."],
                "The command prints its result.",
            ))
        }
    }
    Ok(())
}

const HELP: &str = "bf: the Black Factory engine

State:    init | record put|get|list | id new | ledger check|impact | plan check
Run:      run start|stop | status | log | next | brief | report | unit start|land
Proof:    evidence run|attach|verify|fail|unreachable | checkpoint close
Humans:   gate open|approve|reject|status | human-turn | test-mode grant
Pages:    view <kind> | render <kind>
More:     ask | answer | questions | amend | cost | stuck | doctor [--perf] | guard

Every command that prints data takes --json. Exit codes: 0 done, 2 refused, 1 error.
See docs/engine.md.";
