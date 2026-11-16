//! U-33: the next step follows the check settings. It does not ask for a check the human switched
//! off, and it offers a close only when the engine would accept it.

mod common;
use common::Fx;

fn two_units_landed(fx: &Fx) {
    fx.tiny();
    fx.name_every_check("R-1");
    fx.design_reviews_off();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
}

fn asked(n: &serde_json::Value) -> Vec<String> {
    n["directives"][0]["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("no checks asked for: {n}"))
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_next_step_does_not_ask_for_a_check_the_human_switched_off() {
    let fx = Fx::with_checks();
    two_units_landed(&fx);
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "run-checkpoint-checks", "{n}");
    // Every named check is on: the verifier is asked for the accessibility check too (X-92).
    assert!(asked(&n).contains(&"X-92".to_string()), "{n}");
    fx.run_checks_all_off();
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "run-checkpoint-checks", "{n}");
    assert_eq!(asked(&n), vec!["X-3".to_string()], "{n}");
}

#[test]
fn the_next_step_offers_the_close_only_when_the_engine_would_accept_it() {
    let fx = Fx::with_checks();
    two_units_landed(&fx);
    fx.run_checks_all_off();
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
    let n = fx.must(&["next", "--json"]).json();
    assert_eq!(n["directives"][0]["action"], "close-checkpoint", "{n}");
    // What was offered is accepted.
    fx.must(&["checkpoint", "close", "K-1"]);
}

#[test]
fn an_on_check_without_a_receipt_still_holds_a_requirement_the_off_checks_also_prove() {
    let fx = Fx::with_checks();
    two_units_landed(&fx);
    // Everything named is off, X-3 is on and has no receipt: R-1 is also proven by the off checks,
    // yet the close is held by X-3.
    fx.run_checks_all_off();
    let o = fx.refused(&["checkpoint", "close", "K-1"]);
    assert!(o.stderr.contains("X-3"), "{}", o.stderr);
}

#[test]
fn a_requirement_proven_only_by_switched_off_checks_does_not_hold_the_close() {
    let fx = Fx::with_checks();
    fx.tiny();
    // X-3 no longer proves R-1: only the named checks (all off) do.
    let f = fx.root.join("x3.toml");
    std::fs::write(&f, "id=\"X-3\"\nkind=\"check\"\ntitle=\"Both files\"\nhow=\"command\"\ncommand=\"ls src/hello.txt src/bye.txt\"\nproves=[\"R-2\",\"C-1\"]\nscope=\"checkpoint\"\n").unwrap();
    let o = fx.bf(&[
        "record",
        "put",
        "check",
        "--file",
        f.to_str().unwrap(),
        "--replace",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    fx.name_every_check("R-1");
    fx.design_reviews_off();
    fx.start_test_run();
    fx.build_unit(
        "U-1",
        "src/hello.txt",
        "X-1",
        &["test", "-f", "src/hello.txt"],
    );
    fx.build_unit("U-2", "src/bye.txt", "X-2", &["test", "-f", "src/bye.txt"]);
    fx.run_checks_all_off();
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
}
