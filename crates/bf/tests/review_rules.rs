//! U-73: the review rules (R-97, R-98, D-72): rounds and the park, the holds of a gate and of an
//! amendment in the shape it was designed, the human's answer and the page data. The tests call the
//! module's functions the way the hooks do: `on_attach`, then the receipt, then the events.

mod common;
use bf::checks::review::{self, Attach};
use bf::events;
use bf::refusal::R;
use common::Fx;
use serde_json::{json, Value};
use std::fs;

const CHECKS: [(&str, &str); 5] = [
    ("X-11", "idea-review"),
    ("X-12", "requirements-review"),
    ("X-13", "architecture-review"),
    ("X-14", "plan-review"),
    ("X-15", "amendment-review"),
];

fn put_ok(fx: &Fx, kind: &str, text: &str) {
    let o = fx.put(kind, text);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn put_replace(fx: &Fx, kind: &str, text: &str) {
    let f = fx.root.join("replace.toml");
    fs::write(&f, text).unwrap();
    let o = fx.bf(&["record", "put", kind, "--file", f.to_str().unwrap(), "--replace"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let _ = fs::remove_file(&f);
}

/// A project with the defaults (every review check on), the tiny records and a check record for
/// each review check.
fn setup() -> Fx {
    let fx = Fx::with_checks();
    fx.tiny();
    for (id, name) in CHECKS {
        put_ok(
            &fx,
            "check",
            &format!(
                "id=\"{id}\"\nkind=\"check\"\ntitle=\"{name}\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"{name}\"\n"
            ),
        );
    }
    fx
}

fn check_id(gate_or_amend: &str) -> &'static str {
    let name = if gate_or_amend.starts_with("A-") {
        "amendment-review".to_string()
    } else {
        format!("{gate_or_amend}-review")
    };
    CHECKS.iter().find(|c| c.1 == name).unwrap().0
}

/// What the hooks do for an outcome on a review check: the rules, the sealed receipt with the
/// fields, the evidence event and, when the rules say so, the park.
fn attach(
    fx: &Fx,
    check: &str,
    outcome: &str,
    token: Option<&str>,
    amendment: Option<&str>,
    kind: Option<&str>,
) -> R<Option<Attach>> {
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    let rec = p.get_record(check).unwrap().unwrap();
    let a = review::on_attach(&p, &st, &rec, outcome, token, amendment, kind)?;
    let a = a.expect("a design review check");
    let ty = match outcome {
        "met" => "attach",
        "failed" => "fail",
        _ => "unreachable",
    };
    let mut v = json!({
        "type": ty, "check": check, "reason": "I read every record of the stage",
        "file_hash": "ab12", "file_name": "review.md", "content_hash": "c0",
        "at": "2026-01-01T00:00:00Z", "check_hash": bf::evidence::record_hash(&rec),
    });
    for (k, x) in a.fields().as_object().unwrap() {
        v[k] = x.clone();
    }
    let n = fs::read_dir(p.receipts_dir()).map(|d| d.count()).unwrap_or(0) + 1;
    let id = format!("rc-{n:05}");
    v["id"] = json!(id);
    let key = bf::evidence::read_key(&p).unwrap();
    v["seal"] = json!(bf::evidence::seal(&key, &v));
    fs::create_dir_all(p.receipts_dir()).unwrap();
    fs::write(
        p.receipts_dir().join(format!("{id}.toml")),
        toml::to_string_pretty(&bf::store::strip_nulls(v)).unwrap(),
    )
    .unwrap();
    events::append(&p, "evidence.attach", json!({"receipt": id, "check": check})).unwrap();
    if a.park {
        events::append(&p, review::PARK_EVENT, json!({"subject": a.subject, "round": a.round}))
            .unwrap();
    }
    Ok(Some(a))
}

fn gate_review(fx: &Fx, gate: &str, outcome: &str) -> Attach {
    attach(fx, check_id(gate), outcome, None, None, None).unwrap().unwrap()
}

fn amend_review(fx: &Fx, a: &str, kind: &str, outcome: &str) -> Attach {
    attach(fx, "X-15", outcome, None, Some(a), Some(kind)).unwrap().unwrap()
}

fn gate_lines(fx: &Fx, gate: &str) -> Vec<String> {
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    review::gate_blocker(&p, &st, gate).unwrap()
}

fn amend_lines(fx: &Fx, a: &str) -> Vec<String> {
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    review::amendment_blocker(&p, &st, a).unwrap()
}

fn finding(id: &str, gate_or_amend: &str, sev: &str, extra: &str) -> String {
    let which = if gate_or_amend.starts_with("A-") {
        format!("amendment=\"{gate_or_amend}\"")
    } else {
        format!("gate=\"{gate_or_amend}\"")
    };
    format!("id=\"{id}\"\nkind=\"finding\"\ntitle=\"Title of {id}\"\n{which}\nseverity=\"{sev}\"\naction=\"Change it\"\n{extra}")
}

#[test]
fn each_gate_is_held_without_a_review_and_after_a_record_changes() {
    let fx = setup();
    fs::write(fx.root.join("factory/idea.md"), "A small greeting tool.\n").unwrap();
    #[allow(clippy::type_complexity)]
    let changes: [(&str, Box<dyn Fn(&Fx, u32)>); 4] = [
        (
            "idea",
            Box::new(|fx, n| {
                fs::write(fx.root.join("factory/idea.md"), format!("A greeting tool, take {n}.\n"))
                    .unwrap()
            }),
        ),
        (
            "requirements",
            Box::new(|fx, n| {
                put_replace(fx, "requirement", &format!("id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists {n}\"\nacceptance=[\"src/hello.txt exists\"]\n"))
            }),
        ),
        (
            "architecture",
            Box::new(|fx, n| {
                put_replace(fx, "component", &format!("id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages {n}\"\n"))
            }),
        ),
        (
            "plan",
            Box::new(|fx, n| {
                put_replace(fx, "check", &format!("id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files {n}\"\nhow=\"command\"\ncommand=\"ls src/hello.txt src/bye.txt\"\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nscope=\"checkpoint\"\n"))
            }),
        ),
    ];
    for (gate, change) in &changes {
        let held = gate_lines(&fx, gate);
        assert!(held.iter().any(|l| l.contains("no review")), "{gate}: {held:?}");
        gate_review(&fx, gate, "met");
        assert_eq!(gate_lines(&fx, gate), Vec::<String>::new(), "{gate}");
        change(&fx, 2);
        let held = gate_lines(&fx, gate);
        assert!(held.iter().any(|l| l.contains("no review")), "{gate} after a change: {held:?}");
        // A failed review at the current records also counts as a review.
        gate_review(&fx, gate, "failed");
        assert_eq!(gate_lines(&fx, gate), Vec::<String>::new(), "{gate}");
    }
    // A record of another stage does not need the review again.
    put_replace(&fx, "component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages again\"\n");
    assert_eq!(gate_lines(&fx, "requirements"), Vec::<String>::new());
}

#[test]
fn a_gate_without_a_review_check_or_with_it_off_is_not_held() {
    let fx = setup();
    assert!(gate_lines(&fx, "ui").is_empty());
    assert!(gate_lines(&fx, "build").is_empty());
    fx.must(&["checks", "propose", "plan-review=off:the plan is settled", "--scope", "default"]);
    fx.human("Yes, switch the plan review off.");
    fx.must(&["checks", "set", "--quote", "switch the plan review off"]);
    assert!(gate_lines(&fx, "plan").is_empty());
    assert!(!gate_lines(&fx, "idea").is_empty());
}

#[test]
fn an_open_high_finding_holds_and_refuses_met_and_a_low_one_does_not() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "requirements", "high", ""));
    put_ok(&fx, "finding", &finding("F-2", "requirements", "low", ""));
    let refused = attach(&fx, "X-12", "met", None, None, None).expect_err("met refused");
    assert!(format!("{refused:?}").contains("F-1") || refused.to_string().contains("F-1"));
    gate_review(&fx, "requirements", "failed");
    let held = gate_lines(&fx, "requirements");
    assert!(held.iter().any(|l| l.contains("F-1")), "{held:?}");
    assert!(!held.iter().any(|l| l.contains("F-2")), "{held:?}");
    // A medium one holds too; fixing the high one leaves the medium one.
    put_ok(&fx, "finding", &finding("F-3", "requirements", "medium", ""));
    put_replace(&fx, "finding", &finding("F-1", "requirements", "high", "outcome=\"fixed\"\n"));
    let held = gate_lines(&fx, "requirements");
    assert!(held.len() == 1 && held[0].contains("F-3"), "{held:?}");
    put_replace(&fx, "finding", &finding("F-3", "requirements", "medium", "outcome=\"fixed\"\n"));
    gate_review(&fx, "requirements", "met");
    assert!(gate_lines(&fx, "requirements").is_empty());
}

#[test]
fn a_recorded_accepted_without_the_humans_words_does_not_release() {
    let fx = setup();
    gate_review(&fx, "idea", "met");
    put_ok(&fx, "finding", &finding("F-1", "idea", "high", "outcome=\"accepted\"\n"));
    assert!(gate_lines(&fx, "idea").iter().any(|l| l.contains("F-1")));
}

#[test]
fn three_rounds_park_and_only_later_words_release_with_a_fresh_set_of_rounds() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "requirements", "high", ""));
    fx.human("Please look at the requirements again before the park.");
    for want in 1..=3u32 {
        let a = gate_review(&fx, "requirements", "failed");
        assert_eq!(a.round, want);
        assert_eq!(a.park, want == 3, "round {want}");
    }
    let p = fx.project();
    assert!(review::parked(&p, "requirements").unwrap());
    println!("parked after {} rounds", review::rounds(&p, "requirements").unwrap().1);
    let e = attach(&fx, "X-12", "failed", None, None, None).expect_err("parked");
    assert!(e.to_string().contains("bf review release"), "{e}");
    assert!(gate_lines(&fx, "requirements").iter().any(|l| l.contains("parked")));

    // Words written before the park cannot release it.
    let before = "Please look at the requirements again before the park.";
    assert!(review::check_release(&p, "requirements", before).is_err());
    fx.human("I read the findings and the review may start over.");
    let turn = review::check_release(&p, "requirements", "the review may start over").unwrap();
    events::append(
        &p,
        review::RELEASE_EVENT,
        json!({"subject": "requirements", "turn": turn, "quote": "the review may start over"}),
    )
    .unwrap();
    assert!(!review::parked(&p, "requirements").unwrap());
    assert_eq!(review::rounds(&p, "requirements").unwrap().1, 0);
    // The same words cannot be taken twice.
    put_ok(&fx, "finding", &finding("F-9", "requirements", "medium", ""));
    let again = review::check_accept(&p, &["F-9".to_string()], "the review may start over");
    assert!(again.is_err(), "words are taken once");
    // A fresh set of three rounds.
    for want in 1..=3u32 {
        let a = gate_review(&fx, "requirements", "failed");
        assert_eq!(a.round, want);
        assert_eq!(a.park, want == 3);
    }
    println!("released and parked again after {} fresh rounds", review::rounds(&fx.project(), "requirements").unwrap().1);
    // Nothing to release once it is released: a release of a review not parked is refused.
    assert!(review::check_release(&fx.project(), "idea", "the review may start over").is_err());
}

#[test]
fn a_review_that_ends_clean_at_the_third_round_does_not_park() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "plan", "high", ""));
    gate_review(&fx, "plan", "failed");
    gate_review(&fx, "plan", "failed");
    put_replace(&fx, "finding", &finding("F-1", "plan", "high", "outcome=\"fixed\"\n"));
    let a = gate_review(&fx, "plan", "met");
    assert!(!a.park);
}

#[test]
fn the_humans_accept_needs_words_after_the_review() {
    let fx = setup();
    fx.human("The idea is fine as it is, I accept whatever it finds.");
    put_ok(&fx, "finding", &finding("F-1", "idea", "high", ""));
    gate_review(&fx, "idea", "failed");
    let p = fx.project();
    let ids = vec!["F-1".to_string()];
    assert!(review::check_accept(&p, &ids, "I accept whatever it finds").is_err(), "written before the review");
    fx.human("I accept the point about the scope.");
    let turn = review::check_accept(&p, &ids, "I accept the point about the scope").unwrap();
    events::append(
        &p,
        review::ACCEPT_EVENT,
        json!({"subject": "idea", "ids": ["F-1"], "turn": turn, "quote": "I accept the point about the scope"}),
    )
    .unwrap();
    put_replace(&fx, "finding", &finding("F-1", "idea", "high", "outcome=\"accepted\"\n"));
    assert!(gate_lines(&fx, "idea").is_empty());
    let page = review::page_section(&p, &events::replay(&p).unwrap(), "idea").unwrap();
    assert_eq!(page["findings"]["holding"][0]["outcome"], "accepted");
    assert_eq!(page["findings"]["holding"][0]["words"], "I accept the point about the scope");
    assert!(review::check_accept(&p, &ids, "I accept the point about the scope").is_err(), "not open any more");
}

fn open_amendment(fx: &Fx) -> String {
    let o = fx.must(&["amend", "open", "R-2", "--json"]).json();
    o["id"].as_str().or(o["amend"].as_str()).unwrap_or("A-1").to_string()
}

#[test]
fn a_solo_review_with_a_request_counts_one_round_with_its_stage_review() {
    let fx = setup();
    let a = open_amendment(&fx);
    let held = amend_lines(&fx, &a);
    assert!(held.iter().any(|l| l.contains("kind amendment")), "{held:?}");
    // The solo reviewer asks for a review of the requirements stage.
    put_ok(&fx, "finding", &finding("F-1", &a, "medium", "asks=\"requirements\"\n"));
    amend_review(&fx, &a, "amendment", "failed");
    let held = amend_lines(&fx, &a);
    assert!(held.iter().any(|l| l.contains("requirements stage")), "{held:?}");
    let s = amend_review(&fx, &a, "requirements", "met");
    assert_eq!(s.round, 1);
    let p = fx.project();
    let (rs, done) = review::rounds(&p, &a).unwrap();
    assert_eq!((done, rs.len(), rs[0].reviews.len()), (1, 1, 2));
    assert!(amend_lines(&fx, &a).is_empty(), "{:?}", amend_lines(&fx, &a));
    // A change of the amendment's records needs the reviews again.
    put_replace(&fx, "requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"A farewell exists, changed\"\nacceptance=[\"src/bye.txt exists\"]\n");
    assert!(!amend_lines(&fx, &a).is_empty());
    // A stage reviewer's open finding holds the approval until fixed.
    amend_review(&fx, &a, "amendment", "failed");
    put_ok(&fx, "finding", &finding("F-2", &a, "high", "gate=\"requirements\"\n"));
    let held = amend_lines(&fx, &a);
    assert!(held.iter().any(|l| l.contains("F-2")), "{held:?}");
}

#[test]
fn an_ask_in_the_last_round_parks_first_and_its_stage_review_follows_the_release() {
    let fx = setup();
    let a = open_amendment(&fx);
    put_ok(&fx, "finding", &finding("F-1", &a, "medium", "asks=\"requirements\"\n"));
    for want in 1..=3u32 {
        let r = amend_review(&fx, &a, "amendment", "failed");
        assert_eq!((r.round, r.park), (want, want == 3));
    }
    let p = fx.project();
    assert!(review::parked(&p, &a).unwrap());
    let e = attach(&fx, "X-15", "met", None, Some(&a), Some("requirements")).expect_err("parked");
    assert!(e.to_string().contains("bf review release"), "{e}");
    fx.human("I read the amendment findings and the review may go on.");
    let turn = review::check_release(&p, &a, "the review may go on").unwrap();
    events::append(
        &p,
        review::RELEASE_EVENT,
        json!({"subject": a, "turn": turn, "quote": "the review may go on"}),
    )
    .unwrap();
    attach(&fx, "X-15", "met", None, Some(&a), Some("requirements")).unwrap();
    // The stage review is no round of the fresh set.
    let (rs, done) = review::rounds(&fx.project(), &a).unwrap();
    assert_eq!((done, rs.len()), (0, 0));
    // It is still shown on the page, and the hold no longer names the asked stage.
    let page = review::page_section(&fx.project(), &events::replay(&fx.project()).unwrap(), &a).unwrap();
    assert_eq!(page["answering_parked"].as_array().unwrap().len(), 1, "{page}");
    let held = amend_lines(&fx, &a);
    assert!(!held.iter().any(|l| l.contains("requirements stage")), "{held:?}");
}

#[test]
fn a_split_needs_each_stage_review_and_then_a_newer_amendment_review() {
    let fx = setup();
    put_ok(&fx, "component", "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n");
    let o = fx.must(&["amend", "open", "R-2", "C-9", "--json"]).json();
    let a = o["id"].as_str().or(o["amend"].as_str()).unwrap_or("A-1").to_string();
    let p = fx.project();
    events::append(&p, review::SPLIT_EVENT, json!({"amendment": a, "stage": "requirements"})).unwrap();
    events::append(&p, review::SPLIT_EVENT, json!({"amendment": a, "stage": "architecture"})).unwrap();
    let held = amend_lines(&fx, &a);
    assert!(held.iter().any(|l| l.contains("requirements stage")), "{held:?}");
    assert!(held.iter().any(|l| l.contains("architecture stage")), "{held:?}");
    // The agreement review first, then the stages: it is older than a stage review.
    amend_review(&fx, &a, "amendment", "met");
    amend_review(&fx, &a, "requirements", "met");
    amend_review(&fx, &a, "architecture", "met");
    let held = amend_lines(&fx, &a);
    assert!(held.iter().any(|l| l.contains("older")), "{held:?}");
    let last = amend_review(&fx, &a, "amendment", "met");
    assert_eq!(last.round, 2);
    assert!(amend_lines(&fx, &a).is_empty(), "{:?}", amend_lines(&fx, &a));
    let (rs, done) = review::rounds(&p, &a).unwrap();
    assert_eq!(done, 2);
    assert_eq!(rs[1].reviews.len(), 3, "the stage reviews belong to the round they lead up to");
}

#[test]
fn a_plan_repair_needs_no_amendment_review() {
    let fx = setup();
    let o = fx.must(&["amend", "open", "--repair", "X-3", "--json"]).json();
    let a = o["id"].as_str().or(o["amend"].as_str()).unwrap_or("A-1").to_string();
    assert!(amend_lines(&fx, &a).is_empty());
}

#[test]
fn the_amendment_hold_is_lifted_when_the_check_is_off() {
    let fx = setup();
    let a = open_amendment(&fx);
    assert!(!amend_lines(&fx, &a).is_empty());
    fx.must(&["checks", "propose", "amendment-review=off:reviewed by the human", "--scope", "default"]);
    fx.human("Yes, switch the amendment review off.");
    fx.must(&["checks", "set", "--quote", "switch the amendment review off"]);
    assert!(amend_lines(&fx, &a).is_empty());
}

#[test]
fn an_outcome_is_taken_only_with_the_reviewers_token_of_that_kind() {
    let fx = setup();
    fx.set_policy("tokens = false", "tokens = true");
    let p = fx.project();
    let a = open_amendment(&fx);
    let t_idea = bf::run::issue_token(&p, "design:design-reviewer:idea").unwrap();
    let t_req = bf::run::issue_token(&p, "design:design-reviewer:requirements").unwrap();
    // No token, the token of another kind, a wrong check.
    assert!(attach(&fx, "X-11", "met", None, None, None).is_err());
    assert!(attach(&fx, "X-11", "met", Some(&t_req), None, None).is_err());
    assert!(attach(&fx, "X-12", "met", Some(&t_idea), None, None).is_err());
    let ok = attach(&fx, "X-11", "met", Some(&t_idea), None, None).unwrap().unwrap();
    assert_eq!((ok.kind.as_str(), ok.subject.as_str(), ok.amendment.clone()), ("idea", "idea", None));
    assert_eq!(ok.records_hash, review::hash_for(&p, "idea", None).unwrap());
    // An amendment: the design reviewer for its kinds, the UI/UX verifier for ui.
    let t_am = bf::run::issue_token(&p, &format!("{a}:design-reviewer:amendment")).unwrap();
    let t_ui = bf::run::issue_token(&p, &format!("{a}:uiux-verifier:ui")).unwrap();
    let r = attach(&fx, "X-15", "met", Some(&t_am), None, None).unwrap().unwrap();
    assert_eq!(r.amendment.as_deref(), Some(a.as_str()));
    assert_eq!(r.kind, "amendment");
    let u = attach(&fx, "X-15", "met", Some(&t_ui), None, None).unwrap().unwrap();
    assert_eq!(u.kind, "ui");
    assert!(attach(&fx, "X-15", "met", Some(&t_idea), None, None).is_err(), "a gate token on an amendment check");
}

#[test]
fn design_findings_are_put_only_with_the_token_of_their_kind() {
    let fx = setup();
    fx.set_policy("tokens = false", "tokens = true");
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    let a = open_amendment(&fx);
    let st2 = events::replay(&p).unwrap();
    let _ = st;
    let rec = |t: &str| -> Value { toml::from_str::<Value>(t).unwrap() };
    let t_req = bf::run::issue_token(&p, "design:design-reviewer:requirements").unwrap();
    let f = rec(&finding("F-1", "requirements", "high", ""));
    assert!(review::may_put_finding(&p, &st2, &f, None, None).is_err());
    assert!(review::may_put_finding(&p, &st2, &f, None, Some("nope")).is_err());
    review::may_put_finding(&p, &st2, &f, None, Some(&t_req)).unwrap();
    // Never accepted by a put.
    let acc = rec(&finding("F-1", "requirements", "high", "outcome=\"accepted\"\n"));
    assert!(review::may_put_finding(&p, &st2, &acc, None, Some(&t_req)).is_err());
    // A request for a stage review: only from the solo amendment reviewer, once per stage.
    let t_am = bf::run::issue_token(&p, &format!("{a}:design-reviewer:amendment")).unwrap();
    let ask = rec(&finding("F-2", &a, "medium", "asks=\"plan\"\n"));
    assert!(review::may_put_finding(&p, &st2, &ask, None, Some(&t_req)).is_err());
    review::may_put_finding(&p, &st2, &ask, None, Some(&t_am)).unwrap();
    let gate_ask = rec(&finding("F-3", "plan", "medium", "asks=\"plan\"\n"));
    let t_plan = bf::run::issue_token(&p, "design:design-reviewer:plan").unwrap();
    assert!(review::may_put_finding(&p, &st2, &gate_ask, None, Some(&t_plan)).is_err());
    put_ok_tokens(&fx, &finding("F-2", &a, "medium", "asks=\"plan\"\n"));
    let again = rec(&finding("F-4", &a, "medium", "asks=\"plan\"\n"));
    assert!(review::may_put_finding(&p, &st2, &again, None, Some(&t_am)).is_err(), "one per stage");
    // With a split, the amendment reviewer asks for nothing.
    events::append(&p, review::SPLIT_EVENT, json!({"amendment": a, "stage": "plan"})).unwrap();
    let other = rec(&finding("F-5", &a, "medium", "asks=\"requirements\"\n"));
    assert!(review::may_put_finding(&p, &st2, &other, None, Some(&t_am)).is_err());
    // A finding of a unit is not this rule's.
    let unit = rec("id=\"F-6\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\n");
    review::may_put_finding(&p, &st2, &unit, None, None).unwrap();
}

fn put_ok_tokens(fx: &Fx, text: &str) {
    // The record goes in through the file store, as the hook would after the rule allowed it.
    let rec: Value = toml::from_str(text).unwrap();
    fx.project().write_record(&rec).unwrap();
}

#[test]
fn the_page_data_shows_rounds_findings_and_the_park() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "architecture", "high", ""));
    put_ok(&fx, "finding", &finding("F-2", "architecture", "low", ""));
    gate_review(&fx, "architecture", "failed");
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    let v = review::page_section(&p, &st, "architecture").unwrap();
    assert_eq!(v["check"], "architecture-review");
    assert_eq!(v["on"], true);
    assert_eq!(v["round"], 1);
    assert_eq!(v["max_rounds"], 3);
    assert_eq!(v["parked"], false);
    assert_eq!(v["rounds"][0]["reviews"][0]["outcome"], "failed");
    assert_eq!(v["rounds"][0]["reviews"][0]["says"], "I read every record of the stage");
    assert_eq!(v["rounds"][0]["reviews"][0]["file"], "review.md");
    assert_eq!(v["findings"]["holding"][0]["id"], "F-1");
    assert_eq!(v["findings"]["holding"][0]["action"], "Change it");
    assert_eq!(v["findings"]["low"][0]["id"], "F-2");
}
