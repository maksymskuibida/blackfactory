//! `bf preset`: the legal presets. A preset is one data file per document and language in a
//! `presets/` folder: `<kind>.<language>.toml` with the title, the draft mark, the named blanks and
//! the body. `bf preset list` shows them; `bf preset fill <kind> <language> <values-file> [<out>]`
//! fills the blanks from a JSON or TOML file of values and writes a draft that always carries the
//! draft mark at the top and at the end. A new document or language is a new file; no engine code
//! changes (R-21, C-10).
//!
//! Where the presets are looked up, in this order: `presets/` beside the running engine (and one or
//! two folders above it), the folder named by `BF_PRESETS_DIR`, and `presets/` in the current
//! folder and the folders above it up to the repository root (the first folder with `.git`; with
//! none, the current folder only). The first file for a kind and language wins and a later file for
//! the same kind and language is refused as invalid, so a file elsewhere cannot replace a shipped
//! preset. A file whose kind and language do not match its name is refused, and so is a draft mark
//! that does not say DRAFT or ENTWURF. `fill` prints the file it used.

use crate::refusal::{err, refuse, R};
use crate::util;
use serde_json::{json, Value};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One preset, read and checked.
struct Preset {
    kind: String,
    language: String,
    title: String,
    draft_mark: String,
    blanks: Vec<(String, String)>,
    body: String,
    file: PathBuf,
    /// True when the file is one of the presets that ship with the engine.
    shipped: bool,
}

/// The folders to read, each with whether it holds the presets that ship with the engine. The
/// shipped folders come first and win: a later file for the same kind and language is refused, so
/// a file elsewhere cannot replace a shipped preset.
fn search_dirs() -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = vec![];
    // Shipped: `presets/` beside the running program, one and two folders above it (a source
    // checkout has them above `target/`; an installation can put them beside `bin/`). Never
    // further up, and never above a repository root.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for a in dir.ancestors().take(3) {
                v.push((a.join("presets"), true));
                if a.join(".git").exists() {
                    break;
                }
            }
        }
    }
    if let Ok(d) = std::env::var("BF_PRESETS_DIR") {
        if !d.trim().is_empty() {
            v.push((PathBuf::from(d), false));
        }
    }
    // The project's own folder: the current folder and the folders above it, up to and including
    // the repository root (the first folder that holds `.git`), and never above it. With no
    // repository root, only the current folder.
    if let Ok(c) = std::env::current_dir() {
        let mut up: Vec<PathBuf> = vec![];
        let mut rooted = false;
        for a in c.ancestors() {
            up.push(a.join("presets"));
            if a.join(".git").exists() {
                rooted = true;
                break;
            }
        }
        if !rooted {
            up.truncate(1);
        }
        v.extend(up.into_iter().map(|p| (p, false)));
    }
    let mut seen = BTreeSet::new();
    v.retain(|(p, _)| seen.insert(p.clone()));
    v
}

fn is_blank_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// The names written as `{{NAME}}` in a text, in order of first use.
fn placeholders(body: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut rest = body;
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => {
                let name = after[..j].trim().to_string();
                if !out.contains(&name) {
                    out.push(name);
                }
                rest = &after[j + 2..];
            }
            None => break,
        }
    }
    out
}

fn load(path: &Path) -> Result<Preset, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read: {e}"))?;
    let v: toml::Table = text.parse().map_err(|e| format!("not valid TOML: {e}"))?;
    let get = |k: &str| -> Result<String, String> {
        match v.get(k).and_then(|x| x.as_str()).map(|x| x.trim()) {
            Some(x) if !x.is_empty() => Ok(x.to_string()),
            _ => Err(format!("`{k}` is missing or empty")),
        }
    };
    let kind = get("kind")?;
    let language = get("language")?;
    let expected = format!("{kind}.{language}");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem != expected {
        return Err(format!(
            "the file is named `{stem}.toml` but holds kind `{kind}` and language `{language}`; name it `{expected}.toml`"
        ));
    }
    let title = get("title")?;
    let draft_mark = get("draft_mark")?;
    let upper = draft_mark.to_uppercase();
    if !(upper.contains("DRAFT") || upper.contains("ENTWURF")) {
        return Err(
            "`draft_mark` must say DRAFT or ENTWURF, so a document made from it is plainly a draft"
                .into(),
        );
    }
    let body = get("body")?;
    let mut blanks = vec![];
    if let Some(arr) = v.get("blanks").and_then(|x| x.as_array()) {
        for b in arr {
            let name = b.get("name").and_then(|x| x.as_str()).unwrap_or("").trim();
            if !is_blank_name(name) {
                return Err(format!(
                    "blank name `{name}` must be UPPER_SNAKE (letters A-Z, digits, underscore)"
                ));
            }
            let hint = b.get("hint").and_then(|x| x.as_str()).unwrap_or("").trim();
            blanks.push((name.to_string(), hint.to_string()));
        }
    }
    let declared: BTreeSet<&str> = blanks.iter().map(|b| b.0.as_str()).collect();
    if declared.len() != blanks.len() {
        return Err("a blank is declared twice".into());
    }
    for p in placeholders(&format!("{title}\n{body}")) {
        if !declared.contains(p.as_str()) {
            return Err(format!(
                "the body uses `{{{{{p}}}}}` but it is not declared in blanks"
            ));
        }
    }
    Ok(Preset {
        kind,
        language,
        title,
        draft_mark,
        blanks,
        body,
        file: path.to_path_buf(),
        shipped: false,
    })
}

/// Every preset found, and every file that is not a valid preset.
fn scan() -> (Vec<Preset>, Vec<Value>) {
    let mut found: BTreeMap<(String, String), Preset> = BTreeMap::new();
    let mut bad = vec![];
    for (d, shipped) in search_dirs() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for f in files {
            match load(&f) {
                Ok(mut p) => {
                    p.shipped = shipped;
                    match found.entry((p.kind.clone(), p.language.clone())) {
                        Entry::Vacant(e) => {
                            e.insert(p);
                        }
                        Entry::Occupied(e) if e.get().file == p.file => {}
                        Entry::Occupied(e) => bad.push(json!({
                            "file": f.display().to_string(),
                            "problem": format!(
                                "it shadows the preset `{} {}` in {}; a preset file cannot replace another, so rename its kind or language, or remove it",
                                p.kind, p.language, e.get().file.display()
                            )
                        })),
                    }
                }
                Err(why) => bad.push(json!({"file": f.display().to_string(), "problem": why})),
            }
        }
    }
    (found.into_values().collect(), bad)
}

fn list() -> R<Value> {
    let (ps, bad) = scan();
    let items: Vec<Value> = ps
        .iter()
        .map(|p| {
            json!({
                "kind": p.kind, "language": p.language, "title": p.title,
                "blanks": p.blanks.iter().map(|b| json!({"name": b.0, "hint": b.1})).collect::<Vec<_>>(),
                "file": p.file.display().to_string(),
            })
        })
        .collect();
    Ok(json!({"presets": items, "invalid": bad}))
}

fn read_values(path: &str) -> R<BTreeMap<String, String>> {
    let text = fs::read_to_string(path).map_err(|e| err(format!("cannot read {path}: {e}")))?;
    let v: Value = if path.ends_with(".toml") {
        let t: toml::Table = text
            .parse()
            .map_err(|e| err(format!("{path} is not valid TOML: {e}")))?;
        serde_json::to_value(t)?
    } else {
        serde_json::from_str(&text).map_err(|e| err(format!("{path} is not valid JSON: {e}")))?
    };
    let Some(o) = v.as_object() else {
        return Err(err(format!(
            "{path} must hold one object of blank names and values"
        )));
    };
    let mut m = BTreeMap::new();
    for (k, x) in o {
        let s = match x {
            Value::String(s) => s.clone(),
            Value::Number(_) | Value::Bool(_) => x.to_string(),
            _ => return Err(err(format!("the value of {k} in {path} must be text"))),
        };
        m.insert(k.clone(), s);
    }
    Ok(m)
}

/// Fill the body in one pass: a value that itself holds `{{` is not filled again.
fn fill_body(body: &str, vals: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => {
                let name = after[..j].trim();
                out.push_str(vals.get(name).map(|x| x.as_str()).unwrap_or(""));
                rest = &after[j + 2..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn fill(rest: &[String]) -> R<Value> {
    let need = |i: usize, what: &str| -> R<&str> {
        rest.get(i).map(|x| x.as_str()).ok_or_else(|| {
            refuse(
                "run `bf preset fill` with a missing argument",
                format!("it needs {what}"),
                &["Run `bf preset fill <kind> <language> <values-file> [<out-file>]`; `bf preset list` shows the kinds and languages."],
                "The command prints the draft, or writes it to the out file.",
            )
        })
    };
    let kind = need(0, "the kind of document")?;
    let lang = need(1, "the language")?;
    let values = need(2, "a file of values (JSON or TOML) for the blanks")?;
    let (ps, bad) = scan();
    let Some(p) = ps.iter().find(|p| p.kind == kind && p.language == lang) else {
        let mut why = format!("there is no preset for `{kind}` in `{lang}`");
        if !bad.is_empty() {
            why.push_str(&format!(
                " (and {} preset file(s) are invalid; `bf preset list` shows them)",
                bad.len()
            ));
        }
        return Err(refuse(
            format!("fill the preset {kind} {lang}"),
            why,
            &["Run `bf preset list` and use a kind and language it shows, or add a preset file."],
            "`bf preset fill` writes a draft.",
        ));
    };
    let vals = read_values(values)?;
    let missing: Vec<&str> = p
        .blanks
        .iter()
        .map(|b| b.0.as_str())
        .filter(|n| vals.get(*n).is_none_or(|v| v.trim().is_empty()))
        .collect();
    if !missing.is_empty() {
        return Err(refuse(
            format!("fill the preset {kind} {lang}"),
            format!("these blanks have no value: {}", missing.join(", ")),
            &["Add every missing blank to the values file, with the project's facts, and run the command again."],
            "`bf preset fill` writes a draft with no blank left.",
        ));
    }
    let text = format!(
        "{mark}\n\n# {title}\n\n{body}\n\n{mark}\n",
        mark = p.draft_mark,
        title = fill_body(&p.title, &vals),
        body = fill_body(&p.body, &vals).trim_end()
    );
    let mut out = json!({
        "kind": p.kind, "language": p.language, "draft": true, "draft_mark": p.draft_mark,
        "source": p.file.display().to_string(), "shipped": p.shipped,
    });
    if let Some(o) = rest.get(3) {
        util::atomic_write(Path::new(o), text.as_bytes())?;
        out["written"] = json!(o);
    } else {
        out["text"] = json!(text);
    }
    Ok(out)
}

/// Entry point of `bf preset <sub> ...`.
pub fn run(sub: &str, rest: &[String]) -> R<Value> {
    match sub {
        "list" | "" => list(),
        "fill" => fill(rest),
        _ => Err(refuse(
            format!("run `bf preset {sub}`"),
            "this command group has `list` and `fill` only",
            &["Run `bf preset list`, or `bf preset fill <kind> <language> <values-file> [<out-file>]`."],
            "The command prints its result.",
        )),
    }
}
