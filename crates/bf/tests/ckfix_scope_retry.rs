//! R-70, D-89, D-91: the scope reviewer answers a checkpoint fix's file request with `retry` and the words
//! a corrected patch must meet; a request is answered by exactly one event, and nothing is granted beside
//! a `retry` or an `extension`.

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
        &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/a.txt"],
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
        &["evidence", "run", "X-2", "--", "grep", "-q", "fixed", "src/a.txt"],
    );
    assert!(!o.all().is_empty());
    one_directive(fx, "fix-checkpoint");
    k
}

fn ask(fx: &Fx, k: &Path, files: Value) -> Out {
    fx.bf_stdin(
        k,
        &["report", "K-1", "--role", "builder", "--result", "blocked", "--need-files", "-"],
        &files.to_string(),
    )
}

fn answer(fx: &Fx, v: Value) -> Out {
    fx.bf_stdin(&fx.root, &["checkpoint", "scope", "K-1", "--verdict", "-"], &v.to_string())
}

fn patch_for(path: &str, add: &str) -> String {
    format!("--- a/{path}\n+++ b/{path}\n@@ -1 +1,2 @@\n context\n+{add}\n")
}

fn req(path: &str, add: &str) -> Value {
    json!({"path": path, "why": "the fix needs it", "patch": patch_for(path, add)})
}

fn builder_brief(fx: &Fx) -> String {
    fs::read_to_string(fx.must(&["brief", "K-1", "--role", "builder"]).stdout.trim()).unwrap()
}

fn fix_state(fx: &Fx) -> (Vec<Value>, usize, usize) {
    let st = bf::events::replay(&fx.project()).unwrap();
    let f = st.ck_fix.clone().unwrap();
    (f.refused, f.grants.len(), st.ignored.len())
}

#[test]
fn ckfix_scope_retry_a_retry_grants_nothing_and_the_builder_is_told_the_must() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    assert_eq!(ask(&fx, &k, json!([req(A, "one")])).code, 0);
    let o = answer(
        &fx,
        json!([{"path": A, "verdict": "retry", "why": "removes a line", "must": "show the removed line"}]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d["granted"].as_array().unwrap().is_empty(), "{d}");
    assert!(d["ask_again"].to_string().contains("show the removed line"), "{d}");
    assert!(d["ask_again"].to_string().contains(A), "{d}");
    let b = builder_brief(&fx);
    assert!(b.contains(A) && b.contains("show the removed line") && b.contains("--need-files -"), "{b}");
    let (refused, grants, ignored) = fix_state(&fx);
    assert_eq!((refused.len(), grants, ignored), (1, 0, 0));
}

#[test]
fn ckfix_scope_retry_a_needed_and_a_retry_file_write_one_refuse_and_no_grant() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    assert_eq!(ask(&fx, &k, json!([req(M, "m"), req(A, "one")])).code, 0);
    let o = answer(
        &fx,
        json!([
            {"path": M, "verdict": "needed", "why": "called by the fix"},
            {"path": A, "verdict": "retry", "why": "short", "must": "name the removed line"},
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d["granted"].as_array().unwrap().is_empty(), "{d}");
    let (refused, grants, ignored) = fix_state(&fx);
    assert_eq!((refused.len(), grants, ignored), (1, 0, 0));
    let r = &refused[0];
    assert_eq!(r["refusal"], "retry", "{r}");
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{r}");
    let m = files.iter().find(|x| x["path"] == M).unwrap();
    let a = files.iter().find(|x| x["path"] == A).unwrap();
    assert_eq!(m["verdict"], "needed");
    assert_eq!(a["verdict"], "retry");
    assert_eq!(a["must"], "name the removed line");
    assert_eq!(a["round"], 1);
    assert!(a["patch"].as_str().unwrap().contains("one"), "{a}");
    assert_eq!(bf::scope::retry::rounds_used(&refused, A), 1);
    assert_eq!(bf::scope::retry::rounds_used(&refused, M), 0);
    let b = builder_brief(&fx);
    assert!(b.contains("name the removed line") && b.contains(M), "{b}");
    // The needed file is asked for again with its same patch; the retry file with a corrected one.
    let o = ask(&fx, &k, json!([req(M, "m"), req(A, "two")]));
    assert_eq!(o.code, 0, "{}", o.all());
    one_directive(&fx, "review-scope");
    let o = answer(
        &fx,
        json!([
            {"path": M, "verdict": "needed", "why": "called by the fix"},
            {"path": A, "verdict": "needed", "why": "now right"},
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["granted"].as_array().unwrap().len(), 2, "{d}");
    // The grants hold for this fix only.
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d["granted"].as_array().unwrap().is_empty(), "{d}");
}

#[test]
fn ckfix_scope_retry_an_extension_beside_a_retry_grants_nothing_and_counts_no_round() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let ext = "tests/other.rs";
    let _ = &k;
    assert_eq!(ask(&fx, &k, json!([req(M, "m"), req(A, "one"), req(ext, "x")])).code, 0);
    let o = answer(
        &fx,
        json!([
            {"path": M, "verdict": "needed", "why": "called by the fix"},
            {"path": A, "verdict": "retry", "why": "short", "must": "name the removed line"},
            {"path": ext, "verdict": "extension", "why": "another feature"},
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d["granted"].as_array().unwrap().is_empty(), "{d}");
    assert!(d["not_granted"].to_string().contains(ext), "{d}");
    let (refused, grants, ignored) = fix_state(&fx);
    assert_eq!((refused.len(), grants, ignored), (1, 0, 0));
    assert_eq!(refused[0]["refusal"], "extension");
    assert_eq!(bf::scope::retry::rounds_used(&refused, A), 0);
    let a = refused[0]["files"].as_array().unwrap().iter().find(|x| x["path"] == A).unwrap().clone();
    assert_eq!((a["verdict"].as_str().unwrap(), a["must"].as_str().unwrap()), ("retry", "name the removed line"));
    let b = builder_brief(&fx);
    assert!(b.contains("name the removed line") && b.contains(ext) && b.contains("--result gap"), "{b}");
    // The extension file is refused naming it.
    let o = ask(&fx, &k, json!([req(ext, "y")]));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains(ext), "{}", o.stderr);
    // The corrected request without it is judged, and a needed answer grants for this fix.
    assert_eq!(ask(&fx, &k, json!([req(M, "m"), req(A, "two")])).code, 0);
    let o = answer(
        &fx,
        json!([
            {"path": M, "verdict": "needed", "why": "called by the fix"},
            {"path": A, "verdict": "needed", "why": "now right"},
        ]),
    );
    assert_eq!(o.code, 0, "{}", o.all());
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["granted"].as_array().unwrap().len(), 2, "{d}");
}

#[test]
fn ckfix_scope_retry_a_file_is_judged_the_limit_of_times_then_goes_to_the_human() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let max = fx.project().policy.limits.max_rounds;
    for round in 1..=max {
        let add = format!("r{round}");
        let o = ask(&fx, &k, json!([req(A, &add)]));
        assert_eq!(o.code, 0, "round {round}: {}", o.all());
        let o = answer(
            &fx,
            json!([{"path": A, "verdict": "retry", "why": "still short", "must": "do more"}]),
        );
        assert_eq!(o.code, 0, "{}", o.all());
        if round < max {
            // The same patch again is refused naming the file.
            let o = ask(&fx, &k, json!([req(A, &add)]));
            assert_eq!(o.code, 2, "{}", o.all());
            assert!(o.stderr.contains(A), "{}", o.stderr);
        }
    }
    let (refused, grants, ignored) = fix_state(&fx);
    assert_eq!((refused.len() as u32, grants, ignored), (max, 0, 0));
    assert_eq!(refused.last().unwrap()["refusal"], "retry_spent");
    assert_eq!(refused.last().unwrap()["files"][0]["verdict"], "retry_spent");
    assert!(bf::scope::retry::spent(&refused, A, max));
    let d = one_directive(&fx, "fix-checkpoint");
    assert!(d["not_granted"].to_string().contains(A), "{d}");
    assert!(builder_brief(&fx).contains("--result gap"));
    let o = ask(&fx, &k, json!([req(A, "again")]));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains(A) && o.stderr.contains("spent"), "{}", o.stderr);
    // The way on is a gap report naming the file, which goes to the human.
    let o = fx.bf_in(
        &k,
        &["report", "K-1", "--role", "builder", "--result", "gap", "--note", &format!("{A}: still needed")],
    );
    assert_eq!(o.code, 2, "{}", o.all());
    let d = one_directive(&fx, "plan-gap");
    assert_eq!(d["role"], "human", "{d}");
    assert!(d["missing"].to_string().contains(A), "{d}");
}

#[test]
fn ckfix_scope_retry_a_file_already_in_the_list_is_refused_and_a_new_fix_has_fresh_rounds() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let o = ask(&fx, &k, json!([req("src/a.txt", "x")]));
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("src/a.txt"), "{}", o.stderr);
    assert_eq!(ask(&fx, &k, json!([req(A, "one")])).code, 0);
    answer(
        &fx,
        json!([{"path": A, "verdict": "retry", "why": "short", "must": "do more"}]),
    );
    assert_eq!(fix_state(&fx).0.len(), 1);
    fx.must(&["checkpoint", "fix", "K-1", "--abort"]);
    one_directive(&fx, "fix-checkpoint");
    let (refused, grants, _) = fix_state(&fx);
    assert!(refused.is_empty() && grants == 0);
    assert_eq!(ask(&fx, &k, json!([req(A, "one")])).code, 0);
    one_directive(&fx, "review-scope");
}
