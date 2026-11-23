//! D-124: the engine holds on the Linux runner as on macOS. The test project of these tests lies under
//! `/tmp`, which is where `TempDir` puts it on Linux (on macOS it canonicalises to `/private/tmp`, so the
//! case is the same): the folder `/tmp` then holds the project's protected places, and `mktemp -p /tmp`
//! must be judged by the new file it makes, not by the folder.

mod common;
use common::*;
use serde_json::json;
use std::path::{Path, PathBuf};

fn decision(fx: &Fx, cwd: &Path, cmd: &str) -> (String, String) {
    let payload =
        json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    match o.stdout.trim() {
        "" | "{}" => ("abstain".into(), String::new()),
        s => {
            let v: serde_json::Value = serde_json::from_str(s).unwrap();
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

/// A project under `/tmp` whose unit U-1 may write `src/hello.txt`, with its working copy started.
fn place() -> (Fx, PathBuf) {
    let fx = Fx::new_in_tmp();
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/hello.txt exists\"]\n"),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/hello.txt\"]\ncheckpoint=\"K-1\"\nestimate=2\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"The file\"\nhow=\"command\"\ncommand=\"ls src/hello.txt\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"The message\"\nafter=[\"U-1\"]\nproves=[\"R-1\",\"C-1\"]\nchecks=[\"X-3\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("records");
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    let _ = std::fs::create_dir_all(wc.join("src"));
    (fx, wc)
}

fn rows(v: &[String], fx: &Fx, cwd: &Path, want_deny: bool, must_name: &[&str]) {
    let mut wrong = Vec::new();
    for cmd in v {
        let (d, why) = decision(fx, cwd, cmd);
        let denied = d == "deny";
        if denied != want_deny {
            wrong.push(format!(
                "{} but wanted {}: {cmd}\n    {}",
                if denied { "refused" } else { "passed" },
                if want_deny { "refused" } else { "passed" },
                why.lines().collect::<Vec<_>>().join(" | ")
            ));
        } else if want_deny && !must_name.is_empty() && !must_name.iter().any(|n| why.contains(n)) {
            wrong.push(format!(
                "refused without naming {must_name:?}: {cmd}\n    {}",
                why.lines().collect::<Vec<_>>().join(" | ")
            ));
        }
    }
    assert!(wrong.is_empty(), "wrong answers:\n{}", wrong.join("\n"));
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// A file made in `/tmp` is no write to the folder `/tmp`, though the project lies in it.
#[test]
fn mktemp_in_the_temporary_folder_passes_from_a_working_copy() {
    let (fx, wc) = place();
    rows(
        &s(&[
            "mktemp -p /tmp x.XXXX",
            "mktemp -p/tmp x.XXXX",
            "mktemp -dp /tmp x.XXXX",
            "mktemp -d -p /tmp x.XXXX",
            "mktemp --tmpdir=/tmp x.XXXX",
            "mktemp --tmp=/tmp x.XXXX",
            "mktemp --tmpdir x.XXXX",
            "mktemp -t x.XXXX",
            "mktemp -tq x.XXXX",
            "mktemp -q -p /tmp --suffix=.txt x.XXXX",
            "mktemp -p /tmp -- x.XXXX",
            "mktemp",
        ]),
        &fx,
        &wc,
        false,
        &[],
    );
}

/// Folders deleted or opened whole, and files made inside a protected place, stay refused, however the
/// folder and the template are spelled.
#[test]
fn mktemp_into_a_protected_place_and_whole_folders_stay_refused() {
    let (fx, wc) = place();
    let root = fx.root.to_string_lossy().to_string();
    let sib = fx.root.parent().unwrap().join("sib");
    std::fs::create_dir_all(&sib).unwrap();
    let project = fx.root.file_name().unwrap().to_string_lossy().to_string();
    let reaching_factory = vec![
        "mktemp -p factory x.XXXX".to_string(),
        "mktemp -pfactory x.XXXX".to_string(),
        "mktemp -dpfactory x.XXXX".to_string(),
        "mktemp -qdp factory x.XXXX".to_string(),
        "mktemp --tmpdir=factory x.XXXX".to_string(),
        "mktemp --tmp=factory x.XXXX".to_string(),
        "mktemp --t=factory x.XXXX".to_string(),
        "mktemp -t -p factory x.XXXX".to_string(),
        "mktemp -p factory -t x.XXXX".to_string(),
        "mktemp -tq factory/x.XXXX".to_string(),
        "mktemp -td factory/x.XXXX".to_string(),
        "mktemp -tq -p /tmp factory/x.XXXX".to_string(),
        "mktemp -t q -p /tmp factory/x.XXXX".to_string(),
        format!("mktemp -p /tmp {root}/factory/x.XXXX"),
        format!("mktemp -p {root}/src ../factory/x.XXXX"),
        format!("mktemp -p {} ../{project}/factory/x.XXXX", sib.display()),
        "mktemp -d -p factory/.. factory/x.XXXX".to_string(),
        "TMPDIR=factory mktemp -t x.XXXX".to_string(),
        "TMPDIR=factory mktemp --tmpdir x.XXXX".to_string(),
        "export TMPDIR=factory; mktemp -t x.XXXX".to_string(),
    ];
    // (`.claude/` as a whole is not protected, only the files and folders `protected_paths` names.)
    let reaching_claude = s(&[
        "mktemp -p . .claude/settings.json",
        "mktemp -p .claude settings.local.json",
        "mktemp -p . .claude/skills/factory-panel/x.XXXX",
        "mktemp -tq .claude/settings.json",
    ]);
    let unreadable = s(&[
        "mktemp --d x.XXXX",
        "mktemp --frobnicate x.XXXX",
        "mktemp -z x.XXXX",
        "mktemp x.XXXX -p factory",
        "mktemp -p /tmp --suffix=/x x.XXXX",
        "mktemp --tmpdir factory x.XXXX",
    ]);
    let whole = s(&["rm -rf /tmp", "chmod -R u+w /tmp", "rm -rf /tmp/"]);
    for cwd in [wc.as_path(), fx.root.as_path()] {
        rows(&reaching_factory, &fx, cwd, true, &["factory/"]);
        rows(&reaching_claude, &fx, cwd, true, &[".claude"]);
        rows(&unreadable, &fx, cwd, true, &["mktemp"]);
        rows(&whole, &fx, cwd, true, &[]);
    }
}
