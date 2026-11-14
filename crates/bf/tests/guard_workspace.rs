//! The guard's shell rules in a workspace and in a unit's working copy: the git folder of a code
//! repository is protected from the shell as it is from the write tools, a copy out by an absolute path is
//! a read, a relative link target is read from the link's folder, and a recursive permission change of a
//! whole working copy is judged like a delete of it.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn decision(fx: &Fx, cwd: &Path, tool: &str, input: Value) -> (String, String) {
    let payload = json!({"tool_name": tool, "tool_input": input, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => ("pass".into(), String::new()),
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

fn refused(fx: &Fx, cwd: &Path, cmd: &str) -> String {
    let (d, why) = decision(fx, cwd, "Bash", json!({ "command": cmd }));
    assert_eq!(d, "deny", "must be refused: {cmd}");
    why
}

fn passes(fx: &Fx, cwd: &Path, cmd: &str) {
    let (d, why) = decision(fx, cwd, "Bash", json!({ "command": cmd }));
    assert_ne!(d, "deny", "must pass: {cmd}\n{why}");
}

fn records(fx: &Fx, files: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[{files}]\ncheckpoint=\"K-1\"\n")),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n".to_string()),
    ] {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("records");
    fx.start_test_run();
}

fn working_copy(fx: &Fx) -> PathBuf {
    PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

#[test]
fn a_code_repositorys_git_folder_is_closed_to_shell_writes_from_the_workspace_root() {
    let fx = Fx::workspace(&["app"]);
    let a = fx.repo_dir("app").display().to_string();
    for cmd in [
        format!("echo 'touch /tmp/x' > {a}/.git/hooks/post-merge"),
        format!("cp /tmp/h {a}/.git/hooks/post-merge"),
        format!("cp /tmp/post-merge {a}/.git/hooks/"),
        format!("chmod +x {a}/.git/hooks/post-merge"),
        format!("printf 'a=b' >> {a}/.git/config"),
        format!("cp /tmp/config {a}/.git/config"),
        format!("rm -rf {a}/.git"),
        format!("mv {a}/.git /tmp/x"),
        format!("rm -rf {a}"),
        format!("sed -i 's/a/b/' {a}/.git/config"),
        format!("cd {a} && echo x > .git/hooks/pre-commit"),
    ] {
        let why = refused(&fx, &fx.root, &cmd);
        assert!(why.contains("/.git"), "the refusal names the path: {why}");
    }
}

#[test]
fn everyday_commands_on_a_code_repository_still_pass_from_the_workspace_root() {
    let fx = Fx::workspace(&["app"]);
    let a = fx.repo_dir("app").display().to_string();
    for cmd in [
        format!("git -C {a} log --oneline -3"),
        format!("git -C {a} status --short"),
        format!("ls {a}"),
        format!("ls -la {a}/.git/hooks"),
        format!("cat {a}/.git/config"),
        format!("cp {a}/.git/config /tmp/config.copy"),
        format!("cp {a}/README.md /tmp/README.copy"),
        format!("echo x > {a}/notes.txt"),
        format!("cd {a} && git log -1"),
        "find . -name '.DS_Store' -delete".to_string(),
        "mkdir -p .bf/drafts && cp /tmp/n.md .bf/drafts/".to_string(),
    ] {
        passes(&fx, &fx.root, &cmd);
    }
}

#[test]
fn the_write_tool_and_the_shell_judge_the_git_folder_alike() {
    let fx = Fx::workspace(&["app"]);
    let hook = fx.repo_dir("app").join(".git/hooks/post-merge");
    let (d, _) = decision(
        &fx,
        &fx.root,
        "Write",
        json!({"file_path": hook.to_string_lossy()}),
    );
    assert_eq!(d, "deny");
}

#[test]
fn a_builder_in_a_workspace_cannot_reach_a_code_repositorys_git_folder() {
    let fx = Fx::workspace(&["app"]);
    records(&fx, "\"src/**\"");
    let wc = working_copy(&fx);
    let a = fx.repo_dir("app").display().to_string();
    refused(&fx, &wc, &format!("echo x > {a}/.git/hooks/post-merge"));
    refused(&fx, &wc, &format!("cp /tmp/c {a}/.git/config"));
}

#[test]
fn copying_a_record_out_by_its_absolute_path_is_a_read() {
    let fx = Fx::new();
    fx.tiny();
    let r = fx.root.display().to_string();
    for cmd in [
        format!("python3 -c \"import shutil; shutil.copy('{r}/factory/records/R-1.toml', '/tmp/r1.toml')\""),
        format!("python3 -c \"import json,sys; json.dump(json.load(open('{r}/.bf/guard.json')), open('/tmp/o.json','w'))\""),
        format!("python3 -c \"print(open('{r}/factory/policy.toml').read())\""),
        format!("python3 -c \"import tomllib; d=tomllib.load(open('{r}/factory/records/R-1.toml','rb')); open('/tmp/o.txt','w').write(d['name'])\""),
        format!("python3 -c \"import tomllib; d=tomllib.load(open('{r}/factory/records/R-1.toml','rb')); items=[1,2]; items.remove(1); open('/tmp/o.txt','w').write(str(items))\""),
    ] {
        passes(&fx, &fx.root, &cmd);
    }
    // A write of the same file by its absolute path stays refused, in each form.
    for cmd in [
        format!("python3 -c \"import os; f=open('{r}/factory/policy.toml'); os.unlink(f.name)\""),
        format!("python3 -c \"import os; os.chmod(open('{r}/factory/policy.toml').name, 0o777)\""),
        format!("python3 -c \"import os; f=open('{r}/factory/policy.toml'); os.remove(f.name)\""),
        format!("python3 -c \"import shutil; shutil.copy('/tmp/x', '{r}/factory/policy.toml')\""),
        format!("python3 -c \"open('{r}/factory/policy.toml', mode='w').write('x')\""),
        format!("python3 -c \"p='{r}/factory/policy.toml'; open(p, 'w').write('x')\""),
        format!("python3 -c \"import os; os.chmod('{r}/factory/policy.toml', 0o777)\""),
        format!("python3 - <<'EOF'\nf = open('{r}/factory/policy.toml', mode='w')\nEOF"),
        format!("node -e \"const fs=require('fs'); const s=fs.createReadStream('{r}/factory/records/R-1.toml'); fs.writeFileSync(s.path,'x')\""),
        format!("ruby -e \"File.write(File.open('{r}/factory/policy.toml').path, 'x')\""),
        format!("python3 -c \"import os; f=open('{r}/factory/policy.toml'); os.remove(f.name)\""),
        format!("python3 -c \"import pathlib; f=open('{r}/factory/policy.toml'); pathlib.Path(f.name).unlink()\""),
        format!("ruby -e \"f=File.open('{r}/factory/policy.toml'); system 'rm ' + f.path\""),
    ] {
        refused(&fx, &fx.root, &cmd);
    }
    // Plain node code that loads the `path` module is not a write.
    passes(
        &fx,
        &fx.root,
        &format!("node -e \"const p=require('path'); const fs=require('fs'); console.log(p.join('a','b'), fs.readFileSync('{r}/factory/policy.toml','utf8'))\""),
    );
}

#[test]
fn a_relative_link_target_is_read_from_the_links_folder() {
    let fx = Fx::new();
    records(&fx, "\"src/**\", \"build/**\"");
    let wc = working_copy(&fx);
    std::fs::create_dir_all(wc.join("src")).unwrap();
    std::fs::create_dir_all(wc.join("build")).unwrap();
    passes(&fx, &wc, "ln -s ../src/a.rs build/a.rs");
    passes(&fx, &wc, "ln -s ../src/a.rs build/");
    // From the link's folder this points out of the copy, though from the root it would not.
    refused(&fx, &wc, "ln -s ../../x build/a.rs");
}

#[test]
fn a_relative_link_made_with_r_reads_its_target_from_the_working_directory() {
    let fx = Fx::new();
    fx.tiny();
    std::fs::create_dir_all(fx.root.join("build")).unwrap();
    refused(&fx, &fx.root, "ln -sr factory/records/R-1.toml build/r");
    refused(
        &fx,
        &fx.root,
        "ln -s --relative factory/records/R-1.toml build/r",
    );
    refused(&fx, &fx.root, "ln -s ../factory/records/R-1.toml build/r");
    passes(&fx, &fx.root, "ln -sr README.md build/readme");
}

#[test]
fn a_recursive_permission_change_of_the_whole_working_copy_is_refused() {
    let fx = Fx::new();
    records(&fx, "\"src/**\"");
    let wc = working_copy(&fx);
    std::fs::create_dir_all(wc.join("src")).unwrap();
    for cmd in ["chmod -R 000 .", "chmod -R 755 ./", "chown -R nobody ."] {
        let why = refused(&fx, &wc, cmd);
        assert!(why.contains("whole working copy"), "{why}");
    }
    for cmd in [
        "chmod -R go-w .",
        "chmod -R 755 src/a",
        "chmod +x src/run.sh",
        "chmod 644 .",
    ] {
        passes(&fx, &wc, cmd);
    }
}
