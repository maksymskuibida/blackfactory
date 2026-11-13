mod common;
use common::*;
use std::fs;

#[test]
fn tiny_project_end_to_end_in_a_granted_test_run() {
    let fx = Fx::new();
    fx.tiny();

    // The plan is sound, but the full ledger is not closed: nothing has proof yet.
    fx.must(&["plan", "check"]);
    fx.must(&["ledger", "check", "--plan"]);
    let o = fx.refused(&["ledger", "check"]);
    assert!(o.stderr.contains("evidence"), "{}", o.stderr);

    // Nothing starts without a plan gate.
    fx.start_test_run();
    let st = fx.must(&["status", "--json"]).json();
    assert_eq!(st["test_mode"], "granted");
    assert_eq!(st["run"], "run-1");

    // Only U-1 is ready; U-2 waits for it and for the handoff.
    let next = fx.must(&["next", "--json"]).json();
    let d = next["directives"].as_array().unwrap();
    assert_eq!(d.len(), 1, "{next}");
    assert_eq!(d[0]["unit"], "U-1");
    assert_eq!(d[0]["role"], "builder");
    assert!(next["waiting"][0].as_str().unwrap().contains("U-2"));

    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    let b = fx.must(&["brief", "U-2", "--role", "builder"]);
    let brief = fs::read_to_string(b.stdout.trim()).unwrap();
    assert!(
        brief.contains("A farewell exists") && brief.contains("the greeting wording"),
        "{brief}"
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);

    // The checkpoint cannot close until its own check has a receipt at the merged content.
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(o.stderr.contains("R-1"), "{}", o.stderr);
    fx.must_in(
        &fx.work("K-1"),
        &[
            "evidence",
            "run",
            "X-3",
            "--",
            "ls",
            "src/hello.txt",
            "src/bye.txt",
        ],
    );
    fx.must(&["checkpoint", "close", "K-1"]);

    // A simulated run can never reach the main branch.
    let o = fx.refused(&["checkpoint", "close", "K-1", "--merge"]);
    assert!(o.stderr.contains("simulated"), "{}", o.stderr);
    assert!(!fx.root.join("src/hello.txt").exists());

    fx.must(&["ledger", "check"]);

    // Every view kind emits valid contract JSON.
    for kind in [
        "index",
        "gate",
        "questions",
        "ledger",
        "plan",
        "run",
        "checkpoint",
        "coverage",
    ] {
        let v = fx.must(&["view", kind, "--json"]).json();
        assert_eq!(v["contract"], 1, "{kind}");
        assert_eq!(v["kind"], kind);
        assert!(
            v["id"].is_string() && v["page"]["title"].is_string() && v["sections"].is_array(),
            "{kind}: {v}"
        );
    }
    let cp = fx.must(&["view", "checkpoint", "K-1"]).json();
    assert_eq!(
        cp["sections"][0]["rows"][0]["handled"]["tone"], "ok",
        "{cp}"
    );

    // Render into the fixture template.
    let outp = fx.root.join("page.html");
    fx.must(&[
        "render",
        "plan",
        "--template",
        fixture("template.html").to_str().unwrap(),
        "--out",
        outp.to_str().unwrap(),
    ]);
    let html = fs::read_to_string(&outp).unwrap();
    assert!(
        html.contains("\"contract\":1") && !html.contains("/*BF_DATA*/"),
        "{html}"
    );
    assert!(
        html.contains("const BF_DATA = {"),
        "placeholder `null` should be replaced"
    );

    // The real page template, when it is present, takes the data at its marker.
    let real =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../views/templates/page.html");
    if real.exists() {
        let outp = fx.root.join("real.html");
        for kind in ["index", "plan", "ledger", "coverage", "run", "checkpoint"] {
            fx.must(&[
                "render",
                kind,
                "--template",
                real.to_str().unwrap(),
                "--out",
                outp.to_str().unwrap(),
            ]);
            let html = fs::read_to_string(&outp).unwrap();
            let start = html.find("id=\"bf-data\">").unwrap() + "id=\"bf-data\">".len();
            let end = html[start..].find("</script>").unwrap();
            let v: serde_json::Value = serde_json::from_str(&html[start..start + end]).unwrap();
            assert_eq!(v["kind"], kind);
        }
    }

    // The log is intact and the doctor is happy.
    fx.must(&["log", "--verify"]);
    fx.must(&["doctor"]);
}
