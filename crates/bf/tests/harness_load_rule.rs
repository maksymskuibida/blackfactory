//! A busy machine is a warning only (D-110, D-111): the role files and the conductor's skill never say to skip
//! a check for load. Each test reads the shipped text.

use std::fs;
use std::path::{Path, PathBuf};

fn read(rel: &str) -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const FILES: [&str; 5] = [
    "harness/claude/agents/verifier.md",
    "harness/claude/agents/reviewer.md",
    "harness/claude/agents/builder.md",
    "harness/claude/skills/factory/checkpoint.md",
    "harness/claude/skills/factory/build-loop.md",
];

#[test]
fn no_file_skips_a_check_for_load() {
    for f in FILES {
        let t = read(f);
        assert!(!t.contains("do not run it"), "{f} says do not run it");
        for l in t.lines().filter(|l| l.contains("busy machine")) {
            assert!(
                !l.contains("bf evidence unreachable"),
                "{f} marks a check unreachable for load"
            );
        }
    }
}

#[test]
fn the_role_files_hold_the_rule() {
    for f in &FILES[..3] {
        let t = read(f);
        for s in [
            "On a busy machine the load is a warning only",
            "a check runs when its turn comes",
            "its receipt carries the load the engine saw",
            "with the human-absent flag on, end your turn without running it",
            "No sleep, poll or retry loop in either case",
        ] {
            assert!(t.contains(s), "{f} lacks: {s}");
        }
    }
}

#[test]
fn the_builder_formats_before_it_reports() {
    assert!(read(FILES[2]).contains("run the project's formatter on the files you changed"));
}

#[test]
fn the_skill_files_hold_the_rule() {
    for f in &FILES[3..] {
        let t = read(f);
        for s in [
            "never tells a worker to skip a check for load",
            "run again, accept by amendment, open a fix unit",
            "instead of running a third time",
        ] {
            assert!(t.contains(s), "{f} lacks: {s}");
        }
    }
    assert!(read("harness/claude/skills/factory/amendments.md")
        .contains("Never ask a designer to measure, build or test"));
}
