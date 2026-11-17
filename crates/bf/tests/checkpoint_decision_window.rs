// R-100, D-75: no human decision on a checkpoint finding, and no release of a parked checkpoint review, is
// taken while any checkpoint fix is at work; each takes only words written after the latest end, abort or
// park of a checkpoint fix. The replay applies the same rule through one function and names each cause.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const OLD0: &str = "Some early words, before the plan.";
const OLD: &str = "Looks good to me, keep going.";
const PASS: &str = "test -f src/a.txt";
const GREP: &str = "grep -q ok src/a.txt";

fn plan(fx: &Fx, x2: &str) {
    let x2 = format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{x2}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n".into()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".into()),
        ("check", x2),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".into()),
        ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"B\"\nacceptance=[\"src/b.txt exists\"]\n".into()),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"u2\"\ndelivers=[\"R-2\"]\nfiles=[\"src/b.txt\"]\ncheckpoint=\"K-2\"\ndepends_on=[\"U-1\"]\n".into()),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"c4\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"checkpoint\"\n".into()),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"c5\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n".into()),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-2\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".into()),
        ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k2\"\nafter=[\"U-2\"]\nproves=[\"R-2\"]\nchecks=[\"X-4\",\"X-3\"]\n".into()),
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

fn setup(x2: &str) -> Fx {
    let fx = Fx::new();
    fx.human(OLD0);
    plan(&fx, x2);
    start(&fx);
    fx
}

fn brief(fx: &Fx) -> Out {
    fx.bf(&["brief", "K-1", "--role", "reviewer", "--json"])
}

fn f(sev: &str, title: &str) -> Value {
    json!({"title": title, "severity": sev, "where": "src/a.txt:1", "action": "make it right"})
}

fn report(fx: &Fx, findings: Value) -> Out {
    brief(fx);
    let body = json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": findings});
    let o = fx.bf_stdin(&fx.root, &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"], &body.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    o
}

fn say(tag: &str, o: &Out) {
    let one = |s: &str| s.lines().take(6).collect::<Vec<_>>().join(" | ");
    eprintln!("{tag}: exit {} | {} | {}", o.code, one(&o.stdout), one(&o.stderr));
}

fn actions(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"].as_array().unwrap().iter()
        .map(|d| d["action"].as_str().unwrap_or("").to_string()).collect()
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

fn fix_built(fx: &Fx, text: &str) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), text).unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fix"]);
    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "builder", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn fix_clean(fx: &Fx) {
    let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "reviewer", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn accept(fx: &Fx, ids: &str, quote: &str) -> Out {
    fx.bf(&["checkpoint", "finding", "K-1", "--accept", ids, "--quote", quote])
}

fn abort(fx: &Fx) -> Out {
    let o = fx.bf(&["checkpoint", "fix", "K-1", "--abort"]);
    say("abort with no words", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    o
}

fn refused_at_work(tag: &str, o: &Out) {
    say(tag, o);
    assert_eq!(o.code, 2, "{tag}: {}", o.all());
    let t = o.all();
    assert!(t.contains("checkpoint fix is at work"), "{tag}: {t}");
    assert!(t.contains("bf next"), "{tag}: {t}");
    assert!(t.contains("bf checkpoint fix K-1 --abort"), "{tag}: {t}");
}

fn refused_before(tag: &str, o: &Out) {
    say(tag, o);
    assert_eq!(o.code, 2, "{tag}: {}", o.all());
    assert!(o.all().contains("written before"), "{tag}: {}", o.all());
}

fn turn_seq(fx: &Fx, text: &str) -> u64 {
    let st = bf::events::replay_log(&bf::events::load_log(&fx.project()).unwrap());
    st.turns.iter().find(|t| t.text == text).unwrap().seq
}

fn ignored(fx: &Fx) -> Vec<String> {
    let s = fx.must(&["status", "--json"]).json();
    s["ignored_events"].as_array().unwrap().iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()
}

fn open_phase(fx: &Fx) -> String {
    let s = fx.must(&["status", "--json"]).json();
    s["checkpoint_fix"]["open"]["phase"].as_str().unwrap_or("").to_string()
}

// x1b: a fix for failed checks that `bf next` opens after an abort and a sync.
#[test]
fn words_written_during_a_checks_fix_do_not_accept_the_high_finding() {
    let fx = setup(GREP);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort(&fx);
    fs::create_dir_all(fx.root.join("docs")).unwrap();
    fs::write(fx.root.join("docs/note.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "docs/note.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    say("sync", &fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]));
    say("verifier runs X-2", &fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "grep", "-q", "ok", "src/a.txt"], ""));
    eprintln!("next: {:?}", actions(&fx));
    assert_eq!(open_phase(&fx), "fix", "a fix for the failed check is at work");
    fx.human(OLD);
    refused_at_work("accept during the checks fix", &accept(&fx, "r1.1", OLD));
    fix_built(&fx, "ok\n");
    fix_clean(&fx);
    assert_eq!(open_phase(&fx), "", "the fix ended");
    refused_before("accept after the fix on the same words", &accept(&fx, "r1.1", OLD));
    fx.human("I accept the high finding as it is.");
    let o = accept(&fx, "r1.1", "I accept the high finding as it is.");
    say("accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

// x2: a fix of one finding does not let words decide another.
#[test]
fn words_written_during_the_fix_of_another_finding_decide_nothing() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong"), f("medium", "the name is wrong")]));
    abort(&fx);
    fx.human("Fix r1.1 first, please.");
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1", "--quote", "Fix r1.1 first"]);
    say("pick r1.1", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human(OLD);
    refused_at_work("accept r1.2 during the r1.1 fix", &accept(&fx, "r1.2", OLD));
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--decline", "r1.2", "--reason", "not now", "--quote", OLD]);
    refused_at_work("decline r1.2 during the r1.1 fix", &o);
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.2", "--quote", OLD]);
    refused_at_work("pick r1.2 during the r1.1 fix", &o);
    fix_built(&fx, "right now\n");
    fix_clean(&fx);
    assert_eq!(open_phase(&fx), "");
    refused_before("accept r1.2 after the fix ended", &accept(&fx, "r1.2", OLD));
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.2", "--quote", OLD]);
    refused_before("pick r1.2 after the fix ended", &o);
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    say("close", &o);
    assert_ne!(o.code, 0);
    assert!(o.all().contains("r1.2"), "{}", o.all());
    fx.human("I accept r1.2 as it is.");
    let o = accept(&fx, "r1.2", "I accept r1.2 as it is.");
    say("accept r1.2 on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// x3: a low pick with no words opens a fix; words written during it do not accept the high finding.
#[test]
fn words_written_during_a_low_fix_do_not_accept_the_high_finding() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong"), f("low", "a small thing")]));
    abort(&fx);
    let o = fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.2"]);
    say("low pick, no words", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human(OLD);
    refused_at_work("accept r1.1 during the low fix", &accept(&fx, "r1.1", OLD));
    abort(&fx);
    refused_before("accept r1.1 after the abort on the same words", &accept(&fx, "r1.1", OLD));
    fx.human("I accept r1.1 as it is.");
    let o = accept(&fx, "r1.1", "I accept r1.1 as it is.");
    say("accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    eprintln!("fates: {}", fates(&fx));
}

// A fix at work, an abort with no words, and a message written after it decides.
#[test]
fn the_abort_needs_no_words_and_a_message_after_it_decides() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    assert_eq!(open_phase(&fx), "fix");
    fx.human(OLD);
    refused_at_work("accept during the review fix", &accept(&fx, "r1.1", OLD));
    abort(&fx);
    fx.human("I accept r1.1 as it is.");
    let o = accept(&fx, "r1.1", "I accept r1.1 as it is.");
    assert_eq!(o.code, 0, "{}", o.all());
}

// The replay: a settle appended while a fix is at work names the fix, not the turn and quote.
#[test]
fn a_settle_appended_while_a_fix_is_at_work_is_listed_with_the_at_work_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.finding.settle",
        json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "accepted", "reason": "", "turn": t, "quote": OLD}),
    );
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("while a checkpoint fix was at work"), "{notes:?}");
    assert!(!notes[0].contains("no valid human turn"), "{notes:?}");
    assert!(fates(&fx).is_empty() || !fates(&fx).contains("accepted"), "{}", fates(&fx));
    abort(&fx);
    fx.human("I accept r1.1 as it is.");
    let o = accept(&fx, "r1.1", "I accept r1.1 as it is.");
    assert_eq!(o.code, 0, "{}", o.all());
}

// The replay: a review release appended while a fix is at work. No command reaches a parked review with a
// fix at work, so the event is appended through the library.
#[test]
fn a_review_release_appended_while_a_fix_is_at_work_is_listed_with_the_at_work_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.review.release",
        json!({"checkpoint": "K-1", "after_round": 1, "turn": t, "quote": OLD}),
    );
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("review release") && notes[0].contains("while a checkpoint fix was at work"), "{notes:?}");
}

// x7: the release of a fix that was not parked is ignored, and the fix keeps its phase.
#[test]
fn a_fix_release_of_a_fix_that_is_not_parked_is_listed_with_the_not_parked_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    fix_built(&fx, "try 1\n");
    assert_eq!(open_phase(&fx), "review");
    let before = fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"].clone();
    let t = turn_seq(&fx, OLD0);
    let r = bf::events::append(&fx.project(), "checkpoint.fix.release", json!({"checkpoint": "K-1", "turn": t, "quote": OLD0}));
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("not parked"), "{notes:?}");
    assert_eq!(open_phase(&fx), "review");
    let after = fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"].clone();
    eprintln!("open fix after the ignored release: {after}");
    assert_eq!(after["rounds"], before["rounds"], "rounds kept");
    assert_eq!(after["attempts"], before["attempts"], "attempts kept");
}

// F-136 (a): a decline with no reason, written when no fix is at work, is listed with its own note.
#[test]
fn a_decline_with_no_reason_is_listed_with_the_no_reason_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort(&fx);
    fx.human("I decline r1.1, it is fine.");
    let t = turn_seq(&fx, "I decline r1.1, it is fine.");
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.finding.settle",
        json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "declined", "reason": "", "turn": t, "quote": "I decline r1.1"}),
    );
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("declined with no reason"), "{notes:?}");
    assert!(!notes[0].contains("at work"), "{notes:?}");
}

// F-136 (b): a fix pick with words appended while another fix is at work is listed with the at-work note.
#[test]
fn a_fix_pick_with_words_appended_while_a_fix_is_at_work_is_listed_with_the_at_work_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.fix.open",
        json!({"checkpoint": "K-1", "source": "review", "human": {"turn": t, "quote": OLD}, "finding_ids": [{"round": 1, "index": 1}], "findings": [f("high", "the answer is wrong")]}),
    );
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("while a checkpoint fix was at work"), "{notes:?}");
    assert!(!notes[0].contains("no valid human turn"), "{notes:?}");
    assert_eq!(open_phase(&fx), "fix");
}

// F-136 (c): words written during a fix that has since ended are listed with the turn-and-quote note, not
// the at-work note.
#[test]
fn words_from_before_the_bound_are_listed_with_the_turn_and_quote_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    abort(&fx);
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.finding.settle",
        json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "accepted", "reason": "", "turn": t, "quote": OLD}),
    );
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("settle ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("no valid human turn and quote"), "{notes:?}");
    assert!(!notes[0].contains("while a checkpoint fix was at work"), "{notes:?}");
}

#[test]
fn a_fix_release_on_words_from_before_the_park_is_listed_with_the_turn_and_quote_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    let r = bf::events::append(&fx.project(), "checkpoint.fix.park", json!({"checkpoint": "K-1", "reason": "stuck", "attempts": 1}));
    assert!(r.is_ok());
    assert_eq!(open_phase(&fx), "parked");
    let r = bf::events::append(&fx.project(), "checkpoint.fix.release", json!({"checkpoint": "K-1", "turn": t, "quote": OLD}));
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("release ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("no valid human turn and quote"), "{notes:?}");
    assert!(!notes[0].contains("while a checkpoint fix was at work"), "{notes:?}");
    assert_eq!(open_phase(&fx), "parked");
}

// A K-1 era run (no checkpoint review) writes none of the new decision events and replays clean.
#[test]
fn a_run_with_the_checkpoint_review_off_has_no_ignored_event() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    let o = fx.bf(&["log", "--verify"]);
    say("log --verify", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}
