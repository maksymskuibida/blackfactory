// R-104: a parked sync is released, and a hand-resolved sync recorded, only on words written after the sync
// needed them (the park, the opening), in the command and in the replay of the log.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn land_unit(fx: &Fx) {
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

fn commit_on_main(repo: &Path) {
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/a.txt"), "from main\n").unwrap();
    git(repo, &["add", "src/a.txt"]);
    git(repo, &["commit", "-q", "-m", "work on main"]);
}

/// A sync of main into K-1 that conflicts and stays open. `old` is written before it opens, unused.
fn open_sync(fx: &Fx, old: Option<&str>) {
    plan(fx);
    land_unit(fx);
    commit_on_main(&fx.root);
    if let Some(words) = old {
        fx.human(words);
    }
    fx.human("Yes, bring main into the checkpoint, and let the agents sort out conflicts.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--local",
            "--quote",
            "bring main into the checkpoint",
            "--json",
        ])
        .json();
    assert_eq!(o["sync_open"], true, "{o}");
}

fn ignored(fx: &Fx) -> Vec<String> {
    fx.must(&["status", "--json"]).json()["ignored_events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect()
}

fn turn_seq(fx: &Fx, text: &str) -> u64 {
    let st = bf::events::replay_log(&bf::events::load_log(&fx.project()).unwrap());
    st.turns.iter().find(|t| t.text == text).unwrap().seq
}

fn sync_phase(fx: &Fx) -> Value {
    fx.must(&["status", "--json"]).json()["sync_open"].clone()
}

#[test]
fn a_parked_syncs_release_takes_only_words_written_after_the_park() {
    let fx = Fx::new();
    open_sync(&fx, None);
    fx.set_policy("max_rounds = 3", "max_rounds = 1");
    let old = "Maybe let it go on, thanks.";
    fx.human(old);
    fx.must_in(
        &fx.work("K-1"),
        &[
            "report",
            "K-1",
            "--role",
            "builder",
            "--result",
            "blocked",
            "--note",
            "I cannot tell which line is right",
        ],
    );
    assert_eq!(sync_phase(&fx)["phase"], "parked", "{}", sync_phase(&fx));
    // The words written before the park are refused, in the command and in the replay.
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--release", "--quote", old]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("bf next"), "{}", o.stderr);
    bf::events::append(
        &fx.project(),
        "checkpoint.sync.release",
        json!({"checkpoint": "K-1", "turn": turn_seq(&fx, old), "quote": old}),
    )
    .unwrap();
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("sync release"), "{notes:?}");
    assert_eq!(sync_phase(&fx)["phase"], "parked");
    // New words release it.
    fx.human("Yes, release it and try again.");
    fx.must(&[
        "checkpoint",
        "sync",
        "K-1",
        "--release",
        "--quote",
        "release it and try",
    ]);
    assert_eq!(sync_phase(&fx)["phase"], "resolve", "{}", sync_phase(&fx));
    assert_eq!(ignored(&fx).len(), 1);
}

#[test]
fn a_hand_resolved_sync_takes_only_words_written_after_it_opened() {
    let fx = Fx::new();
    let old = "Maybe I will fix it by hand.";
    open_sync(&fx, Some(old));
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "from the unit\nfrom main\n").unwrap();
    git(&k, &["add", "src/a.txt"]);
    git(&k, &["commit", "-q", "--no-edit"]);
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--resolved", "--quote", old]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("bf next"), "{}", o.stderr);
    bf::events::append(
        &fx.project(),
        "checkpoint.sync",
        json!({"checkpoint": "K-1", "repos": [], "reopen_checks": [], "receipt_floor": 0,
               "turn": turn_seq(&fx, old), "quote": old}),
    )
    .unwrap();
    let notes = ignored(&fx);
    eprintln!("ignored: {notes:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("checkpoint sync"), "{notes:?}");
    assert!(!sync_phase(&fx).is_null(), "the sync is still open");
    fx.human("I resolved it myself, thanks.");
    let o = fx
        .must(&[
            "checkpoint",
            "sync",
            "K-1",
            "--resolved",
            "--quote",
            "resolved it myself",
            "--json",
        ])
        .json();
    assert_eq!(o["synced"], true, "{o}");
    assert!(sync_phase(&fx).is_null());
}
