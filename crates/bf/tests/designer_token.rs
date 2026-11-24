//! U-70: a record of a design stage, or of an amendment that is not a plan repair, is accepted only with
//! the token the engine issued for that stage or amendment (R-67), keeps the model the designer
//! reported, and the pages say who made it and show the policy in force (R-66).

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;

const REQ: &str = "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/a.txt exists\"]\n";
const COMP: &str = "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n";

/// A project with role tokens on.
fn shipped() -> Fx {
    let fx = Fx::new();
    fx.set_policy("tokens = false", "tokens = true");
    fx
}

fn put_with(fx: &Fx, kind: &str, text: &str, extra: &[&str]) -> common::Out {
    let f = fx.root.join("designer-token-record.toml");
    fs::write(&f, text).unwrap();
    let mut a = vec!["record", "put", kind, "--file", f.to_str().unwrap()];
    a.extend_from_slice(extra);
    let o = fx.bf(&a);
    let _ = fs::remove_file(&f);
    o
}

fn token(fx: &Fx, subject: &str, role: &str) -> String {
    fx.must(&["roles", "token", subject, role, "--json"]).json()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn rows(page: &Value) -> Vec<Value> {
    page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().cloned().unwrap_or_default())
        .collect()
}

fn row<'a>(all: &'a [Value], id: &str) -> &'a Value {
    all.iter()
        .find(|r| r["id"] == id && !r["made_by"].is_null())
        .unwrap_or_else(|| panic!("no row {id} with its maker: {all:?}"))
}

#[test]
fn a_design_record_without_the_designers_token_is_refused_naming_the_token() {
    let fx = shipped();
    let o = put_with(&fx, "requirement", REQ, &[]);
    assert_ne!(o.code, 0, "{}", o.all());
    println!("{}", o.stderr);
    assert!(o.stderr.contains("designer's token"), "{}", o.stderr);
    assert!(o.stderr.contains("bf roles token design designer"), "{}", o.stderr);
    // Another role's token does not do, and nothing was written.
    let other = token(&fx, "design", "lawyer");
    let o = put_with(&fx, "requirement", REQ, &["--token", &other, "--model", "m-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("designer's token"), "{}", o.stderr);
    assert!(fx.bf(&["record", "get", "R-1"]).code != 0);
}

#[test]
fn with_the_designers_token_it_is_accepted_and_keeps_the_reported_model() {
    let fx = shipped();
    let t = token(&fx, "design", "designer");
    // The model is the agent's own report, and it is asked for.
    let o = put_with(&fx, "requirement", REQ, &["--token", &t]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("--model"), "{}", o.stderr);
    let o = put_with(&fx, "requirement", REQ, &["--token", &t, "--model", "model-x"]);
    assert_eq!(o.code, 0, "{}", o.all());
    // The same token puts the next record: it is spent by the report, not by a put.
    let o = put_with(&fx, "component", COMP, &["--token", &t, "--model", "model-x"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let log = fs::read_to_string(fx.root.join(".bf/log.jsonl")).or_else(|_| {
        Ok::<String, std::io::Error>(fx.must(&["log", "--tail", "50", "--json"]).stdout)
    });
    assert!(log.unwrap().contains("model-x"));
    // The page data says the designer agent made it, and on which model.
    let page = fx.must(&["view", "gate", "requirements"]).json();
    let all = rows(&page);
    let r = row(&all, "R-1");
    assert_eq!(r["made_by"]["by"], "designer agent", "{r}");
    assert_eq!(r["made_by"]["model"], "model-x", "{r}");
    assert_eq!(r["made_by"]["role"], "designer", "{r}");
}

#[test]
fn the_human_lifts_the_rule_for_one_stage_in_their_words() {
    let fx = shipped();
    // Without the words there is nothing to lift with.
    let o = fx.bf(&["roles", "lift", "requirements", "--quote", "write them yourself"]);
    assert_ne!(o.code, 0, "{}", o.all());
    fx.human("Yes, you may write the requirements yourself, no designer is needed for them.");
    let o = fx.bf(&["roles", "lift", "requirements", "--quote", "draft them all yourself"]);
    assert_ne!(o.code, 0, "words that are not in the message: {}", o.all());
    fx.must(&[
        "roles",
        "lift",
        "requirements",
        "--quote",
        "you may write the requirements yourself",
    ]);
    // Accepted without a token, for that stage only.
    let o = put_with(&fx, "requirement", REQ, &[]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = put_with(&fx, "component", COMP, &[]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("designer's token"), "{}", o.stderr);
    // The page says it was made without a designer, with the quote.
    let page = fx.must(&["view", "gate", "requirements"]).json();
    let all = rows(&page);
    let r = row(&all, "R-1");
    assert_eq!(r["made_by"]["by"], "conductor", "{r}");
    assert!(
        r["made_by"]["lifted"]
            .as_str()
            .unwrap()
            .contains("you may write the requirements yourself"),
        "{r}"
    );
    // A lifted rule is lifted once, and one message lifts once.
    fx.refused(&["roles", "lift", "requirements", "--quote", "you may write the requirements yourself"]);
    fx.refused(&["roles", "lift", "nowhere", "--quote", "you may"]);
}

/// The tiny design, put with the rule off, then the rule on.
fn tiny_with_tokens() -> Fx {
    let fx = Fx::tiny_project();
    fx.set_policy("tokens = false", "tokens = true");
    fx
}

#[test]
fn a_plan_repair_needs_no_designer_and_an_amendment_needs_its_designers_token() {
    let fx = tiny_with_tokens();
    let o = fx.must(&["amend", "open", "--repair", "X-1", "--json"]);
    let _ = o;
    // A plan repair: its check is put with no token.
    let changed = "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file again\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n";
    let o = put_with(&fx, "check", changed, &["--replace"]);
    assert_eq!(o.code, 0, "{}", o.all());
    // A plan repair has no designer to issue a token for.
    fx.refused(&["roles", "token", "A-1", "designer"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "done with the repair"]);

    // An amendment that holds a design part needs the token.
    fx.must(&["amend", "open", "R-2", "--json"]);
    let newer = "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"A farewell exists, kindly\"\nacceptance=[\"src/bye.txt exists\"]\n";
    let o = put_with(&fx, "requirement", newer, &["--replace"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("designer's token"), "{}", o.stderr);
    assert!(o.stderr.contains("A-2"), "{}", o.stderr);
    // Its brief carries the tier and the effort of the amendment's kind of work.
    let tok = token(&fx, "A-2", "designer");
    let brief = fx.must(&["brief", "A-2", "--role", "designer", "--json"]).json();
    let md = fs::read_to_string(brief["path"].as_str().unwrap()).unwrap();
    assert!(md.contains("tier strong, effort high"), "{md}");
    assert!(md.contains("--model"), "{md}");
    // The designer's token puts it, with the model reported.
    let o = put_with(&fx, "requirement", newer, &["--replace", "--token", &tok, "--model", "model-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let run = fx.must(&["view", "run"]).json();
    let all = rows(&run);
    let r = row(&all, "R-2");
    assert_eq!(r["made_by"]["model"], "model-y", "{r}");
    assert_eq!(r["made_by"]["subject"], "A-2", "{r}");
}

#[test]
fn a_stage_agents_token_puts_only_the_ids_its_stage_was_given() {
    let fx = tiny_with_tokens();
    fx.must(&["amend", "open", "R-2", "C-1", "--json"]);
    let general = token(&fx, "A-1", "designer");
    fx.must(&[
        "review", "split", "A-1", "--stage", "requirements=R-2", "--stage", "architecture=C-1",
        "--token", &general,
    ]);
    let stage = fx
        .must(&[
            "review", "token", "--subject", "A-1", "--stage", "requirements", "--role",
            "designer", "--json",
        ])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let r2 = "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"A farewell exists, kindly\"\nacceptance=[\"src/bye.txt exists\"]\n";
    let c1 = "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages, kindly\"\n";
    let o = put_with(&fx, "requirement", r2, &["--replace", "--token", &stage, "--model", "model-z"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = put_with(&fx, "component", c1, &["--replace", "--token", &stage, "--model", "model-z"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("designer's token"), "{}", o.stderr);
}

#[test]
fn the_page_data_shows_the_policy_in_force() {
    let fx = Fx::new();
    let page = fx.must(&["view", "run"]).json();
    for k in ["conductor", "designer", "amender", "advisor", "builder", "reviewer", "checkpoint_reviewer", "verifier"] {
        assert!(page["policy"][k]["tier"].is_string(), "{k}: {}", page["policy"]);
        assert!(page["policy"][k]["effort"].is_string(), "{k}");
    }
    assert_eq!(page["policy"]["designer"]["tier"], json!("strong"));
    let sec = page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "policy")
        .expect("a policy section");
    assert_eq!(sec["rows"].as_array().unwrap().len(), 8);
    let g = fx.must(&["view", "gate", "plan"]).json();
    assert_eq!(g["policy"]["amender"]["effort"], json!("high"));
    assert!(g["review"]["check"].is_string(), "{}", g["review"]);
}

#[test]
fn a_gate_is_refused_for_want_of_its_review_and_a_stage_token_still_puts() {
    let fx = Fx::with_checks();
    fx.checks_decided_as_default();
    fx.tiny();
    fx.set_policy("tokens = false", "tokens = true");
    // The review of the requirements is on and none was made: the gate does not open on that count.
    let o = fx.bf(&["gate", "open", "requirements"]);
    assert_ne!(o.code, 0, "{}", o.all());
    println!("{}", o.stderr);
    assert!(
        o.stderr.contains("review"),
        "the refusal names the review: {}",
        o.stderr
    );
    // A stage agent's token puts a record its stage was given, with the reviews on.
    fx.must(&["amend", "open", "R-2", "--json"]);
    let general = token(&fx, "A-1", "designer");
    fx.must(&["review", "split", "A-1", "--stage", "requirements=R-2", "--token", &general]);
    let stage = fx
        .must(&[
            "review", "token", "--subject", "A-1", "--stage", "requirements", "--role",
            "designer", "--json",
        ])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let r2 = "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"A farewell exists, kindly\"\nacceptance=[\"src/bye.txt exists\"]\n";
    let o = put_with(&fx, "requirement", r2, &["--replace", "--token", &stage, "--model", "model-z"]);
    assert_eq!(o.code, 0, "{}", o.all());
}
