//! U-107 (D-115, D-116, FU-47): a landing whose merge conflicts sends the unit back to its builder
//! with the conflict left in its working copy, one review round judges the resolution, a unit may
//! change files outside its list, and a conflict in a protected path is aborted.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn plan(fx: &Fx) {
    let mut recs = vec![(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string(),
    )];
    for (n, f) in [(1, "a"), (2, "b")] {
        recs.push(("unit", format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/{f}.txt\"]\ncheckpoint=\"K-1\"\n")));
        recs.push(("check", format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n")));
    }
    recs.push(("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"k\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".into()));
    recs.push(("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\"]\nchecks=[\"X-9\"]\n".into()));
    for (k, t) in recs {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.commit_all("records");
    fx.start_test_run();
}

fn start(fx: &Fx, u: &str) -> PathBuf {
    PathBuf::from(fx.must(&["unit", "start", u, "--json"]).json()["working_copy"].as_str().unwrap())
}

fn write(wc: &Path, file: &str, text: &str) {
    if let Some(d) = wc.join(file).parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(wc.join(file), text).unwrap();
}

fn proof(fx: &Fx, wc: &Path, n: &str) {
    let (x, u) = (format!("X-{n}"), format!("U-{n}"));
    fx.must_in(wc, &["evidence", "run", &x, "--unit", &u, "--", "true"]);
}

fn state(fx: &Fx, u: &str) -> bf::events::UnitSt {
    bf::events::replay(&fx.project()).unwrap().units[u].clone()
}

fn merging(wc: &Path) -> bool {
    let g = git(wc, &["rev-parse", "--git-path", "MERGE_HEAD"]);
    let p = PathBuf::from(&g);
    (if p.is_absolute() { p } else { wc.join(g) }).exists()
}

fn log(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// Both units are started; each writes `file` and is built and reviewed clean.
fn both_built(fx: &Fx, file: &str) -> (PathBuf, PathBuf) {
    let w1 = start(fx, "U-1");
    let w2 = start(fx, "U-2");
    for (w, n, text) in [(&w1, "1", "one\n"), (&w2, "2", "two\n")] {
        write(w, file, text);
        proof(fx, w, n);
        let u = format!("U-{n}");
        fx.must(&["report", &u, "--result", "built"]);
        fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
    }
    (w1, w2)
}

#[test]
fn land_conflict_goes_to_the_builder_then_one_review_round_then_lands() {
    let fx = Fx::new();
    plan(&fx);
    let w1 = start(&fx, "U-1");
    let w2 = start(&fx, "U-2");
    for (w, n, text) in [(&w1, "1", "one\n"), (&w2, "2", "two\n")] {
        write(w, "src/shared.txt", text);
        proof(&fx, w, n);
    }
    write(&w1, "src/from_one.txt", "brought in from one\n");
    proof(&fx, &w1, "1");
    for n in ["1", "2"] {
        let u = format!("U-{n}");
        fx.must(&["report", &u, "--result", "built"]);
        fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
    }
    fx.must(&["unit", "land", "U-1"]);
    // The second landing conflicts: the unit goes back to its builder.
    let o = fx.bf(&["unit", "land", "U-2"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("src/shared.txt") && o.all().contains("builder"), "{}", o.all());
    let u2 = state(&fx, "U-2");
    assert_eq!(u2.state, "fixing");
    assert_eq!(u2.attempts, 0);
    assert!(merging(&w2));
    assert!(fs::read_to_string(w2.join("src/shared.txt")).unwrap().contains("<<<<<<<"));
    let evs = log(&fx);
    assert!(!evs.iter().any(|e| e["kind"] == "unit.park"));
    let sent = evs.iter().rev().find(|e| e["kind"] == "unit.report" && e["unit"] == "U-2").unwrap();
    assert_eq!((sent["role"].as_str(), sent["from"].as_str(), sent["to"].as_str()), (Some("engine"), Some("reviewed"), Some("fixing")));
    // The builder's brief names the file and holds the engine's note.
    let brief = fs::read_to_string(fx.project().briefs_dir().join("U-2-builder.md")).unwrap();
    assert!(brief.contains("src/shared.txt") && brief.contains("Sent back from landing"), "{brief}");
    assert!(brief.contains(sent["note"].as_str().unwrap()), "{brief}");
    // The builder resolves; its report finishes the merge and the unit goes to review.
    write(&w2, "src/shared.txt", "resolved\n");
    proof(&fx, &w2, "2");
    let o = fx.bf_in(&w2, &["report", "U-2", "--result", "built"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(state(&fx, "U-2").state, "review");
    let rb = fs::read_to_string(fx.project().briefs_dir().join("U-2-reviewer.md")).unwrap();
    assert!(rb.contains("resolving the landing conflict") && rb.contains("+resolved"), "{rb}");
    assert!(!rb.contains("brought in from one"), "{rb}");
    // One clean round and it lands as any unit.
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    assert_eq!(state(&fx, "U-2").state, "reviewed");
    fx.must(&["unit", "land", "U-2"]);
    let u2 = state(&fx, "U-2");
    assert_eq!((u2.state.as_str(), u2.attempts), ("landed", 0));
}

#[test]
fn land_conflict_a_unit_that_changes_a_file_outside_its_list_lands() {
    let fx = Fx::new();
    plan(&fx);
    fx.build_unit("U-1", "src/elsewhere.txt", "X-1", &["true"]);
    assert_eq!(state(&fx, "U-1").state, "landed");
}

#[test]
fn land_conflict_a_conflict_in_a_protected_path_is_aborted_and_named() {
    let fx = Fx::new();
    plan(&fx);
    let (_w1, w2) = both_built(&fx, ".claude/settings.json");
    fx.must(&["unit", "land", "U-1"]);
    let o = fx.bf(&["unit", "land", "U-2"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains(".claude/settings.json") && o.all().contains("protected"), "{}", o.all());
    let sync = log(&fx).into_iter().rev().find(|e| e["kind"] == "unit.sync").unwrap();
    assert_eq!(sync["how"], "aborted");
    assert_eq!(sync["files"], serde_json::json!([".claude/settings.json"]));
    assert!(!merging(&w2));
    assert_eq!(fs::read_to_string(w2.join(".claude/settings.json")).unwrap(), "two\n");
}
