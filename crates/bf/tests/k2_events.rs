// D-91, H-21 to H-25: the events of the repair judge, a checkpoint fix's file requests, saved work and
// reopening, and the accept of findings on a parked review replay to the state the handoffs name. Each
// test hand-writes a log and replays it; no command runs.

use bf::events::{replay_log, Log, State};
use serde_json::{json, Value};

/// A log of the events given, numbered from 1; an event with no `kind` of its own key `text` is a human turn.
fn log(list: Vec<Value>) -> Log {
    let events = list
        .into_iter()
        .enumerate()
        .map(|(i, mut e)| {
            e["seq"] = json!(i as u64 + 1);
            e["at"] = json!("2026-11-19T10:00:00Z");
            e
        })
        .collect();
    Log {
        events,
        chain_ok: true,
        chain_error: None,
        head_error: None,
        last_hash: String::new(),
        valid_len: 0,
        file_len: 0,
    }
}

fn turn() -> Value {
    json!({"kind": "human.turn", "text": "Yes, I approve of that.", "transcript": "match"})
}

fn replay(list: Vec<Value>) -> State {
    replay_log(&log(list))
}

const QUOTE: &str = "I approve of that";

fn open_narrowed() -> Vec<Value> {
    vec![
        json!({"kind": "amend.open", "amend": "A-1", "ids": ["R-1"], "impact_ids": ["R-1"], "repair": true}),
        json!({"kind": "amend.narrow", "amend": "A-1", "keep": ["R-1"], "waive": [], "set_hash": "h1",
               "changes": [{"id": "R-1", "how": "changed"}, {"id": "U-2", "how": "changed"}]}),
    ]
}

fn amend<'a>(st: &'a State, id: &str) -> &'a bf::events::Amend {
    st.amends.iter().find(|a| a.id == id).unwrap()
}

#[test]
fn an_approval_by_the_judge_that_no_stub_counts_stays_narrowed_with_one_ignored_line() {
    let mut l = open_narrowed();
    l.push(json!({"kind": "amend.judge", "amend": "A-1", "verdict": "go", "reasons": "ok",
                  "model": "m", "fingerprint": "h1"}));
    l.push(json!({"kind": "amend.approve", "amend": "A-1", "by": "judge", "judge_seq": 3}));
    let st = replay(l);
    let a = amend(&st, "A-1");
    assert_eq!(a.status, "narrowed");
    assert_eq!(a.verdict.as_ref().unwrap()["verdict"], "go");
    assert!(!a.judged && !a.human_only);
    assert_eq!(st.ignored.len(), 1, "{:?}", st.ignored);
    assert!(st.judged_unseen.is_empty());
    assert!(!bf::judge::approve_counts(&st, &json!({"by": "judge"})));
}

#[test]
fn an_amendment_opened_from_a_design_is_human_only() {
    let st = replay(vec![
        json!({"kind": "amend.open", "amend": "A-1", "ids": ["R-1"], "repair": true, "from_design": "K-1"}),
        json!({"kind": "amend.open", "amend": "A-2", "ids": ["R-2"], "repair": true}),
    ]);
    assert!(amend(&st, "A-1").human_only && amend(&st, "A-1").from_design);
    assert!(!amend(&st, "A-2").human_only);
}

#[test]
fn a_failed_judge_makes_the_amendment_human_only_and_lists_its_records() {
    let mut l = open_narrowed();
    l.push(json!({"kind": "amend.judge.token", "amend": "A-1", "fingerprint": "h1"}));
    l.push(json!({"kind": "amend.judge.failed", "amend": "A-1", "fingerprint": "h1", "reason": "no verdict"}));
    let st = replay(l);
    let a = amend(&st, "A-1");
    assert!(a.human_only);
    assert_eq!(a.judge_turns.len(), 1);
    assert_eq!(a.judge_turns[0]["fingerprint"], "h1");
    assert_eq!(a.failed.as_ref().unwrap()["reason"], "no verdict");
    assert_eq!(st.asked.get("R-1").map(String::as_str), Some("A-1"));
    assert_eq!(st.asked.get("U-2").map(String::as_str), Some("A-1"));
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
}

#[test]
fn the_records_of_a_withdrawn_ask_stay_asked_until_it_is_seen() {
    let mut l = open_narrowed();
    l.push(json!({"kind": "amend.judge", "amend": "A-1", "verdict": "ask", "reasons": "unsure",
                  "model": "m", "fingerprint": "h1"}));
    l.push(json!({"kind": "amend.revise", "amend": "A-1", "ids": ["R-1"], "impact_ids": ["R-1"]}));
    l.push(json!({"kind": "amend.withdraw", "amend": "A-1", "reason": "smaller one"}));
    let st = replay(l.clone());
    let a = amend(&st, "A-1");
    assert_eq!(a.status, "withdrawn");
    assert!(a.human_only, "human_only holds after a revision");
    assert_eq!(st.asked.len(), 2, "{:?}", st.asked);

    // Seen with no valid human turn changes nothing.
    let mut bad = l.clone();
    bad.push(json!({"kind": "amend.judge.seen", "amend": "A-1", "turn": 99, "quote": QUOTE}));
    let st = replay(bad);
    assert_eq!(st.asked.len(), 2);
    assert!(!amend(&st, "A-1").seen);
    assert_eq!(st.ignored.len(), 1);

    // Seen after the verdict clears it.
    let mut good = l;
    good.push(turn());
    let t = good.len() as u64;
    good.push(json!({"kind": "amend.judge.seen", "amend": "A-1", "turn": t, "quote": QUOTE}));
    let st = replay(good);
    assert!(st.asked.is_empty(), "{:?}", st.asked);
    assert!(amend(&st, "A-1").seen);
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
}

#[test]
fn a_verdict_for_another_narrowing_changes_nothing() {
    let mut l = open_narrowed();
    l.push(json!({"kind": "amend.judge", "amend": "A-1", "verdict": "ask", "reasons": "r",
                  "model": "m", "fingerprint": "old"}));
    let st = replay(l);
    assert!(amend(&st, "A-1").verdict.is_none());
    assert!(!amend(&st, "A-1").human_only);
    assert!(st.asked.is_empty());
    assert_eq!(st.ignored.len(), 1);
}

fn fix_open() -> Value {
    json!({"kind": "checkpoint.fix.open", "checkpoint": "K-1", "source": "check", "content": "c1",
           "failed": [{"check": "X-1"}], "allowed": ["src/a.txt"], "units": ["U-1"]})
}

#[test]
fn a_fix_asks_for_files_is_granted_or_refused_and_goes_back_to_work() {
    let st = replay(vec![
        fix_open(),
        json!({"kind": "checkpoint.fix.scope.request", "checkpoint": "K-1",
               "files": [{"path": "tests/b.rs", "why": "needs a test", "patch": "+x"}]}),
    ]);
    let f = st.ck_fix.as_ref().unwrap();
    assert_eq!(f.phase, "scope");
    assert_eq!(f.request.len(), 1);

    let st = replay(vec![
        fix_open(),
        json!({"kind": "checkpoint.fix.scope.request", "checkpoint": "K-1", "files": [{"path": "tests/b.rs", "why": "w", "patch": "p"}]}),
        json!({"kind": "checkpoint.fix.scope.grant", "checkpoint": "K-1", "files": [{"path": "tests/b.rs", "why": "w", "patch": "p"}]}),
        json!({"kind": "checkpoint.fix.scope.request", "checkpoint": "K-1", "files": [{"path": "src/z.rs", "why": "v", "patch": "q"}]}),
        json!({"kind": "checkpoint.fix.scope.refuse", "checkpoint": "K-1", "refusal": "hard", "files": ["src/z.rs"]}),
    ]);
    let f = st.ck_fix.as_ref().unwrap();
    assert_eq!(f.phase, "fix");
    assert_eq!(f.grants.len(), 1);
    assert_eq!(f.grants[0].files, vec!["tests/b.rs"]);
    assert_eq!(f.grants[0].reasons, vec!["w"]);
    assert_eq!(f.grants[0].patches, vec!["p"]);
    assert_eq!(f.grants[0].seq, 3);
    assert_eq!(f.refused.len(), 1);
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
    assert!(bf::fixscope::granted(&st, "K-1").is_empty());

    // A grant with no request is ignored.
    let st = replay(vec![
        fix_open(),
        json!({"kind": "checkpoint.fix.scope.grant", "checkpoint": "K-1", "files": [{"path": "tests/b.rs"}]}),
    ]);
    assert!(st.ck_fix.as_ref().unwrap().grants.is_empty());
    assert_eq!(st.ignored.len(), 1);
}

#[test]
fn an_abort_of_a_fix_for_a_failed_check_keeps_the_saved_work_and_declines_nothing() {
    let st = replay(vec![
        fix_open(),
        json!({"kind": "checkpoint.fix.abort", "checkpoint": "K-1", "saved": "evidence/saved/K-1-1"}),
        json!({"kind": "checkpoint.fix.open", "checkpoint": "K-1", "source": "check", "content": "c1",
               "failed": [{"check": "X-1"}], "from_saved": "evidence/saved/K-1-1"}),
    ]);
    assert!(st.ck_declined.is_empty(), "{:?}", st.ck_declined);
    assert_eq!(st.ck_fix_aborts.len(), 1);
    assert_eq!(st.ck_fix_aborts[0].saved, "evidence/saved/K-1-1");
    assert_eq!(st.ck_fix.as_ref().unwrap().from_saved, "evidence/saved/K-1-1");
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
}

fn fix_with_low_findings() -> Vec<Value> {
    vec![
        fix_open(),
        json!({"kind": "checkpoint.fix.built", "checkpoint": "K-1"}),
        json!({"kind": "checkpoint.fix.reviewed", "checkpoint": "K-1", "round": 1, "result": "findings",
               "to": "low", "content": "c2", "heads": [], "covered": "all",
               "findings": [{"title": "a", "severity": "low"}, {"title": "b", "severity": "low"}]}),
    ]
}

#[test]
fn the_accept_of_a_fix_reviews_findings_counts_only_with_a_valid_turn_and_settles_it_clean() {
    let mut l = fix_with_low_findings();
    assert_eq!(replay(l.clone()).ck_fix.unwrap().phase, "low");

    // No turn at all: nothing changes.
    let mut none = l.clone();
    none.push(json!({"kind": "checkpoint.fix.finding.settle", "checkpoint": "K-1", "fate": "accepted",
                     "ids": [{"round": 1, "index": 1}], "turn": 77, "quote": QUOTE}));
    let st = replay(none);
    assert!(st.accepted.is_empty());
    assert_eq!(st.ck_fix.as_ref().unwrap().findings.len(), 2);
    assert_eq!(st.ignored.len(), 1);

    // A turn written before the review does not count either.
    let mut early = vec![turn()];
    early.extend(l.clone());
    early.push(json!({"kind": "checkpoint.fix.finding.settle", "checkpoint": "K-1", "fate": "accepted",
                      "ids": [{"round": 1, "index": 1}], "turn": 1, "quote": QUOTE}));
    let st = replay(early);
    assert!(st.accepted.is_empty());
    assert_eq!(st.ignored.len(), 1);

    // Valid turns: one finding, then the other.
    l.push(turn());
    let t1 = l.len() as u64;
    l.push(json!({"kind": "checkpoint.fix.finding.settle", "checkpoint": "K-1", "fate": "accepted",
                  "ids": [{"round": 1, "index": 1}], "turn": t1, "quote": QUOTE}));
    let st = replay(l.clone());
    let f = st.ck_fix.as_ref().unwrap();
    assert_eq!(f.findings.len(), 1);
    assert_eq!(f.phase, "low");
    assert_eq!(f.accepted.len(), 1);
    assert_eq!(f.accepted[0].level, "low");
    assert_eq!(f.accepted[0].quote, QUOTE);
    assert_eq!(f.accepted[0].turn, t1);
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);

    l.push(turn());
    let t2 = l.len() as u64;
    l.push(json!({"kind": "checkpoint.fix.finding.settle", "checkpoint": "K-1", "fate": "accepted",
                  "ids": [{"round": 1, "index": 2}], "turn": t2, "quote": QUOTE}));
    let st = replay(l);
    let f = st.ck_fix.as_ref().unwrap();
    assert!(f.findings.is_empty());
    assert_eq!(f.phase, "review");
    assert_eq!(f.rounds_log.last().unwrap()["result"], "clean");
    assert_eq!(st.accepted.len(), 2);
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
}

fn parked_unit() -> Vec<Value> {
    vec![
        json!({"kind": "unit.start", "unit": "U-1"}),
        json!({"kind": "unit.report", "unit": "U-1", "result": "findings", "from": "review", "to": "parked",
               "findings": ["F-1", "F-2"], "round": 1, "content": "c", "head": "h", "covered": "all"}),
        json!({"kind": "unit.park", "unit": "U-1", "reason": "more than 3 review rounds"}),
    ]
}

#[test]
fn the_accept_of_a_units_findings_counts_only_with_a_valid_turn_and_unparks_a_cleared_unit() {
    let l = parked_unit();
    assert_eq!(replay(l.clone()).units["U-1"].open_findings, vec!["F-1", "F-2"]);

    // With no valid turn nothing changes.
    let mut none = l.clone();
    none.push(json!({"kind": "unit.finding.accept", "unit": "U-1", "findings": ["F-1", "F-2"],
                     "turn": 50, "quote": QUOTE}));
    let st = replay(none);
    assert_eq!(st.units["U-1"].state, "parked");
    assert_eq!(st.units["U-1"].open_findings.len(), 2);
    assert!(st.accepted.is_empty());
    assert_eq!(st.ignored.len(), 1);

    // A turn written before the review does not count.
    let mut early = vec![turn()];
    early.extend(l.clone());
    early.push(json!({"kind": "unit.finding.accept", "unit": "U-1", "findings": ["F-1", "F-2"],
                      "turn": 1, "quote": QUOTE}));
    let st = replay(early);
    assert_eq!(st.units["U-1"].state, "parked");
    assert_eq!(st.ignored.len(), 1);

    // One accepted: still parked. Both: reviewed.
    let mut one = l.clone();
    one.push(turn());
    let t1 = one.len() as u64;
    one.push(json!({"kind": "unit.finding.accept", "unit": "U-1", "findings": [{"id": "F-1", "severity": "high"}],
                    "turn": t1, "quote": QUOTE}));
    let st = replay(one.clone());
    assert_eq!(st.units["U-1"].state, "parked");
    assert_eq!(st.units["U-1"].open_findings, vec!["F-2"]);
    assert_eq!(st.accepted[0].level, "high");
    assert_eq!(st.accepted[0].subject, "U-1");

    one.push(turn());
    let t2 = one.len() as u64;
    one.push(json!({"kind": "unit.finding.accept", "unit": "U-1", "findings": ["F-2"],
                    "levels": {"F-2": "low"}, "turn": t2, "quote": QUOTE}));
    let st = replay(one);
    assert_eq!(st.units["U-1"].state, "reviewed");
    assert!(st.units["U-1"].open_findings.is_empty());
    assert_eq!(st.accepted.len(), 2);
    assert_eq!(st.accepted[1].level, "low");
    assert_eq!(st.accepted[1].turn, t2);
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
}

#[test]
fn an_accept_of_an_aborted_fix_never_settles_a_finding_of_the_reopened_fix() {
    let two = |round: u64| {
        json!({"kind": "checkpoint.fix.reviewed", "checkpoint": "K-1", "round": round, "result": "findings",
               "to": "low", "content": "c2", "heads": [], "covered": "all",
               "findings": [{"title": "x", "severity": "low"}, {"title": "y", "severity": "low"}]})
    };
    let mut l = vec![
        fix_open(),
        json!({"kind": "checkpoint.fix.built", "checkpoint": "K-1"}),
        two(1),
        turn(),
    ];
    let t1 = l.len() as u64;
    l.push(json!({"kind": "checkpoint.fix.finding.settle", "checkpoint": "K-1", "fate": "accepted",
                  "ids": [{"round": 1, "index": 1}], "turn": t1, "quote": QUOTE}));
    l.push(json!({"kind": "checkpoint.fix.abort", "checkpoint": "K-1", "saved": "evidence/saved/K-1-1"}));
    l.push(json!({"kind": "checkpoint.fix.open", "checkpoint": "K-1", "source": "check", "content": "c1",
                  "failed": [{"check": "X-1"}], "from_saved": "evidence/saved/K-1-1"}));
    l.push(json!({"kind": "checkpoint.fix.built", "checkpoint": "K-1"}));
    l.push(two(1));
    l.push(turn());
    let t2 = l.len() as u64;
    l.push(json!({"kind": "checkpoint.fix.finding.settle", "checkpoint": "K-1", "fate": "accepted",
                  "ids": [{"round": 1, "index": 2}], "turn": t2, "quote": QUOTE}));
    let st = replay(l);
    let f = st.ck_fix.as_ref().unwrap();
    assert_eq!(f.phase, "low", "{:?}", st.ignored);
    assert_eq!(f.findings.len(), 1);
    assert_eq!(f.findings[0]["title"], "x");
    assert!(f.rounds_log.iter().all(|r| r["result"] != "clean"));
    assert_eq!(f.accepted.len(), 1);
    assert_eq!(f.accepted[0].index, 2);
    assert_eq!(f.inherited.len(), 1);
}

#[test]
fn an_accept_naming_a_finding_the_unit_does_not_have_is_ignored_and_a_partial_one_does_not_unpark() {
    let mut l = parked_unit();
    l.push(turn());
    let t = l.len() as u64;
    l.push(json!({"kind": "unit.finding.accept", "unit": "U-1", "findings": ["F-1", "F-9"],
                  "turn": t, "quote": QUOTE}));
    let st = replay(l.clone());
    assert_eq!(st.units["U-1"].state, "parked");
    assert_eq!(st.units["U-1"].open_findings.len(), 2);
    assert!(st.accepted.is_empty());
    assert_eq!(st.ignored.len(), 1);
    assert!(st.ignored[0].contains("does not have"), "{:?}", st.ignored);
    // Only part of the report's list: still parked.
    let mut p = parked_unit();
    p.push(turn());
    let t = p.len() as u64;
    p.push(json!({"kind": "unit.finding.accept", "unit": "U-1", "findings": ["F-1"],
                  "turn": t, "quote": QUOTE}));
    assert_eq!(replay(p).units["U-1"].state, "parked");
}

#[test]
fn a_verdict_on_a_human_only_amendment_is_ignored_after_a_failed_judge_or_an_ask() {
    for first in [
        json!({"kind": "amend.judge.failed", "amend": "A-1", "fingerprint": "h1", "reason": "no verdict"}),
        json!({"kind": "amend.judge", "amend": "A-1", "fingerprint": "h1", "verdict": "ask", "reasons": []}),
    ] {
        let mut l = open_narrowed();
        l.push(first);
        l.push(json!({"kind": "amend.judge", "amend": "A-1", "fingerprint": "h1", "verdict": "go", "reasons": []}));
        let st = replay(l);
        let a = amend(&st, "A-1");
        assert!(a.human_only);
        assert_ne!(a.verdict.as_ref().map(|v| v["verdict"].clone()), Some(json!("go")));
        assert_eq!(st.ignored.len(), 1, "{:?}", st.ignored);
    }
}

#[test]
fn a_refuse_event_keeps_every_field_of_its_file_entries() {
    let entry = json!({"path": "src/z.rs", "why": "v", "patch": "q", "verdict": "refuse", "round": 2, "must": true});
    let st = replay(vec![
        fix_open(),
        json!({"kind": "checkpoint.fix.scope.request", "checkpoint": "K-1", "files": [{"path": "src/z.rs", "why": "v", "patch": "q"}]}),
        json!({"kind": "checkpoint.fix.scope.refuse", "checkpoint": "K-1", "refusal": "hard", "files": [entry.clone()]}),
    ]);
    let f = st.ck_fix.as_ref().unwrap();
    assert_eq!(f.refused.len(), 1);
    assert_eq!(f.refused[0]["files"][0], entry);
}
