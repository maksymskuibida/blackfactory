//! U-101 (D-115, D-116): the request, the review and the grant of files for a unit are removed. The
//! commands refuse with the text of a key, and a log of the old mechanism replays to the same unit
//! states while `bf next` gives each unit its builder's directive.

mod common;
use common::*;
use serde_json::{json, Map, Value};
use std::fs;

const STRINGS: &str = include_str!("../strings/en/scope_removed.toml");

/// Five independent units of one checkpoint; unit `U-i` lists `src/f<i>.txt`.
fn project(fx: &Fx) {
    project_with(fx, false);
}

/// As `project`; `chain` makes U-2 depend on U-1.
fn project_with(fx: &Fx, chain: bool) {
    let mut recs: Vec<(&str, String)> = Vec::new();
    let (mut ids, mut reqs) = (Vec::new(), Vec::new());
    for i in 1..=5 {
        recs.push(("requirement", format!("id=\"R-{i}\"\nkind=\"requirement\"\ntitle=\"Part {i}\"\nacceptance=[\"src/f{i}.txt exists\"]\n")));
        let dep = if chain && i == 2 { "depends_on=[\"U-1\"]\n" } else { "" };
        recs.push(("unit", format!("id=\"U-{i}\"\nkind=\"unit\"\ntitle=\"Part {i}\"\ndelivers=[\"R-{i}\"]\nfiles=[\"src/f{i}.txt\"]\n{dep}checkpoint=\"K-1\"\nestimate=1\n")));
        recs.push(("check", format!("id=\"X-{i}\"\nkind=\"check\"\ntitle=\"File {i}\"\nhow=\"command\"\ncommand=\"test -f src/f{i}.txt\"\nproves=[\"R-{i}\"]\nscope=\"unit\"\nunit=\"U-{i}\"\n")));
        ids.push(format!("\"U-{i}\""));
        reqs.push(format!("\"R-{i}\""));
    }
    let all = reqs.join(",");
    recs.push(("check", format!("id=\"X-9\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -d src\"\nproves=[{all}]\nscope=\"checkpoint\"\n")));
    recs.push(("checkpoint", format!("id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"All\"\nafter=[{}]\nproves=[{all}]\nchecks=[\"X-9\"]\n", ids.join(","))));
    for (k, t) in &recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    fs::create_dir_all(fx.root.join("src")).unwrap();
    fs::write(fx.root.join("src/other.txt"), "o\n").unwrap();
    fx.commit_all("records and files");
    fx.start_test_run();
}

/// Append one event of the old mechanism, hash-chained, and move the head.
fn append(fx: &Fx, kind: &str, fields: Value) {
    let p = fx.project();
    let key = p.read_key().unwrap();
    let log = p.log_path();
    let text = fs::read_to_string(&log).unwrap();
    let count = text.lines().count() + 1;
    let last = serde_json::from_str::<Value>(text.lines().last().unwrap()).unwrap();
    let prev = last["h"].as_str().unwrap().to_string();
    let mut m = Map::new();
    m.insert("seq".into(), json!(count));
    m.insert("at".into(), last["at"].clone());
    m.insert("kind".into(), json!(kind));
    m.insert("prev".into(), json!(prev));
    for (k, v) in fields.as_object().unwrap() {
        m.insert(k.clone(), v.clone());
    }
    let h = bf::events::event_hash(&key, &prev, &m);
    m.insert("h".into(), json!(h.clone()));
    let mut out = text;
    out.push_str(&serde_json::to_string(&Value::Object(m)).unwrap());
    out.push('\n');
    fs::write(&log, out).unwrap();
    let mut head = bf::events::read_head(&p).unwrap_or_default();
    head.count = count as u64;
    head.last_hash = h;
    bf::events::write_head(&p, &head).unwrap();
}

fn file(path: &str) -> Value {
    json!({"path": path, "why": "needed", "patch": "+x\n", "new": false})
}

/// The old log: U-1 has an open request, U-2 a granted file, U-3 a request refused as an extension,
/// U-4 waits for a file U-5 holds, and an order puts U-5 after U-4.
fn old_log(fx: &Fx) {
    project(fx);
    for u in ["U-1", "U-2", "U-3", "U-4"] {
        fx.must(&["unit", "start", u, "--json"]);
    }
    append(fx, "unit.scope.request", json!({"unit": "U-1", "from": "running", "files": [file("src/f5.txt")], "note": "", "cost": 0.0}));
    append(fx, "unit.scope.grant", json!({"unit": "U-2", "n": 1, "files": [file("docs/g.md")], "marks": [], "request": 0, "note": "", "cost": 0.0}));
    append(fx, "unit.scope.refused", json!({"unit": "U-3", "refusal": "extension", "from": "running", "files": [file("src/f1.txt")], "request": 0}));
    append(fx, "unit.scope.refused", json!({"unit": "U-4", "refusal": "hard", "from": "running", "files": [file("src/f5.txt")], "note": "", "cost": 0.0, "wait_for": ["U-5"]}));
    append(fx, "unit.order", json!({"unit": "U-5", "after": "U-4", "path": "src/f5.txt", "cause": "an old request"}));
}

fn directives(fx: &Fx) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .clone()
}

fn of(ds: &[Value], unit: &str) -> Vec<Value> {
    ds.iter().filter(|d| d["unit"] == unit).cloned().collect()
}

fn key_text(key: &str) -> String {
    let t: toml::Table = toml::from_str(STRINGS).unwrap();
    t[key].as_str().unwrap().to_string()
}

#[test]
fn a_request_for_files_is_refused_with_the_text_of_a_key() {
    let fx = Fx::new();
    project(&fx);
    fx.must(&["unit", "start", "U-1", "--json"]);
    let list = json!([{"path": "src/other.txt", "why": "w", "patch": "+x\n"}]).to_string();
    let o = fx.bf_stdin(
        &fx.work("U-1"),
        &["report", "U-1", "--result", "blocked", "--need-files", "-"],
        &list,
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains(&key_text("scope.removed.because")), "{}", o.stderr);
    assert!(o.stderr.contains(&key_text("scope.removed.step")), "{}", o.stderr);
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    assert!(!log.contains("unit.scope.request"));
}

#[test]
fn bf_unit_scope_is_refused_with_the_text_of_a_key() {
    let fx = Fx::new();
    project(&fx);
    fx.must(&["unit", "start", "U-1", "--json"]);
    let o = fx.bf(&["unit", "scope", "U-1", "--revert", "1", "--quote", "no"]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains(&key_text("scope.removed.because")), "{}", o.stderr);
}

#[test]
fn the_old_log_replays_to_the_same_unit_states_and_the_same_files() {
    let fx = Fx::new();
    old_log(&fx);
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    assert!(st.ignored.is_empty(), "{:?}", st.ignored);
    for (u, want) in [("U-1", "running"), ("U-2", "running"), ("U-3", "blocked"), ("U-4", "blocked")] {
        assert_eq!(st.unit_state(u), want, "{u}");
    }
    assert!(st.scope["U-1"].open.is_some(), "the request stays in the replay");
    assert_eq!(st.scope["U-3"].refused.len(), 1);
    assert_eq!(st.orders.len(), 1);
    // The guard table lists the record's files and the grant of the old log.
    let pl = bf::plan::load(&p).unwrap();
    let files = |id: &str| bf::scope::unit_files(&st, pl.unit(id).unwrap());
    assert_eq!(files("U-1"), vec!["src/f1.txt"]);
    assert_eq!(files("U-2"), vec!["src/f2.txt", "docs/g.md"]);
    assert_eq!(files("U-3"), vec!["src/f3.txt"]);
}

#[test]
fn bf_next_gives_a_free_unit_its_builders_directive_a_blocked_one_the_human_and_no_scope_directive() {
    let fx = Fx::new();
    old_log(&fx);
    let ds = directives(&fx);
    for d in &ds {
        assert_ne!(d["action"], "review-scope", "{d}");
        assert_ne!(d["role"], "reviewer", "{d}");
    }
    for u in ["U-1", "U-2"] {
        let own = of(&ds, u);
        assert_eq!(own.len(), 1, "{u}: {ds:?}");
        assert_eq!(own[0]["role"], "builder", "{u}");
    }
    // A unit an old refusal blocked goes to the human, and the note gives the reason from a key, never the
    // raw word of the refusal (FU-36).
    for u in ["U-3", "U-4"] {
        let own = of(&ds, u);
        assert_eq!(own.len(), 1, "{u}: {ds:?}");
        assert_eq!(own[0]["action"], "decide-unit", "{u}");
        let note = own[0]["note"].as_str().unwrap();
        assert!(!note.contains("(extension)") && !note.contains("(hard)") && !note.contains("retry_spent"), "{note}");
    }
    // The order an old request made no longer holds U-5 back.
    let five = of(&ds, "U-5");
    assert_eq!(five.len(), 1, "{ds:?}");
    assert_eq!(five[0]["action"], "start-unit");
}

#[test]
fn a_builder_report_is_taken_for_a_unit_whose_request_was_open() {
    let fx = Fx::new();
    old_log(&fx);
    let wc = fx.work("U-1");
    fs::write(wc.join("src/f1.txt"), "x\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/f1.txt"]);
    let o = fx.bf_in(&wc, &["report", "U-1", "--result", "built"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_file_listed_by_a_unit_that_depends_on_the_asker_is_not_held_beside() {
    let fx = Fx::new();
    project_with(&fx, true);
    fx.must(&["unit", "start", "U-1", "--json"]);
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    let pl = bf::plan::load(&p).unwrap();
    // U-2 comes after U-1 by the records, so `bf unit format` of its file is not refused for U-1.
    assert_eq!(bf::scope::holder_beside(&p, &pl, &st, "U-1", "src/f2.txt"), None);
    assert_eq!(bf::scope::hard_rule(&p, &pl, &st, "U-1", "src/f2.txt"), None);
    // U-3 may run beside U-1, so its file is still held.
    assert_eq!(
        bf::scope::holder_beside(&p, &pl, &st, "U-1", "src/f3.txt").as_deref(),
        Some("U-3")
    );
}
