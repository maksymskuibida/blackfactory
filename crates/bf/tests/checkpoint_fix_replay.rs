// R-94, D-68, TH-44: a checkpoint fix of review findings records how it was opened, and the replay of the
// log accepts it only when the events that justify that way are there.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn start(fx: &Fx, review_on: bool) {
    if review_on {
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
    }
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

fn report(fx: &Fx, finding: Value) -> Out {
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
        &json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": [finding]})
            .to_string(),
    )
}

fn finding(severity: &str) -> Value {
    json!({"title": "the answer is wrong", "severity": severity, "where": "src/a.txt:1", "action": "make it right"})
}

fn events(fx: &Fx) -> Vec<Value> {
    bf::events::load_log(&fx.project()).unwrap().events
}

fn fix_opens(fx: &Fx) -> Vec<Value> {
    events(fx)
        .into_iter()
        .filter(|e| e["kind"] == "checkpoint.fix.open")
        .collect()
}

fn ignored(fx: &Fx) -> Vec<String> {
    fx.must(&["status", "--json"]).json()["ignored_events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect()
}

fn fix_is_open(fx: &Fx) -> bool {
    !fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"].is_null()
}

fn round_findings(fx: &Fx, round: u64) -> Value {
    events(fx)
        .into_iter()
        .find(|e| e["kind"] == "checkpoint.review.done" && e["round"] == round)
        .map(|e| e["findings"].clone())
        .unwrap()
}

/// A high finding is reported, and its fix is aborted: the log holds one honest, marked opening.
fn honest_then_aborted() -> Fx {
    let fx = Fx::new();
    plan(&fx);
    start(&fx, true);
    brief(&fx);
    let o = report(&fx, finding("high"));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    fx
}

#[test]
fn the_reviewers_report_of_a_high_finding_opens_its_fix_marked_report() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx, true);
    brief(&fx);
    let o = report(&fx, finding("high"));
    assert_eq!(o.code, 0, "{}", o.all());
    let opens = fix_opens(&fx);
    assert_eq!(opens.len(), 1, "{opens:?}");
    assert_eq!(opens[0]["opened_by"], "report", "{}", opens[0]);
    assert!(fix_is_open(&fx));
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn the_release_of_a_parked_checkpoint_opens_its_fix_marked_release() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    plan(&fx);
    start(&fx, true);
    brief(&fx);
    let o = report(&fx, finding("high"));
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(fix_opens(&fx).is_empty());
    fx.human("Release it, go on with the fix.");
    fx.must(&[
        "checkpoint",
        "review",
        "K-1",
        "--release",
        "--quote",
        "Release it",
    ]);
    let opens = fix_opens(&fx);
    assert_eq!(opens.len(), 1, "{opens:?}");
    assert_eq!(opens[0]["opened_by"], "release", "{}", opens[0]);
    assert!(fix_is_open(&fx), "the replay accepts the release's fix");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_pick_of_a_high_finding_in_the_humans_words_opens_its_fix_marked_pick() {
    let fx = honest_then_aborted();
    fx.human("Yes, open the fix again.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1",
        "--quote",
        "open the fix again",
    ]);
    let opens = fix_opens(&fx);
    assert_eq!(opens.len(), 2, "{opens:?}");
    assert_eq!(opens[1]["opened_by"], "pick", "{}", opens[1]);
    assert!(opens[1]["human"]["turn"].as_u64().is_some());
    assert!(fix_is_open(&fx), "the replay accepts the pick's fix");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_pick_of_a_low_finding_opens_its_fix_marked_low() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx, true);
    brief(&fx);
    fx.must_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    let o = report(&fx, finding("low"));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.1"]);
    let opens = fix_opens(&fx);
    assert_eq!(opens.len(), 1, "{opens:?}");
    assert_eq!(opens[0]["opened_by"], "low", "{}", opens[0]);
    assert!(opens[0]["human"].is_null());
    assert!(fix_is_open(&fx), "the replay accepts the low pick's fix");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_fix_of_a_failed_check_writes_no_way_and_stays_accepted() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx, false);
    let opens = fix_opens(&fx);
    assert!(
        opens.iter().all(|e| e.get("opened_by").is_none()),
        "{opens:?}"
    );
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

/// An opening of review findings the way a forger would append it.
fn forged(opened_by: Option<&str>, findings: Value, ids: Value, source: &str) -> Value {
    let mut ev = json!({
        "checkpoint": "K-1", "source": source, "failed": [], "findings": findings,
        "finding_ids": ids, "before": [], "content": "forged", "human": null, "wave": 0,
    });
    if let Some(how) = opened_by {
        ev["opened_by"] = json!(how);
    }
    ev
}

#[test]
fn a_fix_opening_the_log_does_not_justify_is_ignored_and_opens_no_fix() {
    let fx = honest_then_aborted();
    assert!(!fix_is_open(&fx));
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
    let ids = json!([{"round": 1, "index": 1}]);
    let round = round_findings(&fx, 1);
    let mut other = finding("high");
    other["title"] = json!("another answer is wrong");
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "low naming a high finding",
            forged(Some("low"), json!([finding("high")]), ids.clone(), "review"),
            "not low",
        ),
        (
            "low labels over the id of a high finding",
            forged(Some("low"), json!([finding("low")]), ids.clone(), "review"),
            "not low",
        ),
        (
            "pick with no turn",
            forged(
                Some("pick"),
                json!([finding("high")]),
                ids.clone(),
                "review",
            ),
            "by a pick cites no human turn",
        ),
        (
            "release not followed by a release",
            forged(
                Some("release"),
                json!([finding("high")]),
                ids.clone(),
                "review",
            ),
            "not followed by the human's release",
        ),
        (
            "report naming a round a report already opened a fix for",
            forged(Some("report"), round.clone(), ids.clone(), "review"),
            "already opened a fix for this round",
        ),
        (
            "report whose findings differ from its round's",
            forged(Some("report"), json!([other]), ids.clone(), "review"),
            "differ from its round's",
        ),
        (
            "an unknown way",
            forged(Some("by-hand"), round.clone(), ids.clone(), "review"),
            "unknown way",
        ),
        (
            "no opened_by after a marked one",
            forged(None, round.clone(), ids.clone(), "review"),
            "records no way it was opened",
        ),
        (
            "source check naming finding ids",
            forged(None, json!([]), ids.clone(), "check"),
            "failed check names review findings",
        ),
    ];
    for (name, ev, words) in cases {
        let before = ignored(&fx);
        bf::events::append(&fx.project(), "checkpoint.fix.open", ev).unwrap();
        let after = ignored(&fx);
        eprintln!("{name}: {after:?}");
        assert_eq!(after.len(), before.len() + 1, "{name}: {after:?}");
        let note = after.last().unwrap();
        assert!(note.contains(words), "{name}: {note}");
        assert!(note.starts_with("event "), "{name}: {note}");
        assert!(!fix_is_open(&fx), "{name}: it opened a fix");
    }
}

#[test]
fn a_log_whose_fix_openings_carry_no_way_still_opens_them() {
    let fx = Fx::new();
    plan(&fx);
    start(&fx, true);
    brief(&fx);
    bf::events::append(
        &fx.project(),
        "checkpoint.fix.open",
        forged(
            None,
            json!([finding("high")]),
            json!([{"round": 1, "index": 1}]),
            "review",
        ),
    )
    .unwrap();
    assert!(fix_is_open(&fx), "an unmarked opening replays as before");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_report_opening_for_a_round_that_parked_the_checkpoint_is_ignored() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    plan(&fx);
    start(&fx, true);
    brief(&fx);
    let o = report(&fx, finding("high"));
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(fix_opens(&fx).is_empty(), "the parked round opened no fix");
    let ev = forged(
        Some("report"),
        round_findings(&fx, 1),
        json!([{"round": 1, "index": 1}]),
        "review",
    );
    bf::events::append(&fx.project(), "checkpoint.fix.open", ev).unwrap();
    let notes = ignored(&fx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("a round that parked the checkpoint"),
        "{notes:?}"
    );
    assert!(!fix_is_open(&fx), "the forged report opened a fix");
}
