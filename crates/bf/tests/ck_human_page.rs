//! U-140 (R-81, S-7, J-7, D-171, D-62): the checkpoint page and `bf status` show a human finding with its words
//! and its source, what a waiting one waits for, the human findings a fix took and those a parked fix holds.

#![allow(dead_code)]

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


fn page(fx: &Fx) -> Value {
    fx.must(&["view", "checkpoint", "K-1", "--json"]).json()
}

fn section_rows(pg: &Value, id: &str) -> Vec<Value> {
    pg["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .map(|s| s["rows"].as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
}

fn row_of(pg: &Value, id: &str) -> Value {
    section_rows(pg, "findings")
        .into_iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no finding row {id}: {pg}"))
}

fn body_text(row: &Value) -> String {
    row["body"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The human writes `said` and the conductor files `quote` of it with no fix.
fn file_plain(fx: &Fx, said: &str, quote: &str, extra: &[&str]) {
    fx.human(said);
    let mut args = vec!["checkpoint", "finding", "K-1", "--human", "--quote", quote];
    args.extend(extra);
    let o = fx.bf(&args);
    assert_eq!(o.code, 0, "{}", o.all());
}

fn assert_waits(fx: &Fx, key: &str) {
    let pg = page(fx);
    let row = row_of(&pg, "r1.1");
    assert_eq!(row["state"], "wants_fix", "{row}");
    assert_eq!(row["waits_for"], key, "{row}");
    assert!(
        body_text(&row).contains("Waits for a fix"),
        "{key}: {}",
        body_text(&row)
    );
    let st = fx.must(&["status", "--json"]).json();
    let f = &st["checkpoint_reviews"][0]["findings"][0];
    assert_eq!(f["state"], "wants_fix", "{st}");
    assert_eq!(f["waits_for"], key, "{st}");
}

#[test]
fn the_page_shows_a_human_finding_with_its_words_and_its_source() {
    let fx = Fx::new();
    project(&fx);
    file_plain(
        &fx,
        "The footer is wrong. Also the header.",
        "The footer is wrong.",
        &[],
    );
    let pg = page(&fx);
    let row = row_of(&pg, "r1.1");
    assert_eq!(row["source"], "human", "{row}");
    assert_eq!(row["severity"], "high", "{row}");
    assert_eq!(row["state"], "open", "{row}");
    assert_eq!(row["weight_given"], false, "{row}");
    assert_eq!(row["quote"]["text"], "The footer is wrong.", "{row}");
    assert!(row["quote"]["turn"].as_u64().unwrap() > 0, "{row}");
    let tags = row["tags"].to_string();
    assert!(tags.contains("weight not given"), "{tags}");
    assert!(body_text(&row).contains("message"), "{row}");
    // A weight the human named carries no such tag.
    file_plain(
        &fx,
        "The header is wrong, medium.",
        "The header is wrong, medium.",
        &["--severity", "medium"],
    );
    let row = row_of(&page(&fx), "r2.1");
    assert_eq!(row["severity"], "medium", "{row}");
    assert_eq!(row["weight_given"], true, "{row}");
    assert!(!row["tags"].to_string().contains("weight not given"));
    // The human's round is no row of the reviews section.
    let reviews = section_rows(&page(&fx), "reviews");
    assert!(reviews.is_empty(), "{reviews:?}");
}

#[test]
fn a_waiting_human_finding_says_what_it_waits_for() {
    // An open round of the review.
    let fx = Fx::new();
    project(&fx);
    review_brief(&fx);
    filed(&fx, "The footer is wrong.");
    let row = row_of(&page(&fx), "r2.1");
    assert_eq!(row["state"], "wants_fix");
    assert_eq!(row["waits_for"], "round");
    assert!(body_text(&row).contains("Waits for a fix"));
    assert!(row["handled"]["text"]
        .as_str()
        .unwrap()
        .contains("waits for a fix"));
    // A fix at work.
    let fx = Fx::new();
    project(&fx);
    open_fix(&fx);
    filed(&fx, "The footer is wrong.");
    let row = row_of(&page(&fx), "r2.1");
    assert_eq!(row["waits_for"], "fix_round", "{row}");
    // A unit that has not landed.
    let fx = Fx::new();
    project_with(&fx, false, Second::Same);
    filed(&fx, "The footer is wrong.");
    assert_waits(&fx, "units");
    // A sync.
    let fx = Fx::new();
    project(&fx);
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/a.txt"), "work on main\n").unwrap();
    git(&fx.root, &["add", "src/a.txt"]);
    git(&fx.root, &["commit", "-q", "-m", "work on main"]);
    fx.human("Yes, bring main in now.");
    fx.must(&[
        "checkpoint",
        "sync",
        "K-1",
        "--local",
        "--quote",
        "bring main in",
        "--json",
    ]);
    filed(&fx, "The footer is wrong.");
    assert_waits(&fx, "sync");
    // A fix of another checkpoint.
    let fx = Fx::new();
    project_with(&fx, false, Second::Other);
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
    filed(&fx, "The footer is wrong.");
    assert_waits(&fx, "other_fix");
    // The plan gate, then a check that waits for the human.
    let fx = Fx::new();
    project(&fx);
    fx.must(&["gate", "open", "plan"]);
    filed(&fx, "The footer is wrong.");
    assert_waits(&fx, "plan_gate");
    fx.human("I approve the plan again.");
    fx.must(&[
        "gate",
        "approve",
        "plan",
        "--quote",
        "I approve the plan again.",
    ]);
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
    assert_waits(&fx, "check");
    // The budget cap.
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
    assert_waits(&fx, "budget");
}

#[test]
fn a_waiting_human_finding_of_a_stopped_run_waits_for_the_run() {
    let fx = Fx::new();
    project(&fx);
    let run = bf::events::replay(&fx.project())
        .unwrap()
        .active_run()
        .map(|r| r.id.clone())
        .unwrap();
    bf::events::append(&fx.project(), "run.stop", json!({ "run": run })).unwrap();
    filed(&fx, "The footer is wrong.");
    assert_waits(&fx, "run");
}

#[test]
fn a_fixs_row_names_the_human_findings_it_took_and_the_round_they_joined() {
    let fx = Fx::new();
    waiting_on_low(&fx);
    filed(&fx, "The footer is wrong.");
    let pg = page(&fx);
    let fixes = section_rows(&pg, "fixes");
    let row = fixes.last().expect("the open fix");
    let body = body_text(row);
    assert!(body.contains("Human findings taken: r2.1"), "{body}");
    assert!(body.contains("while the fix waited on low findings"), "{body}");
    assert!(body.contains("Outside the stored design: r2.1"), "{body}");
    assert!(!body.contains("held for the release"), "{body}");
}

#[test]
fn a_fix_parked_at_its_round_limit_shows_the_human_findings_it_holds() {
    let fx = Fx::new();
    fx.set_policy("max_rounds = 3", "max_rounds = 1");
    project(&fx);
    open_fix(&fx);
    built_with(&fx, "changed by the fix\n");
    assert_eq!(
        fix_review(&fx, vec![finding("still wrong", "high")]).code,
        0
    );
    filed(&fx, "The footer is wrong.");
    built_with(&fx, "changed again\n");
    assert_eq!(fix_review(&fx, vec![finding("and again", "high")]).code, 0);
    assert_eq!(phase(&fx), "parked");
    let pg = page(&fx);
    let fixes = section_rows(&pg, "fixes");
    let body = body_text(fixes.last().unwrap());
    assert!(
        body.contains("Human findings held for the release: r2.1"),
        "{body}"
    );
    assert!(body.contains("at the park of the fix"), "{body}");
    // The release sends the fix to work holding them: nothing is held any more.
    fx.human("Yes, go on with the fix.");
    fx.must(&[
        "checkpoint",
        "fix",
        "K-1",
        "--release",
        "--quote",
        "go on with the fix",
    ]);
    let pg = page(&fx);
    let body = body_text(section_rows(&pg, "fixes").last().unwrap());
    assert!(!body.contains("held for the release"), "{body}");
    assert!(body.contains("Human findings taken: r2.1"), "{body}");
}

#[test]
fn bf_status_gives_the_source_human_and_what_a_waiting_finding_waits_for() {
    let fx = Fx::new();
    project(&fx);
    review_brief(&fx);
    filed(&fx, "The footer is wrong.");
    let st = fx.must(&["status", "--json"]).json();
    let list = st["checkpoint_reviews"][0]["findings"].as_array().unwrap();
    let f = list.iter().find(|f| f["id"] == "r2.1").expect("the finding");
    assert_eq!(f["source"], "human", "{f}");
    assert_eq!(f["state"], "wants_fix", "{f}");
    assert_eq!(f["waits_for"], "round", "{f}");
    let plain = fx.must(&["status"]).stdout;
    assert!(
        plain.contains("r2.1") && plain.contains("waits for a fix") && plain.contains("round"),
        "{plain}"
    );
}

#[test]
fn every_new_page_text_comes_from_a_key() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let toml = fs::read_to_string(dir.join("strings/en/pages_human.toml")).unwrap();
    let audit = fs::read_to_string(dir.join("strings/en/pages_audit.toml")).unwrap();
    for k in [
        "page.quote.label",
        "page.finding.weight.not_given",
        "page.finding.state.wants_fix",
        "page.finding.wants_fix.round",
        "page.finding.wants_fix.fix_round",
        "page.finding.wants_fix.sync",
        "page.finding.wants_fix.units",
        "page.finding.wants_fix.other_fix",
        "page.finding.wants_fix.run",
        "page.finding.wants_fix.plan_gate",
        "page.finding.wants_fix.budget",
        "page.finding.wants_fix.check",
        "page.fix.human_added",
        "page.fix.outside_design",
        "page.fix.human_held",
    ] {
        assert!(toml.contains(&format!("\"{k}\" = ")), "{k}");
    }
    assert!(audit.contains("\"page.finding.source.human\" = "));
    let src = fs::read_to_string(dir.join("src/view.rs")).unwrap();
    let start = src.find("fn human_lines").unwrap();
    let end = src.find("fn fix_rows").unwrap();
    let rows_start = src.find("fn checkpoint_finding_rows").unwrap();
    let rows_end = src.find("fn checkpoint_review_rows").unwrap();
    for body in [&src[start..end], &src[rows_start..rows_end]] {
        for word in [
            "Waits for a fix",
            "Human findings taken",
            "Outside the stored design",
            "held for the release",
            "weight not given",
            "The human's words",
        ] {
            assert!(!body.contains(word), "{word} is in code");
        }
    }
}
