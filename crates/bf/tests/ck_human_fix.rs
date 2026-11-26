//! U-142 (R-81, D-169, D-170, TH-22): a human finding filed with a fix request gets its fix at the first point
//! the checkpoint's order and the engine's holds allow: at once when nothing holds it, after the open round
//! reports, added to a fix that waits, or at the next round of a fix at work.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
enum Second {
    No,
    /// A second unit of K-1 that is not started.
    Same,
    /// A second checkpoint K-2 with its own landed unit U-2.
    Other,
}

/// One unit landed, one checkpoint listing a command check (X-2) and the review (X-3); with `audits` the
/// security audit (X-4) and the legal audit (X-5) too; the run started.
fn project_with(fx: &Fx, audits: bool, second: Second) {
    let mut ids = vec!["X-2", "X-3"];
    if audits {
        ids.extend(["X-4", "X-5"]);
    }
    let ids = ids
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
    ];
    if audits {
        recs.push(("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"security\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"security-audit\"\n".to_string()));
        recs.push(("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"legal\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"legal-audit\"\n".to_string()));
    }
    let mut after = "\"U-1\"".to_string();
    if second != Second::No {
        let k2 = if second == Second::Same { "K-1" } else { "K-2" };
        recs.push(("unit", format!("id=\"U-2\"\nkind=\"unit\"\ntitle=\"u2\"\ndelivers=[\"R-1\",\"J-1\"]\nfiles=[\"src/b.txt\", \"tests/b_test.txt\"]\ncheckpoint=\"{k2}\"\n")));
        recs.push(("check", "id=\"X-6\"\nkind=\"check\"\ntitle=\"c6\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n".to_string()));
    }
    if second == Second::Same {
        after = "\"U-1\",\"U-2\"".to_string();
    }
    if second == Second::Other {
        recs.push(("check", "id=\"X-7\"\nkind=\"check\"\ntitle=\"c7\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\n".to_string()));
        recs.push(("check", "id=\"X-8\"\nkind=\"check\"\ntitle=\"review2\"\nhow=\"agent\"\nproves=[\"R-1\",\"J-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n".to_string()));
        recs.push(("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k2\"\nafter=[\"U-2\"]\nproves=[\"R-1\",\"J-1\"]\nchecks=[\"X-7\",\"X-8\"]\n".to_string()));
    }
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[{after}]\nproves=[\"R-1\",\"J-1\"]\nchecks=[{}]\n", ids.join(","))));
    for (k, t) in recs {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
    let mut propose = vec![
        "checks",
        "propose",
        "checkpoint-review=on",
        "idea-review=off:the test does not exercise it",
        "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it",
        "plan-review=off:the test does not exercise it",
    ];
    if audits {
        propose.push("security-audit=on");
    }
    propose.extend(["--scope", "default"]);
    fx.must(&propose);
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

fn project(fx: &Fx) {
    project_with(fx, false, Second::No);
}

fn areas() -> Value {
    json!([{"area": "the close rule", "how": "read"}])
}

fn finding(title: &str, severity: &str) -> Value {
    json!({"title": title, "severity": severity, "where": "src/a.txt:1", "action": "change it"})
}

fn review_brief_of(fx: &Fx, k: &str) -> String {
    let o = fx.must(&["brief", k, "--role", "reviewer", "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

fn review_brief(fx: &Fx) -> String {
    review_brief_of(fx, "K-1")
}

fn review_report_of(fx: &Fx, k: &str, check: &str, file: &str, body: Value) -> Out {
    fx.bf_in(
        &fx.work(&format!("{k}-review")),
        &["evidence", "run", check, "--", "test", "-f", file],
    );
    fx.bf_stdin(
        &fx.root,
        &[
            "report",
            k,
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

fn review_report(fx: &Fx, body: Value) -> Out {
    review_report_of(fx, "K-1", "X-2", "src/a.txt", body)
}

/// The review runs and files `findings`.
fn review_with(fx: &Fx, findings: Vec<Value>) {
    review_brief(fx);
    let o = review_report(fx, json!({"areas": areas(), "findings": findings}));
    assert_eq!(o.code, 0, "{}", o.all());
}

/// The human writes `said`; the conductor files `quote` of it as a finding and asks for a fix with `ask`.
fn file_fix(fx: &Fx, said: &str, quote: &str, ask: &str) -> Out {
    fx.human(said);
    fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--human",
        "--quote",
        quote,
        "--fix",
        "--fix-quote",
        ask,
        "--json",
    ])
}

fn filed(fx: &Fx, quote: &str) {
    let said = format!("{quote} Please fix it.");
    let o = file_fix(fx, &said, quote, "Please fix it.");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn events_of(fx: &Fx, kind: &str) -> Vec<Value> {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == kind)
        .collect()
}

fn state(fx: &Fx) -> bf::events::State {
    bf::events::replay(&fx.project()).unwrap()
}

fn wait(fx: &Fx) -> Vec<(u32, u32)> {
    state(fx)
        .ck_human_wait
        .get("K-1")
        .cloned()
        .unwrap_or_default()
}

fn ignored(fx: &Fx) -> Vec<String> {
    state(fx).ignored
}

fn fix_json(fx: &Fx) -> Value {
    fx.must(&["status", "--json"]).json()["checkpoint_fix"]["open"].clone()
}

fn phase(fx: &Fx) -> Value {
    fix_json(fx)["phase"].clone()
}

fn fix_of_k1(fx: &Fx) -> bf::events::CkFixOpen {
    state(fx).ck_fix.expect("a fix is open")
}

fn finding_state(fx: &Fx, id: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["checkpoint_reviews"][0]["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("no finding {id}: {st}"))["state"]
        .as_str()
        .unwrap()
        .to_string()
}

fn why(fx: &Fx) -> Option<&'static str> {
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    let pl = bf::plan::load(&p).unwrap();
    bf::ckhuman::waits_for(&p, &st, &pl, "K-1").unwrap()
}

/// The conductor's builder changes `src/a.txt` to `text`, commits it and reports built.
fn built_with(fx: &Fx, text: &str) {
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), text).unwrap();
    common::git(&k, &["add", "-A"]);
    common::git(&k, &["commit", "-q", "-m", "fix"]);
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
}

fn fix_review(fx: &Fx, findings: Vec<Value>) -> Out {
    fx.bf_stdin(
        &fx.work("K-1"),
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
        &json!({ "findings": findings }).to_string(),
    )
}

fn fix_review_low(fx: &Fx) -> Out {
    fix_review(
        fx,
        vec![finding("low one", "low"), finding("low two", "low")],
    )
}

fn fix_clean(fx: &Fx) -> Out {
    fx.bf_in(
        &fx.work("K-1"),
        &["report", "K-1", "--role", "reviewer", "--result", "built"],
    )
}

fn fix_brief(fx: &Fx, role: &str) -> String {
    let o = fx.must(&["brief", "K-1", "--role", role, "--json"]);
    fs::read_to_string(o.json()["path"].as_str().unwrap()).unwrap()
}

fn next_json(fx: &Fx) -> Value {
    fx.must(&["next", "--json"]).json()
}

fn directive(fx: &Fx, action: &str) -> Option<Value> {
    next_json(fx)["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["action"] == action)
        .cloned()
}

/// The checkpoint review files a high finding (r1.1): the report opens the fix for it, at work.
fn open_fix(fx: &Fx) {
    review_with(fx, vec![finding("the big problem", "high")]);
    assert_eq!(phase(fx), "fix");
}

/// A fix waiting on two low findings of its own review (f1.1 and f1.2).
fn waiting_on_low(fx: &Fx) {
    project(fx);
    open_fix(fx);
    built_with(fx, "changed by the fix\n");
    let o = fix_review_low(fx);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(fx), "low");
}

#[test]
fn a_human_finding_with_a_fix_before_the_review_opens_a_fix_at_once() {
    let fx = Fx::new();
    project(&fx);
    filed(&fx, "The footer is wrong.");
    let f = fix_of_k1(&fx);
    assert_eq!(f.phase, "fix");
    assert_eq!(f.origin_ids, vec![(1, 1)]);
    assert!(wait(&fx).is_empty());
    let opens = events_of(&fx, "checkpoint.fix.open");
    assert_eq!(opens.len(), 1, "{opens:?}");
    assert_eq!(opens[0]["opened_by"], "human");
    assert_eq!(opens[0]["finding_ids"], json!([{"round": 1, "index": 1}]));
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
    // The builder's directive carries the finding with the source human and the human's words.
    let d = directive(&fx, "fix-checkpoint").expect("a fix directive");
    assert_eq!(d["findings"][0]["source"], "human", "{d}");
    assert_eq!(d["findings"][0]["action"], "The footer is wrong.", "{d}");
    assert_eq!(d["findings"][0]["id"], "r1.1", "{d}");
}

#[test]
fn a_fix_for_a_human_finding_opens_while_an_audit_is_due_but_not_open() {
    let fx = Fx::new();
    project_with(&fx, true, Second::No);
    review_with(&fx, vec![]);
    let d = next_json(&fx);
    assert!(
        d["directives"].to_string().contains("audit-checkpoint"),
        "{d}"
    );
    assert!(state(&fx).ck_fix.is_none());
    filed(&fx, "The footer is wrong.");
    assert_eq!(fix_of_k1(&fx).phase, "fix");
    assert!(wait(&fx).is_empty());
    assert_eq!(
        events_of(&fx, "checkpoint.fix.open")[0]["opened_by"],
        "human"
    );
}

#[test]
fn a_human_finding_filed_while_the_review_round_is_open_waits_and_joins_the_one_fix() {
    let fx = Fx::new();
    project(&fx);
    review_brief(&fx);
    filed(&fx, "The footer is wrong.");
    // The round is open: no fix opens, `bf next` opens none, the finding waits.
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(wait(&fx), vec![(2, 1)]);
    assert_eq!(why(&fx), Some("round"));
    next_json(&fx);
    assert!(state(&fx).ck_fix.is_none());
    // The round reports: one fix opens holding the review's finding and the human's.
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [finding("the big problem", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let opens = events_of(&fx, "checkpoint.fix.open");
    assert_eq!(opens.len(), 1, "{opens:?}");
    assert_eq!(opens[0]["opened_by"], "report");
    let f = fix_of_k1(&fx);
    assert_eq!(f.origin_ids, vec![(1, 1), (2, 1)]);
    assert!(wait(&fx).is_empty());
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_human_finding_waits_for_an_open_sync_and_opens_its_fix_when_the_sync_ends() {
    let fx = Fx::new();
    project(&fx);
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/a.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "src/a.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    let o = fx.must(&[
        "checkpoint",
        "sync",
        "K-1",
        "--local",
        "--quote",
        "bring main in",
        "--json",
    ]);
    assert_eq!(o.json()["sync_open"], true, "{}", o.all());
    filed(&fx, "The footer is wrong.");
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(wait(&fx), vec![(1, 1)]);
    assert_eq!(why(&fx), Some("sync"));
    next_json(&fx);
    assert!(state(&fx).ck_fix.is_none());
    fx.must(&["checkpoint", "sync", "K-1", "--abort"]);
    next_json(&fx);
    assert_eq!(fix_of_k1(&fx).phase, "fix");
    assert!(wait(&fx).is_empty());
}

#[test]
fn a_human_finding_waits_for_a_fix_of_another_checkpoint_and_opens_its_fix_when_that_one_ends() {
    let fx = Fx::new();
    project_with(&fx, false, Second::Other);
    // A fix of K-2 is open (K-2's units cannot land before K-1 closes, so the opening is written as the
    // replay reads a fix of a failed check).
    bf::events::append(
        &fx.project(),
        "checkpoint.fix.open",
        json!({"checkpoint": "K-2", "source": "check",
               "failed": [{"check": "X-7", "command": "test -f src/b.txt", "exit": 1, "tail": ""}],
               "findings": [], "human": null, "finding_ids": [], "allowed": ["src/b.txt"],
               "test_dirs": [], "units": ["U-2"], "before": [], "content": "k2", "wave": 0,
               "from_saved": "", "confirm": true}),
    )
    .unwrap();
    assert_eq!(fix_of_k1(&fx).checkpoint, "K-2");
    filed(&fx, "The footer is wrong.");
    assert_eq!(fix_of_k1(&fx).checkpoint, "K-2");
    assert_eq!(wait(&fx), vec![(1, 1)]);
    assert_eq!(why(&fx), Some("other_fix"));
    bf::events::append(
        &fx.project(),
        "checkpoint.fix.abort",
        json!({"checkpoint": "K-2", "saved": ""}),
    )
    .unwrap();
    next_json(&fx);
    let f = fix_of_k1(&fx);
    assert_eq!(f.checkpoint, "K-1");
    assert_eq!(f.origin_ids, vec![(1, 1)]);
    assert!(wait(&fx).is_empty());
}

#[test]
fn a_human_finding_waits_for_a_unit_that_has_not_landed_and_opens_its_fix_at_the_landing() {
    let fx = Fx::new();
    project_with(&fx, false, Second::Same);
    filed(&fx, "The footer is wrong.");
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(wait(&fx), vec![(1, 1)]);
    assert_eq!(why(&fx), Some("units"));
    fx.build_unit("U-2", "src/b.txt", "X-6", &["test", "-f", "src/b.txt"]);
    next_json(&fx);
    let f = fix_of_k1(&fx);
    assert_eq!(f.origin_ids, vec![(1, 1)]);
    assert!(wait(&fx).is_empty());
}

#[test]
fn a_human_finding_with_a_fix_at_the_budget_cap_opens_no_fix_until_the_cap_is_lifted() {
    let fx = Fx::new();
    project(&fx);
    bf::events::append(
        &fx.project(),
        "unit.role-report",
        json!({"unit": "U-1", "role": "auditor", "cost": 2.0}),
    )
    .unwrap();
    fx.set_policy("cap = 0.0", "cap = 1.0");
    filed(&fx, "The footer is wrong.");
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(wait(&fx), vec![(1, 1)]);
    assert_eq!(why(&fx), Some("budget"));
    let o = fx.bf(&["next"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(state(&fx).ck_fix.is_none());
    fx.set_policy("cap = 1.0", "cap = 0.0");
    next_json(&fx);
    assert_eq!(fix_of_k1(&fx).origin_ids, vec![(1, 1)]);
    assert!(wait(&fx).is_empty());
}

#[test]
fn a_human_finding_with_a_fix_waits_for_the_plan_gate_and_for_an_unreachable_check() {
    let fx = Fx::new();
    project(&fx);
    // The plan gate opened again: the finding waits, and `bf next` lists why.
    fx.must(&["gate", "open", "plan"]);
    filed(&fx, "The footer is wrong.");
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(why(&fx), Some("plan_gate"));
    let d = directive(&fx, "see-waiting-human-findings").expect("an information entry");
    assert_eq!(d["info"], true, "{d}");
    assert_eq!(d["findings"][0]["waits_for"], "plan_gate", "{d}");
    fx.human("I approve the plan again.");
    fx.must(&[
        "gate",
        "approve",
        "plan",
        "--quote",
        "I approve the plan again.",
    ]);
    // A check marked unreachable at the checkpoint's content: the human decides it first.
    let o = fx.bf_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "unreachable",
            "X-2",
            "--reason",
            "the service is down",
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(why(&fx), Some("check"));
    next_json(&fx);
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(wait(&fx), vec![(1, 1)]);
}

#[test]
fn a_human_finding_filed_while_a_fix_is_at_work_joins_its_next_round_after_a_clean_review() {
    let fx = Fx::new();
    project(&fx);
    open_fix(&fx);
    filed(&fx, "The footer is wrong.");
    // The fix is at work: nothing is appended, the finding waits.
    assert_eq!(wait(&fx), vec![(2, 1)]);
    assert_eq!(why(&fx), Some("fix_round"));
    assert!(events_of(&fx, "checkpoint.fix.add").is_empty());
    let d = directive(&fx, "see-waiting-human-findings").expect("an information entry");
    assert_eq!(d["findings"][0]["waits_for"], "fix_round", "{d}");
    built_with(&fx, "changed by the fix\n");
    assert_eq!(phase(&fx), "review");
    // The review reports clean: the fix goes to its next round holding the human's finding.
    let o = fix_clean(&fx);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(phase(&fx), "fix");
    assert!(wait(&fx).is_empty());
    let adds = events_of(&fx, "checkpoint.fix.add");
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0]["human"], true);
    assert_eq!(adds[0]["step"], "review");
    assert!(adds[0].get("turn").is_none() || adds[0]["turn"].is_null());
    let f = fix_of_k1(&fx);
    assert_eq!(f.outside_design, vec!["r2.1".to_string()]);
    assert!(f
        .findings
        .iter()
        .any(|x| x["action"] == "The footer is wrong."));
    assert_eq!(f.rounds, 0);
    assert!(
        state(&fx).ck_fixes.is_empty(),
        "the fix must not have ended"
    );
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
    // It ends after the next round passes.
    built_with(&fx, "changed again\n");
    assert_eq!(fix_clean(&fx).code, 0);
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(state(&fx).ck_fixes.len(), 1);
}

#[test]
fn the_round_a_human_finding_alone_opens_counts_against_no_limit() {
    let fx = Fx::new();
    fx.set_policy("checkpoint_reviews = 3", "checkpoint_reviews = 1");
    project(&fx);
    let reviews = |fx: &Fx| {
        fx.must(&["status", "--json"]).json()["checkpoint_reviews"][0]["reviews"]
            .as_u64()
            .unwrap_or(0)
    };
    // A fix opened for the human finding alone: the round it comes from is no review.
    filed(&fx, "The footer is wrong.");
    assert_eq!(reviews(&fx), 0);
    fx.pass_checkpoint_fix("changed by the fix\n");
    assert_eq!(reviews(&fx), 0);
    let st = fx.must(&["status", "--json"]).json();
    assert_ne!(st["checkpoint_reviews"][0]["parked"], true, "{st}");
    // A round entered only for a human finding counts against the fix's limit only when its review filed a
    // critical, high or medium finding: a review that filed low findings only leaves the count as it was.
    let fx = Fx::new();
    project(&fx);
    open_fix(&fx);
    filed(&fx, "The footer is wrong.");
    built_with(&fx, "changed by the fix\n");
    assert_eq!(fix_review_low(&fx).code, 0);
    assert_eq!(phase(&fx), "fix");
    assert_eq!(fix_json(&fx)["rounds"], 0, "{}", fix_json(&fx));
    // A review that filed a high finding counts the round.
    built_with(&fx, "changed again\n");
    assert_eq!(
        fix_review(&fx, vec![finding("still wrong", "high")]).code,
        0
    );
    assert_eq!(fix_json(&fx)["rounds"], 1, "{}", fix_json(&fx));
}

#[test]
fn a_human_finding_filed_while_a_fix_waits_on_low_findings_puts_it_back_to_work() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    let rounds = fix_json(&fx)["rounds"].clone();
    filed(&fx, "The footer is wrong.");
    assert_eq!(phase(&fx), "fix");
    assert_eq!(fix_json(&fx)["rounds"], rounds);
    assert!(wait(&fx).is_empty());
    let adds = events_of(&fx, "checkpoint.fix.add");
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0]["step"], "low");
    assert_eq!(adds[0]["human"], true);
    assert_eq!(adds[0]["finding_ids"], json!([{"round": 2, "index": 1}]));
    let f = fix_of_k1(&fx);
    assert_eq!(f.outside_design, vec!["r2.1".to_string()]);
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_fix_parked_at_its_round_limit_keeps_the_waiting_human_findings_until_the_release() {
    let fx = Fx::new();
    fx.set_policy("max_rounds = 3", "max_rounds = 1");
    project(&fx);
    open_fix(&fx);
    built_with(&fx, "changed by the fix\n");
    assert_eq!(
        fix_review(&fx, vec![finding("still wrong", "high")]).code,
        0
    );
    assert_eq!(phase(&fx), "fix");
    filed(&fx, "The footer is wrong.");
    assert_eq!(wait(&fx), vec![(2, 1)]);
    built_with(&fx, "changed again\n");
    // The round limit parks the fix: the human finding is added to it and it stays parked.
    assert_eq!(fix_review(&fx, vec![finding("and again", "high")]).code, 0);
    assert_eq!(phase(&fx), "parked");
    assert!(wait(&fx).is_empty());
    let adds = events_of(&fx, "checkpoint.fix.add");
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0]["step"], "park");
    assert!(fix_of_k1(&fx)
        .findings
        .iter()
        .any(|x| x["action"] == "The footer is wrong."));
    // Only the human's release sends it to work.
    fx.human("Yes, go on with the fix.");
    fx.must(&[
        "checkpoint",
        "fix",
        "K-1",
        "--release",
        "--quote",
        "go on with the fix",
    ]);
    assert_eq!(phase(&fx), "fix");
    assert!(fix_of_k1(&fx)
        .findings
        .iter()
        .any(|x| x["action"] == "The footer is wrong."));
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}

#[test]
fn a_human_add_the_replay_meets_while_the_builder_is_at_work_is_ignored() {
    let fx = Fx::new();
    project(&fx);
    open_fix(&fx);
    filed(&fx, "The footer is wrong.");
    assert_eq!(wait(&fx), vec![(2, 1)]);
    let p = fx.project();
    for step in [
        "review", "low", "parked", "park", "design", "gap", "nonsense",
    ] {
        bf::events::append(
            &p,
            "checkpoint.fix.add",
            json!({"checkpoint": "K-1", "human": true, "step": step,
                   "finding_ids": [{"round": 2, "index": 1}], "filing": 1}),
        )
        .unwrap();
    }
    // An add for a finding that waits for no fix is ignored too.
    bf::events::append(
        &p,
        "checkpoint.fix.add",
        json!({"checkpoint": "K-1", "human": true, "step": "review",
               "finding_ids": [{"round": 1, "index": 1}], "filing": 1}),
    )
    .unwrap();
    let notes = ignored(&fx);
    assert_eq!(notes.len(), 8, "{notes:?}");
    assert!(
        notes
            .iter()
            .all(|n| n.contains("human findings were added")),
        "{notes:?}"
    );
    assert_eq!(phase(&fx), "fix");
    assert_eq!(wait(&fx), vec![(2, 1)]);
    assert!(fix_of_k1(&fx).outside_design.is_empty());
}

#[test]
fn a_gap_add_is_taken_only_right_after_the_fix_left_gap() {
    let fx = Fx::new();
    project(&fx);
    open_fix(&fx);
    filed(&fx, "The footer is wrong.");
    let p = fx.project();
    bf::events::append(
        &p,
        "checkpoint.fix.gap",
        json!({"checkpoint": "K-1", "reason": "r", "missing": ["src/x.txt"]}),
    )
    .unwrap();
    assert_eq!(phase(&fx), "gap");
    bf::events::append(
        &p,
        "checkpoint.fix.add",
        json!({"checkpoint": "K-1", "human": true, "step": "gap",
               "finding_ids": [{"round": 2, "index": 1}], "filing": 1}),
    )
    .unwrap();
    let notes = ignored(&fx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("human findings were added"), "{notes:?}");
    assert_eq!(phase(&fx), "gap");
    assert_eq!(wait(&fx), vec![(2, 1)]);
}

#[test]
fn a_fix_opened_by_the_way_human_with_no_waiting_filing_is_ignored_by_the_replay() {
    let fx = Fx::new();
    project(&fx);
    bf::events::append(
        &fx.project(),
        "checkpoint.fix.open",
        json!({"checkpoint": "K-1", "source": "review", "failed": [], "findings": [], "human": null,
               "finding_ids": [], "allowed": ["src/a.txt"], "test_dirs": [], "units": ["U-1"],
               "before": [], "content": "x", "wave": 0, "from_saved": "", "opened_by": "human",
               "confirm": true}),
    )
    .unwrap();
    assert!(state(&fx).ck_fix.is_none());
    let notes = ignored(&fx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("holds no human finding that waits"),
        "{notes:?}"
    );
}

#[test]
fn a_human_fix_gets_the_same_first_directive_as_a_fix_opened_for_review_findings() {
    let human = Fx::new();
    project(&human);
    filed(&human, "The footer is wrong.");
    let a = directive(&human, "fix-checkpoint").expect("a directive");
    let review = Fx::new();
    project(&review);
    open_fix(&review);
    let b = directive(&review, "fix-checkpoint").expect("a directive");
    let keys = |d: &Value| -> Vec<String> { d.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(keys(&a), keys(&b));
    for k in ["role", "action", "source", "tier", "checkpoint"] {
        assert_eq!(a[k], b[k], "{k}");
    }
}

#[test]
fn the_builders_brief_names_a_joined_human_finding_outside_the_design() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    filed(&fx, "The footer is wrong.");
    let md = fix_brief(&fx, "builder");
    assert!(md.contains("outside the stored design"), "{md}");
    assert!(
        md.contains("r2.1") && md.contains("The footer is wrong."),
        "{md}"
    );
    let d = directive(&fx, "fix-checkpoint").expect("a directive");
    let h = d["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "r2.1")
        .cloned()
        .expect("the human finding");
    assert_eq!(h["source"], "human", "{d}");
    assert_eq!(h["outside_design"], true, "{d}");
    built_with(&fx, "changed again\n");
    let md = fix_brief(&fx, "reviewer");
    assert!(md.contains("outside the stored design"), "{md}");
    assert!(md.contains("The footer is wrong."), "{md}");
}

#[test]
fn an_abort_leaves_the_human_findings_open_with_no_wait() {
    // A finding the fix held.
    let fx = Fx::new();
    project(&fx);
    filed(&fx, "The footer is wrong.");
    assert_eq!(fix_of_k1(&fx).origin_ids, vec![(1, 1)]);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    assert!(wait(&fx).is_empty());
    next_json(&fx);
    assert!(state(&fx).ck_fix.is_none(), "no fix opens again for it");
    assert_eq!(finding_state(&fx, "r1.1"), "open");
    // A finding the fix was to take at its next round.
    let fx = Fx::new();
    project(&fx);
    open_fix(&fx);
    filed(&fx, "The footer is wrong.");
    assert_eq!(wait(&fx), vec![(2, 1)]);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    assert!(wait(&fx).is_empty());
    next_json(&fx);
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(finding_state(&fx, "r2.1"), "open");
}

#[test]
fn accepting_a_waiting_human_finding_ends_its_wait() {
    let fx = Fx::new();
    project(&fx);
    review_brief(&fx);
    filed(&fx, "The footer is wrong.");
    assert_eq!(wait(&fx), vec![(2, 1)]);
    fx.human("Yes, accept the footer one.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--accept",
        "r2.1",
        "--quote",
        "accept the footer one",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(wait(&fx).is_empty());
    // The round reports with a finding: the fix opens for it alone.
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [finding("the big problem", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(fix_of_k1(&fx).origin_ids, vec![(1, 1)]);
}

#[test]
fn a_human_finding_joined_through_an_add_is_answered_at_the_fix_end_and_must_be_confirmed() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    // One added while the fix waits on its low findings (r2.1).
    filed(&fx, "The footer is wrong.");
    assert_eq!(phase(&fx), "fix");
    // One filed while its review has the turn (r3.1), added at its next round.
    built_with(&fx, "changed again\n");
    assert_eq!(phase(&fx), "review");
    filed(&fx, "The header is wrong.");
    assert_eq!(wait(&fx), vec![(3, 1)]);
    assert_eq!(fix_review_low(&fx).code, 0);
    assert_eq!(phase(&fx), "fix");
    assert!(wait(&fx).is_empty());
    built_with(&fx, "changed a third time\n");
    assert_eq!(fix_clean(&fx).code, 0);
    assert!(state(&fx).ck_fix.is_none());
    for id in ["r1.1", "r2.1", "r3.1"] {
        assert_eq!(finding_state(&fx, id), "answered", "{id}");
    }
    // The next review names each answered finding.
    let md = review_brief(&fx);
    assert!(md.contains("Answered findings to confirm"), "{md}");
    assert!(md.contains("r2.1") && md.contains("r3.1"), "{md}");
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"]}),
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r2.1"), "{}", o.all());
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1", "r2.1", "r3.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    for id in ["r1.1", "r2.1", "r3.1"] {
        assert_eq!(finding_state(&fx, id), "fixed", "{id}");
    }
    // With the review switched off, a joined finding is fixed when its fix ends.
    let fx = Fx::new();
    project(&fx);
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=off:faster",
        "--scope",
        "run",
    ]);
    fx.human("Yes, switch the review off.");
    fx.must(&["checks", "set", "--quote", "switch the review off"]);
    filed(&fx, "The footer is wrong.");
    built_with(&fx, "changed by the fix\n");
    assert_eq!(fix_review_low(&fx).code, 0);
    assert_eq!(phase(&fx), "low");
    filed(&fx, "The header is wrong.");
    assert_eq!(phase(&fx), "fix");
    built_with(&fx, "changed again\n");
    assert_eq!(fix_clean(&fx).code, 0);
    assert!(state(&fx).ck_fix.is_none());
    assert_eq!(finding_state(&fx, "r1.1"), "fixed");
    assert_eq!(finding_state(&fx, "r2.1"), "fixed");
}

#[test]
fn bf_next_lists_what_a_waiting_human_finding_waits_for() {
    let fx = Fx::new();
    project(&fx);
    // An open round of the review.
    review_brief(&fx);
    filed(&fx, "The footer is wrong.");
    let d = directive(&fx, "see-waiting-human-findings").expect("an entry");
    assert_eq!(d["role"], "human");
    assert_eq!(d["info"], true);
    assert_eq!(d["checkpoint"], "K-1");
    assert_eq!(d["findings"][0]["id"], "r2.1", "{d}");
    assert_eq!(d["findings"][0]["waits_for"], "round", "{d}");
    assert!(
        d["findings"][0]["waits_text"]
            .as_str()
            .unwrap()
            .contains("round"),
        "{d}"
    );
    assert!(d["note"].as_str().unwrap().contains("r2.1"), "{d}");
    // The information is no step for the human: the round's reviewer still goes on.
    let n = next_json(&fx);
    assert_eq!(n["human_needed"], false, "{n}");
    // A fix at work.
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [finding("the big problem", "high")]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(wait(&fx).is_empty());
    filed(&fx, "The header is wrong.");
    let d = directive(&fx, "see-waiting-human-findings").expect("an entry");
    assert_eq!(d["findings"][0]["waits_for"], "fix_round", "{d}");
    // The run and the budget end `bf next`; the reason is read from the same function.
    let p = fx.project();
    let pl = bf::plan::load(&p).unwrap();
    let mut st = bf::events::replay(&p).unwrap();
    st.cost = 100.0;
    let mut capped = p.clone();
    capped.policy.budget.cap = 1.0;
    st.ck_fix = None;
    assert_eq!(
        bf::ckhuman::waits_for(&capped, &st, &pl, "K-1").unwrap(),
        Some("budget")
    );
    st.runs.clear();
    assert_eq!(
        bf::ckhuman::waits_for(&p, &st, &pl, "K-1").unwrap(),
        Some("run")
    );
    for key in [
        "round",
        "fix_round",
        "sync",
        "other_fix",
        "units",
        "run",
        "plan_gate",
        "budget",
        "check",
    ] {
        let text = bf::ckhuman::waits_text(key, "K-1");
        assert!(!text.is_empty() && !text.contains("{k}"), "{key}: {text}");
    }
}

#[test]
fn the_other_cases_of_the_request_replay_to_the_same_fixes() {
    // Before the review: the fix opens at once, ends, and the close is refused until the review confirms.
    let fx = Fx::new();
    project(&fx);
    filed(&fx, "The footer is wrong.");
    fx.pass_checkpoint_fix("changed by the fix\n");
    assert_eq!(finding_state(&fx, "r1.1"), "answered");
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_ne!(o.code, 0, "{}", o.all());
    review_brief(&fx);
    let o = review_report(&fx, json!({"areas": areas(), "findings": []}));
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("r1.1"), "{}", o.all());
    let o = review_report(
        &fx,
        json!({"areas": areas(), "findings": [], "confirmed": ["r1.1"]}),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(finding_state(&fx, "r1.1"), "fixed");
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
    // The log replays to the same fixes, with nothing ignored and nothing waiting.
    let st = state(&fx);
    assert_eq!(st.ck_fixes.len(), 1);
    assert!(st.ck_human_wait.get("K-1").map_or(true, |w| w.is_empty()));
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
    // A closed checkpoint refuses the filing.
    fx.human("The header is wrong. Please fix it.");
    let o = fx.bf(&[
        "checkpoint",
        "finding",
        "K-1",
        "--human",
        "--quote",
        "The header is wrong.",
        "--fix",
        "--fix-quote",
        "Please fix it.",
    ]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert!(o.all().contains("closed"), "{}", o.all());
    // During a round of the review, the fix opens with its findings (the case of the TH-22 test); with a fix at
    // work, the finding joins the next round (the case of the clean review test); at the low findings it goes
    // back to work (the low test): each replays with nothing ignored.
    let fx = Fx::new();
    waiting_on_low(&fx);
    filed(&fx, "The footer is wrong.");
    assert_eq!(phase(&fx), "fix");
    assert!(ignored(&fx).is_empty(), "{:?}", ignored(&fx));
}
