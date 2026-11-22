//! D-110, D-113: the machine's load is written into every receipt of a command check as a warning only;
//! a hold made only by `unreachable` marks decides nothing and parks nothing.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const OLD: &str = "Looks good to me, keep going.";

fn cores() -> f64 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f64
}

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
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on",
        "idea-review=off:the test does not exercise it",
        "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it",
        "plan-review=off:the test does not exercise it",
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

fn receipt(fx: &Fx, id: &str) -> toml::Table {
    let dir = fx.state_dir().join("receipts");
    let path = [
        dir.join(format!("{id}.toml")),
        fx.root.join("factory/receipts").join(format!("{id}.toml")),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .unwrap_or_else(|| panic!("no receipt {id}"));
    fs::read_to_string(path).unwrap().parse().unwrap()
}

#[test]
fn a_receipt_of_a_command_check_holds_the_load_and_the_cores() {
    let fx = Fx::new();
    plan(&fx, "test -f src/a.txt");
    start(&fx);
    let o = fx.bf_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "run",
            "X-2",
            "--json",
            "--",
            "test",
            "-f",
            "src/a.txt",
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let v = o.json();
    assert!(v["load"].is_number(), "{v}");
    assert_eq!(v["cores"].as_f64(), Some(cores()), "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let r = receipt(&fx, &id);
    assert!(r.contains_key("load") && r.contains_key("cores"), "{r:?}");
    // verify shows the load line and the seal still holds.
    let o = fx.bf_in(&fx.work("K-1"), &["evidence", "verify"]);
    assert!(
        o.stdout.contains("load ") && o.stdout.contains(" cores"),
        "{}",
        o.all()
    );
    assert!(o.stdout.contains("seal=ok"), "{}", o.all());
}

#[test]
fn a_busy_machine_warns_and_the_check_still_passes() {
    let fx = Fx::new();
    plan(&fx, "test -f src/a.txt");
    start(&fx);
    let load = format!("{}", cores());
    let o = fx.bf_env(
        &fx.work("K-1"),
        &[
            "evidence",
            "run",
            "X-2",
            "--json",
            "--",
            "test",
            "-f",
            "src/a.txt",
        ],
        "",
        &[("BF_TEST_LOAD", &load)],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        o.stderr.matches("Warning: the machine is busy").count(),
        1,
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("runs all the same"), "{}", o.stderr);
    assert_eq!(o.json()["load"].as_f64(), Some(cores()));
}

#[test]
fn a_quiet_machine_prints_no_warning() {
    let fx = Fx::new();
    plan(&fx, "test -f src/a.txt");
    start(&fx);
    let o = fx.bf_env(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
        "",
        &[("BF_TEST_LOAD", "0.0")],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!o.stderr.contains("busy"), "{}", o.stderr);
}

#[test]
fn unreachable_says_a_mark_for_load_is_not_wanted() {
    let fx = Fx::new();
    plan(&fx, "test -f src/a.txt");
    start(&fx);
    let o = fx.bf_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "unreachable",
            "X-2",
            "--reason",
            "the program is missing",
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(
        o.stdout.contains("A mark for load is not wanted"),
        "{}",
        o.all()
    );
}

fn finding_to_human(fx: &Fx) {
    fx.bf(&["brief", "K-1", "--role", "reviewer", "--json"]);
    let body = json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": [{"title": "the answer is wrong", "severity": "high", "where": "src/a.txt:1", "action": "make it right"}]});
    let o = fx.bf_stdin(
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
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf(&["checkpoint", "fix", "K-1", "--abort"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn accept(fx: &Fx, quote: &str) -> Out {
    fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        "r1.1",
        "--quote",
        quote,
    ])
}

fn marked(kind: &str) -> Fx {
    let fx = Fx::new();
    fx.human("Some early words, before the plan.");
    plan(&fx, "test -f src/a.txt");
    start(&fx);
    finding_to_human(&fx);
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["evidence", kind, "X-2", "--reason", "the service is down"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["next", "--json"]);
    fx.human(OLD);
    fx
}

#[test]
fn words_written_during_a_hold_of_unreachable_marks_only_count() {
    let fx = marked("unreachable");
    let o = accept(&fx, OLD);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn words_written_during_the_hold_still_count_after_a_passing_run_lifts_the_mark() {
    let fx = marked("unreachable");
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["next", "--json"]);
    let o = accept(&fx, OLD);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn words_written_during_a_hold_with_a_failed_mark_are_refused() {
    let fx = marked("fail");
    let o = accept(&fx, OLD);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.all()
            .contains("the human is not being asked about the findings of K-1"),
        "{}",
        o.all()
    );
}

#[test]
fn a_hold_logged_before_the_unit_still_holds_on_replay() {
    let fx = marked("unreachable");
    let p = fx.project();
    bf::events::append(
        &p,
        "checkpoint.hold",
        json!({"checkpoint": "K-1", "cause": "marked", "detail": "X-2"}),
    )
    .unwrap();
    let st = bf::events::replay_log(&bf::events::load_log(&p).unwrap());
    assert!(bf::events::decision_bound(&st, "K-1", 0, u64::MAX).is_err());
}

#[test]
fn a_blocked_fix_builder_over_unreachable_marks_leaves_the_fix_open() {
    let fx = Fx::new();
    plan(&fx, "grep -q fixed src/a.txt");
    // The unit lands, then the checkpoint's check fails and opens a fix.
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
    let k = fx.work("K-1");
    fx.bf_in(
        &k,
        &[
            "evidence",
            "run",
            "X-2",
            "--",
            "grep",
            "-q",
            "fixed",
            "src/a.txt",
        ],
    );
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    let o = fx.bf_in(
        &k,
        &[
            "evidence",
            "unreachable",
            "X-2",
            "--reason",
            "the service is down",
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf_in(
        &k,
        &[
            "report",
            "K-1",
            "--role",
            "builder",
            "--result",
            "blocked",
            "--note",
            "the service is down",
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let v: Value = fx.must(&["status", "--json"]).json();
    let fix = &v["checkpoint_fix"];
    assert_eq!(fix["open"]["phase"], "fix", "{v}");
    let n = fx.must(&["next", "--json"]).json();
    assert!(!n.to_string().contains("parked"), "{n}");
    // The note reaches the next brief of the builder.
    let brief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        brief.contains("the service is down") && brief.contains("Earlier blocked reports"),
        "{brief}"
    );
    // Each report counts as an attempt: the limit parks the fix, so the same directive is not repeated for ever.
    let mut parked = false;
    for _ in 0..10 {
        let o = fx.bf_in(
            &k,
            &[
                "report",
                "K-1",
                "--role",
                "builder",
                "--result",
                "blocked",
                "--note",
                "still down",
            ],
        );
        assert_eq!(o.code, 0, "{}", o.all());
        let v: Value = fx.must(&["status", "--json"]).json();
        if v["checkpoint_fix"]["open"]["phase"] == "parked" {
            parked = true;
            break;
        }
    }
    assert!(parked, "the attempts limit parks the fix");
}
