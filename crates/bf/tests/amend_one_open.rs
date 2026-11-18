//! U-18: only one amendment is open at a time (R-38). It is revised or withdrawn before a new one opens.

mod common;
use common::Fx;
use std::fs;

fn narrow_all(fx: &Fx, a: &str, opened: &serde_json::Value) {
    let f = fx.root.join("narrow.json");
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

fn amendments(fx: &Fx) -> Vec<serde_json::Value> {
    fx.must(&["status", "--json"]).json()["amendments"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn opening_a_second_amendment_is_refused_and_names_the_open_one() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    // Same records or other records: either way it is refused.
    for ids in [["R-2"], ["R-1"]] {
        let o = fx.refused(&["amend", "open", ids[0]]);
        assert!(o.stderr.contains("A-1"), "{}", o.stderr);
        assert!(o.stderr.contains("revise"), "{}", o.stderr);
        assert!(o.stderr.contains("withdraw"), "{}", o.stderr);
    }
    assert_eq!(amendments(&fx).len(), 1);
}

#[test]
fn revising_changes_the_records_and_keeps_the_id() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    let o = fx.must(&["amend", "revise", "R-1", "--json"]).json();
    assert_eq!(o["amend"], "A-1");
    let a = amendments(&fx);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["id"], "A-1");
    assert_eq!(a[0]["status"], "open");
    assert_eq!(a[0]["ids"], serde_json::json!(["R-1"]));
    // Still one open amendment: a new one is refused.
    fx.refused(&["amend", "open", "R-2"]);
}

#[test]
fn revising_without_an_open_amendment_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    fx.refused(&["amend", "revise", "R-1"]);
}

#[test]
fn a_revised_amendment_needs_a_human_message_after_the_revision() {
    let fx = Fx::new();
    fx.tiny();
    // U-19: both sets cover every stage and link their requirement, so only the revision stops approval.
    for (k, t) in [
        ("component", "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n"),
        ("component", "id=\"C-8\"\nkind=\"component\"\ntitle=\"Greeting wording\"\nlinks=[\"R-1\"]\n"),
        ("decision", "id=\"D-91\"\nkind=\"decision\"\ntitle=\"The screens are not affected: none shows the wording\"\nmaker=\"agent\"\nstage=\"ui\"\n"),
        ("decision", "id=\"D-92\"\nkind=\"decision\"\ntitle=\"The plan is not affected: the units already exist\"\nmaker=\"agent\"\nstage=\"plan\"\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, "A-1", &opened);
    // The human says yes to the narrowed amendment, then it is revised.
    fx.human("Yes, approve the change to the second requirement.");
    let revised = fx
        .must(&["amend", "revise", "R-1", "C-8", "D-91", "D-92", "--json"])
        .json();
    // The earlier message does not count, and the revised amendment must be narrowed again.
    fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    narrow_all(&fx, "A-1", &revised);
    fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    fx.human("Yes, approve the revised change.");
    fx.must(&[
        "amend",
        "approve",
        "A-1",
        "--quote",
        "approve the revised change",
    ]);
    assert_eq!(amendments(&fx)[0]["status"], "approved");
}

#[test]
fn withdrawing_makes_room_for_a_new_amendment() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["amend", "open", "R-2"]);
    fx.refused(&["amend", "open", "R-1"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "wrong records"]);
    let o = fx.must(&["amend", "open", "R-1", "--json"]).json();
    assert_eq!(o["amend"], "A-2");
    let a = amendments(&fx);
    assert_eq!(a[0]["status"], "withdrawn");
    assert_eq!(a[1]["status"], "open");
}

const NEW_C: &str = "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n";

fn put_decision(fx: &Fx, id: &str, stage: &str) {
    let o = fx.put(
        "decision",
        &format!("id=\"{id}\"\nkind=\"decision\"\ntitle=\"The {stage} stage is not affected\"\nmaker=\"agent\"\nstage=\"{stage}\"\n"),
    );
    assert_eq!(o.code, 0, "{}", o.all());
}

/// R-43: narrowing and approving are refused while a named record has no content; a new record put
/// while the amendment waits is shown as new with every field.
#[test]
fn amend_one_open_a_named_record_with_no_content_blocks_narrow_and_approve() {
    let fx = Fx::new();
    fx.tiny();
    put_decision(&fx, "D-91", "ui");
    put_decision(&fx, "D-92", "plan");
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    let f = fx.root.join("narrow.json");
    let write_narrow = |keep: &serde_json::Value| {
        fs::write(
            &f,
            serde_json::json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
        )
        .unwrap();
    };
    write_narrow(&opened["impact_ids"]);
    let o = fx.refused(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    assert!(o.stderr.contains("C-9"), "{}", o.stderr);
    // The new record is put while the amendment waits.
    let o = fx.put("component", NEW_C);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    // It is shown on the page as new, with every field.
    let v = fx.must(&["view", "run", "--json"]).json();
    let secs = v["sections"].as_array().unwrap();
    let ch = secs
        .iter()
        .find(|s| s["id"] == "A-1-changes")
        .expect("changes");
    let row = ch["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "C-9")
        .expect("C-9 row");
    assert_eq!(row["handled"]["text"], "new", "{row}");
    let body = row["body"].to_string();
    for field in ["title", "links", "kind", "id"] {
        assert!(body.contains(&format!("{field}: added")), "{field}: {body}");
    }
    // The record is taken away again: approval is refused and names it.
    fs::remove_file(fx.root.join("factory/records/C-9.toml")).unwrap();
    fx.human("Yes, approve the change to the second requirement.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert!(o.stderr.contains("C-9"), "{}", o.stderr);
}

/// R-43: after the approval the first put of a record that did not exist then is refused, and the
/// refusal says where new records are named.
#[test]
fn amend_one_open_a_new_record_after_the_approval_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let f = fx.root.join("x.toml");
    fs::write(
        &f,
        "id=\"X-1\"\nkind=\"check\"\ntitle=\"Greeting file\"\nhow=\"command\"\ncommand=\"test -s src/hello.txt\"\nproves=[\"R-1\",\"C-1\",\"H-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n",
    )
    .unwrap();
    fx.must(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let opened = fx.must(&["amend", "revise", "X-1", "--json"]).json();
    narrow_all(&fx, "A-1", &opened);
    fx.human("Yes, approve the new command.");
    fx.must(&[
        "amend",
        "approve",
        "A-1",
        "--quote",
        "approve the new command",
    ]);
    // A new check, named by no amendment: refused, and no content takes force.
    let o = fx.put(
        "check",
        "id=\"X-7\"\nkind=\"check\"\ntitle=\"New\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("bf amend open"), "{}", o.stderr);
    assert!(o.stderr.contains("bf amend revise"), "{}", o.stderr);
}

/// R-43: the refusal of narrowing for an added ID points to revise.
#[test]
fn amend_one_open_narrowing_cannot_add_an_id_and_points_to_revise() {
    let fx = Fx::new();
    fx.tiny();
    let opened = fx.must(&["amend", "open", "R-2", "--json"]).json();
    let mut keep = opened["impact_ids"].as_array().unwrap().clone();
    keep.push(serde_json::json!("R-1"));
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        serde_json::json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    let o = fx.refused(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    assert!(o.stderr.contains("bf amend revise"), "{}", o.stderr);
}

/// R-52, S-26: `bf log --tail` prints the amendment events with the ids of their records.
#[test]
fn amend_one_open_the_log_leaves_record_contents_out_of_amendment_events() {
    let fx = Fx::new();
    fx.tiny();
    put_decision(&fx, "D-91", "ui");
    put_decision(&fx, "D-92", "plan");
    let o = fx.put("component", NEW_C);
    assert_eq!(o.code, 0, "{}", o.all());
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, "A-1", &opened);
    fx.human("Yes, approve the change to the second requirement.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    let o = fx.must(&["log", "--tail", "30"]);
    let mut seen = 0;
    for line in o.stdout.lines() {
        assert!(line.len() <= 2000, "{} bytes", line.len());
        if line.contains("\"amend.narrow\"") || line.contains("\"amend.approve\"") {
            seen += 1;
            assert!(line.contains("\"records\""), "{line}");
            assert!(line.contains("C-9"), "{line}");
            assert!(line.contains("[contents left out]"), "{line}");
            assert!(!line.contains("Farewell wording"), "{line}");
        }
    }
    assert_eq!(seen, 2, "{}", o.stdout);
    assert!(o.stdout.contains("bf log --whole"), "{}", o.stdout);
    // The flag prints an event whole, and the chain is unchanged.
    let n = o
        .stdout
        .lines()
        .find(|l| l.contains("\"amend.narrow\""))
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l.split(" [contents").next().unwrap())
                .unwrap()
        })
        .unwrap()["seq"]
        .as_u64()
        .unwrap();
    let w = fx.must(&["log", "--whole", &n.to_string()]);
    assert!(w.stdout.contains("Farewell wording"), "{}", w.stdout);
    fx.must(&["log", "--verify"]);
    // The recent events of the run page do the same.
    let v = fx.must(&["view", "run", "--json"]).json();
    let ev = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "events")
        .unwrap()
        .to_string();
    assert!(!ev.contains("Farewell wording"), "{ev}");
}

/// R-43 acceptance 4: a log written by an earlier engine holds a record bound after its approval
/// (`amend.bind`); it replays to the same state: the ID is not frozen, and the content bound is the
/// content that may be put again.
#[test]
fn amend_one_open_a_log_with_a_record_bound_after_its_approval_replays_the_same() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    fx.must(&["amend", "open", "--repair", "X-1"]);
    let opened = fx.must(&["amend", "revise", "X-1", "--json"]).json();
    narrow_all(&fx, "A-1", &opened);
    fx.human("Yes, approve the repair.");
    let p = fx.project();
    let st = bf::events::replay(&p).unwrap();
    let turn = st.turns.last().unwrap().seq;
    // An earlier engine approved a page that named X-7 before it had any content.
    bf::events::append(
        &p,
        "amend.approve",
        serde_json::json!({"amend": "A-1", "turn": turn, "quote": "approve the repair",
            "content": {"X-7": {"h": "missing"}},
            "reopen_units": [], "reopen_checkpoints": [], "reopen_checks": [], "receipt_floor": 0}),
    )
    .unwrap();
    let rec = serde_json::json!({"id": "X-7", "kind": "check", "title": "New", "how": "command",
        "command": "true", "proves": ["R-1"], "scope": "unit", "unit": "U-1"});
    let text = "id=\"X-7\"\nkind=\"check\"\ntitle=\"New\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n";
    let before = bf::events::replay(&p).unwrap();
    assert!(bf::gates::approved_missing(&before, "X-7"));
    // Without the binding the first put of the record is refused.
    let o = fx.put("check", text);
    assert_eq!(o.code, 2, "{}", o.all());
    bf::events::append(
        &p,
        "amend.bind",
        serde_json::json!({"id": "X-7", "content_h": bf::evidence::record_hash(&rec), "unit": "U-1"}),
    )
    .unwrap();
    let after = bf::events::replay(&p).unwrap();
    assert!(!bf::gates::approved_missing(&after, "X-7"));
    assert_eq!(
        after.approved_content["X-7"].1["h"],
        bf::evidence::record_hash(&rec)
    );
    assert_eq!(after.approved_content["X-7"].1["unit"], "U-1");
    // The same content is put again; a different one is still refused.
    let o = fx.put("check", text);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.put("check", &text.replace("\"New\"", "\"Other\""));
    assert_eq!(o.code, 2, "{}", o.all());
    fx.must(&["log", "--verify"]);
}

/// R-52: an event of a narrowing with long reasons is still at most 2,000 bytes with its mark, on the
/// log and on the run page, and `--json` leaves the contents out as well.
#[test]
fn amend_one_open_a_narrowing_with_long_reasons_stays_within_the_line_limit() {
    let fx = Fx::new();
    fx.tiny();
    put_decision(&fx, "D-91", "ui");
    put_decision(&fx, "D-92", "plan");
    let o = fx.put("component", NEW_C);
    assert_eq!(o.code, 0, "{}", o.all());
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    let ids: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let named = ["R-2", "C-9", "D-91", "D-92"];
    let dropped: Vec<&String> = ids.iter().filter(|i| !named.contains(&i.as_str())).collect();
    assert!(dropped.len() >= 3, "{ids:?}");
    let keep: Vec<&String> = ids.iter().filter(|i| named.contains(&i.as_str())).collect();
    let reasons: serde_json::Map<String, serde_json::Value> = dropped
        .iter()
        .map(|i| (i.to_string(), serde_json::json!("Long reason. ".repeat(60))))
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        serde_json::json!({"amend": "A-1", "keep": keep, "waive": [], "reasons": reasons})
            .to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let o = fx.must(&["log", "--tail", "3"]);
    let line = o
        .stdout
        .lines()
        .find(|l| l.contains("\"amend.narrow\""))
        .expect("narrow event");
    assert!(line.len() <= 2000, "{} bytes", line.len());
    assert!(line.ends_with("[contents left out]"), "{line}");
    let j = fx.must(&["log", "--tail", "3", "--json"]).stdout;
    assert!(!j.contains("Farewell wording"), "{j}");
    let evs: Vec<serde_json::Value> = serde_json::from_str(&j).unwrap();
    let n = evs.iter().find(|e| e["kind"] == "amend.narrow").expect("narrow");
    assert!(n.get("changes").is_none() && n.get("records").is_some(), "{n}");
    let v = fx.must(&["view", "run", "--json"]).json();
    for s in v["sections"].as_array().unwrap().iter().filter(|s| s["id"] == "events") {
        for r in s["rows"].as_array().unwrap() {
            for b in r["body"].as_array().unwrap() {
                assert!(b["text"].as_str().unwrap().len() <= 2000, "{b}");
            }
        }
    }
}
