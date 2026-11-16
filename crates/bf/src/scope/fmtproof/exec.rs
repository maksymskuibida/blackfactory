//! Running programs with a clean environment, and finding rustfmt.

use crate::programs;
use crate::store::{s, Project};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
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
    for k in ["HOME", "RUSTUP_HOME", "CARGO_HOME", "RUSTUP_TOOLCHAIN"] {
        if let Ok(v) = std::env::var(k) {
            c.env(k, v);
        }
    }
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
    let e = String::from_utf8_lossy(&err);
    Err(e
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("no message")
        .trim()
        .to_string())
}

/// rustfmt from the policy's check PATH: (path, hash, version), unless it is missing or lies inside
/// the project folder, the unit's repository or a working copy.
pub(super) fn locate_rustfmt(p: &Project, repo: &Path) -> Result<Value, String> {
    let l = programs::locate(p, "rustfmt")
        .ok_or("the formatter `rustfmt` is not on the policy's check PATH")?;
    let f = std::fs::canonicalize(s(&l, "path")).map_err(|e| e.to_string())?;
    for d in [p.root.clone(), repo.to_path_buf(), p.work_dir()] {
        if f.starts_with(std::fs::canonicalize(&d).unwrap_or(d)) {
            return Err("the formatter `rustfmt` lies inside the project folder or a working copy, which the engine does not run".into());
        }
    }
    let v = exec(
        s(&l, "path"),
        &p.policy.exec.path,
        &["--version"],
        &p.work_dir(),
        b"",
    )?;
    Ok(
        json!({"path": s(&l, "path"), "hash": s(&l, "hash"), "version": String::from_utf8_lossy(&v).trim()}),
    )
}
