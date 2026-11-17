// R-103 (guards, K2R6-2): the refusal of a checkpoint review release while a checkpoint fix is at work, and
// the report bound of a fix pick on replay. Both assert on exit codes, the state that results and the
// commands a refusal names, never on the sentence of the cause.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const OLD0: &str = "Some early words, before the plan.";
const WORDS: &str = "Looks good to me, keep going.";
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

/// A run whose checkpoint check X-2 fails until src/a.txt holds "ok", with one review round before parking.
fn setup() -> Fx {
    let fx = Fx::new();
    fx.human(OLD0);
    plan(&fx, GREP);
    start(&fx);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    fx
}

fn brief(fx: &Fx) -> Out {
    fx.bf(&["brief", "K-1", "--role", "reviewer", "--json"])
}

fn finding_high() -> Value {
    json!({"title": "the answer is wrong", "severity": "high", "where": "src/a.txt:1", "action": "make it right"})
}

fn report_high(fx: &Fx) {
    brief(fx);
    let body = json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": [finding_high()]});
    let o = fx.bf_stdin(&fx.root, &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"], &body.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

fn fail_x2(fx: &Fx) {
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "grep", "-q", "ok", "src/a.txt"], "");
    assert_ne!(o.code, 0, "X-2 must fail here: {}", o.all());
}

fn actions(fx: &Fx) -> Vec<String> {
    fx.must(&["next", "--json"]).json()["directives"].as_array().unwrap().iter()
        .map(|d| d["action"].as_str().unwrap_or("").to_string()).collect()
}

fn fix_end(fx: &Fx) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "ok\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fix"]);
    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "builder", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "reviewer", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn open_phase(fx: &Fx) -> String {
    fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"]["phase"].as_str().unwrap_or("").to_string()
}

fn parked(fx: &Fx) -> bool {
    fx.must(&["status", "--json"]).json()["checkpoint_reviews"][0]["parked"].as_bool().unwrap_or(false)
}

fn ignored(fx: &Fx) -> Vec<String> {
    let s = fx.must(&["status", "--json"]).json();
    s["ignored_events"].as_array().unwrap().iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()
}

fn turn_seq(fx: &Fx, text: &str) -> u64 {
    let st = bf::events::replay_log(&bf::events::load_log(&fx.project()).unwrap());
    st.turns.iter().find(|t| t.text == text).unwrap().seq
}

fn release(fx: &Fx, quote: &str) -> Out {
    fx.bf(&["checkpoint", "review", "K-1", "--release", "--quote", quote])
}

// z3: the checkpoint is parked, bf next opens a checks fix while it is parked, and the human's words written
// during that fix release nothing, during the fix or after it; a new message releases.
#[test]
fn a_review_release_on_words_written_during_a_fix_is_refused_and_stays_refused_after_it() {
    let fx = setup();
    report_high(&fx);
    assert!(parked(&fx), "round 1's high finding parks the checkpoint with no fix");
    assert_eq!(open_phase(&fx), "", "no fix yet");
    fail_x2(&fx);
    eprintln!("next after the failing check: {:?}", actions(&fx));
    assert_eq!(open_phase(&fx), "fix", "bf next opened a checks fix on the parked checkpoint");
    assert!(parked(&fx));

    fx.human(WORDS);
    let o = release(&fx, WORDS);
    eprintln!("release during the fix: exit {} | {}", o.code, o.all());
    assert_eq!(o.code, 2, "{}", o.all());
    let t = o.all();
    assert!(t.contains("bf next"), "{t}");
    assert!(t.contains("bf checkpoint fix K-1 --abort"), "{t}");
    assert!(parked(&fx), "still parked");
    assert_eq!(open_phase(&fx), "fix", "the fix is untouched");

    fix_end(&fx);
    assert_eq!(open_phase(&fx), "", "the fix ended");
    assert!(parked(&fx), "the checkpoint is still parked after the fix");
    let o = release(&fx, WORDS);
    eprintln!("release after the fix on the same words: exit {} | {}", o.code, o.all());
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(parked(&fx), "still parked");
    assert_eq!(open_phase(&fx), "");

    fx.human("Release it, please.");
    let o = release(&fx, "Release it, please.");
    eprintln!("release on a new message: exit {} | {}", o.code, o.all());
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!parked(&fx), "released");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

// M3: a round reports a high finding after the latest checkpoint fix ended. A fix pick that cites words
// written between that end and the report is ignored on replay and the finding stays open; words written
// after the report are taken.
#[test]
fn a_fix_pick_on_replay_takes_only_words_written_after_the_round_report() {
    let fx = setup();
    // A checks fix, opened by bf next on the failing check, runs and ends before any review round.
    fail_x2(&fx);
    eprintln!("next after the failing check: {:?}", actions(&fx));
    assert_eq!(open_phase(&fx), "fix", "a checks fix is at work");
    fix_end(&fx);
    assert_eq!(open_phase(&fx), "", "the fix ended");

    // Words written after that end and before the round's report.
    fx.human(WORDS);
    let before_report = turn_seq(&fx, WORDS);
    report_high(&fx);
    assert!(parked(&fx), "the high finding parks the checkpoint with no fix");
    assert_eq!(open_phase(&fx), "");
    fx.human("Fix the high finding, please.");
    let after_report = turn_seq(&fx, "Fix the high finding, please.");
    assert!(after_report > before_report);

    let pick = |turn: u64, quote: &str| {
        bf::events::append(
            &fx.project(),
            "checkpoint.fix.open",
            json!({"checkpoint": "K-1", "source": "review", "human": {"turn": turn, "quote": quote},
                   "finding_ids": [{"round": 1, "index": 1}], "findings": [finding_high()]}),
        )
    };
    assert!(pick(before_report, WORDS).is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored after the pick on the early words: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(open_phase(&fx), "", "no fix opened from words written before the report");
    assert!(parked(&fx), "the finding stays open");

    assert!(pick(after_report, "Fix the high finding, please.").is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored after the pick on the later words: {notes:?}");
    assert_eq!(notes.len(), 1, "only the early pick is ignored: {notes:?}");
    assert_eq!(open_phase(&fx), "fix", "words written after the report are taken");
}
