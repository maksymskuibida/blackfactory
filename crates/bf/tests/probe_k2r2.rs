//! Probes of the K-2 checkpoint review, round 2.

mod common;
use common::*;
use serde_json::json;
use std::fs;

// ---- probe 1: a judged go gives a unit an existing test file ----

fn unit(id: &str, cp: &str, file: &str, deps: &[&str], est: u32) -> String {
    format!(
        "id=\"{id}\"\nkind=\"unit\"\ntitle=\"Unit {id}\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"{file}\"]\ndepends_on={}\ncheckpoint=\"{cp}\"\nestimate={est}\nfull_suite=\"fixture\"\n",
        json!(deps)
    )
}

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"Things exist\"\nacceptance=[\"they exist\"]\n".to_string()),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Things\"\n".to_string()),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    for i in 1..=6 {
        let o = fx.put("unit", &unit(&format!("U-{i}"), "K-1", &format!("src/f{i}.txt"), &[], 5));
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let o = fx.put("unit", &unit("U-7", "K-2", "src/f7.txt", &[], 1));
    assert_eq!(o.code, 0, "{}", o.all());
    for i in 1..=7 {
        let t = format!("id=\"X-{}\"\nkind=\"check\"\ntitle=\"Unit {i}\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{i}\"\n", 10 + i);
        assert_eq!(fx.put("check", &t).code, 0);
    }
    for (k, t) in [
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"First\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"Second\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test.\"\ntitle=\"One\"\nafter=[\"U-1\",\"U-2\",\"U-3\",\"U-4\",\"U-5\",\"U-6\"]\nproves=[\"R-1\",\"C-1\"]\nchecks=[\"X-1\"]\n"),
        ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test.\"\ntitle=\"Two\"\nafter=[\"U-7\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    // An existing test file that no unit holds: a proof the human approved.
    fs::create_dir_all(fx.root.join("crates/bf/tests")).unwrap();
    fs::write(fx.root.join("crates/bf/tests/existing_guard.rs"), "#[test]\nfn holds() { assert!(true); }\n").unwrap();
    fx.commit_all("plan");
    fx.start_test_run();
}

#[test]
fn probe_k2r2_a_go_that_gives_a_unit_an_existing_test_file_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    let o = fx.must(&["amend", "open", "--repair", "U-1", "--json"]).json();
    let f = fx.root.join("edit.toml");
    fs::write(&f, unit("U-1", "K-1", "src/f1.txt\", \"crates/bf/tests/existing_guard.rs", &[], 5)).unwrap();
    fx.must(&["record", "put", "unit", "--file", f.to_str().unwrap(), "--replace"]);
    fs::remove_file(&f).unwrap();
    let aid = o["amend"].as_str().unwrap().to_string();
    let n = fx.root.join("narrow.json");
    fs::write(&n, json!({"amend": aid, "keep": o["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", n.to_str().unwrap()]);
    let tok = fx.must(&["roles", "token", &aid, "judge", "--json"]).json()["token"].as_str().unwrap().to_string();
    let d = fx.root.join(".claude/agents");
    fs::create_dir_all(&d).unwrap();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness/claude/agents/repair-judge.md"),
        d.join("repair-judge.md"),
    )
    .unwrap();
    let payload = json!({"tool_name": "Bash", "cwd": fx.root.to_string_lossy(),
        "tool_input": {"command": format!("bf amend judge {aid} --file - --token {tok} --model m")},
        "session_id": "s1", "agent_id": "g1", "agent_type": "repair-judge"});
    assert_eq!(fx.bf_stdin(&fx.root, &["guard"], &payload.to_string()).code, 0);
    let out = fx.bf_stdin(
        &fx.root,
        &["amend", "judge", &aid, "--file", "-", "--token", &tok, "--model", "m"],
        &json!({"verdict": "go", "reasons": "plain words"}).to_string(),
    );
    // R-69: `files` may change only by a moved entry or a new test file; this file exists already.
    assert_eq!(out.code, 2, "a go gave U-1 an existing test file: {}", out.all());
    assert!(out.stderr.contains("crates/bf/tests/existing_guard.rs"), "{}", out.all());
}

// ---- probe 2: the accept of findings on a unit parked while fixing ----

fn finding(fx: &Fx, id: &str, severity: &str) {
    let o = fx.put(
        "finding",
        &format!("id=\"{id}\"\nkind=\"finding\"\ntitle=\"t {id}\"\nunit=\"U-1\"\nseverity=\"{severity}\"\n"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn probe_k2r2_accepting_findings_of_a_unit_parked_while_fixing_does_not_make_it_reviewed() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    finding(&fx, "F-1", "high");
    let p = fx.project();
    // The review filed F-1 and sent the unit back to its builder (round 1 of 3, not a parked review).
    bf::events::append(
        &p,
        "unit.report",
        json!({"unit": "U-1", "result": "findings", "from": "review", "to": "fixing",
               "findings": ["F-1"], "round": 1, "content": "c", "head": "h", "covered": "all"}),
    )
    .unwrap();
    // The builder then went idle in the middle of its fix and the unit was parked for it.
    bf::events::append(&p, "unit.park", json!({"unit": "U-1", "reason": "idle for 90 minutes"})).unwrap();
    let words = "I accept that finding as it is";
    fx.human(words);
    let o = fx.bf(&["unit", "accept", "U-1", "F-1", "--quote", words, "--json"]);
    let st = fx.must(&["status", "--json"]).json();
    let state = st["units"].as_array().unwrap().iter().find(|u| u["id"] == "U-1").unwrap()["state"]
        .as_str()
        .unwrap()
        .to_string();
    // R-71 lets the human accept findings of a parked review; a unit parked mid-fix holds work no
    // reviewer saw, so it must not become `reviewed` (and so landable) by the accept.
    assert_ne!(state, "reviewed", "accept made a mid-fix unit reviewed: {}", o.all());
}
