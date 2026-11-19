//! The account's home (component: session home, C-3, D-51): one function, `account_home`, asks the
//! account database (`getpwuid_r` on the real user id) once for each command. It never reads `HOME`
//! or any variable. It gives no home when the database has no entry, or an empty, relative or
//! missing one. This module is the only place of the engine that reads `HOME` or
//! `BF_TEST_ACCOUNT_HOME` (the source test in `tests/account_home.rs` holds that).
//!
//! The cargo feature `test_seam` (off by default, asked for by no release build or install) lets
//! `BF_TEST_ACCOUNT_HOME` replace the answer, after the database was asked all the same.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(feature = "test_seam")]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(feature = "test_seam")]
static LOOKUPS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "test_seam")]
static LOOKUP_NANOS: AtomicU64 = AtomicU64::new(0);

static HOME: OnceLock<Option<PathBuf>> = OnceLock::new();

/// The real home the account lookup gave, kept by the one `resolve` call (test seam build only).
#[cfg(feature = "test_seam")]
static REAL: OnceLock<Option<PathBuf>> = OnceLock::new();

/// The account's home, asked of the account database once for each command (each process).
pub fn account_home() -> Option<PathBuf> {
    HOME.get_or_init(resolve).clone()
}

/// The account database's answer alone, with no seam: the real home of the real user id. It asks
/// the database itself, so it is for a test of the lookup and never for a command.
pub fn database_home() -> Option<PathBuf> {
    lookup()
}

/// The real home the one lookup of this command gave, whatever the seam answered in its place (test
/// seam build only). It never asks the database again.
#[cfg(feature = "test_seam")]
pub fn real_home() -> Option<PathBuf> {
    let _ = account_home();
    REAL.get().cloned().flatten()
}

#[cfg(feature = "test_seam")]
fn resolve() -> Option<PathBuf> {
    let t = std::time::Instant::now();
    let real = lookup();
    LOOKUP_NANOS.store(t.elapsed().as_nanos() as u64, Ordering::SeqCst);
    let _ = REAL.set(real.clone());
    match std::env::var_os("BF_TEST_ACCOUNT_HOME") {
        Some(v) => absolute(PathBuf::from(v)),
        None => real,
    }
}

#[cfg(not(feature = "test_seam"))]
fn resolve() -> Option<PathBuf> {
    lookup()
}

/// A folder that is not empty and is absolute, or no home.
fn absolute(p: PathBuf) -> Option<PathBuf> {
    if p.as_os_str().is_empty() || !p.is_absolute() {
        None
    } else {
        Some(p)
    }
}

/// The account database's home for a user: no home when it is empty, not absolute or not a folder
/// on the disk (C-3).
pub fn usable(p: PathBuf) -> Option<PathBuf> {
    absolute(p).filter(|h| h.is_dir())
}

/// Whether the state folder holds a folder of the project: the one check of the seam refusal, given
/// the real state folder and the project's id (C-3, D-48; test seam build only).
#[cfg(feature = "test_seam")]
pub fn state_folder_holds_project(state: &Path, project_id: &str) -> bool {
    !project_id.is_empty() && state.join(project_id).exists()
}

/// The real state folder of the account, `.local/state/blackfactory` under the real home the one
/// lookup gave (test seam build only).
#[cfg(feature = "test_seam")]
pub fn real_state_folder() -> Option<PathBuf> {
    real_home().map(|h| h.join(".local/state/blackfactory"))
}

#[cfg(unix)]
fn lookup() -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    #[cfg(feature = "test_seam")]
    LOOKUPS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: `getuid` takes no argument; `getpwuid_r` writes only into the structure and the
    // buffer given, and the answer is copied before the buffer is dropped.
    unsafe {
        let uid = libc::getuid();
        let mut size = 1024usize;
        loop {
            let mut pwd: libc::passwd = std::mem::zeroed();
            let mut buf = vec![0u8; size];
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let rc = libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            );
            if rc == libc::ERANGE && size < (1 << 20) {
                size *= 4;
                continue;
            }
            if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
                return None;
            }
            let dir = CStr::from_ptr(pwd.pw_dir).to_bytes();
            return usable(PathBuf::from(OsStr::from_bytes(dir)));
        }
    }
}

#[cfg(not(unix))]
fn lookup() -> Option<PathBuf> {
    None
}

/// A path of the adapter data with its `~` filled: `~` and `~/rest` lie under the account's home;
/// any other path is itself. A `~` path with no home gives nothing.
pub fn resolve_tilde(path: &str) -> Option<PathBuf> {
    if path == "~" {
        return account_home();
    }
    match path.strip_prefix("~/") {
        Some(rest) => account_home().map(|h| h.join(rest)),
        None => Some(Path::new(path).to_path_buf()),
    }
}

/// Sets `HOME` in a child's environment to the account's home, the one place that names the
/// variable for a child; with no home the variable is left out. The caller's `HOME` is never passed.
pub fn set_env(c: &mut std::process::Command) {
    match account_home() {
        Some(h) => c.env("HOME", h),
        None => c.env_remove("HOME"),
    };
}

/// The calls to the account database so far in this process (test seam build only).
#[cfg(feature = "test_seam")]
pub fn lookup_count() -> usize {
    LOOKUPS.load(Ordering::SeqCst)
}

/// The time of the resolver's call to the database, in nanoseconds (test seam build only).
#[cfg(feature = "test_seam")]
pub fn lookup_nanos() -> u64 {
    LOOKUP_NANOS.load(Ordering::SeqCst)
}

/// The seam's folder, when the seam build has one set (even an empty one).
#[cfg(feature = "test_seam")]
pub fn seam_set() -> bool {
    std::env::var_os("BF_TEST_ACCOUNT_HOME").is_some()
}
