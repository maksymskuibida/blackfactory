// R-103, D-79: the human is not being asked about a checkpoint's findings while a sync of main is open, a
// checkpoint fix is at work, a check of the checkpoint is marked, a worker's question on it is unanswered or a
// wave check holds its close. One predicate (`bf::events::hold`) decides it for `bf next` and for the five
// decisions; words written during any hold stay refused after it; the replay applies the same predicate; and
// a sync of main does not start while a checkpoint fix is open.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const OLD0: &str = "Some early words, before the plan.";
const OLD: &str = "Looks good to me, keep going.";
const NEW: &str = "Fine, I have thought about it: accept it.";
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

/// The state every test starts from, built through the full `bf` calls (unit start, evidence run, the
/// two reports and land): kept by one test so that the path of building it stays covered.
fn setup_built(x2: &str) -> Fx {
    let fx = Fx::new();
    fx.human(OLD0);
    plan(&fx, x2);
    start(&fx);
    fx
}

fn pass_state(fx: &Fx) {
    fx.human(OLD0);
    plan(fx, PASS);
    start(fx);
}

fn grep_state(fx: &Fx) {
    fx.human(OLD0);
    plan(fx, GREP);
    start(fx);
}

/// The states of this file, each built once by those calls and copied per test (D-140).
const STATES: [(&str, Build); 2] = [("checkpoint_hold::pass", pass_state), ("checkpoint_hold::grep", grep_state)];

/// The same state as `setup_built`, copied.
fn setup(x2: &str) -> Fx {
    let (name, build) = if x2 == PASS { STATES[0] } else { STATES[1] };
    Fx::state_copy(name, build)
}

/// `fixture_state_copy::required_sandbox_is_met_for_state_copy`: each state of this file meets the
/// sandbox under `BF_TEST_SANDBOX_REQUIRE=1`.
pub(crate) fn require_sandbox_for_states() {
    for (name, build) in STATES {
        require_sandbox_from_env(&Fx::state_copy(name, build));
    }
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
    let one = |s: &str| s.lines().take(8).collect::<Vec<_>>().join(" | ");
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

fn ignored(fx: &Fx) -> Vec<String> {
    let s = fx.must(&["status", "--json"]).json();
    s["ignored_events"].as_array().unwrap().iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()
}

fn turn_seq(fx: &Fx, text: &str) -> u64 {
    let st = bf::events::replay_log(&bf::events::load_log(&fx.project()).unwrap());
    st.turns.iter().find(|t| t.text == text).unwrap().seq
}

fn commit_fix(fx: &Fx, text: &str) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), text).unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fix"]);
}

fn fix_built(fx: &Fx, text: &str) {
    commit_fix(fx, text);
    let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "builder", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn fix_clean(fx: &Fx) {
    let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "reviewer", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn accept(fx: &Fx, ids: &str, quote: &str) -> Out {
    fx.bf(&["checkpoint", "finding", "K-1", "--accept", ids, "--quote", quote])
}

fn decline(fx: &Fx, ids: &str, quote: &str) -> Out {
    fx.bf(&["checkpoint", "finding", "K-1", "--decline", ids, "--reason", "not a defect", "--quote", quote])
}

fn pick(fx: &Fx, ids: &str, quote: &str) -> Out {
    fx.bf(&["checkpoint", "finding", "K-1", "--fix-findings", ids, "--quote", quote])
}

fn release_review(fx: &Fx, quote: &str) -> Out {
    fx.bf(&["checkpoint", "review", "K-1", "--release", "--quote", quote])
}

/// A decision refused during a hold: exit 2, the hold named, the command that ends it, and `bf next`.
fn refused_by_hold(tag: &str, o: &Out, cause: &str, ends: &str) {
    say(tag, o);
    assert_eq!(o.code, 2, "{tag}: {}", o.all());
    let t = o.all();
    assert!(t.contains("the human is not being asked about the findings of K-1"), "{tag}: {t}");
    assert!(t.contains(cause), "{tag}: {t}");
    assert!(t.contains(ends), "{tag}: {t}");
    assert!(t.contains("bf next"), "{tag}: {t}");
}

fn refused_before(tag: &str, o: &Out) {
    say(tag, o);
    assert_eq!(o.code, 2, "{tag}: {}", o.all());
    assert!(o.all().contains("written before"), "{tag}: {}", o.all());
}

fn abort_fix(fx: &Fx) {
    let o = fx.bf(&["checkpoint", "fix", "K-1", "--abort"]);
    say("abort the fix with no words", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn conflict_on_main(fx: &Fx) {
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/a.txt"), "from main\n").unwrap();
    git(&fx.root, &["add", "src/a.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
}

fn open_conflicting_sync(fx: &Fx) {
    conflict_on_main(fx);
    fx.human("Yes, bring main in now.");
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]);
    say("sync", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    let phase = fx.must(&["status", "--json"]).json()["sync_open"]["phase"].clone();
    assert_eq!(phase, json!("resolve"), "the sync opens at its conflict");
    assert_eq!(actions(fx), vec!["resolve-sync"]);
}

fn resolve_sync(fx: &Fx) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "from the unit\nfrom main\n").unwrap();
    git(&k, &["add", "src/a.txt"]);
    git(&k, &["commit", "-q", "--no-edit"]);
    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "builder", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "reviewer", "--result", "built"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(fx.must(&["status", "--json"]).json()["sync_open"].is_null());
}

/// After the hold: a clean second round if one is asked for, the check receipt, then the close.
fn close_ok(fx: &Fx, x2: &[&str]) {
    if actions(fx).iter().any(|a| a == "review-checkpoint") {
        brief(fx);
        let rc = fx.work("K-1-review");
        if rc.exists() {
            let mut a = vec!["evidence", "run", "X-2", "--"];
            a.extend_from_slice(x2);
            fx.bf_stdin(&rc, &a, "");
        }
        let body = json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": []});
        let o = fx.bf_stdin(&fx.root, &["report", "K-1", "--role", "reviewer", "--result", "built", "--findings", "-"], &body.to_string());
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let mut a = vec!["evidence", "run", "X-2", "--"];
    a.extend_from_slice(x2);
    fx.bf_stdin(&fx.work("K-1"), &a, "");
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    say("close", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    let v = fx.bf(&["log", "--verify"]);
    say("log --verify", &v);
    assert_eq!(v.code, 0, "{}", v.all());
    assert!(ignored(fx).is_empty(), "{:?}", ignored(fx));
}

// y1, y2: a sync of main that conflicts opens after the aborted fix of a high finding. During it no decision
// is taken, and the words written during it decide nothing after it either; a new message does.
#[test]
fn words_written_during_an_open_sync_decide_nothing_during_it_or_after() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    open_conflicting_sync(&fx);
    fx.human(OLD);
    let ends = "bf checkpoint sync K-1 --abort";
    refused_by_hold("y1 --accept during the sync", &accept(&fx, "r1.1", OLD), "a sync of main into K-1 is open", ends);
    refused_by_hold("y1 --decline during the sync", &decline(&fx, "r1.1", OLD), "a sync of main into K-1 is open", ends);
    refused_by_hold("y1 --fix-findings during the sync", &pick(&fx, "r1.1", OLD), "a sync of main into K-1 is open", ends);
    assert!(!fates(&fx).contains("accepted"), "{}", fates(&fx));
    resolve_sync(&fx);
    // y2: the same words after the sync ended.
    refused_before("y2 --accept after the sync on the words written during it", &accept(&fx, "r1.1", OLD));
    refused_before("y2 --decline after the sync on the same words", &decline(&fx, "r1.1", OLD));
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("y2 --accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    close_ok(&fx, &["test", "-f", "src/a.txt"]);
}

// No lock: the sync's own abort needs no words, and a message written after it decides.
#[test]
fn the_sync_abort_needs_no_words_and_a_new_message_decides() {
    let fx = setup_built(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    open_conflicting_sync(&fx);
    fx.human(OLD);
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--abort"]);
    say("abort the sync with no words", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    refused_before("--accept after the abort on the words written during the sync", &accept(&fx, "r1.1", OLD));
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("--accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// y3: a check marked unreachable by the verifier: bf next gives decide-check; no decision is taken during
// the mark, and the words written during it stay refused after a passing run lifts it.
#[test]
fn words_written_while_a_check_is_marked_decide_nothing_during_it_or_after() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "fail", "X-2", "--reason", "the service is down"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(actions(&fx), vec!["decide-check"]);
    fx.human(OLD);
    let ends = "decide-check";
    refused_by_hold("y3 --accept while a check is marked", &accept(&fx, "r1.1", OLD), "a check of K-1 is marked failed or unreachable (X-2)", ends);
    refused_by_hold("y3 --decline while a check is marked", &decline(&fx, "r1.1", OLD), "X-2", ends);
    refused_by_hold("y3 --fix-findings while a check is marked", &pick(&fx, "r1.1", OLD), "X-2", ends);
    // The verifier's new passing run lifts the mark and needs no words.
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    refused_before("y3 --accept after the mark was lifted, on the words written during it", &accept(&fx, "r1.1", OLD));
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("y3 --accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// y4: a parked review, a sync opened and ended; the review release on words from during the sync is refused,
// and a new message releases it.
#[test]
fn a_parked_review_is_not_released_on_words_written_during_a_sync() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    open_conflicting_sync(&fx);
    fx.human(OLD);
    refused_by_hold("y4 review --release during the sync", &release_review(&fx, OLD), "a sync of main into K-1 is open", "bf checkpoint sync K-1 --abort");
    resolve_sync(&fx);
    refused_before("y4 review --release after the sync, words written during it", &release_review(&fx, OLD));
    fx.human("Release it, please.");
    let o = release_review(&fx, "Release it, please.");
    say("y4 review --release on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

// z3: a parked checkpoint with a checks fix at work (commands only reach it): the release is refused during the
// fix, the same words are refused after it, and a new message releases.
#[test]
fn a_parked_review_is_not_released_while_a_checks_fix_works() {
    let fx = setup(GREP);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "grep", "-q", "ok", "src/a.txt"], "");
    say("verifier runs X-2 (fails)", &o);
    let a = actions(&fx);
    eprintln!("next: {a:?}");
    fx.human(OLD);
    refused_by_hold("z3 review --release while the fix works", &release_review(&fx, OLD), "a checkpoint fix is at work", "bf checkpoint fix K-1 --abort");
    fix_built(&fx, "ok\n");
    fix_clean(&fx);
    refused_before("z3 review --release after the fix, on the words written during it", &release_review(&fx, OLD));
    fx.human("Release it, please.");
    let o = release_review(&fx, "Release it, please.");
    say("z3 review --release on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// A fix at work holds the accept, the decline and the pick, and the abort needs no words.
#[test]
fn a_checkpoint_fix_at_work_holds_every_decision() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong"), f("high", "the other answer is wrong")]));
    let a = actions(&fx);
    eprintln!("next: {a:?}");
    fx.human(OLD);
    let ends = "bf checkpoint fix K-1 --abort";
    let cause = "a checkpoint fix is at work on K-1";
    refused_by_hold("--accept during the fix", &accept(&fx, "r1.2", OLD), cause, ends);
    refused_by_hold("--decline during the fix", &decline(&fx, "r1.2", OLD), cause, ends);
    refused_by_hold("--fix-findings during the fix", &pick(&fx, "r1.2", OLD), cause, ends);
    abort_fix(&fx);
    refused_before("--accept after the abort, on the words written during the fix", &accept(&fx, "r1.2", OLD));
    fx.human(NEW);
    let o = accept(&fx, "r1.2", NEW);
    assert_eq!(o.code, 0, "{}", o.all());
}

// A worker's question on the checkpoint, unanswered: the human is not asked about its findings; the words
// written before the answer stay refused after it.
#[test]
fn words_written_before_the_answer_to_a_worker_question_decide_nothing() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    let o = fx.bf(&["ask", "--unit", "K-1", "--title", "Which of the two should win?"]);
    say("a worker asks", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    let q = fx.must(&["questions", "--open", "--json"]).json()[0]["id"].as_str().unwrap().to_string();
    fx.human(OLD);
    let ends = format!("bf answer {q}");
    refused_by_hold("--accept while the question is unanswered", &accept(&fx, "r1.1", OLD), &format!("a worker's question on K-1 ({q}) is unanswered"), &ends);
    refused_by_hold("--decline while the question is unanswered", &decline(&fx, "r1.1", OLD), &q, &ends);
    refused_by_hold("--fix-findings while the question is unanswered", &pick(&fx, "r1.1", OLD), &q, &ends);
    // The answer needs the human's words for itself and nothing about the finding.
    fx.human("The first one should win.");
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.json");
    fs::write(&a, "{\"answer\": \"The first one\"}").unwrap();
    let o = fx.bf(&["answer", &q, "--file", a.to_str().unwrap()]);
    say("the human answers", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    refused_before("--accept after the answer, on the words written before it", &accept(&fx, "r1.1", OLD));
    refused_before("--accept after the answer, on the words of the answer", &accept(&fx, "r1.1", "The first one should win."));
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("--accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// F-140: a parked review is not released while a check is marked failed, nor on the words written during it.
#[test]
fn a_parked_review_is_not_released_while_a_check_is_marked() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "fail", "X-2", "--reason", "the service is down"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    // No `bf next` runs here, so no fix is opened for the failed check: the mark alone holds the decision.
    fx.human(OLD);
    refused_by_hold("review --release while a check is marked", &release_review(&fx, OLD), "a check of K-1 is marked failed or unreachable (X-2)", "decide-check");
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    refused_before("review --release after the mark, on the words written during it", &release_review(&fx, OLD));
    fx.human("Release it, please.");
    let o = release_review(&fx, "Release it, please.");
    say("review --release on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// F-140, D-110: a parked review is released on words written while a check is marked unreachable only
// (such a hold decides nothing); `bf next` still gives decide-check.
#[test]
fn a_parked_review_is_released_while_a_check_is_marked_unreachable_only() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "unreachable", "X-2", "--reason", "the service is down"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(actions(&fx), vec!["decide-check", "run-checkpoint-checks"]);
    fx.human(OLD);
    let o = release_review(&fx, OLD);
    say("review --release while a check is marked unreachable", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_parked_review_is_not_released_while_a_worker_question_is_unanswered() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    let o = fx.bf(&["ask", "--unit", "K-1", "--title", "Which of the two should win?"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let q = fx.must(&["questions", "--open", "--json"]).json()[0]["id"].as_str().unwrap().to_string();
    fx.human(OLD);
    refused_by_hold("review --release while the question is unanswered", &release_review(&fx, OLD), &format!("a worker's question on K-1 ({q}) is unanswered"), &format!("bf answer {q}"));
    fx.human("The first one should win.");
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.json");
    fs::write(&a, "{\"answer\": \"The first one\"}").unwrap();
    let o = fx.bf(&["answer", &q, "--file", a.to_str().unwrap()]);
    assert_eq!(o.code, 0, "{}", o.all());
    refused_before("review --release after the answer, on the words written before it", &release_review(&fx, OLD));
    fx.human("Release it, please.");
    let o = release_review(&fx, "Release it, please.");
    say("review --release on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// F-140: a wave check that holds the close, the hold logged by the observer (not appended by hand), refuses
// every decision through the command, and a message written after it ends decides.
#[test]
fn a_wave_check_that_holds_the_close_refuses_the_decisions_through_the_command() {
    let fx = Fx::new();
    fx.human(OLD0);
    plan(&fx, PASS);
    let r = fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"wave\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[]\nscope=\"wave\"\n");
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    fx.commit_all("wave check");
    fx.must(&["checks", "propose", "checkpoint-review=on", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it", "--scope", "default"]);
    fx.human("Yes, set the checkpoint review that way.");
    fx.must(&["checks", "set", "--quote", "set the checkpoint review that way"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.must(&["checks", "wave", "propose", "override", "test -f src/a.txt"]);
    fx.human("Use that wave check.");
    fx.must(&["checks", "wave", "set", "--quote", "Use that wave check"]);
    fx.must(&["checks", "unit", "propose", "none"]);
    fx.human("No checks for every unit in this run.");
    fx.must(&["checks", "unit", "set", "--quote", "No checks for every unit"]);
    let wc = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    let a = actions(&fx);
    eprintln!("next: {a:?}");
    assert!(a.iter().any(|x| x == "wave-check"), "{a:?}");
    assert!(!a.iter().any(|x| x.starts_with("decide-checkpoint")), "{a:?}");
    fx.human(OLD);
    let ends = "the wave check passing";
    let cause = "a wave check holds the close of K-1";
    refused_by_hold("--accept while a wave check holds the close", &accept(&fx, "r1.1", OLD), cause, ends);
    refused_by_hold("--decline while a wave check holds the close", &decline(&fx, "r1.1", OLD), cause, ends);
    refused_by_hold("--fix-findings while a wave check holds the close", &pick(&fx, "r1.1", OLD), cause, ends);
    let log = bf::events::load_log(&fx.project()).unwrap();
    let held = log.events.iter().any(|e| e["kind"] == "checkpoint.hold" && e["cause"] == "wave");
    assert!(held, "the observer logged no wave hold");
    let o = fx.bf(&["checkpoint", "wave-check", "K-1", "--wave", "1"]);
    assert_eq!(o.code, 0, "{}", o.all());
    refused_before("--accept after the wave check, on the words written during the hold", &accept(&fx, "r1.1", OLD));
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("--accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// K2R6-3 (z4): a review release appended right after a checks fix opened is not the release's own fix: ignored.
#[test]
fn a_review_release_appended_right_after_a_checks_fix_opened_is_ignored() {
    let fx = setup(GREP);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human("Release it, please.");
    fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "grep", "-q", "ok", "src/a.txt"], "");
    let a = actions(&fx);
    eprintln!("next: {a:?}");
    let s = fx.must(&["status", "--json"]).json();
    assert_eq!(s["checkpoint_fix"]["open"]["source"], "check", "{s}");
    let t = turn_seq(&fx, "Release it, please.");
    let r = bf::events::append(
        &fx.project(),
        "checkpoint.review.release",
        json!({"checkpoint": "K-1", "after_round": 1, "turn": t, "quote": "Release it, please."}),
    );
    assert!(r.is_ok());
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("review release") && notes[0].contains("a checkpoint fix was at work"), "{notes:?}");
    let s = fx.must(&["status", "--json"]).json();
    assert_eq!(s["checkpoint_reviews"][0]["parked"], json!(true), "{s}");
}

// M3: a fix opened on words written before its round's report, with no fix end after them, is ignored on
// replay: the report is the bound there, not only the end of a fix.
#[test]
fn a_fix_pick_on_words_written_before_the_round_report_is_ignored_on_replay() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    fx.human(OLD0);
    plan(&fx, PASS);
    start(&fx);
    fx.human(OLD);
    report(&fx, json!([f("high", "the answer is wrong")]));
    let t = turn_seq(&fx, OLD);
    bf::events::append(
        &fx.project(),
        "checkpoint.fix.open",
        json!({"checkpoint": "K-1", "source": "review", "findings": [f("high", "the answer is wrong")],
               "finding_ids": [{"round": 1, "index": 1}], "before": [],
               "human": {"turn": t, "quote": OLD}}),
    )
    .unwrap();
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("words written before the bound"), "{notes:?}");
}

// The release's own fix is still set aside: the command's release replays with no ignored event (y4 and z3
// above check it too); here the release of a parked review with open findings opens its fix first.
#[test]
fn the_fix_a_review_release_opened_is_set_aside_on_replay() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human("Release it, please.");
    let o = release_review(&fx, "Release it, please.");
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
    let s = fx.must(&["status", "--json"]).json();
    assert_eq!(s["checkpoint_fix"]["open"]["source"], "review", "{s}");
}

// Replay: a settle appended through the library during an open sync is ignored, and the note names the sync.
#[test]
fn a_settle_appended_during_an_open_sync_is_listed_with_the_sync_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    open_conflicting_sync(&fx);
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
    assert!(notes[0].contains("a finding settled while a sync of main into K-1 was open"), "{notes:?}");
    assert!(!fates(&fx).contains("accepted"), "{}", fates(&fx));
}

// Replay: a settle appended while a check is marked, and one appended while a worker's question is unanswered,
// are ignored with the cause in the note; so is a settle on words from before the end of the hold.
#[test]
fn a_settle_appended_during_a_marked_check_or_an_unanswered_question_is_listed_with_its_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    fx.bf_stdin(&fx.work("K-1"), &["evidence", "fail", "X-2", "--reason", "the service is down"], "");
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    let settle = |t: u64, q: &str| {
        bf::events::append(
            &fx.project(),
            "checkpoint.finding.settle",
            json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "accepted", "reason": "", "turn": t, "quote": q}),
        )
        .unwrap();
    };
    settle(t, OLD);
    let notes = ignored(&fx);
    eprintln!("ignored while marked: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("a check of K-1 was marked failed or unreachable (X-2)"), "{notes:?}");
    // The passing run lifts the mark: the same words are now before the bound.
    fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
    settle(t, OLD);
    let notes = ignored(&fx);
    eprintln!("ignored after the mark: {notes:?}");
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[1].contains("words written before the bound"), "{notes:?}");
    // A question on the checkpoint.
    let o = fx.bf(&["ask", "--unit", "K-1", "--title", "Which of the two should win?"]);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human(NEW);
    let t2 = turn_seq(&fx, NEW);
    settle(t2, NEW);
    let notes = ignored(&fx);
    eprintln!("ignored while asked: {notes:?}");
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes[2].contains("a worker's question on K-1") && notes[2].contains("was unanswered"), "{notes:?}");
    assert!(!fates(&fx).contains("accepted"), "{}", fates(&fx));
}

// Replay: a wave check that holds the close is a hold of the same predicate (appended as the engine logs it).
#[test]
fn a_settle_appended_while_a_wave_check_holds_the_close_is_listed_with_the_wave_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    fx.must(&["next", "--json"]);
    bf::events::append(
        &fx.project(),
        "checkpoint.hold",
        json!({"checkpoint": "K-1", "cause": "wave", "detail": "the wave check of wave 1 of K-1 has not passed (failed)"}),
    )
    .unwrap();
    bf::events::append(
        &fx.project(),
        "checkpoint.finding.settle",
        json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "accepted", "reason": "", "turn": t, "quote": OLD}),
    )
    .unwrap();
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("a wave check held the close of K-1"), "{notes:?}");
}

// K2R6-5 (z5, z5b): a sync of main is refused while a checkpoint fix is open, and runs after the fix's abort.
#[test]
fn a_sync_of_main_is_refused_while_a_checkpoint_fix_is_open() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    let a = actions(&fx);
    eprintln!("next (the fix opens): {a:?}");
    fs::create_dir_all(fx.root.join("docs")).unwrap();
    fs::write(fx.root.join("docs/note.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "docs/note.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]);
    say("z5 sync while the fix is at work", &o);
    assert_eq!(o.code, 2, "{}", o.all());
    let t = o.all();
    assert!(t.contains("a checkpoint fix of K-1 is open"), "{t}");
    assert!(t.contains("bf checkpoint fix K-1 --abort") && t.contains("bf next"), "{t}");
    assert!(!fx.work("K-1").join("docs/note.txt").exists());
    abort_fix(&fx);
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]);
    say("z5 sync after the abort", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(fx.work("K-1").join("docs/note.txt").exists());
}

// K2R6-5 (z5b): a parked checkpoint fix is open too: the sync is refused, naming the fix's abort and `bf next`.
#[test]
fn a_sync_of_main_is_refused_while_a_checkpoint_fix_is_parked() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    let a = actions(&fx);
    eprintln!("next (the fix opens): {a:?}");
    let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "builder", "--result", "blocked", "--note", "stuck"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    let s = fx.must(&["status", "--json"]).json();
    eprintln!("fix: {}", s["checkpoint_fix"]);
    assert_eq!(actions(&fx), vec!["decide-checkpoint-fix"]);
    fs::create_dir_all(fx.root.join("docs")).unwrap();
    fs::write(fx.root.join("docs/note.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "docs/note.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]);
    say("z5b sync while the fix is parked", &o);
    assert_eq!(o.code, 2, "{}", o.all());
    let t = o.all();
    assert!(t.contains("a checkpoint fix of K-1 is open"), "{t}");
    assert!(t.contains("bf checkpoint fix K-1 --abort") && t.contains("bf next"), "{t}");
    assert!(!fx.work("K-1").join("docs/note.txt").exists());
    abort_fix(&fx);
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]);
    say("z5b sync after the abort", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// Old logs: a run with the checkpoint review off writes no hold event that matters and replays clean.
#[test]
fn a_run_with_the_checkpoint_review_off_has_no_ignored_event() {
    let fx = Fx::tiny_run();
    fx.build_unit("U-1", "src/hello.txt", "X-1", &["test", "-f", "src/hello.txt"]);
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    let o = fx.bf(&["log", "--verify"]);
    say("log --verify", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

/// (action, class): "asking", "holding:<Hold case>" or "neither".
const CLASSES: &[(&str, &str)] = &[
    ("decide-checkpoint-review", "asking"),
    ("decide-checkpoint-fix", "asking"),
    ("decide-low-findings", "asking"),
    ("decide-sync", "holding:Sync"),
    ("judge-sync-block", "holding:Sync"),
    ("resolve-sync", "holding:Sync"),
    ("review-sync", "holding:Sync"),
    ("fix-checkpoint", "holding:Fix"),
    ("review-checkpoint-fix", "holding:Fix"),
    ("go-on-checkpoint-fix", "holding:Fix"),
    ("fix-wave", "holding:Fix"),
    ("review-wave-fix", "holding:Fix"),
    ("plan-gap", "holding:Fix"),
    ("decide-check", "holding:Marked"),
    ("answer-question", "holding:Question"),
    ("decide-wave-check", "holding:Wave"),
    ("wave-check", "neither"),
    ("review-checkpoint", "neither"),
    ("run-checkpoint-checks", "neither"),
    ("audit-checkpoint", "neither"),
    ("close-checkpoint", "neither"),
    ("fix-checkpoint-gaps", "neither"),
    ("decide-gate", "holding:Run"),
    ("open-gate", "holding:Run"),
    ("add-records", "holding:Run"),
    ("answer-wave-checks", "neither"),
    ("answer-unit-checks", "neither"),
    ("start-unit", "neither"),
    ("build", "neither"),
    ("fix", "neither"),
    ("review", "neither"),
    ("security-audit", "neither"),
    ("land", "neither"),
    ("decide-unit", "neither"),
    ("judge-repair", "neither"),
    ("withdraw-repair", "neither"),
    ("decide-repair", "neither"),
    ("see-judged-repair", "neither"),
    ("see-waiting-human-findings", "neither"),
    ("open-plan-repair", "neither"),
    ("judge-block", "neither"),
];

// The guard of closure: every directive action `bf next` can give is classified as asking the human about
// findings, as holding them (with its `Hold` case), or as neither. A new action fails here until it is placed.
#[test]
fn every_directive_action_is_classified_as_asking_holding_or_neither() {
    let table = CLASSES;
    let holds = ["Sync", "Fix", "Marked", "Question", "Wave", "Run"];
    for (a, c) in table {
        let ok = matches!(*c, "asking" | "neither")
            || c.strip_prefix("holding:").map(|h| holds.contains(&h)).unwrap_or(false);
        assert!(ok, "`{a}` has the class `{c}`, which is not asking, neither or holding:<Hold case>");
    }
    let found = actions_in_source(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    eprintln!("directive actions found in the source: {found:?}");
    assert!(found.len() >= 20, "{found:?}");
    for a in &found {
        assert!(
            table.iter().any(|(t, _)| t == a),
            "the directive action `{a}` is not classified: place it in the table of this test as asking, holding (with its Hold case) or neither"
        );
    }
    for (t, _) in table {
        assert!(found.iter().any(|a| a == t), "the table names `{t}`, which no directive in the source gives any more");
    }
}

/// The sites where `"action":` takes a variable: (file, the text after `"action": `). The first gives the two
/// plan-question actions, both in the table (`answer-wave-checks`, `answer-unit-checks`); the second is a
/// finding's own action text in a review round, not a directive.
const VARIABLE_ACTION_SITES: &[(&str, &str)] = &[("next.rs", "action,"), ("ckreview.rs", "s(f, \"action\")"), ("ckreview.rs", "s(&f, \"action\")")];

/// Every string a directive's `"action":` takes in the source: a literal, or the literals of an `if` choice.
fn actions_in_source(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        if path.is_dir() {
            out.extend(actions_in_source(&path));
            continue;
        }
        if path.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for line in text.lines() {
            let mut rest = line;
            while let Some(i) = rest.find("\"action\": ") {
                rest = &rest[i + "\"action\": ".len()..];
                if let Some(lit) = rest.strip_prefix('"') {
                    if let Some(end) = lit.find('"') {
                        out.push(lit[..end].to_string());
                    }
                } else if rest.starts_with("if ") {
                    // `if cond { "a" } else { "b" }`: the literals right after an opening brace.
                    let upto = rest.find("}, ").map(|j| j + 1).unwrap_or(rest.len());
                    for part in rest[..upto].split("{ \"").skip(1) {
                        if let Some(end) = part.find("\" }") {
                            out.push(part[..end].to_string());
                        }
                    }
                } else {
                    // Neither a literal nor an `if` choice: only a site listed here may build its action so.
                    let name = path.file_name().unwrap().to_str().unwrap();
                    let allowed = VARIABLE_ACTION_SITES.iter().any(|(f, e)| *f == name && rest.starts_with(e));
                    assert!(
                        allowed,
                        "{name}: a directive action built from `{}` is neither a literal nor an if-choice: classify its values in the table of this test and list the site in VARIABLE_ACTION_SITES",
                        rest.trim()
                    );
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------------------------------------
// U-93: the early returns of `bf next` (the plan gate not approved, the budget cap reached) are holds of the
// one predicate; switching the checkpoint review off is bound like the other decisions; the low decision is
// offered whenever nothing holds; the observer runs after aborts, openings and gate events.
// ---------------------------------------------------------------------------------------------------------

fn gate_open_plan(fx: &Fx) {
    let o = fx.bf(&["gate", "open", "plan"]);
    say("gate open plan (again, during the run)", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn approve_plan(fx: &Fx, words: &str) {
    fx.human(words);
    let o = fx.bf(&["gate", "approve", "plan", "--quote", words]);
    say("gate approve plan", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn hold_events(fx: &Fx) -> Vec<(u64, String)> {
    bf::events::load_log(&fx.project()).unwrap().events.iter()
        .filter(|e| e["kind"] == "checkpoint.hold")
        .map(|e| (e["seq"].as_u64().unwrap(), e["cause"].as_str().unwrap_or("").to_string()))
        .collect()
}

fn last_seq_of(fx: &Fx, kind: &str) -> u64 {
    bf::events::load_log(&fx.project()).unwrap().events.iter()
        .filter(|e| e["kind"] == kind).map(|e| e["seq"].as_u64().unwrap()).next_back().unwrap_or(0)
}

// a8: the plan gate opened again during the run: bf next gives the gate decision alone, so no decision on a
// finding is taken on words written then, and the words stay refused after the gate is approved.
#[test]
fn words_written_while_the_plan_gate_is_open_again_decide_nothing_during_it_or_after() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    gate_open_plan(&fx);
    assert_eq!(actions(&fx), vec!["decide-gate"]);
    fx.human(OLD);
    let cause = "the plan gate of the run is open again";
    let ends = "bf gate approve plan";
    refused_by_hold("a8 --accept while the plan gate is open", &accept(&fx, "r1.1", OLD), cause, ends);
    refused_by_hold("a8 --decline while the plan gate is open", &decline(&fx, "r1.1", OLD), cause, ends);
    refused_by_hold("a8 --fix-findings while the plan gate is open", &pick(&fx, "r1.1", OLD), cause, ends);
    assert!(!fates(&fx).contains("accepted"), "{}", fates(&fx));
    approve_plan(&fx, "I approve the plan again.");
    refused_before("a8 --accept after the gate's approval, on the words written while it was open", &accept(&fx, "r1.1", OLD));
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("a8 --accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    close_ok(&fx, &["test", "-f", "src/a.txt"]);
}

// a8, rejected: the plan gate rejected is a hold too (bf next gives open-gate), and its hold ends with the
// approval of a later round.
#[test]
fn a_rejected_plan_gate_holds_the_decisions_until_it_is_opened_and_approved_again() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    gate_open_plan(&fx);
    fx.human("No, not this plan.");
    let o = fx.bf(&["gate", "reject", "plan", "--quote", "not this plan"]);
    say("gate reject plan", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(actions(&fx).iter().any(|a| a == "open-gate"), "{:?}", actions(&fx));
    fx.human(OLD);
    refused_by_hold("a8 --accept while the plan gate is rejected", &accept(&fx, "r1.1", OLD), "the plan gate of the run is rejected", "bf gate open plan");
    gate_open_plan(&fx);
    approve_plan(&fx, "I approve the corrected plan.");
    refused_before("a8 --accept after the approval, on the words written while the gate was rejected", &accept(&fx, "r1.1", OLD));
    fx.human(NEW);
    assert_eq!(accept(&fx, "r1.1", NEW).code, 0);
}

// a8, replay: the same accept appended through the library while the gate is open is listed as ignored, with
// the gate in the note; after the approval the old words are listed too.
#[test]
fn a_settle_appended_while_the_plan_gate_is_open_is_listed_with_the_gate_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    gate_open_plan(&fx);
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    let settle = |t: u64, q: &str| {
        bf::events::append(
            &fx.project(),
            "checkpoint.finding.settle",
            json!({"checkpoint": "K-1", "ids": [{"round": 1, "index": 1}], "fate": "accepted", "reason": "", "turn": t, "quote": q}),
        )
        .unwrap();
    };
    settle(t, OLD);
    let notes = ignored(&fx);
    eprintln!("ignored while the gate is open: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("a finding settled while the plan gate was open"), "{notes:?}");
    approve_plan(&fx, "I approve the plan again.");
    settle(t, OLD);
    let notes = ignored(&fx);
    eprintln!("ignored after the approval: {notes:?}");
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[1].contains("words written before the bound"), "{notes:?}");
    assert!(!fates(&fx).contains("accepted"), "{}", fates(&fx));
}

// The budget cap reached: bf next refuses, the five decisions are refused naming the cap, the words written
// then stay refused after the cap is raised, and a new message decides.
#[test]
fn words_written_while_the_budget_cap_is_reached_decide_nothing_during_it_or_after() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    // A role's reported cost takes the spending past a cap of 1.0.
    bf::events::append(&fx.project(), "unit.role-report", json!({"unit": "U-1", "role": "auditor", "cost": 2.0})).unwrap();
    fx.set_policy("cap = 0.0", "cap = 1.0");
    abort_fix(&fx);
    let o = fx.bf(&["next"]);
    say("bf next with the cap reached", &o);
    assert_eq!(o.code, 2, "{}", o.all());
    fx.human(OLD);
    let cause = "the budget cap is reached";
    let ends = "budget.cap";
    refused_by_hold("budget --accept", &accept(&fx, "r1.1", OLD), cause, ends);
    refused_by_hold("budget --decline", &decline(&fx, "r1.1", OLD), cause, ends);
    refused_by_hold("budget --fix-findings", &pick(&fx, "r1.1", OLD), cause, ends);
    assert!(!fates(&fx).contains("accepted"), "{}", fates(&fx));
    assert!(hold_events(&fx).iter().any(|(_, c)| c == "budget"), "{:?}", hold_events(&fx));
    fx.set_policy("cap = 1.0", "cap = 0.0");
    refused_before("budget --accept after the cap was raised, on the words written while it was reached", &accept(&fx, "r1.1", OLD));
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("budget --accept on a new message", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    close_ok(&fx, &["test", "-f", "src/a.txt"]);
}

// a1, a1b: switching the checkpoint review off for the run is a decision bound like the others. The proposal
// is made before the sync (a1) or during it (a1b); the words written during the sync are refused during it
// and after it, the close still refuses on the open finding, and new words after the sync switch it off.
fn review_off_flow(propose_during: bool) {
    let tag = if propose_during { "a1b" } else { "a1" };
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    let propose = |fx: &Fx| {
        let o = fx.bf(&["checks", "propose", "checkpoint-review=off:faster", "--scope", "run"]);
        say(&format!("{tag} propose the review off"), &o);
        assert_eq!(o.code, 0, "{}", o.all());
    };
    if !propose_during {
        propose(&fx);
    }
    open_conflicting_sync(&fx);
    if propose_during {
        propose(&fx);
    }
    fx.human(OLD);
    refused_by_hold(&format!("{tag} checks set during the sync"), &fx.bf(&["checks", "set", "--quote", OLD]), "a sync of main into K-1 is open", "bf checkpoint sync K-1 --abort");
    resolve_sync(&fx);
    refused_before(&format!("{tag} checks set after the sync, on the words written during it"), &fx.bf(&["checks", "set", "--quote", OLD]));
    fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    say(&format!("{tag} close with the high finding open"), &o);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.all().contains("r1.1"), "{}", o.all());
    fx.human(NEW);
    let o = fx.bf(&["checks", "set", "--quote", NEW]);
    say(&format!("{tag} checks set on a new message"), &o);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn the_review_switched_off_on_words_written_during_a_sync_is_refused_during_it_and_after() {
    review_off_flow(false);
}

#[test]
fn the_review_switched_off_by_a_proposal_made_during_a_sync_takes_no_words_from_it() {
    review_off_flow(true);
}

// a1, replay: the same `checks.set` appended through the library during the sync is listed as ignored with the
// sync in the note, and the review stays required.
#[test]
fn a_review_switch_off_appended_during_a_sync_is_listed_with_the_sync_note() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    let o = fx.bf(&["checks", "propose", "checkpoint-review=off:faster", "--scope", "run"]);
    assert_eq!(o.code, 0, "{}", o.all());
    open_conflicting_sync(&fx);
    fx.human(OLD);
    let t = turn_seq(&fx, OLD);
    let st = bf::events::replay_log(&bf::events::load_log(&fx.project()).unwrap());
    let prop = st.checks_proposals.last().unwrap().clone();
    bf::events::append(
        &fx.project(),
        "checks.set",
        json!({"scope": prop.scope, "proposal": prop.seq, "turn": t, "quote": OLD,
               "changes": bf::events::changes_to_json(&prop.changes), "run": prop.run}),
    )
    .unwrap();
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("the checkpoint review was switched off while a sync of main into K-1 was open"), "{notes:?}");
    let st = bf::events::replay_log(&bf::events::load_log(&fx.project()).unwrap());
    assert!(bf::ckreview::required(&st), "the review stays required");
}

// a3b: a low finding of round 1 is open and round 2 is at work after a sync that changed the content: bf next
// gives the low decision, and the human's words then settle it.
#[test]
fn the_low_decision_is_given_while_a_review_round_is_at_work() {
    let fx = setup(PASS);
    brief(&fx);
    fx.bf_stdin(&fx.work("K-1-review"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
    report(&fx, json!([f("low", "a small thing")]));
    fs::create_dir_all(fx.root.join("docs")).unwrap();
    fs::write(fx.root.join("docs/note.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "docs/note.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]);
    assert_eq!(o.code, 0, "{}", o.all());
    brief(&fx);
    let a = actions(&fx);
    eprintln!("next during round 2: {a:?}");
    assert!(a.iter().any(|x| x == "review-checkpoint"), "{a:?}");
    assert!(a.iter().any(|x| x == "decide-low-findings"), "{a:?}");
    fx.human(NEW);
    let o = accept(&fx, "r1.1", NEW);
    say("a3b --accept on words written after the low decision was given", &o);
    assert_eq!(o.code, 0, "{}", o.all());
}

// a7: a mark, the fix that changes the content, the fix's abort. The abort brings the mark back: the log holds
// the hold again before any other command, and words written after the abort stay refused.
#[test]
fn a_hold_an_abort_brings_back_is_logged_before_the_next_command() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    eprintln!("next (the fix opens): {:?}", actions(&fx));
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "fail", "X-2", "--reason", "the service is down"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    commit_fix(&fx, "from the unit, changed\n");
    eprintln!("next after the fix changed the content: {:?}", actions(&fx));
    let before = hold_events(&fx);
    assert_eq!(before.last().map(|x| x.1.as_str()), Some(""), "the mark ended with the content: {before:?}");
    abort_fix(&fx);
    let abort = last_seq_of(&fx, "checkpoint.fix.abort");
    let after = hold_events(&fx);
    eprintln!("holds {after:?}, abort at {abort}");
    let last = after.last().unwrap();
    assert!(last.0 > abort && last.1 == "marked", "the abort brought the mark back and the log holds it: {after:?}");
    fx.human(OLD);
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    refused_before("a7 --accept on words written after the abort, then the verifier's passing run", &accept(&fx, "r1.1", OLD));
    fx.human(NEW);
    assert_eq!(accept(&fx, "r1.1", NEW).code, 0);
}

// ---------------------------------------------------------------------------------------------------------
// The closure table against behaviour: for each holding row a scenario in which `bf next` gives the action
// and a decision is refused with that row's Hold case; for each asking row a scenario in which it is given and
// a decision is taken.
// ---------------------------------------------------------------------------------------------------------

/// A plan with a wave check that fails, the first wave landed; a high finding of K-1 filed and its fix aborted.
fn wave_scenario() -> Fx {
    let fx = Fx::new();
    fx.human(OLD0);
    plan(&fx, PASS);
    let r = fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"wave\"\nhow=\"command\"\ncommand=\"grep -q ok src/a.txt\"\nproves=[]\nscope=\"wave\"\n");
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    fx.commit_all("wave check");
    fx.must(&["checks", "propose", "checkpoint-review=on", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it", "--scope", "default"]);
    fx.human("Yes, set the checkpoint review that way.");
    fx.must(&["checks", "set", "--quote", "set the checkpoint review that way"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.must(&["checks", "wave", "propose", "override", "grep -q ok src/a.txt"]);
    fx.human("Use that wave check.");
    fx.must(&["checks", "wave", "set", "--quote", "Use that wave check"]);
    fx.must(&["checks", "unit", "propose", "none"]);
    fx.human("No checks for every unit in this run.");
    fx.must(&["checks", "unit", "set", "--quote", "No checks for every unit"]);
    let wc = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    fx
}

/// The scenario in which `bf next` gives `action`, with the high finding r1.1 of K-1 open.
fn scenario(action: &str) -> Fx {
    let fx = match action {
        "decide-gate" | "open-gate" | "add-records" => {
            let fx = setup(PASS);
            report(&fx, json!([f("high", "the answer is wrong")]));
            abort_fix(&fx);
            gate_open_plan(&fx);
            match action {
                "open-gate" => {
                    fx.human("No, not this plan.");
                    fx.must(&["gate", "reject", "plan", "--quote", "not this plan"]);
                }
                "add-records" => fx.set_policy("in_order = false", "in_order = true"),
                _ => {}
            }
            fx
        }
        "resolve-sync" | "review-sync" | "decide-sync" | "judge-sync-block" => {
            let fx = setup(PASS);
            if action == "decide-sync" {
                fx.set_policy("max_rounds = 3", "max_rounds = 1");
            }
            report(&fx, json!([f("high", "the answer is wrong")]));
            abort_fix(&fx);
            open_conflicting_sync(&fx);
            let k = fx.work("K-1");
            match action {
                "review-sync" => {
                    fs::write(k.join("src/a.txt"), "from the unit\nfrom main\n").unwrap();
                    git(&k, &["add", "src/a.txt"]);
                    git(&k, &["commit", "-q", "--no-edit"]);
                    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "builder", "--result", "built"], "");
                    assert_eq!(o.code, 0, "{}", o.all());
                }
                "decide-sync" => {
                    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "builder", "--result", "blocked"], "");
                    assert_eq!(o.code, 0, "{}", o.all());
                }
                "judge-sync-block" => {
                    let o = fx.bf_stdin(&k, &["report", "K-1", "--role", "builder", "--result", "blocked", "--note", "which line is right"], "");
                    assert_eq!(o.code, 0, "{}", o.all());
                }
                _ => {}
            }
            fx
        }
        "fix-checkpoint" | "review-checkpoint-fix" | "plan-gap" => {
            let fx = setup(PASS);
            report(&fx, json!([f("high", "the answer is wrong")]));
            actions(&fx);
            match action {
                "review-checkpoint-fix" => fix_built(&fx, "from the unit, fixed\n"),
                "plan-gap" => {
                    let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "builder", "--result", "gap", "--note", "src/needs.txt: needs behaviour no record describes"], "");
                    assert_eq!(o.code, 2, "{}", o.all());
                }
                _ => {}
            }
            fx
        }
        "fix-wave" | "review-wave-fix" | "decide-wave-check" => {
            let fx = wave_scenario();
            // A sync of main changes the content, so the aborted fix no longer bars a fix at the new one.
            fs::create_dir_all(fx.root.join("docs")).unwrap();
            fs::write(fx.root.join("docs/note.txt"), "work on main\n").unwrap();
            git(&fx.root, &["add", "docs/note.txt"]);
            git(&fx.root, &["commit", "-q", "-m", "work on main"]);
            fx.human("Yes, bring main in now.");
            let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "bring main in"]);
            assert_eq!(o.code, 0, "{}", o.all());
            let o = fx.bf(&["checkpoint", "wave-check", "K-1", "--wave", "1"]);
            say("the wave check runs (and fails)", &o);
            actions(&fx);
            match action {
                "review-wave-fix" => fix_built(&fx, "ok\n"),
                "decide-wave-check" => abort_fix(&fx),
                _ => {}
            }
            fx
        }
        "decide-check" => {
            let fx = setup(PASS);
            report(&fx, json!([f("high", "the answer is wrong")]));
            abort_fix(&fx);
            let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "fail", "X-2", "--reason", "the service is down"], "");
            assert_eq!(o.code, 0, "{}", o.all());
            fx
        }
        "answer-question" => {
            let fx = setup(PASS);
            report(&fx, json!([f("high", "the answer is wrong")]));
            abort_fix(&fx);
            let o = fx.bf(&["ask", "--unit", "K-1", "--title", "Which of the two should win?"]);
            assert_eq!(o.code, 0, "{}", o.all());
            fx
        }
        "decide-checkpoint-review" => {
            let fx = setup(PASS);
            fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
            report(&fx, json!([f("high", "the answer is wrong")]));
            fx
        }
        "decide-checkpoint-fix" => {
            let fx = setup(PASS);
            report(&fx, json!([f("high", "the answer is wrong")]));
            actions(&fx);
            abort_fix(&fx);
            fx
        }
        "go-on-checkpoint-fix" => {
            // The fix's review filed a low finding, the human accepted it, and the process stopped before
            // the checks ran: only the accept is on the log, and the review is settled.
            let fx = setup(PASS);
            report(&fx, json!([f("high", "the answer is wrong")]));
            actions(&fx);
            fix_built(&fx, "from the unit, fixed\n");
            let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"], &json!([f("low", "a small thing")]).to_string());
            assert_eq!(o.code, 0, "{}", o.all());
            fx.human("Fine, accept the small thing.");
            let turn = last_seq_of(&fx, "human.turn");
            bf::events::append(&fx.project(), "checkpoint.fix.finding.settle", json!({"checkpoint": "K-1", "fate": "accepted",
                "ids": [{"round": 1, "index": 1}], "levels": ["low"], "turn": turn, "quote": "Fine, accept the small thing."})).unwrap();
            fx
        }
        "decide-low-findings" => {
            let fx = setup(PASS);
            brief(&fx);
            fx.bf_stdin(&fx.work("K-1-review"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
            report(&fx, json!([f("low", "a small thing")]));
            fx
        }
        other => panic!("no scenario for `{other}`: write one for its row of the table"),
    };
    let given = actions(&fx);
    eprintln!("scenario `{action}`: bf next gives {given:?}");
    assert!(given.iter().any(|a| a == action), "bf next gives {given:?}, not `{action}`");
    fx
}

/// What a refusal says for each Hold case.
fn phrase(case: &str) -> &'static str {
    match case {
        "Sync" => "a sync of main into K-1 is open",
        "Fix" => "a checkpoint fix is at work on K-1",
        "Marked" => "is marked failed or unreachable",
        "Question" => "a worker's question on K-1",
        "Wave" => "a wave check holds the close of K-1",
        "Run" => "the plan gate of the run is",
        other => panic!("no wording for the Hold case `{other}`"),
    }
}

/// One row of the table against behaviour. A holding row: in its scenario a decision is refused with the row's
/// Hold case. An asking row: in its scenario a decision is taken. A row of the class "neither" is not judged.
fn check_row(action: &str) {
    let class = CLASSES.iter().find(|(a, _)| *a == action).map(|(_, c)| *c).unwrap_or_else(|| panic!("`{action}` is not in the table"));
    let fx = scenario(action);
    if let Some(case) = class.strip_prefix("holding:") {
        fx.human(OLD);
        let o = accept(&fx, "r1.1", OLD);
        eprintln!("holding row `{action}` ({case}): --accept on words written while it is given: exit {} | {}", o.code, o.stderr.lines().take(4).collect::<Vec<_>>().join(" | "));
        assert_eq!(o.code, 2, "`{action}`: {}", o.all());
        assert!(o.all().contains("the human is not being asked about the findings of K-1"), "`{action}`: {}", o.all());
        assert!(o.all().contains(phrase(case)), "`{action}` is classified holding:{case}, but the refusal says: {}", o.all());
        assert!(!fates(&fx).contains("accepted"), "`{action}`: {}", fates(&fx));
    } else {
        assert_eq!(class, "asking", "`{action}` is neither asking nor holding: it has no behaviour to check");
        fx.human(NEW);
        let ids = if action == "decide-low-findings" { "low" } else { "r1.1" };
        let o = accept(&fx, ids, NEW);
        eprintln!("asking row `{action}`: --accept on words written while it is given: exit {}", o.code);
        assert_eq!(o.code, 0, "`{action}` is classified asking, but a decision is refused: {}", o.all());
        assert!(fates(&fx).contains("accepted"), "`{action}`: {}", fates(&fx));
    }
}

/// The rows that have a test of their own below; every holding and asking row of the table must be here.
const ROW_TESTS: &[&str] = &[
    "decide-checkpoint-review", "decide-checkpoint-fix", "decide-low-findings", "decide-sync", "judge-sync-block", "resolve-sync", "review-sync",
    "fix-checkpoint", "review-checkpoint-fix", "fix-wave", "review-wave-fix", "plan-gap", "decide-check", "answer-question",
    "decide-wave-check", "decide-gate", "open-gate", "add-records",
    "go-on-checkpoint-fix",
];

macro_rules! row_test {
    ($name:ident, $action:expr) => {
        #[test]
        fn $name() {
            check_row($action);
        }
    };
}

row_test!(row_decide_checkpoint_review_asks, "decide-checkpoint-review");
row_test!(row_decide_checkpoint_fix_asks, "decide-checkpoint-fix");
row_test!(row_decide_low_findings_asks, "decide-low-findings");
row_test!(row_decide_sync_holds, "decide-sync");
row_test!(row_judge_sync_block_holds, "judge-sync-block");
row_test!(row_resolve_sync_holds, "resolve-sync");
row_test!(row_review_sync_holds, "review-sync");
row_test!(row_fix_checkpoint_holds, "fix-checkpoint");
row_test!(row_review_checkpoint_fix_holds, "review-checkpoint-fix");
row_test!(row_go_on_checkpoint_fix_holds, "go-on-checkpoint-fix");
row_test!(row_fix_wave_holds, "fix-wave");
row_test!(row_review_wave_fix_holds, "review-wave-fix");
row_test!(row_plan_gap_holds, "plan-gap");
row_test!(row_decide_check_holds, "decide-check");
row_test!(row_answer_question_holds, "answer-question");
row_test!(row_decide_wave_check_holds, "decide-wave-check");
row_test!(row_decide_gate_holds, "decide-gate");
row_test!(row_open_gate_holds, "open-gate");
row_test!(row_add_records_holds, "add-records");

#[test]
fn every_asking_and_holding_row_of_the_table_has_its_behaviour_test() {
    for (a, c) in CLASSES {
        if *c != "neither" {
            assert!(ROW_TESTS.contains(a), "`{a}` is classified `{c}`: add a scenario for it and a row test, so its class is checked against behaviour");
        }
    }
    for a in ROW_TESTS {
        assert!(CLASSES.iter().any(|(t, _)| t == a), "{a}");
    }
}

// K2R7-7: each decision command observes the holds before it judges. A mark that stood and no longer stands
// because the content changed with no event (a commit in the checkpoint's copy) is ended in the log only by
// the command's own look: the words written after the change are then refused as written before the end. A
// command that did not look would find the old mark and refuse for it instead.
fn mark_then_change_the_content(fx: &Fx) {
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "fail", "X-2", "--reason", "the service is down"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(hold_events(fx).last().map(|x| x.1.clone()), Some("marked".to_string()));
    commit_fix(fx, "from the unit, changed\n");
}

#[test]
fn accept_decline_and_pick_look_at_the_holds_before_they_judge() {
    for what in ["accept", "decline", "pick"] {
        let fx = setup(PASS);
        report(&fx, json!([f("high", "the answer is wrong")]));
        abort_fix(&fx);
        mark_then_change_the_content(&fx);
        fx.human(OLD);
        let o = match what {
            "accept" => accept(&fx, "r1.1", OLD),
            "decline" => decline(&fx, "r1.1", OLD),
            _ => pick(&fx, "r1.1", OLD),
        };
        eprintln!("{what}: the mark ended with the content, no event logged it");
        refused_before(&format!("{what} on words written after the content changed"), &o);
        assert_eq!(hold_events(&fx).last().map(|x| x.1.clone()), Some(String::new()), "the command logged the end: {:?}", hold_events(&fx));
    }
}

#[test]
fn the_review_release_looks_at_the_holds_before_it_judges() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    mark_then_change_the_content(&fx);
    fx.human("Release it, please.");
    refused_before("review --release on words written after the content changed", &release_review(&fx, "Release it, please."));
}

#[test]
fn the_fix_release_looks_at_the_holds_before_it_judges() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    actions(&fx);
    let o = fx.bf_stdin(&fx.work("K-1"), &["report", "K-1", "--role", "builder", "--result", "blocked", "--note", "stuck"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(actions(&fx), vec!["decide-checkpoint-fix"]);
    mark_then_change_the_content(&fx);
    fx.human("Try it again, please.");
    let o = fx.bf(&["checkpoint", "fix", "K-1", "--release", "--quote", "Try it again, please."]);
    refused_before("fix --release on words written after the content changed", &o);
}

// K2R6-3 (own_turn): a review release appended right after a fix that a pick opened on the human's own words
// is not that release's own fix, so the open fix holds it: ignored, with the fix in the note.
#[test]
fn a_review_release_appended_right_after_a_fix_picked_in_the_humans_words_is_ignored() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    // Both messages are written first, so the release is the very next event after the pick's fix.
    fx.human(OLD);
    fx.human("Release it, please.");
    let t = turn_seq(&fx, OLD);
    let t2 = turn_seq(&fx, "Release it, please.");
    bf::events::append(
        &fx.project(),
        "checkpoint.fix.open",
        json!({"checkpoint": "K-1", "source": "review", "findings": [f("high", "the answer is wrong")],
               "finding_ids": [{"round": 1, "index": 1}], "before": [],
               "human": {"turn": t, "quote": OLD}}),
    )
    .unwrap();
    bf::events::append(
        &fx.project(),
        "checkpoint.review.release",
        json!({"checkpoint": "K-1", "after_round": 1, "turn": t2, "quote": "Release it, please."}),
    )
    .unwrap();
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("review release") && notes[0].contains("a checkpoint fix was at work"), "{notes:?}");
}

// The refusal of a review switch-off names the checkpoint whose decision is held, not the first one with an
// open finding: with two checkpoints open and a hold on the second only, the failing one is the second.
#[test]
fn the_review_switch_off_bound_names_the_checkpoint_whose_hold_refuses() {
    let fx = setup(PASS);
    report(&fx, json!([f("high", "the answer is wrong")]));
    abort_fix(&fx);
    let p = fx.project();
    bf::events::append(&p, "checkpoint.hold", json!({"checkpoint": "K-2", "cause": "question", "detail": "Q-1"})).unwrap();
    let st = bf::events::replay_log(&bf::events::load_log(&p).unwrap());
    let open = vec!["K-1".to_string(), "K-2".to_string()];
    let r = bf::events::review_off_bound(&st, &open, 1, u64::MAX);
    eprintln!("bound over K-1 and K-2 with a hold on K-2: {r:?}");
    match r {
        Err((k, h)) => {
            assert_eq!(k, "K-2");
            assert_eq!(h.checkpoint(), "K-2");
            assert!(h.because().contains("K-2"), "{}", h.because());
        }
        Ok(b) => panic!("no refusal, bound {b}"),
    }
    assert!(bf::events::review_off_bound(&st, &open[..1], 1, u64::MAX).is_ok(), "K-1 alone has no hold");
}

/// A builder's edit of `rel` in the checkpoint's working copy, judged by the guard as the hook calls it.
fn builder_edit_allowed(fx: &Fx, rel: &str) -> bool {
    let wc = fx.work("K-1");
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Edit",
        "cwd": wc.to_string_lossy(),
        "agent_type": "builder",
        "tool_input": {"file_path": wc.join(rel).to_string_lossy(), "old_string": "a", "new_string": "b"},
    });
    let root = fx.root.to_string_lossy().to_string();
    let o = fx.bf_stdin(&fx.root, &["guard", "--root", &root], &payload.to_string());
    !o.stdout.contains("\"deny\"")
}

// A fix opened by the release of a parked review hands its files to the guard: the overlay is written after
// the release event, so the fix's builder may edit the files of the checkpoint's units. A `bf next` puts an
// overlay that is missing back.
#[test]
fn a_fix_opened_by_a_review_release_lets_its_builder_write() {
    let fx = setup(PASS);
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    report(&fx, json!([f("high", "the answer is wrong")]));
    fx.human("Release it, please.");
    let o = release_review(&fx, "Release it, please.");
    say("release the parked review", &o);
    assert_eq!(o.code, 0, "{}", o.all());
    let overlay = fx.state_dir().join("ckfix.json");
    let body: Value = serde_json::from_str(&fs::read_to_string(&overlay).expect("the overlay is written")).unwrap();
    assert_eq!(body["body"]["checkpoint"], "K-1", "{body}");
    assert!(builder_edit_allowed(&fx, "src/a.txt"), "the fix's builder may edit a file of K-1's units");
    // An overlay that went missing is put back by the next read of the directives.
    fs::remove_file(&overlay).unwrap();
    assert!(!builder_edit_allowed(&fx, "src/a.txt"), "with no overlay the edit is refused");
    fx.must(&["next", "--json"]);
    assert!(overlay.exists(), "bf next writes the overlay again");
    assert!(builder_edit_allowed(&fx, "src/a.txt"), "and the edit is allowed again");
}
