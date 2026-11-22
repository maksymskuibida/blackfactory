//! R-52, R-53, R-54: the guard's shell rules judge where a command writes and what it runs, resolved to
//! a path, and never the words of its text.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn decision(fx: &Fx, cwd: &Path, cmd: &str) -> (String, String) {
    let payload =
        json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
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

fn setup() -> (Fx, PathBuf) {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    (fx, wc)
}

fn passes(fx: &Fx, cwd: &Path, cmd: &str) {
    let (d, why) = decision(fx, cwd, cmd);
    assert_ne!(d, "deny", "should pass: {cmd}\n{why}");
}

fn refused(fx: &Fx, cwd: &Path, cmd: &str) -> String {
    let (d, why) = decision(fx, cwd, cmd);
    assert_eq!(d, "deny", "should be refused: {cmd}");
    why
}

// ------------------------------------------------ the false refusals seen in this run

#[test]
fn an_edit_script_on_standard_input_may_hold_source_text_naming_the_factory_folders() {
    let (fx, wc) = setup();
    let cmd = "python3 - <<'EOF'\n\
        s = open('src/hello.txt').read()\n\
        s = s.replace('\"factory/\"', '\"factory/\", \".bf/\"')\n\
        # protected: factory/, .bf/, .claude/settings.json, factory/policy.toml\n\
        open('src/hello.txt', 'w').write(s)\n\
        EOF";
    passes(&fx, &wc, cmd);
    passes(&fx, &fx.root, cmd);
    // The same text unquoted and with an expanding delimiter is still data to python.
    passes(
        &fx,
        &wc,
        "python3 - <<EOF\nprint('factory/records .bf/guard.json')\nEOF",
    );
}

#[test]
fn a_file_written_by_heredoc_into_the_units_own_files_passes() {
    let (fx, wc) = setup();
    passes(
        &fx,
        &wc,
        "cat > src/hello.txt <<'EOF'\nthe factory/ folder and .bf/ and factory/policy.toml are protected\nEOF",
    );
    passes(
        &fx,
        &wc,
        "cat >> src/hello.txt <<EOF\nfactory/records/R-1.toml\nEOF",
    );
    // A test file that quotes the paths it checks.
    passes(
        &fx,
        &wc,
        "tee src/hello.txt <<'EOF'\nassert!(!bash(\"echo x > factory/records/R-1.toml\"));\nEOF",
    );
}

#[test]
fn a_shell_write_in_the_units_copy_outside_its_list_passes_and_a_protected_one_is_refused() {
    let (fx, wc) = setup();
    for cmd in [
        "cat > src/other.txt <<'EOF'\nx\nEOF",
        "echo x > src/a.txt",
        "sed -i 's/a/b/' src/a.txt",
        "cd src && echo x > b.txt",
        "touch README.md",
    ] {
        passes(&fx, &wc, cmd);
    }
    for cmd in [
        "echo x > factory/records/R-1.toml",
        "touch factory/policy.toml",
    ] {
        refused(&fx, &wc, cmd);
    }
}

#[test]
fn an_in_place_edit_whose_pattern_holds_the_word_factory_passes() {
    let (fx, wc) = setup();
    passes(&fx, &wc, "sed -i 's/factory/plant/g' src/hello.txt");
    passes(
        &fx,
        &wc,
        "sed -i 's|factory/records|records|' src/hello.txt",
    );
    passes(
        &fx,
        &wc,
        "sed -i -e 's/.bf/.xx/' -e '/factory/d' src/hello.txt",
    );
    passes(&fx, &wc, "sed -i.bak 's/factory/x/' src/hello.txt");
    passes(&fx, &wc, "perl -pi -e 's/factory/plant/' src/hello.txt");
    passes(&fx, &fx.root, "sed -i 's/factory/x/' notes.md");
}

#[test]
fn a_reviewer_may_file_findings_on_the_engines_standard_input() {
    let (fx, wc) = setup();
    let cmd = "bf record put finding --file - <<'EOF'\n\
        id=\"F-1\"\nkind=\"finding\"\n\
        title=\"writes factory/policy.toml and .bf/guard.json, reads BF_STATE_HOME\"\n\
        EOF";
    passes(&fx, &wc, cmd);
    passes(&fx, &fx.root, cmd);
    let unquoted =
        "bf record put finding --file - <<EOF\ntext: factory/ .bf/ .claude/settings.json\nEOF";
    passes(&fx, &wc, unquoted);
}

#[test]
fn quoted_strings_and_data_arguments_are_not_scanned() {
    let (fx, wc) = setup();
    for cmd in [
        "echo \"do not touch factory/policy.toml\"",
        "grep -rn \"factory/\\|.bf/\" src",
        "git commit -m \"explain why factory/ and .bf/ are protected\"",
        "python3 -c \"print('factory .bf policy.toml')\"",
        "echo 'BF_STATE_HOME is where the state lives'",
        "cat factory/policy.toml",
        "ls .bf",
        "git status && git diff -- factory",
    ] {
        passes(&fx, &wc, cmd);
        passes(&fx, &fx.root, cmd);
    }
}

// ------------------------------------------------ what must stay refused

#[test]
fn a_redirect_into_the_records_folder_is_refused() {
    let (fx, wc) = setup();
    refused(&fx, &fx.root, "echo x > factory/records/R-1.toml");
    refused(&fx, &fx.root, "echo x >> ./factory/records/R-1.toml");
    refused(&fx, &fx.root, "echo x >| factory/records/R-1.toml");
    refused(&fx, &fx.root, "echo x &> factory/records/R-1.toml");
    refused(
        &fx,
        &fx.root,
        "cat <<EOF > factory/records/R-1.toml\nx\nEOF",
    );
    refused(&fx, &wc, "echo x > ../../../factory/records/R-1.toml");
    refused(
        &fx,
        &wc,
        &format!("echo x > {}/factory/records/R-1.toml", fx.root.display()),
    );
    refused(&fx, &fx.root, "sh -c 'echo x > factory/records/R-1.toml'");
}

#[test]
fn a_script_that_opens_the_policy_by_its_absolute_path_is_refused() {
    let (fx, wc) = setup();
    let policy = fx.root.join("factory/policy.toml");
    let p = policy.display();
    refused(
        &fx,
        &wc,
        &format!("python3 - <<'EOF'\nopen('{p}', 'w').write('x')\nEOF"),
    );
    refused(
        &fx,
        &fx.root,
        &format!("python3 -c \"open('{p}','w').write('x')\""),
    );
    refused(
        &fx,
        &fx.root,
        &format!("node -e \"require('fs').writeFileSync('{p}', 'x')\""),
    );
    // Climbing out of a working copy by relative path is a path too.
    refused(
        &fx,
        &wc,
        "python3 -c \"open('../../../factory/policy.toml','w')\"",
    );
}

#[test]
fn an_in_place_edit_of_the_policy_by_a_relative_path_from_a_working_copy_is_refused() {
    let (fx, wc) = setup();
    refused(&fx, &wc, "sed -i 's/a/b/' ../../../factory/policy.toml");
    refused(&fx, &wc, "sed -i -e 's/a/b/' ../../../factory/policy.toml");
    refused(
        &fx,
        &wc,
        "perl -pi -e 's/a/b/' ../../../factory/policy.toml",
    );
    refused(&fx, &fx.root, "sed -i s/a/b/ factory/policy.toml");
}

#[test]
fn removing_the_guard_table_and_writing_the_hook_settings_are_refused() {
    let (fx, wc) = setup();
    refused(&fx, &fx.root, "rm .bf/guard.json");
    refused(&fx, &fx.root, "rm -rf .bf");
    refused(&fx, &fx.root, "rm -rf ./factory");
    refused(&fx, &wc, "rm ../../../.bf/guard.json");
    refused(&fx, &fx.root, "mv .bf/guard.json /tmp/g.json");
    refused(&fx, &fx.root, "echo '{}' > .claude/settings.json");
    refused(&fx, &fx.root, "cp /tmp/s.json .claude/settings.json");
    refused(&fx, &fx.root, "tee .claude/settings.json < /tmp/s.json");
    refused(&fx, &fx.root, "find factory -name '*.toml' -delete");
    refused(&fx, &fx.root, "git checkout -- factory");
}

#[test]
fn a_folder_change_decides_where_a_relative_path_lands() {
    let (fx, _wc) = setup();
    refused(&fx, &fx.root, "cd factory && echo x > policy.toml");
    refused(&fx, &fx.root, "cd factory; rm -rf records");
    refused(&fx, &fx.root, "sh -c 'cd factory && echo x > policy.toml'");
    // The change holds for the commands after it, in the shell that made it.
    passes(&fx, &fx.root, "cd src && echo x > factory.txt");
    passes(&fx, &fx.root, "sh -c 'cd factory'; echo x > a.txt");
}

#[test]
fn what_it_would_execute_is_judged_by_its_path() {
    let (fx, wc) = setup();
    refused(&fx, &fx.root, "./factory/run.sh");
    refused(&fx, &fx.root, "bash factory/run.sh");
    refused(&fx, &fx.root, "python3 .bf/tool.py");
    refused(&fx, &fx.root, "source factory/env.sh");
    refused(&fx, &wc, "sh ../../../factory/run.sh");
    passes(&fx, &fx.root, "bash scripts/run.sh");
    passes(&fx, &wc, "bash -c 'echo factory'");
}

#[test]
fn the_state_folder_is_still_off_limits_and_its_names_in_a_heredoc_are_not() {
    let (fx, wc) = setup();
    let sd = fx.state_dir();
    refused(&fx, &fx.root, &format!("cat {}/key", sd.display()));
    refused(&fx, &fx.root, &format!("cd {} && cat key", sd.display()));
    refused(&fx, &fx.root, "cat $BF_STATE_HOME/*/key");
    refused(
        &fx,
        &fx.root,
        &format!("python3 -c \"print(open('{}/key').read())\"", sd.display()),
    );
    // The engine's own standard input is not scanned.
    passes(
        &fx,
        &wc,
        &format!(
            "bf record put finding --file - <<'EOF'\n{}/key and BF_STATE_HOME\nEOF",
            sd.display()
        ),
    );
}

#[test]
fn a_path_built_at_run_time_is_refused_only_where_it_could_write() {
    let (fx, _wc) = setup();
    refused(&fx, &fx.root, "cd \"$D\" && echo x > a.txt");
    refused(&fx, &fx.root, "echo x > \"$D\"/factory/policy.toml");
    refused(&fx, &fx.root, "rm -rf $(pwd)/factory");
    // Reading is not stopped.
    passes(&fx, &fx.root, "cd \"$D\" && cat a.txt");
    // A write to a path that starts with a variable the line does not assign could land anywhere, the
    // protected places included, so it is refused; assigning the variable in the line makes it known.
    refused(&fx, &fx.root, "echo x > \"$OUT\"");
    refused(&fx, &fx.root, "cargo test 2>&1 | tee \"$LOG\"");
    passes(
        &fx,
        &fx.root,
        "LOG=/tmp/t.log; cargo test 2>&1 | tee \"$LOG\"",
    );
}

#[test]
fn a_units_writes_still_stay_inside_its_working_copy() {
    let (fx, wc) = setup();
    refused(&fx, &wc, "echo x > ../../src/hello.txt");
    refused(&fx, &wc, "cd src && echo x > ../../../src/hello.txt");
    refused(&fx, &wc, "cp a.txt ../U-2/a.txt");
    passes(&fx, &wc, "cd src && echo x > hello.txt");
    passes(&fx, &wc, "echo x > /tmp/elsewhere.txt");
}

// ------------------------------------------------ R-53: what the refusal says

#[test]
fn a_refusal_names_the_path_and_the_part_of_the_command_that_reached_it() {
    let (fx, wc) = setup();
    let why = refused(&fx, &wc, "echo x > ../../../factory/policy.toml");
    let policy = fx.root.join("factory/policy.toml");
    assert!(
        why.contains(&policy.display().to_string()),
        "the refusal names the resolved path: {why}"
    );
    assert!(
        why.contains("the redirect `> ../../../factory/policy.toml`"),
        "the refusal names the part of the command: {why}"
    );
    assert_four_parts(&why);
    assert!(
        !why.contains("no longer names") && !why.to_lowercase().contains("keep the name"),
        "the refusal no longer tells the agent to keep names out of the command: {why}"
    );

    let why = refused(&fx, &fx.root, "rm -rf factory/records");
    assert!(
        why.contains("`rm` with the argument `factory/records`"),
        "{why}"
    );
    assert!(why.contains("delete"), "{why}");
    assert!(
        why.contains("Remove only files inside your working copy"),
        "the steps say what to do for this case: {why}"
    );

    let why = refused(
        &fx,
        &fx.root,
        &format!("python3 -c \"open('{}','w')\"", policy.display()),
    );
    assert!(why.contains("the code given to `python3`"), "{why}");
    assert!(why.contains(&policy.display().to_string()), "{why}");
    assert!(
        why.contains("Open files from a path inside your working copy"),
        "{why}"
    );

    let why = refused(&fx, &fx.root, "./factory/run.sh");
    assert!(why.contains("the program `./factory/run.sh`"), "{why}");
    assert!(
        why.contains("Run a program or script that lives in your working copy"),
        "{why}"
    );
}

// ------------------------------------------------ the fourth review: Y-1, Y-2, Y-7

#[test]
fn reading_a_record_with_a_text_tool_or_an_interpreter_passes_from_the_main_checkout() {
    let (fx, _wc) = setup();
    for cmd in [
        "awk 'NR<3' factory/records/R-1.toml",
        "awk '/title/' .bf/briefs/U-1-builder.md",
        "perl -ne 'print if /title/' factory/records/R-1.toml",
        "ruby -ne 'puts $_' factory/records/R-1.toml",
        "python3 -c \"import sys; print(open(sys.argv[1]).read())\" factory/records/R-1.toml",
        "python3 -c \"print('see factory/records/R-1.toml')\"",
        "node -e \"console.log(process.argv[1])\" factory/records/R-1.toml",
        "python3 - <<'EOF'\nimport glob\nfor f in glob.glob('factory/records/*.toml'):\n    print(f)\nEOF",
        "python3 - <<'EOF'\nprint('factory/policy.toml and .bf/guard.json are protected')\nEOF",
        "python3 script.py factory/records",
        "node script.js factory/records/R-1.toml",
        "python3 -m json.tool .bf/guard.json",
    ] {
        passes(&fx, &fx.root, cmd);
    }
}

#[test]
fn code_that_writes_a_protected_path_is_still_refused_whatever_the_spelling() {
    let (fx, wc) = setup();
    let policy = fx.root.join("factory/policy.toml");
    refused(
        &fx,
        &fx.root,
        "python3 -c \"open('factory/policy.toml','w').write('x')\"",
    );
    refused(
        &fx,
        &fx.root,
        &format!("python3 -c \"open('{}','w')\"", policy.display()),
    );
    refused(&fx, &fx.root, "sed -i 's/a/b/' factory/policy.toml");
    refused(&fx, &fx.root, "echo x > factory/records/R-1.toml");
    refused(&fx, &fx.root, "rm .claude/settings.json");
    // From a working copy, up into the main checkout.
    let up = format!("sed -i 's/a/b/' {}", policy.display());
    refused(&fx, &wc, &up);
}

#[test]
fn making_the_folder_of_the_units_own_file_passes_and_other_folder_forms_stay_refused() {
    let (fx, wc) = setup();
    for cmd in [
        "mkdir -p src",
        "mkdir src",
        "mkdir -p src && echo hello > src/hello.txt",
        "install -d src",
        "python3 -c \"import os; os.makedirs('src', exist_ok=True); open('src/hello.txt','w').write('x')\"",
        // A folder the list does not name is a folder of the copy too.
        "mkdir -p lib",
        "install -d lib",
        "python3 -c \"import os; os.makedirs('lib')\"",
    ] {
        passes(&fx, &wc, cmd);
    }
    // A folder of the copy that the list names passes too.
    passes(&fx, &wc, "rmdir src");
    passes(&fx, &wc, "rm -rf src");
    for cmd in [
        "rmdir factory",
        "rm -rf factory",
        "mkdir -p factory",
    ] {
        refused(&fx, &wc, cmd);
    }
}

#[test]
fn moves_copies_and_links_in_interpreter_code_are_judged_by_their_destination() {
    let (fx, wc) = setup();
    for cmd in [
        "python3 -c \"import os; os.replace('/tmp/x','factory/policy.toml')\"",
        "python3 -c \"import shutil; shutil.move('/tmp/x','factory/policy.toml')\"",
        "python3 -c \"import shutil; shutil.copy('/tmp/x','factory/policy.toml')\"",
        "python3 -c \"import os; os.symlink('/tmp/x','factory/policy.toml')\"",
        "node -e \"require('fs').cpSync('/tmp/x','factory/policy.toml')\"",
        "ruby -e \"File.write('factory/policy.toml','x')\"",
        "python3 -c \"from pathlib import Path; Path('factory/policy.toml').open('w')\"",
    ] {
        refused(&fx, &fx.root, cmd);
    }
    for cmd in [
        "python3 -c \"s='abc'.replace('a','b'); print(s)\"",
        "python3 -c \"import shutil; shutil.copy('factory/policy.toml','/tmp/x')\"",
    ] {
        passes(&fx, &fx.root, cmd);
    }
    // In a unit's copy, outside its list: the list is a recommendation, so it passes.
    passes(
        &fx,
        &wc,
        "python3 -c \"import os; os.replace('/tmp/x','other.txt')\"",
    );
}

#[test]
fn a_path_object_makes_the_folder_of_the_units_own_file() {
    let (fx, wc) = setup();
    passes(
        &fx,
        &wc,
        "python3 -c \"from pathlib import Path; Path('src').mkdir(parents=True, exist_ok=True)\"",
    );
    passes(
        &fx,
        &wc,
        "python3 -c \"from pathlib import Path; Path('lib').mkdir(parents=True)\"",
    );
    refused(
        &fx,
        &wc,
        "python3 -c \"from pathlib import Path; Path('factory').mkdir(parents=True)\"",
    );
}

#[test]
fn a_recursive_copy_of_a_pattern_into_the_root_is_refused() {
    let (fx, _wc) = setup();
    for cmd in ["cp -R /tmp/x/* .", "cp -a /tmp/x/* ./"] {
        refused(&fx, &fx.root, cmd);
    }
    passes(&fx, &fx.root, "cp -R /tmp/x/* ./docs");
}

#[test]
fn a_target_that_holds_a_protected_place_is_refused_like_a_target_inside_it() {
    let (fx, _wc) = setup();
    for cmd in [
        "mv .claude .claude2",
        "rm -rf .claude",
        "rm -r .claude/",
        "find .claude -delete",
        "chmod 000 .claude",
        "ln -sfn /tmp/x .claude",
        "mv .git .git2",
        "rm -rf .git",
        "rm -rf .",
        "git checkout .",
        "git checkout -- .",
        "git restore .",
        "git -C factory checkout -- .",
        "cp -R /tmp/x/factory .",
        "cp -R /tmp/x/. .",
        "cp -R /tmp/x/.claude .",
        "rsync -a /tmp/x/ ./",
    ] {
        refused(&fx, &fx.root, cmd);
    }
    let why = refused(&fx, &fx.root, "rm -rf .claude");
    assert!(why.contains("rm"), "{why}");
    for cmd in [
        "git checkout main",
        "git restore --staged src/a.txt",
        "rm -rf target",
        "rm -f src/a.o",
        "cp -R /tmp/x/docs ./docs",
        "mv build/out.txt build/out2.txt",
        "ls -la .claude",
    ] {
        passes(&fx, &fx.root, cmd);
    }
}
