//! The harness walkthrough: the exact command sequence that `harness/claude/skills/factory/SKILL.md`
//! and the agent files in `harness/claude/agents/` tell the conductor, builder, reviewer and
//! verifier to run, with the flags they name. Feeding `bf human-turn` stands in for the human's typed
//! messages and is part of the fixture. Every command is also put through `bf guard` first, as the
//! hooks would, from the working directory the role works in.
//!
//! Each test states in its comments what it checks: the order of the commands, which role runs which
//! command and from which folder, and what the engine answers.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

struct Walk<'a> {
    fx: &'a Fx,
    template: PathBuf,
}

impl<'a> Walk<'a> {
    fn new(fx: &'a Fx) -> Walk<'a> {
        // `install.sh` puts the template here; the skill names this path.
        let dir = fx.root.join(".claude/skills/factory");
        fs::create_dir_all(&dir).unwrap();
        let tpl = dir.join("page.html");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views/templates/page.html"),
            &tpl,
        )
        .unwrap();
        Walk { fx, template: tpl }
    }

    /// Would the hook let this Bash command through from this folder?
    fn guard_bash(&self, cwd: &Path, cmd: &str) {
        let payload = json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd.to_string_lossy()});
        let o = self
            .fx
            .bf_stdin(cwd, &["guard"], &payload.to_string())
            .json();
        assert_ne!(
            o["hookSpecificOutput"]["permissionDecision"],
            "deny",
            "the hook denies `{cmd}` in {}: {o}",
            cwd.display()
        );
    }

    fn guard_write(&self, cwd: &Path, file: &Path) {
        let payload = json!({"tool_name": "Write", "tool_input": {"file_path": file.to_string_lossy()}, "cwd": cwd.to_string_lossy()});
        let o = self
            .fx
            .bf_stdin(cwd, &["guard"], &payload.to_string())
            .json();
        assert_ne!(
            o["hookSpecificOutput"]["permissionDecision"],
            "deny",
            "the hook denies a write to {}: {o}",
            file.display()
        );
    }

    /// Run `bf <args>` the way a role would: past the guard first, then for real.
    fn bf(&self, cwd: &Path, args: &[&str]) -> Out {
        self.guard_bash(cwd, &format!("bf {}", args.join(" ")));
        self.fx.bf_in(cwd, args)
    }

    fn ok(&self, cwd: &Path, args: &[&str]) -> Out {
        let o = self.bf(cwd, args);
        assert_eq!(
            o.code,
            0,
            "bf {args:?} in {} failed:\n{}",
            cwd.display(),
            o.all()
        );
        o
    }

    fn conductor(&self, args: &[&str]) -> Out {
        self.ok(&self.fx.root, args)
    }

    /// Step A of the design phase: `bf id new`, a file in `.bf/drafts/` that starts with the id, `bf record put`.
    fn new_id(&self, kind: &str) -> String {
        self.conductor(&["id", "new", kind])
            .stdout
            .trim()
            .to_string()
    }

    fn put(&self, kind: &str, id: &str, body: &str) -> Out {
        let f = self.fx.root.join(".bf/drafts").join(format!("{id}.toml"));
        self.guard_write(&self.fx.root, &f);
        fs::write(&f, format!("id = \"{id}\"\n{body}")).unwrap();
        self.conductor(&["record", "put", kind, "--file", f.to_str().unwrap()])
    }

    fn render(&self, kind_and_id: &[&str], page: &str) -> serde_json::Value {
        let out = self.fx.root.join(".bf/pages").join(page);
        let mut a = vec!["render"];
        a.extend_from_slice(kind_and_id);
        let (t, o) = (
            self.template.to_string_lossy().to_string(),
            out.to_string_lossy().to_string(),
        );
        a.extend(["--template", &t, "--out", &o]);
        self.conductor(&a);
        let html = fs::read_to_string(&out).unwrap();
        let open = "id=\"bf-data\">";
        let start = html.find(open).unwrap() + open.len();
        let end = html[start..].find("</script>").unwrap();
        let v: Value = serde_json::from_str(&html[start..start + end]).unwrap();
        assert_contract(page, &v);
        v
    }

    fn check_command(&self, check: &str) -> Vec<String> {
        let c = self.conductor(&["record", "get", check, "--json"]).json();
        c["command"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .map(|x| x.to_string())
            .collect()
    }
}

#[allow(dead_code)]
struct Ids {
    r1: String,
    r2: String,
    s1: String,
    j1: String,
    c1: String,
    t1: String,
    th1: String,
    d1: String,
    i1: String,
    u1: String,
    u2: String,
    h1: String,
    x1: String,
    x2: String,
    x3: String,
    k1: String,
}

/// Steps A of every stage: draft all the records of the design.
fn draft_everything(w: &Walk) -> Ids {
    let fx = w.fx;
    // idea
    let i1 = w.new_id("idea");
    w.put(
        "idea",
        &i1,
        "kind = \"idea\"\ntitle = \"A tiny greeter\"\ntext = \"Says hello and goodbye. [desc]\"\n",
    );
    let idea_md = fx.root.join("factory/idea.md");
    w.guard_write(&fx.root, &idea_md);
    fs::write(&idea_md, "# A tiny greeter\n\nSays hello and goodbye. [desc]\n\nLeft out of v1: other languages. [assumption]\n").unwrap();
    let d1 = w.new_id("decision");
    w.put("decision", &d1, "kind = \"decision\"\ntitle = \"English only in v1.\"\nchosen = \"English\"\nrejected = \"Several languages: too much for a first version.\"\n");
    // requirements
    let (r1, r2) = (w.new_id("requirement"), w.new_id("requirement"));
    w.put("requirement", &r1, "kind = \"requirement\"\ntitle = \"A visitor is greeted.\"\nacceptance = [\"src/hello.txt exists\"]\nlabel = \"core\"\n");
    w.put("requirement", &r2, "kind = \"requirement\"\ntitle = \"A visitor is told goodbye.\"\nacceptance = [\"src/bye.txt exists\"]\n");
    // ui
    let (s1, j1) = (w.new_id("screen"), w.new_id("journey"));
    w.put(
        "screen",
        &s1,
        "kind = \"screen\"\ntitle = \"The greeting screen shows one line.\"\n",
    );
    w.put(
        "journey",
        &j1,
        "kind = \"journey\"\ntitle = \"A visitor arrives, reads the greeting, leaves.\"\n",
    );
    // architecture
    let (c1, t1, th1) = (
        w.new_id("component"),
        w.new_id("target"),
        w.new_id("threat"),
    );
    w.put(
        "component",
        &c1,
        "kind = \"component\"\ntitle = \"The messages module holds both texts.\"\n",
    );
    w.put(
        "target",
        &t1,
        "kind = \"target\"\ntitle = \"Both texts load in under 50 ms.\"\nmeasure = \"50 ms\"\n",
    );
    w.put("threat", &th1, "kind = \"threat\"\ntitle = \"A text could be changed by a visitor.\"\nmitigation = \"Texts are read-only files.\"\n");
    // plan
    let (u1, u2, h1) = (w.new_id("unit"), w.new_id("unit"), w.new_id("handoff"));
    let (x1, x2, x3, k1) = (
        w.new_id("check"),
        w.new_id("check"),
        w.new_id("check"),
        w.new_id("checkpoint"),
    );
    w.put("unit", &u1, &format!("kind = \"unit\"\ntitle = \"The greeting\"\ndelivers = [\"{r1}\", \"{s1}\", \"{c1}\", \"{d1}\"]\nfiles = [\"src/hello.txt\"]\ncheckpoint = \"{k1}\"\nestimate = 2\n"));
    w.put("unit", &u2, &format!("kind = \"unit\"\ntitle = \"The goodbye\"\ndelivers = [\"{r2}\", \"{j1}\", \"{t1}\", \"{th1}\"]\nfiles = [\"src/bye.txt\"]\ndepends_on = [\"{u1}\"]\ncheckpoint = \"{k1}\"\nestimate = 1\n"));
    w.put("handoff", &h1, &format!("kind = \"handoff\"\nfrom = \"{u1}\"\nto = \"{u2}\"\nthing = \"the greeting wording\"\ncontract = \"Same tone as the greeting.\"\n"));
    w.put("check", &x1, &format!("kind = \"check\"\ntitle = \"The greeting file exists\"\nhow = \"command\"\ncommand = \"test -f src/hello.txt\"\nproves = [\"{r1}\", \"{s1}\", \"{c1}\", \"{d1}\", \"{h1}\"]\nscope = \"unit\"\nunit = \"{u1}\"\n"));
    w.put("check", &x2, &format!("kind = \"check\"\ntitle = \"The goodbye file exists\"\nhow = \"command\"\ncommand = \"test -f src/bye.txt\"\nproves = [\"{r2}\", \"{j1}\", \"{t1}\", \"{th1}\"]\nscope = \"unit\"\nunit = \"{u2}\"\n"));
    w.put("check", &x3, &format!("kind = \"check\"\ntitle = \"Both files exist\"\nhow = \"command\"\ncommand = \"ls src/hello.txt src/bye.txt\"\nproves = [\"{r1}\", \"{r2}\", \"{s1}\", \"{j1}\", \"{c1}\", \"{t1}\", \"{th1}\", \"{d1}\", \"{h1}\"]\nscope = \"checkpoint\"\n"));
    w.put("checkpoint", &k1, &format!("kind = \"checkpoint\"\ntitle = \"Both messages work\"\nafter = [\"{u1}\", \"{u2}\"]\nproves = [\"{r1}\", \"{r2}\", \"{s1}\", \"{j1}\", \"{c1}\", \"{t1}\", \"{th1}\", \"{d1}\", \"{h1}\"]\nchecks = [\"{x3}\"]\n"));
    Ids {
        r1,
        r2,
        s1,
        j1,
        c1,
        t1,
        th1,
        d1,
        i1,
        u1,
        u2,
        h1,
        x1,
        x2,
        x3,
        k1,
    }
}

#[test]
fn the_conductor_builder_reviewer_and_verifier_follow_their_instructions_under_a_test_grant() {
    let fx = Fx::new();
    let w = Walk::new(&fx);
    // The shipped policy has role tokens on; the fixture turns them off for tests that do not need them.
    fx.set_policy("tokens = false", "tokens = true");
    fx.set_policy("in_order = false", "in_order = true");

    // Start or resume: `bf status --json`.
    let st = w.conductor(&["status", "--json"]).json();
    assert_eq!(st["run"], Value::Null);

    let ids = draft_everything(&w);
    assert_eq!(
        (
            ids.r1.as_str(),
            ids.u1.as_str(),
            ids.x3.as_str(),
            ids.k1.as_str()
        ),
        ("R-1", "U-1", "X-3", "K-1")
    );
    w.conductor(&["record", "list"]);
    w.conductor(&["record", "get", &ids.r1]);

    // Step B: ask first. A JSON list in the `questions` shape, `bf ask --file`, render, answer, `bf questions --open`.
    let q = fx.root.join(".bf/drafts/q.json");
    fs::write(&q, json!([{"title": "Which tone?", "context": "The greeter speaks to visitors.", "kind": "single", "stage": "requirements",
        "options": [{"key": "A", "text": "Warm", "proposed": true, "why": "Friendly."}, {"key": "B", "text": "Plain"}]}]).to_string()).unwrap();
    w.conductor(&["ask", "--file", q.to_str().unwrap()]);
    let refused = w.bf(&fx.root, &["gate", "open", "requirements"]);
    assert_eq!(
        refused.code, 2,
        "D-19: a stage's gate waits for its questions"
    );
    w.render(&["questions"], "questions.html");
    let a = fx.root.join(".bf/drafts/a1.json");
    fs::write(
        &a,
        json!({"choice": "A", "note": "warm please"}).to_string(),
    )
    .unwrap();
    fx.human("Done, I filled in the form.");
    w.conductor(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    assert_eq!(
        w.conductor(&["questions", "--open", "--json"])
            .json()
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Test mode, in the order the skill gives: all records drafted, `bf plan check`, the grant right after
    // the human's message, `bf run start`, then each gate in order, approved with --simulated.
    w.conductor(&["plan", "check"]);
    fx.human("Yes, I grant a test run. Simulate the approvals.");
    w.conductor(&[
        "test-mode",
        "grant",
        "--quote",
        "Yes, I grant a test run. Simulate the approvals.",
    ]);
    w.conductor(&["run", "start"]);
    let mut gate_ids = Vec::new();
    for kind in ["idea", "requirements", "ui", "architecture", "plan"] {
        let gid = w
            .conductor(&["gate", "open", kind])
            .stdout
            .trim()
            .to_string();
        assert_eq!(gid, format!("{kind}-r1"));
        w.render(&["gate", &gid], &format!("gate-{gid}.html"));
        w.conductor(&["gate", "approve", &gid, "--simulated"]);
        gate_ids.push(gid);
    }
    let st = w.conductor(&["status", "--json"]).json();
    assert!(st["gates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|g| g["status"] == "approved" && g["simulated"] == true));

    // Build phase: the `bf next --json` loop.
    let mut seen: Vec<String> = Vec::new();
    let mut findings_round_done = false;
    let mut idle_reason = Value::Null;
    for _ in 0..40 {
        let n = w.conductor(&["next", "--json"]).json();
        if n["can_start"] == false {
            assert_eq!(
                n["human_needed"], false,
                "nothing should wait for the human in a test run: {n}"
            );
            idle_reason = n["idle_reason"].clone();
            break;
        }
        for d in n["directives"].as_array().unwrap() {
            let (role, action) = (d["role"].as_str().unwrap(), d["action"].as_str().unwrap());
            seen.push(format!("{role}:{action}"));
            for k in ["tier", "working_copy", "brief", "limits"] {
                if role != "engine" {
                    assert!(!d[k].is_null(), "directive lacks `{k}`: {d}");
                }
            }
            match (role, action) {
                ("builder", "start-unit" | "build" | "fix") => {
                    let u = d["unit"].as_str().unwrap();
                    // Starting a worker: `bf unit start <U>` first (resumes), then `bf brief <U> --role builder`.
                    let wc = PathBuf::from(
                        w.conductor(&["unit", "start", u, "--json"]).json()["working_copy"]
                            .as_str()
                            .unwrap(),
                    );
                    assert_eq!(wc, PathBuf::from(d["working_copy"].as_str().unwrap()));
                    let brief = PathBuf::from(
                        w.conductor(&["brief", u, "--role", "builder"])
                            .stdout
                            .trim(),
                    );
                    assert_eq!(brief, PathBuf::from(d["brief"].as_str().unwrap()));
                    let text = fs::read_to_string(&brief).unwrap();
                    if action == "fix" {
                        assert!(
                            text.contains("Wrong tone") && text.contains("## Findings"),
                            "the fix brief lists the findings:\n{text}"
                        );
                    }
                    // The builder works in the working copy: a real change, past the hook.
                    let (file, check) = if u == ids.u1 {
                        ("src/hello.txt", &ids.x1)
                    } else {
                        ("src/bye.txt", &ids.x2)
                    };
                    let path = wc.join(file);
                    w.guard_write(&wc, &path);
                    fs::create_dir_all(wc.join("src")).unwrap();
                    fs::write(
                        &path,
                        if action == "fix" {
                            "Hello, warmly!\n"
                        } else {
                            "Hello.\n"
                        }
                        .replace("Hello", if u == ids.u1 { "Hello" } else { "Goodbye" }),
                    )
                    .unwrap();
                    let cmd = w.check_command(check);
                    let mut a = vec!["evidence", "run", check.as_str(), "--unit", u, "--"];
                    a.extend(cmd.iter().map(|x| x.as_str()));
                    w.ok(&wc, &a);
                    let result = if action == "fix" { "fixed" } else { "built" };
                    // With role tokens on (the default) a report carries the token of the directive;
                    // the builder's prompt must pass it on.
                    let tok = &d["token"].as_str().unwrap().to_string();
                    w.ok(&wc, &["report", u, "--result", result, "--token", tok]);
                }
                ("reviewer", "review") => {
                    let u = d["unit"].as_str().unwrap();
                    let wc = PathBuf::from(d["working_copy"].as_str().unwrap());
                    let tok = &d["token"].as_str().unwrap().to_string();
                    let brief = PathBuf::from(
                        w.conductor(&["brief", u, "--role", "reviewer"])
                            .stdout
                            .trim(),
                    );
                    assert!(fs::read_to_string(brief).unwrap().contains("## Receipts"));
                    // The reviewer's reading: receipts, then the diff.
                    w.ok(&wc, &["evidence", "verify", "--unit", u]);
                    w.guard_bash(&wc, "git status --short");
                    w.guard_bash(&wc, "git diff");
                    if u == ids.u1 && !findings_round_done {
                        // Findings: a JSON array in a temporary file outside the project, written with a heredoc.
                        let tmp = tempfile::tempdir().unwrap();
                        let f = tmp.path().join("findings.json");
                        w.guard_bash(
                            &wc,
                            &format!(
                                "cat > {} <<'EOF'\n[{{\"title\":\"Wrong tone\"}}]\nEOF",
                                f.display()
                            ),
                        );
                        fs::write(&f, json!([{"title": "Wrong tone", "severity": "medium", "where": "src/hello.txt:1", "action": "Make it warm"}]).to_string()).unwrap();
                        let o = w.ok(
                            &wc,
                            &[
                                "report",
                                u,
                                "--result",
                                "findings",
                                "--role",
                                "reviewer",
                                "--token",
                                tok,
                                "--findings",
                                f.to_str().unwrap(),
                                "--json",
                            ],
                        );
                        assert_eq!(o.json()["state"], "fixing");
                        findings_round_done = true;
                    } else {
                        // Nothing found: the same command with no file.
                        let o = w.ok(
                            &wc,
                            &[
                                "report", u, "--result", "findings", "--role", "reviewer",
                                "--token", tok, "--json",
                            ],
                        );
                        assert_eq!(o.json()["state"], "reviewed");
                    }
                }
                ("engine", "land") => {
                    let u = d["unit"].as_str().unwrap();
                    assert_eq!(d["command"], format!("bf unit land {u}"));
                    w.conductor(&["unit", "land", u]);
                }
                ("verifier", "run-checkpoint-checks") => {
                    let k = d["checkpoint"].as_str().unwrap();
                    let brief = PathBuf::from(
                        w.conductor(&["brief", k, "--role", "verifier"])
                            .stdout
                            .trim(),
                    );
                    assert!(fs::read_to_string(brief)
                        .unwrap()
                        .contains("blind verifier"));
                    let wc = PathBuf::from(d["working_copy"].as_str().unwrap());
                    for c in d["checks"].as_array().unwrap() {
                        let c = c.as_str().unwrap();
                        let cmd = w.check_command(c);
                        let mut a = vec!["evidence", "run", c, "--"];
                        a.extend(cmd.iter().map(|x| x.as_str()));
                        // A receipt is for the content of the folder the command ran in. Run once from the
                        // project folder, the command sees the main checkout's files and its receipt does not
                        // count for the checkpoint; the checkpoint's working copy (named in the brief) is the
                        // folder that counts.
                        if !seen.iter().any(|s| s == "verifier:wrong-folder-tried") {
                            seen.push("verifier:wrong-folder-tried".into());
                            let wrong = w.bf(&fx.root, &a);
                            assert_eq!(wrong.code, 1, "{}", wrong.all()); // the command fails in the main checkout: a failing receipt
                            let n2 = w.conductor(&["next", "--json"]).json();
                            assert_eq!(
                                n2["directives"][0]["action"], "run-checkpoint-checks",
                                "a receipt from the wrong folder must not count: {n2}"
                            );
                        }
                        w.ok(&wc, &a);
                    }
                }
                ("engine", "close-checkpoint") => {
                    let k = d["checkpoint"].as_str().unwrap();
                    w.conductor(&["checkpoint", "close", k]);
                }
                other => panic!("the walkthrough has no step for {other:?}: {d}"),
            }
        }
    }
    assert!(
        idle_reason.as_str().unwrap_or("").starts_with("run done"),
        "{idle_reason}"
    );
    for want in [
        "builder:start-unit",
        "reviewer:review",
        "builder:fix",
        "engine:land",
        "verifier:run-checkpoint-checks",
        "engine:close-checkpoint",
    ] {
        assert!(
            seen.iter().any(|s| s == want),
            "the loop never saw {want}: {seen:?}"
        );
    }

    // End: the ledger passes, the status summarises, and the main branch is untouched by a test run.
    w.conductor(&["ledger", "check"]);
    let st = w.conductor(&["status", "--json"]).json();
    assert!(st["units"]
        .as_array()
        .unwrap()
        .iter()
        .all(|u| u["state"] == "landed"));
    assert_eq!(st["checkpoints_closed"][0], "K-1");
    let o = w.bf(&fx.root, &["checkpoint", "close", "K-1", "--merge"]);
    assert_eq!(o.code, 2, "a test run never reaches main");
    assert!(!fx.root.join("src/hello.txt").exists());

    // Every page the skill names renders with the real template and validates; the index links match the names.
    let idx = w.render(&["index"], "index.html");
    let mut hrefs: Vec<String> = Vec::new();
    for sec in idx["sections"].as_array().unwrap() {
        for r in sec["rows"].as_array().unwrap() {
            hrefs.push(r["href"].as_str().unwrap().to_string());
        }
    }
    assert!(hrefs.len() >= 9, "{hrefs:?}");
    for h in &hrefs {
        let stem = h.trim_end_matches(".html");
        let args: Vec<&str> = if let Some(g) = stem.strip_prefix("gate-") {
            vec!["gate", g]
        } else if let Some(k) = stem.strip_prefix("checkpoint-") {
            vec!["checkpoint", k]
        } else {
            vec![stem]
        };
        w.render(&args, h);
        assert!(fx.root.join(".bf/pages").join(h).exists(), "{h}");
    }
    for h in [
        "questions.html",
        "plan.html",
        "run.html",
        "ledger.html",
        "coverage.html",
    ] {
        assert!(
            hrefs.iter().any(|x| x == h),
            "the index lacks {h}: {hrefs:?}"
        );
    }
    assert!(ids.i1 == "I-1" && !gate_ids.is_empty());
}

#[test]
fn the_design_phase_with_real_approvals_rejections_and_rounds() {
    let fx = Fx::shipped();
    let w = Walk::new(&fx);
    // The shipped policy as it is: role tokens on, the transcript only noted. The human's messages
    // arrive the way Claude Code's hook delivers them, before the transcript holds them. Nothing in
    // this test edits the policy.
    let shipped = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    assert!(shipped.contains("transcript_match = \"soft\""), "{shipped}");
    // Idea.
    let i1 = w.new_id("idea");
    w.put("idea", &i1, "kind = \"idea\"\ntitle = \"A tiny greeter\"\n");
    fs::write(
        fx.root.join("factory/idea.md"),
        "# A tiny greeter\n\nSays hello. [desc]\n",
    )
    .unwrap();
    let gid = w
        .conductor(&["gate", "open", "idea"])
        .stdout
        .trim()
        .to_string();
    w.render(&["gate", &gid], "gate-idea-r1.html");
    // Step D/E: the conductor must not approve before the human has written.
    assert_eq!(
        w.bf(
            &fx.root,
            &[
                "gate",
                "approve",
                &gid,
                "--quote",
                "Yes, I grant a test run. Simulate the approvals."
            ]
        )
        .code,
        2
    );
    fx.human("Approved. Go on.");
    w.conductor(&["gate", "approve", &gid, "--quote", "Approved. Go on."]);

    // Requirements: the human asks for changes. `bf gate reject`, revise the changed record, open again.
    let r1 = w.new_id("requirement");
    w.put(
        "requirement",
        &r1,
        "kind = \"requirement\"\ntitle = \"A visitor is greeted.\"\n",
    );
    let g1 = w
        .conductor(&["gate", "open", "requirements"])
        .stdout
        .trim()
        .to_string();
    assert_eq!(g1, "requirements-r1");
    fx.human("No, make the greeting say the visitor's name.");
    w.conductor(&[
        "gate",
        "reject",
        &g1,
        "--quote",
        "No, make the greeting say the visitor's name.",
    ]);
    // Revising a record that already exists means putting it again with `--replace`; without it the engine
    // refuses to overwrite a record whose content differs.
    let f = fx.root.join(".bf/drafts/R-1.toml");
    fs::write(
        &f,
        format!(
            "id = \"{r1}\"\nkind = \"requirement\"\ntitle = \"A visitor is greeted by name.\"\n"
        ),
    )
    .unwrap();
    let o = w.bf(
        &fx.root,
        &[
            "record",
            "put",
            "requirement",
            "--file",
            f.to_str().unwrap(),
        ],
    );
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("--replace"));
    w.conductor(&[
        "record",
        "put",
        "requirement",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let g2 = w
        .conductor(&["gate", "open", "requirements"])
        .stdout
        .trim()
        .to_string();
    assert_eq!(g2, "requirements-r2");
    let v = w.render(&["gate", &g2], "gate-requirements-r2.html");
    assert_eq!(v["page"]["round"]["n"], 2);
    assert_eq!(v["page"]["round"]["changed"], 1);
    fx.human("Yes, that is what I meant.");
    w.conductor(&[
        "gate",
        "approve",
        &g2,
        "--quote",
        "Yes, that is what I meant.",
    ]);

    // One typed message approves one thing.
    let s1 = w.new_id("screen");
    w.put(
        "screen",
        &s1,
        "kind = \"screen\"\ntitle = \"The greeting screen.\"\n",
    );
    let c1 = w.new_id("component");
    w.put(
        "component",
        &c1,
        "kind = \"component\"\ntitle = \"The messages module.\"\n",
    );
    let d1 = w.new_id("decision");
    w.put(
        "decision",
        &d1,
        "kind = \"decision\"\ntitle = \"Keep it in one module.\"\nchosen = \"one module\"\nrejected = \"two\"\n",
    );
    // The gates open in order: the architecture waits for the ui.
    let o = w.bf(&fx.root, &["gate", "open", "architecture"]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("`ui` gate comes first"), "{}", o.stderr);
    w.conductor(&["gate", "open", "ui"]);
    fx.human("The screen looks good, approve ui and architecture.");
    w.conductor(&[
        "gate",
        "approve",
        "ui-r1",
        "--quote",
        "The screen looks good, approve ui and architecture.",
    ]);
    w.conductor(&["gate", "open", "architecture"]);
    let o = w.bf(
        &fx.root,
        &[
            "gate",
            "approve",
            "architecture-r1",
            "--quote",
            "The screen looks good, approve ui and architecture.",
        ],
    );
    assert_eq!(o.code, 2, "the second approval needs its own message");
    assert_four_parts(&o.stderr);
    fx.human("And approve the architecture too.");
    w.conductor(&[
        "gate",
        "approve",
        "architecture-r1",
        "--quote",
        "And approve the architecture too.",
    ]);

    // Plan: `bf gate open plan` refuses while `bf plan check` fails, then passes once the plan is whole.
    let (u1, x1, k1) = (w.new_id("unit"), w.new_id("check"), w.new_id("checkpoint"));
    w.put("unit", &u1, &format!("kind = \"unit\"\ntitle = \"All of it\"\ndelivers = [\"{r1}\", \"{s1}\", \"{c1}\", \"{d1}\"]\nfiles = [\"src/\"]\ncheckpoint = \"{k1}\"\n"));
    let o = w.bf(&fx.root, &["gate", "open", "plan"]);
    assert_eq!(o.code, 2);
    assert!(
        o.stderr.contains("plan has") || o.stderr.contains("problem"),
        "{}",
        o.stderr
    );
    w.put("check", &x1, &format!("kind = \"check\"\ntitle = \"It exists\"\nhow = \"command\"\ncommand = \"test -f src/a.txt\"\nproves = [\"{r1}\", \"{s1}\", \"{c1}\", \"{d1}\"]\nscope = \"unit\"\nunit = \"{u1}\"\n"));
    w.put("checkpoint", &k1, &format!("kind = \"checkpoint\"\ntitle = \"Done\"\nafter = [\"{u1}\"]\nproves = [\"{r1}\", \"{s1}\", \"{c1}\", \"{d1}\"]\nchecks = [\"{x1}\"]\n"));
    w.conductor(&["plan", "check"]);
    let gp = w
        .conductor(&["gate", "open", "plan"])
        .stdout
        .trim()
        .to_string();
    let v = w.render(&["gate", &gp], "gate-plan-r1.html");
    assert!(v["sections"].to_string().contains("\"coverage\""));
    fx.human("The plan is fine. Approved.");
    w.conductor(&[
        "gate",
        "approve",
        &gp,
        "--quote",
        "The plan is fine. Approved.",
    ]);

    // Build phase begins: plan check, run start, next offers the first worker.
    w.conductor(&["plan", "check"]);
    w.conductor(&["run", "start"]);
    let n = w.conductor(&["next", "--json"]).json();
    assert_eq!(n["can_start"], true);
    assert_eq!(n["directives"][0]["role"], "builder");
    // A reviewed unit lands; the checkpoint closes; the conductor never passes --merge.
    let wc = PathBuf::from(
        w.conductor(&["unit", "start", &u1, "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "x\n").unwrap();
    w.ok(
        &wc,
        &[
            "evidence",
            "run",
            &x1,
            "--unit",
            &u1,
            "--",
            "test",
            "-f",
            "src/a.txt",
        ],
    );
    let tb = fx.take_token(&u1, "builder");
    w.ok(
        &wc,
        &["report", &u1, "--result", "built", "--token", tb.trim()],
    );
    w.conductor(&["next", "--json"]);
    let tr = fx.take_token(&u1, "reviewer");
    w.ok(
        &wc,
        &[
            "report",
            &u1,
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--token",
            tr.trim(),
        ],
    );
    w.conductor(&["unit", "land", &u1]);
    let n = w.conductor(&["next", "--json"]).json();
    // The unit's check is also the checkpoint's check and the merged content equals the unit's, so the
    // engine offers the closure itself.
    assert_eq!(n["directives"][0]["role"], "engine", "{n}");
    assert_eq!(n["directives"][0]["action"], "close-checkpoint");
    w.conductor(&["checkpoint", "close", &k1]);
    let n = w.conductor(&["next", "--json"]).json();
    assert_eq!(n["can_start"], false);
    assert!(
        n["idle_reason"].as_str().unwrap().starts_with("run done"),
        "{n}"
    );
    // The conductor never passes --merge (SKILL.md); a real run is merged by the human.
    assert!(!fx.root.join("src/a.txt").exists());
    let gid = w
        .conductor(&["gate", "open", "checkpoint-step", "--ref", &k1])
        .stdout
        .trim()
        .to_string();
    fx.human("Merge it, I approve.");
    w.conductor(&["gate", "approve", &gid, "--quote", "Merge it, I approve."]);
    w.ok(&fx.root, &["checkpoint", "close", &k1, "--merge"]); // what the human may do
    assert!(fx.root.join("src/a.txt").exists());
    // From `bf init` to a merged checkpoint, and the policy file is as the engine first accepted it.
    assert_eq!(
        fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap(),
        shipped
    );
    let doctor = fx.bf(&["doctor"]);
    assert!(
        !doctor.stdout.contains("PROBLEM: policy"),
        "{}",
        doctor.stdout
    );
}

#[test]
fn a_checkpoint_with_a_human_step_follows_the_skill() {
    let fx = Fx::new();
    let w = Walk::new(&fx);
    fx.tiny();
    // The planner added a human step to the checkpoint.
    let f = fx.root.join(".bf/drafts/K-1.toml");
    fs::write(&f, "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\",\"H-1\"]\nchecks=[\"X-3\"]\nhuman_step=\"Read both messages aloud\"\non_human_step=\"stop\"\n").unwrap();
    w.conductor(&[
        "record",
        "put",
        "checkpoint",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let f = fx.root.join(".bf/drafts/X-3.toml");
    fs::write(&f, "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"ls src/hello.txt src/bye.txt\"\nproves=[\"R-1\",\"R-2\",\"C-1\",\"H-1\"]\nscope=\"checkpoint\"\n").unwrap();
    w.conductor(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx.must_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "run",
            "X-3",
            "--",
            "ls",
            "src/hello.txt",
            "src/bye.txt",
        ],
    );
    // Checks met, human step open: `bf next` names it; closing refuses until the step gate is approved.
    let n = w.conductor(&["next", "--json"]).json();
    let d = &n["directives"][0];
    assert_eq!(
        (
            d["role"].as_str(),
            d["action"].as_str(),
            d["gate_kind"].as_str()
        ),
        (Some("human"), Some("open-gate"), Some("checkpoint-step"))
    );
    assert_eq!(d["on_human_step"], "stop");
    assert_eq!(w.bf(&fx.root, &["checkpoint", "close", "K-1"]).code, 2);
    let gid = w
        .conductor(&["gate", "open", "checkpoint-step", "--ref", "K-1"])
        .stdout
        .trim()
        .to_string();
    assert_eq!(gid, "checkpoint-step-K-1-r1");
    let v = w.render(&["gate", &gid], &format!("gate-{gid}.html"));
    assert_eq!(v["sections"][0]["rows"][0]["needs_you"], true);
    let n = w.conductor(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "decide-gate");
    assert_eq!(n["directives"][0]["gate"], gid.as_str());
    fx.human("I read them aloud, both are fine.");
    // In a test run the step gate is approved as a simulation (the run holds the grant).
    w.conductor(&["gate", "approve", &gid, "--simulated"]);
    w.conductor(&["checkpoint", "close", "K-1"]);
}
