//! The shared compile cache (R-76, D-104): where its folder lies, who may use it and what every other
//! run is denied. The human names the sccache program in the policy; nothing else turns it on. This
//! module decides and opens folders; it starts no server (that is a later unit's).
//!
//! The folder of a project is `Library/Caches/blackfactory-cc/<project id>/<intent hash>/` under the
//! account's home (never `HOME`). Each part is made and opened one at a time without following a
//! link, and the profiles name the literal path, never a resolved one.

use crate::session::home::account_home;
use crate::store::Project;
use crate::util;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The folder that holds the caches of every project, relative to the account's home.
pub const TOP_PARTS: [&str; 3] = ["Library", "Caches", "blackfactory-cc"];

/// What a check gets of the cache.
#[derive(Clone, Debug, PartialEq)]
pub enum Use {
    /// The check may use the cache, whose folders these are.
    Units(Paths),
    /// No cache for this run, and why (the text of a strings key).
    Off(String),
}

/// The folders of one project's cache, by their literal paths.
#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    /// `<home>/Library/Caches/blackfactory-cc/<project id>/<intent hash>`
    pub folder: PathBuf,
    /// The cache itself, which the servers write.
    pub units: PathBuf,
    /// One folder for each server of a unit check.
    pub runs: PathBuf,
}

/// The folder that holds every project's cache, under `home`.
pub fn top_in(home: &Path) -> PathBuf {
    TOP_PARTS.iter().fold(home.to_path_buf(), |a, p| a.join(p))
}

/// The profile's denial of the whole cache folder while the setting is on, else nothing. It names the
/// literal path only.
pub fn profile_denial(p: &Project) -> String {
    profile_denial_in(p, account_home().as_deref())
}

/// The same, with the account's home given.
pub fn profile_denial_in(p: &Project, home: Option<&Path>) -> String {
    if p.policy.exec.compile_cache.is_empty() {
        return String::new();
    }
    match home {
        Some(h) => crate::sandbox::cache_denial(&top_in(h).to_string_lossy()),
        None => String::new(),
    }
}

/// The intents of the workspace as D-48 lays them out: a `factory/records/` folder is the intent
/// `default/first`, and each folder under `factory/spaces/<space>/intents/` is one; nothing else under
/// `factory/spaces/` counts. Sorted.
pub fn intents(p: &Project) -> Vec<(String, String)> {
    let factory = p.root.join("factory");
    let mut out = Vec::new();
    if factory.join("records").is_dir() {
        out.push(("default".to_string(), "first".to_string()));
    }
    let mut spaces: Vec<_> = fs::read_dir(factory.join("spaces"))
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    spaces.sort_by_key(|e| e.file_name());
    for sp in spaces {
        if !sp.path().is_dir() {
            continue;
        }
        let space = sp.file_name().to_string_lossy().to_string();
        let mut ints: Vec<_> = fs::read_dir(sp.path().join("intents"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        ints.sort();
        out.extend(ints.into_iter().map(|i| (space.clone(), i)));
    }
    out
}

/// The workspace's one intent as (space, intent), or nothing when there are none or several.
pub fn one_intent(p: &Project) -> Option<(String, String)> {
    let mut all = intents(p);
    if all.len() == 1 {
        all.pop()
    } else {
        None
    }
}

/// The first 8 hex digits of the SHA-256 of `<space>/<intent>`.
pub fn intent_hash(space: &str, intent: &str) -> String {
    let d = Sha256::digest(format!("{space}/{intent}").as_bytes());
    util::hex(&d)[..8].to_string()
}

fn note(key: &str, values: &[(&str, &str)]) -> String {
    crate::strings::t(&format!("cli.compile_cache.{key}"), values)
}

/// Make and open each of `parts` below `home`, one at a time, without following a link: `openat` with
/// `O_NOFOLLOW` and `O_DIRECTORY`, and `mkdirat` for a missing part. A part that is a link, is not a
/// folder or is not owned by the account's user is refused with a note. The answer is the literal
/// path (`home` joined with the parts), never a resolved one.
pub fn open_folder(home: &Path, parts: &[&str]) -> Result<PathBuf, String> {
    let cstr =
        |b: &[u8]| CString::new(b.to_vec()).map_err(|_| note("off.part_name", &[("part", "")]));
    let hc = cstr(home.as_os_str().as_bytes())?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
    // SAFETY: plain system calls on C strings that live through each call; every fd is closed below.
    let mut fd = unsafe { libc::open(hc.as_ptr(), flags) };
    if fd < 0 {
        return Err(note("off.part_open", &[("path", &home.to_string_lossy())]));
    }
    let mut path = home.to_path_buf();
    for part in parts {
        if part.is_empty() || *part == "." || *part == ".." || part.contains('/') {
            unsafe { libc::close(fd) };
            return Err(note("off.part_name", &[("part", part)]));
        }
        path.push(part);
        let shown = path.to_string_lossy().to_string();
        let name = match cstr(part.as_bytes()) {
            Ok(n) => n,
            Err(e) => {
                unsafe { libc::close(fd) };
                return Err(e);
            }
        };
        unsafe {
            libc::mkdirat(fd, name.as_ptr(), 0o700);
        }
        let next = unsafe { libc::openat(fd, name.as_ptr(), flags | libc::O_NOFOLLOW) };
        if next < 0 {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            let got =
                unsafe { libc::fstatat(fd, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
            unsafe { libc::close(fd) };
            let kind = if got != 0 {
                "off.part_open"
            } else if st.st_mode & libc::S_IFMT == libc::S_IFLNK {
                "off.part_link"
            } else {
                "off.part_not_folder"
            };
            return Err(note(kind, &[("path", &shown)]));
        }
        unsafe { libc::close(fd) };
        fd = next;
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::fstat(fd, &mut st) } == 0;
        if !ok || st.st_uid != unsafe { libc::geteuid() } {
            unsafe { libc::close(fd) };
            return Err(note("off.part_owner", &[("path", &shown)]));
        }
    }
    unsafe { libc::close(fd) };
    Ok(path)
}

/// Whether the setting names a usable program: an absolute path of a regular executable file.
pub fn usable_program(program: &str) -> bool {
    let p = Path::new(program);
    p.is_absolute()
        && fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

fn project_id_ok(id: &str) -> bool {
    id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// The parts of the project's cache folder below the home, with the setting on; the reason when the
/// cache cannot be used, whatever the run.
fn base_parts(p: &Project, home: Option<&Path>) -> Result<Vec<String>, String> {
    let program = &p.policy.exec.compile_cache;
    if !usable_program(program) {
        return Err(note("off.program", &[("program", program)]));
    }
    if home.is_none() {
        return Err(note("off.home", &[]));
    }
    let id = &p.config.project_id;
    if !project_id_ok(id) {
        return Err(note("off.project_id", &[("id", id)]));
    }
    let Some((space, intent)) = one_intent(p) else {
        return Err(note(
            "off.intents",
            &[("count", &intents(p).len().to_string())],
        ));
    };
    let mut parts: Vec<String> = TOP_PARTS.iter().map(|s| s.to_string()).collect();
    parts.push(id.clone());
    parts.push(intent_hash(&space, &intent));
    Ok(parts)
}

/// Make and open `base` and then `last` below `home`.
fn open_last(home: &Path, base: &[String], last: &str) -> Result<PathBuf, String> {
    let mut parts: Vec<&str> = base.iter().map(|s| s.as_str()).collect();
    parts.push(last);
    open_folder(home, &parts)
}

/// The folders of the project's cache with the setting on, opened in `home`.
fn folders_in(p: &Project, home: Option<&Path>) -> Result<Paths, String> {
    let base = base_parts(p, home)?;
    let home = home.unwrap_or(Path::new("/"));
    let units = open_last(home, &base, "units")?;
    let runs = open_last(home, &base, "t")?;
    let folder = units.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok(Paths {
        folder,
        units,
        runs,
    })
}

/// What a check gets of the cache, with the account's home and the sandbox's state given.
pub fn tier_with(
    p: &Project,
    home: Option<&Path>,
    sandbox: bool,
    check_scope: &str,
    unit: Option<&str>,
    dir: &Path,
) -> Use {
    let program = &p.policy.exec.compile_cache;
    if !usable_program(program) {
        return Use::Off(note("off.program", &[("program", program)]));
    }
    if !own_copy(p, check_scope, unit, dir) {
        return Use::Off(note("off.scope", &[]));
    }
    // Only a unit check in its own copy opens (and so makes) the folders, and a link shows as the
    // reason even where there is no sandbox.
    let folders = folders_in(p, home);
    match folders {
        Ok(_) if !sandbox => Use::Off(note("off.sandbox", &[])),
        Ok(f) => Use::Units(f),
        Err(why) => Use::Off(why),
    }
}

/// Is this a check of scope `unit` that runs in its unit's own working copy?
pub fn own_copy(p: &Project, check_scope: &str, unit: Option<&str>, dir: &Path) -> bool {
    check_scope == "unit" && unit.is_some_and(|u| dir == p.work_dir().join(u))
}

/// What a check gets of the cache: `Units` only for a check of scope `unit` in its own unit's copy
/// with everything usable, else `Off` with the reason.
pub fn tier(p: &Project, check_scope: &str, unit: Option<&str>, dir: &Path) -> Use {
    let sandbox = p.policy.exec.sandbox && crate::sandbox::available();
    tier_with(
        p,
        account_home().as_deref(),
        sandbox,
        check_scope,
        unit,
        dir,
    )
}

/// The note a receipt carries for a run: empty with the setting off, for a run that may use the
/// cache and for a run that is not a unit check in its own copy; else why a unit check has none.
pub fn receipt_note(p: &Project, check_scope: &str, unit: Option<&str>, dir: &Path) -> String {
    if p.policy.exec.compile_cache.is_empty() || !own_copy(p, check_scope, unit, dir) {
        return String::new();
    }
    match tier(p, check_scope, unit, dir) {
        Use::Off(why) => why,
        Use::Units(_) => String::new(),
    }
}

/// The paths a unit's builder uses: the cache and the unit's own socket in `s/`.
#[derive(Clone, Debug, PartialEq)]
pub struct BuilderPaths {
    pub units: PathBuf,
    pub socket: PathBuf,
}

/// The builder's paths for `unit` in `home`, or why there are none (R-76 acceptance 14).
pub fn builder_paths_in(
    p: &Project,
    home: Option<&Path>,
    unit: &str,
) -> Result<BuilderPaths, String> {
    let base = base_parts(p, home)?;
    let home = home.unwrap_or(Path::new("/"));
    let units = open_last(home, &base, "units")?;
    let socket = open_last(home, &base, "s")?.join(format!("{unit}.sock"));
    Ok(BuilderPaths { units, socket })
}

/// The builder's paths for `unit`.
pub fn builder_paths(p: &Project, unit: &str) -> Result<BuilderPaths, String> {
    builder_paths_in(p, account_home().as_deref(), unit)
}

/// What `bf doctor` says of the cache.
#[derive(Clone, Debug, PartialEq)]
pub struct DoctorFacts {
    /// The program the policy names, empty when the cache is off.
    pub program: String,
    /// The folders, or why the cache cannot be used.
    pub state: Result<Paths, String>,
    /// The size of `units/` in bytes.
    pub size: u64,
    /// The folders of the live servers in `t/`.
    pub servers: Vec<String>,
    /// The builders' sockets in `s/`.
    pub sockets: Vec<String>,
}

/// The facts for `bf doctor`, with the account's home given.
pub fn doctor_facts_in(p: &Project, home: Option<&Path>) -> DoctorFacts {
    let program = p.policy.exec.compile_cache.clone();
    if program.is_empty() {
        return DoctorFacts {
            program,
            state: Err(String::new()),
            size: 0,
            servers: vec![],
            sockets: vec![],
        };
    }
    let state = folders_in(p, home);
    let (mut size, mut servers, mut sockets) = (0, vec![], vec![]);
    if let Ok(f) = &state {
        size = dir_size(&f.units);
        servers = names(&f.runs);
        sockets = names(&f.folder.join("s"));
    }
    DoctorFacts {
        program,
        state,
        size,
        servers,
        sockets,
    }
}

/// The facts for `bf doctor`.
pub fn doctor_facts(p: &Project) -> DoctorFacts {
    doctor_facts_in(p, account_home().as_deref())
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

/// The size of the files under `dir`, counting no link's target.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(m) = fs::symlink_metadata(e.path()) else {
            continue;
        };
        if m.is_dir() {
            total += dir_size(&e.path());
        } else if m.is_file() {
            total += m.len();
        }
    }
    total
}
