//! The fix of the K-3 checkpoint review, round 7: `bf evidence run <check> -- bf scan ...` runs the scan
//! in the engine's own process and writes its receipt at the content it scanned, and a scan refuses a
//! receipt when the files changed while its tool ran.

mod common;
use common::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn standin(fx: &Fx, body: &str) {
    let bin = fx.root.parent().unwrap().join("standins");
    fs::create_dir_all(&bin).unwrap();
    fx.set_policy(
        "/usr/bin:/bin:/usr/sbin:/sbin",
        &format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    let g = bin.join("gitleaks");
    fs::write(&g, format!("#!/bin/sh\n{body}echo '[]'\nexit 0\n")).unwrap();
    fs::set_permissions(&g, fs::Permissions::from_mode(0o755)).unwrap();
}

fn scan_project(tool_body: &str) -> Fx {
    let fx = Fx::new();
    standin(&fx, tool_body);
    fx.tiny();
    for (id, sub, name) in [
        ("X-70", "secrets", "secret-scan"),
        ("X-71", "licences", "dependency-licence-check"),
    ] {
        let t = format!("id=\"{id}\"\nkind=\"check\"\ntitle=\"{name}\"\nhow=\"command\"\ncommand=\"bf scan {sub}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\nnamed=\"{name}\"\n");
        let o = fx.put("check", &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let k = "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Both messages\"\nafter=[\"U-1\",\"U-2\"]\nproves=[\"R-1\",\"R-2\",\"C-1\"]\nchecks=[\"X-3\",\"X-70\",\"X-71\"]\n";
    let f = fx.root.join("k1.toml");
    fs::write(&f, k).unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "checkpoint",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.commit_all("scans");
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx
}

fn receipts(fx: &Fx) -> usize {
    fs::read_dir(fx.root.join("factory/receipts"))
        .map(|d| d.count())
        .unwrap_or(0)
}

#[test]
fn a_checkpoint_closes_with_both_scans_run_by_evidence_run() {
    let fx = scan_project("");
    let k = fx.work("K-1");
    // The way the verifier's role file names: each command check through `bf evidence run`.
    let a = fx.bf_in(
        &k,
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
    assert_eq!(a.code, 0, "{}", a.all());
    let a = fx.bf_in(
        &k,
        &["evidence", "run", "X-70", "--", "bf", "scan", "secrets"],
    );
    assert_eq!(a.code, 0, "{}", a.all());
    let b = fx.bf_in(
        &k,
        &["evidence", "run", "X-71", "--", "bf", "scan", "licences"],
    );
    assert_eq!(b.code, 0, "{}", b.all());
    assert!(b.all().contains("X-71"), "{}", b.all());
    let v = fx.bf_in(&k, &["evidence", "verify"]);
    assert!(
        v.code == 0 && v.all().contains("X-70") && v.all().contains("X-71"),
        "{}",
        v.all()
    );
    // The scan receipts are for the copy's content, and the close takes them as the proof.
    let o = fx.bf(&["checkpoint", "close", "K-1"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn evidence_run_of_a_scan_names_the_check_it_was_given() {
    let fx = scan_project("");
    let k = fx.work("K-1");
    // The licence scan is not the proof of the secret check, and a command that is not the check's own
    // is refused as before.
    let o = fx.bf_in(
        &k,
        &["evidence", "run", "X-70", "--", "bf", "scan", "licences"],
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert_four_parts(&o.stderr);
}

#[test]
fn a_scan_whose_tool_changes_the_files_writes_no_receipt() {
    let fx = scan_project("echo $$ >> raced.txt\n");
    let k = fx.work("K-1");
    let n = receipts(&fx);
    // Directly.
    let o = fx.bf_in(&k, &["scan", "secrets"]);
    assert_ne!(o.code, 0, "{}", o.all());
    assert_four_parts(&o.stderr);
    assert!(
        o.stderr.contains("changed while the scan ran"),
        "{}",
        o.stderr
    );
    assert_eq!(receipts(&fx), n, "a refused scan wrote a receipt");
    // Through `bf evidence run`.
    let o = fx.bf_in(
        &k,
        &["evidence", "run", "X-70", "--", "bf", "scan", "secrets"],
    );
    assert_ne!(o.code, 0, "{}", o.all());
    assert_four_parts(&o.stderr);
    assert!(
        o.stderr.contains("changed while the scan ran"),
        "{}",
        o.stderr
    );
    assert_eq!(receipts(&fx), n, "a refused scan wrote a receipt");
}
