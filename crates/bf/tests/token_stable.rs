//! U-91 (FU-43, D-112, D-113): a worker whose work is open keeps one live token on every read of
//! `bf next`, until its report spends it or its work is released, even while another hold hides its
//! directive.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

/// One unit, one checkpoint with a command check and the checkpoint review, role tokens on.
fn project(fx: &Fx, two: bool) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A file exists\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\"]\ncheckpoint=\"K-1\"\n"),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-3\"\nkind=\"check\"\ntitle=\"review\"\nhow=\"agent\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"checkpoint-review\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\",\"X-3\"]\n"),
    ] {
        let r = fx.put(k, t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    if two {
        // A second checkpoint with its own unit, so that a check of K-1 can be marked while K-2 is reviewed.
        for (k, t) in [
            ("requirement", "id=\"R-2\"\nkind=\"requirement\"\ntitle=\"B\"\nacceptance=[\"src/b.txt exists\"]\n"),
            ("unit", "id=\"U-2\"\nkind=\"unit\"\ntitle=\"u2\"\ndelivers=[\"R-2\"]\nfiles=[\"src/b.txt\"]\ncheckpoint=\"K-2\"\ndepends_on=[\"U-1\"]\n"),
            ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"c4\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"checkpoint\"\n"),
            ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"c5\"\nhow=\"command\"\ncommand=\"test -f src/b.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-2\"\n"),
            ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k2\"\nafter=[\"U-2\"]\nproves=[\"R-2\"]\nchecks=[\"X-4\",\"X-3\"]\n"),
        ] {
            let r = fx.put(k, t);
            assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
        }
    }
    fx.commit_all("records");
    fx.set_policy("tokens = false", "tokens = true");
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on",
        "idea-review=off:the test does not exercise it",
        "requirements-review=off:the test does not exercise it",
        "architecture-review=off:the test does not exercise it",
        "plan-review=off:the test does not exercise it",
        "--scope",
        "default",
    ]);
    fx.human("Yes, the checkpoint review is on.");
    fx.must(&["checks", "set", "--quote", "the checkpoint review is on"]);
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
}

fn start_unit(fx: &Fx) -> PathBuf {
    PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    )
}

fn build(fx: &Fx, wc: &PathBuf) {
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
    fx.must_in(
        wc,
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
}

fn token_in_file(fx: &Fx, key: &str) -> Option<String> {
    let t = fs::read_to_string(fx.project().tokens_path()).unwrap_or_default();
    let v: Value = serde_json::from_str(&t).unwrap_or(json!({}));
    v.get(key).and_then(|x| x.as_str()).map(|x| x.to_string())
}

fn shown(fx: &Fx, key: &str) -> Option<String> {
    let (id, role) = key.split_once(':').unwrap();
    let n = fx.must(&["next", "--json"]).json();
    n["directives"].as_array().unwrap().iter().find_map(|d| {
        let who = d["unit"].as_str().or_else(|| d["checkpoint"].as_str());
        if who == Some(id) && d["role"] == role {
            d["token"].as_str().map(|t| t.to_string())
        } else {
            None
        }
    })
}

fn clean(fx: &Fx, k: &str, token: &str) -> Out {
    let body = json!({"areas": [{"area": "the close rule", "how": "read"}], "findings": []});
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
            "--token",
            token,
        ],
        &body.to_string(),
    )
}

#[test]
fn a_checkpoint_reviewer_keeps_its_token_while_another_hold_hides_its_directive() {
    let fx = Fx::new();
    project(&fx, true);
    let wc = start_unit(&fx);
    build(&fx, &wc);
    let b = shown(&fx, "U-1:builder").expect("builder token");
    fx.must(&["report", "U-1", "--result", "built", "--token", &b]);
    let r = shown(&fx, "U-1:reviewer").expect("unit reviewer token");
    fx.must(&[
        "report", "U-1", "--result", "built", "--role", "reviewer", "--token", &r,
    ]);
    fx.must(&["unit", "land", "U-1"]);
    // The review round starts with the brief.
    fx.must(&["brief", "K-1", "--role", "reviewer", "--json"]);
    let tok = shown(&fx, "K-1:reviewer").expect("the reviewer's token");
    for i in 0..10 {
        if i == 3 {
            // A check of ANOTHER checkpoint (K-2) is marked: it shows a decide-check beside the review.
            // K-2's working copy is its branch from main, as the engine makes it for a checkpoint.
            let cpb = fx.project().checkpoint_branch("K-2");
            // Its unit counts as landed, so the checkpoint is judged and its mark shows a decide-check.
            let landed = json!({"unit": "U-2", "hash": "h", "run": "run-1"});
            bf::events::append(&fx.project(), "unit.land", landed).unwrap();
            let copy = fx.work("K-2");
            git(&fx.root, &["branch", &cpb, "main"]);
            git(&fx.root, &["worktree", "add", copy.to_str().unwrap(), &cpb]);
            let o = fx.bf_stdin(
                &copy,
                &[
                    "evidence",
                    "unreachable",
                    "X-4",
                    "--reason",
                    "the service is down",
                ],
                "",
            );
            assert_eq!(o.code, 0, "{}", o.all());
        }
        if i == 6 {
            // A check of the checkpoint itself is marked: its decide-check hides the reviewer's directive.
            let o = fx.bf_stdin(
                &fx.work("K-1"),
                &[
                    "evidence",
                    "unreachable",
                    "X-2",
                    "--reason",
                    "the service is down",
                ],
                "",
            );
            assert_eq!(o.code, 0, "{}", o.all());
        }
        let seen = shown(&fx, "K-1:reviewer");
        if let Some(s) = &seen {
            assert_eq!(s, &tok, "read {i}");
        }
        if i >= 6 {
            assert!(seen.is_none(), "read {i}: the directive should be hidden");
        }
        assert_eq!(
            token_in_file(&fx, "K-1:reviewer").as_deref(),
            Some(tok.as_str()),
            "read {i}"
        );
    }
    let t = fs::read_to_string(fx.project().tokens_path()).unwrap();
    assert_eq!(t.matches("K-1:reviewer").count(), 1, "{t}");
    let n = fx.must(&["next", "--json"]).json();
    let acts: Vec<String> = n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| format!("{}:{}", d["action"], d["checkpoint"]))
        .collect();
    assert!(
        acts.iter()
            .any(|a| a.contains("decide-check") && a.contains("K-2")),
        "{acts:?}"
    );
    // The reviewer runs the check in its copy; a passing run lifts the mark.
    let o = fx.bf_stdin(
        &fx.work("K-1-review"),
        &["evidence", "run", "X-2", "--", "test", "-f", "src/a.txt"],
        "",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = clean(&fx, "K-1", &tok);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(token_in_file(&fx, "K-1:reviewer").is_none());
}

#[test]
fn a_builder_keeps_its_token_while_its_unit_waits_behind_a_held_directive() {
    let fx = Fx::new();
    project(&fx, false);
    let wc = start_unit(&fx);
    build(&fx, &wc);
    let tok = shown(&fx, "U-1:builder").expect("builder token");
    fx.must(&["unit", "pause", "U-1"]);
    for i in 0..10 {
        let seen = shown(&fx, "U-1:builder");
        assert!(seen.is_none(), "read {i}: a paused unit has no directive");
        assert_eq!(
            token_in_file(&fx, "U-1:builder").as_deref(),
            Some(tok.as_str()),
            "read {i}"
        );
    }
    fx.must(&["unit", "resume", "U-1"]);
    assert_eq!(shown(&fx, "U-1:builder").as_deref(), Some(tok.as_str()));
    let t = fs::read_to_string(fx.project().tokens_path()).unwrap();
    assert_eq!(t.matches("U-1:builder").count(), 1, "{t}");
    let o = fx.bf(&["report", "U-1", "--result", "built", "--token", &tok]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_token_is_gone_after_its_report_and_after_its_unit_landed() {
    let fx = Fx::new();
    project(&fx, false);
    let wc = start_unit(&fx);
    build(&fx, &wc);
    let b = shown(&fx, "U-1:builder").expect("builder token");
    fx.must(&["report", "U-1", "--result", "built", "--token", &b]);
    assert!(token_in_file(&fx, "U-1:builder").is_none());
    shown(&fx, "U-1:reviewer").expect("unit reviewer token");
    assert!(token_in_file(&fx, "U-1:builder").is_none());
    let r = token_in_file(&fx, "U-1:reviewer").unwrap();
    fx.must(&[
        "report", "U-1", "--result", "built", "--role", "reviewer", "--token", &r,
    ]);
    assert!(token_in_file(&fx, "U-1:reviewer").is_none());
    fx.must(&["unit", "land", "U-1"]);
    fx.must(&["next", "--json"]);
    assert!(token_in_file(&fx, "U-1:builder").is_none());
    assert!(token_in_file(&fx, "U-1:reviewer").is_none());
}

#[test]
fn a_kept_token_is_dropped_when_its_work_closes_without_a_report() {
    let fx = Fx::new();
    project(&fx, false);
    let wc = start_unit(&fx);
    build(&fx, &wc);
    let tok = shown(&fx, "U-1:builder").expect("builder token");
    // Paused, the directive is hidden and only the open-work rule keeps the token.
    fx.must(&["unit", "pause", "U-1"]);
    assert!(shown(&fx, "U-1:builder").is_none());
    assert_eq!(
        token_in_file(&fx, "U-1:builder").as_deref(),
        Some(tok.as_str())
    );
    // The unit leaves the open states with no report from this token: the next read drops it.
    let landed = json!({"unit": "U-1", "hash": "h", "run": "run-1"});
    bf::events::append(&fx.project(), "unit.land", landed).unwrap();
    fx.must(&["next", "--json"]);
    assert!(token_in_file(&fx, "U-1:builder").is_none());
    let t = fs::read_to_string(fx.project().tokens_path()).unwrap_or_default();
    assert_eq!(t.matches("U-1:builder").count(), 0, "{t}");
}
