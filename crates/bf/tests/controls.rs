//! More attacks on the trust layer, and the ordinary commands that must keep working: the policy that
//! must not change unnoticed, transcripts, how `bf` may be called, the sandbox around checks, the
//! folders programs are found in, the main branch, tokens shown once, and false refusals.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn guard_decision(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> String {
    guard_with(fx, cwd, tool, input, &[])
}

fn guard_with(fx: &Fx, cwd: &Path, tool: &str, input: Value, extra: &[&str]) -> String {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
    let mut args = vec!["guard"];
    args.extend_from_slice(extra);
    let o = fx.bf_stdin(cwd, &args, &payload.to_string());
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
    PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

// ------------------------------------------------------------ the policy

#[test]
fn a_policy_edited_by_hand_stops_every_command_until_the_human_accepts_it() {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("tokens = false", "tokens = true");
    fx.start_test_run();
    let wc = started_unit(&fx);
    fx.must(&["next", "--json"]);
    let tok = fx.take_token("U-1", "builder");
    // One evaded write turns the safety settings off.
    let pol = fx.root.join("factory/policy.toml");
    let t = fs::read_to_string(&pol).unwrap();
    fs::write(
        &pol,
        t.replace("tokens = true", "tokens = false")
            .replace("allow = \"abstain\"", "allow = \"allow\""),
    )
    .unwrap();
    for cmd in [
        vec!["status"],
        vec!["next"],
        vec!["report", "U-1", "--result", "built"],
        vec!["ledger", "check"],
    ] {
        let o = fx.bf_in(if cmd[0] == "report" { &wc } else { &fx.root }, &cmd);
        assert_eq!(o.code, 2, "{cmd:?} must refuse: {}", o.all());
        assert!(
            o.stderr.contains("policy") && o.stderr.contains("bf policy accept"),
            "{}",
            o.stderr
        );
        assert_four_parts(&o.stderr);
    }
    // The guard still holds the accepted settings: nothing is switched to allow, tokens stay on.
    assert_eq!(
        guard_decision(&fx, &fx.root, "Bash", json!({"command": "ls"})),
        "abstain"
    );
    assert_eq!(
        guard_decision(
            &fx,
            &fx.root,
            "Write",
            json!({"file_path": pol.to_string_lossy()})
        ),
        "deny"
    );
    // An agent cannot re-accept by running `init` again.
    fx.must(&["init"]);
    assert_eq!(fx.bf(&["status"]).code, 2);
    // The human reads the change and accepts it with their words; without a turn nothing happens.
    let d = fx.must(&["policy", "diff"]);
    assert!(
        d.stdout.contains("- tokens = true") && d.stdout.contains("+ tokens = false"),
        "{}",
        d.stdout
    );
    assert_eq!(
        fx.bf(&[
            "policy",
            "accept",
            "--quote",
            "Yes, I accept the new policy."
        ])
        .code,
        2
    );
    fx.human("Yes, I accept this policy.");
    fx.must(&["policy", "accept", "--quote", "Yes, I accept this policy."]);
    fx.must(&["status"]);
    let log = fx.must(&["log", "--json"]).json();
    assert!(log
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "policy.accept"));
    // Tokens are off now because the human said so; the old token still exists but is not needed.
    let _ = tok;
    fx.must_in(&wc, &["report", "U-1", "--result", "built"]);
}

// ------------------------------------------------------------ transcripts and how `bf` is called

fn payload(fx: &Fx, prompt: &str, transcript: &Path) -> String {
    json!({"hook_event_name": "UserPromptSubmit", "session_id": "s", "cwd": fx.root.to_string_lossy(), "prompt": prompt, "transcript_path": transcript.to_string_lossy()}).to_string()
}

#[test]
fn a_turn_is_recorded_from_the_payload_and_the_transcript_is_only_noted() {
    let fx = Fx::new();
    let user_line = |t: &str| {
        format!(
            "{}\n",
            json!({"type": "user", "message": {"role": "user", "content": t}})
        )
    };
    let turn = |prompt: &str, tr: &Path| {
        fx.bf_stdin(
            &fx.root,
            &["human-turn", "--json"],
            &payload(&fx, prompt, tr),
        )
        .json()
    };
    // A file elsewhere, a file that is not there, a file that holds no human message, a file that
    // ends with another message: each is recorded, with the note saying what it was.
    let elsewhere = tempfile::tempdir().unwrap();
    let fake = elsewhere.path().join("t.jsonl");
    fs::write(&fake, user_line("I approve the architecture")).unwrap();
    assert_eq!(turn("one", &fake)["event"]["transcript"], "absent");
    let real = fx.transcript_root().join("session.jsonl");
    assert_eq!(turn("two", &real)["event"]["transcript"], "absent");
    fs::write(&real, "not json\n").unwrap();
    assert_eq!(turn("three", &real)["event"]["transcript"], "unparsed");
    fs::write(&real, user_line("an earlier message")).unwrap();
    let v = turn("four", &real);
    assert_eq!(v["recorded"], true, "{v}");
    assert_eq!(v["event"]["transcript"], "differs");
    // Tool results, meta lines and API blobs are not the human's message; the last real one counts.
    let blob = json!({"type": "api-request-blob", "message": {"role": "user", "content": "<system-reminder>x</system-reminder>"}});
    let meta =
        json!({"type": "user", "isMeta": true, "message": {"role": "user", "content": "caveat"}});
    let tool = json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "x"}]}});
    fs::write(
        &real,
        format!("{}{blob}\n{meta}\n{tool}\n", user_line("five")),
    )
    .unwrap();
    assert_eq!(turn("five", &real)["event"]["transcript"], "match");
    // Other checks on the payload still hold: a subagent's payload is not a human turn.
    let mut pl: serde_json::Value = serde_json::from_str(&payload(&fx, "six", &real)).unwrap();
    pl["agent_id"] = json!("a1");
    let o = fx.bf_stdin(&fx.root, &["human-turn", "--json"], &pl.to_string());
    assert_eq!(o.json()["recorded"], false);
    // Strict is an opt-in: then the transcript must already end with the prompt.
    fx.set_policy(
        "transcript_match = \"soft\"",
        "transcript_match = \"strict\"",
    );
    fs::write(&real, user_line("an earlier message")).unwrap();
    let o = turn("seven", &real);
    assert_eq!(o["recorded"], false, "{o}");
    assert!(o["event"]["reason"].as_str().unwrap().contains("strict"));
    fs::write(&real, user_line("eight")).unwrap();
    assert_eq!(turn("eight", &real)["recorded"], true);
}

#[test]
fn a_payload_shaped_like_claude_codes_is_recorded_before_the_transcript_has_the_prompt() {
    let fx = Fx::new();
    // The first prompt of a session: the transcript file is not written yet.
    let tr = fx.transcript_root().join("fresh-session.jsonl");
    let payload = json!({
        "session_id": "0b6f1c3e-aaaa-bbbb-cccc-1234567890ab",
        "transcript_path": tr.to_string_lossy(),
        "cwd": fx.root.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "Yes, the requirements look right to me."
    });
    let o = fx.bf_stdin(&fx.root, &["human-turn", "--json"], &payload.to_string());
    assert_eq!(o.json()["recorded"], true, "{}", o.all());
    assert_eq!(o.json()["event"]["transcript"], "absent");
    // The next prompt: the transcript holds only the previous one.
    fs::write(
        &tr,
        format!(
            "{}\n",
            json!({"type": "user", "message": {"role": "user", "content": "Yes, the requirements look right to me."}})
        ),
    )
    .unwrap();
    let mut second = payload.clone();
    second["prompt"] = json!("Ship it, and thanks.");
    let o = fx.bf_stdin(&fx.root, &["human-turn", "--json"], &second.to_string());
    assert_eq!(o.json()["recorded"], true, "{}", o.all());
    assert_eq!(o.json()["event"]["transcript"], "differs");
    // A gate is decided on it, and the gate status shows what the transcript said.
    let gid = fx
        .must(&["gate", "open", "requirements"])
        .stdout
        .trim()
        .to_string();
    let mut third = payload.clone();
    third["prompt"] = json!("I approve the requirements.");
    fx.bf_stdin(&fx.root, &["human-turn"], &third.to_string());
    fx.must(&[
        "gate",
        "approve",
        &gid,
        "--quote",
        "I approve the requirements.",
    ]);
    let st = fx.must(&["gate", "status", "--json"]);
    assert!(st.stdout.contains("transcript_check"), "{}", st.stdout);
}

#[test]
fn the_guard_reads_bf_calls_literally_and_keeps_agents_out_of_the_transcripts() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let wc = started_unit(&fx);
    let root = fx.root.to_string_lossy().to_string();
    for cmd in [
        "bf $(echo human)-turn < p.json".to_string(),
        "bf \"$(printf \"human\\x2dturn\")\" < p.json".to_string(),
        "h=hum; bf ${h}an-turn".to_string(),
        "eval bf status".to_string(),
        "sh -c 'bf status'".to_string(),
        "bash -c \"bf status\"".to_string(),
        "env bf status".to_string(),
        "echo status | xargs bf".to_string(),
        "command bf status".to_string(),
        "echo `bf status`".to_string(),
        "x=$(bf status)".to_string(),
        "bf gat$(echo e) approve plan".to_string(),
        "bf nonsense".to_string(),
        format!("{root}/target/debug/bf $SUB"),
    ] {
        assert_eq!(
            guard_decision(&fx, &fx.root, "Bash", json!({"command": cmd.clone()})),
            "deny",
            "{cmd}"
        );
    }
    // Plain calls pass, and so do words that only look like bf.
    for cmd in [
        "bf status",
        "bf --json status",
        "./target/release/bf gate open plan",
        "echo bf",
        "git commit -m 'use bf status'",
        "ls bf-notes.txt",
    ] {
        assert_ne!(
            guard_decision(&fx, &fx.root, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
    // The transcript root: no agent writes there, in any way; reading is fine.
    let tr = fx.transcript_root();
    assert_eq!(
        guard_decision(
            &fx,
            &fx.root,
            "Write",
            json!({"file_path": tr.join("fake.jsonl").to_string_lossy()})
        ),
        "deny"
    );
    for cmd in [
        format!("echo x > {}/fake.jsonl", tr.display()),
        format!("cp /tmp/x {}/fake.jsonl", tr.display()),
        format!("rm {}/session.jsonl", tr.display()),
    ] {
        assert_eq!(
            guard_decision(&fx, &wc, "Bash", json!({"command": cmd.clone()})),
            "deny",
            "{cmd}"
        );
    }
    assert_ne!(
        guard_decision(
            &fx,
            &fx.root,
            "Bash",
            json!({"command": format!("cat {}/session.jsonl", tr.display())})
        ),
        "deny"
    );
    // From a working copy only these bf commands pass the guard.
    for cmd in [
        "bf evidence run X-1 --unit U-1 -- sh src/test.sh",
        "bf evidence verify --unit U-1",
        "bf evidence fail X-1 --reason no",
        "bf evidence unreachable X-1 --reason no",
        "bf report U-1 --result built --token t",
        "bf record put finding --file f.toml",
        "bf record get U-1",
        "bf record list",
        "bf status",
    ] {
        assert_ne!(
            guard_decision(&fx, &wc, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
    for cmd in [
        "bf log",
        "bf view plan",
        "bf ledger check",
        "bf plan check",
        "bf id new unit",
        "bf doctor",
        "bf record put unit --file f.toml",
        "bf next",
        "bf brief U-1 --role reviewer",
        "bf gate status",
        "bf unit land U-1",
    ] {
        assert_eq!(
            guard_decision(&fx, &wc, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd} from a working copy"
        );
        // The engine refuses the same by the real current directory.
        let args: Vec<&str> = cmd.split(' ').skip(1).collect();
        let o = fx.bf_in(&wc, &args);
        assert_eq!(
            o.code,
            2,
            "`{cmd}` from a working copy must be refused by the engine: {}",
            o.all()
        );
    }
    assert_eq!(fx.bf_in(&wc, &["status"]).code, 0);
}

// ------------------------------------------------------------ the sandbox around checks

fn sandbox_active(fx: &Fx) -> bool {
    let d = fx.bf(&["doctor", "--json"]).json();
    d["checks"].as_array().unwrap().iter().any(|c| {
        c["check"] == "sandbox" && c["note"].as_str().unwrap().contains("under sandbox-exec")
    })
}

#[test]
fn a_check_cannot_read_the_key_or_write_the_records_but_an_ordinary_script_passes() {
    let fx = Fx::new();
    fx.tiny();
    let key = fx.state_dir().join("key");
    let secret = fs::read_to_string(&key).unwrap();
    let wc_name = "";
    let _ = wc_name;
    fx.put("check", &format!("id=\"X-8\"\nkind=\"check\"\ntitle=\"steal\"\nhow=\"command\"\ncommand=\"cat {}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n", key.display()));
    fx.put("check", &format!("id=\"X-9\"\nkind=\"check\"\ntitle=\"write\"\nhow=\"command\"\ncommand=\"touch {}/factory/planted.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n", fx.root.display()));
    fx.put("check", "id=\"X-7\"\nkind=\"check\"\ntitle=\"ordinary\"\nhow=\"command\"\ncommand=\"sh ordinary.sh\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    fs::write(fx.root.join("ordinary.sh"), "#!/bin/sh\nd=$(mktemp -d)\necho hi > \"$d/a\"\ncat \"$d/a\" > out-inside-project.txt\ncat ordinary.sh > /dev/null\nrm -r \"$d\"\n").unwrap();
    fs::write(fx.root.join(".gitignore"), ".bf/\nout-inside-project.txt\n").unwrap();
    let o = fx.bf(&["evidence", "run", "X-7", "--", "sh", "ordinary.sh"]);
    assert_eq!(o.code, 0, "an ordinary script that reads and writes in its working copy and its temporary folder must pass: {}", o.all());
    let rc = fs::read_to_string(fx.root.join("factory/receipts/rc-00001.toml")).unwrap();
    if !sandbox_active(&fx) {
        eprintln!("note: no sandbox on this system; the receipts say sandbox = \"none\"");
        assert!(rc.contains("sandbox = \"none\""), "{rc}");
        return;
    }
    assert!(rc.contains("sandbox = \"sandbox-exec\""), "{rc}");
    // Reading the key fails and the key does not reach the receipt.
    let o = fx.bf(&["evidence", "run", "X-8", "--", "cat", key.to_str().unwrap()]);
    assert_eq!(o.code, 1, "{}", o.all());
    let all = fs::read_to_string(fx.root.join("factory/receipts/rc-00002.toml")).unwrap();
    assert!(
        all.contains("exit = 1") && !all.contains(secret.trim()),
        "{all}"
    );
    assert!(!o.all().contains(secret.trim()));
    // The check cannot write under factory/ either.
    let o = fx.bf(&[
        "evidence",
        "run",
        "X-9",
        "--",
        "touch",
        &format!("{}/factory/planted.txt", fx.root.display()),
    ]);
    assert_eq!(o.code, 1, "{}", o.all());
    assert!(!fx.root.join("factory/planted.txt").exists());
    // None of these runs gives a passing receipt for the failing checks.
    let rcs = bf::evidence::load_receipts(&fx.project()).unwrap();
    assert!(rcs
        .iter()
        .filter(|r| r.check() != "X-7")
        .all(|r| !r.passes()));
}

#[test]
fn the_sandbox_costs_a_little_and_the_policy_can_switch_it_off() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    fx.put("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"true\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    let time = |n: usize| {
        let mut v: Vec<f64> = (0..n)
            .map(|_| {
                let t = std::time::Instant::now();
                let o = fx.bf(&["evidence", "run", "X-5", "--", "true"]);
                assert_eq!(o.code, 0, "{}", o.all());
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let with = time(7);
    let was_active = sandbox_active(&fx);
    fx.set_policy("sandbox = true", "sandbox = false");
    assert!(!sandbox_active(&fx));
    let without = time(7);
    println!("\nbf evidence run: median {with:.0} ms with the sandbox ({}), {without:.0} ms without; it adds {:.0} ms", if was_active { "active" } else { "not available here" }, with - without);
    assert!(with < 5000.0 && without < 5000.0);
    let rc = fs::read_to_string(
        fx.root
            .join("factory/receipts")
            .join(format!("rc-{:05}.toml", 14)),
    )
    .unwrap_or_default();
    assert!(rc.is_empty() || rc.contains("sandbox = \"none\""));
}

// ------------------------------------------------------------ the folders programs are found in

#[test]
fn a_changed_folder_on_the_check_path_does_not_stop_a_check_and_doctor_only_informs() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    fx.put("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"true\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    let extra = tempfile::tempdir().unwrap();
    let extra_dir = fs::canonicalize(extra.path()).unwrap();
    fx.set_policy(
        "path = \"/usr/bin:/bin:/usr/sbin:/sbin\"",
        &format!(
            "path = \"{}:/usr/bin:/bin:/usr/sbin:/sbin\"",
            extra_dir.display()
        ),
    );
    fx.must(&["evidence", "run", "X-5", "--", "true"]);
    // A version manager rehashes its shims: the folder changes, the check still runs.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(extra_dir.join("shim"), "#!/bin/sh\nexit 0\n").unwrap();
    let o = fx.must(&["evidence", "run", "X-5", "--", "true"]);
    assert!(!o.stderr.contains("refused"), "{}", o.stderr);
    // A folder inside the project is only information in `bf doctor`.
    let inside = fx.root.join("tools");
    fs::create_dir_all(&inside).unwrap();
    fx.set_policy("path = \"", &format!("path = \"{}:", inside.display()));
    fx.must(&["evidence", "run", "X-5", "--", "true"]);
    let d = fx.bf(&["doctor"]);
    assert!(
        d.stdout.contains("check path: ok") && d.stdout.contains("inside the project"),
        "{}",
        d.stdout
    );
}

// ------------------------------------------------------------ tokens taken once

#[test]
fn a_role_token_is_handed_out_for_a_directive_just_given_and_a_second_request_voids_the_first() {
    let fx = Fx::new();
    fx.tiny();
    fx.set_policy("tokens = false", "tokens = true");
    fx.start_test_run();
    // Nothing is handed out before a directive has been given.
    let o = fx.refused(&["token", "take", "U-1", "--role", "builder"]);
    assert!(
        o.stderr.contains("did not give that directive"),
        "{}",
        o.stderr
    );
    let first = fx.must(&["next", "--json"]).json();
    assert!(
        first["directives"][0]["token"].is_null(),
        "`bf next` shows no token itself: {first}"
    );
    assert_eq!(
        first["directives"][0]["token_command"],
        "bf token take U-1 --role builder"
    );
    let tok = fx.take_token("U-1", "builder");
    // A second request (a conductor that lost its context, or a script that asked first) is served,
    // and says loudly that the earlier token is void now.
    let o = fx.must(&["token", "take", "U-1", "--role", "builder"]);
    assert!(
        o.stderr.contains("already taken") && o.stderr.contains("void"),
        "{}",
        o.stderr
    );
    let tok2 = o.stdout.trim().to_string();
    assert_ne!(tok, tok2);
    let wc = started_unit(&fx);
    assert_eq!(
        fx.bf_in(
            &wc,
            &["report", "U-1", "--result", "built", "--token", "guess"]
        )
        .code,
        2
    );
    // A worker cannot ask for tokens.
    assert_eq!(
        fx.bf_in(&wc, &["token", "take", "U-1", "--role", "reviewer"])
            .code,
        2
    );
    // The first taker's token no longer works; the second taker's does.
    assert_eq!(
        fx.bf_in(
            &wc,
            &["report", "U-1", "--result", "built", "--token", &tok]
        )
        .code,
        2,
        "the token that was voided"
    );
    fx.must_in(
        &wc,
        &["report", "U-1", "--result", "built", "--token", &tok2],
    );
    // The reviewer's directive is new, so its token can be taken.
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["role"], "reviewer");
    let rt = fx.take_token("U-1", "reviewer");
    // The human can still clear every unspent token and offer.
    let o = fx.bf(&["tokens", "reset", "--quote", "Yes, reset them."]);
    assert_eq!(o.code, 2, "needs a human turn");
    fx.human("Yes, reset them.");
    fx.must(&["tokens", "reset", "--quote", "Yes, reset them."]);
    fx.refused(&["token", "take", "U-1", "--role", "reviewer"]);
    fx.must(&["next", "--json"]);
    let rt2 = fx.take_token("U-1", "reviewer");
    assert_ne!(rt, rt2);
    assert_eq!(
        fx.bf_in(
            &wc,
            &["report", "U-1", "--result", "built", "--role", "reviewer", "--token", &rt]
        )
        .code,
        2,
        "the old token is gone"
    );
    fx.must_in(
        &wc,
        &[
            "report", "U-1", "--result", "built", "--role", "reviewer", "--token", &rt2,
        ],
    );
    assert_eq!(fx.bf_in(&wc, &["tokens", "reset", "--quote", "x"]).code, 2);
}

// ------------------------------------------------------------ the guard and ordinary commands

#[test]
fn ordinary_reads_and_git_housekeeping_are_not_refused() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let wc = started_unit(&fx);
    for (cwd, cmd) in [
        (&fx.root, "cat factory/records/R-1.toml 2>/dev/null"),
        (&fx.root, "jq . factory/records/R-1.toml"),
        (&fx.root, "sed -n 1,3p factory/records/R-1.toml"),
        (&fx.root, "awk 'NR<3' factory/records/R-1.toml"),
        (&fx.root, "sort factory/records/R-1.toml | uniq"),
        (&fx.root, "ls factory/records >/dev/null 2>&1"),
        (&fx.root, "mkdir -p .bf/pages"),
        (&fx.root, "git stash list"),
        (&fx.root, "git checkout bf/K-1 -- src/hello.txt"),
        (&wc, "cat ../../briefs/U-1-builder.md >/dev/null"),
        (&wc, "git status && git diff"),
    ] {
        assert_ne!(
            guard_decision(&fx, cwd, "Bash", json!({"command": cmd})),
            "deny",
            "`{cmd}` is an ordinary read"
        );
    }
    // Still refused: writes, and anything that names the state folder.
    for cmd in [
        "sed -i s/a/b/ factory/records/R-1.toml",
        "cat factory/records/R-1.toml > factory/x",
        "jq . factory/records/R-1.toml > factory/records/R-2.toml",
    ] {
        assert_eq!(
            guard_decision(&fx, &fx.root, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
    // A landed unit's old working copy is a unit's, not the main checkout.
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    assert_ne!(
        guard_decision(&fx, &wc, "Bash", json!({"command": "git stash"})),
        "deny"
    );
    assert_eq!(
        guard_decision(
            &fx,
            &wc,
            "Write",
            json!({"file_path": wc.join("src/hello.txt").to_string_lossy()})
        ),
        "deny",
        "a landed unit's copy is closed"
    );
}

#[test]
fn the_state_folder_cannot_be_found_by_its_parent_or_by_variables() {
    let fx = Fx::new();
    fx.tiny();
    let base = fx.state_base.to_string_lossy().to_string();
    assert_eq!(
        guard_decision(
            &fx,
            &fx.root,
            "Grep",
            json!({"pattern": "x", "path": base.clone()})
        ),
        "deny"
    );
    assert_eq!(
        guard_decision(
            &fx,
            &fx.root,
            "Read",
            json!({"file_path": fx.state_dir().join("head.json").to_string_lossy()})
        ),
        "deny"
    );
    for cmd in [
        "cat $BF_STATE_HOME/*/key",
        "cat $XDG_STATE_HOME/blackfactory/*/key",
        format!("cat {base}/*/key").as_str(),
    ] {
        assert_eq!(
            guard_decision(&fx, &fx.root, "Bash", json!({"command": cmd})),
            "deny",
            "{cmd}"
        );
    }
}

#[test]
fn a_named_project_is_the_only_project_the_guard_looks_at() {
    let fx = Fx::new();
    fx.tiny();
    // A folder above the project that happens to have a factory/ folder.
    let above = fx.root.parent().unwrap().join("factory");
    fs::create_dir_all(above.join("log")).unwrap();
    fs::write(above.join("log/events.jsonl"), "").unwrap();
    let input = json!({"file_path": fx.root.join("src/a.rs").to_string_lossy()});
    assert_eq!(
        guard_with(&fx, &fx.root, "Write", input.clone(), &[]),
        "deny",
        "without --root the outer folder takes the project's place"
    );
    let r = fx.root.to_string_lossy().to_string();
    assert_ne!(
        guard_with(&fx, &fx.root, "Write", input, &["--root", &r]),
        "deny"
    );
    assert_eq!(
        guard_with(
            &fx,
            &fx.root,
            "Write",
            json!({"file_path": fx.root.join("factory/policy.toml").to_string_lossy()}),
            &["--root", &r]
        ),
        "deny"
    );
}

// ------------------------------------------------------------ the table counter, bad records, programs

#[test]
fn an_older_guard_table_put_back_is_not_trusted() {
    let fx = Fx::new();
    fx.tiny();
    let table = fx.root.join(".bf/guard.json");
    let old = fs::read_to_string(&table).unwrap();
    // A later command writes a newer table.
    fx.start_test_run();
    started_unit(&fx);
    let newer = fs::read_to_string(&table).unwrap();
    assert_ne!(old, newer, "the table should have changed");
    // The newer table works: a read inside the project is not denied.
    let probe = json!({"file_path": fx.root.join("README.md").to_string_lossy()});
    assert_ne!(
        guard_decision(&fx, &fx.root, "Read", probe.clone()),
        "deny",
        "the current table must not refuse a plain read"
    );
    // The older copy, signature intact, is refused.
    fs::write(&table, &old).unwrap();
    let o = fx.bf_stdin(
        &fx.root,
        &["guard"],
        &json!({"tool_name": "Read", "tool_input": probe, "cwd": fx.root.to_string_lossy()})
            .to_string(),
    );
    assert!(
        o.stdout.contains("deny") && o.stdout.contains("older"),
        "{}",
        o.all()
    );
    // Writing the table again (any engine command that does) heals it.
    fs::write(&table, &newer).unwrap();
    assert_ne!(
        guard_decision(
            &fx,
            &fx.root,
            "Read",
            json!({"file_path": fx.root.join("README.md").to_string_lossy()})
        ),
        "deny"
    );
}

#[test]
fn one_unreadable_record_file_does_not_break_status_and_is_named() {
    let fx = Fx::new();
    fx.tiny();
    fs::write(
        fx.root.join("factory/records/R-9.toml"),
        "id = \"R-9\"\nkind = [ this is not toml\n",
    )
    .unwrap();
    let st = fx.bf(&["status", "--json"]);
    assert_eq!(st.code, 0, "{}", st.all());
    let j = st.json();
    let bad = j["bad_records"].as_array().unwrap();
    assert_eq!(bad.len(), 1, "{j}");
    assert!(bad[0]["file"].as_str().unwrap().contains("R-9"), "{j}");
    // The text form names it too, and the other records still show.
    let t = fx.bf(&["status"]);
    assert_eq!(t.code, 0, "{}", t.all());
    assert!(t.stdout.contains("R-9"), "{}", t.stdout);
    // Ordinary commands carry on with the rest.
    assert_eq!(fx.bf(&["record", "list", "--kind", "unit"]).code, 0);
    // Asking for the broken record by id says which file and why.
    let g = fx.refused(&["record", "get", "R-9"]);
    assert!(g.all().contains("R-9"), "{}", g.all());
    // The plan check flags it.
    let c = fx.bf(&["plan", "check"]);
    assert!(c.all().contains("R-9"), "{}", c.all());
    assert_eq!(fx.bf(&["doctor"]).code, 2);
    fs::remove_file(fx.root.join("factory/records/R-9.toml")).unwrap();
    assert_eq!(fx.bf(&["status"]).code, 0);
}

#[test]
fn a_program_replaced_after_the_plan_gate_is_noted_in_the_receipt_and_the_check_still_runs() {
    let fx = Fx::new();
    fx.tiny();
    let tools = fx.root.parent().unwrap().join("tools");
    fs::create_dir_all(&tools).unwrap();
    let tool = tools.join("checkit");
    fs::write(&tool, "#!/bin/sh\ntest -f src/hello.txt\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let rec = fx.root.join("factory/records/X-1.toml");
    let t = fs::read_to_string(&rec).unwrap();
    fs::write(
        &rec,
        t.replace("test -f src/hello.txt", tool.to_str().unwrap()),
    )
    .unwrap();
    fx.commit_all("check uses a tool");
    fx.start_test_run();
    let wc = started_unit(&fx);
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hi\n").unwrap();
    let run = |fx: &Fx| {
        fx.bf_in(
            &wc,
            &[
                "evidence",
                "run",
                "X-1",
                "--unit",
                "U-1",
                "--",
                tool.to_str().unwrap(),
            ],
        )
    };
    let o = run(&fx);
    assert_eq!(o.code, 0, "the recorded program runs: {}", o.all());
    // Replace the program's contents (a longer file, so size differs whatever the clock says).
    fs::write(&tool, "#!/bin/sh\n# changed\ntrue\n").unwrap();
    let o = run(&fx);
    assert_eq!(o.code, 0, "an upgrade is ordinary: {}", o.all());
    assert!(
        o.stderr.contains("checkit") && o.stderr.contains("recorded when the plan was approved"),
        "{}",
        o.stderr
    );
    let rcs = bf::evidence::load_receipts(&fx.project()).unwrap();
    assert!(
        rcs.iter().any(|r| r.v["program_note"]
            .as_str()
            .unwrap_or("")
            .contains("checkit")),
        "the receipt carries the note"
    );
    // The change is noted once; the next run is quiet.
    let o = run(&fx);
    assert_eq!(o.code, 0);
    assert!(!o.stderr.contains("note:"), "{}", o.stderr);
}
