//! The checkpoint fix (R-45 to R-48, R-65): a failed checkpoint check or a finding of the checkpoint review
//! opens a fix for a builder in the checkpoint's own working copy; the fix is limited to the files of the
//! checkpoint's units, reviewed in rounds, checked again and attributed to units.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const K_CHECK: &str = "grep -q fixed src/a.txt";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{K_CHECK}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn land_unit(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
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

/// The verifier runs the checkpoint's check in the checkpoint's working copy: it fails.
fn failing_run(fx: &Fx) -> PathBuf {
    let k = fx.work("K-1");
    let o = fx.bf_in(
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
    assert!(!o.all().is_empty());
    k
}

fn one_directive(fx: &Fx, action: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "one directive while a fix is open: {n}");
    assert_eq!(ds[0]["action"], action, "{n}");
    ds[0].clone()
}

fn guard_says(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> String {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => "abstain".into(),
        t => serde_json::from_str::<Value>(t).unwrap()["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap()
            .to_string(),
    }
}

fn commit(k: &Path, msg: &str) {
    git(k, &["add", "-A"]);
    git(k, &["commit", "-q", "-m", msg]);
}

fn builder_report(fx: &Fx, k: &Path, result: &str) -> Out {
    fx.bf_in(
        k,
        &["report", "K-1", "--role", "builder", "--result", result],
    )
}

fn read(fx: &Fx, role: &str) -> String {
    fs::read_to_string(fx.must(&["brief", "K-1", "--role", role]).stdout.trim()).unwrap()
}

#[test]
fn a_failed_check_gives_a_fix_for_a_builder_with_the_failure_in_its_brief() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = failing_run(&fx);
    // Not a question to the human: a builder's directive.
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["role"], "builder", "{d}");
    assert_eq!(d["failing_checks"][0], "X-2", "{d}");
    assert!(d["may_change"].to_string().contains("src/a.txt"), "{d}");
    let b = read(&fx, "builder");
    assert!(
        b.contains("X-2")
            && b.contains(K_CHECK)
            && b.contains("Output tail")
            && b.contains("src/a.txt"),
        "{b}"
    );
    assert!(b.contains("weakening a test"), "{b}");
    // The guard holds the fix to the file set: a unit's file and a new test file in its test folder only.
    let w = |f: &str| {
        guard_says(
            &fx,
            &k,
            "Write",
            json!({"file_path": k.join(f).to_string_lossy()}),
        )
    };
    assert_ne!(w("src/a.txt"), "deny");
    assert_ne!(w("tests/new_test.txt"), "deny");
    assert_eq!(w("src/other.txt"), "deny");
    // Nothing else starts, and the status shows the fix.
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["checkpoint_fix"]["open"]["phase"], "fix", "{st}");
    let line = fx.must(&["progress"]).stdout;
    assert!(line.contains("K-1 checkpoint being fixed"), "{line}");
}

#[test]
fn a_verifier_who_marks_a_check_failed_also_opens_a_fix() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    fx.must_in(
        &k,
        &[
            "evidence",
            "fail",
            "X-2",
            "--reason",
            "the answer is not fixed",
        ],
    );
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["failing_checks"][0], "X-2", "{d}");
    assert!(read(&fx, "builder").contains("the answer is not fixed"));
}

#[test]
fn a_units_review_is_cold_first_then_starts_from_the_findings_and_the_changes() {
    let fx = Fx::new();
    plan(&fx);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    let wc = fx.work("U-1");
    fx.must(&["unit", "start", "U-1"]);
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "one\n").unwrap();
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "one"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    let rb = fs::read_to_string(
        fx.must(&["brief", "U-1", "--role", "reviewer"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        rb.contains("Review round 1")
            && rb.contains("cold")
            && rb.contains("anywhere in the working copy"),
        "{rb}"
    );
    let f = r#"[{"title":"too short","severity":"low","where":"src/a.txt","action":"add more"}]"#;
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
    fs::write(wc.join("src/a.txt"), "one\ntwo\n").unwrap();
    git(&wc, &["add", "-A"]);
    git(&wc, &["commit", "-q", "-m", "two"]);
    fx.must(&["report", "U-1", "--result", "fixed"]);
    let rb = fs::read_to_string(
        fx.must(&["brief", "U-1", "--role", "reviewer"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        rb.contains("Review round 2")
            && rb.contains("too short")
            && rb.contains("Changes since round 1")
            && rb.contains("+two"),
        "{rb}"
    );
    // Each round is recorded with its number, the content it judged and what it covered.
    let st = fx.must(&["status", "--json"]).json();
    let rounds = &st["units"][0]["review_rounds"];
    assert_eq!(rounds[0]["round"], 1, "{st}");
    assert!(!rounds[0]["content"].as_str().unwrap().is_empty(), "{st}");
    assert!(
        rounds[0]["covered"]
            .as_str()
            .unwrap()
            .contains("everything"),
        "{st}"
    );
}

#[test]
fn a_check_that_could_not_run_still_goes_to_the_human() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    fx.must_in(
        &k,
        &[
            "evidence",
            "unreachable",
            "X-2",
            "--reason",
            "the service is down",
        ],
    );
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "decide-check", "{n}");
    assert!(fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"].is_null());
}

#[test]
fn the_review_of_a_checkpoint_files_findings_and_gets_the_same_directive() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = fx.work("K-1");
    let findings = json!([{"title": "the answer is wrong", "severity": "high", "where": "src/a.txt", "action": "fix it"}]);
    let o = fx.bf_stdin(
        &k,
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
        &findings.to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["source"], "review", "{d}");
    assert_eq!(d["findings"][0]["severity"], "high", "{d}");
    let b = read(&fx, "builder");
    assert!(b.contains("[high] the answer is wrong at src/a.txt"), "{b}");
}

#[test]
fn a_fix_goes_through_review_rounds_then_the_checks_run_again_and_units_are_attributed() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = failing_run(&fx);
    one_directive(&fx, "fix-checkpoint");
    // Nothing changed: refused.
    assert_eq!(builder_report(&fx, &k, "built").code, 2);
    // A change that still fails the check counts as a failing attempt and is refused.
    fs::write(k.join("src/a.txt"), "still wrong\n").unwrap();
    commit(&k, "first try");
    let o = builder_report(&fx, &k, "built");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("X-2 fails"), "{}", o.stderr);
    assert_eq!(
        fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"]["attempts"],
        1
    );
    // A good one, with a new test file beside the unit's tests.
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::create_dir_all(k.join("tests")).unwrap();
    fs::write(k.join("tests/new_test.txt"), "covers the fix\n").unwrap();
    commit(&k, "second try");
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    let d = one_directive(&fx, "review-checkpoint-fix");
    assert_eq!(d["role"], "reviewer");
    assert_eq!(d["round"], 1);
    let rb = read(&fx, "reviewer");
    assert!(
        rb.contains("Review round 1") && rb.contains("cold") && rb.contains("src/a.txt"),
        "{rb}"
    );
    // Round 1 files a finding: back to the builder, who sees it.
    let findings = json!([{"title": "no edge case", "severity": "medium", "where": "src/a.txt", "action": "handle it"}]);
    let o = fx.bf_stdin(
        &k,
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
        &findings.to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    one_directive(&fx, "fix-checkpoint");
    assert!(read(&fx, "builder").contains("no edge case"));
    fs::write(k.join("src/a.txt"), "fixed\nedge case\n").unwrap();
    commit(&k, "edge case");
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "fixed"],
    );
    // Round 2: the earlier finding and the changes since round 1.
    let d = one_directive(&fx, "review-checkpoint-fix");
    assert_eq!(d["round"], 2, "{d}");
    let rb = read(&fx, "reviewer");
    assert!(
        rb.contains("Review round 2")
            && rb.contains("no edge case")
            && rb.contains("Changes since round 1")
            && rb.contains("+edge case")
            && rb.contains("anywhere in the working copy"),
        "{rb}"
    );
    let o = fx.must_in(
        &k,
        &[
            "report", "K-1", "--role", "reviewer", "--result", "built", "--json",
        ],
    );
    let v = o.json();
    assert_eq!(v["phase"], "done", "{v}");
    assert_eq!(v["units"][0], "U-1", "{v}");
    // The engine ran the checks again on the new content: a passing receipt exists for it.
    let ev = fx.must_in(&k, &["evidence", "verify", "--checkpoint", "K-1"]);
    assert!(ev.stdout.contains("X-2"), "{}", ev.all());
    // The fix is recorded with its files and units; the checkpoint page and the unit's row show it.
    let st = fx.must(&["status", "--json"]).json();
    assert!(st["checkpoint_fix"]["open"].is_null(), "{st}");
    let done = &st["checkpoint_fix"]["done"][0];
    assert_eq!(done["id"], "K-1-fix-1", "{done}");
    assert_eq!(done["units"][0], "U-1", "{done}");
    let files = done["files"].to_string();
    assert!(
        files.contains("src/a.txt") && files.contains("tests/new_test.txt"),
        "{done}"
    );
    assert_eq!(done["by_file"]["tests/new_test.txt"][0], "U-1", "{done}");
    assert_eq!(done["review_rounds"].as_array().unwrap().len(), 2, "{done}");
    assert_eq!(done["review_rounds"][1]["round"], 2);
    assert!(!done["review_rounds"][0]["content"]
        .as_str()
        .unwrap()
        .is_empty());
    assert_eq!(st["units"][0]["checkpoint_fixes"][0], "K-1-fix-1", "{st}");
    // The guard is back to refusing every write in the checkpoint's copy.
    assert_eq!(
        guard_says(
            &fx,
            &k,
            "Write",
            json!({"file_path": k.join("src/a.txt").to_string_lossy()})
        ),
        "deny"
    );
}

#[test]
fn a_change_outside_the_files_is_refused_as_a_plan_gap_for_the_human() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = failing_run(&fx);
    one_directive(&fx, "fix-checkpoint");
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("src/other.txt"), "not this checkpoint's\n").unwrap();
    // An existing file of another folder is not a new test file either.
    commit(&k, "more than allowed");
    let o = builder_report(&fx, &k, "built");
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
    assert!(
        o.stderr.contains("plan gap") && o.stderr.contains("src/other.txt"),
        "{}",
        o.stderr
    );
    let d = one_directive(&fx, "plan-gap");
    assert_eq!(d["role"], "human", "{d}");
    assert!(d["missing"].to_string().contains("src/other.txt"), "{d}");
    assert!(d["note"].as_str().unwrap().contains("amendment"), "{d}");
    // The checkpoint goes back as it was, and no fix opens again for that content: the old way holds.
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    assert!(!k.join("src/other.txt").exists());
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "run-checkpoint-checks", "{n}");
    assert!(fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"].is_null());
}

#[test]
fn a_builder_can_report_a_gap_and_a_new_test_file_only_counts_when_it_is_new() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = failing_run(&fx);
    one_directive(&fx, "fix-checkpoint");
    let o = fx.bf_in(
        &k,
        &[
            "report",
            "K-1",
            "--role",
            "builder",
            "--result",
            "gap",
            "--note",
            "needs behaviour no record describes",
        ],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    let d = one_directive(&fx, "plan-gap");
    assert!(
        d["missing"].to_string().contains("no record describes"),
        "{d}"
    );
}

#[test]
fn rounds_and_failing_attempts_count_against_the_unit_limits_and_a_parked_fix_goes_to_the_human() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = failing_run(&fx);
    one_directive(&fx, "fix-checkpoint");
    // A blocked builder parks the fix.
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
            "cannot reproduce",
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "decide-checkpoint-fix");
    assert_eq!(d["role"], "human", "{d}");
    assert!(fx.must(&["progress"]).stdout.contains("K-1 parked"));
    // The human releases it; it is back with its builder.
    fx.human("Yes, try the fix again.");
    fx.must(&[
        "checkpoint",
        "fix",
        "K-1",
        "--release",
        "--quote",
        "try the fix again",
    ]);
    one_directive(&fx, "fix-checkpoint");
    // More review rounds than the limit park it too.
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    commit(&k, "fix");
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    let findings =
        json!([{"title": "again", "severity": "low", "where": "src/a.txt", "action": "x"}]);
    for round in 1..=4 {
        let d = one_directive(&fx, "review-checkpoint-fix");
        assert_eq!(d["round"], round, "{d}");
        let o = fx.bf_stdin(
            &k,
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
            &findings.to_string(),
        );
        assert_eq!(o.code, 0, "{}", o.all());
        if round == 4 {
            break;
        }
        one_directive(&fx, "fix-checkpoint");
        fs::write(k.join("src/a.txt"), format!("fixed {round}\n")).unwrap();
        commit(&k, "another");
        fx.must_in(
            &k,
            &["report", "K-1", "--role", "builder", "--result", "built"],
        );
    }
    one_directive(&fx, "decide-checkpoint-fix");
}

fn report_findings(fx: &Fx, k: &Path) {
    let findings = json!([{"title": "no edge case", "severity": "medium", "where": "src/a.txt", "action": "handle it"}]);
    let o = fx.bf_stdin(
        k,
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
        &findings.to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

fn section<'a>(page: &'a Value, id: &str) -> &'a Value {
    page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no section {id}: {page}"))
}

fn row_text(row: &Value) -> String {
    row["body"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_pages_show_a_fix_with_its_files_units_and_both_review_rounds() {
    let fx = Fx::new();
    plan(&fx);
    land_unit(&fx);
    let k = failing_run(&fx);
    one_directive(&fx, "fix-checkpoint");
    // While the fix is open the page already shows it, with what set it off.
    let open = fx.must(&["view", "checkpoint", "K-1", "--json"]).json();
    assert_contract("checkpoint (fix open)", &open);
    let t = row_text(&section(&open, "fixes")["rows"][0]);
    assert!(t.contains("X-2") && t.contains(K_CHECK), "{t}");
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::create_dir_all(k.join("tests")).unwrap();
    fs::write(k.join("tests/new_test.txt"), "covers the fix\n").unwrap();
    commit(&k, "first");
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    report_findings(&fx, &k);
    fs::write(k.join("src/a.txt"), "fixed\nedge case\n").unwrap();
    commit(&k, "second");
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "fixed"],
    );
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
    let page = fx.must(&["view", "checkpoint", "K-1", "--json"]).json();
    assert_contract("checkpoint", &page);
    let fixes = section(&page, "fixes");
    println!("{}", serde_json::to_string_pretty(fixes).unwrap());
    let rows = fixes["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{fixes}");
    let t = row_text(&rows[0]);
    assert_eq!(rows[0]["id"], "K-1-fix-1");
    assert!(t.contains("X-2") && t.contains(K_CHECK), "{t}");
    assert!(
        t.contains("src/a.txt") && t.contains("tests/new_test.txt"),
        "{t}"
    );
    assert!(t.contains("Units: U-1"), "{t}");
    assert!(
        t.contains("Review round 1 judged content") && t.contains("Result: findings"),
        "{t}"
    );
    assert!(
        t.contains("Review round 2 judged content") && t.contains("Result: clean"),
        "{t}"
    );
    // The unit's row on the checkpoint page and on the run page names the fix.
    let units = section(&page, "units");
    assert!(row_text(&units["rows"][0]).contains("K-1-fix-1"), "{units}");
    let run = fx.must(&["view", "run", "--json"]).json();
    assert_contract("run", &run);
    let t = row_text(&section(&run, "units")["rows"][0]);
    assert!(t.contains("K-1-fix-1"), "{t}");
    assert!(
        t.contains("Review round 1 judged content") && t.contains("covered:"),
        "{t}"
    );
}

#[test]
fn a_fix_may_not_overwrite_a_test_file_that_was_already_there() {
    let fx = Fx::new();
    plan(&fx);
    fs::create_dir_all(fx.root.join("tests")).unwrap();
    fs::write(fx.root.join("tests/old_test.txt"), "an existing test\n").unwrap();
    fx.commit_all("an existing test file");
    land_unit(&fx);
    let k = failing_run(&fx);
    one_directive(&fx, "fix-checkpoint");
    assert!(k.join("tests/old_test.txt").exists());
    let w = |f: &str| {
        guard_says(
            &fx,
            &k,
            "Write",
            json!({"file_path": k.join(f).to_string_lossy()}),
        )
    };
    assert_eq!(w("tests/old_test.txt"), "deny");
    assert_ne!(w("tests/new_test.txt"), "deny");
    // A test file a unit lists stays writable.
    assert_ne!(w("tests/a_test.txt"), "deny");
}
