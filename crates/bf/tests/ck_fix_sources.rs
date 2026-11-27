//! U-135 (R-78, D-160, TH-21): the fix's builder and reviewer see every finding the fix was opened for with
//! its source, and a fix review ends none of them.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit, one checkpoint listing a command check, the review (X-3), a security audit (X-4), a legal audit
/// (X-5) and the `extra` agent checks `(id, named)`.
fn project(fx: &Fx, extra: &[(&str, &str)]) {
    let mut ids = vec!["X-2", "X-3", "X-4", "X-5"]
        .into_iter()
        .map(|x| format!("\"{x}\""))
        .collect::<Vec<_>>();
    let mut recs: Vec<(&str, String)> = vec![
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n".to_string()),
        ("journey", "id=\"J-1\"\nkind=\"journey\"\ntitle=\"Say hello\"\nsteps=[\"open\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\",\"J-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\n".to_string()),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".to_string()),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"security\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"security-audit\"\n".to_string()),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"legal\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"legal-audit\"\n".to_string()),
    ];
    for (id, named) in extra {
        ids.push(format!("\"{id}\""));
        recs.push(("check", format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"{named}\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"{named}\"\n")));
    }
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\",\"J-1\"]\nchecks=[{}]\n", ids.join(","))));
    for (k, t) in recs {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on",
        "security-audit=on",
        "idea-review=off:the test does not exercise it",
        "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it",
        "plan-review=off:the test does not exercise it",
        "--scope",
        "default",
    ]);
    fx.human("Yes, the checkpoint review is on.");
    fx.must(&["checks", "set", "--quote", "the checkpoint review is on"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
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
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}


fn areas() -> Value {
    json!([{"area": "the close rule", "how": "read"}])
}

fn finding(title: &str, severity: &str) -> Value {
    json!({"title": title, "severity": severity, "where": "src/a.txt:1", "action": "change it"})
}

fn audit_brief(fx: &Fx, role: &str, check: &str) -> Out {
    fx.bf(&["brief", "K-1", "--role", role, "--check", check, "--json"])
}

fn audit_report(fx: &Fx, role: &str, check: &str, body: Value, token: Option<&str>) -> Out {
    let mut args = vec![
        "report",
        "K-1",
        "--role",
        role,
        "--check",
        check,
        "--result",
        "findings",
        "--findings",
        "-",
        "--json",
    ];
    if let Some(t) = token {
        args.extend(["--token", t]);
    }
    fx.bf_stdin(&fx.root, &args, &body.to_string())
}


fn rounds(fx: &Fx) -> Vec<Value> {
    fx.must(&["status", "--json"]).json()["checkpoint_reviews"][0]["rounds"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}


fn review_brief(fx: &Fx) -> String {
    let o = fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

/// The review's report on an open round: its copy runs the checkpoint's command check, then it reports `body`.
fn review_report(fx: &Fx, body: Value) -> Out {
    // A repeat of the run on the same content is refused and settles nothing: the first run is the receipt.
    fx.bf_in(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
    );
    fx.bf_stdin(
        &fx.root,
        &[
            "report",
            "K-1",
            "--role",
            "reviewer",
            "--result",
            "findings",
            "--findings",
            "-",
        ],
        &body.to_string(),
    )
}

fn outcome_of(fx: &Fx, round: u64, id: &str) -> Value {
    let r = rounds(fx)
        .into_iter()
        .find(|r| r["round"] == round)
        .unwrap();
    r["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("no outcome {id}: {r}"))
}


/// The review files a low finding (round 1), the security audit a high one (round 2); one fix takes both.
fn open_fix_on_both(fx: &Fx) {
    project(fx, &[]);
    review_brief(fx);
    let o = review_report(
        fx,
        json!({"areas": areas(), "findings": [finding("a small thing", "low")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(audit_brief(fx, "security-auditor", "X-4").code, 0);
    let o = audit_report(
        fx,
        "security-auditor",
        "X-4",
        json!({"areas": areas(), "findings": [finding("leaks a secret", "high")]}),
        None,
    );
    assert_eq!(o.code, 0, "{}", o.all());
    fx.human("Fix both of them.");
    fx.must(&[
        "checkpoint",
        "finding",
        "K-1",
        "--fix-findings",
        "r1.1,r2.1",
        "--quote",
        "Fix both of them.",
    ]);
}

fn fix_brief(fx: &Fx, role: &str) -> String {
    let o = fx.must(&["brief", "K-1", "--role", role, "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

fn built(fx: &Fx) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "changed by the fix\n").unwrap();
    common::git(&k, &["add", "-A"]);
    common::git(&k, &["commit", "-q", "-m", "fix"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
}

#[test]
fn the_builders_brief_lists_an_audit_finding_with_its_source() {
    let fx = Fx::new();
    open_fix_on_both(&fx);
    let md = fix_brief(&fx, "builder");
    assert!(md.contains("Findings this fix answers"), "{md}");
    let audit = md.lines().find(|l| l.contains("r2.1")).expect(&md);
    assert!(
        audit.contains("the audit X-4")
            && audit.contains("high")
            && audit.contains("leaks a secret")
            && audit.contains("src/a.txt:1")
            && audit.contains("change it"),
        "{audit}"
    );
    let review = md.lines().find(|l| l.contains("r1.1")).expect(&md);
    assert!(review.contains("the checkpoint review"), "{review}");
    let d = fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == "fix-checkpoint")
        .cloned()
        .expect("a fix directive");
    let listed = d["findings"].as_array().unwrap();
    let x = listed.iter().find(|x| x["id"] == "r2.1").expect("r2.1");
    assert_eq!(x["source"], "the audit X-4", "{x}");
}

#[test]
fn the_fix_reviewers_brief_asks_to_judge_each_finding_and_ends_none() {
    let fx = Fx::new();
    open_fix_on_both(&fx);
    built(&fx);
    let md = fix_brief(&fx, "reviewer");
    for needle in ["r1.1", "r2.1", "the audit X-4", "the checkpoint review", "leaks a secret"] {
        assert!(md.contains(needle), "{needle}: {md}");
    }
    assert!(md.contains("Judge for each finding below"), "{md}");
    assert!(md.contains("file a finding of your own"), "{md}");
    assert!(md.contains("You end none of them"), "{md}");
}

#[test]
fn a_fix_review_report_with_confirmed_is_refused() {
    let fx = Fx::new();
    open_fix_on_both(&fx);
    built(&fx);
    let k = fx.work("K-1");
    for body in [
        json!({"confirmed": ["r2.1"]}),
        json!({"still_open": [{"id": "r2.1", "why": "x"}]}),
        json!({"fixed": ["r1.1"]}),
    ] {
        let o = fx.bf_stdin(
            &k,
            &["report", "K-1", "--role", "reviewer", "--result", "built", "--findings", "-"],
            &body.to_string(),
        );
        assert_ne!(o.code, 0, "{body}: {}", o.all());
    }
    let bare = json!([{"title": "x", "severity": "low", "confirmed": ["r2.1"]}]);
    let o = fx.bf_stdin(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built", "--findings", "-"],
        &bare.to_string(),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    let o = fx.bf_stdin(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "built", "--findings", "-"],
        &json!({"confirmed": ["r2.1"]}).to_string(),
    );
    assert!(o.all().contains("confirmed"), "{}", o.all());
    assert!(o.all().contains("next round"), "{}", o.all());
    assert_eq!(outcome_of(&fx, 2, "r2.1")["state"], "open");
}

#[test]
fn a_fix_review_files_its_own_finding_for_an_audit_finding_not_answered_and_the_fix_goes_back() {
    let fx = Fx::new();
    open_fix_on_both(&fx);
    built(&fx);
    let k = fx.work("K-1");
    let o = fx.bf_stdin(
        &k,
        &["report", "K-1", "--role", "reviewer", "--result", "findings", "--findings", "-"],
        &json!({"findings": [{"title": "the secret still leaks (r2.1 not answered)",
            "severity": "high", "where": "src/a.txt:1", "action": "remove it"}]})
        .to_string(),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["checkpoint_fix"]["open"]["phase"], "fix", "{st}");
    assert_eq!(
        st["checkpoint_fix"]["open"]["review_rounds"][0]["result"],
        "findings",
        "{st}"
    );
    let d = fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == "fix-checkpoint")
        .cloned()
        .expect("a fix directive");
    let listed = d["findings"].as_array().unwrap();
    assert!(listed.iter().any(|x| x["id"] == "r2.1"), "{d}");
    assert!(
        listed
            .iter()
            .any(|x| x["id"] == "f1.1" && x["title"].as_str().unwrap().contains("still leaks")),
        "{d}"
    );
    let md = fix_brief(&fx, "builder");
    assert!(md.contains("still leaks"), "{md}");
    assert!(md.contains("r2.1"), "{md}");
    assert_eq!(outcome_of(&fx, 2, "r2.1")["state"], "open");
}
