//! U-143 (R-81, D-170, TH-22): the guard refuses `bf checkpoint finding --human` to every session
//! whose hook payload names an agent type, and passes the conductor.

mod common;
use common::*;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const AGENTS: [&str; 8] = [
    "builder",
    "reviewer",
    "verifier",
    "designer",
    "design-reviewer",
    "knowledge-writer",
    "security-auditor",
    "repair-judge",
];

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// U-1 started on a project; returns the fixture and the unit's working copy.
fn setup() -> (Fx, PathBuf) {
    let fx = Fx::new();
    let recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n".into()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ntext=\"Build a.\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\n".into()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".into()),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n".into()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"All\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-4\"]\n".into()),
    ];
    for (k, t) in &recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fx.set_policy("tokens = false", "tokens = true");
    write(&fx.root, "src/a.rs", "fn a() {}\n");
    fx.commit_all("files");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    (fx, wc)
}

fn guard(fx: &Fx, cwd: &Path, agent: Option<&str>, cmd: &str) -> String {
    let mut payload = json!({"tool_name": "Bash", "tool_input": {"command": cmd},
        "cwd": cwd.to_string_lossy(), "session_id": "s"});
    if let Some(a) = agent {
        payload["agent_type"] = json!(a);
        payload["agent_id"] = json!("agent-1");
    }
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    o.stdout
}

fn strings() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("strings/en/ck_human_guard.toml"))
        .unwrap()
}

#[test]
fn the_guard_refuses_a_human_filing_to_every_agent_type_and_passes_the_conductor() {
    let (fx, wc) = setup();
    let cmd = "bf checkpoint finding K-1 --human --quote \"fix it\"";
    for cwd in [fx.root.clone(), wc.clone()] {
        for a in AGENTS {
            let out = guard(&fx, &cwd, Some(a), cmd);
            assert!(
                out.contains("deny") && out.contains(a),
                "{a} in {}: {out}",
                cwd.display()
            );
        }
        // The conductor works in the main checkout; a working copy is a worker's place.
        if cwd == fx.root {
            assert!(!guard(&fx, &cwd, None, cmd).contains("deny"));
        }
        // A flag before the subcommand, after it or as `--human=`, and the batch form.
        for c in [
            "bf checkpoint --human finding K-1 --quote x",
            "bf --json checkpoint finding K-1 --human --findings -",
            "bf checkpoint finding K-1 --quote x --fix --human",
        ] {
            assert!(
                guard(&fx, &cwd, Some("reviewer"), c).contains("deny"),
                "{c}"
            );
            if cwd == fx.root {
                assert!(!guard(&fx, &cwd, None, c).contains("deny"), "{c}");
            }
        }
    }
}

#[test]
fn a_human_filing_behind_cargo_run_or_a_run_time_word_is_refused_to_an_agent() {
    let (fx, wc) = setup();
    for cmd in [
        "cargo run -- checkpoint finding K-1 --human --quote x",
        "cargo run -q checkpoint finding K-1 --human --quote x",
        "bf checkpoint $E finding K-1 --human --quote x",
        "bf checkpoint ${E}finding K-1 --human --quote x",
        "bf checkpoint f?nding K-1 --human --quote x",
        "bf checkpoint x(N) finding K-1 --human --quote x",
        "bf $C finding K-1 --human --quote x",
        "C=checkpoint; bf $C finding K-1 --human --quote x",
        "bf che*point finding K-1 --human --quote x",
    ] {
        for cwd in [&fx.root, &wc] {
            let out = guard(&fx, cwd, Some("builder"), cmd);
            assert!(
                out.contains("deny") && out.contains("builder"),
                "{cmd}: {out}"
            );
        }
    }
}

#[test]
fn a_filing_without_human_is_judged_as_today() {
    let (fx, wc) = setup();
    for cmd in [
        "bf checkpoint finding K-1 --severity low",
        "bf checkpoint status K-1",
        "bf report U-1 --result built --token t --note \"--human\"",
    ] {
        let with_agent = guard(&fx, &wc, Some("reviewer"), cmd);
        let none = guard(&fx, &fx.root, None, cmd);
        let human_row = with_agent.contains("only the conductor files the human's words");
        assert!(!human_row, "{cmd}: {with_agent}");
        assert!(!none.contains("only the conductor files"), "{cmd}: {none}");
    }
}

#[test]
fn every_text_of_the_row_comes_from_a_key() {
    let (fx, wc) = setup();
    let toml = strings();
    let want = [
        "human_finding.guard.refused",
        "human_finding.guard.because",
        "human_finding.guard.step",
        "human_finding.guard.worked",
        "human_finding.guard.opaque.refused",
        "human_finding.guard.opaque.because",
        "human_finding.guard.opaque.step",
        "human_finding.guard.opaque.worked",
    ];
    for k in want {
        assert!(toml.contains(&format!("\"{k}\" = ")), "{k}");
    }
    let plain = guard(
        &fx,
        &wc,
        Some("builder"),
        "bf checkpoint finding K-1 --human --quote x",
    );
    assert!(plain.contains("only the conductor files the human's words as a finding"));
    assert!(plain.contains("agent type `builder`"));
    let opaque = guard(
        &fx,
        &wc,
        Some("builder"),
        "bf checkpoint $E finding K-1 --human --quote x",
    );
    assert!(opaque.contains("words built at run time"), "{opaque}");
    // The source holds none of the row's text outside the strings.
    let src =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/guard.rs")).unwrap();
    assert!(!src.contains("only the conductor files"));
}

#[test]
fn a_run_time_word_after_the_subcommand_of_a_filing_is_refused_to_an_agent() {
    let (fx, wc) = setup();
    for cmd in [
        "bf checkpoint finding K-1 $F --quote x",
        "bf checkpoint finding K-1 ${F} --quote x",
        "F=--human; bf checkpoint finding K-1 $F --quote x",
    ] {
        for cwd in [&fx.root, &wc] {
            let out = guard(&fx, cwd, Some("builder"), cmd);
            assert!(
                out.contains("deny") && out.contains("builder"),
                "{cmd}: {out}"
            );
        }
        let c = guard(&fx, &fx.root, None, cmd);
        assert!(!c.contains("deny"), "{cmd}: {c}");
    }
}

#[test]
fn a_run_time_subcommand_with_a_run_time_word_is_refused_to_an_agent_as_a_human_filing() {
    let (fx, wc) = setup();
    for cmd in [
        "C=checkpoint; F=--human; bf $C finding K-1 $F --quote x",
        "S=finding; F=--human; bf checkpoint $S K-1 $F --quote x",
        "bf checkpoint $(echo finding --human) K-1 --quote x",
        "S=\"finding --human\"; bf checkpoint ${=S} K-1 --quote x",
        "bf checkpoint $S K-1 --quote x",
    ] {
        for cwd in [&fx.root, &wc] {
            let out = guard(&fx, cwd, Some("builder"), cmd);
            assert!(
                out.contains("deny") && out.contains("builder"),
                "{cmd}: {out}"
            );
        }
    }
}
