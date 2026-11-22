//! The project-wide engine lock. Commands that change a checkpoint's working copy (landing, closing,
//! syncing, starting a unit) hold it for their whole git work, so two of them never run together.
//! A second one waits its turn, up to a set time, and is refused cleanly only when that time passes.

use crate::refusal::{err, refuse, R};
use crate::store::Project;
use std::fs;

/// Held until dropped; the operating system frees it if the process dies.
pub struct EngineLock {
    _file: fs::File,
}

/// How long a command waits for the lock before it is refused, in milliseconds. The default is
/// three minutes; `BF_LOCK_WAIT_MS` shortens or lengthens it (the tests use it).
fn wait_ms() -> u64 {
    std::env::var("BF_LOCK_WAIT_MS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(180_000)
}

fn take(p: &Project, name: &str, what: &str, ms: u64) -> R<EngineLock> {
    fs::create_dir_all(p.dot_bf())?;
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join(name))?;
    let start = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(EngineLock { _file: file }),
            Err(fs::TryLockError::WouldBlock) => {
                if start.elapsed().as_millis() as u64 >= ms {
                    return Err(refuse(
                        what.to_string(),
                        "another engine command is running and did not finish in time",
                        &["Wait for it to finish, then run this command again."],
                        "The command runs on its own.",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(fs::TryLockError::Error(e)) => {
                return Err(err(format!("cannot lock the engine: {e}")));
            }
        }
    }
}

/// Take the lock, waiting for it up to the set time; refuse cleanly when the time passes. The
/// operating system frees the lock if its holder dies, so waiting cannot hang on a crash.
pub fn engine(p: &Project, what: &str) -> R<EngineLock> {
    take(p, "engine.lock", what, wait_ms())
}

/// A short lock around a read and write of the role-token file, so two writers cannot lose each
/// other's change.
pub fn tokens(p: &Project) -> R<EngineLock> {
    take(
        p,
        "tokens.lock",
        "change the role tokens",
        wait_ms().max(10_000),
    )
}

/// The file name of a checkpoint's turn lock and mark: the checkpoint's ID with every character that is
/// no letter, digit, `-` or `_` made a `_`.
fn turn_stem(k: &str) -> String {
    k.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Open the lock of a checkpoint's landing turn for a test or a take, creating the file.
fn turn_file(p: &Project, k: &str) -> R<fs::File> {
    fs::create_dir_all(p.dot_bf())?;
    Ok(fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join(format!("land-{}.lock", turn_stem(k))))?)
}

/// Take the lock that holds a checkpoint's landing turn, without waiting: `None` when a turn of that
/// checkpoint is running. The operating system frees it if the process dies.
pub fn turn(p: &Project, k: &str) -> R<Option<EngineLock>> {
    let file = turn_file(p, k)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(EngineLock { _file: file })),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(e)) => Err(err(crate::strings::t(
            "land_merge.lock.error",
            &[("e", &e.to_string())],
        ))),
    }
}

/// Whether a landing turn of the checkpoint is running now: one test of its lock.
pub fn turn_open(p: &Project, k: &str) -> bool {
    match turn_file(p, k) {
        Ok(file) => matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)),
        Err(_) => false,
    }
}

/// The mark a turn writes beside its lock file before its first merge: the commit each repository's
/// checkpoint branch stands on and the units of the turn.
pub fn turn_mark(p: &Project, k: &str) -> std::path::PathBuf {
    p.dot_bf().join(format!("land-{}.mark", turn_stem(k)))
}
