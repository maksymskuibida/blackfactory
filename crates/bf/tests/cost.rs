//! The cost a role report carries counts in the spend.

mod common;
use common::Fx;

#[test]
fn the_cost_of_a_role_report_counts_in_the_spend() {
    let fx = Fx::tiny_run();
    fx.must(&["unit", "start", "U-1"]);
    let o = fx.bf(&[
        "report",
        "U-1",
        "--result",
        "built",
        "--role",
        "security-auditor",
        "--cost",
        "7.5",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let spent = fx.bf(&["status", "--json"]).json()["spent"]
        .as_f64()
        .unwrap();
    assert_eq!(spent, 7.5);
    let o = fx.bf(&["report", "U-1", "--result", "built", "--cost", "2.5"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let spent = fx.bf(&["status", "--json"]).json()["spent"]
        .as_f64()
        .unwrap();
    assert_eq!(spent, 10.0);
}

#[test]
fn the_cost_of_a_role_report_on_the_design_counts_in_the_total() {
    let fx = Fx::new();
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/a.txt exists\"]\n"),
        ("screen", "id=\"S-1\"\nkind=\"screen\"\ntitle=\"The greeting page\"\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let o = fx.bf(&[
        "report",
        "design",
        "--result",
        "built",
        "--role",
        "security-expert",
        "--cost",
        "3.25",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let c = fx.must(&["cost", "status", "--json"]).json();
    assert_eq!(c["spent"].as_f64().unwrap(), 3.25, "{c}");
}

#[test]
fn a_role_report_cost_that_reaches_the_cap_stops_new_work() {
    let fx = Fx::tiny_project();
    fx.set_policy("cap = 0.0", "cap = 2.0");
    fx.start_test_run();
    fx.must(&["unit", "start", "U-1"]);
    let o = fx.bf(&[
        "report",
        "U-1",
        "--result",
        "built",
        "--role",
        "security-auditor",
        "--cost",
        "2.0",
    ]);
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["next"]);
    assert!(o.stderr.contains("budget cap"), "{}", o.stderr);
    fx.refused(&["unit", "start", "U-2"]);
}
