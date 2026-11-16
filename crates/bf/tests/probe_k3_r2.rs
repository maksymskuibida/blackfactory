//! Probes of the second checkpoint review of K-3. Each test states the right answer and fails while
//! the fault is there.

mod common;
use common::Fx;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

/// q1: `bf scan licences --check <ID>` must not prove a check whose command is not that scan.
#[test]
fn q1_a_scan_named_for_another_check_proves_nothing() {
    let fx = Fx::new();
    fx.tiny();
    // A check of U-1 that can never pass.
    let o = fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"Never\"\nhow=\"command\"\ncommand=\"false\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("x9");
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    fx.must_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/hello.txt"]);
    // X-9 is never run. From the project folder the licence scan is pointed at the working copy and
    // named for X-9: it finds no lockfile, so it passes.
    let scan = fx.bf(&["-C", wc.to_str().unwrap(), "scan", "licences", "--check", "X-9", "--unit", "U-1"]);
    let verify = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    let built = fx.bf(&["report", "U-1", "--result", "built"]);
    let clean = fx.bf(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    let land = fx.bf(&["unit", "land", "U-1"]);
    eprintln!(
        "scan {} | verify {} | built {} | clean {} | land {}\n{}\n{}\n{}",
        scan.code, verify.code, built.code, clean.code, land.code,
        scan.all(), verify.all(), land.all()
    );
    assert!(
        scan.code != 0 || verify.code != 0,
        "a licence scan wrote a passing receipt for X-9, whose command it is not, and verify took it"
    );
    assert_ne!(land.code, 0, "U-1 landed though its check X-9 never ran and cannot pass");
}

/// q3: a scan can be run where its receipt counts, or the refusal says how.
#[test]
fn q3_a_scan_can_be_proven_at_the_content_of_a_working_copy() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new();
    let bin = fx.root.parent().unwrap().join("standins");
    fs::create_dir_all(&bin).unwrap();
    fx.set_policy(
        "/usr/bin:/bin:/usr/sbin:/sbin",
        &format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    let g = bin.join("gitleaks");
    fs::write(&g, "#!/bin/sh\necho '[]'\nexit 0\n").unwrap();
    fs::set_permissions(&g, fs::Permissions::from_mode(0o755)).unwrap();
    fx.tiny();
    let o = fx.put("check", "id=\"X-70\"\nkind=\"check\"\ntitle=\"secret-scan\"\nhow=\"command\"\ncommand=\"bf scan secrets\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\nnamed=\"secret-scan\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("x70");
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
    // Where a check's command runs: the working copy.
    let inside = fx.bf_in(&wc, &["scan", "secrets"]);
    // Where the refusal sends the caller: the project folder.
    let outside = fx.bf(&["scan", "secrets", "--unit", "U-1"]);
    let verify = fx.bf_in(&wc, &["evidence", "verify", "--unit", "U-1"]);
    eprintln!(
        "inside {} | outside {} | verify {}\n{}\n{}\n{}",
        inside.code, outside.code, verify.code, inside.all(), outside.all(), verify.all()
    );
    let counted = !verify.all().contains("NOT MET X-70");
    assert!(
        inside.code == 0 || counted || inside.stderr.contains("-C"),
        "the scan is refused where its check runs, the run the refusal names does not count at the working copy's content, and nothing names the form that does"
    );
}

fn file(fx: &Fx, name: &str, v: &Value) -> String {
    let f = fx.root.join(name);
    fs::write(&f, serde_json::to_string(v).unwrap()).unwrap();
    f.to_str().unwrap().to_string()
}

/// q2: an acknowledged legal finding whose record is changed by a plain put is no longer acknowledged.
#[test]
fn q2_a_changed_legal_finding_needs_its_acknowledgement_again() {
    let fx = Fx::new();
    let f = json!({"title": "Consent for tracking", "markets": ["Atlantis"],
        "risk": "consent", "researched": false, "lawyer_check": true, "severity": "low",
        "text": "A small matter."});
    fx.must(&["legal", "finding", &file(&fx, "f.json", &f)]);
    let q = fx.must(&["legal", "questions", "--json"]).json();
    let qid = q["asked"][0]["question"].as_str().unwrap().to_string();
    sleep(Duration::from_millis(1100));
    fx.human("Yes, I acknowledge it.");
    fx.must(&["answer", &qid, "--file", &file(&fx, "a.json", &json!({"choice": "A"}))]);
    assert!(bf::legal::unacknowledged(&fx.project()).unwrap().is_empty());
    // The record is rewritten after the acknowledgement, with no word of the human.
    let mut rec = fx.must(&["record", "get", "L-1", "--json"]).json();
    rec["title"] = json!("Selling personal data without consent");
    rec["text"] = json!("A grave matter the human never read.");
    rec["severity"] = json!("critical");
    let put = fx.bf(&["record", "put", "legal", "--file", &file(&fx, "l.json", &rec), "--replace"]);
    let open = bf::legal::unacknowledged(&fx.project()).unwrap();
    eprintln!("put {}: {}\nunacknowledged: {open:?}", put.code, put.all());
    assert!(
        put.code != 0 || !open.is_empty(),
        "L-1 was rewritten after the human acknowledged it and still counts as acknowledged"
    );
}

/// q4: `bf preset fill` writes its draft only where a draft may go, never over a protected file.
#[test]
fn q4_a_preset_draft_is_not_written_over_a_record_or_the_hook_settings() {
    let fx = Fx::new();
    fx.tiny();
    let o = fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"Never\"\nhow=\"command\"\ncommand=\"false\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("x9");
    fx.start_test_run();
    // The plan gate is approved: X-9 changes only by amendment.
    let o = fx.put("check", "id=\"X-9\"\nkind=\"check\"\ntitle=\"Never\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n");
    assert_ne!(o.code, 0, "a plain put of X-9 after the plan gate must be refused");
    // A preset of the project's own whose draft mark is a comment and whose body is a record.
    fs::create_dir_all(fx.root.join("presets")).unwrap();
    fs::write(
        fx.root.join("presets/note.en.toml"),
        "kind = \"note\"\nlanguage = \"en\"\ntitle = \"note\"\ndraft_mark = \"# DRAFT\"\nbody = '''\nid = \"X-9\"\nkind = \"check\"\ntitle = \"Never\"\nhow = \"command\"\ncommand = \"true\"\nproves = [\"R-1\"]\nscope = \"unit\"\nunit = \"U-1\"\n'''\n",
    )
    .unwrap();
    fs::write(fx.root.join("v.json"), "{}").unwrap();
    let rec = fx.root.join("factory/records/X-9.toml");
    let before = fs::read_to_string(&rec).unwrap();
    let fill = fx.bf(&["preset", "fill", "note", "en", "v.json", "factory/records/X-9.toml"]);
    let after = fs::read_to_string(&rec).unwrap();
    let hooks = fx.root.join(".claude/settings.json");
    let hooks_before = fs::read_to_string(&hooks).ok();
    let fill2 = fx.bf(&["preset", "fill", "note", "en", "v.json", ".claude/settings.json"]);
    let hooks_after = fs::read_to_string(&hooks).ok();
    // What the rewritten record then lets through.
    let o = fx.bf(&["unit", "start", "U-1", "--json"]);
    let mut tail = format!("start {}", o.code);
    if o.code == 0 {
        let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
        fs::create_dir_all(wc.join("src")).unwrap();
        fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
        let a = fx.bf_in(&wc, &["evidence", "run", "X-1", "--unit", "U-1", "--", "test", "-f", "src/hello.txt"]);
        let b = fx.bf_in(&wc, &["evidence", "run", "X-9", "--unit", "U-1", "--", "true"]);
        let c = fx.bf(&["report", "U-1", "--result", "built"]);
        let d = fx.bf(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
        let e = fx.bf(&["unit", "land", "U-1"]);
        tail = format!(
            "start 0 | X-1 {} | X-9 {} | built {} | clean {} | land {}\n{}\n{}",
            a.code, b.code, c.code, d.code, e.code, b.all(), e.all()
        );
    }
    eprintln!(
        "fill record {}: {}\nrecord changed: {}\nfill hooks {}: {}\nhooks changed: {}\n{tail}",
        fill.code,
        fill.all(),
        before != after,
        fill2.code,
        fill2.all(),
        hooks_before != hooks_after
    );
    assert!(before == after, "the record X-9 was rewritten by `bf preset fill` after the plan gate");
    assert!(hooks_before == hooks_after, "the hook settings were written by `bf preset fill`");
}
