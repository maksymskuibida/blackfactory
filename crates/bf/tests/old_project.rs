//! A project made by the engine as it was before the new engine: the fixture in
//! `tests/fixtures/old_project/` (the project folder and the engine's state folder beside it).
//! The first test proves that today's engine reads it and leaves every file of it as it was. Later
//! tests reuse `OldProject` and `hash_tree` to prove the same of the new engine.

mod common;
use common::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// The path of every file below `dir` (relative, with `/`) and its SHA-256, in path order.
pub fn hash_tree(dir: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, bf::util::sha256_file(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// The fixture as shipped in the repository.
pub fn old_project_fixture() -> PathBuf {
    fixture("old_project")
}

/// A copy of the fixture in a temporary folder: the project and the engine's state folder.
pub struct OldProject {
    _tmp: TempDir,
    /// The copy of the project (`factory/` and the files beside it).
    pub root: PathBuf,
    /// The copy of the state folder, given to the engine as `BF_STATE_HOME`.
    pub state_base: PathBuf,
}

impl OldProject {
    pub fn copy() -> OldProject {
        let tmp = TempDir::new().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let src = old_project_fixture();
        for part in ["project", "state"] {
            let st = Command::new("cp")
                .arg("-R")
                .arg(src.join(part))
                .arg(base.join(part))
                .status()
                .unwrap();
            assert!(st.success());
        }
        OldProject {
            root: base.join("project"),
            state_base: base.join("state"),
            _tmp: tmp,
        }
    }

    /// Run the engine at `bin` in the project copy.
    pub fn run_with(&self, bin: &str, args: &[&str]) -> Out {
        let o = Command::new(bin)
            .env("BF_STATE_HOME", &self.state_base)
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        }
    }

    /// Run the engine under test.
    pub fn run(&self, args: &[&str]) -> Out {
        self.run_with(bin(), args)
    }

    /// The hash of every file of the copy, project and state folder.
    pub fn hashes(&self) -> BTreeMap<String, String> {
        let mut all = BTreeMap::new();
        for (part, dir) in [("project", &self.root), ("state", &self.state_base)] {
            for (k, v) in hash_tree(dir) {
                all.insert(format!("{part}/{k}"), v);
            }
        }
        all
    }
}

fn print_hashes(label: &str, h: &BTreeMap<String, String>) {
    println!("hashes {label}:");
    for (k, v) in h {
        println!("  {v}  {k}");
    }
}

#[test]
fn fixture_is_what_the_old_engine_made_and_holds_no_private_text() {
    let h = hash_tree(&old_project_fixture());
    for must in [
        "project/factory/log/events.jsonl",
        "project/factory/policy.toml",
        "project/factory/receipts/rc-00001.toml",
        "project/factory/receipts/rc-00003.toml",
        "project/factory/records/K-1.toml",
    ] {
        assert!(h.contains_key(must), "the fixture lacks {must}");
    }
    assert!(
        !h.keys().any(|k| k.contains(".bf/")),
        "the fixture holds session files"
    );
    for k in h.keys() {
        let text = fs::read(old_project_fixture().join(k)).unwrap();
        let text = String::from_utf8_lossy(&text);
        assert!(!text.contains("/Users/"), "{k} holds a home path");
        assert!(!text.contains("/home/"), "{k} holds a home path");
    }
}

#[test]
fn todays_engine_reads_the_old_project_and_changes_no_file() {
    let p = OldProject::copy();
    let before = p.hashes();
    assert_eq!(before, {
        let mut want = BTreeMap::new();
        for (part, name) in [("project", "project"), ("state", "state")] {
            for (k, v) in hash_tree(&old_project_fixture().join(name)) {
                want.insert(format!("{part}/{k}"), v);
            }
        }
        want
    });
    print_hashes("before", &before);

    let st = p.run(&["status", "--json"]);
    assert_eq!(st.code, 0, "{}", st.all());
    let st = st.json();
    assert_eq!(st["run"], "run-1");
    let units = st["units"].to_string();
    assert!(units.contains("U-1") && units.contains("U-2"), "{st}");
    let gates = st["gates"].as_array().unwrap();
    assert!(
        gates
            .iter()
            .any(|g| g["id"] == "plan-r1" && g["status"] == "approved"),
        "{st}"
    );

    let next = p.run(&["next", "--json"]);
    assert_eq!(next.code, 0, "{}", next.all());
    next.json();

    let log = p.run(&["log", "--verify"]);
    assert_eq!(log.code, 0, "{}", log.all());
    assert!(log.stdout.contains("chain ok (27 events)"), "{}", log.all());

    let after = p.hashes();
    print_hashes("after", &after);
    assert_eq!(
        before, after,
        "an old project's files changed by being read"
    );
}
