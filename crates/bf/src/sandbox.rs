//! Running the engine's child processes under a sandbox, where the system has one.
//!
//! On macOS every process the engine starts (a check command, a git call) runs under
//! `/usr/bin/sandbox-exec` with a profile that allows everything except: reading or writing the
//! project's state folder (the key), writing under `factory/` and under the main checkout's `.bf/`
//! outside the working copy in use, and the network. Where `sandbox-exec` is missing or refuses to
//! run, the child runs without it and the receipt says so.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Does `sandbox-exec` exist and run a trivial command under a profile? Checked once per process.
pub fn available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
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
    for deny in [format!("{root}/.git/hooks"), format!("{root}/.claude")] {
        p.push_str(&format!("(deny file-write* (subpath {}))\n", q(&deny)));
    }
    p.push_str(&format!(
        "(deny file-write* (literal {}))\n",
        q(&format!("{root}/.git/config"))
    ));
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

/// A command that runs `program` under `profile`.
pub fn command(profile: &str, program: &str) -> Command {
    let mut c = Command::new(SANDBOX_EXEC);
    c.arg("-p").arg(profile).arg(program);
    c
}
