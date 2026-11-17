//! The programs checks start. When the plan gate is approved the engine records, for each command
//! check, where its program is found (on the policy's check PATH) and a hash of that file. Before a
//! check runs, the program is looked up again. A program that moved or whose file changed does not stop
//! the check (an upgrade is ordinary); the change is recorded in the receipt and printed, and the new
//! state becomes the recorded one. (Only the first word of the command is covered.)

use crate::refusal::R;
use crate::store::{s, Project};
use crate::util;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn path_file(p: &Project) -> PathBuf {
    p.state_dir().join("programs.json")
}

fn read(p: &Project) -> Value {
    fs::read_to_string(path_file(p))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

/// Where a program word is found, as it is named on the check PATH (a link is not followed): a path as
/// given, or the first match. A program that is a link may behave by the name it is started under (the
/// proxies of a rustup install are links to `rustup`), so this is the path to start it by.
fn found(p: &Project, word: &str) -> Option<PathBuf> {
    if word.contains('/') {
        let w = PathBuf::from(word);
        let whole = fs::canonicalize(w.parent()?).ok()?.join(w.file_name()?);
        return Some(whole).filter(|c| c.is_file());
    }
    let path = if p.policy.exec.path.is_empty() {
        "/usr/bin:/bin:/usr/sbin:/sbin"
    } else {
        &p.policy.exec.path
    };
    path.split(':')
        .filter(|d| !d.is_empty())
        .filter_map(|d| Some(fs::canonicalize(d).ok()?.join(word)))
        .find(|c| c.is_file())
}

/// The file `found` leads to, with every link followed.
fn resolve(p: &Project, word: &str) -> Option<PathBuf> {
    found(p, word).and_then(|c| fs::canonicalize(c).ok())
}

fn stat(path: &std::path::Path) -> Option<(u64, u64)> {
    let m = fs::metadata(path).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some((m.len(), t))
}

fn entry(p: &Project, word: &str) -> Option<Value> {
    let run = found(p, word)?;
    let path = fs::canonicalize(&run).ok()?;
    let (size, mtime) = stat(&path)?;
    let hash = util::sha256_file(&path).ok()?;
    Some(
        json!({"word": word, "path": path.to_string_lossy(), "run": run.to_string_lossy(), "size": size, "mtime": mtime, "hash": hash}),
    )
}

/// Where a program is found on the check PATH and a hash of that file: `{word, path, run, size, mtime,
/// hash}` (`path` has every link followed, `run` is the name to start the program by), or nothing when
/// the program is not there. The scan runner uses it to refuse a missing tool and to record where the
/// tool it started was found.
pub fn locate(p: &Project, word: &str) -> Option<Value> {
    entry(p, word)
}

fn first_word(check: &Value) -> Option<String> {
    match check.get("command") {
        Some(Value::String(c)) => c.split_whitespace().next().map(|w| w.to_string()),
        Some(Value::Array(a)) => a.first().and_then(|w| w.as_str()).map(|w| w.to_string()),
        _ => None,
    }
}

/// Record the program of every command check. Called when the plan gate is approved and when the
/// human accepts the programs as they are now.
pub fn record_all(p: &Project) -> R<()> {
    let mut out = json!({});
    for c in p.list_records(crate::schema::kind_by_name("check"))? {
        if s(&c, "how") != "command" {
            continue;
        }
        if let Some(e) = first_word(&c).and_then(|w| entry(p, &w)) {
            out[s(&c, "id")] = e;
        }
    }
    util::atomic_write(&path_file(p), serde_json::to_string(&out)?.as_bytes())?;
    Ok(())
}

/// Before a check runs: the program it starts is compared with the one recorded at the plan gate.
/// Returns a note when it changed (moved, or different contents); never refuses. A check with no record
/// yet (added after the plan was approved) is recorded now. The new state replaces the old one, so a
/// change is noted once, in the receipt of the first run after it.
pub fn verify(p: &Project, check_id: &str, word: &str) -> R<Option<String>> {
    let mut all = read(p);
    let Some(stored) = all.get(check_id).cloned() else {
        if let Some(e) = entry(p, word) {
            all[check_id] = e;
            util::atomic_write(&path_file(p), serde_json::to_string(&all)?.as_bytes())?;
        }
        return Ok(None);
    };
    let Some(path) = resolve(p, word) else {
        return Ok(None);
    };
    let (size, mtime) = stat(&path).unwrap_or((0, 0));
    let same_file = path.to_string_lossy() == s(&stored, "path");
    if same_file
        && size == stored["size"].as_u64().unwrap_or(0)
        && mtime == stored["mtime"].as_u64().unwrap_or(0)
    {
        return Ok(None);
    }
    let hash = util::sha256_file(&path).ok();
    let note = if !same_file {
        Some(format!(
            "the program `{word}` is now found at {}, not at {} where it was when the plan was approved",
            path.display(),
            s(&stored, "path")
        ))
    } else if hash.as_deref() != stored["hash"].as_str() {
        Some(format!(
            "the file {} is not the one recorded when the plan was approved (it changed)",
            path.display()
        ))
    } else {
        None
    };
    if let Some(e) = entry(p, word) {
        all[check_id] = e;
        util::atomic_write(&path_file(p), serde_json::to_string(&all)?.as_bytes())?;
    }
    Ok(note)
}
