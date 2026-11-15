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
    // No review starts while the human is asked, so none can let the checkpoint close.
    let b = brief(&fx);
    assert_eq!(b.code, 2, "{}", b.all());
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
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(c.code, 0, "{}", c.all());
}

#[test]
fn the_repeat_review_brief_after_a_passed_fix_carries_the_fixed_sentence_on_open_findings() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    report(&fx, json!({"areas": areas(), "findings": [high()]}));
    fx.pass_checkpoint_fix("changed by the fix\n");
    brief(&fx);
    let text = fs::read_to_string(fx.project().briefs_dir().join("K-1-reviewer.md")).unwrap();
    assert!(
        text.contains("stays open and holds the close until a fix opened for it passes its review or the human settles it"),
        "{text}"
    );
    assert!(text.contains("fails when it runs no test"), "{text}");
}

#[test]
fn after_a_release_an_aborted_fix_leaves_the_released_finding_open_and_the_human_decides() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 2");
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.pass_checkpoint_fix("changed by the fix\n");
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
    // The first finding was fixed before the second review; the second is open again after the abort.
    assert!(
        fates(&fx, 1)[0].starts_with("fixed by"),
        "{:?}",
        fates(&fx, 1)
    );
    assert_eq!(fates(&fx, 2), vec!["open".to_string()]);
    let d = directives(&fx);
    eprintln!("next: {d:?}");
    assert!(d.contains(&"decide-checkpoint-fix".to_string()), "{d:?}");
    assert!(!d.contains(&"review-checkpoint".to_string()), "{d:?}");
    // No review starts while the human is asked, so none can let the checkpoint close.
    assert_eq!(brief(&fx).code, 2);
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(c.code, 2, "{}", c.all());
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
    assert!(again.stderr.contains("already used"), "{}", again.stderr);
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

// R-92, D-66, TH-42: a decision on a finding or on parking takes only a human message written after the
// event that put it to the human.
const BEFORE_ONE: &str = "Carry on with the build, thanks.";
const BEFORE_TWO: &str = "Looks good to me, keep going.";

/// Two sentences written before the plan; round 1 files a high finding and its fix is aborted.
fn aborted_after_old_words() -> Fx {
    let fx = Fx::new();
    fx.human(BEFORE_ONE);
    fx.human(BEFORE_TWO);
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    fx
}

fn pick_args<'a>(how: &'a str, quote: &'a str) -> Vec<&'a str> {
    let mut v = vec!["checkpoint", "finding", "K-1"];
    match how {
        "fix" => v.extend(["--fix-findings", "r1.1"]),
        "accept" => v.extend(["--accept", "r1.1"]),
        _ => v.extend(["--decline", "r1.1", "--reason", "it is how it is meant to be"]),
    }
    v.extend(["--quote", quote]);
    v
}

#[test]
fn words_written_before_the_finding_was_put_to_the_human_decide_nothing() {
    for how in ["fix", "accept", "decline"] {
        let fx = aborted_after_old_words();
        for old in [BEFORE_ONE, BEFORE_TWO] {
            let o = fx.bf(&pick_args(how, old));
            eprintln!("{how} with old words: {}", o.stderr);
            assert_eq!(o.code, 2, "{how}: {}", o.all());
            assert!(o.stderr.contains("written before"), "{how}: {}", o.stderr);
            assert!(o.stderr.contains("new message"), "{how}: {}", o.stderr);
        }
        assert!(directives(&fx).contains(&"decide-checkpoint-fix".to_string()));
        // A message written after the abort decides.
        fx.human("Yes, that is my decision on it.");
        let o = fx.bf(&pick_args(how, "that is my decision on it"));
        assert_eq!(o.code, 0, "{how}: {}", o.all());
        if how == "fix" {
            println!("the pick prints: {}", o.stdout.trim());
            assert!(o.stdout.contains("a fix opened for r1.1"), "{}", o.stdout);
        }
    }
}

#[test]
fn a_release_takes_only_words_written_after_the_report_that_parked_the_checkpoint() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    fx.human(BEFORE_ONE);
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(directives(&fx).contains(&"decide-checkpoint-review".to_string()));
    let o = fx.bf(&[
        "checkpoint",
        "review",
        "K-1",
        "--release",
        "--quote",
        BEFORE_ONE,
    ]);
    eprintln!("release with old words: {}", o.stderr);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("written before"), "{}", o.stderr);
    assert!(o.stderr.contains("new message"), "{}", o.stderr);
    fx.human("Release it, go on with the fix.");
    fx.must(&[
        "checkpoint",
        "review",
        "K-1",
        "--release",
        "--quote",
        "Release it",
    ]);
    assert!(directives(&fx).contains(&"fix-checkpoint".to_string()));
}

#[test]
fn a_settle_event_that_cites_old_words_is_ignored_on_replay() {
    let fx = aborted_after_old_words();
    let p = fx.project();
    let st = bf::events::replay_log(&bf::events::load_log(&p).unwrap());
    let old = st.turns.iter().find(|t| t.text == BEFORE_TWO).unwrap().seq;
    bf::events::append(
        &p,
        "checkpoint.finding.settle",
        json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "accepted",
               "reason": "", "turn": old, "quote": BEFORE_TWO}),
    )
    .unwrap();
    let s = fx.must(&["status", "--json"]).json();
    eprintln!("ignored: {}", s["ignored_events"]);
    assert_eq!(s["ignored_events"].as_array().unwrap().len(), 1, "{s}");
    fx.must(&["log", "--verify"]);
    assert!(directives(&fx).contains(&"decide-checkpoint-fix".to_string()));
    assert_eq!(fates(&fx, 1), vec!["open".to_string()]);
}

#[test]
fn no_review_starts_while_the_human_is_asked_what_becomes_of_an_open_finding() {
    let fx = aborted_after_old_words();
    let before = directives(&fx);
    let o = fx.bf(&["brief", "K-1", "--role", "reviewer"]);
    eprintln!("brief while the human is asked: {}", o.stderr);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("decide-checkpoint-fix"), "{}", o.stderr);
    assert!(o.stderr.contains("--fix-findings"), "{}", o.stderr);
    assert!(o.stderr.contains("--accept"), "{}", o.stderr);
    assert!(o.stderr.contains("--decline"), "{}", o.stderr);
    assert!(o.stderr.contains("bf next"), "{}", o.stderr);
    assert_eq!(directives(&fx), before);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(
        st["checkpoint_reviews"][0]["rounds"].as_array().unwrap().len(),
        1,
        "no round opened: {st}"
    );
}

#[test]
fn the_usage_and_the_close_name_the_words_and_the_open_finding() {
    let fx = aborted_after_old_words();
    let o = fx.bf(&["checkpoint", "finding", "K-1"]);
    eprintln!("usage: {}", o.stderr);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("--fix-findings r1.2,r1.3 --quote"), "{}", o.stderr);
    assert!(o.stderr.contains("critical, high or medium"), "{}", o.stderr);
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    eprintln!("close: {}", c.stderr);
    assert_eq!(c.code, 2, "{}", c.all());
    let because = c.stderr.lines().find(|l| l.starts_with("because:")).unwrap_or("");
    assert!(because.contains("r1.1"), "{}", c.stderr);
    assert!(!because.contains("ID(s) have no proof"), "{}", c.stderr);
    // With no message after the abort the refusal lists its first step once.
    let o = fx.bf(&pick_args("fix", "an unwritten sentence"));
    eprintln!("no such words: {}", o.stderr);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(o.stderr.matches("Ask the human in chat").count(), 1, "{}", o.stderr);
}

// R-96, D-70, TH-46: while a fix opened for a finding is at work the human is not asked about it, so words
// written during the fix end nothing; a parked fix bounds the words by its park.
const DURING: &str = "Looks fine so far, keep going.";

fn fix_at_work() -> Fx {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(directives(&fx).contains(&"fix-checkpoint".to_string()));
    fx
}

fn settle_args<'a>(how: &'a str, quote: &'a str) -> Vec<&'a str> {
    let mut v = vec!["checkpoint", "finding", "K-1"];
    if how == "accept" {
        v.extend(["--accept", "r1.1"]);
    } else {
        v.extend(["--decline", "r1.1", "--reason", "it is how it is meant to be"]);
    }
    v.extend(["--quote", quote]);
    v
}

#[test]
fn words_written_while_the_fix_runs_neither_accept_nor_decline_its_finding() {
    for how in ["accept", "decline"] {
        let fx = fix_at_work();
        fx.human(DURING);
        let o = fx.bf(&settle_args(how, DURING));
        eprintln!("{how} while the fix runs: {}", o.stderr);
        assert_eq!(o.code, 2, "{how}: {}", o.all());
        assert!(o.stderr.contains("bf checkpoint fix K-1 --abort"), "{how}: {}", o.stderr);
        assert!(o.stderr.contains("bf next"), "{how}: {}", o.stderr);
        assert_eq!(fates(&fx, 1), vec!["open".to_string()]);
        // The abort needs no words; the words written during the fix still decide nothing after it.
        fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
        let o = fx.bf(&settle_args(how, DURING));
        eprintln!("{how} on the same words after the abort: {}", o.stderr);
        assert_eq!(o.code, 2, "{how}: {}", o.all());
        assert!(o.stderr.contains("written before"), "{how}: {}", o.stderr);
        // A message written after the abort decides.
        fx.human("Yes, that is my decision on it.");
        let o = fx.bf(&settle_args(how, "that is my decision on it"));
        println!("{how} after the abort: {}", o.stdout.trim());
        assert_eq!(o.code, 0, "{how}: {}", o.all());
        assert!(fates(&fx, 1)[0].starts_with(if how == "accept" { "accepted" } else { "declined" }));
    }
}

#[test]
fn a_parked_fix_takes_only_words_written_after_it_parked_for_its_findings() {
    for how in ["accept", "decline"] {
        let fx = fix_at_work();
        fx.human(DURING);
        let o = fx.bf_in(
            &fx.work("K-1"),
            &["report", "K-1", "--role", "builder", "--result", "blocked", "--note", "stuck"],
        );
        assert_eq!(o.code, 0, "{}", o.all());
        assert!(directives(&fx).contains(&"decide-checkpoint-fix".to_string()));
        let o = fx.bf(&settle_args(how, DURING));
        eprintln!("{how} on words written before the park: {}", o.stderr);
        assert_eq!(o.code, 2, "{how}: {}", o.all());
        assert!(o.stderr.contains("written before"), "{how}: {}", o.stderr);
        fx.human("Yes, that is my decision on it.");
        let o = fx.bf(&settle_args(how, "that is my decision on it"));
        println!("{how} after the park: {}", o.stdout.trim());
        assert_eq!(o.code, 0, "{how}: {}", o.all());
    }
}

#[test]
fn a_settle_event_appended_while_the_fix_runs_is_ignored_on_replay() {
    let fx = fix_at_work();
    fx.human(DURING);
    let p = fx.project();
    let st = bf::events::replay_log(&bf::events::load_log(&p).unwrap());
    let t = st.turns.iter().find(|t| t.text == DURING).unwrap().seq;
    bf::events::append(
        &p,
        "checkpoint.finding.settle",
        json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "accepted",
               "reason": "", "turn": t, "quote": DURING}),
    )
    .unwrap();
    let s = fx.must(&["status", "--json"]).json();
    eprintln!("ignored: {}", s["ignored_events"]);
    assert_eq!(s["ignored_events"].as_array().unwrap().len(), 1, "{s}");
    fx.must(&["log", "--verify"]);
    assert_eq!(fates(&fx, 1), vec!["open".to_string()]);
    assert!(directives(&fx).contains(&"fix-checkpoint".to_string()));
}

fn low() -> Value {
    json!({"title": "a small thing", "severity": "low", "where": "src/a.txt:1", "action": "tidy"})
}

fn round_events(fx: &Fx, round: u32, content: &str, findings: Vec<Value>) {
    let p = fx.project();
    bf::events::append(
        &p,
        "checkpoint.review.open",
        json!({"checkpoint": "K-1", "round": round, "content": content, "heads": [], "covered": "the whole checkpoint, cold"}),
    )
    .unwrap();
    bf::events::append(
        &p,
        "checkpoint.review.done",
        json!({"checkpoint": "K-1", "round": round, "content": content,
               "result": if findings.is_empty() { "clean" } else { "findings" },
               "findings": findings, "areas": areas(), "probes": [], "note": ""}),
    )
    .unwrap();
}

fn content_of_round_one(fx: &Fx) -> String {
    bf::ckreview::rounds(&fx.project(), "K-1").unwrap()[0].content.clone()
}

#[test]
fn an_older_open_high_finding_holds_a_later_clean_review_of_the_same_content() {
    let fx = aborted_after_old_words();
    let content = content_of_round_one(&fx);
    round_events(&fx, 2, &content, vec![]);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let d = directives(&fx);
    eprintln!("next: {d:?}");
    assert!(d.contains(&"decide-checkpoint-fix".to_string()), "{d:?}");
    let c = fx.bf(&["checkpoint", "close", "K-1"]);
    eprintln!("close: {}", c.stderr);
    assert_eq!(c.code, 2, "{}", c.all());
    let because = c.stderr.lines().find(|l| l.starts_with("because:")).unwrap_or("");
    assert!(because.contains("r1.1"), "{}", c.stderr);
}

#[test]
fn a_release_sends_the_open_findings_of_every_round_to_the_fix() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 2");
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let content = content_of_round_one(&fx);
    let mut second = high();
    second["title"] = json!("the second answer is wrong");
    round_events(&fx, 2, &content, vec![second]);
    assert!(directives(&fx).contains(&"decide-checkpoint-review".to_string()));
    fx.human("Release it, go on with the fix.");
    let o = fx.bf(&["checkpoint", "review", "K-1", "--release", "--quote", "Release it"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let d = next_directive(&fx, "fix-checkpoint").expect("the findings go to a fix");
    let titles: Vec<String> = d["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["title"].as_str().unwrap().to_string())
        .collect();
    eprintln!("the fix holds: {titles:?}");
    assert_eq!(titles.len(), 2, "{d}");
    assert!(titles.contains(&"the answer is wrong".to_string()), "{d}");
    assert!(titles.contains(&"the second answer is wrong".to_string()), "{d}");
}

#[test]
fn a_fix_open_event_that_cites_words_written_before_its_bound_is_ignored_on_replay() {
    let fx = aborted_after_old_words();
    let p = fx.project();
    let st = bf::events::replay_log(&bf::events::load_log(&p).unwrap());
    let old = st.turns.iter().find(|t| t.text == BEFORE_TWO).unwrap().seq;
    bf::events::append(
        &p,
        "checkpoint.fix.open",
        json!({"checkpoint": "K-1", "source": "review", "findings": [high()],
               "finding_ids": [{"round": 1, "index": 1}], "before": [],
               "human": {"turn": old, "quote": BEFORE_TWO}}),
    )
    .unwrap();
    let s = fx.must(&["status", "--json"]).json();
    eprintln!("ignored: {}", s["ignored_events"]);
    assert_eq!(s["ignored_events"].as_array().unwrap().len(), 1, "{s}");
    fx.must(&["log", "--verify"]);
    assert!(directives(&fx).contains(&"decide-checkpoint-fix".to_string()));
}

#[test]
fn a_review_release_event_that_cites_words_written_before_the_report_is_ignored_on_replay() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    fx.human(BEFORE_ONE);
    plan(&fx);
    start(&fx);
    brief(&fx);
    let o = report(&fx, json!({"areas": areas(), "findings": [high()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    let p = fx.project();
    let st = bf::events::replay_log(&bf::events::load_log(&p).unwrap());
    let old = st.turns.iter().find(|t| t.text == BEFORE_ONE).unwrap().seq;
    bf::events::append(
        &p,
        "checkpoint.review.release",
        json!({"checkpoint": "K-1", "after_round": 1, "turn": old, "quote": BEFORE_ONE}),
    )
    .unwrap();
    let s = fx.must(&["status", "--json"]).json();
    eprintln!("ignored: {}", s["ignored_events"]);
    assert_eq!(s["ignored_events"].as_array().unwrap().len(), 1, "{s}");
    fx.must(&["log", "--verify"]);
    assert!(directives(&fx).contains(&"decide-checkpoint-review".to_string()));
}

#[test]
fn a_repeat_review_brief_gives_the_outcome_open_for_a_low_finding_left_open() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx);
    brief(&fx);
    fx.must_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let o = report(&fx, json!({"areas": areas(), "findings": [low()]}));
    assert_eq!(o.code, 0, "{}", o.all());
    // The content changes, so a repeat review starts while the low finding is still open.
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "changed after the first review\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "change"]);
    let o = brief(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
    let text = fs::read_to_string(fx.project().briefs_dir().join("K-1-reviewer.md")).unwrap();
    let line = text.lines().find(|l| l.contains("a small thing")).unwrap_or("");
    eprintln!("brief line: {line}");
    assert!(line.contains("Outcome: open"), "{text}");
}
