//! Running programs with a clean environment, and finding rustfmt.

use crate::programs;
use crate::store::{s, Project};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Run a program with no shell, a clean environment, the folder, input on standard input and a
/// 60-second limit: its standard output, or the first line of its error.
pub(super) fn exec(
    prog: &str,
    path: &str,
    args: &[&str],
    cwd: &Path,
    input: &[u8],
) -> Result<Vec<u8>, String> {
    let mut c = Command::new(prog);
    c.args(args).current_dir(cwd).env_clear().env("PATH", path);
    for k in ["RUSTUP_HOME", "CARGO_HOME", "RUSTUP_TOOLCHAIN"] {
        if let Ok(v) = std::env::var(k) {
            c.env(k, v);
        }
    }
    crate::session::home::set_env(&mut c);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut ch = c.spawn().map_err(|e| e.to_string())?;
    let mut si = ch.stdin.take().unwrap();
    let data = input.to_vec();
    std::thread::spawn(move || si.write_all(&data));
    let (mut so, mut se) = (ch.stdout.take().unwrap(), ch.stderr.take().unwrap());
    let ro = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = so.read_to_end(&mut v);
        v
    });
    let re = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = se.read_to_end(&mut v);
        v
    });
    let t0 = Instant::now();
    let st = loop {
        match ch.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if t0.elapsed() < Duration::from_secs(60) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = ch.kill();
                let _ = ch.wait();
                return Err("it did not finish in 60 seconds".into());
            }
        }
    };
    let (out, err) = (ro.join().unwrap_or_default(), re.join().unwrap_or_default());
    if st.success() {
        return Ok(out);
    }
    Err(failure(st, &out, &err))
}

/// The first line of text that is not blank.
fn first_line(b: &[u8]) -> Option<String> {
    let t = String::from_utf8_lossy(b);
    let l = t.lines().find(|l| !l.trim().is_empty())?;
    Some(l.trim().to_string())
}

/// Why a program failed, in one sentence: the first line of its standard error, or when it wrote none,
/// how it ended and the first line of its standard output.
fn failure(st: ExitStatus, out: &[u8], err: &[u8]) -> String {
    if let Some(e) = first_line(err) {
        return e;
    }
    let how = match (st.code(), st.signal()) {
        (Some(c), _) => format!("it ended with exit code {c}"),
        (None, Some(s)) => format!("it was stopped by signal {s}"),
        _ => "it ended without an exit code".to_string(),
    };
    match first_line(out) {
        Some(o) => {
            format!("{how}, wrote nothing to standard error, and its standard output began `{o}`")
        }
        None => format!("{how} and wrote nothing to standard output or standard error"),
    }
}

/// rustfmt from the policy's check PATH: (path, hash, version), unless it is missing or lies inside
/// the project folder, the unit's repository or a working copy.
pub(super) fn locate_rustfmt(p: &Project, repo: &Path) -> Result<Value, String> {
    let l = programs::locate(p, "rustfmt")
        .ok_or("the formatter `rustfmt` is not on the policy's check PATH")?;
    let f = std::fs::canonicalize(s(&l, "path")).map_err(|e| e.to_string())?;
    for d in [p.root.clone(), repo.to_path_buf(), p.work_dir()] {
        let d = std::fs::canonicalize(&d).unwrap_or(d);
        if f.starts_with(&d) || Path::new(s(&l, "run")).starts_with(&d) {
            return Err("the formatter `rustfmt` lies inside the project folder or a working copy, which the engine does not run".into());
        }
    }
    let v = exec(
        s(&l, "run"),
        &p.policy.exec.path,
        &["--version"],
        &p.work_dir(),
        b"",
    )?;
    Ok(
        json!({"path": s(&l, "path"), "run": s(&l, "run"), "hash": s(&l, "hash"), "version": String::from_utf8_lossy(&v).trim()}),
    )
}
