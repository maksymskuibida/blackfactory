//! U-95: two units of one checkpoint that no `depends_on` path orders share no files, whatever their
//! wave numbers; units of different checkpoints are never compared (FU-40, D-112).

mod common;
use common::*;

fn unit(id: &str, file: &str, deps: &[&str], cp: &str) -> String {
    let deps: Vec<String> = deps.iter().map(|d| format!("\"{d}\"")).collect();
    format!(
        "id=\"{id}\"\nkind=\"unit\"\ntitle=\"extra\"\ndelivers=[\"R-1\"]\nfiles=[\"{file}\"]\ndepends_on=[{}]\ncheckpoint=\"{cp}\"\nestimate=1\n",
        deps.join(",")
    )
}

fn clash(fx: &Fx, a: &str, b: &str) -> bool {
    let o = fx.bf(&["plan", "check"]);
    let all = o.all();
    all.contains("can run together") && all.contains(a) && all.contains(b) && all.contains("both touch")
}

fn any_clash(fx: &Fx) -> bool {
    fx.bf(&["plan", "check"]).all().contains("both touch")
}

#[test]
fn one_checkpoint_different_waves_no_path_is_a_problem() {
    let fx = Fx::new();
    fx.tiny();
    // U-2 is wave 2 (bye.txt); U-5 is wave 1 and shares bye.txt, with no path to U-2.
    fx.put("unit", &unit("U-5", "src/bye.txt", &[], "K-1"));
    assert!(clash(&fx, "U-2", "U-5"), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn a_direct_path_orders_the_pair() {
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-5", "src/bye.txt", &["U-2"], "K-1"));
    assert!(!any_clash(&fx), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn a_path_through_a_third_unit_orders_the_pair() {
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-3", "src/mid.txt", &["U-2"], "K-1"));
    fx.put("unit", &unit("U-5", "src/bye.txt", &["U-3"], "K-1"));
    assert!(!any_clash(&fx), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn units_of_different_checkpoints_are_not_compared() {
    let fx = Fx::new();
    fx.tiny();
    // Same wave as U-1, same file, other checkpoint.
    fx.put("unit", &unit("U-5", "src/hello.txt", &[], "K-2"));
    assert!(!any_clash(&fx), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn one_checkpoint_one_wave_is_still_a_problem() {
    let fx = Fx::new();
    fx.tiny();
    fx.put("unit", &unit("U-5", "src/hello.txt", &[], "K-1"));
    assert!(clash(&fx, "U-1", "U-5"), "{}", fx.bf(&["plan", "check"]).all());
}

#[test]
fn the_waves_field_is_unchanged() {
    let fx = Fx::new();
    fx.tiny();
    let j = fx.bf(&["plan", "check", "--json"]).json();
    assert_eq!(j["waves"], serde_json::json!({"1": ["U-1"], "2": ["U-2"]}), "{j}");
}
