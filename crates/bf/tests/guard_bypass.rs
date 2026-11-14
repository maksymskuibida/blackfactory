//! The second review of the first checkpoint found ways around the guard's shell rules: the human turn
//! command behind a wrapper the parse did not know, write targets that cannot be resolved by reading
//! them, and writers the parsed rule had stopped knowing. Each case proven there is a test here, and the
//! calls that must keep passing sit beside them.

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

// ------------------------------------------------ the human turn behind a wrapper

#[test]
fn the_human_turn_command_is_refused_behind_any_wrapper() {
    let (fx, wc) = setup();
    for cmd in [
        "timeout 5 bf human-turn",
        "gtimeout 5 bf human-turn < p.json",
        "stdbuf -o0 bf human-turn",
        "stdbuf -oL -eL bf human-turn",
        "flock /tmp/lock bf human-turn",
        "flock -n /tmp/lock bf human-turn",
        "script -q /dev/null bf human-turn",
        "setsid bf human-turn",
        "ionice -c3 bf human-turn",
        "taskset 1 bf human-turn",
        "find . -maxdepth 0 -exec bf human-turn \\;",
        "find . -maxdepth 0 -execdir bf human-turn {} +",
        "some-unknown-wrapper --flag bf human-turn",
        "some-unknown-wrapper bf -C /x human-turn",
        "B=bf; some-unknown-wrapper $B human-turn",
        "echo 'bf human-turn' | sh",
        "printf 'ls\\nbf human-turn\\n' | bash",
        "cat <<'EOF' | sh\nbf human-turn\nEOF",
        "sh <<< 'bf human-turn'",
        "echo x | xargs -n1 bf human-turn",
    ] {
        refused(&fx, &fx.root, cmd);
        refused(&fx, &wc, cmd);
    }
}

#[test]
fn the_words_bf_and_human_turn_as_data_still_pass() {
    let (fx, wc) = setup();
    for cmd in [
        "echo bf human-turn is a command of the engine",
        "grep -rn bf human-turn docs",
        "cat docs/engine.md | grep -c human-turn",
        "echo 'ls' | sh",
        "timeout 5 ls",
        "stdbuf -o0 cat notes.md",
        "find . -maxdepth 1 -exec echo {} \\;",
    ] {
        passes(&fx, &fx.root, cmd);
    }
    // From a working copy, the wrapper does not hide a call that is allowed there.
    passes(&fx, &wc, "timeout 60 bf status");
    passes(&fx, &wc, "stdbuf -o0 bf evidence run X-1 --unit U-1 -- ls");
}

#[test]
fn a_command_that_a_find_executes_is_judged() {
    let (fx, wc) = setup();
    refused(
        &fx,
        &fx.root,
        "find . -maxdepth 0 -exec rm factory/policy.toml \\;",
    );
    refused(
        &fx,
        &fx.root,
        "find . -maxdepth 0 -exec sh -c 'echo x > factory/policy.toml' \\;",
    );
    refused(&fx, &wc, "find . -maxdepth 0 -exec bf next \\;");
    passes(&fx, &fx.root, "find src -name '*.tmp' -exec rm {} \\;");
}

#[test]
fn code_piped_into_a_shell_is_read_as_a_command_line() {
    let (fx, wc) = setup();
    refused(&fx, &fx.root, "echo 'echo x > factory/policy.toml' | sh");
    refused(
        &fx,
        &fx.root,
        "printf 'cd factory\\necho x > policy.toml\\n' | bash",
    );
    refused(&fx, &wc, "echo 'bf next' | sh");
    passes(&fx, &fx.root, "echo 'echo x > notes.md' | sh");
}

// ------------------------------------------------ targets that cannot be read as they stand

#[test]
fn a_write_target_that_could_land_in_a_protected_place_is_refused() {
    let (fx, _wc) = setup();
    for cmd in [
        // a glob
        "echo x > fact*/policy.toml",
        "echo x > ./f?ctory/policy.toml",
        "echo x > [f]actory/policy.toml",
        "rm -r .b*/",
        "rm -rf fact*",
        "tee fact*/policy.toml",
        "cp a.txt .bf/g*",
        // a brace
        "echo x > {factory,tmp}/policy.toml",
        "echo x > {tmp,factory}/policy.toml",
        "touch {a,b}/x factory/{policy,gates}.toml",
        "rm -r {.bf,x}",
        // a command substitution
        "echo x > $(echo factory)/policy.toml",
        "echo x > ./$(echo factory)/policy.toml",
        "echo x > `echo factory`/policy.toml",
        "echo x > $(pwd)/factory/policy.toml",
        // a variable assigned in the same line
        "D=factory; echo x > $D/policy.toml",
        "D=factory && rm -r $D",
        "export D=.bf; rm -r ${D}/records",
        "D=fact; echo x > ${D}ory/policy.toml",
        "X=factory/policy.toml; echo x > $X",
        "D=factory; cd $D && echo x > policy.toml",
    ] {
        refused(&fx, &fx.root, cmd);
    }
}

#[test]
fn a_write_target_with_a_wildcard_that_cannot_reach_a_protected_place_passes() {
    let (fx, wc) = setup();
    for cmd in [
        "echo x > /tmp/out-*.txt",
        "echo x > $(mktemp)",
        "echo x > out/$(date +%s)/a.txt",
        "echo x > {a,b}/c.txt",
        "rm -f target/*.o",
        "rm -f /tmp/build-*",
        "touch notes-{1,2}.md",
        "D=out; echo x > $D/a.txt",
        "echo x > $OUT/a.txt",
        "ls fact*/",
        "cat factory/*.toml > /tmp/all.txt",
    ] {
        passes(&fx, &fx.root, cmd);
    }
    passes(&fx, &wc, "rm -f /tmp/build-*");
    passes(&fx, &wc, "echo x > src/hello.txt");
    // In a working copy, a path before its first wildcard that is outside the copy is refused.
    refused(&fx, &wc, "echo x > ../../../notes-*.md");
}

// ------------------------------------------------ the writers the parsed rule must know

#[test]
fn the_common_writers_are_known_again() {
    let (fx, _wc) = setup();
    for cmd in [
        // sorting into a file, an output file given to a filter
        "sort -o factory/policy.toml in.txt",
        "sort -ofactory/policy.toml in.txt",
        "sort in.txt --output=factory/policy.toml",
        "sort --output factory/policy.toml in.txt",
        "uniq in.txt factory/policy.toml",
        "xxd in.bin factory/policy.toml",
        "xxd -r -c 16 in.hex factory/policy.toml",
        "base64 -o factory/policy.toml in.txt",
        "openssl enc -base64 -in a -out factory/policy.toml",
        "gpg --output factory/policy.toml --decrypt a.gpg",
        "zip factory/policy.zip a.txt",
        "ditto a.txt factory/policy.toml",
        "curl -o factory/policy.toml http://x",
        // the compressors
        "gzip factory/policy.toml",
        "gzip -k factory/policy.toml",
        "gunzip factory/policy.toml.gz",
        "bzip2 factory/policy.toml",
        "xz factory/policy.toml",
        "zstd factory/policy.toml",
        // split
        "split in.txt factory/part-",
        "cd factory && split in.txt",
        "split -l 10 in.txt .bf/part-",
        // the line editors
        "ed factory/policy.toml",
        "ex -s factory/policy.toml",
        "vi -es factory/policy.toml",
        "vim +q factory/policy.toml",
        "nano factory/policy.toml",
        // unpacking an archive relative to the working directory
        "cd factory && tar xf ../a.tar",
        "cd factory && tar -xzf ../a.tgz",
        "cd .bf && tar --extract --file ../a.tar",
        "tar xf a.tar factory/policy.toml",
        "tar -xf a.tar -C factory",
        "tar xf a.tar --directory=factory",
        "cd factory && unzip ../a.zip",
        "unzip a.zip -d factory",
        "unzip a.zip factory/policy.toml",
        "cd .bf && curl -O http://x/a",
        "cd factory && wget http://x/a",
        "mktemp factory/tmp.XXXX",
        "mkfifo factory/pipe",
    ] {
        refused(&fx, &fx.root, cmd);
    }
}

#[test]
fn the_same_writers_pass_where_they_write_elsewhere_or_only_read() {
    let (fx, wc) = setup();
    for cmd in [
        "sort factory/policy.toml -o /tmp/sorted.txt",
        "sort factory/policy.toml > /tmp/sorted.txt",
        "uniq factory/policy.toml /tmp/u.txt",
        "gzip -c factory/policy.toml > /tmp/p.gz",
        "gzip --stdout factory/policy.toml | wc -c",
        "gzip -dc factory/policy.toml.gz",
        "gzip -t factory/policy.toml.gz",
        "xz -l factory/policy.toml.xz",
        "tar tf a.tar",
        "tar cf /tmp/a.tar -C factory .",
        "tar -czf /tmp/a.tgz factory",
        "tar xf a.tar -C /tmp/out",
        "tar xf a.tar",
        "unzip -l factory/a.zip",
        "unzip a.zip -d /tmp/out",
        "split in.txt out-",
        "split -l 5 in.txt",
        "vi notes.md",
        "ed notes.md",
        "xxd factory/policy.toml",
        "zip /tmp/a.zip factory/policy.toml",
        "ditto factory /tmp/f",
    ] {
        passes(&fx, &fx.root, cmd);
    }
    // In a unit's working copy, unpacking into the copy's own folders is the unit's business.
    passes(&fx, &wc, "tar xf /tmp/a.tar");
    passes(&fx, &wc, "sort -o src/hello.txt src/hello.txt");
}

#[test]
fn a_writer_in_a_unit_is_held_to_the_units_file_scope() {
    let (fx, wc) = setup();
    for cmd in [
        "sort -o src/other.txt in.txt",
        "gzip src/other.txt",
        "split in.txt src/part-",
        "ed README.md",
    ] {
        let why = refused(&fx, &wc, cmd);
        assert!(why.contains("file scope"), "{cmd}: {why}");
    }
}

// ------------------------------------------------ findings F-32 to F-35

#[test]
fn a_variable_assigned_again_later_does_not_hide_a_protected_target() {
    let (fx, _wc) = setup();
    for cmd in [
        "D=factory; echo x > $D/policy.toml; D=tmp",
        "D=factory; D=tmp; echo x > $D/policy.toml; D=factory2",
        "D=tmp; D=factory; rm -r $D; D=other",
    ] {
        refused(&fx, &fx.root, cmd);
    }
}

#[test]
fn a_wildcard_target_in_a_unit_is_held_to_the_file_scope() {
    let (fx, wc) = setup();
    for cmd in ["rm -f src/*.rs", "rm -rf src/*"] {
        let why = refused(&fx, &wc, cmd);
        assert!(why.contains("file scope"), "{cmd}: {why}");
    }
}

#[test]
fn a_compressor_option_value_is_not_a_file() {
    let (fx, wc) = setup();
    passes(&fx, &wc, "xz -T 4 src/hello.txt");
    passes(&fx, &wc, "zstd -b 9 src/hello.txt");
    let why = refused(&fx, &wc, "xz -T 4 src/other.txt");
    assert!(why.contains("file scope"), "{why}");
}

// ------------------------------------------------ the third review of the first checkpoint

#[test]
fn an_in_place_edit_with_an_empty_suffix_word_is_read_with_its_files() {
    let (fx, wc) = setup();
    passes(&fx, &wc, "sed -i '' 's/hello/bye/' src/hello.txt");
    passes(&fx, &wc, "sed -i '' -e 's/hello/bye/' src/hello.txt");
    passes(&fx, &wc, "sed -i.bak 's/hello/bye/' src/hello.txt");
    let why = refused(&fx, &wc, "sed -i '' 's/a/b/' src/other.txt");
    assert!(why.contains("file scope"), "{why}");
    refused(&fx, &fx.root, "sed -i '' 's/a/b/' factory/policy.toml");
    refused(&fx, &wc, "sed -i '' 's/a/b/' ../../../factory/policy.toml");
}

#[test]
fn a_loop_variable_a_read_variable_and_a_default_value_are_read_with_their_values() {
    let (fx, _wc) = setup();
    for cmd in [
        "for d in factory; do rm -rf $d/records; done",
        "for d in tmp factory; do echo x > $d/policy.toml; done",
        "for d in factory\ndo\n  rm -r \"$d\"\ndone",
        "echo factory | while read d; do rm -r $d; done",
        "printf 'tmp\\nfactory\\n' | while read -r d; do rm -r $d; done",
        "read d <<< factory; rm -r $d",
        "read -r a b <<< \"x factory\"; rm -r $b",
        "rm -r ${D:-factory}/records",
        "echo x > ${D:-.bf}/guard.json",
        "echo x > ${D-factory}/policy.toml",
        "cd ${D:-factory} && echo x > policy.toml",
        ": ${D:=factory}; echo x > $D/policy.toml",
        ": ${D=.bf}; rm $D/guard.json",
        // A read variable from input the guard cannot see, then a delete.
        "find factory -type f | while read f; do rm \"$f\"; done",
        "cat list | while read f; do rm $f; done",
        "find factory -name policy.toml | while read f; do cp /tmp/evil \"$f\"; done",
        "cat list | while read f; do gzip $f; done",
    ] {
        refused(&fx, &fx.root, cmd);
    }
    for cmd in [
        "for d in out build; do rm -rf $d/x; done",
        "echo out | while read d; do rm -r $d; done",
        "read d <<< out; rm -r $d",
        "rm -r ${D:-out}/records",
        "for f in factory; do cat $f/policy.toml; done",
    ] {
        passes(&fx, &fx.root, cmd);
    }
}

#[test]
fn a_copy_onto_a_read_variable_is_refused_as_a_write_not_a_delete() {
    let (fx, _wc) = setup();
    let why = refused(
        &fx,
        &fx.root,
        "find factory -name policy.toml | while read f; do cp /tmp/evil \"$f\"; done",
    );
    assert!(why.contains("would write the files"), "{why}");
    assert!(!why.contains("delete or edit in place"), "{why}");
}

#[test]
fn a_writer_started_through_xargs_is_judged_by_its_input() {
    let (fx, wc) = setup();
    for cmd in [
        "echo factory/policy.toml | xargs rm",
        "printf 'factory/policy.toml\\n' | xargs -n1 rm -f",
        "echo factory | xargs -I{} rm -r {}/records",
        "echo factory/policy.toml | xargs touch",
        "echo factory/policy.toml | xargs -t chmod 600",
        "cat <<EOF | xargs rm\nfactory/policy.toml\nEOF",
        // The input is unknown and the writer deletes or edits in place.
        "find . -name '*.tmp' | xargs rm",
        "ls | xargs rm -f",
        "git ls-files | xargs sed -i s/a/b/",
        "xargs rm < list.txt",
        "find factory -name '*.toml' | xargs truncate -s 0",
        "find factory -name '*.toml' | xargs mv -t /tmp",
        "echo out | xargs -a list rm",
        "echo out | xargs --arg-file=list rm",
        // The input becomes the write target.
        "find factory -name policy.toml | xargs cp /tmp/evil",
        "find factory -name policy.toml | xargs -I{} cp /tmp/evil {}",
        "find factory -name '*.toml' | xargs gzip",
        "find factory -name '*.toml' | xargs ln -sf /tmp/evil",
    ] {
        refused(&fx, &fx.root, cmd);
    }
    for cmd in [
        "ls | xargs echo",
        "find . -name '*.rs' | xargs grep -n foo",
        "echo out/a.o | xargs rm",
        "echo a b | xargs -n1 echo",
        "ls | xargs cp -t out",
        "ls | xargs gzip -c",
    ] {
        passes(&fx, &fx.root, cmd);
    }
    passes(&fx, &wc, "echo src/hello.txt | xargs touch");
    let why = refused(&fx, &wc, "echo src/other.txt | xargs touch");
    assert!(why.contains("file scope"), "{why}");
    let why = refused(&fx, &wc, "find . -name '*.tmp' | xargs rm");
    assert!(why.contains("xargs"), "{why}");
    assert!(why.contains("Name the files"), "{why}");
}

#[test]
fn code_for_an_interpreter_that_names_a_protected_file_from_the_main_checkout_is_refused() {
    let (fx, wc) = setup();
    for cmd in [
        "python3 -c \"open('factory/policy.toml','w').write('x')\"",
        "python3 -c \"open('./factory/policy.toml','w')\"",
        "python3 - <<'EOF'\nopen('factory/policy.toml', 'w').write('x')\nEOF",
        "node -e \"require('fs').writeFileSync('.bf/guard.json', 'x')\"",
        "perl -e 'open(F, \">.claude/settings.json\")'",
        "echo \"open('factory/policy.toml','w')\" | python3",
    ] {
        refused(&fx, &fx.root, cmd);
    }
    // The bare words, and source text in a working copy, are data.
    passes(
        &fx,
        &fx.root,
        "python3 -c \"print('factory .bf policy.toml')\"",
    );
    passes(&fx, &fx.root, "python3 -c \"print('src/main.py')\"");
    passes(
        &fx,
        &wc,
        "python3 - <<'EOF'\nprint('factory/policy.toml')\nEOF",
    );
}

#[test]
fn inline_code_is_judged_like_code_on_standard_input_for_the_human_turn_command() {
    let (fx, wc) = setup();
    for cmd in [
        "python3 -c \"import os; os.system('bf human-turn')\"",
        "python3 -c \"import subprocess; subprocess.run(['bf', 'human-turn'])\"",
        "python3 - <<'EOF'\nimport os\nos.system('bf human-turn')\nEOF",
        "echo \"import os; os.system('bf human-turn')\" | python3",
        "awk 'BEGIN { system(\"bf human-turn\") }'",
        "perl -e 'system(\"bf\", \"human-turn\")'",
        "node -e \"require('child_process').execSync('bf human-turn')\"",
        // Programs started without a call word.
        "perl -e 'open(F, \"bf human-turn |\")'",
        "perl -e 'open(F, \"-|\", \"bf\", \"human-turn\")'",
        "ruby -e 'open(\"|bf human-turn\")'",
        "php -r 'proc_open(\"bf human-turn\", [], $p);'",
        "awk 'BEGIN { \"bf human-turn\" | getline x }'",
        "awk 'BEGIN { print \"x\" | \"bf human-turn\" }'",
    ] {
        refused(&fx, &fx.root, cmd);
        refused(&fx, &wc, cmd);
    }
    // The phrase alone is not a call.
    for cmd in [
        "python3 -c \"print('bf human-turn')\"",
        "python3 -c \"print('docs: run bf human-turn from the hook')\"",
        "python3 - <<'EOF'\nprint('bf human-turn')\nEOF",
        "awk 'BEGIN { print \"bf human-turn\" }'",
    ] {
        passes(&fx, &fx.root, cmd);
    }
}
