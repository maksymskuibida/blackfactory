//! How a run flows: checkpoints that build on each other, questions from workers, amendments that
//! re-open work, and handoffs that are proved by checks that really run.

mod common;
use common::*;
use std::fs;

/// Two checkpoints in plan order: K-1 holds U-1, K-2 holds U-2, and U-2's checkpoint check needs the
/// file U-1 made.
fn two_checkpoints(fx: &Fx) {
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"a exists\"]\n"),
        ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"B\"\nacceptance=[\"b exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"A\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"B\"\ndelivers=[\"R-2\"]\nfiles=[\"src/b.txt\"]\ncheckpoint=\"K-2\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"a\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"b\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"a again\"\nhow=\"command\"\ncommand=\"ls src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"both\"\nhow=\"command\"\ncommand=\"ls src/a.txt src/b.txt\"\nproves=[\"R-1\",\"R-2\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"First\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-3\"]\n"),
        ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\ntitle=\"Second\"\nafter=[\"U-2\"]\nproves=[\"R-1\",\"R-2\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("records");
}

/// U-1 and U-2 are both in K-1, so both can run at once.
fn two_units_in_one_checkpoint(fx: &Fx) {
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"a exists\"]\n"),
        ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"B\"\nacceptance=[\"b exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"A\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"B\"\ndelivers=[\"R-2\"]\nfiles=[\"src/b.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"a\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"b\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"both\"\nhow=\"command\"\ncommand=\"ls src/a.txt src/b.txt\"\nproves=[\"R-1\",\"R-2\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"All\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("records");
}

#[test]
fn a_later_checkpoint_builds_on_the_earlier_one_and_the_run_flows_through_without_a_merge() {
    let fx = Fx::new();
    two_checkpoints(&fx);
    fx.start_test_run();
    // U-2 waits for K-1 to close: its branch is cut from K-1.
    let n = fx.must(&["next", "--json"]).json();
    assert!(n["waiting"].to_string().contains("builds on K-1"), "{n}");
    fx.refused(&["unit", "start", "U-2"]);
    fx.build_unit("U-1", "src/a.txt", "X-1", &["test", "-f", "src/a.txt"]);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["role"], "verifier", "{n}");
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-3", "--", "ls", "src/a.txt"],
    );
    fx.must(&["checkpoint", "close", "K-1"]);
    // Nothing was merged to main, and the run goes on to the next checkpoint.
    assert!(!fx.root.join("src/a.txt").exists());
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["unit"], "U-2", "{n}");
    let wc = fx.must(&["unit", "start", "U-2", "--json"]).json()["working_copy"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        std::path::Path::new(&wc).join("src/a.txt").exists(),
        "the second checkpoint sees the first one's work"
    );
    // Finish U-2 by hand (it is already started).
    fs::write(std::path::Path::new(&wc).join("src/b.txt"), "b\n").unwrap();
    fx.must_in(
        std::path::Path::new(&wc),
        &[
            "evidence",
            "run",
            "X-2",
            "--unit",
            "U-2",
            "--",
            "test",
            "-f",
            "src/b.txt",
        ],
    );
    fx.must(&["report", "U-2", "--result", "built"]);
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-2"]);
    // The last checkpoint holds everything: its check over both files passes in its working copy.
    fx.must_in(
        &fx.work("K-2"),
        &[
            "evidence",
            "run",
            "X-4",
            "--",
            "ls",
            "src/a.txt",
            "src/b.txt",
        ],
    );
    fx.must(&["checkpoint", "close", "K-2"]);
    assert!(fx.work("K-2").join("src/a.txt").exists());
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["idle_reason"].as_str().unwrap().starts_with("run done"),
        "{n}"
    );
    // Main is untouched until the human's go-ahead, and one merge of the last checkpoint brings both.
    fx.refused(&["checkpoint", "close", "K-2", "--merge"]);
}

#[test]
fn a_worker_asks_the_human_and_only_its_unit_waits() {
    let fx = Fx::new();
    two_units_in_one_checkpoint(&fx);
    fx.start_test_run();
    let wc1 = std::path::PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fx.must(&["unit", "start", "U-2"]);
    // A question without a stage or a unit is an error with a recipe.
    let o = fx.refused(&["ask", "--title", "Which colour?"]);
    assert!(
        o.stderr.contains("--stage") && o.stderr.contains("--unit"),
        "{}",
        o.stderr
    );
    // The worker asks from its working copy: tagged by itself, no file needed.
    let o = fx.must_in(
        &wc1,
        &[
            "ask",
            "--title",
            "Greeting or farewell first?",
            "--context",
            "The brief does not say.",
            "--options",
            "A=greeting;B=farewell",
            "--json",
        ],
    );
    let q = o.json();
    assert_eq!(q[0]["unit"], "U-1", "{q}");
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert!(
        ds.iter().any(|d| d["role"] == "human"
            && d["action"] == "answer-question"
            && d["unit"] == "U-1"),
        "{n}"
    );
    assert!(
        !ds.iter()
            .any(|d| d["role"] == "builder" && d["unit"] == "U-1"),
        "{n}"
    );
    assert!(
        ds.iter()
            .any(|d| d["role"] == "builder" && d["unit"] == "U-2"),
        "{n}"
    );
    // The stage gate rule is untouched: an architecture question still holds its gate, and this one does not.
    fx.must(&["gate", "open", "idea"]);
    // The human answers; the answer is in the next brief and the unit goes on.
    fx.human("A, the greeting first.");
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.json");
    fs::write(&a, "{\"answer\": \"A\"}").unwrap();
    fx.must(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    let n = fx.must(&["next", "--json"]).json();
    assert!(
        n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["role"] == "builder" && d["unit"] == "U-1"),
        "{n}"
    );
    let brief = fs::read_to_string(
        fx.must(&["brief", "U-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        brief.contains("Greeting or farewell first?") && brief.contains("answered: A"),
        "{brief}"
    );
}

#[test]
fn an_approved_amendment_reopens_what_it_touches_and_nothing_else() {
    let fx = Fx::new();
    // Two checkpoints: K-1 has U-1, K-2 has U-2.
    two_checkpoints(&fx);
    fx.start_test_run();
    fx.build_unit("U-1", "src/a.txt", "X-1", &["test", "-f", "src/a.txt"]);
    fx.must_in(
        &fx.work("K-1"),
        &["evidence", "run", "X-3", "--", "ls", "src/a.txt"],
    );
    fx.must(&["checkpoint", "close", "K-1"]);
    fx.build_unit("U-2", "src/b.txt", "X-2", &["test", "-f", "src/b.txt"]);
    fx.must_in(
        &fx.work("K-2"),
        &[
            "evidence",
            "run",
            "X-4",
            "--",
            "ls",
            "src/a.txt",
            "src/b.txt",
        ],
    );
    fx.must(&["checkpoint", "close", "K-2"]);
    assert_eq!(
        fx.bf(&["ledger", "check"]).code,
        0,
        "closed before the amendment"
    );
    // The human changes R-2 only.
    let imp = fx.must(&["amend", "open", "R-2", "--json"]).json();
    let keep: Vec<String> = imp["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        keep.contains(&"U-2".to_string()) && !keep.contains(&"U-1".to_string()),
        "{imp}"
    );
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("n.json");
    fs::write(
        &f,
        serde_json::json!({"keep": keep, "waive": [], "reasons": {}}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, approve the change to the second requirement.");
    fx.must(&["amend", "approve", "--quote", "approve the change"]);
    // U-2 and K-2 are open again; U-1 and K-1 keep their state and approvals.
    let st = fx.must(&["status", "--json"]).json();
    let state = |u: &str| {
        st["units"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == u)
            .unwrap()["state"]
            .clone()
    };
    assert_eq!(state("U-1"), "landed", "{st}");
    assert_eq!(state("U-2"), "running", "{st}");
    let closed = st["checkpoints_closed"].to_string();
    assert!(closed.contains("K-1") && !closed.contains("K-2"), "{st}");
    // The ledger reads open until the new evidence exists, and `bf next` offers exactly the re-opened work.
    assert_eq!(fx.bf(&["ledger", "check"]).code, 2);
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "{n}");
    assert_eq!(ds[0]["unit"], "U-2");
    assert_eq!(ds[0]["amendment"], "A-1");
    // Old receipts of the changed checks do not land the unit again; new ones do.
    fx.must(&["report", "U-2", "--result", "built"]);
    fx.must(&["report", "U-2", "--result", "built", "--role", "reviewer"]);
    let o = fx.refused(&["unit", "land", "U-2"]);
    assert!(o.stderr.contains("X-2"), "{}", o.stderr);
    let wc = fx.work("U-2");
    fs::write(wc.join("src/b.txt"), "b, changed\n").unwrap();
    fx.must_in(
        &wc,
        &[
            "evidence",
            "run",
            "X-2",
            "--unit",
            "U-2",
            "--",
            "test",
            "-f",
            "src/b.txt",
        ],
    );
    fx.must(&["unit", "land", "U-2"]);
    fx.must_in(
        &fx.work("K-2"),
        &[
            "evidence",
            "run",
            "X-4",
            "--",
            "ls",
            "src/a.txt",
            "src/b.txt",
        ],
    );
    fx.must(&["checkpoint", "close", "K-2"]);
    assert_eq!(
        fx.bf(&["ledger", "check"]).code,
        0,
        "{}",
        fx.bf(&["ledger", "check"]).all()
    );
}

fn replace_record(fx: &Fx, kind: &str, toml_text: &str) {
    let f = fx.root.join(".bf/r.toml");
    fs::create_dir_all(fx.root.join(".bf")).unwrap();
    fs::write(&f, toml_text).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        kind,
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_handoff_proved_only_by_a_check_nothing_runs_is_a_plan_problem_and_a_ledger_row_needs_evidence()
{
    let fx = Fx::new();
    fx.tiny();
    // X-1 stops proving the handoff; X-9 does, but no unit owns it and no checkpoint lists it.
    replace_record(&fx, "check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -f src/hello.txt\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n");
    fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"Tone\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"H-1\"]\nscope=\"checkpoint\"\n");
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("H-1") && o.stderr.contains("nothing runs"),
        "{}",
        o.stderr
    );
    // A checkpoint lists it: the plan is fine, and the handoff is a ledger row that needs evidence.
    replace_record(&fx, "checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\",\"X-9\"]\n");
    fx.must(&["plan", "check"]);
    // R-1, R-2, C-1 and the handoff: four parts.
    let rows = fx.must(&["ledger", "check", "--plan", "--json"]).json();
    assert_eq!(rows["parts"], 4, "the handoff is a ledger row: {rows}");
    fx.commit_all("records");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    let o = fx.bf(&["ledger", "check"]);
    assert_eq!(o.code, 2, "H-1 has no evidence yet: {}", o.all());
    assert!(o.stderr.contains("H-1"), "{}", o.stderr);
}

fn ordered(fx: &Fx) {
    fx.set_policy("in_order = false", "in_order = true");
}

#[test]
fn gates_open_in_order_and_never_empty() {
    let fx = Fx::new();
    ordered(&fx);
    // Nothing behind a gate: it says what it needs, in the usual four parts.
    assert_four_parts(&fx.refused(&["gate", "open", "requirements"]).stderr);
    let o = fx.refused(&["gate", "open", "idea"]);
    assert!(
        o.stderr.contains("nothing to approve") && o.stderr.contains("idea"),
        "{}",
        o.stderr
    );
    fs::write(fx.root.join("factory/idea.md"), "# A tiny greeter\n").unwrap();
    // The order rule: requirements waits for the idea.
    let o = fx.refused(&["gate", "open", "requirements"]);
    assert!(o.stderr.contains("`idea` gate comes first"), "{}", o.stderr);
    fx.must(&["gate", "open", "idea"]);
    fx.human("The idea is right, approved.");
    fx.must(&["gate", "approve", "idea", "--quote", "approved"]);
    // The stage has nothing yet.
    let o = fx.refused(&["gate", "open", "requirements"]);
    assert!(
        o.stderr.contains("at least one requirement"),
        "{}",
        o.stderr
    );
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"a\"]\n",
    );
    fx.must(&["gate", "open", "requirements"]);
    // A rejected gate's next round is not held by the order rule.
    fx.human("No, change the wording.");
    fx.must(&[
        "gate",
        "reject",
        "requirements",
        "--quote",
        "No, change the wording.",
    ]);
    fx.must(&["gate", "open", "requirements"]);
    fx.human("Yes, those are right.");
    fx.must(&[
        "gate",
        "approve",
        "requirements",
        "--quote",
        "Yes, those are right.",
    ]);
    // ui needs a screen or a journey; architecture needs a component and a decision.
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(
        o.stderr.contains("`ui` gate comes first") && o.stderr.contains("gate skip ui"),
        "{}",
        o.stderr
    );
    let o = fx.refused(&["gate", "open", "ui"]);
    assert!(o.stderr.contains("screen or journey"), "{}", o.stderr);
    fx.put(
        "journey",
        "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\n",
    );
    fx.must(&["gate", "open", "ui"]);
    fx.human("The journey is fine, go ahead.");
    fx.must(&["gate", "approve", "ui", "--quote", "go ahead"]);
    fx.put(
        "component",
        "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n",
    );
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(
        o.stderr.contains("one component and one decision"),
        "{}",
        o.stderr
    );
    fx.put(
        "decision",
        "id=\"D-1\"\nkind=\"decision\"\ntitle=\"One module\"\nchosen=\"a\"\nrejected=\"b\"\n",
    );
    // No threat record: it opens, and the page says so.
    fx.must(&["gate", "open", "architecture"]);
    let page = fx
        .must(&["view", "gate", "architecture"])
        .json()
        .to_string();
    assert!(page.contains("No threat was recorded"), "{page}");
    fx.human("Architecture approved.");
    fx.must(&["gate", "approve", "architecture", "--quote", "approved"]);
    // The plan needs a whole plan with a unit and a checkpoint.
    let o = fx.refused(&["gate", "open", "plan"]);
    assert_four_parts(&o.stderr);
}

#[test]
fn a_product_with_no_interface_skips_the_ui_gate_in_the_humans_words() {
    let fx = Fx::new();
    ordered(&fx);
    fs::write(fx.root.join("factory/idea.md"), "# A library\n").unwrap();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"a\"]\n",
    );
    fx.put(
        "component",
        "id=\"C-1\"\nkind=\"component\"\ntitle=\"Core\"\n",
    );
    fx.put(
        "decision",
        "id=\"D-1\"\nkind=\"decision\"\ntitle=\"d\"\nchosen=\"a\"\nrejected=\"b\"\n",
    );
    fx.must(&["gate", "open", "idea"]);
    fx.human("Idea approved.");
    fx.must(&["gate", "approve", "idea", "--quote", "approved"]);
    fx.must(&["gate", "open", "requirements"]);
    fx.human("Requirements approved.");
    fx.must(&["gate", "approve", "requirements", "--quote", "approved"]);
    // Only a human's words skip it, and only the ui gate.
    fx.refused(&["gate", "skip", "ui", "--quote", "There is no interface."]);
    fx.refused(&["gate", "skip", "plan", "--quote", "There is no interface."]);
    fx.human("There is no interface, skip the ui gate.");
    fx.must(&["gate", "skip", "ui", "--quote", "skip the ui gate"]);
    // Once.
    fx.human("Skip it again, please.");
    fx.refused(&["gate", "skip", "ui", "--quote", "Skip it again"]);
    // It counts as passed for the order rule; the log, the status, the index and the architecture page say so.
    fx.must(&["gate", "open", "architecture"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["skipped_gates"][0]["gate"], "ui", "{st}");
    assert!(fx.must(&["log", "--json"]).stdout.contains("gate.skip"));
    let idx = fx.must(&["view", "index"]).json().to_string();
    assert!(idx.contains("The ui gate was skipped"), "{idx}");
    let arch = fx
        .must(&["view", "gate", "architecture"])
        .json()
        .to_string();
    assert!(
        arch.contains("UI gate skipped") && arch.contains("skip the ui gate"),
        "{arch}"
    );
}

#[test]
fn a_check_judged_by_an_agent_needs_a_stated_outcome_from_the_verifier() {
    let fx = Fx::new();
    fx.set_policy("tokens = false", "tokens = true");
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"a\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"A\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"a\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"Read the screen aloud\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"K\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-5\"]\n"),
    ] {
        assert_eq!(fx.put(k, t).code, 0);
    }
    fx.commit_all("records");
    fx.start_test_run();
    // Build U-1 with the tokens the engine shows.
    let wc = std::path::PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "a\n").unwrap();
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
            "src/a.txt",
        ],
    );
    let bt = fx.take_token("U-1", "builder");
    fx.must_in(&wc, &["report", "U-1", "--result", "built", "--token", &bt]);
    let rt = fx.take_token("U-1", "reviewer");
    fx.must_in(
        &wc,
        &[
            "report", "U-1", "--result", "built", "--role", "reviewer", "--token", &rt,
        ],
    );
    fx.must(&["unit", "land", "U-1"]);
    let n = fx.must(&["next", "--json"]).json();
    let d = &n["directives"][0];
    assert_eq!(d["role"], "verifier", "{n}");
    let vt = d["token"].as_str().unwrap().to_string();
    let k = fx.work("K-1");
    let shot = fx.root.join(".bf/shot.txt");
    fs::write(&shot, "the screen reads: hello\n").unwrap();
    let shot_s = shot.to_str().unwrap();
    // A file without an outcome is supporting material and settles nothing.
    fx.must_in(&k, &["evidence", "attach", "X-5", shot_s]);
    assert!(fx
        .refused(&["checkpoint", "close", "K-1"])
        .stderr
        .contains("no sealed passing receipt"));
    // An outcome needs a sentence, and the verifier's token: a builder's token is not enough.
    let o = fx.bf_in(
        &k,
        &[
            "evidence",
            "attach",
            "X-5",
            shot_s,
            "--outcome",
            "met",
            "--token",
            &vt,
        ],
    );
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("one sentence"), "{}", o.stderr);
    let says = "I read the screen text and it says hello";
    for bad in [None, Some(bt.as_str())] {
        let mut a = vec![
            "evidence",
            "attach",
            "X-5",
            shot_s,
            "--outcome",
            "met",
            "--says",
            says,
        ];
        if let Some(t) = bad {
            a.extend(["--token", t]);
        }
        let o = fx.bf_in(&wc, &a);
        assert_eq!(o.code, 2, "{}", o.all());
        assert!(o.stderr.contains("verifier's token"), "{}", o.stderr);
    }
    // `failed` is a mark: the human decides; a later `met` at the same content supersedes it.
    fx.must_in(
        &k,
        &[
            "evidence",
            "attach",
            "X-5",
            shot_s,
            "--outcome",
            "failed",
            "--says",
            "the screen is blank",
            "--token",
            &vt,
        ],
    );
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "decide-check", "{n}");
    // The human says look again: the token is still on the directive.
    assert_eq!(n["directives"][0]["verifier_token"], vt.as_str(), "{n}");
    fx.must_in(
        &k,
        &[
            "evidence",
            "attach",
            "X-5",
            shot_s,
            "--outcome",
            "unreachable",
            "--says",
            "the device is not here",
            "--token",
            &vt,
        ],
    );
    fx.must_in(
        &k,
        &[
            "evidence",
            "attach",
            "X-5",
            shot_s,
            "--outcome",
            "met",
            "--says",
            says,
            "--token",
            &vt,
        ],
    );
    // The pages show the outcome, the sentence and the file.
    let page = fx.must(&["view", "checkpoint", "K-1"]).json().to_string();
    assert!(
        page.contains("met") && page.contains(says) && page.contains("shot.txt"),
        "{page}"
    );
    let cov = fx.must(&["view", "coverage"]).json().to_string();
    assert!(
        cov.contains("Judged by an agent") && cov.contains("the screen is blank"),
        "{cov}"
    );
    fx.must(&["checkpoint", "close", "K-1"]);
}
