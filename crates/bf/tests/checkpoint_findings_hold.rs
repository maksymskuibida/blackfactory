// R-88, D-62: a finding of a checkpoint review stays open until a fix opened for that very finding has passed
// its review or the human settles it, and a checkpoint never closes while one of its fixes is open.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"B\"\nacceptance=[\"src/b.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"u2\"\ndelivers=[\"R-2\"]\nfiles=[\"src/b.txt\"]\ncheckpoint=\"K-2\"\ndepends_on=[\"U-1\"]\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"c4\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"c5\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-2\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n"),
        ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k2\"\nafter=[\"U-2\"]\nproves=[\"R-2\"]\nchecks=[\"X-4\",\"X-3\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn start(fx: &Fx) {
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on",
        "--scope",
        "default",
    ]);
    fx.human("Yes, set the checkpoint review that way.");
    fx.must(&[
        "checks",
        "set",
        "--quote",
        "set the checkpoint review that way",
    ]);
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

fn brief(fx: &Fx) -> Out {
    fx.bf(&["brief", "K-1", "--role", "reviewer", "--json"])
}

fn report(fx: &Fx, body: Value) -> Out {
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

fn areas() -> Value {
    json!([{"area": "the close rule", "how": "read"}])
}

fn high() -> Value {
    json!({"title": "the answer is wrong", "severity": "high", "where": "src/a.txt:1", "action": "make it right"})
}

fn directives(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["action"].as_str().unwrap_or("").to_string())
        .collect()
}

fn fates(fx: &Fx, round: u64) -> Vec<String> {
    let st = fx.must(&["status", "--json"]).json();
    let r = st["checkpoint_reviews"][0]["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["round"].as_u64() == Some(round))
        .unwrap()
        .clone();
    r["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["fate"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_finding_filed_again_is_open_on_its_own_and_the_close_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "changed by the fix\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fix"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
    assert!(
        fates(&fx, 1)[0].starts_with("fixed by"),
        "{:?}",
        fates(&fx, 1)
    );
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    let f = fates(&fx, 2);
    eprintln!("round 2 fate: {f:?}");
    assert_eq!(
        f,
        vec!["open".to_string()],
        "the finding filed again is open, not fixed"
    );
    assert!(fates(&fx, 1)[0].starts_with("fixed by"));
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    eprintln!("close: {}", c.stderr);
    assert_eq!(c.code, 2, "{}", c.all());
    assert!(c.stderr.contains("a fix of it is open"), "{}", c.stderr);
    assert!(c.stderr.contains("K-1-fix-2"), "{}", c.stderr);
    assert!(c.stderr.contains("bf next"), "{}", c.stderr);
}

#[test]
fn the_close_is_refused_while_a_fix_the_human_picked_is_open() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    fx.must_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let mut low = high();
    low["severity"] = json!("low");
    let o = report(&fx, json!({"areas": areas(), "findings": [low]}));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1"]);
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    eprintln!("close: {}", c.stderr);
    assert_eq!(c.code, 2, "{}", c.all());
    assert!(c.stderr.contains("a fix of it is open"), "{}", c.stderr);
    assert!(c.stderr.contains("K-1-fix-1"), "{}", c.stderr);
    assert!(c.stderr.contains("bf next"), "{}", c.stderr);
    let st = fx.must(&["status", "--json"]).json();
    assert!(
        !st["checkpoint_fix"]["open"].is_null(),
        "the fix is still open"
    );
}

#[test]
fn a_fix_aborted_leaves_its_finding_open_and_the_human_decides() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    report(&fx, json!({"areas": areas(), "findings": [high()]}));
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    assert_eq!(fates(&fx, 1), vec!["open".to_string()]);
    let d = directives(&fx);
    eprintln!("next: {d:?}");
    assert!(d.contains(&"decide-checkpoint-fix".to_string()), "{d:?}");
    assert!(!d.contains(&"review-checkpoint".to_string()), "{d:?}");
    // A review at that content that files nothing does not let the checkpoint close.
    brief(&fx);
    fx.must_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let o = report(&fx, json!({"areas": areas(), "findings": []}));
    assert_eq!(o.code, 0, "{}", o.all());
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    eprintln!("close: {}", c.stderr);
    assert_eq!(c.code, 2, "{}", c.all());
    assert!(c.stderr.contains("checkpoint review"), "{}", c.stderr);
    // The human declines the finding in their own words, and the close goes through.
    fx.human("Decline it: the answer is right for our use.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--decline",
        "r1.1",
        "--reason",
        "the answer is right for our use",
        "--quote",
        "Decline it: the answer is right for our use.",
    ]);
    assert!(
        fates(&fx, 1)[0].starts_with("declined"),
        "{:?}",
        fates(&fx, 1)
    );
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(c.code, 0, "{}", c.all());
}

#[test]
fn the_repeat_review_brief_says_an_open_finding_stays_open() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    report(&fx, json!({"areas": areas(), "findings": [high()]}));
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    brief(&fx);
    let text = fs::read_to_string(fx.project().briefs_dir().join("K-1-reviewer.md")).unwrap();
    assert!(
        text.contains("stays open and holds the close until a fix opened for it passes its review or the human settles it"),
        "{text}"
    );
    assert!(text.contains("fails when it runs no test"), "{text}");
}

#[test]
fn a_release_then_an_aborted_fix_keeps_both_findings_open_and_the_human_decides() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 2");
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    brief(&fx);
    let mut medium = high();
    medium["title"] = json!("the second answer is wrong");
    medium["severity"] = json!("medium");
    let o = report(&fx, json!({"areas": areas(), "findings": [medium]}));
    assert_eq!(o.code, 0, "{}", o.all());
    let d = directives(&fx);
    assert!(d.contains(&"decide-checkpoint-review".to_string()), "{d:?}");
    fx.human("Release it, go on with the fix.");
    fx.must(&[
        "checkpoint",
        "review",
        "K-1",
        "--release",
        "--quote",
        "Release it",
    ]);
    let d = directives(&fx);
    assert!(d.contains(&"fix-checkpoint".to_string()), "{d:?}");
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    assert_eq!(fates(&fx, 1), vec!["open".to_string()]);
    assert_eq!(fates(&fx, 2), vec!["open".to_string()]);
    let d = directives(&fx);
    eprintln!("next: {d:?}");
    assert!(d.contains(&"decide-checkpoint-fix".to_string()), "{d:?}");
    assert!(!d.contains(&"review-checkpoint".to_string()), "{d:?}");
    // A review at that content that files nothing does not let the checkpoint close.
    brief(&fx);
    fx.must_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let o = report(&fx, json!({"areas": areas(), "findings": []}));
    assert_eq!(o.code, 0, "{}", o.all());
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(c.code, 2, "{}", c.all());
    assert_eq!(fates(&fx, 1), vec!["open".to_string()]);
    assert_eq!(fates(&fx, 2), vec!["open".to_string()]);
}

fn next_directive(fx: &Fx, action: &str) -> Option<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == action)
        .cloned()
}

// R-91, D-65, TH-41: only the human's words take a parked checkpoint out of parking.
#[test]
fn a_parked_checkpoint_refuses_the_pick_to_fix_with_and_without_words() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(directives(&fx).contains(&"decide-checkpoint-review".to_string()));
    let a = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1"]);
    eprintln!("without words: {}", a.stderr);
    assert_eq!(a.code, 2, "{}", a.all());
    assert!(a.stderr.contains("--release"), "{}", a.stderr);
    fx.human("Fix it please.");
    let b = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1",
        "--quote",
        "Fix it please.",
    ]);
    eprintln!("with words: {}", b.stderr);
    assert_eq!(b.code, 2, "{}", b.all());
    assert!(b.stderr.contains("--release"), "{}", b.stderr);
    assert!(directives(&fx).contains(&"decide-checkpoint-review".to_string()));
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(c.code, 2, "{}", c.all());
}

#[test]
fn a_high_finding_is_picked_to_be_fixed_only_in_the_humans_words_taken_once() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    report(&fx, json!({"areas": areas(), "findings": [high()]}));
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let d = next_directive(&fx, "decide-checkpoint-fix").expect("the human is asked");
    assert!(
        d["commands"][0].as_str().unwrap().contains("--quote"),
        "{d}"
    );
    assert!(d["note"].as_str().unwrap().contains("aborted"), "{d}");
    let a = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1"]);
    eprintln!("without words: {}", a.stderr);
    assert_eq!(a.code, 2, "{}", a.all());
    assert!(a.stderr.contains("--quote"), "{}", a.stderr);
    fx.human("Yes, open the fix again.");
    let q = [
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1",
        "--quote",
        "open the fix again",
    ];
    fx.must(&q);
    assert!(directives(&fx).contains(&"fix-checkpoint".to_string()));
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let again = fx.bf(&q);
    eprintln!("same words again: {}", again.stderr);
    assert_eq!(again.code, 2, "{}", again.all());
}

#[test]
fn after_an_abort_and_a_sync_the_human_is_asked_and_the_close_names_the_finding() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    report(&fx, json!({"areas": areas(), "findings": [high()]}));
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    fs::create_dir_all(fx.root.join("docs")).unwrap();
    fs::write(fx.root.join("docs/note.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "docs/note.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    fx.must(&["checkpoint", "sync", "K-1", "--quote", "bring main in"]);
    let d = directives(&fx);
    eprintln!("next: {d:?}");
    assert!(d.contains(&"decide-checkpoint-fix".to_string()), "{d:?}");
    assert!(!d.contains(&"review-checkpoint".to_string()), "{d:?}");
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    eprintln!("close: {}", c.stderr);
    assert_eq!(c.code, 2, "{}", c.all());
    assert!(c.stderr.contains("r1.1"), "{}", c.stderr);
    assert!(c.stderr.contains("high"), "{}", c.stderr);
    assert!(c.stderr.contains("the answer is wrong"), "{}", c.stderr);
    assert!(c.stderr.contains("bf next"), "{}", c.stderr);
    assert!(
        !c.stderr.contains("has not run on this content"),
        "{}",
        c.stderr
    );
}

#[test]
fn the_reviewers_brief_gives_the_nextest_example_probe_command() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    let text = fs::read_to_string(fx.project().briefs_dir().join("K-1-reviewer.md")).unwrap();
    assert!(text.contains("cargo nextest run -E test(=p1)"), "{text}");
    assert!(!text.contains("cargo test p1"), "{text}");
    assert!(text.contains("fails when it selects no test"), "{text}");
}
