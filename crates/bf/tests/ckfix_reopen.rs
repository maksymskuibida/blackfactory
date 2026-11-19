//! R-70, D-89: a checkpoint fix opened again for a failed check, and the fix of a failed lint check that
//! finishes with no review round when the lint output names every change.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// A `cargo` that fails `clippy` while a file still holds the word `bad`, and names the lines in clippy's form.
const FAKE_CARGO: &str = "#!/bin/sh\n\
rc=0\n\
for f in src/a.txt src/b.txt; do\n\
  n=0\n\
  while IFS= read -r line; do\n\
    n=$((n+1))\n\
    case \"$line\" in bad*) rc=1; echo \"warning: a bad line\"; echo \" --> $f:$n:1\"; echo \"  |\"; echo \"$n | $line\"; echo \"  |\";; esac\n\
  done < \"$f\"\n\
done\n\
exit $rc\n";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"lint\"\nhow=\"command\"\ncommand=\"cargo clippy\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/b.txt"), "bad one\n").unwrap();
    fx.commit_all("records");
}

/// A lint run that fails in the checkpoint's working copy, and the fix it opens.
fn failed_lint(fx: &Fx) -> PathBuf {
    let bin = fx.root.parent().unwrap().join("fakebin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("cargo"), FAKE_CARGO).unwrap();
    fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
    fx.set_policy(
        "/usr/bin:/bin:/usr/sbin:/sbin",
        &format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
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
    fs::write(wc.join("src/a.txt"), "bad first\nkeep\n").unwrap();
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
    let o = fx.bf_in(&k, &["evidence", "run", "X-2", "--", "cargo", "clippy"]);
    assert!(!o.all().is_empty());
    k
}

fn commit(k: &std::path::Path) {
    git(k, &["add", "-A"]);
    git(k, &["commit", "-q", "-m", "fix"]);
}

fn built(fx: &Fx, k: &std::path::Path) -> Out {
    fx.bf_in(
        k,
        &[
            "report", "K-1", "--role", "builder", "--result", "built", "--json",
        ],
    )
}

fn open_phase(fx: &Fx) -> Value {
    fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"]["phase"].clone()
}

#[test]
fn ckfix_reopen_an_aborted_fix_of_a_failed_check_opens_again_with_open() {
    let fx = Fx::new();
    plan(&fx);
    let k = failed_lint(&fx);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    // A fix that is open is not opened twice.
    let o = fx.refused(&["checkpoint", "fix", "K-1", "--open"]);
    assert!(o.stderr.contains("already open"), "{}", o.stderr);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    assert!(open_phase(&fx).is_null());
    let o = fx.must(&["checkpoint", "fix", "K-1", "--open"]);
    assert!(o.stdout.contains("X-2"), "{}", o.stdout);
    assert_eq!(open_phase(&fx), "fix");
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    let _ = k;
}

#[test]
fn ckfix_reopen_refuses_when_no_check_failed() {
    let fx = Fx::new();
    plan(&fx);
    let k = failed_lint(&fx);
    fx.must(&["next", "--json"]);
    fs::write(k.join("src/a.txt"), "good\n").unwrap();
    fs::write(k.join("src/b.txt"), "good\n").unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["checkpoint", "fix", "K-1", "--open"]);
    assert!(o.stderr.contains("K-1"), "{}", o.stderr);
}

#[test]
fn ckfix_reopen_a_lint_fix_that_changes_only_the_named_lines_needs_no_review_round() {
    let fx = Fx::new();
    plan(&fx);
    let k = failed_lint(&fx);
    fx.must(&["next", "--json"]);
    // The file the lint output names but the unit never had: no grant is needed.
    fs::write(k.join("src/a.txt"), "good first\nkeep\n").unwrap();
    fs::write(k.join("src/b.txt"), "good one\n").unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "done", "{}", o.all());
    assert!(open_phase(&fx).is_null());
    let st = fx.must(&["status", "--json"]).json();
    let done = &st["checkpoint_fix"]["done"][0];
    assert!(done["files"].to_string().contains("src/b.txt"), "{done}");
    assert!(
        done["review_rounds"].as_array().unwrap().is_empty(),
        "{done}"
    );
}

#[test]
fn ckfix_reopen_a_lint_fix_that_changes_another_line_goes_to_review() {
    let fx = Fx::new();
    plan(&fx);
    let k = failed_lint(&fx);
    fx.must(&["next", "--json"]);
    fs::write(k.join("src/a.txt"), "good first\nkept\n").unwrap();
    fs::write(k.join("src/b.txt"), "good one\n").unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
    assert_eq!(open_phase(&fx), "review");
}

#[test]
fn ckfix_reopen_the_guard_lets_a_lint_fix_write_the_named_file_and_no_other() {
    let fx = Fx::new();
    plan(&fx);
    let _k = failed_lint(&fx);
    fx.must(&["next", "--json"]);
    let may = fx.must(&["next", "--json"]).json()["directives"][0]["may_change"].to_string();
    assert!(
        may.contains("src/b.txt") && may.contains("src/a.txt"),
        "{may}"
    );
}

// ---- R-58 and the K-1 case: the formatter's own changes ------------------------------------------

const SYS: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// A `cargo` whose `fmt` is the real rustfmt in check mode on `src/misc.rs` and whose `clippy` fails while
/// `src/lib.rs` holds `unwrap_or(false)`, naming the line in clippy's form.
const CARGO_K1: &str = "#!/bin/sh\n\
case \"$1\" in\n\
  fmt) exec rustfmt --check --edition 2021 src/misc.rs ;;\n\
  clippy) n=$(grep -n 'unwrap_or(false)' src/lib.rs | cut -d: -f1)\n\
    if [ -n \"$n\" ]; then echo 'warning: manual is_some_and'; echo \" --> src/lib.rs:$n:5\"; echo '  |'; echo \"$n |     x.map(|v| v > 1).unwrap_or(false)\"; echo '  |'; exit 1; fi ;;\n\
esac\n\
exit 0\n";

const LIB_BAD: &str =
    "pub fn f(x: Option<u32>) -> bool {\n    x.map(|v| v > 1).unwrap_or(false)\n}\n";
const LIB_GOOD: &str = "pub fn f(x: Option<u32>) -> bool {\n    x.is_some_and(|v| v > 1)\n}\n";
/// A blank first line and `mod` declarations out of order.
const MISC_BAD: &str = "\nmod tests;\nmod alpha;\n\npub fn g() {}\n";
const MISC_GOOD: &str = "mod alpha;\nmod tests;\n\npub fn g() {}\n";

fn rustfmt_dir() -> String {
    let path = std::env::var("PATH").unwrap();
    path.split(':')
        .find(|d| std::path::Path::new(d).join("rustfmt").exists())
        .expect("these tests need rustfmt on PATH")
        .to_string()
}

fn plan_checks(fx: &Fx, checks: &[(&str, &str)]) {
    plan_checks_ex(fx, checks, false);
}

/// `review` adds the checkpoint's review as a check of the checkpoint.
fn plan_checks_ex(fx: &Fx, checks: &[(&str, &str)], review: bool) {
    let mut ids: Vec<String> = checks.iter().map(|c| format!("\"{}\"", c.0)).collect();
    if review {
        ids.push("\"X-9\"".to_string());
    }
    let mut recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".into()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n".into()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".into()),
    ];
    if review {
        recs.push(("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".into()));
    }
    for (id, cmd) in checks {
        recs.push(("check", format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"{id}\"\nhow=\"command\"\ncommand=\"{cmd}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")));
    }
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[{}]\n", ids.join(","))));
    for (k, t) in &recs {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
}

/// The unit lands; the checkpoint's working copy then holds the repository's files.
fn land(fx: &Fx, cargo: Option<&str>) -> PathBuf {
    land_ex(fx, cargo, false)
}

/// `review` switches the checkpoint review on for the run.
fn land_ex(fx: &Fx, cargo: Option<&str>, review: bool) -> PathBuf {
    let bin = fx.root.parent().unwrap().join("fakebin");
    fs::create_dir_all(&bin).unwrap();
    let mut path = format!("{}:{SYS}", rustfmt_dir());
    if let Some(c) = cargo {
        fs::write(bin.join("cargo"), c).unwrap();
        fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
        path = format!("{}:{path}", bin.display());
    }
    fx.set_policy(SYS, &path);
    if review {
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
    fs::write(wc.join("src/a.txt"), "one\n").unwrap();
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
    fx.work("K-1")
}

fn base_files(fx: &Fx, files: &[(&str, &str)]) {
    for (f, t) in files {
        let p = fx.root.join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
    }
    fx.commit_all("records");
}

#[test]
fn ckfix_reopen_the_k1_case_is_fixed_with_no_review_round_and_no_amendment() {
    let fx = Fx::new();
    plan_checks(
        &fx,
        &[("X-2", "cargo fmt --check"), ("X-3", "cargo clippy")],
    );
    base_files(
        &fx,
        &[
            ("src/lib.rs", LIB_BAD),
            ("src/misc.rs", MISC_BAD),
            ("src/alpha.rs", "// a\n"),
            ("src/tests.rs", "// t\n"),
        ],
    );
    let k = land(&fx, Some(CARGO_K1));
    fx.bf_in(
        &k,
        &["evidence", "run", "X-2", "--", "cargo", "fmt", "--check"],
    );
    fx.bf_in(&k, &["evidence", "run", "X-3", "--", "cargo", "clippy"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    // Neither file is a file of the unit, and no grant is given.
    let may = n["directives"][0]["may_change"].to_string();
    assert!(
        may.contains("src/lib.rs") && may.contains("src/misc.rs"),
        "{may}"
    );
    fs::write(k.join("src/lib.rs"), LIB_GOOD).unwrap();
    fs::write(k.join("src/misc.rs"), MISC_GOOD).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "done", "{}", o.all());
    assert!(open_phase(&fx).is_null());
    let page = fx.must(&["view", "checkpoint", "K-1"]).json().to_string();
    assert!(page.contains("Finished with no review round"), "{page}");
    assert!(!page.contains("Reviewed clean"), "{page}");
    let st = fx.must(&["status", "--json"]).json();
    let done = &st["checkpoint_fix"]["done"][0];
    assert!(
        done["review_rounds"].as_array().unwrap().is_empty(),
        "{done}"
    );
    let log = fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    assert!(!log.contains("\"amend."), "an amendment was made");
}

#[test]
fn ckfix_reopen_a_lint_fix_that_also_changes_an_assertion_goes_to_review() {
    let fx = Fx::new();
    plan_checks(&fx, &[("X-2", "cargo clippy")]);
    let tests = "#[test]\nfn t() {\n    assert_eq!(1, 1);\n}\n";
    base_files(&fx, &[("src/lib.rs", &format!("{LIB_BAD}{tests}"))]);
    let k = land(&fx, Some(CARGO_K1));
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "cargo", "clippy"]);
    fx.must(&["next", "--json"]);
    let changed = tests.replace("(1, 1)", "(1, 2)");
    fs::write(k.join("src/lib.rs"), format!("{LIB_GOOD}{changed}")).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
}

#[test]
fn ckfix_reopen_a_file_only_the_formatter_changed_is_accepted_with_its_mark_and_checked_again() {
    let fx = Fx::new();
    plan_checks(&fx, &[("X-2", "grep -q fixed src/a.txt")]);
    base_files(
        &fx,
        &[
            ("src/misc.rs", MISC_BAD),
            ("src/alpha.rs", "// a\n"),
            ("src/tests.rs", "// t\n"),
            ("src/z.rs", "fn z( ) {}\n"),
        ],
    );
    let k = land(&fx, None);
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
    fx.must(&["next", "--json"]);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(k.join("src/misc.rs"), MISC_GOOD).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
    let b = fx.must(&["next", "--json"]).json();
    assert_eq!(b["directives"][0]["action"], "review-checkpoint-fix", "{b}");
    let brief = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "reviewer"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        brief.contains("src/misc.rs") && brief.contains("format only"),
        "{brief}"
    );
    // The reviewer files a finding; the builder then changes the accepted file again.
    let findings = serde_json::json!([{"title": "t", "severity": "medium", "where": "src/a.txt", "action": "a"}]);
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
    fs::write(
        k.join("src/misc.rs"),
        "mod alpha;\nmod tests;\n\npub fn g() { }\n",
    )
    .unwrap();
    commit(&k);
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "fixed"],
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.all().contains("src/misc.rs") && o.all().contains("no longer equals"),
        "{}",
        o.all()
    );
}

#[test]
fn ckfix_reopen_a_file_with_another_change_than_formatting_is_a_gap_naming_the_reason() {
    let fx = Fx::new();
    plan_checks(&fx, &[("X-2", "grep -q fixed src/a.txt")]);
    base_files(
        &fx,
        &[
            ("src/misc.rs", MISC_BAD),
            ("src/alpha.rs", "// a\n"),
            ("src/tests.rs", "// t\n"),
        ],
    );
    let k = land(&fx, None);
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
    fx.must(&["next", "--json"]);
    fs::write(k.join("src/a.txt"), "fixed\n").unwrap();
    fs::write(
        k.join("src/misc.rs"),
        "mod alpha;\nmod tests;\n\npub fn g() { 1; }\n",
    )
    .unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(
        o.all().contains("src/misc.rs") && o.all().contains("more than formatting"),
        "{}",
        o.all()
    );
}

// ---- F-280, F-282: what a lint fix may add, and which named files join its list -------------------

/// Fails while `src/lib.rs` holds `unwrap_or(false)`, naming that line and also a protected file, a link
/// and a file of a nested `.git` folder.
const CARGO_NAMES_MORE: &str = "#!/bin/sh\n\
n=$(grep -n 'unwrap_or(false)' src/lib.rs | cut -d: -f1)\n\
if [ -n \"$n\" ]; then\n\
  echo 'warning: manual is_some_and'\n\
  echo \" --> src/lib.rs:$n:5\"; echo '  |'; echo \"$n |     x\"; echo '  |'\n\
  echo \" --> .claude/x.rs:1:1\"\n\
  echo \" --> src/link.rs:1:1\"\n\
  echo \" --> vendor/.git/y.rs:1:1\"\n\
  exit 1\n\
fi\n\
exit 0\n";

fn lint_only_fix(fx: &Fx, cargo: &str, files: &[(&str, &str)]) -> PathBuf {
    plan_checks(fx, &[("X-2", "cargo clippy")]);
    base_files(fx, files);
    let k = land(fx, Some(cargo));
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "cargo", "clippy"]);
    fx.must(&["next", "--json"]);
    k
}

#[test]
fn ckfix_reopen_a_lint_fix_that_inserts_a_function_beside_the_named_line_goes_to_review() {
    let fx = Fx::new();
    let k = lint_only_fix(&fx, CARGO_K1, &[("src/lib.rs", LIB_BAD)]);
    fs::write(
        k.join("src/lib.rs"),
        format!("{LIB_GOOD}pub fn extra() -> u32 {{\n    7\n}}\n"),
    )
    .unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
}

#[test]
fn ckfix_reopen_a_pure_insertion_next_to_the_named_line_goes_to_review() {
    let fx = Fx::new();
    let k = lint_only_fix(&fx, CARGO_K1, &[("src/lib.rs", LIB_BAD)]);
    // The named line stays as it is for the checker's grep, so insert directly after it and fix it after.
    let text = "pub fn f(x: Option<u32>) -> bool {\n    x.is_some_and(|v| v > 1)\n    // extra\n    // lines\n}\n";
    fs::write(k.join("src/lib.rs"), text).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
}

#[test]
fn ckfix_reopen_a_named_line_rewritten_as_more_lines_goes_to_review() {
    let fx = Fx::new();
    let k = lint_only_fix(&fx, CARGO_K1, &[("src/lib.rs", LIB_BAD)]);
    let text =
        "pub fn f(x: Option<u32>) -> bool {\n    let y = x;\n    y.is_some_and(|v| v > 1)\n}\n";
    fs::write(k.join("src/lib.rs"), text).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
}

#[test]
fn ckfix_reopen_a_lint_fix_that_also_makes_the_file_executable_goes_to_review() {
    let fx = Fx::new();
    let k = lint_only_fix(&fx, CARGO_K1, &[("src/lib.rs", LIB_BAD)]);
    fs::write(k.join("src/lib.rs"), LIB_GOOD).unwrap();
    fs::set_permissions(k.join("src/lib.rs"), fs::Permissions::from_mode(0o755)).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
}

#[test]
fn ckfix_reopen_a_lint_fix_that_turns_the_named_file_into_a_binary_goes_to_review() {
    let fx = Fx::new();
    let k = lint_only_fix(&fx, CARGO_K1, &[("src/lib.rs", LIB_BAD)]);
    fs::write(k.join("src/lib.rs"), b"\0\x01\x02 binary\0").unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
}

#[test]
fn ckfix_reopen_the_output_cannot_add_a_protected_place_a_link_or_a_nested_git_file_to_the_list() {
    let fx = Fx::new();
    plan_checks(&fx, &[("X-2", "cargo clippy")]);
    fs::create_dir_all(fx.root.join(".claude")).unwrap();
    fs::create_dir_all(fx.root.join("vendor/.git")).unwrap();
    base_files(
        &fx,
        &[
            ("src/lib.rs", LIB_BAD),
            (".claude/x.rs", "// x\n"),
            ("vendor/.git/y.rs", "// y\n"),
        ],
    );
    std::os::unix::fs::symlink("lib.rs", fx.root.join("src/link.rs")).unwrap();
    fx.commit_all("link");
    let k = land(&fx, Some(CARGO_NAMES_MORE));
    fx.bf_in(&k, &["evidence", "run", "X-2", "--", "cargo", "clippy"]);
    let n = fx.must(&["next", "--json"]).json();
    let may = n["directives"][0]["may_change"].to_string();
    assert!(may.contains("src/lib.rs"), "{may}");
    for no in [".claude/x.rs", "src/link.rs", "vendor/.git/y.rs"] {
        assert!(!may.contains(no), "{no} joined the list: {may}");
    }
}

/// A fix whose only change is a file outside every unit's list that only the formatter changed: no unit is
/// attributed to it. The fix finishes through its reviewer.
fn format_only_fix_done(fx: &Fx, review: bool) {
    plan_checks_ex(fx, &[("X-2", "cmp -s src/misc.rs src/good.rs")], review);
    base_files(
        fx,
        &[
            ("src/misc.rs", MISC_BAD),
            ("src/good.rs", MISC_GOOD),
            ("src/alpha.rs", "// a\n"),
            ("src/tests.rs", "// t\n"),
        ],
    );
    let k = land_ex(fx, None, review);
    fx.bf_in(
        &k,
        &[
            "evidence",
            "run",
            "X-2",
            "--",
            "cmp",
            "-s",
            "src/misc.rs",
            "src/good.rs",
        ],
    );
    fx.must(&["next", "--json"]);
    fs::write(k.join("src/misc.rs"), MISC_GOOD).unwrap();
    commit(&k);
    let o = built(fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    );
    let st = fx.must(&["status", "--json"]).json();
    assert!(
        st["checkpoint_fix"]["open"].is_null(),
        "{}",
        st["checkpoint_fix"]
    );
}

#[test]
fn ckfix_reopen_the_checkpoint_reviewers_brief_shows_the_format_only_mark() {
    let fx = Fx::new();
    format_only_fix_done(&fx, true);
    let o = fx.must(&["brief", "K-1", "--role", "reviewer"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let text = fs::read_to_string(fx.project().briefs_dir().join("K-1-reviewer.md")).unwrap();
    assert!(text.contains("`src/misc.rs`, marked format only"), "{text}");
}

#[test]
fn ckfix_reopen_the_checkpoint_page_and_status_show_the_mark_with_no_unit_attributed() {
    let fx = Fx::new();
    format_only_fix_done(&fx, false);
    let st = fx.must(&["status", "--json"]).json();
    let done = &st["checkpoint_fix"]["done"][0];
    assert!(done["units"].as_array().unwrap().is_empty(), "{done}");
    assert_eq!(done["marks"]["src/misc.rs"], "format only", "{done}");
    let page = fx.must(&["view", "checkpoint", "K-1"]).json().to_string();
    assert!(
        page.contains("Accepted as format only: `src/misc.rs`"),
        "{page}"
    );
}

#[test]
fn ckfix_reopen_the_page_says_reviewed_clean_only_for_a_fix_that_had_a_review_round() {
    let fx = Fx::new();
    format_only_fix_done(&fx, true);
    let page = fx.must(&["view", "checkpoint", "K-1"]).json().to_string();
    assert!(page.contains("Reviewed clean"), "{page}");
    assert!(!page.contains("no review round"), "{page}");
}

#[test]
fn ckfix_reopen_a_format_fix_that_also_makes_the_formatted_file_executable_goes_to_review() {
    let fx = Fx::new();
    plan_checks(
        &fx,
        &[("X-2", "cargo fmt --check"), ("X-3", "cargo clippy")],
    );
    base_files(
        &fx,
        &[
            ("src/lib.rs", LIB_BAD),
            ("src/misc.rs", MISC_BAD),
            ("src/alpha.rs", "// a\n"),
            ("src/tests.rs", "// t\n"),
        ],
    );
    let k = land(&fx, Some(CARGO_K1));
    fx.bf_in(
        &k,
        &["evidence", "run", "X-2", "--", "cargo", "fmt", "--check"],
    );
    fx.bf_in(&k, &["evidence", "run", "X-3", "--", "cargo", "clippy"]);
    fx.must(&["next", "--json"]);
    fs::write(k.join("src/lib.rs"), LIB_GOOD).unwrap();
    fs::write(k.join("src/misc.rs"), MISC_GOOD).unwrap();
    fs::set_permissions(k.join("src/misc.rs"), fs::Permissions::from_mode(0o755)).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
    assert_eq!(open_phase(&fx), "review");
}

#[test]
fn ckfix_reopen_a_lint_fix_that_also_makes_another_file_executable_goes_to_review() {
    let fx = Fx::new();
    let k = lint_only_fix(&fx, CARGO_K1, &[("src/lib.rs", LIB_BAD)]);
    fs::write(k.join("src/lib.rs"), LIB_GOOD).unwrap();
    // A file of the unit's list: its only change is the mode.
    fs::set_permissions(k.join("src/a.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
}

#[test]
fn ckfix_reopen_the_page_says_no_review_for_a_fix_rebuilt_lint_only_after_a_findings_round() {
    let fx = Fx::new();
    let k = lint_only_fix(&fx, CARGO_K1, &[("src/lib.rs", LIB_BAD)]);
    fs::write(
        k.join("src/lib.rs"),
        format!("{LIB_GOOD}pub fn extra() -> u32 {{\n    7\n}}\n"),
    )
    .unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.json()["phase"], "review", "{}", o.all());
    let findings = serde_json::json!([{"title": "extra code", "severity": "medium", "where": "src/lib.rs", "action": "remove it"}]);
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
    fx.must(&["next", "--json"]);
    fs::write(k.join("src/lib.rs"), LIB_GOOD).unwrap();
    commit(&k);
    let o = built(&fx, &k);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["phase"], "done", "{}", o.all());
    let page = fx.must(&["view", "checkpoint", "K-1"]).json().to_string();
    assert!(page.contains("Finished with no review round"), "{page}");
    assert!(!page.contains("Reviewed clean"), "{page}");
}
