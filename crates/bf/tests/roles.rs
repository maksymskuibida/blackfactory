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
    assert_eq!(roles.len(), 5);
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
    let fx = Fx::new();
    fx.tiny();
    for n in NAMES {
        let v = fx.must(&["brief", "U-1", "--role", n, "--json"]).json();
        let path = v["path"].as_str().unwrap();
        assert!(path.ends_with(&format!("U-1-{n}.md")), "{path}");
        let md = fs::read_to_string(path).unwrap();
        assert!(md.contains(&format!("# Brief: U-1 as {n}")));
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
    let fx = Fx::new();
    fx.tiny();
    let t1 = fx
        .must(&["roles", "token", "U-1", "lawyer", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let t1b = fx
        .must(&["roles", "token", "U-1", "lawyer", "--json"])
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
    assert!(fx.bf(&["roles", "token", "U-9", "lawyer"]).code != 0);
    assert!(fx.bf(&["roles", "token", "U-1", "builder"]).code != 0);
}

#[test]
fn an_issued_role_token_survives_bf_next_and_reports_once() {
    let fx = Fx::new();
    fx.set_policy("tokens = false", "tokens = true");
    fx.tiny();
    fx.start_test_run();
    let t = fx
        .must(&["roles", "token", "U-1", "security-expert", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    fx.must(&["next", "--json"]);
    fx.must(&["next", "--json"]);
    let again = fx
        .must(&["roles", "token", "U-1", "security-expert", "--json"])
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(t, again, "the token survives `bf next`");
    // A wrong token is refused; the right one reports and is spent; the unit does not move.
    let before = fx.must(&["status", "--json"]).json();
    let bad = fx.bf(&[
        "report",
        "U-1",
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
        "U-1",
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
        "U-1",
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
fn a_threat_put_needs_the_security_experts_token() {
    let fx = Fx::new();
    let wc = wc_of_u1(&fx);
    let none = put_in(&fx, &wc, "threat", THREAT, None);
    assert!(
        none.code != 0 && none.all().contains("token of a role that writes it"),
        "{}",
        none.all()
    );
    for role in ["security-auditor", "lawyer", "legal-auditor"] {
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
    let t = role_token(&fx, "security-expert");
    let ok = put_in(&fx, &wc, "threat", THREAT, Some(&t));
    assert_eq!(ok.code, 0, "{}", ok.all());
    assert!(
        !ok.all().contains("token of a role that writes it"),
        "{}",
        ok.all()
    );
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
    for role in ["security-expert", "security-auditor", "uiux-verifier"] {
        let t = role_token(&fx, role);
        let o = put_in(&fx, &wc, "legal", LEGAL, Some(&t));
        assert!(
            o.code != 0 && o.all().contains("token of a role that writes it"),
            "{role}: {}",
            o.all()
        );
    }
    for (role, id) in [("lawyer", "L-1"), ("legal-auditor", "L-2")] {
        let t = role_token(&fx, role);
        let body = LEGAL.replace("L-1", id);
        let ok = put_in(&fx, &wc, "legal", &body, Some(&t));
        assert_eq!(ok.code, 0, "{role}: {}", ok.all());
    }
}
