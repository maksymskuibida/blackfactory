//! U-139 (R-80): the human accepts a unit's finding that only a record holds, also alone, and a parked unit
//! goes to `reviewed` only when no finding of it, in the log or in a record, stays open.

mod common;
use common::*;
use serde_json::json;

fn finding(fx: &Fx, id: &str, severity: &str) {
    let o = fx.put(
        "finding",
        &format!(
            "id=\"{id}\"\nkind=\"finding\"\ntitle=\"t {id}\"\nunit=\"U-1\"\nseverity=\"{severity}\"\n"
        ),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

/// U-1 started with a unit review finding F-1 (listed by the log) and an auditor's finding F-2 (a record
/// only), parked in its review.
fn parked() -> Fx {
    let fx = Fx::tiny_run();
    let wc = std::path::PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    std::fs::create_dir_all(wc.join("src")).unwrap();
    std::fs::write(wc.join("src/hello.txt"), "from the unit\n").unwrap();
    fx.must_in(
        &wc,
        &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/hello.txt"],
    );
    let content = fx.project().content_hash(&wc).unwrap();
    finding(&fx, "F-1", "high");
    finding(&fx, "F-2", "medium");
    let p = fx.project();
    bf::events::append(
        &p,
        "unit.report",
        json!({"unit": "U-1", "result": "findings", "from": "review", "to": "parked",
               "findings": ["F-1"], "round": 1, "content": content, "head": "h", "covered": "all"}),
    )
    .unwrap();
    bf::events::append(
        &p,
        "unit.park",
        json!({"unit": "U-1", "reason": "more than 3 review rounds"}),
    )
    .unwrap();
    fx
}

fn state_of(fx: &Fx, id: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .map(|u| u["state"].as_str().unwrap().to_string())
        .unwrap_or_default()
}

fn outcome(fx: &Fx, id: &str) -> String {
    fx.must(&["record", "get", id, "--json"]).json()["outcome"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn accept_events(fx: &Fx) -> Vec<serde_json::Value> {
    std::fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e["kind"] == "unit.finding.accept")
        .collect()
}

const WORDS: &str = "I accept the audit finding as it is";
const LATER: &str = "And I accept the rest of them too";

#[test]
fn an_accept_of_only_an_auditors_record_finding_is_taken() {
    let fx = parked();
    fx.human(WORDS);
    let o = fx
        .must(&["unit", "accept", "U-1", "F-2", "--quote", WORDS, "--json"])
        .json();
    assert_eq!(o["state"], "parked", "{o}");
    assert_eq!(outcome(&fx, "F-2"), "accepted");
    assert_eq!(outcome(&fx, "F-1"), "");
    let evs = accept_events(&fx);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["records_only"], json!(["F-2"]));
    assert_eq!(evs[0]["settled_all"], json!(false));
}

#[test]
fn accepting_the_logged_finding_leaves_the_unit_parked_while_a_record_finding_is_open() {
    let fx = parked();
    fx.human(WORDS);
    let o = fx
        .must(&["unit", "accept", "U-1", "F-1", "--quote", WORDS, "--json"])
        .json();
    assert_eq!(o["state"], "parked", "{o}");
    assert_eq!(state_of(&fx, "U-1"), "parked");
    assert_eq!(outcome(&fx, "F-1"), "accepted");
    assert_eq!(outcome(&fx, "F-2"), "");
    assert_eq!(accept_events(&fx)[0]["settled_all"], json!(false));
}

#[test]
fn accepting_the_record_finding_next_sends_the_unit_to_reviewed_and_it_lands() {
    let fx = parked();
    fx.human(WORDS);
    fx.must(&["unit", "accept", "U-1", "F-1", "--quote", WORDS]);
    assert_eq!(state_of(&fx, "U-1"), "parked");
    fx.human(LATER);
    let o = fx
        .must(&["unit", "accept", "U-1", "F-2", "--quote", LATER, "--json"])
        .json();
    assert_eq!(o["state"], "reviewed", "{o}");
    assert_eq!(state_of(&fx, "U-1"), "reviewed");
    assert_eq!(outcome(&fx, "F-2"), "accepted");
    assert_eq!(accept_events(&fx)[1]["settled_all"], json!(true));
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state_of(&fx, "U-1"), "landed");
}

#[test]
fn an_old_accept_event_replays_as_before() {
    let fx = parked();
    let p = fx.project();
    fx.human(WORDS);
    let turn = bf::events::replay(&p).unwrap().turns.last().unwrap().seq;
    // An event of the old form: no `settled_all`; the log's findings are all accepted, so the unit is reviewed.
    bf::events::append(
        &p,
        "unit.finding.accept",
        json!({"unit": "U-1", "findings": [{"id": "F-1", "severity": "high"}], "records_only": [],
               "turn": turn, "quote": WORDS}),
    )
    .unwrap();
    assert_eq!(state_of(&fx, "U-1"), "reviewed");
}
