//! R-81, D-54, TH-33: `install.sh` copies the panel into the project's skills folder by the rule of its
//! other files, and the guard keeps an agent's hands off the installed copy.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A copy of what `install.sh` reads, taken once: `harness/claude` and, two folders up from it,
/// `views/templates`. Nothing of a test reads the live folders after the copy.
struct Harness {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Harness {
    fn new() -> Harness {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap().join("copy");
        let live = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        fs::create_dir_all(root.join("harness")).unwrap();
        fs::create_dir_all(root.join("views")).unwrap();
        for (from, to) in [
            ("harness/claude", "harness/claude"),
            ("views/templates", "views/templates"),
        ] {
            let st = Command::new("cp")
                .arg("-R")
                .arg(live.join(from))
                .arg(root.join(to))
                .status()
                .unwrap();
            assert!(st.success());
        }
        Harness { _tmp: tmp, root }
    }

    fn script(&self) -> PathBuf {
        self.root.join("harness/claude/install.sh")
    }
}

fn install(h: &Harness, args: &[&str], dir: &Path) -> Out {
    let state = dir.parent().unwrap().join("state-home");
    let o = Command::new("bash")
        .arg(h.script())
        .args(args)
        .arg(dir)
        .env("BF_STATE_HOME", state)
        .output()
        .unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).to_string(),
    }
}

/// A new git repository in a temporary folder, inside a parent that holds nothing else.
fn project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let p = fs::canonicalize(tmp.path()).unwrap().join("proj");
    fs::create_dir_all(&p).unwrap();
    git(&p, &["init", "-q", "-b", "main"]);
    (tmp, p)
}

fn panel_dir(root: &Path) -> PathBuf {
    root.join(".claude/skills/factory-panel")
}

fn files_below(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, base: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().to_string());
            }
        }
    }
    let mut v = vec![];
    walk(dir, dir, &mut v);
    v.sort();
    v
}

fn check_copy(root: &Path) {
    let d = panel_dir(root);
    assert!(d.join(".claude-plugin/plugin.json").is_file());
    assert!(d.join("hooks/hooks.json").is_file());
    assert!(!d.join("tests").exists(), "tests/ must not be installed");
    assert!(
        !d.join("fixtures").exists(),
        "fixtures/ must not be installed"
    );
    let plugin: Value =
        serde_json::from_str(&fs::read_to_string(d.join(".claude-plugin/plugin.json")).unwrap())
            .unwrap();
    assert_eq!(plugin["name"], "factory-panel");
}

fn installs_into(h: &Harness, root: &Path, args: &[&str]) {
    let o = install(h, args, root);
    assert_eq!(o.code, 0, "{}", o.all());
    check_copy(root);
    assert!(o
        .stdout
        .contains("created    .claude/skills/factory-panel/hooks/hooks.json"));

    // A second run reports every panel file unchanged.
    let o2 = install(h, args, root);
    assert_eq!(o2.code, 0, "{}", o2.all());
    let lines: Vec<&str> = o2
        .stdout
        .lines()
        .filter(|l| l.contains("skills/factory-panel/"))
        .collect();
    assert!(!lines.is_empty());
    for l in &lines {
        assert!(l.contains("unchanged"), "not unchanged: {l}");
    }

    // A changed file is replaced and the old copy kept beside it.
    let f = panel_dir(root).join("hooks/hooks.json");
    let original = fs::read_to_string(&f).unwrap();
    fs::write(&f, "changed\n").unwrap();
    // A file the panel no longer has is named and left.
    fs::write(panel_dir(root).join("old.txt"), "x").unwrap();
    let o3 = install(h, args, root);
    assert_eq!(o3.code, 0, "{}", o3.all());
    assert!(o3
        .stdout
        .contains("updated    .claude/skills/factory-panel/hooks/hooks.json"));
    assert_eq!(fs::read_to_string(&f).unwrap(), original);
    assert_eq!(
        fs::read_to_string(panel_dir(root).join("hooks/hooks.json.bak")).unwrap(),
        "changed\n"
    );

    assert!(o3.stdout.contains("old.txt"), "{}", o3.all());
    assert!(panel_dir(root).join("old.txt").is_file());
}

#[test]
fn the_panel_is_installed_into_a_project_by_the_rule_of_the_other_files() {
    let h = Harness::new();
    let (_tmp, p) = project();
    installs_into(&h, &p, &[]);
}

#[test]
fn the_panel_is_installed_into_a_workspace() {
    let h = Harness::new();
    let fx = Fx::workspace(&["repo"]);
    installs_into(&h, &fx.root, &["--workspace"]);
}

#[test]
fn nothing_outside_the_project_is_written() {
    let h = Harness::new();
    let (tmp, p) = project();
    let before = files_below(&p.parent().unwrap().join("."))
        .into_iter()
        .filter(|f| !f.starts_with("proj/"))
        .collect::<Vec<_>>();
    let o = install(&h, &[], &p);
    assert_eq!(o.code, 0, "{}", o.all());
    let after = files_below(tmp.path())
        .into_iter()
        .filter(|f| !f.starts_with("proj/"))
        .collect::<Vec<_>>();
    assert_eq!(before, after);
}

#[test]
fn a_symbolic_link_at_the_panel_folder_is_refused() {
    let h = Harness::new();
    let (tmp, p) = project();
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(p.join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, panel_dir(&p)).unwrap();
    let o = install(&h, &[], &p);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("symbolic link"), "{}", o.all());
    assert!(files_below(&elsewhere).is_empty());
}

#[test]
fn a_symbolic_link_at_a_backup_path_is_refused_and_writes_nothing() {
    let h = Harness::new();
    let (tmp, p) = project();
    assert_eq!(install(&h, &[], &p).code, 0);
    let outside = tmp.path().join("outside.txt");
    let f = panel_dir(&p).join("hooks/hooks.json");
    fs::write(&f, "changed\n").unwrap();
    std::os::unix::fs::symlink(&outside, panel_dir(&p).join("hooks/hooks.json.bak")).unwrap();
    let o = install(&h, &[], &p);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("symbolic link"), "{}", o.all());
    assert!(!outside.exists());
    assert_eq!(fs::read_to_string(&f).unwrap(), "changed\n");
}

#[test]
fn a_deep_symbolic_link_leaves_nothing_written() {
    let h = Harness::new();
    let (tmp, p) = project();
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(panel_dir(&p).join("src")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, panel_dir(&p).join("src/render")).unwrap();
    let o = install(&h, &[], &p);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(files_below(&elsewhere).is_empty());
    assert!(!panel_dir(&p).join(".claude-plugin").exists());
    assert!(!p.join(".claude/skills/factory").exists());
}

#[test]
fn a_change_in_another_copy_of_the_harness_does_not_reach_the_installs() {
    let h = Harness::new();
    let other = Harness::new();
    let (_tmp, p) = project();
    assert_eq!(install(&h, &[], &p).code, 0);
    let live_file = other.root.join("harness/claude/panel/hooks/hooks.json");
    assert!(live_file.is_file());
    fs::write(&live_file, "changed in the other copy\n").unwrap();
    let o = install(&h, &[], &p);
    assert_eq!(o.code, 0, "{}", o.all());
    let lines: Vec<&str> = o
        .stdout
        .lines()
        .filter(|l| l.contains("skills/factory-panel/"))
        .collect();
    assert!(!lines.is_empty());
    for l in &lines {
        assert!(l.contains("unchanged"), "not unchanged: {l}");
    }
}

#[test]
fn the_script_each_test_runs_lies_under_its_temporary_folder() {
    let h = Harness::new();
    assert!(h.script().starts_with(&h.root));
    assert!(h
        .script()
        .starts_with(h._tmp.path().canonicalize().unwrap()));
    assert!(!h.script().starts_with(env!("CARGO_MANIFEST_DIR")));
    assert!(h.root.join("views/templates/page.html").is_file());
}

fn guard(fx: &Fx, tool: &str, input: Value) -> (String, String) {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": fx.root.to_string_lossy()});
    let o = fx.bf_stdin(&fx.root, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => ("abstain".into(), String::new()),
        s => {
            let v: Value = serde_json::from_str(s).unwrap();
            let h = &v["hookSpecificOutput"];
            (
                h["permissionDecision"].as_str().unwrap().to_string(),
                h["permissionDecisionReason"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
        }
    }
}

#[test]
fn the_guard_refuses_writes_into_the_installed_panel_and_allows_reading() {
    let fx = Fx::new();
    let file = fx
        .root
        .join(".claude/skills/factory-panel/hooks/hooks.json");
    let file = file.to_string_lossy().to_string();

    let (d, why) = guard(&fx, "Write", json!({"file_path": file, "content": "x"}));
    assert_eq!(d, "deny", "{why}");
    assert!(why.contains(".claude/skills/factory-panel"), "{why}");

    let (d, why) = guard(
        &fx,
        "Bash",
        json!({"command": "echo x > .claude/skills/factory-panel/hooks/hooks.json"}),
    );
    assert_eq!(d, "deny", "{why}");
    assert!(why.contains(".claude/skills/factory-panel"), "{why}");

    let (d, why) = guard(&fx, "Read", json!({"file_path": file}));
    assert_ne!(d, "deny", "{why}");
    let (d, why) = guard(
        &fx,
        "Bash",
        json!({"command": "cat .claude/skills/factory-panel/hooks/hooks.json"}),
    );
    assert_ne!(d, "deny", "{why}");
}

/// D-140: the installer renders all worker-rules files in one python3 call; the installed tree and the
/// standard output are the same as those of the installer before that change, kept as a fixture copy.
#[test]
fn the_batch_render_installs_the_same_tree_and_prints_the_same_output_as_the_per_file_render() {
    let old = Harness::new();
    let new = Harness::new();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/install_before_batch_render.sh");
    fs::copy(&fixture, old.script()).unwrap();
    let (_t1, p_old) = project();
    let (_t2, p_new) = project();
    let o_old = install(&old, &[], &p_old);
    let o_new = install(&new, &[], &p_new);
    assert_eq!(o_old.code, 0, "{}", o_old.all());
    assert_eq!(o_new.code, 0, "{}", o_new.all());
    let norm = |o: &Out, p: &Path| {
        // The project and the state folder both lie under the project's parent folder.
        o.stdout
            .replace(p.parent().unwrap().to_str().unwrap(), "<tmp>")
    };
    assert_eq!(norm(&o_old, &p_old), norm(&o_new, &p_new));
    assert_eq!(o_old.stderr, o_new.stderr);
    let root = |p: &Path| p.join(".claude");
    let (a, b) = (files_below(&root(&p_old)), files_below(&root(&p_new)));
    assert_eq!(a, b);
    assert!(a.iter().any(|f| f.starts_with("agents/")), "{a:?}");
    for f in &a {
        let mut x = fs::read(root(&p_old).join(f)).unwrap();
        let mut y = fs::read(root(&p_new).join(f)).unwrap();
        if f == "settings.json" {
            // It names the state folder, which lies under each project's own parent folder.
            let cut = |b: Vec<u8>, p: &Path| {
                String::from_utf8(b)
                    .unwrap()
                    .replace(p.parent().unwrap().to_str().unwrap(), "<tmp>")
                    .into_bytes()
            };
            x = cut(x, &p_old);
            y = cut(y, &p_new);
        }
        assert!(x == y, "installed file differs: {f}");
    }
}
