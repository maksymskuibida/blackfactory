//! At the release only whole blocks of the engine's instruction leave a brief; a brief that only
//! quotes the block markers keeps that text (U-131, R-74).

mod common;
use common::*;
use serde_json::json;
use std::fs;

const CMD: &str = "/factory:human-absent";

fn say(fx: &Fx, session: &str, text: &str) {
    let payload = json!({
        "session_id": session,
        "cwd": fx.root.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "UserPromptSubmit",
        "prompt": text,
    });
    let o = fx.bf_stdin(&fx.root, &["human-turn"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

fn begin() -> String {
    bf::strings::t("absent_briefs.begin", &[])
}

fn end() -> String {
    bf::strings::t("absent_briefs.end", &[])
}

fn title() -> String {
    bf::strings::t("absent_briefs.title", &[])
}

/// A diff of the strings file, as a fix-review brief shows it.
fn quote() -> String {
    format!(
        "# Brief: K-6 as reviewer\n\n```diff\n+++ b/crates/bf/strings/en/absent_briefs.toml\n\
         +\"absent_briefs.begin\" = \"{}\"\n+\"absent_briefs.title\" = \"{}\"\n\
         +\"absent_briefs.end\" = \"{}\"\n```\n\nmore text\n",
        begin(),
        title(),
        end()
    )
}

fn write_brief(fx: &Fx, name: &str, text: &str) -> std::path::PathBuf {
    let dir = fx.project().briefs_dir();
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_brief_that_quotes_the_markers_keeps_the_quote() {
    let fx = Fx::tiny_run();
    let text = quote();
    let path = write_brief(&fx, "K-6-reviewer.md", &text);
    bf::absent::clear_briefs(&fx.project()).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn a_stamped_brief_that_quotes_the_markers_loses_only_its_block() {
    let fx = Fx::tiny_run();
    let plain = quote();
    let path = write_brief(
        &fx,
        "K-6-reviewer.md",
        &format!("{}{plain}", bf::absent::brief_block()),
    );
    bf::absent::clear_briefs(&fx.project()).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), plain);
}

#[test]
fn a_begin_line_without_the_title_or_without_an_end_stays() {
    let fx = Fx::tiny_run();
    let kept = format!(
        "intro\n{b}\nnot the title\n{b}\n## {t}\nno end line here\n",
        b = begin(),
        t = title()
    );
    let tail = "more text\n";
    let path = write_brief(
        &fx,
        "U-9-builder.md",
        &format!("{kept}{}{tail}", bf::absent::brief_block()),
    );
    let open = format!("text\n{}\n## {}\n", begin(), title());
    let path2 = write_brief(&fx, "U-8-builder.md", &open);
    bf::absent::clear_briefs(&fx.project()).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{kept}{tail}"));
    assert_eq!(fs::read_to_string(&path2).unwrap(), open);
}

#[test]
fn the_release_keeps_a_quote_in_a_brief() {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    let path = fx.project().briefs_dir().join("U-1-builder.md");
    let plain = fs::read_to_string(&path).unwrap();
    say(&fx, "s1", CMD);
    fx.must(&["unit", "start", "U-1"]);
    let stamped = fs::read_to_string(&path).unwrap();
    assert!(stamped.starts_with(&begin()), "{stamped}");
    let q = format!("\n+\"absent_briefs.begin\" = \"{}\"\n+text\n+\"absent_briefs.end\" = \"{}\"\n", begin(), end());
    fs::write(&path, format!("{stamped}{q}")).unwrap();
    say(&fx, "s1", "hello");
    say(&fx, "s1", "yes");
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{plain}{q}"));
}
