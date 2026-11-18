//! U-2 (C-17, R-31 to R-33, R-62, R-63, TH-13): the checks runner keeps the whole output of a failed run,
//! runs one command once on one content, runs the checks every unit must pass in order, runs a unit's own
//! tests in the suite's place, and holds heavy checks to the plan's limit.

mod common;
use common::*;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const A: &str = "test -f src/a.txt";

struct Setup {
    fx: Fx,
    wc: PathBuf,
}

/// A plan with one unit U-1 (own check X-1), a checkpoint, and `extra` more records as (kind, text). The
/// project's list of checks every unit must pass is `list` (a TOML array body), chosen for U-1.
fn setup(unit_extra: &str, extra: &[(&str, String)], list: &str) -> Setup {
    setup_mode(unit_extra, extra, list, &["project"])
}

/// The same, with the run's answer to the question of the checks every unit must pass: `unit_mode` is the
/// words after `bf checks unit propose`.
fn setup_mode(unit_extra: &str, extra: &[(&str, String)], list: &str, unit_mode: &[&str]) -> Setup {
    let fx = Fx::new();
    if !list.is_empty() {
        fx.set_policy("list = []", &format!("list = [{list}]"));
    }
    let mut recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n{unit_extra}")),
        ("check", format!("id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"{A}\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n")),
        ("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[{}]\n", checkpoint_checks(extra))),
    ];
    for (k, t) in extra {
        recs.push((k, t.clone()));
    }
    for (k, t) in &recs {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    fx.human("yes, make this a test run with simulated approvals");
    fx.must(&[
        "test-mode",
        "grant",
        "--quote",
        "yes, make this a test run with simulated approvals",
    ]);
    fx.must(&["run", "start"]);
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    let mut propose = vec!["checks", "unit", "propose"];
    propose.extend_from_slice(unit_mode);
    fx.must(&propose);
    fx.human("Use the project's list.");
    fx.must(&["checks", "unit", "set", "--quote", "Use the project's list"]);
    fx.must(&["gate", "open", "plan"]);
    fx.must(&["gate", "approve", "plan", "--simulated"]);
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "one\n").unwrap();
    Setup { fx, wc }
}

/// The checks the checkpoint lists: X-1 and every extra check of the scope `checkpoint`.
fn checkpoint_checks(extra: &[(&str, String)]) -> String {
    let mut ids = vec!["\"X-1\"".to_string()];
    for (k, t) in extra {
        if *k == "check" && t.contains("scope=\"checkpoint\"") {
            let id = t.split('"').nth(1).unwrap_or("");
            ids.push(format!("\"{id}\""));
        }
    }
    ids.join(", ")
}

fn run_args<'a>(check: &'a str, cmd: &'a [&'a str]) -> Vec<&'a str> {
    let mut v = vec!["evidence", "run", check, "--unit", "U-1", "--"];
    v.extend_from_slice(cmd);
    v
}

fn log_count(fx: &Fx, kind: &str) -> usize {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter(|l| l.contains(&format!("\"{kind}\"")))
        .count()
}

fn receipt(fx: &Fx, id: &str) -> toml::Value {
    toml::from_str(&fs::read_to_string(fx.root.join(format!("factory/receipts/{id}.toml"))).unwrap())
        .unwrap()
}

#[test]
fn runner_one_run_counts_one_receipt_for_two_checks_that_hold_one_command() {
    let s = setup("", &[], &format!("{{ name = \"lint\", command = \"{A}\" }}"));
    let o = s.fx.must_in(&s.wc, &run_args("X-1", &["test", "-f", "src/a.txt"]));
    let first = o.stdout.split_whitespace().next().unwrap().to_string();
    let o = s
        .fx
        .must_in(&s.wc, &run_args("attached:lint", &["test", "-f", "src/a.txt"]));
    let second = o.stdout.split_whitespace().next().unwrap().to_string();
    assert_ne!(first, second);
    // One run in the log, one counted; both checks hold a receipt that passes.
    assert_eq!(log_count(&s.fx, "evidence.run"), 1);
    assert_eq!(log_count(&s.fx, "evidence.counted"), 1);
    let r = receipt(&s.fx, &second);
    assert_eq!(r["counted_from"].as_str(), Some(first.as_str()));
    assert_eq!(r["check"].as_str(), Some("attached:lint"));
    assert_eq!(r["exit"].as_integer(), Some(0));
}

#[test]
fn runner_one_run_a_changed_content_is_a_new_run() {
    let s = setup("", &[], &format!("{{ name = \"lint\", command = \"{A}\" }}"));
    s.fx.must_in(&s.wc, &run_args("X-1", &["test", "-f", "src/a.txt"]));
    fs::write(s.wc.join("src/a.txt"), "two\n").unwrap();
    s.fx.must_in(&s.wc, &run_args("attached:lint", &["test", "-f", "src/a.txt"]));
    assert_eq!(log_count(&s.fx, "evidence.run"), 2);
    assert_eq!(log_count(&s.fx, "evidence.counted"), 0);
}

#[test]
fn runner_one_run_the_same_command_elsewhere_or_in_the_sandbox_or_not_is_another_run() {
    use bf::evidence::{find_run, Receipt, RunKey};
    let key = RunKey {
        command: "cargo test".into(),
        content: "c1".into(),
        repos: "/r".into(),
        folder: ".bf/work/U-1".into(),
        sandbox: "sandbox-exec".into(),
    };
    let rec = |v: serde_json::Value| Receipt {
        v,
        seal_ok: true,
        stale: false,
        how: "command".into(),
    };
    let base = serde_json::json!({"type": "run", "check": "X-1", "command": "cargo test",
        "content_hash": "c1", "repos": "/r", "folder": ".bf/work/U-1", "sandbox": "sandbox-exec", "exit": 1});
    let receipts = vec![rec(base.clone())];
    assert!(find_run(&receipts, &key).is_some(), "a failing run is found too");
    for (field, other) in [
        ("folder", ".bf/work/U-2"),
        ("repos", "/other"),
        ("sandbox", "none"),
        ("content_hash", "c2"),
        ("command", "cargo test --all"),
    ] {
        let mut v = base.clone();
        v[field] = serde_json::json!(other);
        assert!(find_run(&[rec(v)], &key).is_none(), "{field} differs");
    }
    let mut unsealed = rec(base);
    unsealed.seal_ok = false;
    assert!(find_run(&[unsealed], &key).is_none());
}

const TWO_FAILING: &str = "#!/bin/sh\ncat <<'EOF'\n        PASS [   0.010s] bf ok::fine\n        FAIL [   0.011s] bf mod_a::first_fails\n        FAIL [   0.012s] bf mod_b::second_fails\n  stdout ---\n--- STDERR:              bf mod_a::first_fails ---\nassertion one broke\nleft 1 right 2\n\n--- STDERR:              bf mod_b::second_fails ---\nassertion two broke\n\n------------\n     Summary [   0.05s] 3 tests run: 1 passed, 2 failed\n        FAIL [   0.011s] bf mod_a::first_fails\n        FAIL [   0.012s] bf mod_b::second_fails\nEOF\nexit 100\n";

#[test]
fn runner_one_run_the_nextest_form_of_two_failures_is_read() {
    let v = bf::evidence::failing_tests(TWO_FAILING);
    let names: Vec<&str> = v.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["mod_a::first_fails", "mod_b::second_fails"]);
    assert!(v[0].1.contains("assertion one broke"), "{:?}", v[0]);
    assert!(v[1].1.contains("assertion two broke"), "{:?}", v[1]);
}

#[test]
fn runner_one_run_the_cargo_test_form_of_a_failure_is_read_too() {
    let out = "running 2 tests\ntest a::good ... ok\ntest a::bad ... FAILED\n\nfailures:\n\n---- a::bad stdout ----\nthread panicked\nsecond line\n\nfailures:\n    a::bad\n";
    let v = bf::evidence::failing_tests(out);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].0, "a::bad");
    assert!(v[0].1.contains("thread panicked"), "{:?}", v[0]);
}

#[test]
fn runner_one_run_a_failing_run_keeps_its_output_and_counts_for_the_other_check_too() {
    // Both checks hold the one command `sh suite.sh`, which fails with two failing tests.
    let list = "{ name = \"lint\", command = \"sh suite.sh\" }";
    let own = "id=\"X-9\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"sh suite.sh\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string();
    let s = setup("", &[("check", own)], list);
    fs::write(s.wc.join("suite.sh"), TWO_FAILING).unwrap();
    let o = s.fx.bf_in(&s.wc, &run_args("X-9", &["sh", "suite.sh"]));
    assert_eq!(o.code, 1, "{}", o.all());
    assert!(o.stderr.contains("mod_a::first_fails"), "{}", o.stderr);
    assert!(o.stderr.contains("mod_b::second_fails"), "{}", o.stderr);
    assert!(o.stderr.contains("assertion one broke"), "{}", o.stderr);
    assert!(o.stderr.contains("output-"), "{}", o.stderr);
    let id = o.stdout.split_whitespace().next().unwrap().to_string();
    let r = receipt(&s.fx, &id);
    let names: Vec<&str> = r["failing_tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["mod_a::first_fails", "mod_b::second_fails"]);
    // The receipt stores the path relative to the project root (F-156), the command prints it absolute.
    let rel = r["output_file"].as_str().unwrap();
    assert!(rel.starts_with("factory/receipts/output-"), "{rel}");
    assert!(o.stderr.contains(&s.fx.root.join(rel).to_string_lossy().to_string()), "{}", o.stderr);
    // No field of the receipt holds an absolute path into the project root (F-182).
    let root = s.fx.root.to_string_lossy().to_string();
    for field in ["repos", "folder"] {
        let text = r[field].to_string();
        assert!(!text.contains(&root), "{field} holds the root: {text}");
    }
    let whole = fs::read_to_string(s.fx.root.join(rel)).unwrap();
    // The folder keeps the whole outputs out of a commit.
    let ignore = fs::read_to_string(s.fx.root.join("factory/receipts/.gitignore")).unwrap();
    assert!(ignore.contains("output-*.txt"), "{ignore}");
    assert!(whole.contains("Summary"), "{whole}");
    // The same failing command under another check is no fresh try.
    let o2 = s.fx.bf_in(&s.wc, &run_args("attached:lint", &["sh", "suite.sh"]));
    assert_eq!(o2.code, 1, "{}", o2.all());
    assert!(o2.stderr.contains("mod_b::second_fails"), "{}", o2.stderr);
    assert_eq!(log_count(&s.fx, "evidence.run"), 1);
    assert_eq!(log_count(&s.fx, "evidence.counted"), 1);
}

#[test]
fn runner_one_run_lint_before_the_suite_and_no_suite_run_before_lint_passed() {
    let list = "{ name = \"lint\", command = \"test -f src/missing.txt\" }, { name = \"suite\", command = \"test -f src/a.txt\" }";
    let s = setup("", &[], list);
    let o = s.fx.bf_in(
        &s.wc,
        &run_args("attached:suite", &["test", "-f", "src/a.txt"]),
    );
    assert_ne!(o.code, 0);
    assert!(o.stderr.contains("attached:lint"), "{}", o.stderr);
    assert_eq!(log_count(&s.fx, "evidence.run"), 0);
    // Lint fails: the suite is still refused.
    let o = s.fx.bf_in(
        &s.wc,
        &run_args("attached:lint", &["test", "-f", "src/missing.txt"]),
    );
    assert_eq!(o.code, 1, "{}", o.all());
    let o = s.fx.bf_in(
        &s.wc,
        &run_args("attached:suite", &["test", "-f", "src/a.txt"]),
    );
    assert!(o.stderr.contains("attached:lint"), "{}", o.stderr);
    // Lint passes on changed content: the suite may run.
    fs::write(s.wc.join("src/missing.txt"), "x\n").unwrap();
    s.fx.must_in(
        &s.wc,
        &run_args("attached:lint", &["test", "-f", "src/missing.txt"]),
    );
    s.fx.must_in(
        &s.wc,
        &run_args("attached:suite", &["test", "-f", "src/a.txt"]),
    );
}

#[test]
fn runner_one_run_a_unit_that_names_tests_runs_them_in_the_suites_place() {
    let list = "{ name = \"lint\", command = \"test -f src/a.txt\" }, { name = \"suite\", command = \"test -f src/never.txt\" }";
    let close = "id=\"X-8\"\nkind=\"check\"\ntitle=\"s\"\nhow=\"command\"\ncommand=\"test -f src/never.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".to_string();
    let s = setup("tests=\"test -d src\"\n", &[("check", close)], list);
    s.fx.must_in(&s.wc, &run_args("attached:lint", &["test", "-f", "src/a.txt"]));
    // The suite's own command is refused for this unit: its tests run in its place.
    let o = s.fx.bf_in(
        &s.wc,
        &run_args("attached:suite", &["test", "-f", "src/never.txt"]),
    );
    assert_ne!(o.code, 0);
    assert!(o.stderr.contains("defines its own command"), "{}", o.stderr);
    let o = s.fx.must_in(&s.wc, &run_args("attached:suite", &["test", "-d", "src"]));
    assert!(o.stdout.contains("attached:suite"), "{}", o.stdout);
}

#[test]
fn runner_one_run_a_unit_that_holds_both_tests_and_a_reason_is_refused() {
    let fx = Fx::new();
    let r = fx.put(
        "unit",
        "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\ntests=\"true\"\nfull_suite=\"why\"\n",
    );
    assert_ne!(r.code, 0, "{}", r.all());
    assert!(r.all().contains("never both"), "{}", r.all());
}

#[test]
fn runner_one_run_the_plan_settings_record_holds_a_limit_of_at_least_one() {
    let fx = Fx::new();
    let bad = fx.put(
        "plan_settings",
        "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"s\"\nheavy_limit=0\n",
    );
    assert_ne!(bad.code, 0, "{}", bad.all());
    let ok = fx.put(
        "plan_settings",
        "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"s\"\nheavy_limit=1\n",
    );
    assert_eq!(ok.code, 0, "{}", ok.all());
}

fn heavy_check(id: &str, script: &str) -> (&'static str, String) {
    (
        "check",
        format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"h\"\nhow=\"command\"\ncommand=\"sh {script}\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\nheavy=true\n"),
    )
}

fn plan_problems(fx: &Fx) -> String {
    let o = fx.bf(&["plan", "check"]);
    o.all()
}

#[test]
fn runner_one_run_a_plan_with_a_heavy_check_and_no_limit_is_a_problem() {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\nfull_suite=\"light suite\"\n".to_string()),
        heavy_check("X-1", "h.sh"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n".to_string()),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    let text = plan_problems(&fx);
    assert!(text.contains("heavy_limit"), "{text}");
    let ok = fx.put(
        "plan_settings",
        "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"s\"\nheavy_limit=1\n",
    );
    assert_eq!(ok.code, 0);
    let text = plan_problems(&fx);
    assert!(!text.contains("heavy_limit"), "{text}");
}

#[test]
fn runner_one_run_a_plan_that_names_tests_refuses_a_unit_with_neither_field() {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\ntests=\"true\"\n"),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"b\"]\ncheckpoint=\"K-1\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\", \"U-2\"]\nproves=[\"R-1\"]\n"),
    ] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    let text = plan_problems(&fx);
    assert!(text.contains("U-2 neither names its `tests`"), "{text}");
    assert!(!text.contains("U-1 neither names"), "{text}");
}

#[test]
fn runner_one_run_an_old_plan_with_neither_field_reads_as_today() {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\n"),
    ] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    assert!(!plan_problems(&fx).contains("neither names its"));
}

#[test]
fn runner_one_run_two_heavy_checks_at_a_limit_of_one_do_not_overlap_and_the_second_waits() {
    let settings = (
        "plan_settings",
        "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"s\"\nheavy_limit=1\n".to_string(),
    );
    let s = setup(
        "full_suite=\"A light suite.\"\n",
        &[settings, heavy_check("X-2", "h1.sh"), heavy_check("X-3", "h2.sh")],
        "",
    );
    fs::write(s.wc.join("h1.sh"), "sleep 2\n").unwrap();
    fs::write(s.wc.join("h2.sh"), "sleep 2\n").unwrap();
    let spawn = |check: &str, script: &str| {
        Command::new(bin())
            .env("BF_STATE_HOME", &s.fx.state_base)
            .args(["evidence", "run", check, "--unit", "U-1", "--", "sh", script])
            .current_dir(&s.wc)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = spawn("X-2", "h1.sh");
    // The first one holds the place before the second starts: its place file exists.
    let places = s.fx.state_base.join("places");
    while !places.join("place-1").exists() {
        std::thread::yield_now();
    }
    let b = spawn("X-3", "h2.sh");
    let (a, b) = (a.wait_with_output().unwrap(), b.wait_with_output().unwrap());
    assert_eq!(a.status.code(), Some(0), "{}", String::from_utf8_lossy(&a.stderr));
    assert_eq!(b.status.code(), Some(0), "{}", String::from_utf8_lossy(&b.stderr));
    let (ea, eb) = (
        String::from_utf8_lossy(&a.stderr).to_string(),
        String::from_utf8_lossy(&b.stderr).to_string(),
    );
    assert!(!ea.contains("waits for a place"), "{ea}");
    assert!(eb.contains("waits for a place"), "{eb}");
    assert!(eb.contains("X-3") && eb.contains("held by pid"), "{eb}");
    // The waits are no attempt: both runs passed, so no attempt was counted against the unit.
    assert_eq!(log_count(&s.fx, "evidence.run"), 2);
    // F-149: the wait leaves a sign of life of the unit, so the idle limit does not count it.
    assert!(log_count(&s.fx, "evidence.wait") >= 1);
}

#[test]
fn runner_one_run_a_unit_that_names_tests_takes_the_suites_place_in_the_runs_override_list() {
    // F-147: the run's override list holds no `kind`; an entry named `full-suite` is the suite.
    let close = "id=\"X-8\"\nkind=\"check\"\ntitle=\"s\"\nhow=\"command\"\ncommand=\"test -f src/never.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".to_string();
    let s = setup_mode(
        "tests=\"test -d src\"\n",
        &[("check", close)],
        "",
        &["override", "lint=test -f src/a.txt", "full-suite=test -f src/never.txt"],
    );
    s.fx.must_in(&s.wc, &run_args("attached:lint", &["test", "-f", "src/a.txt"]));
    let o = s.fx.bf_in(
        &s.wc,
        &run_args("attached:full-suite", &["test", "-f", "src/never.txt"]),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    let o = s.fx.must_in(&s.wc, &run_args("attached:full-suite", &["test", "-d", "src"]));
    assert!(o.stdout.contains("attached:full-suite"), "{}", o.stdout);
}

#[test]
fn runner_one_run_a_plan_with_a_unit_of_tests_needs_a_check_that_runs_the_suite_at_the_close() {
    let fx = Fx::new();
    fx.set_policy("list = []", "list = [{ name = \"suite\", command = \"sh suite.sh\" }]");
    let unit = "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\ntests=\"true\"\n";
    let chk = "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n";
    let ck = |checks: &str| format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[{checks}]\n");
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", unit.to_string()),
        ("check", chk.to_string()),
        ("checkpoint", ck("\"X-1\"")),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    let text = plan_problems(&fx);
    assert!(text.contains("lists no check that runs the suite (sh suite.sh)"), "{text}");
    let suite = "id=\"X-2\"\nkind=\"check\"\ntitle=\"s\"\nhow=\"command\"\ncommand=\"sh suite.sh\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n";
    assert_eq!(fx.put("check", suite).code, 0);
    fs::write(fx.root.join("factory/records/K-1.toml"), ck("\"X-1\", \"X-2\"")).unwrap();
    let text = plan_problems(&fx);
    assert!(!text.contains("lists no check that runs the suite"), "{text}");
}

#[test]
fn runner_one_run_a_unit_whose_tests_hold_a_shell_operator_is_a_plan_problem() {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\ntests=\"a && b\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\n"),
    ] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    let text = plan_problems(&fx);
    assert!(text.contains("U-1: its `tests` hold `&&`"), "{text}");
}

#[test]
fn runner_one_run_a_later_check_with_no_needs_is_a_plan_note() {
    let fx = Fx::new();
    let chk = |id: &str| format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n");
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\nfull_suite=\"light\"\n".to_string()),
        ("check", chk("X-1")),
        ("check", chk("X-2")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n".to_string()),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    let pl = bf::plan::load(&fx.project()).unwrap();
    assert_eq!(pl.notes.len(), 1, "{:?}", pl.notes);
    assert!(pl.notes[0].contains("X-2") && pl.notes[0].contains("needs"), "{:?}", pl.notes);
    assert!(!pl.problems.iter().any(|x| x.contains("needs")), "{:?}", pl.problems);
    // `bf plan check` prints the note once, as the string gives it, and the JSON holds it in `notes`.
    let o = fx.bf(&["plan", "check"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let lines: Vec<&str> = o.stdout.lines().collect();
    assert!(lines[0].starts_with("plan ok"), "{}", o.stdout);
    assert_eq!(lines.len(), 2, "{}", o.stdout);
    assert_eq!(lines[1], pl.notes[0]);
    assert!(lines[1].starts_with("note: ") && !lines[1].starts_with("note: note:"), "{}", lines[1]);
    let j = fx.bf(&["plan", "check", "--json"]).json();
    assert_eq!(j["notes"], serde_json::json!(pl.notes));
}

#[test]
fn timing_runner_one_run_a_heavy_check_over_its_limit_is_stopped_and_frees_its_place() {
    let settings = (
        "plan_settings",
        "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"s\"\nheavy_limit=1\n".to_string(),
    );
    let s = setup(
        "full_suite=\"A light suite.\"\n",
        &[settings, heavy_check("X-2", "hang.sh"), heavy_check("X-3", "ok.sh")],
        "",
    );
    fs::write(s.wc.join("hang.sh"), "sleep 600\n").unwrap();
    fs::write(s.wc.join("ok.sh"), "true\n").unwrap();
    let began = std::time::Instant::now();
    let o = s.fx.bf_env(
        &s.wc,
        &run_args("X-2", &["sh", "hang.sh"]),
        "",
        &[("BF_HEAVY_LIMIT_SECONDS", "1")],
    );
    assert_eq!(o.code, 1, "{}", o.all());
    assert!(began.elapsed().as_secs() < 60);
    let id = o.stdout.split_whitespace().next().unwrap().to_string();
    let r = receipt(&s.fx, &id);
    assert_eq!(r["exit"].as_integer(), Some(124));
    assert!(r["output_tail"].as_str().unwrap().contains("longer than its limit"));
    // The place is free again: the next heavy check does not wait.
    let o = s.fx.must_in(&s.wc, &run_args("X-3", &["sh", "ok.sh"]));
    assert!(!o.stderr.contains("waits for a place"), "{}", o.stderr);
}
