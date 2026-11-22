// R-104, K2R7-7: each decision on a checkpoint's findings (the finding settle, the fix pick, the review
// release and the fix release) looks at the holds itself before it judges, and the replay of a review
// release sets aside only the fix the release opened, never one that carries a human turn of its own.
//
// The `events::append` hook already logs every hold a test creates through an event, so a decision that
// lost its own look at the holds would pass those tests. Here the hold starts with no event: a check is
// marked unreachable at the checkpoint's content, the content is changed by hand and `bf next` logs the
// hold's end, a sentence is written, and the content is put back by hand, so the mark stands again with
// nothing logged. Only the decision's own look at the holds sees it.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const OLD0: &str = "Some early words, before the plan.";
const WORDS: &str = "Fine, I have thought about it: go on with it.";
const WORDS2: &str = "And once more, please do go on with it.";
const PASS: &str = "test -f src/a.txt";

fn plan(fx: &Fx) {
    let x2 = format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{PASS}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n".into()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".into()),
        ("check", x2),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".into()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n".into()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn start(fx: &Fx) {
    fx.must(&["checks", "propose", "checkpoint-review=on", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it", "--scope", "default"]);
    fx.human("Yes, set the checkpoint review that way.");
    fx.must(&["checks", "set", "--quote", "set the checkpoint review that way"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

fn setup(parking_reviews: bool) -> Fx {
    let fx = Fx::new();
    if parking_reviews {
        fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    }
    fx.human(OLD0);
    plan(&fx);
    start(&fx);
    fx
}

fn high() -> Value {
    json!({"title": "the answer is wrong", "severity": "high", "where": "src/a.txt:1", "action": "make it right"})
}

fn report_high(fx: &Fx) {
    fx.bf(&["brief", "K-1", "--role", "reviewer", "--json"]);
    let body = json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": [high()]});
    let o = fx.bf_stdin(&fx.root, &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"], &body.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

fn kinds(fx: &Fx) -> Vec<String> {
    bf::events::load_log(&fx.project()).unwrap().events.iter().map(|e| e["kind"].as_str().unwrap_or("").to_string()).collect()
}

fn count(fx: &Fx, kind: &str) -> usize {
    kinds(fx).iter().filter(|k| *k == kind).count()
}

fn ignored(fx: &Fx) -> Vec<String> {
    let s = fx.must(&["status", "--json"]).json();
    s["ignored_events"].as_array().unwrap().iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()
}

fn fates(fx: &Fx) -> String {
    let st = fx.must(&["status", "--json"]).json();
    let mut out = Vec::new();
    for r in st["checkpoint_reviews"][0]["rounds"].as_array().into_iter().flatten() {
        for o in r["outcomes"].as_array().into_iter().flatten() {
            out.push(format!("{}={}", o["id"].as_str().unwrap_or(""), o["fate"].as_str().unwrap_or("")));
        }
    }
    out.join("; ")
}

fn fix_phase(fx: &Fx) -> Value {
    fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"]["phase"].clone()
}

fn abort_fix(fx: &Fx) {
    let o = fx.bf(&["checkpoint", "fix", "K-1", "--abort"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn head(dir: &std::path::Path) -> String {
    git(dir, &["rev-parse", "HEAD"]).trim().to_string()
}

/// A hold that starts with no event. A check is marked unreachable at the checkpoint's content (the hold's
/// start is logged), the content is changed by hand and `bf next` logs the hold's end, `words` are written,
/// and the content is put back by hand: the mark stands again, and nothing is logged for it.
fn hold_with_no_event(fx: &Fx, words: &str) {
    let k = fx.work("K-1");
    let marked = head(&k);
    let o = fx.bf_stdin(&k, &["evidence", "fail", "X-2", "--reason", "the service is down"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    let holds = count(fx, "checkpoint.hold");
    assert!(holds >= 1, "the mark's start is logged");
    fs::write(k.join("src/a.txt"), "changed by hand\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "by hand"]);
    fx.must(&["next", "--json"]);
    assert_eq!(count(fx, "checkpoint.hold"), holds + 1, "the hold's end is logged");
    fx.human(words);
    let before = kinds(fx).len();
    git(&k, &["reset", "--hard", "-q", &marked]);
    assert_eq!(kinds(fx).len(), before, "putting the content back logs nothing");
}

/// The decision is refused (exit 2), names `bf next`, and the hold it saw is now logged.
fn refused(tag: &str, fx: &Fx, o: &Out, holds_before: usize) {
    assert_eq!(o.code, 2, "{tag}: {}", o.all());
    assert!(o.all().contains("bf next"), "{tag}: {}", o.all());
    assert_eq!(count(fx, "checkpoint.hold"), holds_before + 1, "{tag}: the hold the decision saw is logged");
}

#[test]
fn a_finding_settle_on_words_written_before_a_hold_with_no_event_is_refused() {
    for (tag, args) in [
        ("accept", vec!["--accept", "r1.1", "--quote", WORDS]),
        ("decline", vec!["--decline", "r1.1", "--reason", "not a defect", "--quote", WORDS]),
    ] {
        let fx = setup(false);
        report_high(&fx);
        abort_fix(&fx);
        hold_with_no_event(&fx, WORDS);
        let holds = count(&fx, "checkpoint.hold");
        let settles = count(&fx, "checkpoint.finding.settle");
        let mut a = vec!["checkpoint", "finding", "K-1"];
        a.extend(args);
        let o = fx.bf(&a);
        refused(tag, &fx, &o, holds);
        assert_eq!(count(&fx, "checkpoint.finding.settle"), settles, "{tag}");
        assert_eq!(fates(&fx), "r1.1=open", "{tag}: {}", fates(&fx));
        assert!(ignored(&fx).is_empty(), "{tag}: {:?}", ignored(&fx));
    }
}

#[test]
fn a_fix_pick_on_words_written_before_a_hold_with_no_event_is_refused() {
    let fx = setup(false);
    report_high(&fx);
    abort_fix(&fx);
    hold_with_no_event(&fx, WORDS);
    let holds = count(&fx, "checkpoint.hold");
    let opens = count(&fx, "checkpoint.fix.open");
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1", "--quote", WORDS]);
    refused("pick", &fx, &o, holds);
    assert_eq!(count(&fx, "checkpoint.fix.open"), opens);
    assert_eq!(fix_phase(&fx), Value::Null, "no fix opened");
    assert_eq!(fates(&fx), "r1.1=open", "{}", fates(&fx));
}

#[test]
fn a_review_release_on_words_written_before_a_hold_with_no_event_is_refused() {
    let fx = setup(true);
    report_high(&fx);
    assert_eq!(count(&fx, "checkpoint.fix.open"), 0, "the review is parked");
    hold_with_no_event(&fx, WORDS);
    let holds = count(&fx, "checkpoint.hold");
    let o = fx.bf(&["checkpoint", "review", "K-1", "--release", "--quote", WORDS]);
    refused("review release", &fx, &o, holds);
    assert_eq!(count(&fx, "checkpoint.review.release"), 0);
    assert_eq!(count(&fx, "checkpoint.fix.open"), 0, "the parked review opened no fix");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_fix_release_on_words_written_before_a_hold_with_no_event_is_refused() {
    let fx = setup(false);
    report_high(&fx);
    let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "builder", "--result", "blocked", "--note", "stuck"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(fix_phase(&fx), json!("parked"));
    hold_with_no_event(&fx, WORDS);
    let holds = count(&fx, "checkpoint.hold");
    let o = fx.bf(&["checkpoint", "fix", "K-1", "--release", "--quote", WORDS]);
    refused("fix release", &fx, &o, holds);
    assert_eq!(count(&fx, "checkpoint.fix.release"), 0);
    assert_eq!(fix_phase(&fx), json!("parked"), "the fix stays parked");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

// The replay of a review release: the fix the release itself opened (the event just before it, with no human
// turn of its own) is set aside; a fix opened by a pick, with its own human turn, is not, so the release
// written right after it is ignored and the fix stays open.
#[test]
fn a_review_release_right_after_a_fix_with_its_own_human_turn_is_ignored() {
    let fx = setup(true);
    report_high(&fx);
    assert_eq!(count(&fx, "checkpoint.fix.open"), 0, "the review is parked");
    fx.human(WORDS);
    fx.human(WORDS2);
    let log = bf::events::load_log(&fx.project()).unwrap();
    let turn_of = |text: &str| -> u64 {
        let st = bf::events::replay_log(&log);
        st.turns.iter().find(|t| t.text == text).unwrap().seq
    };
    let (t1, t2) = (turn_of(WORDS), turn_of(WORDS2));
    let round = log.events.iter().find(|e| e["kind"] == "checkpoint.review.done" && e["round"] == 1).unwrap()["findings"].clone();
    let open = json!({
        "checkpoint": "K-1", "source": "review", "opened_by": "pick", "failed": [], "findings": round,
        "finding_ids": [{"round": 1, "index": 1}], "before": [], "content": "own",
        "human": {"turn": t1, "quote": WORDS}, "wave": 0,
    });
    bf::events::append(&fx.project(), "checkpoint.fix.open", open).unwrap();
    assert!(ignored(&fx).is_empty(), "the pick's own opening replays: {:?}", ignored(&fx));
    assert_eq!(fix_phase(&fx), json!("fix"));
    bf::events::append(
        &fx.project(),
        "checkpoint.review.release",
        json!({"checkpoint": "K-1", "after_round": 1, "turn": t2, "quote": WORDS2}),
    )
    .unwrap();
    let notes = ignored(&fx);
    assert_eq!(notes.len(), 1, "the release is listed as ignored: {notes:?}");
    let last = kinds(&fx).len();
    assert!(notes[0].starts_with(&format!("event {last}: ")), "it is the release: {notes:?}");
    assert_eq!(fix_phase(&fx), json!("fix"), "the pick's fix is not set aside");
}
