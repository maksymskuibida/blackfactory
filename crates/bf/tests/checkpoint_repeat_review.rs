//! U-38: the repeat review of a checkpoint (R-69 to R-72): a repeat review is given the report before it,
//! what became of each finding and the probes kept; every report says which areas were probed and how; a
//! probe that exposed a fault becomes a permanent test through the fix; a checkpoint closes when a review
//! files no high and no medium finding, and parks for the human after the number of reviews the policy sets.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit with a source file and a test folder, one checkpoint with a command check that the reviewer
/// runs, and the review as a named check.
fn project(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on",
        "--scope",
        "default",
    ]);
    fx.human("Yes, the checkpoint review is on.");
    fx.must(&["checks", "set", "--quote", "the checkpoint review is on"]);
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

fn directive(fx: &Fx, action: &str) -> Option<Value> {
    let n = fx.must(&["next", "--json"]).json();
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == action)
        .cloned()
}

fn brief_text(fx: &Fx) -> String {
    let o = fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

/// The reviewer's report: areas, probes and findings through `--findings -`.
fn report(fx: &Fx, body: Value) -> Out {
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
        &body.to_string(),
    )
}

fn areas() -> Value {
    json!([
        {"area": "the close rule", "how": "read"},
        {"area": "the file check", "how": "built and run"}
    ])
}

fn finding(title: &str, severity: &str) -> Value {
    json!({"title": title, "severity": severity, "where": "src/a.txt:1", "action": "change it"})
}

fn run_checks_in_copy(fx: &Fx) {
    fx.must_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
}

fn status(fx: &Fx) -> Value {
    fx.must(&["status", "--json"]).json()
}

#[test]
fn a_report_without_the_areas_probed_is_refused_with_findings_and_without() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    // Findings in a bare list, no areas.
    let o = report(&fx, json!([finding("t", "high")]));
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("areas"), "{}", o.stderr);
    // An object with an empty list of areas.
    let o = report(
        &fx,
        json!({"areas": [], "findings": [finding("t", "high")]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    // An area probed in a way that is none of the three.
    let o = report(
        &fx,
        json!({"areas": [{"area": "x", "how": "guessed"}], "findings": [finding("t", "high")]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("how"), "{}", o.stderr);
    // A clean report needs them too.
    run_checks_in_copy(&fx);
    let o = fx.bf(&[
        "report", "K-1", "--role", "reviewer", "--result", "findings",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("areas"), "{}", o.stderr);
    let o = report(&fx, json!({"areas": areas(), "findings": []}));
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn the_areas_are_kept_with_the_review_shown_on_the_page_and_given_to_the_next_review() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    let o = report(
        &fx,
        json!({"areas": areas(), "findings": [finding("wrong answer", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let st = status(&fx);
    let round = &st["checkpoint_reviews"][0]["rounds"][0];
    assert_eq!(round["areas"][0]["area"], "the close rule", "{st}");
    assert_eq!(round["areas"][1]["how"], "built and run", "{st}");
    let page = fx.must(&["view", "checkpoint", "K-1"]).json();
    let body = page["sections"][0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "K-1-review")
        .unwrap()["body"]
        .to_string();
    assert!(
        body.contains("the close rule") && body.contains("built and run"),
        "{body}"
    );
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let md = brief_text(&fx);
    assert!(md.contains("Area: the close rule (read)"), "{md}");
    assert!(
        md.contains("Areas the reviews before you probed: the close rule; the file check"),
        "{md}"
    );
}

#[test]
fn the_first_review_is_cold_and_a_repeat_one_gets_the_report_the_fates_and_the_difference() {
    let fx = Fx::new();
    project(&fx);
    let first = brief_text(&fx);
    assert!(first.contains("cold"), "{first}");
    assert!(
        !first.contains("Earlier rounds") && !first.contains("Outcome:"),
        "{first}"
    );
    assert!(
        first.contains("<summary>U-1</summary>") && first.contains("K-1-review.diff"),
        "{first}"
    );
    let o = report(
        &fx,
        json!({"areas": areas(), "findings": [finding("first problem", "high"), finding("second problem", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The fix: one finding is declined on the human's words, and the human sends the checkpoint back.
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    fx.human("Decline the second one: it is how the file is meant to be.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--decline",
        "r1.2",
        "--reason",
        "that is the design",
        "--quote",
        "Decline the second one",
    ]);
    let md = brief_text(&fx);
    assert!(
        md.contains("round 2") && md.contains("Earlier rounds"),
        "{md}"
    );
    assert!(md.contains("whole checkpoint"), "{md}");
    assert!(
        md.contains("first problem") && md.contains("Outcome: open"),
        "{md}"
    );
    assert!(md.contains("Outcome: declined: that is the design"), "{md}");
    assert!(
        md.contains("confirm that each finding marked fixed"),
        "{md}"
    );
    assert!(
        md.contains("angles the earlier reviews did not take"),
        "{md}"
    );
    assert!(
        md.contains("files only new problems") || md.contains("File only new problems"),
        "{md}"
    );
    assert!(
        md.contains("<summary>U-1</summary>"),
        "scope stays the whole checkpoint: {md}"
    );
    // A repeat review does not file again what was declined.
    let o = report(
        &fx,
        json!({"areas": areas(), "findings": [finding("second problem", "high")]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("declined"), "{}", o.stderr);
}

#[test]
fn a_decline_needs_a_reason_and_the_humans_words_and_an_accept_needs_their_words() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    report(
        &fx,
        json!({"areas": areas(), "findings": [finding("a", "high")]}),
    );
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    // No reason.
    fx.human("Decline it.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--decline",
        "r1.1",
        "--quote",
        "Decline it.",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("reason"), "{}", o.stderr);
    // Words the human never wrote.
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--decline",
        "r1.1",
        "--reason",
        "r",
        "--quote",
        "I never said this",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        "r1.1",
        "--quote",
        "I never said this",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--decline",
        "r1.1",
        "--reason",
        "by design",
        "--quote",
        "Decline it.",
    ]);
    let st = status(&fx);
    let outcomes = &st["checkpoint_reviews"][0]["rounds"][0]["outcomes"];
    assert_eq!(outcomes[0]["fate"], "declined: by design", "{st}");
    // Ended once: it cannot be ended again.
    fx.human("Accept it.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        "r1.1",
        "--quote",
        "Accept it.",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
}

#[test]
fn a_checkpoint_cannot_close_while_a_high_or_medium_finding_of_its_latest_review_is_open() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 5");
    project(&fx);
    for severity in ["high", "medium", "critical"] {
        brief_text(&fx);
        let o = report(
            &fx,
            json!({"areas": areas(), "findings": [finding("open one", severity)]}),
        );
        assert_eq!(o.code, 0, "{}", o.all());
        // The fix opened for the finding is open: the close names it.
        let o = fx.refused(&["checkpoint", "close", "K-1"]);
        assert!(
            o.stderr.contains("a fix of it is open"),
            "{severity}: {}",
            o.stderr
        );
        fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
        // The fix ended without passing its review: the finding stays open and holds the close.
        let o = fx.refused(&["checkpoint", "close", "K-1"]);
        assert!(
            o.stderr.contains("checkpoint review"),
            "{severity}: {}",
            o.stderr
        );
    }
}

#[test]
fn a_review_with_only_low_findings_lets_the_checkpoint_close_and_the_human_gets_them_together() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    run_checks_in_copy(&fx);
    let o = report(
        &fx,
        json!({"areas": areas(), "findings": [finding("nit one", "low"), finding("nit two", "low")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // No fix opened, and the human is asked about the low ones.
    assert!(directive(&fx, "fix-checkpoint").is_none());
    let d = directive(&fx, "decide-low-findings").expect("the low findings are listed");
    assert_eq!(d["role"], "human", "{d}");
    assert_eq!(d["findings"].as_array().unwrap().len(), 2, "{d}");
    assert!(!fx.work("K-1-review").exists());
    // The human accepts them together.
    fx.human("I accept the low ones as they are.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        "low",
        "--quote",
        "I accept the low ones",
    ]);
    assert!(directive(&fx, "decide-low-findings").is_none());
    let st = status(&fx);
    let outcomes = &st["checkpoint_reviews"][0]["rounds"][0]["outcomes"];
    assert!(
        outcomes[0]["fate"]
            .as_str()
            .unwrap()
            .starts_with("accepted"),
        "{st}"
    );
    assert!(
        outcomes[1]["fate"]
            .as_str()
            .unwrap()
            .starts_with("accepted"),
        "{st}"
    );
    let c = fx.must(&["checkpoint", "close", "K-1", "--json"]).json();
    assert_eq!(c["closed"], true, "{c}");
}

#[test]
fn the_human_can_pick_low_findings_to_be_fixed() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    run_checks_in_copy(&fx);
    report(
        &fx,
        json!({"areas": areas(), "findings": [finding("nit one", "low"), finding("nit two", "low")]}),
    );
    fx.must(&["checkpoint", "finding", "K-1", "--fix-findings", "r1.2"]);
    let d = directive(&fx, "fix-checkpoint").expect("a fix opened");
    assert_eq!(d["findings"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["findings"][0]["title"], "nit two", "{d}");
}

#[test]
fn a_bare_reviewer_report_opens_no_fix_with_no_round_open_and_never_lifts_a_park() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 2");
    project(&fx);
    // No round is open (no brief yet): a findings report with no areas is refused and opens no fix.
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
        &json!([finding("bare", "high")]).to_string(),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(directive(&fx, "fix-checkpoint").is_none());
    // Park the checkpoint with two reviews.
    for title in ["one", "two"] {
        brief_text(&fx);
        let o = report(
            &fx,
            json!({"areas": areas(), "findings": [finding(title, "high")]}),
        );
        assert_eq!(o.code, 0, "{}", o.all());
        if title == "one" {
            fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
        }
    }
    assert!(directive(&fx, "decide-checkpoint-review").is_some());
    // A bare report, with or without areas, does not open a fix or lift the park.
    for body in [
        json!([finding("bare", "high")]),
        json!({"areas": areas(), "findings": [finding("bare", "high")]}),
    ] {
        let o = report(&fx, body);
        assert_ne!(o.code, 0, "{}", o.all());
    }
    assert!(directive(&fx, "fix-checkpoint").is_none());
    assert!(directive(&fx, "decide-checkpoint-review").is_some());
}

#[test]
fn the_checkpoint_parks_after_the_set_number_of_reviews_and_only_the_human_releases_it() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 2");
    project(&fx);
    brief_text(&fx);
    assert_eq!(
        report(
            &fx,
            json!({"areas": areas(), "findings": [finding("one", "high")]})
        )
        .code,
        0
    );
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    brief_text(&fx);
    let o = report(
        &fx,
        json!({"areas": areas(), "findings": [finding("two", "medium")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // Two reviews with a medium finding open: parked, no fix, no third brief.
    let d = directive(&fx, "decide-checkpoint-review").expect("the human is asked");
    assert_eq!(d["role"], "human", "{d}");
    assert!(directive(&fx, "fix-checkpoint").is_none());
    assert!(directive(&fx, "review-checkpoint").is_none());
    fx.refused(&["brief", "K-1", "--role", "reviewer"]);
    // No human words: no release.
    let o = fx.bf(&[
        "checkpoint",
        "review",
        "K-1",
        "--release",
        "--quote",
        "release it",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
    fx.human("Release it, go on with the fix.");
    fx.must(&[
        "checkpoint",
        "review",
        "K-1",
        "--release",
        "--quote",
        "Release it",
    ]);
    let d = directive(&fx, "fix-checkpoint").expect("the findings go to a fix");
    // Every high or medium finding still open since the last release goes to the fix, not only the latest's.
    assert_eq!(d["findings"][0]["title"], "one", "{d}");
    assert_eq!(d["findings"][1]["title"], "two", "{d}");
    assert!(directive(&fx, "decide-checkpoint-review").is_none());
}

#[test]
fn the_number_of_reviews_is_a_setting_in_the_policy_with_three_as_shipped() {
    let fx = Fx::new();
    let t = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    assert!(t.contains("checkpoint_reviews = 3"), "{t}");
    assert_eq!(bf::store::Limits::default().checkpoint_reviews, 3);
}

#[test]
fn a_probe_is_kept_with_what_it_meant_to_show_and_a_finding_names_it() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    fs::create_dir_all(copy.join("tests")).unwrap();
    fs::write(copy.join("tests/p1.sh"), "grep -q fixed src/a.txt\n").unwrap();
    let probe = json!({"name": "p1", "meant": "show that the file says fixed", "showed": "it does not",
        "file": "tests/p1.sh", "command": "sh tests/p1.sh", "expect": "the file says fixed"});
    // A finding that names a probe the report does not list.
    let mut f = finding("not fixed", "high");
    f["probe"] = json!("p9");
    let o = report(
        &fx,
        json!({"areas": areas(), "probes": [probe.clone()], "findings": [f.clone()]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.stderr.contains("p9"), "{}", o.stderr);
    f["probe"] = json!("p1");
    let o = report(
        &fx,
        json!({"areas": areas(), "probes": [probe], "findings": [f]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let st = status(&fx);
    let kept = &st["checkpoint_reviews"][0]["rounds"][0]["probes"][0];
    assert_eq!(kept["meant"], "show that the file says fixed", "{st}");
    assert_eq!(kept["showed"], "it does not", "{st}");
    let stored = fx.root.join(kept["stored"].as_str().unwrap());
    assert!(stored.is_file(), "{}", stored.display());
    assert!(fs::read_to_string(&stored)
        .unwrap()
        .contains("grep -q fixed"));
    // The next review is given it.
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let md = brief_text(&fx);
    assert!(
        md.contains("Probe p1") && md.contains("show that the file says fixed"),
        "{md}"
    );
}

#[test]
fn a_probe_with_a_home_path_a_link_a_mail_address_or_the_users_name_is_refused() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    fs::create_dir_all(copy.join("tests")).unwrap();
    let user = std::env::var("USER").unwrap_or_default();
    let mut bodies = vec![
        "cat /Users/someone/notes.txt\n".to_string(),
        "cat /home/someone/notes.txt\n".to_string(),
        "curl https://example.org/private\n".to_string(),
        "mail somebody@example.org\n".to_string(),
    ];
    if user.chars().count() >= 4 {
        bodies.push(format!("echo {user} was here\n"));
    }
    for body in bodies {
        fs::write(copy.join("tests/p.sh"), &body).unwrap();
        let probe = json!({"name": "p", "meant": "m", "showed": "s", "file": "tests/p.sh", "command": "sh tests/p.sh"});
        let o = report(
            &fx,
            json!({"areas": areas(), "probes": [probe], "findings": [finding("t", "high")]}),
        );
        assert_ne!(o.code, 0, "{body}: {}", o.all());
        assert!(o.stderr.contains("probe `p`"), "{}", o.stderr);
    }
    // The words of the report are scanned too.
    let probe = json!({"name": "p", "meant": "see /Users/someone/x", "showed": "s"});
    let o = report(
        &fx,
        json!({"areas": areas(), "probes": [probe], "findings": [finding("t", "high")]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    // A clean probe is stored.
    fs::write(copy.join("tests/p.sh"), "test -f src/a.txt\n").unwrap();
    let probe = json!({"name": "p", "meant": "m", "showed": "s", "file": "tests/p.sh", "command": "sh tests/p.sh"});
    let o = report(
        &fx,
        json!({"areas": areas(), "probes": [probe], "findings": [finding("t", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_probe_that_exposed_a_fault_must_be_a_permanent_test_that_passes_before_the_fix_counts() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    let copy = fx.work("K-1-review");
    fs::create_dir_all(copy.join("tests")).unwrap();
    fs::write(copy.join("tests/probe_p1.sh"), "grep -q fixed src/a.txt\n").unwrap();
    let probe = json!({"name": "p1", "meant": "show the file says fixed", "showed": "it does not",
        "file": "tests/probe_p1.sh", "command": "sh tests/probe_p1.sh", "expect": "the file says fixed"});
    let mut f = finding("not fixed", "high");
    f["probe"] = json!("p1");
    // A finding that names a probe needs the file, the command and the expected answer.
    let bare = json!({"name": "p1", "meant": "m", "showed": "s"});
    let o = report(
        &fx,
        json!({"areas": areas(), "probes": [bare], "findings": [f.clone()]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    let o = report(
        &fx,
        json!({"areas": areas(), "probes": [probe], "findings": [f]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let k = fx.work("K-1");
    let b = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        b.contains("permanent test") && b.contains("tests/probe_p1.sh"),
        "{b}"
    );
    let builder = |fx: &Fx| {
        fx.bf_in(
            &k,
            &["report", "K-1", "--role", "builder", "--result", "built"],
        )
    };
    // The code is fixed, but the probe is not in the suite: the fix does not count.
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fix"]);
    let o = builder(&fx);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.stderr.contains("permanent test") && o.stderr.contains("p1"),
        "{}",
        o.stderr
    );
    // The probe is added, but the code is not right: its answer is not the expected one.
    fs::write(k.join("src/a.txt"), "wrong\n").unwrap();
    fs::create_dir_all(k.join("tests")).unwrap();
    fs::write(k.join("tests/probe_p1.sh"), "grep -q fixed src/a.txt\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "probe, not fixed"]);
    let o = builder(&fx);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.stderr.contains("exit 1") || o.stderr.contains("gave exit"),
        "{}",
        o.stderr
    );
    // Both: it counts and goes to review.
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fixed"]);
    let o = builder(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(directive(&fx, "review-checkpoint-fix").is_some());
}

#[test]
fn a_finding_is_fixed_by_the_unit_whose_files_the_fix_changed() {
    let fx = Fx::new();
    project(&fx);
    brief_text(&fx);
    report(
        &fx,
        json!({"areas": areas(), "findings": [finding("not fixed", "high")]}),
    );
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "fix"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
    let md = brief_text(&fx);
    assert!(md.contains("Outcome: fixed by U-1"), "{md}");
}
