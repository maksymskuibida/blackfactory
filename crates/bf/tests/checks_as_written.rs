//! A check's command is one program and its arguments, run without a shell: the engine refuses a command
//! that holds a shell operator (or, written as one string, a quote), so no check passes without running
//! what its record says. The ledger and the doctor read checks and changes as landing does.

mod common;
use common::*;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

/// Replace the record of check `id` of the tiny project with `body` (the lines after `kind`).
fn put_check(fx: &Fx, id: &str, command_line: &str, scope: &str, unit: &str) {
    let proves = if id == "X-3" {
        "[\"R-1\",\"R-2\",\"C-1\"]"
    } else {
        "[\"R-1\",\"C-1\",\"H-1\"]"
    };
    let unit_line = if unit.is_empty() {
        String::new()
    } else {
        format!("unit=\"{unit}\"\n")
    };
    let text = format!(
        "id=\"{id}\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\n{command_line}\nproves={proves}\nscope=\"{scope}\"\n{unit_line}"
    );
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("c.toml");
    fs::write(&f, text).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn receipt_files(fx: &Fx) -> usize {
    fs::read_dir(fx.root.join("factory/receipts"))
        .map(|d| d.flatten().count())
        .unwrap_or(0)
}

#[test]
fn a_command_with_an_operator_or_a_quote_is_a_plan_problem_naming_the_check_the_word_and_both_ways_out(
) {
    for (line, word) in [
        ("command=\"test -f a && test -f b\"", "&&"),
        ("command=\"ls src | wc -l\"", "|"),
        ("command=\"ls src > out.txt\"", ">"),
        ("command=\"sh -c 'test -f a'\"", "'test"),
    ] {
        let fx = Fx::new();
        fx.tiny();
        put_check(&fx, "X-3", line, "checkpoint", "");
        let o = fx.refused(&["plan", "check"]);
        let e = &o.stderr;
        assert!(e.contains("X-3") && e.contains(&format!("`{word}`")), "{e}");
        assert!(
            e.contains("one check per program") && e.contains("list"),
            "{e}"
        );
    }
}

#[test]
fn a_list_form_command_with_a_quoted_argument_and_one_program_checks_are_not_problems() {
    let fx = Fx::new();
    fx.tiny();
    put_check(
        &fx,
        "X-3",
        "command=[\"sh\", \"-c\", \"test -f a && test -f b\"]",
        "checkpoint",
        "",
    );
    fx.must(&["plan", "check"]);
}

#[test]
fn evidence_run_refuses_a_check_joined_by_and_and_writes_no_receipt() {
    let fx = Fx::new();
    fx.tiny();
    put_check(&fx, "X-3", "command=\"true && false\"", "checkpoint", "");
    let before = receipt_files(&fx);
    let o = fx.refused(&["evidence", "run", "X-3", "--", "true", "&&", "false"]);
    assert!(
        o.stderr.contains("X-3") && o.stderr.contains("`&&`"),
        "{}",
        o.stderr
    );
    assert!(
        o.stderr.contains("one check per program") && o.stderr.contains("list"),
        "{}",
        o.stderr
    );
    assert_eq!(receipt_files(&fx), before);
}

#[test]
fn an_override_of_the_unit_checks_or_the_wave_checks_joined_by_and_and_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    let o = fx.refused(&[
        "checks",
        "unit",
        "propose",
        "override",
        "lint=true && false",
    ]);
    assert!(
        o.stderr.contains("lint") && o.stderr.contains("`&&`"),
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("one check per program"), "{}", o.stderr);
    let o = fx.refused(&["checks", "wave", "propose", "override", "true && false"]);
    assert!(
        o.stderr.contains("`&&`") && o.stderr.contains("one check per program"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_wave_override_with_a_quoted_argument_is_stored_as_words_and_passes() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    fx.must(&[
        "checks",
        "wave",
        "propose",
        "override",
        "sh -c 'test -f src/a.txt'",
    ]);
    fx.human("Yes, use that wave check for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "use that wave check"]);
    let list = fx
        .must(&["record", "list", "--kind", "check", "--json"])
        .json();
    let rec = list
        .as_array()
        .cloned()
        .or_else(|| list["records"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .find(|r| r["scope"] == "wave")
        .unwrap_or_else(|| panic!("no wave check record: {list}"));
    assert_eq!(
        rec["command"],
        json!(["sh", "-c", "test -f src/a.txt"]),
        "{rec}"
    );
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/a.txt"), "x\n").unwrap();
    let id = rec["id"].as_str().unwrap().to_string();
    let o = fx.must(&[
        "evidence",
        "run",
        &id,
        "--json",
        "--",
        "sh",
        "-c",
        "test -f src/a.txt",
    ]);
    assert_eq!(o.json()["exit"], 0, "{}", o.all());
}

fn review_project(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n"),
    ] {
        let r = fx.put(k, t);
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

#[test]
fn a_probe_whose_command_holds_an_operator_is_refused_at_the_report() {
    let fx = Fx::new();
    review_project(&fx);
    fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    let copy = fx.work("K-1-review");
    fs::create_dir_all(copy.join("tests")).unwrap();
    fs::write(copy.join("tests/p1.sh"), "grep -q fixed src/a.txt\n").unwrap();
    let probe = json!({"name": "p1", "meant": "m", "showed": "s", "file": "tests/p1.sh",
        "command": "sh tests/p1.sh && true", "expect": "the file says fixed"});
    let finding = json!({"title": "t", "severity": "high", "where": "src/a.txt:1", "action": "a", "probe": "p1"});
    let body = json!({"areas": [{"area": "the close rule", "how": "read"}, {"area": "the file check", "how": "built and run"}],
        "probes": [probe], "findings": [finding]});
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
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.stderr.contains("p1") && o.stderr.contains("`&&`"),
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("one check per program"), "{}", o.stderr);
}

#[test]
fn a_handoff_proved_by_a_unit_check_that_failed_and_then_passed_shows_its_receipt() {
    let fx = Fx::new();
    fx.tiny();
    let marker_dir = tempfile::tempdir().unwrap();
    let marker = marker_dir.path().join("marker");
    put_check(
        &fx,
        "X-1",
        &format!("command=\"test -f {}\"", marker.display()),
        "unit",
        "U-1",
    );
    fx.commit_all("x1");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "U-1 content\n").unwrap();
    let cmd = [
        "test".to_string(),
        "-f".to_string(),
        marker.display().to_string(),
    ];
    let mut a: Vec<&str> = vec!["evidence", "run", "X-1", "--unit", "U-1", "--"];
    a.extend(cmd.iter().map(|s| s.as_str()));
    let o = fx.bf_in(&wc, &a);
    assert_ne!(o.code, 0, "{}", o.all());
    fs::write(&marker, "x").unwrap();
    let mut b: Vec<&str> = vec![
        "evidence",
        "run",
        "X-1",
        "--unit",
        "U-1",
        "--attempt",
        "2",
        "--reason",
        "it was flaky",
        "--",
    ];
    b.extend(cmd.iter().map(|s| s.as_str()));
    fx.must_in(&wc, &b);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    let j = fx.bf(&["ledger", "check", "--json"]).json();
    // The row of H-1 has its evidence, so it is not among the gaps.
    let gaps = j["gaps"].as_array().cloned().unwrap_or_default();
    assert!(gaps.iter().all(|g| g["id"] != "H-1"), "{j}");
}

#[test]
fn the_doctor_does_not_report_a_changed_tracked_junk_file_but_reports_a_real_change() {
    let fx = Fx::new();
    fs::write(fx.root.join(".DS_Store"), "tracked\n").unwrap();
    git(&fx.root, &["add", "-f", ".DS_Store"]);
    git(&fx.root, &["commit", "-q", "-m", "junk"]);
    fx.tiny();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    let k = fx.work("K-1");
    assert!(k.join(".git").exists(), "{}", k.display());
    fs::write(k.join(".DS_Store"), "changed\n").unwrap();
    let o = fx.bf(&["doctor"]);
    assert!(!o.all().contains("not committed"), "{}", o.all());
    let tracked = git(&k, &["ls-files"])
        .lines()
        .find(|l| *l != ".DS_Store")
        .map(|s| s.to_string());
    let tracked = tracked.expect("a tracked file");
    fs::write(k.join(&tracked), "changed for real\n").unwrap();
    let o = fx.bf(&["doctor"]);
    assert!(o.all().contains("not committed"), "{}", o.all());
}

#[test]
fn an_operator_joined_to_a_word_is_a_plan_problem_and_an_honest_word_is_not() {
    for (line, word) in [
        ("command=\"true a&&false\"", "a&&false"),
        ("command=\"ls a|b\"", "a|b"),
        ("command=\"sleep 1&\"", "1&"),
        ("command=\"cargo test 2>/dev/null\"", "2>/dev/null"),
    ] {
        let fx = Fx::new();
        fx.tiny();
        put_check(&fx, "X-3", line, "checkpoint", "");
        let o = fx.refused(&["plan", "check"]);
        assert!(o.stderr.contains(&format!("`{word}`")), "{}", o.stderr);
    }
    let fx = Fx::new();
    fx.tiny();
    put_check(
        &fx,
        "X-3",
        "command=\"cargo nextest run --filter=a:b src/x.rs\"",
        "checkpoint",
        "",
    );
    fx.must(&["plan", "check"]);
}

#[test]
fn an_unquoted_backtick_or_substitution_in_an_override_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    let o = fx.refused(&["checks", "unit", "propose", "override", "lint=echo $(x)"]);
    assert!(
        o.stderr.contains("lint") && o.stderr.contains("$(x)"),
        "{}",
        o.stderr
    );
    let o = fx.refused(&["checks", "wave", "propose", "override", "echo `x`"]);
    assert!(o.stderr.contains("`x`"), "{}", o.stderr);
    let o = fx.refused(&["checks", "wave", "propose", "override", "true a&&false"]);
    assert!(o.stderr.contains("a&&false"), "{}", o.stderr);
    fx.must(&["checks", "wave", "propose", "override", "sh -c 'a && b'"]);
}
