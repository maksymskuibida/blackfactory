//! Engine commands that change a checkpoint's working copy take turns.

mod common;
use common::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn two_reviewed_units(fx: &Fx) {
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

#[test]
fn a_second_command_is_refused_at_once_while_one_holds_the_lock() {
    let fx = Fx::new();
    two_reviewed_units(&fx);
    let p = fx.project();
    let held = bf::lock::engine(&p, "hold").unwrap();
    let o = fx.bf(&["unit", "land", "U-1"]);
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
