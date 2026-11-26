//! U-136 (R-79): findings the human picks reach an open checkpoint fix that waits for them.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit, one checkpoint listing a command check, the review (X-3), a security audit (X-4), a legal audit
/// (X-5) and the `extra` agent checks `(id, named)`.
fn project(fx: &Fx, extra: &[(&str, &str)]) {
    let mut ids = vec!["X-2", "X-3", "X-4", "X-5"]
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
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"security\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"security-audit\"\n".to_string()),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"legal\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"legal-audit\"\n".to_string()),
    ];
    for (id, named) in extra {
        ids.push(format!("\"{id}\""));
        recs.push(("check", format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"{named}\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"{named}\"\n")));
    }
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
        "security-audit=on",
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

fn fix_brief(fx: &Fx, role: &str) -> String {
    let o = fx.must(&["brief", "K-1", "--role", role, "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

fn built(fx: &Fx) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "changed by the fix\n").unwrap();
    common::git(&k, &["add", "-A"]);
    common::git(&k, &["commit", "-q", "-m", "fix"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
}

/// The checkpoint review files a high finding (r1.1): the report opens the fix for it.
fn open_fix(fx: &Fx) {
    project(fx, &[]);
    review_brief(fx);
    let o = review_report(
        fx,
        json!({"areas": areas(), "findings": [finding("the big problem", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(fx), "fix");
}

fn fix_review_low(fx: &Fx) -> Out {
    let k = fx.work("K-1");
    fx.bf_stdin(
        &k,
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
        &json!({"findings": [finding("low one", "low"), finding("low two", "low")]}).to_string(),
    )
}

/// A fix waiting on two low findings of its own review (f1.1 and f1.2).
fn waiting_on_low(fx: &Fx) {
    open_fix(fx);
    built(fx);
    let o = fix_review_low(fx);
    assert_eq!(o.code, 0, "{}", o.all());
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["checkpoint_fix"]["open"]["phase"], "low", "{st}");
}

fn send(fx: &Fx, ids: &str, words: &str) -> Out {
    fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        ids,
        "--quote",
        words,
    ])
}

fn phase(fx: &Fx) -> Value {
    fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"]["phase"].clone()
}

#[test]
fn fix_findings_takes_an_f_id_of_the_open_fixs_review() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    fx.human("Send the first low one to the fix.");
    let o = send(&fx, "f9.9", "Send the first low one to the fix.");
    assert_ne!(o.code, 0);
    assert!(o.all().contains("f9.9"), "{}", o.all());
    assert!(o.all().contains("f<round>.<number>"), "{}", o.all());
    let o = send(&fx, "f1.1", "Send the first low one to the fix.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "fix");
}

#[test]
fn a_fix_waiting_on_low_findings_takes_the_named_finding_and_goes_back_to_work() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    let rounds = fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"]["rounds"].clone();
    fx.human("Fix the second low one as well.");
    let o = send(&fx, "f1.2", "Fix the second low one as well.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "fix");
    assert_eq!(
        fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"]["rounds"],
        rounds
    );
    let md = fix_brief(&fx, "builder");
    assert!(md.contains("low two"), "{md}");
    let d = fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == "fix-checkpoint")
        .cloned()
        .expect("a fix directive");
    assert!(
        d["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["title"] == "low two"),
        "{d}"
    );
}

#[test]
fn the_release_from_the_low_findings_carries_every_open_low_finding() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    fx.human("Go on, fix them all.");
    let o = fx.must(&[
        "checkpoint",
        "fix",
        "K-1",
        "--release",
        "--quote",
        "Go on, fix them all.",
        "--json",
    ]);
    let v = o.json();
    assert_eq!(v["carried"], json!(["f1.1", "f1.2"]), "{v}");
    assert!(v["message"].as_str().unwrap().contains("f1.1, f1.2"), "{v}");
    assert_eq!(phase(&fx), "fix");
    let md = fix_brief(&fx, "builder");
    assert!(md.contains("low one") && md.contains("low two"), "{md}");
}

#[test]
fn sending_a_finding_to_a_fix_at_work_is_refused() {
    let fx = Fx::new();
    open_fix(&fx);
    fx.human("Also send the small one.");
    let o = send(&fx, "r1.1", "Also send the small one.");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("at work"), "{}", o.all());
    assert!(
        o.all().contains("bf checkpoint fix K-1 --abort"),
        "{}",
        o.all()
    );
    assert_eq!(phase(&fx), "fix");
}

#[test]
fn the_other_runs_case_one_sent_one_accepted_and_the_brief_holds_the_one_sent() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    fx.human("Accept the second low one.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        "f1.2",
        "--quote",
        "Accept the second low one.",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "low");
    fx.human("Send the first low one to the fix.");
    let o = send(&fx, "f1.1", "Send the first low one to the fix.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "fix");
    let md = fix_brief(&fx, "builder");
    let part = &md[md.find("Findings to fix").expect(&md)..];
    assert!(part.contains("low one"), "{md}");
    assert!(!part.contains("low two"), "{md}");
}

#[test]
fn words_written_before_the_low_findings_were_put_are_refused() {
    let fx = Fx::new();
    open_fix(&fx);
    fx.human("Send everything small to the fix.");
    built(&fx);
    assert_eq!(fix_review_low(&fx).code, 0);
    let o = send(&fx, "f1.1", "Send everything small to the fix.");
    assert_ne!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "low");
}

/// The review files a low finding (r1.1) that no fix takes; the security audit files a high one (r2.1) and the
/// human sends that to a fix, which is open and at work.
fn open_fix_with_small_one(fx: &Fx) {
    project(fx, &[]);
    review_brief(fx);
    let o = review_report(
        fx,
        json!({"areas": areas(), "findings": [finding("the small one", "low")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf(&["brief", "K-1", "--role", "security-auditor", "--check", "X-4", "--json"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "K-1",
            "--role",
            "security-auditor",
            "--check",
            "X-4",
            "--result",
            "findings",
            "--findings",
            "-",
        ],
        &json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Fix the leak.");
    let o = send(fx, "r2.1", "Fix the leak.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(fx), "fix");
}

/// The fix's builder works again and its reviewer passes it: the fix ends.
fn finish_fix(fx: &Fx) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "changed again by the fix\n").unwrap();
    common::git(&k, &["add", "-A"]);
    common::git(&k, &["commit", "-q", "-m", "fix again"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
}

fn fate_of(fx: &Fx, round: usize, i: usize) -> String {
    let st = fx.must(&["status", "--json"]).json();
    let r = st["checkpoint_reviews"][0]["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["round"].as_u64() == Some(round as u64))
        .unwrap()
        .clone();
    r["outcomes"][i]["fate"].as_str().unwrap().to_string()
}

#[test]
fn an_r_finding_sent_to_a_fix_waiting_on_low_findings_is_answered_when_the_fix_ends() {
    let fx = Fx::new();
    open_fix_with_small_one(&fx);
    built(&fx);
    assert_eq!(fix_review_low(&fx).code, 0);
    assert_eq!(phase(&fx), "low");
    fx.human("Send the small one to the fix.");
    let o = send(&fx, "r1.1", "Send the small one to the fix.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "fix");
    finish_fix(&fx);
    assert!(fate_of(&fx, 2, 0).starts_with("answered by"));
    let f = fate_of(&fx, 1, 0);
    assert!(f.starts_with("answered by"), "{f}");
}

#[test]
fn an_r_finding_sent_to_a_parked_fix_is_answered_when_the_fix_ends() {
    let fx = Fx::new();
    open_fix_with_small_one(&fx);
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.fix.park",
        json!({"checkpoint": "K-1", "reason": "stuck", "attempts": 1}),
    );
    assert!(r.is_ok());
    assert_eq!(phase(&fx), "parked");
    fx.human("Send the small one to the fix.");
    let o = send(&fx, "r1.1", "Send the small one to the fix.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "fix");
    finish_fix(&fx);
    let f = fate_of(&fx, 1, 0);
    assert!(f.starts_with("answered by"), "{f}");
}

#[test]
fn the_replay_ignores_a_fix_add_naming_an_r_finding_that_has_a_fate() {
    let fx = Fx::new();
    open_fix_with_small_one(&fx);
    built(&fx);
    assert_eq!(fix_review_low(&fx).code, 0);
    fx.human("Accept the small one.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        "r1.1",
        "--quote",
        "Accept the small one.",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Send it anyway.");
    let turn = bf::events::replay_log(&bf::events::load_log(&fx.project()).unwrap())
        .turns
        .iter()
        .find(|t| t.text == "Send it anyway.")
        .unwrap()
        .seq;
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.fix.add",
        json!({"checkpoint": "K-1", "finding_ids": [{"round": 1, "index": 1}], "fix_findings": [], "turn": turn, "quote": "Send it anyway."}),
    );
    assert!(r.is_ok());
    let notes = fx.must(&["status", "--json"]).json()["ignored_events"].to_string();
    assert!(notes.contains("not an open finding the fix can take"), "{notes}");
    assert_eq!(phase(&fx), "low");
}

/// F-672: a fix opened for a failed check only has no finding ids of its own; a finding sent to it while it
/// waits on low findings is still answered when the fix ends.
#[test]
fn an_r_finding_sent_to_a_waiting_fix_opened_for_a_failed_check_is_answered_when_the_fix_ends() {
    let fx = Fx::new();
    project(&fx, &[]);
    review_brief(&fx);
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [finding("the small one", "low")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    for (role, check) in [("security-auditor", "X-4"), ("legal-auditor", "X-5")] {
        let o = fx.bf(&["brief", "K-1", "--role", role, "--check", check, "--json"]);
        assert_eq!(o.code, 0, "{}", o.all());
        let o = fx.bf_stdin(
            &fx.root,
            &["report", "K-1", "--role", role, "--check", check, "--result", "findings", "--findings", "-"],
            &json!({"areas": areas(), "findings": []}).to_string(),
        );
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["evidence", "fail", "X-2", "--reason", "the file is gone"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["next", "--json"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["checkpoint_fix"]["open"]["source"], "check", "{st}");
    built(&fx);
    assert_eq!(fix_review_low(&fx).code, 0);
    assert_eq!(phase(&fx), "low");
    fx.human("Send the small one to the fix.");
    let o = send(&fx, "r1.1", "Send the small one to the fix.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "fix");
    finish_fix(&fx);
    let f = fate_of(&fx, 1, 0);
    assert!(f.starts_with("answered by"), "{f}");
}
