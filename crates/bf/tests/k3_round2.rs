//! The fixes of the K-3 checkpoint review, round 2, as permanent tests: the plan in force is what the
//! human approved, never a draft, never a record changed after its approval, never what a withdrawn
//! amendment left; and a forged answer of the human to a design review counts for nothing.

mod common;
use bf::checks::review;
use bf::events;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const X1: &str = "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n";
const K1: &str = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[]\n";

fn put_x(fx: &Fx, text: &str) -> Out {
    let f = fx.root.join("x.toml");
    fs::write(&f, text).unwrap();
    let kind = if text.contains("kind=\"checkpoint\"") {
        "checkpoint"
    } else {
        "check"
    };
    let o = fx.bf(&[
        "record",
        "put",
        kind,
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let _ = fs::remove_file(&f);
    o
}

fn start() -> (Fx, PathBuf) {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    (fx, wc)
}

/// Report built and reviewed, then land; the landing's outcome.
fn land(fx: &Fx) -> Out {
    fx.bf(&["report", "U-1", "--result", "built"]);
    fx.bf(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.bf(&["unit", "land", "U-1"])
}

/// Open the repair `n` of X-1, put `text`, narrow it and let the human approve it.
fn approve_repair(fx: &Fx, n: &str, text: &str) {
    let opened = fx
        .must(&["amend", "open", "--repair", "X-1", "--json"])
        .json();
    let put = put_x(fx, text);
    assert_eq!(put.code, 0, "{}", put.all());
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": n, "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, approve the new command.");
    fx.must(&["amend", "approve", n, "--quote", "approve the new command"]);
}

/// U-1 with two checks, X-1 and X-4, started and ready for work.
fn two_checks() -> (Fx, PathBuf) {
    let fx = Fx::new();
    fx.tiny();
    let f = fx.root.join("x4.toml");
    fs::write(&f, "id=\"X-4\"\nkind=\"check\"\ntitle=\"Folder\"\nhow=\"command\"\ncommand=\"test -d src\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n").unwrap();
    let o = fx.bf(&["record", "put", "check", "--file", f.to_str().unwrap()]);
    let _ = fs::remove_file(&f);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("x4");
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    (fx, wc)
}

fn moved_check_holds_landing(draft: &str) {
    let (fx, wc) = two_checks();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let put = put_x(&fx, draft);
    assert_eq!(put.code, 0, "{}", put.all());
    let run = fx.bf_in(
        &wc,
        &[
            "evidence", "run", "X-4", "--unit", "U-1", "--", "test", "-d", "src",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.all());
    let l = land(&fx);
    assert!(l.code != 0, "the unit landed without X-1: {}", l.all());
    assert!(
        l.all().contains("X-1"),
        "the refusal does not name X-1: {}",
        l.all()
    );
    assert!(!wc.join("src/hello.txt").exists());
}

/// q7: a check moved to another unit by a draft still holds the landing of its unit, whatever other
/// checks the unit has.
#[test]
fn q7_a_check_moved_away_by_a_draft_still_holds_the_landing_of_its_unit() {
    moved_check_holds_landing(&X1.replace("unit=\"U-1\"", "unit=\"U-2\""));
}

/// q7b: nor does a check moved to checkpoint scope.
#[test]
fn q7b_a_check_moved_to_checkpoint_scope_still_holds_the_landing() {
    moved_check_holds_landing(
        &X1.replace("scope=\"unit\"\nunit=\"U-1\"\n", "scope=\"checkpoint\"\n"),
    );
}

/// q4: a record covered by an approved amendment is not changed again with no amendment open.
#[test]
fn q4_a_record_changed_after_its_approved_amendment_is_a_draft() {
    let (fx, wc) = start();
    approve_repair(&fx, "A-1", &X1.replace("test -f", "test -s"));
    let put = put_x(&fx, &X1.replace("test -f src/hello.txt", "true"));
    assert!(
        put.code != 0,
        "the record was changed with no amendment open: {}",
        put.all()
    );
    // Whatever reached the disk by another way does not count either.
    let p = fx.project();
    let mut rec = p.get_record("X-1").unwrap().unwrap();
    rec["command"] = json!("true");
    p.write_record(&rec).unwrap();
    let run = fx.bf_in(
        &wc,
        &["evidence", "run", "X-1", "--unit", "U-1", "--", "true"],
    );
    assert!(
        run.code != 0,
        "evidence ran on a record the human did not approve: {}",
        run.all()
    );
    let l = land(&fx);
    assert!(l.code != 0, "the unit landed on it: {}", l.all());
    // The content the human approved can be put back with no amendment.
    let back = put_x(&fx, &X1.replace("test -f", "test -s"));
    assert_eq!(back.code, 0, "{}", back.all());
}

/// q3: a second amendment that names a record it leaves as the first one approved does not make it a draft.
#[test]
fn q3_a_second_amendment_naming_an_unchanged_record_leaves_it_in_force() {
    let (fx, wc) = start();
    approve_repair(&fx, "A-1", &X1.replace("test -f", "test -s"));
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let run = fx.bf_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-1",
            "--unit",
            "U-1",
            "--",
            "test",
            "-s",
            "src/hello.txt",
        ],
    );
    assert_eq!(
        run.code,
        0,
        "the record as approved was refused: {}",
        run.all()
    );
    // Putting back the content of the plan gate over what the first amendment approved is a draft.
    let put = put_x(&fx, X1);
    assert_eq!(put.code, 0, "{}", put.all());
    let run = fx.bf_in(
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
            "src/hello.txt",
        ],
    );
    assert!(
        run.code != 0,
        "the gate's content over the approved one counted: {}",
        run.all()
    );
}

/// q2: what a withdrawn amendment left changed does not count for evidence or landing.
#[test]
fn q2_a_withdrawn_amendment_leaves_no_force_to_its_changes() {
    let (fx, wc) = start();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let put = put_x(&fx, &X1.replace("test -f src/hello.txt", "true"));
    assert_eq!(put.code, 0, "{}", put.all());
    fx.must(&["amend", "withdraw", "A-1", "--reason", "no"]);
    let run = fx.bf_in(
        &wc,
        &["evidence", "run", "X-1", "--unit", "U-1", "--", "true"],
    );
    assert!(
        run.code != 0,
        "the changed command counted after the withdrawal: {}",
        run.all()
    );
    let l = land(&fx);
    assert!(l.code != 0, "{}", l.all());
    // The content of the plan is put back with no amendment, and the unit works again.
    let back = put_x(&fx, X1);
    assert_eq!(back.code, 0, "{}", back.all());
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    let run = fx.bf_in(
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
            "src/hello.txt",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.all());
}

fn built_both() -> Fx {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx
}

/// q6: a checkpoint record that an unapproved amendment changed is a draft: it closes nothing.
#[test]
fn q6_a_checkpoint_draft_that_drops_its_check_closes_nothing() {
    let fx = built_both();
    fx.must(&["amend", "open", "--repair", "K-1"]);
    let put = put_x(&fx, K1);
    assert_eq!(put.code, 0, "{}", put.all());
    let o = fx.bf(&["checkpoint", "close", "K-1", "--json"]);
    assert!(o.code != 0, "{}", o.all());
    assert!(
        o.all().contains("K-1") && o.all().contains("not approved"),
        "{}",
        o.all()
    );
}

fn setup() -> Fx {
    let fx = Fx::with_checks();
    fx.tiny();
    fx
}

/// q9: a release, an accept or a lift that cites no valid message does not release, accept or lift.
#[test]
fn q9_forged_answers_do_not_release_accept_or_lift() {
    let fx = setup();
    let p = fx.project();
    events::append(
        &p,
        review::PARK_EVENT,
        json!({"subject": "requirements", "round": 3}),
    )
    .unwrap();
    assert!(review::parked(&p, "requirements").unwrap());
    events::append(
        &p,
        review::RELEASE_EVENT,
        json!({"subject": "requirements", "turn": 9999, "quote": "never said"}),
    )
    .unwrap();
    let st = events::replay(&p).unwrap();
    assert!(
        st.ignored
            .iter()
            .any(|n| n.contains("design-review.release")),
        "{:?}",
        st.ignored
    );
    assert!(
        review::parked(&p, "requirements").unwrap(),
        "a forged release released the review"
    );
    // A finding recorded as accepted with a forged accept stays open.
    let o = fx.put("finding", "id=\"F-1\"\nkind=\"finding\"\ntitle=\"T\"\ngate=\"requirements\"\nseverity=\"high\"\naction=\"Change it\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    let mut rec = p.get_record("F-1").unwrap().unwrap();
    rec["outcome"] = json!("accepted");
    p.write_record(&rec).unwrap();
    events::append(
        &p,
        review::ACCEPT_EVENT,
        json!({"subject": "requirements", "ids": ["F-1"], "turn": 9999, "quote": "never said"}),
    )
    .unwrap();
    let f = review::findings(&p, "requirements").unwrap();
    assert_eq!(f.iter().find(|f| f.id == "F-1").unwrap().outcome, "open");
    // A forged lift does not lift: a real message can still lift the rule.
    events::append(
        &p,
        "designer.lift",
        json!({"subject": "requirements", "turn": 9999, "message": "m", "quote": "never said"}),
    )
    .unwrap();
    fx.human("Go on, draft it yourself, no designer needed.");
    let o = fx.bf(&[
        "roles",
        "lift",
        "requirements",
        "--quote",
        "draft it yourself",
    ]);
    assert_eq!(o.code, 0, "a forged lift counted: {}", o.all());
}

/// q9b: words written before the park do not release it.
#[test]
fn q9b_words_written_before_the_park_do_not_release_it() {
    let fx = setup();
    let p = fx.project();
    fx.human("I read it all, start the review over.");
    let st = events::replay(&p).unwrap();
    let seq = st.turns.last().unwrap().seq;
    events::append(
        &p,
        review::PARK_EVENT,
        json!({"subject": "requirements", "round": 3}),
    )
    .unwrap();
    events::append(
        &p,
        review::RELEASE_EVENT,
        json!({"subject": "requirements", "turn": seq, "quote": "start the review over"}),
    )
    .unwrap();
    assert!(
        review::parked(&p, "requirements").unwrap(),
        "words from before the park released it"
    );
    let _: Value = json!(null);
}

/// A record an amendment named before it existed is put while the amendment waits; the approval then
/// binds what was put, and a later change needs an amendment.
#[test]
fn a_record_named_before_it_existed_is_put_while_the_amendment_waits() {
    let (fx, _wc) = start();
    let x9 = |cmd: &str| {
        format!("id=\"X-9\"\nkind=\"check\"\ntitle=\"Folder\"\nhow=\"command\"\ncommand=\"{cmd}\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n")
    };
    let opened = fx
        .must(&["amend", "open", "--repair", "X-9", "--json"])
        .json();
    // The record is put while the amendment waits for the human.
    let o = put_x(&fx, &x9("test -d src"));
    assert_eq!(o.code, 0, "{}", o.all());
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, add the new check.");
    fx.must(&["amend", "approve", "A-1", "--quote", "add the new check"]);
    // The approval binds the content: the run is not frozen, a unit starts, and the same content is put again.
    let d = fx.bf(&["doctor"]);
    assert!(
        !d.all().contains("frozen records: PROBLEM"),
        "the run is frozen after the put: {}",
        d.all()
    );
    let o = fx.bf(&["unit", "start", "U-2"]);
    assert!(
        !o.all().contains("was changed after the plan gate"),
        "the plan is frozen after the put: {}",
        o.all()
    );
    let o = put_x(&fx, &x9("test -d src"));
    assert_eq!(o.code, 0, "{}", o.all());
    let o = put_x(&fx, &x9("true"));
    assert!(
        o.code != 0,
        "the record changed again with no amendment: {}",
        o.all()
    );
}

/// The refusal's own steps end in the record put: open the amendment, put the record while it waits,
/// narrow, the human approves, and the record is in force.
#[test]
fn the_printed_steps_of_the_refusal_end_with_the_record_in_force() {
    let (fx, _wc) = start();
    let changed = X1.replace("test -f src/hello.txt", "test -d src");
    let o = put_x(&fx, &changed);
    assert!(o.code != 0, "{}", o.all());
    let opened = fx
        .must(&["amend", "open", "--repair", "X-1", "--json"])
        .json();
    let o = put_x(&fx, &changed);
    assert_eq!(o.code, 0, "put while the amendment waits: {}", o.all());
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, change the check.");
    fx.must(&["amend", "approve", "A-1", "--quote", "change the check"]);
    let o = put_x(&fx, &changed);
    assert_eq!(o.code, 0, "{}", o.all());
    let d = fx.bf(&["doctor"]);
    assert!(!d.all().contains("frozen records: PROBLEM"), "{}", d.all());
}
