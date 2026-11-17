//! U-72: the review checks, the design reviewer and UI/UX designer roles, and the records each kind of
//! review judges with their hash.

mod common;
use bf::checks::{self, ReviewKind};
use common::Fx;

const REVIEWS: [&str; 5] = [
    "idea-review",
    "requirements-review",
    "architecture-review",
    "plan-review",
    "amendment-review",
];

fn row<'a>(v: &'a serde_json::Value, key: &str, name: &str) -> &'a serde_json::Value {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn the_checks_list_shows_the_five_review_checks() {
    let fx = Fx::with_checks();
    let v = fx.must(&["checks", "list", "--json"]).json();
    for n in REVIEWS {
        let r = row(&v, "checks", n);
        assert_eq!(r["family"], "review", "{n}");
        assert_eq!(r["stage"], "design", "{n}");
        assert_eq!(r["how"], "agent", "{n}");
        assert_eq!(r["required"], true, "{n}");
    }
    assert_eq!(checks::in_family(checks::Family::Review).len(), 6);
}

#[test]
fn the_roles_list_shows_the_design_reviewer_and_the_designer_with_their_kinds() {
    let fx = Fx::with_checks();
    let v = fx.must(&["roles", "list", "--json"]).json();
    let roles = v["roles"].as_array().unwrap();
    let find = |n: &str| roles.iter().find(|r| r["name"] == n).unwrap().clone();

    let rev = find("design-reviewer");
    assert_eq!(rev["stage"], "design");
    assert_eq!(rev["family"], "review");
    assert_eq!(rev["tier"], "reviewer");
    assert_eq!(rev["blind"], false);
    assert_eq!(
        rev["kinds"],
        serde_json::json!(["idea", "requirements", "architecture", "plan"])
    );
    assert_eq!(
        rev["amendment_kinds"],
        serde_json::json!(["amendment", "requirements", "architecture", "legal", "plan"])
    );
    assert!(rev["writes"].to_string().contains("finding"));

    let ux = find("uiux-designer");
    assert_eq!(ux["stage"], "design");
    assert_eq!(ux["family"], "uiux");
    let w = ux["writes"].to_string();
    assert!(w.contains("screen") && w.contains("journey"), "{w}");

    assert!(find("lawyer")["writes"].to_string().contains("markets"));
}

/// Record a decision of the default the way the engine did before the review checks existed: its
/// changes carry no name of a check added later.
fn decide_as_an_old_decision_did(fx: &Fx) {
    let p = fx.project();
    let prop = bf::events::append(
        &p,
        "checks.propose",
        serde_json::json!({"scope": "default", "changes": [], "run": null}),
    )
    .unwrap();
    fx.human("Yes, keep every check on.");
    let st = bf::events::replay(&p).unwrap();
    let turn = st.turns.last().unwrap().seq;
    bf::events::append(
        &p,
        "checks.set",
        serde_json::json!({
            "scope": "default",
            "proposal": prop["seq"],
            "turn": turn,
            "quote": "keep every check on",
            "changes": [],
            "run": null,
        }),
    )
    .unwrap();
}

#[test]
fn a_decision_made_now_keeps_the_review_checks_required() {
    let fx = Fx::with_checks();
    fx.checks_decided_as_default();
    let s = fx.must(&["checks", "state", "--json"]).json();
    for n in REVIEWS {
        let r = row(&s, "default", n);
        assert_eq!(r["required"], true, "{n}: {r}");
    }
    // The human can still switch one off, with a reason.
    fx.must(&[
        "checks",
        "propose",
        "idea-review=off:the idea is settled",
        "--scope",
        "default",
    ]);
    fx.human("Yes, switch the idea review off.");
    fx.must(&["checks", "set", "--quote", "switch the idea review off"]);
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert_eq!(row(&s, "default", "idea-review")["required"], false);
    assert_eq!(row(&s, "default", "plan-review")["required"], true);
}

#[test]
fn a_check_added_after_the_decision_is_off_with_its_reason_until_the_human_switches_it_on() {
    let fx = Fx::with_checks();
    // Nothing is decided yet: every check, the new ones too, is required.
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert_eq!(row(&s, "default", "idea-review")["required"], true);

    decide_as_an_old_decision_did(&fx);
    let s = fx.must(&["checks", "state", "--json"]).json();
    for n in REVIEWS {
        let r = row(&s, "default", n);
        assert_eq!(r["required"], false, "{n}");
        assert!(
            r["reason"].as_str().unwrap().contains("added after"),
            "{n}: {r}"
        );
    }
    // A check the decision held stays required.
    assert_eq!(row(&s, "default", "secret-scan")["required"], true);
    assert_eq!(row(&s, "default", "checkpoint-review")["required"], true);
    let l = fx.must(&["checks", "list"]).stdout;
    assert!(l.contains("added after"), "{l}");

    fx.must(&["checks", "propose", "idea-review=on", "--scope", "default"]);
    fx.human("Yes, switch the idea review on.");
    fx.must(&["checks", "set", "--quote", "switch the idea review on"]);
    let s = fx.must(&["checks", "state", "--json"]).json();
    assert_eq!(row(&s, "default", "idea-review")["required"], true);
    // The new decision holds every name the catalogue has now, so the others are no longer late.
    assert_eq!(row(&s, "default", "plan-review")["required"], true);
}

fn write(fx: &Fx, id: &str, body: &str) {
    let dir = fx.root.join("factory/records");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{id}.toml")), body).unwrap();
}

fn rec(id: &str, kind: &str, title: &str, extra: &str) -> String {
    format!("id=\"{id}\"\nkind=\"{kind}\"\ntitle=\"{title}\"\n{extra}")
}

#[test]
fn the_hash_of_a_kind_changes_with_its_records_and_not_with_those_of_another_kind() {
    let fx = Fx::with_checks();
    write(&fx, "I-1", &rec("I-1", "idea", "the idea", ""));
    write(&fx, "R-1", &rec("R-1", "requirement", "a requirement", ""));
    write(
        &fx,
        "J-1",
        &rec("J-1", "journey", "a journey", "steps=[\"a\",\"b\"]\n"),
    );
    write(&fx, "C-1", &rec("C-1", "component", "a component", ""));
    write(&fx, "U-1", &rec("U-1", "unit", "a unit", ""));
    std::fs::write(fx.root.join("factory/idea.md"), "the idea text\n").unwrap();

    let h = |k: ReviewKind| checks::records_hash(&fx.project(), k, None).unwrap();
    let before = (
        h(ReviewKind::Idea),
        h(ReviewKind::Requirements),
        h(ReviewKind::Architecture),
        h(ReviewKind::Plan),
    );
    assert_eq!(before.0, h(ReviewKind::Idea), "the hash is stable");

    // A requirement changes: only the requirements hash moves.
    write(&fx, "R-1", &rec("R-1", "requirement", "a changed requirement", ""));
    assert_ne!(before.1, h(ReviewKind::Requirements));
    assert_eq!(before.0, h(ReviewKind::Idea));
    assert_eq!(before.2, h(ReviewKind::Architecture));
    assert_eq!(before.3, h(ReviewKind::Plan));

    // A journey is a requirements record too.
    let req = h(ReviewKind::Requirements);
    write(
        &fx,
        "J-1",
        &rec("J-1", "journey", "a journey", "steps=[\"a\",\"c\"]\n"),
    );
    assert_ne!(req, h(ReviewKind::Requirements));

    // A component changes: only the architecture hash moves.
    write(&fx, "C-1", &rec("C-1", "component", "a changed component", ""));
    assert_ne!(before.2, h(ReviewKind::Architecture));
    assert_eq!(before.3, h(ReviewKind::Plan));

    // The idea text is part of the idea.
    std::fs::write(fx.root.join("factory/idea.md"), "the idea, edited\n").unwrap();
    assert_ne!(before.0, h(ReviewKind::Idea));

    // A unit belongs to the plan alone.
    write(&fx, "U-1", &rec("U-1", "unit", "a changed unit", ""));
    assert_ne!(before.3, h(ReviewKind::Plan));
}

#[test]
fn an_amendment_is_judged_by_the_records_its_set_names() {
    let fx = Fx::with_checks();
    write(&fx, "R-1", &rec("R-1", "requirement", "a requirement", ""));
    write(&fx, "R-2", &rec("R-2", "requirement", "another", ""));
    write(&fx, "C-1", &rec("C-1", "component", "a component", ""));
    let set = vec!["R-1".to_string(), "C-1".to_string()];
    let p = fx.project();
    let ids = |k: ReviewKind| -> Vec<String> {
        checks::kind_records(&p, k, Some(&set))
            .unwrap()
            .into_iter()
            .map(|x| x.0)
            .collect()
    };
    assert_eq!(ids(ReviewKind::Amendment), vec!["C-1", "R-1"]);
    // A stage kind inside an amendment holds only that stage's records of the set.
    assert_eq!(ids(ReviewKind::Requirements), vec!["R-1"]);
    assert_eq!(ids(ReviewKind::Architecture), vec!["C-1"]);

    let a = checks::records_hash(&p, ReviewKind::Amendment, Some(&set)).unwrap();
    write(&fx, "R-2", &rec("R-2", "requirement", "another, changed", ""));
    assert_eq!(
        a,
        checks::records_hash(&fx.project(), ReviewKind::Amendment, Some(&set)).unwrap()
    );
    write(&fx, "R-1", &rec("R-1", "requirement", "changed", ""));
    assert_ne!(
        a,
        checks::records_hash(&fx.project(), ReviewKind::Amendment, Some(&set)).unwrap()
    );
}
