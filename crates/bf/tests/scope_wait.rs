//! U-101 (A-73): a unit refused a file that a unit beside it holds waits, and goes back to work by
//! itself when the holder lands or is put after it (R-107, D-82).

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// `n` independent units of one checkpoint; unit `U-i` holds `src/f<i>.txt`.
fn project(fx: &Fx, n: usize) {
    let mut recs: Vec<(&str, String)> = vec![(
        "component",
        "id=\"C-1\"\nkind=\"component\"\ntitle=\"Parts\"\n".into(),
    )];
    let (mut ids, mut reqs) = (Vec::new(), Vec::new());
    for i in 1..=n {
        let c = if i == 1 { ",\"C-1\"" } else { "" };
        recs.push(("requirement", format!("id=\"R-{i}\"\nkind=\"requirement\"\ntitle=\"Part {i}\"\nacceptance=[\"src/f{i}.txt exists\"]\n")));
        recs.push(("unit", format!("id=\"U-{i}\"\nkind=\"unit\"\ntitle=\"Part {i}\"\ndelivers=[\"R-{i}\"{c}]\nfiles=[\"src/f{i}.txt\"]\ncheckpoint=\"K-1\"\nestimate=1\n")));
        recs.push(("check", format!("id=\"X-{i}\"\nkind=\"check\"\ntitle=\"File {i}\"\nhow=\"command\"\ncommand=\"test -f src/f{i}.txt\"\nproves=[\"R-{i}\"{c}]\nscope=\"unit\"\nunit=\"U-{i}\"\n")));
        ids.push(format!("\"U-{i}\""));
        reqs.push(format!("\"R-{i}\""));
    }
    let all = format!("{},\"C-1\"", reqs.join(","));
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

fn start(fx: &Fx, id: &str) -> PathBuf {
    let o = fx.must(&["unit", "start", id, "--json"]);
    PathBuf::from(o.json()["working_copy"].as_str().unwrap())
}

/// Run `bf` with the words of `line` as arguments, in `dir`, with `input` on stdin.
fn bf(fx: &Fx, dir: &Path, line: &str, input: &str) -> Out {
    let o = fx.bf_stdin(dir, &line.split(' ').collect::<Vec<_>>(), input);
    assert_eq!(o.code, 0, "{line}: {}", o.all());
    o
}

fn land(fx: &Fx, n: usize, wc: &Path) {
    fs::write(wc.join(format!("src/f{n}.txt")), "content\n").unwrap();
    let ev = format!("evidence run X-{n} --unit U-{n} -- test -f src/f{n}.txt");
    let rv = format!("report U-{n} --result built --role reviewer");
    for (dir, line) in [(wc, ev), (&fx.root, format!("report U-{n} --result built"))] {
        bf(fx, dir, &line, "");
    }
    bf(fx, wc, &rv, "");
    bf(fx, &fx.root, &format!("unit land U-{n}"), "");
}

fn ask(fx: &Fx, id: &str, wc: &Path, paths: &[&str]) -> Value {
    let p = "--- a\n+++ b\n@@ -1 +1 @@\n-a\n+b\n";
    let l: Vec<Value> = paths.iter().map(|f| json!({"path": f, "why": "needed", "patch": p})).collect();
    let a = format!("report {id} --result blocked --need-files - --json");
    bf(fx, wc, &a, &json!(l).to_string()).json()
}

fn next(fx: &Fx) -> Value {
    fx.must(&["next", "--json"]).json()
}

fn state(fx: &Fx, id: &str) -> String {
    bf::events::replay(&fx.project()).unwrap().unit_state(id).to_string()
}

fn directive(n: &Value, unit: &str, role: &str) -> Option<Value> {
    let d = n["directives"].as_array().unwrap();
    d.iter().find(|d| d["unit"] == unit && d["role"] == role).cloned()
}

fn waiting_line(n: &Value, unit: &str) -> Option<String> {
    let p = format!("{unit}: waiting for");
    let w = n["waiting"].as_array().unwrap();
    w.iter().filter_map(|l| l.as_str()).find(|l| l.starts_with(&p)).map(String::from)
}

/// A (U-1) is blocked, waiting for B's (U-2) file; returns A's and B's working copies.
fn a_waits_for_b(fx: &Fx) -> (PathBuf, PathBuf) {
    project(fx, 2);
    let (wa, wb) = (start(fx, "U-1"), start(fx, "U-2"));
    let v = ask(fx, "U-1", &wa, &["src/f2.txt"]);
    assert_eq!(v["state"], "blocked", "{v}");
    let n = next(fx);
    let line = waiting_line(&n, "U-1").unwrap_or_else(|| panic!("no waiting line: {n}"));
    assert!(
        line.contains("src/f2.txt") && line.contains("U-2"),
        "{line}"
    );
    assert!(directive(&n, "U-1", "human").is_none(), "{n}");
    (wa, wb)
}

fn counts(fx: &Fx, id: &str) -> (u32, u32) {
    let st = bf::events::replay(&fx.project()).unwrap();
    (st.units[id].rounds, st.units[id].attempts)
}

#[test]
fn a_unit_waits_for_a_file_a_unit_beside_it_holds_and_resumes_when_that_unit_lands() {
    let fx = Fx::new();
    let (_wa, wb) = a_waits_for_b(&fx);
    let before = counts(&fx, "U-1");
    assert_eq!(before, (0, 1));
    land(&fx, 2, &wb);
    assert_eq!(state(&fx, "U-1"), "running");
    assert_eq!(counts(&fx, "U-1"), before);
    let d = directive(&next(&fx), "U-1", "builder").unwrap();
    assert_eq!(d["ask_again"], json!(["src/f2.txt"]), "{d}");
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    assert!(
        log.contains("unit.scope.resume") && !log.contains("unit.release"),
        "{log}"
    );
}

fn amend_after(fx: &Fx) {
    let opened = fx.must(&["amend", "open", "--repair", "U-2", "--json"]).json();
    let u2 = "id=\"U-2\"\nkind=\"unit\"\ntitle=\"Part 2\"\ndelivers=[\"R-2\"]\nfiles=[\"src/f2.txt\"]\ndepends_on=[\"U-1\"]\ncheckpoint=\"K-1\"\nestimate=1\nfull_suite=\"fixture\"\n";
    let f = fx.root.join("u2.toml");
    fs::write(&f, u2).unwrap();
    let put = format!("record put unit --file {} --replace", f.to_str().unwrap());
    fx.must(&put.split(' ').collect::<Vec<_>>());
    let n = fx.root.join("narrow.json");
    fs::write(
        &n,
        json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", n.to_str().unwrap()]);
    fx.human("Yes, approve the order.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the order"]);
}

#[test]
fn an_amendment_that_puts_the_holder_after_the_unit_resumes_it_too() {
    let fx = Fx::new();
    a_waits_for_b(&fx);
    let before = counts(&fx, "U-1");
    assert_eq!(before, (0, 1));
    amend_after(&fx);
    assert_eq!(state(&fx, "U-1"), "running");
    assert_eq!(counts(&fx, "U-1"), before);
    let d = directive(&next(&fx), "U-1", "builder").unwrap();
    assert_eq!(d["ask_again"], json!(["src/f2.txt"]), "{d}");
}

fn human_note(fx: &Fx, id: &str) -> String {
    let d =
        directive(&next(fx), id, "human").unwrap_or_else(|| panic!("no human directive for {id}"));
    assert_eq!(d["action"], "decide-unit", "{d}");
    d["note"].as_str().unwrap().to_string()
}

#[test]
fn one_protected_file_among_held_ones_keeps_the_unit_for_the_human() {
    let fx = Fx::new();
    project(&fx, 2);
    let (wa, wb) = (start(&fx, "U-1"), start(&fx, "U-2"));
    let v = ask(&fx, "U-1", &wa, &["src/f2.txt", "factory/note.txt"]);
    assert_eq!(v["state"], "blocked", "{v}");
    assert!(waiting_line(&next(&fx), "U-1").is_none());
    human_note(&fx, "U-1");
    land(&fx, 2, &wb);
    assert_eq!(state(&fx, "U-1"), "blocked");
}

#[test]
fn an_extension_verdict_stays_blocked() {
    let fx = Fx::new();
    project(&fx, 2);
    let (wa, wb) = (start(&fx, "U-1"), start(&fx, "U-2"));
    ask(&fx, "U-1", &wa, &["src/other.txt"]);
    let verdict =
        json!([{"path": "src/other.txt", "verdict": "extension", "why": "a different feature"}]);
    let a = "report U-1 --role reviewer --result scope --verdict -";
    bf(&fx, &wa, a, &verdict.to_string());
    assert_eq!(state(&fx, "U-1"), "blocked");
    human_note(&fx, "U-1");
    land(&fx, 2, &wb);
    assert_eq!(state(&fx, "U-1"), "blocked");
}

#[test]
fn a_unit_refused_up_to_the_attempt_limit_is_parked_and_not_resumed() {
    let fx = Fx::new();
    project(&fx, 4);
    fx.set_policy("max_attempts = 10", "max_attempts = 3");
    let wa = start(&fx, "U-1");
    let others: Vec<PathBuf> = (2..=4).map(|i| start(&fx, &format!("U-{i}"))).collect();
    for (i, wc) in others.iter().enumerate() {
        let n = i + 2;
        let v = ask(&fx, "U-1", &wa, &[&format!("src/f{n}.txt")]);
        assert_eq!(v["state"], "blocked", "{v}");
        land(&fx, n, wc);
        let want = if n < 4 { "running" } else { "parked" };
        assert_eq!(state(&fx, "U-1"), want);
    }
    assert_eq!(counts(&fx, "U-1"), (0, 3));
    // The note of a parked unit names the accept beside the release (U-61).
    let note = human_note(&fx, "U-1");
    assert!(note.contains("bf unit release U-1"), "{note}");
    assert!(note.contains("bf unit accept U-1"), "{note}");
}

/// The directive of the circle: the conductor's, with the ring and the files.
fn circle_directive(fx: &Fx, id: &str) -> Value {
    let d = directive(&next(fx), id, "conductor")
        .unwrap_or_else(|| panic!("no conductor directive for {id}"));
    assert_eq!(d["action"], "open-plan-repair", "{d}");
    d
}

#[test]
fn a_circle_of_waits_is_for_the_conductor_in_both_units() {
    let fx = Fx::new();
    project(&fx, 2);
    let (wa, wb) = (start(&fx, "U-1"), start(&fx, "U-2"));
    ask(&fx, "U-1", &wa, &["src/f2.txt"]);
    assert!(waiting_line(&next(&fx), "U-1").is_some());
    ask(&fx, "U-2", &wb, &["src/f1.txt"]);
    let n = next(&fx);
    assert!(
        waiting_line(&n, "U-1").is_none() && waiting_line(&n, "U-2").is_none(),
        "{n}"
    );
    assert!(directive(&n, "U-1", "human").is_none() && directive(&n, "U-2", "human").is_none(), "{n}");
    for (id, other) in [("U-1", "U-2"), ("U-2", "U-1")] {
        let d = circle_directive(&fx, id);
        let note = d["note"].as_str().unwrap();
        assert!(note.contains(&format!("{id} -> {other} -> {id}")), "{note}");
        assert!(note.contains("bf amend open --repair"), "{note}");
        assert!(note.contains("src/f1.txt") || note.contains("src/f2.txt"), "{note}");
        assert!(!note.contains("is the human's"), "{note}");
    }
}

#[test]
fn an_open_repair_for_the_circle_is_the_step_in_hand() {
    let fx = Fx::new();
    project(&fx, 2);
    let (wa, wb) = (start(&fx, "U-1"), start(&fx, "U-2"));
    ask(&fx, "U-1", &wa, &["src/f2.txt"]);
    ask(&fx, "U-2", &wb, &["src/f1.txt"]);
    circle_directive(&fx, "U-1");
    fx.must(&["amend", "open", "--repair", "U-2"]);
    let n = next(&fx);
    assert!(directive(&n, "U-1", "conductor").is_none(), "{n}");
    assert!(
        n["waiting"].as_array().unwrap().iter().any(|l| l.as_str().is_some_and(|l| l.contains("A-1"))),
        "{n}"
    );
}
