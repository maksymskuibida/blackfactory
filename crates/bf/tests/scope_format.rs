//! U-104 (R-109, D-83, TH-55): a file outside a unit's list whose change rustfmt proves is formatting
//! only is accepted with no human; anything else is refused; landing checks the content again.

mod common;
use common::*;
use std::fs;
use std::path::{Path, PathBuf};

const PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
const MESSY: &str = "fn f( a:u32 )->u32{ a+1 }\n";
const NEAT: &str = "fn f(a: u32) -> u32 {\n    a + 1\n}\n";
const WIDE: &str =
    "fn wide(first_argument: u32, second_argument: u32) -> u32 {\n    first_argument\n}\n";
const NARROW: &str = "fn wide(\n    first_argument: u32,\n    second_argument: u32,\n) -> u32 {\n    first_argument\n}\n";

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

fn rustfmt_dir() -> String {
    let path = std::env::var("PATH").unwrap();
    let mut dirs = path
        .split(':')
        .filter(|d| Path::new(d).join("rustfmt").exists());
    dirs.next()
        .expect("these tests need rustfmt on PATH")
        .to_string()
}

/// Two units of one checkpoint that may run beside each other; U-3 holds src/h.rs. The policy's
/// check PATH is `path`, or has rustfmt's folder in front.
fn setup(path: Option<&str>) -> (Fx, PathBuf) {
    setup_with(path, &[])
}

/// As `setup`, with more files at the base (a later entry replaces an earlier one of the same path).
fn setup_with(path: Option<&str>, extra: &[(&str, &str)]) -> (Fx, PathBuf) {
    let fx = Fx::new();
    fx.set_policy(
        PATH,
        &path.map_or(format!("{}:{PATH}", rustfmt_dir()), String::from),
    );
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n"),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Parts\"\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/a.rs\",\"src/own.rs\"]\ncheckpoint=\"K-1\"\nestimate=2\n"),
        ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Part C\"\ndelivers=[\"R-1\"]\nfiles=[\"beside/y.txt\",\"src/h.rs\"]\ncheckpoint=\"K-1\"\nestimate=1\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"C\"\nhow=\"command\"\ncommand=\"test -f beside/y.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"All\"\nafter=[\"U-1\",\"U-3\"]\nproves=[\"R-1\",\"C-1\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        assert_eq!(fx.put(k, t).code, 0);
    }
    let cargo = "[package]\nname = \"toy\"\nedition = \"2021\"\n";
    let files = [("Cargo.toml", cargo), ("rustfmt.toml", "max_width = 100\n")];
    let more = [("src/f.rs", MESSY), ("src/g.rs", MESSY), ("src/w.rs", WIDE)];
    let rest = ["src/a.rs", "src/own.rs", "src/h.rs", "beside/y.txt"].map(|f| (f, "fn a() {}\n"));
    for (f, text) in files
        .into_iter()
        .chain(more)
        .chain(rest)
        .chain(extra.iter().copied())
    {
        write(&fx.root, f, text);
    }
    write(&fx.root, "docs/n.md", "old\n");
    write(&fx.root, "src/gone.rs", MESSY);
    fx.commit_all("files");
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    let wc = fx.work("U-1");
    (fx, wc)
}

fn log_len(fx: &Fx) -> usize {
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    log.lines().count()
}

fn unit(fx: &Fx) -> serde_json::Value {
    let st = fx.must(&["status", "--json"]).json();
    let u = &st["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == "U-1")
        .unwrap()
        .clone();
    serde_json::json!([u["state"], u["rounds"], u["attempts"]])
}

/// The command is refused naming the files and the reason, and the log keeps its length.
fn refused(fx: &Fx, files: &[&str], reason: &str) {
    let before = log_len(fx);
    let mut a = vec!["unit", "format", "U-1"];
    a.extend_from_slice(files);
    let o = fx.refused(&a);
    for p in files.iter().chain(&[reason]) {
        assert!(o.stderr.contains(p), "no `{p}` in {}", o.stderr);
    }
    assert_eq!(log_len(fx), before, "a refusal records nothing");
}

#[test]
fn accepts_a_file_that_is_only_formatted() {
    let (fx, wc) = setup(None);
    write(&wc, "src/f.rs", NEAT);
    let before = unit(&fx);
    let o = fx.must(&["unit", "format", "U-1", "src/f.rs", "--json"]);
    assert!(o.json()["line"].as_str().unwrap().contains("src/f.rs"));
    assert_eq!(unit(&fx), before);
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    let last = log.lines().last().unwrap();
    assert!(
        last.contains("unit.scope.grant") && last.contains("format only"),
        "{last}"
    );
    assert!(["result_sha256", "config_blob", "2021"]
        .iter()
        .all(|k| last.contains(k)));
}

#[test]
fn refuses_more_than_formatting() {
    let (fx, wc) = setup(None);
    write(&wc, "src/f.rs", &NEAT.replace("a + 1", "a + 2"));
    refused(&fx, &["src/f.rs"], "more than formatting");
    write(&wc, "src/w.rs", NARROW);
    write(&wc, "rustfmt.toml", "max_width = 60\n");
    refused(&fx, &["src/w.rs"], "more than formatting");
}

#[test]
fn refuses_when_the_formatter_is_missing_or_inside_the_project() {
    let (fx, wc) = setup(Some("/nonexistent-folder"));
    write(&wc, "src/f.rs", NEAT);
    refused(&fx, &["src/f.rs"], "not on");
    let (fx, wc) = setup(None);
    write(&wc, "src/f.rs", NEAT);
    write(&wc, "bin/rustfmt", "#!/bin/sh\ncat\n");
    let bin = wc.join("bin");
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    fs::set_permissions(bin.join("rustfmt"), mode).unwrap();
    fx.set_policy(&rustfmt_dir(), &bin.to_string_lossy());
    refused(&fx, &["src/f.rs"], "inside the project folder");
}

#[test]
fn refuses_files_that_are_not_format_only_changes() {
    let (fx, wc) = setup(None);
    write(&wc, "src/new.rs", NEAT);
    fs::remove_file(wc.join("src/gone.rs")).unwrap();
    write(&wc, "docs/n.md", "new\n");
    write(&wc, "src/own.rs", "fn a( ) {}\n");
    write(&wc, "src/h.rs", "fn a( ) {}\n");
    refused(&fx, &["src/new.rs"], "new");
    refused(&fx, &["src/gone.rs"], "deleted");
    refused(&fx, &["docs/n.md"], "no formatter");
    refused(&fx, &["src/g.rs"], "unchanged");
    refused(&fx, &["src/own.rs"], "inside the unit's files");
    fx.must(&["unit", "start", "U-3", "--json"]);
    refused(&fx, &["src/h.rs"], "U-3 holds it");
}

#[test]
fn one_refused_path_among_good_ones_records_none() {
    let (fx, wc) = setup(None);
    write(&wc, "src/f.rs", NEAT);
    write(&wc, "docs/n.md", "new\n");
    let before = log_len(&fx);
    let o = fx.refused(&["unit", "format", "U-1", "src/f.rs", "docs/n.md"]);
    assert!(o.stderr.contains("docs/n.md: no formatter"), "{}", o.stderr);
    assert_eq!(log_len(&fx), before, "a refusal records nothing");
    assert!(!fx
        .must(&["status", "--json"])
        .stdout
        .contains("format only"));
}

/// Build the unit with one accepted file, edit it again when `edit` is given, and try to land.
fn landing(edit: Option<&str>) -> Out {
    let (fx, wc) = setup(None);
    write(&wc, "src/f.rs", NEAT);
    fx.must(&["unit", "format", "U-1", "src/f.rs"]);
    if let Some(e) = edit {
        write(&wc, "src/f.rs", e);
    }
    write(&wc, "src/a.rs", "fn a() { }\n");
    let ev = "evidence run X-1 --unit U-1 -- test -f src/a.rs";
    fx.must_in(&wc, &ev.split(' ').collect::<Vec<_>>());
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.bf(&["unit", "land", "U-1"])
}

#[test]
fn landing_does_not_check_the_accepted_content_again() {
    // The second look guarded the list, and a file outside the list may be changed in any way.
    let o = landing(Some(&NEAT.replace("a + 1", "a + 3")));
    assert_eq!(o.code, 0, "{}", o.all());
    let o = landing(None);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// `bf unit format` with a time limit, so a hang fails fast.
fn format_within(fx: &Fx, file: &str, secs: u64) -> Out {
    let mut c = std::process::Command::new(bin());
    c.env("BF_STATE_HOME", &fx.state_base)
        .args(["unit", "format", "U-1", file, "--json"])
        .current_dir(&fx.root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = c.spawn().unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > end {
            child.kill().ok();
            child.wait().ok();
            panic!("bf unit format did not finish in {secs} seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let o = child.wait_with_output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).to_string(),
    }
}

#[test]
fn takes_the_edition_of_the_workspace_for_a_member() {
    let extra = [
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/m\"]\n\n[workspace.package]\nedition = \"2018\"\n"),
        ("crates/m/Cargo.toml", "[package]\nname = \"m\"\nedition.workspace = true\n"),
        ("crates/m/src/lib.rs", MESSY),
    ];
    let (fx, wc) = setup_with(None, &extra);
    write(&wc, "crates/m/src/lib.rs", NEAT);
    let o = format_within(&fx, "crates/m/src/lib.rs", 60);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.json()["line"].as_str().unwrap().contains("lib.rs"));
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    assert!(log.lines().last().unwrap().contains("2018"));
}

#[test]
fn takes_the_edition_of_a_root_package_that_is_also_the_workspace() {
    let extra = [
        (
            "Cargo.toml",
            "[package]\nname = \"r\"\nedition.workspace = true\n\n[workspace]\n\n[workspace.package]\nedition = \"2018\"\n",
        ),
        ("src/lib.rs", MESSY),
    ];
    let (fx, wc) = setup_with(None, &extra);
    write(&wc, "src/lib.rs", NEAT);
    let o = format_within(&fx, "src/lib.rs", 60);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    assert!(log.lines().last().unwrap().contains("2018"));
}

/// A folder outside the project with an executable `rustfmt` that is a link to a program named
/// `rustup`, the way a rustup install lays out its proxies. The program does its work only when it is
/// started under the name `rustfmt`; under any other name it exits with 2 and says nothing.
fn proxy_dir(real_rustfmt: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let script = format!(
        "#!/bin/sh\ncase \"${{0##*/}}\" in\n  rustfmt) exec {real_rustfmt}/rustfmt \"$@\" ;;\n  *) exit 2 ;;\nesac\n"
    );
    write(d.path(), "rustup", &script);
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    fs::set_permissions(d.path().join("rustup"), mode).unwrap();
    std::os::unix::fs::symlink("rustup", d.path().join("rustfmt")).unwrap();
    d
}

#[test]
fn runs_a_formatter_that_is_a_link_under_the_name_it_was_found_by() {
    let real = rustfmt_dir();
    let proxy = proxy_dir(&real);
    let (fx, wc) = setup(Some(&format!("{}:{PATH}", proxy.path().display())));
    write(&wc, "src/f.rs", NEAT);
    let o = fx.must(&["unit", "format", "U-1", "src/f.rs", "--json"]);
    assert!(o.json()["line"].as_str().unwrap().contains("src/f.rs"));
}

#[test]
fn says_how_a_formatter_failed_when_it_wrote_no_error() {
    let d = tempfile::tempdir().unwrap();
    let script = "#!/bin/sh\nif [ \"$1\" = --version ]; then echo rustfmt 1.0; exit 0; fi\necho boom\nexit 3\n";
    write(d.path(), "rustfmt", script);
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    fs::set_permissions(d.path().join("rustfmt"), mode).unwrap();
    let (fx, wc) = setup(Some(&format!("{}:{PATH}", d.path().display())));
    write(&wc, "src/f.rs", NEAT);
    let o = fx.refused(&["unit", "format", "U-1", "src/f.rs"]);
    assert!(o.stderr.contains("exit code 3"), "{}", o.stderr);
    assert!(o.stderr.contains("boom"), "{}", o.stderr);
    assert!(!o.stderr.contains("no message"), "{}", o.stderr);
}
