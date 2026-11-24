//! The five new roles: names and limits, briefs written by the engine, tokens.

mod common;
use common::*;
use std::fs;

const NAMES: [&str; 5] = [
    "uiux-verifier",
    "security-expert",
    "security-auditor",
    "lawyer",
    "legal-auditor",
];

#[test]
fn the_roles_are_listed_with_their_limits() {
    let fx = Fx::new();
    let v = fx.must(&["roles", "list", "--json"]).json();
    let roles = v["roles"].as_array().unwrap();
    // The five roles of this file, the design reviewer, the UI/UX designer and the designer.
    assert_eq!(roles.len(), 8);
    for n in ["design-reviewer", "uiux-designer"] {
        assert!(roles.iter().any(|r| r["name"] == n), "{n}");
    }
    for n in NAMES {
        let r = roles
            .iter()
            .find(|r| r["name"] == n)
            .unwrap_or_else(|| panic!("{n}"));
        for k in ["reads", "writes", "runs", "stage", "tier"] {
            assert!(!r[k].is_null(), "{n} lacks {k}");
        }
        assert_eq!(r["may_write_code"], false);
        assert!(!r["runs"].as_array().unwrap().is_empty());
    }
    let one = fx
        .must(&["roles", "show", "security-expert", "--json"])
        .json();
    assert_eq!(one["stage"], "design");
    let a = fx
        .must(&["roles", "show", "security-auditor", "--json"])
        .json();
    assert_eq!(a["stage"], "built");
    assert!(fx.bf(&["roles", "show", "nobody"]).code != 0);
    assert!(fx.bf(&["roles", "bogus"]).code != 0);
}

#[test]
fn the_engine_writes_a_brief_for_each_role() {
    let fx = Fx::tiny_project();
    for n in NAMES {
        let subject = if matches!(n, "security-expert" | "lawyer") {
            "design"
        } else {
            "U-1"
        };
        let v = fx.must(&["brief", subject, "--role", n, "--json"]).json();
        let path = v["path"].as_str().unwrap();
        assert!(path.ends_with(&format!("{subject}-{n}.md")), "{path}");
        let md = fs::read_to_string(path).unwrap();
        assert!(md.contains(&format!("# Brief: {subject} as {n}")));
        assert!(md.contains("R-1"));
        assert!(md.contains("## You may"));
        assert!(md.contains("## Limits"));
    }
    let md = fs::read_to_string(
        fx.must(&["brief", "U-1", "--role", "uiux-verifier", "--json"])
            .json()["path"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(md.contains("blind"));
    let o = fx.bf(&["brief", "U-1", "--role", "janitor"]);
    assert!(o.code != 0);
}

#[test]
fn each_role_gets_its_own_stable_token() {
    let fx = Fx::tiny_project();
    let t1 = fx
        .must(&["roles", "token", "U-1", "security-auditor", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let t1b = fx
        .must(&["roles", "token", "U-1", "security-auditor", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let t2 = fx
        .must(&["roles", "token", "U-1", "legal-auditor", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(t1, t1b);
    assert_ne!(t1, t2);
    assert_eq!(t1.len(), 24);
    assert!(fx.bf(&["roles", "token", "U-9", "legal-auditor"]).code != 0);
    assert!(fx.bf(&["roles", "token", "U-1", "builder"]).code != 0);
}

#[test]
fn an_issued_role_token_survives_bf_next_and_reports_once() {
    let fx = Fx::tiny_project();
    fx.set_policy("tokens = false", "tokens = true");
    fx.start_test_run();
    let t = fx
        .must(&["roles", "token", "design", "security-expert", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    fx.must(&["next", "--json"]);
    fx.must(&["next", "--json"]);
    let again = fx
        .must(&["roles", "token", "design", "security-expert", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(t, again, "the token survives `bf next`");
    // A wrong token is refused; the right one reports and is spent; the unit does not move.
    let before = fx.must(&["status", "--json"]).json();
    let bad = fx.bf(&[
        "report",
        "design",
        "--result",
        "built",
        "--role",
        "security-expert",
        "--token",
        "nope",
    ]);
    assert!(bad.code != 0);
    let ok = fx.must(&[
        "report",
        "design",
        "--result",
        "built",
        "--role",
        "security-expert",
        "--token",
        &t,
        "--json",
    ]);
    assert_eq!(ok.json()["role"], "security-expert");
    let after = fx.must(&["status", "--json"]).json();
    assert_eq!(
        before["units"], after["units"],
        "the unit's state did not move"
    );
    let twice = fx.bf(&[
        "report",
        "design",
        "--result",
        "built",
        "--role",
        "security-expert",
        "--token",
        &t,
    ]);
    assert!(twice.code != 0, "the token is spent");
}

#[test]
fn the_uiux_verifier_is_staged_at_built() {
    let fx = Fx::new();
    let v = fx
        .must(&["roles", "show", "uiux-verifier", "--json"])
        .json();
    assert_eq!(v["stage"], "built");
}

fn wc_of_u1(fx: &Fx) -> std::path::PathBuf {
    fx.tiny();
    fx.start_test_run();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    std::path::PathBuf::from(o.json()["working_copy"].as_str().unwrap())
}

fn role_token(fx: &Fx, role: &str) -> String {
    fx.must(&["roles", "token", "U-1", role, "--json"]).json()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn put_in(fx: &Fx, wc: &std::path::Path, kind: &str, body: &str, token: Option<&str>) -> Out {
    let f = wc.join("rec.toml");
    fs::write(&f, body).unwrap();
    let mut args = vec!["record", "put", kind, "--file", "rec.toml"];
    if let Some(t) = token {
        args.push("--token");
        args.push(t);
    }
    fx.bf_in(wc, &args)
}

const THREAT: &str = "id=\"TH-1\"\nkind=\"threat\"\ntitle=\"Token theft\"\nprotects=\"the tokens\"\nattack=\"read the state folder\"\nmitigation=\"the folder is not readable\"\n";
const LEGAL: &str = "id=\"L-1\"\nkind=\"legal\"\ntitle=\"Consent\"\nseverity=\"high\"\n";

#[test]
fn a_threat_put_from_a_working_copy_is_refused_for_every_role() {
    let fx = Fx::new();
    let wc = wc_of_u1(&fx);
    let none = put_in(&fx, &wc, "threat", THREAT, None);
    assert!(
        none.code != 0 && none.all().contains("token of a role that writes it"),
        "{}",
        none.all()
    );
    // The only writer of a threat is the security expert, who works at the design stage from the
    // main checkout: no role can put a threat from a working copy.
    for role in ["security-auditor", "legal-auditor", "uiux-verifier"] {
        let t = role_token(&fx, role);
        let o = put_in(&fx, &wc, "threat", THREAT, Some(&t));
        assert!(
            o.code != 0 && o.all().contains("token of a role that writes it"),
            "{role}: {}",
            o.all()
        );
    }
    let wrong = put_in(&fx, &wc, "threat", THREAT, Some("0123456789abcdef01234567"));
    assert!(wrong.code != 0 && wrong.all().contains("token of a role that writes it"));
    // The security expert works at the design stage: no token for it exists on a unit.
    let o = fx.bf(&["roles", "token", "U-1", "security-expert"]);
    assert!(o.code != 0, "{}", o.all());
}

#[test]
fn a_legal_put_needs_the_lawyers_or_the_legal_auditors_token() {
    let fx = Fx::new();
    let wc = wc_of_u1(&fx);
    let none = put_in(&fx, &wc, "legal", LEGAL, None);
    assert!(
        none.code != 0 && none.all().contains("token of a role that writes it"),
        "{}",
        none.all()
    );
    for role in ["security-auditor", "uiux-verifier"] {
        let t = role_token(&fx, role);
        let o = put_in(&fx, &wc, "legal", LEGAL, Some(&t));
        assert!(
            o.code != 0 && o.all().contains("token of a role that writes it"),
            "{role}: {}",
            o.all()
        );
    }
    let t = role_token(&fx, "legal-auditor");
    let ok = put_in(&fx, &wc, "legal", LEGAL, Some(&t));
    assert_eq!(ok.code, 0, "legal-auditor: {}", ok.all());
    // The lawyer works at the design stage: no token for it exists on a unit.
    assert!(fx.bf(&["roles", "token", "U-1", "lawyer"]).code != 0);
}

/// The token of a builder stays the same while its unit waits for the human's answer.
#[test]
fn a_builder_keeps_its_token_while_its_question_is_open() {
    use std::path::PathBuf;
    let fx = Fx::tiny_project();
    fx.set_policy("tokens = false", "tokens = true");
    fx.start_test_run();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    let before = fx.take_token("U-1", "builder");
    fx.must_in(
        &wc,
        &[
            "ask",
            "--title",
            "Which greeting?",
            "--context",
            "The brief does not say.",
        ],
    );
    // The conductor looks at the next step while the human has not answered yet.
    fx.must(&["next", "--json"]);
    fx.human("Say hello.");
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.json");
    fs::write(&a, "{\"answer\": \"hello\"}").unwrap();
    fx.must(&["answer", "Q-1", "--file", a.to_str().unwrap()]);
    assert_eq!(fx.take_token("U-1", "builder"), before);
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/hello.txt"), "hello\n").unwrap();
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
            "src/hello.txt",
        ],
    );
    let o = fx.bf_in(
        &wc,
        &["report", "U-1", "--result", "built", "--token", &before],
    );
    assert_eq!(o.code, 0, "{}", o.all());
}
