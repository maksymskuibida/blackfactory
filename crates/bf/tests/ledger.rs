//! The ledger and the close follow one rule for an ID whose only checks are switched off.

mod common;
use common::Fx;
use std::fs;

/// A tiny plan whose only checkpoint check carries the name of a check the human switched off.
fn plan_with_off_check(fx: &Fx) {
    fx.tiny();
    let x3 = "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"ls src/hello.txt src/bye.txt\"\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nscope=\"checkpoint\"\nnamed=\"accessibility\"\n";
    let f = fx.root.join("x3.toml");
    fs::write(&f, x3).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let _ = fs::remove_file(&f);
    fx.commit_all("x3");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
}

#[test]
fn close_ledger_and_run_stop_agree_and_each_names_the_ids_left_unproven() {
    let fx = Fx::new();
    plan_with_off_check(&fx);
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
    for id in ["R-1", "R-2", "C-1"] {
        assert!(o.stdout.contains(id), "{}", o.all());
    }
    assert!(o.stdout.contains("accessibility"), "{}", o.all());
    let l = fx.bf(&["ledger", "check"]);
    assert_eq!(l.code, 0, "{}", l.all());
    assert!(l.stdout.contains("not demanded"), "{}", l.all());
    for id in ["R-1", "R-2", "C-1"] {
        assert!(l.stdout.contains(id), "{}", l.all());
    }
    let j = fx.bf(&["ledger", "check", "--json"]).json();
    assert_eq!(j["unproven_off"].as_array().unwrap().len(), 3, "{j}");
    let s = fx.bf(&["run", "stop"]);
    assert_eq!(s.code, 0, "{}", s.all());
    for id in ["R-1", "R-2", "C-1"] {
        assert!(s.stdout.contains(id), "{}", s.all());
    }
}

#[test]
fn a_check_off_for_the_first_run_only_is_judged_by_that_run() {
    let fx = Fx::new();
    plan_with_off_check(&fx);
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf(&["run", "stop"]);
    assert_eq!(o.code, 0, "{}", o.all());
    // The human switches the check back on for the project, and a second run starts.
    let o = fx.bf(&[
        "checks",
        "propose",
        "--scope",
        "default",
        "accessibility=on",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Yes, switch accessibility back on.");
    let o = fx.bf(&["checks", "set", "--quote", "switch accessibility back on"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.bf(&["run", "start"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let l = fx.bf(&["ledger", "check"]);
    assert_eq!(l.code, 0, "{}", l.all());
    for id in ["R-1", "R-2", "C-1"] {
        assert!(l.stdout.contains(id), "{}", l.all());
    }
    assert!(l.stdout.contains("off in run-1"), "{}", l.all());
    let s = fx.bf(&["run", "stop"]);
    assert_eq!(s.code, 0, "{}", s.all());
    for id in ["R-1", "R-2", "C-1"] {
        assert!(s.stdout.contains(id), "{}", s.all());
    }
    assert!(s.stdout.contains("off in run-1"), "{}", s.all());
}
