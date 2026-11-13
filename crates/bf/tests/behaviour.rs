//! Behaviours of the engine: handoff proofs, ID rules, test grants before a run, verifier outcomes, findings on
//! stdin, close then merge, brief wording, and what the plan gate page shows.

mod common;
use common::*;
use std::fs;

// ------------------------------------------------------------

#[test]
fn checks_and_checkpoints_may_prove_handoffs_and_every_handoff_needs_a_proof() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["plan", "check"]); // X-1 proves H-1
    let k = fx.must(&["record", "get", "K-1", "--json"]).json();
    assert_eq!(k["proves"].as_array().unwrap().len(), 3);
    // A checkpoint may name the handoff too.
    let o = fx.put("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\",\"H-1\"]\nchecks=[\"X-3\",\"X-1\"]\n");
    assert_eq!(o.code, 2, "replacing needs --replace: {}", o.all());
    // Take H-1 out of every check: the plan is refused.
    let x1 = "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n";
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("x1.toml");
    fs::write(&f, x1).unwrap();
    fx.must(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("handoff H-1 is proved by no check"),
        "{}",
        o.stderr
    );
    // A handoff proof by a check that runs at the checkpoint is fine and does not stall the receiver.
    let x3 = "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"ls src/hello.txt src/bye.txt\"\nproves=[\"R-1\",\"R-2\",\"C-1\",\"H-1\"]\nscope=\"checkpoint\"\n";
    fs::write(&f, x3).unwrap();
    fx.must(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    fx.must(&["plan", "check"]);
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["unit"] == "U-2"),
        "{n}"
    );
    // Nonsense IDs are still refused.
    let o = fx.put("check", "id=\"X-7\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"U-1\"]\nscope=\"checkpoint\"\n");
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("proves"));
}

// ------------------------------------------------------------

#[test]
fn put_never_renumbers_silently() {
    let fx = Fx::new();
    // No id: the engine assigns one and says so, in text and in JSON.
    let o = fx.put("requirement", "kind=\"requirement\"\ntitle=\"First\"\n");
    assert!(
        o.stdout
            .contains("put R-1 (the file had no id, so the engine assigned R-1"),
        "{}",
        o.all()
    );
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("r.toml");
    fs::write(&f, "kind=\"requirement\"\ntitle=\"Second\"\n").unwrap();
    let j = fx
        .must(&[
            "record",
            "put",
            "requirement",
            "--file",
            f.to_str().unwrap(),
            "--json",
        ])
        .json();
    assert_eq!(j["put"], "R-2");
    assert_eq!(j["assigned"], true);
    // An id taken by a different record is refused, with the way out.
    let o = fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"Not the first\"\n",
    );
    assert_eq!(o.code, 2);
    assert_four_parts(&o.stderr);
    assert!(
        o.stderr.contains("already a record")
            && o.stderr.contains("--replace")
            && o.stderr.contains("bf id new"),
        "{}",
        o.stderr
    );
    assert_eq!(
        fx.must(&["record", "get", "R-1", "--json"]).json()["title"],
        "First"
    );
    // The same file again is not a different record.
    assert_eq!(
        fx.put(
            "requirement",
            "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"First\"\n"
        )
        .code,
        0
    );
    // --replace revises it on purpose.
    fs::write(
        &f,
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"First, revised\"\n",
    )
    .unwrap();
    fx.must(&[
        "record",
        "put",
        "requirement",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(
        fx.must(&["record", "get", "R-1", "--json"]).json()["title"],
        "First, revised"
    );
    // `id new` reserves: two calls, two numbers; a put with the printed id lands exactly there.
    let a = fx
        .must(&["id", "new", "requirement"])
        .stdout
        .trim()
        .to_string();
    let b = fx
        .must(&["id", "new", "requirement"])
        .stdout
        .trim()
        .to_string();
    assert_eq!((a.as_str(), b.as_str()), ("R-3", "R-4"));
    assert_eq!(
        fx.put(
            "requirement",
            &format!("id=\"{a}\"\nkind=\"requirement\"\ntitle=\"Third\"\n")
        )
        .code,
        0
    );
    let o = fx.put(
        "requirement",
        "kind=\"requirement\"\ntitle=\"Unnumbered\"\n",
    );
    assert!(
        o.stdout.contains("assigned R-5"),
        "a reserved number is not handed out again: {}",
        o.all()
    );
    // Findings are the one kind a reviewer replaces by putting it again.
    fx.put(
        "finding",
        "id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\n",
    );
    assert_eq!(
        fx.put(
            "finding",
            "id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\noutcome=\"accepted\"\n"
        )
        .code,
        0
    );
}

// ------------------------------------------------------------

#[test]
fn a_grant_before_the_run_covers_the_design_gates_in_order() {
    let fx = Fx::new();
    fx.tiny();
    // Without a grant a simulated approval satisfies nothing.
    fx.must(&["gate", "open", "idea"]);
    let o = fx.refused(&["gate", "approve", "idea", "--simulated"]);
    assert!(o.stderr.contains("no granted test run"), "{}", o.stderr);
    fx.human("yes, this is a test run");
    let g = fx
        .must(&[
            "test-mode",
            "grant",
            "--json",
            "--quote",
            "yes, this is a test run",
        ])
        .json();
    assert_eq!(g["applies_to"], "the next run");
    assert_eq!(
        fx.must(&["status", "--json"]).json()["test_mode"],
        "pending"
    );
    // No run exists yet; the design gates go through in order.
    for kind in ["idea", "requirements", "ui", "architecture", "plan"] {
        // The idea gate was opened above (and refused a simulated approval); the others open now.
        if kind != "idea" {
            fx.must(&["gate", "open", kind]);
        }
        let gid = format!("{kind}-r1");
        let a = fx.bf(&["gate", "approve", &gid, "--simulated", "--json"]);
        assert_eq!(a.code, 0, "{kind}: {}", a.all());
        assert_eq!(a.json()["simulated"], true);
    }
    // Starting the run consumes the grant, and the run is a test run.
    fx.must(&["run", "start"]);
    assert_eq!(
        fx.must(&["status", "--json"]).json()["test_mode"],
        "granted"
    );
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
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
    // Anything touched by a simulated approval can never reach main.
    fx.must(&["checkpoint", "close", "K-1"]);
    let o = fx.refused(&["checkpoint", "close", "K-1", "--merge"]);
    assert!(o.stderr.contains("simulated"), "{}", o.stderr);
}

#[test]
fn a_grant_does_not_let_a_run_gate_or_a_second_run_borrow_it() {
    let fx = Fx::new();
    fx.tiny();
    fx.human("yes, test run please");
    fx.must(&["test-mode", "grant", "--quote", "yes, test run please"]);
    // The step gate of a checkpoint is not a design gate: it needs a run.
    fx.must(&["gate", "open", "checkpoint-step", "--ref", "K-1"]);
    let o = fx.refused(&["gate", "approve", "checkpoint-step-K-1-r1", "--simulated"]);
    assert!(o.stderr.contains("no granted test run"), "{}", o.stderr);
    // After the run starts the grant is spent: a later run without a new grant is a real run.
    fx.must(&["run", "start"]);
    fx.must(&["run", "stop"]);
    fx.must(&["run", "start"]);
    assert_eq!(fx.must(&["status", "--json"]).json()["test_mode"], "none");
}

// ------------------------------------------------------------

fn two_units_landed(fx: &Fx) {
    fx.tiny();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
}

#[test]
fn a_verifier_can_mark_a_check_unreachable_or_failed() {
    let fx = Fx::new();
    two_units_landed(&fx);
    let wc = fx.work("K-1");
    let o = fx.bf_in(&wc, &["evidence", "unreachable", "X-3"]);
    assert_eq!(o.code, 2, "a reason is required");
    assert_four_parts(&o.stderr);
    let r = fx
        .must_in(
            &wc,
            &[
                "evidence",
                "unreachable",
                "X-3",
                "--reason",
                "the staging service is down",
                "--json",
            ],
        )
        .json();
    assert_eq!(r["type"], "unreachable");
    // It is not a pass: the checkpoint will not close and says why.
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(
        o.stderr.contains("X-3 is marked unreachable")
            && o.stderr.contains("staging service is down"),
        "{}",
        o.stderr
    );
    // `bf next` hands it to the human, not to the verifier again.
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["can_start"], false);
    assert_eq!(n["human_needed"], true);
    assert_eq!(n["directives"][0]["action"], "decide-check");
    assert_eq!(n["directives"][0]["checks"][0]["outcome"], "unreachable");
    assert!(
        n["idle_reason"].as_str().unwrap().contains("unreachable"),
        "{n}"
    );
    // Views show it.
    let v = fx.must(&["view", "checkpoint", "K-1"]).json();
    assert_contract("checkpoint", &v);
    let row = &v["sections"][0]["rows"][0];
    assert_eq!(row["needs_you"], true);
    assert!(
        row["body"].to_string().contains("staging service is down"),
        "{row}"
    );
    // A receipt for the same check at the same content replaces the mark.
    fx.must_in(
        &wc,
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
    assert_eq!(n["directives"][0]["action"], "close-checkpoint", "{n}");
    fx.must(&["checkpoint", "close", "K-1"]);
}

#[test]
fn a_failed_check_is_recorded_the_same_way_and_never_counts_as_proof() {
    let fx = Fx::new();
    two_units_landed(&fx);
    let wc = fx.work("K-1");
    fx.must_in(
        &wc,
        &[
            "evidence",
            "fail",
            "X-3",
            "--reason",
            "the farewell is rude",
        ],
    );
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(o.stderr.contains("X-3 is marked fail"), "{}", o.stderr);
    // `evidence verify` does not call a failure ok.
    let v = fx.bf_in(&wc, &["evidence", "verify", "--checkpoint", "K-1"]);
    assert_eq!(v.code, 2, "{}", v.all());
    // The brief tells the verifier how to say it.
    let b = fx.must(&["brief", "K-1", "--role", "verifier"]);
    let text = fs::read_to_string(b.stdout.trim()).unwrap();
    assert!(
        text.contains("bf evidence unreachable") && text.contains("bf evidence fail"),
        "{text}"
    );
}

// ------------------------------------------------------------

#[test]
fn findings_can_be_piped_in() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    let json = r#"[{"title":"Hello is spelled wrong","severity":"medium","where":"src/hello.txt:1","action":"Fix the spelling"}]"#;
    let o = fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "U-1",
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--findings",
            "-",
            "--json",
        ],
        json,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["state"], "fixing");
    assert_eq!(
        fx.must(&["record", "get", "F-1", "--json"]).json()["title"],
        "Hello is spelled wrong"
    );
    // Bad input is a refusal, not a crash.
    fx.must(&["report", "U-1", "--result", "fixed"]);
    let o = fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "U-1",
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--findings",
            "-",
        ],
        "not json",
    );
    assert_eq!(o.code, 2);
    assert_four_parts(&o.stderr);
}

// ------------------------------------------------------------

#[test]
fn close_then_close_with_merge_merges_once_in_a_real_run() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("approved");
    fx.must(&["gate", "approve", "plan", "--quote", "approved"]);
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
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
    let c = fx.must(&["checkpoint", "close", "K-1", "--json"]).json();
    assert_eq!(c["merged"], false);
    assert!(!fx.root.join("src/hello.txt").exists());
    // Merging needs the human's go-ahead after the closure.
    let o = fx.refused(&["checkpoint", "close", "K-1", "--merge"]);
    assert!(o.stderr.contains("go-ahead"), "{}", o.stderr);
    let c = fx.merge_checkpoint("K-1").json();
    assert_eq!(c["merged"], true);
    assert!(fx.root.join("src/hello.txt").exists());
    let merges = git(&fx.root, &["rev-list", "--merges", "--count", "main"]);
    fx.must(&["checkpoint", "close", "K-1", "--merge"]);
    assert_eq!(
        merges,
        git(&fx.root, &["rev-list", "--merges", "--count", "main"])
    );
    // A plain close after the merge changes nothing either.
    let c = fx.must(&["checkpoint", "close", "K-1", "--json"]).json();
    assert_eq!(c["already"], true);
}

#[test]
fn close_then_merge_is_refused_for_a_test_run() {
    let fx = Fx::new();
    two_units_landed(&fx);
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
    fx.must(&["checkpoint", "close", "K-1"]);
    let o = fx.refused(&["checkpoint", "close", "K-1", "--merge"]);
    assert!(o.stderr.contains("simulated"), "{}", o.stderr);
    assert!(!fx.root.join("src/hello.txt").exists());
}

// ------------------------------------------------------------

#[test]
fn the_brief_does_not_forbid_the_folder_the_builder_works_in() {
    let fx = Fx::new();
    fx.tiny();
    let o = fx.must(&["brief", "U-1", "--role", "builder"]);
    let text = fs::read_to_string(o.stdout.trim()).unwrap();
    assert!(
        !text.contains("Never edit anything under `factory/` or `.bf/`"),
        "{text}"
    );
    assert!(
        text.contains("Work only inside your working copy") && text.contains(".bf/work/U-1"),
        "{text}"
    );
    let o = fx.must(&["brief", "K-1", "--role", "verifier"]);
    let text = fs::read_to_string(o.stdout.trim()).unwrap();
    assert!(
        !text.contains("Never edit anything under `factory/` or `.bf/`."),
        "{text}"
    );
}

#[test]
fn starting_a_unit_in_flight_resumes_it_and_help_works_everywhere() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    let f = serde_json::json!([{"title": "t"}]).to_string();
    fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "U-1",
            "--result",
            "findings",
            "--role",
            "reviewer",
            "--findings",
            "-",
        ],
        &f,
    );
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["units"][0]["state"], "fixing");
    let r = fx.must(&["unit", "start", "U-1", "--json"]).json();
    assert_eq!(r["resumed"], true);
    for a in [
        vec!["--help"],
        vec!["gate", "--help"],
        vec!["record", "put", "--help"],
        vec!["-h"],
    ] {
        let o = fx.bf(&a);
        assert_eq!(o.code, 0, "{a:?}: {}", o.all());
        assert!(o
            .stdout
            .contains("Every command that prints data takes --json"));
    }
}

// ------------------------------------------------------------

#[test]
fn needs_you_is_set_only_where_the_human_is_waited_for() {
    let fx = Fx::new();
    two_units_landed(&fx);
    let l = fx.must(&["view", "ledger"]).json();
    for r in l["sections"][0]["rows"].as_array().unwrap() {
        assert_eq!(r["needs_you"], false, "{r}");
    }
    let c = fx.must(&["view", "checkpoint", "K-1"]).json();
    for r in c["sections"][0]["rows"].as_array().unwrap() {
        assert_eq!(r["needs_you"], false, "nothing is marked: {r}");
    }
    // A parked unit does wait for the human.
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    fx.must(&["report", "U-1", "--result", "blocked"]);
    let r = fx.must(&["view", "run"]).json();
    assert_eq!(r["sections"][0]["rows"][0]["needs_you"], true);
    assert_eq!(r["sections"][0]["rows"][1]["needs_you"], false);
}

#[test]
fn the_plan_gate_shows_what_is_being_approved() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["gate", "open", "plan"]);
    let v = fx.must(&["view", "gate", "plan"]).json();
    assert_contract("plan gate", &v);
    let sections = v["sections"].as_array().unwrap();
    let units = sections.iter().find(|s| s["id"] == "unit").unwrap();
    let kinds: Vec<&str> = units["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["waves", "coverage"], "{units}");
    let cov = &units["blocks"][1]["rows"];
    assert_eq!(cov.as_array().unwrap().len(), 4); // R-1, R-2, C-1 and the handoff H-1
    assert!(
        cov.as_array()
            .unwrap()
            .iter()
            .all(|r| r["state"] == "planned" && r["unit"].as_str().unwrap().starts_with("U-")),
        "{cov}"
    );
    // Each check row lists the IDs it proves, and each checkpoint row too.
    let rows = |id: &str| {
        sections.iter().find(|s| s["id"] == id).unwrap()["rows"]
            .as_array()
            .unwrap()
            .clone()
    };
    let x1 = rows("check")
        .into_iter()
        .find(|r| r["id"] == "X-1")
        .unwrap();
    let table = x1["body"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "table")
        .unwrap();
    let ids: Vec<&str> = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["R-1", "C-1", "H-1"]);
    assert_eq!(table["rows"][0][1], "A greeting exists");
    assert_eq!(x1["refs"].as_array().unwrap().len(), 3);
    let k1 = rows("checkpoint")
        .into_iter()
        .find(|r| r["id"] == "K-1")
        .unwrap();
    let table = k1["body"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "table")
        .unwrap();
    assert_eq!(table["rows"].as_array().unwrap().len(), 3);
    // A gap shows as a gap.
    fx.put(
        "requirement",
        "id=\"R-5\"\nkind=\"requirement\"\ntitle=\"Orphan\"\n",
    );
    let v = fx.must(&["view", "gate", "plan"]).json();
    let cov = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "unit")
        .unwrap()["blocks"][1]["rows"]
        .clone();
    assert!(
        cov.as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == "R-5" && r["state"] == "gap"),
        "{cov}"
    );
}

#[test]
fn a_worker_may_file_an_open_finding_for_a_unit_in_review_and_nothing_more() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    let d = tempfile::tempdir().unwrap();
    let put = |body: &str| {
        let f = d.path().join("f.toml");
        fs::write(&f, body).unwrap();
        fx.bf_in(
            &wc,
            &["record", "put", "finding", "--file", f.to_str().unwrap()],
        )
    };
    // The unit is still being built: no review, no finding.
    assert_eq!(
        put("id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\n").code,
        2
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    assert_eq!(
        put("id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\n").code,
        0
    );
    // Closing a finding is not a worker's act, whoever it is.
    let o = put("id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\noutcome=\"accepted\"\n");
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
    // The human can, from the main checkout.
    let f = d.path().join("h.toml");
    fs::write(
        &f,
        "id=\"F-1\"\nkind=\"finding\"\ntitle=\"t\"\nunit=\"U-1\"\noutcome=\"accepted\"\n",
    )
    .unwrap();
    fx.must(&["record", "put", "finding", "--file", f.to_str().unwrap()]);
    let r = fx
        .must(&[
            "report", "U-1", "--result", "findings", "--role", "reviewer", "--json",
        ])
        .json();
    assert_eq!(
        r["state"], "reviewed",
        "an accepted finding does not hold the unit back"
    );
}
