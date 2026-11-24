//! U-7: the legal engine. Markets and kinds of risk are free words, a finding that claims research
//! lists its sources, and a finding with the lawyer mark holds the run open until the human answers
//! its closing question after a message of their own.

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;

fn source() -> Value {
    json!({"url": "https://example.org/law", "title": "The act", "read_on": "2026-11-01",
           "grade": "official", "why": "It is the publisher of the act."})
}

fn finding(mark: bool) -> Value {
    json!({"title": "Consent for tracking", "markets": ["Atlantis", "the Northern Union"],
           "risk": "weird new thing no list has", "researched": true, "sources": [source()],
           "lawyer_check": mark, "severity": "high"})
}

fn file(fx: &Fx, name: &str, v: &Value) -> String {
    let f = fx.root.join(name);
    fs::write(&f, serde_json::to_string(v).unwrap()).unwrap();
    f.to_str().unwrap().to_string()
}

#[test]
fn a_finding_names_any_market_and_any_kind_of_risk() {
    let fx = Fx::new();
    let f = file(&fx, "f.json", &finding(false));
    let o = fx.must(&["legal", "finding", &f, "--json"]);
    let id = o.json()["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("L-"));
    let rec = fx.must(&["record", "get", &id, "--json"]).json();
    assert_eq!(rec["market"], "Atlantis, the Northern Union");
    let l = fx.must(&["legal", "list", "--json"]).json();
    assert_eq!(l["findings"][0]["risk"], "weird new thing no list has");
}

#[test]
fn a_markets_statement_takes_any_names_or_none() {
    let fx = Fx::new();
    let o = fx.put(
        "markets",
        "id=\"M-1\"\nkind=\"markets\"\ntitle=\"Markets\"\nmarkets=[\"Atlantis\",\"Lemuria\"]\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.put(
        "markets",
        "id=\"M-2\"\nkind=\"markets\"\ntitle=\"None\"\nmarkets=[]\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn research_without_a_source_is_refused() {
    let fx = Fx::new();
    let mut v = finding(false);
    v["sources"] = json!([]);
    let f = file(&fx, "f.json", &v);
    let o = fx.refused(&["legal", "finding", &f]);
    assert!(o.stderr.contains("lists no source"), "{}", o.stderr);
    // Nothing was written.
    assert!(fx.must(&["legal", "list", "--json"]).json()["findings"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn a_finding_without_research_says_so_and_needs_no_source() {
    let fx = Fx::new();
    let v = json!({"title": "Guess", "markets": ["Mars"], "risk": "tax", "researched": false,
                   "lawyer_check": true});
    let f = file(&fx, "f.json", &v);
    fx.must(&["legal", "finding", &f]);
    let l = fx.must(&["legal", "list", "--json"]).json();
    assert_eq!(l["findings"][0]["researched"], false);
    // The flag is required, not assumed.
    let v = json!({"title": "Guess", "markets": ["Mars"], "risk": "tax", "lawyer_check": true});
    let f = file(&fx, "g.json", &v);
    let o = fx.refused(&["legal", "finding", &f]);
    assert!(o.stderr.contains("researched"), "{}", o.stderr);
}

#[test]
fn every_source_needs_its_five_parts() {
    let fx = Fx::new();
    for key in ["url", "title", "read_on", "grade", "why"] {
        let mut v = finding(false);
        v["sources"][0].as_object_mut().unwrap().remove(key);
        let f = file(&fx, "f.json", &v);
        let o = fx.refused(&["legal", "finding", &f]);
        assert!(o.stderr.contains(key), "{key}: {}", o.stderr);
    }
    let mut v = finding(false);
    v["sources"][0]["grade"] = json!("trust me");
    let f = file(&fx, "f.json", &v);
    fx.refused(&["legal", "finding", &f]);
}

#[test]
fn a_finding_must_name_its_markets_and_its_risk() {
    let fx = Fx::new();
    let mut v = finding(false);
    v["markets"] = json!([]);
    fx.refused(&["legal", "finding", &file(&fx, "a.json", &v)]);
    let mut v = finding(false);
    v["risk"] = json!("");
    fx.refused(&["legal", "finding", &file(&fx, "b.json", &v)]);
    let mut v = finding(false);
    v["outcome"] = json!("accepted");
    fx.refused(&["legal", "finding", &file(&fx, "c.json", &v)]);
}

fn ack(fx: &Fx, q: &str, key: &str) -> common::Out {
    let a = fx.root.join("ans.json");
    fs::write(&a, json!({"choice": key}).to_string()).unwrap();
    fx.bf(&["answer", q, "--file", a.to_str().unwrap()])
}

#[test]
fn a_marked_finding_holds_the_close_until_the_human_answers_after_their_own_message() {
    let fx = Fx::with_checks();
    fx.tiny();
    fx.name_every_check("R-1");
    fx.design_reviews_off();
    fx.start_test_run();
    fx.run_checks_all_off();
    let f = file(&fx, "f.json", &finding(true));
    fx.must(&["legal", "finding", &f]);

    // Not asked yet: the close names the finding.
    let o = fx.refused(&["run", "stop"]);
    assert!(
        o.stderr.contains("L-1") && o.stderr.contains("not acknowledged"),
        "{}",
        o.stderr
    );

    let q = fx.must(&["legal", "questions", "--json"]).json();
    let qid = q["asked"][0]["question"].as_str().unwrap().to_string();
    assert_eq!(q["asked"][0]["finding"], "L-1");
    let qrec = fx.must(&["record", "get", &qid, "--json"]).json();
    assert_eq!(qrec["stage"], "close");
    assert!(qrec["context"].as_str().unwrap().contains("https://example.org/law"));
    // Asking again does not ask twice.
    assert!(fx.must(&["legal", "questions", "--json"]).json()["asked"]
        .as_array()
        .unwrap()
        .is_empty());

    // No human message after the question: the agent cannot acknowledge in the human's place.
    let o = ack(&fx, &qid, "A");
    assert_eq!(o.code, 2, "{}", o.all());
    let o = fx.refused(&["run", "stop"]);
    assert!(o.stderr.contains("L-1"), "{}", o.stderr);

    fx.human("I read the finding.");
    // Answering "not yet" does not acknowledge.
    let o = ack(&fx, &qid, "B");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["run", "stop"]);
    assert!(o.stderr.contains("L-1"), "{}", o.stderr);

    // The finding is asked again and acknowledged after a new human message.
    let q2 = fx.must(&["legal", "questions", "--json"]).json();
    let qid2 = q2["asked"][0]["question"].as_str().unwrap().to_string();
    assert_ne!(qid, qid2);
    fx.human("Yes, I acknowledge it.");
    assert_eq!(ack(&fx, &qid2, "A").code, 0);
    assert!(bf::legal::unacknowledged(&fx.project()).unwrap().is_empty());
    fx.must(&["run", "stop"]);
}

#[test]
fn a_question_record_written_with_an_answer_is_not_an_acknowledgement() {
    let fx = Fx::new();
    fx.must(&["legal", "finding", &file(&fx, "f.json", &finding(true))]);
    let q = fx.must(&["legal", "questions", "--json"]).json();
    let qid = q["asked"][0]["question"].as_str().unwrap().to_string();
    // Forge the answer into the record without the answer command.
    let mut rec = fx.must(&["record", "get", &qid, "--json"]).json();
    rec["answer"] = json!({"choice": "A"});
    rec["answered_at"] = json!("2026-11-15T00:00:00Z");
    rec["answered_turn"] = json!(1);
    let f = file(&fx, "forged.json", &rec);
    let _ = fx.bf(&["record", "put", "question", "--file", &f, "--replace"]);
    let open = bf::legal::unacknowledged(&fx.project()).unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    // The forged answer leads to a fresh question the human can answer.
    let again = fx.must(&["legal", "questions", "--json"]).json();
    let qid2 = again["asked"][0]["question"].as_str().unwrap().to_string();
    assert_ne!(qid, qid2);
}

#[test]
fn a_marked_finding_cannot_be_unmarked_by_filing_it_again() {
    let fx = Fx::new();
    fx.must(&["legal", "finding", &file(&fx, "f.json", &finding(true))]);
    let mut v = finding(false);
    v["id"] = json!("L-1");
    fx.refused(&["legal", "finding", &file(&fx, "g.json", &v)]);
    let open = bf::legal::unacknowledged(&fx.project()).unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
}

#[test]
fn an_unmarked_finding_never_holds_the_close_and_a_changed_one_must_be_asked_again() {
    let fx = Fx::new();
    fx.must(&["legal", "finding", &file(&fx, "a.json", &finding(false))]);
    assert!(bf::legal::unacknowledged(&fx.project()).unwrap().is_empty());
    fx.must(&["legal", "finding", &file(&fx, "b.json", &finding(true))]);
    let q = fx.must(&["legal", "questions", "--json"]).json();
    let qid = q["asked"][0]["question"].as_str().unwrap().to_string();
    fx.human("Acknowledged.");
    assert_eq!(ack(&fx, &qid, "A").code, 0);
    assert!(bf::legal::unacknowledged(&fx.project()).unwrap().is_empty());
    // Put the second finding again with a change: the old acknowledgement no longer counts.
    let mut v = finding(true);
    v["id"] = json!("L-2");
    v["title"] = json!("Changed");
    fx.must(&["legal", "finding", &file(&fx, "c.json", &v)]);
    assert_eq!(bf::legal::unacknowledged(&fx.project()).unwrap().len(), 1);
}
