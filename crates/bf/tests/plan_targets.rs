//! U-56: the plan check reads a check's `cargo test` command against the package's real test targets
//! (FU-30 point 4). A new check is a problem; a check of an older plan is a note.

mod common;
use bf::plan::targets::{read_command, Read};
use common::*;
use std::fs;
use std::path::Path;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| x.to_string()).collect()
}

fn package(root: &Path) {
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nautotests = false\n\n[[test]]\nname = \"all\"\npath = \"tests/all.rs\"\n",
    )
    .unwrap();
    fs::write(root.join("tests/all.rs"), "").unwrap();
    fs::write(root.join("tests/account_home.rs"), "").unwrap();
    fs::write(root.join("src/lib.rs"), "mod inner_part;\n").unwrap();
}

/// A plan check that really succeeded: exit 0, a line starting `plan ok:` and no refusal (a refusal's
/// text also says `plan ok`, so the words alone prove nothing).
fn assert_plan_ok(o: &Out, ctx: &str) {
    let all = o.all();
    assert_eq!(o.code, 0, "{ctx}: {all}");
    assert!(!all.contains("refused:"), "{ctx}: {all}");
    assert!(all.lines().any(|l| l.starts_with("plan ok:")), "{ctx}: {all}");
}

fn check_with(command: &str) -> String {
    format!(
        "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"{command}\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"
    )
}

fn fresh(command: &str) -> String {
    let fx = Fx::new();
    package(&fx.root);
    fx.tiny();
    let tmp = tempfile::TempDir::new().unwrap();
    let f = tmp.path().join("check.toml");
    fs::write(&f, check_with(command)).unwrap();
    fx.must(&["record", "put", "check", "--file", f.to_str().unwrap(), "--replace"]);
    fx.bf(&["plan", "check"]).all()
}

#[test]
fn plan_targets_reads_a_command() {
    let r = read_command(&words(
        "cargo nextest run --package=bf --test one -E x plan_targets:: other",
    ))
    .unwrap();
    assert_eq!(
        r,
        Read {
            package: Some("bf".into()),
            tests: vec!["one".into()],
            filters: vec!["plan_targets::".into()],
        }
    );
    assert!(read_command(&words("cargo build --test x")).is_none());
    assert!(read_command(&words("npm test --test x")).is_none());
    let r = read_command(&words("cargo test -p demo --lib -- a::")).unwrap();
    assert_eq!(r.package.as_deref(), Some("demo"));
    assert_eq!(r.filters, vec!["a::".to_string()]);
}

#[test]
fn plan_targets_a_missing_test_target_is_a_problem() {
    let out = fresh("cargo test --test harness_worker_rules");
    assert!(out.contains("--test harness_worker_rules"), "{out}");
    assert!(out.contains("the plan has 1 problem"), "{out}");
}

#[test]
fn plan_targets_a_listed_target_and_a_real_filter_pass() {
    for c in [
        "cargo test --test all",
        "cargo nextest run --package=demo account_home::",
        "cargo nextest run --package=demo inner_part::",
        "cargo test --lib",
    ] {
        let out = fresh(c);
        assert!(!out.contains("problem(s)"), "{c}: {out}");
    }
}

#[test]
fn plan_targets_a_filter_that_matches_nothing_is_a_problem() {
    let out = fresh("cargo nextest run --package=demo nothing_here::");
    assert!(out.contains("nothing_here::"), "{out}");
    assert!(out.contains("the plan has 1 problem"), "{out}");
}

#[test]
fn plan_targets_another_program_is_not_judged() {
    let out = fresh("test -f --test nothing_here::");
    assert!(!out.contains("problem(s)"), "{out}");
}


struct Old {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    base: PathBuf,
}

impl Old {
    fn new() -> Old {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        for part in ["project", "state"] {
            let st = Command::new("cp")
                .arg("-R")
                .arg(fixture("old_project").join(part))
                .arg(base.join(part))
                .status()
                .unwrap();
            assert!(st.success());
        }
        let root = base.join("project");
        package(&root);
        Old {
            root,
            base,
            _tmp: tmp,
        }
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let o = Command::new(bin())
            .env("BF_STATE_HOME", self.base.join("state"))
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            o.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )
    }

    fn must(&self, args: &[&str]) -> String {
        let (c, out) = self.run(args);
        assert_eq!(c, 0, "{args:?}: {out}");
        out
    }

    fn put_command(&self, command: &str) {
        let rec = fs::read_to_string(self.root.join("factory/records/X-1.toml")).unwrap();
        assert!(rec.contains("test -f src/hello.txt"), "{rec}");
        let rec = rec.replacen("test -f src/hello.txt", command, 1);
        let f = self.root.join("put.toml");
        fs::write(&f, rec).unwrap();
        self.must(&[
            "record",
            "put",
            "check",
            "--file",
            f.to_str().unwrap(),
            "--replace",
        ]);
        fs::remove_file(f).unwrap();
    }

    fn say(&self, text: &str) {
        use std::io::Write;
        let payload = json!({"session_id": "0b6f1c3e-aaaa-bbbb-cccc-1234567890ab",
            "cwd": self.root.to_string_lossy(), "permission_mode": "default",
            "hook_event_name": "UserPromptSubmit", "prompt": text});
        let mut c = Command::new(bin())
            .env("BF_STATE_HOME", self.base.join("state"))
            .arg("human-turn")
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .ok();
        let o = c.wait_with_output().unwrap();
        assert!(o.status.success());
    }

    /// A plan repair over X-1 whose new command is `command`, approved with the human's words.
    fn amend_command(&self, command: &str) -> (i32, String) {
        let opened: Value =
            serde_json::from_str(&self.must(&["amend", "open", "--repair", "X-1", "--json"]))
                .unwrap();
        self.put_command(command);
        let keep: Vec<String> = opened["impact_ids"]
            .as_array()
            .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
            .unwrap_or_default();
        let aid = opened["amend"]
            .as_str()
            .or_else(|| opened["id"].as_str())
            .unwrap_or("A-1")
            .to_string();
        let f = self.root.join("narrow.json");
        fs::write(
            &f,
            json!({"amend": aid, "keep": keep, "waive": []}).to_string(),
        )
        .unwrap();
        self.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
        fs::remove_file(f).unwrap();
        self.say("Yes, approve the change.");
        self.run(&["amend", "approve", &aid, "--quote", "approve the change"])
    }
}

#[test]
fn plan_targets_a_check_of_an_older_plan_is_a_note() {
    let p = Old::new();
    // The plan gate is approved, so the record file is changed in place, as an old plan's file is.
    let file = p.root.join("factory/records/X-1.toml");
    let rec = fs::read_to_string(&file).unwrap();
    assert!(rec.contains("test -f src/hello.txt"), "{rec}");
    fs::write(
        &file,
        rec.replacen("test -f src/hello.txt", "cargo test --test harness_worker_rules", 1),
    )
    .unwrap();
    let out = p.run(&["plan", "check"]).1;
    assert!(out.contains("plan ok"), "{out}");
    let note = out
        .lines()
        .find(|l| l.contains("note: X-1:"))
        .unwrap_or_else(|| panic!("no note for X-1: {out}"));
    assert!(note.contains("--test harness_worker_rules"), "{out}");
}

#[test]
fn plan_targets_a_marked_amendment_makes_the_old_plans_command_a_problem() {
    let p = Old::new();
    // The approval itself is refused: it would add the problem, and an approval adds none (R-69).
    let (code, out) = p.amend_command("cargo test --test harness_worker_rules");
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("would add 1 plan problem"), "{out}");
    assert!(!out.contains("note: X-1:"), "{out}");
    assert!(out.contains("X-1:"), "{out}");
    assert!(out.contains("--test harness_worker_rules"), "{out}");
    // Nothing was approved, so the plan stays as the old plan's: the command is a note, not a problem.
    let out = p.run(&["plan", "check"]).1;
    assert!(out.contains("plan ok"), "{out}");
    assert!(out.contains("note: X-1:"), "{out}");
    assert!(out.contains("--test harness_worker_rules"), "{out}");
}

#[test]
fn plan_targets_a_test_file_a_unit_will_create_is_not_missing() {
    let fx = Fx::new();
    package(&fx.root);
    fx.tiny();
    let unit = fs::read_to_string(fx.root.join("factory/records/U-1.toml")).unwrap();
    let unit = unit.replacen("src/hello.txt", "src/hello.txt\", \"tests/new_one.rs", 1);
    let tmp = tempfile::TempDir::new().unwrap();
    let f = tmp.path().join("u.toml");
    fs::write(&f, unit).unwrap();
    fx.must(&[
        "record",
        "put",
        "unit",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    for (c, autotests) in [
        ("cargo nextest run --package=demo new_one::", false),
        ("cargo test --test new_one", true),
    ] {
        if autotests {
            // With autotests on every file of tests/ is a target, so a file a unit will create is one.
            fs::write(
                fx.root.join("Cargo.toml"),
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
        }
        let g = tmp.path().join("c.toml");
        fs::write(&g, check_with(c)).unwrap();
        fx.must(&[
            "record",
            "put",
            "check",
            "--file",
            g.to_str().unwrap(),
            "--replace",
        ]);
        let o = fx.bf(&["plan", "check"]);
        assert_plan_ok(&o, c);
        assert!(!o.all().contains("note: X-1"), "{c}: {}", o.all());
    }
}

#[test]
fn plan_targets_a_test_file_only_on_the_run_branch_is_not_missing() {
    let fx = Fx::new();
    package(&fx.root);
    fx.tiny();
    git(&fx.root, &["checkout", "-b", "bf/K-1"]);
    fs::write(fx.root.join("tests/landed_one.rs"), "").unwrap();
    git(&fx.root, &["add", "-A"]);
    git(&fx.root, &["commit", "-m", "landed"]);
    git(&fx.root, &["checkout", "-"]);
    assert!(!fx.root.join("tests/landed_one.rs").exists());
    let tmp = tempfile::TempDir::new().unwrap();
    let put = |command: &str| {
        let f = tmp.path().join("c.toml");
        fs::write(&f, check_with(command)).unwrap();
        fx.must(&[
            "record",
            "put",
            "check",
            "--file",
            f.to_str().unwrap(),
            "--replace",
        ]);
        fx.bf(&["plan", "check"])
    };
    let o = put("cargo nextest run --package=demo landed_one::");
    assert_plan_ok(&o, "landed_one");
    assert!(!o.all().contains("note: X-1"), "{}", o.all());
    let o = put("cargo nextest run --package=demo not_landed::");
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("not_landed::"), "{}", o.all());
}

#[test]
fn plan_targets_a_cargo_quiet_switch_takes_no_value() {
    let r = read_command(&words("cargo nextest run --cargo-quiet foo::")).unwrap();
    assert_eq!(r.filters, vec!["foo::".to_string()]);
}
