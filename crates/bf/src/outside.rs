//! The files a piece of work changed outside a recommended list, with the lines changed (D-115).
//! The module knows nothing of units, fixes or syncs: a caller gives its own diff and its own list.

use crate::gitx;
use crate::strings::t;
use std::path::Path;

/// A changed file that no entry of the list matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outside {
    pub path: String,
    /// The lines changed, as inclusive ranges in the new file. A removal is the line it stood before.
    /// Empty for a file with no line change: a binary file, a mode change, an empty new file.
    pub ranges: Vec<(u32, u32)>,
    pub new: bool,
    pub deleted: bool,
    /// The diff header of the file could not be read: `path` is then the header's own text.
    pub unreadable: bool,
}

/// The files changed from `base` to `tip` that no entry of `list` matches, from one
/// `git diff --unified=0`. An empty `tip` means the working copy, and a file git does not track yet
/// is then listed as a new file. The files the engine never counts as content (OS junk) are left out.
pub fn changed(repo: &Path, base: &str, tip: &str, list: &[String]) -> Vec<Outside> {
    let mut args: Vec<String> = [
        "-c",
        "core.quotePath=false",
        "diff",
        "--unified=0",
        "--no-color",
        "--no-renames",
        base,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if !tip.is_empty() {
        args.push(tip.to_string());
    }
    args.push("--".into());
    args.push(".".into());
    args.extend(gitx::junk_excludes());
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let text = match gitx::run(repo, &a, &[]) {
        Ok(o) if o.ok => o.stdout,
        _ => return Vec::new(),
    };
    let mut out = parse_diff(&text);
    if tip.is_empty() {
        let mut args: Vec<String> = [
            "-c",
            "core.quotePath=false",
            "ls-files",
            "-z",
            "--others",
            "--exclude-standard",
            "--",
            ".",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        args.extend(gitx::junk_excludes());
        let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        if let Ok(o) = gitx::run(repo, &a, &[]) {
            for f in o.stdout.split('\0').filter(|f| !f.is_empty()) {
                let lines = std::fs::read(repo.join(f))
                    .map(|b| b.iter().filter(|c| **c == b'\n').count() as u32)
                    .unwrap_or(0);
                out.push(Outside {
                    path: f.to_string(),
                    ranges: if lines == 0 {
                        Vec::new()
                    } else {
                        vec![(1, lines)]
                    },
                    new: true,
                    deleted: false,
                    unreadable: false,
                });
            }
        }
    }
    out.retain(|o| !list.iter().any(|g| crate::util::path_matches(g, &o.path)));
    out
}

/// The path of a `diff --git a/P b/P` header (the two names are equal: renames are off). Git writes a
/// name with a quote, a backslash or a control character as a C-style quoted text, in both places.
fn header_path(rest: &str) -> String {
    if rest.starts_with('"') {
        let Some((first, after)) = unquote(rest) else {
            return String::new();
        };
        let ok = unquote(after.trim_start()).map(|(second, _)| second);
        return match (
            first.strip_prefix("a/"),
            ok.as_deref().and_then(|b| b.strip_prefix("b/")),
        ) {
            (Some(a), Some(b)) if a == b => a.to_string(),
            _ => String::new(),
        };
    }
    let Some(rest) = rest.strip_prefix("a/") else {
        return String::new();
    };
    let n = rest.len();
    if n < 4 || n % 2 == 0 {
        return String::new();
    }
    let l = (n - 3) / 2;
    match (rest.get(..l), rest.get(l..l + 3), rest.get(l + 3..)) {
        (Some(a), Some(" b/"), Some(b)) if a == b => b.to_string(),
        _ => String::new(),
    }
}

/// A C-style quoted text at the start of `t`: its bytes unescaped, and what follows the closing quote.
fn unquote(t: &str) -> Option<(String, &str)> {
    let b = t.as_bytes();
    if b.first() != Some(&b'"') {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), &t[i + 1..])),
            b'\\' => {
                i += 1;
                let c = *b.get(i)?;
                match c {
                    b'a' => out.push(7),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'v' => out.push(11),
                    b'0'..=b'7' => {
                        let mut v = 0u32;
                        let mut k = 0;
                        while k < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                            v = v * 8 + (b[i] - b'0') as u32;
                            i += 1;
                            k += 1;
                        }
                        out.push(v as u8);
                        continue;
                    }
                    other => out.push(other),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    None
}

/// Only the header lines of each block are read, and the hunk headers: a line of content starts with a
/// space, `+`, `-` or a backslash, so it can never be read as either.
/// The files of a `git diff --unified=0` text, with the lines changed.
pub fn parse_diff(text: &str) -> Vec<Outside> {
    let mut out: Vec<Outside> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = header_path(rest);
            let unreadable = path.is_empty();
            out.push(Outside {
                path: if unreadable { rest.to_string() } else { path },
                unreadable,
                ranges: Vec::new(),
                new: false,
                deleted: false,
            });
        } else if line.starts_with("new file mode") {
            if let Some(cur) = out.last_mut() {
                cur.new = true;
            }
        } else if line.starts_with("deleted file mode") {
            if let Some(cur) = out.last_mut() {
                cur.deleted = true;
            }
        } else if let Some(h) = line.strip_prefix("@@ ") {
            let plus = h.split_whitespace().find(|w| w.starts_with('+'));
            if let (Some(plus), Some(cur)) = (plus, out.last_mut()) {
                let spec = &plus[1..];
                let (from, n) = match spec.split_once(',') {
                    Some((a, b)) => (a.parse::<u32>().unwrap_or(0), b.parse::<u32>().unwrap_or(1)),
                    None => (spec.parse::<u32>().unwrap_or(0), 1),
                };
                let end = if n == 0 { from } else { from + n - 1 };
                cur.ranges.push((from, end));
            }
        }
    }
    out
}

fn ranges_text(ranges: &[(u32, u32)]) -> String {
    let parts: Vec<String> = ranges
        .iter()
        .map(|(a, b)| {
            let (from, to) = (a.to_string(), b.to_string());
            if a == b {
                t("outside.range.one", &[("from", &from)])
            } else {
                t("outside.range.many", &[("from", &from), ("to", &to)])
            }
        })
        .collect();
    parts.join(&t("outside.range.join", &[]))
}

/// A name as one line of text that cannot end its code span or start a line of the brief: a backslash,
/// a backtick and every control or line-separator character is written as an escape.
fn printable(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '`' => out.push_str("\\x60"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// The brief section for the files outside the list; `role` is `reviewer`, `builder` or `sync`.
/// An empty list gives an empty text.
pub fn section(items: &[Outside], role: &str) -> String {
    if items.is_empty() {
        return String::new();
    }
    let intro = match role {
        "builder" => t("outside.intro.builder", &[]),
        "sync" => t("outside.intro.sync", &[]),
        _ => t("outside.intro.reviewer", &[]),
    };
    let mut md = format!("## {}\n\n{}\n\n", t("outside.title", &[]), intro);
    for o in items {
        let path = printable(&o.path);
        if o.unreadable {
            md.push_str(&t("outside.item.unreadable", &[("path", &path)]));
        } else if o.new {
            md.push_str(&t("outside.item.new", &[("path", &path)]));
        } else if o.deleted {
            md.push_str(&t("outside.item.deleted", &[("path", &path)]));
        } else if o.ranges.is_empty() {
            md.push_str(&t("outside.item.other", &[("path", &path)]));
        } else {
            md.push_str(&t(
                "outside.item.lines",
                &[("path", &path), ("ranges", &ranges_text(&o.ranges))],
            ));
        }
        md.push('\n');
    }
    md.push('\n');
    md
}

/// The builder's latest report as a claim for the reviewer: its result and its note, every line of the
/// note behind `> `, cut at 2000 characters.
pub fn claim(result: &str, note: &str) -> String {
    let mut md = format!(
        "## {}\n\n{}\n\n{}\n\n",
        t("outside.claim.title", &[]),
        t("outside.claim.intro", &[]),
        t("outside.claim.result", &[("result", result)])
    );
    let note = note.trim();
    if note.is_empty() {
        return md;
    }
    let total = note.chars().count();
    let shown: String = note.chars().take(2000).collect();
    md.push_str(&format!("{}\n\n", t("outside.claim.note", &[])));
    let shown = shown.replace("\r\n", "\n").replace(
        ['\r', '\u{2028}', '\u{2029}', '\u{85}', '\u{b}', '\u{c}'],
        "\n",
    );
    for l in shown.lines() {
        md.push_str(&format!("> {l}\n"));
    }
    md.push('\n');
    if total > 2000 {
        md.push_str(&t(
            "outside.claim.cut",
            &[("count", &(total - 2000).to_string())],
        ));
        md.push_str("\n\n");
    }
    md
}
