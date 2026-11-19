//! Tests of the strings table, the lookup, the refusal built from a key and the terminal form (U-1, C-12).

mod common;
use bf::refusal::{from_key, Fail};
use bf::strings;

#[test]
fn table_is_sorted_and_without_empty_text() {
    let keys: Vec<&str> = strings::keys().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(keys, sorted);
    assert!(keys.iter().all(|k| !strings::t(k, &[]).is_empty()));
}

#[test]
fn missing_key_gives_empty_text() {
    assert_eq!(strings::t("cli.no.such.key", &[("a", "b")]), "");
}

#[test]
fn places_are_filled_and_control_characters_stripped() {
    let t = strings::t("cli.not_built.refused", &[("command", "sp\x1b[31mace\n")]);
    assert_eq!(t, "run `bf sp[31mace`");
    assert_eq!(strings::t("cli.state", &[("sign", "x")]), "x ");
}

#[test]
fn refusal_from_key_has_four_parts() {
    let Fail::Refused(r) = from_key("cli.not_built", &[("command", "space")]) else {
        panic!()
    };
    assert_eq!(r.refused, "run `bf space`");
    assert_eq!(r.do_this.len(), 1);
    let t = r.text();
    assert!(t.starts_with("refused: run `bf space`\nbecause: "));
    assert!(t.contains("\ndo this:\n  1. "));
    assert!(t.ends_with("it worked when: `bf help` prints the usage.\n"));
}

#[test]
fn terminal_form() {
    assert_eq!(
        strings::context_line(Some("main"), Some("I-1")),
        "space: main  intent: I-1"
    );
    assert_eq!(strings::context_line(None, None), "");
    let rows = vec![
        vec!["U-1".to_string(), "built".to_string(), "x".to_string()],
        vec!["U-10".to_string(), "ok".to_string(), "y".to_string()],
    ];
    let g = strings::group("Units", &rows);
    assert_eq!(g, "Units\nU-1   built  x\nU-10  ok     y\n");
    assert_eq!(strings::info("hi"), "note: hi");
    assert_eq!(strings::state("✓", "passed"), "✓ passed");
}

#[test]
fn new_commands_answer_not_built() {
    let dir = std::env::temp_dir().join(format!("bf-strings-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for c in ["intent", "space", "waiting", "knowledge", "rules", "audit"] {
        let o = std::process::Command::new(common::bin())
            .args([c, "--json"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "{c}");
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(
            v["because"],
            strings::t("cli.not_built.because", &[]),
            "{c}"
        );
        assert_eq!(
            v["refused"],
            strings::t("cli.not_built.refused", &[("command", c)]),
            "{c}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn literal_braces_are_written_doubled() {
    assert_eq!(
        strings::fill("{{a, b}} {{{{NAME}}}} {name}", &[("name", "x\n")]),
        "{a, b} {{NAME}} x"
    );
}
