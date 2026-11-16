//! U-23: the design-stage roles work at the design stage (C-8, R-5, R-12). A design-stage role has a
//! brief and a token before any plan exists, reports outside a run, settles design-stage checks with
//! its token, and a token ends with the run it was issued in.

mod common;
use common::Fx;
use std::fs;

const SCREEN: &str = "id=\"S-1\"\nkind=\"screen\"\ntitle=\"The greeting page\"\n";

fn design_records(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("screen", SCREEN),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Messages\"\n"),
        ("decision", "id=\"D-1\"\nkind=\"decision\"\ntitle=\"One module\"\nchosen=\"a\"\nrejected=\"b\"\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
}

fn token(fx: &Fx, subject: &str, role: &str) -> String {
    fx.must(&["roles", "token", subject, role, "--json"]).json()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn report(fx: &Fx, subject: &str, role: &str, tok: &str) -> common::Out {
    fx.bf(&[
        "report", subject, "--result", "built", "--role", role, "--token", tok, "--json",
    ])
}

#[test]
fn a_design_role_has_a_brief_and_a_token_before_any_plan_exists() {
    let fx = Fx::new();
    design_records(&fx);
    for role in ["security-expert", "lawyer", "uiux-verifier"] {
        let v = fx
            .must(&["brief", "design", "--role", role, "--json"])
            .json();
        let path = v["path"].as_str().unwrap();
        assert!(path.ends_with(&format!("design-{role}.md")), "{path}");
        let md = fs::read_to_string(path).unwrap();
        assert!(md.contains(&format!("# Brief: design as {role}")));
        assert!(md.contains("R-1") && md.contains("S-1"), "{md}");
        let t = token(&fx, "design", role);
        assert_eq!(t.len(), 24);
        assert_eq!(t, token(&fx, "design", role), "the token is stable");
    }
}

#[test]
fn a_design_role_reports_outside_a_run_and_the_token_is_spent() {
    let fx = Fx::new();
    fx.set_policy("tokens = false", "tokens = true");
    design_records(&fx);
    let t = token(&fx, "design", "security-expert");
    assert!(report(&fx, "design", "security-expert", "nope").code != 0);
    let ok = report(&fx, "design", "security-expert", &t);
    assert_eq!(ok.code, 0, "{}", ok.all());
    assert_eq!(ok.json()["role"], "security-expert");
    assert!(
        report(&fx, "design", "security-expert", &t).code != 0,
        "spent"
    );
}

#[test]
fn a_role_works_only_at_its_stage() {
    let fx = Fx::new();
    fx.tiny();
    // Built-stage roles have no design-stage subject (the UI/UX verifier works at both).
    for role in ["security-auditor", "legal-auditor"] {
        assert!(
            fx.bf(&["roles", "token", "design", role]).code != 0,
            "{role}"
        );
        assert!(
            fx.bf(&["brief", "design", "--role", role]).code != 0,
            "{role}"
        );
    }
    assert_eq!(
        fx.bf(&["roles", "token", "design", "uiux-verifier"]).code,
        0
    );
    // Design-stage roles have no unit.
    for role in ["security-expert", "lawyer"] {
        assert!(fx.bf(&["roles", "token", "U-1", role]).code != 0, "{role}");
        assert!(fx.bf(&["brief", "U-1", "--role", role]).code != 0, "{role}");
    }
    assert!(fx.bf(&["roles", "token", "nowhere", "lawyer"]).code != 0);
    let o = fx.bf(&["roles", "token", "U-1", "security-auditor"]);
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn a_token_ends_with_the_run_it_was_issued_in() {
    let fx = Fx::new();
    fx.set_policy("tokens = false", "tokens = true");
    fx.tiny();
    let before_run = token(&fx, "design", "lawyer");
    fx.start_test_run();
    let r = report(&fx, "design", "lawyer", &before_run);
    assert!(
        r.code != 0,
        "the token of the design stage ends when a run starts"
    );
    // A token issued inside the run works there, and is a new one.
    let t = token(&fx, "design", "lawyer");
    assert_ne!(t, before_run);
    let ok = report(&fx, "design", "lawyer", &t);
    assert_eq!(ok.code, 0, "{}", ok.all());
    let a = token(&fx, "U-1", "security-auditor");
    fx.must(&["run", "stop"]);
    let r = report(&fx, "U-1", "security-auditor", &a);
    assert!(r.code != 0, "{}", r.all());
    let state = fs::read_to_string(fx.state_dir().join("tokens.json")).unwrap();
    assert!(
        !state.contains(&a),
        "the token is gone from the state folder"
    );
}

fn ui_check(fx: &Fx) {
    let o = fx.put("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"The screens were judged\"\nhow=\"agent\"\nproves=[\"S-1\"]\nscope=\"checkpoint\"\nnamed=\"design-uiux-verification\"\n");
    assert_eq!(o.code, 0, "{}", o.all());
}

fn attach(fx: &Fx, tok: Option<&str>) -> common::Out {
    let f = fx.root.join("shot.txt");
    fs::write(&f, "the screen at 375px\n").unwrap();
    // The screen state and the accessibility result need their own evidence before the whole
    // verification can be met.
    let mut last = None;
    for says in [
        "S-1 state default: the greeting page at 375px",
        "accessibility: every label read against the standard",
        "I opened the page at 375px and read every label",
    ] {
        let mut a = vec![
            "evidence",
            "attach",
            "X-1",
            f.to_str().unwrap(),
            "--outcome",
            "met",
            "--says",
            says,
        ];
        if let Some(t) = tok {
            a.push("--token");
            a.push(t);
        }
        let o = fx.bf(&a);
        if o.code != 0 {
            return o;
        }
        last = Some(o);
    }
    last.unwrap()
}

#[test]
fn an_outcome_on_a_design_stage_check_needs_the_token_of_a_role_of_its_family() {
    let fx = Fx::with_checks();
    fx.set_policy("tokens = false", "tokens = true");
    design_records(&fx);
    ui_check(&fx);
    let o = attach(&fx, None);
    assert!(o.code != 0 && o.all().contains("token"), "{}", o.all());
    assert!(attach(&fx, Some("0123456789abcdef01234567")).code != 0);
    // A role of another family does not settle a UI/UX check.
    let lawyer = token(&fx, "design", "lawyer");
    assert!(attach(&fx, Some(&lawyer)).code != 0);
    fx.refused(&["gate", "open", "ui"]);
    let t = token(&fx, "design", "uiux-verifier");
    let o = attach(&fx, Some(&t));
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["gate", "open", "ui"]);
}

#[test]
fn the_design_stage_receipt_is_tied_to_the_screens_it_judged() {
    let fx = Fx::with_checks();
    design_records(&fx);
    ui_check(&fx);
    let o = attach(&fx, None);
    assert_eq!(o.code, 0, "{}", o.all());
    // The screens change after they were judged: the receipt no longer counts.
    let f = fx.root.join("revise.toml");
    fs::write(
        &f,
        "id=\"S-1\"\nkind=\"screen\"\ntitle=\"The greeting page, redesigned\"\n",
    )
    .unwrap();
    fx.must(&[
        "record",
        "put",
        "screen",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    let o = fx.refused(&["gate", "open", "ui"]);
    assert!(
        o.stderr.contains("design-stage UI/UX verification"),
        "{}",
        o.stderr
    );
    // Judged again on the new screens, it counts.
    let o = attach(&fx, None);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.must(&["gate", "open", "ui"]);
}

#[test]
fn the_markets_statement_belongs_to_the_architecture_stage() {
    let fx = Fx::with_checks();
    design_records(&fx);
    fx.put(
        "threat",
        "id=\"TH-1\"\nkind=\"threat\"\ntitle=\"A visitor changes a text\"\nprotects=\"The texts\"\nattack=\"A visitor edits a file by hand\"\nmitigation=\"The texts are read-only files\"\n",
    );
    let o = fx.put(
        "markets",
        "id=\"M-1\"\nkind=\"markets\"\ntitle=\"Served markets\"\nmarkets=[\"Norway\"]\n",
    );
    assert_eq!(o.code, 0, "{}", o.all());
    // The threat model stands once the security expert has reported after the last threat change.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let o = fx.bf(&[
        "report",
        "design",
        "--result",
        "built",
        "--role",
        "security-expert",
        "--json",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    std::thread::sleep(std::time::Duration::from_millis(50));
    fx.must(&["gate", "open", "architecture"]);
    let snap = bf::gates::snapshot(&fx.project(), "architecture", None).unwrap();
    assert!(snap.get("M-1").is_some(), "{snap}");
    let page = fx
        .must(&["view", "gate", "architecture"])
        .json()
        .to_string();
    assert!(
        page.contains("M-1") && page.contains("Served markets"),
        "the architecture gate page shows the markets statement: {page}"
    );
    assert_eq!(
        bf::gates::stage_of_kind("markets"),
        Some("architecture"),
        "the markets statement is frozen with the architecture gate"
    );
}
