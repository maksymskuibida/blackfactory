//! U-6: the scan runner. Stand-in tools (small shell scripts) take the place of the outside scanners.

mod common;
use common::Fx;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const SECRET: &str = "AKIAPLANTEDSECRET1234567890";

fn tool(dir: &Path, name: &str, body: &str) {
    fs::create_dir_all(dir).unwrap();
    let f = dir.join(name);
    fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A project whose check PATH starts with a folder of stand-in tools.
fn fx_with_tools() -> (Fx, PathBuf) {
    let fx = Fx::new();
    let bin = fx.root.parent().unwrap().join("standins");
    fs::create_dir_all(&bin).unwrap();
    fx.set_policy(
        "/usr/bin:/bin:/usr/sbin:/sbin",
        &format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    for (id, sub, name) in [
        ("X-70", "secrets", "secret-scan"),
        ("X-71", "licences", "dependency-licence-check"),
    ] {
        let t = format!(
            "id=\"{id}\"\nkind=\"check\"\ntitle=\"{name}\"\nhow=\"command\"\ncommand=\"bf scan {sub}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"{name}\"\n"
        );
        let o = fx.put("check", &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    (fx, bin)
}

/// A stand-in scanner that reports the planted secret only in the files run (`--no-git`) or only in the
/// history run, like a scanner that sees the uncommitted file or the old commit.
fn leak_in(files: bool) -> String {
    let quiet = if files {
        "case \"$*\" in *--no-git*) ;; *) echo '[]'; exit 0;; esac"
    } else {
        "case \"$*\" in *--no-git*) echo '[]'; exit 0;; esac"
    };
    format!("{quiet}\n{}", leak_report())
}

fn leak_report() -> String {
    format!(
        "cat <<'EOF'\n[{{\"RuleID\":\"aws-key\",\"File\":\"src/app.env\",\"StartLine\":3,\"Commit\":\"abcdef0123456789\",\"Secret\":\"{SECRET}\",\"Match\":\"key={SECRET}\",\"Description\":\"found {SECRET}\"}}]\nEOF\nexit 1"
    )
}

fn all_receipts(fx: &Fx) -> String {
    let mut t = String::new();
    for e in fs::read_dir(fx.root.join("factory/receipts"))
        .unwrap()
        .flatten()
    {
        t.push_str(&fs::read_to_string(e.path()).unwrap());
    }
    t
}

#[test]
fn a_planted_secret_fails_the_check_and_never_shows() {
    let (fx, bin) = fx_with_tools();
    // The secret is in a file that is not committed: only the run over the files sees it.
    tool(&bin, "gitleaks", &leak_in(true));
    let o = fx.must(&["scan", "secrets", "--json"]);
    let v = o.json();
    assert_eq!(v["ok"], false);
    assert_eq!(v["count"], 1);
    assert_eq!(v["findings"][0]["rule"], "aws-key");
    assert_eq!(v["findings"][0]["file"], "src/app.env");
    assert_eq!(v["findings"][0]["commit"], "abcdef012345");
    assert!(!o.all().contains(SECRET), "{}", o.all());
    let rc = all_receipts(&fx);
    assert!(rc.contains("exit = 1"), "{rc}");
    assert!(!rc.contains(SECRET), "the receipt shows the secret");
    let log = fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap_or_default();
    assert!(!log.contains(SECRET));
    let _ = fx;
}

#[test]
fn a_secret_only_in_the_history_fails_too() {
    let (fx, bin) = fx_with_tools();
    tool(&bin, "gitleaks", &leak_in(false));
    let o = fx.must(&["scan", "secrets", "--json"]);
    let v = o.json();
    assert_eq!(v["ok"], false);
    assert_eq!(v["count"], 1);
    assert!(!o.all().contains(SECRET));
}

#[test]
fn an_unreadable_report_shows_nothing_the_scanner_printed() {
    let (fx, bin) = fx_with_tools();
    tool(
        &bin,
        "gitleaks",
        &format!("echo 'match: {SECRET}' >&2; echo 'oops {SECRET}'; exit 2"),
    );
    let o = fx.refused(&["scan", "secrets"]);
    assert!(o.stderr.contains("gitleaks"));
    assert!(!o.all().contains(SECRET), "{}", o.all());
    assert!(!all_receipts(&fx).contains(SECRET));
}

#[test]
fn a_replaced_scanner_is_named_in_the_result_too() {
    let (fx, bin) = fx_with_tools();
    tool(&bin, "gitleaks", "echo '[]'");
    fx.must(&["scan", "secrets"]);
    tool(&bin, "gitleaks", "echo '[]'  # replaced");
    let v = fx.must(&["scan", "secrets", "--json"]).json();
    assert!(v["tool_note"].as_str().unwrap().contains("the file"), "{v}");
}

#[test]
fn a_clean_scan_passes_and_records_where_the_tool_was_found() {
    let (fx, bin) = fx_with_tools();
    tool(&bin, "gitleaks", "echo '[]'");
    let v = fx.must(&["scan", "secrets", "--json"]).json();
    assert_eq!(v["ok"], true);
    let path = v["tool_path"].as_str().unwrap();
    assert!(path.ends_with("standins/gitleaks"), "{path}");
    assert_eq!(v["tool_hash"].as_str().unwrap().len(), 64);
    let rc = all_receipts(&fx);
    assert!(rc.contains("tool_path"));
    assert!(rc.contains("exit = 0"));
}

#[test]
fn a_missing_secret_scanner_is_a_refusal_naming_the_tool() {
    let fx = Fx::new();
    let o = fx.refused(&["scan", "secrets"]);
    assert!(o.stderr.contains("gitleaks"), "{}", o.stderr);
    assert!(!fx.root.join("factory/receipts").exists() || all_receipts(&fx).is_empty());
}

#[test]
fn a_scanner_that_prints_no_report_is_not_a_pass() {
    let (fx, bin) = fx_with_tools();
    tool(&bin, "gitleaks", "echo 'segfault' >&2; exit 0");
    let o = fx.refused(&["scan", "secrets"]);
    assert!(o.stderr.contains("gitleaks"));
    tool(&bin, "gitleaks", "echo '[]'; exit 3");
    fx.refused(&["scan", "secrets"]);
}

#[test]
fn a_replaced_scanner_is_noted_in_the_receipt() {
    let (fx, bin) = fx_with_tools();
    tool(&bin, "gitleaks", "echo '[]'");
    fx.must(&["scan", "secrets"]);
    tool(&bin, "gitleaks", "echo '[]'  # replaced");
    fx.must(&["scan", "secrets"]);
    assert!(all_receipts(&fx).contains("program_note = \"the file"));
}

fn lockfile(fx: &Fx, name: &str) {
    fs::write(fx.root.join(name), "x\n").unwrap();
}

#[test]
fn permissive_licences_pass() {
    let (fx, bin) = fx_with_tools();
    lockfile(&fx, "Cargo.lock");
    tool(
        &bin,
        "cargo-license",
        r#"echo '[{"name":"a","version":"1.0.0","license":"MIT OR Apache-2.0"},{"name":"b","version":"2.0.0","license":"MPL-2.0"}]'"#,
    );
    let v = fx.must(&["scan", "licences", "--json"]).json();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["packages"], 2);
    assert_eq!(v["weak_copyleft"][0], "MPL-2.0");
}

#[test]
fn strong_copyleft_and_unknown_become_legal_findings() {
    let (fx, bin) = fx_with_tools();
    lockfile(&fx, "package-lock.json");
    tool(
        &bin,
        "license-checker",
        r#"echo '{"left@1.0.0":{"licenses":"GPL-3.0"},"right@2.1.0":{"licenses":"UNKNOWN"},"fine@1.0.0":{"licenses":"MIT"},"dual@1.0.0":{"licenses":"(GPL-3.0 AND MIT)"}}'"#,
    );
    let v = fx.must(&["scan", "licences", "--json"]).json();
    assert_eq!(v["ok"], false, "{v}");
    let f = v["findings"].as_array().unwrap();
    assert_eq!(f.len(), 3, "{v}");
    let l = fx.must(&["legal", "list", "--json"]).json();
    assert_eq!(l["findings"].as_array().unwrap().len(), 3);
    // Run again: the same findings are reused, not filed twice.
    fx.must(&["scan", "licences"]);
    let l = fx.must(&["legal", "list", "--json"]).json();
    assert_eq!(l["findings"].as_array().unwrap().len(), 3);
}

#[test]
fn a_project_states_its_own_allowed_and_refused_licences() {
    let (fx, bin) = fx_with_tools();
    lockfile(&fx, "poetry.lock");
    tool(
        &bin,
        "pip-licenses",
        r#"echo '[{"Name":"x","Version":"1","License":"GPL-3.0"},{"Name":"y","Version":"1","License":"MIT"},{"Name":"z","Version":"1","License":"Custom-Corp"}]'"#,
    );
    fs::write(
        fx.root.join("factory/licences.toml"),
        "allow = [\"GPL-3.0\", \"Custom-Corp\"]\nrefuse = [\"MIT\"]\n",
    )
    .unwrap();
    let v = fx.must(&["scan", "licences", "--json"]).json();
    let f = v["findings"].as_array().unwrap();
    assert_eq!(f.len(), 1, "{v}");
    assert_eq!(f[0]["licence"], "MIT");
}

#[test]
fn a_missing_licence_tool_is_a_refusal_naming_the_tool() {
    let (fx, _bin) = fx_with_tools();
    lockfile(&fx, "go.sum");
    let o = fx.refused(&["scan", "licences"]);
    assert!(o.stderr.contains("go-licenses"), "{}", o.stderr);
}

#[test]
fn every_kind_of_project_has_its_tool() {
    let names: Vec<&str> = bf::scan::ECOSYSTEMS.iter().map(|e| e.name).collect();
    assert_eq!(
        names,
        [
            "rust",
            "javascript-typescript",
            "python",
            "go",
            "swift-cocoapods",
            "gradle"
        ]
    );
    let _: Option<Value> = None;
}

#[test]
fn go_swift_and_gradle_are_read() {
    let (fx, bin) = fx_with_tools();
    lockfile(&fx, "go.sum");
    lockfile(&fx, "Podfile.lock");
    lockfile(&fx, "gradle.lockfile");
    tool(&bin, "go-licenses", "echo 'example.org/m,https://x,MIT'");
    // The real tools write a file and print nothing the runner could read.
    tool(
        &bin,
        "license-plist",
        r#"mkdir -p build/bf-licences; echo '[{"name":"Pod","version":"1","license":"Apache-2.0"}]' > build/bf-licences/license_plist.json"#,
    );
    tool(
        &bin,
        "gradle",
        r#"mkdir -p build/reports/dependency-license; echo '{"dependencies":[{"moduleName":"g","moduleVersion":"1","moduleLicenses":[{"moduleLicense":"BSD-3-Clause"}]}]}' > build/reports/dependency-license/index.json"#,
    );
    let v = fx.must(&["scan", "licences", "--json"]).json();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["packages"], 3);
    assert_eq!(v["projects"].as_array().unwrap().len(), 3);
}

#[test]
fn a_gradle_run_with_no_report_file_is_a_refusal_naming_the_tool() {
    let (fx, bin) = fx_with_tools();
    lockfile(&fx, "gradle.lockfile");
    // Prints JSON, writes no file: the report is read from the file, so this is not a pass.
    tool(&bin, "gradle", r#"echo '{"dependencies":[]}'"#);
    let o = fx.refused(&["scan", "licences"]);
    assert!(o.stderr.contains("gradle"), "{}", o.stderr);
    assert!(o.stderr.contains("not checked"), "{}", o.stderr);
}
