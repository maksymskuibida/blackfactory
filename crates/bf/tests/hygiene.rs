//! T-6 (every refusal has four parts), T-8 (no network code) and a few input-handling checks.

mod common;
use common::*;
use std::fs;
use std::path::Path;

#[test]
fn t6_every_kind_of_refusal_has_the_four_parts() {
    let fx = Fx::new();
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.toml");
    fs::write(
        &bad,
        "id = \"R-1\"\nkind = \"requirement\"\ntitle = \"\"\nfoo = 1\n",
    )
    .unwrap();
    let cases: Vec<Vec<&str>> = vec![
        vec![
            "record",
            "put",
            "requirement",
            "--file",
            bad.to_str().unwrap(),
        ],
        vec!["record", "put", "nonsense", "--file", bad.to_str().unwrap()],
        vec![
            "record",
            "put",
            "requirement",
            "--file",
            "/nonexistent/file.toml",
        ],
        vec!["record", "put", "requirement"],
        vec!["record", "get", "R-99"],
        vec!["record", "get", "xyz"],
        vec!["record", "list", "--kind", "nonsense"],
        vec!["id", "new", "nonsense"],
        vec!["id", "sideways"],
        vec!["next"],
        vec!["unit", "start", "U-1"],
        vec!["unit", "land", "U-1"],
        vec!["report", "U-1", "--result", "built"],
        vec!["report", "U-1"],
        vec!["brief", "U-1", "--role", "boss"],
        vec!["gate", "open", "bogus"],
        vec!["gate", "open", "checkpoint-step"],
        vec!["gate", "approve", "nothing"],
        vec!["gate", "reject", "nothing"],
        vec!["gate", "sideways"],
        vec!["test-mode", "grant"],
        vec!["evidence", "run", "X-1", "--", "true"],
        vec!["evidence", "run", "X-1"],
        vec!["evidence", "attach", "X-1", "/nonexistent"],
        vec!["checkpoint", "close", "K-9"],
        vec!["amend", "open"],
        vec!["amend", "approve"],
        vec!["run", "stop"],
        vec!["view", "nonsense"],
        vec![
            "render",
            "plan",
            "--template",
            "/nonexistent.html",
            "--out",
            "/tmp/x.html",
        ],
        vec!["ledger", "impact"],
        vec!["bogus-command"],
    ];
    for c in &cases {
        let o = fx.bf(c);
        assert_eq!(o.code, 2, "{c:?} should be refused (exit 2):\n{}", o.all());
        assert_four_parts(&o.stderr);
    }
    // Refusals that need a little state.
    fx.put("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"x\"\ndelivers=[\"R-5\"]\nfiles=[\"a\"]\ncheckpoint=\"K-1\"\n");
    fx.refused(&["plan", "check"]);
    fx.refused(&["run", "start"]);
    // Outside any project.
    let o = fx.bf_in(dir.path(), &["status"]);
    assert_eq!(o.code, 2);
    assert_four_parts(&o.stderr);
    let o = fx.bf_in(dir.path(), &["init"]);
    assert_eq!(o.code, 2);
    assert_four_parts(&o.stderr);
    // The same refusal as data.
    let j = fx.bf(&["record", "get", "R-99", "--json"]);
    assert_eq!(j.code, 2);
    let v = j.json();
    for k in ["refused", "because", "do_this", "it_worked_when"] {
        assert!(!v[k].is_null(), "{k}");
    }
    assert!(!v["do_this"].as_array().unwrap().is_empty());
}

#[test]
fn t8_the_engine_has_no_network_code() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let banned = [
        "std::net",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "hyper",
        "ureq",
        "tokio",
        "async fn",
        "http://",
        "https://",
    ];
    for e in fs::read_dir(&src).unwrap().flatten() {
        let t = fs::read_to_string(e.path()).unwrap();
        for b in banned {
            // The test module of this very check and doc comments are allowed to mention nothing: scan code only.
            let hits: Vec<&str> = t
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .filter(|l| l.contains(b))
                .collect();
            assert!(
                hits.is_empty(),
                "{} contains `{b}`: {hits:?}",
                e.path().display()
            );
        }
    }
    let lock =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")).unwrap();
    for c in [
        "reqwest",
        "hyper",
        "ureq",
        "tokio",
        "curl",
        "isahc",
        "openssl",
        "rustls",
        "async-std",
        "mio",
    ] {
        assert!(
            !lock.contains(&format!("name = \"{c}\"")),
            "dependency {c} found"
        );
    }
}

#[test]
fn init_is_idempotent_and_ignores_the_derived_folder() {
    let fx = Fx::new();
    let gi = fs::read_to_string(fx.root.join(".gitignore")).unwrap();
    assert_eq!(gi.lines().filter(|l| l.trim() == ".bf/").count(), 1);
    let o = fx.must(&["init"]);
    assert!(o.stdout.contains("already initialised"));
    let gi = fs::read_to_string(fx.root.join(".gitignore")).unwrap();
    assert_eq!(gi.lines().filter(|l| l.trim() == ".bf/").count(), 1);
    assert!(fx.root.join(".bf/guard.json").exists() && fx.state_dir().join("key").exists());
    assert!(
        !fx.root.join(".bf/key").exists(),
        "the key lives outside the project"
    );
    // The private key and derived files never show up in git status.
    let st = git(&fx.root, &["status", "--porcelain"]);
    assert!(!st.contains(".bf"), "{st}");
}

#[test]
fn records_round_trip_and_id_new_counts_up() {
    let fx = Fx::new();
    assert_eq!(fx.must(&["id", "new", "requirement"]).stdout.trim(), "R-1");
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"One\"\n",
    );
    fx.put(
        "requirement",
        "id=\"R-7\"\nkind=\"requirement\"\ntitle=\"Seven\"\n",
    );
    assert_eq!(fx.must(&["id", "new", "requirement"]).stdout.trim(), "R-8");
    assert_eq!(fx.must(&["id", "new", "threat"]).stdout.trim(), "TH-1");
    // `id new` reserved R-8, so a record without an id gets the next number after it, and says so.
    let o = fx.put("requirement", "kind=\"requirement\"\ntitle=\"Nine\"\n");
    assert!(o.stdout.contains("assigned R-9"), "{}", o.all());
    let g = fx.must(&["record", "get", "R-7", "--json"]).json();
    assert_eq!(g["title"], "Seven");
    let l = fx
        .must(&["record", "list", "--kind", "requirement", "--json"])
        .json();
    assert_eq!(l.as_array().unwrap().len(), 3);
}

#[test]
fn approved_records_change_only_by_amendment() {
    let fx = Fx::new();
    fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"One\"\n",
    );
    fx.must(&["gate", "open", "requirements"]);
    fx.human("approved");
    fx.must(&["gate", "approve", "requirements", "--quote", "approved"]);
    let o = fx.put(
        "requirement",
        "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"One, changed\"\n",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert_four_parts(&o.stderr);
    assert!(o.stderr.contains("amendment"));
    // The same content is not a change.
    assert_eq!(
        fx.put(
            "requirement",
            "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"One\"\n"
        )
        .code,
        0
    );
}
