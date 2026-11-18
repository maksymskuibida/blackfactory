//! U-50 (R-31): a written unit brief carries the sentences of the strings keys: the builder's brief the
//! rule that a required check runs only through the engine, the reviewer's brief that and the rule that it
//! reads receipts. The sentences are read through the keys, never written here.

mod common;
use common::*;
use std::fs;

const BUILD_KEY: &str = "cli.evidence.brief.rule.checks_engine";
const REVIEW_KEY: &str = "cli.evidence.brief.rule.reviewer_receipts";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\nfull_suite=\"fixture\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn brief_of(fx: &Fx, role: &str) -> String {
    let path = fx.must(&["brief", "U-1", "--role", role]).stdout.trim().to_string();
    fs::read_to_string(path).unwrap()
}

#[test]
fn the_unit_builder_brief_carries_the_checks_engine_sentence_and_not_the_reviewers() {
    let fx = Fx::new();
    plan(&fx);
    let (b_rule, r_rule) = (bf::strings::t(BUILD_KEY, &[]), bf::strings::t(REVIEW_KEY, &[]));
    assert!(!b_rule.is_empty() && b_rule != BUILD_KEY, "the key has no sentence: {b_rule}");
    assert!(!r_rule.is_empty() && r_rule != REVIEW_KEY, "the key has no sentence: {r_rule}");
    let build = brief_of(&fx, "builder");
    assert!(build.contains(&b_rule), "{build}");
    assert!(!build.contains(&r_rule), "{build}");
}

#[test]
fn the_unit_reviewer_brief_carries_both_sentences() {
    let fx = Fx::new();
    plan(&fx);
    let (b_rule, r_rule) = (bf::strings::t(BUILD_KEY, &[]), bf::strings::t(REVIEW_KEY, &[]));
    assert!(!b_rule.is_empty() && b_rule != BUILD_KEY, "the key has no sentence: {b_rule}");
    assert!(!r_rule.is_empty() && r_rule != REVIEW_KEY, "the key has no sentence: {r_rule}");
    let review = brief_of(&fx, "reviewer");
    assert!(review.contains(&b_rule), "{review}");
    assert!(review.contains(&r_rule), "{review}");
}
