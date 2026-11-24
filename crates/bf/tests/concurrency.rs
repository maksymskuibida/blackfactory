//! Engine commands that change a checkpoint's working copy take turns.

mod common;
use common::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn two_planned_units(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"a\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"b\"\ndelivers=[\"R-1\"]\nfiles=[\"src/b.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"a\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"b\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"both\"\nhow=\"command\"\ncommand=\"ls src/a.txt src/b.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\"]\nchecks=[\"X-3\"]\n"),
    ] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    fx.commit_all("records");
    fx.start_test_run();
}

fn two_reviewed_units(fx: &Fx) {
    two_planned_units(fx);
    for (u, f, x) in [("U-1", "src/a.txt", "X-1"), ("U-2", "src/b.txt", "X-2")] {
        let wc = PathBuf::from(
            fx.must(&["unit", "start", u, "--json"]).json()["working_copy"]
                .as_str()
                .unwrap(),
        );
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join(f), "x\n").unwrap();
        fx.must_in(
            &wc,
            &["evidence", "run", x, "--unit", u, "--", "test", "-f", f],
        );
        fx.must(&["report", u, "--result", "built"]);
        fx.must(&["report", u, "--result", "built", "--role", "reviewer"]);
    }
}

#[test]
fn two_landings_at_the_same_moment_leave_nothing_half_done() {
    // The race is narrow, so it is tried several times.
    for _ in 0..6 {
        let fx = Fx::new();
        two_reviewed_units(&fx);
        let mut hs = Vec::new();
        for u in ["U-1", "U-2"] {
            let root = fx.root.clone();
            let sb = fx.state_base.clone();
            hs.push(std::thread::spawn(move || {
                let o = Command::new(bin())
                    .current_dir(&root)
                    .args(["unit", "land", u])
                    .env("BF_STATE_HOME", &sb)
                    .output()
                    .unwrap();
                (
                    o.status.code(),
                    String::from_utf8_lossy(&o.stderr).to_string(),
                )
            }));
        }
        for h in hs {
            let (code, err) = h.join().unwrap();
            // Either it landed, or it was refused cleanly because another command was running.
            assert!(
                code == Some(0) || err.contains("another engine command is running"),
                "{code:?} {err}"
            );
        }
        // Whatever was refused lands when run again; the working copy was left whole.
        let wt = fx.work("K-1");
        assert_eq!(
            git(&wt, &["status", "--porcelain", "--untracked-files=no"]),
            ""
        );
        for u in ["U-1", "U-2"] {
            fx.must(&["unit", "land", u]);
        }
        let tree = git(&fx.root, &["ls-tree", "-r", "--name-only", "bf/K-1", "src"]);
        assert!(
            tree.contains("src/a.txt") && tree.contains("src/b.txt"),
            "{tree}"
        );
        let d = fx.bf(&["doctor"]);
        assert!(!d.stdout.contains("PROBLEM"), "{}", d.all());
    }
}

fn spawn_bf(fx: &Fx, args: Vec<String>, stdin: String) -> std::thread::JoinHandle<(i32, String)> {
    let root = fx.root.clone();
    let sb = fx.state_base.clone();
    std::thread::spawn(move || {
        use std::io::Write;
        let mut ch = Command::new(bin())
            .current_dir(&root)
            .args(&args)
            .env("BF_STATE_HOME", &sb)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        ch.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
        let o = ch.wait_with_output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).to_string(),
        )
    })
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

#[test]
fn two_units_started_at_the_same_moment_both_start() {
    for _ in 0..4 {
        let fx = Fx::new();
        two_planned_units(&fx);
        let hs: Vec<_> = ["U-1", "U-2"]
            .iter()
            .map(|u| spawn_bf(&fx, args(&["unit", "start", u]), String::new()))
            .collect();
        for h in hs {
            let (code, err) = h.join().unwrap();
            assert_eq!(code, 0, "{err}");
        }
    }
}

#[test]
fn a_landing_beside_a_start_both_go_through() {
    for _ in 0..4 {
        let fx = Fx::new();
        two_planned_units(&fx);
        let wc = PathBuf::from(
            fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
                .as_str()
                .unwrap(),
        );
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join("src/a.txt"), "x\n").unwrap();
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
        let h1 = spawn_bf(&fx, args(&["unit", "land", "U-1"]), String::new());
        let h2 = spawn_bf(&fx, args(&["unit", "start", "U-2"]), String::new());
        for h in [h1, h2] {
            let (code, err) = h.join().unwrap();
            assert_eq!(code, 0, "{err}");
        }
    }
}

#[test]
fn a_waiting_command_goes_through_when_the_lock_is_freed() {
    let fx = Fx::new();
    two_reviewed_units(&fx);
    let p = fx.project();
    let held = bf::lock::engine(&p, "hold").unwrap();
    let h = spawn_bf(&fx, args(&["unit", "land", "U-1"]), String::new());
    // The landing takes its checkpoint's turn lock and then waits for the engine lock, so an open
    // turn shows that the command has reached the wait. Poll for it, for 5 seconds at most.
    let started = std::time::Instant::now();
    while !bf::lock::turn_open(&p, "K-1") {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the command never reached the lock"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !h.is_finished(),
        "the command finished while the lock was held"
    );
    drop(held);
    let (code, err) = h.join().unwrap();
    assert_eq!(code, 0, "{err}");
}

#[test]
fn findings_filed_at_the_same_moment_each_keep_their_own_number() {
    for _ in 0..3 {
        let fx = Fx::new();
        two_planned_units(&fx);
        for (u, f, x) in [("U-1", "src/a.txt", "X-1"), ("U-2", "src/b.txt", "X-2")] {
            let wc = PathBuf::from(
                fx.must(&["unit", "start", u, "--json"]).json()["working_copy"]
                    .as_str()
                    .unwrap(),
            );
            fs::create_dir_all(wc.join("src")).unwrap();
            fs::write(wc.join(f), "x\n").unwrap();
            fx.must_in(
                &wc,
                &["evidence", "run", x, "--unit", u, "--", "test", "-f", f],
            );
            fx.must(&["report", u, "--result", "built"]);
        }
        let hs: Vec<_> = ["U-1", "U-2"]
            .iter()
            .map(|u| {
                let list = format!(
                    "[{{\"title\":\"finding on {u}\",\"severity\":\"medium\",\"where\":\"src/x.txt\",\"action\":\"fix it\"}}]"
                );
                spawn_bf(
                    &fx,
                    args(&["report", u, "--result", "findings", "--role", "reviewer", "--findings", "-"]),
                    list,
                )
            })
            .collect();
        for h in hs {
            let (code, err) = h.join().unwrap();
            assert_eq!(code, 0, "{err}");
        }
        let l = fx
            .must(&["record", "list", "--kind", "finding", "--json"])
            .json();
        assert_eq!(l.as_array().unwrap().len(), 2, "{l}");
    }
}

#[test]
fn role_tokens_issued_at_the_same_moment_are_all_kept() {
    let fx = Fx::new();
    fx.tiny();
    let p = fx.project();
    let hs: Vec<_> = (0..8)
        .map(|i| {
            let p = p.clone();
            std::thread::spawn(move || bf::run::issue_token(&p, &format!("U-{i}:scout")).unwrap())
        })
        .collect();
    let toks: Vec<String> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    for (i, t) in toks.iter().enumerate() {
        assert_eq!(
            &bf::run::issue_token(&p, &format!("U-{i}:scout")).unwrap(),
            t
        );
    }
}

#[test]
fn a_command_that_waits_too_long_is_refused_cleanly() {
    let fx = Fx::new();
    two_reviewed_units(&fx);
    let p = fx.project();
    let held = bf::lock::engine(&p, "hold").unwrap();
    let o = fx.bf_env(
        &fx.root,
        &["unit", "land", "U-1"],
        "",
        &[("BF_LOCK_WAIT_MS", "300")],
    );
    assert_ne!(o.code, 0);
    assert!(
        o.all().contains("another engine command is running"),
        "{}",
        o.all()
    );
    drop(held);
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn the_doctor_reports_a_checkpoint_working_copy_with_changes() {
    let fx = Fx::new();
    two_reviewed_units(&fx);
    fx.must(&["unit", "land", "U-1"]);
    let wt = fx.work("K-1");
    assert!(!fx.bf(&["doctor"]).stdout.contains("PROBLEM"));
    // A staged rename, as a landing that stopped half way leaves.
    git(&wt, &["mv", "src/a.txt", "src/c.txt"]);
    let d = fx.bf(&["doctor"]);
    assert!(d.stdout.contains("PROBLEM"), "{}", d.all());
    assert!(d.all().contains("K-1"), "{}", d.all());
}
