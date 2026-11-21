//! Two jobs. It lists every test file in `tests/` so that they build into one test program
//! (`tests/all.rs`), and it merges `strings/en/*.toml` into one sorted table that the binary holds
//! (C-12, D-62). The build fails, naming the key, for a doubled key, an empty text, a broken alias,
//! a malformed named place and a named place that a caller does not fill. `{{` and `}}` write a
//! literal brace.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=tests");
    println!("cargo:rerun-if-changed=strings");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    test_modules(&manifest, &out_dir);
    strings_table(&manifest, &out_dir);
    adapter_data(&manifest, &out_dir);
}

/// `harness/claude/adapter.toml` compiled into `adapter::DATA` (C-24): the build fails, naming the
/// key, for a missing or empty one.
fn adapter_data(manifest: &Path, out_dir: &Path) {
    let path = manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(manifest)
        .join("harness")
        .join("claude")
        .join("adapter.toml");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| fail(format!("cannot read {}: {e}", path.display())));
    let table: toml::Table = text
        .parse()
        .unwrap_or_else(|e| fail(format!("adapter.toml is not valid TOML: {e}")));
    let text_of = |k: &str| -> String {
        match table.get(k).and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => fail(format!("adapter.toml: key `{k}` is missing or not a text")),
        }
    };
    let list_of = |k: &str| -> Vec<String> {
        let items: Vec<String> = table
            .get(k)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        if items.is_empty() {
            fail(format!(
                "adapter.toml: key `{k}` is missing or has no texts"
            ));
        }
        items
    };
    let mut out = String::from("pub static DATA: Data = Data {\n");
    for k in [
        "session_dir",
        "session_file",
        "key_pid",
        "key_proc_start",
        "key_session_id",
        "fallback_variable",
        "config_variable",
        "transcript_dir",
    ] {
        writeln!(out, "    {k}: {:?},", text_of(k)).unwrap();
    }
    for k in [
        "subagent_fields",
        "programs",
        "absent_commands",
        "nested_forms",
        "ask_modes",
    ] {
        writeln!(out, "    {k}: &{:?},", list_of(k)).unwrap();
    }
    out.push_str("};\n");
    std::fs::write(out_dir.join("adapter_data.rs"), out).unwrap();
}

fn test_modules(manifest: &Path, out_dir: &Path) {
    let tests = manifest.join("tests");
    let mut names: Vec<String> = std::fs::read_dir(&tests)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "rs"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|n| n != "all")
        .collect();
    names.sort();

    let mut out = String::new();
    for n in &names {
        let path = tests.join(format!("{n}.rs"));
        writeln!(out, "#[path = {:?}]\nmod {n};", path.to_string_lossy()).unwrap();
    }
    std::fs::write(out_dir.join("test_modules.rs"), out).unwrap();
}

fn fail(msg: String) -> ! {
    eprintln!("strings error: {msg}");
    std::process::exit(1);
}

/// The places a value names. `{{` and `}}` write a literal brace; every other `{name}` must be a
/// well-formed place: lowercase letters, digits and `_`.
fn places(key: &str, value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            '}' => fail(format!(
                "key `{key}`: a `}}` closes no place (write `}}}}` for a literal brace)"
            )),
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for n in chars.by_ref() {
                    if n == '}' {
                        closed = true;
                        break;
                    }
                    name.push(n);
                }
                if !closed {
                    fail(format!(
                        "key `{key}`: a `{{` opens a place that is never closed (write `{{{{` for a literal brace)"
                    ));
                }
                let ok = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                if !ok {
                    fail(format!(
                        "key `{key}`: the place `{{{name}}}` is not a lowercase name that a caller can fill"
                    ));
                }
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            _ => {}
        }
    }
    out
}

/// One call of `t("key", &[("name", ..)])` or `from_key("stem", &[..])` with a literal key and a
/// literal list of values, found in the source: the key and the names it passes.
struct Call {
    key: String,
    stem: bool,
    names: Vec<String>,
    at: String,
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn calls_in(path: &Path, src: &str, out: &mut Vec<Call>) {
    for (pat, stem) in [("t(\"", false), ("from_key(\"", true)] {
        let mut from = 0;
        while let Some(i) = src[from..].find(pat) {
            let start = from + i;
            from = start + pat.len();
            let prev = src[..start].chars().next_back();
            if prev.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &src[from..];
            let Some(end) = rest.find('"') else { continue };
            let key = &rest[..end];
            if key.is_empty() || key.contains('\\') {
                continue;
            }
            let after = rest[end + 1..].trim_start();
            let Some(after) = after.strip_prefix(',') else {
                continue;
            };
            let Some(list) = after.trim_start().strip_prefix("&[") else {
                continue;
            };
            // The list ends at its matching `]`.
            let mut depth = 1;
            let mut in_str = false;
            let mut prev_c = ' ';
            let mut stop = list.len();
            for (k, c) in list.char_indices() {
                if in_str {
                    if c == '"' && prev_c != '\\' {
                        in_str = false;
                    }
                } else if c == '"' {
                    in_str = true;
                } else if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                    if depth == 0 {
                        stop = k;
                        break;
                    }
                }
                prev_c = c;
            }
            let list = &list[..stop];
            let mut names = Vec::new();
            let mut l = list;
            while let Some(p) = l.find("(\"") {
                let r = &l[p + 2..];
                let Some(q) = r.find('"') else { break };
                if r[q + 1..].trim_start().starts_with(',') {
                    names.push(r[..q].to_string());
                }
                l = &r[q + 1..];
            }
            let line = src[..start].matches('\n').count() + 1;
            out.push(Call {
                key: key.to_string(),
                stem,
                names,
                at: format!("{}:{line}", path.display()),
            });
        }
    }
}

/// A key whose text names a place that a caller in `src/` does not pass fails the build.
fn check_callers(manifest: &Path, table: &BTreeMap<String, String>) {
    let mut files = Vec::new();
    rs_files(&manifest.join("src"), &mut files);
    files.sort();
    let mut calls = Vec::new();
    for f in &files {
        if let Ok(src) = std::fs::read_to_string(f) {
            calls_in(f, &src, &mut calls);
        }
    }
    for c in &calls {
        let keys: Vec<(&String, &String)> = if c.stem {
            let prefix = format!("{}.", c.key);
            table
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .collect()
        } else {
            table.get_key_value(&c.key).into_iter().collect()
        };
        for (k, v) in keys {
            for name in places(k, v) {
                if !c.names.contains(&name) {
                    fail(format!(
                        "key `{k}`: the text names the place `{{{name}}}`, which the caller at {} does not fill",
                        c.at
                    ));
                }
            }
        }
    }
}

/// The keys that only a build with the test seam carries.
fn is_seam_key(k: &str) -> bool {
    k == "cli.version.test_seam_build"
        || k == "cli.doctor.test_seam_build"
        || k.starts_with("cli.refuse.test_seam_real_project.")
}

fn strings_table(manifest: &Path, out_dir: &Path) {
    let seam = std::env::var_os("CARGO_FEATURE_TEST_SEAM").is_some();
    let dir = manifest.join("strings").join("en");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| fail(format!("cannot read {}: {e}", dir.display())))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();

    let mut raw: BTreeMap<String, (String, String)> = BTreeMap::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let text =
            std::fs::read_to_string(f).unwrap_or_else(|e| fail(format!("cannot read {name}: {e}")));
        let table: toml::Table = text
            .parse()
            .unwrap_or_else(|e| fail(format!("{name} is not valid TOML: {e}")));
        for (k, v) in table {
            // A build without the test seam carries neither of its lines nor any text of its
            // refusal (C-3, TH-5): the keys are left out before anything reads them.
            if !seam && is_seam_key(&k) {
                continue;
            }
            let Some(s) = v.as_str() else {
                fail(format!("key `{k}` in {name}: the value is not a text"));
            };
            if s.is_empty() {
                fail(format!("key `{k}` in {name}: the text is empty"));
            }
            let valid_key = !k.is_empty()
                && k.split('.').all(|w| {
                    !w.is_empty()
                        && w.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                });
            if !valid_key {
                fail(format!(
                    "key `{k}` in {name}: a key is lowercase words joined by dots"
                ));
            }
            if let Some((other, _)) = raw.insert(k.clone(), (name.clone(), s.to_string())) {
                fail(format!("key `{k}` stands twice: in {other} and in {name}"));
            }
        }
    }

    // Resolve aliases: a value that is `@` and another key reads that key's text.
    let mut table: BTreeMap<String, String> = BTreeMap::new();
    for (k, (file, v)) in &raw {
        let mut cur = v.clone();
        let mut seen = vec![k.clone()];
        while let Some(target) = cur.strip_prefix('@') {
            let target = target.to_string();
            if seen.contains(&target) {
                fail(format!(
                    "key `{k}` in {file}: the alias loops through `{target}`"
                ));
            }
            match raw.get(&target) {
                Some((_, tv)) => cur = tv.clone(),
                None => fail(format!(
                    "key `{k}` in {file}: the alias points at `{target}`, which is no key"
                )),
            }
            seen.push(target);
        }
        places(k, &cur);
        table.insert(k.clone(), cur);
    }

    check_callers(manifest, &table);

    // The build of the test seam fails when either of its texts is empty (TH-5).
    if seam {
        for k in ["cli.version.test_seam_build", "cli.doctor.test_seam_build"] {
            if table.get(k).is_none_or(|v| v.trim().is_empty()) {
                fail(format!("key `{k}`: the test seam build needs a text"));
            }
        }
    }

    let mut out = String::from("pub(crate) static TABLE: &[(&str, &str)] = &[\n");
    for (k, v) in &table {
        writeln!(out, "    ({k:?}, {v:?}),").unwrap();
    }
    out.push_str("];\n");
    std::fs::write(out_dir.join("strings_table.rs"), out).unwrap();
}
