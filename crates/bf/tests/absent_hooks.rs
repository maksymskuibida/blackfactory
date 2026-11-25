//! The engine's answers to the harness's other hooks while the run's human-absent flag is on
//! (U-72, R-74, R-72, D-96).

mod common;
use common::*;
use serde_json::{json, Value};

const CMD: &str = "/factory:human-absent";

fn say(fx: &Fx, session: &str, text: &str) {
    let payload = json!({
        "session_id": session,
        "cwd": fx.root.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "UserPromptSubmit",
        "prompt": text,
    });
    let o = fx.bf_stdin(&fx.root, &["human-turn"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

fn hook(fx: &Fx, event: &str, session: &str, extra: Value) -> Out {
    let mut payload = json!({"session_id": session, "cwd": fx.root.to_string_lossy()});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        payload[k] = v;
    }
    let o = fx.bf_stdin(&fx.root, &["hook", event], &payload.to_string());
    assert_eq!(o.code, 0, "a hook always exits 0: {}", o.all());
    o
}

fn status(fx: &Fx) -> Value {
    fx.must(&["absent", "status", "--json"]).json()
}

fn on(fx: &Fx) -> bool {
    status(fx)["on"] == true
}

fn question() -> String {
    "You wrote to the run while the human-absent flag is on. Are you back? Answer `yes` to end the flag, or `stay absent` to keep it on."
        .to_string()
}

fn ask_payload(answer: &str) -> Value {
    let q = question();
    json!({
        "tool_name": "AskUserQuestion",
        "tool_input": {"questions": [{"question": q, "options": [{"label": "yes"}, {"label": "stay absent"}]}]},
        "tool_response": {"answers": {q.clone(): answer}},
    })
}

#[test]
fn a_permission_prompt_is_answered_no_while_the_flag_is_on_and_not_otherwise() {
    let fx = Fx::tiny_run();
    let o = hook(&fx, "permission", "s1", json!({"tool_name": "Bash"}));
    assert_eq!(o.stdout.trim(), "", "flag off: nothing is printed");
    say(&fx, "s1", CMD);
    let o = hook(&fx, "permission", "other", json!({"tool_name": "Bash"}));
    let d = &o.json()["hookSpecificOutput"];
    assert_eq!(d["hookEventName"], "PermissionRequest");
    assert_eq!(d["decision"]["behavior"], "deny");
    assert!(d["decision"]["message"].as_str().unwrap().contains("bf ask"));
    let o = hook(&fx, "permission", "s1", json!({}));
    assert_eq!(o.json()["hookSpecificOutput"]["decision"]["behavior"], "deny");
}

#[test]
fn a_tool_servers_request_is_declined() {
    let fx = Fx::tiny_run();
    say(&fx, "s1", CMD);
    let o = hook(&fx, "elicitation", "s1", json!({"mcp_server_name": "x"}));
    assert_eq!(o.json()["hookSpecificOutput"]["action"], "decline");
}

#[test]
fn an_early_stop_with_a_ready_unit_is_turned_back_once_for_each_head() {
    let fx = Fx::tiny_run();
    assert_eq!(hook(&fx, "stop", "s1", json!({})).stdout.trim(), "");
    say(&fx, "s1", CMD);
    let o = hook(&fx, "stop", "s1", json!({}));
    let v = o.json();
    assert_eq!(v["decision"], "block");
    assert!(v["reason"].as_str().unwrap().contains("start-unit"));
    assert_eq!(
        hook(&fx, "stop", "s1", json!({})).stdout.trim(),
        "",
        "a second stop at the same head passes"
    );
}

#[test]
fn a_subagents_stop_and_a_returning_sessions_stop_pass() {
    let fx = Fx::tiny_run();
    say(&fx, "s1", CMD);
    let o = hook(&fx, "stop", "s1", json!({"agent_id": "a1"}));
    assert_eq!(o.stdout.trim(), "");
    say(&fx, "s1", "hello");
    assert_eq!(hook(&fx, "stop", "s1", json!({})).stdout.trim(), "");
}

#[test]
fn a_stop_after_a_stay_is_turned_back_again() {
    let fx = Fx::tiny_run();
    say(&fx, "s1", CMD);
    say(&fx, "s1", "hello");
    assert_eq!(hook(&fx, "stop", "s1", json!({})).stdout.trim(), "");
    let o = hook(&fx, "ask-answer", "s1", ask_payload("stay absent"));
    assert!(o.stdout.contains("fully on again"), "{}", o.all());
    assert!(on(&fx));
    assert_eq!(hook(&fx, "stop", "s1", json!({})).json()["decision"], "block");
}

#[test]
fn the_ask_menus_yes_releases_only_where_the_flag_is_returning() {
    let fx = Fx::tiny_run();
    say(&fx, "s1", CMD);
    let o = hook(&fx, "ask-answer", "s1", ask_payload("yes"));
    assert_eq!(o.stdout.trim(), "", "not returning yet");
    assert!(on(&fx));
    say(&fx, "s1", "hello");
    let mut other = ask_payload("yes");
    other["tool_input"]["questions"][0]["question"] = json!("Something else?");
    other["tool_response"] = json!({"answers": {"Something else?": "yes"}});
    hook(&fx, "ask-answer", "s1", other);
    assert!(on(&fx), "another question changes nothing");
    let mut sub = ask_payload("yes");
    sub["agent_id"] = json!("a1");
    hook(&fx, "ask-answer", "s1", sub);
    assert!(on(&fx), "a subagent's answer changes nothing");
    let o = hook(&fx, "ask-answer", "s1", ask_payload("yes"));
    assert!(!on(&fx));
    assert!(o.json()["systemMessage"].as_str().unwrap().contains("off"));
    assert_eq!(status(&fx)["last_release"]["by"], "ask");
}

#[test]
fn the_start_of_a_session_prints_the_flag_and_the_instruction() {
    let fx = Fx::tiny_run();
    assert_eq!(hook(&fx, "start", "n1", json!({"source": "startup"})).stdout.trim(), "");
    say(&fx, "s1", CMD);
    let o = hook(&fx, "start", "n2", json!({"source": "resume"}));
    let c = o.json()["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(c.contains("human absent since"), "{c}");
    assert!(c.contains("bf ask"), "{c}");
}

#[test]
fn a_notice_that_the_session_waits_is_journalled_and_shown() {
    let fx = Fx::tiny_run();
    say(&fx, "s1", CMD);
    let o = hook(&fx, "idle", "s1", json!({"message": "Claude is waiting for your input"}));
    assert_eq!(o.stdout.trim(), "");
    let v = status(&fx);
    assert!(v["waits"][0].as_str().unwrap().contains("waiting for your input"));
    let text = fx.must(&["absent", "status"]).stdout;
    assert!(text.contains("Waiting for input"), "{text}");
}

#[test]
fn an_event_never_reached_is_named_until_it_is_called() {
    let fx = Fx::tiny_run();
    let v = status(&fx);
    let never = |v: &Value| -> Vec<String> {
        v["hooks_never_reached"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    assert!(never(&v).contains(&"permission".to_string()));
    let text = fx.must(&["absent", "status"]).stdout;
    assert!(text.contains("permission (PermissionRequest)"), "{text}");
    hook(&fx, "permission", "s1", json!({}));
    assert!(!never(&status(&fx)).contains(&"permission".to_string()));
    assert!(never(&status(&fx)).contains(&"stop".to_string()));
    let o = {
        let payload = json!({"session_id": "s1", "cwd": fx.root.to_string_lossy(),
            "permission_mode": "default", "hook_event_name": "UserPromptSubmit", "prompt": CMD});
        fx.bf_stdin(&fx.root, &["human-turn"], &payload.to_string())
    };
    assert!(o.stdout.contains("not yet called"), "{}", o.all());
}

#[test]
fn the_fragment_has_one_entry_for_each_event_of_the_adapter() {
    let frag: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../harness/claude/settings.fragment.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for e in bf::adapter::DATA.hook_events {
        let entries = frag["hooks"][e.harness].as_array().unwrap_or_else(|| panic!("{}", e.harness));
        let want = format!("bf hook {}", e.event);
        let hit = entries
            .iter()
            .find(|x| x["hooks"][0]["command"] == want.as_str())
            .unwrap_or_else(|| panic!("{want}"));
        assert_eq!(hit["matcher"].as_str().unwrap_or(""), e.matcher);
        assert_eq!(hit["hooks"][0]["timeout"], e.timeout);
        if !e.answer.is_empty() {
            assert!(serde_json::from_str::<Value>(e.answer).is_ok(), "{}", e.event);
        }
    }
}

/// A unit started, its file written, its check run and `built` reported: left in review.
fn built_not_landed(fx: &Fx, id: &str, file: &str, check: &str) {
    let o = fx.must(&["unit", "start", id, "--json"]);
    let wc = std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    std::fs::create_dir_all(wc.join("src")).unwrap();
    std::fs::write(wc.join(file), format!("{id} content\n")).unwrap();
    fx.must_in(&wc, &["evidence", "run", check, "--unit", id, "--", "test", "-f", file]);
    fx.must(&["report", id, "--result", "built"]);
}

fn only_step(fx: &Fx, role: &str, action: &str) {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "{n}");
    assert_eq!(ds[0]["role"], role, "{n}");
    assert_eq!(ds[0]["action"], action, "{n}");
}

fn stop_turned_back_naming(fx: &Fx, action: &str) {
    say(fx, "s1", CMD);
    let v = hook(fx, "stop", "s1", json!({})).json();
    assert_eq!(v["decision"], "block", "{v}");
    assert!(v["reason"].as_str().unwrap().contains(action), "{v}");
    assert_eq!(hook(fx, "stop", "s1", json!({})).stdout.trim(), "");
}

#[test]
fn a_stop_with_only_a_land_step_is_turned_back() {
    let fx = Fx::tiny_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    built_not_landed(&fx, "U-2", "src/bye.txt", "X-2");
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    only_step(&fx, "engine", "land");
    stop_turned_back_naming(&fx, "land");
}

#[test]
fn a_stop_with_only_a_close_checkpoint_step_is_turned_back() {
    let fx = Fx::tiny_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-3", "--", "ls", "src/hello.txt", "src/bye.txt"],
    );
    only_step(&fx, "engine", "close-checkpoint");
    stop_turned_back_naming(&fx, "close-checkpoint");
}

#[test]
fn a_stop_with_only_a_reviewers_step_is_turned_back() {
    let fx = Fx::tiny_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    built_not_landed(&fx, "U-2", "src/bye.txt", "X-2");
    only_step(&fx, "reviewer", "review");
    stop_turned_back_naming(&fx, "review");
}

#[test]
fn a_stop_with_only_a_human_step_passes() {
    let fx = Fx::tiny_project();
    fx.must(&["run", "start"]);
    let n = fx.must(&["next", "--json"]).json();
    assert!(n["directives"].as_array().unwrap().iter().all(|d| d["role"] == "human"), "{n}");
    say(&fx, "s1", CMD);
    assert_eq!(hook(&fx, "stop", "s1", json!({})).stdout.trim(), "");
}
