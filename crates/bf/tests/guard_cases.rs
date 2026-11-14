//! The third review's wrong guard answers, kept as one table so a fault found once cannot hide again.
//!
//! Each row has the place the command runs (`Root` is the main checkout, `Wc` a unit's working copy,
//! `Cpwc` a checkpoint's working copy), the command, and the answer the guard must give. A row that only a
//! later unit can make pass is marked as waiting for that unit by its ID and is checked for the answer it
//! has today (refused); when the unit lands the row is changed to assert the answer it wants.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    Root,
    Wc,
    Cpwc,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Want {
    Refused,
    Passed,
}

struct Case {
    n: u32,
    place: Place,
    want: Want,
    /// The unit that has to land before this row can pass; the row is then not asserted.
    waits: Option<&'static str>,
    cmd: &'static str,
}

use Place::*;
use Want::*;

const fn c(n: u32, place: Place, want: Want, cmd: &'static str) -> Case {
    Case {
        n,
        place,
        want,
        waits: None,
        cmd,
    }
}

const fn waiting(n: u32, place: Place, want: Want, unit: &'static str, cmd: &'static str) -> Case {
    Case {
        n,
        place,
        want,
        waits: Some(unit),
        cmd,
    }
}

fn table() -> Vec<Case> {
    vec![
        c(1, Root, Refused, "ruby -i -pe 'x' factory/policy.toml"),
        c(2, Root, Refused, "ln -s factory/policy.toml l && echo x > l"),
        c(3, Root, Refused, "ln -s factory l && echo x > l/policy.toml"),
        c(4, Root, Refused, "cd fac* && echo x > policy.toml"),
        c(
            5,
            Root,
            Refused,
            "for D in factory; do echo x > $D/policy.toml; done",
        ),
        c(6, Root, Refused, "echo x > ${D:-factory}/policy.toml"),
        c(
            7,
            Root,
            Refused,
            "read D <<< factory; echo x > $D/policy.toml",
        ),
        c(8, Root, Refused, "echo factory/policy.toml | xargs rm"),
        c(9, Root, Refused, "xargs rm < list.txt"),
        c(10, Root, Refused, "xargs -a list.txt rm"),
        c(
            11,
            Root,
            Refused,
            "python3 -c \"open('factory/policy.toml','w').write('x')\"",
        ),
        c(
            12,
            Root,
            Refused,
            "python3 <<'EOF'\nopen('factory/policy.toml','w').write('x')\nEOF",
        ),
        c(
            13,
            Root,
            Refused,
            "node -e \"require('fs').writeFileSync('factory/policy.toml','x')\"",
        ),
        c(
            14,
            Root,
            Refused,
            "awk 'BEGIN{print 1 > \"factory/policy.toml\"}'",
        ),
        c(15, Root, Refused, "awk 'BEGIN{system(\"bf human-turn\")}'"),
        c(
            16,
            Root,
            Refused,
            "python3 -c 'import subprocess; subprocess.run([\"bf\",\"human-turn\"])'",
        ),
        c(
            17,
            Root,
            Refused,
            "make -f /dev/stdin <<'EOF'\nall:\n\tbf human-turn\nEOF",
        ),
        c(18, Root, Refused, "cargo run -q -- human-turn"),
        c(
            19,
            Wc,
            Passed,
            "sed -i '' 's/factory\\/records/x/' src/hello.txt",
        ),
        c(20, Wc, Passed, "rm -f src/hello.txt.bak"),
        c(
            21,
            Wc,
            Refused,
            "python3 -c \"open('src/other.txt','w').write('x')\"",
        ),
        c(22, Wc, Refused, "tar xf /tmp/a.tar"),
        c(23, Wc, Refused, "unzip /tmp/a.zip"),
        c(24, Wc, Refused, "git merge main"),
        waiting(
            25,
            Cpwc,
            Passed,
            "U-35",
            "bf report K-1 --result built --role security-auditor --token abc",
        ),
        waiting(
            26,
            Cpwc,
            Passed,
            "U-35",
            "bf record put legal --file /tmp/l.toml --token abc",
        ),
        c(
            27,
            Root,
            Refused,
            "for f in factory/records/*.toml; do sed -i '' 's/a/b/' \"$f\"; done",
        ),
        c(
            28,
            Root,
            Refused,
            "for f in factory/records/*.toml; do echo x >> $f; done",
        ),
        c(
            29,
            Root,
            Refused,
            "ls factory/records/*.toml | xargs sed -i '' 's/a/b/'",
        ),
        c(
            30,
            Root,
            Refused,
            "find factory/records -name '*.toml' | xargs sed -i 's/a/b/'",
        ),
        c(
            31,
            Root,
            Refused,
            "find factory/records -name '*.toml' -print0 | xargs -0 perl -pi -e 's/a/b/'",
        ),
        c(32, Root, Refused, "echo x > $UNKNOWN/policy.toml"),
        c(33, Root, Refused, "echo x > $UNKNOWN"),
        c(34, Root, Refused, "echo x > \"$TMPDIR/out.txt\""),
        c(35, Wc, Passed, "sed -i '' 's/a/b/' src/hello.txt"),
        // Position taken: a push stays refused from a working copy. A builder never pushes, no engine
        // command touches the network, and a push can carry a branch the engine owns.
        c(36, Wc, Refused, "git push"),
        c(37, Wc, Passed, "python3 -c \"print('bf human-turn')\""),
        c(
            38,
            Wc,
            Passed,
            "python3 -c \"s = open('src/hello.txt').read(); open('src/hello.txt', 'w').write(s + ' bf human-turn')\"",
        ),
        c(
            39,
            Wc,
            Passed,
            "perl -0pi -e 's/a/bf human-turn/' src/hello.txt",
        ),
    ]
}

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

/// A project whose two units have landed, so checkpoint K-1's working copy exists.
fn checkpoint_place() -> (Fx, PathBuf) {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    let cpwc = fx.work("K-1");
    assert!(cpwc.is_dir(), "the checkpoint's working copy exists");
    (fx, cpwc)
}

/// A project with unit U-1 started: its working copy is where the `Wc` rows run.
fn unit_place() -> (Fx, PathBuf) {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    (fx, wc)
}

#[test]
fn every_case_the_third_review_measured_wrong_answers_as_expected() {
    let (fx, wc) = unit_place();
    let (fx2, cpwc) = checkpoint_place();
    let mut wrong = Vec::new();
    for case in table() {
        let (f, cwd) = match case.place {
            Root => (&fx, fx.root.clone()),
            Wc => (&fx, wc.clone()),
            Cpwc => (&fx2, cpwc.clone()),
        };
        let (d, why) = decision(f, &cwd, case.cmd);
        let got = if d == "deny" { Refused } else { Passed };
        // A waiting row is held to the answer it has today (refused); once its unit lands, the row
        // loses its `waits` and is held to the answer it wants.
        let expect = if case.waits.is_some() {
            Refused
        } else {
            case.want
        };
        if got != expect {
            wrong.push(format!(
                "#{} ({:?}) wanted {:?}, got {:?}: {}\n    {}",
                case.n,
                case.place,
                case.want,
                got,
                case.cmd.replace('\n', "\\n"),
                why.lines().next().unwrap_or("")
            ));
        }
    }
    assert!(wrong.is_empty(), "wrong answers:\n{}", wrong.join("\n"));
}

/// The fixes must not refuse the honest neighbours of each case.
#[test]
fn the_honest_neighbours_of_the_cases_still_pass() {
    let (fx, wc) = unit_place();
    let passes: &[(Place, &str)] = &[
        (Root, "ln -s notes.txt link-to-notes"),
        (Root, "cd out* && echo x > a.txt"),
        (Root, "make -f /dev/stdin <<'EOF'\nall:\n\techo hello\nEOF"),
        (Root, "cargo run -q -- status"),
        (Root, "D=out; echo x > $D/a.txt"),
        (Root, "echo x > $(mktemp)"),
        (
            Wc,
            "python3 -c \"s = open('src/other.txt').read(); print(s)\"",
        ),
        (Wc, "python3 -c \"open('src/hello.txt','w').write('x')\""),
        (Wc, "tar xf /tmp/a.tar src/hello.txt"),
        (Wc, "git revert HEAD"),
        (Wc, "rm -f src/hello.txt~"),
        (
            Wc,
            "python3 -c \"from pathlib import Path; Path('src/hello.txt').write_text('hello')\"",
        ),
        (
            Wc,
            "python3 -c \"from pathlib import Path; Path('src/hello.txt').write_bytes(b'')\"",
        ),
        (
            Wc,
            "python3 -c \"import shutil; shutil.copyfile('src/hello.txt','src/hello.txt')\"",
        ),
        // A copy only reads its source.
        (
            Wc,
            "python3 -c \"import shutil; shutil.copyfile('src/other.txt','/tmp/x')\"",
        ),
        (
            Wc,
            "node -e \"require('fs').copyFileSync('src/other.txt','/tmp/x')\"",
        ),
        // Known gap, stated in docs/engine.md: a path object held in a variable names its path
        // outside the call that writes, so the write is not seen.
        (
            Wc,
            "python3 -c \"from pathlib import Path; p = Path('src/other.txt'); p.write_text('x')\"",
        ),
    ];
    for (place, cmd) in passes {
        let cwd = if *place == Root {
            fx.root.clone()
        } else {
            wc.clone()
        };
        let (d, why) = decision(&fx, &cwd, cmd);
        assert_ne!(d, "deny", "should pass: {cmd}\n{why}");
    }
    let refuses: &[(Place, &str)] = &[
        // A backup is only removed beside a file the unit may touch.
        (Wc, "rm -f src/other.txt.bak"),
        (Wc, "rm -f src/other.txt"),
        (
            Wc,
            "python3 -c \"from pathlib import Path; Path('src/other.txt').write_text('')\"",
        ),
        (
            Wc,
            "python3 -c \"from pathlib import Path; Path('src/other.txt').write_text('hello')\"",
        ),
        (
            Wc,
            "python3 -c \"from pathlib import Path; Path('src/other.txt').touch()\"",
        ),
        (
            Wc,
            "python3 -c \"import shutil; shutil.copyfile('/tmp/x','src/other.txt')\"",
        ),
        (
            Wc,
            "python3 -c \"import shutil; shutil.copy2('/tmp/x','src/other.txt')\"",
        ),
        (
            Wc,
            "python3 -c \"import os; os.rename('/tmp/x','src/other.txt')\"",
        ),
        (
            Wc,
            "node -e \"require('fs').copyFileSync('/tmp/x','src/other.txt')\"",
        ),
        (
            Wc,
            "node -e \"require('fs').renameSync('/tmp/x','src/other.txt')\"",
        ),
        (
            Root,
            "python3 -c \"import shutil; shutil.copyfile('/tmp/x','factory/policy.toml')\"",
        ),
        (Wc, "python3 -c \"import os; os.remove('src/other.txt')\""),
        (Root, "rm factory/policy.toml.bak"),
        (Root, "ln factory/policy.toml hard"),
    ];
    for (place, cmd) in refuses {
        let cwd = if *place == Root {
            fx.root.clone()
        } else {
            wc.clone()
        };
        let (d, _) = decision(&fx, &cwd, cmd);
        assert_eq!(d, "deny", "should be refused: {cmd}");
    }
}
