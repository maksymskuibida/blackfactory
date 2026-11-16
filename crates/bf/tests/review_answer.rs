//! U-76: `bf review finding` and `bf review release`, and a finding of a gate or an amendment put only
//! as the review rules allow (R-97, D-72, TH-47). The words of the human count only when written after
//! the review (or the park) and are taken once.

mod common;
use bf::checks::review;
use bf::events;
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

/// What the hooks do for an outcome on a review check: the rules, the sealed receipt and the event.
fn attach(fx: &Fx, check: &str, outcome: &str, amendment: Option<&str>, kind: Option<&str>) {
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    let rec = p.get_record(check).unwrap().unwrap();
    let a = review::on_attach(&p, &st, &rec, outcome, None, amendment, kind)
        .unwrap()
        .expect("a design review check");
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
        events::append(&p, review::PARK_EVENT, json!({"subject": a.subject, "round": a.round})).unwrap();
    }
}

fn finding(id: &str, gate_or_amend: &str, sev: &str, extra: &str) -> String {
    let which = if gate_or_amend.starts_with("A-") {
        format!("amendment=\"{gate_or_amend}\"")
    } else {
        format!("gate=\"{gate_or_amend}\"")
    };
    format!("id=\"{id}\"\nkind=\"finding\"\ntitle=\"Title of {id}\"\n{which}\nseverity=\"{sev}\"\naction=\"Change it\"\n{extra}")
}

fn gate_lines(fx: &Fx, gate: &str) -> Vec<String> {
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    review::gate_blocker(&p, &st, gate).unwrap()
}

#[test]
fn an_accept_quoting_words_written_before_the_review_is_refused_and_one_after_is_taken() {
    let fx = setup();
    fx.human("The scope reads fine to me, go on with it.");
    put_ok(&fx, "finding", &finding("F-1", "idea", "high", ""));
    attach(&fx, "X-11", "failed", None, None);
    let o = fx.refused(&["review", "finding", "idea", "--accept", "F-1", "--quote", "The scope reads fine"]);
    assert!(o.stderr.contains("written before"), "{}", o.stderr);
    assert!(gate_lines(&fx, "idea").iter().any(|l| l.contains("F-1")));
    let rec = fx.must(&["record", "get", "F-1", "--json"]).json();
    assert_ne!(rec["outcome"], "accepted");

    fx.human("I accept the point about the scope.");
    let o = fx.must(&["review", "finding", "idea", "--accept", "F-1", "--quote", "accept the point about the scope", "--json"]).json();
    assert_eq!(o["accepted"], json!(["F-1"]), "{o}");
    let rec = fx.must(&["record", "get", "F-1", "--json"]).json();
    assert_eq!(rec["outcome"], "accepted");
    assert!(gate_lines(&fx, "idea").is_empty(), "{:?}", gate_lines(&fx, "idea"));
    let p = fx.project();
    let log = events::load_log(&p).unwrap().events;
    let e = log.iter().find(|e| e["kind"] == review::ACCEPT_EVENT).expect("the accept event");
    assert_eq!(e["quote"], "accept the point about the scope");
    assert_eq!(e["subject"], "idea");
    // The same words are not taken a second time.
    put_ok(&fx, "finding", &finding("F-2", "idea", "medium", ""));
    attach(&fx, "X-11", "failed", None, None);
    let o = fx.refused(&["review", "finding", "idea", "--accept", "F-2", "--quote", "accept the point about the scope"]);
    assert!(o.stderr.contains("already used"), "{}", o.stderr);
}

#[test]
fn an_accept_names_findings_of_its_own_subject_and_open_ones_only() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "plan", "high", ""));
    attach(&fx, "X-11", "failed", None, None);
    fx.human("I accept this one.");
    // F-1 is a finding of the plan, not of the idea.
    let o = fx.refused(&["review", "finding", "idea", "--accept", "F-1", "--quote", "accept this one"]);
    assert!(o.stderr.contains("not a finding of idea"), "{}", o.stderr);
    fx.refused(&["review", "finding", "idea", "--accept", "F-404", "--quote", "accept this one"]);
    fx.refused(&["review", "finding", "build", "--accept", "F-1", "--quote", "accept this one"]);
    fx.refused(&["review", "finding", "idea", "--quote", "accept this one"]);
    // No review of the plan exists yet to answer.
    let o = fx.refused(&["review", "finding", "plan", "--accept", "F-1", "--quote", "accept this one"]);
    assert!(o.stderr.contains("no review"), "{}", o.stderr);
}

#[test]
fn a_release_on_older_words_is_refused_and_one_after_the_park_is_taken() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "requirements", "high", ""));
    fx.human("Look at the requirements again, please.");
    for _ in 0..3 {
        attach(&fx, "X-12", "failed", None, None);
    }
    assert!(review::parked(&fx.project(), "requirements").unwrap());
    let o = fx.refused(&["review", "release", "requirements", "--quote", "Look at the requirements again"]);
    assert!(o.stderr.contains("written before"), "{}", o.stderr);
    fx.refused(&["review", "release", "idea", "--quote", "Look at the requirements again"]);
    fx.human("I read it all, start the review over.");
    fx.must(&["review", "release", "requirements", "--quote", "start the review over"]);
    assert!(!review::parked(&fx.project(), "requirements").unwrap());
    // Released once; the words are used and the review is not parked.
    fx.refused(&["review", "release", "requirements", "--quote", "start the review over"]);
}

#[test]
fn a_finding_put_as_accepted_or_fixed_without_the_kinds_token_is_refused() {
    let fx = setup();
    fx.set_policy("tokens = false", "tokens = true");
    let token = fx.must(&["review", "token", "requirements", "--json"]).json()["token"].as_str().unwrap().to_string();
    let other = fx.must(&["review", "token", "plan", "--json"]).json()["token"].as_str().unwrap().to_string();
    let put = |text: &str, tok: Option<&str>, replace: bool| {
        let f = fx.root.join("f.toml");
        fs::write(&f, text).unwrap();
        let mut a = vec!["record", "put", "finding", "--file", f.to_str().unwrap()];
        if let Some(t) = tok {
            a.extend(["--token", t]);
        }
        if replace {
            a.push("--replace");
        }
        fx.bf(&a)
    };
    // No token, and another kind's token: refused.
    assert_ne!(put(&finding("F-1", "requirements", "high", ""), None, false).code, 0);
    assert_ne!(put(&finding("F-1", "requirements", "high", ""), Some(&other), false).code, 0);
    let o = put(&finding("F-1", "requirements", "high", ""), Some(&token), false);
    assert_eq!(o.code, 0, "{}", o.all());
    for outcome in ["fixed", "accepted"] {
        let text = finding("F-1", "requirements", "high", &format!("outcome=\"{outcome}\"\n"));
        let o = put(&text, None, true);
        assert_eq!(o.code, 2, "{outcome}: {}", o.all());
        let o = put(&text, Some(&other), true);
        assert_eq!(o.code, 2, "{outcome}: {}", o.all());
    }
    // Even with the kind's token, a put never accepts.
    let text = finding("F-1", "requirements", "high", "outcome=\"accepted\"\n");
    let o = put(&text, Some(&token), true);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("human accepts"), "{}", o.stderr);
    let rec = fx.must(&["record", "get", "F-1", "--json"]).json();
    assert!(rec["outcome"].is_null() || rec["outcome"] == "open", "{rec}");
    // The kind's token marks it fixed.
    let text = finding("F-1", "requirements", "high", "outcome=\"fixed\"\n");
    let o = put(&text, Some(&token), true);
    assert_eq!(o.code, 0, "{}", o.all());
    let _: Value = fx.must(&["record", "get", "F-1", "--json"]).json();
}

#[test]
fn the_usage_lines_of_bf_review_are_in_the_help() {
    let fx = Fx::new();
    let o = fx.must(&["--help"]);
    for line in ["finding <gate or A-n> --accept <F-ids> --quote", "release <gate or A-n> --quote"] {
        assert!(o.stdout.contains(line), "{}", o.stdout);
    }
}
