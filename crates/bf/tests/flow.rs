//! How a run flows: checkpoints that build on each other, questions from workers, amendments that
//! re-open work, and handoffs that are proved by checks that really run.

mod common;
use common::*;
use std::fs;
use std::path::PathBuf;

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

/// R-1, C-1, D-1 (and J-1 when `journey`) delivered by one unit, proved by its check and one checkpoint:
/// a whole plan with no idea file yet.
fn whole_plan(fx: &Fx, journey: bool) {
    let mut del = vec!["R-1", "C-1", "D-1"];
    if journey {
        del.push("J-1");
    }
    let list = del
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(",");
    let mut recs: Vec<(&str, String)> = vec![
        (
            "requirement",
            "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"a\"]\n".into(),
        ),
        (
            "component",
            "id=\"C-1\"\nkind=\"component\"\ntitle=\"Core\"\n".into(),
        ),
        (
            "decision",
            "id=\"D-1\"\nkind=\"decision\"\ntitle=\"d\"\nchosen=\"a\"\nrejected=\"b\"\n".into(),
        ),
    ];
    if journey {
        recs.push((
            "journey",
            "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\n".into(),
        ));
    }
    recs.extend([
        ("unit", format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"A\"\ndelivers=[{list}]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n")),
        ("check", format!("id=\"X-1\"\nkind=\"check\"\ntitle=\"a\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[{list}]\nscope=\"unit\"\nunit=\"U-1\"\n")),
        ("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"K\"\nafter=[\"U-1\"]\nproves=[{list}]\nchecks=[\"X-1\"]\n")),
    ]);
    for (k, t) in recs {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("records");
}

/// Ask `bf next`, and do what its one directive says, asserting that the command it offers runs.
fn walk_the_gates(fx: &Fx, skip_ui: bool) {
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..30 {
        if fx.must(&["status", "--json"]).json()["gates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["kind"] == "plan" && g["status"] == "approved")
        {
            break;
        }
        let n = fx.must(&["next", "--json"]).json();
        let ds = n["directives"].as_array().unwrap();
        assert_eq!(ds.len(), 1, "one directive at a time: {n}");
        let d = &ds[0];
        let kind = d["gate_kind"].as_str().unwrap_or("").to_string();
        match d["action"].as_str().unwrap() {
            "add-records" => {
                // The directive says what is missing; the conductor writes it, or has the ui gate skipped.
                if kind == "idea" {
                    assert!(d["missing"].as_str().unwrap().contains("idea"), "{n}");
                    fs::write(fx.root.join("factory/idea.md"), "# A tiny greeter\n").unwrap();
                    seen.push("add-records idea".into());
                } else {
                    assert_eq!(kind, "ui", "{n}");
                    assert!(skip_ui, "{n}");
                    assert!(
                        d["skip_command"].as_str().unwrap().contains("gate skip ui"),
                        "{n}"
                    );
                    fx.human("There is no interface here, skip the ui gate.");
                    fx.must(&["gate", "skip", "ui", "--quote", "skip the ui gate"]);
                    seen.push("skip ui".into());
                }
            }
            "open-gate" => {
                // The offered command runs without refusal.
                let cmd = d["command"].as_str().unwrap();
                let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
                let o = fx.bf(&args);
                assert_eq!(o.code, 0, "`{cmd}` was offered and refused: {}", o.all());
                seen.push(format!("open {kind}"));
            }
            "decide-gate" => {
                let gid = d["gate"].as_str().unwrap().to_string();
                let said = format!("The {kind} is right, approved.");
                fx.human(&said);
                fx.must(&["gate", "approve", &gid, "--quote", &said]);
                seen.push(format!("decide {kind}"));
            }
            a => panic!("unexpected directive {a}: {n}"),
        }
    }
    let want: Vec<&str> = if skip_ui {
        vec![
            "add-records idea",
            "open idea",
            "decide idea",
            "open requirements",
            "decide requirements",
            "skip ui",
            "open architecture",
            "decide architecture",
            "open plan",
            "decide plan",
        ]
    } else {
        vec![
            "add-records idea",
            "open idea",
            "decide idea",
            "open requirements",
            "decide requirements",
            "open ui",
            "decide ui",
            "open architecture",
            "decide architecture",
            "open plan",
            "decide plan",
        ]
    };
    assert_eq!(seen, want);
}

#[test]
fn bf_next_walks_the_five_gates_in_order_and_every_offered_command_runs() {
    // A project folder, the ui gate skipped.
    let fx = Fx::new();
    ordered(&fx);
    whole_plan(&fx, false);
    fx.must(&["run", "start"]);
    walk_the_gates(&fx, true);
    // A workspace, the ui gate decided.
    let fx = Fx::workspace(&["app"]);
    ordered(&fx);
    whole_plan(&fx, true);
    fx.must(&["run", "start"]);
    walk_the_gates(&fx, false);
}

#[test]
fn bf_next_never_offers_a_later_gate_while_an_earlier_one_waits() {
    let fx = Fx::new();
    ordered(&fx);
    whole_plan(&fx, false);
    fs::write(fx.root.join("factory/idea.md"), "# idea\n").unwrap();
    fx.must(&["run", "start"]);
    fx.must(&["gate", "open", "idea"]);
    let n = fx.must(&["next", "--json"]).json();
    let d = &n["directives"][0];
    assert_eq!(d["action"], "decide-gate", "{n}");
    assert_eq!(d["gate"], "idea-r1");
    assert!(!n.to_string().contains("bf gate open plan"), "{n}");
}

fn ask_file(fx: &Fx, name: &str, body: serde_json::Value) -> PathBuf {
    let f = fx.root.join(".bf").join(name);
    fs::create_dir_all(fx.root.join(".bf")).unwrap();
    fs::write(&f, body.to_string()).unwrap();
    f
}

fn answer_file(fx: &Fx, body: serde_json::Value) -> PathBuf {
    ask_file(fx, "answer.json", body)
}

#[test]
fn every_choice_question_takes_an_other_answer_in_the_humans_words() {
    let fx = Fx::new();
    let q = ask_file(
        &fx,
        "q.json",
        serde_json::json!([
            {"title": "Which store?", "kind": "single", "stage": "architecture", "options": [{"key": "A", "text": "Files"}, {"key": "B", "text": "A database"}]},
            {"title": "Which extras?", "kind": "multi", "stage": "architecture", "options": [{"key": "A", "text": "Cache"}, {"key": "B", "text": "Search"}]},
            {"title": "Closed list", "kind": "single", "other": false, "stage": "architecture", "options": [{"key": "A", "text": "Yes"}]},
            {"title": "Say it", "kind": "text", "stage": "architecture"}
        ]),
    );
    fx.must(&["ask", "--file", q.to_str().unwrap()]);
    // The view says which questions take Other: choice questions by default, not text, not when switched off.
    let v = fx.must(&["view", "questions"]).json();
    let qs = v["questions"].as_array().unwrap();
    assert_eq!(qs[0]["other"], true);
    assert_eq!(qs[1]["other"], true);
    assert_eq!(qs[2]["other"], false);
    assert!(qs[3].get("other").is_none(), "{}", qs[3]);
    // `other` is reserved: a question whose own option uses it is refused.
    let bad = ask_file(
        &fx,
        "bad.json",
        serde_json::json!([{"title": "x", "kind": "single", "stage": "ui", "options": [{"key": "other", "text": "Something else"}]}]),
    );
    let o = fx.refused(&["ask", "--file", bad.to_str().unwrap()]);
    assert_four_parts(&o.stderr);
    assert!(o.stderr.contains("reserved"), "{}", o.stderr);
    fx.human("I filled in the form.");
    // Other chosen without text is refused; with text it is kept.
    let a = answer_file(&fx, serde_json::json!({"choice": "other"}));
    let o = fx.refused(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    assert_four_parts(&o.stderr);
    assert!(o.stderr.contains("no text"), "{}", o.stderr);
    let a = answer_file(&fx, serde_json::json!({"choice": "other", "other": "   "}));
    fx.refused(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    let a = answer_file(
        &fx,
        serde_json::json!({"choice": "other", "other": " Plain text on a USB stick ", "note": "for now"}),
    );
    let r = fx
        .must(&["answer", "Q-1", "--file", a.to_str().unwrap(), "--json"])
        .json();
    assert_eq!(r["answer"]["choice"], "other");
    assert_eq!(r["answer"]["other"], "Plain text on a USB stick");
    // A multi question: the choices and the text.
    let a = answer_file(&fx, serde_json::json!({"choices": ["A", "other"]}));
    fx.refused(&["answer", "Q-2", "--file", a.to_str().unwrap()]);
    let a = answer_file(
        &fx,
        serde_json::json!({"choices": ["A", "other"], "other": "A mirror"}),
    );
    fx.must(&["answer", "Q-2", "--file", a.to_str().unwrap()]);
    // Text without the key chosen is kept as given and does not count as the choice.
    let a = answer_file(
        &fx,
        serde_json::json!({"choice": "A", "other": "never mind"}),
    );
    let r = fx
        .must(&["answer", "Q-3", "--file", a.to_str().unwrap(), "--json"])
        .json();
    assert_eq!(r["answer"]["other"], "never mind");
    assert_eq!(r["answer"]["choice"], "A");
    // Other is switched off for Q-3: it cannot be chosen there (a fresh question, since Q-3 is answered).
    let off = ask_file(
        &fx,
        "off.json",
        serde_json::json!([{"title": "Closed again", "kind": "single", "other": false, "stage": "ui", "options": [{"key": "A", "text": "Yes"}]}]),
    );
    let id = fx
        .must(&["ask", "--file", off.to_str().unwrap(), "--json"])
        .json()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    fx.human("And this one too.");
    let a = answer_file(&fx, serde_json::json!({"choice": "other", "other": "x"}));
    let o = fx.refused(&["answer", &id, "--file", a.to_str().unwrap()]);
    assert!(o.stderr.contains("does not take an Other"), "{}", o.stderr);
    // The human's text is what every view and listing shows, never the bare word.
    let listing = fx.must(&["questions"]).stdout;
    assert!(
        listing.contains("Plain text on a USB stick") && !listing.contains("answered: other"),
        "{listing}"
    );
    let v = fx.must(&["view", "questions"]).json();
    assert_eq!(
        v["questions"][0]["answer"]["other"],
        "Plain text on a USB stick"
    );
    assert_eq!(v["questions"][1]["answer"]["other"], "A mirror");
    // An answer from an earlier build (no `other`) reads as before, and the view gives it an empty one.
    assert_eq!(v["questions"][2]["answer"]["choice"], "A");
}

#[test]
fn a_unit_brief_quotes_the_text_of_an_other_answer() {
    let fx = Fx::new();
    two_units_in_one_checkpoint(&fx);
    fx.start_test_run();
    let wc = std::path::PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fx.must_in(
        &wc,
        &[
            "ask",
            "--title",
            "Which greeting?",
            "--options",
            "A=Hello;B=Hi",
        ],
    );
    fx.human("Answered.");
    let a = answer_file(
        &fx,
        serde_json::json!({"choice": "other", "other": "Good morning"}),
    );
    fx.must(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    let brief = fs::read_to_string(
        fx.must(&["brief", "U-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        brief.contains("answered: Good morning") && !brief.contains("answered: other"),
        "{brief}"
    );
}

#[test]
fn a_run_with_no_recorded_human_message_says_so_and_a_normal_run_does_not() {
    let fx = Fx::new();
    // A conductor at work (questions asked), and not one message of the human recorded.
    let q = ask_file(
        &fx,
        "q.json",
        serde_json::json!([{"title": "Which store?", "kind": "text", "stage": "architecture"}]),
    );
    let o = fx.must(&["ask", "--file", q.to_str().unwrap()]);
    assert!(
        o.stderr
            .contains("note: no message of the human has been recorded"),
        "{}",
        o.stderr
    );
    assert!(
        o.stderr.contains("factory/") && o.stderr.contains("settings.json"),
        "{}",
        o.stderr
    );
    let st = fx.must(&["status", "--json"]).json();
    assert!(
        st["human_turns_note"]
            .as_str()
            .unwrap()
            .contains("Start Claude Code"),
        "{st}"
    );
    assert!(fx
        .must(&["status"])
        .stdout
        .contains("note: no message of the human"));
    let d = fx.bf(&["doctor"]);
    assert!(
        d.stdout
            .contains("human turns: ok (note: no message of the human"),
        "{}",
        d.stdout
    );
    // The refusal at a decision points at the same cause first.
    fx.must(&["gate", "open", "idea"]);
    let o = fx.refused(&["gate", "approve", "idea", "--quote", "Approved."]);
    assert!(
        o.stderr.contains("no human turn was recorded at all"),
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("Start Claude Code"), "{}", o.stderr);
    // Once a message is recorded, nothing is printed.
    fx.human("Hello, I am here.");
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["human_turns_note"], "");
    assert!(!fx
        .must(&["status"])
        .stdout
        .contains("no message of the human"));
    let o = fx.must(&["ask", "--file", q.to_str().unwrap()]);
    assert!(
        !o.stderr.contains("no message of the human"),
        "{}",
        o.stderr
    );
    assert!(fx
        .bf(&["doctor"])
        .stdout
        .contains("human turns: ok (1 message(s)"));
}

#[test]
fn bf_next_carries_the_note_in_its_json_and_on_stderr() {
    let fx = Fx::new();
    whole_plan(&fx, false);
    fx.must(&["run", "start"]);
    let o = fx.must(&["next", "--json"]);
    assert!(
        o.stderr.contains("note: no message of the human"),
        "{}",
        o.stderr
    );
    assert!(o.json()["human_turns_note"]
        .as_str()
        .unwrap()
        .contains("hook"));
    fx.human("Here.");
    let o = fx.must(&["next", "--json"]);
    assert!(
        !o.stderr.contains("no message of the human"),
        "{}",
        o.stderr
    );
    assert_eq!(o.json()["human_turns_note"], "");
}
