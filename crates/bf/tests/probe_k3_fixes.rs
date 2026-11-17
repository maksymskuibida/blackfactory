//! Tests added with the fixes of the K-3 checkpoint review, round 1, beside the stored probes in
//! `probe_k3.rs`: a unit file list in an unapproved repair, the lift bound, and forged review answers.

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

fn not_affected(fx: &Fx, id: &str, stage: &str) {
    put_ok(
        fx,
        "decision",
        &format!("id=\"{id}\"\nkind=\"decision\"\ntitle=\"The {stage} stage is not affected\"\nmaker=\"agent\"\nstage=\"{stage}\"\n"),
    );
}

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

/// p4b: a unit's file list changed inside a plan repair that the human has not approved grants no
/// file: the guard refuses the write, and the unit does not land on it.
#[test]
fn probe_p4b_a_file_list_changed_in_an_unapproved_repair_grants_nothing() {
    let fx = Fx::new();
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fx.must(&["amend", "open", "--repair", "U-1"]);
    let f = fx.root.join("u1.toml");
    fs::write(&f, "id=\"U-1\"\nkind=\"unit\"\ntitle=\"Greeting\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"src/hello.txt\",\"src/other.txt\"]\ncheckpoint=\"K-1\"\nestimate=2\n").unwrap();
    let put = fx.bf(&["record", "put", "unit", "--file", f.to_str().unwrap(), "--replace"]);
    let _ = fs::remove_file(&f);
    fs::create_dir_all(wc.join("src")).unwrap();
    let (d, why) = judge(&fx, &wc, "echo x > src/other.txt");
    assert_eq!(d, "deny", "put {}: the draft file list granted a write: {why}", put.code);
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    let run = fx.bf_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/hello.txt"]);
    fx.bf(&["report", "U-1", "--result", "built"]);
    fx.bf(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    let land = fx.bf(&["unit", "land", "U-1"]);
    assert!(land.code != 0, "run {} land {}: {}", run.code, land.code, land.all());
}

/// The designer lift takes words written after the amendment was opened, not before.
#[test]
fn the_designer_lift_of_an_amendment_takes_no_words_written_before_it_opened() {
    let fx = setup();
    put_ok(&fx, "component", "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n");
    not_affected(&fx, "D-91", "ui");
    not_affected(&fx, "D-92", "plan");
    fx.human("Draft this one yourself, no designer needed.");
    fx.must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"]);
    let o = fx.bf(&["roles", "lift", "A-1", "--quote", "Draft this one yourself"]);
    assert_eq!(o.code, 2, "words written before the amendment lifted its rule: {}", o.all());
    fx.human("Go on, draft it yourself, no designer needed.");
    let o = fx.bf(&["roles", "lift", "A-1", "--quote", "draft it yourself"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// The replay ignores a design review answer that cites no valid turn and quote, and lets one that
/// does use up its message.
#[test]
fn the_replay_ignores_forged_review_answers_and_a_valid_one_uses_its_message() {
    let fx = setup();
    let p = fx.project();
    for (kind, ev) in [
        (review::ACCEPT_EVENT, json!({"subject": "requirements", "ids": ["F-1"], "turn": 9999, "quote": "yes"})),
        (review::RELEASE_EVENT, json!({"subject": "requirements", "turn": 0, "quote": "yes"})),
        ("designer.lift", json!({"subject": "requirements", "turn": 9999, "message": "m", "quote": "yes"})),
    ] {
        events::append(&p, kind, ev).unwrap();
    }
    let st = events::replay(&p).unwrap();
    let ignored = st.ignored.iter().filter(|n| n.contains("cites no valid human turn and quote")).count();
    assert_eq!(ignored, 3, "{:?}", st.ignored);
    fx.human("I accept this one as it is.");
    let st = events::replay(&p).unwrap();
    let seq = st.turns.last().unwrap().seq;
    events::append(&p, review::RELEASE_EVENT, json!({"subject": "requirements", "turn": seq, "quote": "accept this one"})).unwrap();
    let st = events::replay(&p).unwrap();
    assert!(st.turns.iter().find(|t| t.seq == seq).unwrap().used, "the release used no message");
}
