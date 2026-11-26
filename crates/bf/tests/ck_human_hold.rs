//! K-8 fix (R-81): an open blocking human finding holds `bf checkpoint close` also with the checkpoint review
//! switched off, and `bf next` does not offer the close while it stands.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit landed, one checkpoint listing a command check (X-2) and the review (X-3), the run started.
fn project(fx: &Fx, review: &str) {
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
        review,
        "idea-review=off:the test does not exercise it",
        "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it",
        "plan-review=off:the test does not exercise it",
        "--scope",
        "default",
    ]);
    fx.human("Yes, set the checkpoint review that way.");
    fx.must(&["checks", "set", "--quote", "set the checkpoint review that way"]);
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


fn actions(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["action"].as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn with_the_review_off_an_open_human_high_finding_holds_the_close() {
    let fx = Fx::new();
    project(&fx, "checkpoint-review=off:the test does not exercise it");
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    fx.human("The footer is wrong. Please look at it.");
    let o = file(&fx, "The footer is wrong.", Some("high"));
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r1.1"), "{}", o.all());
}

#[test]
fn bf_next_does_not_offer_the_close_while_an_open_human_finding_holds_it() {
    let fx = Fx::new();
    project(&fx, "checkpoint-review=on");
    review_with(&fx, vec![]);
    assert!(actions(&fx).contains(&"close-checkpoint".to_string()), "{:?}", actions(&fx));
    fx.human("The footer is wrong. Please look at it.");
    let o = file(&fx, "The footer is wrong.", Some("high"));
    assert_eq!(o.code, 0, "{}", o.all());
    let a = actions(&fx);
    assert!(!a.contains(&"close-checkpoint".to_string()), "{a:?}");
}
