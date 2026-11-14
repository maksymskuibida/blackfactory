//! The project-wide engine lock. Commands that change a checkpoint's working copy (landing, closing,
//! syncing, starting a unit) hold it for their whole git work, so two of them never run together.
//! A second one is refused at once instead of waiting.

use crate::refusal::{err, refuse, R};
use crate::store::Project;
use std::fs;

/// Held until dropped; the operating system frees it if the process dies.
pub struct EngineLock {
    _file: fs::File,
}

/// Take the lock, or refuse because another engine command holds it.
pub fn engine(p: &Project, what: &str) -> R<EngineLock> {
    fs::create_dir_all(p.dot_bf())?;
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(p.dot_bf().join("engine.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(EngineLock { _file: file }),
        Err(fs::TryLockError::WouldBlock) => Err(refuse(
            what.to_string(),
            "another engine command is running",
            &["Wait for it to finish, then run this command again."],
            "The command runs on its own.",
        )),
        Err(fs::TryLockError::Error(e)) => Err(err(format!("cannot lock the engine: {e}"))),
    }
}
