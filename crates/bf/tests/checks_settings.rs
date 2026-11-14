//! U-2: the check settings: proposal, recorded choice for the project and for one run, and state.

mod common;
use bf::checks;
use bf::events;
use common::Fx;

const OFF: &str = "secret-scan=off:no secrets can exist in a docs-only project";

fn names_off(v: &serde_json::Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["required"] == false)
        .map(|x| x["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_default_choice_needs_a_human_quote_after_the_proposal_and_shows_the_reason() {
    let fx = Fx::with_checks();
    fx.human("please keep the checks as they are, thanks");
    // Nothing proposed yet.
    let o = fx.refused(&["checks", "set", "--quote", "keep the checks"]);
    assert!(o.all().contains("no proposal"), "{}", o.all());

    let p = fx.must(&["checks", "propose", OFF, "--json"]).json();
    assert_eq!(p["changed"], false);
    assert!(p["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["name"] == "secret-scan" && x["change"] == "off" && x["reason"].is_string()));
    // The proposal changed nothing.
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert!(names_off(&s, "default").is_empty());

    // The only human message came before the proposal.
    let o = fx.refused(&["checks", "set", "--quote", "keep the checks"]);
    assert!(o.all().contains("human"), "{}", o.all());
    // No quote at all.
    fx.human("yes, switch that one off");
    fx.refused(&["checks", "set"]);
    // A quote that is not in the message.
    fx.refused(&["checks", "set", "--quote", "switch everything off"]);
    // A quote of a question.
    fx.human("Should I switch that one off? Is this right?");
    fx.refused(&["checks", "set", "--quote", "Is this right?"]);
    // Changes that differ from the proposal.
    fx.refused(&[
        "checks",
        "set",
        "accessibility=off:later",
        "--quote",
        "yes, switch that one off",
    ]);

    let o = fx.must(&["checks", "set", "--quote", "yes, switch that one off"]);
    assert!(o.stdout.contains("no secrets can exist"), "{}", o.stdout);
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert_eq!(names_off(&s, "default"), vec!["secret-scan"]);
    let file = std::fs::read_to_string(fx.root.join("factory/checks.json")).unwrap();
    assert!(file.contains("secret-scan"));
    // The message is used: it cannot confirm a second proposal.
    fx.must(&[
        "checks",
        "propose",
        "accessibility=off:not a screen product",
    ]);
    fx.refused(&["checks", "set", "--quote", "yes, switch that one off"]);
}

#[test]
fn a_check_switched_off_needs_a_reason_and_a_known_name() {
    let fx = Fx::with_checks();
    fx.refused(&["checks", "propose", "secret-scan=off:"]);
    fx.refused(&["checks", "propose", "secret-scan=off"]);
    fx.refused(&["checks", "propose", "no-such-check=off:x"]);
    fx.refused(&["checks", "propose", "secret-scan=off:x", "--scope", "run"]);
}

#[test]
fn a_new_run_starts_from_the_default_and_a_run_choice_leaves_the_default_alone() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.human("yes, test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, test run with simulated approvals",
    ]);
    // Set the project default before any run.
    fx.must(&["checks", "propose", OFF, "--scope", "default"]);
    fx.human("agreed, secret scan is off for the project");
    fx.must(&["checks", "set", "--quote", "agreed, secret scan is off"]);

    fx.must(&["run", "start"]);
    let log = std::fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    let start = log
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|e| e["kind"] == "run.start")
        .unwrap();
    assert_eq!(
        start["checks_off"]["secret-scan"],
        "no secrets can exist in a docs-only project"
    );
    let st = events::replay(&fx.project()).unwrap();
    let names: Vec<_> = checks::required_in_run(&st, "run-1")
        .iter()
        .map(|c| c.name)
        .collect();
    assert!(!names.contains(&"secret-scan"));
    assert!(names.contains(&"accessibility"));

    // A change for this run only.
    fx.must(&[
        "checks",
        "propose",
        "legal-audit=off:the run changes no code",
        "secret-scan=on",
    ]);
    fx.human("fine, do it for this run only");
    fx.must(&["checks", "set", "--quote", "do it for this run only"]);
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert_eq!(names_off(&s, "run_checks"), vec!["legal-audit"]);
    assert_eq!(names_off(&s, "default"), vec!["secret-scan"]);
    let st = events::replay(&fx.project()).unwrap();
    assert!(checks::required_now(&st)
        .iter()
        .any(|c| c.name == "secret-scan"));
    assert!(!checks::required_now(&st)
        .iter()
        .any(|c| c.name == "legal-audit"));

    // The next run starts from the default again.
    fx.must(&["run", "stop"]);
    fx.must(&["run", "start"]);
    let st = events::replay(&fx.project()).unwrap();
    let names: Vec<_> = checks::required_in_run(&st, "run-2")
        .iter()
        .map(|c| c.name)
        .collect();
    assert!(names.contains(&"legal-audit"));
    assert!(!names.contains(&"secret-scan"));
    // The first run keeps its own choice.
    let first: Vec<_> = checks::required_in_run(&st, "run-1")
        .iter()
        .map(|c| c.name)
        .collect();
    assert!(!first.contains(&"legal-audit"));
}

#[test]
fn a_setting_event_without_a_valid_turn_is_ignored_on_replay() {
    let fx = Fx::with_checks();
    fx.must(&["checks", "propose", OFF, "--scope", "default"]);
    let p = fx.project();
    // An agent that writes the event itself, with no human turn behind it.
    events::append(
        &p,
        "checks.set",
        serde_json::json!({"scope":"default","proposal":1,"turn":1,"quote":"x",
            "changes": [{"check":"secret-scan","off":"because"}]}),
    )
    .unwrap();
    let st = events::replay(&p).unwrap();
    assert!(st.checks_default.is_empty());
    assert!(!st.ignored.is_empty());
}

fn start_test_run(fx: &Fx) {
    fx.tiny();
    fx.name_every_check("R-1");
    fx.human("yes, test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, test run with simulated approvals",
    ]);
    fx.must(&["run", "start"]);
}

#[test]
fn a_proposal_is_confirmed_once_and_only_in_its_own_run() {
    let fx = Fx::with_checks();
    start_test_run(&fx);
    fx.must(&["checks", "propose", OFF]);
    fx.human("yes, switch that one off");
    fx.must(&["checks", "set", "--quote", "yes, switch that one off"]);
    // Already recorded: a later message cannot record it again.
    fx.human("yes, once more please");
    let o = fx.refused(&["checks", "set", "--quote", "yes, once more please"]);
    assert!(o.all().contains("no proposal"), "{}", o.all());
    // A proposal of run-1 cannot be recorded into run-2.
    fx.must(&["checks", "propose", "legal-audit=off:later"]);
    fx.must(&["run", "stop"]);
    fx.must(&["run", "start"]);
    fx.human("yes, do the legal one too");
    fx.refused(&["checks", "set", "--quote", "yes, do the legal one too"]);
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert!(names_off(&s, "run_checks")
        .iter()
        .all(|n| n != "legal-audit"));
}

#[test]
fn a_proposal_for_both_shows_the_default_and_the_run() {
    let fx = Fx::with_checks();
    start_test_run(&fx);
    let p = fx
        .must(&["checks", "propose", OFF, "--scope", "both", "--json"])
        .json();
    assert!(p["default_checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["name"] == "secret-scan" && x["reason"].is_string()));
    let o = fx.must(&["checks", "propose", OFF, "--scope", "both"]);
    assert!(o.stdout.contains("project default"), "{}", o.stdout);
}

#[test]
fn the_run_page_shows_the_reason_of_a_switched_off_check() {
    let fx = Fx::with_checks();
    fx.must(&["checks", "propose", OFF, "--scope", "default"]);
    fx.human("agreed, secret scan is off for the project");
    fx.must(&["checks", "set", "--quote", "agreed, secret scan is off"]);
    let v = fx.must(&["view", "run"]).json();
    assert!(v.to_string().contains("no secrets can exist"), "{v}");
}

#[test]
fn a_setting_that_cites_an_older_proposal_than_the_one_answered_is_ignored_on_replay() {
    let fx = Fx::with_checks();
    fx.must(&["checks", "propose", OFF, "--scope", "default"]);
    let a = events::replay(&fx.project()).unwrap().checks_proposals[0].seq;
    fx.must(&[
        "checks",
        "propose",
        "accessibility=on",
        "--scope",
        "default",
    ]);
    fx.human("yes, go ahead with the harmless one");
    let st = events::replay(&fx.project()).unwrap();
    let turn = st.turns.last().unwrap().seq;
    // The agent cites proposal A against the human's answer to proposal B.
    events::append(
        &fx.project(),
        "checks.set",
        serde_json::json!({"scope":"default","proposal":a,"turn":turn,
            "quote":"yes, go ahead with the harmless one",
            "changes": [{"check":"secret-scan","off":"no secrets can exist in a docs-only project"}]}),
    )
    .unwrap();
    let st = events::replay(&fx.project()).unwrap();
    assert!(st.checks_default.is_empty());
    assert!(!st.ignored.is_empty());
}
