//! The fix of the K-3 checkpoint review, round 6: a scan proves only the check whose command it is,
//! a preset draft is written only under `.bf/drafts/`, a scan is run where its check runs, and a
//! legal finding changed after its acknowledgement is asked again.

mod common;
use common::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn standin(fx: &Fx) {
    let bin = fx.root.parent().unwrap().join("standins");
    fs::create_dir_all(&bin).unwrap();
    fx.set_policy(
        "/usr/bin:/bin:/usr/sbin:/sbin",
        &format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    let g = bin.join("gitleaks");
    fs::write(&g, "#!/bin/sh\necho '[]'\nexit 0\n").unwrap();
    fs::set_permissions(&g, fs::Permissions::from_mode(0o755)).unwrap();
}

fn scan_project() -> Fx {
    let fx = Fx::new();
    standin(&fx);
    fx.tiny();
    for (id, sub, name) in [
        ("X-70", "secrets", "secret-scan"),
        ("X-71", "licences", "dependency-licence-check"),
    ] {
        let t = format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"{name}\"\nhow=\"command\"\ncommand=\"bf scan {sub}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"{name}\"\n");
        let o = fx.put("check", &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let k = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\",\"X-70\",\"X-71\"]\n";
    let f = fx.root.join("k1.toml");
    fs::write(&f, k).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "checkpoint",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("scans");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx
}

#[test]
fn a_secret_scan_named_for_the_licence_check_or_a_command_check_is_refused() {
    let fx = scan_project();
    let k = fx.work("K-1");
    let top = k.to_str().unwrap();
    // The secret scan named for the licence check, whose command is another scan.
    let o = fx.bf(&["-C", top, "scan", "secrets", "--check", "X-71"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert_four_parts(&o.stderr);
    // A scan named for an ordinary command check.
    let o = fx.bf(&["-C", top, "scan", "licences", "--check", "X-3"]);
    assert_ne!(o.code, 0, "{}", o.all());
    // A scan named for a record that does not exist.
    let o = fx.bf(&["-C", top, "scan", "secrets", "--check", "X-999"]);
    assert_ne!(o.code, 0, "{}", o.all());
    // Named for its own check it is accepted.
    let o = fx.bf(&["-C", top, "scan", "secrets", "--check", "X-70"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn both_scans_run_in_a_checkpoints_copy_and_count_at_its_content() {
    let fx = scan_project();
    let k = fx.work("K-1");
    let a = fx.bf_in(&k, &["scan", "secrets"]);
    assert_eq!(a.code, 0, "{}", a.all());
    let b = fx.bf_in(&k, &["scan", "licences"]);
    assert_eq!(b.code, 0, "{}", b.all());
    let v = fx.bf_in(&k, &["evidence", "verify"]);
    assert!(
        v.code == 0 && v.all().contains("X-70") && v.all().contains("X-71"),
        "{}",
        v.all()
    );
    // Anything else of the project folder's is still refused in the copy.
    let o = fx.bf_in(&k, &["scan", "bogus"]);
    assert_ne!(o.code, 0);
}

#[test]
fn a_refusal_in_a_working_copy_names_the_scan_form() {
    let fx = scan_project();
    let o = fx.bf_in(&fx.work("K-1"), &["gate", "open", "plan"]);
    assert_ne!(o.code, 0);
    assert!(
        o.stderr.contains("bf scan secrets|licences"),
        "{}",
        o.stderr
    );
}

fn drafts_setup() -> (Fx, String) {
    let fx = Fx::new();
    fx.tiny();
    fs::write(fx.root.join("v.json"), "{}").unwrap();
    fs::create_dir_all(fx.root.join("presets")).unwrap();
    fs::write(
        fx.root.join("presets/note.en.toml"),
        "kind = \"note\"\nlanguage = \"en\"\ntitle = \"note\"\ndraft_mark = \"# DRAFT\"\nbody = 'text'\n",
    )
    .unwrap();
    (fx, "v.json".into())
}

#[test]
fn a_draft_goes_only_under_the_drafts_folder() {
    let (fx, v) = drafts_setup();
    let ok = fx.bf(&["preset", "fill", "note", "en", &v, "memo.md", "--json"]);
    assert_eq!(ok.code, 0, "{}", ok.all());
    assert!(fx.root.join(".bf/drafts/memo.md").is_file());
    let inside = fx.bf(&["preset", "fill", "note", "en", &v, ".bf/drafts/sub/two.md"]);
    assert_eq!(inside.code, 0, "{}", inside.all());
    assert!(fx.root.join(".bf/drafts/sub/two.md").is_file());
    let abs = fx.root.join("elsewhere.md");
    for out in [
        abs.to_str().unwrap().to_string(),
        "factory/policy.toml".into(),
        ".bf/drafts/../../factory/x.md".into(),
        ".claude/settings.json".into(),
        ".bf/drafts".into(),
    ] {
        let o = fx.bf(&["preset", "fill", "note", "en", &v, &out]);
        assert_ne!(o.code, 0, "{out}: {}", o.all());
        assert_four_parts(&o.stderr);
    }
    assert!(!abs.exists());
}

#[test]
fn a_link_in_the_drafts_folder_cannot_lead_a_draft_out() {
    let (fx, v) = drafts_setup();
    let dir = fx.root.join(".bf/drafts");
    fs::create_dir_all(&dir).unwrap();
    let rec = fx.root.join("factory/policy.toml");
    let before = fs::read(&rec).unwrap();
    std::os::unix::fs::symlink(&rec, dir.join("link.md")).unwrap();
    let o = fx.bf(&["preset", "fill", "note", "en", &v, "link.md"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert_eq!(fs::read(&rec).unwrap(), before);
    let _: PathBuf = rec;
}
