//! U-36: the model policy (R-66, C-21, D-44): a tier, an effort and an optional model for every kind of
//! work, carried on every directive and printed with the policy in force.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const SHIPPED: [(&str, &str, &str); 8] = [
    ("conductor", "standard", "medium"),
    ("designer", "strong", "high"),
    ("amender", "strong", "high"),
    ("advisor", "strong", "high"),
    ("builder", "standard", "medium"),
    ("reviewer", "strong", "high"),
    ("checkpoint_reviewer", "strong", "high"),
    ("verifier", "standard", "low"),
];

fn directives(fx: &Fx) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .unwrap()
        .clone()
}

fn find(fx: &Fx, action: &str) -> Option<Value> {
    directives(fx).into_iter().find(|d| d["action"] == action)
}

#[test]
fn every_kind_of_work_has_an_entry_with_the_shipped_tier_and_effort() {
    let fx = Fx::new();
    let w = fx.must(&["policy", "--json"]).json()["work"].clone();
    for (kind, tier, effort) in SHIPPED {
        assert_eq!(w[kind]["tier"], tier, "{kind}: {w}");
        assert_eq!(w[kind]["effort"], effort, "{kind}: {w}");
        assert!(w[kind]["model"].is_null(), "{kind}: {w}");
    }
}

#[test]
fn bf_policy_prints_the_advising_entry() {
    let fx = Fx::new();
    let o = fx.must(&["policy"]);
    assert!(
        o.stdout.contains("advisor: tier strong effort high"),
        "{}",
        o.stdout
    );
}

#[test]
fn a_model_named_outright_is_printed_and_changes_only_when_the_human_accepts() {
    let fx = Fx::new();
    let pol = fx.root.join("factory/policy.toml");
    let t = fs::read_to_string(&pol).unwrap();
    fs::write(
        &pol,
        format!("{t}\n[models.advisor]\nclaude = \"a-model\"\n"),
    )
    .unwrap();
    let w = fx.must(&["policy", "--json"]).json()["work"].clone();
    assert!(w["advisor"]["model"].is_null(), "not accepted yet: {w}");
    fx.human("Yes, I accept the new policy.");
    fx.must(&["policy", "accept", "--quote", "I accept the new policy"]);
    let w = fx.must(&["policy", "--json"]).json()["work"].clone();
    assert_eq!(w["advisor"]["model"]["claude"], "a-model", "{w}");
}

#[test]
fn a_builder_and_a_conductor_directive_carry_their_tier_and_effort() {
    let fx = Fx::new();
    fx.tiny();
    fx.must(&["run", "start"]);
    let d = &directives(&fx)[0];
    assert_eq!(d["role"], "human", "{d}");
    assert_eq!(d["tier"], "standard", "{d}");
    assert_eq!(d["effort"], "medium", "{d}");
    fx.answer_plan_questions();
    fx.must(&["gate", "open", "plan"]);
    fx.human("approved");
    fx.must(&["gate", "approve", "plan", "--quote", "approved"]);
    let d = find(&fx, "start-unit").expect("a builder directive");
    assert_eq!(d["role"], "builder", "{d}");
    assert_eq!(d["tier"], "standard", "{d}");
    assert_eq!(d["effort"], "medium", "{d}");
    println!("{d}");
}

/// One unit, one checkpoint with a command check and the review, the unit landed.
fn project(fx: &Fx) {
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
    fx.commit_all("records");
    fx.must(&[
        "checks",
        "propose",
        "checkpoint-review=on", "idea-review=off:the test does not exercise it", "requirements-review=off:the test does not exercise it", "architecture-review=off:the test does not exercise it", "plan-review=off:the test does not exercise it",
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
    // The unit's review is asked for before the clean report.
    let d = find(fx, "review").expect("a unit review directive");
    assert_eq!(d["role"], "reviewer", "{d}");
    assert_eq!(d["tier"], "strong", "{d}");
    assert_eq!(d["effort"], "high", "{d}");
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

#[test]
fn a_reviewer_and_a_verifier_directive_carry_their_tier_and_effort() {
    let fx = Fx::new();
    project(&fx);
    let r = find(&fx, "review-checkpoint").expect("a checkpoint review directive");
    assert_eq!(r["tier"], "strong", "{r}");
    assert_eq!(r["effort"], "high", "{r}");
    let v = find(&fx, "run-checkpoint-checks").expect("a verifier directive");
    assert_eq!(v["tier"], "standard", "{v}");
    assert_eq!(v["effort"], "low", "{v}");
    println!("{}", json!({"reviewer": r, "verifier": v}));
}

#[test]
fn a_changed_policy_changes_the_directive() {
    let fx = Fx::new();
    fx.set_policy("verifier = \"standard\"", "verifier = \"cheap\"");
    fx.set_policy("verifier = \"low\"", "verifier = \"medium\"");
    project(&fx);
    let v = find(&fx, "run-checkpoint-checks").expect("a verifier directive");
    assert_eq!(v["tier"], "cheap", "{v}");
    assert_eq!(v["effort"], "medium", "{v}");
}
