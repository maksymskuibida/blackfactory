//! U-103 (D-117): the conductor puts the resolver of a blocked sync of the main branch back to work, at
//! most three times, and the sync is parked for the human only after that.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fx.commit_all("records");
}

fn land_unit(fx: &Fx) {
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
        &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"],
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

fn commit_on_main(repo: &Path) {
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/a.txt"), "from main\n").unwrap();
    git(repo, &["add", "src/a.txt"]);
    git(repo, &["commit", "-q", "-m", "work on main"]);
}

/// A sync of main into K-1 that conflicts and stays open, with role tokens on.
fn open_sync(fx: &Fx) {
    plan(fx);
    land_unit(fx);
    commit_on_main(&fx.root);
    fx.set_policy("tokens = false", "tokens = true");
    fx.human("Yes, bring main into the checkpoint, and let the agents sort out conflicts.");
    let o = fx
        .must(&[
            "checkpoint", "sync", "K-1", "--local", "--quote", "bring main into the checkpoint",
            "--json",
        ])
        .json();
    assert_eq!(o["sync_open"], true, "{o}");
}

fn directives(fx: &Fx) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .clone()
}

fn directive(fx: &Fx, action: &str) -> Value {
    let ds = directives(fx);
    assert_eq!(ds.len(), 1, "one directive while a sync is open: {ds:?}");
    assert_eq!(ds[0]["action"], action, "{ds:?}");
    ds[0].clone()
}

fn phase(fx: &Fx) -> String {
    fx.must(&["status", "--json"]).json()["sync_open"]["phase"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn block(fx: &Fx, note: Option<&str>) -> Out {
    let d = directive(fx, "resolve-sync");
    let token = d["token"].as_str().unwrap().to_string();
    let mut args = vec![
        "report", "K-1", "--role", "builder", "--result", "blocked", "--token", &token,
    ];
    if let Some(n) = note {
        args.push("--note");
        args.push(n);
    }
    fx.must_in(&fx.work("K-1"), &args)
}

fn judge(fx: &Fx, judgment: &str) -> Out {
    fx.must(&[
        "checkpoint", "sync", "K-1", "--unblock", "--judgment", judgment, "--reason",
        "the file is settled by now",
    ])
}

#[test]
fn a_blocked_report_with_a_reason_gives_the_conductor_and_not_the_human_a_directive() {
    let fx = Fx::new();
    open_sync(&fx);
    block(&fx, Some("I cannot tell which line is right"));
    assert_eq!(phase(&fx), "blocked");
    let d = directive(&fx, "judge-sync-block");
    assert_eq!(d["role"], "conductor");
    assert_eq!(d["checkpoint"], "K-1");
    assert_eq!(d["reason"], "I cannot tell which line is right");
    assert_eq!(d["releases_left"], 3);
    assert!(directives(&fx).iter().all(|d| d["role"] != "human"));
    // The resolver's own report is refused while the conductor judges.
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["report", "K-1", "--role", "builder", "--result", "built"],
    );
    assert_eq!(o.code, 2, "{}", o.all());
}

#[test]
fn three_fixed_judgments_each_give_the_resolver_a_new_turn_and_the_fourth_block_parks() {
    let fx = Fx::new();
    open_sync(&fx);
    let mut tokens: Vec<String> = vec![];
    for n in 1..=3 {
        let t = directive(&fx, "resolve-sync")["token"].as_str().unwrap().to_string();
        assert!(!tokens.contains(&t), "a new token each turn");
        tokens.push(t);
        block(&fx, Some(&format!("stuck for the {n}th time")));
        let d = directive(&fx, "judge-sync-block");
        assert_eq!(d["releases_left"], 4 - n);
        let o = judge(&fx, "fixed");
        assert!(o.stdout.contains(&format!("release {n} of 3")), "{}", o.all());
        assert_eq!(phase(&fx), "resolve");
    }
    let t = directive(&fx, "resolve-sync")["token"].as_str().unwrap().to_string();
    assert!(!tokens.contains(&t));
    block(&fx, Some("stuck for good"));
    assert_eq!(phase(&fx), "parked");
    let d = directive(&fx, "decide-sync");
    assert_eq!(d["role"], "human");
    assert!(d["note"].as_str().unwrap().contains("stuck for good"));
    // A judgment is refused once the sync is parked.
    let o = fx.bf(&[
        "checkpoint", "sync", "K-1", "--unblock", "--judgment", "fixed", "--reason", "x",
    ]);
    assert_eq!(o.code, 2, "{}", o.all());
}

#[test]
fn a_blocked_report_with_no_reason_parks_the_sync_at_once() {
    let fx = Fx::new();
    open_sync(&fx);
    block(&fx, None);
    assert_eq!(phase(&fx), "parked");
    directive(&fx, "decide-sync");
}

#[test]
fn a_blocked_report_of_the_reviewer_parks_the_sync() {
    let fx = Fx::new();
    open_sync(&fx);
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "from the unit\nfrom main\n").unwrap();
    git(&k, &["add", "src/a.txt"]);
    git(&k, &["commit", "-q", "--no-edit"]);
    let t = directive(&fx, "resolve-sync")["token"].as_str().unwrap().to_string();
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built", "--token", &t],
    );
    let t = directive(&fx, "review-sync")["token"].as_str().unwrap().to_string();
    fx.must_in(
        &k,
        &[
            "report", "K-1", "--role", "reviewer", "--result", "blocked", "--note", "no way", "--token",
            &t,
        ],
    );
    assert_eq!(phase(&fx), "parked");
    directive(&fx, "decide-sync");
}

#[test]
fn the_command_is_refused_from_a_working_copy_and_to_a_worker_session() {
    let fx = Fx::new();
    open_sync(&fx);
    block(&fx, Some("which line is right"));
    let o = fx.bf_in(
        &fx.work("K-1"),
        &["checkpoint", "sync", "K-1", "--unblock", "--judgment", "fixed", "--reason", "x"],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(phase(&fx), "blocked");
    let cmd = "bf checkpoint sync K-1 --unblock --judgment fixed --reason x";
    let mut payload = json!({"tool_name": "Bash", "tool_input": {"command": cmd},
        "cwd": fx.root.to_string_lossy(), "session_id": "s"});
    payload["agent_type"] = json!("builder");
    payload["agent_id"] = json!("agent-1");
    let o = fx.bf_stdin(&fx.root, &["guard"], &payload.to_string());
    assert!(o.stdout.contains("deny"), "{}", o.all());
    // The conductor's own session passes.
    payload.as_object_mut().unwrap().remove("agent_type");
    payload.as_object_mut().unwrap().remove("agent_id");
    let o = fx.bf_stdin(&fx.root, &["guard"], &payload.to_string());
    assert!(!o.stdout.contains("deny"), "{}", o.all());
    // An empty reason and a wrong judgment are refused with their reasons.
    let o = fx.bf(&["checkpoint", "sync", "K-1", "--unblock", "--judgment", "fixed"]);
    assert_eq!(o.code, 2, "{}", o.all());
    let o = fx.bf(&[
        "checkpoint", "sync", "K-1", "--unblock", "--judgment", "maybe", "--reason", "x",
    ]);
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(phase(&fx), "blocked");
}

#[test]
fn a_wait_judgment_leaves_the_sync_blocked() {
    let fx = Fx::new();
    open_sync(&fx);
    block(&fx, Some("which line is right"));
    judge(&fx, "wait");
    assert_eq!(phase(&fx), "blocked");
    // No unit is left that could land, so the human is asked rather than left with nothing.
    let d = directive(&fx, "decide-sync");
    assert_eq!(d["role"], "human");
    // Its release command works: the sync goes back to its resolver.
    fx.human("Yes, release it and try again.");
    fx.must(&["checkpoint", "sync", "K-1", "--release", "--quote", "release it and try"]);
    assert_eq!(phase(&fx), "resolve");
    directive(&fx, "resolve-sync");
}

#[test]
fn the_humans_release_starts_the_count_again() {
    let fx = Fx::new();
    open_sync(&fx);
    for n in 1..=3 {
        block(&fx, Some(&format!("stuck {n}")));
        judge(&fx, "fixed");
    }
    block(&fx, Some("stuck again"));
    assert_eq!(phase(&fx), "parked");
    fx.human("Yes, release it and try again.");
    fx.must(&["checkpoint", "sync", "K-1", "--release", "--quote", "release it and try"]);
    assert_eq!(phase(&fx), "resolve");
    block(&fx, Some("stuck after the release"));
    let d = directive(&fx, "judge-sync-block");
    assert_eq!(d["releases_left"], 3, "{d}");
}

#[test]
fn an_old_log_with_a_park_replays_to_parked() {
    let fx = Fx::new();
    open_sync(&fx);
    // What a log written before this unit holds: the resolver's blocked report was a park.
    bf::events::append(
        &fx.project(),
        "checkpoint.sync.park",
        json!({"checkpoint": "K-1", "reason": "the resolver is blocked"}),
    )
    .unwrap();
    let st = bf::events::replay(&fx.project()).unwrap();
    assert_eq!(st.sync_open.unwrap().phase, "parked");
    directive(&fx, "decide-sync");
}

#[test]
fn a_report_that_changed_a_file_neither_side_changed_is_taken_and_the_file_is_named() {
    let fx = Fx::new();
    open_sync(&fx);
    let k = fx.work("K-1");
    fs::write(k.join("src/a.txt"), "from the unit\nfrom main\n").unwrap();
    fs::write(k.join("src/other.txt"), "not asked for\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "--no-edit"]);
    let t = directive(&fx, "resolve-sync")["token"].as_str().unwrap().to_string();
    fx.must_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "built", "--token", &t],
    );
    assert_eq!(phase(&fx), "review");
    let log = fs::read_to_string(fx.project().log_path()).unwrap();
    let built = log
        .lines()
        .find(|l| l.contains("\"checkpoint.sync.built\""))
        .expect("a built event");
    assert!(built.contains("src/other.txt"), "{built}");
}
