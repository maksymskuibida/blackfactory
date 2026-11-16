//! U-76: approval of an amendment is held on its review in the shape it was designed (R-98, D-72):
//! one solo review, an asked-for stage review, or the stage reviews with the review that the stages
//! agree. A plan repair needs none.

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

use std::path::Path;

const LINKED_COMPONENT: &str =
    "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n";

fn not_affected(fx: &Fx, id: &str, stage: &str) {
    put_ok(
        fx,
        "decision",
        &format!("id=\"{id}\"\nkind=\"decision\"\ntitle=\"The {stage} stage is not affected\"\nmaker=\"agent\"\nstage=\"{stage}\"\n"),
    );
}

fn narrow_all(fx: &Fx, opened: &Value) {
    let keep: Vec<String> = opened["impact_ids"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    let f: &Path = &fx.root.join("narrow.json");
    fs::write(f, json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

/// A covered, narrowed amendment A-1 of a design change, ready for the human's words.
fn narrowed() -> Fx {
    let fx = setup();
    put_ok(&fx, "component", LINKED_COMPONENT);
    not_affected(&fx, "D-91", "ui");
    not_affected(&fx, "D-92", "plan");
    let opened = fx.must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"]).json();
    narrow_all(&fx, &opened);
    fx
}

fn approve(fx: &Fx) -> common::Out {
    fx.human("Yes, approve the change to the second requirement.");
    fx.bf(&["amend", "approve", "A-1", "--quote", "approve the change"])
}

#[test]
fn an_amendment_with_no_review_is_refused_and_the_refusal_names_it() {
    let fx = narrowed();
    let o = approve(&fx);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("kind amendment"), "{}", o.stderr);
    // A review of the amendment as it is lets the approval through.
    attach(&fx, "X-15", "met", Some("A-1"), Some("amendment"));
    let o = approve(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn an_open_request_for_a_stage_review_holds_until_that_review_exists() {
    let fx = narrowed();
    put_ok(&fx, "finding", &finding("F-1", "A-1", "medium", "asks=\"requirements\"\n"));
    attach(&fx, "X-15", "failed", Some("A-1"), Some("amendment"));
    let o = approve(&fx);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("requirements stage"), "{}", o.stderr);
    attach(&fx, "X-15", "met", Some("A-1"), Some("requirements"));
    let o = approve(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn an_open_high_finding_holds_the_approval_by_name_and_the_humans_words_release_it() {
    let fx = narrowed();
    put_ok(&fx, "finding", &finding("F-1", "A-1", "high", ""));
    put_ok(&fx, "finding", &finding("F-2", "A-1", "low", ""));
    attach(&fx, "X-15", "failed", Some("A-1"), Some("amendment"));
    let o = approve(&fx);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("F-1"), "{}", o.stderr);
    assert!(!o.stderr.contains("F-2"), "{}", o.stderr);
    fx.human("I accept the first finding of the amendment.");
    fx.must(&["review", "finding", "A-1", "--accept", "F-1", "--quote", "accept the first finding"]);
    let o = approve(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_split_amendment_without_the_agreement_review_is_refused() {
    let fx = narrowed();
    let p = fx.project();
    for stage in ["requirements", "architecture"] {
        events::append(&p, review::SPLIT_EVENT, json!({"amendment": "A-1", "stage": stage})).unwrap();
    }
    attach(&fx, "X-15", "met", Some("A-1"), Some("requirements"));
    attach(&fx, "X-15", "met", Some("A-1"), Some("architecture"));
    let o = approve(&fx);
    assert_eq!(o.code, 2, "{}", o.all());
    println!("{}", o.stderr);
    assert!(o.stderr.contains("kind amendment"), "{}", o.stderr);
    assert!(!o.stderr.contains("requirements stage"), "{}", o.stderr);
    attach(&fx, "X-15", "met", Some("A-1"), Some("amendment"));
    let o = approve(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_plan_repair_is_approved_with_no_review() {
    let fx = setup();
    let opened = fx.must(&["amend", "open", "--repair", "X-1", "--json"]).json();
    let f = fx.root.join("x1.toml");
    fs::write(&f, "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -s src/hello.txt\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n").unwrap();
    fx.must(&["record", "put", "check", "--file", f.to_str().unwrap(), "--replace"]);
    narrow_all(&fx, &opened);
    let o = approve(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
}
