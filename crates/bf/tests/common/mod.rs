#![allow(dead_code)]
//! Shared helpers for the integration tests: a temporary git project and the real `bf` binary.

pub mod schema;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout is not JSON ({e}): {}\nstderr: {}",
                self.stdout, self.stderr
            )
        })
    }
    pub fn all(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

pub struct Fx {
    _tmp: TempDir,
    scratch: TempDir,
    pub root: PathBuf,
    /// Base of the per-project state folders (the sealing key and the log head live below it).
    pub state_base: PathBuf,
}

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_bf")
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

impl Fx {
    /// A fresh git repository with one commit and `bf init` done.
    pub fn new() -> Fx {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap().join("proj");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        fs::write(root.join("README.md"), "# toy\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "init"]);
        let state_base = root.parent().unwrap().join("state");
        let tr_root = root.parent().unwrap().join("transcripts");
        fs::create_dir_all(&tr_root).unwrap();
        // The fixture's policy: a fixed PATH, no role tokens (the token tests turn them on), and a
        // transcript root inside the temporary folder. `bf init` keeps a policy that is already there.
        let git_bin = [
            "/usr/bin/git",
            "/opt/homebrew/bin/git",
            "/usr/local/bin/git",
        ]
        .iter()
        .find(|g| Path::new(g).exists())
        .unwrap_or(&"/usr/bin/git");
        let pol = bf::store::default_policy_text("/usr/bin:/bin:/usr/sbin:/sbin", git_bin)
            .replace("tokens = true", "tokens = false")
            .replace(
                "transcript_root = \"~/.claude/projects\"",
                &format!("transcript_root = {:?}", tr_root.to_string_lossy()),
            );
        fs::create_dir_all(root.join("factory")).unwrap();
        fs::write(root.join("factory/policy.toml"), pol).unwrap();
        let fx = Fx {
            _tmp: tmp,
            scratch: TempDir::new().unwrap(),
            root,
            state_base,
        };
        let o = fx.bf(&["init", "--name", "toy"]);
        assert_eq!(o.code, 0, "{}", o.all());
        git(&fx.root, &["add", "-A"]);
        git(&fx.root, &["commit", "-q", "-m", "bf init"]);
        fx
    }

    pub fn bf_in(&self, dir: &Path, args: &[&str]) -> Out {
        self.bf_stdin(dir, args, "")
    }

    pub fn bf(&self, args: &[&str]) -> Out {
        self.bf_in(&self.root, args)
    }

    pub fn bf_stdin(&self, dir: &Path, args: &[&str], stdin: &str) -> Out {
        self.bf_env(dir, args, stdin, &[])
    }

    pub fn bf_env(&self, dir: &Path, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Out {
        let mut c = Command::new(bin());
        c.env("BF_STATE_HOME", &self.state_base);
        c.args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            c.env(k, v);
        }
        let mut child = c.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
        let o = child.wait_with_output().unwrap();
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        }
    }

    /// Record the human's typed message the way the harness hook does: the payload names a
    /// transcript file, and the file ends with this message.
    pub fn human(&self, text: &str) -> Out {
        let tr = self.transcript_root().join("transcript.jsonl");
        let line =
            serde_json::json!({"type": "user", "message": {"role": "user", "content": text}});
        std::fs::write(&tr, format!("{line}\n")).unwrap();
        let payload = serde_json::json!({
            "session_id": "s1", "transcript_path": tr.to_string_lossy(), "cwd": self.root.to_string_lossy(),
            "hook_event_name": "UserPromptSubmit", "prompt": text
        });
        self.bf_stdin(&self.root, &["human-turn"], &payload.to_string())
    }

    /// Where the fixture's policy says the harness keeps transcripts.
    pub fn transcript_root(&self) -> PathBuf {
        self.state_base.parent().unwrap().join("transcripts")
    }

    /// Change the policy the way the human does: edit the file, say so in chat, accept it.
    pub fn set_policy(&self, from: &str, to: &str) {
        let pol = self.root.join("factory/policy.toml");
        let t = fs::read_to_string(&pol).unwrap();
        assert!(t.contains(from), "policy has no `{from}`");
        fs::write(&pol, t.replace(from, to)).unwrap();
        self.human("Yes, I accept the new policy.");
        self.must(&[
            "policy",
            "accept",
            "--quote",
            "Yes, I accept the new policy.",
        ]);
    }

    /// The per-project state folder (key, log head, role tokens).
    pub fn state_dir(&self) -> PathBuf {
        let cfg =
            std::fs::read_to_string(self.root.join("factory/config.toml")).unwrap_or_default();
        let id = cfg
            .lines()
            .find_map(|l| l.strip_prefix("project_id = "))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default();
        self.state_base.join(id)
    }

    /// The status of a gate, read from `bf status`.
    pub fn must_state_gate(&self, id: &str) -> String {
        let st = self.must(&["status", "--json"]).json();
        st["gates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == id)
            .unwrap()["status"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A project handle for tests that call the library directly.
    pub fn project(&self) -> bf::store::Project {
        let mut p = bf::store::Project::discover(&self.root).unwrap();
        p.state_base = Some(self.state_base.clone());
        p.reload_policy().unwrap();
        p
    }

    pub fn put(&self, kind: &str, toml_text: &str) -> Out {
        let n = fs::read_dir(self.scratch.path()).unwrap().count();
        let f = self.scratch.path().join(format!("rec{n}.toml"));
        fs::write(&f, toml_text).unwrap();
        self.bf(&["record", "put", kind, "--file", f.to_str().unwrap()])
    }

    pub fn must(&self, args: &[&str]) -> Out {
        let o = self.bf(args);
        assert_eq!(o.code, 0, "bf {:?} failed:\n{}", args, o.all());
        o
    }

    pub fn must_in(&self, dir: &Path, args: &[&str]) -> Out {
        let o = self.bf_in(dir, args);
        assert_eq!(
            o.code,
            0,
            "bf {:?} in {} failed:\n{}",
            args,
            dir.display(),
            o.all()
        );
        o
    }

    pub fn refused(&self, args: &[&str]) -> Out {
        let o = self.bf(args);
        assert_eq!(o.code, 2, "bf {:?} should be refused:\n{}", args, o.all());
        assert_four_parts(&o.stderr);
        o
    }

    pub fn commit_all(&self, msg: &str) {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", msg]);
    }

    pub fn work(&self, id: &str) -> PathBuf {
        self.root.join(".bf/work").join(id)
    }

    /// The tiny project used by the end-to-end and attack tests.
    pub fn tiny(&self) {
        let recs: &[(&str, &str)] = &[
            ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/hello.txt exists\"]\n"),
            ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"A farewell exists\"\nacceptance=[\"src/bye.txt exists\"]\n"),
            ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n"),
            ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/hello.txt\"]\ncheckpoint=\"K-1\"\nestimate=2\n"),
            ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"Farewell\"\ndelivers=[\"R-2\"]\nfiles=[\"src/bye.txt\"]\ndepends_on=[\"U-1\"]\ncheckpoint=\"K-1\"\nestimate=1\n"),
            ("handoff", "id=\"H-1\"\nkind=\"handoff\"\nfrom=\"U-1\"\nto=\"U-2\"\nthing=\"the greeting wording\"\ncontract=\"Same tone as the greeting.\"\n"),
            ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
            ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"Farewell file\"\nhow=\"command\"\ncommand=\"test -f src/bye.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
            ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"ls src/hello.txt src/bye.txt\"\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nscope=\"checkpoint\"\n"),
            ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\n"),
        ];
        for (k, t) in recs {
            let o = self.put(k, t);
            assert_eq!(o.code, 0, "{}", o.all());
        }
        self.commit_all("records");
    }

    /// Grant test mode, start a run and approve the plan gate as a simulation.
    pub fn start_test_run(&self) {
        self.human("yes, make this a test run with simulated approvals");
        self.must(&[
            "test-mode",
            "grant",
            "--quote",
            "yes, make this a test run with simulated approvals",
        ]);
        self.must(&["run", "start"]);
        self.must(&["gate", "open", "plan"]);
        self.must(&["gate", "approve", "plan", "--simulated"]);
    }

    /// A real run's way to main: close the checkpoint, the human says merge in chat, the step gate is
    /// approved with their words, and only then does `--merge` work. Returns the output of the merge.
    pub fn merge_checkpoint(&self, k: &str) -> Out {
        self.must(&["checkpoint", "close", k]);
        let gid = self
            .must(&["gate", "open", "checkpoint-step", "--ref", k])
            .stdout
            .trim()
            .to_string();
        self.human("Looks good, merge it. I approve.");
        self.must(&["gate", "approve", &gid, "--quote", "I approve"]);
        self.bf(&["checkpoint", "close", k, "--merge", "--json"])
    }

    /// Start a unit, write its file, run its check, report built and clean, and land it.
    pub fn build_unit(&self, id: &str, file: &str, check: &str, cmd: &[&str]) {
        let o = self.must(&["unit", "start", id, "--json"]);
        let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join(file), format!("{id} content\n")).unwrap();
        let mut a = vec!["evidence", "run", check, "--unit", id, "--"];
        a.extend_from_slice(cmd);
        self.must_in(&wc, &a);
        self.must(&["report", id, "--result", "built"]);
        self.must(&["report", id, "--result", "built", "--role", "reviewer"]);
        self.must(&["unit", "land", id]);
    }
}

pub fn assert_four_parts(text: &str) {
    for part in [
        "refused: ",
        "because: ",
        "do this:",
        "  1. ",
        "it worked when: ",
    ] {
        assert!(text.contains(part), "refusal lacks `{part}`:\n{text}");
    }
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The view contract's JSON Schema (`views/contract/view.schema.json`).
pub fn view_schema() -> serde_json::Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views/contract/view.schema.json");
    serde_json::from_str(&fs::read_to_string(p).expect("views/contract/view.schema.json")).unwrap()
}

pub fn assert_contract(kind: &str, v: &serde_json::Value) {
    let errs = schema::validate(&view_schema(), v);
    assert!(
        errs.is_empty(),
        "{kind} view breaks the contract:\n{}\n{v:#}",
        errs.join("\n")
    );
}
