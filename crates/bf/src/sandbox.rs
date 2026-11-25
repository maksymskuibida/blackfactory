//! Running the engine's child processes under a sandbox, where the system has one.
//!
//! On macOS every process the engine starts (a check command, a git call) runs under
//! `/usr/bin/sandbox-exec` with a profile that allows everything except: reading or writing the
//! project's state folder (the key), writing under `factory/` and under the main checkout's `.bf/`
//! outside the working copy in use, and the network. Where `sandbox-exec` is missing or refuses to
//! run, the child runs without it and the receipt says so.

pub mod compile_cache;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Does `sandbox-exec` exist and run a trivial command under a profile? Checked once per process, and
/// only when a git call or a check needs the answer (a process that runs neither starts no probe).
pub fn available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        note_probe();
        cfg!(target_os = "macos")
            && Path::new(SANDBOX_EXEC).exists()
            && Command::new(SANDBOX_EXEC)
                .args([
                    "-p",
                    "(version 1)(allow default)(deny network*)",
                    "/usr/bin/true",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    })
}

/// A test build appends one line to the file `BF_TEST_PROBE_LOG` names each time it starts the probe,
/// so a test can count probes (C-3). A release build has no such seam.
#[cfg(feature = "test_seam")]
fn note_probe() {
    use std::io::Write;
    if let Some(f) = std::env::var_os("BF_TEST_PROBE_LOG") {
        if let Ok(mut h) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(f)
        {
            let _ = writeln!(h, "probe {}", std::process::id());
        }
    }
}

#[cfg(not(feature = "test_seam"))]
fn note_probe() {}

fn q(p: &str) -> String {
    format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The profile for a check command run in `wc` (the working copy in use).
pub fn check_profile(
    state_dir: &str,
    root: &str,
    wc: &str,
    network: bool,
    transcripts: &str,
    repos: &[String],
) -> String {
    let mut p = String::from("(version 1)\n(allow default)\n");
    if !network {
        p.push_str("(deny network*)\n");
    }
    p.push_str(&format!(
        "(deny file-read* file-write* (subpath {}))\n",
        q(state_dir)
    ));
    p.push_str(&format!(
        "(deny file-write* (subpath {}))\n",
        q(&format!("{root}/factory"))
    ));
    p.push_str(&format!(
        "(deny file-write* (subpath {}))\n",
        q(&format!("{root}/.bf"))
    ));
    // Git runs a hook or reads its config outside the sandbox later, so a check may not plant either; the
    // harness's settings and the transcripts are not a check's to write.
    p.push_str(&format!(
        "(deny file-write* (subpath {}))\n",
        q(&format!("{root}/.claude"))
    ));
    for base in std::iter::once(root).chain(repos.iter().map(|r| r.as_str())) {
        p.push_str(&format!(
            "(deny file-write* (subpath {}))\n",
            q(&format!("{base}/.git/hooks"))
        ));
        p.push_str(&format!(
            "(deny file-write* (literal {}))\n",
            q(&format!("{base}/.git/config"))
        ));
    }
    if !transcripts.is_empty() {
        p.push_str(&format!(
            "(deny file-write* (subpath {}))\n",
            q(transcripts)
        ));
    }
    if wc != root {
        p.push_str(&format!("(allow file-write* (subpath {}))\n", q(wc)));
        p.push_str(&format!(
            "(deny file-write* (subpath {}))\n",
            q(&format!("{wc}/factory"))
        ));
    }
    p
}

/// The denial of the whole compile cache folder (R-76): reads and writes, by its literal path, which
/// goes after every other rule of a check's profile while the setting is on.
pub fn cache_denial(top: &str) -> String {
    format!(
        "(deny file-read* file-write* (subpath {t}))\n(deny network-outbound (remote unix-socket (subpath {t})))\n",
        t = q(top)
    )
}

/// The header every profile starts with, which a profile built from another one repeats.
const HEADER: &str = "(version 1)\n(allow default)\n";

/// The profile of the compile cache's server for a unit check (R-76 acceptance 9, TH-20). `check_profile`
/// is the check's profile built with no network and without the denial of the cache folder. Every write
/// is denied; then writes are allowed in `units/` of `folder`, in the run's folder and at `/dev/null`;
/// then the check profile's own rules come again in their order (so no allowance opens a denial); last
/// the server may bind and accept on its one Unix socket. `wc` is the run's working copy, which the
/// check profile's rules already allow.
pub fn cache_server_profile(check_profile: &str, wc: &str, folder: &str, run_dir: &str) -> String {
    let _ = wc;
    let rules = check_profile.strip_prefix(HEADER).unwrap_or(check_profile);
    let socket = format!("{run_dir}/{}", compile_cache::server::SOCKET_NAME);
    let mut p = String::from(HEADER);
    p.push_str("(deny network*)\n(deny file-write*)\n");
    for allowed in [format!("{folder}/units"), run_dir.to_string()] {
        p.push_str(&format!("(allow file-write* (subpath {}))\n", q(&allowed)));
    }
    p.push_str(&format!(
        "(allow file-write* (literal {}))\n",
        q("/dev/null")
    ));
    p.push_str(rules);
    p.push_str(&format!(
        "(allow network-bind network-inbound (local unix-socket (path-literal {})))\n",
        q(&socket)
    ));
    p
}

/// The one rule that lets a check connect to the server's socket and nothing else of the network; it
/// goes after the denial of the cache folder.
pub fn cache_client_rule(socket: &str) -> String {
    format!(
        "(allow network-outbound (remote unix-socket (path-literal {})))\n",
        q(socket)
    )
}

/// What a check that uses the cache may do in the cache folder beyond the socket (after the folder's
/// denial): read and write in the run's own `tmp/`, and make no hard link to a file elsewhere in the
/// cache folder (`top`, the denial's path). The folders above `tmp/` give their metadata alone (a literal rule for each, no listing and
/// no read of a file), because without it the system refuses to enter `tmp/` as a working directory, and
/// a test that makes a project there and runs a command in it would fail.
pub fn cache_tmp_rules(top: &str, tmp: &str) -> String {
    let mut p = String::new();
    // From the cache's top (`.../blackfactory-cc`, the denial's path) down to the folder that holds `tmp/`.
    let mut chain: Vec<&Path> = Vec::new();
    let mut cur = Path::new(tmp).parent();
    while let Some(c) = cur {
        chain.push(c);
        if c == Path::new(top) {
            break;
        }
        cur = c.parent();
    }
    for c in chain.iter().rev() {
        p.push_str(&format!(
            "(allow file-read-metadata (literal {}))\n",
            q(&c.to_string_lossy())
        ));
    }
    p.push_str(&format!(
        "(allow file-read* file-write* (subpath {t}))\n(deny file-link (subpath {f}))\n(allow file-link (subpath {t}))\n",
        t = q(tmp),
        f = q(top)
    ));
    p
}

/// What a command in the checkpoint reviewer's copy may not do beyond a check (TH-16): move a branch or
/// a tag of the repository the copy came from. The copy is a worktree, so its git data lives in the
/// repository's own `.git`; the refs and the main checkout's HEAD are out of reach, the copy's own index
/// and objects are not.
pub fn review_profile(base: &str, repos: &[String]) -> String {
    let mut p = String::from(base);
    for r in repos {
        p.push_str(&format!(
            "(deny file-write* (subpath {}))\n",
            q(&format!("{r}/.git/refs"))
        ));
        for f in ["packed-refs", "HEAD", "ORIG_HEAD"] {
            p.push_str(&format!(
                "(deny file-write* (literal {}))\n",
                q(&format!("{r}/.git/{f}"))
            ));
        }
    }
    p
}

/// Whether `dir` is a checkpoint reviewer's copy: a folder `<K>-review` directly under the work folder.
pub fn is_review_copy(work_dir: &Path, dir: &Path) -> bool {
    dir.strip_prefix(work_dir)
        .ok()
        .and_then(|rel| rel.components().next())
        .and_then(|c| c.as_os_str().to_str())
        .map(|n| n.starts_with("K-") && n.ends_with("-review"))
        .unwrap_or(false)
}

/// The profile for git calls: the state folder and the network are out of reach, and `factory/` is
/// not written.
pub fn git_profile(state_dir: &str, root: &str) -> String {
    let mut p = String::from("(version 1)\n(allow default)\n(deny network*)\n");
    p.push_str(&format!(
        "(deny file-read* file-write* (subpath {}))\n",
        q(state_dir)
    ));
    p.push_str(&format!(
        "(deny file-write* (subpath {}))\n",
        q(&format!("{root}/factory"))
    ));
    p
}

/// The profile with its network denial taken out: for the one git call that fetches the main branch.
pub fn allow_network(profile: &str) -> String {
    profile.replace("(deny network*)\n", "")
}

/// A command that runs `program` under `profile`.
pub fn command(profile: &str, program: &str) -> Command {
    let mut c = Command::new(SANDBOX_EXEC);
    c.arg("-p").arg(profile).arg(program);
    c
}
