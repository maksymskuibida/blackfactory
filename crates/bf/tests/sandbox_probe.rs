//! U-116 (E-1): the engine asks whether the sandbox works only at the first git call or check that
//! needs it, not at every load of the policy. A process that runs no git starts no probe, one that runs
//! git starts exactly one, and git still runs under the sandbox when the policy asks for it. The probes
//! are counted through the test build's `BF_TEST_PROBE_LOG` (C-3).

mod common;
use common::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A fresh, empty probe log beside the fixture.
fn probe_log(fx: &Fx, name: &str) -> PathBuf {
    let log = fx.root.parent().unwrap().join(format!("{name}.probes"));
    fs::write(&log, "").unwrap();
    log
}

fn probes(log: &Path) -> usize {
    fs::read_to_string(log).unwrap_or_default().lines().count()
}

/// Run `bf` in the fixture with the probe log set, and give its output and the probes it started.
fn run_counted(fx: &Fx, args: &[&str], name: &str) -> (Out, usize) {
    let log = probe_log(fx, name);
    let o = fx.bf_env(
        &fx.root,
        args,
        "",
        &[("BF_TEST_PROBE_LOG", &log.to_string_lossy())],
    );
    (o, probes(&log))
}

#[test]
fn record_list_starts_no_probe() {
    let fx = Fx::new();
    fx.tiny();
    let (o, n) = run_counted(&fx, &["record", "list", "--json"], "list");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(n, 0, "`bf record list` started the sandbox probe");
}

#[test]
fn checks_propose_starts_no_probe() {
    let fx = Fx::with_checks();
    let (o, n) = run_counted(&fx, &["checks", "propose", "--scope", "default"], "propose");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(n, 0, "`bf checks propose` started the sandbox probe");
}

#[test]
fn a_command_that_runs_git_starts_exactly_one_probe() {
    let fx = Fx::new();
    let (o, n) = run_counted(&fx, &["status", "--json"], "status");
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(o.json()["main"]["tip"].as_str().unwrap().len() >= 40);
    assert_eq!(n, 1, "`bf status` runs git several times and probes once");
}

#[test]
fn a_command_that_runs_git_with_the_sandbox_off_starts_no_probe() {
    let fx = Fx::new();
    fx.set_policy("sandbox = true", "sandbox = false");
    let (o, n) = run_counted(&fx, &["status", "--json"], "status-off");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(n, 0, "the sandbox switched off is never asked about");
}

/// The sandbox line of `bf doctor`'s text answer.
fn sandbox_line(text: &str) -> String {
    text.lines()
        .find(|l| l.contains("sandbox"))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn doctor_prints_the_same_with_the_probe_taken_or_not() {
    let fx = Fx::new();
    // Not yet taken: the doctor's process asks the sandbox first at its own sandbox line.
    let (first, n) = run_counted(&fx, &["doctor"], "doctor");
    assert_eq!(n, 1, "`bf doctor` asks the sandbox once");
    // Already taken: the answer this test process got from the same probe gives the same line.
    let here = bf::sandbox::available();
    let line = sandbox_line(&first.all());
    assert!(!line.is_empty(), "{}", first.all());
    assert_eq!(
        line.contains("WITHOUT"),
        !here,
        "the doctor's sandbox line disagrees with the probe: {line}"
    );
    let (second, _) = run_counted(&fx, &["doctor"], "doctor-again");
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.code, second.code);
    let json = fx.bf(&["doctor", "--json"]).json();
    let note = json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "sandbox")
        .unwrap()["note"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(line.contains(&note), "{line}\n{note}");
}

/// The real git of the fixture's policy, as the policy names it.
fn real_git(fx: &Fx) -> String {
    let pol = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    let line = pol.lines().find(|l| l.starts_with("git = ")).unwrap();
    line.split('"').nth(1).unwrap().to_string()
}

/// The policy names as its git a script that tries to write into the state folder, logs whether the
/// write was denied, and runs the real git. Gives the log.
fn writing_git(fx: &Fx) -> PathBuf {
    let dir = fx.root.parent().unwrap().join("gitwrite");
    fs::create_dir_all(&dir).unwrap();
    let log = dir.join("writes.log");
    let target = fx.state_dir().join("written-by-git");
    let script = dir.join("git");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nif ( : > '{t}' ) 2>/dev/null; then echo allowed >> '{l}'; rm -f '{t}'; else echo denied >> '{l}'; fi\nexec '{g}' \"$@\"\n",
            t = target.display(),
            l = log.display(),
            g = real_git(fx)
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let real = real_git(fx);
    fx.set_policy(
        &format!("git = \"{real}\""),
        &format!("git = \"{}\"", script.display()),
    );
    fs::write(&log, "").unwrap();
    log
}

fn writes(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| l.to_string())
        .collect()
}

/// Whether this test runs as an engine check under the engine's own sandbox: the engine marks a check
/// with `BF_EVIDENCE=1`, and `sandbox-exec` is there but cannot start a second sandbox inside the first.
fn inside_the_engines_sandbox() -> bool {
    std::env::var("BF_EVIDENCE").as_deref() == Ok("1")
        && Path::new(bf::sandbox::SANDBOX_EXEC).exists()
        && !std::process::Command::new(bf::sandbox::SANDBOX_EXEC)
            .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

#[test]
fn git_runs_under_the_sandbox_when_the_policy_asks() {
    if !cfg!(target_os = "macos") {
        eprintln!("not macOS: the engine has no sandbox here");
        return;
    }
    if !bf::sandbox::available() {
        // On macOS the probe fails only inside the engine's own sandbox (a check run of X-205), where a
        // second sandbox cannot start. Anywhere else a failed probe is a failure, never a silent pass.
        assert!(
            inside_the_engines_sandbox(),
            "sandbox-exec does not work on this Mac outside the engine's sandbox: the denied write was not checked"
        );
        // Inside the engine's sandbox the engine's own git runs unsandboxed, and says so. The denied
        // write is proven only by a run outside it, and the receipt tells which: a run of X-205 whose
        // receipt says `sandbox = "sandbox-exec"` took this branch and did not check the denied write.
        let fx = Fx::new();
        let doctor = fx.must(&["doctor"]).all();
        assert!(sandbox_line(&doctor).contains("WITHOUT"), "{doctor}");
        eprintln!(
            "NOT CHECKED: the denied write cannot be shown inside the engine's sandbox; run `cargo nextest run --package=bf sandbox_probe::` outside it"
        );
        return;
    }
    let fx = Fx::new();
    let log = writing_git(&fx);
    let o = fx.must(&["status", "--json"]);
    assert!(o.json()["main"]["tip"].as_str().unwrap().len() >= 40);
    let w = writes(&log);
    assert!(!w.is_empty(), "the engine ran no git");
    assert!(
        w.iter().all(|l| l == "denied"),
        "a git call of the engine wrote into the state folder: {w:?}"
    );
}

#[test]
fn git_runs_outside_the_sandbox_when_the_policy_turns_it_off() {
    let fx = Fx::new();
    let log = writing_git(&fx);
    fx.set_policy("sandbox = true", "sandbox = false");
    fs::write(&log, "").unwrap();
    let o = fx.must(&["status", "--json"]);
    assert!(o.json()["main"]["tip"].as_str().unwrap().len() >= 40);
    let w = writes(&log);
    assert!(!w.is_empty(), "the engine ran no git");
    assert!(w.iter().all(|l| l == "allowed"), "{w:?}");
}
