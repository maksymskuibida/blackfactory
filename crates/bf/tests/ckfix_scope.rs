//! R-70, D-115: a checkpoint fix names its files in a gap report, and an abort saves its work for the next
//! fix of the checkpoint.

mod common;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const K_CHECK: &str = "grep -q fixed src/a.txt";

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"x\"\nacceptance=[\"y\"]\n".to_string()),
        ("unit", "id=\"U-1\"\nkind=\"unit\"\ntitle=\"u\"\ndelivers=[\"R-1\"]\nfiles=[\"src/a.txt\", \"tests/a_test.txt\"]\ncheckpoint=\"K-1\"\n".to_string()),
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"c\"\nhow=\"command\"\ncommand=\"test -f src/a.txt\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-1\"\n".to_string()),
        ("check", format!("id=\"X-2\"\nkind=\"check\"\ntitle=\"c2\"\nhow=\"command\"\ncommand=\"{K_CHECK}\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n")),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"k\"\nafter=[\"U-1\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n".to_string()),
    ] {
        let r = fx.put(k, &t);
        assert_eq!(r.code, 0, "{k}: {}{}", r.stdout, r.stderr);
    }
    fs::create_dir_all(fx.root.join("tests/common")).unwrap();
    fs::write(fx.root.join("tests/common/mod.rs"), "pub fn helper() {}\n").unwrap();
    fs::write(fx.root.join("tests/attacks.rs"), "// attacks\nfn a() {}\n").unwrap();
    fx.commit_all("records");
}

fn land_unit(fx: &Fx) {
    fx.must(&["gate", "open", "plan"]);
    fx.human("I approve the plan.");
    fx.must(&["gate", "approve", "plan", "--quote", "I approve the plan."]);
    fx.must(&["run", "start"]);
    fx.answer_plan_questions();
    let wc = PathBuf::from(
        fx.must(&["unit", "start", "U-1", "--json"]).json()["working_copy"]
            .as_str()
            .unwrap(),
    );
    fs::create_dir_all(wc.join("src")).unwrap();
    fs::write(wc.join("src/a.txt"), "from the unit\n").unwrap();
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
            "src/a.txt",
        ],
    );
    fx.must(&["report", "U-1", "--result", "built"]);
    fx.must(&["report", "U-1", "--result", "built", "--role", "reviewer"]);
    fx.must(&["unit", "land", "U-1"]);
}

fn failing_run(fx: &Fx) -> PathBuf {
    let k = fx.work("K-1");
    let o = fx.bf_in(
        &k,
        &[
            "evidence",
            "run",
            "X-2",
            "--",
            "grep",
            "-q",
            "fixed",
            "src/a.txt",
        ],
    );
    assert!(!o.all().is_empty());
    k
}

fn one_directive(fx: &Fx, action: &str) -> Value {
    let n = fx.must(&["next", "--json"]).json();
    let ds = n["directives"].as_array().unwrap();
    assert_eq!(ds.len(), 1, "one directive while a fix is open: {n}");
    assert_eq!(ds[0]["action"], action, "{n}");
    ds[0].clone()
}

fn open_fix(fx: &Fx) -> PathBuf {
    plan(fx);
    land_unit(fx);
    let k = failing_run(fx);
    one_directive(fx, "fix-checkpoint");
    k
}

#[test]
fn ckfix_scope_a_gap_names_its_files_and_a_file_of_the_list_is_refused() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    let gap = |note: &str| {
        fx.bf_in(
            &k,
            &[
                "report", "K-1", "--role", "builder", "--result", "gap", "--note", note,
            ],
        )
    };
    let o = gap("needs behaviour no record describes");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("names none"), "{}", o.stderr);
    let o = gap("src/a.txt needs more behaviour");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(
        o.stderr.contains("src/a.txt is already in the files"),
        "{}",
        o.stderr
    );
    // A place in the file (`:12`, `:12:3`) is not part of its path.
    for note in [
        "src/a.txt:12 needs more behaviour",
        "see src/a.txt:12:3, it needs more",
    ] {
        let o = gap(note);
        assert_eq!(o.code, 2, "{}", o.all());
        assert!(
            o.stderr.contains("src/a.txt is already in the files"),
            "{}",
            o.stderr
        );
    }
    one_directive(&fx, "fix-checkpoint");
    let o = gap("src/new_module.txt: behaviour no record describes");
    assert_eq!(o.code, 2, "{}", o.all());
    let d = one_directive(&fx, "plan-gap");
    assert!(
        d["missing"].to_string().contains("src/new_module.txt"),
        "{d}"
    );
}

#[test]
fn ckfix_scope_an_abort_saves_the_work_and_the_next_fix_starts_from_it() {
    let fx = Fx::new();
    let k = open_fix(&fx);
    fs::write(k.join("src/a.txt"), "half done\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "half"]);
    fs::write(k.join("src/other.txt"), "outside\n").unwrap();
    git(&k, &["add", "-A"]);
    git(&k, &["commit", "-q", "-m", "outside"]);
    fs::create_dir_all(k.join("tests")).unwrap();
    fs::write(k.join("tests/new_test.txt"), "a new test\n").unwrap();
    let v = fx
        .must(&["checkpoint", "fix", "K-1", "--abort", "--json"])
        .json();
    let saved = v["saved"].as_str().unwrap().to_string();
    assert!(saved.starts_with("factory/evidence/fix-saves/K-1/"), "{v}");
    assert!(fx.root.join(&saved).join("saved.json").is_file());
    assert_eq!(
        fs::read_to_string(k.join("src/a.txt")).unwrap(),
        "from the unit\n"
    );
    assert!(!k.join("tests/new_test.txt").exists());
    // The failing check opens a new fix, which starts from the saved work.
    let d = one_directive(&fx, "fix-checkpoint");
    assert_eq!(d["from_saved"], saved.as_str(), "{d}");
    let note = fs::read_to_string(fx.root.join(&saved).join("reopened.json")).unwrap_or_default();
    assert_eq!(
        fs::read_to_string(k.join("src/a.txt")).unwrap(),
        "half done\n",
        "{note}"
    );
    assert_eq!(
        fs::read_to_string(k.join("tests/new_test.txt")).unwrap(),
        "a new test\n"
    );
    assert!(!k.join("src/other.txt").exists());
    let b = fs::read_to_string(
        fx.must(&["brief", "K-1", "--role", "builder"])
            .stdout
            .trim(),
    )
    .unwrap();
    assert!(
        b.contains(&saved) && b.contains("src/other.txt") && b.contains("outside the files"),
        "{b}"
    );
    // Only the next fix starts from it: an abort with nothing to save saves nothing.
    git(&k, &["add", "-A"]);
    git(&k, &["stash", "-q", "-u"]);
    let v = fx
        .must(&["checkpoint", "fix", "K-1", "--abort", "--json"])
        .json();
    assert_eq!(v["saved"], "", "{v}");
}
