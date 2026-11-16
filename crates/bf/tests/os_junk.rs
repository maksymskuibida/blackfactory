//! Files an operating system writes by itself into a folder it shows (`.DS_Store`, `._*`,
//! `Thumbs.db`) are not content: they never change the content hash, never count as uncommitted
//! and are never landed. Every other name counts as before.

mod common;
use common::*;
use std::fs;
use std::path::{Path, PathBuf};

const JUNK: [&str; 3] = [".DS_Store", "._x", "Thumbs.db"];

/// The working copies under `wc`: itself, or one per repository for a checkpoint over several.
fn copies(wc: &Path) -> Vec<PathBuf> {
    if wc.join(".git").exists() {
        return vec![wc.to_path_buf()];
    }
    fs::read_dir(wc)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".git").exists())
        .collect()
}

/// Write every junk name at the top and in a subfolder of every working copy under `wc`.
fn junk(wc: &Path, text: &str) {
    for c in copies(wc) {
        fs::create_dir_all(c.join("src")).unwrap();
        for n in JUNK {
            fs::write(c.join(n), text).unwrap();
            fs::write(c.join("src").join(n), text).unwrap();
        }
    }
}

fn plan(fx: &Fx, repos: &[&str]) {
    let mut checks = String::new();
    let mut units = Vec::new();
    for (i, r) in repos.iter().enumerate() {
        let n = i + 1;
        let repo = if r.is_empty() {
            String::new()
        } else {
            format!("repo=\"{r}\"\n")
        };
        let f = format!("src/f{n}.txt");
        let t = format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"u{n}\"\ndelivers=[\"R-1\"]\nfiles=[\"{f}\"]\n{repo}checkpoint=\"K-1\"\n");
        assert_eq!(fx.put("unit", &t).code, 0);
        let c = format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"c{n}\"\nhow=\"command\"\ncommand=\"test -f {f}\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n");
        assert_eq!(fx.put("check", &c).code, 0);
        units.push(format!("\"U-{n}\""));
        checks.push_str(&c);
    }
    let kx = repos.len() + 1;
    let r = fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n",
    );
    assert_eq!(r.code, 0);
    let c = format!("id=\"X-{kx}\"\nkind=\"check\"\ntitle=\"ck\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n");
    assert_eq!(fx.put("check", &c).code, 0);
    let k = format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[{}]\nproves=[\"R-1\"]\nchecks=[\"X-{kx}\"]\n", units.join(","));
    let r = fx.put("checkpoint", &k);
    assert_eq!(r.code, 0, "{}", r.all());
    fx.commit_all("records");
}

fn real_run(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
}

/// Start unit `n`, write its file, run its check, then let the system write junk, then verify,
/// report and land it.
fn build_with_junk(fx: &Fx, n: usize) {
    let id = format!("U-{n}");
    let wc = PathBuf::from(
        fx.must(&["unit", "start", &id, "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join(format!("src/f{n}.txt")), "content\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            &format!("X-{n}"),
            "--unit",
            &id,
            "--",
            "test",
            "-f",
            &format!("src/f{n}.txt"),
        ],
    );
    junk(&wc, "after the check\n");
    assert_eq!(fx.bf(&["evidence", "verify", "--unit", &id]).code, 0);
    fx.must(&["report", &id, "--result", "built"]);
    // A file that changes or disappears after the check changes nothing either.
    junk(&wc, "changed again\n");
    fs::remove_file(wc.join("src/.DS_Store")).unwrap();
    fx.must(&["report", &id, "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", &id]);
}

/// No junk file is on `rev`, except the top-level `.DS_Store` the human committed before the
/// run in the project folder: that one must still hold its pre-run content.
fn assert_no_junk_on(repo: &Path, rev: &str) {
    let files = git(repo, &["ls-tree", "-r", "--name-only", rev]);
    for l in files.lines() {
        let name = l.rsplit('/').next().unwrap();
        if l == ".DS_Store" && repo.join(".bf").exists() {
            let blob = git(repo, &["show", &format!("{rev}:.DS_Store")]);
            assert_eq!(blob, "tracked", "landing changed tracked junk on {rev}");
            continue;
        }
        assert!(
            name != ".DS_Store" && name != "Thumbs.db" && !name.starts_with("._"),
            "junk on {rev}: {l}"
        );
    }
}

fn assert_none_landed(repo: &Path) {
    assert_no_junk_on(repo, "bf/K-1");
}

fn walk(fx: &Fx, repos: &[&str], dirs: &[PathBuf], repeat_after_sync: bool) {
    real_run(fx);
    for i in 1..=repos.len() {
        build_with_junk(fx, i);
    }
    for d in dirs {
        assert_none_landed(d);
        assert!(git(d, &["ls-tree", "-r", "--name-only", "bf/K-1"]).contains("src/f"));
    }
    let kx = format!("X-{}", repos.len() + 1);
    let k = fx.work("K-1");
    let run_k = || fx.must_in(&k, &["evidence", "run", &kx, "--", "true"]);
    run_k();
    // The sync's test for a clean working copy passes with junk present.
    junk(&k, "in the checkpoint\n");
    let main = dirs[0].clone();
    fs::write(main.join("on_main.txt"), "main\n").unwrap();
    git(&main, &["add", "on_main.txt"]);
    git(&main, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    let o = fx.must(&[
        "checkpoint",
        "sync",
        "K-1",
        "--local",
        "--quote",
        "bring main in",
        "--json",
    ]);
    assert_eq!(o.json()["synced"], true, "{}", o.all());
    if repeat_after_sync {
        fx.must_in(
            &k,
            &[
                "evidence",
                "run",
                &kx,
                "--attempt",
                "2",
                "--reason",
                "run again after the sync",
                "--",
                "true",
            ],
        );
    } else {
        fx.must_in(&k, &["evidence", "run", &kx, "--", "true"]);
    }
    // Junk after the check: the receipt stays valid and the checkpoint closes.
    junk(&k, "after the checkpoint check\n");
    let out = fx.merge_checkpoint("K-1");
    assert_eq!(out.code, 0, "{}", out.all());
    for d in dirs {
        assert_none_landed_on_main(d);
    }
}

fn assert_none_landed_on_main(repo: &Path) {
    assert_no_junk_on(repo, "main");
}

#[test]
fn in_a_project_folder_junk_never_counts_and_is_never_landed() {
    let fx = Fx::new();
    // A tracked junk file, changed after the check, does not count either.
    fs::write(fx.root.join(".DS_Store"), "tracked\n").unwrap();
    plan(&fx, &[""]);
    assert!(git(&fx.root, &["ls-files"]).contains(".DS_Store"));
    walk(&fx, &[""], std::slice::from_ref(&fx.root), false);
}

#[test]
fn in_a_workspace_with_two_repositories_junk_never_counts_and_is_never_landed() {
    let fx = Fx::workspace(&["app", "lib"]);
    plan(&fx, &["app", "lib"]);
    walk(
        &fx,
        &["app", "lib"],
        &[fx.repo_dir("app"), fx.repo_dir("lib")],
        true,
    );
}

#[test]
fn a_new_file_of_any_other_name_still_changes_the_hash() {
    let fx = Fx::new();
    plan(&fx, &[""]);
    real_run(&fx);
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/f1.txt"), "content\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-1",
            "--unit",
            "U-1",
            "--",
            "test",
            "-f",
            "src/f1.txt",
        ],
    );
    assert_eq!(fx.bf(&["evidence", "verify", "--unit", "U-1"]).code, 0);
    for name in ["DS_Store.txt", "my.DS_Store", "x._y", "Thumbs.db.bak", "_x"] {
        fs::write(wc.join(name), "x\n").unwrap();
        assert_ne!(
            fx.bf(&["evidence", "verify", "--unit", "U-1"]).code,
            0,
            "{name} must count"
        );
        fs::remove_file(wc.join(name)).unwrap();
        assert_eq!(fx.bf(&["evidence", "verify", "--unit", "U-1"]).code, 0);
    }
}
