//! The compile cache's rules that need no server (R-76, D-104, TH-20): the policy key, the wrapper
//! variables every check gets, who may use the cache, the folder's opening without following a link,
//! and the denial in every check's profile. Every test sets the account's home through the test seam to
//! a folder of its own, so none reads or writes the account's real cache folder.

mod common;
use bf::sandbox::compile_cache::{self as cc, Use};
use common::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

/// A folder of the test's own, canonical, to be the account's home.
fn own_home() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let h = fs::canonicalize(t.path()).unwrap();
    (t, h)
}

/// An executable file standing for the sccache program.
fn fake_program(dir: &Path) -> String {
    let f = dir.join("fake-sccache");
    fs::write(&f, "#!/bin/sh\necho ran >> \"$0.ran\"\nexec \"$@\"\n").unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
    f.to_string_lossy().to_string()
}

/// The project of `fx` with the setting on and a project id of 16 hex digits.
fn project_with(fx: &Fx, program: &str) -> bf::store::Project {
    let mut p = fx.project();
    p.policy.exec.compile_cache = program.to_string();
    p.config.project_id = "0123456789abcdef".to_string();
    p
}

fn units_copy(p: &bf::store::Project) -> PathBuf {
    p.work_dir().join("U-1")
}

fn tier(p: &bf::store::Project, home: &Path) -> Use {
    cc::tier_with(p, Some(home), true, "unit", Some("U-1"), &units_copy(p))
}

fn off_note(u: Use) -> String {
    match u {
        Use::Off(n) => {
            assert!(!n.is_empty(), "an Off carries its note");
            n
        }
        Use::Units(p) => panic!("expected Off, got {p:?}"),
    }
}

fn put_intent(fx: &Fx, space: &str, intent: &str) {
    fs::create_dir_all(
        fx.root
            .join("factory/spaces")
            .join(space)
            .join("intents")
            .join(intent),
    )
    .unwrap();
}

#[test]
fn the_template_holds_the_setting_empty_and_its_comment() {
    let fx = Fx::new_real();
    let text = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    assert!(text.contains("compile_cache = \"\""), "{text}");
    let comment = bf::strings::t("cli.policy.compile_cache.comment", &[]);
    assert!(!comment.is_empty());
    assert!(text.contains(&format!("# {comment}")), "{text}");
    assert!(fx.project().policy.exec.compile_cache.is_empty());
}

#[test]
fn a_policy_without_the_key_reads_it_as_empty() {
    let e: bf::store::ExecPol = toml::from_str("sandbox = true\nnetwork = false\n").unwrap();
    assert_eq!(e.compile_cache, "");
    assert_eq!(bf::store::ExecPol::default().compile_cache, "");
}

/// The probe: prints each variable, `unset` for one that is not there and nothing for an empty one.
fn write_probe(dir: &Path) {
    fs::write(
        dir.join("probe.sh"),
        "echo \"RW=[${RUSTC_WRAPPER-unset}] RWW=[${RUSTC_WORKSPACE_WRAPPER-unset}] CB=[${CARGO_BUILD_RUSTC_WRAPPER-unset}] CBW=[${CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER-unset}] SD=[${SCCACHE_DIR-unset}] SX=[${SCCACHE_X-unset}]\"\n",
    )
    .unwrap();
}

fn replace_check(fx: &Fx, id: &str, command: &str, scope: &str, unit: &str) {
    let proves = if scope == "checkpoint" {
        "[\"R-1\",\"R-2\",\"C-1\"]"
    } else {
        "[\"R-1\",\"C-1\",\"H-1\"]"
    };
    let unit_line = if unit.is_empty() {
        String::new()
    } else {
        format!("unit=\"{unit}\"\n")
    };
    let text = format!(
        "id=\"{id}\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\ncommand={command}\nproves={proves}\nscope=\"{scope}\"\n{unit_line}"
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

const CALLER: [(&str, &str); 6] = [
    ("RUSTC_WRAPPER", "/caller/wrapper"),
    ("RUSTC_WORKSPACE_WRAPPER", "/caller/workspace-wrapper"),
    ("CARGO_BUILD_RUSTC_WRAPPER", "/caller/cargo-wrapper"),
    ("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "/caller/cargo-ws"),
    ("SCCACHE_DIR", "/caller/dir"),
    ("SCCACHE_X", "x"),
];

fn expected_probe() -> &'static str {
    "RW=[] RWW=[] CB=[unset] CBW=[unset] SD=[unset] SX=[unset]"
}

fn probe_run(fx: &Fx, dir: &Path, check: &str, unit: Option<&str>, home: &Path) -> serde_json::Value {
    let mut args = vec!["evidence", "run", check];
    if let Some(u) = unit {
        args.extend(["--unit", u]);
    }
    args.extend(["--json", "--", "sh", "probe.sh"]);
    let mut env: Vec<(&str, &str)> = CALLER.to_vec();
    env.push(("BF_TEST_ACCOUNT_HOME", home.to_str().unwrap()));
    let o = fx.bf_env(dir, &args, "", &env);
    assert_eq!(o.code, 0, "{}", o.all());
    o.json()
}

fn env_names_policy() -> String {
    let names: Vec<String> = CALLER.iter().map(|(k, _)| format!("\"{k}\"")).collect();
    format!("env = [{}]", names.join(", "))
}

/// A project with the probe as the command of X-1 (a unit check of U-1) and X-3 (a checkpoint check),
/// then the test run started, so the records are in the plan and U-1's copy holds the probe.
fn probe_project(sandboxed: bool, before_start: impl FnOnce(&Fx)) -> Fx {
    let fx = if sandboxed {
        Fx::tiny_project_sandboxed()
    } else {
        Fx::tiny_project()
    };
    before_start(&fx);
    write_probe(&fx.root);
    replace_check(&fx, "X-3", "[\"sh\", \"probe.sh\"]", "checkpoint", "");
    replace_check(&fx, "X-1", "[\"sh\", \"probe.sh\"]", "unit", "U-1");
    fx.commit_all("probe");
    fx.start_test_run();
    fx
}

fn start_unit(fx: &Fx) -> PathBuf {
    PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

#[test]
fn a_callers_wrapper_never_reaches_a_check_and_both_wrapper_variables_are_empty() {
    for on in [false, true] {
        let (_t, home) = own_home();
        let program = fake_program(&home);
        let fx = probe_project(false, |fx| {
            fx.set_policy("env = []", &env_names_policy());
            if on {
                fx.set_policy(
                    "compile_cache = \"\"",
                    &format!("compile_cache = \"{program}\""),
                );
            }
        });
        // A check of scope checkpoint, in the main copy.
        let v = probe_run(&fx, &fx.root, "X-3", None, &home);
        assert!(
            v["output_tail"].as_str().unwrap().contains(expected_probe()),
            "setting on = {on}: {v}"
        );
        // A check of scope unit, in its unit's own copy.
        let wc = start_unit(&fx);
        let v = probe_run(&fx, &wc, "X-1", Some("U-1"), &home);
        assert!(
            v["output_tail"].as_str().unwrap().contains(expected_probe()),
            "setting on = {on}: {v}"
        );
    }
}

#[test]
fn a_unit_check_with_a_link_at_the_cache_folder_runs_without_the_cache_and_says_why() {
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let fx = probe_project(false, |fx| {
        fx.set_policy(
            "compile_cache = \"\"",
            &format!("compile_cache = \"{program}\""),
        );
    });
    fs::create_dir_all(home.join("Library/Caches")).unwrap();
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    symlink(&elsewhere, home.join("Library/Caches/blackfactory-cc")).unwrap();
    let wc = start_unit(&fx);
    let v = probe_run(&fx, &wc, "X-1", Some("U-1"), &home);
    assert!(
        v["output_tail"].as_str().unwrap().contains(expected_probe()),
        "{v}"
    );
    let note = v["compile_cache_note"].as_str().unwrap();
    assert!(note.contains("is a link"), "{v}");
    // The link's target was not written.
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn the_profile_names_the_literal_path_and_only_it() {
    let (_t, home) = own_home();
    let fx = Fx::new();
    let p = project_with(&fx, "/usr/bin/true");
    let top = cc::top_in(&home);
    let denial = cc::profile_denial_in(&p, Some(&home));
    assert_eq!(
        denial,
        format!(
            "(deny file-read* file-write* (subpath \"{}\"))\n",
            top.display()
        )
    );
    // A link at the cache folder changes nothing in the denial: it still names the literal path.
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(home.join("Library/Caches")).unwrap();
    symlink(&elsewhere, &top).unwrap();
    assert_eq!(cc::profile_denial_in(&p, Some(&home)), denial);
    assert!(!denial.contains("elsewhere"));
    // With the setting empty there is no denial.
    let p = project_with(&fx, "");
    assert_eq!(cc::profile_denial_in(&p, Some(&home)), "");
}

#[test]
fn a_checkpoint_run_reads_what_a_link_at_the_cache_folder_points_at() {
    if !sandbox_probe() {
        return;
    }
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("f"), "visible\n").unwrap();
    fs::create_dir_all(home.join("Library/Caches")).unwrap();
    symlink(&elsewhere, home.join("Library/Caches/blackfactory-cc")).unwrap();
    let fx = probe_project(true, |fx| {
        fx.set_policy(
            "compile_cache = \"\"",
            &format!("compile_cache = \"{program}\""),
        );
    });
    fs::write(
        fx.root.join("probe.sh"),
        format!("cat {}/f\n", elsewhere.display()),
    )
    .unwrap();
    let o = fx.bf_env(
        &fx.root,
        &["evidence", "run", "X-3", "--json", "--", "sh", "probe.sh"],
        "",
        &[("BF_TEST_ACCOUNT_HOME", home.to_str().unwrap())],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(o.json()["output_tail"].as_str().unwrap().contains("visible"));
}

/// A project whose checkpoint check X-3 runs `probe.sh`, with its units built and landed, so the
/// checkpoint's copy `K-1` exists. `before_start` runs before the records are committed and the test
/// run starts, and may write any file (they are committed with the rest).
fn checkpoint_project(sw: Switch, before_start: impl FnOnce(&Fx)) -> Fx {
    let fx = Fx::tiny_project_with(sw);
    before_start(&fx);
    replace_check(&fx, "X-3", "[\"sh\", \"probe.sh\"]", "checkpoint", "");
    fx.commit_all("probe");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    assert!(fx.work("K-1").is_dir());
    fx
}

#[test]
fn a_checkpoint_copy_whose_cargo_config_names_a_wrapper_never_runs_it() {
    for network in [false, true] {
        let (_t, home) = own_home();
        let program = fake_program(&home);
        let units = home.join("Library/Caches/blackfactory-cc/0123456789abcdef/aaaaaaaa/units");
        fs::create_dir_all(&units).unwrap();
        fs::write(units.join("entry"), "planted\n").unwrap();
        let cargo = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|d| d.join("cargo"))
            .find(|c| c.is_file())
            .expect("cargo is on the test's PATH");
        let target = home.join("tiny-target");
        let sw = if sandbox_probe() {
            Switch::All
        } else {
            Switch::Off
        };
        let fx = checkpoint_project(sw, |fx| {
            fx.set_policy(
                "compile_cache = \"\"",
                &format!("compile_cache = \"{program}\""),
            );
            if network {
                fx.set_policy("network = false", "network = true");
            }
            // A tiny crate whose cargo config names the fake program and `units/`.
            let t = fx.root.join("tiny");
            fs::create_dir_all(t.join("src")).unwrap();
            fs::create_dir_all(t.join(".cargo")).unwrap();
            fs::write(
                t.join("Cargo.toml"),
                "[package]\nname = \"tiny\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n",
            )
            .unwrap();
            fs::write(t.join(".gitignore"), "Cargo.lock\n").unwrap();
            fs::write(t.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
            fs::write(
                t.join(".cargo/config.toml"),
                format!(
                    "[build]\nrustc-wrapper = \"{program}\"\n[env]\nSCCACHE_DIR = \"{}\"\n",
                    units.display()
                ),
            )
            .unwrap();
            let script = format!(
                "export PATH={dir}:$PATH; cd tiny && {cargo} build --offline --quiet --target-dir {t} >{log} 2>&1; echo built=$?\n\
                 test -e {program}.ran && echo fake=ran || echo fake=none\n\
                 cat {u}/entry >/dev/null 2>&1; echo read=$?\n\
                 echo planted > {u}/new 2>/dev/null; echo write=$?\n",
                dir = cargo.parent().unwrap().display(),
                cargo = cargo.display(),
                log = home.join("cargo.log").display(),
                t = target.display(),
                u = units.display()
            );
            fs::write(fx.root.join("probe.sh"), script).unwrap();
        });
        let mut env = vec![("BF_TEST_ACCOUNT_HOME", home.to_str().unwrap().to_string())];
        let real_home = std::env::var("HOME").unwrap_or_default();
        for (k, d) in [("RUSTUP_HOME", ".rustup"), ("CARGO_HOME", ".cargo")] {
            let v = std::env::var(k).unwrap_or_else(|_| format!("{real_home}/{d}"));
            env.push((k, v));
        }
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // The run is in the checkpoint's own copy.
        let o = fx.bf_env(
            &fx.work("K-1"),
            &["evidence", "run", "X-3", "--json", "--", "sh", "probe.sh"],
            "",
            &env,
        );
        assert_eq!(o.code, 0, "network {network}: {}", o.all());
        let v = o.json();
        let tail = v["output_tail"].as_str().unwrap();
        assert!(
            tail.contains("built=0"),
            "network {network}: {tail} {}",
            fs::read_to_string(home.join("cargo.log")).unwrap_or_default()
        );
        assert!(tail.contains("fake=none"), "network {network}: {tail}");
        assert!(!Path::new(&format!("{program}.ran")).exists());
        if v["sandbox"] == "sandbox-exec" {
            assert!(tail.contains("read=1"), "network {network}: {tail}");
            assert!(tail.contains("write=1"), "network {network}: {tail}");
            assert!(!units.join("new").exists());
        }
    }
}

/// A scan run as a check (`bf evidence run <check> -- bf scan secrets`) in the checkpoint's copy, with a
/// stand-in tool that reports what it sees.
#[test]
fn a_scan_run_as_a_check_gets_neither_a_callers_wrapper_nor_the_cache_folder() {
    let (_t, home) = own_home();
    let program = fake_program(&home);
    let cache = home.join("Library/Caches/blackfactory-cc/x");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("entry"), "planted\n").unwrap();
    let report = home.join("scan-report.txt");
    let sw = if sandbox_probe() {
        Switch::All
    } else {
        Switch::Off
    };
    let fx = Fx::new_with(sw);
    let bin = fx.root.parent().unwrap().join("standins");
    fs::create_dir_all(&bin).unwrap();
    fx.set_policy(
        "/usr/bin:/bin:/usr/sbin:/sbin",
        &format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    fx.set_policy("env = []", &env_names_policy());
    fx.set_policy(
        "compile_cache = \"\"",
        &format!("compile_cache = \"{program}\""),
    );
    let tool = bin.join("gitleaks");
    fs::write(
        &tool,
        format!(
            "#!/bin/sh\necho \"RW=[${{RUSTC_WRAPPER-unset}}] RWW=[${{RUSTC_WORKSPACE_WRAPPER-unset}}] CB=[${{CARGO_BUILD_RUSTC_WRAPPER-unset}}] SD=[${{SCCACHE_DIR-unset}}]\" > {r}\ncat {c}/entry >/dev/null 2>&1; echo read=$? >> {r}\necho '[]'\nexit 0\n",
            r = report.display(),
            c = cache.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    fx.tiny();
    let t = "id=\"X-70\"\nkind=\"check\"\ntitle=\"secret-scan\"\nhow=\"command\"\ncommand=\"bf scan secrets\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"secret-scan\"\n";
    assert_eq!(fx.put("check", t).code, 0);
    let k = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\",\"X-70\"]\n";
    let f = fx.root.join("k1.toml");
    fs::write(&f, k).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "checkpoint",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("scan");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    let mut env: Vec<(&str, &str)> = CALLER.to_vec();
    env.push(("BF_TEST_ACCOUNT_HOME", home.to_str().unwrap()));
    let o = fx.bf_env(
        &fx.work("K-1"),
        &["evidence", "run", "X-70", "--", "bf", "scan", "secrets"],
        "",
        &env,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let seen = fs::read_to_string(&report).unwrap();
    assert!(
        seen.contains("RW=[] RWW=[] CB=[unset] SD=[unset]"),
        "{seen}"
    );
    if sandbox_probe() {
        assert!(seen.contains("read=1"), "{seen}");
    }
}

/// A part owned by another user needs a test that can make one: only a process with the right to
/// change an owner can, so the test is skipped for any other user.
#[test]
fn a_part_owned_by_another_user_gives_off_where_the_test_can_make_one() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipped: changing a folder's owner needs the root user");
        return;
    }
    let (_t, home) = own_home();
    let fx = Fx::new();
    let program = fake_program(&home);
    let p = project_with(&fx, &program);
    let top = cc::top_in(&home);
    fs::create_dir_all(&top).unwrap();
    std::os::unix::fs::chown(&top, Some(12345), None).unwrap();
    assert!(off_note(tier(&p, &home)).contains("not owned"));
}

#[test]
fn only_a_unit_check_in_its_own_copy_may_use_the_cache() {
    let (_t, home) = own_home();
    let fx = Fx::new();
    let program = fake_program(&home);
    let p = project_with(&fx, &program);
    let own = units_copy(&p);
    let paths = match tier(&p, &home) {
        Use::Units(x) => x,
        other => panic!("{other:?}"),
    };
    assert!(paths.units.is_dir() && paths.runs.is_dir());
    assert_eq!(
        paths.units,
        cc::top_in(&home)
            .join("0123456789abcdef")
            .join(cc::intent_hash("default", "first"))
            .join("units")
    );
    let with = |scope: &str, unit: Option<&str>, dir: &Path| {
        cc::tier_with(&p, Some(&home), true, scope, unit, dir)
    };
    let wd = p.work_dir();
    for (what, u) in [
        ("checkpoint copy", with("unit", Some("U-1"), &wd.join("K-1"))),
        ("wave-check copy", with("unit", Some("U-1"), &wd.join("W-1"))),
        ("review copy", with("unit", Some("U-1"), &wd.join("K-1-review"))),
        ("root", with("unit", Some("U-1"), &p.root)),
        ("checkpoint scope", with("checkpoint", Some("U-1"), &own)),
        ("another unit's copy", with("unit", Some("U-1"), &wd.join("U-2"))),
        ("no unit", with("unit", None, &own)),
    ] {
        assert!(matches!(u, Use::Off(_)), "{what}");
    }
    // Without the sandbox there is no cache.
    assert!(matches!(
        cc::tier_with(&p, Some(&home), false, "unit", Some("U-1"), &own),
        Use::Off(_)
    ));
}

#[test]
fn a_cache_that_cannot_be_used_gives_off_with_its_note() {
    let (_t, home) = own_home();
    let fx = Fx::new();
    let program = fake_program(&home);
    let top = cc::top_in(&home);
    let hash = cc::intent_hash("default", "first");
    let id = "0123456789abcdef";
    let part = |last: &str| top.join(id).join(&hash).join(last);

    // A link at the cache folder, at `units/` and at `t/`.
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    for (name, link) in [
        ("folder", top.clone()),
        ("units", part("units")),
        ("t", part("t")),
    ] {
        let (_t2, home2) = own_home();
        let top2 = cc::top_in(&home2);
        let mapped = PathBuf::from(
            link.to_string_lossy()
                .replace(home.to_str().unwrap(), home2.to_str().unwrap()),
        );
        if name == "folder" {
            fs::create_dir_all(top2.parent().unwrap()).unwrap();
        } else {
            fs::create_dir_all(mapped.parent().unwrap()).unwrap();
            if name == "t" {
                fs::create_dir_all(top2.join(id).join(&hash).join("units")).unwrap();
            }
        }
        symlink(&elsewhere, &mapped).unwrap();
        let p = project_with(&fx, &program);
        let note = off_note(tier(&p, &home2));
        assert!(note.contains("is a link"), "{name}: {note}");
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0, "{name}");
    }

    // A part that is a file.
    let (_t2, home2) = own_home();
    fs::create_dir_all(home2.join("Library")).unwrap();
    fs::write(home2.join("Library/Caches"), "file").unwrap();
    let p = project_with(&fx, &program);
    assert!(off_note(tier(&p, &home2)).contains("is not a folder"));

    // A project id not of 16 hex digits.
    let mut p = project_with(&fx, &program);
    for bad in ["", "abc", "0123456789abcdeg", "0123456789abcdef0"] {
        p.config.project_id = bad.to_string();
        assert!(off_note(tier(&p, &home)).contains("16 hex digits"), "{bad}");
    }

    // A relative or missing program.
    for bad in ["sccache", "", "/no/such/program"] {
        let p = project_with(&fx, bad);
        assert!(off_note(tier(&p, &home)).contains("executable"), "{bad}");
    }

    // No home.
    let p = project_with(&fx, &program);
    let u = cc::tier_with(&p, None, true, "unit", Some("U-1"), &units_copy(&p));
    assert!(off_note(u).contains("home"));
}

#[test]
fn the_cache_is_on_for_exactly_one_intent_and_its_folder_is_that_intents() {
    let (_t, home) = own_home();
    let program = fake_program(&home);

    // One flat intent: the project's `factory/records/` folder, which `Fx::new` makes.
    let fx = Fx::new();
    assert!(fx.root.join("factory/records").is_dir());
    let p = project_with(&fx, &program);
    let flat = match tier(&p, &home) {
        Use::Units(x) => x,
        other => panic!("{other:?}"),
    };
    assert!(flat
        .units
        .to_string_lossy()
        .contains(&cc::intent_hash("default", "first")));

    // Two intents: a records folder and one under spaces.
    put_intent(&fx, "default", "second");
    let p = project_with(&fx, &program);
    let note = off_note(tier(&p, &home));
    assert!(note.contains("2 intents"), "{note}");

    // Two folders under spaces, no records folder.
    let fx = Fx::new();
    fs::remove_dir_all(fx.root.join("factory/records")).unwrap();
    put_intent(&fx, "default", "one");
    put_intent(&fx, "other", "two");
    let p = project_with(&fx, &program);
    assert!(off_note(tier(&p, &home)).contains("2 intents"));

    // No intent.
    let fx = Fx::new();
    fs::remove_dir_all(fx.root.join("factory/records")).unwrap();
    let p = project_with(&fx, &program);
    assert!(off_note(tier(&p, &home)).contains("0 intents"));
    assert_eq!(cc::one_intent(&p), None);

    // One folder under `factory/spaces/default/intents/` beside the workspace's log under `factory/spaces/`.
    let fx = Fx::new();
    fs::remove_dir_all(fx.root.join("factory/records")).unwrap();
    put_intent(&fx, "default", "alpha");
    fs::write(fx.root.join("factory/spaces/log.jsonl"), "").unwrap();
    fs::create_dir_all(fx.root.join("factory/spaces/notes")).unwrap();
    let p1 = project_with(&fx, &program);
    assert_eq!(
        cc::one_intent(&p1),
        Some(("default".to_string(), "alpha".to_string()))
    );
    let a = match tier(&p1, &home) {
        Use::Units(x) => x,
        other => panic!("{other:?}"),
    };
    assert!(a
        .units
        .to_string_lossy()
        .contains(&cc::intent_hash("default", "alpha")));

    // Another project with a different intent gets a different folder.
    let fx2 = Fx::new();
    fs::remove_dir_all(fx2.root.join("factory/records")).unwrap();
    put_intent(&fx2, "default", "beta");
    let p2 = project_with(&fx2, &program);
    let b = match tier(&p2, &home) {
        Use::Units(x) => x,
        other => panic!("{other:?}"),
    };
    assert_ne!(a.units, b.units);
}

#[test]
fn the_intent_hash_is_the_first_eight_digits_of_the_sha256_of_space_and_intent() {
    // sha256("default/first")
    use sha2::{Digest, Sha256};
    let full: String = Sha256::digest(b"default/first")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(cc::intent_hash("default", "first"), full[..8]);
}

#[test]
fn a_part_is_opened_without_following_a_link_and_made_when_missing() {
    let (_t, home) = own_home();
    let made = cc::open_folder(&home, &["a", "b", "c"]).unwrap();
    assert_eq!(made, home.join("a/b/c"));
    assert!(made.is_dir());
    // Again: the same answer.
    assert_eq!(cc::open_folder(&home, &["a", "b", "c"]).unwrap(), made);
    // Names that step out are refused.
    assert!(cc::open_folder(&home, &["a", ".."]).is_err());
    assert!(cc::open_folder(&home, &["a/b"]).is_err());
    // A link in the middle is refused and its target is untouched.
    let elsewhere = home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    symlink(&elsewhere, home.join("a/link")).unwrap();
    assert!(cc::open_folder(&home, &["a", "link", "x"]).is_err());
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn builder_paths_and_doctor_facts_follow_the_same_rules() {
    let (_t, home) = own_home();
    let fx = Fx::new();
    let program = fake_program(&home);
    let p = project_with(&fx, &program);
    let b = cc::builder_paths_in(&p, Some(&home), "U-7").unwrap();
    assert!(b.socket.ends_with("s/U-7.sock"));
    assert!(b.units.ends_with("units"));
    let f = cc::doctor_facts_in(&p, Some(&home));
    assert!(f.state.is_ok());
    assert_eq!(f.program, program);
    assert!(f.sockets.is_empty() && f.servers.is_empty());
    let off = project_with(&fx, "");
    assert!(cc::builder_paths_in(&off, Some(&home), "U-7").is_err());
    assert_eq!(cc::doctor_facts_in(&off, Some(&home)).program, "");
}
