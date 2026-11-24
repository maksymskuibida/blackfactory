//! Probes of the K-3 checkpoint review, round 1. Each test asserts the right answer, so a probe that
//! exposes a fault fails until the fault is fixed.

mod common;
use bf::checks::review;
use bf::events;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const CHECKS: [(&str, &str); 5] = [
    ("X-11", "idea-review"),
    ("X-12", "requirements-review"),
    ("X-13", "architecture-review"),
    ("X-14", "plan-review"),
    ("X-15", "amendment-review"),
];

fn put_ok(fx: &Fx, kind: &str, text: &str) {
    let o = fx.put(kind, text);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn setup() -> Fx {
    let fx = Fx::with_checks();
    fx.tiny();
    for (id, name) in CHECKS {
        put_ok(
            &fx,
            "check",
            &format!(
                "id=\"{id}\"\nkind=\"check\"\ntitle=\"{name}\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"{name}\"\n"
            ),
        );
    }
    fx
}

/// What the hooks do for an outcome on a review check: the rules, the sealed receipt and the event.
fn attach(fx: &Fx, check: &str, outcome: &str, amendment: Option<&str>, kind: Option<&str>) {
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    let rec = p.get_record(check).unwrap().unwrap();
    let a = review::on_attach(&p, &st, &rec, outcome, None, amendment, kind)
        .unwrap()
        .expect("a design review check");
    let ty = match outcome {
        "met" => "attach",
        "failed" => "fail",
        _ => "unreachable",
    };
    let mut v = json!({
        "type": ty, "check": check, "reason": "I read every record of the stage",
        "file_hash": "ab12", "file_name": "review.md", "content_hash": "c0",
        "at": "2026-01-01T00:00:00Z", "check_hash": bf::evidence::record_hash(&rec),
    });
    for (k, x) in a.fields().as_object().unwrap() {
        v[k] = x.clone();
    }
    let n = fs::read_dir(p.receipts_dir()).map(|d| d.count()).unwrap_or(0) + 1;
    let id = format!("rc-{n:05}");
    v["id"] = json!(id);
    let key = bf::evidence::read_key(&p).unwrap();
    v["seal"] = json!(bf::evidence::seal(&key, &v));
    fs::create_dir_all(p.receipts_dir()).unwrap();
    fs::write(
        p.receipts_dir().join(format!("{id}.toml")),
        toml::to_string_pretty(&bf::store::strip_nulls(v)).unwrap(),
    )
    .unwrap();
    events::append(&p, "evidence.attach", json!({"receipt": id, "check": check})).unwrap();
}

fn finding(id: &str, gate_or_amend: &str, sev: &str, extra: &str) -> String {
    let which = if gate_or_amend.starts_with("A-") {
        format!("amendment=\"{gate_or_amend}\"")
    } else {
        format!("gate=\"{gate_or_amend}\"")
    };
    format!("id=\"{id}\"\nkind=\"finding\"\ntitle=\"Title of {id}\"\n{which}\nseverity=\"{sev}\"\naction=\"Change it\"\n{extra}")
}

fn gate_lines(fx: &Fx, gate: &str) -> Vec<String> {
    let p = fx.project();
    let st = events::replay(&p).unwrap();
    review::gate_blocker(&p, &st, gate).unwrap()
}

fn not_affected(fx: &Fx, id: &str, stage: &str) {
    put_ok(
        fx,
        "decision",
        &format!("id=\"{id}\"\nkind=\"decision\"\ntitle=\"The {stage} stage is not affected\"\nmaker=\"agent\"\nstage=\"{stage}\"\n"),
    );
}

/// A covered, narrowed amendment A-1 of a design change, ready for the human's words.
fn narrowed() -> Fx {
    let fx = setup();
    put_ok(&fx, "component", "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n");
    not_affected(&fx, "D-91", "ui");
    not_affected(&fx, "D-92", "plan");
    let opened = fx.must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"]).json();
    let keep: Vec<String> = opened["impact_ids"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    let f: &Path = &fx.root.join("narrow.json");
    fs::write(f, json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fx
}

/// p1: a holding finding of a gate cannot be taken off its gate by a plain put with no token.
#[test]
fn probe_p1_a_gate_finding_is_not_detached_by_a_put_without_the_token() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "requirements", "high", ""));
    attach(&fx, "X-12", "failed", None, None);
    fx.set_policy("tokens = false", "tokens = true");
    assert!(gate_lines(&fx, "requirements").iter().any(|l| l.contains("F-1")));
    // The same finding, put again from the main checkout with no token: its gate left out, a unit named.
    let o = fx.put(
        "finding",
        "id=\"F-1\"\nkind=\"finding\"\ntitle=\"Title of F-1\"\nunit=\"U-1\"\nseverity=\"high\"\naction=\"Change it\"\n",
    );
    let held = gate_lines(&fx, "requirements");
    assert!(
        o.code == 2 && held.iter().any(|l| l.contains("F-1")),
        "the put ended with {} and the gate is held by {held:?}",
        o.code
    );
}

/// p1b: nor by moving it to a gate that has no review.
#[test]
fn probe_p1b_a_gate_finding_is_not_moved_to_a_gate_without_a_review() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "requirements", "high", ""));
    attach(&fx, "X-12", "failed", None, None);
    fx.set_policy("tokens = false", "tokens = true");
    let o = fx.put("finding", &finding("F-1", "ui", "high", ""));
    let held = gate_lines(&fx, "requirements");
    assert!(
        o.code == 2 && held.iter().any(|l| l.contains("F-1")),
        "the put ended with {} and the gate is held by {held:?}",
        o.code
    );
}

/// p2: the words that accepted a finding of an amendment do not approve the amendment too.
#[test]
fn probe_p2_words_that_accepted_a_finding_do_not_approve_the_amendment() {
    let fx = narrowed();
    put_ok(&fx, "finding", &finding("F-1", "A-1", "high", ""));
    attach(&fx, "X-15", "failed", Some("A-1"), Some("amendment"));
    fx.human("I accept the first finding of the amendment.");
    fx.must(&["review", "finding", "A-1", "--accept", "F-1", "--quote", "accept the first finding"]);
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "accept the first finding"]);
    assert_eq!(o.code, 2, "one message decided twice: {}", o.all());
}

/// p3: the words that lifted the designer rule do not approve the amendment too.
#[test]
fn probe_p3_words_that_lifted_the_designer_rule_do_not_approve_the_amendment() {
    let fx = narrowed();
    attach(&fx, "X-15", "met", Some("A-1"), Some("amendment"));
    fx.human("Draft this one yourself, no designer needed.");
    fx.must(&["roles", "lift", "A-1", "--quote", "Draft this one yourself"]);
    let o = fx.bf(&["amend", "approve", "A-1", "--quote", "Draft this one yourself"]);
    assert_eq!(o.code, 2, "one message decided twice: {}", o.all());
}

/// p3b: the words that released a parked review do not approve the gate's later decisions; here the
/// same message is taken for a lift after it released a review.
#[test]
fn probe_p3b_words_that_released_a_review_do_not_lift_the_designer_rule() {
    let fx = setup();
    put_ok(&fx, "finding", &finding("F-1", "requirements", "high", ""));
    for _ in 0..3 {
        let p = fx.project();
        let st = events::replay(&p).unwrap();
        let rec = p.get_record("X-12").unwrap().unwrap();
        let a = review::on_attach(&p, &st, &rec, "failed", None, None, None).unwrap().unwrap();
        attach(&fx, "X-12", "failed", None, None);
        if a.park {
            events::append(&p, review::PARK_EVENT, json!({"subject": a.subject, "round": a.round})).unwrap();
        }
    }
    assert!(review::parked(&fx.project(), "requirements").unwrap());
    fx.human("I read it all, start the review over.");
    fx.must(&["review", "release", "requirements", "--quote", "start the review over"]);
    let o = fx.bf(&["roles", "lift", "requirements", "--quote", "start the review over"]);
    assert_eq!(o.code, 2, "one message decided twice: {}", o.all());
}

/// p4: a check's command changed inside a plan repair that the human has not approved does not let
/// its unit land.
#[test]
fn probe_p4_a_check_changed_in_an_unapproved_repair_does_not_land_a_unit() {
    let fx = Fx::tiny_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let f = fx.root.join("x1.toml");
    fs::write(&f, "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n").unwrap();
    let put = fx.bf(&["record", "put", "check", "--file", f.to_str().unwrap(), "--replace"]);
    let _ = fs::remove_file(&f);
    // The unit delivers nothing: src/hello.txt is never written. An in-scope file is touched so there is a change.
    fs::create_dir_all(wc.join("src")).unwrap();
    let run = fx.bf_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "true"]);
    let b = fx.bf(&["report", "U-1", "--result", "built"]);
    let r = fx.bf(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    let land = fx.bf(&["unit", "land", "U-1"]);
    let st = fx.must(&["status", "--json"]).json();
    let open = st["amendments"].as_array().map(|a| a.iter().any(|x| x["status"] != "approved")).unwrap_or(false);
    assert!(
        land.code != 0,
        "put {} run {} built {} reviewed {} land {} with the amendment still unapproved ({open}): {}",
        put.code, run.code, b.code, r.code, land.code, land.all()
    );
}

// ---- the guard

fn judge(fx: &Fx, cwd: &Path, cmd: &str) -> (String, String) {
    let payload = json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": cwd.to_string_lossy()});
    let o = fx.bf_stdin(cwd, &["guard"], &payload.to_string());
    match o.stdout.trim() {
        "" | "{}" => ("pass".into(), String::new()),
        t => {
            let v: Value = serde_json::from_str(t).unwrap();
            let h = &v["hookSpecificOutput"];
            (
                h["permissionDecision"].as_str().unwrap_or("?").to_string(),
                h["permissionDecisionReason"].as_str().unwrap_or("").to_string(),
            )
        }
    }
}

fn guard_project(fx: &Fx) -> PathBuf {
    let recs: &[(&str, &str)] = &[
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ntext=\"Build a.\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"All\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-4\"]\n"),
    ];
    for (k, t) in recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/a.rs"), "fn a() {}\n").unwrap();
    fx.commit_all("records and files");
    fx.start_test_run();
    PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap())
}

/// p5: git itself refuses `--cacheinfo=<mode>,<object>,<path>`, so the guard's reading of the two
/// forms git takes is whole. The probe fails if a git that takes the form appears.
#[test]
fn probe_p5_git_takes_no_cacheinfo_with_an_equals_sign() {
    let tmp = tempfile::TempDir::new().unwrap();
    let scratch = tmp.path().join("g");
    fs::create_dir_all(&scratch).unwrap();
    git(&scratch, &["init", "-q"]);
    fs::write(scratch.join("t"), "/etc/hosts").unwrap();
    let blob = git(&scratch, &["hash-object", "-w", "t"]);
    let o = std::process::Command::new("git")
        .current_dir(&scratch)
        .args(["update-index", "--add", &format!("--cacheinfo=120000,{blob},a.rs")])
        .output()
        .unwrap();
    assert!(!o.status.success(), "git took the form with an equals sign");
}

// ---- the replay of a fix opening (R-94)

fn fixplan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

/// p6: a fix opening that names review findings under a source no honest path writes is ignored.
#[test]
fn probe_p6_a_fix_opening_with_an_unknown_source_naming_review_findings_is_ignored() {
    let fx = Fx::new();
    fixplan(&fx);
    fx.must(&["checks", "propose", "checkpoint-review=on", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it", "--scope", "default"]);
    fx.human("Yes, set the checkpoint review that way.");
    fx.must(&["checks", "set", "--quote", "set the checkpoint review that way"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    fx.bf(&["brief", "K-1", "--role", "reviewer", "--json"]);
    let high = json!({"title": "the answer is wrong", "severity": "high", "where": "src/a.txt:1", "action": "make it right"});
    let o = fx.bf_stdin(
        &fx.root,
        &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"],
        &json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": [high.clone()]}).to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let open = |fx: &Fx| !fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"].is_null();
    assert!(!open(&fx));
    for source in ["other", ""] {
        let ev = json!({
            "checkpoint": "K-1", "source": source, "failed": [], "findings": [high.clone()],
            "finding_ids": [{"round": 1, "index": 1}], "before": [], "content": "forged", "human": null, "wave": 0,
        });
        bf::events::append(&fx.project(), "checkpoint.fix.open", ev).unwrap();
        assert!(!open(&fx), "a forged opening with the source `{source}` opened a fix of a high review finding");
    }
}

/// p7: the conductor runs the design review's commands from the main checkout under the guard.
#[test]
fn probe_p7_the_guard_lets_the_conductor_run_bf_review() {
    let fx = Fx::new();
    let _wc = guard_project(&fx);
    for cmd in [
        "bf review token requirements",
        "bf review brief requirements",
        "bf review token amendment --subject A-1",
        "bf review split A-1 --stage requirements=R-1 --token abc",
        "bf review release requirements --quote \"start over\"",
    ] {
        let (d, r) = judge(&fx, &fx.root, cmd);
        assert_eq!(d, "pass", "{cmd}: {r}");
    }
}
