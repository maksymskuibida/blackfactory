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

/// F-87: ruby hands the file object a read call returns to a delete call. Both calls delete only on
/// their owners (`File`, `FileUtils`); a key or a method of the same name on anything else passes.
#[test]
fn ruby_deleting_through_a_file_object_a_read_returns_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    let r = fx.root.display().to_string();
    for cmd in [
        format!("ruby -e \"File.delete(File.open('{r}/factory/policy.toml'))\""),
        format!("ruby -e \"require 'fileutils'; FileUtils.rm(File.open('{r}/factory/records/R-1.toml'))\""),
    ] {
        refused(&fx, &fx.root, &cmd);
    }
    for cmd in [
        format!("ruby -e \"f=File.open('{r}/factory/policy.toml'); File.delete(f)\""),
        format!("ruby -e \"require 'fileutils'; f=File.new('{r}/factory/records/R-1.toml'); FileUtils.rm_f(f)\""),
        format!("ruby -e \"File.unlink(File.open('{r}/factory/policy.toml'))\""),
        format!("ruby -e \"require 'fileutils'; FileUtils.rm_rf(File.open('{r}/factory/records/R-1.toml').path)\""),
        "ruby -e \"File.delete(File.open('factory/policy.toml'))\"".to_string(),
    ] {
        refused(&fx, &fx.root, &cmd);
    }
    for cmd in [
        format!("ruby -e \"h={{'delete'=>1}}; puts h['delete']; puts File.read('{r}/factory/policy.toml')\""),
        format!("ruby -e \"a=[1,2]; a.delete(1); puts File.open('{r}/factory/policy.toml').read\""),
        format!("ruby -e \"require 'fileutils'; puts File.open('{r}/factory/records/R-1.toml').read.size\""),
    ] {
        passes(&fx, &fx.root, &cmd);
    }
}

/// F-106: the path is a relative literal held in a variable, and the delete names only the variable.
#[test]
fn a_delete_through_a_variable_that_holds_a_relative_path_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    for cmd in [
        "ruby -e \"f=File.open('factory/policy.toml'); File.delete(f)\"",
        "ruby -e \"require 'fileutils'; f=File.open('factory/records/R-1.toml'); FileUtils.rm_f(f)\"",
        // F-108: the same family in python and node.
        "python3 -c \"import os; f=open('factory/policy.toml'); os.remove(f.name)\"",
        "node -e \"const f='factory/policy.toml'; fs.rmSync(f)\"",
    ] {
        refused(&fx, &fx.root, cmd);
    }
    // Reading the same file, and a delete that names nothing protected, pass.
    passes(
        &fx,
        &fx.root,
        "ruby -e \"puts File.open('factory/policy.toml').read\"",
    );
    passes(&fx, &fx.root, "ruby -e \"f='/tmp/x.txt'; File.delete(f)\"");
}

#[test]
fn a_delete_through_a_variable_is_held_to_the_file_scope_in_a_working_copy() {
    let fx = Fx::new();
    records(&fx, "\"src/a.rs\"");
    let wc = working_copy(&fx);
    std::fs::create_dir_all(wc.join("src")).unwrap();
    refused(
        &fx,
        &wc,
        "ruby -e \"f=File.open('factory/policy.toml'); File.delete(f)\"",
    );
    refused(
        &fx,
        &wc,
        "ruby -e \"f=File.open('factory/records/x.toml'); File.delete(f)\"",
    );
    passes(
        &fx,
        &wc,
        "ruby -e \"f=File.open('src/a.rs'); File.delete(f)\"",
    );
}

/// F-111, F-113: node deletes by `unlinkSync` and the promise forms, and writes through a variable.
#[test]
fn node_unlink_and_writes_through_a_variable_are_refused() {
    let fx = Fx::new();
    fx.tiny();
    for cmd in [
        "node -e \"const fs=require('fs'); let p='factory/policy.toml'; fs.unlinkSync(p)\"",
        "node -e \"const p='factory/policy.toml'; require('fs').rmdirSync(p)\"",
        "node -e \"const p='factory/policy.toml'; require('fs').promises.unlink(p)\"",
        "node -e \"const p='factory/policy.toml'; require('fs').promises.rm(p)\"",
        "node -e \"const p='factory/policy.toml'; fs.writeFileSync(p,'x')\"",
        "python3 -c \"p='factory/policy.toml'; open(p,'w').write('x')\"",
        "ruby -e \"p='factory/policy.toml'; File.write(p,'x')\"",
    ] {
        refused(&fx, &fx.root, cmd);
    }
}

#[test]
fn node_unlink_through_a_variable_is_held_to_the_file_scope() {
    let fx = Fx::new();
    records(&fx, "\"src/a.rs\"");
    let wc = working_copy(&fx);
    std::fs::create_dir_all(wc.join("src")).unwrap();
    refused(
        &fx,
        &wc,
        "node -e \"const p='factory/policy.toml'; require('fs').unlinkSync(p)\"",
    );
    passes(
        &fx,
        &wc,
        "node -e \"const p='src/a.rs'; require('fs').unlinkSync(p)\"",
    );
}

/// F-112: a printed word next to a delete is not a path.
#[test]
fn a_printed_word_next_to_a_delete_is_not_a_write() {
    let fx = Fx::new();
    records(&fx, "\"src/a.rs\"");
    let wc = working_copy(&fx);
    std::fs::create_dir_all(wc.join("src")).unwrap();
    passes(
        &fx,
        &wc,
        "python3 -c \"import os; print('done'); os.remove('src/a.rs')\"",
    );
    passes(
        &fx,
        &wc,
        "node -e \"console.log('ok'); require('fs').rmSync('src/a.rs')\"",
    );
    let r = fx.root.display().to_string();
    passes(
        &fx,
        &fx.root,
        &format!("python3 -c \"import os; os.remove('/tmp/x.txt'); print(open('{r}/factory/policy.toml').read())\""),
    );
}

/// F-119: copies and opens for writing whose destination is a variable.
#[test]
fn copies_and_opens_through_a_variable_are_refused() {
    let fx = Fx::new();
    fx.tiny();
    for cmd in [
        "node -e \"const fs=require('fs'); const p='factory/policy.toml'; fs.copyFileSync('/tmp/x', p)\"",
        "node -e \"const fs=require('fs'); const p='factory/policy.toml'; fs.openSync(p,'w')\"",
        "node -e \"const fs=require('fs'); const p='factory/policy.toml'; fs.cpSync('/tmp/x', p)\"",
        "python3 -c \"import shutil; p='factory/policy.toml'; shutil.copy('/tmp/x', p)\"",
        "python3 -c \"import shutil; p='factory/policy.toml'; shutil.copy2('/tmp/x', p)\"",
        "python3 -c \"import shutil; p='factory/policy.toml'; shutil.move('/tmp/x', p)\"",
        "ruby -e \"require 'fileutils'; p='factory/policy.toml'; FileUtils.cp('/tmp/x', p)\"",
        "ruby -e \"require 'fileutils'; p='factory/policy.toml'; FileUtils.mv('/tmp/x', p)\"",
    ] {
        refused(&fx, &fx.root, cmd);
    }
    // Neighbours: the variable is the source, or the open is a read.
    passes(
        &fx,
        &fx.root,
        "python3 -c \"import shutil; p='factory/policy.toml'; shutil.copy(p, '/tmp/x')\"",
    );
    passes(
        &fx,
        &fx.root,
        "node -e \"const fs=require('fs'); const p='/tmp/x'; fs.openSync(p,'r')\"",
    );
}

/// F-120: a word with a period and a joined path are not a write of something else.
#[test]
fn a_printed_word_with_a_period_and_a_joined_path_pass_in_a_working_copy() {
    let fx = Fx::new();
    records(&fx, "\"src/a.rs\"");
    let wc = working_copy(&fx);
    std::fs::create_dir_all(wc.join("src")).unwrap();
    passes(
        &fx,
        &wc,
        "python3 -c \"import os; p='src/a.rs'; print('ok.'); os.remove(p)\"",
    );
    passes(
        &fx,
        &wc,
        "python3 -c \"import os; p=os.path.join('src','a.rs'); os.remove(p)\"",
    );
    refused(
        &fx,
        &wc,
        "python3 -c \"import os; p=os.path.join('factory','policy.toml'); os.remove(p)\"",
    );
}

/// F-107: the other call forms on the owners.
#[test]
fn other_ruby_call_forms_of_a_file_delete_are_refused() {
    let fx = Fx::new();
    fx.tiny();
    let r = fx.root.display().to_string();
    for cmd in [
        format!("ruby -e \"f=File.open('{r}/factory/policy.toml'); File::delete(f)\""),
        format!("ruby -e \"require 'fileutils'; f=File.open('{r}/factory/policy.toml'); FileUtils::rm(f)\""),
        format!("ruby -e \"f=File.open('{r}/factory/policy.toml'); File.send(:delete, f)\""),
        format!("ruby -e \"f=File.open('{r}/factory/policy.toml'); File.public_send('unlink', f)\""),
        format!("ruby -e \"require 'fileutils'; include FileUtils; f=File.open('{r}/factory/policy.toml'); rm(f)\""),
        format!("ruby -e \"f=File.open('{r}/factory/policy.toml'); puts \\\"#{{File.delete(f)}}\\\"\""),
    ] {
        refused(&fx, &fx.root, &cmd);
    }
    // A bare `rm` with no include, and `delete` on other owners, pass.
    for cmd in [
        format!("ruby -e \"def rm(x); end; rm(1); puts File.read('{r}/factory/policy.toml')\""),
        format!("ruby -e \"s='a'.delete('a'); m={{}}; m.delete(1); puts File.read('{r}/factory/policy.toml')\""),
    ] {
        passes(&fx, &fx.root, &cmd);
    }
}

/// F-109: the words of a call inside a string are data.
#[test]
fn a_delete_call_named_in_a_ruby_string_is_not_a_call() {
    let fx = Fx::new();
    fx.tiny();
    let r = fx.root.display().to_string();
    passes(
        &fx,
        &fx.root,
        &format!("ruby -e \"puts 'use File.delete(f) to remove'; puts File.read('{r}/factory/policy.toml')\""),
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
