#![allow(dead_code)]
//! Shared helpers for the integration tests: a temporary git project and the real `bf` binary.

pub mod schema;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout is not JSON ({e}): {}\nstderr: {}",
                self.stdout, self.stderr
            )
        })
    }
    pub fn all(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

pub struct Fx {
    _tmp: TempDir,
    scratch: TempDir,
    pub root: PathBuf,
    /// Base of the per-project state folders (the sealing key and the log head live below it).
    pub state_base: PathBuf,
    /// A copied lifecycle state's own transcript folder, inside the state's shared transcript root
    /// (see `Fx::state_copy`); None for every other fixture, whose transcripts lie beside the project.
    tr: Option<TempDir>,
}

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_bf")
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// The fixture's sandbox switch: `All` writes `sandbox = true` into the fixture's policy, `Off` writes
/// `sandbox = false`. The plain constructors take it from `BF_TEST_SANDBOX` (see `sandbox_switch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    All,
    Off,
}

/// The switch the environment gives: `All` when `BF_TEST_SANDBOX` is `all`, `Off` otherwise. The one
/// place the variable is read.
pub fn sandbox_switch() -> Switch {
    if std::env::var("BF_TEST_SANDBOX").as_deref() == Ok("all") {
        Switch::All
    } else {
        Switch::Off
    }
}

/// Does `sandbox-exec` run here, from this test process? The same probe the engine runs, so an engine
/// started by the test sees the same answer. False on a system without it, and on macOS inside a
/// sandbox where a nested one does not start (a check the engine runs).
pub fn sandbox_probe() -> bool {
    cfg!(target_os = "macos")
        && Path::new("/usr/bin/sandbox-exec").exists()
        && Command::new("/usr/bin/sandbox-exec")
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
}

/// Run one passing check in the fixture through the engine and give its receipt's `sandbox` value
/// (`sandbox-exec` or `none`). Adds records of its own. In a project the check runs in the main
/// checkout; in a workspace, whose main folder holds no code, it runs in a unit's working copy of a
/// test run. A fixture with role tokens on (`shipped`) has them switched off first, the human's way,
/// so the records can be put; the sandbox line of its policy stays as it is. A project whose test run
/// has started (`Fx::tiny_run`) takes no new record, so there the tiny project's own check X-1 runs in
/// U-1's working copy.
pub fn receipt_sandbox(fx: &Fx) -> String {
    let pol = fs::read_to_string(fx.root.join("factory/policy.toml")).unwrap();
    if pol.contains("tokens = true") {
        fx.set_policy("tokens = true", "tokens = false");
    }
    let workspace = fx.root.join("factory/workspace.toml").exists();
    let started = |u: &str| {
        bf::events::replay(&fx.project())
            .map(|s| s.units.contains_key(u))
            .unwrap_or(false)
    };
    if !workspace && fx.must(&["status", "--json"]).json()["run"].is_string() && started("U-1") {
        return receipt_sandbox_in_copy(fx);
    }
    if !workspace && fx.must(&["status", "--json"]).json()["run"].is_string() {
        let wc = PathBuf::from(
            fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
                .as_str()
                .unwrap(),
        );
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
        let o = fx.bf_in(
            &wc,
            &[
                "evidence",
                "run",
                "X-1",
                "--unit",
                "U-1",
                "--",
                "test",
                "-f",
                "src/hello.txt",
            ],
        );
        assert_eq!(o.code, 0, "{}", o.all());
        return receipt_sandbox_of(fx, "X-1");
    }
    let mut recs: Vec<(&str, &str)> = vec![
        ("requirement", "id=\"R-900\"\nkind=\"requirement\"\ntitle=\"sandbox probe\"\n"),
        ("check", "id=\"X-900\"\nkind=\"check\"\ntitle=\"true\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-900\"]\nscope=\"checkpoint\"\n"),
    ];
    if workspace {
        recs = vec![
            ("requirement", "id=\"R-900\"\nkind=\"requirement\"\ntitle=\"sandbox probe\"\n"),
            ("unit", "id=\"U-900\"\nkind=\"unit\"\ntitle=\"Probe\"\ndelivers=[\"R-900\"]\nfiles=[\"src/probe.txt\"]\ncheckpoint=\"K-900\"\nestimate=1\n"),
            ("check", "id=\"X-900\"\nkind=\"check\"\ntitle=\"true\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-900\"]\nscope=\"unit\"\nunit=\"U-900\"\n"),
            ("check", "id=\"X-901\"\nkind=\"check\"\ntitle=\"true again\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-900\"]\nscope=\"checkpoint\"\n"),
            ("checkpoint", "id=\"K-900\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Probe\"\nafter=[\"U-900\"]\nproves=[\"R-900\"]\nchecks=[\"X-901\"]\n"),
        ];
    }
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("sandbox probe check");
    let o = if workspace {
        fx.start_test_run();
        let wc = PathBuf::from(
            fx.must(&["unit", "start", "U-900", "--json"]).json()["working_copy"]
                .as_str()
                .unwrap(),
        );
        fx.bf_in(
            &wc,
            &["evidence", "run", "X-900", "--unit", "U-900", "--", "true"],
        )
    } else {
        fx.bf(&["evidence", "run", "X-900", "--", "true"])
    };
    assert_eq!(o.code, 0, "{}", o.all());
    receipt_sandbox_of(fx, "X-900")
}

/// `receipt_sandbox` on a run whose units are already at work or landed (a lifecycle state, see
/// `Fx::state_copy`): a command check of a unit whose working copy is there runs in it, or else a command
/// check of a checkpoint whose working copy is there runs in that, each with its record's own command.
/// The check may fail; its receipt still says how it ran.
fn receipt_sandbox_in_copy(fx: &Fx) -> String {
    let checks = fx
        .must(&["record", "list", "--kind", "check", "--json"])
        .json();
    let checks = checks.as_array().cloned().unwrap_or_default();
    let words = |c: &serde_json::Value| -> Vec<String> {
        c["command"]
            .as_str()
            .unwrap_or("")
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    };
    let run = |dir: &Path, id: &str, unit: Option<&str>, cmd: Vec<String>| {
        let mut a: Vec<&str> = vec!["evidence", "run", id];
        if let Some(u) = unit {
            a.extend(["--unit", u]);
        }
        let count = || bf::evidence::load_receipts(&fx.project()).unwrap().len();
        let n = count();
        let mut b = a.clone();
        b.push("--");
        b.extend(cmd.iter().map(|s| s.as_str()));
        let mut o = fx.bf_in(dir, &b);
        // A check that already ran on this content runs again only as a stated repeat.
        if let Some(at) = o.stderr.split("--attempt ").nth(1) {
            let at: String = at.chars().take_while(|c| c.is_ascii_digit()).collect();
            a.extend(["--attempt", &at, "--reason", "sandbox probe", "--"]);
            a.extend(cmd.iter().map(|s| s.as_str()));
            o = fx.bf_in(dir, &a);
        }
        assert!(count() > n, "the check left no receipt: {}", o.all());
        receipt_sandbox_of(fx, id)
    };
    for c in &checks {
        let (Some(u), Some(id)) = (c["unit"].as_str(), c["id"].as_str()) else {
            continue;
        };
        if c["how"] == "command" && fx.work(u).is_dir() {
            return run(&fx.work(u), id, Some(u), words(c));
        }
    }
    let cps = fx
        .must(&["record", "list", "--kind", "checkpoint", "--json"])
        .json();
    for k in cps.as_array().cloned().unwrap_or_default() {
        let kid = k["id"].as_str().unwrap_or("");
        if !fx.work(kid).is_dir() {
            continue;
        }
        for id in k["checks"].as_array().cloned().unwrap_or_default() {
            if let Some(c) = checks
                .iter()
                .find(|c| c["id"] == id && c["how"] == "command")
            {
                return run(&fx.work(kid), id.as_str().unwrap(), None, words(c));
            }
        }
    }
    panic!("no working copy of this run has a command check to run")
}

/// The `sandbox` value of the last receipt of a check.
pub fn receipt_sandbox_of(fx: &Fx, check: &str) -> String {
    let rcs = bf::evidence::load_receipts(&fx.project()).unwrap();
    let rc = rcs
        .iter()
        .rev()
        .find(|r| r.check() == check)
        .expect("no receipt for the probe check");
    rc.v["sandbox"]
        .as_str()
        .unwrap_or_else(|| panic!("the receipt names no sandbox: {}", rc.v))
        .to_string()
}

/// On macOS with `BF_TEST_SANDBOX_REQUIRE=1`, fail (never skip) unless the environment's switch is
/// `all`, the probe succeeds, and a check's receipt on `fx` says `sandbox-exec`. Without the variable,
/// or on another system, it asserts nothing.
pub fn require_sandbox_from_env(fx: &Fx) {
    if !cfg!(target_os = "macos") || std::env::var("BF_TEST_SANDBOX_REQUIRE").as_deref() != Ok("1")
    {
        return;
    }
    assert_eq!(
        sandbox_switch(),
        Switch::All,
        "BF_TEST_SANDBOX_REQUIRE=1 needs BF_TEST_SANDBOX=all"
    );
    assert!(
        sandbox_probe(),
        "BF_TEST_SANDBOX_REQUIRE=1: sandbox-exec does not start here, so the sandbox is not met"
    );
    let s = receipt_sandbox(fx);
    assert_eq!(
        s, "sandbox-exec",
        "BF_TEST_SANDBOX_REQUIRE=1: the check ran without the sandbox"
    );
}

// ---------------------------------------------------------------------------------------------------
// The template store (D-140): `Fx::new`, `Fx::with_checks`, `Fx::workspace` and their forms copy a
// project built once per key by a real `git init` and `bf init`, instead of running them every time.
// `Fx::tiny_project` and `Fx::tiny_run` copy one built further by `tiny()` (and `start_test_run()`).

/// The text of this file, hashed from memory into every template key.
const COMMON_TEXT: &str = include_str!("mod.rs");

/// The stand-in for the transcript root in the policy text a key is taken from: the real root names
/// the build folder, which differs for every build.
const TR_PLACEHOLDER: &str = "@TRANSCRIPT_ROOT@";

fn sha_hex(parts: &[&[u8]]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The `bf` binary's identity: its path, size, modification time and inode, read with one `stat`.
/// The binary itself is never read.
pub fn binary_identity() -> String {
    use std::os::unix::fs::MetadataExt;
    let m = fs::metadata(bin()).expect("stat the bf binary");
    format!(
        "{}|{}|{}.{}|{}",
        bin(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ino()
    )
}

/// What a template is made by: the constructor's kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `Fx::new`: named checks off.
    New,
    /// `Fx::with_checks`: the catalogue's defaults.
    WithChecks,
    /// `Fx::workspace`: a workspace pointing at these sibling repositories, named checks off.
    Workspace(Vec<String>),
    /// `Fx::tiny_project`: `Fx::new`, then `tiny()` and its commit.
    Tiny,
    /// `Fx::tiny_run`: `Fx::new`, then `tiny()` and `start_test_run()`.
    TinyRun,
}

impl Kind {
    fn name(&self) -> String {
        match self {
            Kind::New => "new".into(),
            Kind::WithChecks => "with_checks".into(),
            Kind::Workspace(r) => format!("workspace:{}", r.join(",")),
            Kind::Tiny => "tiny".into(),
            Kind::TinyRun => "tiny_run".into(),
        }
    }
    fn top(&self) -> &'static str {
        match self {
            Kind::Workspace(_) => "ws",
            _ => "proj",
        }
    }
    /// The real build: today's `git init`, `bf init` and the constructor's own steps.
    fn build_real(&self, sw: Switch) -> Fx {
        let sandbox = sw == Switch::All;
        match self {
            Kind::New => {
                let fx = Fx::build_with(true, &[], sandbox);
                fx.checks_all_off();
                fx
            }
            Kind::WithChecks => Fx::build_with(true, &[], sandbox),
            Kind::Workspace(r) => {
                let repos: Vec<&str> = r.iter().map(|s| s.as_str()).collect();
                let fx = Fx::build_with(true, &repos, sandbox);
                fx.checks_all_off();
                fx
            }
            // Built by the same helper calls a test makes, never by writing records or events.
            Kind::Tiny => {
                let fx = Kind::New.build_real(sw);
                fx.tiny();
                fx
            }
            Kind::TinyRun => {
                let fx = Kind::New.build_real(sw);
                fx.tiny();
                fx.start_test_run();
                fx
            }
        }
    }
}

/// An open lock file held with `flock`; dropping it releases the lock.
struct Flock(fs::File);

impl Flock {
    fn take(path: &Path, exclusive: bool) -> Flock {
        use std::os::unix::io::AsRawFd;
        let f = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        let op = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        loop {
            // SAFETY: the descriptor is open for the life of `f`.
            if unsafe { libc::flock(f.as_raw_fd(), op) } == 0 {
                break;
            }
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::Interrupted {
                panic!("flock {}: {e}", path.display());
            }
        }
        Flock(f)
    }
}

impl Drop for Flock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: the descriptor is still open.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

type Pause = Option<Box<dyn Fn()>>;

thread_local! {
    static COPY_PAUSE: std::cell::RefCell<Pause> = const { std::cell::RefCell::new(None) };
    static BUILD_PAUSE: std::cell::RefCell<Pause> = const { std::cell::RefCell::new(None) };
}

/// Test only: a pause this thread runs inside each copy out of the store, while the copy holds the
/// store lock.
pub fn set_copy_pause(p: Pause) {
    COPY_PAUSE.with(|c| *c.borrow_mut() = p);
}

/// Test only: a pause this thread runs inside each template build, after the real build and before
/// the template is put into the store.
pub fn set_build_pause(p: Pause) {
    BUILD_PAUSE.with(|c| *c.borrow_mut() = p);
}

fn run_pause(which: &'static std::thread::LocalKey<std::cell::RefCell<Pause>>) {
    which.with(|c| {
        if let Some(f) = c.borrow().as_ref() {
            f()
        }
    });
}

/// Copy a folder tree, keeping modes and links.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap_or_else(|e| panic!("create {}: {e}", to.display()));
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let ft = e.file_type().unwrap();
        let dst = to.join(e.file_name());
        if ft.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(e.path()).unwrap(), &dst).unwrap();
        } else if ft.is_dir() {
            copy_tree(&e.path(), &dst);
        } else if let Err(err) = fs::copy(e.path(), &dst) {
            // A lock file git removed while the tree was read is not part of the project.
            if err.kind() != std::io::ErrorKind::NotFound {
                panic!("copy {}: {err}", e.path().display());
            }
        }
    }
    fs::set_permissions(to, fs::metadata(from).unwrap().permissions()).unwrap();
}

/// A store of templates: a folder and the binary identity its keys are made for.
pub struct TemplateStore {
    pub dir: PathBuf,
    pub ident: String,
}

impl TemplateStore {
    /// The store every fixture uses: `target/tmp/bf-fixture-templates`, for the current binary.
    pub fn shared() -> TemplateStore {
        TemplateStore {
            dir: Path::new(env!("CARGO_TARGET_TMPDIR")).join("bf-fixture-templates"),
            ident: binary_identity(),
        }
    }

    /// A store in another folder, for a given binary identity (the store's own tests).
    pub fn at(dir: &Path, ident: &str) -> TemplateStore {
        TemplateStore {
            dir: dir.to_path_buf(),
            ident: ident.to_string(),
        }
    }

    fn ident_prefix(&self) -> String {
        sha_hex(&[self.ident.as_bytes()])[..12].to_string()
    }

    /// The key of a template: the binary identity, the text of this file, the constructor's kind and
    /// its policy text (the sandbox setting included). The name starts with the identity's own part.
    pub fn key(&self, kind: &Kind, sw: Switch) -> String {
        self.key_named(&kind.name(), sw)
    }

    /// The key of a template made by the named builder: a constructor's kind, or a lifecycle state's
    /// build function (`state:<name>`).
    pub fn key_named(&self, name: &str, sw: Switch) -> String {
        let pol = fixture_policy(true, sw == Switch::All, TR_PLACEHOLDER);
        let k = sha_hex(&[
            self.ident.as_bytes(),
            COMMON_TEXT.as_bytes(),
            name.as_bytes(),
            pol.as_bytes(),
        ]);
        format!("{}-{}", self.ident_prefix(), &k[..24])
    }

    fn lock_path(&self) -> PathBuf {
        self.dir.join("store.lock")
    }

    /// Copy the template of the key out of the store into a new temporary folder, holding the store
    /// lock shared. None when the store does not hold the key.
    fn try_copy(&self, key: &str) -> Option<TempDir> {
        let _l = Flock::take(&self.lock_path(), false);
        let tpl = self.dir.join(key);
        if !tpl.is_dir() {
            return None;
        }
        let tmp = TempDir::new().unwrap();
        let dst = fs::canonicalize(tmp.path()).unwrap();
        for e in fs::read_dir(&tpl).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                copy_tree(&p, &dst.join(e.file_name()));
            } else {
                fs::copy(&p, dst.join(e.file_name())).unwrap();
            }
        }
        run_pause(&COPY_PAUSE);
        Some(tmp)
    }

    /// Build the key's template with `make`; the caller holds the key's build lock. The build folder
    /// is removed once the template is taken, unless `keep` (a test-only form) leaves it on disk.
    fn build(&self, key: &str, make: &dyn Fn() -> Fx, keep: bool) {
        // A temporary folder left by a builder that died.
        let partial_prefix = format!("{key}.partial-");
        for e in fs::read_dir(&self.dir).unwrap().flatten() {
            if e.file_name().to_string_lossy().starts_with(&partial_prefix) {
                let _ = fs::remove_dir_all(e.path());
            }
        }
        let fx = make();
        run_pause(&BUILD_PAUSE);
        let Fx {
            _tmp, scratch, tr, ..
        } = fx;
        let build_dir = fs::canonicalize(_tmp.path()).unwrap();
        let partial = self
            .dir
            .join(format!("{partial_prefix}{}", std::process::id()));
        copy_tree(&build_dir, &partial);
        drop(tr);
        if keep {
            drop(scratch);
            let _ = _tmp.keep();
            self.put_in_place(key, &partial);
            return;
        }
        _tmp.close().unwrap();
        drop(scratch);
        assert!(!build_dir.exists(), "the build folder is still there");
        self.put_in_place(key, &partial);
    }

    /// Move a finished template into place, holding the store lock exclusive for the rename only, and
    /// remove the templates of another binary.
    fn put_in_place(&self, key: &str, partial: &Path) {
        {
            let _x = Flock::take(&self.lock_path(), true);
            fs::rename(partial, self.dir.join(key)).unwrap();
        }
        self.remove_other_binaries();
    }

    /// Remove the templates of another binary identity, each with the store lock held exclusive.
    fn remove_other_binaries(&self) {
        let mine = format!("{}-", self.ident_prefix());
        for e in fs::read_dir(&self.dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.contains(".partial-")
                || name.ends_with(".lock")
                || name.starts_with(&mine)
                || !e.file_type().map(|t| t.is_dir()).unwrap_or(false)
            {
                continue;
            }
            let _x = Flock::take(&self.lock_path(), true);
            let _ = fs::remove_dir_all(e.path());
        }
    }

    /// A copy of the key's template in a new temporary folder, built first by `make` if the store does
    /// not hold it: one builder per key under the key's own build lock, outside the store lock.
    fn copy_out(&self, key: &str, make: &dyn Fn() -> Fx, keep: bool) -> TempDir {
        fs::create_dir_all(&self.dir).unwrap();
        loop {
            if let Some(t) = self.try_copy(key) {
                return t;
            }
            let _b = Flock::take(&self.dir.join(format!("{key}.build.lock")), true);
            if self.dir.join(key).is_dir() {
                continue;
            }
            self.build(key, make, keep);
        }
    }

    /// A fixture copied from the key's template, built first if the store does not hold it. Also
    /// gives the build folder the template was made in (it no longer exists).
    pub fn fixture(&self, kind: &Kind, sw: Switch) -> (Fx, PathBuf) {
        let key = self.key(kind, sw);
        let tmp = self.copy_out(&key, &|| kind.build_real(sw), false);
        let base = fs::canonicalize(tmp.path()).unwrap();
        let fx = Fx {
            _tmp: tmp,
            scratch: TempDir::new().unwrap(),
            root: base.join(kind.top()),
            state_base: base.join("state"),
            tr: None,
        };
        let old = fx.adopt_copy();
        (fx, old)
    }
}

impl Fx {
    /// The one step a copy needs: its policy names the build folder's transcript root, so the policy
    /// is changed to this copy's and accepted through the engine with the human's words, and the
    /// last commit (`bf init`, or the tiny project's records) takes it. Returns the build folder.
    fn adopt_copy(&self) -> PathBuf {
        self.retarget_transcripts(&self.transcript_root())
    }

    /// Point the policy's transcript root at `root`, the human's way, and amend the last commit with
    /// it (see `adopt_copy`). Returns the folder the old root lay in.
    fn retarget_transcripts(&self, root: &Path) -> PathBuf {
        let pol = fs::read_to_string(self.root.join("factory/policy.toml")).unwrap();
        let old_line = pol
            .lines()
            .find(|l| l.starts_with("transcript_root = "))
            .expect("the template's policy names no transcript root")
            .to_string();
        let old_tr: String = toml::from_str::<toml::Table>(&old_line).unwrap()["transcript_root"]
            .as_str()
            .unwrap()
            .to_string();
        let build_dir = Path::new(&old_tr).parent().unwrap().to_path_buf();
        let new_line = format!("transcript_root = {:?}", root.to_string_lossy());
        // The files the acceptance changes that were clean in the template (the policy, and the log
        // where the template had committed it) join the commit of `bf init`, so the copy's tree is
        // as clean or as changed as a real init's.
        let before = dirty_paths(&self.root);
        self.set_policy(&old_line, &new_line);
        let mut add: Vec<String> = vec!["factory/policy.toml".into()];
        add.extend(
            dirty_paths(&self.root)
                .into_iter()
                .filter(|p| !before.contains(p) && p != "factory/policy.toml"),
        );
        let mut args: Vec<&str> = vec!["add", "--"];
        args.extend(add.iter().map(|s| s.as_str()));
        git(&self.root, &args);
        git(&self.root, &["commit", "-q", "--amend", "--no-edit"]);
        build_dir
    }
}

// ---------------------------------------------------------------------------------------------------
// Lifecycle states (D-140, F-3): `Fx::state_copy` builds a state of a run (units started, built,
// landed) once per test binary and key by a build function of the test file, and copies it per test.
// The copies live in a store of their own, keyed by the test program's identity as well, since the
// build functions live there.

/// The identity of this test program, read with one `stat`, like the `bf` binary's.
pub fn test_identity() -> String {
    use std::os::unix::fs::MetadataExt;
    let exe = std::env::current_exe().expect("the test program's path");
    let m = fs::metadata(&exe).expect("stat the test program");
    format!(
        "{}|{}|{}.{}|{}",
        exe.display(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ino()
    )
}

/// How a state is copied. The plain forms use `StateOpts::default()`: repair, canary, build folder
/// removed. The others are for the copy's own tests.
#[derive(Clone, Copy, Debug)]
pub struct StateOpts {
    /// Run `git worktree repair` after the copy.
    pub repair: bool,
    /// Run the path and order canary after the copy, and fail the test on any finding.
    pub canary: bool,
    /// Keep the build folder on disk (test only; it is built under a key of its own).
    pub keep_build: bool,
}

impl Default for StateOpts {
    fn default() -> StateOpts {
        StateOpts {
            repair: true,
            canary: true,
            keep_build: false,
        }
    }
}

/// A build function of a lifecycle state.
pub type Build = fn(&Fx);

/// The file of a stored state that names its build folder; the copy reads it and removes it.
const BUILD_PATH_FILE: &str = "build.path";

impl TemplateStore {
    /// The store every lifecycle state uses: `target/tmp/bf-fixture-states`, for the current `bf`
    /// binary and the current test program.
    pub fn states() -> TemplateStore {
        TemplateStore {
            dir: Path::new(env!("CARGO_TARGET_TMPDIR")).join("bf-fixture-states"),
            ident: format!("{}|{}", binary_identity(), test_identity()),
        }
    }
}

/// Copy every file of one folder into another (one level).
fn copy_files(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).into_iter().flatten().flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

/// Every file of a tree, never following links, never entering a folder named in `skip`.
fn files_under(dir: &Path, skip: &[&str], out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if !skip.iter().any(|s| e.file_name() == *s) {
                files_under(&p, skip, out);
            }
        } else if ft.is_file() {
            out.push(p);
        }
    }
}

/// The working copies (git worktrees) of a copied state: every `.git` file under the main checkout.
fn worktree_paths(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    files_under(root, &[".git"], &mut files);
    let mut wts: Vec<PathBuf> = files
        .into_iter()
        .filter(|p| p.file_name().map(|n| n == ".git").unwrap_or(false))
        .map(|p| p.parent().unwrap().to_path_buf())
        .collect();
    wts.sort();
    wts
}

/// The path a `.git` file or a `gitdir` file names, made absolute against the folder it lies in.
fn named_path(file: &Path) -> PathBuf {
    let t = fs::read_to_string(file).unwrap_or_default();
    let t = t.trim();
    let t = t.strip_prefix("gitdir: ").unwrap_or(t);
    let p = Path::new(t);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        file.parent().unwrap().join(p)
    }
}

/// The path and order canary of a copied state: what it finds, one line each; empty when the copy is
/// sound. `git worktree list` names only paths inside the copy; every `.git` file of a working copy
/// and every `gitdir` file under `.git/worktrees` points inside it; `bf log --verify` passes; and no
/// file holds the build folder's path, except the sealed log, its head and the receipts, which keep
/// it as history.
pub fn state_canary(fx: &Fx, build_dir: &Path) -> Vec<String> {
    let base = fs::canonicalize(fx.root.parent().unwrap()).unwrap();
    let mut found = Vec::new();
    let o = Command::new("git")
        .arg("-C")
        .arg(&fx.root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .unwrap();
    if !o.status.success() {
        found.push(format!(
            "worktree: git worktree list failed: {}",
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    for l in String::from_utf8_lossy(&o.stdout).lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            if !Path::new(p).starts_with(&base) {
                found.push(format!("worktree: git worktree list names {p}"));
            }
        }
    }
    for wt in worktree_paths(&fx.root) {
        let g = named_path(&wt.join(".git"));
        if !g.starts_with(&base) {
            found.push(format!(
                "worktree: {} points at {}",
                wt.join(".git").display(),
                g.display()
            ));
        }
    }
    for e in fs::read_dir(fx.root.join(".git/worktrees"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let f = e.path().join("gitdir");
        let g = named_path(&f);
        if !g.starts_with(&base) {
            found.push(format!(
                "worktree: {} points at {}",
                f.display(),
                g.display()
            ));
        }
    }
    let v = fx.bf(&["log", "--verify"]);
    if v.code != 0 {
        found.push(format!("log: bf log --verify failed: {}", v.all()));
    }
    // The sealed log and the receipts as each working copy checks them out from a commit are the same
    // history.
    let p = fx.project();
    let mut history = vec![p.log_path(), p.head_path()];
    let mut receipts = vec![p.receipts_dir()];
    for wt in worktree_paths(&fx.root) {
        history.push(wt.join("factory/log/events.jsonl"));
        receipts.push(wt.join("factory/receipts"));
    }
    let mut needles = vec![build_dir.to_string_lossy().to_string()];
    if let Ok(short) = build_dir.strip_prefix("/private") {
        needles.push(Path::new("/").join(short).to_string_lossy().to_string());
    }
    let mut files = Vec::new();
    files_under(&base, &[], &mut files);
    for f in files {
        if history.contains(&f) || receipts.iter().any(|r| f.starts_with(r)) {
            continue;
        }
        let Ok(bytes) = fs::read(&f) else { continue };
        if needles
            .iter()
            .any(|n| bytes.windows(n.len()).any(|w| w == n.as_bytes()))
        {
            found.push(format!(
                "path: {} holds the build folder's path",
                f.display()
            ));
        }
    }
    found
}

impl Fx {
    /// A lifecycle state built once per test program and key by `build` on `Fx::new()`, copied for this
    /// test, repaired and checked by the canary (`state_canary`). `name` names the build function and is
    /// part of the key; the sandbox follows `BF_TEST_SANDBOX`, like `Fx::new()`.
    pub fn state_copy(name: &str, build: fn(&Fx)) -> Fx {
        Fx::state_copy_with(name, sandbox_switch(), build)
    }

    /// `state_copy` with the sandbox switch given for this state (part of its key).
    pub fn state_copy_with(name: &str, sw: Switch, build: fn(&Fx)) -> Fx {
        Fx::state_copy_in(
            &TemplateStore::states(),
            name,
            sw,
            build,
            StateOpts::default(),
        )
        .0
    }

    /// The general form: a state of `store`, copied as `opts` says. Also gives the build folder.
    ///
    /// The build starts from `Fx::new_with(sw)` and first points the policy's transcript root at a
    /// folder of the store that outlives the build (`<key>.transcripts`), since the root is in the
    /// policy every working copy checks out; each copy then writes its transcript in a folder of its
    /// own below it. The build folder is copied into the store and removed, so every absolute path a
    /// copy still holds names a folder that no longer exists. After the copy, `git worktree repair`
    /// runs with every working copy's path, `bf init` writes the signed guard table again (it touches
    /// nothing else in an initialised project), and the canary runs.
    pub fn state_copy_in(
        store: &TemplateStore,
        name: &str,
        sw: Switch,
        build: fn(&Fx),
        opts: StateOpts,
    ) -> (Fx, PathBuf) {
        let kept = if opts.keep_build { ":kept" } else { "" };
        let key = store.key_named(&format!("state:{name}{kept}"), sw);
        let tr_root = store.dir.join(format!("{key}.transcripts"));
        fs::create_dir_all(&tr_root).unwrap();
        let tr_root = fs::canonicalize(&tr_root).unwrap();
        let make = || {
            let mut fx = Fx::new_with(sw);
            fx.tr = Some(TempDir::new_in(&tr_root).unwrap());
            fx.retarget_transcripts(&tr_root);
            build(&fx);
            let base = fs::canonicalize(fx.root.parent().unwrap()).unwrap();
            copy_files(&fx.transcript_root(), &base.join("transcripts"));
            fs::write(
                base.join(BUILD_PATH_FILE),
                base.to_string_lossy().as_bytes(),
            )
            .unwrap();
            fx
        };
        let tmp = store.copy_out(&key, &make, opts.keep_build);
        let base = fs::canonicalize(tmp.path()).unwrap();
        let build_dir = PathBuf::from(fs::read_to_string(base.join(BUILD_PATH_FILE)).unwrap());
        fs::remove_file(base.join(BUILD_PATH_FILE)).unwrap();
        let tr = TempDir::new_in(&tr_root).unwrap();
        copy_files(&base.join("transcripts"), tr.path());
        let fx = Fx {
            _tmp: tmp,
            scratch: TempDir::new().unwrap(),
            root: base.join("proj"),
            state_base: base.join("state"),
            tr: Some(tr),
        };
        if opts.repair {
            fx.repair_worktrees();
        }
        fx.must(&["init"]);
        fx.rewrite_unit_briefs(&build_dir);
        if opts.canary {
            let found = state_canary(&fx, &build_dir);
            assert!(
                found.is_empty(),
                "the copy of state `{name}` fails the canary:\n{}",
                found.join("\n")
            );
        }
        (fx, build_dir)
    }

    /// A unit's brief names its working copy's absolute path: each one that names the build folder is
    /// written again with `bf brief`, the engine command that writes it and nothing else.
    fn rewrite_unit_briefs(&self, build_dir: &Path) {
        let old = build_dir.to_string_lossy().to_string();
        for e in fs::read_dir(self.root.join(".bf/briefs"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".md") else {
                continue;
            };
            let Some((unit, role)) = stem.rsplit_once('-') else {
                continue;
            };
            if !unit.starts_with("U-")
                || !fs::read_to_string(e.path())
                    .unwrap_or_default()
                    .contains(&old)
            {
                continue;
            }
            self.must(&["brief", unit, "--role", role]);
        }
    }

    /// `git worktree repair` in the main checkout with the path of every working copy.
    pub fn repair_worktrees(&self) {
        let wts = worktree_paths(&self.root);
        if wts.is_empty() {
            return;
        }
        let paths: Vec<String> = wts
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let mut args: Vec<&str> = vec!["worktree", "repair"];
        args.extend(paths.iter().map(|s| s.as_str()));
        git(&self.root, &args);
    }
}

/// Switch off git's automatic maintenance in a fixture repository, so no detached `git gc` writes in
/// a build folder while its template is copied or after it is removed.
fn no_auto_maintenance(dir: &Path) {
    git(dir, &["config", "gc.auto", "0"]);
    git(dir, &["config", "maintenance.auto", "false"]);
}

/// The paths `git status` names as changed or new in a checkout.
fn dirty_paths(dir: &Path) -> Vec<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    assert!(o.status.success(), "git status failed");
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| l[3..].to_string())
        .collect()
}

/// The fixture's policy: a fixed PATH, no role tokens when `tokens_off` (the token tests turn them
/// on), the sandbox as the switch says, and the given transcript root.
fn fixture_policy(tokens_off: bool, sandbox: bool, tr_root: &str) -> String {
    let git_bin = [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/usr/bin/git",
    ]
    .iter()
    .find(|g| Path::new(g).exists())
    .unwrap_or(&"/usr/bin/git");
    bf::store::default_policy_text("/usr/bin:/bin:/usr/sbin:/sbin", git_bin)
        .replace(
            "tokens = true",
            if tokens_off {
                "tokens = false"
            } else {
                "tokens = true"
            },
        )
        .replace(
            "in_order = true",
            if tokens_off {
                "in_order = false"
            } else {
                "in_order = true"
            },
        )
        .replace(
            "sandbox = true",
            if sandbox {
                "sandbox = true"
            } else {
                "sandbox = false"
            },
        )
        .replace(
            "transcript_root = \"~/.claude/projects\"",
            &format!("transcript_root = {tr_root:?}"),
        )
}

impl Fx {
    /// A fresh git repository with one commit and `bf init` done, copied from a template (D-140).
    /// Every named check is switched off for the project: these fixtures exercise the engine's other
    /// rules. `with_checks` keeps the defaults. The sandbox follows `BF_TEST_SANDBOX` (off unless `all`).
    pub fn new() -> Fx {
        Fx::new_with(sandbox_switch())
    }

    /// The same as `new`, with the sandbox switch given for this fixture instead of the environment's.
    pub fn new_with(sw: Switch) -> Fx {
        TemplateStore::shared().fixture(&Kind::New, sw).0
    }

    /// The same as `new` followed by `tiny()`, copied from a template built by those calls (D-140).
    pub fn tiny_project() -> Fx {
        Fx::tiny_project_with(sandbox_switch())
    }

    /// `tiny_project` with the sandbox switch given for this fixture.
    pub fn tiny_project_with(sw: Switch) -> Fx {
        TemplateStore::shared().fixture(&Kind::Tiny, sw).0
    }

    /// `tiny_project` with the sandbox always on.
    pub fn tiny_project_sandboxed() -> Fx {
        Fx::tiny_project_with(Switch::All)
    }

    /// The same as `new` followed by `tiny()` and `start_test_run()`, copied from a template built by
    /// those calls (D-140).
    pub fn tiny_run() -> Fx {
        Fx::tiny_run_with(sandbox_switch())
    }

    /// `tiny_run` with the sandbox switch given for this fixture.
    pub fn tiny_run_with(sw: Switch) -> Fx {
        TemplateStore::shared().fixture(&Kind::TinyRun, sw).0
    }

    /// `tiny_run` with the sandbox always on.
    pub fn tiny_run_sandboxed() -> Fx {
        Fx::tiny_run_with(Switch::All)
    }

    /// The same as `new`, made by a real `git init` and `bf init` in this test: for the tests about
    /// init, project ids, the key, the policy's acceptance and the first events of the log.
    pub fn new_real() -> Fx {
        Fx::new_real_with(sandbox_switch())
    }

    /// `new_real` with the sandbox switch given for this fixture.
    pub fn new_real_with(sw: Switch) -> Fx {
        Kind::New.build_real(sw)
    }

    /// `with_checks` made by a real `bf init` in this test.
    pub fn with_checks_real() -> Fx {
        Kind::WithChecks.build_real(sandbox_switch())
    }

    /// `with_checks_real` with the sandbox switch given for this fixture.
    pub fn with_checks_real_with(sw: Switch) -> Fx {
        Kind::WithChecks.build_real(sw)
    }

    /// `workspace` made by a real `bf init --workspace` in this test.
    pub fn workspace_real(repos: &[&str]) -> Fx {
        Fx::workspace_real_with(repos, sandbox_switch())
    }

    /// `workspace_real` with the sandbox switch given for this fixture.
    pub fn workspace_real_with(repos: &[&str], sw: Switch) -> Fx {
        Kind::Workspace(repos.iter().map(|s| s.to_string()).collect()).build_real(sw)
    }

    /// The same as `new`, with the sandbox always on: for the tests that exercise it.
    pub fn sandboxed() -> Fx {
        Fx::new_with(Switch::All)
    }

    /// The same as `new`, with the catalogue's defaults: every named check required, and nothing
    /// decided by the human yet (`checks_decided_as_default` decides it).
    pub fn with_checks() -> Fx {
        Fx::with_checks_with(sandbox_switch())
    }

    /// `with_checks` with the sandbox switch given for this fixture.
    pub fn with_checks_with(sw: Switch) -> Fx {
        TemplateStore::shared().fixture(&Kind::WithChecks, sw).0
    }

    /// `with_checks` with the sandbox always on.
    pub fn with_checks_sandboxed() -> Fx {
        Fx::with_checks_with(Switch::All)
    }

    /// Decide the project default as the catalogue has it (nothing switched off), the way the human
    /// does: a proposal, their words, the recorded choice.
    pub fn checks_decided_as_default(&self) {
        self.must(&["checks", "propose", "--scope", "default"]);
        self.human("Yes, keep every check on.");
        self.must(&["checks", "set", "--quote", "keep every check on"]);
    }

    /// The same, with the shipped policy as it is: role tokens on and the sandbox on. Only the PATH,
    /// the git program and the transcript folder (which must live inside the temporary folder) differ.
    pub fn shipped() -> Fx {
        let fx = Fx::build_with(false, &[], true);
        fx.checks_all_off();
        fx
    }

    /// A workspace (its own git repository, with `bf init --workspace` done) that points at the named
    /// code repositories, each a sibling folder with one commit and a README.
    pub fn workspace(repos: &[&str]) -> Fx {
        Fx::workspace_with(repos, sandbox_switch())
    }

    /// `workspace` with the sandbox switch given for this fixture.
    pub fn workspace_with(repos: &[&str], sw: Switch) -> Fx {
        let kind = Kind::Workspace(repos.iter().map(|s| s.to_string()).collect());
        TemplateStore::shared().fixture(&kind, sw).0
    }

    /// `workspace` with the sandbox always on.
    pub fn workspace_sandboxed(repos: &[&str]) -> Fx {
        Fx::workspace_with(repos, Switch::All)
    }

    /// The main checkout of a code repository made by `workspace`.
    pub fn repo_dir(&self, name: &str) -> PathBuf {
        self.root.parent().unwrap().join(name)
    }

    fn build_with(tokens_off: bool, repos: &[&str], sandbox: bool) -> Fx {
        let workspace = !repos.is_empty();
        let tmp = TempDir::new().unwrap();
        let root =
            fs::canonicalize(tmp.path())
                .unwrap()
                .join(if workspace { "ws" } else { "proj" });
        fs::create_dir_all(&root).unwrap();
        if workspace {
            for r in repos {
                let d = root.parent().unwrap().join(r);
                fs::create_dir_all(&d).unwrap();
                git(&d, &["init", "-q", "-b", "main"]);
                no_auto_maintenance(&d);
                fs::write(d.join("README.md"), format!("# {r}\n")).unwrap();
                git(&d, &["add", "-A"]);
                git(&d, &["commit", "-q", "-m", "init"]);
            }
        } else {
            git(&root, &["init", "-q", "-b", "main"]);
            no_auto_maintenance(&root);
            fs::write(root.join("README.md"), "# toy\n").unwrap();
            git(&root, &["add", "-A"]);
            git(&root, &["commit", "-q", "-m", "init"]);
        }
        let state_base = root.parent().unwrap().join("state");
        let tr_root = root.parent().unwrap().join("transcripts");
        fs::create_dir_all(&tr_root).unwrap();
        // A transcript root inside the temporary folder. `bf init` keeps a policy that is already there.
        let pol = fixture_policy(tokens_off, sandbox, &tr_root.to_string_lossy());
        fs::create_dir_all(root.join("factory")).unwrap();
        fs::write(root.join("factory/policy.toml"), pol).unwrap();
        let fx = Fx {
            _tmp: tmp,
            scratch: TempDir::new().unwrap(),
            root,
            state_base,
            tr: None,
        };
        let o = if workspace {
            fx.bf(&["init", "--workspace", "--name", "ws"])
        } else {
            fx.bf(&["init", "--name", "toy"])
        };
        assert_eq!(o.code, 0, "{}", o.all());
        for r in repos {
            let o = fx.bf(&["repo", "add", r, &format!("../{r}")]);
            assert_eq!(o.code, 0, "{}", o.all());
        }
        git(&fx.root, &["add", "-A"]);
        git(&fx.root, &["commit", "-q", "-m", "bf init"]);
        fx
    }

    /// A check record for every named check in the catalogue, each proving `proves` and run by no one:
    /// enough for the plan check, and for tests that start from a project with the defaults.
    pub fn name_every_check(&self, proves: &str) {
        for (i, c) in bf::checks::all().iter().enumerate() {
            let how = c.proof.name();
            let cmd = if how == "command" {
                "command=\"true\"\n"
            } else {
                ""
            };
            let t = format!(
                "id=\"X-{}\"\nkind=\"check\"\ntitle=\"{}\"\nhow=\"{how}\"\n{cmd}proves=[\"{proves}\"]\nscope=\"checkpoint\"\nnamed=\"{}\"\n",
                90 + i,
                c.name,
                c.name
            );
            let o = self.put("check", &t);
            assert_eq!(o.code, 0, "{}", o.all());
        }
        // The plan check wants the choice decided first, through the engine, unless it already is.
        if !self.root.join("factory/checks.json").exists() {
            self.checks_decided_as_default();
        }
        // Each is listed by the last checkpoint, where a built-stage check counts.
        let list = self
            .bf(&["record", "list", "--kind", "checkpoint", "--json"])
            .json();
        let last = list.as_array().unwrap().last().unwrap().clone();
        let mut k = self
            .bf(&["record", "get", last["id"].as_str().unwrap(), "--json"])
            .json();
        let mut ids: Vec<serde_json::Value> = k["checks"].as_array().cloned().unwrap_or_default();
        for i in 0..bf::checks::all().len() {
            ids.push(serde_json::json!(format!("X-{}", 90 + i)));
        }
        k["checks"] = serde_json::Value::Array(ids);
        let f = self.root.join("k-revised.toml");
        fs::write(&f, toml::to_string(&k).unwrap()).unwrap();
        let o = self.bf(&[
            "record",
            "put",
            "checkpoint",
            "--file",
            f.to_str().unwrap(),
            "--replace",
        ]);
        assert_eq!(o.code, 0, "{}", o.all());
        let _ = fs::remove_file(&f);
        self.commit_all("named checks");
    }

    /// Switch every named check off for the project, the way the human does: a proposal, their words,
    /// the recorded choice. Tests of the engine's other rules need no named check.
    pub fn checks_all_off(&self) {
        let changes: Vec<String> = bf::checks::all()
            .iter()
            .map(|c| format!("{}=off:the test does not exercise it", c.name))
            .collect();
        let mut args: Vec<&str> = vec!["checks", "propose"];
        args.extend(changes.iter().map(|x| x.as_str()));
        self.must(&args);
        self.human("Yes, switch those checks off for this test.");
        self.must(&[
            "checks",
            "set",
            "--quote",
            "Yes, switch those checks off for this test.",
        ]);
    }

    /// Switch the design reviews off, as the human does: the test does not exercise them, and a design
    /// gate with its review on opens only after the review. The other checks stay as they are.
    pub fn design_reviews_off(&self) {
        self.must(&[
            "checks",
            "propose",
            "idea-review=off:the test does not exercise it",
            "requirements-review=off:the test does not exercise it",
            "architecture-review=off:the test does not exercise it",
            "plan-review=off:the test does not exercise it",
            "--scope",
            "default",
        ]);
        self.human("Yes, switch the design reviews off for this test.");
        self.must(&[
            "checks",
            "set",
            "--quote",
            "switch the design reviews off for this test",
        ]);
    }

    /// Switch every named check off for the active run only, the way the human does, so the run may
    /// be stopped: stopping a run asks for proof of every required check.
    pub fn run_checks_all_off(&self) {
        let changes: Vec<String> = bf::checks::all()
            .iter()
            .map(|c| format!("{}=off:the test stops the run early", c.name))
            .collect();
        let mut args: Vec<&str> = vec!["checks", "propose", "--scope", "run"];
        args.extend(changes.iter().map(|x| x.as_str()));
        self.must(&args);
        self.human("Yes, switch those checks off for this run.");
        self.must(&[
            "checks",
            "set",
            "--quote",
            "switch those checks off for this run",
        ]);
    }

    pub fn bf_in(&self, dir: &Path, args: &[&str]) -> Out {
        self.bf_stdin(dir, args, "")
    }

    pub fn bf(&self, args: &[&str]) -> Out {
        self.bf_in(&self.root, args)
    }

    pub fn bf_stdin(&self, dir: &Path, args: &[&str], stdin: &str) -> Out {
        self.bf_env(dir, args, stdin, &[])
    }

    pub fn bf_env(&self, dir: &Path, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Out {
        let mut c = Command::new(bin());
        c.env("BF_STATE_HOME", &self.state_base);
        c.args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            c.env(k, v);
        }
        let mut child = c.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
        let o = child.wait_with_output().unwrap();
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        }
    }

    /// Record the human's typed message the way Claude Code's `UserPromptSubmit` hook delivers it: the
    /// payload carries the session, the transcript path, the working directory and the prompt, and the
    /// transcript does not hold the prompt yet (the harness writes it after the hook ran). It holds the
    /// previous message, or nothing before the first one. The prompt is appended afterwards.
    pub fn human(&self, text: &str) -> Out {
        let tr = self.transcript_root().join("transcript.jsonl");
        let line = |t: &str| {
            format!(
                "{}\n",
                serde_json::json!({"type": "user", "message": {"role": "user", "content": t}})
            )
        };
        let payload = serde_json::json!({
            "session_id": "0b6f1c3e-aaaa-bbbb-cccc-1234567890ab",
            "transcript_path": tr.to_string_lossy(),
            "cwd": self.root.to_string_lossy(),
            "permission_mode": "default",
            "hook_event_name": "UserPromptSubmit",
            "prompt": text
        });
        let o = self.bf_stdin(&self.root, &["human-turn"], &payload.to_string());
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&tr)
            .unwrap();
        f.write_all(line(text).as_bytes()).unwrap();
        o
    }

    /// Where the fixture's policy says the harness keeps transcripts.
    pub fn transcript_root(&self) -> PathBuf {
        match &self.tr {
            Some(t) => fs::canonicalize(t.path()).unwrap(),
            None => self.state_base.parent().unwrap().join("transcripts"),
        }
    }

    /// Change the policy the way the human does: edit the file, say so in chat, accept it.
    pub fn set_policy(&self, from: &str, to: &str) {
        let pol = self.root.join("factory/policy.toml");
        let t = fs::read_to_string(&pol).unwrap();
        assert!(t.contains(from), "policy has no `{from}`");
        fs::write(&pol, t.replace(from, to)).unwrap();
        self.human("Yes, I accept the new policy.");
        self.must(&[
            "policy",
            "accept",
            "--quote",
            "Yes, I accept the new policy.",
        ]);
    }

    /// The per-project state folder (key, log head, role tokens).
    pub fn state_dir(&self) -> PathBuf {
        let cfg =
            std::fs::read_to_string(self.root.join("factory/config.toml")).unwrap_or_default();
        let id = cfg
            .lines()
            .find_map(|l| l.strip_prefix("project_id = "))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default();
        self.state_base.join(id)
    }

    /// The status of a gate, read from `bf status`.
    pub fn must_state_gate(&self, id: &str) -> String {
        let st = self.must(&["status", "--json"]).json();
        st["gates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == id)
            .unwrap()["status"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A project handle for tests that call the library directly.
    pub fn project(&self) -> bf::store::Project {
        let mut p = bf::store::Project::discover(&self.root).unwrap();
        p.state_base = Some(self.state_base.clone());
        p.reload_policy().unwrap();
        p
    }

    /// Puts a record. A unit of a fixture that names neither `tests` nor `full_suite` gets a `full_suite`
    /// reason, since a plan written now must name one (F-151); `put_raw` puts the text as it is.
    pub fn put(&self, kind: &str, toml_text: &str) -> Out {
        let text = if kind == "unit"
            && !toml_text.contains("tests=")
            && !toml_text.contains("tests =")
            && !toml_text.contains("full_suite")
        {
            format!("{toml_text}full_suite=\"fixture\"\n")
        } else {
            toml_text.to_string()
        };
        self.put_raw(kind, &text)
    }

    pub fn put_raw(&self, kind: &str, toml_text: &str) -> Out {
        let n = fs::read_dir(self.scratch.path()).unwrap().count();
        let f = self.scratch.path().join(format!("rec{n}.toml"));
        fs::write(&f, toml_text).unwrap();
        self.bf(&["record", "put", kind, "--file", f.to_str().unwrap()])
    }

    pub fn must(&self, args: &[&str]) -> Out {
        let o = self.bf(args);
        assert_eq!(o.code, 0, "bf {:?} failed:\n{}", args, o.all());
        o
    }

    pub fn must_in(&self, dir: &Path, args: &[&str]) -> Out {
        let o = self.bf_in(dir, args);
        assert_eq!(
            o.code,
            0,
            "bf {:?} in {} failed:\n{}",
            args,
            dir.display(),
            o.all()
        );
        o
    }

    pub fn refused(&self, args: &[&str]) -> Out {
        let o = self.bf(args);
        assert_eq!(o.code, 2, "bf {:?} should be refused:\n{}", args, o.all());
        assert_four_parts(&o.stderr);
        o
    }

    pub fn commit_all(&self, msg: &str) {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", msg]);
    }

    pub fn work(&self, id: &str) -> PathBuf {
        self.root.join(".bf/work").join(id)
    }

    /// The open checkpoint fix of K-1 passes: its builder changes `src/a.txt` to `text`, commits it, and
    /// its reviewer passes it. The findings it was opened for are then fixed, and no fix is open.
    pub fn pass_checkpoint_fix(&self, text: &str) {
        let k = self.work("K-1");
        fs::write(k.join("src/a.txt"), text).unwrap();
        git(&k, &["add", "-A"]);
        git(&k, &["commit", "-q", "-m", "fix"]);
        self.must_in(
            &k,
            &["report", "K-1", "--role", "builder", "--result", "built"],
        );
        self.must_in(
            &k,
            &["report", "K-1", "--role", "reviewer", "--result", "built"],
        );
    }

    /// The tiny project used by the end-to-end and attack tests.
    pub fn tiny(&self) {
        let recs: &[(&str, &str)] = &[
            ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/hello.txt exists\"]\n"),
            ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"A farewell exists\"\nacceptance=[\"src/bye.txt exists\"]\n"),
            ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n"),
            ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/hello.txt\"]\ncheckpoint=\"K-1\"\nestimate=2\n"),
            ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"Farewell\"\ndelivers=[\"R-2\"]\nfiles=[\"src/bye.txt\"]\ndepends_on=[\"U-1\"]\ncheckpoint=\"K-1\"\nestimate=1\n"),
            ("handoff", "id=\"H-1\"\nkind=\"handoff\"\nfrom=\"U-1\"\nto=\"U-2\"\nthing=\"the greeting wording\"\ncontract=\"Same tone as the greeting.\"\n"),
            ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
            ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"Farewell file\"\nhow=\"command\"\ncommand=\"test -f src/bye.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
            ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"ls src/hello.txt src/bye.txt\"\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nscope=\"checkpoint\"\n"),
            ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\n"),
        ];
        for (k, t) in recs {
            let o = self.put(k, t);
            assert_eq!(o.code, 0, "{}", o.all());
        }
        self.commit_all("records");
    }

    /// Grant test mode, start a run and approve the plan gate as a simulation.
    pub fn start_test_run(&self) {
        self.human("yes, make this a test run with simulated approvals");
        self.must(&[
            "test-mode",
            "grant",
            "--quote",
            "yes, make this a test run with simulated approvals",
        ]);
        self.must(&["run", "start"]);
        self.answer_plan_questions();
        self.must(&["gate", "open", "plan"]);
        self.must(&["gate", "approve", "plan", "--simulated"]);
    }

    /// The two questions of the run's plan stage, answered with no wave checks and no checks for every
    /// unit, so the plan gate can open.
    pub fn answer_plan_questions(&self) {
        self.must(&["checks", "wave", "propose", "none"]);
        self.human("No wave checks for this run.");
        self.must(&["checks", "wave", "set", "--quote", "No wave checks"]);
        self.must(&["checks", "unit", "propose", "none"]);
        self.human("No checks for every unit in this run.");
        self.must(&[
            "checks",
            "unit",
            "set",
            "--quote",
            "No checks for every unit",
        ]);
    }

    /// `bf next` shows the token of every open worker directive, every time it is asked.
    pub fn take_token(&self, unit: &str, role: &str) -> String {
        let n = self.must(&["next", "--json"]).json();
        n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["unit"] == unit && d["role"] == role)
            .unwrap_or_else(|| panic!("no {role} directive for {unit}: {n}"))["token"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A real run's way to main: close the checkpoint, the human says merge in chat, the step gate is
    /// approved with their words, and only then does `--merge` work. Returns the output of the merge.
    pub fn merge_checkpoint(&self, k: &str) -> Out {
        self.must(&["checkpoint", "close", k]);
        let gid = self
            .must(&["gate", "open", "checkpoint-step", "--ref", k])
            .stdout
            .trim()
            .to_string();
        self.human("Looks good, merge it. I approve.");
        self.must(&["gate", "approve", &gid, "--quote", "I approve"]);
        self.bf(&["checkpoint", "close", k, "--merge", "--json"])
    }

    /// Start a unit, write its file, run its check, report built and clean, and land it.
    pub fn build_unit(&self, id: &str, file: &str, check: &str, cmd: &[&str]) {
        let o = self.must(&["unit", "start", id, "--json"]);
        let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join(file), format!("{id} content\n")).unwrap();
        let mut a = vec!["evidence", "run", check, "--unit", id, "--"];
        a.extend_from_slice(cmd);
        self.must_in(&wc, &a);
        self.must(&["report", id, "--result", "built"]);
        self.must(&["report", id, "--result", "built", "--role", "reviewer"]);
        self.must(&["unit", "land", id]);
    }
}

pub fn assert_four_parts(text: &str) {
    for part in [
        "refused: ",
        "because: ",
        "do this:",
        "  1. ",
        "it worked when: ",
    ] {
        assert!(text.contains(part), "refusal lacks `{part}`:\n{text}");
    }
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The view contract's JSON Schema (`views/contract/view.schema.json`).
pub fn view_schema() -> serde_json::Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views/contract/view.schema.json");
    serde_json::from_str(&fs::read_to_string(p).expect("views/contract/view.schema.json")).unwrap()
}

pub fn assert_contract(kind: &str, v: &serde_json::Value) {
    let errs = schema::validate(&view_schema(), v);
    assert!(
        errs.is_empty(),
        "{kind} view breaks the contract:\n{}\n{v:#}",
        errs.join("\n")
    );
}

impl Fx {
    /// The same as `new`, with the project made under `/tmp`, as on the Linux runner (on macOS the
    /// temporary folder lies under `/var/folders`, and `/tmp` is a link to `/private/tmp`, where the
    /// project is then found). The folder of `TempDir` is read from `TMPDIR` when the fixture is made:
    /// a test that calls this runs in a process of its own and sets it for that process only.
    pub fn new_in_tmp() -> Fx {
        std::env::set_var("TMPDIR", "/tmp");
        let fx = Fx::new();
        assert!(
            fx.root.starts_with("/tmp") || fx.root.starts_with("/private/tmp"),
            "the project is not under /tmp: {}",
            fx.root.display()
        );
        fx
    }
}
