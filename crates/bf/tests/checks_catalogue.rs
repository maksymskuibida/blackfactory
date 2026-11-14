//! U-1: the check catalogue, the new record kinds and fields, and the wired command groups.

use bf::checks::{self, Family, Stage};
use bf::schema::{kind_by_name, validate};
use serde_json::json;

#[test]
fn every_named_check_of_three_families_is_required_by_default() {
    let names = [
        "design-uiux-verification",
        "built-uiux-verification",
        "accessibility",
        "threat-model",
        "security-audit",
        "secret-scan",
        "dependency-licence-check",
        "markets-statement",
        "legal-audit",
    ];
    for n in names {
        let c = checks::by_name(n).unwrap_or_else(|| panic!("missing {n}"));
        assert!(c.required, "{n}");
    }
    assert_eq!(checks::required_by_default().len(), checks::all().len());
    for f in [Family::Uiux, Family::Security, Family::Legal] {
        assert!(!checks::in_family(f).is_empty());
    }
    assert_eq!(
        checks::by_name("threat-model").unwrap().stage,
        Stage::Design
    );
    assert_eq!(checks::by_name("secret-scan").unwrap().stage, Stage::Built);
}

#[test]
fn new_kinds_and_fields_validate() {
    let ok = |k: &str, v: serde_json::Value| {
        let p = validate(kind_by_name(k).unwrap(), &v);
        assert!(p.is_empty(), "{k}: {p:?}");
    };
    let bad = |k: &str, v: serde_json::Value| {
        assert!(!validate(kind_by_name(k).unwrap(), &v).is_empty(), "{k}");
    };
    ok(
        "markets",
        json!({"id":"M-1","kind":"markets","title":"t","markets":["DE","US"]}),
    );
    bad("markets", json!({"id":"M-1","kind":"markets","title":"t"}));
    ok(
        "legal",
        json!({"id":"L-1","kind":"legal","title":"t","severity":"high","outcome":"open"}),
    );
    bad(
        "legal",
        json!({"id":"L-1","kind":"legal","title":"t","unit":"U-1"}),
    );
    ok(
        "screen",
        json!({"id":"S-1","kind":"screen","title":"t","interface":"web"}),
    );
    bad(
        "screen",
        json!({"id":"S-1","kind":"screen","title":"t","interface":"fax"}),
    );
    ok(
        "journey",
        json!({"id":"J-1","kind":"journey","title":"t","steps":["a","b"]}),
    );
    bad(
        "journey",
        json!({"id":"J-1","kind":"journey","title":"t","steps":"a"}),
    );
    ok(
        "decision",
        json!({"id":"D-1","kind":"decision","title":"t","maker":"agent","stage":"plan"}),
    );
    bad(
        "decision",
        json!({"id":"D-1","kind":"decision","title":"t","maker":"robot"}),
    );
}

#[test]
fn each_new_command_group_has_its_own_entry_point() {
    assert!(checks::run("list", &[]).is_ok());
    assert!(bf::scan::run("x", &[]).is_err());
    assert!(bf::legal::run("x", &[]).is_err());
    assert!(bf::preset::run("x", &[]).is_err());
    assert!(bf::uiux::run("x", &[]).is_err());
    assert!(bf::roles::run("x", &[]).is_err());
}
