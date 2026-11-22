//! U-60: the plan page shows each unit's tests or full-suite reason, the heavy limit and the heavy checks; the
//! amendment page shows a unit's tests beside its files; the scope reviewer's brief holds the unit's tests and
//! the question; the checkpoint and run pages hold the rows of the judged repairs, grants and accepted findings
//! (R-62, R-63, D-92, H-25).

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn recs(fx: &Fx, list: &[(&str, &str)]) {
    for (k, t) in list {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
}

/// Three units: one names its tests, one runs the full suite and says why, one says neither.
fn project(fx: &Fx, extra: &[(&str, &str)]) {
    recs(
        fx,
        &[
            ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A\"\nacceptance=[\"src/a.rs exists\"]\n"),
            ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part A\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ntests=\"cargo nextest run --package=x a::\"\ncheckpoint=\"K-1\"\nestimate=1\n"),
            ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"Part B\"\ndelivers=[\"R-1\"]\nfiles=[\"src/b.rs\"]\nfull_suite=\"the build script reads every test\"\ncheckpoint=\"K-1\"\nestimate=1\n"),
            ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Part C\"\ndelivers=[\"R-1\"]\nfiles=[\"src/c.rs\"]\ncheckpoint=\"K-1\"\nestimate=1\n"),
            ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
            ("check", "id=\"X-11\"\nkind=\"check\"\ntitle=\"A\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
            ("check", "id=\"X-12\"\nkind=\"check\"\ntitle=\"B\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
            ("check", "id=\"X-13\"\nkind=\"check\"\ntitle=\"C\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
            ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"All\"\nafter=[\"U-1\",\"U-2\",\"U-3\"]\nproves=[\"R-1\"]\nchecks=[\"X-1\"]\n"),
        ],
    );
    recs(fx, extra);
    // The fixture's `put` adds a full-suite reason to a unit that has none: write the third unit as it is.
    fs::write(
        fx.root.join("factory/records/U-3.toml"),
        "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Part C\"\ndelivers=[\"R-1\"]\nfiles=[\"src/c.rs\"]\ncheckpoint=\"K-1\"\nestimate=1\n",
    )
    .unwrap();
    fx.commit_all("records");
}

fn all_text(v: &Value) -> String {
    v.to_string()
}

fn rows_of<'a>(page: &'a Value, section: &str) -> &'a Vec<Value> {
    page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == section)
        .unwrap_or_else(|| panic!("no section {section}: {page}"))["rows"]
        .as_array()
        .unwrap()
}

#[test]
fn the_plan_page_shows_each_units_tests_or_reason_and_the_heavy_limit() {
    let fx = Fx::new();
    project(
        &fx,
        &[
            ("plan_settings", "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"Limits\"\nheavy_limit=2\n"),
            ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"Heavy\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nheavy=true\n"),
        ],
    );
    let page = fx.must(&["view", "plan", "--json"]).json();
    assert_contract("plan", &page);
    let units = rows_of(&page, "units");
    let body = |id: &str| all_text(&units.iter().find(|r| r["id"] == id).unwrap()["body"]);
    assert!(
        body("U-1").contains("cargo nextest run --package=x a::"),
        "{page}"
    );
    assert!(
        body("U-2").contains("the build script reads every test"),
        "{page}"
    );
    assert!(body("U-3").contains("Names no tests"), "{page}");
    let heavy = page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "heavy")
        .expect("a heavy section");
    let t = all_text(heavy);
    assert!(t.contains("At most 2 heavy checks"), "{t}");
    assert!(t.contains("X-2"), "{t}");
}

#[test]
fn the_plan_gate_page_titles_the_plan_settings_section_from_its_keys() {
    let fx = Fx::new();
    project(
        &fx,
        &[("plan_settings", "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"Limits\"\nheavy_limit=2\n")],
    );
    let page = fx.must(&["view", "gate", "plan", "--json"]).json();
    assert_contract("gate", &page);
    let sec = page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "plan_settings")
        .unwrap_or_else(|| panic!("no plan_settings section: {page}"));
    assert_eq!(sec["title"], "Plan settings", "{sec}");
    assert_eq!(sec["hint"], "The limit of heavy checks at once", "{sec}");
    let rows = sec["rows"].as_array().unwrap();
    let row = rows.iter().find(|r| r["id"] == "PL-1").expect("a PL-1 row");
    assert!(
        all_text(row).contains("Limit of heavy checks at once: 2"),
        "{row}"
    );
    assert!(!page.to_string().contains("Plan_settingss"), "{page}");
}

#[test]
fn the_plan_page_says_when_no_limit_is_set() {
    let fx = Fx::new();
    project(&fx, &[]);
    let page = fx.must(&["view", "plan", "--json"]).json();
    assert_contract("plan", &page);
    let heavy = page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "heavy")
        .unwrap();
    let t = all_text(heavy);
    assert!(t.contains("sets no limit"), "{t}");
    assert!(t.contains("No check is marked heavy"), "{t}");
}

#[test]
fn the_checkpoint_and_run_pages_hold_the_rows_for_grants_findings_and_judged_repairs() {
    let fx = Fx::new();
    project(&fx, &[]);
    let k = fx.must(&["view", "checkpoint", "K-1", "--json"]).json();
    assert_contract("checkpoint", &k);
    for id in ["fix-grants", "accepted"] {
        assert!(rows_of(&k, id).is_empty(), "{id}: {k}");
    }
    let r = fx.must(&["view", "run", "--json"]).json();
    assert_contract("run", &r);
    assert!(rows_of(&r, "judged-repairs").is_empty(), "{r}");
}

const WORDS: &str = "I accept those findings";

fn append(fx: &Fx, kind: &str, data: Value) {
    bf::events::append(&fx.project(), kind, data).unwrap();
}

/// The log position of the human's latest message.
fn last_turn(fx: &Fx) -> u64 {
    bf::events::replay(&fx.project())
        .unwrap()
        .turns
        .last()
        .unwrap()
        .seq
}

fn title_of(page: &Value, section: &str, id: &str) -> String {
    let row = rows_of(page, section)
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no row {id} in {section}: {page}"));
    all_text(row)
}

#[test]
fn the_pages_show_a_filled_grant_an_accepted_finding_and_a_judged_repair_until_it_is_seen() {
    let fx = Fx::new();
    start_project(&fx);
    // A unit finding the human accepted, with its level.
    append(
        &fx,
        "unit.report",
        json!({"unit": "U-1", "result": "findings", "from": "review", "to": "parked",
               "findings": ["F-1"], "round": 1, "content": "c", "head": "h", "covered": "all"}),
    );
    append(
        &fx,
        "unit.park",
        json!({"unit": "U-1", "reason": "more than 3 review rounds"}),
    );
    fx.human("Yes, I accept those findings.");
    let t = last_turn(&fx);
    append(
        &fx,
        "unit.finding.accept",
        json!({"unit": "U-1", "findings": [{"id": "F-1", "severity": "high"}], "turn": t, "quote": WORDS}),
    );
    // A fix of the checkpoint: one granted file with its reason, and one accepted finding.
    append(
        &fx,
        "checkpoint.fix.open",
        json!({"checkpoint": "K-1", "source": "check", "content": "c1", "failed": [{"check": "X-1"}],
               "allowed": ["src/a.rs"], "units": ["U-1"]}),
    );
    let file = json!([{"path": "tests/b.rs", "why": "needs a test", "patch": "+x"}]);
    append(
        &fx,
        "checkpoint.fix.scope.request",
        json!({"checkpoint": "K-1", "files": file}),
    );
    append(
        &fx,
        "checkpoint.fix.scope.grant",
        json!({"checkpoint": "K-1", "files": file}),
    );
    append(&fx, "checkpoint.fix.built", json!({"checkpoint": "K-1"}));
    append(
        &fx,
        "checkpoint.fix.reviewed",
        json!({"checkpoint": "K-1", "round": 1, "result": "findings", "to": "low", "content": "c2",
               "heads": [], "covered": "all", "findings": [{"title": "a", "severity": "low"}]}),
    );
    fx.human("Yes, I accept those findings.");
    let t = last_turn(&fx);
    append(
        &fx,
        "checkpoint.fix.finding.settle",
        json!({"checkpoint": "K-1", "fate": "accepted", "ids": [{"round": 1, "index": 1}], "turn": t, "quote": WORDS}),
    );
    // Repairs the judge did not settle: one it passed on and the human withdrew, one it failed on.
    append(
        &fx,
        "amend.open",
        json!({"amend": "A-1", "ids": ["R-1"], "impact_ids": ["R-1"], "repair": true}),
    );
    append(
        &fx,
        "amend.narrow",
        json!({"amend": "A-1", "keep": ["R-1"], "waive": [], "set_hash": "h1", "changes": []}),
    );
    append(
        &fx,
        "amend.judge",
        json!({"amend": "A-1", "verdict": "ask", "reasons": "the wording is unclear", "model": "m", "fingerprint": "h1"}),
    );
    append(
        &fx,
        "amend.withdraw",
        json!({"amend": "A-1", "reason": "a smaller one"}),
    );
    append(
        &fx,
        "amend.open",
        json!({"amend": "A-2", "ids": ["R-1"], "impact_ids": ["R-1"], "repair": true}),
    );
    append(
        &fx,
        "amend.narrow",
        json!({"amend": "A-2", "keep": ["R-1"], "waive": [], "set_hash": "h2", "changes": []}),
    );
    append(
        &fx,
        "amend.judge.token",
        json!({"amend": "A-2", "fingerprint": "h2"}),
    );
    append(
        &fx,
        "amend.judge.failed",
        json!({"amend": "A-2", "fingerprint": "h2", "reason": "no verdict came"}),
    );
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);

    let k = fx.must(&["view", "checkpoint", "K-1", "--json"]).json();
    assert_contract("checkpoint", &k);
    let grant = title_of(&k, "fix-grants", "K-1-grant-1");
    assert!(
        grant.contains("tests/b.rs") && grant.contains("needs a test"),
        "{grant}"
    );
    let unit_accept = title_of(&k, "accepted", "U-1-F-1");
    assert!(
        unit_accept.contains("level high") && unit_accept.contains(WORDS),
        "{unit_accept}"
    );
    let fix_accept = title_of(&k, "accepted", "K-1-1.1");
    assert!(
        fix_accept.contains("level low") && fix_accept.contains(WORDS),
        "{fix_accept}"
    );

    let r = fx.must(&["view", "run", "--json"]).json();
    assert_contract("run", &r);
    let ask = title_of(&r, "judged-repairs", "A-1");
    assert!(
        ask.contains("passed to you") && ask.contains("withdrawn"),
        "{ask}"
    );
    assert!(ask.contains("the wording is unclear"), "{ask}");
    let failed = title_of(&r, "judged-repairs", "A-2");
    assert!(
        failed.contains("failed") && failed.contains("no verdict came"),
        "{failed}"
    );

    // The human says they saw the first one: its row goes, the other stays.
    fx.human("Yes, I accept those findings.");
    let t = last_turn(&fx);
    append(
        &fx,
        "amend.judge.seen",
        json!({"amend": "A-1", "turn": t, "quote": WORDS}),
    );
    let r = fx.must(&["view", "run", "--json"]).json();
    assert_contract("run", &r);
    let ids: Vec<&Value> = rows_of(&r, "judged-repairs")
        .iter()
        .map(|x| &x["id"])
        .collect();
    assert_eq!(ids, vec!["A-2"], "{r}");
}

/// A copy of the shipped project whose plan was approved before units named their tests.
struct Old {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    base: PathBuf,
}

impl Old {
    fn new() -> Old {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        for part in ["project", "state"] {
            let st = std::process::Command::new("cp")
                .arg("-R")
                .arg(fixture("old_project").join(part))
                .arg(base.join(part))
                .status()
                .unwrap();
            assert!(st.success());
        }
        Old {
            root: base.join("project"),
            base,
            _tmp: tmp,
        }
    }

    fn run(&self, args: &[&str], stdin: &str) -> (i32, String, String) {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut c = Command::new(bin())
            .env("BF_STATE_HOME", self.base.join("state"))
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
        let o = c.wait_with_output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        )
    }

    fn must(&self, args: &[&str]) -> String {
        let (c, out, err) = self.run(args, "");
        assert_eq!(c, 0, "{args:?}: {out}{err}");
        out
    }

    fn put(&self, kind: &str, text: &str, replace: bool) {
        let f = self.root.join("put.toml");
        fs::write(&f, text).unwrap();
        let mut a = vec!["record", "put", kind, "--file", f.to_str().unwrap()];
        if replace {
            a.push("--replace");
        }
        self.must(&a);
        fs::remove_file(f).unwrap();
    }

    fn record(&self, id: &str) -> String {
        fs::read_to_string(self.root.join(format!("factory/records/{id}.toml"))).unwrap()
    }
}

#[test]
fn the_plan_page_says_a_unit_of_an_old_plan_runs_the_full_suite() {
    let old = Old::new();
    let page: Value = serde_json::from_str(&old.must(&["view", "plan", "--json"])).unwrap();
    assert_contract("plan", &page);
    for id in ["U-1", "U-2"] {
        let b = all_text(
            &rows_of(&page, "units")
                .iter()
                .find(|r| r["id"] == id)
                .unwrap()["body"],
        );
        assert!(
            b.contains("approved before units named their tests"),
            "{id}: {b}"
        );
        assert!(!b.contains("Names no tests"), "{id}: {b}");
    }
}

#[test]
fn the_amendment_page_shows_tests_beside_changed_files_and_judges_a_new_unit_as_new() {
    let old = Old::new();
    let opened: Value = serde_json::from_str(
        &old.must(&["amend", "open", "--repair", "U-1", "U-2", "U-3", "--json"]),
    )
    .unwrap();
    // U-1 changes its files; U-2 changes only its title; U-3 is new. None names tests or a reason.
    let u1 = old
        .record("U-1")
        .replacen("src/hello.txt", "src/hello2.txt", 1);
    old.put("unit", &u1, true);
    let u2 = old
        .record("U-2")
        .replacen("title = \"Farewell\"", "title = \"Goodbye\"", 1);
    assert_ne!(u2, old.record("U-2"));
    old.put("unit", &u2, true);
    old.put(
        "unit",
        "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Third\"\ndelivers=[\"R-2\"]\nfiles=[\"src/c.txt\"]\ncheckpoint=\"K-1\"\nestimate=1\n",
        false,
    );
    let f = old.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}).to_string(),
    )
    .unwrap();
    old.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    fs::remove_file(f).unwrap();

    let page: Value = serde_json::from_str(&old.must(&["view", "run", "--json"])).unwrap();
    assert_contract("run", &page);
    let body = |section: &str, id: &str| {
        all_text(
            &rows_of(&page, section)
                .iter()
                .find(|r| r["id"] == id)
                .unwrap_or_else(|| panic!("no row {id} in {section}: {page}"))["body"],
        )
    };
    // The unit whose files changed: files and tests beside each other, in both sections. Its approval makes
    // it a new unit, so it must name its tests.
    for section in ["A-1-changes", "A-1-plan"] {
        let b = body(section, "U-1");
        assert!(b.contains("Files: src/hello2.txt"), "{section}: {b}");
        assert!(b.contains("Names no tests"), "{section}: {b}");
    }
    // A unit the amendment adds is judged as new, not as a unit of the old plan.
    for section in ["A-1-changes", "A-1-plan"] {
        let b = body(section, "U-3");
        assert!(b.contains("Files: src/c.txt"), "{section}: {b}");
        assert!(b.contains("Names no tests"), "{section}: {b}");
        assert!(
            !b.contains("approved before units named their tests"),
            "{section}: {b}"
        );
    }
    // The unit whose files did not change has no files or tests line in the list of changes, and keeps the
    // old-plan line in the plan stage.
    let b = body("A-1-changes", "U-2");
    assert!(!b.contains("Files:") && !b.contains("full suite"), "{b}");
    let b = body("A-1-plan", "U-2");
    assert!(b.contains("approved before units named their tests"), "{b}");
}

fn start_project(fx: &Fx) -> PathBuf {
    project(fx, &[]);
    // A plan with a unit that says neither is refused: the third unit runs the full suite.
    fs::write(
        fx.root.join("factory/records/U-3.toml"),
        "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Part C\"\ndelivers=[\"R-1\"]\nfiles=[\"src/c.rs\"]\nfull_suite=\"fixture\"\ncheckpoint=\"K-1\"\nestimate=1\n",
    )
    .unwrap();
    fx.commit_all("third unit");
    fx.start_test_run();
    PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}
