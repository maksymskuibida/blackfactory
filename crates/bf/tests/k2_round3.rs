//! K-2 fix round 3: an accepted unit lands only on the content of the round that filed the findings (R-95), and
//! a resolved merge whose file keeps a setext heading is reported (the `=======` mark is a line of its own).

mod common;
use common::*;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

// ---- an accepted unit is bound to the reviewed content ----

fn state_of(fx: &Fx, id: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["units"].as_array().unwrap().iter().find(|u| u["id"] == id).unwrap()["state"]
        .as_str()
        .unwrap()
        .to_string()
}

fn parked_with_finding(fx: &Fx) -> PathBuf {
    fx.tiny();
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/hello.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    let o = fx.put(
        "finding",
        "id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\nseverity=\"high\"\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let content = fx.project().content_hash(&wc).unwrap();
    bf::events::append(
        &fx.project(),
        "unit.report",
        json!({"unit": "U-1", "result": "findings", "from": "review", "to": "parked",
               "findings": ["F-1"], "round": 1, "content": content, "head": "h", "covered": "all"}),
    )
    .unwrap();
    wc
}

#[test]
fn k2_round3_an_accepted_unit_changed_after_the_review_is_sent_back_to_review_at_landing() {
    let fx = Fx::new();
    let wc = parked_with_finding(&fx);
    let words = "I accept that finding as it is";
    fx.human(words);
    fx.must(&["unit", "accept", "U-1", "F-1", "--quote", words]);
    assert_eq!(state_of(&fx, "U-1"), "reviewed");
    // Work no reviewer saw, with fresh receipts.
    fs::write(wc.join("src/hello.txt"), "hello, changed after the review\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/hello.txt"]);
    let o = fx.refused(&["unit", "land", "U-1"]);
    assert!(o.all().contains("review"), "{}", o.all());
    assert_eq!(state_of(&fx, "U-1"), "review");
}

#[test]
fn k2_round3_an_accepted_unit_with_the_reviewed_content_still_lands() {
    let fx = Fx::new();
    parked_with_finding(&fx);
    let words = "I accept that finding as it is";
    fx.human(words);
    fx.must(&["unit", "accept", "U-1", "F-1", "--quote", words]);
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state_of(&fx, "U-1"), "landed");
}

// ---- a resolved merge with a setext heading ----

fn plan(fx: &Fx) {
    let mut recs = vec![(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string(),
    )];
    for (n, f) in [(1, "a"), (2, "b")] {
        recs.push(("unit", format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/{f}.md\"]\ncheckpoint=\"K-1\"\n")));
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

fn write(wc: &Path, file: &str, text: &str) {
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join(file), text).unwrap();
}

fn merging(wc: &Path) -> bool {
    let g = git(wc, &["rev-parse", "--git-path", "MERGE_HEAD"]);
    let p = PathBuf::from(&g);
    (if p.is_absolute() { p } else { wc.join(g) }).exists()
}

#[test]
fn k2_round3_a_resolved_markdown_file_with_a_setext_heading_finishes_the_merge() {
    let fx = Fx::new();
    plan(&fx);
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-2", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    write(&wc, "src/b.md", "mine b\n");
    let list = git(&fx.root, &["worktree", "list", "--porcelain"]);
    let ck = list
        .split("\n\n")
        .find(|b| b.contains("refs/heads/bf/K-1"))
        .and_then(|b| b.lines().next())
        .map(|l| PathBuf::from(l.trim_start_matches("worktree ")))
        .expect("the checkpoint worktree");
    write(&ck, "src/b.md", "theirs\n");
    git(&ck, &["add", "-A"]);
    git(&ck, &["commit", "-q", "-m", "theirs"]);
    fx.must(&["unit", "start", "U-2"]);
    assert!(merging(&wc));
    assert!(fs::read_to_string(wc.join("src/b.md")).unwrap().contains("<<<<<<<"));
    // Resolved by hand; the file keeps a setext heading whose underline starts with the seven signs.
    write(&wc, "src/b.md", "Title\n==========\n\nresolved\n");
    fx.must_in(&wc, &["evidence", "run", "X-2", "--unit", "U-2", "--", "true"]);
    let o = fx.bf_in(&wc, &["report", "U-2", "--result", "built"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(!merging(&wc));
}

#[test]
fn k2_round3_a_lone_separator_line_is_still_a_conflict_mark() {
    let fx = Fx::new();
    plan(&fx);
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-2", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    write(&wc, "src/b.md", "mine b\n");
    let list = git(&fx.root, &["worktree", "list", "--porcelain"]);
    let ck = list
        .split("\n\n")
        .find(|b| b.contains("refs/heads/bf/K-1"))
        .and_then(|b| b.lines().next())
        .map(|l| PathBuf::from(l.trim_start_matches("worktree ")))
        .expect("the checkpoint worktree");
    write(&ck, "src/b.md", "theirs\n");
    git(&ck, &["add", "-A"]);
    git(&ck, &["commit", "-q", "-m", "theirs"]);
    fx.must(&["unit", "start", "U-2"]);
    write(&wc, "src/b.md", "one\n=======\ntwo\n");
    fx.must_in(&wc, &["evidence", "run", "X-2", "--unit", "U-2", "--", "true"]);
    let o = fx.bf_in(&wc, &["report", "U-2", "--result", "built"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(merging(&wc));
}
