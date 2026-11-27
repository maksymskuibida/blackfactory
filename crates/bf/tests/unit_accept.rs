//! U-61 (R-71, H-24): the human accepts the findings of a parked unit in their own words, and the unit
//! goes to `reviewed` when every open finding is fixed or accepted.

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

/// U-1 started, its review listed F-1 and F-2 and parked; F-3 is a finding of U-1 in the records only.
fn parked() -> Fx {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    finding(&fx, "F-1", "high");
    finding(&fx, "F-2", "low");
    let p = fx.project();
    bf::events::append(
        &p,
        "unit.report",
        json!({"unit": "U-1", "result": "findings", "from": "review", "to": "parked",
               "findings": ["F-1", "F-2"], "round": 1, "content": "c", "head": "h", "covered": "all"}),
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

const WORDS: &str = "I accept both findings as they are";
const LATER: &str = "And I accept the rest of them too";

#[test]
fn a_high_and_a_low_finding_accepted_in_one_message_land_the_unit_with_no_new_round() {
    let fx = parked();
    assert_eq!(state_of(&fx, "U-1"), "parked");
    fx.human(WORDS);
    let o = fx
        .must(&[
            "unit", "accept", "U-1", "F-1", "F-2", "--quote", WORDS, "--json",
        ])
        .json();
    assert_eq!(o["state"], "reviewed", "{o}");
    assert_eq!(state_of(&fx, "U-1"), "reviewed");
    assert_eq!(outcome(&fx, "F-1"), "accepted");
    assert_eq!(outcome(&fx, "F-2"), "accepted");
    let log = std::fs::read_to_string(fx.project().log_path()).unwrap();
    let ev = log
        .lines()
        .filter(|l| l.contains("\"unit.finding.accept\""))
        .count();
    assert_eq!(ev, 1);
    assert!(log.contains("\"high\"") && log.contains(WORDS));
}

#[test]
fn words_written_before_the_park_are_refused() {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    finding(&fx, "F-1", "high");
    fx.human(WORDS);
    let p = fx.project();
    bf::events::append(
        &p,
        "unit.report",
        json!({"unit": "U-1", "result": "findings", "from": "review", "to": "parked",
               "findings": ["F-1"], "round": 1, "content": "c", "head": "h", "covered": "all"}),
    )
    .unwrap();
    bf::events::append(
        &p,
        "unit.park",
        json!({"unit": "U-1", "reason": "more than 3 review rounds"}),
    )
    .unwrap();
    let o = fx.refused(&["unit", "accept", "U-1", "F-1", "--quote", WORDS]);
    assert!(o.all().contains("before"), "{}", o.all());
    assert_eq!(state_of(&fx, "U-1"), "parked");
    assert_eq!(outcome(&fx, "F-1"), "");
}

#[test]
fn a_partial_accept_leaves_the_rest_open_and_the_unit_parked_until_every_finding_is_settled() {
    let fx = parked();
    finding(&fx, "F-3", "medium");
    fx.human(WORDS);
    // Names a finding the unit does not have.
    let o = fx.refused(&["unit", "accept", "U-1", "F-2", "F-9", "--quote", WORDS]);
    assert!(o.all().contains("F-9"), "{}", o.all());
    assert_eq!(outcome(&fx, "F-2"), "");
    // The low one alone: the high and the records-only medium stay open.
    let o = fx
        .must(&["unit", "accept", "U-1", "F-2", "--quote", WORDS, "--json"])
        .json();
    assert_eq!(o["state"], "parked", "{o}");
    assert_eq!(state_of(&fx, "U-1"), "parked");
    assert_eq!(outcome(&fx, "F-2"), "accepted");
    assert_eq!(outcome(&fx, "F-1"), "");
    // The rest later, in a new message, with the records-only one named beside a listed one.
    fx.human(LATER);
    let o = fx
        .must(&[
            "unit", "accept", "U-1", "F-1,F-3", "--quote", LATER, "--json",
        ])
        .json();
    assert_eq!(o["state"], "reviewed", "{o}");
    assert_eq!(outcome(&fx, "F-3"), "accepted");
}

#[test]
fn a_low_accepted_while_a_high_stays_open_parks_the_unit_and_the_high_accepted_later_reviews_it() {
    let fx = parked();
    fx.human(WORDS);
    let o = fx
        .must(&["unit", "accept", "U-1", "F-2", "--quote", WORDS, "--json"])
        .json();
    assert_eq!(o["state"], "parked", "{o}");
    assert_eq!(outcome(&fx, "F-2"), "accepted");
    assert_eq!(outcome(&fx, "F-1"), "");
    fx.human(LATER);
    let o = fx
        .must(&["unit", "accept", "U-1", "F-1", "--quote", LATER, "--json"])
        .json();
    assert_eq!(o["state"], "reviewed", "{o}");
    assert_eq!(state_of(&fx, "U-1"), "reviewed");
    assert_eq!(outcome(&fx, "F-1"), "accepted");
}

#[test]
fn an_accept_naming_only_records_only_findings_is_taken_and_the_unit_stays_parked() {
    let fx = parked();
    finding(&fx, "F-3", "medium");
    fx.human(WORDS);
    let o = fx
        .must(&["unit", "accept", "U-1", "F-3", "--quote", WORDS, "--json"])
        .json();
    assert_eq!(o["state"], "parked", "{o}");
    assert_eq!(outcome(&fx, "F-3"), "accepted");
    assert_eq!(state_of(&fx, "U-1"), "parked");
    // The rest, in a new message: the unit is reviewed.
    fx.human(LATER);
    fx.must(&["unit", "accept", "U-1", "F-1,F-2", "--quote", LATER]);
    assert_eq!(state_of(&fx, "U-1"), "reviewed");
}

#[test]
fn a_unit_that_is_not_parked_has_nothing_to_accept() {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.human(WORDS);
    let o = fx.refused(&["unit", "accept", "U-1", "F-1", "--quote", WORDS]);
    assert!(o.all().contains("running"), "{}", o.all());
}
