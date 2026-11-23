//! The human ends an open checkpoint review round: `bf checkpoint review <K> --abort --quote "..."`.
//! The sequence is the one that deadlocked a run: a checkpoint's units landed, its review round opened, an
//! amendment reopened a unit, and the round could never report, so landing waited for ever.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit lands, the review round of K-1 opens, an amendment reopens the unit and the unit is built and
/// reviewed again. Landing then waits for the round.
fn deadlocked() -> Fx {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    fx.must(&[
        "checks", "propose", "checkpoint-review=on",
        "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it",
        "--scope", "default",
    ]);
    fx.human("Yes, the checkpoint review is on.");
    fx.must(&["checks", "set", "--quote", "the checkpoint review is on"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    build_and_review(&fx, true);
    fx.must(&["unit", "land", "U-1"]);
    // The review round opens (its reviewer never reports).
    fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    // An amendment reopens the unit.
    let p = fx.project();
    bf::events::append(&p, "amend.open", json!({"amend": "A-1", "ids": ["R-1"], "impact": null, "impact_ids": ["U-1"]})).unwrap();
    bf::events::append(&p, "amend.narrow", json!({"amend": "A-1", "keep": ["U-1"], "waive": [], "reasons": {}})).unwrap();
    fx.human("Yes, approve the amendment.");
    let turn = bf::events::replay(&p).unwrap().turns.last().unwrap().seq;
    bf::events::append(
        &p,
        "amend.approve",
        json!({"amend": "A-1", "turn": turn, "quote": "approve the amendment", "reopen_units": ["U-1"],
               "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0, "renamed": []}),
    )
    .unwrap();
    build_and_review(&fx, false);
    fx
}

fn build_and_review(fx: &Fx, start: bool) {
    let wc = if start {
        PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap())
    } else {
        fx.work("U-1")
    };
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), if start { "from the unit\n" } else { "amended\n" }).unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
}

fn land_line(fx: &Fx) -> Value {
    fx.must(&["unit", "land", "U-1", "--json"]).json()
}

#[test]
fn abort_review_round_ends_the_round_and_landing_goes_on() {
    let fx = deadlocked();
    let w = land_line(&fx);
    assert_eq!(w["waiting"], true, "{w}");
    let line = w["line"].as_str().unwrap();
    assert!(line.contains("review round"), "{line}");
    assert!(line.contains("checkpoint review K-1 --abort"), "the way out is named: {line}");

    // No quote, and a quote that is not the human's: refused, the round stays open.
    let o = fx.bf(&["checkpoint", "review", "K-1", "--abort"]);
    assert_eq!(o.code, 2, "{}", o.all());
    let o = fx.bf(&["checkpoint", "review", "K-1", "--abort", "--quote", "go on, end it"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(land_line(&fx)["waiting"], true);

    fx.human("I end the dead review round of K-1.");
    let o = fx.must(&["checkpoint", "review", "K-1", "--abort", "--quote", "end the dead review round"]);
    assert!(o.stdout.contains("K-1"), "{}", o.all());
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    assert!(log.contains("\"result\":\"void\"") && log.contains("aborted_by_human"), "{log}");

    // The same words cannot end another round, and there is no open round now.
    let o = fx.bf(&["checkpoint", "review", "K-1", "--abort", "--quote", "end the dead review round"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("no open review round"), "{}", o.all());

    // The round counts as no review, and the unit lands.
    let l = land_line(&fx);
    assert_ne!(l["waiting"], true, "{l}");
    assert_eq!(l["landed"], true, "{l}");
}

#[test]
fn abort_review_round_refuses_a_checkpoint_with_no_open_round() {
    let fx = deadlocked();
    fx.human("I end the dead review round of K-1.");
    fx.must(&["checkpoint", "review", "K-1", "--abort", "--quote", "end the dead review round"]);
    fx.human("Again, end any round of K-1.");
    let o = fx.bf(&["checkpoint", "review", "K-1", "--abort", "--quote", "end any round"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("no open review round"), "{}", o.all());
}
