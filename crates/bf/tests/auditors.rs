//! U-35: the auditors really do their part. A built-stage role is briefed for a checkpoint and reports
//! from the checkpoint's working copy with its token; an auditor files a finding for its unit in any
//! state after the unit has started, with its token; nothing else gets through.

mod common;
use common::*;
use std::fs;

const FINDING: &str = "kind=\"finding\"\ntitle=\"A secret in the diff\"\nunit=\"U-1\"\nseverity=\"high\"\nwhere=\"src/hello.txt\"\naction=\"Remove the key and rotate it\"\n";

fn token(fx: &Fx, subject: &str, role: &str) -> String {
    fx.must(&["roles", "token", subject, role, "--json"]).json()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn put_in(fx: &Fx, dir: &std::path::Path, kind: &str, text: &str, tok: &str) -> Out {
    let f = dir.join("rec.toml");
    fs::write(&f, text).unwrap();
    fx.bf_in(
        dir,
        &[
            "record",
            "put",
            kind,
            "--file",
            f.to_str().unwrap(),
            "--token",
            tok,
        ],
    )
}

/// U-1 started and still running (not in review), with tokens on.
fn running_unit(fx: &Fx) -> std::path::PathBuf {
    fx.tiny();
    fx.start_test_run();
    fx.set_policy("tokens = false", "tokens = true");
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap())
}

#[test]
fn a_built_stage_role_is_briefed_for_a_checkpoint() {
    let fx = Fx::tiny_project();
    for role in ["security-auditor", "legal-auditor", "uiux-verifier"] {
        let v = fx.must(&["brief", "K-1", "--role", role, "--json"]).json();
        let path = v["path"].as_str().unwrap();
        assert!(path.ends_with(&format!("K-1-{role}.md")), "{path}");
        let md = fs::read_to_string(path).unwrap();
        assert!(md.contains(&format!("# Brief: K-1 as {role}")), "{md}");
        assert!(md.contains("Checkpoint: Both messages"), "{md}");
        assert!(md.contains("R-1") && md.contains("R-2"), "{md}");
    }
    // A design-stage role is not briefed for a checkpoint.
    fx.refused(&["brief", "K-1", "--role", "security-expert"]);
    fx.refused(&["brief", "K-9", "--role", "security-auditor"]);
}

#[test]
fn a_finding_from_a_working_copy_never_replaces_one_that_is_filed() {
    let fx = Fx::new();
    let wc = running_unit(&fx);
    let tok = token(&fx, "U-1", "security-auditor");
    let first = format!("id=\"F-1\"\n{FINDING}");
    let o = put_in(&fx, &wc, "finding", &first, &tok);
    assert_eq!(o.code, 0, "{}", o.all());
    // The same ID again, reworded and with `--replace`, is refused and the record stays as filed.
    let again = first.replace("A secret in the diff", "Nothing to see");
    let f = wc.join("again.toml");
    fs::write(&f, &again).unwrap();
    for replace in [false, true] {
        let mut args = vec![
            "record",
            "put",
            "finding",
            "--file",
            f.to_str().unwrap(),
            "--token",
            &tok,
        ];
        if replace {
            args.push("--replace");
        }
        let o = fx.bf_in(&wc, &args);
        assert!(o.code != 0, "{}", o.all());
        assert!(o.stderr.contains("exists"), "{}", o.stderr);
    }
    let got = fx.must(&["record", "get", "F-1"]);
    assert!(
        got.stdout.contains("A secret in the diff"),
        "{}",
        got.stdout
    );
    assert!(!got.stdout.contains("Nothing to see"), "{}", got.stdout);
}

#[test]
fn an_auditor_files_a_finding_while_the_unit_is_still_running() {
    let fx = Fx::new();
    let wc = running_unit(&fx);
    let tok = token(&fx, "U-1", "security-auditor");
    let o = put_in(&fx, &wc, "finding", FINDING, &tok);
    assert_eq!(o.code, 0, "{}", o.all());
    println!("accepted beyond the review window: {}", o.stdout);
    let list = fx.must(&["record", "list", "--kind", "finding"]);
    assert!(
        list.stdout.contains("A secret in the diff"),
        "{}",
        list.stdout
    );
}

#[test]
fn a_finding_needs_the_token_and_the_units_own_copy() {
    let fx = Fx::new();
    let wc = running_unit(&fx);
    // No token, a wrong token, and the token of a role that files no findings.
    assert!(put_in(&fx, &wc, "finding", FINDING, "").code != 0);
    assert!(put_in(&fx, &wc, "finding", FINDING, "0123456789abcdef01234567").code != 0);
    let other = token(&fx, "U-1", "uiux-verifier");
    assert!(put_in(&fx, &wc, "finding", FINDING, &other).code != 0);
    // A token for another unit does not open this one.
    let tok = token(&fx, "U-2", "security-auditor");
    assert!(put_in(&fx, &wc, "finding", FINDING, &tok).code != 0);
    // A unit that has not started takes no finding, even with the token of a started copy.
    let tok = token(&fx, "U-1", "security-auditor");
    let text = FINDING.replace("U-1", "U-2");
    assert!(put_in(&fx, &wc, "finding", &text, &tok).code != 0);
    // A finding is never born fixed or accepted.
    let text = format!("{FINDING}outcome=\"accepted\"\n");
    assert!(put_in(&fx, &wc, "finding", &text, &tok).code != 0);
    assert!(!fx
        .must(&["record", "list", "--kind", "finding"])
        .stdout
        .contains("A secret"));
}

#[test]
fn an_auditor_reports_and_files_from_the_checkpoints_working_copy() {
    let fx = Fx::tiny_project();
    // A unit of a second checkpoint, which the first checkpoint's auditor may not file findings for.
    for (kind, text) in [
        ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Other\"\ndelivers=[\"R-2\"]\nfiles=[\"src/other.txt\"]\ncheckpoint=\"K-0\"\nestimate=1\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"Other file\"\nhow=\"command\"\ncommand=\"test -f src/other.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"Other file again\"\nhow=\"command\"\ncommand=\"test -f src/other.txt\"\nproves=[\"R-2\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-0\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Other\"\nafter=[\"U-3\"]\nproves=[\"R-2\"]\nchecks=[\"X-5\"]\n"),
    ] {
        let o = fx.put(kind, text);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("more records");
    fx.start_test_run();
    // The first checkpoint is built and merged before the second one starts.
    fx.build_unit(
        "U-3",
        "src/other.txt",
        "X-4",
        &["test", "-f", "src/other.txt"],
    );
    fx.must_in(
        &fx.work("K-0"),
        &[
            "evidence",
            "run",
            "X-5",
            "--",
            "test",
            "-f",
            "src/other.txt",
        ],
    );
    fx.must(&["checkpoint", "close", "K-0"]);
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.set_policy("tokens = false", "tokens = true");
    let k = fx.work("K-1");
    assert!(k.exists(), "the checkpoint has a working copy");
    let tok = token(&fx, "K-1", "security-auditor");
    // No token, and the token of a role that files no findings: refused, with no fall back to the
    // token-less worker rule.
    assert!(put_in(&fx, &k, "finding", FINDING, "").code != 0);
    let verifier = token(&fx, "K-1", "uiux-verifier");
    assert!(put_in(&fx, &k, "finding", FINDING, &verifier).code != 0);
    let other = FINDING.replace("U-1", "U-9");
    assert!(put_in(&fx, &k, "finding", &other, &tok).code != 0);
    // A started unit of another checkpoint is not this checkpoint's unit.
    let outside = FINDING.replace("U-1", "U-3");
    assert!(put_in(&fx, &k, "finding", &outside, &tok).code != 0);
    // A finding record from the checkpoint's copy is refused: an auditor files through its audit round (D-164).
    let o = put_in(&fx, &k, "finding", FINDING, &tok);
    assert!(o.code != 0, "{}", o.all());
    assert!(o.all().contains("bf report K-1 --role security-auditor"), "{}", o.all());
    // The report, from the checkpoint's copy.
    let o = fx.bf_in(
        &k,
        &[
            "report",
            "K-1",
            "--result",
            "built",
            "--role",
            "security-auditor",
            "--token",
            &tok,
            "--json",
        ],
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The token is spent by the report.
    let o = fx.bf_in(
        &k,
        &[
            "report",
            "K-1",
            "--result",
            "built",
            "--role",
            "security-auditor",
            "--token",
            &tok,
        ],
    );
    assert!(o.code != 0);
    // The legal auditor's legal record from the checkpoint's copy is refused too, with any token.
    let legal = "kind=\"legal\"\ntitle=\"Cookie notice\"\n";
    let lt = token(&fx, "K-1", "legal-auditor");
    let wrong = token(&fx, "K-1", "uiux-verifier");
    assert!(put_in(&fx, &k, "legal", legal, &wrong).code != 0);
    let o = put_in(&fx, &k, "legal", legal, &lt);
    assert!(o.code != 0, "{}", o.all());
    assert!(o.all().contains("--role legal-auditor"), "{}", o.all());
    // Nothing else is put from a checkpoint's copy.
    let o = put_in(
        &fx,
        &k,
        "requirement",
        "kind=\"requirement\"\ntitle=\"x\"\n",
        &tok,
    );
    assert!(o.code != 0);
}
