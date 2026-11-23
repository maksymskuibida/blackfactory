// D-110: the human accepts a failed or unreachable check of a checkpoint in their own words; a check marked
// unreachable is offered to the verifier again; the note of `decide-unit` holds no raw refusal word.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const OLD0: &str = "Some early words, before the plan.";
const OLD: &str = "Looks good to me, keep going.";
const NEW: &str = "Fine, I have thought about it: accept it.";
const PASS: &str = "test -f src/a.txt";

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
    fx.must(&["checks", "propose", "checkpoint-review=off:the test does not exercise it", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it", "--scope", "default"]);
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


fn commit_fix(fx: &Fx, text: &str) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), text).unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fix"]);
}

fn mark(fx: &Fx, how: &str) {
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", how, "X-2", "--reason", "the service is down"], "");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn next(fx: &Fx) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"].as_array().unwrap().clone()
}

fn actions(fx: &Fx) -> Vec<String> {
    next(fx).iter().map(|d| d["action"].as_str().unwrap_or("").to_string()).collect()
}

fn accept(fx: &Fx, ids: &str, quote: &str) -> Out {
    fx.bf(&["checkpoint", "check", "K-1", "--accept", ids, "--quote", quote])
}

fn ignored(fx: &Fx) -> Vec<String> {
    let s = fx.must(&["status", "--json"]).json();
    s["ignored_events"].as_array().unwrap().iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()
}

fn log_events(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.project().log_path()).unwrap().lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect()
}

fn kinds(fx: &Fx) -> Vec<String> {
    log_events(fx).iter().map(|e| e["kind"].as_str().unwrap_or("").to_string()).collect()
}

fn marked(how: &str) -> Fx {
    let fx = setup(PASS);
    mark(&fx, how);
    let a = actions(&fx);
    assert!(a.contains(&"decide-check".to_string()), "{a:?}");
    fx
}

#[test]
fn the_accept_is_refused_on_words_written_before_the_directive() {
    let fx = setup(PASS);
    fx.human(OLD);
    mark(&fx, "unreachable");
    assert!(actions(&fx).contains(&"decide-check".to_string()));
    let o = accept(&fx, "X-2", OLD);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("written before"), "{}", o.all());
    assert!(!kinds(&fx).contains(&"checkpoint.check.accept".to_string()));
}

#[test]
fn the_accept_is_refused_from_a_working_copy() {
    let fx = marked("unreachable");
    fx.human(NEW);
    let o = fx.bf_in(&fx.work("K-1"), &["checkpoint", "check", "K-1", "--accept", "X-2", "--quote", NEW]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(!kinds(&fx).contains(&"checkpoint.check.accept".to_string()));
}

#[test]
fn the_accept_is_refused_while_the_human_absent_flag_is_on() {
    let fx = marked("unreachable");
    let payload = json!({"session_id": "s1", "cwd": fx.root.to_string_lossy(), "permission_mode": "default",
        "hook_event_name": "UserPromptSubmit", "prompt": "/factory:human-absent"});
    fx.bf_stdin(&fx.root, &["human-turn"], &payload.to_string());
    assert_eq!(fx.must(&["absent", "status", "--json"]).json()["on"], true);
    fx.human(NEW);
    let o = accept(&fx, "X-2", NEW);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("absent"), "{}", o.all());
    assert!(!kinds(&fx).contains(&"checkpoint.check.accept".to_string()));
}

#[test]
fn the_accept_is_refused_for_a_check_with_no_mark_and_for_a_closed_checkpoint() {
    let fx = setup(PASS);
    fx.human(NEW);
    let o = accept(&fx, "X-2", NEW);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("no failed or unreachable mark"), "{}", o.all());
    let o = fx.bf_stdin(&fx.work("K-1"), &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"], "");
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "close", "K-1"]);
    fx.human("Fine, accept it after the close, I say.");
    let o = accept(&fx, "X-2", "accept it after the close");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("closed"), "{}", o.all());
}

#[test]
fn after_an_accept_the_checkpoint_is_not_held_and_closes_with_the_words_in_its_event() {
    for how in ["unreachable"] {
        let fx = marked(how);
        let units_before = fx.must(&["status", "--json"]).json()["units"].clone();
        fx.human(NEW);
        let o = accept(&fx, "X-2", NEW);
        assert_eq!(o.code, 0, "{}", o.all());
        let acts = actions(&fx);
        assert!(!acts.contains(&"decide-check".to_string()), "{acts:?}");
        assert!(!kinds(&fx).iter().any(|k| k.starts_with("amend.")), "{:?}", kinds(&fx));
        assert_eq!(fx.must(&["status", "--json"]).json()["units"], units_before);
        assert!(acts.contains(&"close-checkpoint".to_string()), "{acts:?}");
        let c = fx.must(&["checkpoint", "close", "K-1"]);
        assert!(c.stdout.contains("accepted by the human") && c.stdout.contains(NEW), "{}", c.all());
        let ev = log_events(&fx).into_iter().find(|e| e["kind"] == "checkpoint.close").unwrap();
        let a = &ev["accepted_checks"][0];
        assert_eq!(a["check"], "X-2");
        assert_eq!(a["mark"], how_mark(how));
        assert_eq!(a["quote"], NEW);
        assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
        assert_eq!(fx.bf(&["log", "--verify"]).code, 0);
    }
}

fn how_mark(how: &str) -> &'static str {
    if how == "fail" { "fail" } else { "unreachable" }
}

#[test]
fn a_new_commit_on_the_checkpoint_ends_the_accept() {
    let fx = marked("unreachable");
    fx.human(NEW);
    assert_eq!(accept(&fx, "X-2", NEW).code, 0);
    commit_fix(&fx, "from the unit, changed\n");
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    mark(&fx, "unreachable");
    assert!(actions(&fx).contains(&"decide-check".to_string()));
}

#[test]
fn with_only_unreachable_marks_next_holds_both_directives_and_the_verifiers_has_load_and_cores() {
    let fx = marked("unreachable");
    let ds = next(&fx);
    let d = ds.iter().find(|d| d["action"] == "decide-check").expect("decide-check");
    assert!(d["note"].as_str().unwrap().contains("bf checkpoint check K-1 --accept"), "{d}");
    let v = ds.iter().find(|d| d["action"] == "run-checkpoint-checks").expect("the verifier's directive");
    assert_eq!(v["role"], "verifier");
    assert_eq!(v["checks"], json!(["X-2"]));
    assert!(v["load"].is_number() && v["cores"].as_u64().unwrap() > 0, "{v}");
}

#[test]
fn the_decide_unit_note_prints_a_reason_for_each_refusal_word() {
    for w in ["retry", "retry_spent", "extension", "other"] {
        let t = bf::strings::t(&format!("check_accept.refusal.{w}"), &[]);
        assert!(!t.is_empty(), "{w}");
        assert!(!t.contains('_'), "{t}");
    }
}

#[test]
fn a_failed_mark_with_a_fix_at_work_is_not_accepted_until_the_fix_is_parked() {
    let fx = setup(PASS);
    mark(&fx, "fail");
    assert!(actions(&fx).contains(&"fix-checkpoint".to_string()));
    fx.human(NEW);
    let o = accept(&fx, "X-2", NEW);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("held first"), "{}", o.all());
    assert!(!kinds(&fx).contains(&"checkpoint.check.accept".to_string()));
}
