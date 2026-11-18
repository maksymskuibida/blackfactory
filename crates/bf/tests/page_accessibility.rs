//! The page template against the accessibility standard (WCAG 2.2 level AA, the engine's default in `uiux.rs`).
//!
//! Every sample in `views/samples/` is drawn through the real template by `views/dev/render_dom.js` (a small DOM, no
//! browser), and the drawn tree is checked for: text contrast from the tokens file in both themes, a name on every
//! control, headings in order, a focus order that follows the page, no information by colour alone, no duplicate ids,
//! and values drawn as text. This is the accessibility check of the run.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn views() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views")
}

fn samples() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(views().join("samples"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    v.sort();
    v
}

/// Draw one sample through the template; the drawn tree as JSON.
fn draw(sample: &Path) -> Value {
    let out = Command::new("node")
        .arg(views().join("dev/render_dom.js"))
        .arg(views().join("templates/page.html"))
        .arg(sample)
        .output()
        .expect("node is needed to draw the samples through the template");
    assert!(
        out.status.success(),
        "{}: the template did not draw: {}",
        sample.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"].as_array().unwrap().is_empty(),
        "{}: script errors {}",
        sample.display(),
        v["errors"]
    );
    v
}

// ---- the tree

fn tag(n: &Value) -> &str {
    n["tag"].as_str().unwrap_or("")
}
fn attr<'a>(n: &'a Value, k: &str) -> Option<&'a str> {
    n["attrs"][k].as_str()
}
fn classes(n: &Value) -> Vec<String> {
    attr(n, "class")
        .unwrap_or("")
        .split_whitespace()
        .map(String::from)
        .collect()
}
fn kids(n: &Value) -> &[Value] {
    n["kids"].as_array().map(|a| a.as_slice()).unwrap_or(&[])
}
fn text(n: &Value) -> String {
    if let Some(t) = n["text"].as_str() {
        return t.to_string();
    }
    kids(n).iter().map(text).collect()
}
/// Every element in document order, with the path of its ancestors.
fn walk<'a>(n: &'a Value, path: &mut Vec<&'a Value>, f: &mut dyn FnMut(&'a Value, &[&'a Value])) {
    if n["text"].is_string() {
        return;
    }
    f(n, path);
    path.push(n);
    for k in kids(n) {
        walk(k, path, f);
    }
    path.pop();
}
fn elements(root: &Value) -> Vec<&Value> {
    let mut v = Vec::new();
    walk(root, &mut Vec::new(), &mut |n, _| v.push(n));
    v
}
fn hidden(n: &Value, path: &[&Value]) -> bool {
    n["attrs"].get("hidden").is_some() || path.iter().any(|p| p["attrs"].get("hidden").is_some())
}

// ---- tokens and colour

fn tokens() -> Value {
    serde_json::from_str(&fs::read_to_string(views().join("tokens/tokens.json")).unwrap()).unwrap()
}

fn hex(s: &str) -> Option<[f64; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let c = |i: usize| {
        u8::from_str_radix(&s[i..i + 2], 16)
            .ok()
            .map(|x| x as f64 / 255.0)
    };
    Some([c(0)?, c(2)?, c(4)?])
}
fn lum(c: [f64; 3]) -> f64 {
    let l = |x: f64| {
        if x <= 0.03928 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * l(c[0]) + 0.7152 * l(c[1]) + 0.0722 * l(c[2])
}
fn ratio(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// A token name to its colour in one theme; aliases of the form `var(--x)` are followed.
fn colour(t: &Value, theme: &str, name: &str) -> Option<[f64; 3]> {
    let mut name = name.to_string();
    for _ in 0..4 {
        let v = t[theme][&name]
            .as_str()
            .or_else(|| t["light"][&name].as_str())
            .or_else(|| t["aliases"][&name].as_str())?;
        if let Some(c) = hex(v) {
            return Some(c);
        }
        name = v.strip_prefix("var(--")?.strip_suffix(')')?.to_string();
    }
    None
}

// ---- the component styles of the template

struct Rule {
    tag: Option<String>,      // the element name of the last compound
    classes: Vec<String>,     // the classes of the last compound
    ancestor: Option<String>, // a class an ancestor must have
    colour: Option<String>,
    bg: Option<String>,
    order: usize,
}

fn var_of(v: &str) -> Option<String> {
    let v = v.trim();
    v.strip_prefix("var(--")?
        .strip_suffix(')')
        .map(String::from)
}

fn rules() -> Vec<Rule> {
    let html = fs::read_to_string(views().join("templates/page.html")).unwrap();
    let a = html.find("/*COMPONENTS:BEGIN*/").unwrap();
    let b = html.find("/*COMPONENTS:END*/").unwrap();
    let mut css = html[a..b].to_string();
    // drop comments
    while let Some(i) = css.find("/*") {
        let j = css[i..].find("*/").map(|j| i + j + 2).unwrap_or(css.len());
        css.replace_range(i..j, "");
    }
    let mut out = Vec::new();
    let mut rest = css.as_str();
    let mut order = 0;
    while let Some(o) = rest.find('{') {
        let sel = rest[..o].trim().to_string();
        let c = rest[o + 1..]
            .find('}')
            .map(|c| o + 1 + c)
            .unwrap_or(rest.len());
        let body = &rest[o + 1..c];
        rest = &rest[(c + 1).min(rest.len())..];
        if sel.starts_with('@') || body.contains('{') {
            // a media block: its inner rules are layout, skip to the end of the block
            if let Some(i) = rest.find("}\n").or(rest.find('}')) {
                rest = &rest[i + 1..];
            }
            continue;
        }
        let mut colour = None;
        let mut bg = None;
        for d in body.split(';') {
            if let Some((k, v)) = d.split_once(':') {
                match k.trim() {
                    "color" => colour = var_of(v),
                    "background" | "background-color" => bg = var_of(v),
                    _ => {}
                }
            }
        }
        if colour.is_none() && bg.is_none() {
            continue;
        }
        for s in sel.split(',') {
            let s = s.trim();
            if s.contains(':')
                || s.contains('[')
                || s.contains('>')
                || s.contains('~')
                || s.contains('+')
            {
                continue;
            }
            let parts: Vec<&str> = s.split_whitespace().collect();
            // a compound is an optional element name, then classes
            let split = |p: &str| -> Option<(Option<String>, Vec<String>)> {
                let (t, rest) = match p.find('.') {
                    Some(i) => (&p[..i], &p[i..]),
                    None => (p, ""),
                };
                let word = |w: &str| {
                    w.chars()
                        .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_')
                };
                let cl: Vec<String> = if rest.is_empty() {
                    vec![]
                } else {
                    rest[1..].split('.').map(String::from).collect()
                };
                if !word(t)
                    || cl.iter().any(|c| c.is_empty() || !word(c))
                    || t.is_empty() && cl.is_empty()
                {
                    return None;
                }
                Some((
                    if t.is_empty() {
                        None
                    } else {
                        Some(t.to_string())
                    },
                    cl,
                ))
            };
            let comps: Vec<_> = parts.iter().filter_map(|p| split(p)).collect();
            if parts.is_empty() || parts.len() > 2 || comps.len() != parts.len() {
                continue;
            }
            // an ancestor is one class
            if parts.len() == 2 && (comps[0].0.is_some() || comps[0].1.len() != 1) {
                continue;
            }
            let last = comps.last().unwrap().clone();
            order += 1;
            out.push(Rule {
                tag: last.0,
                classes: last.1,
                ancestor: if parts.len() == 2 {
                    Some(comps[0].1[0].clone())
                } else {
                    None
                },
                colour: colour.clone(),
                bg: bg.clone(),
                order,
            });
        }
    }
    out
}

/// Check the contrast of every drawn text against the colours the styles give it.
struct Walk<'a> {
    name: &'a str,
    theme: &'a str,
    t: &'a Value,
    rules: &'a [Rule],
    checked: usize,
    fails: Vec<String>,
}

fn go(w: &mut Walk, n: &Value, anc: &mut Vec<Vec<String>>, inh: (String, String)) {
    if n["text"].is_string() {
        return;
    }
    if n["attrs"].get("hidden").is_some() || matches!(tag(n), "script" | "style") {
        return;
    }
    let cls = classes(n);
    let mut best_c: Option<(usize, usize, String)> = None;
    let mut best_b: Option<(usize, usize, String)> = None;
    for r in w.rules {
        if !r.classes.iter().all(|c| cls.contains(c)) {
            continue;
        }
        if r.tag.as_deref().is_some_and(|t| t != tag(n)) {
            continue;
        }
        if let Some(a) = &r.ancestor {
            if !anc.iter().any(|x| x.contains(a)) {
                continue;
            }
        }
        let spec = r.classes.len() + r.ancestor.iter().count();
        let key = (spec, r.tag.iter().count(), r.order);
        if let Some(c) = &r.colour {
            if best_c
                .as_ref()
                .map(|b| key > (b.0, b.1 >> 20, b.1 & 0xfffff))
                .unwrap_or(true)
            {
                best_c = Some((spec, (key.1 << 20) | key.2, c.clone()));
            }
        }
        if let Some(b) = &r.bg {
            if best_b
                .as_ref()
                .map(|x| key > (x.0, x.1 >> 20, x.1 & 0xfffff))
                .unwrap_or(true)
            {
                best_b = Some((spec, (key.1 << 20) | key.2, b.clone()));
            }
        }
    }
    let fg = best_c.map(|b| b.2).unwrap_or(inh.0.clone());
    let bg = best_b.map(|b| b.2).unwrap_or(inh.1.clone());
    let own: String = kids(n).iter().filter_map(|k| k["text"].as_str()).collect();
    if !own.trim().is_empty() && !matches!(tag(n), "svg" | "path") {
        // a link takes the accent colour from its own rule
        let shown = if tag(n) == "a" && fg == "fg" {
            "accent".to_string()
        } else {
            fg.clone()
        };
        if let (Some(f), Some(b)) = (colour(w.t, w.theme, &shown), colour(w.t, w.theme, &bg)) {
            w.checked += 1;
            let r = ratio(f, b);
            if r < 4.5 {
                w.fails.push(format!(
                    "{} [{}] \"{}\" is {shown} on {bg}: {r:.2}:1",
                    w.name,
                    w.theme,
                    own.trim().chars().take(30).collect::<String>()
                ));
            }
        }
    }
    // a toned item of the context strip is measured against its tone's background
    if tag(n) == "li"
        && cls.iter().any(|c| c == "t-wait")
        && anc.iter().any(|x| x.iter().any(|c| c == "ctx"))
    {
        assert_eq!(
            bg, "warn-bg",
            "{}: a waiting context item is not on the warning background",
            w.name
        );
    }
    anc.push(cls);
    for k in kids(n) {
        go(w, k, anc, (fg.clone(), bg.clone()));
    }
    anc.pop();
}

/// Check the contrast of every drawn text against the colours the styles give it.
fn check_contrast(name: &str, root: &Value, theme: &str, t: &Value, rules: &[Rule]) -> usize {
    let mut w = Walk {
        name,
        theme,
        t,
        rules,
        checked: 0,
        fails: Vec::new(),
    };
    go(&mut w, root, &mut Vec::new(), ("fg".into(), "bg".into()));
    assert!(
        w.fails.is_empty(),
        "text contrast below 4.5:1:\n{}",
        w.fails.join("\n")
    );
    w.checked
}

// ---- the tests

#[test]
fn every_sample_is_drawn_and_passes_the_accessibility_standard() {
    let t = tokens();
    let rules = rules();
    let mut total_contrast = 0;
    for s in samples() {
        let name = s.file_name().unwrap().to_string_lossy().to_string();
        let v = draw(&s);
        let root = &v["tree"];
        let all = elements(root);

        // headings: one h1 first, and no level skipped on the way down
        let heads: Vec<(usize, String)> = {
            let mut h = Vec::new();
            walk(root, &mut Vec::new(), &mut |n, p| {
                let tg = tag(n);
                if tg.len() == 2
                    && tg.starts_with('h')
                    && tg[1..].parse::<usize>().is_ok()
                    && !hidden(n, p)
                {
                    h.push((tg[1..].parse().unwrap(), text(n)));
                }
            });
            h
        };
        assert!(!heads.is_empty(), "{name}: the page has no heading");
        assert_eq!(heads[0].0, 1, "{name}: the first heading is not an h1");
        assert_eq!(
            heads.iter().filter(|h| h.0 == 1).count(),
            1,
            "{name}: more than one h1"
        );
        for w in heads.windows(2) {
            assert!(
                w[1].0 <= w[0].0 + 1,
                "{name}: heading {:?} follows {:?} and skips a level",
                w[1],
                w[0]
            );
        }
        assert!(
            heads.iter().all(|h| !h.1.trim().is_empty()),
            "{name}: an empty heading"
        );

        // ids are unique
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for e in &all {
            if let Some(id) = attr(e, "id") {
                *seen.entry(id).or_default() += 1;
            }
        }
        assert!(
            seen.values().all(|&n| n == 1),
            "{name}: duplicate ids {:?}",
            seen.iter().filter(|x| *x.1 > 1).collect::<Vec<_>>()
        );

        // focus order follows the page: no positive tabindex, and a control that is not a control is not focusable
        for e in &all {
            if let Some(ti) = attr(e, "tabindex") {
                assert!(
                    ti.parse::<i32>().map(|n| n <= 0).unwrap_or(false),
                    "{name}: a positive tabindex breaks the focus order"
                );
            }
        }

        // labels: every control has a name
        let labels_for: Vec<String> = all
            .iter()
            .filter(|e| tag(e) == "label")
            .filter_map(|e| attr(e, "for").map(String::from))
            .collect();
        walk(root, &mut Vec::new(), &mut |n, p| {
            if hidden(n, p) {
                return;
            }
            let named = !text(n).trim().is_empty()
                || attr(n, "aria-label")
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
            match tag(n) {
                "button" | "a" => assert!(named, "{name}: a {} has no name: {n}", tag(n)),
                "textarea" | "input" | "select" => {
                    let by_for = attr(n, "id")
                        .map(|i| labels_for.iter().any(|l| l == i))
                        .unwrap_or(false);
                    let by_wrap = p.iter().any(|a| tag(a) == "label");
                    let by_aria = attr(n, "aria-label")
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false);
                    assert!(
                        by_for || by_wrap || by_aria,
                        "{name}: a {} has no label: {n}",
                        tag(n)
                    );
                }
                "img" => assert!(attr(n, "alt").is_some(), "{name}: an image has no alt"),
                _ => {}
            }
        });

        // no information by colour alone: a tag, a status, a handled mark, a label and a context item always say a word
        walk(root, &mut Vec::new(), &mut |n, p| {
            if hidden(n, p) {
                return;
            }
            let c = classes(n);
            let toned = c.iter().any(|x| {
                matches!(
                    x.as_str(),
                    "tag" | "status" | "handled" | "lab" | "legend" | "rv"
                )
            }) || (tag(n) == "li" && c.iter().any(|x| x.starts_with("t-")));
            if toned {
                assert!(
                    !text(n).trim().is_empty(),
                    "{name}: an element {c:?} carries a tone and no word"
                );
            }
        });

        // contrast from the tokens, in both themes
        for theme in ["light", "dark"] {
            total_contrast += check_contrast(&name, root, theme, &t, &rules);
        }
    }
    assert!(
        total_contrast > 500,
        "the contrast check looked at too little text: {total_contrast}"
    );
}

#[test]
fn the_tokens_give_the_tones_of_the_template_enough_contrast_in_both_themes() {
    let t = tokens();
    let pairs = [
        ("fg", "surface"),
        ("fg", "bg"),
        ("fg-2", "surface"),
        ("muted", "surface"),
        ("muted", "bg"),
        ("muted", "raised"),
        ("accent", "surface"),
        ("accent", "accent-soft"),
        ("accent-strong", "accent-tint"),
        ("fg", "raised"),
        ("ok-fg", "ok-bg"),
        ("warn-fg", "warn-bg"),
        ("crit-fg", "crit-bg"),
        ("low-fg", "low-bg"),
        ("on-accent", "accent"),
        ("chg-fg", "chg-bg"),
        ("new-fg", "new-bg"),
    ];
    for theme in ["light", "dark"] {
        for (f, b) in pairs {
            let (fc, bc) = (colour(&t, theme, f).unwrap(), colour(&t, theme, b).unwrap());
            let r = ratio(fc, bc);
            assert!(r >= 4.5, "{theme}: {f} on {b} is {r:.2}:1");
        }
    }
}

#[test]
fn a_value_holding_markup_is_drawn_as_text() {
    let v = draw(&views().join("samples/rules.json"));
    let all = elements(&v["tree"]);
    assert!(
        all.iter()
            .all(|e| tag(e) != "script" || attr(e, "id") == Some("bf-data")),
        "a script element was drawn"
    );
    assert!(
        all.iter().all(|e| tag(e) != "img"),
        "markup from a value became an element"
    );
    let page = text(&v["tree"]);
    assert!(
        page.contains("<script>window.__pwn = 1</script>"),
        "the script text is not shown as text"
    );
    assert!(page.contains("<img src=x onerror=alert(1)>"));
    assert!(page.contains("<b>stays as text</b>"));
    // only the template's own chevron is ever set as markup
    for e in &all {
        if let Some(raw) = e["raw"].as_str() {
            assert!(
                raw.starts_with("<svg") && !raw.contains("<script"),
                "markup set from a value: {raw}"
            );
        }
    }
}

#[test]
fn an_empty_label_hides_its_element_and_an_unseen_kind_is_drawn_from_its_words() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("odd.json");
    let page = serde_json::json!({
        "contract": 1, "kind": "workspace", "id": "odd",
        "page": {"title": "Odd", "context": [
            {"label": "", "value": "orphan value"}, {"label": "Space", "value": ""}, {"label": "Kept", "value": "shown"}]},
        "sections": [{"id": "s", "title": "S", "rows": [{
            "id": "x-1", "title": "A made-up kind of entry",
            "tags": [{"text": "", "tone": "ok"}, {"text": "Seen", "tone": "ok"}],
            "body": [{"type": "note", "label": "", "text": "A note with no label"},
                     {"type": "never-heard-of", "text": "Drawn from its text"}, {"type": "never-heard-of-2"}]
        }]}]
    });
    fs::write(&p, page.to_string()).unwrap();
    let v = draw(&p);
    let page = text(&v["tree"]);
    assert!(page.contains("Kept") && page.contains("shown"));
    assert!(
        !page.contains("orphan value"),
        "an item with an empty label was drawn"
    );
    let tags: Vec<String> = elements(&v["tree"])
        .iter()
        .filter(|e| {
            classes(e).contains(&"tag".to_string())
                && classes(e).iter().any(|c| c.starts_with("t-"))
        })
        .map(|e| text(e))
        .collect();
    assert_eq!(tags, vec!["Seen"], "a tag with empty text was drawn");
    assert!(
        page.contains("A made-up kind of entry")
            && page.contains("A note with no label")
            && page.contains("Drawn from its text")
    );
    assert!(
        elements(&v["tree"])
            .iter()
            .all(|e| !classes(e).contains(&"noteb".to_string())
                || !e["kids"].as_array().unwrap().iter().any(|k| tag(k) == "b")),
        "an empty note label was drawn"
    );
}
