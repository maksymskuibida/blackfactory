//! The design reviewer's role file names every kind with its rules, the stage review request and
//! its limit, and forbids approving and deciding for the human.

use std::fs;
use std::path::{Path, PathBuf};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn role() -> String {
    read("harness/claude/agents/design-reviewer.md")
}

fn has(text: &str, needles: &[&str]) {
    for n in needles {
        assert!(text.contains(n), "the role file should contain `{n}`");
    }
}

#[test]
fn the_role_file_has_a_header_with_read_and_search_tools_and_bash() {
    let t = role();
    has(&t, &["name: design-reviewer", "tools: Read, Glob, Grep, Bash"]);
}

#[test]
fn the_role_file_reads_only_what_the_brief_names() {
    has(
        &role(),
        &[
            "Never read the drafts folder, the conductor's chat or any notes",
            "Read only those and the files the brief names",
        ],
    );
}

#[test]
fn the_role_file_names_every_kind() {
    has(
        &role(),
        &[
            "**idea**",
            "**requirements**",
            "**architecture**",
            "**plan**",
            "**legal**",
            "**amendment**",
            "**agreement**",
        ],
    );
}

#[test]
fn the_role_file_carries_the_rules_of_each_kind() {
    has(
        &role(),
        &[
            "every claim has a source tag",
            "the assumptions the go decision rests on are named",
            "each success measure can be measured",
            "scope, what v1 leaves out and the risks are stated and do not contradict each other",
            "every acceptance statement can be tested",
            "no two requirements contradict each other",
            "the requirements cover the idea",
            "every requirement is served by a component or a decision",
            "every decision names the options it rejected",
            "data, money, identity or the network",
            "every target can be measured",
            "every requirement is delivered by a unit and proved by a check",
            "at most 8 points",
            "each check proves what it claims",
            "the stops are sensible",
            "the markets are named",
            "each legal record has a source",
            "every stage the amendment touches by the rules of that stage",
            "the stage coverage",
            "what the amendment leaves unproved",
            "judge only that the stages agree with each other",
        ],
    );
}

#[test]
fn the_role_file_says_what_a_finding_holds_and_when_an_earlier_one_is_fixed() {
    has(
        &role(),
        &[
            "a severity (critical, high, medium, low)",
            "the action that fixes it",
            "would make the human approve something wrong",
            "Mark an earlier finding fixed only after reading the change",
        ],
    );
}

#[test]
fn the_role_file_limits_the_stage_review_request() {
    has(
        &role(),
        &[
            "Only the solo amendment reviewer may ask",
            "large or specialised",
            "names the stage review you ask for",
            "at most once per stage",
            "A stage reviewer never asks",
        ],
    );
}

#[test]
fn the_role_file_forbids_approving_and_deciding_for_the_human() {
    has(
        &role(),
        &[
            "Decide anything for the human, or approve anything",
            "The human accepts a finding only in their own words",
        ],
    );
}

#[test]
fn the_role_file_settles_the_review_and_stops_when_parked() {
    has(
        &role(),
        &[
            "bf evidence attach <check> <file> --outcome met|failed",
            "one sentence",
            "because it is parked, stop and say so",
        ],
    );
}

/// Every file under the folders, at any depth, with more than `cap` lines, with its count.
fn files_over(dirs: &[PathBuf], cap: usize) -> Vec<(PathBuf, usize)> {
    let mut over = Vec::new();
    for dir in dirs {
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let n = fs::read_to_string(&p).unwrap().lines().count();
                    if n > cap {
                        over.push((p, n));
                    }
                }
            }
        }
    }
    over.sort();
    over
}

#[test]
fn each_harness_instruction_file_stays_at_most_two_hundred_lines() {
    let dirs = [repo("harness/claude/skills"), repo("harness/claude/agents")];
    let over = files_over(&dirs, 200);
    let named: Vec<String> = over
        .iter()
        .map(|(p, n)| format!("{} has {n} lines", p.display()))
        .collect();
    assert!(over.is_empty(), "{}", named.join("; "));
}

#[test]
fn the_walk_returns_only_the_file_over_the_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let sub = tmp.path().join("deep");
    fs::create_dir(&sub).unwrap();
    fs::write(tmp.path().join("ok.md"), "x\n".repeat(200)).unwrap();
    fs::write(sub.join("long.md"), "x\n".repeat(201)).unwrap();
    let over = files_over(&[tmp.path().to_path_buf()], 200);
    assert_eq!(over, vec![(sub.join("long.md"), 201)]);
}
