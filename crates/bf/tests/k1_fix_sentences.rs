//! K-1 fix: the sentences of R-31 (a required check runs only through the engine; a narrow command by hand
//! only debugs a failing test the engine run named; named tests take the suite's place) are in the shipped
//! role file and in the briefs the engine writes, and the proposal of the checks every unit must pass meets
//! R-31, R-33 and R-62.

mod common;
use common::*;
use std::fs;

const BUILD_KEY: &str = "cli.evidence.brief.rule.checks_engine";
const REVIEW_KEY: &str = "cli.evidence.brief.rule.reviewer_receipts";

fn plan(fx: &Fx, unit_extra: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n{unit_extra}")),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

#[test]
fn the_builder_role_file_says_run_checks_through_the_engine_and_narrow_runs_only_debug() {
    let text = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness/claude/agents/builder.md"),
    )
    .unwrap();
    assert!(text.contains("Never run the command of a required check by hand"), "{text}");
    assert!(text.contains("only through `bf evidence run`"), "{text}");
    assert!(text.contains("only to debug a failing test the engine run named"), "{text}");
    assert!(!text.contains("run only the tests the unit adds or changes, selected by name"), "{text}");
}

fn brief_of(fx: &Fx, role: &str) -> String {
    let path = fx.must(&["brief", "U-1", "--role", role]).stdout.trim().to_string();
    fs::read_to_string(path).unwrap()
}

#[test]
fn a_written_brief_says_the_same_and_the_reviewers_adds_that_it_reads_receipts() {
    let fx = Fx::new();
    plan(&fx, "full_suite=\"a light suite\"\n");
    let build = brief_of(&fx, "builder");
    let review = brief_of(&fx, "reviewer");
    let (b_rule, r_rule) = (bf::strings::t(BUILD_KEY, &[]), bf::strings::t(REVIEW_KEY, &[]));
    assert!(b_rule.contains("only through `bf evidence run`"), "{b_rule}");
    assert!(b_rule.contains("only to debug a failing test the engine run named"), "{b_rule}");
    assert!(b_rule.contains("in the suite's place"), "{b_rule}");
    assert!(build.contains(&b_rule), "{build}");
    assert!(review.contains(&b_rule) && review.contains(&r_rule), "{review}");
    assert!(!build.contains(&r_rule), "{build}");
}

/// The proposal is the same with and without named tests, and holds format and lint before the suite,
/// a suite that runs to its end, and the note that named tests take the suite's place.
#[test]
fn the_unit_checks_question_proposes_format_and_lint_before_a_suite_that_runs_to_its_end() {
    let mut seen = Vec::new();
    for extra in ["tests=\"true\"\n", "full_suite=\"a light suite\"\n"] {
        let fx = Fx::new();
        plan(&fx, extra);
        fs::write(fx.root.join(["Cargo", "toml"].join(".")), "[package]\n").unwrap();
        fx.must(&["run", "start"]);
        let q = fx.must(&["checks", "unit", "question", "--json"]).json();
        let kinds: Vec<String> = q["question"]["proposal"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds, ["format", "lint", "suite"], "{q}");
        let suite = q["question"]["proposal"][2]["command"].as_str().unwrap();
        assert!(suite.contains("nextest") && suite.contains("--no-fail-fast"), "{suite}");
        let note = q["question"]["note"].as_str().unwrap();
        assert!(note.contains(&bf::strings::t("cli.checks.unit.question.note_tests", &[])), "{note}");
        seen.push(q["question"]["proposal"].clone());
    }
    assert_eq!(seen[0], seen[1]);
}

/// R-31 in the briefs of a checkpoint fix: the builder's holds the engine sentence, the reviewer's both.
#[test]
fn a_checkpoint_fix_brief_carries_the_same_sentences_by_role() {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\nfull_suite=\"a light suite\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"grep -q fixed src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = std::path::PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    let k = fx.work("K-1");
    let _ = fx.bf_in(&k, &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "fix-checkpoint", "{n}");
    let read = |role: &str| {
        let path = fx.must(&["brief", "K-1", "--role", role]).stdout.trim().to_string();
        fs::read_to_string(path).unwrap()
    };
    let (build, review) = (read("builder"), read("reviewer"));
    let (b_rule, r_rule) = (bf::strings::t(BUILD_KEY, &[]), bf::strings::t(REVIEW_KEY, &[]));
    assert!(build.contains(&b_rule), "{build}");
    assert!(!build.contains(&r_rule), "{build}");
    assert!(review.contains(&b_rule) && review.contains(&r_rule), "{review}");
}
