//! U-74: the tokens and briefs of the design reviewer and the stage agents of an amendment, and the
//! general designer's split.

mod common;
use common::Fx;

fn open(fx: &Fx) {
    fx.tiny();
    fx.must(&["amend", "open", "R-2", "C-1", "--json"]);
}

fn general(fx: &Fx) -> String {
    bf::run::issue_token(&fx.project(), "A-1:designer").unwrap()
}

fn tok(fx: &Fx, args: &[&str]) -> String {
    let mut a = vec!["review", "token"];
    a.extend_from_slice(args);
    a.push("--json");
    fx.must(&a).json()["token"].as_str().unwrap().to_string()
}

#[test]
fn a_token_of_one_kind_is_refused_for_another() {
    let fx = Fx::new();
    open(&fx);
    // A kind of an amendment needs the amendment; a kind of a gate refuses one.
    let o = fx.refused(&["review", "token", "legal"]);
    assert!(o.stderr.contains("legal"), "{}", o.stderr);
    fx.refused(&["review", "token", "idea", "--subject", "A-1"]);
    fx.refused(&["review", "token", "nonsense"]);
    let a = tok(&fx, &["requirements", "--subject", "A-1"]);
    let b = tok(&fx, &["architecture", "--subject", "A-1"]);
    assert_ne!(a, b);
    let p = fx.project();
    assert!(bf::run::holds_token(&p, "A-1:design-reviewer:requirements", Some(&a)));
    assert!(!bf::run::holds_token(&p, "A-1:design-reviewer:architecture", Some(&a)));
    assert!(!bf::run::holds_token(&p, "A-1:design-reviewer:requirements", Some(&b)));
    // The same kind gives the same token until it is spent.
    assert_eq!(a, tok(&fx, &["requirements", "--subject", "A-1"]));
    // A gate's reviewer has its own key.
    let g = tok(&fx, &["plan"]);
    assert!(bf::run::holds_token(&p, "design:design-reviewer:plan", Some(&g)));
}

#[test]
fn a_token_on_an_amendment_is_refused_when_none_is_open() {
    let fx = Fx::new();
    fx.tiny();
    let o = fx.refused(&["review", "token", "amendment", "--subject", "A-1"]);
    assert!(o.stderr.contains("A-1"), "{}", o.stderr);
    fx.refused(&["review", "token", "--subject", "A-1", "--stage", "ui", "--role", "uiux-verifier"]);
    fx.must(&["amend", "open", "R-2", "--json"]);
    tok(&fx, &["amendment", "--subject", "A-1"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "changed my mind"]);
    fx.refused(&["review", "token", "amendment", "--subject", "A-1"]);
}

#[test]
fn a_stage_token_is_refused_before_a_split_for_a_stage_not_named_and_a_second_time() {
    let fx = Fx::new();
    open(&fx);
    let before = fx.refused(&["review", "token", "--subject", "A-1", "--stage", "requirements", "--role", "designer"]);
    println!("{}", before.stderr);
    assert!(before.stderr.contains("not split"), "{}", before.stderr);
    let g = general(&fx);
    fx.must(&["review", "split", "A-1", "--stage", "requirements=R-2", "--token", &g]);
    let other = fx.refused(&["review", "token", "--subject", "A-1", "--stage", "architecture", "--role", "designer"]);
    assert!(other.stderr.contains("did not name"), "{}", other.stderr);
    // The wrong role for the stage.
    fx.refused(&["review", "token", "--subject", "A-1", "--stage", "requirements", "--role", "lawyer"]);
    let t = tok(&fx, &["--subject", "A-1", "--stage", "requirements", "--role", "designer"]);
    assert!(bf::run::holds_token(&fx.project(), "A-1:designer:requirements", Some(&t)));
    let again = fx.refused(&["review", "token", "--subject", "A-1", "--stage", "requirements", "--role", "designer"]);
    assert!(again.stderr.contains("at most one"), "{}", again.stderr);
}

#[test]
fn a_split_checks_its_ids_and_its_token_and_a_stage_token_cannot_split() {
    let fx = Fx::new();
    open(&fx);
    let g = general(&fx);
    fx.refused(&["review", "split", "A-1", "--stage", "requirements=R-2", "--token", "wrong"]);
    fx.refused(&["review", "split", "A-1", "--stage", "requirements=R-1", "--token", &g]);
    fx.refused(&["review", "split", "A-1", "--stage", "nowhere=R-2", "--token", &g]);
    fx.refused(&["review", "split", "A-1", "--token", &g]);
    let o = fx.must(&[
        "review", "split", "A-1", "--stage", "requirements=R-2", "--stage", "architecture=C-1",
        "--token", &g, "--json",
    ]);
    assert_eq!(o.json()["split"]["architecture"][0], "C-1");
    let p = fx.project();
    assert!(bf::roles::design::stage_may_put(&p, "A-1", "requirements", "R-2"));
    assert!(!bf::roles::design::stage_may_put(&p, "A-1", "requirements", "C-1"));
    assert!(!bf::roles::design::stage_may_put(&p, "A-1", "plan", "R-2"));
    let st = tok(&fx, &["--subject", "A-1", "--stage", "requirements", "--role", "designer"]);
    let o = fx.refused(&["review", "split", "A-1", "--stage", "architecture=C-1", "--token", &st]);
    assert!(o.stderr.contains("cannot split"), "{}", o.stderr);
}

#[test]
fn a_brief_of_each_kind_holds_its_records_and_rules_and_no_drafts_path() {
    let fx = Fx::new();
    open(&fx);
    let brief = |args: &[&str]| -> String {
        let mut a = vec!["review", "brief"];
        a.extend_from_slice(args);
        a.push("--json");
        let path = fx.must(&a).json()["path"].as_str().unwrap().to_string();
        assert!(path.contains(".bf/briefs/"), "{path}");
        std::fs::read_to_string(path).unwrap()
    };
    let req = brief(&["requirements"]);
    assert!(req.contains("R-1") && req.contains("every acceptance statement"), "{req}");
    assert!(!req.contains("C-1"), "{req}");
    let arch = brief(&["architecture"]);
    assert!(arch.contains("C-1") && arch.contains("rejected"), "{arch}");
    let plan = brief(&["plan"]);
    assert!(plan.contains("U-1") && plan.contains("ordered"), "{plan}");
    let am = brief(&["amendment", "--subject", "A-1"]);
    assert!(am.contains("R-2") && am.contains("C-1") && am.contains("Before") && am.contains("Asking for a stage review"), "{am}");
    assert!(am.contains("Tier:") && am.contains("Effort:"), "{am}");
    let stage = brief(&["requirements", "--subject", "A-1"]);
    assert!(stage.contains("cannot request further reviews"), "{stage}");
    let g = general(&fx);
    fx.must(&["review", "split", "A-1", "--stage", "requirements=R-2", "--token", &g]);
    let sa = brief(&["requirements", "--subject", "A-1", "--role", "designer"]);
    assert!(sa.contains("- R-2") && !sa.contains("- C-1\n") && sa.contains("Tier:"), "{sa}");
    for b in [&req, &arch, &plan, &am, &stage, &sa] {
        assert!(!b.contains("drafts"), "{b}");
    }
    fx.refused(&["review", "brief", "legal", "--subject", "A-1", "--role", "lawyer"]);
}
