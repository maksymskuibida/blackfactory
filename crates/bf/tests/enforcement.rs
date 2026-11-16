//! U-3: the engine refuses when a required check has no proof (R-4, R-5, C-3), the threat model
//! (R-12), checkpoints that stop or do not (R-32) and amendments that can be withdrawn (R-35).

mod common;
use common::Fx;
use std::fs;

const SCREEN: &str = "id=\"S-1\"\nkind=\"screen\"\ntitle=\"The greeting page\"\n";
const THREAT: &str = "id=\"TH-1\"\nkind=\"threat\"\ntitle=\"A visitor changes a text\"\nprotects=\"The texts\"\nattack=\"A visitor edits a file by hand\"\nmitigation=\"The texts are read-only files\"\n";

/// Put a record again with `--replace`, the way a planner revises it between gate rounds.
fn revise(fx: &Fx, kind: &str, text: &str) {
    let f = fx.root.join("revise.toml");
    fs::write(&f, text).unwrap();
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

/// The security expert reports on the threat model at the design stage, a second after the last change.
fn expert_reports(fx: &Fx) {
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let o = fx.bf(&[
        "report",
        "design",
        "--result",
        "built",
        "--role",
        "security-expert",
        "--json",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    std::thread::sleep(std::time::Duration::from_millis(50));
}

fn design_records(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("screen", SCREEN),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n"),
        ("decision", "id=\"D-1\"\nkind=\"decision\"\ntitle=\"One module\"\nchosen=\"a\"\nrejected=\"b\"\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
}

/// Switch every named check off except the ones named, the way the human does.
fn only_on(fx: &Fx, keep: &[&str]) {
    let changes: Vec<String> = bf::checks::all()
        .iter()
        .filter(|c| !keep.contains(&c.name))
        .map(|c| format!("{}=off:not needed in this test", c.name))
        .collect();
    let mut a: Vec<&str> = vec!["checks", "propose"];
    a.extend(changes.iter().map(|x| x.as_str()));
    fx.must(&a);
    fx.human("Yes, switch the others off.");
    fx.must(&["checks", "set", "--quote", "switch the others off"]);
}

// ---- R-5: the design gates ----

#[test]
fn the_ui_gate_needs_the_design_stage_verification_of_the_ui() {
    let fx = Fx::with_checks();
    design_records(&fx);
    let o = fx.refused(&["gate", "open", "ui"]);
    assert!(
        o.stderr.contains("design-stage UI/UX verification"),
        "{}",
        o.stderr
    );
    // A check record that names it, with a passing judgment attached.
    let o = fx.put("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"The screens were judged\"\nhow=\"agent\"\nproves=[\"S-1\"]\nscope=\"checkpoint\"\nnamed=\"design-uiux-verification\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    fx.refused(&["gate", "open", "ui"]);
    // A failed judgment is no proof.
    let f = fx.root.join("shot.txt");
    fs::write(&f, "the screen at 375px\n").unwrap();
    fx.must(&[
        "evidence",
        "attach",
        "X-1",
        f.to_str().unwrap(),
        "--outcome",
        "failed",
        "--says",
        "the label is cut off",
    ]);
    fx.refused(&["gate", "open", "ui"]);
    fx.must(&[
        "evidence",
        "attach",
        "X-1",
        f.to_str().unwrap(),
        "--outcome",
        "met",
        "--says",
        "S-1 state default: the default state shows every label",
    ]);
    fx.must(&[
        "evidence",
        "attach",
        "X-1",
        f.to_str().unwrap(),
        "--outcome",
        "met",
        "--says",
        "accessibility: contrast and focus order read at 375px",
    ]);
    fx.must(&[
        "evidence",
        "attach",
        "X-1",
        f.to_str().unwrap(),
        "--outcome",
        "met",
        "--says",
        "I opened the page at 375px and read every label",
    ]);
    fx.must(&["gate", "open", "ui"]);
}

#[test]
fn the_ui_gate_does_not_ask_for_a_check_the_human_switched_off() {
    let fx = Fx::with_checks();
    design_records(&fx);
    fx.refused(&["gate", "open", "ui"]);
    only_on(&fx, &[]);
    fx.must(&["gate", "open", "ui"]);
}

#[test]
fn the_architecture_gate_needs_a_threat_and_a_markets_statement() {
    let fx = Fx::with_checks();
    design_records(&fx);
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(o.stderr.contains("no threat is recorded"), "{}", o.stderr);
    assert!(o.stderr.contains("no markets statement"), "{}", o.stderr);
    // A threat alone is not enough while the legal checks are on.
    let o = fx.put("threat", THREAT);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(!o.stderr.contains("no threat is recorded"), "{}", o.stderr);
    assert!(o.stderr.contains("no markets statement"), "{}", o.stderr);
    // The threat model stands only after the security expert reports on it.
    assert!(o.stderr.contains("has not reported"), "{}", o.stderr);
    expert_reports(&fx);
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(!o.stderr.contains("has not reported"), "{}", o.stderr);
    // A statement of no market counts.
    let o = fx.put(
        "markets",
        "id=\"M-1\"\nkind=\"markets\"\ntitle=\"No market\"\nmarkets=[]\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["gate", "open", "architecture"]);
}

#[test]
fn the_architecture_gate_follows_the_settings_for_the_threat_model_and_the_legal_checks() {
    let fx = Fx::with_checks();
    design_records(&fx);
    // Threat model on, legal checks off: a threat is enough.
    only_on(&fx, &["threat-model"]);
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(o.stderr.contains("no threat is recorded"), "{}", o.stderr);
    assert!(!o.stderr.contains("markets"), "{}", o.stderr);
    fx.put("threat", THREAT);
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(o.stderr.contains("has not reported"), "{}", o.stderr);
    expert_reports(&fx);
    fx.must(&["gate", "open", "architecture"]);
}

#[test]
fn a_change_to_a_threat_needs_a_newer_report_of_the_security_expert() {
    let fx = Fx::with_checks();
    design_records(&fx);
    only_on(&fx, &["threat-model"]);
    fx.put("threat", THREAT);
    expert_reports(&fx);
    // The threat changes after the report: the report is older than the change.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    revise(
        &fx,
        "threat",
        &THREAT.replace("read-only files", "files nobody can write"),
    );
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(o.stderr.contains("has not reported"), "{}", o.stderr);
    expert_reports(&fx);
    fx.must(&["gate", "open", "architecture"]);
}

#[test]
fn the_architecture_gate_follows_the_settings_the_other_way_round_too() {
    let fx = Fx::with_checks();
    design_records(&fx);
    // Legal checks on, threat model off: the markets statement is demanded and a threat is not.
    only_on(&fx, &["markets-statement"]);
    let o = fx.refused(&["gate", "open", "architecture"]);
    assert!(o.stderr.contains("no markets statement"), "{}", o.stderr);
    assert!(!o.stderr.contains("no threat is recorded"), "{}", o.stderr);
    fx.put(
        "markets",
        "id=\"M-1\"\nkind=\"markets\"\ntitle=\"No market\"\nmarkets=[]\n",
    );
    fx.must(&["gate", "open", "architecture"]);
}

// ---- R-12: the threat model ----

#[test]
fn a_threat_names_what_is_protected_how_it_is_attacked_and_how_the_design_answers_it() {
    let fx = Fx::new();
    let o = fx.put(
        "threat",
        "id=\"TH-1\"\nkind=\"threat\"\ntitle=\"A visitor changes a text\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    for f in ["protects", "attack", "mitigation"] {
        assert!(o.stderr.contains(f), "{f}: {}", o.stderr);
    }
    assert_eq!(fx.put("threat", THREAT).code, 0);
}

#[test]
fn the_ledger_refuses_while_a_threat_has_no_check_that_proves_its_answer() {
    let fx = Fx::new();
    fx.put("threat", THREAT);
    let o = fx.refused(&["ledger", "check", "--plan"]);
    assert!(
        o.stderr.contains("TH-1") && o.stderr.contains("no unit, no check"),
        "{}",
        o.stderr
    );
}

#[test]
fn the_architecture_page_shows_the_threat_model() {
    let fx = Fx::new();
    design_records(&fx);
    fx.put("threat", THREAT);
    fx.must(&["gate", "open", "architecture"]);
    let page = fx
        .must(&["view", "gate", "architecture"])
        .json()
        .to_string();
    assert!(
        page.contains("The texts") && page.contains("A visitor edits a file by hand"),
        "{page}"
    );
    assert!(page.contains("The texts are read-only files"), "{page}");
}

// ---- R-4 and C-3: the plan, the checkpoint and the run ----

#[test]
fn the_plan_check_names_each_required_check_that_has_no_check_record() {
    let fx = Fx::with_checks();
    fx.checks_decided_as_default();
    fx.tiny();
    let o = fx.refused(&["plan", "check"]);
    for c in bf::checks::all() {
        // The threat model and the markets statement are proven by their records.
        // The design reviews are proven by the review of their gate, not by a check record either.
        let proven_by_record = c.name == "threat-model"
            || c.name == "markets-statement"
            || c.name.ends_with("-review") && c.name != "checkpoint-review";
        assert_eq!(
            o.stderr.contains(&format!("`{}`", c.name)),
            !proven_by_record,
            "{}: {}",
            c.name,
            o.stderr
        );
    }
    // Naming them all in the plan settles it.
    fx.name_every_check("R-1");
    fx.must(&["plan", "check"]);
    // A check the human switched off is not demanded.
    let fx = Fx::with_checks();
    fx.tiny();
    only_on(&fx, &["secret-scan"]);
    let o = fx.refused(&["plan", "check"]);
    assert!(
        o.stderr.contains("`secret-scan`") && !o.stderr.contains("`accessibility`"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_check_record_may_only_name_a_check_of_the_catalogue() {
    let fx = Fx::with_checks();
    let o = fx.put("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"t\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"no-such-check\"\n");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("not a named check"), "{}", o.stderr);
}

fn built_project(fx: &Fx, checkpoint_extra: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"The file\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"Secret scan\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"secret-scan\"\n".to_string()),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"The file again\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"Accessibility\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"accessibility\"\n".to_string()),
        ("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"K\"\nafter=[\"U-1\"]\nproves=[\"R-1\",\"C-1\"]\nchecks=[\"X-2\",\"X-4\",\"X-5\"]\n{checkpoint_extra}")),
    ] {
        let o = fx.put(k, &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("records");
    only_on(fx, &["secret-scan", "accessibility"]);
    fx.start_test_run();
    fx.build_unit("U-1", "src/a.txt", "X-1", &["test", "-f", "src/a.txt"]);
}

#[test]
fn a_checkpoint_closes_only_with_a_passing_receipt_for_each_required_check_it_lists() {
    let fx = Fx::with_checks();
    built_project(&fx, "");
    let wc = fx.work("K-1");
    fx.must_in(
        &wc,
        &["evidence", "run", "X-4", "--", "test", "-f", "src/a.txt"],
    );
    // The designed parts are proven, but the required secret scan and accessibility have no receipt.
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(
        o.stderr.contains("`secret-scan`") && o.stderr.contains("X-2"),
        "{}",
        o.stderr
    );
    assert!(
        o.stderr.contains("`accessibility`") && o.stderr.contains("X-5"),
        "{}",
        o.stderr
    );
    fx.must_in(&wc, &["evidence", "run", "X-2", "--", "true"]);
    fx.refused(&["checkpoint", "close", "K-1"]);
    fx.must_in(&wc, &["evidence", "run", "X-5", "--", "true"]);
    let c = fx.must(&["checkpoint", "close", "K-1", "--json"]).json();
    assert_eq!(c["closed"], true);
    assert_eq!(c["checks_off"].as_array().unwrap().len(), 13, "{c}");
}

#[test]
fn the_ledger_refuses_to_close_the_run_and_names_each_required_check_with_no_proof() {
    let fx = Fx::with_checks();
    built_project(&fx, "");
    let wc = fx.work("K-1");
    fx.must_in(
        &wc,
        &["evidence", "run", "X-4", "--", "test", "-f", "src/a.txt"],
    );
    fx.must_in(&wc, &["evidence", "run", "X-2", "--", "true"]);
    // Accessibility is off for the run while the checkpoint closes, then the human switches it on.
    fx.must(&[
        "checks",
        "propose",
        "accessibility=off:no screens in this test",
        "--scope",
        "run",
    ]);
    fx.human("Yes, accessibility is off for this run.");
    fx.must(&[
        "checks",
        "set",
        "--quote",
        "accessibility is off for this run",
    ]);
    fx.must(&["checkpoint", "close", "K-1"]);
    fx.must(&["checks", "propose", "accessibility=on", "--scope", "run"]);
    fx.human("Yes, accessibility is on again.");
    fx.must(&["checks", "set", "--quote", "accessibility is on again"]);
    // Accessibility is required, its check record exists and nothing ran it.
    let o = fx.refused(&["ledger", "check"]);
    assert!(
        o.stderr.contains("accessibility") && o.stderr.contains("X-5"),
        "{}",
        o.stderr
    );
    assert!(!o.stderr.contains("secret-scan"), "{}", o.stderr);
    // Stopping the run is refused the same way.
    let o = fx.refused(&["run", "stop"]);
    assert!(o.stderr.contains("accessibility"), "{}", o.stderr);
    // The human switches it off for the run: it is not demanded, and the summary says it was off.
    fx.must(&[
        "checks",
        "propose",
        "accessibility=off:no screens in this test",
        "--scope",
        "run",
    ]);
    fx.human("Yes, accessibility is off for this run.");
    fx.must(&[
        "checks",
        "set",
        "--quote",
        "accessibility is off for this run",
    ]);
    let o = fx.must(&["ledger", "check"]);
    assert!(o.stdout.contains("ledger closed"), "{}", o.stdout);
    assert!(
        o.stdout
            .contains("accessibility was off (no screens in this test)"),
        "{}",
        o.stdout
    );
    let j = fx.must(&["ledger", "check", "--json"]).json();
    assert!(
        j["checks_off"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "accessibility"),
        "{j}"
    );
}

// ---- R-32: a checkpoint stops or has no stop ----

#[test]
fn a_checkpoint_has_two_forms_only_and_carry_on_is_gone() {
    let fx = Fx::new();
    let base = "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"K\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\n";
    let o = fx.put(
        "checkpoint",
        &format!("{base}human_step=\"Read it\"\non_human_step=\"carry-on\"\n"),
    );
    assert_eq!(o.code, 2, "{}", o.all());
    // A stop says why.
    let o = fx.put("checkpoint", &format!("{base}stop=true\n"));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("stop_why"), "{}", o.stderr);
    let o = fx.put(
        "checkpoint",
        &format!("{base}stop=true\nstop_why=\"The human reads the page before it ships.\"\n"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // A checkpoint with no stop says why too.
    let base = base.replace("K-1", "K-2");
    let o = fx.put("checkpoint", &format!("{base}stop=false\n"));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("stop_why"), "{}", o.stderr);
    let o = fx.put(
        "checkpoint",
        &format!("{base}stop=false\nstop_why=\"Nothing here needs the human's eyes.\"\n"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_record_from_an_earlier_build_that_says_carry_on_is_read_as_a_stop() {
    let fx = Fx::new();
    fx.tiny();
    let f = fx.root.join("factory/records/K-1.toml");
    let t = fs::read_to_string(&f).unwrap();
    fs::write(
        &f,
        format!("{t}human_step = \"Read both messages\"\non_human_step = \"carry-on\"\n"),
    )
    .unwrap();
    fx.commit_all("an earlier build's record");
    let page = fx.must(&["view", "plan"]).json().to_string();
    assert!(page.contains("stops and waits for you"), "{page}");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    let wc = fx.work("K-1");
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
    let d = n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["checkpoint"] == "K-1")
        .unwrap()
        .clone();
    assert_eq!(d["on_human_step"], "stop", "{d}");
    assert_eq!(d["action"], "open-gate", "{d}");
    // The closure waits for the human's step.
    fx.refused(&["checkpoint", "close", "K-1"]);
}

#[test]
fn the_plan_page_shows_for_every_checkpoint_whether_a_stop_is_proposed_and_why() {
    let fx = Fx::new();
    fx.tiny();
    let page = fx.must(&["view", "plan"]).json().to_string();
    assert!(page.contains("no stop"), "{page}");
    revise(&fx, "checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\nstop=true\nstop_why=\"The greeting wording is the human's to judge.\"\n");
    let page = fx.must(&["view", "plan"]).json().to_string();
    assert!(
        page.contains("stops and waits for you")
            && page.contains("The greeting wording is the human's to judge."),
        "{page}"
    );
}

#[test]
fn the_human_changes_a_stop_by_rejecting_the_plan_with_their_words_and_the_next_round_shows_it() {
    let fx = Fx::new();
    fx.tiny();
    let g1 = fx.must(&["gate", "open", "plan"]).stdout.trim().to_string();
    fx.human("Please make the checkpoint stop for me, so I can read the messages. Reject.");
    fx.must(&[
        "gate",
        "reject",
        &g1,
        "--note",
        "stop at K-1",
        "--quote",
        "make the checkpoint stop for me",
    ]);
    revise(&fx, "checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\"]\nstop=true\nstop_why=\"You asked to read the messages first.\"\n");
    let g2 = fx.must(&["gate", "open", "plan"]).stdout.trim().to_string();
    assert_ne!(g1, g2);
    let page = fx.must(&["view", "gate", &g2]).json().to_string();
    assert!(
        page.contains("Changed since the last round") && page.contains("stops and waits for you"),
        "{page}"
    );
    // Approving the gate approves the stops it shows.
    fx.human("Good, approved.");
    fx.must(&["gate", "approve", &g2, "--quote", "approved"]);
    assert_eq!(fx.must_state_gate(&g2), "approved");
}

// ---- R-35: amendments ----

fn narrow_all(fx: &Fx, a: &str, opened: &serde_json::Value) {
    let f = fx.root.join("narrow.json");
    // Keep the whole impact set the engine printed when the amendment opened.
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    fs::write(
        &f,
        serde_json::json!({"amend": a, "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

fn amendment_status(fx: &Fx, id: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["amendments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn an_open_amendment_can_be_withdrawn_and_the_log_keeps_both_facts() {
    let fx = Fx::new();
    fx.tiny();
    let o = fx.must(&["amend", "open", "R-2", "--json"]).json();
    assert_eq!(o["amend"], "A-1");
    assert_eq!(amendment_status(&fx, "A-1"), "open");
    fx.must(&[
        "amend",
        "withdraw",
        "A-1",
        "--reason",
        "the requirement is fine as it is",
    ]);
    assert_eq!(amendment_status(&fx, "A-1"), "withdrawn");
    let log = fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    assert!(
        log.contains("\"amend.open\"") && log.contains("\"amend.withdraw\""),
        "{log}"
    );
    assert!(log.contains("the requirement is fine as it is"));
    // A withdrawn amendment can no longer be narrowed, approved or withdrawn again.
    let f = fx.root.join("n.json");
    fs::write(&f, "{\"amend\":\"A-1\",\"keep\":[\"R-2\"],\"waive\":[]}").unwrap();
    fx.refused(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx.human("Yes, approve it.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve it"]);
    assert!(o.stderr.contains("withdrawn"), "{}", o.stderr);
    fx.refused(&["amend", "withdraw", "A-1"]);
    fx.refused(&["amend", "withdraw", "A-9"]);
}

#[test]
fn an_approved_amendment_cannot_be_withdrawn() {
    let fx = Fx::new();
    fx.tiny();
    // U-19: an amendment is approved only when every design stage is covered and its requirement is linked.
    for (k, t) in [
        ("component", "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n"),
        ("decision", "id=\"D-91\"\nkind=\"decision\"\ntitle=\"The screens are not affected: no screen shows the farewell\"\nmaker=\"agent\"\nstage=\"ui\"\n"),
        ("decision", "id=\"D-92\"\nkind=\"decision\"\ntitle=\"The plan is not affected: the farewell unit already exists\"\nmaker=\"agent\"\nstage=\"plan\"\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, "A-1", &opened);
    fx.human("Yes, approve the change to the second requirement.");
    fx.must(&["amend", "approve", "--quote", "approve the change"]);
    assert_eq!(amendment_status(&fx, "A-1"), "approved");
    let o = fx.refused(&["amend", "withdraw", "A-1"]);
    assert!(o.stderr.contains("approved"), "{}", o.stderr);
}

#[test]
fn a_second_amendment_is_refused_while_one_is_open() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    let o = fx.refused(&["amend", "open", "R-1"]);
    assert!(o.stderr.contains("A-1"), "{}", o.stderr);
    assert!(
        o.stderr.contains("revise") && o.stderr.contains("withdraw"),
        "{}",
        o.stderr
    );
    assert_eq!(amendment_status(&fx, "A-1"), "open");
}

#[test]
fn a_legal_records_acceptance_is_reserved_for_the_human() {
    let fx = Fx::new();
    let f = fx.root.join("l.toml");
    std::fs::write(
        &f,
        "id=\"L-1\"\nkind=\"legal\"\ntitle=\"Licence gap\"\noutcome=\"accepted\"\n",
    )
    .unwrap();
    // A new record cannot be created as accepted.
    fx.refused(&["record", "put", "legal", "--file", f.to_str().unwrap()]);
    let open = fx.root.join("l-open.toml");
    std::fs::write(
        &open,
        "id=\"L-1\"\nkind=\"legal\"\ntitle=\"Licence gap\"\noutcome=\"open\"\n",
    )
    .unwrap();
    fx.must(&["record", "put", "legal", "--file", open.to_str().unwrap()]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    // No human message at all: refused.
    fx.refused(&[
        "record",
        "put",
        "legal",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    fx.human("Yes, I accept the licence gap.");
    fx.must(&[
        "record",
        "put",
        "legal",
        "--file",
        f.to_str().unwrap(),
        "--replace",
        "--quote",
        "I accept the licence gap",
    ]);
}
