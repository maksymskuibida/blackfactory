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

fn sample(name: &str) -> serde_json::Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../views/samples")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(p).unwrap_or_else(|_| panic!("sample {name}")))
        .unwrap()
}

#[test]
fn each_new_page_kind_has_a_sample_that_validates() {
    for kind in ["workspace", "space", "rules"] {
        let v = sample(kind);
        assert_eq!(v["kind"], kind);
        assert_contract(kind, &v);
    }
    // the workspace page leaves out what it has not; a space page has a space and no intent
    assert!(
        sample("workspace").get("space").is_none() && sample("workspace").get("intent").is_none()
    );
    assert!(sample("space").get("space").is_some() && sample("space").get("intent").is_none());
    for n in ["checkpoint-fix", "run-cost", "plan-tests", "gate-links"] {
        let v = sample(n);
        assert_contract(n, &v);
        assert!(v["intent"].is_string() && v["space"].is_string(), "{n}");
        assert!(
            v["page"]["context"]
                .as_array()
                .map(|c| c.len() == 3)
                .unwrap_or(false),
            "{n}: the strip holds the space, the intent and the state"
        );
    }
}

#[test]
fn the_contract_grows_by_additions_only_and_an_old_page_validates_unchanged() {
    let schema = view_schema();
    assert_eq!(schema["properties"]["contract"]["const"], 1);
    let kinds: Vec<&str> = schema["properties"]["kind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    for k in [
        "index",
        "gate",
        "questions",
        "ledger",
        "plan",
        "run",
        "checkpoint",
        "coverage",
        "report",
    ] {
        assert!(kinds.contains(&k), "kind {k} was removed");
    }
    assert_eq!(
        schema["required"],
        serde_json::json!(["contract", "kind", "id", "page", "sections"])
    );
    let row = &schema["$defs"]["row"]["properties"];
    for f in [
        "id",
        "title",
        "label",
        "look_first",
        "needs_you",
        "round",
        "round_note",
        "handled",
        "body",
        "refs",
        "picks",
        "refs_label",
        "note",
        "href",
        "meta",
    ] {
        assert!(row.get(f).is_some(), "row field {f} was removed");
    }
    // a page of an old project: none of the new fields, still valid
    for n in [
        "index",
        "gate",
        "questions",
        "ledger",
        "plan",
        "run",
        "checkpoint",
        "coverage",
        "plan-gate",
    ] {
        let v = sample(n);
        assert!(
            v.get("space").is_none() && v["page"].get("context").is_none(),
            "{n} is an old page"
        );
        assert_contract(n, &v);
    }
    // a page of a new kind built without the new fields is valid too
    let bare = serde_json::json!({"contract": 1, "kind": "rules", "id": "r", "page": {"title": "t"}, "sections": []});
    assert_contract("bare rules page", &bare);
    // the new parts are checked: a tag needs its text, a context item its label and value, a tone is one of the set
    let mut v = sample("space");
    v["sections"][0]["rows"][0]["tags"] = serde_json::json!([{"tone": "ok"}]);
    assert!(!schema::validate(&schema, &v).is_empty());
    let mut v = sample("space");
    v["page"]["context"] = serde_json::json!([{"value": "x"}]);
    assert!(!schema::validate(&schema, &v).is_empty());
    let mut v = sample("rules");
    v["sections"][0]["rows"][0]["tags"][0]["tone"] = serde_json::json!("purple");
    assert!(!schema::validate(&schema, &v).is_empty());
}

/// D-37, D-62: a label in a new sample is the text of a key of the unit's own strings file.
#[test]
fn every_label_in_the_new_samples_is_a_key_text_of_the_strings_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views");
    let strings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("contract/strings.json")).unwrap())
            .unwrap();
    let known: Vec<&str> = strings
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(known.iter().all(|t| !t.is_empty()));
    let mut seen = 0;
    for n in [
        "workspace",
        "space",
        "rules",
        "checkpoint-fix",
        "run-cost",
        "plan-tests",
        "gate-links",
    ] {
        let v = sample(n);
        let mut labels: Vec<String> = Vec::new();
        let mut add = |x: &serde_json::Value| {
            if let Some(s) = x.as_str() {
                labels.push(s.to_string())
            }
        };
        add(&v["page"]["context_label"]);
        add(&v["page"]["status"]["text"]);
        // crumbs: a name of the invented data (a space, an intent, a checkpoint) is not a label
        let names: Vec<String> = v["page"]["context"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["value"].as_str().map(String::from))
            .collect();
        for c in v["page"]["crumbs"].as_array().into_iter().flatten() {
            let t = c.as_str().unwrap_or("");
            let is_name = names.iter().any(|x| x == t) || t.starts_with("K-") || t == "Billing";
            if !is_name {
                add(c);
            }
        }
        for c in v["page"]["context"].as_array().into_iter().flatten() {
            add(&c["label"]);
        }
        for s in v["sections"].as_array().unwrap() {
            add(&s["title"]);
            for b in s["blocks"].as_array().into_iter().flatten() {
                add(&b["label"]);
                add(&b["unit"]);
            }
            for r in s["rows"].as_array().into_iter().flatten() {
                add(&r["refs_label"]);
                for t in r["tags"].as_array().into_iter().flatten() {
                    if t["text"] != "Unheard-of kind" {
                        add(&t["text"]);
                    }
                }
                // the pick texts of a decision (gate-links) are invented data
                if n != "gate-links" {
                    for p in r["picks"].as_array().into_iter().flatten() {
                        add(&p["text"]);
                    }
                }
                for b in r["body"].as_array().into_iter().flatten() {
                    add(&b["label"]);
                    for h in b["head"].as_array().into_iter().flatten() {
                        add(h);
                    }
                }
            }
        }
        for l in labels {
            assert!(
                known.contains(&l.as_str()),
                "{n}: the label {l:?} is not in views/contract/strings.json"
            );
            seen += 1;
        }
    }
    assert!(seen > 40, "{seen}");
}
