//! The harness adapter data (component: the harness adapter, C-24): what the engine knows of the
//! harness, compiled into the binary by `build.rs` from `harness/claude/adapter.toml`.

/// The adapter data of one harness. A path that starts with `~/` is under the account's home
/// (`session::home::resolve_tilde`).
pub struct Data {
    /// The folder of the session files.
    pub session_dir: &'static str,
    /// The name form of a session file; `<pid>` stands for the process number.
    pub session_file: &'static str,
    pub key_pid: &'static str,
    pub key_proc_start: &'static str,
    pub key_session_id: &'static str,
    /// The variable that names the session when the walk finds no file.
    pub fallback_variable: &'static str,
    /// The harness's own configuration variable.
    pub config_variable: &'static str,
    /// The fields of a subagent's hook payload.
    pub subagent_fields: &'static [&'static str],
    /// The programs that are the harness itself.
    pub programs: &'static [&'static str],
    /// The folder of the transcripts.
    pub transcript_dir: &'static str,
    /// The forms of the command that sets the human-absent flag (R-72); the first is the one named.
    pub absent_commands: &'static [&'static str],
    /// The forms a started session's first message begins with (D-94).
    pub nested_forms: &'static [&'static str],
    /// The permission modes in which a tool asks before it runs.
    pub ask_modes: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/adapter_data.rs"));

/// The name of the session file of a process.
pub fn session_file_name(pid: u32) -> String {
    DATA.session_file.replace("<pid>", &pid.to_string())
}
