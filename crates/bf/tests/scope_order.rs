//! U-106 (A-73, D-85): a requested file held beside by a unit that has not started puts that unit after
//! the asker as an order in the run log; the records never change, and no order makes a cycle.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn write(root: &Path, rel: &str, text: &str) {
    let f = root.join(rel);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

/// Units on one checkpoint: A = U-1 (asks, holds src/a.rs), B = U-3 (holds beside/y.txt) and
/// M = U-4 (depends on B, holds mid/z.txt).
fn project(fx: &Fx) {
    let unit = |id: &str, file: &str, dep: &str, r: &str| {
        format!("id=\"{id}\"\nkind=\"unit\"\ntitle=\"Part {id}\"\ndelivers=[\"{r}\"]\nfiles=[\"{file}\"]\n{dep}checkpoint=\"K-1\"\nestimate=1\n")
    };
    let check = |id: &str, file: &str, r: &str, u: &str| {
        format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"C {id}\"\nhow=\"command\"\ncommand=\"test -f {file}\"\nproves=[\"{r}\"]\nscope=\"unit\"\nunit=\"{u}\"\n")
    };
    let req = |id: &str, file: &str| {
        format!("id=\"{id}\"\nkind=\"requirement\"\ntitle=\"R {id}\"\nacceptance=[\"{file} exists\"]\n")
    };
    let recs: Vec<(&str, String)> = vec![
        ("requirement", req("R-1", "src/a.rs")),
        ("requirement", req("R-3", "beside/y.txt")),
        ("requirement", req("R-4", "mid/z.txt")),
        ("unit", unit("U-1", "src/a.rs", "", "R-1")),
        ("unit", unit("U-3", "beside/y.txt", "", "R-3")),
        ("unit", unit("U-4", "mid/z.txt", "depends_on=[\"U-3\"]\n", "R-4")),
        ("check", check("X-1", "src/a.rs", "R-1", "U-1")),
        ("check", check("X-3", "beside/y.txt", "R-3", "U-3")),
        ("check", check("X-4", "mid/z.txt", "R-4", "U-4")),
        ("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"All\"\nhow=\"command\"\ncommand=\"test -f src/a.rs\"\nproves=[\"R-1\",\"R-3\",\"R-4\"]\nscope=\"checkpoint\"\n".to_string()),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"All\"\nafter=[\"U-1\",\"U-3\",\"U-4\"]\nproves=[\"R-1\",\"R-3\",\"R-4\"]\nchecks=[\"X-9\"]\n".to_string()),
    ];
    for (k, t) in &recs {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{k}: {}", o.all());
    }
    for f in ["src/a.rs", "beside/y.txt", "mid/z.txt"] {
        write(&fx.root, f, "x\n");
    }
    fx.commit_all("records and files");
    fx.start_test_run();
}

fn start(fx: &Fx, id: &str) -> PathBuf {
    PathBuf::from(
        fx.must(&["unit", "start", id, "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

fn ask(fx: &Fx, wc: &Path, path: &str) -> Out {
    let list = json!([{"path": path, "why": "R-1", "patch": "--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n"}]);
    fx.bf_stdin(
        wc,
        &["report", "U-1", "--result", "blocked", "--need-files", "-", "--json"],
        &list.to_string(),
    )
}

fn directives(fx: &Fx) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .clone()
}

fn offered(fx: &Fx, unit: &str) -> bool {
    directives(fx).iter().any(|d| d["unit"] == unit)
}

fn log(fx: &Fx) -> String {
    fs::read_to_string(fx.project().log_path()).unwrap()
}

fn orders(fx: &Fx) -> Vec<Value> {
    bf::events::replay(&fx.project()).unwrap().orders
}

fn records(fx: &Fx) -> String {
    let d = fx.root.join("factory/records");
    let mut names: Vec<_> = fs::read_dir(d).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    names
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Make U-1 depend on `dep` through an approved repair amendment (the records change).
fn depend_on(fx: &Fx, dep: &str) {
    let o = fx.must(&["amend", "open", "--repair", "--json", "U-1"]).json();
    let toml = format!("id=\"U-1\"\nkind=\"unit\"\ntitle=\"Part U-1\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.rs\"]\ndepends_on=[\"{dep}\"]\ncheckpoint=\"K-1\"\nestimate=1\n");
    let f = fx.root.join("x.toml");
    fs::write(&f, toml).unwrap();
    fx.must(&["record", "put", "unit", "--file", f.to_str().unwrap(), "--replace"]);
    let keep = o["impact_ids"].clone();
    let n = fx.root.join("narrow.json");
    fs::write(&n, json!({"amend": "A-1", "keep": keep, "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", n.to_str().unwrap()]);
    fx.human("Yes, approve the change.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the change"]);
}

#[test]
fn a_holder_that_has_not_started_is_put_after_the_asker() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx, "U-1");
    let before = records(&fx);
    let o = ask(&fx, &wc, "beside/y.txt");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["state"], "running", "{}", o.all());
    let ord = orders(&fx);
    assert_eq!(ord.len(), 1, "{ord:?}");
    assert_eq!(ord[0]["unit"], "U-3");
    assert_eq!(ord[0]["after"], "U-1");
    assert_eq!(ord[0]["path"], "beside/y.txt");
    assert!(log(&fx).contains("\"unit.order\""));
    // The request goes to a scope reviewer.
    assert!(directives(&fx)
        .iter()
        .any(|d| d["unit"] == "U-1" && d["role"] == "reviewer"));
    // B is not offered until A lands; the reason names the asker, the file and the event.
    assert!(!offered(&fx, "U-3"));
    let n = fx.must(&["next"]).all();
    let line = n
        .lines()
        .find(|l| l.contains("U-3: it comes after U-1"))
        .unwrap_or_else(|| panic!("no waiting line for U-3: {n}"));
    assert!(line.contains("beside/y.txt") && line.contains("event"), "{line}");
    // The records are unchanged.
    assert_eq!(records(&fx), before);
    // Asking again writes no second order.
    ask(&fx, &wc, "beside/y.txt");
    assert_eq!(orders(&fx).len(), 1);
    // The reviewer grants the file and A lands; B is offered then.
    let list = json!([{"path": "beside/y.txt", "verdict": "needed", "why": "the change needs it"}]);
    let j = fx.bf_stdin(
        &wc,
        &["report", "U-1", "--role", "reviewer", "--result", "scope", "--verdict", "-", "--json"],
        &list.to_string(),
    );
    assert_eq!(j.code, 0, "{}", j.all());
    write(&wc, "beside/y.txt", "y\n");
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.rs"]);
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
    assert!(offered(&fx, "U-3"), "{:?}", directives(&fx));
}

#[test]
fn a_holder_that_has_started_keeps_the_refusal() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx, "U-1");
    start(&fx, "U-3");
    let o = ask(&fx, &wc, "beside/y.txt");
    assert_eq!(o.code, 0, "{}", o.all());
    let v = o.json();
    assert_eq!(v["state"], "blocked", "{v}");
    assert!(v["refused"][0].to_string().contains("U-3 holds it"), "{v}");
    assert!(orders(&fx).is_empty());
    assert!(!log(&fx).contains("\"unit.order\""));
}

#[test]
fn an_asker_that_depends_on_the_holder_through_another_unit_gets_no_order() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx, "U-1");
    depend_on(&fx, "U-4");
    let wc = if wc.exists() { wc } else { start(&fx, "U-1") };
    let o = ask(&fx, &wc, "beside/y.txt");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["state"], "blocked", "{}", o.all());
    assert!(orders(&fx).is_empty());
}

#[test]
fn an_amendment_that_makes_the_asker_depend_on_the_holder_ignores_the_order() {
    let fx = Fx::new();
    project(&fx);
    let wc = start(&fx, "U-1");
    ask(&fx, &wc, "beside/y.txt");
    assert!(!offered(&fx, "U-3"));
    depend_on(&fx, "U-3");
    // The order stays in the log but the records win: B is ready again.
    assert_eq!(orders(&fx).len(), 1);
    assert!(offered(&fx, "U-3"), "{:?}", directives(&fx));
}
