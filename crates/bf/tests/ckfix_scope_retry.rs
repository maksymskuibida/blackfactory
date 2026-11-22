//! D-115: a checkpoint fix's request for files is refused whatever it holds, so no round of the old retry
//! rule is counted and no refusal is recorded.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const K_CHECK: &str = "grep -q fixed src/a.txt";
const A: &str = "tests/attacks.rs";
const M: &str = "tests/common/mod.rs";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{K_CHECK}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fs::create_dir_all(fx.root.join("tests/common")).unwrap();
    fs::write(fx.root.join(M), "pub fn helper() {}\n").unwrap();
    fs::write(fx.root.join(A), "// attacks\nfn a() {}\n").unwrap();
    fs::write(fx.root.join("tests/other.rs"), "// other\nfn o() {}\n").unwrap();
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
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

fn one_directive(fx: &Fx, action: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "one directive while a fix is open: {n}");
    assert_eq!(ds[0]["action"], action, "{n}");
    ds[0].clone()
}

fn open_fix(fx: &Fx) -> PathBuf {
    plan(fx);
    land_unit(fx);
    let k = fx.work("K-1");
    let o = fx.bf_in(
        &k,
        &[
            "evidence",
            "run",
            "X-2",
            "--",
            "grep",
            "-q",
            "fixed",
            "src/a.txt",
        ],
    );
    assert!(!o.all().is_empty());
    one_directive(fx, "fix-checkpoint");
    k
}

fn ask(fx: &Fx, k: &Path, files: Value) -> Out {
    fx.bf_stdin(
        k,
        &[
            "report",
            "K-1",
            "--role",
            "builder",
            "--result",
            "blocked",
            "--need-files",
            "-",
        ],
        &files.to_string(),
    )
}

fn patch_for(path: &str, add: &str) -> String {
    format!("--- a/{path}\n+++ b/{path}\n@@ -1 +1,2 @@\n context\n+{add}\n")
}

fn req(path: &str, add: &str) -> Value {
    json!({"path": path, "why": "the fix needs it", "patch": patch_for(path, add)})
}

fn builder_brief(fx: &Fx) -> String {
    fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap()
}

#[test]
fn ckfix_scope_retry_a_request_counts_no_round_and_records_no_refusal() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    for add in ["one", "two"] {
        let o = ask(&fx, &k, json!([req(A, add), req(M, add)]));
        assert_eq!(o.code, 2, "{}", o.all());
    }
    let st = bf::events::replay(&fx.project()).unwrap();
    let f = st.ck_fix.clone().unwrap();
    assert_eq!(
        (f.refused.len(), f.grants.len(), st.ignored.len()),
        (0, 0, 0)
    );
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d.get("ask_again").is_none(), "{d}");
    assert!(!builder_brief(&fx).contains("--need-files"));
}
