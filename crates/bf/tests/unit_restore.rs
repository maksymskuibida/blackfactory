//! U-103 (R-109, D-84): a builder puts a file it changed outside its scope back to the base with
//! `bf unit restore`, recorded in the log; one refused path writes none; the guard lets only that
//! unit subcommand through.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// One unit that lists src/a.rs, on a project with some other files at the base.
fn setup() -> (Fx, PathBuf, String) {
    let fx = Fx::new();
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ntext=\"Build a.\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"All\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.set_policy("tokens = false", "tokens = true");
    write(&fx.root, "src/a.rs", "fn a() {}\n");
    write(&fx.root, "src/other.rs", "fn o() {}\n");
    write(&fx.root, "src/second.rs", "fn s() {}\n");
    write(&fx.root, "docs/n.md", "old\n");
    fx.commit_all("files");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    let token = fx.take_token("U-1", "builder");
    (fx, wc, token)
}

fn log(fx: &Fx) -> String {
    fs::read_to_string(fx.project().log_path()).unwrap()
}

fn unit(fx: &Fx) -> Value {
    let st = fx.must(&["status", "--json"]).json();
    let u = st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == "U-1")
        .unwrap()
        .clone();
    json!([u["state"], u["rounds"], u["attempts"]])
}

fn run(fx: &Fx, wc: &Path, token: &str, paths: &[&str]) -> Out {
    let mut a = vec!["unit", "restore", "U-1"];
    a.extend_from_slice(paths);
    a.extend_from_slice(&["--token", token]);
    fx.bf_in(wc, &a)
}

fn refused(fx: &Fx, wc: &Path, token: &str, paths: &[&str], why: &str) {
    let n = log(fx).lines().count();
    let o = run(fx, wc, token, paths);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
    assert!(o.stderr.contains(why), "no `{why}` in {}", o.stderr);
    assert_eq!(log(fx).lines().count(), n, "a refusal records nothing");
}

#[test]
fn an_edited_file_gets_its_base_content_and_the_log_the_event() {
    let (fx, wc, t) = setup();
    write(&wc, "src/other.rs", "fn   o( ) { }\n");
    let before = unit(&fx);
    let o = fx.must_in(
        &wc,
        &[
            "unit",
            "restore",
            "U-1",
            "./src/other.rs",
            "--token",
            &t,
            "--json",
        ],
    );
    assert!(o.json()["line"].as_str().unwrap().contains("src/other.rs"));
    assert_eq!(
        fs::read_to_string(wc.join("src/other.rs")).unwrap(),
        "fn o() {}\n"
    );
    let last = log(&fx).lines().last().unwrap().to_string();
    assert!(
        last.contains("unit.scope.restore")
            && last.contains("src/other.rs")
            && last.contains("blob"),
        "{last}"
    );
    assert_eq!(unit(&fx), before);
    // The token is not spent: the builder goes on.
    write(&wc, "src/other.rs", "x\n");
    fx.must_in(
        &wc,
        &["unit", "restore", "U-1", "src/other.rs", "--token", &t],
    );
}

#[test]
fn a_new_file_outside_the_list_is_removed() {
    let (fx, wc, t) = setup();
    write(&wc, "docs/new.md", "new\n");
    fx.must_in(
        &wc,
        &["unit", "restore", "U-1", "docs/new.md", "--token", &t],
    );
    assert!(!wc.join("docs/new.md").exists());
    assert!(log(&fx).lines().last().unwrap().contains("absent"));
}

#[test]
fn a_file_inside_the_list_is_refused_and_left() {
    let (fx, wc, t) = setup();
    write(&wc, "src/a.rs", "mine\n");
    refused(&fx, &wc, &t, &["src/a.rs"], "inside the unit's files");
    assert_eq!(fs::read_to_string(wc.join("src/a.rs")).unwrap(), "mine\n");
}

#[test]
fn protected_places_and_links_are_refused() {
    let (fx, wc, t) = setup();
    refused(&fx, &wc, &t, &["factory/x.toml"], "factory/x.toml");
    refused(&fx, &wc, &t, &["sub/.git/config"], "sub/.git/config");
    symlink(wc.join("src/other.rs"), wc.join("lnk")).unwrap();
    refused(&fx, &wc, &t, &["lnk"], "symbolic link");
}

#[test]
fn one_refused_path_among_good_ones_writes_none() {
    let (fx, wc, t) = setup();
    write(&wc, "src/other.rs", "changed\n");
    write(&wc, "src/a.rs", "mine\n");
    refused(
        &fx,
        &wc,
        &t,
        &["src/other.rs", "src/a.rs"],
        "no file was written",
    );
    assert_eq!(
        fs::read_to_string(wc.join("src/other.rs")).unwrap(),
        "changed\n"
    );
}

#[test]
fn a_wrong_token_is_refused() {
    let (fx, wc, _t) = setup();
    write(&wc, "src/other.rs", "changed\n");
    let n = log(&fx).lines().count();
    let o = run(&fx, &wc, "wrong", &["src/other.rs"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
    assert_eq!(
        fs::read_to_string(wc.join("src/other.rs")).unwrap(),
        "changed\n"
    );
    assert_eq!(log(&fx).lines().count(), n);
}

fn judge(fx: &Fx, cwd: &Path, cmd: &str) -> String {
    let payload =
        json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    match o.stdout.trim() {
        "" | "{}" => "pass".into(),
        t => serde_json::from_str::<Value>(t).unwrap()["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap_or("?")
            .to_string(),
    }
}

#[test]
fn the_guard_lets_restore_through_and_nothing_else_of_unit() {
    let (fx, wc, _t) = setup();
    assert_eq!(
        judge(&fx, &wc, "bf unit restore U-1 src/other.rs --token abc"),
        "pass"
    );
    assert_eq!(judge(&fx, &wc, "bf unit release U-1 --quote x"), "deny");
    assert_eq!(judge(&fx, &wc, "bf unit land U-1"), "deny");
    let o = fx.bf_in(&wc, &["unit", "land", "U-1"]);
    assert_eq!(o.code, 2, "{}", o.all());
}
