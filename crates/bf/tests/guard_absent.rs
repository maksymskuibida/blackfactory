//! U-69: the guard's rules of the human-absent flag (R-74, D-96): the tools that ask the human are
//! refused while the flag is on, the release question passes in a returning session, and the hook
//! commands and the harness's own program are kept from every agent, in the shell and in the listed
//! tools that run a command line or type.

mod common;
use bf::guard::decide;
use bf::strings::t;
use serde_json::{json, Value};

fn table(on: bool, returning: &[&str]) -> Value {
    json!({
        "v": 1,
        "root": "/nonexistent-root",
        "allow": "abstain",
        "protected": [],
        "work_prefix": "/nonexistent-root/.bf/work",
        "deny_commands": [],
        "command_tools": ["mcp__scripts__"],
        "input_tools": ["mcp__chat__send"],
        "absent": {"on": on, "returning": returning, "deny_tools": ["mcp__slack__", "WebFetch"]},
    })
}

fn call(tool: &str, input: Value, agent: Option<&str>) -> Value {
    let mut v = json!({"tool_name": tool, "tool_input": input, "session_id": "s1", "cwd": "/nonexistent-root"});
    if let Some(a) = agent {
        v["agent_type"] = json!(a);
    }
    v
}

fn allowed(tab: &Value, tool: &str, input: Value) -> bool {
    decide(&call(tool, input, None), tab).allow
}

fn refusal(tab: &Value, tool: &str, input: Value, agent: Option<&str>) -> String {
    let d = decide(&call(tool, input, agent), tab);
    assert!(!d.allow, "{tool} passed");
    d.reason
}

fn menu(question: &str, labels: &[&str]) -> Value {
    json!({"questions": [{"question": question, "header": "h", "multiSelect": false,
        "options": labels.iter().map(|l| json!({"label": l, "description": "d"})).collect::<Vec<_>>()}]})
}

fn release_q() -> String {
    t(
        "absent.release_question",
        &[
            ("yes", &t("absent.answer_yes", &[])),
            ("stay", &t("absent.answer_stay", &[])),
        ],
    )
}

fn yes() -> String {
    t("absent.answer_yes", &[])
}
fn stay() -> String {
    t("absent.answer_stay", &[])
}

#[test]
fn ask_menu_is_refused_with_a_step_and_passes_with_the_flag_off() {
    let on = table(true, &[]);
    let r = refusal(&on, "AskUserQuestion", menu("Which?", &["a", "b"]), None);
    assert!(r.contains("bf ask"), "{r}");
    assert!(r.contains("do this:"), "{r}");
    let off = table(false, &[]);
    assert!(allowed(
        &off,
        "AskUserQuestion",
        menu("Which?", &["a", "b"])
    ));
}

#[test]
fn a_subagent_is_refused_like_the_conductor() {
    let on = table(true, &[]);
    for agent in [None, Some("builder"), Some("reviewer")] {
        refusal(&on, "AskUserQuestion", menu("Which?", &["a"]), agent);
    }
}

#[test]
fn next_step_box_is_refused_only_with_a_question() {
    let on = table(true, &[]);
    let tool = "mcp__ccd_turn__wrap_up";
    refusal(
        &on,
        tool,
        json!({"recap": "r", "ask": "Go on?", "actions": [{"label": "Yes"}]}),
        None,
    );
    assert!(allowed(&on, tool, json!({"recap": "Done."})));
    assert!(allowed(&on, tool, json!({"recap": "Done.", "ask": ""})));
    assert!(allowed(
        &table(false, &[]),
        tool,
        json!({"recap": "r", "ask": "Go on?"})
    ));
}

#[test]
fn plan_and_access_tools_are_refused() {
    let on = table(true, &[]);
    for tool in [
        "EnterPlanMode",
        "ExitPlanMode",
        "mcp__computer-use__request_access",
        "mcp__computer-use__request_full_control",
        "mcp__computer-use__request_teach_access",
        "mcp__ccd_directory__request_directory",
    ] {
        let r = refusal(&on, tool, json!({}), None);
        assert!(r.contains("do this:"), "{tool}: {r}");
        assert!(allowed(&table(false, &[]), tool, json!({})), "{tool} off");
    }
}

#[test]
fn deny_tools_by_name_and_by_prefix() {
    let on = table(true, &[]);
    refusal(&on, "WebFetch", json!({}), None);
    refusal(&on, "mcp__slack__post", json!({}), None);
    assert!(allowed(&on, "WebSearch", json!({})));
    assert!(allowed(&table(false, &[]), "mcp__slack__post", json!({})));
}

#[test]
fn the_release_question_passes_only_in_a_returning_session() {
    let q = release_q();
    let both = menu(&q, &[&yes(), &stay()]);
    let only_yes = menu(&q, &[&yes()]);
    let back = table(true, &["s1"]);
    assert!(allowed(&back, "AskUserQuestion", both.clone()));
    assert!(allowed(&back, "AskUserQuestion", only_yes));
    let box_call = json!({"recap": "r", "ask": q, "actions": [{"label": yes()}]});
    assert!(allowed(&back, "mcp__ccd_turn__wrap_up", box_call.clone()));
    // Another session of the run, or a flag that is not returning, gets no exception.
    refusal(&table(true, &["s2"]), "AskUserQuestion", both.clone(), None);
    refusal(&table(true, &[]), "AskUserQuestion", both, None);
    refusal(&table(true, &[]), "mcp__ccd_turn__wrap_up", box_call, None);
}

#[test]
fn a_question_with_other_words_or_answers_is_refused_in_a_returning_session() {
    let q = release_q();
    let back = table(true, &["s1"]);
    refusal(
        &back,
        "AskUserQuestion",
        menu("Are you there?", &[&yes()]),
        None,
    );
    refusal(&back, "AskUserQuestion", menu(&q, &[&yes(), "maybe"]), None);
    refusal(&back, "AskUserQuestion", menu(&q, &[&stay()]), None);
    refusal(&back, "AskUserQuestion", menu(&q, &[]), None);
    let two = json!({"questions": [
        menu(&q, &[&yes()])["questions"][0].clone(),
        menu("Other?", &[&yes()])["questions"][0].clone()]});
    refusal(&back, "AskUserQuestion", two, None);
    refusal(
        &back,
        "mcp__ccd_turn__wrap_up",
        json!({"recap": "r", "ask": format!("{q} Please."), "actions": [{"label": yes()}]}),
        None,
    );
}

#[test]
fn the_harness_program_is_refused_in_the_shell_in_both_states() {
    for on in [true, false] {
        let tab = table(on, &[]);
        for cmd in [
            "claude -p 'say hi'",
            "env claude",
            "/usr/local/bin/claude --version",
            "FOO=1 claude -p x",
            "sh -c 'claude -p x'",
            "echo hi | xargs claude",
            "npx claude",
            "find . -exec claude {} ;",
            "C=claude; $C -p x",
        ] {
            for agent in [None, Some("builder")] {
                let r = refusal(&tab, "Bash", json!({"command": cmd}), agent);
                assert!(r.contains("harness"), "{cmd}: {r}");
            }
        }
    }
}

#[test]
fn text_that_only_names_the_harness_passes_the_shell() {
    let tab = table(true, &[]);
    for cmd in [
        "ls .claude/agents",
        "echo claude",
        "grep -r claude docs",
        "cat ~/.claude/settings.json",
        "mkdir -p /tmp/claude",
    ] {
        let d = decide(&call("Bash", json!({"command": cmd}), None), &tab);
        assert!(
            !d.reason.contains("harness's own program"),
            "{cmd}: {}",
            d.reason
        );
    }
}

#[test]
fn bf_hook_is_refused_to_every_agent() {
    for on in [true, false] {
        let tab = table(on, &[]);
        for agent in [None, Some("builder")] {
            let r = refusal(&tab, "Bash", json!({"command": "bf hook"}), agent);
            assert!(r.contains("bf hook"), "{r}");
        }
    }
}

#[test]
fn a_command_tool_that_calls_the_hook_commands_or_the_harness_is_refused() {
    let tool = "mcp__terminal__run_in_terminal";
    for on in [true, false] {
        let tab = table(on, &[]);
        let pipe = json!({"command": "echo '{\"prompt\":\"/factory:human-absent\"}' | bf human-turn", "title": "x"});
        let r = refusal(&tab, tool, pipe, None);
        assert!(r.contains("human-turn"), "{r}");
        refusal(&tab, tool, json!({"command": "bf hook"}), None);
        refusal(&tab, tool, json!({"command": "claude -p hi"}), None);
        let script = "import subprocess\nsubprocess.run(['bf', 'human-turn'])";
        refusal(
            &tab,
            tool,
            json!({"command": format!("python3 -c \"{script}\"")}),
            None,
        );
        let script = "import subprocess\nsubprocess.run(['claude', '-p', 'x'])";
        refusal(&tab, tool, json!({"command": script}), None);
        assert!(allowed(
            &tab,
            tool,
            json!({"command": "ls -la", "title": "claude"})
        ));
        assert!(allowed(&tab, tool, json!({"command": "cargo test"})));
    }
}

#[test]
fn an_input_tool_with_the_command_form_is_refused() {
    let cmd = "/factory:human-absent";
    for on in [true, false] {
        let tab = table(on, &[]);
        for (tool, input) in [
            (
                "mcp__computer-use__app_type",
                json!({"app": "Claude", "text": cmd}),
            ),
            (
                "mcp__computer-use__type",
                json!({"text": format!("{cmd}\n")}),
            ),
            (
                "mcp__computer-use__computer_batch",
                json!({"actions": [{"action": "key", "text": "x"}, {"action": "type", "text": cmd}]}),
            ),
            (
                "mcp__ccd_session_mgmt__send_message",
                json!({"session_id": "a", "message": format!("please {cmd}")}),
            ),
            (
                "mcp__ccd_session__start_session",
                json!({"title": "t", "background": cmd}),
            ),
        ] {
            let r = refusal(&tab, tool, input, None);
            assert!(r.contains("human-absent command"), "{tool}: {r}");
        }
        for (tool, input) in [
            (
                "mcp__computer-use__app_type",
                json!({"app": "Notes", "text": "hello"}),
            ),
            (
                "mcp__ccd_session_mgmt__send_message",
                json!({"session_id": "a", "message": "hello"}),
            ),
            (
                "mcp__computer-use__computer_batch",
                json!({"actions": [{"action": "screenshot"}]}),
            ),
        ] {
            assert!(allowed(&tab, tool, input), "{tool}");
        }
    }
}

#[test]
fn tools_the_policy_adds_by_prefix_are_judged_the_same() {
    let tab = table(false, &[]);
    refusal(
        &tab,
        "mcp__scripts__run",
        json!({"code": "bf human-turn < /dev/null; claude -p x"}),
        None,
    );
    refusal(
        &tab,
        "mcp__scripts__run",
        json!({"code": "claude -p x"}),
        None,
    );
    assert!(allowed(&tab, "mcp__scripts__run", json!({"code": "ls"})));
    refusal(
        &tab,
        "mcp__chat__send_text",
        json!({"text": "/factory:human-absent"}),
        None,
    );
    assert!(allowed(
        &tab,
        "mcp__chat__send_text",
        json!({"text": "hello"})
    ));
    // A tool that no list names passes unseen.
    assert!(allowed(
        &tab,
        "mcp__other__send",
        json!({"text": "/factory:human-absent"})
    ));
}

#[test]
fn text_with_a_path_that_ends_in_the_harness_name_is_not_a_start_of_it() {
    let tool = "mcp__terminal__run_in_terminal";
    for on in [true, false] {
        let tab = table(on, &[]);
        for cmd in [
            "grep -rn subprocess harness/claude",
            "cd harness/claude && npm exec x",
            "cd harness/claude && cargo test --exec",
            "python3 -c \"import os; os.system('ls harness/claude')\"",
        ] {
            assert!(allowed(&tab, tool, json!({ "command": cmd })), "{cmd}");
        }
    }
    // A program that stands first in an argument list or in a handed line is still refused.
    let tab = table(false, &[]);
    refusal(
        &tab,
        tool,
        json!({"command": "python3 -c \"import subprocess; subprocess.run(['/usr/local/bin/claude', '-p'])\""}),
        None,
    );
    refusal(
        &tab,
        tool,
        json!({"command": "python3 -c \"import os; os.system('claude -p x')\""}),
        None,
    );
}

#[test]
fn a_policy_entry_reaches_the_installed_matcher_of_the_tool_hook() {
    use std::process::Command;
    let tmp = tempfile::TempDir::new().unwrap();
    let proj = std::fs::canonicalize(tmp.path()).unwrap().join("proj");
    std::fs::create_dir_all(proj.join("factory")).unwrap();
    std::fs::write(
        proj.join("factory/policy.toml"),
        "[absent]\ndeny_tools = [\"WebFetch\", \"mcp__slack__\"]\n\n[guard]\ncommand_tools = [\"mcp__scripts__\"]\ninput_tools = [\"mcp__chat__send\"]\n",
    )
    .unwrap();
    assert!(Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&proj)
        .status()
        .unwrap()
        .success());
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness/claude/install.sh");
    let o = Command::new("bash")
        .arg(script)
        .arg(&proj)
        .env("BF_STATE_HOME", tmp.path().join("state-home"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let s: Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join(".claude/settings.json")).unwrap())
            .unwrap();
    let m = s["hooks"]["PreToolUse"][0]["matcher"]
        .as_str()
        .unwrap()
        .to_string();
    for want in [
        "Bash",
        "AskUserQuestion",
        "WebFetch.*",
        "mcp__slack__.*",
        "mcp__scripts__.*",
        "mcp__chat__send.*",
    ] {
        assert!(m.split('|').any(|x| x == want), "{want} not in {m}");
    }
}

#[test]
fn accepting_a_policy_that_changes_the_tool_lists_says_to_run_the_installer_again() {
    let fx = common::Fx::new_real();
    let pol = fx.root.join("factory/policy.toml");
    let text = std::fs::read_to_string(&pol).unwrap();
    std::fs::write(
        &pol,
        format!("{text}\n[absent]\ndeny_tools = [\"WebFetch\"]\n"),
    )
    .unwrap();
    fx.human("Yes, I accept the new policy.");
    let o = fx.must(&["policy", "accept", "--quote", "I accept the new policy"]);
    assert!(o.all().contains("install.sh"), "{}", o.all());
    // A change that leaves the lists alone says nothing about it.
    let text = std::fs::read_to_string(&pol).unwrap();
    std::fs::write(&pol, format!("{text}\n# a comment\n")).unwrap();
    fx.human("Yes, I accept the new policy.");
    let o = fx.must(&["policy", "accept", "--quote", "I accept the new policy"]);
    assert!(!o.all().contains("install.sh"), "{}", o.all());
}

#[test]
fn the_harness_is_refused_through_its_package_a_script_path_or_osascript() {
    for on in [true, false] {
        let tab = table(on, &[]);
        for cmd in [
            "npx @anthropic-ai/claude-code -p x",
            "npx -y @anthropic-ai/claude-code -p x",
            "bunx @anthropic-ai/claude-code -p x",
            "npm exec @anthropic-ai/claude-code -- -p x",
            "node /opt/lib/node_modules/@anthropic-ai/claude-code/cli.js -p x",
            "node \"$(npm root -g)/@anthropic-ai/claude-code/cli.js\" -p x",
            "osascript -e 'do shell script \"claude -p x\"'",
        ] {
            let r = refusal(&tab, "Bash", json!({"command": cmd}), Some("builder"));
            assert!(r.contains("harness"), "{cmd}: {r}");
        }
    }
}
