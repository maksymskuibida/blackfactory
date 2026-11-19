//! The strings own every text the engine shows a human (C-12, D-62): the lookup with named places,
//! and the terminal form of D-38. The table is merged from `strings/en/*.toml` by `build.rs` and
//! compiled into the binary, so nothing is parsed at run time.

include!(concat!(env!("OUT_DIR"), "/strings_table.rs"));

/// The text of a key with no places filled, or `None` for a key with no text.
pub fn raw(key: &str) -> Option<&'static str> {
    TABLE
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|i| TABLE[i].1)
}

/// Every key of the table, sorted.
pub fn keys() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|(k, _)| *k)
}

/// A value as text for a sentence of the terminal: control characters are stripped (TH-16).
pub fn clean(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).collect()
}

/// The filled text of a key. A key with no text gives an empty text, never other words; a place
/// no value is given for is filled with nothing. `{{` and `}}` write a literal brace. Each value is cleaned of control characters.
pub fn t(key: &str, values: &[(&str, &str)]) -> String {
    match raw(key) {
        Some(text) => fill(text, values),
        None => String::new(),
    }
}

/// A text with its named places filled; `{{` and `}}` write a literal brace.
pub fn fill(text: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                for n in chars.by_ref() {
                    if n == '}' {
                        break;
                    }
                    name.push(n);
                }
                if let Some((_, v)) = values.iter().find(|(n, _)| *n == name) {
                    out.push_str(&clean(v));
                }
            }
            _ => out.push(c),
        }
    }
    out
}

// The terminal form of D-38.

/// The context line: the space and the intent a command acted on, or an empty text for neither.
pub fn context_line(space: Option<&str>, intent: Option<&str>) -> String {
    match (space, intent) {
        (Some(s), Some(i)) => t("cli.context.line", &[("space", s), ("intent", i)]),
        (Some(s), None) => t("cli.context.space_only", &[("space", s)]),
        (None, Some(i)) => t("cli.context.intent_only", &[("intent", i)]),
        (None, None) => String::new(),
    }
}

/// A state as a sign with its word.
pub fn state(sign: &str, word: &str) -> String {
    t("cli.state", &[("sign", sign), ("word", word)])
}

/// An information line, kept apart from the human's steps by the note mark.
pub fn info(text: &str) -> String {
    format!("{} {}", t("cli.prefix.note", &[]), clean(text))
}

/// Rows in columns: each column as wide as its widest cell, the last column not padded, columns
/// parted by two spaces. The id stands first in each row.
pub fn columns(rows: &[Vec<String>]) -> Vec<String> {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; cols];
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(clean(c).chars().count());
        }
    }
    rows.iter()
        .map(|r| {
            let mut line = String::new();
            for (i, c) in r.iter().enumerate() {
                let c = clean(c);
                if i > 0 {
                    line.push_str("  ");
                }
                line.push_str(&c);
                if i + 1 < r.len() {
                    for _ in c.chars().count()..widths[i] {
                        line.push(' ');
                    }
                }
            }
            line
        })
        .collect()
}

/// A group: a one-line heading, then its rows aligned in columns, each line ending in a newline.
pub fn group(heading: &str, rows: &[Vec<String>]) -> String {
    let mut s = format!("{}\n", clean(heading));
    for l in columns(rows) {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

/// Groups parted by one empty line.
pub fn groups(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}
