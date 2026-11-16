//! The desktop panel (R-81, R-83, C-22): its manifest and hooks file, the events its module hooks, the
//! commands it runs, its drawing code, its tokens and the CI step that checks it. Each test reads the
//! shipped files or runs the real engine; none needs the Claude desktop app.

mod common;
use common::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn json(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap_or_else(|e| panic!("{rel} is not JSON: {e}"))
}

const PANEL: &str = "harness/claude/panel";

#[test]
fn the_manifest_names_the_panel_and_its_three_options() {
    let m = json(&format!("{PANEL}/.claude-plugin/plugin.json"));
    assert_eq!(m["name"], "factory-panel");
    assert_eq!(m["types"], "./types/index.d.ts");
    let o = &m["userConfig"];
    let panel = &o["panel"];
    assert_eq!(panel["default"], "auto");
    let choices: Vec<&str> = panel["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(choices, ["auto", "command", "off"]);
    assert_eq!(o["pollSeconds"]["default"], 5);
    assert_eq!(o["pollSeconds"]["type"], "number");
    assert_eq!(o["bf"]["default"], "bf");
    assert_eq!(
        o.as_object().unwrap().len(),
        3,
        "the panel has exactly three options"
    );
    let version = read(&format!("{PANEL}/src/version.ts"));
    assert!(
        version.contains(&format!("'{}'", m["version"].as_str().unwrap())),
        "src/version.ts must carry the manifest's version"
    );
}

#[test]
fn the_hooks_file_names_the_one_module() {
    let h = json(&format!("{PANEL}/hooks/hooks.json"));
    assert_eq!(h["modules"], serde_json::json!(["register.tsx"]));
    assert!(repo(&format!("{PANEL}/hooks/register.tsx")).is_file());
    assert!(repo(&format!("{PANEL}/types/index.d.ts")).is_file());
    for f in ["README.md", "src/reads.ts", "src/model.ts", "src/refresh.ts"] {
        assert!(repo(&format!("{PANEL}/{f}")).is_file(), "{f} is missing");
    }
}

#[test]
fn the_module_hooks_exactly_the_listed_events() {
    let src = read(&format!("{PANEL}/hooks/register.tsx"));
    let mut seen = BTreeSet::new();
    for part in src.split("on('").skip(1) {
        let name: String = part.chars().take_while(|c| *c != '\'').collect();
        seen.insert(name);
    }
    let want: BTreeSet<String> = [
        "session.start",
        "session.attach",
        "command.run",
        "ui.close",
        "turn.complete",
        "ui.render",
        "ui.press",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(seen, want);
    // ui.render is matched on the Pane and on the AbovePrompt band only.
    let renders: Vec<&str> = src
        .split("on('ui.render', ")
        .skip(1)
        .map(|p| p.split("async").next().unwrap())
        .collect();
    assert_eq!(renders.len(), 2, "{renders:?}");
    assert!(renders.iter().any(|r| r.contains("component: 'Pane'")));
    assert!(renders
        .iter()
        .any(|r| r.contains("component: 'AbovePrompt'")));
    for banned in ["tool.", "prompt.", "$.prompt", "$.tool", "import("] {
        assert!(!src.contains(banned), "the module must not use `{banned}`");
    }
}

/// The argument lists of src/reads.ts: the fixed ones, and one view per kind.
fn listed_reads() -> Vec<Vec<String>> {
    let src = read(&format!("{PANEL}/src/reads.ts"));
    let block = src
        .split("export const READS = {")
        .nth(1)
        .unwrap()
        .split("} as const")
        .next()
        .unwrap();
    let mut out = Vec::new();
    for line in block.lines().filter(|l| l.contains('[')) {
        let inside = line.split('[').nth(1).unwrap().split(']').next().unwrap();
        out.push(
            inside
                .split(',')
                .map(|w| w.trim().trim_matches('\'').to_string())
                .collect(),
        );
    }
    let kinds = src
        .split("export const VIEW_KINDS = [")
        .nth(1)
        .unwrap()
        .split("] as const")
        .next()
        .unwrap();
    for k in kinds.split(',').map(|k| k.trim().trim_matches('\'')) {
        if k.is_empty() {
            continue;
        }
        let mut v = vec!["view".to_string(), k.to_string()];
        if k == "checkpoint" {
            v.push("K-1".into());
        }
        v.push("--json".into());
        out.push(v);
    }
    out
}

#[test]
fn every_listed_read_exits_zero_prints_json_and_changes_nothing() {
    let reads = listed_reads();
    assert!(
        reads.len() >= 3 + 8,
        "reads.ts lists the three fixed reads and every page kind: {reads:?}"
    );
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let before = fx.must(&["status", "--json"]).json()["events"].clone();
    assert!(before.as_u64().unwrap() > 0);
    for args in &reads {
        let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        // The gate page needs a gate: the plan gate was approved in the test run.
        let a = if a[..2] == ["view", "gate"] {
            vec!["view", "gate", "plan", "--json"]
        } else {
            a
        };
        let o = fx.bf(&a);
        assert_eq!(o.code, 0, "bf {a:?}:\n{}", o.all());
        let _ = o.json();
    }
    let after = fx.must(&["status", "--json"]).json()["events"].clone();
    assert_eq!(before, after, "a read must not append an event");
    // The refusal the reader tells apart from every other failure.
    let nowhere = tempfile::tempdir().unwrap();
    let o = fx.bf_in(nowhere.path(), &["status", "--json"]);
    assert_eq!(o.code, 2);
    assert_eq!(o.json()["refused"], "use a Black Factory project here");
    assert!(read(&format!("{PANEL}/src/reader.ts")).contains("'use a Black Factory project here'"));
}

#[test]
fn reads_ts_holds_no_command_that_changes_state() {
    let src = read(&format!("{PANEL}/src/reads.ts"));
    for banned in ["'next'", "'put'", "'gate'", "'unit'", "'run'", "'report'", "'evidence'", "'amend'"] {
        assert!(!src.contains(&format!("[{banned}")), "{banned} is not a read");
    }
    // No file is read or written, no other program run, by any file of src/ or hooks/.
    for dir in ["src", "hooks"] {
        for f in files_under(&repo(&format!("{PANEL}/{dir}"))) {
            let text = fs::read_to_string(&f).unwrap();
            for banned in ["$.fs", "$.store", "$.http", "$.process.spawn", "factory/", ".bf/"] {
                assert!(!text.contains(banned), "{} uses `{banned}`", f.display());
            }
        }
    }
    let register = read(&format!("{PANEL}/hooks/register.tsx"));
    assert_eq!(register.matches("$.process.run").count(), 1);
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Text with a letter between two drawing tags, as `<Text>words</Text>` has.
fn letter_text_between_tags(source: &str) -> Vec<String> {
    let b: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '>' && !(i > 0 && matches!(b[i - 1], '=' | '-')) {
            let mut j = i + 1;
            while j < b.len() && !matches!(b[j], '<' | '>' | '{' | '}') {
                j += 1;
            }
            let opens_tag = j + 1 < b.len()
                && b[j] == '<'
                && (b[j + 1] == '/' || b[j + 1].is_ascii_uppercase());
            let text: String = b[i + 1..j].iter().collect();
            let is_code = text.contains([')', ';', '(']);
            if opens_tag && !is_code && text.chars().any(|c| c.is_alphabetic()) {
                found.push(text.trim().to_string());
            }
        }
        i += 1;
    }
    found
}

#[test]
fn no_renderer_holds_a_letter_text_between_drawing_tags() {
    assert_eq!(
        letter_text_between_tags("<Text>words</Text>"),
        ["words"],
        "the detector must see words between tags"
    );
    assert!(letter_text_between_tags("<Text>{t('key')}</Text> <Text>{x}</Text>").is_empty());
    let files = files_under(&repo(&format!("{PANEL}/src/render")));
    assert!(files.iter().any(|f| f.extension().is_some_and(|e| e == "tsx")));
    for f in files {
        let text = fs::read_to_string(&f).unwrap();
        let found = letter_text_between_tags(&text);
        assert!(found.is_empty(), "{} holds letter text: {found:?}", f.display());
    }
}

#[test]
fn the_panels_tokens_match_the_tokens_file() {
    let o = Command::new("python3")
        .args(["views/dev/build.py", "--check"])
        .current_dir(repo(""))
        .output()
        .expect("python3 runs");
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let tokens = read(&format!("{PANEL}/src/tokens.ts"));
    assert!(tokens.starts_with("// generated by views/dev/build.py"));
    let file = json("views/tokens/tokens.json");
    let accent = file["dark"]["accent"].as_str().unwrap();
    assert!(tokens.contains(accent), "the dark accent is in the panel's copy");
}

#[test]
fn ci_runs_the_panel_checks_on_every_run_without_a_secret() {
    let ci = read(".github/workflows/ci.yml");
    for needed in [
        "actions/setup-node",
        "@anthropic-ai/claude-code@",
        "claude plugin validate harness/claude/panel",
        "claude plugin test harness/claude/panel",
        "python3 views/dev/build.py --check",
    ] {
        assert!(ci.contains(needed), "ci.yml must hold `{needed}`");
    }
    assert!(!ci.contains("secrets."), "the panel step needs no secret");
    let pinned = ci
        .split("@anthropic-ai/claude-code@")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    assert!(
        pinned.chars().all(|c| c.is_ascii_digit() || c == '.') && pinned.matches('.').count() == 2,
        "the Claude Code version is pinned: {pinned}"
    );
    // The step has no condition: it runs on every run.
    let step = ci.split("Desktop panel").nth(1).unwrap();
    let head: Vec<&str> = step.lines().take(3).collect();
    assert!(!head.iter().any(|l| l.trim_start().starts_with("if:")));
}

#[test]
fn the_readme_names_no_person_home_path_or_private_link() {
    let text = read(&format!("{PANEL}/README.md"));
    for banned in ["/Users/", "/home/", "@gmail", "claude.ai/artifact", "C:\\"] {
        assert!(!text.contains(banned), "README holds `{banned}`");
    }
    for needed in ["panel", "pollSeconds", "bf status --json", "claude plugin validate"] {
        assert!(text.contains(needed), "README should say `{needed}`");
    }
}

#[test]
fn every_word_the_panel_writes_is_in_the_strings_file() {
    let en = read(&format!("{PANEL}/src/strings/en.ts"));
    assert!(en.contains("refresh: 'Refresh'"));
    for f in files_under(&repo(&format!("{PANEL}/src/render"))) {
        let text = fs::read_to_string(&f).unwrap();
        assert!(!text.contains("localisedOr"), "{}", f.display());
    }
}
