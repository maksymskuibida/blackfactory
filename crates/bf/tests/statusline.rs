//! The one-line progress report (R-37): a sign per state, units sharing a line, the review round.

mod common;
use bf::statusline::{render, sign};
use common::*;

fn it(id: &str, state: &str, round: u32) -> (String, String, u32) {
    (id.into(), state.into(), round)
}

#[test]
fn every_state_has_its_own_sign() {
    let states = [
        "running",
        "review",
        "fixing",
        "landed",
        "blocked",
        "verifying",
        "closed",
    ];
    let signs: Vec<&str> = states.iter().map(|s| sign(s).unwrap().0).collect();
    let mut u = signs.clone();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), states.len(), "{signs:?}");
    assert!(sign("parked").is_some());
    assert!(sign("planned").is_none());
}

#[test]
fn one_line_names_units_state_and_round() {
    let l = render(&[it("U-3", "review", 2)]);
    assert_eq!(l, "◐ U-3 in review, round 2");
    assert!(!l.contains('\n'));
}

#[test]
fn units_in_one_state_share_a_segment() {
    let l = render(&[
        it("U-1", "running", 0),
        it("U-2", "running", 0),
        it("U-4", "landed", 0),
    ]);
    assert_eq!(l, "▶ U-1, U-2 building · ✔ U-4 landed");
    assert!(!l.contains('\n'));
}

#[test]
fn checkpoints_and_idle_runs() {
    assert_eq!(
        render(&[it("K-1", "verifying", 0), it("K-0", "closed", 0)]),
        "◎ K-1 checkpoint being verified · ■ K-0 checkpoint closed"
    );
    assert_eq!(render(&[it("U-9", "planned", 0)]), "no unit has started");
}

#[test]
fn the_engine_prints_the_line_from_the_run() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["progress"]);
    assert_eq!(o.stdout.trim(), "no unit has started");
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.must(&["unit", "start", "U-2"]);
    let o = fx.must(&["progress"]);
    assert_eq!(o.stdout.trim().lines().count(), 1);
    assert_eq!(o.stdout.trim(), "▶ U-2 building · ✔ U-1 landed");
    let j = fx.must(&["progress", "--json"]).json();
    assert_eq!(j["line"], "▶ U-2 building · ✔ U-1 landed");
}

#[test]
fn the_round_follows_the_findings() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    assert_eq!(
        fx.must(&["progress"]).stdout.trim(),
        "◐ U-1 in review, round 1"
    );
    let f = r#"[{"title":"t","severity":"low","where":"a","action":"b","text":"c"}]"#;
    let o = fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "U-1",
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--findings",
            "-",
        ],
        f,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        fx.must(&["progress"]).stdout.trim(),
        "↻ U-1 fixing, round 1"
    );
    fx.must(&["report", "U-1", "--result", "fixed"]);
    assert_eq!(
        fx.must(&["progress"]).stdout.trim(),
        "◐ U-1 in review, round 2"
    );
}

#[test]
fn one_segment_per_state_with_the_round_of_each_unit() {
    assert_eq!(
        render(&[
            it("U-3", "review", 2),
            it("U-4", "review", 1),
            it("U-5", "review", 2)
        ]),
        "◐ U-3 (round 2), U-4 (round 1), U-5 (round 2) in review"
    );
    assert_eq!(
        render(&[it("U-3", "review", 2), it("U-5", "review", 2)]),
        "◐ U-3, U-5 in review, round 2"
    );
}

#[test]
fn a_checkpoint_under_review_or_being_fixed_has_its_own_states() {
    assert_eq!(
        render(&[it("K-1", "fix-fixing", 0)]),
        "↻ K-1 checkpoint being fixed"
    );
    assert_eq!(
        render(&[it("K-1", "fix-review", 2)]),
        "◐ K-1 checkpoint fix in review, round 2"
    );
    assert_eq!(
        render(&[it("K-1", "parked", 0), it("U-1", "landed", 0)]),
        "✔ U-1 landed · ⚠ K-1 parked"
    );
}

#[test]
fn blocked_and_parked_units_share_one_segment() {
    assert_eq!(
        render(&[it("U-1", "blocked", 0), it("U-2", "parked", 0)]),
        "⚠ U-1 blocked, U-2 parked"
    );
    assert_eq!(
        render(&[it("U-1", "parked", 0), it("U-2", "parked", 0)]),
        "⚠ U-1, U-2 parked"
    );
}
