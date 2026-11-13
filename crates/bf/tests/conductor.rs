//! Behaviours the harness relies on: `next` says why nothing starts, a checkpoint brief for the
//! verifier, gate rounds, question lists, drafts, the idea brief, findings by record.

mod common;
use common::*;
use serde_json::json;
use std::fs;

#[test]
fn next_says_plainly_when_nothing_can_start_and_why() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    // Before the plan gate: only a human directive.
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["can_start"], false);
    assert_eq!(n["human_needed"], true);
    assert!(n["idle_reason"].as_str().unwrap().contains("human"), "{n}");
    fx.must(&["gate", "open", "plan"]);
    fx.human("approved");
    fx.must(&["gate", "approve", "plan", "--quote", "approved"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["can_start"], true);
    assert!(n["idle_reason"].is_null());
    let d = &n["directives"][0];
    for k in ["role", "unit", "tier", "working_copy", "brief", "limits"] {
        assert!(!d[k].is_null(), "directive lacks {k}: {d}");
    }
    assert_eq!(d["tier"], "standard");
    // U-1 running: still something to do. Park it by exhausting reviews: then only the human is left.
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.must(&["unit", "start", "U-2"]);
    fx.must(&["report", "U-2", "--result", "blocked"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["can_start"], false);
    assert!(n["idle_reason"].as_str().unwrap().contains("U-2"), "{n}");
    assert_eq!(n["directives"][0]["role"], "human");
}

#[test]
fn next_reports_a_finished_run() {
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
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["role"], "verifier", "{n}");
    fx.must_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "run",
            "X-3",
            "--",
            "ls",
            "src/hello.txt",
            "src/bye.txt",
        ],
    );
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["role"], "engine");
    assert_eq!(n["directives"][0]["action"], "close-checkpoint");
    fx.must(&["checkpoint", "close", "K-1"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["can_start"], false);
    assert!(
        n["idle_reason"].as_str().unwrap().starts_with("run done"),
        "{n}"
    );
}

#[test]
fn a_verifier_can_be_briefed_on_a_checkpoint() {
    let fx = Fx::new();
    fx.tiny();
    let o = fx.must(&["brief", "K-1", "--role", "verifier"]);
    let text = fs::read_to_string(o.stdout.trim()).unwrap();
    assert!(
        text.contains("blind verifier")
            && text.contains("A greeting exists")
            && text.contains("X-3"),
        "{text}"
    );
    assert!(!text.contains("src/hello.txt` ")); // no source scope handed over
    let o = fx.refused(&["brief", "K-1", "--role", "builder"]);
    assert!(o.stderr.contains("verifier"));
}

#[test]
fn gate_open_prints_the_id_and_a_rejected_round_is_followed_by_the_next() {
    let fx = Fx::new();
    let o = fx.must(&["gate", "open", "architecture"]);
    assert_eq!(o.stdout.trim(), "architecture-r1");
    fx.refused(&["gate", "open", "architecture"]); // still open
    fx.human("no, change the storage decision");
    fx.must(&[
        "gate",
        "reject",
        "architecture-r1",
        "--note",
        "storage",
        "--quote",
        "no, change the storage decision",
    ]);
    let g = fx.must(&["gate", "status", "--json"]).json();
    assert_eq!(g[0]["status"], "rejected");
    let o = fx.must(&["gate", "open", "architecture"]);
    assert_eq!(o.stdout.trim(), "architecture-r2");
}

#[test]
fn questions_are_asked_as_a_list_and_answered_one_by_one() {
    let fx = Fx::new();
    let dir = tempfile::tempdir().unwrap();
    let q = dir.path().join("q.json");
    fs::write(&q, json!([
        {"title": "Which tone?", "context": "c", "kind": "single", "options": [{"key": "A", "text": "Warm", "proposed": true, "why": "friendly"}, {"key": "B", "text": "Plain"}]},
        {"title": "Anything else?", "kind": "text"},
        {"title": "Which targets?", "kind": "multi", "options": [{"key": "a", "text": "Web"}, {"key": "b", "text": "CLI"}]}
    ]).to_string()).unwrap();
    let o = fx
        .must(&[
            "ask",
            "--file",
            q.to_str().unwrap(),
            "--stage",
            "requirements",
            "--json",
        ])
        .json();
    let ids: Vec<&str> = o
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["Q-1", "Q-2", "Q-3"]);
    // `--stage` fills the stage of every entry that has none: the stage's gate waits for them.
    assert_eq!(
        fx.must(&["record", "get", "Q-2", "--json"]).json()["stage"],
        "requirements"
    );
    fx.refused(&["gate", "open", "requirements"]);
    assert_eq!(
        fx.must(&["questions", "--open", "--json"])
            .json()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    // A bad list writes nothing.
    let bad = dir.path().join("bad.json");
    fs::write(
        &bad,
        json!([{"title": "ok", "kind": "single"}, {"title": "", "kind": "single"}]).to_string(),
    )
    .unwrap();
    fx.refused(&["ask", "--file", bad.to_str().unwrap()]);
    assert_eq!(
        fx.must(&["questions", "--json"])
            .json()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let a = dir.path().join("a.json");
    fs::write(&a, json!({"choice": "A", "note": "because"}).to_string()).unwrap();
    fx.human("done, the form is filled in");
    fx.must(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    let v = fx.must(&["view", "questions"]).json();
    assert_eq!(v["questions"][0]["answer"]["choice"], "A");
    assert_eq!(v["questions"][0]["options"][0]["proposed"], true);
    // render questions needs no id.
    let outp = dir.path().join("q.html");
    fx.must(&[
        "render",
        "questions",
        "--template",
        fixture("template.html").to_str().unwrap(),
        "--out",
        outp.to_str().unwrap(),
    ]);
}

#[test]
fn drafts_and_the_idea_brief_pass_the_guard() {
    let fx = Fx::new();
    assert!(fx.root.join(".bf/drafts").is_dir());
    let g = |path: &str| {
        let payload = json!({"tool_name": "Write", "tool_input": {"file_path": fx.root.join(path).to_string_lossy()}, "cwd": fx.root.to_string_lossy()});
        let o = fx.bf_stdin(&fx.root, &["guard"], &payload.to_string());
        if o.stdout.trim() == "{}" {
            return "allow".to_string(); // nothing denies it: the guard stays silent
        }
        o.json()["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(g(".bf/drafts/r1.toml"), "allow");
    assert_eq!(g("factory/idea.md"), "allow");
    assert_eq!(g("factory/policy.toml"), "deny");
    assert_eq!(g(".bf/guard.json"), "deny");
}

#[test]
fn an_idea_is_a_record_kind_and_idea_md_is_shown_at_the_idea_gate() {
    let fx = Fx::new();
    assert_eq!(
        fx.put(
            "idea",
            "id=\"I-1\"\nkind=\"idea\"\ntitle=\"A greeting service\"\ntext=\"Says hello.\"\n"
        )
        .code,
        0
    );
    fs::write(
        fx.root.join("factory/idea.md"),
        "# A greeting service\n\nSays hello.\n",
    )
    .unwrap();
    fx.must(&["gate", "open", "idea"]);
    let v = fx.must(&["view", "gate", "idea"]).json();
    assert_contract("idea gate", &v);
    let titles: Vec<&str> = v["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert!(
        titles.contains(&"idea.md") && titles.contains(&"I-1.toml"),
        "{titles:?}"
    );
    let rows = v["sections"][0]["rows"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["id"] == "idea") && rows.iter().any(|r| r["id"] == "I-1"));
}

#[test]
fn a_reviewer_with_nothing_to_report_moves_the_unit_on_and_findings_can_be_filed_by_record() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fx.must(&["report", "U-1", "--result", "built"]);
    // No finding on file: `findings` is a clean review.
    let r = fx
        .must(&[
            "report", "U-1", "--result", "findings", "--role", "reviewer", "--json",
        ])
        .json();
    assert_eq!(r["state"], "reviewed");

    // Land U-1 properly, then do U-2: the reviewer files a finding as a record, from the unit's own working copy.
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hi\n").unwrap();
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
            "src/hello.txt",
        ],
    );
    fx.must(&["unit", "land", "U-1"]);
    let o = fx.must(&["unit", "start", "U-2", "--json"]);
    let wc2 = std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fx.must(&["report", "U-2", "--result", "built"]);
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f.toml");
    fs::write(&f, "id=\"F-1\"\nkind=\"finding\"\ntitle=\"Missing farewell wording\"\nunit=\"U-2\"\nseverity=\"high\"\n").unwrap();
    // A worker may file findings and nothing else.
    fx.must_in(
        &wc2,
        &["record", "put", "finding", "--file", f.to_str().unwrap()],
    );
    let req = tmp.path().join("r.toml");
    fs::write(&req, "id=\"R-8\"\nkind=\"requirement\"\ntitle=\"Sneaky\"\n").unwrap();
    let o = fx.bf_in(
        &wc2,
        &[
            "record",
            "put",
            "requirement",
            "--file",
            req.to_str().unwrap(),
        ],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
    // The finding has no outcome field yet: it counts as open, and the reviewer's `findings` sends the unit back.
    let r = fx
        .must(&[
            "report", "U-2", "--result", "findings", "--role", "reviewer", "--json",
        ])
        .json();
    assert_eq!(r["state"], "fixing");
    fx.must(&["report", "U-2", "--result", "fixed"]);
    assert_eq!(
        fx.must(&["record", "get", "F-1", "--json"]).json()["outcome"],
        "fixed"
    );
}
