//! U-8: the legal presets are data files with named blanks; `bf preset list` and `bf preset fill`
//! make a draft that always carries the draft mark.

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;

fn values(fx: &Fx, name: &str, v: &Value) -> String {
    let f = fx.root.join(name);
    fs::write(&f, serde_json::to_string(v).unwrap()).unwrap();
    f.to_str().unwrap().to_string()
}

fn all_blanks(fx: &Fx, kind: &str, lang: &str) -> Value {
    let l = fx.must(&["preset", "list", "--json"]).json();
    let p = l["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == kind && p["language"] == lang)
        .unwrap_or_else(|| panic!("no preset {kind} {lang}"))
        .clone();
    let mut m = serde_json::Map::new();
    for b in p["blanks"].as_array().unwrap() {
        m.insert(b["name"].as_str().unwrap().into(), json!("Filled-Value"));
    }
    Value::Object(m)
}

#[test]
fn the_list_has_three_documents_in_two_languages() {
    let fx = Fx::new();
    let l = fx.must(&["preset", "list", "--json"]).json();
    assert!(l["invalid"].as_array().unwrap().is_empty(), "{l}");
    for kind in ["privacy-policy", "terms-of-use", "impressum"] {
        for lang in ["en", "de"] {
            let found = l["presets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["kind"] == kind && p["language"] == lang);
            assert!(found, "missing {kind} {lang}");
        }
    }
}

#[test]
fn every_filled_document_carries_the_draft_mark_and_no_blank() {
    let fx = Fx::new();
    for kind in ["privacy-policy", "terms-of-use", "impressum"] {
        for lang in ["en", "de"] {
            let vf = values(&fx, "v.json", &all_blanks(&fx, kind, lang));
            let o = fx
                .must(&["preset", "fill", kind, lang, &vf, "--json"])
                .json();
            assert_eq!(o["draft"], true);
            let text = o["text"].as_str().unwrap();
            let mark = o["draft_mark"].as_str().unwrap();
            assert!(!mark.is_empty());
            assert!(text.starts_with(mark), "{kind} {lang}");
            assert!(text.trim_end().ends_with(mark), "{kind} {lang}");
            assert!(!text.contains("{{"), "{kind} {lang}: {text}");
            assert!(text.contains("Filled-Value"));
        }
    }
}

#[test]
fn a_missing_blank_is_refused_by_name() {
    let fx = Fx::new();
    let vf = values(&fx, "v.json", &json!({"COMPANY_NAME": "Acme"}));
    let o = fx.bf(&["preset", "fill", "impressum", "en", &vf]);
    assert_ne!(o.code, 0);
    assert!(o.all().contains("VAT_ID"), "{}", o.all());
}

#[test]
fn the_draft_can_be_written_to_a_file() {
    let fx = Fx::new();
    let vf = values(&fx, "v.json", &all_blanks(&fx, "impressum", "de"));
    // A draft goes only under `.bf/drafts/` of the project: a bare name is placed there.
    let out = fx.root.join(".bf/drafts/impressum.md");
    let o = fx.must(&["preset", "fill", "impressum", "de", &vf, "impressum.md", "--json"]);
    assert_eq!(o.json()["written"], "impressum.md");
    assert!(fs::read_to_string(out).unwrap().starts_with("ENTWURF"));
}

#[test]
fn a_new_language_is_a_new_file() {
    let fx = Fx::new();
    let dir = fx.root.join("extra-presets");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("impressum.fr.toml"),
        "kind=\"impressum\"\nlanguage=\"fr\"\ntitle=\"Mentions\"\ndraft_mark=\"BROUILLON DRAFT\"\nbody=\"{{NAME}}\"\n[[blanks]]\nname=\"NAME\"\n",
    )
    .unwrap();
    let vf = values(&fx, "v.json", &json!({"NAME": "Acme"}));
    let d = dir.to_str().unwrap();
    let o = fx.bf_env(
        &fx.root,
        &["preset", "fill", "impressum", "fr", &vf, "--json"],
        "",
        &[("BF_PRESETS_DIR", d)],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let v: Value = o.json();
    let t = v["text"].as_str().unwrap();
    assert!(t.starts_with("BROUILLON") && t.contains("Acme"));
    assert!(v["source"].as_str().unwrap().ends_with("impressum.fr.toml"));
    assert_eq!(v["shipped"], false);
}

#[test]
fn a_shadowing_preset_in_a_parent_folder_is_refused_and_the_draft_mark_stays() {
    let fx = Fx::new();
    // A repository root with a planted preset above the folder the command runs in.
    fs::create_dir_all(fx.root.join(".git")).unwrap();
    let planted = fx.root.join("presets");
    fs::create_dir_all(&planted).unwrap();
    fs::write(
        planted.join("impressum.en.toml"),
        "kind=\"impressum\"\nlanguage=\"en\"\ntitle=\"Shadow\"\ndraft_mark=\"DRAFT planted\"\nbody=\"planted\"\n",
    )
    .unwrap();
    fs::write(
        planted.join("impressum.de.toml"),
        "kind=\"impressum\"\nlanguage=\"de\"\ntitle=\"Shadow\"\ndraft_mark=\"Final\"\nbody=\"planted\"\n",
    )
    .unwrap();
    let sub = fx.root.join("sub");
    fs::create_dir_all(&sub).unwrap();
    let v: Value = fx
        .bf_env(&sub, &["preset", "list", "--json"], "", &[])
        .json();
    assert_eq!(v["invalid"].as_array().unwrap().len(), 2, "{v}");
    let vf = values(&fx, "v.json", &all_blanks(&fx, "impressum", "en"));
    let o = fx.bf_env(
        &sub,
        &["preset", "fill", "impressum", "en", &vf, "--json"],
        "",
        &[],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let v: Value = o.json();
    assert_eq!(v["shipped"], true, "{v}");
    assert!(!v["source"].as_str().unwrap().contains("planted"));
    let t = v["text"].as_str().unwrap();
    assert!(!t.contains("planted") && t.starts_with(v["draft_mark"].as_str().unwrap()));
    assert!(t.to_uppercase().contains("DRAFT"));
}

#[test]
fn a_preset_above_the_repository_root_is_not_read() {
    let fx = Fx::new();
    let above = fx.root.join("above");
    let repo = above.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(above.join("presets")).unwrap();
    fs::write(
        above.join("presets/memo.en.toml"),
        "kind=\"memo\"\nlanguage=\"en\"\ntitle=\"M\"\ndraft_mark=\"DRAFT\"\nbody=\"m\"\n",
    )
    .unwrap();
    let v: Value = fx
        .bf_env(&repo, &["preset", "list", "--json"], "", &[])
        .json();
    assert!(
        !v["presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["kind"] == "memo"),
        "{v}"
    );
}

#[test]
fn a_draft_mark_without_the_draft_word_is_invalid() {
    let fx = Fx::new();
    let dir = fx.root.join("nomark-presets");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("memo.en.toml"),
        "kind=\"memo\"\nlanguage=\"en\"\ntitle=\"M\"\ndraft_mark=\"Final text\"\nbody=\"m\"\n",
    )
    .unwrap();
    let d = dir.to_str().unwrap();
    let v: Value = fx
        .bf_env(
            &fx.root,
            &["preset", "list", "--json"],
            "",
            &[("BF_PRESETS_DIR", d)],
        )
        .json();
    assert_eq!(v["invalid"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn a_preset_without_a_draft_mark_is_invalid() {
    let fx = Fx::new();
    let dir = fx.root.join("bad-presets");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("x.toml"),
        "kind=\"x\"\nlanguage=\"en\"\ntitle=\"T\"\nbody=\"b\"\n",
    )
    .unwrap();
    let d = dir.to_str().unwrap();
    let o = fx.bf_env(
        &fx.root,
        &["preset", "list", "--json"],
        "",
        &[("BF_PRESETS_DIR", d)],
    );
    let v: Value = o.json();
    assert_eq!(v["invalid"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn a_misnamed_file_is_invalid_and_does_not_shadow() {
    let fx = Fx::new();
    let dir = fx.root.join("misnamed-presets");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("aaa.toml"),
        "kind=\"impressum\"\nlanguage=\"en\"\ntitle=\"Shadow\"\ndraft_mark=\"D\"\nbody=\"shadow\"\n",
    )
    .unwrap();
    let d = dir.to_str().unwrap();
    let v: Value = fx
        .bf_env(
            &fx.root,
            &["preset", "list", "--json"],
            "",
            &[("BF_PRESETS_DIR", d)],
        )
        .json();
    let bad = v["invalid"].as_array().unwrap();
    assert_eq!(bad.len(), 1, "{v}");
    assert!(bad[0]["file"].as_str().unwrap().ends_with("aaa.toml"));
    let shadow = v["presets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["title"] == "Shadow");
    assert!(!shadow, "{v}");
}

#[test]
fn the_shipped_presets_are_found_from_a_folder_with_none() {
    // The project folder has no presets; the engine finds the ones that ship with it.
    let fx = Fx::new();
    assert!(!fx.root.join("presets").exists());
    let v: Value = fx.must(&["preset", "list", "--json"]).json();
    assert_eq!(v["presets"].as_array().unwrap().len(), 6, "{v}");
}
