//! U-63: only a subagent of type `repair-judge` may judge (R-69, D-90, TH-17): the guard's signed one-use
//! pass, the judge's refused commands and tools, the role file check, the marked records and `amend seen`.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;

const AGENT: &str = "repair-judge";

fn unit(id: &str, cp: &str, file: &str, deps: &[&str], est: u32) -> String {
    format!(
        "id=\"{id}\"\nkind=\"unit\"\ntitle=\"Unit {id}\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"{file}\"]\ndepends_on={}\ncheckpoint=\"{cp}\"\nestimate={est}\nfull_suite=\"fixture\"\n",
        json!(deps)
    )
}

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"Things exist\"\nacceptance=[\"they exist\"]\n".to_string()),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Things\"\n".to_string()),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    for i in 1..=6 {
        let t = unit(&format!("U-{i}"), "K-1", &format!("src/f{i}.txt"), &[], 5);
        let o = fx.put("unit", &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let o = fx.put("unit", &unit("U-7", "K-2", "src/f7.txt", &[], 1));
    assert_eq!(o.code, 0, "{}", o.all());
    for i in 1..=7 {
        let t = format!("id=\"X-{}\"\nkind=\"check\"\ntitle=\"Unit {i}\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{i}\"\n", 10 + i);
        let o = fx.put("check", &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    for (k, t) in [
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"First\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"Second\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test.\"\ntitle=\"One\"\nafter=[\"U-1\",\"U-2\",\"U-3\",\"U-4\",\"U-5\",\"U-6\"]\nproves=[\"R-1\",\"C-1\"]\nchecks=[\"X-1\"]\n"),
        ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test.\"\ntitle=\"Two\"\nafter=[\"U-7\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("plan");
    fx.start_test_run();
}

fn put_replace(fx: &Fx, kind: &str, text: &str) {
    let f = fx.root.join("edit.toml");
    fs::write(&f, text).unwrap();
    fx.must(&[
        "record",
        "put",
        kind,
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    fs::remove_file(f).unwrap();
}

/// Open a plan repair over `ids`, put the edits, narrow it. Returns its ID.
fn repair(fx: &Fx, ids: &[&str], edits: &[(&str, String)]) -> String {
    let mut a = vec!["amend", "open", "--repair"];
    a.extend(ids);
    a.push("--json");
    let o = fx.must(&a).json();
    for (k, t) in edits {
        put_replace(fx, k, t);
    }
    let aid = o["amend"].as_str().unwrap().to_string();
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": aid, "keep": o["impact_ids"], "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    aid
}

fn token(fx: &Fx, aid: &str) -> String {
    fx.must(&["roles", "token", aid, "judge", "--json"]).json()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn events(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn kinds(fx: &Fx, kind: &str) -> usize {
    events(fx).iter().filter(|e| e["kind"] == kind).count()
}

fn status_of(fx: &Fx, aid: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["amendments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == aid)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn role_source() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../harness/claude/agents/repair-judge.md")
}

/// The installed role file, as the installer writes it (the harness copy).
fn install_role(fx: &Fx) {
    let d = fx.root.join(".claude/agents");
    fs::create_dir_all(&d).unwrap();
    fs::copy(role_source(), d.join("repair-judge.md")).unwrap();
}

/// What the guard answers for a tool call: `deny` with its reason, or `allow`/`abstain`.
fn hook(fx: &Fx, agent_type: Option<&str>, tool: &str, input: Value) -> (String, String) {
    let mut payload = json!({"tool_name": tool, "tool_input": input, "cwd": fx.root.to_string_lossy(),
        "session_id": "sess-1"});
    if let Some(t) = agent_type {
        payload["agent_type"] = json!(t);
        payload["agent_id"] = json!("agent-1");
    }
    let o = fx.bf_stdin(&fx.root, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => ("abstain".into(), String::new()),
        s => {
            let v: Value = serde_json::from_str(s).unwrap();
            let h = &v["hookSpecificOutput"];
            (
                h["permissionDecision"].as_str().unwrap().to_string(),
                h["permissionDecisionReason"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
        }
    }
}

fn judge_cmd(aid: &str, tok: &str) -> String {
    format!(
        "bf amend judge {aid} --file - --token {tok} --model strong-model <<'EOF'\n{{\"verdict\": \"go\", \"reasons\": \"U-2 waits for U-1: one entry added.\"}}\nEOF"
    )
}

fn pass_file(fx: &Fx) -> std::path::PathBuf {
    fx.state_dir().join("judge-pass.json")
}

fn judge(fx: &Fx, aid: &str, tok: &str, verdict: &str) -> Out {
    fx.bf_stdin(
        &fx.root,
        &[
            "amend",
            "judge",
            aid,
            "--file",
            "-",
            "--token",
            tok,
            "--model",
            "strong-model",
        ],
        &json!({"verdict": verdict, "reasons": "plain words"}).to_string(),
    )
}

fn ready(fx: &Fx) -> String {
    plan(fx);
    install_role(fx);
    repair(
        fx,
        &["U-2"],
        &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))],
    )
}

#[test]
fn judge_guard_a_payload_with_no_agent_type_gets_no_pass_and_the_judge_is_refused() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    let (d, _) = hook(&fx, None, "Bash", json!({"command": judge_cmd(&aid, &tok)}));
    assert_ne!(d, "deny");
    assert!(!pass_file(&fx).exists());
    let o = judge(&fx, &aid, &tok, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("no fresh pass"), "{}", o.stderr);
    assert_eq!(kinds(&fx, "amend.judge.failed"), 1);
    assert_eq!(status_of(&fx, &aid), "open");
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(o.stderr.contains("the human's"), "{}", o.stderr);
}

#[test]
fn judge_guard_another_agent_type_gets_no_pass() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    hook(
        &fx,
        Some("builder"),
        "Bash",
        json!({"command": judge_cmd(&aid, &tok)}),
    );
    assert!(!pass_file(&fx).exists());
}

#[test]
fn judge_guard_the_role_files_here_document_form_passes_the_guard_and_its_verdict_is_taken() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    let (d, why) = hook(
        &fx,
        Some(AGENT),
        "Bash",
        json!({"command": judge_cmd(&aid, &tok)}),
    );
    assert_ne!(d, "deny", "{why}");
    assert!(pass_file(&fx).exists());
    let o = judge(&fx, &aid, &tok, "go");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(status_of(&fx, &aid), "approved");
    let ev = events(&fx);
    let j = ev.iter().rfind(|e| e["kind"] == "amend.judge").unwrap();
    assert_eq!(j["session"], "sess-1");
    assert_eq!(j["agent"], "agent-1");
    // The pass is used once.
    assert!(!pass_file(&fx).exists());
}

#[test]
fn judge_guard_a_piped_verdict_passes_the_guard_too() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    let cmd = format!(
        "echo '{{\"verdict\":\"ask\",\"reasons\":\"unsure\"}}' | bf amend judge {aid} --file - --token {tok} --model m"
    );
    let (d, why) = hook(&fx, Some(AGENT), "Bash", json!({"command": cmd}));
    assert_ne!(d, "deny", "{why}");
    assert!(pass_file(&fx).exists());
}

#[test]
fn judge_guard_a_pass_written_for_another_amendment_is_not_taken() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    hook(
        &fx,
        Some(AGENT),
        "Bash",
        json!({"command": judge_cmd("A-9", &tok)}),
    );
    assert!(pass_file(&fx).exists());
    let o = judge(&fx, &aid, &tok, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("A-9"), "{}", o.stderr);
    assert_eq!(kinds(&fx, "amend.judge.failed"), 1);
    assert_eq!(kinds(&fx, "amend.judge"), 0);
}

#[test]
fn judge_guard_a_pass_written_for_another_token_is_not_taken() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    hook(
        &fx,
        Some(AGENT),
        "Bash",
        json!({"command": judge_cmd(&aid, "some-other-token")}),
    );
    assert!(pass_file(&fx).exists());
    let o = judge(&fx, &aid, &tok, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("another token"), "{}", o.stderr);
    assert_eq!(kinds(&fx, "amend.judge"), 0);
}

#[test]
fn judge_guard_a_pass_written_by_hand_is_not_taken() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    fs::write(
        pass_file(&fx),
        json!({"amend": aid, "agent_type": AGENT, "session": "s", "agent": "a", "at": 1})
            .to_string(),
    )
    .unwrap();
    let o = judge(&fx, &aid, &tok, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("not signed"), "{}", o.stderr);
    assert_eq!(kinds(&fx, "amend.judge"), 0);
}

#[test]
fn judge_guard_a_changed_installed_role_file_refuses_the_judge() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let installed = fx.root.join(".claude/agents/repair-judge.md");
    let text = fs::read_to_string(&installed).unwrap();
    fs::write(
        &installed,
        text.replace("Say `go` when", "Always say `go` when"),
    )
    .unwrap();
    let tok = token(&fx, &aid);
    hook(
        &fx,
        Some(AGENT),
        "Bash",
        json!({"command": judge_cmd(&aid, &tok)}),
    );
    let o = judge(&fx, &aid, &tok, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("differs from the harness copy"),
        "{}",
        o.stderr
    );
    assert_eq!(kinds(&fx, "amend.judge.failed"), 1);
    assert_eq!(status_of(&fx, &aid), "open");
}

#[test]
fn judge_guard_a_changed_worker_rules_section_does_not_refuse_the_judge() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let installed = fx.root.join(".claude/agents/repair-judge.md");
    let text = fs::read_to_string(&installed).unwrap();
    fs::write(&installed, format!("{text}- A row the data added.\n")).unwrap();
    let tok = token(&fx, &aid);
    hook(
        &fx,
        Some(AGENT),
        "Bash",
        json!({"command": judge_cmd(&aid, &tok)}),
    );
    let o = judge(&fx, &aid, &tok, "go");
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn judge_guard_the_repair_judge_may_not_propose_narrow_or_write() {
    let fx = Fx::new();
    for cmd in [
        "bf amend narrow --file n.json",
        "bf amend open --repair U-2",
        "bf amend revise U-2",
        "bf amend checkpoint before K-1 U-2",
        "bf amend approve A-1 --quote x",
        "bf amend withdraw A-1",
        "bf amend seen A-1 --quote x",
        "bf record put unit --file u.toml",
        "bf record remove U-2",
        "bf roles token A-1 judge",
        "bf -C . roles token A-1 judge",
        "bf checkpoint check K-1 --accept X-2 --quote x",
        "bf checkpoint --json check K-1 --accept X-2 --quote x",
        "bf checkpoint --accept X-2 check K-1 --quote x",
        "bf record --json put unit --file u.toml",
        "bf record --json remove U-2",
        "bf amend --json open --repair U-2",
        "bf amend --json approve A-1 --quote x",
        "bf checkpoint $E check K-1 --accept X-2 --quote x",
        "bf checkpoint ${E}check K-1 --accept X-2 --quote x",
        "bf record $E put unit --file u.toml",
        "bf record `true` remove U-2",
        "bf amend $E approve A-1 --quote x",
        "bf checkpoint --note$E x check K-1 --accept X-2 --quote q",
        "bf -C$E checkpoint check K-1 --accept X-2 --quote q",
        "bf checkpoint --root $E check K-1 --accept X-2 --quote q",
        "bf checkpoint --json$E check K-1 --accept X-2 --quote q",
        "bf checkpoint `true` check K-1 --accept X-2 --quote q",
        "bf checkpoint $(true) check K-1 --accept X-2 --quote q",
        "bf record --note$E x put unit --file u.toml",
        "bf amend --note$E x approve A-1 --quote x",
        "bf checkpoint {check,check} K-1 --accept X-2 --quote q",
        "bf record {put,put} unit --file u.toml",
        "bf amend {approve,approve} A-1 --quote x",
        "bf checkpoint {a..b} check K-1 --accept X-2 --quote q",
        "bf checkpoint ch?ck K-1 --accept X-2 --quote q",
        "bf checkpoint ch*k K-1 --accept X-2 --quote q",
        "bf checkpoint [c]heck K-1 --accept X-2 --quote q",
    ] {
        let (d, why) = hook(&fx, Some(AGENT), "Bash", json!({"command": cmd}));
        assert_eq!(d, "deny", "{cmd}");
        assert!(why.contains("repair judge"), "{why}");
        // The same command from no agent type is not refused for this reason.
        let (d, _) = hook(&fx, None, "Bash", json!({"command": cmd}));
        assert_ne!(d, "deny", "{cmd}");
    }
    for tool in ["Write", "Edit", "MultiEdit", "NotebookEdit"] {
        let (d, _) = hook(
            &fx,
            Some(AGENT),
            tool,
            json!({"file_path": "x.txt", "content": "x"}),
        );
        assert_eq!(d, "deny", "{tool}");
        let (d, _) = hook(
            &fx,
            None,
            tool,
            json!({"file_path": "x.txt", "content": "x"}),
        );
        assert_ne!(d, "deny", "{tool}");
    }
    for cmd in [
        "bf record get U-2",
        "bf plan check",
        "bf status",
        "bf record get $ID",
        "bf record get U-2 --note \"$(cat f)\"",
    ] {
        let (d, why) = hook(&fx, Some(AGENT), "Bash", json!({"command": cmd}));
        assert_ne!(d, "deny", "{cmd}: {why}");
    }
}

#[test]
fn judge_guard_patterns_hidden_calls_and_run_time_code_are_refused_to_the_judge() {
    let fx = Fx::new();
    // Each row runs a barred command under zsh or bash although its plain reading may not show one.
    let pattern_rows = [
        // zsh glob qualifiers and groups: `x(N)` vanishes, `(check|zz)` and `chec(k)` match a file.
        "bf checkpoint x(N) check K-1 --accept X-2 --quote q",
        "bf record x(N) put unit --file u.toml",
        "bf amend x(N) approve A-1 --quote x",
        "bf x(N) checkpoint check K-1 --accept X-2 --quote q",
        "bf checkpoint \"x\"(N) check K-1 --accept X-2 --quote q",
        "bf checkpoint (check|zz) K-1 --accept X-2 --quote q",
        "bf checkpoint chec(k) K-1 --accept X-2 --quote q",
        "bf checkpoint (#i)CHECK K-1 --accept X-2 --quote q",
        "bf checkpoint @(check) K-1 --accept X-2 --quote q",
        // zsh's number range `<->` is a pattern, bash's reading is two redirects.
        "bf checkpoint x<-> check K-1 --accept X-2 --quote q",
    ];
    let rows = [
        // zsh extended globs and `brace_ccl`, which a command can switch on.
        "bf checkpoint chec#k K-1 --accept X-2 --quote q",
        "bf checkpoint ch^x K-1 --accept X-2 --quote q",
        "bf checkpoint check~zz K-1 --accept X-2 --quote q",
        "bf checkpoint {c}heck K-1 --accept X-2 --quote q",
        "bf checkpoint =check K-1 --accept X-2 --quote q",
        // A redirect between the words takes its word with it.
        "bf checkpoint {fd}>/dev/null check K-1 --accept X-2 --quote q",
        "bf checkpoint >& /dev/null check K-1 --accept X-2 --quote q",
        "bf checkpoint >&/dev/null check K-1 --accept X-2 --quote q",
        "bf record {fd}<u.toml put unit --file u.toml",
        // Lines the shell runs that are no heredoc body.
        "cat << \"E F\"\nE F\nbf record put unit --file u.toml\nE",
        "(( x = 1 <<2 ))\nbf record put unit --file u.toml\n2",
        "for (( i = 0; i <<2; i++ )); do :; done\nbf record put unit --file u.toml\n2",
        // Code a glob qualifier, a wrapper or a precommand modifier runs.
        "ls /(e:'bf record put unit --file u.toml':)",
        "ls /(e{'bf amend approve A-1 --quote x'})",
        "env -S 'bf record put unit --file u.toml'",
        "env -S'bf record put unit --file u.toml'",
        "flock /tmp/l -c 'bf record put unit --file u.toml'",
        "script -q -c 'bf record put unit --file u.toml' /dev/null",
        "watch -n 1 'bf record put unit --file u.toml'",
        "noglob bf record put unit --file u.toml",
        "nocorrect bf amend approve A-1 --quote x",
        "- bf record put unit --file u.toml",
        "repeat 1 bf record put unit --file u.toml",
        "function f { bf record put unit --file u.toml; }; f",
        "coproc bf record put unit --file u.toml",
        "parallel bf record put unit --file ::: u.toml",
        "sandbox-exec -n no-network bf record put unit --file u.toml",
        "alias q='bf record put'; eval q unit --file u.toml",
        "hash q=/usr/local/bin/bf; q record put unit --file u.toml",
        // A shell's command string is the first word after all its options.
        "bash -c -e 'bf record put unit --file u.toml'",
        "bash --norc -c 'bf record put unit --file u.toml'",
        "bash -o posix -c 'bf record put unit --file u.toml'",
        "bash +x -c 'bf record put unit --file u.toml'",
        "zsh -o extendedglob -c 'bf record put unit --file u.toml'",
        // Quotes the shell closes elsewhere than a plain reading: `$'\''` and a quote inside `${ }`.
        "echo $'\\'' ; bf record put unit --file u.toml # '",
        "echo ${V:-'}'} ; bf record put unit --file u.toml #'",
        // A substitution inside `${ }` runs.
        "echo ${V:-$(bf record put unit --file u.toml)}",
        "echo ${V:-`bf record put unit --file u.toml`}",
        // A backtick escaped inside backticks nests a substitution; `$'...'` with no escape is plain text.
        "echo `echo \\`bf record put unit --file u.toml\\``",
        "eval $'bf record put unit --file u.toml'",
        "sh -c $'bf record put unit --file u.toml'",
        // Wrappers that run the words after their options.
        "pkexec bf record put unit --file u.toml",
        "nsenter -t 1 bf record put unit --file u.toml",
        // A program name the shell makes from a pattern or with zsh's `equals`.
        "setopt equals; =bf record put unit --file u.toml",
        "b[f] record put unit --file u.toml",
        "b? record put unit --file u.toml",
        "b(f) record put unit --file u.toml",
        "{bf,x} record put unit --file u.toml",
        // A redirect before the program takes its word: the program is `bf`.
        "{fd}>/dev/null bf record put unit --file u.toml",
        ">&/dev/null bf record put unit --file u.toml",
        // zsh's short forms run a command inside the same simple command.
        "if [[ -n x ]] bf record put unit --file u.toml",
        "{ true } always { bf record put unit --file u.toml }",
        "coproc CP { bf record put unit --file u.toml; }",
        // The engine built from this project.
        "cargo run -q -- record put unit --file u.toml",
        "cargo run --bin bf checkpoint check K-1 --accept X-2 --quote q",
        // Code the shell builds or evaluates at run time.
        "B=bf; $B record put unit --file u.toml",
        "V='bf record put unit --file u.toml'; eval $V",
        "V='$(bf record put unit --file u.toml)'; echo ${(e)V}",
        "V='$(bf record put unit --file u.toml)'; echo ${V@P}",
        "V='a[$(bf record put unit --file u.toml)]'; echo $((V))",
        "V='a[$(bf record put unit --file u.toml)]'; (( V ))",
        "V='a[$(bf record put unit --file u.toml)]'; [[ V -eq 0 ]]",
        "V='a[$(bf record put unit --file u.toml)]'; printf -v \"$V\" x",
        "V='a[$(bf record put unit --file u.toml)]'; let V",
        "V='a[$(bf record put unit --file u.toml)]'; echo ${a[V]}",
        "V='a[$(bf record put unit --file u.toml)]'; cat <<EOF\n$((V))\nEOF",
        // The judge runs no check, so no check's command either.
        "bf evidence run X-1 -- sh -c 'B=bf; $B record put unit --file u.toml'",
    ];
    for cmd in pattern_rows.iter().chain(rows.iter()) {
        let (d, why) = hook(&fx, Some(AGENT), "Bash", json!({"command": cmd}));
        assert_eq!(d, "deny", "{cmd}");
        assert!(why.contains("repair judge"), "{cmd}: {why}");
    }
    // A pattern group in the words of `bf` cannot be read for any caller.
    for cmd in pattern_rows {
        let (d, why) = hook(&fx, None, "Bash", json!({"command": cmd}));
        assert_eq!(d, "deny", "{cmd}");
        assert!(why.contains("pattern"), "{cmd}: {why}");
    }
    // The judge's own commands, and plain shell around them, still pass.
    for cmd in [
        "printf '%s' '{\"verdict\":\"go\",\"reasons\":\"U-2 [a] (b) waits\"}' | bf amend judge A-1 --file - --token t --model m",
        "bf record get U-2 && bf plan check",
        "bf record get U-2 2>&1 | head -5",
        "bf record get U-2 >&2",
        "bf record get U-2 2>/dev/null >&2",
        "f() { bf status; }; f",
        "if (true); then bf status; fi",
        "(bf status)",
        "for id in U-1 U-2; do bf record get $id; done",
        "cat <<'EOF'\n$((V)) ${(e)V}\nEOF",
    ] {
        let (d, why) = hook(&fx, Some(AGENT), "Bash", json!({"command": cmd}));
        assert_ne!(d, "deny", "{cmd}: {why}");
    }
}

#[test]
fn judge_guard_the_second_review_rows_are_refused_and_the_judges_reading_passes() {
    let fx = Fx::new();
    let v = "V='a[$(bf record put unit --file u.toml)]'";
    let g = "V='/(e:\"bf record put unit --file u.toml\":)'";
    let rows: Vec<String> = [
        // `>&word` is a descriptor only when the whole word is digits or `-`.
        "bf checkpoint >&2x check K-1 --accept X-2 --quote q",
        "bf checkpoint >&-x check K-1 --accept X-2 --quote q",
        "bf record >&2x put unit --file u.toml",
        // The engine's name in any case.
        "BF record put unit --file u.toml",
        "/usr/local/bin/Bf record put unit --file u.toml",
        "./target/release/bF checkpoint check K-1 --accept X-2 --quote q",
        // A `<<` in arithmetic takes no heredoc body, wherever the arithmetic stands.
        "time (( x = 1 <<2 ))\nbf record put unit --file u.toml\n2",
        "for((i=0;i<<2;i++)); do :; done\nbf record put unit --file u.toml\n2",
        "while((0 <<2)); do :; done\nbf record put unit --file u.toml\n2",
        "if((1 <<2)); then :; fi\nbf record put unit --file u.toml\n2",
        "function f { (( x = 1 <<2 )); }\nbf record put unit --file u.toml\n2",
        "{ : } always { (( x = 1 <<2 )) }\nbf record put unit --file u.toml\n2",
        "echo $[ 1 <<2 ]\nbf record put unit --file u.toml\n2",
        // A delimiter that holds `$( )` or `$'..'` is the shell's own.
        "cat <<$(x)\nhi\n$(x)\nbf record put unit --file u.toml",
        "cat <<$'E'\nhi\nE\nbf record put unit --file u.toml\n$'E'",
        // A `#` inside a zsh pattern group is no comment.
        "echo (#)x(N) ; bf record put unit --file u.toml",
        "echo (#)x(N) ; bf checkpoint check K-1 --accept X-2 --quote q",
        "setopt extendedglob; ls -d /(#qe:'bf record put unit --file u.toml':)",
        // zsh's `function f g { }` names two functions with one body.
        "function f g { bf record put unit --file u.toml }; f",
        "function f g { bf checkpoint check K-1 --accept X-2 --quote q }; g",
        // A first word with `=` before `(` is a pattern, not an array.
        "/de[=v](e:'bf record put unit --file u.toml':)",
        // Any character after `e` is read as a delimiter, failing closed.
        "ls -d /(eZ'bf record put unit --file u.toml'Z)",
        // Cargo's own options and a toolchain before `run`.
        "cargo -q run -- record put unit --file u.toml",
        "cargo +stable run -- record put unit --file u.toml",
        "cargo --offline --color never run -- checkpoint check K-1 --accept X-2 --quote q",
        // The command of `bf evidence run` with flags anywhere.
        "bf -C . evidence run X-1 -- bf record put unit --file u.toml",
        "bf evidence --unit U-1 run X-1 -- bf record put unit --file u.toml",
        // bash 5.3's `${ cmd; }`.
        "echo ${ bf record put unit --file u.toml; }",
        "echo ${| bf record put unit --file u.toml; }",
        // A whole call put in a variable, piped to a shell, or behind a shell or wrapper.
        "V='bf record put unit --file u.toml'; eval $V",
        "V='bf record put unit --file u.toml'; $=V",
        "V='bf record put unit --file u.toml'; $V",
        "print 'bf record put unit --file u.toml' | sh",
        "sh <(print 'bf record put unit --file u.toml')",
        "echo 'lmot.u elif-- tinu tup drocer fb' | rev | sh",
        "source =(print 'bf record put unit --file u.toml')",
        "zsh --emulate sh -c 'bf record put unit --file u.toml'",
        "tcsh -c 'bf record put unit --file u.toml'",
        "csh -c 'bf checkpoint check K-1 --accept X-2 --quote q'",
        "zmodload zsh/zpty; zpty q 'bf record put unit --file u.toml'",
        "eval \"$(printf 'bf record put unit --fi%s' 'le u.toml')\"",
        // Found in this round.
        "sh -c $'bf record p\\x75t unit --file u.toml'",
        "printf 'bf record pu%s unit --file u.toml' t | sh",
        "echo 'bf record put unit --file u.toml' > >(sh)",
        "echo 'bf checkpoint check K-1 --accept X-2 --quote q' | xargs -0 sh -c",
        "echo \"${ bf record put unit --file u.toml; }\"",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain(
        [
            // Arithmetic and globbing of a value the guard cannot read.
            "x=abc; echo ${x:V}",
            "x=(a b); echo $x[V]",
            "declare -i x=V",
            "typeset -i x=V",
            "integer x=V",
            "local -i x=V",
            "printf %d V",
            "print -f %d V",
            "repeat V :",
            "[ V -eq 0 ]",
        ]
        .iter()
        .map(|r| format!("{v}; {r}")),
    )
    .chain(
        ["echo $~V", "echo ${~V}", "ls $~V"]
            .iter()
            .map(|r| format!("{g}; {r}")),
    )
    .collect();
    for cmd in &rows {
        let (d, why) = hook(&fx, Some(AGENT), "Bash", json!({"command": cmd}));
        assert_eq!(d, "deny", "{cmd}");
        assert!(why.contains("repair judge"), "{cmd}: {why}");
    }
    // The judge's reading commands pass, a test of a file among them.
    for cmd in [
        "[[ -f u.toml ]] && bf record get U-2",
        "if [[ -n x ]]; then bf status; fi",
        "[[ $ID == U-* ]] && bf record get $ID",
        "test -f u.toml && cat u.toml",
        "grep -n depends_on u.toml | head -3",
        "bf record get U-2 | jq .files",
        "printf '%s\\n' x | head -1",
        "function f { bf status; }; f",
        "ls -la ${ID}",
        "/usr/bin/grep -c x u.toml",
    ] {
        let (d, why) = hook(&fx, Some(AGENT), "Bash", json!({"command": cmd}));
        assert_ne!(d, "deny", "{cmd}: {why}");
    }
}

#[test]
fn judge_guard_a_repair_that_names_a_marked_record_gets_no_judge_token() {
    let fx = Fx::new();
    let aid = ready(&fx);
    bf::ckfix::design::set_stub_answer(vec!["U-2".to_string()]);
    let p = fx.project();
    let r = bf::judge::token(&p, &aid);
    bf::ckfix::design::set_stub_answer(vec![]);
    match r {
        Err(bf::refusal::Fail::Refused(r)) => {
            assert!(
                r.because.contains("U-2") && r.because.contains("marked"),
                "{}",
                r.because
            )
        }
        other => panic!("expected a refusal, got {:?}", other.is_ok()),
    }
    // With the stub empty the token is issued.
    token(&fx, &aid);
}

#[test]
fn judge_guard_the_role_file_stays_within_two_hundred_lines_and_names_the_stdin_form() {
    let t = fs::read_to_string(role_source()).unwrap();
    assert!(t.lines().count() <= 200);
    assert!(t.contains("--file -") && t.contains("never through a file"));
    assert!(t.contains("model: opus"));
}

#[test]
fn judge_guard_the_human_marks_a_judged_ask_as_seen_with_their_own_words() {
    let fx = Fx::new();
    let aid = ready(&fx);
    let tok = token(&fx, &aid);
    hook(
        &fx,
        Some(AGENT),
        "Bash",
        json!({"command": judge_cmd(&aid, &tok)}),
    );
    let o = judge(&fx, &aid, &tok, "ask");
    assert_eq!(o.code, 0, "{}", o.all());
    // No human turn yet: refused.
    let o = fx.refused(&[
        "amend",
        "seen",
        &aid,
        "--quote",
        "I saw the judge's reasons",
    ]);
    assert!(!o.stderr.is_empty());
    fx.human("Yes, I saw the judge's reasons.");
    let o = fx.must(&[
        "amend",
        "seen",
        &aid,
        "--quote",
        "I saw the judge's reasons",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(kinds(&fx, "amend.judge.seen"), 1);
    // Replay took it: it is seen, so a second call is refused.
    let o = fx.refused(&[
        "amend",
        "seen",
        &aid,
        "--quote",
        "I saw the judge's reasons",
    ]);
    assert!(o.stderr.contains("already seen") || !o.stderr.is_empty());
    // An amendment with no verdict has nothing to see.
    let o = fx.refused(&["amend", "seen", "A-7", "--quote", "x"]);
    assert!(o.stderr.contains("A-7"), "{}", o.stderr);
}
