//! U-141 (R-81, D-169, D-170, TH-22): the human files a finding of their own on a checkpoint in their quoted
//! words: a round with the source `human`, taken during every hold, held by the close, confirmed by the review.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit landed, one checkpoint listing a command check (X-2) and the review (X-3), the run started.
fn project(fx: &Fx) {
    let ids = vec!["X-2", "X-3"]
        .into_iter()
        .map(|x| format!("\"{x}\""))
        .collect::<Vec<_>>();
    let mut recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("journey", "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\nsteps=[\"open\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\",\"J-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".to_string()),
    ];
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\",\"J-1\"]\nchecks=[{}]\n", ids.join(","))));
    for (k, t) in recs {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on",
        "idea-review=off:the test does not exercise it",
        "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it",
        "plan-review=off:the test does not exercise it",
        "--scope",
        "default",
    ]);
    fx.human("Yes, the checkpoint review is on.");
    fx.must(&["checks", "set", "--quote", "the checkpoint review is on"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-1",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "src/a.txt",
        ],
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}


fn areas() -> Value {
    json!([{"area": "the close rule", "how": "read"}])
}

fn finding(title: &str, severity: &str) -> Value {
    json!({"title": title, "severity": severity, "where": "src/a.txt:1", "action": "change it"})
}

fn review_brief(fx: &Fx) -> String {
    let o = fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

fn review_report(fx: &Fx, body: Value) -> Out {
    fx.bf_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "K-1",
            "--role",
            "reviewer",
            "--result",
            "findings",
            "--findings",
            "-",
        ],
        &body.to_string(),
    )
}

/// The review runs and files `findings`.
fn review_with(fx: &Fx, findings: Vec<Value>) {
    review_brief(fx);
    let o = review_report(fx, json!({"areas": areas(), "findings": findings}));
    assert_eq!(o.code, 0, "{}", o.all());
}

fn file(fx: &Fx, quote: &str, severity: Option<&str>) -> Out {
    let mut v = vec![
        "checkpoint",
        "finding",
        "K-1",
        "--human",
        "--quote",
        quote,
        "--json",
    ];
    if let Some(s) = severity {
        v.extend(["--severity", s]);
    }
    fx.bf(&v)
}

fn human_events(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == "checkpoint.finding.human")
        .collect()
}

fn findings(fx: &Fx) -> Vec<Value> {
    fx.must(&["status", "--json"]).json()["checkpoint_reviews"][0]["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn state_of(fx: &Fx, id: &str) -> String {
    findings(fx)
        .iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("no finding {id}: {:?}", findings(fx)))["state"]
        .as_str()
        .unwrap()
        .to_string()
}

fn ignored(fx: &Fx) -> Vec<String> {
    bf::events::replay(&fx.project()).unwrap().ignored
}

fn accept(fx: &Fx, id: &str, quote: &str) -> Out {
    fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        id,
        "--quote",
        quote,
    ])
}

fn pass_fix(fx: &Fx) {
    fx.pass_checkpoint_fix("changed by the fix\n");
}

fn receipts_of(fx: &Fx, check: &str) -> usize {
    let mut n = 0;
    for e in fs::read_dir(fx.root.join("factory/receipts"))
        .unwrap()
        .flatten()
    {
        if let Ok(v) = toml::from_str::<Value>(&fs::read_to_string(e.path()).unwrap_or_default()) {
            if v["check"] == check && v["type"] == "attach" {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_human_finding_takes_the_next_round_number_and_the_source_human() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is wrong. Please look at it.");
    let o = file(&fx, "The footer is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    let v = o.json();
    assert_eq!(v["round"], 2, "{v}");
    assert_eq!(v["ids"], json!(["r2.1"]), "{v}");
    let f = findings(&fx);
    let h = f.iter().find(|f| f["id"] == "r2.1").unwrap();
    assert_eq!(h["source"], "human", "{h}");
    assert_eq!(h["state"], "open", "{h}");
    assert_eq!(h["source_kind"], "human", "{h}");
    let st = fx.must(&["status", "--json"]).json();
    let rounds = st["checkpoint_reviews"][0]["rounds"].as_array().unwrap();
    let r = rounds.iter().find(|r| r["round"] == 2).unwrap();
    assert_eq!(r["source"], "human", "{r}");
    assert_eq!(r["result"], "findings", "{r}");
    // A second filing from another message is the next round, and several quotes of one message are one round.
    fx.human("The header is wrong, and so is the menu.");
    let o = fx.bf_stdin(
        &fx.root,
        &["checkpoint", "finding", "K-1", "--human", "--findings", "-", "--json"],
        &json!([{"quote": "The header is wrong", "severity": "medium"}, {"quote": "so is the menu", "severity": "low"}]).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["ids"], json!(["r3.1", "r3.2"]), "{}", o.all());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn its_words_are_the_finding_and_the_weight_is_high_when_none_is_given() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is wrong. More words follow it.");
    assert_eq!(file(&fx, "The footer is wrong. More words follow it.", None).code, 0);
    let long = format!("{} end", "a long sentence without a stop ".repeat(8));
    fx.human(&long);
    assert_eq!(file(&fx, &long, Some("low")).code, 0);
    let ev = human_events(&fx);
    assert_eq!(ev.len(), 2);
    let f = &ev[0]["findings"][0];
    assert_eq!(f["action"], "The footer is wrong. More words follow it.");
    assert_eq!(f["title"], "The footer is wrong.");
    assert_eq!(f["severity"], "high");
    assert_eq!(f["severity_given"], false);
    assert_eq!(f["by"], "human");
    assert_eq!(f["where"], "");
    assert!(f["turn"].as_u64().unwrap() > 0);
    assert_eq!(ev[0]["round"], 2);
    assert_eq!(ev[0]["quotes"], json!(["The footer is wrong. More words follow it."]));
    let g = &ev[1]["findings"][0];
    assert_eq!(g["severity"], "low");
    assert_eq!(g["severity_given"], true);
    assert_eq!(g["action"], json!(long));
    assert_eq!(g["title"].as_str().unwrap().chars().count(), 120);
    // An unknown severity and an empty list are refused.
    fx.human("The sidebar is wrong.");
    let o = file(&fx, "The sidebar is wrong.", Some("urgent"));
    assert_ne!(o.code, 0);
    assert!(o.all().contains("urgent"), "{}", o.all());
    let o = fx.bf_stdin(
        &fx.root,
        &["checkpoint", "finding", "K-1", "--human", "--findings", "-"],
        "[]",
    );
    assert_ne!(o.code, 0, "{}", o.all());
    let o = fx.bf(&["checkpoint", "finding", "K-9", "--human", "--quote", "The sidebar is wrong."]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("no checkpoint of the plan"), "{}", o.all());
}

#[test]
fn a_quote_in_no_message_of_the_human_is_refused() {
    let fx = Fx::new();
    project(&fx);
    // No message at all after the first unit started.
    let o = file(&fx, "Something nobody wrote.", None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().to_lowercase().contains("ask the human"), "{}", o.all());
    fx.human("The footer is wrong.");
    let o = file(&fx, "Something nobody wrote.", None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().to_lowercase().contains("the footer is wrong."), "{}", o.all());
    // One quote of two in no message refuses the whole filing.
    let o = fx.bf_stdin(
        &fx.root,
        &["checkpoint", "finding", "K-1", "--human", "--findings", "-"],
        &json!([{"quote": "The footer is wrong."}, {"quote": "never said"}]).to_string(),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(human_events(&fx).is_empty());
}

#[test]
fn a_message_already_filed_from_is_refused() {
    let fx = Fx::new();
    project(&fx);
    fx.human("The footer is wrong. The header is wrong.");
    assert_eq!(file(&fx, "The footer is wrong.", None).code, 0);
    let o = file(&fx, "The header is wrong.", None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("already filed from"), "{}", o.all());
    assert_eq!(human_events(&fx).len(), 1);
}

#[test]
fn a_message_written_before_the_first_unit_started_is_refused() {
    let fx = Fx::new();
    project(&fx);
    let o = file(&fx, "I approve the plan.", None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.all().contains("before the first unit of K-1 started"),
        "{}",
        o.all()
    );
    assert!(human_events(&fx).is_empty());
}

#[test]
fn one_message_decides_a_finding_and_files_one_in_either_order() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![finding("first low", "low"), finding("second low", "low")]);
    // Decide first, then file.
    fx.human("Accept the first low one. The header is wrong.");
    let o = accept(&fx, "r1.1", "Accept the first low one.");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = file(&fx, "The header is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    // File first, then decide.
    fx.human("The sidebar is wrong. Accept the second low one.");
    let o = file(&fx, "The sidebar is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = accept(&fx, "r1.2", "Accept the second low one.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(state_of(&fx, "r1.1").starts_with("accepted"));
    assert!(state_of(&fx, "r1.2").starts_with("accepted"));
    assert_eq!(state_of(&fx, "r2.1"), "open");
    assert_eq!(state_of(&fx, "r3.1"), "open");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn the_message_that_filed_a_finding_cannot_decide_it() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is wrong.");
    assert_eq!(file(&fx, "The footer is wrong.", None).code, 0);
    let o = accept(&fx, "r2.1", "The footer is wrong.");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("written before"), "{}", o.all());
    assert_eq!(state_of(&fx, "r2.1"), "open");
    fx.human("Yes, accept the footer one.");
    let o = accept(&fx, "r2.1", "accept the footer one");
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(state_of(&fx, "r2.1").starts_with("accepted"));
}

#[test]
fn a_filing_is_taken_while_a_fix_is_at_work_and_while_a_sync_is_open() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![finding("the big problem", "high")]);
    let st = fx.must(&["status", "--json"]).json();
    assert!(!st["checkpoint_fix"]["open"].is_null(), "{st}");
    fx.human("The footer is wrong.");
    let o = file(&fx, "The footer is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    // A sync that stops at a conflict stays open.
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/a.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "src/a.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    let o = fx.must(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in", "--json"]);
    assert_eq!(o.json()["sync_open"], true, "{}", o.all());
    fx.human("The header is wrong.");
    let o = file(&fx, "The header is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(human_events(&fx).len(), 2);
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_filing_is_taken_while_a_reopened_unit_has_not_landed_and_its_decision_stays_held() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![finding("small", "low")]);
    let p = fx.project();
    bf::events::append(&p, "amend.open", json!({"amend": "A-1", "ids": ["R-1"], "impact": null, "impact_ids": ["U-1"]})).unwrap();
    bf::events::append(&p, "amend.narrow", json!({"amend": "A-1", "keep": ["U-1"], "waive": [], "reasons": {}})).unwrap();
    fx.human("Yes, approve the amendment.");
    let turn = bf::events::replay(&p).unwrap().turns.last().unwrap().seq;
    bf::events::append(
        &p,
        "amend.approve",
        json!({"amend": "A-1", "turn": turn, "quote": "approve the amendment", "reopen_units": ["U-1"],
               "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0, "renamed": []}),
    )
    .unwrap();
    fx.must(&["unit", "start", "U-1"]);
    fx.human("The footer is wrong.");
    let o = file(&fx, "The footer is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = accept(&fx, "r2.1", "The footer is wrong.");
    assert_ne!(o.code, 0, "{}", o.all());
    assert_eq!(state_of(&fx, "r2.1"), "open");
}

#[test]
fn a_filing_is_taken_at_the_budget_cap_and_its_decision_stays_held() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![finding("the big problem", "high")]);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    bf::events::append(&fx.project(), "unit.role-report", json!({"unit": "U-1", "role": "auditor", "cost": 2.0})).unwrap();
    fx.set_policy("cap = 0.0", "cap = 1.0");
    let o = fx.bf(&["next"]);
    assert_eq!(o.code, 2, "{}", o.all());
    fx.human("The footer is wrong.");
    let o = file(&fx, "The footer is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = accept(&fx, "r2.1", "The footer is wrong.");
    assert_ne!(o.code, 0, "{}", o.all());
    fx.set_policy("cap = 1.0", "cap = 0.0");
    let o = accept(&fx, "r2.1", "The footer is wrong.");
    assert_ne!(o.code, 0, "{}", o.all());
    fx.human("Yes, accept the footer one.");
    let o = accept(&fx, "r2.1", "accept the footer one");
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_filing_is_taken_while_the_checkpoint_is_parked_at_the_limit_of_reviews_and_its_decision_stays_held() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    project(&fx);
    review_with(&fx, vec![finding("the big problem", "high")]);
    let d = fx.must(&["next", "--json"]).json();
    assert!(d["directives"].to_string().contains("decide-checkpoint-review"), "{d}");
    fx.human("The footer is wrong.");
    let o = file(&fx, "The footer is wrong.", None);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = accept(&fx, "r2.1", "The footer is wrong.");
    assert_ne!(o.code, 0, "{}", o.all());
    assert_eq!(state_of(&fx, "r2.1"), "open");
}

#[test]
fn a_filing_on_a_closed_checkpoint_is_refused() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.must(&["checkpoint", "close", "K-1"]);
    fx.human("The footer is wrong.");
    let o = file(&fx, "The footer is wrong.", None);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("closed"), "{}", o.all());
    assert!(o.all().contains("amendment"), "{}", o.all());
    assert!(human_events(&fx).is_empty());
}

#[test]
fn a_forged_filing_that_names_an_existing_finding_is_ignored_by_the_replay() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is wrong.");
    assert_eq!(file(&fx, "The footer is wrong.", None).code, 0);
    let existing = human_events(&fx)[0]["findings"][0].clone();
    fx.human("The header is wrong.");
    let turn = bf::events::replay(&fx.project()).unwrap().turns.last().unwrap().seq;
    let forged = |round: u32, findings: Value, turn: u64| {
        bf::events::append(
            &fx.project(),
            "checkpoint.finding.human",
            json!({"checkpoint": "K-1", "round": round, "content": "", "findings": findings,
                   "fix": false, "fix_quote": "", "turn": turn, "quotes": ["The header is wrong."]}),
        )
        .unwrap();
    };
    forged(3, json!([existing]), turn);
    let notes = ignored(&fx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("already has"), "{notes:?}");
    // A taken round number, and a message that is no message of the human, are ignored too.
    let own = finding_of("The header is wrong.", turn);
    forged(2, json!([own.clone()]), turn);
    forged(3, json!([own.clone()]), 999_999);
    let notes = ignored(&fx);
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes[1].contains("is taken"), "{notes:?}");
    assert!(notes[2].contains("no valid message"), "{notes:?}");
    assert_eq!(findings(&fx).len(), 1, "{:?}", findings(&fx));
}

fn finding_of(words: &str, turn: u64) -> Value {
    json!({"title": words, "severity": "high", "where": "", "action": words,
           "severity_given": false, "by": "human", "turn": turn})
}

#[test]
fn accepting_a_human_finding_during_a_hold_is_refused() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![finding("the big problem", "high")]);
    fx.human("The footer is wrong.");
    assert_eq!(file(&fx, "The footer is wrong.", None).code, 0);
    fx.human("Yes, accept the footer one.");
    let o = accept(&fx, "r2.1", "accept the footer one");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("bf checkpoint fix K-1 --abort"), "{}", o.all());
    assert_eq!(state_of(&fx, "r2.1"), "open");
}

#[test]
fn the_close_names_an_open_human_finding_with_its_source() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is wrong.");
    assert_eq!(file(&fx, "The footer is wrong.", None).code, 0);
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r2.1"), "{}", o.all());
    assert!(o.all().contains("the human wrote themselves"), "{}", o.all());
    // A low one holds nothing.
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is a little off.");
    assert_eq!(file(&fx, "The footer is a little off.", Some("low")).code, 0);
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// A review that filed nothing, then a human finding (r2.1) the human sends to a fix that passes.
fn answered_human_finding(fx: &Fx) {
    project(fx);
    review_with(fx, vec![]);
    fx.human("The footer is wrong.");
    assert_eq!(file(fx, "The footer is wrong.", None).code, 0);
    fx.human("Yes, fix the footer one.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r2.1",
        "--quote",
        "fix the footer one",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    pass_fix(fx);
}

#[test]
fn the_next_review_must_confirm_an_answered_human_finding() {
    let fx = Fx::new();
    answered_human_finding(&fx);
    assert_eq!(state_of(&fx, "r2.1"), "answered");
    let md = review_brief(&fx);
    assert!(md.contains("Answered findings to confirm"), "{md}");
    assert!(md.contains("r2.1") && md.contains("The footer is wrong."), "{md}");
    let o = review_report(&fx, json!({"areas": areas(), "findings": []}));
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r2.1"), "{}", o.all());
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r2.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(state_of(&fx, "r2.1"), "fixed");
}

#[test]
fn with_the_review_switched_off_a_human_finding_is_fixed_when_its_fix_ends() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is wrong.");
    assert_eq!(file(&fx, "The footer is wrong.", None).code, 0);
    fx.must(&["checks", "propose", "checkpoint-review=off:faster", "--scope", "run"]);
    fx.human("Yes, switch the review off.");
    fx.must(&["checks", "set", "--quote", "switch the review off"]);
    fx.human("Yes, fix the footer one.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r2.1",
        "--quote",
        "fix the footer one",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    pass_fix(&fx);
    assert_eq!(state_of(&fx, "r2.1"), "fixed");
}

#[test]
fn a_human_round_counts_against_no_limit_and_writes_no_receipt() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 2");
    project(&fx);
    review_with(&fx, vec![]);
    let reviews = |fx: &Fx| {
        fx.must(&["status", "--json"]).json()["checkpoint_reviews"][0]["reviews"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(reviews(&fx), 1);
    let receipts = receipts_of(&fx, "X-3");
    for (i, words) in ["The footer is a bit off.", "The header is a bit off."]
        .iter()
        .enumerate()
    {
        fx.human(words);
        assert_eq!(file(&fx, words, Some("low")).code, 0);
        assert_eq!(reviews(&fx), 1, "after filing {i}");
    }
    assert_eq!(receipts_of(&fx, "X-3"), receipts);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["checkpoint_reviews"][0]["parked"], false, "{st}");
}

#[test]
fn a_log_with_no_human_finding_replays_as_before() {
    let fx = Fx::new();
    project(&fx);
    review_with(&fx, vec![finding("the big problem", "high")]);
    assert!(human_events(&fx).is_empty());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
    let f = findings(&fx);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0]["source"], "");
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.ck_human_filed.is_empty() && st.ck_human_seqs.is_empty());
    let _ = PathBuf::new();
}

#[test]
fn a_blocking_human_finding_at_the_limit_of_reviews_does_not_park_the_checkpoint() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    project(&fx);
    review_with(&fx, vec![]);
    fx.human("The footer is wrong.");
    assert_eq!(file(&fx, "The footer is wrong.", Some("high")).code, 0);
    fx.human("The header is wrong.");
    assert_eq!(file(&fx, "The header is wrong.", Some("medium")).code, 0);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["checkpoint_reviews"][0]["parked"], false, "{st}");
    let d = fx.must(&["next", "--json"]).json();
    assert!(!d["directives"].to_string().contains("checkpoint-review\""), "{d}");
    fx.human("Send r2.1 to a fix.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r2.1",
        "--quote",
        "Send r2.1 to a fix.",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
}
