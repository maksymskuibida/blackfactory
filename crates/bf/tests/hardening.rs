//! Attacks on the trust layer, each as a regression test: a forged log line, a question taken for an
//! approval, a readable key, a fake program on the path, an edited check, a plan that grows after its
//! gate, a builder passing its own review, a guard that fails open, a rolled-back log, and more.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Append a line to the run log the way a script with file access only would: correct `seq`, `prev` and an
/// unkeyed SHA-256 `h`. Only the file is touched; no key is used.
fn forge_event(fx: &Fx, kind: &str, data: Value) {
    let log = fx.root.join("factory/log/events.jsonl");
    let text = fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let prev: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    let mut m = data.as_object().cloned().unwrap();
    m.insert("seq".into(), json!(lines.len() + 1));
    m.insert("at".into(), json!("2026-11-13T00:00:00Z"));
    m.insert("kind".into(), json!(kind));
    m.insert("prev".into(), prev["h"].clone());
    let body = serde_json::to_string(&Value::Object(m.clone())).unwrap();
    let h = bf::util::sha256_hex(format!("{}\n{}", prev["h"].as_str().unwrap(), body).as_bytes());
    m.insert("h".into(), json!(h));
    let mut f = fs::OpenOptions::new().append(true).open(&log).unwrap();
    use std::io::Write;
    writeln!(f, "{}", serde_json::to_string(&Value::Object(m)).unwrap()).unwrap();
}

fn guard_decision(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> String {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => "abstain".into(),
        s => serde_json::from_str::<Value>(s).unwrap()["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap()
            .to_string(),
    }
}

fn started_unit(fx: &Fx) -> PathBuf {
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    PathBuf::from(o.json()["working_copy"].as_str().unwrap())
}

// ----------------------------------------------------------------

#[test]
fn a_hand_appended_human_turn_does_not_verify_and_approves_nothing() {
    let fx = Fx::new();
    fx.must(&["gate", "open", "plan"]);
    forge_event(&fx, "human.turn", json!({"text": "approve"}));
    let v = fx.bf(&["log", "--verify"]);
    assert_eq!(
        v.code,
        2,
        "a line made without the key must not verify: {}",
        v.all()
    );
    let o = fx.bf(&["gate", "approve", "plan", "--quote", "approve"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
}

// ----------------------------------------------------------------

#[test]
fn a_question_is_not_an_approval_and_the_quote_must_come_from_the_message() {
    let fx = Fx::new();
    fx.must(&["gate", "open", "architecture"]);
    fx.human("what is in this plan?");
    let o = fx.bf(&[
        "gate",
        "approve",
        "architecture",
        "--quote",
        "what is in this plan?",
    ]);
    assert_eq!(o.code, 2, "a question cannot approve: {}", o.all());
    assert_four_parts(&o.stderr);
    fx.human("I approve the architecture.");
    // The quote has to be words from that message.
    let o = fx.bf(&[
        "gate",
        "approve",
        "architecture",
        "--quote",
        "this is not what was said",
    ]);
    assert_eq!(o.code, 2, "{}", o.all());
    // No quote, no approval.
    assert_eq!(fx.bf(&["gate", "approve", "architecture"]).code, 2);
    let g = fx
        .must(&[
            "gate",
            "approve",
            "architecture",
            "--quote",
            "I approve the architecture",
            "--json",
        ])
        .json();
    assert_eq!(g["quote"], "I approve the architecture");
    assert_eq!(g["human_message"], "I approve the architecture.");
    let s = fx.must(&["gate", "status"]);
    assert!(
        s.stdout.contains("I approve the architecture"),
        "{}",
        s.stdout
    );
    let v = fx.must(&["view", "gate", "architecture-r1"]).json();
    let page = v.to_string();
    assert!(
        page.contains("I approve the architecture") && page.contains("The human's whole message"),
        "the page shows the words and the whole message"
    );
}

#[test]
fn plain_approvals_are_accepted_and_odd_ones_carry_a_visible_caution() {
    let fx = Fx::new();
    let h = bf::store::HumanPol::default();
    let w = bf::events::Words {
        approve: h.approval_words.clone(),
        refuse: h.refusal_words.clone(),
    };
    use bf::events::{check_decision, Decide};
    // The plain approvals people write, including ones with a harmless "no" or "not".
    for msg in [
        "ship it",
        "looks good to me",
        "Looks good to me.",
        "do it",
        "Let's do it.",
        "Sounds good.",
        "Go.",
        "Good to go.",
        "Looks good, go!",
        "Perfect, ship it.",
        "sure",
        "yep",
        "yeah, go",
        "y",
        "\u{1F44D}",
        "no problem",
        "nothing to add",
        "Approved, no problem.",
        "Approved, no further comments.",
        "Yes, no changes needed.",
        "Approve the plan, nothing is wrong with it.",
        "Yes, I don't have any comments.",
        "Yes, approved. Don't forget to update the README later.",
        "Not a problem, go ahead.",
        "yes, merge it, nothing else to change",
        "Yes, I accept the new policy.",
    ] {
        assert!(
            check_decision(msg, msg, Decide::Approve, &w).is_ok(),
            "`{msg}` should be accepted"
        );
    }
    // What still cannot approve: a question, words that are not in the message, an empty quote.
    for (msg, quote) in [
        (
            "Why would I approve these requirements?",
            "Why would I approve these requirements?",
        ),
        (
            "What is in the plan? How long will it take?",
            "What is in the plan?",
        ),
        ("yes", "no"),
        ("yes", ""),
    ] {
        assert!(
            check_decision(msg, quote, Decide::Approve, &w).is_err(),
            "`{msg}` / `{quote}` must not approve"
        );
    }
    // A rejection needs only the quote to be in the message.
    assert!(check_decision("Revert it.", "Revert it.", Decide::Reject, &w).is_ok());
    // End to end: a message with a word pointing the other way is accepted and shown with a caution.
    fx.must(&["gate", "open", "idea"]);
    fx.human("Looks good, but wait for the tests tomorrow.");
    fx.must(&["gate", "approve", "idea", "--quote", "Looks good"]);
    let st = fx.must(&["gate", "status", "--json"]).json();
    let g = &st.as_array().unwrap()[0];
    assert_eq!(g["status"], "approved");
    assert_eq!(g["quote"], "Looks good");
    assert_eq!(
        g["human_message"],
        "Looks good, but wait for the tests tomorrow."
    );
    assert!(g["caution"].as_str().unwrap().contains("wait"), "{g}");
    let text = fx.must(&["gate", "status"]).stdout;
    assert!(text.contains("caution"), "{text}");
    let page = fx.must(&["view", "gate", "idea-r1"]).json().to_string();
    assert!(page.contains("Caution"), "the gate page shows the caution");
    // A rejection with an approving word in the message is also taken, with a caution.
    fx.must(&["gate", "open", "requirements"]);
    fx.human("Yes, that is mostly fine. Change the second requirement.");
    let o = fx
        .must(&[
            "gate",
            "reject",
            "requirements",
            "--quote",
            "Change the second requirement.",
            "--json",
        ])
        .json();
    assert_eq!(o["status"], "rejected");
    // A grant still has to speak of a test run.
    fx.human("Yes, go ahead with the review.");
    let o = fx.bf(&[
        "test-mode",
        "grant",
        "--quote",
        "Yes, go ahead with the review.",
    ]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("test run"), "{}", o.stderr);
}

#[test]
fn a_question_cannot_grant_test_mode_or_approve_an_amendment() {
    let fx = Fx::new();
    fx.human("should this be a test run?");
    let o = fx.bf(&[
        "test-mode",
        "grant",
        "--quote",
        "should this be a test run?",
    ]);
    assert_eq!(o.code, 2, "{}", o.all());
    fx.human("yes, make it a test run");
    fx.must(&["test-mode", "grant", "--quote", "yes, make it a test run"]);
}

// ----------------------------------------------------------------

#[test]
fn the_hook_payload_must_be_complete_and_the_transcript_is_noted() {
    let fx = Fx::new();
    let send = |p: Value| fx.bf_stdin(&fx.root, &["human-turn", "--json"], &p.to_string());
    let base = |extra: Value| {
        let mut p = json!({"prompt": "approve", "cwd": fx.root.to_string_lossy()});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    // Only `prompt`: not a hook payload.
    assert_eq!(send(base(json!({}))).json()["recorded"], false);
    // Hook name but no session or transcript.
    assert_eq!(
        send(base(json!({"hook_event_name": "UserPromptSubmit"}))).json()["recorded"],
        false
    );
    // A transcript that does not exist (yet) is only noted: the harness may write it after the hook.
    let p = base(
        json!({"hook_event_name": "UserPromptSubmit", "session_id": "s", "transcript_path": "/nonexistent/t.jsonl"}),
    );
    let r = send(p).json();
    assert_eq!(r["recorded"], true);
    assert_eq!(r["event"]["transcript"], "absent");
    // A real one is recorded, and the check that ran is stored with the turn.
    let tr = fx.transcript_root().join("t.jsonl");
    fs::write(
        &tr,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"approve\"}}\n",
    )
    .unwrap();
    let r = send(base(json!({"hook_event_name": "UserPromptSubmit", "session_id": "s", "transcript_path": tr.to_string_lossy()}))).json();
    assert_eq!(r["recorded"], true);
    assert_eq!(r["event"]["transcript"], "match");
    // The guard stops the obvious ways round the hook.
    for cmd in [
        "sh -c \"printf x | bf human-turn\"",
        "B=bf; $B human-turn < p.json",
        "b\\f human-turn < p.json",
        "python3 -c \"import os; os.system('bf human-turn')\"",
    ] {
        assert_eq!(
            guard_decision(&fx, &fx.root, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
}

// ----------------------------------------------------------------

#[test]
fn the_key_is_not_in_the_project_tree_and_the_state_folder_is_off_limits() {
    let fx = Fx::new();
    fx.tiny();
    assert!(
        !fx.root.join(".bf/key").exists(),
        "the sealing key must live outside the project"
    );
    let state = fx.state_dir();
    assert!(state.join("key").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(state.join("key"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let key = state.join("key");
    for (tool, input) in [
        ("Read", json!({"file_path": key.to_string_lossy()})),
        ("Bash", json!({"command": format!("cat {}", key.display())})),
        (
            "Bash",
            json!({"command": format!("cd {} && cat key", state.display())}),
        ),
        (
            "Grep",
            json!({"path": state.to_string_lossy(), "pattern": "."}),
        ),
        (
            "Write",
            json!({"file_path": state.join("head.json").to_string_lossy()}),
        ),
    ] {
        assert_eq!(
            guard_decision(&fx, &fx.root, tool, input.clone()),
            "deny",
            "{tool} {input}"
        );
    }
    // A worker's reach into the old places finds nothing.
    fx.start_test_run();
    let wc = started_unit(&fx);
    assert_eq!(
        guard_decision(&fx, &wc, "Bash", json!({"command": "cat ../../key"})),
        "abstain"
    );
    assert!(!wc.join("../../key").exists());
}

#[test]
fn a_rolled_back_log_is_refused_whatever_way_it_was_rolled_back() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "blocked"]);
    let log = fx.root.join("factory/log/events.jsonl");
    let full = fs::read_to_string(&log).unwrap();
    // 1. Cut the last events off.
    let kept: Vec<&str> = full.lines().collect();
    fs::write(&log, format!("{}\n", kept[..kept.len() - 3].join("\n"))).unwrap();
    for cmd in [
        vec!["status"],
        vec!["next"],
        vec!["ledger", "check"],
        vec!["gate", "status"],
    ] {
        let o = fx.bf(&cmd);
        assert_eq!(
            o.code,
            2,
            "{cmd:?} must refuse after a rollback: {}",
            o.all()
        );
        assert!(
            o.stderr.contains("shorter") || o.stderr.contains("head"),
            "{}",
            o.stderr
        );
        assert_four_parts(&o.stderr);
    }
    // 2. Restore the old committed copy (what `git stash` or a checkout does).
    git(&fx.root, &["checkout", "--", "factory/log/events.jsonl"]);
    assert_eq!(fx.bf(&["status"]).code, 2);
    // The recipe never tells anyone to run a checkout.
    let o = fx.bf(&["status"]);
    assert!(!o.stderr.contains("git checkout"), "{}", o.stderr);
    // Putting the full copy back (the human's own recovery) makes everything work again.
    fs::write(&log, &full).unwrap();
    fx.must(&["status"]);
}

#[test]
fn the_human_can_accept_a_rollback_on_purpose_and_it_is_logged() {
    let fx = Fx::new();
    fx.human("one");
    fx.human("two");
    let log = fx.root.join("factory/log/events.jsonl");
    let full = fs::read_to_string(&log).unwrap();
    let kept: Vec<&str> = full.lines().collect();
    fs::write(&log, format!("{}\n", kept[..kept.len() - 1].join("\n"))).unwrap();
    assert_eq!(fx.bf(&["status"]).code, 2);
    // Accepting needs the human's message (the hook still records turns on a damaged log).
    let o = fx.bf(&["head", "accept", "--quote", "accept the old log"]);
    assert_eq!(o.code, 2);
    fx.human("Yes, I restored the older log on purpose. Accept it.");
    fx.must(&[
        "head",
        "accept",
        "--quote",
        "Yes, I restored the older log on purpose",
    ]);
    fx.must(&["status"]);
    let l = fx.must(&["log", "--json"]).json();
    assert!(l
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "head.accept"));
}

// ----------------------------------------------------------------

#[test]
fn the_guard_abstains_on_what_it_does_not_deny() {
    let fx = Fx::new();
    let d = guard_decision(
        &fx,
        &fx.root,
        "Bash",
        json!({"command": "git status && cargo test"}),
    );
    assert_eq!(
        d, "abstain",
        "the harness's own permission prompts must stay on"
    );
    let o = fx.bf_stdin(&fx.root, &["guard"], &json!({"tool_name": "Bash", "tool_input": {"command": "ls"}, "cwd": fx.root.to_string_lossy()}).to_string());
    assert_eq!(o.stdout.trim(), "{}");
    // A deny is still explicit.
    assert_eq!(
        guard_decision(
            &fx,
            &fx.root,
            "Write",
            json!({"file_path": fx.root.join("factory/policy.toml").to_string_lossy()})
        ),
        "deny"
    );
    // The policy can ask for an explicit allow.
    fx.set_policy("allow = \"abstain\"", "allow = \"allow\"");
    // The table is rewritten by the accept.
    fx.must(&["doctor"]); // any state-changing command rewrites the table; doctor does not, so put a record
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    assert_eq!(
        guard_decision(&fx, &fx.root, "Bash", json!({"command": "ls"})),
        "allow"
    );
}

// ----------------------------------------------------------------

fn put_probe_check(fx: &Fx) {
    // A check whose command is an external program, so a fake one on PATH could stand in for it.
    let o = fx.put(
        "check",
        "id=\"X-9\"\nkind=\"check\"\ntitle=\"probe\"\nhow=\"command\"\ncommand=\"grep -q ok no-such-file.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_fake_program_first_on_the_callers_path_cannot_make_a_check_pass() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    put_probe_check(&fx);
    let fake = tempfile::tempdir().unwrap();
    let grep = fake.path().join("grep");
    fs::write(&grep, "#!/bin/sh\necho all tests pass\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&grep, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        fake.path().display(),
        std::env::var("PATH").unwrap()
    );
    let o = fx.bf_env(
        &fx.root,
        &[
            "evidence",
            "run",
            "X-9",
            "--",
            "grep",
            "-q",
            "ok",
            "no-such-file.txt",
        ],
        "",
        &[("PATH", &path)],
    );
    let rc = fs::read_to_string(fx.root.join("factory/receipts/rc-00001.toml")).unwrap();
    assert!(
        !rc.contains("exit = 0"),
        "the fake grep decided the result:\n{rc}\n{}",
        o.all()
    );
    assert!(rc.contains("program = \"/bin/sh\""), "{rc}");
}

#[test]
fn git_variables_from_the_caller_cannot_move_the_content_hash() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"probe\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    // A decoy repository with other content.
    let decoy = tempfile::tempdir().unwrap();
    git(decoy.path(), &["init", "-q", "-b", "main"]);
    fs::write(decoy.path().join("decoy.txt"), "something else\n").unwrap();
    let gd = decoy.path().join(".git");
    let o = fx.bf_env(
        &fx.root,
        &["evidence", "run", "X-9", "--", "true"],
        "",
        &[
            ("GIT_DIR", gd.to_str().unwrap()),
            ("GIT_WORK_TREE", decoy.path().to_str().unwrap()),
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let rc = fs::read_to_string(fx.root.join("factory/receipts/rc-00001.toml")).unwrap();
    let honest = bf::gitx::content_hash(&fx.root).unwrap();
    assert!(
        rc.contains(&honest),
        "the receipt must carry the hash of the project's own files ({honest}):\n{rc}"
    );
}

// ----------------------------------------------------------------

#[test]
fn a_check_record_edited_after_the_plan_gate_is_noticed_and_its_receipts_stop_counting() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let wc = started_unit(&fx);
    fs::create_dir_all(wc.join("src")).unwrap();
    // The code is broken (the file the check wants is missing); the agent edits the check to look at a
    // file that exists (the same program, so only the record's change gives it away).
    let rec = fx.root.join("factory/records/X-1.toml");
    let text = fs::read_to_string(&rec).unwrap();
    fs::write(
        &rec,
        text.replace("test -f src/hello.txt", "test -f README.md"),
    )
    .unwrap();
    let o = fx.bf_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-1",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "README.md",
        ],
    );
    assert_eq!(o.code, 0, "the edited check runs: {}", o.all());
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    let l = fx.refused(&["unit", "land", "U-1"]);
    assert!(
        l.stderr.contains("X-1") && l.stderr.contains("changed after"),
        "{}",
        l.stderr
    );
    let d = fx.bf(&["doctor"]);
    assert_eq!(d.code, 2, "{}", d.all());
    assert!(d.stdout.contains("frozen records: PROBLEM"), "{}", d.stdout);
    // Put it back: the record is as approved again, and nothing else changed.
    fs::write(&rec, text).unwrap();
    assert_eq!(fx.bf(&["doctor"]).code, 0);
}

#[test]
fn the_plan_is_frozen_once_its_gate_is_approved() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let new_unit = "id=\"U-9\"\nkind=\"unit\"\ntitle=\"Anything\"\ndelivers=[\"R-1\"]\nfiles=[\"**\"]\ncheckpoint=\"K-1\"\n";
    let o = fx.put("unit", new_unit);
    assert_eq!(
        o.code,
        2,
        "a new unit after approval needs an amendment: {}",
        o.all()
    );
    assert_four_parts(&o.stderr);
    assert!(o.stderr.contains("amendment"), "{}", o.stderr);
    assert_eq!(fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-9\"\n").code, 2);
    assert_eq!(
        fx.put(
            "checkpoint",
            "id=\"K-2\"\nkind=\"checkpoint\"\ntitle=\"t\"\nafter=[\"U-9\"]\nproves=[\"R-1\"]\n"
        )
        .code,
        2
    );
    // Written straight into the folder, the unit still cannot start: it is not in what was approved.
    fs::write(fx.root.join("factory/records/U-9.toml"), new_unit).unwrap();
    let o = fx.refused(&["unit", "start", "U-9"]);
    assert!(o.stderr.contains("U-9 is new since"), "{}", o.stderr);
    let o = fx.refused(&["unit", "start", "U-1"]);
    assert!(
        o.stderr.contains("U-9"),
        "no unit starts while the plan differs from the approved one"
    );
    // An approved amendment is the way in.
    fs::remove_file(fx.root.join("factory/records/U-9.toml")).unwrap();
    fx.must(&["amend", "open", "U-9"]);
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("n.json");
    fs::write(&f, json!({"keep": ["U-9"], "reasons": {}}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("yes, add that unit");
    fx.must(&["amend", "approve", "--quote", "yes, add that unit"]);
    assert_eq!(fx.put("unit", new_unit).code, 0);
}

// ----------------------------------------------------------------

fn tokens_on(fx: &Fx) {
    fx.set_policy("tokens = false", "tokens = true");
}

fn token_for(fx: &Fx, unit: &str, role: &str) -> String {
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["unit"] == unit && d["role"] == role && d["token_command"].is_string()),
        "no {role} directive for {unit}: {n}"
    );
    fx.take_token(unit, role)
}

#[test]
fn a_builder_cannot_pass_its_own_review() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    tokens_on(&fx);
    let bt = token_for(&fx, "U-1", "builder");
    let wc = started_unit(&fx);
    // No token: refused. Wrong role's token: refused.
    let o = fx.bf_in(&wc, &["report", "U-1", "--result", "built"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
    assert_eq!(
        fx.bf_in(
            &wc,
            &["report", "U-1", "--result", "built", "--token", "nonsense"]
        )
        .code,
        2
    );
    fx.must_in(&wc, &["report", "U-1", "--result", "built", "--token", &bt]);
    // The builder's token is spent, and it is not the reviewer's.
    let again = fx.bf_in(
        &wc,
        &[
            "report", "U-1", "--result", "built", "--role", "reviewer", "--token", &bt,
        ],
    );
    assert_eq!(
        again.code,
        2,
        "a unit cannot pass review on a report that carries the builder's token: {}",
        again.all()
    );
    // The builder cannot fetch the reviewer's token: `next` and `brief` do not run from a working copy.
    for cmd in [vec!["next"], vec!["brief", "U-1", "--role", "reviewer"]] {
        let o = fx.bf_in(&wc, &cmd);
        assert_eq!(o.code, 2, "{cmd:?}: {}", o.all());
        assert_four_parts(&o.stderr);
    }
    // The conductor gets the reviewer's token from `next` and hands it to the reviewer.
    let rt = token_for(&fx, "U-1", "reviewer");
    assert_ne!(rt, bt);
    let r = fx
        .must_in(
            &wc,
            &[
                "report", "U-1", "--result", "built", "--role", "reviewer", "--token", &rt,
                "--json",
            ],
        )
        .json();
    assert_eq!(r["state"], "reviewed");
    // One use only.
    assert_eq!(
        fx.bf_in(
            &wc,
            &["report", "U-1", "--result", "built", "--role", "reviewer", "--token", &rt]
        )
        .code,
        2
    );
}

#[test]
fn without_the_hook_the_engine_still_refuses_what_a_worker_may_not_do() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let wc = started_unit(&fx);
    let d = tempfile::tempdir().unwrap();
    let w = d.path().join("w.toml");
    fs::write(&w, "id=\"R-8\"\nkind=\"requirement\"\ntitle=\"sneaky\"\n").unwrap();
    for cmd in [
        vec![
            "-C",
            "../../..",
            "record",
            "put",
            "requirement",
            "--file",
            w.to_str().unwrap(),
        ],
        vec!["-C", "../../..", "gate", "open", "idea"],
        vec!["gate", "open", "idea"],
        vec!["test-mode", "grant", "--quote", "x"],
        vec!["checkpoint", "close", "K-1", "--merge"],
        vec!["unit", "land", "U-1"],
        vec!["head", "accept", "--quote", "x"],
        vec!["answer", "Q-1", "--file", w.to_str().unwrap()],
    ] {
        let o = fx.bf_in(&wc, &cmd);
        assert_eq!(
            o.code,
            2,
            "{cmd:?} from a working copy must be refused: {}",
            o.all()
        );
        assert_four_parts(&o.stderr);
    }
    assert!(!fx.root.join("factory/records/R-8.toml").exists());
    // A verifier in the checkpoint's working copy gets the same.
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    let o = fx.bf_in(&fx.work("K-1"), &["gate", "open", "idea"]);
    assert_eq!(o.code, 2);
    assert_eq!(fx.bf_in(&fx.work("K-1"), &["status"]).code, 0);
}

// ----------------------------------------------------------------

#[test]
fn the_guard_denies_what_the_reviewer_got_through() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let wc = started_unit(&fx);
    let root = fx.root.to_string_lossy().to_string();
    let deny = |cwd: &Path, tool: &str, input: Value| {
        assert_eq!(
            guard_decision(&fx, cwd, tool, input.clone()),
            "deny",
            "{tool} {input} from {}",
            cwd.display()
        );
    };
    for cmd in [
        "echo x > ./factory/policy.toml".to_string(),
        "echo x > a/../factory/policy.toml".to_string(),
        "rm -rf ./factory".to_string(),
        "find . -name events.jsonl -delete; rm factory/log/events.jsonl".to_string(),
        "sh -c 'echo x > factory/policy.toml'".to_string(),
        "cat <<EOF > factory/policy.toml\nx\nEOF".to_string(),
        format!("echo x > {root}/factory/policy.toml"),
        "git stash".to_string(),
        "git reset --hard".to_string(),
        "git clean -fdx".to_string(),
        "git checkout -- factory".to_string(),
        "git fetch . bf/K-1:main".to_string(),
        "git push origin main".to_string(),
        "g\"it\" push".to_string(),
        "git rebase main".to_string(),
        "git branch -f main bf/K-1".to_string(),
    ] {
        deny(&fx.root, "Bash", json!({"command": cmd}));
    }
    // From outside the project, by the path.
    deny(
        Path::new("/tmp"),
        "Write",
        json!({"file_path": format!("{root}/factory/policy.toml")}),
    );
    deny(
        Path::new("/"),
        "Bash",
        json!({"command": format!("echo x > {root}/factory/policy.toml")}),
    );
    deny(
        Path::new("/tmp"),
        "Bash",
        json!({"command": format!("rm -rf {root}/factory")}),
    );
    // A worker writing outside its own working copy.
    deny(
        &wc,
        "Bash",
        json!({"command": format!("echo x > {root}/src/a.txt")}),
    );
    deny(
        &wc,
        "Bash",
        json!({"command": "echo x > ../../../src/a.txt"}),
    );
    deny(
        &wc,
        "Bash",
        json!({"command": format!("cd {root} && bf next")}),
    );
    deny(
        &wc,
        "Bash",
        json!({"command": format!("git -C {root} merge bf/U-1")}),
    );
    // A symlink inside the working copy that points at a record cannot be written through.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fx.root.join("factory/records/X-1.toml"), wc.join("x1"))
            .unwrap();
        deny(
            &wc,
            "Write",
            json!({"file_path": wc.join("x1").to_string_lossy()}),
        );
    }
    // What a worker may do still passes.
    for cmd in [
        "ls",
        "cargo test",
        "echo x > src/hello.txt",
        "git status",
        "bf report U-1 --result built",
    ] {
        assert_ne!(
            guard_decision(&fx, &wc, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
}

#[test]
fn a_missing_empty_unsigned_or_planted_table_denies_instead_of_allowing() {
    let fx = Fx::new();
    fx.tiny();
    let table = fx.root.join(".bf/guard.json");
    let good = fs::read_to_string(&table).unwrap();
    let write = json!({"file_path": fx.root.join("factory/policy.toml").to_string_lossy()});
    assert_eq!(
        guard_decision(&fx, &fx.root, "Write", write.clone()),
        "deny"
    );
    // An empty table.
    fs::write(&table, "{}").unwrap();
    assert_eq!(
        guard_decision(&fx, &fx.root, "Bash", json!({"command": "ls"})),
        "deny"
    );
    // A table with the right root but no signature.
    let mut t: Value = serde_json::from_str(&good).unwrap();
    t.as_object_mut().unwrap().remove("sig");
    t["protected"] = json!([]);
    fs::write(&table, t.to_string()).unwrap();
    assert_eq!(
        guard_decision(&fx, &fx.root, "Write", write.clone()),
        "deny"
    );
    // A signed table that was edited by hand.
    let mut t: Value = serde_json::from_str(&good).unwrap();
    t["protected"] = json!([]);
    fs::write(&table, t.to_string()).unwrap();
    let o = fx.bf_stdin(&fx.root, &["guard"], &json!({"tool_name": "Bash", "tool_input": {"command": "ls"}, "cwd": fx.root.to_string_lossy()}).to_string());
    assert!(o.stdout.contains("signature"), "{}", o.stdout);
    // No table at all, in a folder that has a factory/.
    fs::remove_file(&table).unwrap();
    assert_eq!(
        guard_decision(&fx, &fx.root, "Write", write.clone()),
        "deny"
    );
    // A table planted deeper (in a folder the agent can write) is ignored: the project's own wins.
    fs::write(&table, &good).unwrap();
    fs::create_dir_all(fx.root.join("src/.bf")).unwrap();
    fs::write(fx.root.join("src/.bf/guard.json"), "{}").unwrap();
    assert_eq!(
        guard_decision(&fx, &fx.root.join("src"), "Write", write),
        "deny"
    );
    // A project with no factory/ and no table is simply not a factory.
    let other = tempfile::tempdir().unwrap();
    assert_ne!(
        guard_decision(&fx, other.path(), "Bash", json!({"command": "ls"})),
        "deny"
    );
}

// ----------------------------------------------------------------

#[test]
fn work_from_a_test_run_that_reaches_main_by_git_is_refused_by_the_engine() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    // A way round the guard: fetch the checkpoint branch onto main from a detached head.
    git(&fx.root, &["checkout", "-q", "--detach"]);
    git(&fx.root, &["fetch", "-q", ".", "bf/K-1:main"]);
    git(&fx.root, &["checkout", "-q", "main"]);
    assert!(git(&fx.root, &["show", "main:src/hello.txt"]).contains("U-1 content"));
    // `status` says so and does not refuse; the commands that rely on main being clean do.
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main"]["ok"], false);
    assert!(st["main"]["detail"].as_str().unwrap().contains("U-1"));
    for cmd in [
        vec!["ledger", "check"],
        vec!["checkpoint", "close", "K-1"],
        vec!["unit", "land", "U-2"],
    ] {
        let o = fx.bf(&cmd);
        assert_eq!(o.code, 2, "{cmd:?} must refuse: {}", o.all());
        assert!(
            o.stderr.contains("main branch")
                && o.stderr.contains("U-1")
                && o.stderr.contains("test run"),
            "{}",
            o.stderr
        );
        assert_four_parts(&o.stderr);
    }
    // Accepting the position does not make test-run work acceptable on main.
    fx.human("Yes, I accept where main is.");
    fx.must(&["head", "accept", "--quote", "Yes, I accept where main is."]);
    assert_eq!(fx.bf(&["ledger", "check"]).code, 2);

    // The same for a unit that never landed.
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let wc = started_unit(&fx);
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "never reviewed\n").unwrap();
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "unreviewed work"]);
    git(&fx.root, &["merge", "-q", "--no-edit", "bf/U-1"]);
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("never landed"), "{}", o.stderr);
}

#[test]
fn a_git_alias_that_moves_main_is_noted_in_status_and_denied_to_agents() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    assert_eq!(fx.must(&["status", "--json"]).json()["main"]["ok"], true);
    // main jumps to the checkpoint branch by an alias: the work is a landed unit's, so nothing stops, but
    // the status says in one line that commits came that the engine did not make.
    git(&fx.root, &["config", "alias.up", "update-ref"]);
    git(&fx.root, &["up", "refs/heads/main", "bf/K-1"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main"]["ok"], true, "{st}");
    assert!(
        st["main"]["note"]
            .as_str()
            .unwrap()
            .contains("did not make"),
        "{st}"
    );
    let text = fx.must(&["status"]);
    assert!(
        text.stdout.contains("note: the main branch has moved on"),
        "{}",
        text.stdout
    );
    // The guard denies setting up the alias.
    let wc = fx.work("U-1");
    {
        let cmd = "git config alias.up update-ref";
        assert_eq!(
            guard_decision(&fx, &fx.root, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
        assert_eq!(
            guard_decision(&fx, &wc, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd} in a working copy"
        );
    }
    // Plumbing that only makes objects, and a subcommand the guard does not know (a user's own alias),
    // are left to the harness's own checks.
    assert_ne!(
        guard_decision(
            &fx,
            &fx.root,
            "Bash",
            json!({"command": "git commit-tree HEAD^{tree} -m x"})
        ),
        "deny"
    );
    assert_ne!(
        guard_decision(&fx, &fx.root, "Bash", json!({"command": "git st"})),
        "deny"
    );
}

#[test]
fn the_projects_first_commit_blocks_nothing_and_other_commits_are_only_noted() {
    let fx = Fx::new();
    fx.tiny();
    // The scaffolding the first commit holds: the records, the log, the ignore file, the harness folder.
    fs::create_dir_all(fx.root.join(".claude/skills/factory")).unwrap();
    fs::write(fx.root.join(".claude/skills/factory/SKILL.md"), "x\n").unwrap();
    fs::write(fx.root.join("notes.txt"), "mine\n").unwrap();
    fx.commit_all("scaffolding and a note");
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main"]["ok"], true);
    assert!(st["main"]["note"].as_str().unwrap().contains("moved on"));
    // Nothing is refused on account of it, and nothing asks for the human's words.
    fx.must(&["plan", "check"]);
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
    fx.must(&["checkpoint", "close", "K-1"]);
    fx.must(&["ledger", "check"]);
}

#[test]
fn parallel_puts_never_lose_a_record() {
    let fx = Fx::new();
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<PathBuf> = (0..20)
        .map(|i| {
            let f = dir.path().join(format!("p{i}.toml"));
            fs::write(
                &f,
                format!("kind=\"requirement\"\ntitle=\"Parallel {i}\"\n"),
            )
            .unwrap();
            f
        })
        .collect();
    std::thread::scope(|sc| {
        for f in &files {
            let fx = &fx;
            sc.spawn(move || {
                let o = fx.bf(&[
                    "record",
                    "put",
                    "requirement",
                    "--file",
                    f.to_str().unwrap(),
                ]);
                assert_eq!(o.code, 0, "{}", o.all());
            });
        }
    });
    let l = fx
        .must(&["record", "list", "--kind", "requirement", "--json"])
        .json();
    assert_eq!(l.as_array().unwrap().len(), 20, "records were lost");
    let mut titles: Vec<String> = l
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_string())
        .collect();
    titles.sort();
    titles.dedup();
    assert_eq!(titles.len(), 20);
}

#[test]
fn an_attached_file_does_not_settle_a_human_check() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    fx.put("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"read aloud\"\nhow=\"human\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    fx.put("check", "id=\"X-6\"\nkind=\"check\"\ntitle=\"screenshot\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    let junk = fx.root.join("junk.txt");
    fs::write(&junk, "anything").unwrap();
    // `junk.txt` is a new file in the project; keep it out of the content hash by ignoring it.
    fs::write(fx.root.join(".gitignore"), ".bf/\njunk.txt\n").unwrap();
    fx.must(&["evidence", "attach", "X-5", junk.to_str().unwrap()]);
    fx.must(&["evidence", "attach", "X-6", junk.to_str().unwrap()]);
    let p = fx.project();
    let rcs = bf::evidence::load_receipts(&p).unwrap();
    let human = rcs.iter().find(|r| r.check() == "X-5").unwrap();
    let agent = rcs.iter().find(|r| r.check() == "X-6").unwrap();
    assert!(!human.passes(), "a file cannot settle a human check");
    assert!(
        agent.passes(),
        "a file is proof for a check done by an agent"
    );
    assert_eq!(
        fx.bf(&["evidence", "verify"]).code,
        2,
        "verify must not call the human check ok"
    );
}

#[test]
fn an_answer_needs_a_human_turn_after_the_question() {
    let fx = Fx::new();
    let d = tempfile::tempdir().unwrap();
    let q = d.path().join("q.json");
    fs::write(
        &q,
        json!([{"title": "Which tone?", "kind": "single", "stage": "requirements"}]).to_string(),
    )
    .unwrap();
    fx.human("an old message, before the question");
    fx.must(&["ask", "--file", q.to_str().unwrap()]);
    let a = d.path().join("a.json");
    fs::write(&a, json!({"choice": "A"}).to_string()).unwrap();
    let o = fx.bf(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    assert_eq!(
        o.code,
        2,
        "an agent cannot answer for the human: {}",
        o.all()
    );
    assert_four_parts(&o.stderr);
    assert!(
        fx.bf(&["gate", "open", "requirements"]).code == 2,
        "the stage's gate stays closed"
    );
    fx.human("done");
    fx.must(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    let r = fx.must(&["record", "get", "Q-1", "--json"]).json();
    assert!(r["answered_turn"].is_number());
}

#[test]
fn merging_onto_main_needs_the_humans_go_ahead_after_the_closure() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("approved");
    fx.must(&["gate", "approve", "plan", "--quote", "approved"]);
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
    // Not closed yet: no merge. Closed but no human go-ahead: no merge.
    assert_eq!(fx.bf(&["checkpoint", "close", "K-1", "--merge"]).code, 2);
    fx.must(&["checkpoint", "close", "K-1"]);
    let o = fx.refused(&["checkpoint", "close", "K-1", "--merge"]);
    assert!(o.stderr.contains("go-ahead"), "{}", o.stderr);
    // The human's go-ahead before the closure does not count: the gate has to be opened after it.
    assert!(!fx.root.join("src/hello.txt").exists());
    let m = fx.merge_checkpoint("K-1").json();
    assert_eq!(m["merged"], true);
    assert!(fx.root.join("src/hello.txt").exists());
    // And the engine remembers the commit it put on main, so its own merge is not a "moved" main.
    fx.must(&["status"]);
}

#[test]
fn with_a_budget_cap_every_report_must_carry_its_cost() {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("cap = 0.0", "cap = 5.0");
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    let o = fx.refused(&["report", "U-1", "--result", "built"]);
    assert!(o.stderr.contains("cost"), "{}", o.stderr);
    let o = fx.refused(&["report", "U-1", "--result", "built", "--cost", "lots"]);
    assert!(o.stderr.contains("not a number"), "{}", o.stderr);
    fx.refused(&["report", "U-1", "--result", "built", "--cost", "-3"]);
    fx.must(&["report", "U-1", "--result", "built", "--cost", "1.25"]);
    assert_eq!(fx.must(&["cost", "status", "--json"]).json()["spent"], 1.25);
}

#[test]
fn leading_zeros_are_not_ids_and_a_copied_receipt_is_not_a_second_receipt() {
    let fx = Fx::new();
    let o = fx.put(
        "requirement",
        "id=\"R-007\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    fx.tiny();
    fx.start_test_run();
    let wc = started_unit(&fx);
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "x\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-1",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "src/hello.txt",
        ],
    );
    let rc = fx.root.join("factory/receipts/rc-00001.toml");
    fs::copy(&rc, fx.root.join("factory/receipts/rc-00009.toml")).unwrap();
    let rows = bf::evidence::load_receipts(&fx.project()).unwrap();
    assert_eq!(rows.len(), 1, "a receipt's id is its file name");
}

// ----------------------------------------------------------------

#[test]
fn a_folder_that_only_starts_with_the_project_path_is_not_inside_it() {
    let fx = Fx::new();
    let evil = format!("{}-evil", fx.root.to_string_lossy());
    fs::create_dir_all(&evil).unwrap();
    let tr = fx.transcript_root().join("evil.jsonl");
    fs::write(
        &tr,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"approve\"}}\n",
    )
    .unwrap();
    let payload = json!({"prompt": "approve", "cwd": evil, "hook_event_name": "UserPromptSubmit", "session_id": "s", "transcript_path": tr.to_string_lossy()});
    let o = fx.bf_stdin(&fx.root, &["human-turn", "--json"], &payload.to_string());
    assert_eq!(o.json()["recorded"], false, "{}", o.all());
    assert!(o.json()["event"]["reason"]
        .as_str()
        .unwrap()
        .contains("outside"));
}

#[test]
fn a_link_into_the_state_folder_is_not_read_or_written_for_the_caller() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    fx.put("check", "id=\"X-6\"\nkind=\"check\"\ntitle=\"screenshot\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    let state = fx.state_dir();
    fs::create_dir_all(fx.root.join(".bf/pages")).unwrap();
    // A file that looks harmless but is a link to the key.
    let link = fx.root.join(".bf/pages/shot.png");
    std::os::unix::fs::symlink(state.join("key"), &link).unwrap();
    let o = fx.refused(&["evidence", "attach", "X-6", link.to_str().unwrap()]);
    assert!(o.all().contains("state folder"), "{}", o.all());
    // The same for a file read as input.
    let q = fx.root.join(".bf/pages/q.json");
    std::os::unix::fs::symlink(state.join("key"), &q).unwrap();
    let o = fx.refused(&["answer", "Q-1", "--file", q.to_str().unwrap()]);
    assert!(o.all().contains("state folder"), "{}", o.all());
    // A page written through a link into factory/ is refused, and the target is untouched.
    let before = fs::read_to_string(fx.root.join("factory/config.toml")).unwrap();
    let out = fx.root.join(".bf/pages/out.html");
    std::os::unix::fs::symlink(fx.root.join("factory/config.toml"), &out).unwrap();
    let o = fx.refused(&[
        "render",
        "plan",
        "--template",
        fixture("template.html").to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(o.all().contains("factory/"), "{}", o.all());
    assert_eq!(
        fs::read_to_string(fx.root.join("factory/config.toml")).unwrap(),
        before
    );
    // A folder that is a link into the state folder is refused too.
    let dir_link = fx.root.join(".bf/pages/d");
    std::os::unix::fs::symlink(&state, &dir_link).unwrap();
    let o = fx.refused(&[
        "render",
        "plan",
        "--template",
        fixture("template.html").to_str().unwrap(),
        "--out",
        dir_link.join("page.html").to_str().unwrap(),
    ]);
    assert!(o.all().contains("state folder"), "{}", o.all());
}

#[test]
fn an_edit_of_the_config_does_not_move_the_main_branch_watch() {
    let fx = Fx::new();
    fx.tiny();
    let cfg = fx.root.join("factory/config.toml");
    let t = fs::read_to_string(&cfg).unwrap();
    assert!(t.contains("main_branch = \"main\""), "{t}");
    fs::write(
        &cfg,
        t.replace("main_branch = \"main\"", "main_branch = \"trunk\""),
    )
    .unwrap();
    // The watch still looks at `main`; doctor says the config disagrees and how to take the change.
    let st = fx.must(&["status", "--json"]).json();
    assert_ne!(st["main"]["tip"], "", "{st}");
    let d = fx.bf(&["doctor"]);
    assert!(
        d.stdout.contains("main branch: PROBLEM") && d.stdout.contains("head accept"),
        "{}",
        d.stdout
    );
    // The human takes the new name on purpose.
    fs::write(&cfg, t).unwrap();
    assert!(!fx.bf(&["doctor"]).stdout.contains("main branch: PROBLEM"));
    // A main branch that is gone is a thing to account for, not "ok".
    git(&fx.root, &["branch", "-m", "main", "elsewhere"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["main"]["ok"], false, "{st}");
    assert!(st["main"]["detail"]
        .as_str()
        .unwrap()
        .contains("does not exist"));
}

#[test]
fn a_check_cannot_plant_a_git_hook_or_edit_the_git_config() {
    if !bf::sandbox::available() {
        return;
    }
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    let hook = fx.root.join(".git/hooks/post-commit");
    let cfgf = fx.root.join(".git/config");
    let cmd = format!(
        "echo 'cp key /tmp/x' > {} ; echo '[alias] x = y' >> {}",
        hook.display(),
        cfgf.display()
    );
    fx.put(
        "check",
        &format!(
            "id=\"X-7\"\nkind=\"check\"\ntitle=\"plants\"\nhow=\"command\"\ncommand={:?}\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n",
            cmd
        ),
    );
    let before = fs::read_to_string(&cfgf).unwrap();
    let o = fx.bf(&["evidence", "run", "X-7", "--", "sh", "-c", &cmd]);
    let _ = o;
    assert!(
        !hook.exists(),
        "the sandbox keeps a check out of .git/hooks"
    );
    assert_eq!(fs::read_to_string(&cfgf).unwrap(), before);
}

#[test]
fn a_policy_that_differs_only_in_comments_and_spacing_is_still_the_accepted_one() {
    let fx = Fx::new();
    let pol = fx.root.join("factory/policy.toml");
    let t = fs::read_to_string(&pol).unwrap();
    fs::write(&pol, format!("# a note by the human\n\n{t}\n\n")).unwrap();
    assert_eq!(fx.bf(&["status"]).code, 0);
    // A real change still stops everything.
    fs::write(&pol, t.replace("tokens = false", "tokens = true")).unwrap();
    assert_eq!(fx.bf(&["status"]).code, 2);
}

#[test]
fn a_policy_file_written_before_init_does_not_shut_the_guard() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap().join("fresh");
    fs::create_dir_all(dir.join("factory")).unwrap();
    fs::write(dir.join("factory/policy.toml"), "[roles]\ntokens = true\n").unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    let fx = Fx::new();
    let payload = json!({"tool_name": "Bash", "tool_input": {"command": "bf init"}, "cwd": dir.to_string_lossy()});
    let o = fx.bf_stdin(&dir, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!o.stdout.contains("deny"), "{}", o.stdout);
}

#[test]
fn an_empty_or_relative_state_variable_never_puts_the_key_inside_the_project() {
    let tmp = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(tmp.path()).unwrap();
    let proj = home.join("proj");
    fs::create_dir_all(&proj).unwrap();
    git(&proj, &["init", "-q", "-b", "main"]);
    fs::write(proj.join("README.md"), "x\n").unwrap();
    git(&proj, &["add", "-A"]);
    git(&proj, &["commit", "-q", "-m", "i"]);
    let o = std::process::Command::new(bin())
        .args(["init", "--name", "d"])
        .current_dir(&proj)
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("BF_STATE_HOME", "")
        .env("XDG_STATE_HOME", "relative/state")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!proj.join("blackfactory").exists());
    assert!(!proj.join("relative").exists());
    assert!(home.join(".local/state/blackfactory").is_dir());
}
