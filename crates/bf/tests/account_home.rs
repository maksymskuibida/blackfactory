//! U-7 (C-3, C-24, D-51, TH-5, T-16): the account's home by the account database, the test seam, the
//! adapter data compiled into the binary, and the source test that no other code reads `HOME`.

mod common;
use common::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SEAM: bool = cfg!(feature = "test_seam");

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ---- the resolver ----------------------------------------------------------------------------

/// The home of the real user by the shell's own lookup of the account name (`getpwnam`), which
/// does not read `HOME` for `~name`.
fn shell_home_of_real_user() -> Option<String> {
    let o = Command::new("sh")
        .arg("-c")
        .arg("eval echo ~\"$(id -un)\"")
        .env("HOME", "/nonexistent-home-for-test")
        .output()
        .ok()?;
    let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!t.is_empty() && !t.starts_with('~')).then_some(t)
}

#[test]
fn the_database_home_is_absolute_and_matches_the_shell_lookup() {
    let h = bf::session::home::database_home().expect("the account has a home");
    assert!(h.is_absolute());
    if let Some(s) = shell_home_of_real_user() {
        assert_eq!(h, PathBuf::from(s));
    }
}

/// The answer of one command of the test build, read from its stats file: the count of calls to the
/// account database, the time of the call and the home the resolver gave (`None` for no home).
struct Stats {
    lookups: u64,
    lookup_ns: f64,
    home: Option<String>,
}

/// Run `bf` as its own process with the variables set on the command alone (never on this shared
/// test process) and read the stats the test build writes. `seam` is `BF_TEST_ACCOUNT_HOME`.
fn run_stats(
    dir: &Path,
    args: &[&str],
    seam: Option<&Path>,
    extra: &[(&str, &Path)],
) -> (std::process::Output, Stats) {
    let tmp = tempfile::tempdir().unwrap();
    let stats = tmp.path().join("stats.json");
    let mut c = Command::new(bin());
    c.args(args)
        .current_dir(dir)
        .env("BF_TEST_ACCOUNT_STATS", &stats)
        .env("HOME", tmp.path())
        .env_remove("BF_TEST_ACCOUNT_HOME")
        .env_remove("BF_STATE_HOME")
        .env_remove("XDG_STATE_HOME");
    if let Some(h) = seam {
        c.env("BF_TEST_ACCOUNT_HOME", h);
    }
    for (k, v) in extra {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    let text = fs::read_to_string(&stats).unwrap_or_else(|e| {
        panic!(
            "no stats for {args:?} ({e}): {}",
            String::from_utf8_lossy(&o.stderr)
        )
    });
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let st = Stats {
        lookups: v["lookups"].as_u64().unwrap(),
        lookup_ns: v["lookup_ns"].as_f64().unwrap(),
        home: v["home"].as_str().map(|x| x.to_string()),
    };
    (o, st)
}

#[test]
fn the_test_build_has_the_seam_on_so_the_seam_tests_run() {
    let o = Command::new(bin())
        .args(["--version", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["test_seam"], serde_json::json!(true));
}

#[test]
fn account_home_never_follows_home_and_the_seam_variable_replaces_only_the_answer() {
    let cwd = tempfile::tempdir().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let real = bf::session::home::database_home();
    // No seam variable: the database's home, never `HOME` (which these runs set to another folder).
    let (o, st) = run_stats(cwd.path(), &["--version"], None, &[]);
    assert!(o.status.success());
    assert_eq!(st.home, real.map(|h| h.to_string_lossy().into_owned()));
    assert_eq!(st.lookups, 1);
    // The seam variable gives that folder in place of the answer; the database is asked all the same.
    let (o, st) = run_stats(cwd.path(), &["--version"], Some(fixture.path()), &[]);
    assert!(o.status.success());
    assert_eq!(st.home, Some(fixture.path().to_string_lossy().into_owned()));
    assert_eq!(st.lookups, 1);
    assert!(st.lookup_ns > 0.0, "the real lookup is timed");
}

#[test]
fn the_seam_with_an_empty_or_relative_folder_gives_no_home() {
    let cwd = tempfile::tempdir().unwrap();
    for v in ["", "relative/dir"] {
        let (o, st) = run_stats(cwd.path(), &["--version"], Some(Path::new(v)), &[]);
        assert!(o.status.success());
        assert_eq!(st.home, None, "{v:?} gives no home");
        assert_eq!(st.lookups, 1);
    }
}

#[test]
fn a_database_home_must_be_a_folder_on_the_disk() {
    use bf::session::home::usable;
    let d = tempfile::tempdir().unwrap();
    assert_eq!(usable(d.path().to_path_buf()), Some(d.path().to_path_buf()));
    assert_eq!(usable(d.path().join("missing")), None);
    let f = d.path().join("file");
    fs::write(&f, "x").unwrap();
    assert_eq!(usable(f), None);
    assert_eq!(usable(PathBuf::new()), None);
    assert_eq!(usable(PathBuf::from("relative")), None);
}

/// Each command makes exactly one call to the account database, whatever its path: a version, a
/// refusal outside a project, a command inside a project and `bf doctor` (T-16, C-3).
#[test]
fn account_home_is_asked_once_for_each_command() {
    let fx = Fx::new();
    let outside = tempfile::tempdir().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let state = fx.state_base.clone();
    let cases: [(&Path, &[&str]); 5] = [
        (outside.path(), &["--version"]),
        (outside.path(), &["--version", "--json"]),
        (outside.path(), &["status"]),
        (&fx.root, &["status"]),
        (&fx.root, &["doctor", "--json"]),
    ];
    for (dir, args) in cases {
        let (o, st) = run_stats(
            dir,
            args,
            Some(fixture.path()),
            &[("BF_STATE_HOME", &state)],
        );
        assert_eq!(
            st.lookups,
            1,
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    // A refusal is among them: `bf status` outside a project is refused.
    let (o, _) = run_stats(outside.path(), &["status"], Some(fixture.path()), &[]);
    assert!(!o.status.success());
}

/// The check of the seam refusal is one function given the real state folder and the project id.
#[test]
fn the_seam_refusal_check_sees_a_project_folder_in_the_state_folder() {
    use bf::session::home::state_folder_holds_project;
    let state = tempfile::tempdir().unwrap();
    assert!(!state_folder_holds_project(state.path(), "p-1234"));
    fs::create_dir_all(state.path().join("p-1234")).unwrap();
    assert!(state_folder_holds_project(state.path(), "p-1234"));
    assert!(!state_folder_holds_project(state.path(), "p-9999"));
    assert!(!state_folder_holds_project(state.path(), ""));
    assert!(!state_folder_holds_project(
        &state.path().join("none"),
        "p-1234"
    ));
}

/// The refusal names neither home and gives the step to unset the variable or use the shipped build.
#[test]
fn the_seam_refusal_names_no_home_and_gives_the_steps() {
    let bf::refusal::Fail::Refused(r) =
        bf::refusal::from_key("cli.refuse.test_seam_real_project", &[])
    else {
        panic!("the key gives a refusal")
    };
    let text = r.text();
    let real = bf::session::home::database_home().unwrap();
    assert!(!text.contains(&*real.to_string_lossy()), "{text}");
    assert!(!text.contains(".local/state"), "{text}");
    assert!(text.contains("BF_TEST_ACCOUNT_HOME"), "{text}");
    assert_eq!(r.do_this.len(), 2);
    assert!(r.do_this[0].contains("Unset"), "{text}");
    assert!(r.do_this[1].contains("shipped build"), "{text}");
}

#[test]
fn tilde_paths_lie_under_the_home_and_other_paths_are_themselves() {
    let home = bf::session::home::account_home();
    assert_eq!(
        bf::session::home::resolve_tilde("/x/y"),
        Some(PathBuf::from("/x/y"))
    );
    assert_eq!(bf::session::home::resolve_tilde("~"), home.clone());
    assert_eq!(
        bf::session::home::resolve_tilde("~/.claude/sessions"),
        home.map(|h| h.join(".claude/sessions"))
    );
    // `~name` is not a home of this account.
    assert_eq!(
        bf::session::home::resolve_tilde("~other/x"),
        Some(PathBuf::from("~other/x"))
    );
}

// ---- the adapter data ------------------------------------------------------------------------

fn shipped_adapter() -> toml::Table {
    let p = crate_dir().join("../../harness/claude/adapter.toml");
    fs::read_to_string(p).unwrap().parse().unwrap()
}

#[test]
fn the_compiled_adapter_data_equals_the_shipped_file_and_names_the_harness_folder() {
    let t = shipped_adapter();
    let s = |k: &str| t[k].as_str().unwrap().to_string();
    let d = &bf::adapter::DATA;
    assert_eq!(d.session_dir, s("session_dir"));
    assert_eq!(d.session_dir, "~/.claude/sessions");
    assert_eq!(d.session_file, "<pid>.json");
    assert_eq!(
        [d.key_pid, d.key_proc_start, d.key_session_id],
        ["pid", "procStart", "sessionId"]
    );
    assert_eq!(d.fallback_variable, s("fallback_variable"));
    // D-51: the harness's own variable, never the engine's `BF_SESSION`.
    assert_eq!(d.fallback_variable, "CLAUDE_CODE_SESSION_ID");
    assert_eq!(d.config_variable, s("config_variable"));
    assert_eq!(d.transcript_dir, s("transcript_dir"));
    assert!(d.transcript_dir.starts_with("~/.claude/"));
    let list = |k: &str| -> Vec<String> {
        t[k].as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(d.subagent_fields.to_vec(), list("subagent_fields"));
    assert_eq!(d.programs.to_vec(), list("programs"));
    assert_eq!(bf::adapter::session_file_name(4242), "4242.json");
}

/// The fields today's turn check reads to mark a subagent's payload (`gates.rs`): the adapter's list
/// holds all of them (D-51).
#[test]
fn the_adapter_subagent_fields_equal_the_turn_checks_list() {
    let src = fs::read_to_string(crate_dir().join("src/gates.rs")).unwrap();
    let at = src
        .find("\"agent_id\"")
        .expect("the turn check names agent_id");
    let open = src[..at].rfind('[').unwrap();
    let close = at + src[at..].find(']').unwrap();
    let mut read: Vec<String> = src[open + 1..close]
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect();
    let mut have: Vec<String> = bf::adapter::DATA
        .subagent_fields
        .iter()
        .map(|x| x.to_string())
        .collect();
    read.sort();
    have.sort();
    assert_eq!(read.len(), 5, "{read:?}");
    assert_eq!(have, read);
}

// ---- the version and the seam ----------------------------------------------------------------

#[test]
fn version_json_carries_the_fixed_test_seam_field() {
    let o = Command::new(bin())
        .args(["--version", "--json"])
        .output()
        .unwrap();
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["test_seam"], serde_json::json!(SEAM));
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn plain_version_names_a_test_build_only_in_a_test_build() {
    let o = Command::new(bin()).arg("--version").output().unwrap();
    assert!(o.status.success());
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(text.starts_with("bf "));
    let said = text.contains(&bf::strings::t("cli.version.test_seam_build", &[]));
    assert_eq!(said, SEAM, "{text}");
}

#[test]
fn the_seam_is_off_by_default_and_the_texts_exist() {
    let cargo = fs::read_to_string(crate_dir().join("Cargo.toml")).unwrap();
    let t: toml::Table = cargo.parse().unwrap();
    assert!(t["features"]["test_seam"].as_array().unwrap().is_empty());
    assert!(t["features"].get("default").is_none());
    assert!(!bf::strings::t("cli.version.test_seam_build", &[]).is_empty());
    assert!(!bf::strings::t("cli.doctor.test_seam_build", &[]).is_empty());
    assert!(!bf::strings::t("cli.refuse.test_seam_real_project.refused", &[]).is_empty());
}

/// The release test runs on the artifact of the release's own build command: the path is given in
/// `BF_RELEASE_ARTIFACT` (the CI job sets it after `cargo build --release --locked -p bf`). In CI a
/// missing artifact fails the test; on a developer's machine, with no `CI` variable, there is no
/// release artifact to test and the test says so and stops, since it never builds one by a command
/// of its own.
#[test]
fn the_release_artifact_is_no_test_build() {
    let Some(path) = std::env::var_os("BF_RELEASE_ARTIFACT") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI gave no BF_RELEASE_ARTIFACT: the release test would check nothing"
        );
        eprintln!("no BF_RELEASE_ARTIFACT and no CI: no release artifact to test");
        return;
    };
    let empty = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(&path)
            .args(args)
            .env("BF_TEST_ACCOUNT_HOME", empty.path())
            .output()
            .unwrap()
    };
    let o = run(&["--version", "--json"]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["test_seam"], serde_json::json!(false));
    // The plain version is one line: no line of the seam, with the seam variable set.
    let o = run(&["--version"]);
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    assert_eq!(text.lines().count(), 1, "{text}");
    // The binary carries neither the variable's name nor any key or text of the seam.
    let bytes = fs::read(&path).unwrap();
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(!has(b"BF_TEST_ACCOUNT_HOME"));
    assert!(!has(b"BF_TEST_ACCOUNT_STATS"));
    assert!(!has(b"test_seam_build"));
    assert!(!has(b"test_seam_real_project"));
    assert!(!has(b"test_seam_checked_state_folder"));
}

/// The doctor of a test build names the state folder it checked: `.local/state/blackfactory` under
/// the home the test reads itself from the account database, and the line of the seam for the
/// human.
#[test]
fn a_test_build_doctor_names_the_state_folder_it_checked() {
    let fx = Fx::new();
    let fixture = tempfile::tempdir().unwrap();
    let env = [("BF_TEST_ACCOUNT_HOME", fixture.path().to_str().unwrap())];
    let o = fx.bf_env(&fx.root, &["doctor", "--json"], "", &env);
    let v = o.json();
    let real = bf::session::home::database_home().unwrap();
    assert_eq!(
        v["test_seam_checked_state_folder"],
        serde_json::json!(real
            .join(".local/state/blackfactory")
            .to_string_lossy()
            .into_owned()),
        "{}",
        o.all()
    );
    let o = fx.bf_env(&fx.root, &["doctor"], "", &env);
    let line = bf::strings::t("cli.doctor.test_seam_build", &[]);
    assert!(!line.is_empty());
    assert!(o.stdout.trim_end().ends_with(&line), "{}", o.all());
    // The engine's own checks are still printed, and the fixture home is not named.
    assert!(o.stdout.lines().count() > 1, "{}", o.all());
    assert!(!o.stdout.contains(&*fixture.path().to_string_lossy()));
}

// ---- the source test -------------------------------------------------------------------------

/// Files that still read `HOME` before the unit that owns each moves it to the resolver. The list
/// may only shrink: it holds no more than `PENDING_LIMIT` entries, each one of `PENDING_FIRST`, and
/// every entry must still read `HOME`.
const PENDING_FIRST: [&str; 0] = [];
const PENDING_LIMIT: usize = 0;
const PENDING: &[&str] = &PENDING_FIRST;

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The code of a line: a whole-line comment is nothing, a trailing comment is cut.
fn code_of(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with("//") {
        return "";
    }
    let mut in_str = false;
    let b = line.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'/' if !in_str && b[i + 1] == b'/' => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// The text of every string literal of a line of code, whatever its prefix (`b"..."`, `c"..."`).
fn literals_of(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match (&mut cur, c) {
            (Some(s), '\\') => {
                s.push('\\');
                if let Some(n) = chars.next() {
                    s.push(n);
                }
            }
            (Some(_), '"') => out.push(cur.take().unwrap()),
            (Some(s), c) => s.push(c),
            (None, '"') => cur = Some(String::new()),
            (None, _) => {}
        }
    }
    out
}

/// The code of a line with its string literals emptied, so a brace in a text counts for nothing.
fn without_literals(code: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match (in_str, c) {
            (true, '\\') => {
                chars.next();
            }
            (_, '"') => in_str = !in_str,
            (false, c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// A name of the two variables in a literal, with the ends a byte or C literal may carry.
fn names_a_home_variable(lit: &str) -> bool {
    let bare = lit
        .strip_suffix("\\0")
        .or_else(|| lit.strip_suffix("\\x00"))
        .unwrap_or(lit);
    bare == "HOME" || bare == "BF_TEST_ACCOUNT_HOME"
}

/// The reads of `HOME` or `BF_TEST_ACCOUNT_HOME` in a line of code, in the forms C-3 lists: the
/// variable's name as a text of any kind (`var`, `var_os`, `env!`, `option_env!`, a byte or C literal
/// for `getenv`, a helper that takes the name), `getenv(`, `home_dir(` and the crates `dirs`, `home`
/// and `directories` called.
fn reads_in(code: &str) -> bool {
    literals_of(code).iter().any(|l| names_a_home_variable(l))
        || code.contains("getenv(")
        || code.contains("home_dir(")
        || code.contains("dirs::")
        || code.contains("directories::")
        || code.contains("home::cargo_home")
        || code.contains("home::rustup_home")
        || code.contains("use home::")
        || code.contains("use dirs")
        || code.contains("use directories")
        || code.contains("extern crate")
}

/// The reads in every file under `src` but the resolver's module and the files in `allowed`.
fn home_reads(src: &Path, allowed: &[&str]) -> Vec<String> {
    let mut files = Vec::new();
    rs_files(src, &mut files);
    files.sort();
    let mut found = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "session/home.rs" || allowed.contains(&rel.as_str()) {
            continue;
        }
        let text = fs::read_to_string(&f).unwrap();
        for (n, line) in text.lines().enumerate() {
            if reads_in(code_of(line)) {
                found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    found
}

/// The lines of the resolver's module that name `BF_TEST_ACCOUNT_HOME` outside an item under
/// `#[cfg(feature = "test_seam")]` (an item that is `cfg(not(...))` is not under it).
fn seam_reads_outside_cfg(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut depth: i64 = 0;
    let mut cfg_from: Option<i64> = None; // the depth at which the cfg item's block opened
    let mut pending = false;
    for (n, line) in text.lines().enumerate() {
        let code = code_of(line);
        let t = code.trim();
        if t.starts_with("#[cfg(") && t.contains("feature = \"test_seam\"") && !t.contains("not(") {
            pending = true;
            continue;
        }
        let bare = without_literals(code);
        let opens = bare.matches('{').count() as i64;
        let closes = bare.matches('}').count() as i64;
        let under = cfg_from.is_some() || pending;
        let names = literals_of(code)
            .iter()
            .any(|l| l.contains("BF_TEST_ACCOUNT_HOME"));
        if names && !under {
            found.push(format!("{}: {}", n + 1, line.trim()));
        }
        if pending && !t.is_empty() {
            pending = false;
            if opens > closes {
                cfg_from = Some(depth);
            }
        }
        depth += opens - closes;
        if let Some(d) = cfg_from {
            if depth <= d {
                cfg_from = None;
            }
        }
    }
    found
}

/// The crates that give a home folder, named as a dependency in a manifest.
fn manifest_names_a_home_crate(manifest: &str) -> bool {
    manifest.lines().any(|l| {
        let t = l.trim();
        ["dirs", "home", "directories", "dirs-next", "dirs_next"]
            .iter()
            .any(|c| {
                t.starts_with(&format!("{c} ="))
                    || t.starts_with(&format!("{c}="))
                    || t.starts_with(&format!("{c}."))
                    || t == format!("[dependencies.{c}]")
                    || t == format!("[dev-dependencies.{c}]")
                    || t == format!("[build-dependencies.{c}]")
            })
    })
}

#[test]
#[allow(clippy::absurd_extreme_comparisons)]
fn no_code_but_the_resolver_reads_home() {
    let src = crate_dir().join("src");
    let found = home_reads(&src, PENDING);
    assert!(
        found.is_empty(),
        "reads of HOME outside the resolver:\n{}",
        found.join("\n")
    );
    // The pending list only shrinks: no longer than it was, with no file it did not hold, and each
    // entry still reads.
    assert!(PENDING.len() <= PENDING_LIMIT);
    let own = home_reads(&src, &[]);
    for f in PENDING {
        assert!(
            PENDING_FIRST.contains(f),
            "{f} is not an entry the list began with"
        );
        assert!(
            own.iter().any(|l| l.starts_with(&format!("{f}:"))),
            "{f} no longer reads HOME: take it off the pending list"
        );
    }
    // The resolver's module names the seam variable only under the feature's cfg.
    let home = fs::read_to_string(src.join("session/home.rs")).unwrap();
    let outside = seam_reads_outside_cfg(&home);
    assert!(outside.is_empty(), "{outside:?}");
    // No crate that gives a home folder is a dependency.
    let manifest = fs::read_to_string(crate_dir().join("Cargo.toml")).unwrap();
    assert!(!manifest_names_a_home_crate(&manifest));
}

#[test]
fn planted_reads_make_the_source_test_fail() {
    let forms = [
        r#"let h = std::env::var("HOME");"#,
        r#"let h = std::env::var_os("HOME");"#,
        r#"let h = env!("HOME");"#,
        r#"let h = std::env::var("BF_TEST_ACCOUNT_HOME");"#,
        r#"let h = std::env::home_dir();"#,
        r#"let h = absolute_env("HOME");"#,
        r#"let h = libc::getenv(b"HOME\0".as_ptr() as *const libc::c_char);"#,
        r#"let h = libc::getenv(name.as_ptr());"#,
        r#"let h = c"HOME";"#,
        r#"let h = b"HOME";"#,
        r#"let h = dirs::home_dir();"#,
        r#"let h = directories::BaseDirs::new();"#,
        r#"use home::home_dir;"#,
        r#"let h = home::cargo_home();"#,
    ];
    for form in forms {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("session")).unwrap();
        fs::write(
            d.path().join("planted.rs"),
            format!("fn f() {{ {form} }}\n"),
        )
        .unwrap();
        // The resolver's own module may read it.
        fs::write(
            d.path().join("session/home.rs"),
            format!("fn f() {{ {form} }}\n"),
        )
        .unwrap();
        let found = home_reads(d.path(), &[]);
        assert_eq!(found.len(), 1, "{form}: {found:?}");
        assert!(found[0].starts_with("planted.rs:1"));
    }
    // A comment is no read, and neither is a path through the engine's own `session::home`.
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("c.rs"),
        "// std::env::var(\"HOME\")\nuse crate::session::home;\nfn f() { home::account_home(); }\n",
    )
    .unwrap();
    assert!(home_reads(d.path(), &[]).is_empty());
}

#[test]
fn a_read_of_the_seam_variable_in_the_resolver_outside_the_cfg_makes_it_fail() {
    let under = "#[cfg(feature = \"test_seam\")]\nfn a() {\n    let _ = std::env::var_os(\"BF_TEST_ACCOUNT_HOME\");\n}\n\
fn b() {\n    #[cfg(feature = \"test_seam\")]\n    {\n        let _ = std::env::var_os(\"BF_TEST_ACCOUNT_HOME\");\n    }\n}\n";
    assert!(seam_reads_outside_cfg(under).is_empty());
    for plain in [
        "fn a() {\n    let _ = std::env::var_os(\"BF_TEST_ACCOUNT_HOME\");\n}\n",
        "#[cfg(not(feature = \"test_seam\"))]\nfn a() {\n    let _ = std::env::var_os(\"BF_TEST_ACCOUNT_HOME\");\n}\n",
        "#[cfg(feature = \"test_seam\")]\nfn a() {}\nfn b() {\n    let _ = std::env::var_os(\"BF_TEST_ACCOUNT_HOME\");\n}\n",
        "fn a() {\n    #[cfg(feature = \"test_seam\")]\n    {\n    }\n    let _ = std::env::var_os(\"BF_TEST_ACCOUNT_HOME\");\n}\n",
        "fn a() { let _ = c\"BF_TEST_ACCOUNT_HOME\"; }\n",
    ] {
        assert_eq!(seam_reads_outside_cfg(plain).len(), 1, "{plain}");
    }
}

#[test]
fn a_home_crate_named_in_a_manifest_makes_the_source_test_fail() {
    for dep in [
        "dirs = \"5\"",
        "home = \"0.5\"",
        "directories = { version = \"5\" }",
        "dirs.workspace = true",
        "[dependencies.dirs]\nversion = \"5\"",
    ] {
        let m = format!("[dependencies]\nserde = \"1\"\n{dep}\n");
        assert!(manifest_names_a_home_crate(&m), "{dep}");
    }
    assert!(!manifest_names_a_home_crate(
        "[dependencies]\nserde = \"1\"\nlibc = \"0.2\"\n"
    ));
}

#[test]
fn the_pending_list_cannot_grow() {
    // The list is empty with a limit of 0: no file may read `HOME` outside the resolver.
    assert_eq!(PENDING_LIMIT, 0);
    assert_eq!(PENDING_FIRST.len(), PENDING_LIMIT);
}

#[test]
fn a_child_sees_the_account_home_and_never_the_callers_home() {
    let other = tempfile::tempdir().unwrap();
    let mut c = Command::new("/bin/sh");
    c.args(["-c", "printf %s \"${HOME-unset}\""])
        .env_clear()
        .env("HOME", other.path());
    bf::session::home::set_env(&mut c);
    let out = String::from_utf8(c.output().unwrap().stdout).unwrap();
    match bf::session::home::account_home() {
        Some(h) => assert_eq!(out, h.to_string_lossy()),
        None => assert_eq!(out, "unset"),
    }
    assert_ne!(out, other.path().to_string_lossy());
}

// ---- the build of the seam fails on an empty text --------------------------------------------

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().filter_map(|e| e.ok()) {
        let (p, q) = (e.path(), to.join(e.file_name()));
        if p.is_dir() {
            copy_dir(&p, &q);
        } else {
            fs::copy(&p, &q).unwrap();
        }
    }
}

/// A copy of the crate (source, build script, strings, the adapter data and the one harness file the
/// source includes) in a temporary workspace, with no test files but `all.rs`.
fn crate_copy() -> tempfile::TempDir {
    let root = crate_dir().join("../..");
    let d = tempfile::tempdir().unwrap();
    for f in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(root.join(f), d.path().join(f)).unwrap();
    }
    let c = d.path().join("crates/bf");
    fs::create_dir_all(c.join("tests")).unwrap();
    for f in ["Cargo.toml", "build.rs"] {
        fs::copy(crate_dir().join(f), c.join(f)).unwrap();
    }
    for dir in ["src", "strings"] {
        copy_dir(&crate_dir().join(dir), &c.join(dir));
    }
    fs::copy(crate_dir().join("tests/all.rs"), c.join("tests/all.rs")).unwrap();
    copy_dir(
        &root.join("harness/claude/agents"),
        &d.path().join("harness/claude/agents"),
    );
    fs::copy(
        root.join("harness/claude/adapter.toml"),
        d.path().join("harness/claude/adapter.toml"),
    )
    .unwrap();
    d
}

/// `cargo check` of the copy, offline, in a target folder of its own below the workspace's.
fn check_copy(copy: &Path, seam: bool) -> std::process::Output {
    let target = crate_dir().join("../../target/account-home-copy-check");
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.args(["check", "--offline", "--lib", "--manifest-path"])
        .arg(copy.join("crates/bf/Cargo.toml"))
        .env("CARGO_TARGET_DIR", target);
    if seam {
        c.args(["--features", "test_seam"]);
    }
    c.output().unwrap()
}

/// C-3, TH-5: a test empties each seam key in a copy of the strings; the build with the feature is
/// refused, naming the key, and the build without it, which carries neither key, is not affected.
#[test]
fn the_seam_build_fails_on_an_empty_or_missing_seam_text() {
    for key in ["cli.version.test_seam_build", "cli.doctor.test_seam_build"] {
        for missing in [false, true] {
            let copy = crate_copy();
            let file = copy.path().join("crates/bf/strings/en/version.toml");
            let text = fs::read_to_string(&file).unwrap();
            let changed: String = text
                .lines()
                .filter_map(|l| {
                    if l.starts_with(&format!("\"{key}\"")) {
                        (!missing).then(|| format!("\"{key}\" = \"\""))
                    } else {
                        Some(l.to_string())
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(&file, changed).unwrap();
            let o = check_copy(copy.path(), true);
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            assert!(!o.status.success(), "{key} missing={missing}: built");
            assert!(err.contains(key), "{key} missing={missing}: {err}");
            let o = check_copy(copy.path(), false);
            assert!(
                o.status.success(),
                "{key} missing={missing}: the build without the feature is affected: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }
}

// ---- T-16: the lookup's time -----------------------------------------------------------------

/// The resolver's call to the account database, timed inside each command of a test build, and
/// counted: exactly one for each command, on a version, a plain command, a refusal and `bf doctor`.
/// The fixture holds the home of its own (the seam), so nothing lies in the developer's real folders.
#[test]
fn timing_account_home_lookup_is_one_call_within_the_time() {
    let fx = Fx::new();
    let outside = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let state = fx.state_base.clone();
    let cases: [(&str, &Path, &[&str]); 4] = [
        ("version", outside.path(), &["--version"]),
        ("plain", &fx.root, &["status"]),
        ("refusal", outside.path(), &["status"]),
        ("doctor", &fx.root, &["doctor", "--json"]),
    ];
    for (name, dir, args) in cases {
        let mut ns = Vec::new();
        for _ in 0..200 {
            let (_, st) = run_stats(dir, args, Some(home.path()), &[("BF_STATE_HOME", &state)]);
            assert_eq!(st.lookups, 1, "{name}");
            ns.push(st.lookup_ns / 1e6);
        }
        ns.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let (median, p99) = (ns[100], ns[197]);
        println!("account lookup, {name}: median {median:.3} ms, p99 {p99:.3} ms");
        assert!(median < 3.0, "{name}: median {median} ms");
        assert!(p99 < 20.0, "{name}: p99 {p99} ms");
    }
}

// ---- review findings: the policy's list never passes the caller's home on; the guard's lookups ---------

/// In every place that builds a check's or a scan's process, the account's home is set after the loop
/// that copies the policy's listed variables, so a policy that lists the home's variable cannot pass the
/// caller's value on.
#[test]
fn the_account_home_is_set_after_the_policys_listed_variables() {
    for f in ["src/evidence.rs", "src/scan.rs"] {
        let text = fs::read_to_string(crate_dir().join(f)).unwrap();
        let loop_at = text
            .find("policy.exec.env")
            .unwrap_or_else(|| panic!("{f}: no loop over the policy's variables"));
        let set_at = text
            .find("session::home::set_env(&mut cmd)")
            .unwrap_or_else(|| panic!("{f}: the account's home is never set for the process"));
        assert!(set_at > loop_at, "{f}: the home is set before the policy's variables");
    }
}

/// A guard call whose paths have no `~` makes no lookup of the account database; one with a `~` path makes one.
#[test]
fn a_guard_call_without_a_tilde_path_makes_no_lookup() {
    if !SEAM {
        return;
    }
    let table = serde_json::json!({"root": "/nonexistent-root"});
    for (tool, input) in [
        ("Write", serde_json::json!({"file_path": "/nonexistent-root/a.txt"})),
        ("Read", serde_json::json!({"file_path": "b.txt"})),
    ] {
        let _ = bf::guard::decide(
            &serde_json::json!({"tool_name": tool, "tool_input": input, "cwd": "/nonexistent-root"}),
            &table,
        );
    }
    assert_eq!(bf::session::home::lookup_count(), 0);
    let _ = bf::guard::decide(
        &serde_json::json!({"tool_name": "Write", "tool_input": {"file_path": "~/a.txt"}, "cwd": "/nonexistent-root"}),
        &table,
    );
    assert_eq!(bf::session::home::lookup_count(), 1);
}
