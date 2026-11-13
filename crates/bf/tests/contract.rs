//! The view contract: every page the engine emits validates against the view contract's schema (`views/contract/view.schema.json`),
//! and renders into the real template.

mod common;
use common::*;
use std::fs;
use std::path::Path;

fn real_template() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views/templates/page.html")
}

#[test]
fn the_validator_accepts_the_sample_pages_and_rejects_broken_pages() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views/samples");
    let mut n = 0;
    for e in fs::read_dir(dir).unwrap().flatten() {
        if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(e.path()).unwrap()).unwrap();
        assert_contract(&e.path().display().to_string(), &v);
        n += 1;
    }
    assert!(n >= 8, "expected the eight samples, found {n}");
    // Broken pages are caught: missing required field, bad enum, wrong type.
    let good = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views/samples/plan.json"),
    )
    .unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
    v["page"]["status"]["tone"] = serde_json::json!("purple");
    assert!(!schema::validate(&view_schema(), &v).is_empty());
    let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
    v.as_object_mut().unwrap().remove("sections");
    assert!(!schema::validate(&view_schema(), &v).is_empty());
}

#[test]
fn every_view_kind_validates_against_the_contract() {
    let fx = Fx::new();
    // Before anything exists.
    for kind in ["index", "questions", "ledger", "plan", "run", "coverage"] {
        assert_contract(kind, &fx.must(&["view", kind, "--json"]).json());
    }
    fx.tiny();
    fs::write(
        fx.root.join("factory/idea.md"),
        "# A greeting service\n\nSay hello and goodbye.\n",
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let q = dir.path().join("q.json");
    fs::write(&q, r#"[{"title":"Which tone?","context":"c","kind":"single","options":[{"key":"A","text":"Warm","proposed":true,"why":"friendly"},{"key":"B","text":"Plain"}]},{"title":"Anything else?","kind":"text"}]"#).unwrap();
    fx.must(&[
        "ask",
        "--file",
        q.to_str().unwrap(),
        "--stage",
        "requirements",
    ]);
    for gate in ["idea", "ui", "plan"] {
        assert_contract(gate, &fx.must(&["view", "gate", gate]).json());
    }
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.must(&["gate", "open", "idea"]);
    // A round 2 shows what changed.
    fx.human("not yet, add a farewell");
    fx.must(&[
        "gate",
        "reject",
        "idea",
        "--quote",
        "not yet, add a farewell",
    ]);
    fs::write(
        fx.root.join("factory/idea.md"),
        "# A greeting service\n\nSay hello and goodbye, politely.\n",
    )
    .unwrap();
    let r2 = fx.must(&["gate", "open", "idea", "--json"]).json();
    assert_eq!(r2["id"], "idea-r2");
    let g = fx.must(&["view", "gate", "idea-r2", "--json"]).json();
    assert_contract("gate round 2", &g);
    assert_eq!(g["page"]["round"]["n"], 2);
    assert_eq!(g["page"]["round"]["changed"], 1);
    assert_eq!(g["sections"][0]["rows"][0]["round"], "changed");
    assert!(g["records"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["title"] == "idea.md" && r["format"] == "markdown"));
    for kind in ["index", "questions", "ledger", "plan", "run", "coverage"] {
        assert_contract(kind, &fx.must(&["view", kind, "--json"]).json());
    }
    for (kind, id) in [
        ("checkpoint", "K-1"),
        ("gate", "plan-r1"),
        ("questions", "requirements"),
    ] {
        assert_contract(kind, &fx.must(&["view", kind, id, "--json"]).json());
    }
    let idx = fx.must(&["view", "index"]).json();
    let rows = idx["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap().clone())
        .collect::<Vec<_>>();
    assert!(rows.iter().all(|r| r["href"].is_string()), "{idx}");
    assert!(rows.iter().any(|r| r["href"] == "gate-idea-r2.html"));
}

#[test]
fn render_with_the_real_template_keeps_valid_json_and_sets_the_title() {
    let tpl = real_template();
    assert!(tpl.exists(), "views/templates/page.html is missing");
    let fx = Fx::new();
    fx.tiny();
    for (kind, id) in [
        ("index", None),
        ("plan", None),
        ("ledger", None),
        ("coverage", None),
        ("run", None),
        ("questions", None),
        ("checkpoint", Some("K-1")),
    ] {
        let outp = fx.root.join(format!("out-{kind}.html"));
        let mut a = vec!["render", kind];
        if let Some(i) = id {
            a.push(i);
        }
        a.extend([
            "--template",
            tpl.to_str().unwrap(),
            "--out",
            outp.to_str().unwrap(),
        ]);
        fx.must(&a);
        let html = fs::read_to_string(&outp).unwrap();
        assert!(!html.contains("/*BF_DATA*/"));
        let open = "id=\"bf-data\">";
        let start = html.find(open).unwrap() + open.len();
        let end = html[start..].find("</script>").unwrap();
        let v: serde_json::Value = serde_json::from_str(&html[start..start + end])
            .unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert_eq!(v["kind"], kind);
        assert_contract(kind, &v);
        let title = v["page"]["title"].as_str().unwrap();
        assert!(
            html.contains(&format!("<title>{title} \u{b7} Black Factory</title>")),
            "{kind}: title not set"
        );
    }
    // A gate by its ID, and a data string that tries to close the script element.
    fx.put("requirement", "id=\"R-9\"\nkind=\"requirement\"\ntitle=\"Break </script><script>alert(1)</script> out\"\n");
    fx.must(&["gate", "open", "requirements"]);
    let outp = fx.root.join("out-gate.html");
    fx.must(&[
        "render",
        "gate",
        "requirements-r1",
        "--template",
        tpl.to_str().unwrap(),
        "--out",
        outp.to_str().unwrap(),
    ]);
    let html = fs::read_to_string(&outp).unwrap();
    assert_eq!(
        html.matches("</script>").count(),
        fs::read_to_string(&tpl)
            .unwrap()
            .matches("</script>")
            .count(),
        "data must not add a closing script tag"
    );
}
