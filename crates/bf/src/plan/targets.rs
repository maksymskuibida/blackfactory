//! U-56: the plan check reads a unit check's `cargo test` or `cargo nextest run` command against the
//! test targets the package really has (FU-30): a `--test <name>` of a target that does not exist, or a
//! name filter ending in `::` that no test file of `tests/` and no `mod` of `src/` carries.

use crate::store::{s, Project};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The test targets of a package: `[[test]]` names, plus every file of `tests/` unless `autotests = false`.
pub struct Targets {
    pub package: String,
    pub dir: PathBuf,
    pub tests: BTreeSet<String>,
    /// Names of test files, test directories and `src/` files that the run's branches hold or a unit of
    /// the plan lists in its `files` (they exist, or will, when the check runs).
    pub extra: BTreeSet<String>,
}

/// Flags of `cargo test` / `cargo nextest run` that take a value in the next word.
const VALUE_FLAGS: &[&str] = &[
    "--package",
    "-p",
    "--profile",
    "-E",
    "--filterset",
    "--test",
    "--bin",
    "--example",
    "--bench",
    "--features",
    "-F",
    "--manifest-path",
    "--target-dir",
    "--target",
    "-j",
    "--jobs",
    "--message-format",
    "--color",
    "--config-file",
    "--cargo-profile",
    "--test-threads",
    "--retries",
    "--run-ignored",
    "--partition",
    "--build-jobs",
    "--exclude",
    "--toolchain",
    "--config",
];

/// A command's reading: the package it names, the `--test` names and the `::` filters.
#[derive(Default, Debug, PartialEq)]
pub struct Read {
    pub package: Option<String>,
    pub tests: Vec<String>,
    pub filters: Vec<String>,
}

/// Read a command of `cargo test` or `cargo nextest run`; None for any other program.
pub fn read_command(words: &[String]) -> Option<Read> {
    let mut i = 0;
    if words.get(i).map(|w| w.as_str()) != Some("cargo") {
        return None;
    }
    i += 1;
    if words.get(i).is_some_and(|w| w.starts_with('+')) {
        i += 1;
    }
    match words.get(i).map(|w| w.as_str()) {
        Some("test") => i += 1,
        Some("nextest") if words.get(i + 1).map(|w| w.as_str()) == Some("run") => i += 2,
        _ => return None,
    }
    let mut r = Read::default();
    let mut after_dd = false;
    while i < words.len() {
        let w = words[i].as_str();
        i += 1;
        if w == "--" && !after_dd {
            after_dd = true;
            continue;
        }
        if !after_dd && w.starts_with('-') {
            let (flag, inline) = match w.split_once('=') {
                Some((f, v)) => (f, Some(v.to_string())),
                None => (w, None),
            };
            let takes = VALUE_FLAGS.contains(&flag);
            let value = if !takes {
                None
            } else if inline.is_some() {
                inline
            } else {
                let v = words.get(i).cloned();
                i += 1;
                v
            };
            match (flag, value) {
                ("--package" | "-p", Some(v)) => r.package = Some(v),
                ("--test", Some(v)) => r.tests.push(v),
                _ => {}
            }
            continue;
        }
        if after_dd && w.starts_with('-') {
            continue;
        }
        if w.ends_with("::") {
            r.filters.push(w.to_string());
        }
    }
    Some(r)
}

/// The test name that a path of a unit's `files` stands for, by the same rule `load` reads paths with: a
/// file of a `tests/` folder is its stem, a folder of `tests/` its own name. None for any other path.
pub fn test_name_of_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.trim().split('/').filter(|x| !x.is_empty()).collect();
    let at = parts.iter().rposition(|x| *x == "tests")?;
    let first = parts.get(at + 1)?;
    let last = at + 2 == parts.len();
    if last {
        first.strip_suffix(".rs").map(|x| x.to_string())
    } else {
        Some(first.to_string())
    }
}

/// A command of `cargo test` or `cargo nextest run` as its shape and its test names: the words with the
/// `--test <name>` pairs and the `<name>::` filters taken out, and the names they held (a filter's first
/// segment). None for a command of any other program.
pub fn command_shape(words: &[String]) -> Option<(Vec<String>, Vec<String>)> {
    let read = read_command(words)?;
    let mut names: Vec<String> = read.tests.clone();
    for f in &read.filters {
        if let Some(first) = f.split("::").find(|x| !x.is_empty()) {
            names.push(first.to_string());
        }
    }
    let mut shape = vec![];
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        i += 1;
        if w == "--test" {
            i += 1;
        } else if w.starts_with("--test=") || (w.ends_with("::") && !w.starts_with('-')) {
            // taken out
        } else {
            shape.push(w.to_string());
        }
    }
    Some((shape, names))
}

/// The manifest of the package: the one named by `--package`, or else the only package of the repository.
fn find_manifest(repo: &Path, package: Option<&str>) -> Option<(String, PathBuf, toml::Table)> {
    let mut found: Vec<(String, PathBuf, toml::Table)> = vec![];
    let mut stack = vec![(repo.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let m = d.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&m) {
            if let Ok(t) = text.parse::<toml::Table>() {
                if let Some(n) = t
                    .get("package")
                    .and_then(|p| p.get("name"))
                    .and_then(|n| n.as_str())
                {
                    found.push((n.to_string(), d.clone(), t));
                }
            }
        }
        if depth >= 3 {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() && !n.starts_with('.') && n != "target" && n != "node_modules"
                {
                    stack.push((e.path(), depth + 1));
                }
            }
        }
    }
    match package {
        Some(p) => found.into_iter().find(|f| f.0 == p),
        None if found.len() == 1 => found.pop(),
        None => None,
    }
}

fn load(repo: &Path, package: Option<&str>, extra_paths: &[String]) -> Option<Targets> {
    let (name, dir, t) = find_manifest(repo, package)?;
    let rel = dir
        .strip_prefix(repo)
        .unwrap_or(Path::new(""))
        .to_path_buf();
    let mut extra = BTreeSet::new();
    let mut extra_tests = BTreeSet::new();
    for path in extra_paths {
        let Ok(r) = Path::new(path).strip_prefix(&rel) else {
            continue;
        };
        let parts: Vec<String> = r
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let stem = |x: &str| x.strip_suffix(".rs").unwrap_or(x).to_string();
        match parts.as_slice() {
            [d, f] if d == "tests" => {
                extra_tests.insert(stem(f));
                extra.insert(stem(f));
            }
            [d, f, ..] if d == "tests" => {
                extra_tests.insert(f.clone());
                extra.insert(f.clone());
            }
            [d, .., f] if d == "src" => {
                extra.insert(stem(f));
            }
            _ => {}
        }
    }
    let auto = t
        .get("package")
        .and_then(|p| p.get("autotests"))
        .and_then(|a| a.as_bool())
        .unwrap_or(true);
    let mut tests = BTreeSet::new();
    for e in t
        .get("test")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(n) = e.get("name").and_then(|n| n.as_str()) {
            tests.insert(n.to_string());
        }
    }
    if auto {
        if let Ok(rd) = std::fs::read_dir(dir.join("tests")) {
            for e in rd.flatten() {
                let p = e.path();
                let stem = p.file_stem().map(|x| x.to_string_lossy().into_owned());
                if p.is_file() && p.extension().is_some_and(|x| x == "rs") {
                    tests.extend(stem);
                } else if p.is_dir() && p.join("main.rs").is_file() {
                    tests.extend(p.file_name().map(|x| x.to_string_lossy().into_owned()));
                }
            }
        }
    }
    if auto {
        tests.extend(extra_tests);
    }
    Some(Targets {
        package: name,
        dir,
        tests,
        extra,
    })
}

fn rs_files(d: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
}

/// Whether `name` is a test file of `tests/` (by its file, whatever `autotests` says: the filter reads
/// names of tests, and a file of `tests/` is one) or a `mod` of `src/`.
fn name_exists(t: &Targets, name: &str) -> bool {
    if t.tests.contains(name)
        || t.extra.contains(name)
        || t.dir.join("tests").join(format!("{name}.rs")).is_file()
        || t.dir.join("tests").join(name).is_dir()
    {
        return true;
    }
    let mut files = vec![];
    rs_files(&t.dir.join("src"), &mut files);
    let needle = format!("mod {name}");
    for f in files {
        if f.file_stem().is_some_and(|x| x == name) {
            return true;
        }
        let Ok(src) = std::fs::read_to_string(&f) else {
            continue;
        };
        for (at, _) in src.match_indices(&needle) {
            let rest = &src[at + needle.len()..];
            let before_ok = src[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            let after_ok = rest
                .chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// The misses of one check's command in the repository `repo`: an empty list when the command is of
/// another program, names nothing missing, or the package cannot be found.
pub fn misses(repo: &Path, check: &Value, extra_paths: &[String]) -> Vec<String> {
    let words = crate::evidence::command_words(check);
    let Some(read) = read_command(&words) else {
        return vec![];
    };
    if read.tests.is_empty() && read.filters.is_empty() {
        return vec![];
    }
    let Some(t) = load(repo, read.package.as_deref(), extra_paths) else {
        return vec![];
    };
    let id = s(check, "id");
    let mut out = vec![];
    for n in &read.tests {
        if !t.tests.contains(n) {
            let mut have: Vec<String> = t.tests.iter().cloned().collect();
            have.extend(["--lib".to_string(), "--bins".to_string()]);
            out.push(crate::strings::t(
                "cli.plan_targets.no_test_target",
                &[
                    ("check", id),
                    ("word", n),
                    ("package", &t.package),
                    ("have", &have.join(", ")),
                ],
            ));
        }
    }
    for f in &read.filters {
        let segs: Vec<&str> = f.split("::").filter(|x| !x.is_empty()).collect();
        if !segs.is_empty() && !segs.iter().any(|g| name_exists(&t, g)) {
            out.push(crate::strings::t(
                "cli.plan_targets.no_filter_match",
                &[("check", id), ("word", f), ("package", &t.package)],
            ));
        }
    }
    out
}

/// The paths of the repository on the main branch in force and on the checkpoint branches (the run's
/// branches), whatever the checked-out tree holds.
fn landed_paths(p: &Project, repo: &crate::store::Repo) -> Vec<String> {
    let git = |args: &[&str]| -> Option<String> {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo.path)
            .args(args)
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let mut refs = vec![repo.branch.clone()];
    let pat = format!("refs/heads/{}K-*", p.config.branch_prefix);
    if let Some(l) = git(&["for-each-ref", "--format=%(refname)", &pat]) {
        refs.extend(l.lines().map(|x| x.to_string()));
    }
    let mut out = BTreeSet::new();
    for r in refs {
        if let Some(l) = git(&["ls-tree", "-r", "--full-tree", "--name-only", &r]) {
            out.extend(l.lines().map(|x| x.to_string()));
        }
    }
    out.into_iter().collect()
}

/// Judge the checks of a plan: a check in `strict` gets a problem, any other a note. A check outside a
/// unit is read in the workspace's only repository, when it has one.
pub fn judge(
    p: &Project,
    units: &[Value],
    checks: &[Value],
    strict: impl Fn(&str) -> bool,
    problems: &mut Vec<String>,
    notes: &mut Vec<String>,
) {
    let mut landed: std::collections::HashMap<PathBuf, Vec<String>> = Default::default();
    for c in checks {
        if s(c, "how") != "command" {
            continue;
        }
        // Only a cargo test command that names a test target or a `::` filter can miss; any other
        // check needs neither the branches' paths nor the units' files.
        let names_a_target = read_command(&crate::evidence::command_words(c))
            .is_some_and(|r| !r.tests.is_empty() || !r.filters.is_empty());
        if !names_a_target {
            continue;
        }
        let unit = units.iter().find(|u| s(u, "id") == s(c, "unit"));
        let repo = match unit {
            Some(u) => p.repo_of_unit(u).ok(),
            None if p.repos.len() == 1 => p.repos.first(),
            None => None,
        };
        let Some(repo) = repo else { continue };
        // Files a unit of the plan lists will exist when its check runs; files on the run's branches do.
        // Gathered once for each repository.
        let paths = landed.entry(repo.path.clone()).or_insert_with(|| {
            let mut paths = landed_paths(p, repo);
            for u in units {
                if p.repo_of_unit(u).is_ok_and(|r| r.path == repo.path) {
                    paths.extend(crate::store::str_list(u, "files"));
                }
            }
            paths
        });
        for m in misses(&repo.path, c, paths) {
            if strict(s(c, "id")) {
                problems.push(m);
            } else {
                notes.push(crate::strings::t("cli.plan_targets.note", &[("text", &m)]));
            }
        }
    }
}
