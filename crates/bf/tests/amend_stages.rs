//! U-19: a design amendment covers each design stage with records or a not affected decision, and
//! approval is refused while a stage is bare or a requirement has no links (R-39, R-40).

mod common;
use common::Fx;
use std::fs;

fn put_ok(fx: &Fx, kind: &str, text: &str) {
    let o = fx.put(kind, text);
    assert_eq!(o.code, 0, "{}", o.all());
}

/// The agent's sentence that a stage is not affected.
fn not_affected(fx: &Fx, id: &str, stage: &str) {
    put_ok(
        fx,
        "decision",
        &format!(
            "id=\"{id}\"\nkind=\"decision\"\ntitle=\"The {stage} stage is not affected by this change\"\nmaker=\"agent\"\nstage=\"{stage}\"\n"
        ),
    );
}

fn narrow_all(fx: &Fx, opened: &serde_json::Value) {
    let keep: Vec<String> = opened["impact_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    let f = fx.root.join("narrow.json");
    fs::write(
        &f,
        serde_json::json!({"amend": "A-1", "keep": keep, "waive": []}).to_string(),
    )
    .unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
}

const LINKED_COMPONENT: &str =
    "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\nlinks=[\"R-2\"]\n";

#[test]
fn a_stage_left_out_is_refused_at_approval_and_named() {
    let fx = Fx::new();
    fx.tiny();
    put_ok(&fx, "component", LINKED_COMPONENT);
    not_affected(&fx, "D-91", "ui");
    // requirements: R-2, ui: D-91, architecture: C-9; the plan stage has neither.
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "--json"])
        .json();
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the change to the second requirement.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert!(o.stderr.contains("plan"), "{}", o.stderr);
    assert!(
        !o.stderr.contains("requirements, ") && !o.stderr.contains("stage(s) ui"),
        "{}",
        o.stderr
    );
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["amendments"][0]["status"], "open", "{st}");
}

#[test]
fn every_stage_left_out_is_named() {
    let fx = Fx::new();
    fx.tiny();
    put_ok(
        &fx,
        "requirement",
        "id=\"R-3\"\nkind=\"requirement\"\ntitle=\"A third\"\nlinks=[\"R-1\"]\n",
    );
    let opened = fx.must(&["amend", "open", "R-3", "--json"]).json();
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the change to the third requirement.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    for stage in ["ui", "architecture", "plan"] {
        assert!(o.stderr.contains(stage), "{stage}: {}", o.stderr);
    }
    // The stages are named in their order.
    let at = o
        .stderr
        .find("ui, architecture, plan")
        .unwrap_or_else(|| panic!("{}", o.stderr));
    let tail = &o.stderr[at..];
    let (u, a, p) = (
        tail.find("ui").unwrap(),
        tail.find("architecture").unwrap(),
        tail.find("plan").unwrap(),
    );
    assert!(u < a && a < p, "{}", o.stderr);
}

#[test]
fn a_record_replaced_after_the_narrowing_is_refused_until_it_is_narrowed_and_read_again() {
    let fx = Fx::new();
    fx.tiny();
    put_ok(&fx, "component", LINKED_COMPONENT);
    not_affected(&fx, "D-91", "ui");
    not_affected(&fx, "D-92", "plan");
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the change to the second requirement.");
    // After the human read it, an agent replaces the sentence of the plan stage.
    let f = fx.root.join("swap.toml");
    fs::write(
        &f,
        "id=\"D-92\"\nkind=\"decision\"\ntitle=\"The plan is not affected: something else\"\nmaker=\"agent\"\nstage=\"plan\"\n",
    )
    .unwrap();
    fx.must(&[
        "record",
        "put",
        "decision",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert!(o.stderr.contains("narrowed"), "{}", o.stderr);
    // Narrowed again and read again, it is approved.
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the change now.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the change"]);
}

#[test]
fn a_requirement_with_no_links_is_refused_by_name() {
    let fx = Fx::new();
    fx.tiny();
    // R-2 has no links and no design part links to it; every stage is otherwise covered.
    put_ok(
        &fx,
        "component",
        "id=\"C-9\"\nkind=\"component\"\ntitle=\"Farewell wording\"\n",
    );
    not_affected(&fx, "D-91", "ui");
    not_affected(&fx, "D-92", "plan");
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the change to the second requirement.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert!(o.stderr.contains("R-2"), "{}", o.stderr);
    assert!(o.stderr.contains("no link"), "{}", o.stderr);
    assert!(!o.stderr.contains("stage(s)"), "{}", o.stderr);
}

#[test]
fn a_covered_amendment_with_linked_requirements_is_approved_by_one_approval() {
    let fx = Fx::new();
    fx.tiny();
    put_ok(&fx, "component", LINKED_COMPONENT);
    not_affected(&fx, "D-91", "ui");
    not_affected(&fx, "D-92", "plan");
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the change to the second requirement.");
    fx.must(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["amendments"][0]["status"], "approved", "{st}");
}

#[test]
fn a_decision_by_the_human_is_no_not_affected_sentence() {
    let fx = Fx::new();
    fx.tiny();
    put_ok(&fx, "component", LINKED_COMPONENT);
    not_affected(&fx, "D-91", "ui");
    put_ok(
        &fx,
        "decision",
        "id=\"D-92\"\nkind=\"decision\"\ntitle=\"x\"\nmaker=\"human\"\nstage=\"plan\"\n",
    );
    let opened = fx
        .must(&["amend", "open", "R-2", "C-9", "D-91", "D-92", "--json"])
        .json();
    narrow_all(&fx, &opened);
    fx.human("Yes, approve the change to the second requirement.");
    let o = fx.refused(&["amend", "approve", "A-1", "--quote", "approve the change"]);
    assert!(o.stderr.contains("plan"), "{}", o.stderr);
}

#[test]
fn the_view_data_shows_the_four_stages_with_records_or_sentence() {
    let fx = Fx::new();
    fx.tiny();
    put_ok(&fx, "component", LINKED_COMPONENT);
    not_affected(&fx, "D-91", "ui");
    fx.must(&["amend", "open", "R-2", "C-9", "D-91"]);
    let v = fx.must(&["view", "run", "--json"]).json();
    let secs = v["sections"].as_array().unwrap();
    let title = |stage: &str| {
        secs.iter()
            .find(|s| s["id"] == format!("A-1-{stage}"))
            .unwrap_or_else(|| panic!("no section for {stage}: {v}"))
    };
    let order: Vec<usize> = ["requirements", "ui", "architecture", "plan"]
        .iter()
        .map(|st| {
            secs.iter()
                .position(|s| s["id"] == format!("A-1-{st}"))
                .unwrap()
        })
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert_eq!(title("requirements")["rows"][0]["id"], "R-2");
    assert_eq!(title("ui")["rows"][0]["id"], "D-91");
    assert_eq!(title("architecture")["rows"][0]["id"], "C-9");
    assert!(title("plan")["rows"].as_array().unwrap().is_empty());
    assert!(title("plan")["title"]
        .as_str()
        .unwrap()
        .contains("neither records"));
}

#[test]
fn a_finding_with_a_gate_and_no_unit_is_put_and_one_with_neither_is_refused() {
    let fx = Fx::new();
    fx.tiny();
    let o = fx.put(
        "finding",
        "id=\"F-1\"\nkind=\"finding\"\ntitle=\"A requirement is vague\"\ngate=\"requirements\"\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.put(
        "finding",
        "id=\"F-2\"\nkind=\"finding\"\ntitle=\"Wording\"\namendment=\"A-1\"\nasks=\"ui\"\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.put(
        "finding",
        "id=\"F-3\"\nkind=\"finding\"\ntitle=\"Belongs nowhere\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("unit"), "{}", o.stderr);
    let o = fx.put(
        "finding",
        "id=\"F-4\"\nkind=\"finding\"\ntitle=\"Bad gate\"\ngate=\"build\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
}
