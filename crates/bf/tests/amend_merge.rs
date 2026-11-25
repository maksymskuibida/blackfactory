//! U-111: inside an amendment a checkpoint that has not started is merged into the one right before
//! it (D-127). The later checkpoints take one number lower and the record of the highest number is
//! removed at the approval, as a removed unit is.

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;

fn put_ok(fx: &Fx, kind: &str, text: &str) {
    let o = fx.put(kind, text);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// Four parts, four units, four checkpoints: `U-n` is on `K-n` and has the check `X-n`; the check of
/// the whole checkpoint `K-n` is `X-(n+4)`. `extra` is put into K-3's record (`extra2` into K-2's), `dep` is the
/// `depends_on` line of U-2, and `share` makes U-2 and U-3 name the same file.
fn plan4(fx: &Fx, extra: &str, dep: &str, share: bool) {
    plan4_with(fx, extra, "", dep, share);
}

fn plan4_with(fx: &Fx, extra: &str, extra2: &str, dep: &str, share: bool) {
    for n in 1..=4 {
        put_ok(
            fx,
            "requirement",
            &format!("id=\"R-{n}\"\nkind=\"requirement\"\ntitle=\"Part {n}\"\nacceptance=[\"src/{n}.txt exists\"]\n"),
        );
        let file = if share && (n == 2 || n == 3) {
            "src/shared.txt".to_string()
        } else {
            format!("src/{n}.txt")
        };
        let d = if n == 2 { dep } else { "" };
        put_ok(
            fx,
            "unit",
            &format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"Unit {n}\"\ndelivers=[\"R-{n}\"]\nfiles=[\"{file}\"]\n{d}checkpoint=\"K-{n}\"\nestimate=1\n"),
        );
        put_ok(
            fx,
            "check",
            &format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"Check {n}\"\nhow=\"command\"\ncommand=\"test -f src/{n}.txt\"\nproves=[\"R-{n}\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n"),
        );
        let x = n + 4;
        put_ok(
            fx,
            "check",
            &format!("id=\"X-{x}\"\nkind=\"check\"\ntitle=\"Whole {n}\"\nhow=\"command\"\ncommand=\"test -f src/{n}.txt\"\nproves=[\"R-{n}\"]\nscope=\"checkpoint\"\n"),
        );
        let more = match n {
            3 => extra,
            2 => extra2,
            _ => "",
        };
        let stop = if n == 4 { "pull_request=true\n" } else { "" };
        put_ok(
            fx,
            "checkpoint",
            &format!("id=\"K-{n}\"\nkind=\"checkpoint\"\ntitle=\"Step {n}\"\nafter=[\"U-{n}\"]\nproves=[\"R-{n}\"]\nchecks=[\"X-{x}\"]\nstop_why=\"Why {n}.\"\n{stop}{more}"),
        );
    }
    fx.commit_all("records");
}

fn rec(fx: &Fx, id: &str) -> Value {
    fx.must(&["record", "get", id, "--json"]).json()
}

fn list(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} in {v}"))
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn open(fx: &Fx) -> Value {
    fx.must(&["amend", "open", "--repair", "K-2", "--json"])
        .json()
}

/// Narrow the open amendment with its whole impact set; returns the output of the narrowing.
fn narrow(fx: &Fx, opened: &Value) -> Value {
    let keep: Vec<String> = list(opened, "impact_ids");
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let log = bf::events::load_log(&fx.project()).unwrap();
    log.events
        .iter()
        .rev()
        .find(|e| e["kind"] == "amend.narrow")
        .cloned()
        .expect("a narrowing in the log")
}

fn approve(fx: &Fx, opened: &Value) {
    narrow(fx, opened);
    fx.human("Yes, approve the merge of the checkpoints.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the merge"]);
}

fn page_rows(fx: &Fx) -> Vec<Value> {
    let v = fx.must(&["view", "run", "--json"]).json();
    let plan = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "A-1-plan")
        .unwrap_or_else(|| panic!("{v}"))
        .clone();
    plan["rows"].as_array().unwrap().clone()
}

#[test]
fn a_merge_puts_the_units_and_proven_parts_on_one_checkpoint_and_renames_the_rest() {
    let fx = Fx::new();
    plan4(&fx, "stop=true\nhuman_step=\"Look at 3.\"\n", "", false);
    fx.start_test_run();
    let opened = open(&fx);
    let o = fx
        .must(&["amend", "checkpoint", "merge", "K-2", "K-3", "--json"])
        .json();
    assert_eq!(o["renamed"], json!([{"from": "K-4", "to": "K-3"}]), "{o}");
    assert_eq!(o["plan_problems"], json!([]), "{o}");
    // Nothing is written before the human approves.
    assert_eq!(rec(&fx, "K-4")["title"], "Step 4");
    assert_eq!(rec(&fx, "U-3")["checkpoint"], "K-3");
    approve(&fx, &opened);

    let k2 = rec(&fx, "K-2");
    assert_eq!(list(&k2, "after"), ["U-2", "U-3"]);
    assert_eq!(list(&k2, "proves"), ["R-2", "R-3"]);
    assert_eq!(list(&k2, "checks"), ["X-6", "X-7"]);
    assert_eq!(k2["title"], "Step 2");
    assert_eq!(k2["stop"], true);
    assert_eq!(k2["stop_why"], "Why 2. Why 3.");
    assert_eq!(k2["human_step"], "Look at 3.");
    assert_eq!(rec(&fx, "U-3")["checkpoint"], "K-2");
    // The last checkpoint is now K-3, with its unit; the file of K-4 is gone.
    let k3 = rec(&fx, "K-3");
    assert_eq!(k3["title"], "Step 4");
    assert_eq!(k3["pull_request"], true);
    assert_eq!(list(&k3, "after"), ["U-4"]);
    assert_eq!(rec(&fx, "U-4")["checkpoint"], "K-3");
    assert!(!fx.root.join("factory/records/K-4.toml").exists());
    fx.must(&["plan", "check"]);
}

#[test]
fn the_title_comes_from_the_option_and_the_text_of_both_is_kept() {
    let fx = Fx::new();
    plan4(&fx, "text=\"Second part.\"\n", "", false);
    let opened = open(&fx);
    fx.must(&[
        "amend",
        "checkpoint",
        "merge",
        "K-2",
        "K-3",
        "--title",
        "Joined",
    ]);
    approve(&fx, &opened);
    let k2 = rec(&fx, "K-2");
    assert_eq!(k2["title"], "Joined");
    assert_eq!(k2["text"], "Second part.");
}

#[test]
fn before_the_approval_the_last_record_is_removed_in_the_narrowing_and_gone_on_the_page() {
    let fx = Fx::new();
    plan4(&fx, "", "", false);
    fx.start_test_run();
    let opened = open(&fx);
    // The same amendment with no merge, narrowed.
    let plain = narrow(&fx, &opened);
    fx.must(&["amend", "checkpoint", "merge", "K-2", "K-3"]);
    let merged = narrow(&fx, &opened);
    assert_ne!(plain["set_hash"], merged["set_hash"], "{merged}");
    let changes = merged["changes"]
        .as_array()
        .unwrap_or_else(|| panic!("{merged}"));
    let k4 = changes
        .iter()
        .find(|c| c["id"] == "K-4")
        .unwrap_or_else(|| panic!("{merged}"));
    assert_eq!(k4["how"], "removed");
    assert_eq!(k4["was"]["title"], "Step 4");
    assert!(k4["record"].is_null());
    // The page shows it as gone, with its old content.
    let rows = page_rows(&fx);
    let gone = rows
        .iter()
        .find(|r| r["handled"]["text"] == "gone")
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert_eq!(gone["title"], "Step 4");
    let moved = rows.iter().find(|r| r["id"] == "K-4").unwrap();
    assert_eq!(moved["handled"]["text"], "moved to K-3");
    // The file is still there until the approval.
    assert!(fx.root.join("factory/records/K-4.toml").exists());
}

#[test]
fn after_the_approval_the_removal_reads_as_approved() {
    let fx = Fx::new();
    plan4(&fx, "", "", false);
    fx.start_test_run();
    let opened = open(&fx);
    fx.must(&["amend", "checkpoint", "merge", "K-2", "K-3"]);
    approve(&fx, &opened);
    assert!(!fx.root.join("factory/records/K-4.toml").exists());
    let log = fs::read_to_string(fx.root.join("factory/log/events.jsonl")).unwrap();
    let ev: Value = log
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["kind"] == "amend.approve")
        .next_back()
        .unwrap();
    assert_eq!(ev["content"]["K-4"]["h"], "missing", "{ev}");
    assert_eq!(ev["renamed"], json!([["K-4", "K-3"]]));
    let o = fx.must(&["plan", "check"]);
    assert!(o.all().contains("plan ok"), "{}", o.all());
    let d = fx.must(&["doctor"]);
    assert!(!d.all().to_lowercase().contains("drift"), "{}", d.all());
}

#[test]
fn a_started_first_checkpoint_takes_a_merge_and_a_moved_unit_waits_for_it() {
    let fx = Fx::new();
    plan4(&fx, "", "depends_on=[\"U-1\"]\n", false);
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    let opened = fx
        .must(&["amend", "open", "--repair", "K-1", "--json"])
        .json();
    fx.must(&["amend", "checkpoint", "merge", "K-1", "K-2"]);
    approve(&fx, &opened);
    assert_eq!(list(&rec(&fx, "K-1"), "after"), ["U-1", "U-2"]);
    assert_eq!(rec(&fx, "U-2")["checkpoint"], "K-1");
    // U-2 depends on U-1, which has not landed.
    let o = fx.refused(&["unit", "start", "U-2"]);
    assert!(o.stderr.contains("U-1"), "{}", o.stderr);
}

#[test]
fn units_of_the_merged_checkpoint_that_share_a_file_are_put_in_order() {
    let fx = Fx::new();
    plan4(&fx, "", "", true);
    let opened = open(&fx);
    fx.must(&["amend", "checkpoint", "merge", "K-2", "K-3"]);
    approve(&fx, &opened);
    let u3 = rec(&fx, "U-3");
    assert!(list(&u3, "depends_on").contains(&"U-2".to_string()), "{u3}");
    fx.must(&["plan", "check"]);
}

fn assert_refused(fx: &Fx, args: &[&str], text: &str) {
    let mut a = vec!["amend", "checkpoint"];
    a.extend_from_slice(args);
    let o = fx.refused(&a);
    assert!(o.stderr.contains(text), "{}", o.stderr);
    // No record was written.
    assert_eq!(rec(fx, "K-2")["title"], "Step 2");
    assert_eq!(rec(fx, "U-3")["checkpoint"], "K-3");
    assert!(fx.root.join("factory/records/K-4.toml").exists());
}

#[test]
fn a_merge_is_refused_for_a_checkpoint_that_is_not_right_after() {
    let fx = Fx::new();
    plan4(&fx, "", "", false);
    open(&fx);
    assert_refused(
        &fx,
        &["merge", "K-2", "K-4"],
        "is not the checkpoint right after",
    );
}

#[test]
fn a_merge_is_refused_for_a_started_second_checkpoint() {
    let fx = Fx::new();
    plan4(&fx, "", "", false);
    fx.start_test_run();
    let p = fx.project();
    for (u, k) in [("U-1", "K-1"), ("U-2", "K-2")] {
        bf::events::append(&p, "unit.land", json!({"unit": u, "hash": "h", "run": "r"})).unwrap();
        bf::events::append(
            &p,
            "checkpoint.close",
            json!({"checkpoint": k, "merged": false, "hash": "h"}),
        )
        .unwrap();
    }
    fx.must(&["unit", "start", "U-3"]);
    open(&fx);
    assert_refused(&fx, &["merge", "K-2", "K-3"], "has started");
}

#[test]
fn a_merge_is_refused_for_a_closed_first_checkpoint() {
    let fx = Fx::new();
    plan4(&fx, "", "", false);
    fx.start_test_run();
    let p = fx.project();
    bf::events::append(
        &p,
        "unit.land",
        json!({"unit": "U-1", "hash": "h", "run": "r"}),
    )
    .unwrap();
    bf::events::append(
        &p,
        "checkpoint.close",
        json!({"checkpoint": "K-1", "merged": false, "hash": "h"}),
    )
    .unwrap();
    fx.must(&["amend", "open", "--repair", "K-2"]);
    let o = fx.refused(&["amend", "checkpoint", "merge", "K-1", "K-2"]);
    assert!(o.stderr.contains("K-1 is closed"), "{}", o.stderr);
    assert_eq!(rec(&fx, "K-1")["title"], "Step 1");
    assert_eq!(rec(&fx, "U-2")["checkpoint"], "K-2");
}

#[test]
fn a_merge_is_refused_when_the_unit_check_lists_differ_and_taken_when_they_agree() {
    let fx = Fx::new();
    let l = |name: &str| {
        format!("unit_checks = [{{ name = \"{name}\", command = \"true\" }}]\nunit_checks_reason = \"Because.\"\n")
    };
    plan4_with(&fx, &l("three"), &l("two"), "", false);
    open(&fx);
    // Both lists are named in the refusal.
    let o = fx.refused(&["amend", "checkpoint", "merge", "K-2", "K-3"]);
    assert!(
        o.stderr.contains("two") && o.stderr.contains("three"),
        "{}",
        o.stderr
    );
    assert_eq!(rec(&fx, "U-3")["checkpoint"], "K-3");

    let fx = Fx::new();
    plan4_with(&fx, &l("same"), &l("same"), "", false);
    let opened = open(&fx);
    fx.must(&["amend", "checkpoint", "merge", "K-2", "K-3"]);
    approve(&fx, &opened);
    assert_eq!(rec(&fx, "K-2")["unit_checks"][0]["name"], "same");

    let fx = Fx::new();
    plan4_with(&fx, &l("only"), "", "", false);
    let opened = open(&fx);
    fx.must(&["amend", "checkpoint", "merge", "K-2", "K-3"]);
    approve(&fx, &opened);
    assert_eq!(rec(&fx, "K-2")["unit_checks"][0]["name"], "only");
}

#[test]
fn a_withdrawn_amendment_that_held_a_merge_leaves_every_record_as_it_was() {
    let fx = Fx::new();
    plan4(&fx, "", "", false);
    let before: Vec<(String, Vec<u8>)> = fs::read_dir(fx.root.join("factory/records"))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_string_lossy().to_string(),
                fs::read(&p).unwrap(),
            )
        })
        .collect();
    let opened = open(&fx);
    fx.must(&["amend", "checkpoint", "merge", "K-2", "K-3"]);
    narrow(&fx, &opened);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "changed my mind"]);
    let mut after: Vec<(String, Vec<u8>)> = fs::read_dir(fx.root.join("factory/records"))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_string_lossy().to_string(),
                fs::read(&p).unwrap(),
            )
        })
        .collect();
    let mut before = before;
    before.sort();
    after.sort();
    assert_eq!(before, after);
}

#[test]
fn the_refusal_with_no_valid_operation_names_merge() {
    let fx = Fx::new();
    plan4(&fx, "", "", false);
    open(&fx);
    let o = fx.refused(&["amend", "checkpoint", "merge", "K-2"]);
    assert!(o.stderr.contains("merge"), "{}", o.stderr);
}

/// Four parts, four units, three checkpoints: K-1 holds U-1, K-2 holds U-2 and U-3, K-3 holds U-4.
fn plan_three(fx: &Fx) {
    for n in 1..=4 {
        put_ok(
            fx,
            "requirement",
            &format!("id=\"R-{n}\"\nkind=\"requirement\"\ntitle=\"Part {n}\"\nacceptance=[\"src/{n}.txt exists\"]\n"),
        );
        let k = [1, 2, 2, 3][n - 1];
        put_ok(
            fx,
            "unit",
            &format!("id=\"U-{n}\"\nkind=\"unit\"\ntitle=\"Unit {n}\"\ndelivers=[\"R-{n}\"]\nfiles=[\"src/{n}.txt\"]\ncheckpoint=\"K-{k}\"\nestimate=1\n"),
        );
        put_ok(
            fx,
            "check",
            &format!("id=\"X-{n}\"\nkind=\"check\"\ntitle=\"Check {n}\"\nhow=\"command\"\ncommand=\"test -f src/{n}.txt\"\nproves=[\"R-{n}\"]\nscope=\"unit\"\nunit=\"U-{n}\"\n"),
        );
    }
    for (x, k, proves, cmd) in [
        (5, 1, "\"R-1\"", "test -f src/1.txt"),
        (6, 2, "\"R-2\",\"R-3\"", "ls src/2.txt src/3.txt"),
        (7, 3, "\"R-4\"", "test -f src/4.txt"),
    ] {
        put_ok(
            fx,
            "check",
            &format!("id=\"X-{x}\"\nkind=\"check\"\ntitle=\"Whole {k}\"\nhow=\"command\"\ncommand=\"{cmd}\"\nproves=[{proves}]\nscope=\"checkpoint\"\n"),
        );
    }
    for (k, after, proves, x) in [
        (1, "\"U-1\"", "\"R-1\"", 5),
        (2, "\"U-2\",\"U-3\"", "\"R-2\",\"R-3\"", 6),
        (3, "\"U-4\"", "\"R-4\"", 7),
    ] {
        put_ok(
            fx,
            "checkpoint",
            &format!("id=\"K-{k}\"\nkind=\"checkpoint\"\ntitle=\"Step {k}\"\nafter=[{after}]\nproves=[{proves}]\nchecks=[\"X-{x}\"]\nstop_why=\"No stop.\"\n"),
        );
    }
    fx.commit_all("records");
}

/// A log written before merges existed holds an insertion. Replaying it still reads its renames, its
/// touched IDs and its approved content as it did, and no record reads as removed.
#[test]
fn a_log_written_before_merges_replays_to_the_same_state() {
    let fx = Fx::new();
    plan_three(&fx);
    fx.start_test_run();
    let opened = fx
        .must(&["amend", "open", "--repair", "K-2", "--json"])
        .json();
    fx.must(&["amend", "checkpoint", "after", "K-1", "U-3"]);
    approve(&fx, &opened);
    let log = bf::events::load_log(&fx.project()).unwrap();
    let find = |kind: &str| {
        log.events
            .iter()
            .find(|e| e["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind}"))
            .clone()
    };
    let (op_ev, approved) = (find("amend.checkpoint"), find("amend.approve"));
    let strs = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    // The renames the event holds: K-2 to K-3 and K-3 to K-4, and no record is removed.
    assert_eq!(op_ev["renamed"], json!([["K-2", "K-3"], ["K-3", "K-4"]]));
    assert_eq!(approved["renamed"], op_ev["renamed"]);
    for e in [
        &op_ev["touched"],
        &json!(approved["content"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>()),
    ] {
        assert!(!strs(e).is_empty());
    }
    for id in strs(&op_ev["touched"]) {
        let entry = &approved["content"][id.as_str()];
        assert!(!entry.is_null(), "{id} is not in the approved content");
        assert_ne!(entry["h"], "missing", "{id} reads as removed");
    }
    for k in ["K-1", "K-2", "K-3", "K-4"] {
        assert!(fx.root.join(format!("factory/records/{k}.toml")).exists());
    }
    // The state read from that log.
    let st = bf::events::replay(&fx.project()).unwrap();
    let a = &st.amends[0];
    assert_eq!(a.status, "approved");
    assert_eq!(a.ops, vec![op_ev["op"].clone()]);
    assert_eq!(a.touched, strs(&op_ev["touched"]));
    assert_eq!(
        bf::events::replay(&fx.project()).unwrap().amends[0].approved,
        a.approved
    );
    let d = fx.must(&["doctor"]);
    assert!(!d.all().to_lowercase().contains("drift"), "{}", d.all());
    fx.must(&["plan", "check"]);
}
