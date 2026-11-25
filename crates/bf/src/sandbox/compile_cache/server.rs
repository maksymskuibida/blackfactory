//! The server of a unit check's compile cache (R-76 acceptance 9 to 12, D-104, TH-20): one sccache
//! server for each run, the engine's own child, in the foreground, on one Unix socket in a folder of
//! the run's own under `t/`. The folder holds the server's two config files and the run's temporary
//! folder `tmp/`. The server ends when the check ends, however it ends, and the folder is removed
//! without following a link.

use super::Paths;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The socket's name in the run's folder: short, because a socket path may not be longer than the limit.
pub const SOCKET_NAME: &str = "s";
/// The longest socket path in bytes the engine accepts.
pub const SOCKET_MAX: usize = 100;
/// How long the server has to accept on its socket.
pub const READY_SECONDS: u64 = 10;
const CONF_NAME: &str = "conf.toml";
const CACHED_NAME: &str = "cached.toml";
const TMP_NAME: &str = "tmp";
/// The file the run's engine holds a lock on for the run's whole life.
pub const LOCK_NAME: &str = "lock";

fn note(key: &str, values: &[(&str, &str)]) -> String {
    crate::strings::t(&format!("cli.compile_cache.{key}"), values)
}

fn cstr(p: &[u8]) -> Option<CString> {
    CString::new(p.to_vec()).ok()
}

/// The run's own folder under `t/`, and what lies in it.
#[derive(Clone, Debug)]
pub struct RunDir {
    /// The lock the engine holds on the run's folder while the run lives (`flock`, closed on exec, so
    /// no child holds it); the sweep of leftover folders skips a folder whose lock is held.
    pub lock: std::sync::Arc<std::fs::File>,
    /// The `t/` folder, by its literal path.
    pub runs: PathBuf,
    /// The folder's name: 8 random hex digits.
    pub name: String,
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub conf: PathBuf,
    pub cached: PathBuf,
    pub tmp: PathBuf,
}

/// The socket path of a run folder named `name` under `runs`.
fn socket_of(runs: &Path, name: &str) -> PathBuf {
    runs.join(name).join(SOCKET_NAME)
}

fn random_hex8() -> Option<String> {
    use std::io::Read;
    let mut b = [0u8; 4];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut b)
        .ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Make the run's folder: `mkdirat` below `t/` opened without following a link (a name that exists is
/// never reused), then the two config files, written fresh and empty, and `tmp/`. The reason when the
/// socket path would be longer than 100 bytes, or a part cannot be made.
pub fn make_run_dir(paths: &Paths) -> Result<RunDir, String> {
    sweep_stale(&paths.runs);
    let runs = paths.runs.clone();
    let probe = socket_of(&runs, "00000000");
    if probe.as_os_str().len() > SOCKET_MAX {
        return Err(note(
            "off.socket_long",
            &[("path", &probe.to_string_lossy())],
        ));
    }
    let open_err = || note("off.run_dir", &[("path", &runs.to_string_lossy())]);
    let rc = cstr(runs.as_os_str().as_bytes()).ok_or_else(open_err)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: plain system calls on C strings that live through each call; every fd is closed.
    let tfd = unsafe { libc::open(rc.as_ptr(), flags) };
    if tfd < 0 {
        return Err(open_err());
    }
    let mut made = None;
    for _ in 0..16 {
        let Some(name) = random_hex8() else { break };
        let c = CString::new(name.clone()).map_err(|_| open_err())?;
        if unsafe { libc::mkdirat(tfd, c.as_ptr(), 0o700) } == 0 {
            made = Some((name, c));
            break;
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            break;
        }
    }
    let Some((name, c)) = made else {
        unsafe { libc::close(tfd) };
        return Err(open_err());
    };
    let dfd = unsafe { libc::openat(tfd, c.as_ptr(), flags) };
    let fail = |dfd: i32| {
        if dfd >= 0 {
            unsafe { libc::close(dfd) };
        }
        remove_at(tfd, &c);
        unsafe { libc::close(tfd) };
        open_err()
    };
    if dfd < 0 {
        return Err(fail(-1));
    }
    for f in [CONF_NAME, CACHED_NAME] {
        let fc = CString::new(f).unwrap_or_default();
        let fd = unsafe {
            libc::openat(
                dfd,
                fc.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(fail(dfd));
        }
        unsafe { libc::close(fd) };
    }
    let lc = CString::new(LOCK_NAME).unwrap_or_default();
    let lfd = unsafe {
        libc::openat(
            dfd,
            lc.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600 as libc::c_uint,
        )
    };
    if lfd < 0 {
        return Err(fail(dfd));
    }
    // SAFETY: the fd is open and owned here; the file takes it over.
    let lock = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(lfd) };
    if unsafe { libc::flock(lfd, libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        drop(lock);
        return Err(fail(dfd));
    }
    let tc = CString::new(TMP_NAME).unwrap_or_default();
    if unsafe { libc::mkdirat(dfd, tc.as_ptr(), 0o700) } != 0 {
        return Err(fail(dfd));
    }
    unsafe {
        libc::close(dfd);
        libc::close(tfd);
    }
    let dir = runs.join(&name);
    Ok(RunDir {
        lock: std::sync::Arc::new(lock),
        socket: dir.join(SOCKET_NAME),
        conf: dir.join(CONF_NAME),
        cached: dir.join(CACHED_NAME),
        tmp: dir.join(TMP_NAME),
        dir,
        name,
        runs,
    })
}

/// A run folder that an engine killed in the middle of a run left behind: older than a minute, and no
/// engine holds its lock. A folder of a run that lives is skipped whatever became of its server (a check
/// whose server was lost keeps running and keeps using its `tmp/`); a folder being made is younger.
pub fn sweep_stale(runs: &Path) {
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(entries) = std::fs::read_dir(runs) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let old = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|d| d >= Duration::from_secs(60));
        if !(old && e.file_type().is_ok_and(|t| t.is_dir())) {
            continue;
        }
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(e.path().join(LOCK_NAME));
        use std::os::fd::AsRawFd;
        // No lock file: the folder was never made whole; a lock that is free: its engine is gone.
        let held = match &lock {
            // Anything but a regular file (a named pipe, say) is no lock: a free one.
            Ok(f) if !f.metadata().is_ok_and(|m| m.is_file()) => false,
            Ok(f) => (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) != 0,
            Err(_) => false,
        };
        if !held {
            remove_tree(runs, &name);
        }
    }
}

/// Remove `name` below the folder open as `parent`, and everything in it, without following a link: a
/// link found is unlinked and never entered, a folder is entered through `openat` with `O_NOFOLLOW`.
fn remove_at(parent: i32, name: &CStr) {
    // SAFETY: system calls on C strings and fds that live through each call; every fd is closed.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstatat(parent, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) != 0 {
            return;
        }
        if st.st_mode & libc::S_IFMT != libc::S_IFDIR {
            libc::unlinkat(parent, name.as_ptr(), 0);
            return;
        }
        // A folder the check made unreadable is opened up first (it is a folder, and a link is not
        // followed).
        libc::fchmodat(parent, name.as_ptr(), 0o700, libc::AT_SYMLINK_NOFOLLOW);
        let fd = libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        );
        if fd >= 0 {
            let dup = libc::dup(fd);
            let dir = if dup >= 0 {
                libc::fdopendir(dup)
            } else {
                std::ptr::null_mut()
            };
            if !dir.is_null() {
                let mut names: Vec<CString> = Vec::new();
                loop {
                    let e = libc::readdir(dir);
                    if e.is_null() {
                        break;
                    }
                    let n = CStr::from_ptr((*e).d_name.as_ptr());
                    if n.to_bytes() != b"." && n.to_bytes() != b".." {
                        names.push(n.to_owned());
                    }
                }
                libc::closedir(dir);
                for n in names {
                    remove_at(fd, &n);
                }
            } else if dup >= 0 {
                libc::close(dup);
            }
            libc::close(fd);
        }
        libc::unlinkat(parent, name.as_ptr(), libc::AT_REMOVEDIR);
    }
}

/// Remove the folder `name` below the folder `parent` and all it holds, never following a link (the
/// folder `parent` itself is opened with `O_NOFOLLOW`).
pub fn remove_tree(parent: &Path, name: &str) {
    let (Some(pc), Some(nc)) = (cstr(parent.as_os_str().as_bytes()), cstr(name.as_bytes())) else {
        return;
    };
    // SAFETY: as above.
    unsafe {
        let fd = libc::open(
            pc.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        );
        if fd < 0 {
            return;
        }
        remove_at(fd, &nc);
        libc::close(fd);
    }
}

/// The server and what the check needs of it. Ends the server and removes the run's folder on `Drop`.
pub struct Server {
    child: Option<Child>,
    /// The sccache program.
    pub program: String,
    /// The server's environment, less the two names that start a server (what `--stop-server` gets).
    stop_env: Vec<(String, String)>,
    pub run: RunDir,
}

impl Server {
    /// The socket the check connects to.
    pub fn socket(&self) -> &Path {
        &self.run.socket
    }

    /// The run's temporary folder, which is the check's `TMPDIR`.
    pub fn tmp(&self) -> &Path {
        &self.run.tmp
    }

    /// The process id of the server, while there is one.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }
}

/// What the server runs with: the program, its profile as a function of the run's folder (`None` runs it
/// with no sandbox, for a test where there is none), the policy's PATH, and the idle timeout in seconds.
pub struct Start<'a> {
    pub program: &'a str,
    pub profile: Option<&'a dyn Fn(&RunDir) -> String>,
    pub path: &'a str,
    pub idle_seconds: u64,
}

/// The server's environment: only these names (R-76 acceptance 9), so no config of the account and no
/// remote store steers it.
pub fn server_env(
    run: &RunDir,
    units: &Path,
    path: &str,
    idle_seconds: u64,
) -> Vec<(String, String)> {
    let s = |p: &Path| p.to_string_lossy().to_string();
    vec![
        ("PATH".into(), path.to_string()),
        ("SCCACHE_DIR".into(), s(units)),
        ("SCCACHE_SERVER_UDS".into(), s(&run.socket)),
        ("SCCACHE_CONF".into(), s(&run.conf)),
        ("SCCACHE_CACHED_CONF".into(), s(&run.cached)),
        ("SCCACHE_CACHE_SIZE".into(), "10G".into()),
        ("SCCACHE_IDLE_TIMEOUT".into(), idle_seconds.to_string()),
        ("SCCACHE_LOCAL_RW_MODE".into(), "READ_WRITE".into()),
        ("TMPDIR".into(), s(&run.tmp)),
    ]
}

/// Make the run's folder and start the server in it as the engine's child, and wait at most ten
/// seconds for the socket to accept. The reason (the text of a strings key) when it cannot be done; the
/// folder is gone again then.
pub fn start(paths: &Paths, s: &Start) -> Result<Server, String> {
    let run = make_run_dir(paths)?;
    let env = server_env(&run, &paths.units, s.path, s.idle_seconds);
    let mut cmd = match s.profile {
        Some(f) => crate::sandbox::command(&f(&run), s.program),
        None => Command::new(s.program),
    };
    cmd.env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env("SCCACHE_START_SERVER", "1")
        .env("SCCACHE_NO_DAEMON", "1")
        .current_dir(&run.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            remove_tree(&run.runs, &run.name);
            return Err(note("off.server_start", &[("why", &e.to_string())]));
        }
    };
    let mut server = Server {
        child: Some(child),
        program: s.program.to_string(),
        stop_env: env,
        run,
    };
    let began = Instant::now();
    loop {
        if UnixStream::connect(&server.run.socket).is_ok() {
            return Ok(server);
        }
        let gone = server
            .child
            .as_mut()
            .and_then(|c| c.try_wait().ok().flatten());
        if let Some(st) = gone {
            server.child = None;
            return Err(note("off.server_exited", &[("status", &st.to_string())]));
        }
        if began.elapsed() >= Duration::from_secs(READY_SECONDS) {
            return Err(note(
                "off.server_slow",
                &[("seconds", &READY_SECONDS.to_string())],
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for(c: &mut Child, limit: Duration) -> bool {
    let began = Instant::now();
    loop {
        if matches!(c.try_wait(), Ok(Some(_))) {
            return true;
        }
        if began.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            if !matches!(c.try_wait(), Ok(Some(_))) {
                let stop = Command::new(&self.program)
                    .arg("--stop-server")
                    .env_clear()
                    .envs(self.stop_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                if let Ok(mut st) = stop {
                    if !wait_for(&mut st, Duration::from_secs(5)) {
                        let _ = st.kill();
                        let _ = st.wait();
                    }
                }
                if !wait_for(&mut c, Duration::from_secs(5)) {
                    let _ = c.kill();
                }
            }
            let _ = c.wait();
        }
        remove_tree(&self.run.runs, &self.run.name);
    }
}
