//! U-86 (R-76 acceptance 9 to 12, D-104, TH-20): the server of a unit check's compile cache, its write
//! profile, the client's rules, the run key that agrees with the receipt, and the end of the server and
//! its folder. A fake program (a shell script that records its arguments and environment and makes the
//! socket) stands for sccache. Every test sets the account's home through the test seam to a folder of
//! its own, so none reads or writes the account's real cache folder.

mod common;
use bf::sandbox::compile_cache::{self as cc, server, Paths};
use common::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

fn own_home() -> (tempfile::TempDir, PathBuf) {
    // A short path: the socket's path may not be longer than 100 bytes, and the system's temporary
    // folder is longer than that with the cache folders below it.
    let t = tempfile::Builder::new()
        .prefix("h")
        .tempdir_in("/tmp")
        .unwrap();
    let h = fs::canonicalize(t.path()).unwrap();
    (t, h)
}

const FAKE: &str = r#"#!/bin/sh
if [ "$1" = "--stop-server" ]; then
  { echo "STOP"; env | sort; } > "$SCCACHE_DIR/stop.rec"
  [ -f "$SCCACHE_DIR/server.pid" ] && kill "$(cat "$SCCACHE_DIR/server.pid")"
  exit 0
fi
{ echo "ARGS $*"; env | sort; } > "$SCCACHE_DIR/server.rec"
echo $$ > "$SCCACHE_DIR/server.pid"
exec /usr/bin/perl -e 'use IO::Socket::UNIX; unlink $ENV{SCCACHE_SERVER_UDS}; my $s = IO::Socket::UNIX->new(Local=>$ENV{SCCACHE_SERVER_UDS}, Listen=>5) or die; while (1) { my $c = $s->accept; }'
"#;

fn script(dir: &Path, name: &str, body: &str) -> String {
    let f = dir.join(name);
    fs::write(&f, body).unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
    f.to_string_lossy().to_string()
}

/// Can this process bind a Unix socket? Not inside a check the engine runs under its own sandbox, which
/// denies the network; a test that needs a socket skips there.
fn can_bind() -> bool {
    let (_t, h) = own_home();
    std::os::unix::net::UnixListener::bind(h.join("probe.sock")).is_ok()
}

fn fake_program(dir: &Path) -> String {
    script(dir, "fake-sccache", FAKE)
}

fn paths_in(home: &Path) -> Paths {
    let folder = home.join("Library/Caches/blackfactory-cc/0123456789abcdef/aaaaaaaa");
    fs::create_dir_all(folder.join("units")).unwrap();
    fs::create_dir_all(folder.join("t")).unwrap();
    Paths {
        units: folder.join("units"),
        runs: folder.join("t"),
        folder,
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn env_keys(rec: &str) -> Vec<String> {
    let mut v: Vec<String> = rec
        .lines()
        .filter(|l| !l.starts_with("ARGS") && l != &"STOP")
        .filter_map(|l| l.split('=').next())
        .filter(|k| !["PWD", "SHLVL", "_", "OLDPWD"].contains(k))
        .map(String::from)
        .collect();
    v.sort();
    v
}

fn value<'a>(rec: &'a str, key: &str) -> &'a str {
    rec.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .unwrap_or("")
}

const SERVER_KEYS: [&str; 12] = [
    "PATH",
    "SCCACHE_CACHED_CONF",
    "SCCACHE_CACHE_SIZE",
    "SCCACHE_CONF",
    "SCCACHE_DIR",
    "SCCACHE_IDLE_TIMEOUT",
    "SCCACHE_LOCAL_RW_MODE",
    "SCCACHE_NO_DAEMON",
    "SCCACHE_SERVER_UDS",
    "SCCACHE_START_SERVER",
    "TMPDIR",
    "",
];

// ---- the server, started by the engine's own code with no sandbox (a fake program) ----

#[test]
fn a_server_gets_only_its_listed_environment_and_its_own_files_and_ends_with_its_folder() {
    if !can_bind() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let paths = paths_in(&home);
    let srv = server::start(
        &paths,
        &server::Start {
            program: &program,
            profile: None,
            path: "/usr/bin:/bin",
            idle_seconds: 123,
        },
    )
    .expect("the fake server accepts on its socket");
    let run = srv.run.clone();
    // The folder is `t/<8 hex>/` with the socket, the two config files and `tmp/`.
    assert_eq!(run.name.len(), 8);
    assert!(run.name.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(run.dir, paths.runs.join(&run.name));
    assert_eq!(
        names(&run.dir),
        ["cached.toml", "conf.toml", "lock", "s", "tmp"]
    );
    assert_eq!(fs::metadata(&run.conf).unwrap().len(), 0);
    assert_eq!(fs::metadata(&run.cached).unwrap().len(), 0);
    let rec = fs::read_to_string(paths.units.join("server.rec")).unwrap();
    assert!(
        rec.starts_with("ARGS \n") || rec.starts_with("ARGS\n"),
        "{rec}"
    );
    let mut want: Vec<String> = SERVER_KEYS
        .iter()
        .filter(|k| !k.is_empty())
        .map(|s| s.to_string())
        .collect();
    want.sort();
    assert_eq!(env_keys(&rec), want, "{rec}");
    assert_eq!(value(&rec, "SCCACHE_START_SERVER"), "1");
    assert_eq!(value(&rec, "SCCACHE_NO_DAEMON"), "1");
    assert_eq!(value(&rec, "SCCACHE_CACHE_SIZE"), "10G");
    assert_eq!(value(&rec, "SCCACHE_LOCAL_RW_MODE"), "READ_WRITE");
    assert_eq!(value(&rec, "SCCACHE_IDLE_TIMEOUT"), "123");
    assert_eq!(value(&rec, "PATH"), "/usr/bin:/bin");
    assert_eq!(value(&rec, "SCCACHE_DIR"), paths.units.to_str().unwrap());
    assert_eq!(
        value(&rec, "SCCACHE_SERVER_UDS"),
        run.socket.to_str().unwrap()
    );
    assert_eq!(value(&rec, "SCCACHE_CONF"), run.conf.to_str().unwrap());
    assert_eq!(
        value(&rec, "SCCACHE_CACHED_CONF"),
        run.cached.to_str().unwrap()
    );
    assert_eq!(value(&rec, "TMPDIR"), run.tmp.to_str().unwrap());
    drop(srv);
    // `--stop-server` ran with the same environment less the two names that start a server.
    let stop = fs::read_to_string(paths.units.join("stop.rec")).unwrap();
    assert!(stop.starts_with("STOP"), "{stop}");
    assert!(!stop.contains("SCCACHE_START_SERVER"), "{stop}");
    assert!(!stop.contains("SCCACHE_NO_DAEMON"), "{stop}");
    assert_eq!(
        value(&stop, "SCCACHE_SERVER_UDS"),
        run.socket.to_str().unwrap()
    );
    assert!(!run.dir.exists(), "the run's folder is gone");
}

#[test]
fn a_server_that_does_not_start_or_a_socket_too_long_gives_a_reason_and_leaves_no_folder() {
    let (_t, home) = own_home();
    let paths = paths_in(&home);
    let dead = script(&home, "dead", "#!/bin/sh\nexit 3\n");
    let r = server::start(
        &paths,
        &server::Start {
            program: &dead,
            profile: None,
            path: "/usr/bin:/bin",
            idle_seconds: 60,
        },
    );
    assert!(r.err().is_some_and(|e| !e.is_empty()));
    assert!(names(&paths.runs).is_empty(), "no folder is left");
    let missing = home.join("no-such-program").to_string_lossy().to_string();
    let r = server::start(
        &paths,
        &server::Start {
            program: &missing,
            profile: None,
            path: "/usr/bin:/bin",
            idle_seconds: 60,
        },
    );
    assert!(r.err().is_some_and(|e| !e.is_empty()));
    assert!(names(&paths.runs).is_empty());
    // A socket path longer than 100 bytes: no folder is made.
    let long = home.join("x".repeat(90));
    fs::create_dir_all(long.join("t")).unwrap();
    let p = Paths {
        folder: long.clone(),
        units: long.join("units"),
        runs: long.join("t"),
    };
    let e = server::make_run_dir(&p).unwrap_err();
    assert!(e.contains("100"), "{e}");
    assert!(names(&p.runs).is_empty());
}

#[test]
fn the_run_folder_is_made_below_a_t_that_is_no_link_and_a_name_is_never_reused() {
    let (_t, home) = own_home();
    let paths = paths_in(&home);
    let a = server::make_run_dir(&paths).unwrap();
    let b = server::make_run_dir(&paths).unwrap();
    assert_ne!(a.name, b.name);
    // A link at `t/` is refused, and what it points at is not written.
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::remove_dir_all(&paths.runs).unwrap();
    symlink(&elsewhere, &paths.runs).unwrap();
    assert!(server::make_run_dir(&paths).is_err());
    assert!(names(&elsewhere).is_empty());
}

#[test]
fn removing_a_run_folder_unlinks_a_link_and_never_enters_it() {
    let (_t, home) = own_home();
    let paths = paths_in(&home);
    let run = server::make_run_dir(&paths).unwrap();
    let outside = home.join("outside");
    fs::create_dir_all(outside.join("deep")).unwrap();
    fs::write(outside.join("f"), "keep\n").unwrap();
    fs::write(outside.join("deep/g"), "keep\n").unwrap();
    symlink(&outside, run.tmp.join("to-outside")).unwrap();
    symlink(outside.join("f"), run.tmp.join("to-file")).unwrap();
    fs::create_dir_all(run.tmp.join("sub/sub2")).unwrap();
    fs::write(run.tmp.join("sub/sub2/x"), "x").unwrap();
    fs::set_permissions(run.tmp.join("sub"), fs::Permissions::from_mode(0o000)).unwrap();
    server::remove_tree(&run.runs, &run.name);
    assert!(!run.dir.exists());
    assert_eq!(fs::read_to_string(outside.join("f")).unwrap(), "keep\n");
    assert_eq!(
        fs::read_to_string(outside.join("deep/g")).unwrap(),
        "keep\n"
    );
}

// ---- the profiles ----

fn check_base() -> String {
    bf::sandbox::check_profile(
        "/state",
        "/root",
        "/root/.bf/work/U-1",
        false,
        "/transcripts",
        &["/repo".to_string()],
    )
}

#[test]
fn the_server_profile_denies_every_write_but_the_listed_ones_and_repeats_the_check_rules_after_them(
) {
    let base = check_base();
    let p = bf::sandbox::cache_server_profile(
        &base,
        "/root/.bf/work/U-1",
        "/cache/f",
        "/cache/f/t/ab12cd34",
    );
    assert!(
        !p.contains("deny file-read* file-write* (subpath \"/cache"),
        "{p}"
    );
    let at = |needle: &str| {
        p.find(needle)
            .unwrap_or_else(|| panic!("`{needle}` is not in\n{p}"))
    };
    let deny_all = at("(deny file-write*)\n");
    let units = at("(allow file-write* (subpath \"/cache/f/units\"))");
    let run = at("(allow file-write* (subpath \"/cache/f/t/ab12cd34\"))");
    let null = at("(allow file-write* (literal \"/dev/null\"))");
    assert!(deny_all < units && units < run && run < null);
    // The check profile's rules come again after those, in their order.
    let rules = base.strip_prefix("(version 1)\n(allow default)\n").unwrap();
    let rules_at = at(rules);
    assert!(null < rules_at, "{p}");
    // The socket rule is the last, after the check's network denial.
    let sock = at("(allow network-bind network-inbound (local unix-socket (path-literal \"/cache/f/t/ab12cd34/s\")))");
    assert!(rules_at < sock);
    assert!(p.ends_with("\")))\n"));
    // Only the one socket is opened for the network.
    assert_eq!(p.matches("unix-socket").count(), 1);
    assert!(p.contains("(deny network*)"));
}

#[test]
fn the_client_rule_adds_only_the_one_connect_and_the_tmp_rules_come_after_the_denial() {
    let r = bf::sandbox::cache_client_rule("/cache/f/t/ab12cd34/s");
    assert_eq!(
        r,
        "(allow network-outbound (remote unix-socket (path-literal \"/cache/f/t/ab12cd34/s\")))\n"
    );
    let top = "/h/Library/Caches/blackfactory-cc";
    let folder = format!("{top}/id/hh");
    let tmp = format!("{folder}/t/ab12cd34/tmp");
    // The link denial names the cache folder of the denial (the top), not the project's own folder.
    let t = bf::sandbox::cache_tmp_rules(top, &tmp);
    let a = t
        .find(&format!(
            "(allow file-read* file-write* (subpath \"{tmp}\"))"
        ))
        .unwrap();
    let d = t
        .find(&format!("(deny file-link (subpath \"{top}\"))"))
        .unwrap();
    let l = t
        .find(&format!("(allow file-link (subpath \"{tmp}\"))"))
        .unwrap();
    assert!(a < d && d < l, "{t}");
    // The folders above `tmp/`, from the cache's top down, give their metadata alone, before the allowance.
    let metadata: Vec<&str> = t
        .lines()
        .filter(|l| l.contains("file-read-metadata"))
        .collect();
    assert_eq!(metadata.len(), 5, "{t}");
    assert!(metadata[0].contains(&format!("(literal \"{top}\")")), "{t}");
    assert!(
        metadata[4].contains(&format!("(literal \"{folder}/t/ab12cd34\")")),
        "{t}"
    );
    assert!(t.rfind("file-read-metadata").unwrap() < a);
    assert_eq!(t.lines().count(), 8);
}

// ---- through the engine ----

fn write_check(fx: &Fx, command: &str) {
    write_check_id(fx, "X-1", command);
}

fn write_check_id(fx: &Fx, id: &str, command: &str) {
    let text = format!(
        "id=\"{id}\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\ncommand={command}\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"
    );
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("c.toml");
    fs::write(&f, text).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// A project with the setting on and a unit check running `probe.sh`, and the unit's copy started.
fn project(program: &str, probe: &str) -> (Fx, PathBuf) {
    let sw = if sandbox_probe() {
        Switch::All
    } else {
        Switch::Off
    };
    let fx = Fx::tiny_project_with(sw);
    fx.set_policy(
        "compile_cache = \"\"",
        &format!("compile_cache = \"{program}\""),
    );
    fs::write(fx.root.join("probe.sh"), probe).unwrap();
    write_check(&fx, "[\"sh\", \"probe.sh\"]");
    // A second unit check with the same command, so a run of it can count the first one's.
    let _ = fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\ncommand=\"sh probe.sh\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n");
    fx.commit_all("probe");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    (fx, wc)
}

fn run(fx: &Fx, wc: &Path, home: &Path) -> (i32, serde_json::Value) {
    run_check(fx, wc, home, "X-1")
}

fn run_check(fx: &Fx, wc: &Path, home: &Path, check: &str) -> (i32, serde_json::Value) {
    let o = fx.bf_env(
        wc,
        &[
            "evidence", "run", check, "--unit", "U-1", "--json", "--", "sh", "probe.sh",
        ],
        "",
        &[("BF_TEST_ACCOUNT_HOME", home.to_str().unwrap())],
    );
    let v = serde_json::from_str(&o.stdout).unwrap_or(serde_json::json!({"raw": o.all()}));
    (o.code, v)
}

fn cache_folder(home: &Path) -> PathBuf {
    let id = names(&home.join("Library/Caches/blackfactory-cc"));
    let top = home.join("Library/Caches/blackfactory-cc").join(&id[0]);
    top.join(&names(&top)[0])
}

fn count(fx: &Fx, kind: &str) -> usize {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter(|l| l.contains(&format!("\"{kind}\"")))
        .count()
}

const CLIENT_PROBE: &str = "echo \"RW=[$RUSTC_WRAPPER] RWW=[$RUSTC_WORKSPACE_WRAPPER] UDS=[$SCCACHE_SERVER_UDS] CONF=[$SCCACHE_CONF] CC=[$SCCACHE_CACHED_CONF] IGN=[$SCCACHE_IGNORE_SERVER_IO_ERROR] DIR=[$SCCACHE_DIR] TMP=[$TMPDIR]\"\n";

#[test]
fn a_unit_check_gets_the_client_variables_and_a_server_of_its_own_and_the_run_folder_is_gone() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let (fx, wc) = project(&program, CLIENT_PROBE);
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["sandbox"], "sandbox-exec+compile-cache", "{v}");
    assert_eq!(v["compile_cache_note"], "");
    let tail = v["output_tail"].as_str().unwrap().to_string();
    let folder = cache_folder(&home);
    let tmp = tail
        .split("TMP=[")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
        .to_string();
    let tmp = PathBuf::from(tmp);
    assert_eq!(tmp.file_name().unwrap(), "tmp");
    let run_dir = tmp.parent().unwrap().to_path_buf();
    assert_eq!(run_dir.parent().unwrap(), folder.join("t"));
    assert!(
        !tmp.starts_with(&fx.root),
        "TMPDIR lies outside every workspace"
    );
    assert!(
        tail.contains(&format!(
            "RW=[{program}] RWW=[] UDS=[{}/s] CONF=[/dev/null] CC=[/dev/null] IGN=[1] DIR=[]",
            run_dir.display()
        )),
        "{tail}"
    );
    // The server held its own files and the listed environment.
    let rec = fs::read_to_string(folder.join("units/server.rec")).unwrap();
    assert_eq!(
        value(&rec, "SCCACHE_CONF"),
        run_dir.join("conf.toml").to_str().unwrap()
    );
    assert_eq!(
        value(&rec, "SCCACHE_CACHED_CONF"),
        run_dir.join("cached.toml").to_str().unwrap()
    );
    assert_eq!(value(&rec, "TMPDIR"), tmp.to_str().unwrap());
    let mut want: Vec<String> = SERVER_KEYS
        .iter()
        .filter(|k| !k.is_empty())
        .map(|s| s.to_string())
        .collect();
    want.sort();
    assert_eq!(env_keys(&rec), want, "{rec}");
    // The server ended, its folder is gone, and the copy holds no new file.
    assert!(folder.join("units/stop.rec").exists());
    assert!(names(&folder.join("t")).is_empty());
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&wc)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).trim().is_empty(),
        "{:?}",
        status
    );
}

#[test]
fn the_check_reaches_its_tmp_and_nothing_else_of_the_cache_folder_and_makes_no_foreign_hard_link() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let probe = r#"echo data > "$TMPDIR/f" && echo write=ok
cat "$TMPDIR/f" > /dev/null && echo read=ok
RUN="$(dirname "$TMPDIR")"
cat "$RUN/conf.toml" 2>/dev/null; echo conf=$?
ls "$RUN/../../units" >/dev/null 2>&1; echo units=$?
mkdir -p "$RUN/../other" 2>/dev/null; echo mk=$?
ln "$TMPDIR/f" "$TMPDIR/inside" 2>/dev/null; echo inside=$?
ln "$RUN/../../units/server.rec" "$TMPDIR/foreign" 2>/dev/null; echo foreign_units=$?
ln "$RUN/../../units/server.rec" "$RUN/../../units/x" 2>/dev/null; echo units_link=$?
ln "$RUN/../other1/f" "$TMPDIR/foreign_run" 2>/dev/null; echo foreign_run=$?
echo "TMPDIR=$TMPDIR"
"#;
    let (fx, wc) = project(&program, "true\n");
    // The first run makes the cache folders; then another run's folder holds a file.
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    let other = cache_folder(&home).join("t/other1");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("f"), "theirs\n").unwrap();
    fs::write(wc.join("probe.sh"), probe).unwrap();
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    assert_eq!(fs::read_to_string(other.join("f")).unwrap(), "theirs\n");
    let t = v["output_tail"].as_str().unwrap();
    for ok in ["write=ok", "read=ok", "inside=0"] {
        assert!(t.contains(ok), "{t}");
    }
    for refused in [
        "conf=1",
        "units=1",
        "mk=1",
        "foreign_units=1",
        "units_link=1",
        "foreign_run=1",
    ] {
        assert!(t.contains(refused), "{refused}: {t}");
    }
    // The files in the cache folder are as they were.
    let folder = cache_folder(&home);
    assert!(!folder.join("units/x").exists());
    assert!(!folder.join("t/other").exists());
    assert!(!folder.join("t/other1/foreign").exists());
}

#[test]
fn the_check_can_stat_the_five_folders_and_work_in_a_project_but_cannot_list_or_read_in_them() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let probe = r#"RUN="$(dirname "$TMPDIR")"
T="$(dirname "$RUN")"
I="$(dirname "$T")"
P="$(dirname "$I")"
C="$(dirname "$P")"
for d in "$C" "$P" "$I" "$T" "$RUN"; do stat "$d" >/dev/null 2>&1; echo stat=$?; done
for d in "$C" "$P" "$I" "$T" "$RUN"; do ls "$d" >/dev/null 2>&1; echo ls=$?; done
cat "$RUN/conf.toml" >/dev/null 2>&1; echo conf=$?
cat "$I/units/server.rec" >/dev/null 2>&1; echo units=$?
mkdir "$TMPDIR/p" && cd "$TMPDIR/p" && pwd >/dev/null && echo cd=ok
"#;
    let (fx, wc) = project(&program, "true\n");
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    fs::write(wc.join("probe.sh"), probe).unwrap();
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    let t = v["output_tail"].as_str().unwrap();
    assert_eq!(t.matches("stat=0").count(), 5, "{t}");
    assert_eq!(t.matches("ls=1").count(), 5, "{t}");
    for s in ["conf=1", "units=1", "cd=ok"] {
        assert!(t.contains(s), "{s}: {t}");
    }
}

#[test]
fn the_server_profile_holds_no_metadata_rule() {
    let p = bf::sandbox::cache_server_profile(
        &check_base(),
        "/root/.bf/work/U-1",
        "/cache/f",
        "/cache/f/t/ab12cd34",
    );
    assert!(!p.contains("file-read-metadata"), "{p}");
}

#[test]
fn a_check_that_plants_a_link_in_its_tmp_leaves_the_target_as_it_was_after_the_removal() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let outside = home.join("outside");
    fs::create_dir_all(outside.join("deep")).unwrap();
    fs::write(outside.join("f"), "keep\n").unwrap();
    fs::write(outside.join("deep/g"), "keep\n").unwrap();
    let probe = format!(
        "ln -s {o} \"$TMPDIR/l1\"\nln -s {o}/f \"$TMPDIR/l2\"\n",
        o = outside.display()
    );
    let (fx, wc) = project(&program, &probe);
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    assert_eq!(fs::read_to_string(outside.join("f")).unwrap(), "keep\n");
    assert_eq!(
        fs::read_to_string(outside.join("deep/g")).unwrap(),
        "keep\n"
    );
    assert!(names(&cache_folder(&home).join("t")).is_empty());
}

#[test]
fn a_project_made_in_the_check_s_tmp_finds_itself_and_not_the_workspace_above() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let probe = "mkdir \"$TMPDIR/proj\" && cd \"$TMPDIR/proj\" && git init -q . && echo top=$(git rev-parse --show-toplevel)\n";
    let (fx, wc) = project(&program, probe);
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    let t = v["output_tail"].as_str().unwrap();
    let top = t.split("top=").nth(1).unwrap().trim();
    assert!(top.ends_with("/tmp/proj"), "{t}");
    assert!(!top.starts_with(fx.root.to_str().unwrap()));
}

#[test]
fn the_server_profile_in_use_reads_units_writes_the_copy_and_cannot_write_elsewhere() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    // A program that tries each thing as the server and writes the answers into `units/`.
    let wc_marker = "@WC@";
    let body = format!(
        r#"#!/bin/sh
if [ "$1" = "--stop-server" ]; then
  [ -f "$SCCACHE_DIR/server.pid" ] && kill "$(cat "$SCCACHE_DIR/server.pid")"
  exit 0
fi
R="$(dirname "$SCCACHE_SERVER_UDS")"
WC="{wc_marker}"
echo seed > "$SCCACHE_DIR/seed" ; echo seed_written=$? > "$SCCACHE_DIR/answers"
{{
cat "$SCCACHE_DIR/seed" >/dev/null; echo read_units=$?
cat "$SCCACHE_CONF" >/dev/null; echo read_run=$?
mkdir -p "$WC/target" && echo x > "$WC/target/o"; echo write_build=$?
echo x > "$WC/factory/y" 2>/dev/null; echo write_wc_factory=$?
echo x > "$WC/../../../factory/y" 2>/dev/null; echo write_root_factory=$?
echo x > "$WC/../U-2-copy" 2>/dev/null; echo write_other_copy=$?
}} >> "$SCCACHE_DIR/answers"
echo $$ > "$SCCACHE_DIR/server.pid"
exec /usr/bin/perl -e 'use IO::Socket::UNIX; unlink $ENV{{SCCACHE_SERVER_UDS}}; my $s = IO::Socket::UNIX->new(Local=>$ENV{{SCCACHE_SERVER_UDS}}, Listen=>5) or die; while (1) {{ my $c = $s->accept; }}'
"#
    );
    let program = script(&home, "probing", &body.replace(wc_marker, "@WC@"));
    let (fx, wc) = project(&program, "true\n");
    // The program needs the copy's path: write it in with the real one (the engine's folder).
    let real = fs::canonicalize(&wc).unwrap();
    let text = fs::read_to_string(&program)
        .unwrap()
        .replace("@WC@", real.to_str().unwrap());
    fs::write(&program, text).unwrap();
    fs::write(real.join(".gitignore"), "target\n").unwrap();
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    let answers = fs::read_to_string(cache_folder(&home).join("units/answers")).unwrap();
    for ok in [
        "seed_written=0",
        "read_units=0",
        "read_run=0",
        "write_build=0",
    ] {
        assert!(answers.contains(ok), "{ok}: {answers}");
    }
    for no in [
        "write_wc_factory=1",
        "write_root_factory=1",
        "write_other_copy=1",
    ] {
        assert!(answers.contains(no), "{no}: {answers}");
    }
}

#[test]
fn a_server_that_does_not_start_gives_a_run_without_the_cache_that_a_second_run_finds_again() {
    let (_t, home) = own_home();
    let dead = script(&home, "dead-sccache", "#!/bin/sh\nexit 3\n");
    let (fx, wc) = project(&dead, "exit 1\n");
    let (code, v) = run(&fx, &wc, &home);
    if !sandbox_probe() {
        // No sandbox: no cache at all, and the note says so.
        assert_eq!(code, 1);
        assert_eq!(v["sandbox"], "none");
        return;
    }
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["sandbox"], "sandbox-exec", "{v}");
    assert!(
        v["compile_cache_note"].as_str().unwrap().contains("server"),
        "{v}"
    );
    assert!(names(&cache_folder(&home).join("t")).is_empty());
    assert_eq!(count(&fx, "evidence.run"), 1);
    // The same command on the same content is found, not run again.
    let (code, v2) = run_check(&fx, &wc, &home, "X-9");
    assert_eq!(code, 1, "{v2}");
    assert_eq!(count(&fx, "evidence.run"), 1);
    assert_eq!(count(&fx, "evidence.counted"), 1);
}

#[test]
fn a_link_at_t_gives_a_run_without_the_cache_and_the_note() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let (fx, wc) = project(&program, "true\n");
    // The first run makes the folders; then `t/` becomes a link.
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    let folder = cache_folder(&home);
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::remove_dir(folder.join("t")).unwrap();
    symlink(&elsewhere, folder.join("t")).unwrap();
    fs::write(wc.join("src_new.txt"), "x\n").ok();
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["sandbox"], "sandbox-exec", "{v}");
    assert!(
        v["compile_cache_note"].as_str().unwrap().contains("link"),
        "{v}"
    );
    assert!(names(&elsewhere).is_empty());
}

#[test]
fn the_server_ends_and_its_folder_goes_after_a_failure_too() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let (fx, wc) = project(&program, "exit 7\n");
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["sandbox"], "sandbox-exec+compile-cache", "{v}");
    let folder = cache_folder(&home);
    assert!(folder.join("units/stop.rec").exists());
    assert!(names(&folder.join("t")).is_empty());
}

#[test]
fn a_server_is_ended_when_an_earlier_receipt_counts() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let (fx, wc) = project(&program, "true\n");
    let (code, _) = run(&fx, &wc, &home);
    assert_eq!(code, 0);
    let folder = cache_folder(&home);
    fs::remove_file(folder.join("units/stop.rec")).unwrap();
    let (code, _) = run_check(&fx, &wc, &home, "X-9");
    assert_eq!(code, 0);
    assert_eq!(count(&fx, "evidence.counted"), 1);
    assert!(
        folder.join("units/stop.rec").exists(),
        "the second server was stopped"
    );
    assert!(names(&folder.join("t")).is_empty());
}

#[test]
fn the_cache_note_is_empty_when_the_setting_is_off() {
    let (_t, home) = own_home();
    let sw = if sandbox_probe() {
        Switch::All
    } else {
        Switch::Off
    };
    let fx = Fx::tiny_project_with(sw);
    fs::write(fx.root.join("probe.sh"), "true\n").unwrap();
    write_check(&fx, "[\"sh\", \"probe.sh\"]");
    fx.commit_all("probe");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["compile_cache_note"], "");
    assert_ne!(v["sandbox"], "sandbox-exec+compile-cache");
    assert!(
        !home.join("Library").exists(),
        "no cache folder is made with the setting off"
    );
    let _ = cc::TOP_PARTS;
}

/// The real sccache, where it is installed: a build of a small crate under the server's profile and
/// through the client rule compiles, and the server counts the request.
#[test]
fn the_real_sccache_serves_a_build_under_the_server_profile() {
    let program = "/opt/homebrew/bin/sccache";
    if !sandbox_probe()
        || !Path::new(program).exists()
        || !Path::new("/opt/homebrew/bin/cargo").exists()
    {
        return;
    }
    let (_t, home) = own_home();
    let probe = "cd c && CARGO_TARGET_DIR=target cargo build 2>&1 && \"$RUSTC_WRAPPER\" --show-stats 2>&1 | grep -i 'compile requests'\n";
    let (fx, wc) = project(program, probe);
    fx.set_policy(
        "/usr/bin:/bin:/usr/sbin:/sbin",
        "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
    );
    fs::write(wc.join(".gitignore"), "c/target\nc/Cargo.lock\n").unwrap();
    fs::create_dir_all(wc.join("c/src")).unwrap();
    fs::write(
        wc.join("c/Cargo.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(wc.join("c/src/main.rs"), "fn main() {}\n").unwrap();
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["sandbox"], "sandbox-exec+compile-cache", "{v}");
    let t = v["output_tail"].as_str().unwrap();
    assert!(
        t.contains("Finished") && t.contains("ompile requests"),
        "{t}"
    );
    assert!(names(&cache_folder(&home).join("t")).is_empty());
}

#[test]
fn a_socket_path_over_100_bytes_gives_a_run_without_the_cache_and_the_note_through_the_engine() {
    if !sandbox_probe() {
        return;
    }
    // The system's own temporary folder is long enough that the socket path passes 100 bytes.
    let t = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(t.path()).unwrap();
    let program = fake_program(&home);
    let (fx, wc) = project(&program, "true\n");
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["sandbox"], "sandbox-exec", "{v}");
    assert!(
        v["compile_cache_note"]
            .as_str()
            .unwrap()
            .contains("100 bytes"),
        "{v}"
    );
    assert!(names(&cache_folder(&home).join("t")).is_empty());
}

#[test]
fn a_run_folder_a_killed_engine_left_behind_goes_at_the_next_run() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let (fx, wc) = project(&program, "true\n");
    let (code, v) = run(&fx, &wc, &home);
    assert_eq!(code, 0, "{v}");
    let runs = cache_folder(&home).join("t");
    let stale = runs.join("deadbeef");
    fs::create_dir_all(stale.join("tmp")).unwrap();
    let young = runs.join("cafebabe");
    fs::create_dir_all(&young).unwrap();
    let st = std::process::Command::new("touch")
        .args(["-t", "200001010000"])
        .arg(&stale)
        .status()
        .unwrap();
    assert!(st.success());
    let (code, v) = run_check(&fx, &wc, &home, "X-9");
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        names(&runs),
        ["cafebabe"],
        "the old folder went, the young one stays"
    );
}

/// A heavy check over its time limit: the server is ended and its folder removed, as after a pass and
/// a failure. A timing test (it holds a time limit), so the default suite leaves it out.
#[test]
fn timing_a_heavy_run_over_its_limit_ends_the_server_and_removes_its_folder() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let sw = Switch::All;
    let fx = Fx::tiny_project_with(sw);
    fx.set_policy(
        "compile_cache = \"\"",
        &format!("compile_cache = \"{program}\""),
    );
    fs::write(fx.root.join("probe.sh"), "sleep 600\n").unwrap();
    let put_replace = |kind: &str, text: &str| {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("r.toml");
        fs::write(&f, text).unwrap();
        let o = fx.bf(&[
            "record",
            "put",
            kind,
            "--file",
            f.to_str().unwrap(),
            "--replace",
        ]);
        assert_eq!(o.code, 0, "{}", o.all());
    };
    put_replace(
        "plan_settings",
        "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"s\"\nheavy_limit=1\n",
    );
    put_replace("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/hello.txt\"]\ncheckpoint=\"K-1\"\nestimate=2\nfull_suite=\"A light suite.\"\n");
    put_replace("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\ncommand=\"sh probe.sh\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\nheavy=true\n");
    fx.commit_all("heavy");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    let o = fx.bf_env(
        &wc,
        &[
            "evidence", "run", "X-1", "--unit", "U-1", "--json", "--", "sh", "probe.sh",
        ],
        "",
        &[
            ("BF_TEST_ACCOUNT_HOME", home.to_str().unwrap()),
            ("BF_HEAVY_LIMIT_SECONDS", "1"),
        ],
    );
    assert_eq!(o.code, 1, "{}", o.all());
    let v = o.json();
    assert_eq!(v["exit"], 124, "{v}");
    assert_eq!(v["sandbox"], "sandbox-exec+compile-cache", "{v}");
    let folder = cache_folder(&home);
    assert!(folder.join("units/stop.rec").exists());
    assert!(names(&folder.join("t")).is_empty());
}

#[test]
fn the_sweep_skips_a_folder_whose_lock_is_held_and_removes_one_whose_lock_is_free() {
    let (_t, home) = own_home();
    let paths = paths_in(&home);
    // A live run: its folder is old and its socket refuses, but its engine holds the lock.
    let live = server::make_run_dir(&paths).unwrap();
    let dead = server::make_run_dir(&paths).unwrap();
    for r in [&live, &dead] {
        let st = std::process::Command::new("touch")
            .args(["-t", "200001010000"])
            .arg(&r.dir)
            .status()
            .unwrap();
        assert!(st.success());
    }
    // The engine of `dead` is gone: its lock is released (the folder stays).
    let dead_dir = dead.dir.clone();
    drop(dead);
    assert!(dead_dir.exists());
    server::sweep_stale(&paths.runs);
    assert!(live.dir.exists(), "a held lock keeps the folder");
    assert!(!dead_dir.exists(), "a free lock lets it go");
    assert!(live.tmp.exists());
}

#[test]
fn the_sweep_does_not_hang_on_a_named_pipe_planted_as_the_lock() {
    let (_t, home) = own_home();
    let paths = paths_in(&home);
    let r = server::make_run_dir(&paths).unwrap();
    let dir = r.dir.clone();
    drop(r);
    let lock = dir.join(server::LOCK_NAME);
    fs::remove_file(&lock).unwrap();
    let st = std::process::Command::new("mkfifo")
        .arg(&lock)
        .status()
        .unwrap();
    assert!(st.success());
    let st = std::process::Command::new("touch")
        .args(["-t", "200001010000"])
        .arg(&dir)
        .status()
        .unwrap();
    assert!(st.success());
    let (tx, rx) = std::sync::mpsc::channel();
    let runs = paths.runs.clone();
    std::thread::spawn(move || {
        server::sweep_stale(&runs);
        let _ = tx.send(());
    });
    assert!(
        rx.recv_timeout(std::time::Duration::from_secs(20)).is_ok(),
        "the sweep hung on a pipe"
    );
    assert!(!dir.exists());
}
