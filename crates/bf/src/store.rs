//! The project on disk: where records live, config, policy, and the guard table.

use crate::refusal::{err, refuse, R};
use crate::schema::{self, Kind};
use crate::util;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// The default policy file. `exec_path` and `git` are captured from the human's shell at `bf init`.
pub fn default_policy_text(exec_path: &str, git: &str) -> String {
    format!(
        r#"# Black Factory policy. Only the human edits this file.
[limits]
max_rounds = 3        # review rounds before a unit is parked
idle_minutes = 60     # a running unit with no event for this long is stuck
max_attempts = 10     # failing check runs and blocked reports in a row before a unit is parked

[tiers]               # model tier by role: cheap, standard or strong
builder = "standard"
reviewer = "strong"
verifier = "standard"

[budget]
cap = 0.0             # 0 means no cap; with a cap, every `bf report` must carry --cost
unit = "USD"

[review]
required = true       # a unit needs a clean review before it can land

[ledger]
kinds = ["requirement", "component", "screen", "journey", "target", "threat", "decision"]

[evidence]
tail_bytes = 4000
max_attach_bytes = 5242880

[human]
sessions = []         # session ids that count as the human's own; empty means any session
# A decision is the quote the conductor names from the human's message; the engine stores both. These
# two lists only add a visible caution beside the decision ("the message also contains: not"). They
# never refuse anything.
approval_words = {approve}
refusal_words = {refuse}
transcript_root = "~/.claude/projects"   # where the harness keeps its transcripts
# "soft" records the turn from the hook payload and only notes what the transcript says (match, differs,
# unparsed, absent). "strict" refuses a turn unless the transcript, under the root above, already ends
# with the prompt; the harness may write the transcript only after the hook runs.
transcript_match = "soft"

[gates]
in_order = true       # a design gate opens only after the earlier ones are approved, and only with records behind it

[roles]
tokens = true         # `bf report` needs the one-use token that `bf next` handed out for the role

[exec]
# Checks run through /bin/sh with exactly this PATH and `git` is this program, whatever the caller's
# environment says. Captured at `bf init`; only the human edits it.
path = {path}
git = {git}
sandbox = true        # run checks and git under sandbox-exec where it works (macOS); see `bf doctor`
network = false       # true lets checks reach the network under the sandbox
env = []              # extra environment variable names a check may see, e.g. ["JAVA_HOME"]

[guard]
allow = "abstain"     # "abstain" prints {{}} and leaves the decision to the harness's own prompts; "allow" approves
protected_extra = []  # more paths agents may not write
"#,
        path = toml::Value::String(exec_path.to_string()),
        git = toml::Value::String(git.to_string()),
        approve = toml::Value::Array(
            DEFAULT_APPROVAL
                .iter()
                .map(|w| toml::Value::String(w.to_string()))
                .collect()
        ),
        refuse = toml::Value::Array(
            DEFAULT_REFUSAL
                .iter()
                .map(|w| toml::Value::String(w.to_string()))
                .collect()
        ),
    )
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Limits {
    pub max_rounds: u32,
    pub idle_minutes: u64,
    pub max_attempts: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_rounds: 3,
            idle_minutes: 60,
            max_attempts: 10,
        }
    }
}
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Tiers {
    pub builder: String,
    pub reviewer: String,
    pub verifier: String,
}
impl Default for Tiers {
    fn default() -> Self {
        Tiers {
            builder: "standard".into(),
            reviewer: "strong".into(),
            verifier: "standard".into(),
        }
    }
}
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Budget {
    pub cap: f64,
    pub unit: String,
}
impl Default for Budget {
    fn default() -> Self {
        Budget {
            cap: 0.0,
            unit: "USD".into(),
        }
    }
}
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Review {
    pub required: bool,
}
impl Default for Review {
    fn default() -> Self {
        Review { required: true }
    }
}
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct LedgerPol {
    pub kinds: Vec<String>,
}
impl Default for LedgerPol {
    fn default() -> Self {
        LedgerPol {
            kinds: [
                "requirement",
                "component",
                "screen",
                "journey",
                "target",
                "threat",
                "decision",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct EvidencePol {
    pub tail_bytes: usize,
    pub max_attach_bytes: u64,
}
impl Default for EvidencePol {
    fn default() -> Self {
        EvidencePol {
            tail_bytes: 4000,
            max_attach_bytes: 5_242_880,
        }
    }
}
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct GuardPol {
    pub allow: String,
    pub protected_extra: Vec<String>,
}
impl Default for GuardPol {
    fn default() -> Self {
        GuardPol {
            allow: "abstain".into(),
            protected_extra: vec![],
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct HumanPol {
    /// Session ids that count as the human's. Empty means any session.
    pub sessions: Vec<String>,
    /// Words that mark a message as an approval.
    pub approval_words: Vec<String>,
    /// Words that mark a message as a refusal; a message holding one cannot approve anything.
    pub refusal_words: Vec<String>,
    /// Where the harness keeps its transcripts. A turn is recorded only if its transcript is there.
    pub transcript_root: String,
    /// `soft` (the default): the turn is recorded from the payload, and what the transcript says is a
    /// note only. `strict`: the transcript's last human message must already be the prompt.
    pub transcript_match: String,
}
fn words(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| s.to_string()).collect()
}
impl Default for HumanPol {
    fn default() -> Self {
        HumanPol {
            sessions: vec![],
            approval_words: words(DEFAULT_APPROVAL),
            refusal_words: words(DEFAULT_REFUSAL),
            transcript_root: "~/.claude/projects".into(),
            transcript_match: "soft".into(),
        }
    }
}

pub const DEFAULT_APPROVAL: &[&str] = &[
    // English
    "approve",
    "approved",
    "yes",
    "ok",
    "okay",
    "go ahead",
    "lgtm",
    "agreed",
    "accept",
    "accepted",
    "fine",
    "proceed",
    "merge it",
    // German
    "ja",
    "genehmigt",
    "freigegeben",
    "einverstanden",
    "okay",
    "passt",
    "weiter",
    "los",
    // French
    "oui",
    "approuve",
    "approuvé",
    "d'accord",
    // Spanish
    "sí",
    "si",
    "vale",
    "aprobado",
    "adelante",
];
pub const DEFAULT_REFUSAL: &[&str] = &[
    // English
    "not",
    "no",
    "nope",
    "don't",
    "dont",
    "never",
    "reject",
    "rejected",
    "stop",
    "wait",
    "hold",
    "cancel",
    "disapprove",
    "change",
    "changes",
    "revise",
    "redo",
    "rework",
    "wrong",
    "instead",
    // German
    "nicht",
    "nein",
    "kein",
    "keine",
    "nie",
    "niemals",
    "ablehnen",
    "abgelehnt",
    "stopp",
    "warte",
    "halt",
    "ändern",
    "falsch",
    // French
    "non",
    "pas",
    "jamais",
    "rejeter",
    "arrêtez",
    "attendez",
    "changer",
    // Spanish
    "nunca",
    "rechazo",
    "rechazar",
    "espera",
    "cambiar",
    "incorrecto",
];

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct GatesPol {
    /// Design gates open in order and never empty (see `bf gate open`).
    pub in_order: bool,
}
impl Default for GatesPol {
    fn default() -> Self {
        GatesPol { in_order: true }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct RolesPol {
    pub tokens: bool,
}
impl Default for RolesPol {
    fn default() -> Self {
        RolesPol { tokens: true }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ExecPol {
    pub path: String,
    pub git: String,
    /// Run checks and git under the system's sandbox where there is one.
    pub sandbox: bool,
    /// Let checks use the network under the sandbox (default no).
    pub network: bool,
    /// More environment variable names a check may see, beyond the short built-in list.
    pub env: Vec<String>,
}
impl Default for ExecPol {
    fn default() -> Self {
        ExecPol {
            path: String::new(),
            git: String::new(),
            sandbox: true,
            network: false,
            env: Vec::new(),
        }
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Policy {
    pub gates: GatesPol,
    pub human: HumanPol,
    pub roles: RolesPol,
    pub exec: ExecPol,
    pub limits: Limits,
    pub tiers: Tiers,
    pub budget: Budget,
    pub review: Review,
    pub ledger: LedgerPol,
    pub evidence: EvidencePol,
    pub guard: GuardPol,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub project: String,
    /// Names the project's state folder (key, log head, role tokens) outside the project tree.
    pub project_id: String,
    pub main_branch: String,
    pub branch_prefix: String,
}
impl Default for Config {
    fn default() -> Self {
        Config {
            project: "project".into(),
            project_id: String::new(),
            main_branch: "main".into(),
            branch_prefix: "bf/".into(),
        }
    }
}

/// The name of the one repository of a project that was not set up as a workspace: the folder itself.
pub const CLASSIC_REPO: &str = "code";

/// A code repository the factory works on.
#[derive(Clone, Debug)]
pub struct Repo {
    pub name: String,
    /// Absolute path of its main checkout. In a project without a workspace file, the project folder.
    pub path: PathBuf,
    /// The main branch the file names.
    pub configured: String,
    /// The main branch in force: the one recorded in the log head when the project started.
    pub branch: String,
}

#[derive(Clone, Debug)]
pub struct Project {
    /// The main checkout, where `factory/` and `.bf/` live.
    pub root: PathBuf,
    /// The nearest working copy above the current directory (a linked worktree or the root).
    pub top: PathBuf,
    pub config: Config,
    pub policy: Policy,
    /// Base folder of the per-project state folders. Tests set it; otherwise the environment decides.
    pub state_base: Option<PathBuf>,
    /// Whether the policy file on disk is the one the human accepted.
    pub policy_state: PolicyState,
    /// The code repositories: those listed in `factory/workspace.toml`, or the project folder itself.
    pub repos: Vec<Repo>,
    /// Whether `factory/workspace.toml` exists (the factory lives apart from its code).
    pub workspace: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyState {
    /// The file on disk is the accepted policy.
    Accepted,
    /// No policy was ever accepted for this project on this machine.
    NeverAccepted,
    /// The file on disk differs from the accepted policy.
    Changed,
    /// The accepted copy in the state folder does not match the hash in the head.
    CopyDamaged,
}

fn read_gitdir_file(p: &Path) -> Option<PathBuf> {
    let s = fs::read_to_string(p).ok()?;
    let rest = s.trim().strip_prefix("gitdir:")?.trim();
    let g = PathBuf::from(rest);
    Some(if g.is_absolute() {
        g
    } else {
        p.parent()?.join(g)
    })
}

/// For a linked worktree's git dir, find the main checkout's root.
fn main_root_from_gitdir(gitdir: &Path) -> Option<PathBuf> {
    let common = fs::read_to_string(gitdir.join("commondir")).ok()?;
    let common = gitdir.join(common.trim());
    let common = fs::canonicalize(common).ok()?;
    common.parent().map(|p| p.to_path_buf())
}

/// The git directory of a working-copy top (handles linked worktrees).
pub fn git_dir_of(top: &Path) -> Option<PathBuf> {
    let dot = top.join(".git");
    if dot.is_dir() {
        Some(dot)
    } else if dot.is_file() {
        read_gitdir_file(&dot)
    } else {
        None
    }
}

/// The readable records, and the files that could not be read (file name, reason).
pub type Scanned = (Vec<Value>, Vec<(String, String)>);

/// An environment variable that holds an absolute path. An empty or relative value counts as unset, so
/// it cannot put the state folder (and the key) inside the project.
fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// The same policy once read: a comment, spacing or order changed on disk is not a change.
fn same_policy(a: &[u8], b: &[u8]) -> bool {
    match (
        toml::from_str::<toml::Value>(&String::from_utf8_lossy(a)),
        toml::from_str::<toml::Value>(&String::from_utf8_lossy(b)),
    ) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

impl Project {
    pub fn discover(start: &Path) -> R<Project> {
        let start = fs::canonicalize(start)
            .map_err(|e| err(format!("cannot open {}: {e}", start.display())))?;
        let mut top: Option<PathBuf> = None;
        for dir in start.ancestors() {
            let dot = dir.join(".git");
            if top.is_none() && dot.exists() {
                top = Some(dir.to_path_buf());
            }
            if dot.is_file() {
                if let Some(gd) = read_gitdir_file(&dot) {
                    if let Some(main) = main_root_from_gitdir(&gd) {
                        if main.join("factory/config.toml").is_file() {
                            return Project::open(
                                main,
                                top.unwrap_or_else(|| dir.to_path_buf()),
                                &start,
                            );
                        }
                    }
                }
                continue;
            }
            if dir.join("factory/config.toml").is_file() {
                let t = top.unwrap_or_else(|| dir.to_path_buf());
                return Project::open(dir.to_path_buf(), t, &start);
            }
        }
        Err(refuse(
            "use a Black Factory project here",
            format!(
                "no `factory/config.toml` was found in {} or any folder above it",
                start.display()
            ),
            &["Change into the project folder, or run `bf init` in the root of a git repository."],
            "`bf status` prints the project name.",
        ))
    }

    fn open(root: PathBuf, top: PathBuf, start: &Path) -> R<Project> {
        // Inside `.bf/work/<name>` the working copy is that folder, whatever git repository it belongs
        // to (a checkpoint over several repositories is a plain folder of working copies).
        let top = match start.strip_prefix(root.join(".bf/work")) {
            Ok(rel) => match rel.components().next() {
                Some(c) => root.join(".bf/work").join(c),
                None => top,
            },
            Err(_) => top,
        };
        let p = Project::open_inner(root, top)?;
        if p.config.project_id.is_empty() {
            return Err(refuse(
                "use this project",
                "its config has no project id, so there is no state folder for the key and the log head",
                &["Run `bf init` once: it adds the id and creates the state folder."],
                "`bf status` prints the project name.",
            ));
        }
        Ok(p)
    }

    /// Load the policy from the accepted copy in the state folder, never from the file on disk, and
    /// note whether the file on disk still equals it. (`state_base` may be set after `open`.)
    pub fn reload_policy(&mut self) -> R<()> {
        let head = crate::events::read_head(self);
        for r in self.repos.iter_mut() {
            r.branch = head
                .as_ref()
                .and_then(|h| h.repos.get(&r.name))
                .and_then(|x| x.branch.clone())
                .unwrap_or_else(|| r.configured.clone());
        }
        let disk = fs::read(self.root.join("factory/policy.toml")).unwrap_or_default();
        match fs::read(self.accepted_policy_path()) {
            Ok(acc) => {
                let acc_hash = util::sha256_hex(&acc);
                let head_hash = crate::events::read_head(self).and_then(|h| h.policy_hash);
                self.policy_state = if head_hash.as_deref() != Some(acc_hash.as_str()) {
                    PolicyState::CopyDamaged
                } else if util::sha256_hex(&disk) != acc_hash && !same_policy(&disk, &acc) {
                    PolicyState::Changed
                } else {
                    PolicyState::Accepted
                };
                self.policy = toml::from_str(&String::from_utf8_lossy(&acc))?;
            }
            Err(_) => {
                self.policy_state = PolicyState::NeverAccepted;
                self.policy = toml::from_str(&String::from_utf8_lossy(&disk)).unwrap_or_default();
            }
        }
        let profile = if self.policy.exec.sandbox && crate::sandbox::available() {
            Some(crate::sandbox::git_profile(
                &self.state_dir_resolved(),
                &self.root.to_string_lossy(),
            ))
        } else {
            None
        };
        crate::gitx::configure(&self.policy.exec.git, &self.policy.exec.path, profile);
        Ok(())
    }

    /// The state folder with symlinks resolved (a sandbox profile needs the real path).
    pub fn state_dir_resolved(&self) -> String {
        let d = self.state_dir();
        fs::canonicalize(&d)
            .unwrap_or(d)
            .to_string_lossy()
            .to_string()
    }

    /// Make the policy file on disk the accepted policy: copy it into the state folder and record its
    /// hash and the time in the head. Called by `bf init` the first time and by `bf policy accept`.
    pub fn store_accepted_policy(&self) -> R<()> {
        let disk = fs::read(self.root.join("factory/policy.toml"))?;
        util::atomic_write(&self.accepted_policy_path(), &disk)?;
        crate::events::ensure_head(self)?;
        let mut h = crate::events::read_head(self).unwrap_or_default();
        h.policy_hash = Some(util::sha256_hex(&disk));
        h.policy_at = Some(util::now_secs());
        crate::events::write_head(self, &h)?;
        Ok(())
    }

    /// The harness's transcript root from the accepted policy, as an absolute path.
    pub fn transcript_root(&self) -> String {
        let r = &self.policy.human.transcript_root;
        let abs = match r.strip_prefix("~/") {
            Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
            None => r.clone(),
        };
        fs::canonicalize(&abs)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(abs)
    }

    /// A file the engine is about to read for the caller: resolved again now, symlinks and all, and
    /// refused if it ended up in the state folder.
    pub fn safe_input(&self, path: &Path) -> R<PathBuf> {
        let canon = fs::canonicalize(path).map_err(|e| {
            refuse(
                format!("read {}", path.display()),
                format!("the file cannot be read: {e}"),
                &["Give the path of an existing file."],
                "The command prints its result.",
            )
        })?;
        if canon.starts_with(self.state_dir_resolved()) {
            return Err(refuse(
                format!("read {}", path.display()),
                "that path leads into the project's state folder (it may be a link): the engine does not read from there for a caller",
                &["Copy the file you mean into the project and give that path."],
                "The command reads the file.",
            ));
        }
        Ok(canon)
    }

    /// A file the engine is about to write for the caller: the folder (and the file, if it is a link)
    /// resolved again now, and refused if it leads into the state folder, `factory/`, `.git/`, the
    /// private part of `.bf/` or the transcripts.
    pub fn safe_output(&self, path: &Path) -> R<PathBuf> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let name = abs
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        let parent = abs.parent().unwrap_or(Path::new("/"));
        fs::create_dir_all(parent)?;
        let mut target = fs::canonicalize(parent)?.join(&name);
        if target.is_symlink() {
            target = fs::canonicalize(&target).unwrap_or(target);
        }
        let root = &self.root;
        let bad = |why: &str| {
            refuse(
                format!("write {}", path.display()),
                format!("that path leads into {why}"),
                &["Write the page somewhere else, for example under .bf/pages/."],
                "The command writes the file.",
            )
        };
        if target.starts_with(self.state_dir_resolved()) {
            return Err(bad("the state folder"));
        }
        if target.starts_with(root.join("factory")) {
            return Err(bad("factory/"));
        }
        if target.starts_with(root.join(".git")) {
            return Err(bad(".git/"));
        }
        let tr = self.transcript_root();
        if !tr.is_empty() && target.starts_with(&tr) {
            return Err(bad("the harness's transcripts"));
        }
        if let Ok(rel) = target.strip_prefix(root.join(".bf")) {
            let r = rel.to_string_lossy();
            if !(r.starts_with("pages/") || r.starts_with("drafts/") || r.starts_with("work/")) {
                return Err(bad("the private part of .bf/"));
            }
        }
        Ok(target)
    }

    pub fn accepted_policy_path(&self) -> PathBuf {
        self.state_dir().join("policy.accepted.toml")
    }

    fn open_inner(root: PathBuf, top: PathBuf) -> R<Project> {
        let config: Config =
            toml::from_str(&fs::read_to_string(root.join("factory/config.toml"))?)?;
        let policy: Policy = match fs::read_to_string(root.join("factory/policy.toml")) {
            Ok(s) => toml::from_str(&s)?,
            Err(_) => Policy::default(),
        };
        let mut p = Project {
            root,
            top,
            config,
            policy,
            state_base: None,
            policy_state: PolicyState::NeverAccepted,
            repos: vec![],
            workspace: false,
        };
        p.load_repos()?;
        p.reload_policy()?;
        Ok(p)
    }

    /// Read the repository list: `factory/workspace.toml`, or the project folder as the one repository.
    fn load_repos(&mut self) -> R<()> {
        let file = self.root.join("factory/workspace.toml");
        self.workspace = file.is_file();
        self.repos = if self.workspace {
            let v: toml::Value = toml::from_str(&fs::read_to_string(&file)?)?;
            let mut out = Vec::new();
            for r in v
                .get("repo")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let name = r
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let path = r.get("path").and_then(|x| x.as_str()).unwrap_or(".");
                let main = r
                    .get("main_branch")
                    .and_then(|x| x.as_str())
                    .unwrap_or("main")
                    .to_string();
                let joined = if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    self.root.join(path)
                };
                out.push(Repo {
                    name,
                    path: fs::canonicalize(&joined).unwrap_or(joined),
                    configured: main.clone(),
                    branch: main,
                });
            }
            out
        } else {
            vec![Repo {
                name: CLASSIC_REPO.into(),
                path: self.root.clone(),
                configured: self.config.main_branch.clone(),
                branch: self.config.main_branch.clone(),
            }]
        };
        Ok(())
    }

    pub fn repo(&self, name: &str) -> R<&Repo> {
        self.repos.iter().find(|r| r.name == name).ok_or_else(|| {
            refuse(
                format!("use the repository `{name}`"),
                "the workspace lists no such repository",
                &["`bf repo list` shows the repositories; `bf repo add <name> <path>` adds one."],
                "`bf repo list` shows it.",
            )
        })
    }

    /// The repository a unit changes: the one it names, or the only one there is.
    pub fn repo_of_unit(&self, unit: &Value) -> R<&Repo> {
        let n = s(unit, "repo");
        if n.is_empty() {
            match self.repos.as_slice() {
                [only] => Ok(only),
                [] => Err(refuse(
                    "find the unit's repository",
                    "the workspace lists no repository",
                    &["`bf repo add <name> <path>` adds one."],
                    "`bf repo list` shows it.",
                )),
                _ => Err(refuse(
                    format!("find the repository of {}", s(unit, "id")),
                    "the workspace lists several repositories and the unit names none",
                    &["Add `repo = \"<name>\"` to the unit (the plan problem says which)."],
                    "`bf plan check` passes.",
                )),
            }
        } else {
            self.repo(n)
        }
    }

    /// Content hash of a working copy. In a project whose one repository is the folder itself, `factory/`
    /// and `.bf/` are left out (they hold the factory's own files); a code repository is hashed whole.
    pub fn content_hash(&self, dir: &Path) -> R<String> {
        crate::gitx::workdir_hash(dir, !self.workspace)
    }

    pub fn factory(&self) -> PathBuf {
        self.root.join("factory")
    }
    pub fn records_dir(&self) -> PathBuf {
        self.root.join("factory/records")
    }
    pub fn receipts_dir(&self) -> PathBuf {
        self.root.join("factory/receipts")
    }
    pub fn evidence_dir(&self) -> PathBuf {
        self.root.join("factory/evidence")
    }
    pub fn log_path(&self) -> PathBuf {
        self.root.join("factory/log/events.jsonl")
    }
    pub fn dot_bf(&self) -> PathBuf {
        self.root.join(".bf")
    }
    pub fn guard_path(&self) -> PathBuf {
        self.root.join(".bf/guard.json")
    }
    /// The per-project state folder, outside the project tree: `<state base>/<project id>/`.
    /// The base is `state_base` (tests), `$BF_STATE_HOME`, `$XDG_STATE_HOME/blackfactory` or
    /// `~/.local/state/blackfactory`. It holds the sealing key, the log head and the role tokens.
    pub fn state_dir(&self) -> PathBuf {
        let base = self
            .state_base
            .clone()
            .or_else(|| absolute_env("BF_STATE_HOME"))
            .or_else(|| absolute_env("XDG_STATE_HOME").map(|x| x.join("blackfactory")))
            .or_else(|| absolute_env("HOME").map(|h| h.join(".local/state/blackfactory")))
            .unwrap_or_else(|| PathBuf::from("/nonexistent-state"));
        base.join(&self.config.project_id)
    }
    pub fn key_path(&self) -> PathBuf {
        self.state_dir().join("key")
    }
    pub fn head_path(&self) -> PathBuf {
        self.state_dir().join("head.json")
    }
    pub fn tokens_path(&self) -> PathBuf {
        self.state_dir().join("tokens.json")
    }
    pub fn read_key(&self) -> Option<String> {
        fs::read_to_string(self.key_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    /// Create the state folder (owner only) and, if absent, the key. Returns true if a key was made.
    pub fn ensure_state(&self) -> R<bool> {
        let dir = self.state_dir();
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        let key = self.key_path();
        if key.is_file() {
            return Ok(false);
        }
        // An older project kept the key in the project tree: move it out.
        let old = self.root.join(".bf/key");
        let text = match fs::read_to_string(&old) {
            Ok(t) if !t.trim().is_empty() => {
                let _ = fs::remove_file(&old);
                t
            }
            _ => util::random_hex(32)?,
        };
        fs::write(&key, text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o600))?;
        }
        Ok(true)
    }
    pub fn work_dir(&self) -> PathBuf {
        self.root.join(".bf/work")
    }
    pub fn briefs_dir(&self) -> PathBuf {
        self.root.join(".bf/briefs")
    }

    pub fn unit_branch(&self, unit: &str) -> String {
        format!("{}{}", self.config.branch_prefix, unit)
    }
    pub fn checkpoint_branch(&self, k: &str) -> String {
        format!("{}{}", self.config.branch_prefix, k)
    }

    // ---- records ----

    pub fn record_path(&self, id: &str) -> R<PathBuf> {
        if schema::parse_id(id).is_none() || schema::kind_of_id(id).is_none() {
            return Err(refuse(
                format!("use the ID `{id}`"),
                "it is not a valid record ID",
                &["Use an ID made of a kind prefix, a dash and a number, for example R-1 or TH-2."],
                "`bf record list` shows the IDs that exist.",
            ));
        }
        Ok(self.records_dir().join(format!("{id}.toml")))
    }

    pub fn get_record(&self, id: &str) -> R<Option<Value>> {
        let p = self.record_path(id)?;
        match fs::read_to_string(&p) {
            Ok(s) => toml::from_str::<Value>(&s).map(Some).map_err(|e| {
                refuse(
                    format!("read record {id}"),
                    format!(
                        "the file factory/records/{id}.toml cannot be read as a record: {}",
                        e.to_string().replace('\n', " ")
                    ),
                    &[
                        "Fix the file, or restore it from git history (the human's step).",
                        "`bf status` lists every record file that cannot be read.",
                    ],
                    format!("`bf record get {id}` prints the record."),
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn require_record(&self, id: &str) -> R<Value> {
        self.get_record(id)?.ok_or_else(|| {
            refuse(
                format!("read record {id}"),
                "no record with that ID exists",
                &["Run `bf record list` to see the IDs, or `bf record put` to add it."],
                format!("`bf record get {id}` prints the record."),
            )
        })
    }

    pub fn write_record(&self, rec: &Value) -> R<()> {
        let id = rec
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err("record without id"))?;
        let path = self.record_path(id)?;
        let clean = strip_nulls(rec.clone());
        let text = toml::to_string_pretty(&clean)?;
        util::atomic_write(&path, text.as_bytes())?;
        Ok(())
    }

    /// Load every record (optionally of one kind), sorted by kind then number.
    pub fn list_records(&self, kind: Option<&Kind>) -> R<Vec<Value>> {
        Ok(self.scan_records(kind)?.0)
    }

    /// Every record that can be read, and the files that cannot (name and error). One bad file
    /// never hides the others: status and doctor name it, and the plan check counts it as a problem.
    pub fn scan_records(&self, kind: Option<&Kind>) -> R<Scanned> {
        let mut out: Vec<(String, u32, Value)> = Vec::new();
        let mut bad: Vec<(String, String)> = Vec::new();
        let dir = self.records_dir();
        let rd = match fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => return Ok((vec![], vec![])),
        };
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".toml") else {
                continue;
            };
            let Some((p, n)) = schema::parse_id(stem) else {
                continue;
            };
            if let Some(k) = kind {
                if k.prefix != p {
                    continue;
                }
            }
            let parsed = fs::read_to_string(ent.path())
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    toml::from_str::<Value>(&s).map_err(|e| e.to_string().replace('\n', " "))
                });
            match parsed {
                Ok(v) => out.push((p.to_string(), n, v)),
                Err(e) => bad.push((name, e)),
            }
        }
        out.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));
        bad.sort();
        Ok((out.into_iter().map(|x| x.2).collect(), bad))
    }

    pub fn next_id(&self, kind: &Kind) -> R<String> {
        let mut max = 0;
        if let Ok(rd) = fs::read_dir(self.records_dir()) {
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().to_string();
                if let Some((p, n)) = name.strip_suffix(".toml").and_then(schema::parse_id) {
                    if p == kind.prefix && n > max {
                        max = n;
                    }
                }
            }
        }
        max = max.max(self.reserved_max(kind));
        Ok(format!("{}-{}", kind.prefix, max + 1))
    }

    /// The highest number handed out by `bf id new` for this kind (derived, in `.bf/reserved.json`).
    fn reserved_max(&self, kind: &Kind) -> u32 {
        fs::read_to_string(self.dot_bf().join("reserved.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get(kind.prefix).and_then(|n| n.as_u64()))
            .unwrap_or(0) as u32
    }

    /// `bf id new`: the next free ID, remembered so that no later `id new` or `put` without an id
    /// hands out the same number. Numbers may have gaps.
    pub fn reserve_id(&self, kind: &Kind) -> R<String> {
        let id = self.next_id(kind)?;
        let n = schema::parse_id(&id).map(|x| x.1).unwrap_or(0);
        let path = self.dot_bf().join("reserved.json");
        let mut v: Value = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({}));
        v[kind.prefix] = json!(n);
        util::atomic_write(&path, serde_json::to_string(&v)?.as_bytes())?;
        Ok(id)
    }

    // ---- guard table ----

    /// Rewrite `.bf/guard.json`. Called by every command that changes state.
    pub fn write_guard(&self) -> R<()> {
        let state = crate::events::replay(self).ok();
        let mut protected: Vec<String> = vec![
            "factory/".into(),
            ".bf/".into(),
            ".claude/settings.json".into(),
            ".claude/settings.local.json".into(),
            ".git/hooks/".into(),
            ".git/config".into(),
        ];
        protected.extend(self.policy.guard.protected_extra.iter().cloned());

        let mut worktrees: Vec<Value> = Vec::new();
        let units = self
            .list_records(schema::kind_by_name("unit"))
            .unwrap_or_default();
        let cps = self
            .list_records(schema::kind_by_name("checkpoint"))
            .unwrap_or_default();
        for u in &units {
            let id = u["id"].as_str().unwrap_or("");
            let active = state
                .as_ref()
                .and_then(|s| s.units.get(id))
                .map(|us| !matches!(us.state.as_str(), "landed" | "parked"))
                .unwrap_or(false);
            if !active {
                continue;
            }
            worktrees.push(json!({
                "kind": "unit",
                "id": id,
                "path": self.work_dir().join(id).to_string_lossy(),
                "files": u["files"],
            }));
        }
        for k in &cps {
            let id = k["id"].as_str().unwrap_or("");
            worktrees.push(json!({
                "kind": "checkpoint",
                "id": id,
                "path": self.work_dir().join(id).to_string_lossy(),
                "files": [],
            }));
        }
        // Each table is newer than the last: the guard refuses a signed table older than the counter
        // kept next to the key, so putting an old table back loosens nothing.
        let gen_path = self.state_dir().join("guard.gen");
        let gen = fs::read_to_string(&gen_path)
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        let table = json!({
            "v": 1,
            "root": self.root.to_string_lossy(),
            "allow": self.policy.guard.allow,
            "protected": protected,
            "work_prefix": self.work_dir().to_string_lossy(),
            "deny_commands": [
                {"words": ["bf", "human-turn"], "reason": "Only the harness hook may record a human turn; an agent that records one could approve its own gate."},
                {"words": ["git", "push"], "reason": "Nothing is pushed until the human decides; no engine command touches the network."},
                {"words": ["git", "update-ref"], "reason": "Branch pointers are moved by the engine only."},
                {"words": ["git", "merge"], "reason": "Merging onto a checkpoint branch or the main branch is done by `bf unit land` and `bf checkpoint close`.", "outside_units_only": true}
            ],
            "transcript_root": self.transcript_root(),
            "gen": gen,
            "state_dir": self.state_dir().to_string_lossy(),
            "worktrees": worktrees,
            "main_branch": self.repos.first().map(|r| r.branch.clone()).unwrap_or_else(|| self.config.main_branch.clone()),
            "repos": self.repos.iter().map(|r| json!({"name": r.name, "path": r.path.to_string_lossy(), "branch": r.branch})).collect::<Vec<_>>(),
            "branch_prefix": self.config.branch_prefix,
        });
        // Sign the table with the key, so a table planted or emptied by hand does not pass.
        let mut table = table;
        if let Some(key) = self.read_key() {
            let sig =
                util::hmac_sha256_hex(key.as_bytes(), serde_json::to_string(&table)?.as_bytes());
            table["sig"] = json!(sig);
        }
        util::atomic_write(
            &self.guard_path(),
            serde_json::to_string_pretty(&table)?.as_bytes(),
        )?;
        util::atomic_write(&gen_path, gen.to_string().as_bytes())?;
        Ok(())
    }
}

pub fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(
            a.into_iter()
                .filter(|v| !v.is_null())
                .map(strip_nulls)
                .collect(),
        ),
        o => o,
    }
}

pub fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}
