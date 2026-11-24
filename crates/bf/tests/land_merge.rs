//! U-99 (D-115, D-116, FU-47): one landing turn for each checkpoint. The turn merges the reviewed units
//! together, runs the run's suite once on the merged content, lands the units in plan order, and sends
//! back only the unit whose merged content fails twice.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn files() -> [(&'static str, &'static str); 3] {
    [("1", "a"), ("2", "b"), ("3", "c")]
}

/// Three units of K-1, each with its own file and unit check, and a run whose suite is `sh suite.sh`: the
/// script logs where it ran, and fails in the checkpoint's copy on a file or a marker the test sets.
fn plan(fx: &Fx) {
    plan_with(fx, false);
}

/// With `named`, each of the three units names its tests (`true`) and a fourth unit names none.
fn plan_with(fx: &Fx, named: bool) {
    plan_full(fx, named, &["suite=sh suite.sh"], None);
}

/// The run's list as `name=command` entries, or, with `project`, the project's own list in the policy.
fn plan_full(fx: &Fx, named: bool, list: &[&str], project: Option<&str>) {
    if let Some(l) = project {
        fx.set_policy("list = []", &format!("list = [{l}]"));
    }
    let mut recs = vec![(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string(),
    )];
    for (n, f) in files() {
        recs.push(("unit", format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u{n}\"\ndelivers=[\"R-1\"]\nfiles=[\"src/{f}.txt\"]\ncheckpoint=\"K-1\"\n{}\n", if named { "tests=\"true\"" } else { "" })));
        recs.push(("check", format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n")));
    }
    if named {
        recs.push(("unit", "id=\"U-4\"\nkind=\"unit\"\ntitle=\"u4\"\ndelivers=[\"R-1\"]\nfiles=[\"src/d.txt\"]\ncheckpoint=\"K-1\"\n".to_string()));
        recs.push(("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-4\"\n".to_string()));
    }
    if named {
        let close = "sh suite.sh";
        recs.push(("check", format!("id=\"X-8\"\nkind=\"check\"\ntitle=\"s\"\nhow=\"command\"\ncommand=\"{close}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")));
    }
    recs.push(("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"k\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".into()));
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\",\"U-2\",\"U-3\"{}]\nproves=[\"R-1\"]\nchecks=[\"X-9\"{}]\n", if named { ",\"U-4\"" } else { "" }, if named { ",\"X-8\"" } else { "" })));
    for (k, t) in recs {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    let out = out_dir(fx);
    fs::create_dir_all(&out).unwrap();
    // The scripts find the out folder from where they run and log the folder's name below `.bf/`, so no
    // file of the project holds an absolute path (a copied state keeps none of its build folder).
    let find_out = "case \"$PWD\" in */.bf/work/*) out=\"$PWD/../../../../out\" ;; *) out=\"$PWD/../out\" ;; esac\n";
    let script = format!(
        "{find_out}echo \"${{PWD##*/.bf/}}\" >> \"$out/suite.log\"\ncase \"$PWD\" in */K-1) ;; *) exit 0 ;; esac\nif [ -f \"$out/untrack\" ] && [ -f src/bad.txt ]; then git rm -q --cached src/a.txt; fi\nif [ -f src/bad.txt ]; then echo \"test land_merge::bad ... FAILED\"; exit 1; fi\nif [ -f \"$out/flaky\" ] && [ ! -f \"$out/flaky.done\" ]; then touch \"$out/flaky.done\"; echo \"test land_merge::flaky ... FAILED\"; exit 1; fi\nexit 0\n"
    );
    fs::write(fx.root.join("suite.sh"), script).unwrap();
    fs::write(fx.root.join("other.sh"), format!("{find_out}echo \"${{PWD##*/.bf/}}\" >> \"$out/other.log\"\n")).unwrap();
    fx.commit_all("records");
    fx.human("yes, make this a test run with simulated approvals");
    fx.must(&["test-mode", "grant", "--quote", "yes, make this a test run with simulated approvals"]);
    fx.must(&["run", "start"]);
    fx.must(&["checks", "wave", "propose", "none"]);
    fx.human("No wave checks for this run.");
    fx.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
    if project.is_some() {
        fx.must(&["checks", "unit", "propose", "project"]);
    } else {
        let mut a = vec!["checks", "unit", "propose", "override"];
        a.extend_from_slice(list);
        fx.must(&a);
    }
    fx.human("Use the suite for every unit.");
    fx.must(&["checks", "unit", "set", "--quote", "Use the suite for every unit"]);
    fx.must(&["gate", "open", "plan"]);
    fx.must(&["gate", "approve", "plan", "--simulated"]);
}

fn out_dir(fx: &Fx) -> PathBuf {
    fx.root.parent().unwrap().join("out")
}

/// How many times the suite ran in the checkpoint's own working copy.
fn k_runs(fx: &Fx) -> usize {
    fs::read_to_string(out_dir(fx).join("suite.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.ends_with("/K-1"))
        .count()
}

/// The receipts of runs in the checkpoint's copy, each a command that ran or counted.
fn k_receipts(fx: &Fx) -> Vec<bf::evidence::Receipt> {
    bf::evidence::load_receipts(&fx.project())
        .unwrap()
        .into_iter()
        .filter(|r| r.v["folder"].as_str() == Some(".bf/work/K-1"))
        .collect()
}

fn built(fx: &Fx, n: &str, file: &str, text: &str) -> PathBuf {
    let u = format!("U-{n}");
    let wc = PathBuf::from(fx.must(&["unit", "start", &u, "--json"]).json()["working_copy"].as_str().unwrap());
    finish(fx, n, &wc, file, text);
    wc
}

fn finish(fx: &Fx, n: &str, wc: &Path, file: &str, text: &str) {
    let u = format!("U-{n}");
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join(file), text).unwrap();
    fx.must_in(wc, &["evidence", "run", &format!("X-{n}"), "--unit", &u, "--", "true"]);
    fx.must_in(wc, &["evidence", "run", "attached:suite", "--unit", &u, "--", "sh", "suite.sh"]);
    fx.must(&["report", &u, "--result", "built"]);
    fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
}

fn all_built(fx: &Fx) -> Vec<PathBuf> {
    files()
        .iter()
        .map(|(n, f)| built(fx, n, &format!("src/{f}.txt"), &format!("unit {n}\n")))
        .collect()
}

// The states the tests start from, each built once by these calls and copied per test (D-140). The test
// `land_merge_one_unit_not_behind_lands_on_its_own_receipt_with_no_new_run` builds its state through the
// full calls, so the path of building it stays covered.

fn planned(fx: &Fx) {
    plan(fx);
}

fn named_planned(fx: &Fx) {
    plan_with(fx, true);
}

fn one_built(fx: &Fx) {
    plan(fx);
    built(fx, "1", "src/a.txt", "one\n");
}

fn two_built(fx: &Fx) {
    one_built(fx);
    built(fx, "2", "src/b.txt", "two\n");
}

fn three_built(fx: &Fx) {
    plan(fx);
    all_built(fx);
}

fn bad_middle(fx: &Fx) {
    one_built(fx);
    built(fx, "2", "src/bad.txt", "bad\n");
    built(fx, "3", "src/c.txt", "three\n");
}

/// A copy of one of the states above, with the sandbox on as in `Fx::sandboxed()`.
fn copy(name: &str, build: fn(&Fx)) -> Fx {
    Fx::state_copy_with(&format!("land_merge::{name}"), Switch::All, build)
}

fn state(fx: &Fx, u: &str) -> bf::events::UnitSt {
    bf::events::replay(&fx.project()).unwrap().units[u].clone()
}

fn log(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.project().log_path())
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

fn lands(fx: &Fx) -> Vec<Value> {
    log(fx).into_iter().filter(|e| e["kind"] == "unit.land").collect()
}

fn mark(fx: &Fx) -> PathBuf {
    fx.root.join(".bf/land-K-1.mark")
}

#[test]
fn land_merge_three_clean_units_land_in_one_turn_with_one_run() {
    let fx = copy("three_built", three_built);
    let wcs: Vec<PathBuf> = ["U-1", "U-2", "U-3"].iter().map(|u| fx.work(u)).collect();
    // The landing removes the working copies, so their content is read first.
    let before: Vec<String> = wcs.iter().map(|w| fx.project().content_hash(w).unwrap()).collect();
    let o = fx.must(&["unit", "land", "U-2", "--json"]).json();
    assert_eq!(o["landed"], true, "{o}");
    assert!(o["line"].as_str().unwrap().contains("landed in the same turn: U-1, U-3"), "{o}");
    for n in ["1", "2", "3"] {
        assert_eq!(state(&fx, &format!("U-{n}")).state, "landed");
    }
    assert_eq!(k_runs(&fx), 1);
    let ks: Vec<_> = k_receipts(&fx).into_iter().filter(|r| r.check() == "attached:suite").collect();
    assert_eq!(ks.len(), 1, "one receipt of the suite on the merged content");
    assert!(ks.iter().all(|r| !r.v["command"].as_str().unwrap().contains("extended")));
    let ls = lands(&fx);
    let units: Vec<&str> = ls.iter().map(|e| e["unit"].as_str().unwrap()).collect();
    assert_eq!(units, ["U-1", "U-2", "U-3"]);
    let p = fx.project();
    let receipts = bf::evidence::load_receipts(&p).unwrap();
    for (e, was) in ls.iter().zip(&before) {
        assert_eq!(e["hash"].as_str().unwrap(), was);
        assert_eq!(e["merged_hash"], ls[0]["merged_hash"]);
        assert_ne!(e["merged_hash"], e["hash"]);
        // The ledger finds the unit's receipts by the unit's own content.
        let u = e["unit"].as_str().unwrap();
        let n = &u[2..];
        for c in [format!("X-{n}"), "attached:suite".to_string()] {
            assert!(bf::evidence::satisfied_unit_at(&receipts, &c, e["hash"].as_str().unwrap()).is_some(), "{u} {c}");
        }
    }
    assert!(!mark(&fx).exists());
}

#[test]
fn land_merge_one_unit_not_behind_lands_on_its_own_receipt_with_no_new_run() {
    let fx = Fx::sandboxed();
    plan(&fx);
    built(&fx, "1", "src/a.txt", "one\n");
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-1").state, "landed");
    assert_eq!(k_runs(&fx), 0);
    assert!(k_receipts(&fx).is_empty());
}

#[test]
fn land_merge_a_receipt_that_names_the_compile_cache_is_not_used() {
    let fx = copy("one_built", one_built);
    let p = fx.project();
    let key = bf::evidence::read_key(&p).unwrap();
    for r in bf::evidence::load_receipts(&p).unwrap().into_iter().filter(|r| r.check() == "attached:suite") {
        let path = p.receipts_dir().join(format!("{}.toml", r.id()));
        let mut v: Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        v["sandbox"] = json!("sandbox-exec+compile-cache");
        v.as_object_mut().unwrap().remove("seal");
        v["seal"] = json!(bf::evidence::seal(&key, &v));
        fs::write(&path, toml::to_string_pretty(&v).unwrap()).unwrap();
    }
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-1").state, "landed");
    assert_eq!(k_runs(&fx), 1, "the suite ran in the checkpoint's copy");
}

#[test]
fn land_merge_a_land_during_a_turn_waits_and_the_turn_keeps_sync_and_fix_out() {
    let fx = copy("two_built", two_built);
    let mut child = Command::new(bin())
        .args(["unit", "land", "U-1"])
        .current_dir(&fx.root)
        .env("BF_STATE_HOME", &fx.state_base)
        .env("BF_HOLD_AT", "land-turn")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut held = String::new();
    BufReader::new(child.stderr.as_mut().unwrap()).read_line(&mut held).unwrap();
    assert!(held.contains("K-1"), "{held}");
    // A second land of the checkpoint waits and counts as no attempt.
    let o = fx.must(&["unit", "land", "U-2", "--json"]).json();
    assert_eq!(o["waiting"], true, "{o}");
    assert!(o["line"].as_str().unwrap().contains("waits"), "{o}");
    let u2 = state(&fx, "U-2");
    assert_eq!((u2.state.as_str(), u2.attempts), ("reviewed", 0));
    // The turn keeps a sync, a fix and a close out, with the key's text naming it.
    for args in [
        vec!["checkpoint", "sync", "K-1", "--local", "--quote", "x"],
        vec!["checkpoint", "fix", "K-1", "--abort"],
        vec!["checkpoint", "close", "K-1"],
    ] {
        let o = fx.bf(&args);
        assert_eq!(o.code, 2, "{}", o.all());
        assert!(o.stderr.contains("landing turn of K-1 is open"), "{}", o.stderr);
    }
    // A third unit reviewed meanwhile lands afterwards.
    built(&fx, "3", "src/c.txt", "three\n");
    // While the turn runs, `bf next` offers no land for U-3: it lists the unit as waiting.
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["directives"].as_array().unwrap().iter().all(|d| d["action"] != "land"),
        "{n}"
    );
    assert!(n["waiting"].to_string().contains("U-3"), "{n}");
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(state(&fx, "U-1").state, "landed");
    assert_eq!(state(&fx, "U-2").state, "landed");
    assert_eq!(state(&fx, "U-3").state, "reviewed");
    fx.must(&["unit", "land", "U-3"]);
    assert_eq!(state(&fx, "U-3").state, "landed");
    // No turn is open now.
    assert!(!fx.bf(&["checkpoint", "fix", "K-1", "--abort"]).all().contains("landing turn"));
}

#[test]
fn land_merge_the_requested_unit_whose_second_merge_fails_is_refused_not_reported_landed() {
    // On a failing suite in the checkpoint's copy the suite leaves U-1's file behind untracked: the three
    // units fail together, the branch goes back, and U-1's second merge stops on that file with no conflict.
    // The suite reads the marker only in the checkpoint's copy, so it is set after the units are built.
    let fx = copy("bad_middle", bad_middle);
    fs::write(out_dir(&fx).join("untrack"), "").unwrap();
    let o = fx.bf(&["unit", "land", "U-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(!o.all().contains("landed on its checkpoint branch"), "{}", o.all());
    assert!(o.all().contains("could not merge"), "{}", o.all());
    assert_eq!(state(&fx, "U-1").state, "reviewed");
}

#[test]
fn land_merge_a_sync_holds_the_turn_lock_so_a_land_waits_behind_it() {
    let fx = copy("one_built", one_built);
    let mut child = Command::new(bin())
        .args(["checkpoint", "sync", "K-1", "--local", "--quote", "x"])
        .current_dir(&fx.root)
        .env("BF_STATE_HOME", &fx.state_base)
        .env("BF_HOLD_AT", "checkpoint-command")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut held = String::new();
    BufReader::new(child.stderr.as_mut().unwrap()).read_line(&mut held).unwrap();
    assert!(held.contains("K-1"), "{held}");
    // The sync holds the turn lock before it works: a land cannot open a turn in that gap.
    let o = fx.must(&["unit", "land", "U-1", "--json"]).json();
    assert_eq!(o["waiting"], true, "{o}");
    assert_eq!(state(&fx, "U-1").state, "reviewed");
    // A second sync is refused for the open turn.
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--local", "--quote", "x"]);
    assert_eq!(o.code, 2, "{}", o.all());
    drop(child.stdin.take());
    let _ = child.wait_with_output().unwrap();
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-1").state, "landed");
}

#[test]
fn land_merge_a_unit_with_an_open_finding_is_left_out() {
    let fx = copy("three_built", three_built);
    let o = fx.put("finding", "id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-2\"\nseverity=\"high\"\naction=\"Change it\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.must(&["unit", "land", "U-1", "--json"]).json();
    assert!(o["line"].as_str().unwrap().contains("U-2 stays reviewed"), "{o}");
    assert_eq!(state(&fx, "U-1").state, "landed");
    assert_eq!(state(&fx, "U-3").state, "landed");
    assert_eq!(state(&fx, "U-2").state, "reviewed");
}

#[test]
fn land_merge_the_unit_whose_merged_content_fails_twice_goes_to_its_builder() {
    let fx = copy("bad_middle", bad_middle);
    let o = fx.must(&["unit", "land", "U-1", "--json"]).json();
    assert_eq!(o["landed"], true, "{o}");
    assert_eq!(state(&fx, "U-1").state, "landed");
    assert_eq!(state(&fx, "U-3").state, "landed");
    let u2 = state(&fx, "U-2");
    assert_eq!((u2.state.as_str(), u2.attempts), ("fixing", 0));
    // The combined run, none for U-1 (its own receipt is on the same content), two for U-2 and one for U-3.
    assert_eq!(k_runs(&fx), 4);
    let sent = log(&fx).into_iter().rev().find(|e| e["kind"] == "unit.report" && e["unit"] == "U-2").unwrap();
    assert_eq!((sent["role"].as_str(), sent["from"].as_str(), sent["to"].as_str()), (Some("engine"), Some("reviewed"), Some("fixing")));
    assert!(sent["note"].as_str().unwrap().contains("land_merge::bad"), "{sent}");
    let brief = fs::read_to_string(fx.project().briefs_dir().join("U-2-builder.md")).unwrap();
    assert!(brief.contains("land_merge::bad"), "{brief}");
    assert!(brief.contains("Fix the failing tests") && !brief.contains("Resolve the conflict"), "{brief}");
    // U-2's merge is out of the branch.
    assert!(!fx.work("K-1").join("src/bad.txt").exists());
    assert!(fx.work("K-1").join("src/c.txt").exists());
    assert!(!mark(&fx).exists());
}

#[test]
fn land_merge_a_suite_that_fails_once_lands_the_unit_with_both_receipts_kept() {
    let fx = copy("two_built", two_built);
    fs::write(out_dir(&fx).join("flaky"), "").unwrap();
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-1").state, "landed");
    assert_eq!(state(&fx, "U-2").state, "landed");
    // The combined run failed once; the units were then landed one at a time.
    let exits: Vec<i64> = k_receipts(&fx).iter().map(|r| r.v["exit"].as_i64().unwrap()).collect();
    assert!(exits.contains(&0) && exits.iter().any(|e| *e != 0), "{exits:?}");
}

#[test]
fn land_merge_one_unit_that_fails_once_is_run_again_before_it_is_blamed() {
    let fx = copy("planned", planned);
    // U-2 is cut before U-1 lands, so it is behind and its merged content is new.
    let w2 = PathBuf::from(fx.must(&["unit", "start", "U-2", "--json"]).json()["working_copy"].as_str().unwrap());
    built(&fx, "1", "src/a.txt", "one\n");
    fx.must(&["unit", "land", "U-1"]);
    finish(&fx, "2", &w2, "src/b.txt", "two\n");
    fs::write(out_dir(&fx).join("flaky"), "").unwrap();
    fx.must(&["unit", "land", "U-2"]);
    assert_eq!(state(&fx, "U-2").state, "landed");
    assert_eq!(k_runs(&fx), 2);
    let ks = k_receipts(&fx);
    assert_eq!(ks.len(), 2);
    assert_eq!(ks[0].v["exit"].as_i64(), Some(1));
    assert_eq!(ks[1].v["exit"].as_i64(), Some(0));
}

fn commit_in(wt: &Path, file: &str) {
    fs::write(wt.join(file), "by hand\n").unwrap();
    git(wt, &["add", "-A"]);
    git(wt, &["commit", "-q", "-m", "by hand"]);
}

fn crashed_turn(fx: &Fx) {
    let o = fx.bf_env(&fx.root, &["unit", "land", "U-1"], "", &[("BF_CRASH_AT", "land-after-merge")]);
    assert_eq!(o.code, 137, "{}", o.all());
    assert!(mark(fx).exists());
    assert!(lands(fx).len() <= 1);
}

#[test]
fn land_merge_after_a_crash_the_next_landing_puts_back_only_the_turns_own_merges() {
    let fx = copy("one_built", one_built);
    fx.must(&["unit", "land", "U-1"]);
    // A commit on the checkpoint branch with no mark is the human's.
    commit_in(&fx.work("K-1"), "by_hand.txt");
    built(&fx, "2", "src/b.txt", "two\n");
    built(&fx, "3", "src/c.txt", "three\n");
    let o = fx.bf_env(&fx.root, &["unit", "land", "U-2"], "", &[("BF_CRASH_AT", "land-detached")]);
    assert_eq!(o.code, 137, "{}", o.all());
    assert!(mark(&fx).exists());
    let k = fx.work("K-1");
    assert!(k.join("src/b.txt").exists() && k.join("src/c.txt").exists());
    fx.must(&["unit", "land", "U-2"]);
    assert!(!mark(&fx).exists());
    assert_eq!(state(&fx, "U-2").state, "landed");
    assert_eq!(state(&fx, "U-3").state, "landed");
    let subjects = git(&k, &["log", "--first-parent", "--format=%s"]);
    assert_eq!(subjects.matches("Land U-2").count(), 1, "{subjects}");
    assert_eq!(subjects.matches("Land U-3").count(), 1, "{subjects}");
    assert!(subjects.contains("by hand"), "{subjects}");
}

#[test]
fn land_merge_a_commit_after_a_fix_with_no_mark_is_kept() {
    let fx = copy("one_built", one_built);
    fx.must(&["unit", "land", "U-1"]);
    commit_in(&fx.work("K-1"), "by_hand.txt");
    built(&fx, "2", "src/b.txt", "two\n");
    fx.must(&["unit", "land", "U-2"]);
    let k = fx.work("K-1");
    assert!(k.join("by_hand.txt").exists());
    assert!(git(&k, &["log", "--first-parent", "--format=%s"]).contains("by hand"));
}

#[test]
fn land_merge_a_mark_with_a_foreign_commit_above_it_refuses_the_landing() {
    let fx = copy("two_built", two_built);
    crashed_turn(&fx);
    let k = fx.work("K-1");
    commit_in(&k, "by_hand.txt");
    let head = git(&k, &["rev-parse", "HEAD"]);
    let o = fx.bf(&["unit", "land", "U-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("by hand"), "{}", o.stderr);
    assert_eq!(git(&k, &["rev-parse", "HEAD"]), head);
    assert!(mark(&fx).exists());
    assert_eq!(state(&fx, "U-1").state, "reviewed");
}

fn hold_land(fx: &Fx, unit: &str) -> (std::process::Child, String) {
    let mut child = Command::new(bin())
        .args(["unit", "land", unit])
        .current_dir(&fx.root)
        .env("BF_STATE_HOME", &fx.state_base)
        .env("BF_HOLD_AT", "land-turn")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut held = String::new();
    BufReader::new(child.stderr.as_mut().unwrap()).read_line(&mut held).unwrap();
    (child, held)
}

#[test]
fn land_merge_a_unit_started_during_a_turn_does_not_hold_a_merge_that_is_put_back() {
    let fx = copy("planned", planned);
    // U-1 is cut before U-2 lands, so its merged content is new and the suite runs on it.
    let w1 = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    built(&fx, "2", "src/b.txt", "two\n");
    fx.must(&["unit", "land", "U-2"]);
    finish(&fx, "1", &w1, "src/bad.txt", "bad\n");
    let (mut child, held) = hold_land(&fx, "U-1");
    assert!(held.contains("K-1"), "{held}");
    // The turn's merge is not on the checkpoint branch while the suite runs.
    let w3 = PathBuf::from(fx.must(&["unit", "start", "U-3", "--json"]).json()["working_copy"].as_str().unwrap());
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(state(&fx, "U-1").state, "fixing");
    assert!(!w3.join("src/bad.txt").exists());
    assert!(!git(&w3, &["log", "--format=%s"]).contains("Land U-1"));
    let k = fx.work("K-1");
    assert!(!git(&k, &["log", "--format=%s"]).contains("Land U-1"));
    assert!(git(&k, &["symbolic-ref", "--short", "HEAD"]).contains("K-1"));
}

#[test]
fn land_merge_a_unit_that_changed_while_the_suite_ran_is_not_landed() {
    let fx = copy("one_built", one_built);
    let (mut child, _) = hold_land(&fx, "U-1");
    let o = fx.put("finding", "id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\nseverity=\"high\"\naction=\"Change it\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("changed while the suite ran"));
    assert_eq!(state(&fx, "U-1").state, "reviewed");
    assert!(lands(&fx).is_empty());
    let k = fx.work("K-1");
    assert!(!git(&k, &["log", "--format=%s"]).contains("Land U-1"));
    assert!(git(&k, &["symbolic-ref", "--short", "HEAD"]).contains("K-1"));
    assert!(!mark(&fx).exists());
}

#[test]
fn land_merge_the_landing_runs_the_full_suite_when_every_unit_names_its_tests() {
    let fx = copy("named_planned", named_planned);
    for (n, f) in files() {
        let u = format!("U-{n}");
        let wc = PathBuf::from(fx.must(&["unit", "start", &u, "--json"]).json()["working_copy"].as_str().unwrap());
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join(format!("src/{f}.txt")), "x\n").unwrap();
        fx.must_in(&wc, &["evidence", "run", &format!("X-{n}"), "--unit", &u, "--", "true"]);
        // The unit's own suite is its named tests.
        fx.must_in(&wc, &["evidence", "run", "attached:suite", "--unit", &u, "--", "true"]);
        fx.must(&["report", &u, "--result", "built"]);
        fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
    }
    fx.must(&["unit", "land", "U-1"]);
    for n in ["1", "2", "3"] {
        assert_eq!(state(&fx, &format!("U-{n}")).state, "landed");
    }
    assert_eq!(k_runs(&fx), 1, "the run's suite.sh ran once in the checkpoint's copy");
}

#[test]
fn land_merge_a_turn_does_not_open_while_a_sync_or_a_fix_of_the_checkpoint_is_open() {
    let fx = copy("planned", planned);
    let p = fx.project();
    let mut st = bf::events::replay(&p).unwrap();
    assert_eq!(bf::run::open_reason(&p, &st, "K-1"), None);
    st.sync_open = Some(bf::events::SyncOpen {
        checkpoint: "K-1".into(),
        repos: vec![],
        phase: "resolve".into(),
        rounds: 0,
        reason: String::new(),
        findings: vec![],
        open_seq: 1,
        park_seq: 0,
        floor: 0,
        failed: vec![],
        unblocks: 0,
        block_seq: 0,
        wait_seq: 0,
    });
    assert_eq!(bf::run::open_reason(&p, &st, "K-1"), Some("sync"));
    assert_eq!(bf::run::open_reason(&p, &st, "K-2"), None);
    st.sync_open = None;
    st.ck_fix = Some(bf::events::CkFixOpen { checkpoint: "K-1".into(), ..Default::default() });
    assert_eq!(bf::run::open_reason(&p, &st, "K-1"), Some("fix"));
    assert_eq!(bf::run::open_reason(&p, &st, "K-2"), None);
}

#[test]
fn land_merge_a_receipt_that_names_the_cache_is_not_reusable_even_when_the_key_matches() {
    use bf::evidence::{Receipt, RunKey};
    let key = |sandbox: &str| RunKey {
        command: "sh suite.sh".into(),
        content: "c1".into(),
        repos: ".".into(),
        folder: ".bf/work/K-1".into(),
        sandbox: sandbox.into(),
    };
    let receipt = |sandbox: &str| Receipt {
        v: json!({"type": "run", "check": "attached:suite", "command": "sh suite.sh", "content_hash": "c1",
            "repos": ".", "folder": ".bf/work/K-1", "sandbox": sandbox, "exit": 0}),
        seal_ok: true,
        stale: false,
        how: "command".into(),
    };
    assert!(bf::run::reusable_run(&[receipt("sandbox-exec")], &[key("sandbox-exec")]));
    // find_run finds this one; only the landing's own rule turns it away.
    assert!(bf::evidence::find_run(&[receipt("sandbox-exec+compile-cache")], &key("sandbox-exec+compile-cache")).is_some());
    assert!(!bf::run::reusable_run(&[receipt("sandbox-exec+compile-cache")], &[key("sandbox-exec+compile-cache")]));
}

#[test]
fn land_merge_the_engines_clean_merge_of_the_judged_content_is_not_a_change() {
    let fx = copy("planned", planned);
    let w2 = PathBuf::from(fx.must(&["unit", "start", "U-2", "--json"]).json()["working_copy"].as_str().unwrap());
    built(&fx, "1", "src/a.txt", "one\n");
    fx.must(&["unit", "land", "U-1"]);
    fs::create_dir_all(w2.join("src")).unwrap();
    fs::write(w2.join("src/b.txt"), "two\n").unwrap();
    git(&w2, &["add", "-A"]);
    git(&w2, &["commit", "-q", "-m", "work"]);
    let judged = git(&w2, &["rev-parse", "HEAD"]);
    fx.must(&["unit", "sync", "U-2"]);
    assert_ne!(git(&w2, &["rev-parse", "HEAD"]), judged);
    assert!(bf::run::clean_engine_merge(&fx.project(), "U-2", &w2, &judged));
    // A commit of the builder in the chain is a change.
    commit_in(&w2, "mine.txt");
    assert!(!bf::run::clean_engine_merge(&fx.project(), "U-2", &w2, &judged));
}

#[test]
fn land_merge_a_failing_suite_charges_no_unit_outside_the_turn_when_every_unit_names_its_tests() {
    let fx = copy("named_planned", named_planned);
    // U-4 names no tests and is at work; it has nothing to do with the turn.
    fx.must(&["unit", "start", "U-4"]);
    let w1 = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    let named = |n: &str, wc: &Path, file: &str| {
        let u = format!("U-{n}");
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join(file), "x\n").unwrap();
        fx.must_in(wc, &["evidence", "run", &format!("X-{n}"), "--unit", &u, "--", "true"]);
        fx.must_in(wc, &["evidence", "run", "attached:suite", "--unit", &u, "--", "true"]);
        fx.must(&["report", &u, "--result", "built"]);
        fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
    };
    let w2 = PathBuf::from(fx.must(&["unit", "start", "U-2", "--json"]).json()["working_copy"].as_str().unwrap());
    named("2", &w2, "src/b.txt");
    fx.must(&["unit", "land", "U-2"]);
    named("1", &w1, "src/bad.txt");
    let o = fx.bf(&["unit", "land", "U-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(state(&fx, "U-1").state, "fixing");
    let u4 = state(&fx, "U-4");
    assert_eq!((u4.state.as_str(), u4.attempts), ("running", 0));
    for e in log(&fx).iter().filter(|e| e["kind"] == "evidence.run" && e["exit"].as_i64() != Some(0)) {
        assert_eq!(e["unit"], "U-1", "{e}");
    }
}

fn other_runs(fx: &Fx) -> usize {
    fs::read_to_string(out_dir(fx).join("other.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.ends_with("/K-1"))
        .count()
}

#[test]
fn land_merge_each_suite_entry_of_the_list_runs_its_own_command_at_landing() {
    let fx = Fx::sandboxed();
    plan_full(&fx, true, &["suite=sh suite.sh", "full-suite=sh other.sh"], None);
    for (n, f) in files() {
        let u = format!("U-{n}");
        let wc = PathBuf::from(fx.must(&["unit", "start", &u, "--json"]).json()["working_copy"].as_str().unwrap());
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join(format!("src/{f}.txt")), "x\n").unwrap();
        fx.must_in(&wc, &["evidence", "run", &format!("X-{n}"), "--unit", &u, "--", "true"]);
        for id in ["attached:suite", "attached:full-suite"] {
            fx.must_in(&wc, &["evidence", "run", id, "--unit", &u, "--", "true"]);
        }
        fx.must(&["report", &u, "--result", "built"]);
        fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
    }
    fx.must(&["unit", "land", "U-1"]);
    assert_eq!(state(&fx, "U-3").state, "landed");
    assert_eq!(k_runs(&fx), 1, "the first entry's command ran once");
    assert_eq!(other_runs(&fx), 1, "the second entry's own command ran once");
}

#[test]
fn land_merge_a_listed_suite_command_with_an_operator_is_refused_for_a_unit_that_names_tests() {
    let fx = Fx::sandboxed();
    // The project's own list is not screened for operators: its second suite entry holds one.
    plan_full(
        &fx,
        true,
        &[],
        Some("{ name = \"suite\", command = \"sh suite.sh\" }, { name = \"suite2\", command = \"sh other.sh && true\" }"),
    );
    let named = |n: &str, wc: &Path, file: &str| {
        let u = format!("U-{n}");
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join(file), "x\n").unwrap();
        fx.must_in(wc, &["evidence", "run", &format!("X-{n}"), "--unit", &u, "--", "true"]);
        for id in ["attached:suite", "attached:suite2"] {
            fx.must_in(wc, &["evidence", "run", id, "--unit", &u, "--", "true"]);
        }
        fx.must(&["report", &u, "--result", "built"]);
        fx.must(&["report", &u, "--result", "built", "--role", "reviewer"]);
    };
    // The unit's receipts are for its named tests, so the landing runs each listed suite entry itself.
    let w1 = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    named("1", &w1, "src/a.txt");
    let o = fx.bf(&["unit", "land", "U-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("which a check cannot use"), "{}", o.stderr);
    assert_eq!(other_runs(&fx), 0, "the entry with the operator never ran");
    assert_eq!(state(&fx, "U-1").state, "reviewed");
    assert!(lands(&fx).is_empty());
}
