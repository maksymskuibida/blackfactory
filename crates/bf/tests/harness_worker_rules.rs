//! R-55, R-56, R-59, R-60, T-7, D-28, D-71: the shipped role files, the conductor's skill, the
//! worker rules data, the installer and the notice file say what the design says.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn harness(rel: &str) -> String {
    read(&format!("harness/claude/{rel}"))
}

/// The file names of a folder below `harness/claude`, sorted.
fn names_in(rel: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(repo(&format!("harness/claude/{rel}")))
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

fn role_files() -> Vec<String> {
    names_in("agents")
}

fn instruction_files() -> Vec<PathBuf> {
    let mut out = vec![];
    for dir in ["harness/claude/skills", "harness/claude/agents"] {
        let mut stack = vec![repo(dir)];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "md") {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

struct Rule {
    key: String,
    text: String,
    files: Vec<String>,
}

fn rules() -> Vec<Rule> {
    let doc: toml::Value = toml::from_str(&harness("worker-rules.toml")).unwrap();
    doc["rule"]
        .as_array()
        .expect("rows named rule")
        .iter()
        .map(|r| Rule {
            key: r["key"].as_str().unwrap().to_string(),
            text: r["text"].as_str().unwrap().to_string(),
            files: r["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap().to_string())
                .collect(),
        })
        .collect()
}

// ---- R-60: the worker rules data ----

#[test]
fn the_data_file_lists_every_named_rule_with_its_files() {
    let rs = rules();
    let keys: Vec<&str> = rs.iter().map(|r| r.key.as_str()).collect();
    let want: Vec<String> = (1..=24).map(|n| format!("WR-{n}")).collect();
    assert_eq!(keys, want.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    for r in &rs {
        assert!(!r.files.is_empty(), "{} names no file", r.key);
        assert!(r.text.len() > 20, "{} has no text", r.key);
    }
}

#[test]
fn every_rule_is_found_in_every_file_the_data_names_for_it() {
    for r in rules() {
        for f in &r.files {
            let t = harness(f);
            assert!(
                t.contains(&r.text),
                "{f} does not carry {} word for word: {}",
                r.key,
                r.text
            );
        }
    }
}

/// The bullets of the worker rules section of a file's text: the section named `## Worker rules`
/// (role files) or `## Worker rules of the conductor` (the skill), up to the next `## ` heading.
fn section_bullets(text: &str) -> Vec<String> {
    let mut in_rules = false;
    let mut out = vec![];
    for line in text.lines() {
        if line.starts_with("## ") {
            in_rules = line == "## Worker rules" || line == "## Worker rules of the conductor";
            continue;
        }
        if let Some(b) = line.strip_prefix("- ").filter(|_| in_rules) {
            out.push(b.to_string());
        }
    }
    out
}

/// The texts of the rows that name a file, in the order of the data.
fn rows_for(rel: &str) -> Vec<String> {
    rules()
        .into_iter()
        .filter(|r| r.files.iter().any(|f| f == rel))
        .map(|r| r.text)
        .collect()
}

/// The files whose worker rules section the installer writes: every role file and the conductor's skill.
fn rule_files() -> Vec<String> {
    let mut v: Vec<String> = role_files().iter().map(|f| format!("agents/{f}")).collect();
    v.push("skills/factory/SKILL.md".to_string());
    v
}

/// The shipped source files hold exactly the rows the data names for them, and no other bullet.
#[test]
fn a_shipped_file_holds_the_rows_the_data_names_for_it_and_no_other_worker_rule_bullet() {
    for rel in rule_files() {
        let got = section_bullets(&harness(&rel));
        assert_eq!(
            got,
            rows_for(&rel),
            "{rel}: the section and the data differ"
        );
        assert!(!got.is_empty(), "{rel} has no worker rules section");
    }
}

/// D-71: the installer writes the sections from the data. Both directions: every row the data names
/// for a file is a bullet of the installed file, and every bullet of the installed file is such a row.
#[test]
fn the_installer_writes_each_worker_rules_section_from_the_data_in_both_directions() {
    let tmp = tempfile::TempDir::new().unwrap();
    let proj = fs::canonicalize(tmp.path()).unwrap().join("proj");
    fs::create_dir_all(&proj).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&proj)
        .status()
        .unwrap()
        .success());
    // A copy of the harness folder whose data names one more row and drops another, to show that the
    // installed text follows the data and not the source files.
    let fake = tmp.path().join("fake");
    let fake_harness = fake.join("harness/claude");
    fs::create_dir_all(&fake_harness).unwrap();
    let cp = Command::new("cp")
        .arg("-R")
        .arg(repo("harness/claude/."))
        .arg(&fake_harness)
        .status()
        .unwrap();
    assert!(cp.success());
    fs::create_dir_all(fake.join("views")).unwrap();
    assert!(Command::new("cp")
        .arg("-R")
        .arg(repo("views/templates"))
        .arg(fake.join("views/templates"))
        .status()
        .unwrap()
        .success());
    let data = fs::read_to_string(fake_harness.join("worker-rules.toml")).unwrap();
    // Drop the row WR-3 (the conductor's skill) from the changed data: its block runs to the next [[rule]].
    let dropped_text = "Never do a step for a worker that the guard or a permission refused that worker; tell the human.";
    let parts: Vec<&str> = data.split("[[rule]]").collect();
    let kept: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|p| !p.contains("key = \"WR-3\""))
        .collect();
    assert_eq!(kept.len() + 1, parts.len(), "WR-3 must be one row of the data");
    assert!(parts.iter().any(|p| p.contains(dropped_text)));
    let mut data = kept.join("[[rule]]");
    data.push_str(
        "\n[[rule]]\nkey = \"WR-99\"\nroles = [\"builder\"]\nfiles = [\"agents/builder.md\", \"skills/factory/SKILL.md\"]\ntext = \"A test row that only the data holds.\"\n",
    );
    fs::write(fake_harness.join("worker-rules.toml"), &data).unwrap();

    for (script, label) in [
        (repo("harness/claude/install.sh"), "shipped"),
        (fake_harness.join("install.sh"), "changed data"),
    ] {
        let target = proj.join(label.replace(' ', "-"));
        fs::create_dir_all(&target).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&target)
            .status()
            .unwrap()
            .success());
        let o = Command::new("bash")
            .arg(&script)
            .arg(&target)
            .env("BF_STATE_HOME", tmp.path().join("state-home"))
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let doc: toml::Value =
            toml::from_str(&fs::read_to_string(fake_harness.join("worker-rules.toml")).unwrap())
                .unwrap();
        let data_rows = |rel: &str| -> Vec<String> {
            let src = if label == "shipped" {
                rows_for(rel)
            } else {
                doc["rule"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| {
                        r["files"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|f| f.as_str() == Some(rel))
                    })
                    .map(|r| r["text"].as_str().unwrap().to_string())
                    .collect()
            };
            src
        };
        for rel in rule_files() {
            let installed_rel = if let Some(rest) = rel.strip_prefix("agents/") {
                format!(".claude/agents/{rest}")
            } else {
                ".claude/skills/factory/SKILL.md".to_string()
            };
            let text = fs::read_to_string(target.join(&installed_rel)).unwrap();
            let got = section_bullets(&text);
            let want = data_rows(&rel);
            for w in &want {
                assert!(
                    got.contains(w),
                    "{label}: {installed_rel} lacks a row of the data: {w}"
                );
            }
            for g in &got {
                assert!(
                    want.contains(g),
                    "{label}: {installed_rel} holds a bullet that is no row of the data: {g}"
                );
            }
            assert_eq!(got, want, "{label}: {installed_rel}");
        }
        if label == "changed data" {
            let b = fs::read_to_string(target.join(".claude/agents/builder.md")).unwrap();
            assert!(b.contains("- A test row that only the data holds."));
            let s = fs::read_to_string(target.join(".claude/skills/factory/SKILL.md")).unwrap();
            assert!(s.contains("- A test row that only the data holds."));
            assert!(
                !s.contains(dropped_text),
                "the installed skill still holds the row dropped from the data"
            );
        } else {
            // The shipped sources are what the installer writes for the shipped data.
            for rel in rule_files() {
                let installed_rel = if let Some(rest) = rel.strip_prefix("agents/") {
                    format!(".claude/agents/{rest}")
                } else {
                    ".claude/skills/factory/SKILL.md".to_string()
                };
                assert_eq!(
                    fs::read_to_string(target.join(&installed_rel)).unwrap(),
                    harness(&rel),
                    "{rel}: the shipped file differs from what the installer writes"
                );
            }
        }
    }
}

#[test]
fn the_fix_designer_reports_every_design_with_ask_and_a_reason_or_go_and_changes_no_record() {
    let t = harness("agents/fix-designer.md");
    assert!(t.contains("`ask`") && t.contains("`go`") && t.contains("reason"));
    assert!(t.contains("always carries `ask`"));
    assert!(!t.contains("bf record put") && !t.contains("bf id new"));
}

#[test]
fn the_rules_name_the_files_the_design_says_they_hold_for() {
    let rs = rules();
    let by = |k: &str| rs.iter().find(|r| r.key == k).unwrap();
    let all_roles: BTreeSet<String> = role_files().iter().map(|f| format!("agents/{f}")).collect();
    for k in ["WR-14", "WR-15", "WR-17", "WR-19", "WR-24"] {
        let got: BTreeSet<String> = by(k)
            .files
            .iter()
            .filter(|f| f.starts_with("agents/"))
            .cloned()
            .collect();
        assert_eq!(got, all_roles, "{k} must reach every role file");
    }
    assert!(by("WR-14")
        .files
        .contains(&"skills/factory/SKILL.md".to_string()));
    for (k, f) in [
        ("WR-16", "agents/builder.md"),
        ("WR-18", "agents/builder.md"),
        ("WR-20", "agents/builder.md"),
        ("WR-21", "agents/reviewer.md"),
        ("WR-22", "agents/designer.md"),
        ("WR-23", "agents/designer.md"),
    ] {
        assert_eq!(by(k).files, vec![f.to_string()], "{k}");
    }
    for k in [
        "WR-1", "WR-2", "WR-3", "WR-4", "WR-5", "WR-6", "WR-7", "WR-8", "WR-9", "WR-10", "WR-11",
        "WR-12", "WR-13",
    ] {
        assert!(
            by(k).files.iter().all(|f| f.starts_with("skills/factory/")),
            "{k} is a conductor rule"
        );
    }
}

#[test]
fn no_shipped_instruction_names_a_build_tool_a_cap_number_or_a_public_repository_rule() {
    let mut files = instruction_files();
    files.push(repo("harness/claude/worker-rules.toml"));
    for p in files {
        let t = fs::read_to_string(&p).unwrap().to_lowercase();
        for bad in [
            "cargo",
            "nextest",
            "pytest",
            "npm ",
            "gradle",
            "maven",
            "jest",
            "vitest",
            "public repository",
            "public code repository",
        ] {
            assert!(!t.contains(bad), "{} names `{bad}`", p.display());
        }
        for line in t.lines() {
            if line.contains("in flight") {
                let body =
                    line.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ');
                assert!(
                    !body.chars().any(|c| c.is_ascii_digit()),
                    "{}: a number for the cap of units in flight: {line}",
                    p.display()
                );
            }
        }
    }
}

// ---- D-71, T-7: length, shape and private marks ----

#[test]
fn every_skill_file_and_role_file_stays_within_two_hundred_lines() {
    for p in instruction_files() {
        let n = fs::read_to_string(&p).unwrap().lines().count();
        assert!(n <= 200, "{} has {n} lines", p.display());
    }
}

#[test]
fn the_skill_is_a_short_loop_with_reference_files_it_names() {
    let skill = harness("skills/factory/SKILL.md");
    assert!(skill.lines().count() <= 100, "SKILL.md is not short");
    let refs = names_in("skills/factory");
    assert!(refs.len() >= 6, "{refs:?}");
    for f in refs.iter().filter(|f| *f != "SKILL.md") {
        assert!(
            skill.contains(&format!("`{f}`")),
            "SKILL.md does not name {f}"
        );
    }
    for f in [
        "design-stages.md",
        "build-loop.md",
        "checkpoint.md",
        "spaces-intents.md",
        "asking.md",
    ] {
        assert!(refs.contains(&f.to_string()), "{f} is missing");
    }
}

#[test]
fn no_harness_file_names_a_person_a_private_project_a_home_path_or_a_private_link() {
    let mut files = instruction_files();
    for rel in [
        "harness/claude/worker-rules.toml",
        "harness/claude/README.md",
        "NOTICE.md",
    ] {
        files.push(repo(rel));
    }
    for p in files {
        let t = fs::read_to_string(&p).unwrap();
        for bad in [
            "/Users/",
            "/home/",
            "C:\\Users",
            "claude.ai/",
            "artifacts/",
            "@gmail",
            "@users.noreply",
        ] {
            assert!(!t.contains(bad), "{} holds `{bad}`", p.display());
        }
        if p.extension().is_some_and(|x| x == "md") && !p.ends_with("README.md") {
            assert!(!t.contains("](http"), "{} holds a link", p.display());
        }
    }
}

// ---- R-55: the conductor reads state from the engine and commits the records ----

#[test]
fn the_skill_says_to_report_a_state_only_as_read_from_the_engine() {
    let skill = harness("skills/factory/SKILL.md");
    for want in [
        "Never report the state of a stage, a unit, a wave, a checkpoint or anything else on an agent's words or on your own judging",
        "read it from `bf next`, `bf status` or `bf progress` first",
    ] {
        assert!(skill.contains(want), "SKILL.md lacks `{want}`");
    }
}

#[test]
fn the_instructions_list_the_commit_of_the_records_right_after_an_approved_amendment() {
    let skill = harness("skills/factory/SKILL.md");
    let am = harness("skills/factory/amendments.md");
    for t in [&skill, &am] {
        assert!(t.contains(
            "The step right after `bf amend approve` is the commit of the factory's records; you neither judge whether to commit nor ask the human."
        ));
    }
    let after = &am[am.find("## After `bf amend approve`").expect("the section")..];
    let first = after.lines().find(|l| l.starts_with("1. ")).unwrap();
    assert!(first.contains("Commit the factory's records"), "{first}");
}

#[test]
fn no_step_of_the_skill_writes_a_time_into_a_questions_page() {
    for f in names_in("skills/factory") {
        let t = harness(&format!("skills/factory/{f}")).to_lowercase();
        for line in t.lines().filter(|l| l.contains("questions")) {
            for bad in [
                "timestamp",
                "current time",
                "the time",
                "date +",
                "answered_at",
                "write the date",
            ] {
                assert!(!line.contains(bad), "{f}: {line}");
            }
        }
    }
}

// ---- R-59: the model and every role file ----

fn model_of(tier: &str) -> &'static str {
    match tier {
        "cheap" => "haiku",
        "standard" => "sonnet",
        "strong" => "opus",
        other => panic!("tier {other}"),
    }
}

/// The kind of work of the shipped policy that a role's work is.
fn kind_of(role: &str) -> &'static str {
    match role {
        "builder" | "knowledge-writer" => "builder",
        "verifier" | "uiux-verifier" => "verifier",
        "repair-judge" => "amender",
        "reviewer" | "security-auditor" | "legal-auditor" | "design-reviewer" => "reviewer",
        "designer" | "uiux-designer" | "fix-designer" | "rule-proposer" | "security-expert"
        | "lawyer" => "designer",
        other => panic!("no kind of work for {other}"),
    }
}

#[test]
fn each_role_file_names_the_model_the_shipped_policy_gives_for_its_kind_of_work() {
    let policy: toml::Value = toml::from_str(&bf::store::default_policy_text(
        "/usr/bin:/bin",
        "/usr/bin/git",
    ))
    .unwrap();
    let files = role_files();
    assert!(files.len() >= 14, "{files:?}");
    for f in files {
        let name = f.trim_end_matches(".md");
        let t = harness(&format!("agents/{f}"));
        assert!(t.starts_with(&format!("---\nname: {name}\n")), "{f}");
        let tier = policy["tiers"][kind_of(name)].as_str().unwrap();
        let want = format!("model: {}", model_of(tier));
        let front: Vec<&str> = t.lines().skip(1).take_while(|l| *l != "---").collect();
        assert!(front.contains(&want.as_str()), "{f} should name `{want}`");
    }
}

#[test]
fn each_role_file_says_a_required_check_runs_only_through_the_engine() {
    for f in role_files() {
        let t = harness(&format!("agents/{f}"));
        assert!(
            t.contains("A required check runs only through the engine; never run one in a shell of your own."),
            "{f}"
        );
    }
}

#[test]
fn the_roles_this_run_adds_have_role_files() {
    for r in [
        "fix-designer",
        "knowledge-writer",
        "rule-proposer",
        "uiux-verifier",
    ] {
        assert!(role_files().contains(&format!("{r}.md")), "{r}");
    }
    assert!(!role_files().contains(&"ux-verifier.md".to_string()));
}

#[test]
fn the_skill_starts_each_worker_on_the_model_its_directive_names_and_passes_no_other() {
    let mut all = String::new();
    for f in names_in("skills/factory") {
        all.push_str(&harness(&format!("skills/factory/{f}")));
        all.push('\n');
    }
    assert!(all.contains(
        "Start every worker on the model its directive names. Never pass another model."
    ));
    assert!(all.contains("on the model its directive names"));
    for bad in [
        "pass the model",
        "pass a model",
        "choose the model",
        "pick the model",
        "model you prefer",
    ] {
        assert!(!all.to_lowercase().contains(bad), "the skill says `{bad}`");
    }
}

#[test]
fn the_installer_installs_every_role_file_and_names_each_file_it_wrote() {
    let tmp = tempfile::TempDir::new().unwrap();
    let proj = fs::canonicalize(tmp.path()).unwrap().join("proj");
    fs::create_dir_all(&proj).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&proj)
        .status()
        .unwrap()
        .success());
    let o = Command::new("bash")
        .arg(repo("harness/claude/install.sh"))
        .arg(&proj)
        .env("BF_STATE_HOME", tmp.path().join("state-home"))
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(
        o.status.success(),
        "{out}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );

    let installed: Vec<String> = {
        let mut v: Vec<String> = fs::read_dir(proj.join(".claude/agents"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        installed,
        role_files(),
        "the installed and shipped role files differ"
    );
    for f in role_files() {
        assert!(
            out.contains(&format!("created    .claude/agents/{f}")),
            "the output does not name {f}:\n{out}"
        );
    }
    for f in names_in("skills/factory") {
        assert!(
            proj.join(".claude/skills/factory").join(&f).is_file(),
            "{f}"
        );
        assert!(
            out.contains(&format!("created    .claude/skills/factory/{f}")),
            "{f}"
        );
    }
    assert!(out.contains("Next:") && out.contains("/factory"), "{out}");

    // A second run changes nothing and says so for each file.
    let o2 = Command::new("bash")
        .arg(repo("harness/claude/install.sh"))
        .arg(&proj)
        .env("BF_STATE_HOME", tmp.path().join("state-home"))
        .output()
        .unwrap();
    let out2 = String::from_utf8_lossy(&o2.stdout).to_string();
    assert!(o2.status.success());
    assert!(
        !out2.contains("created") && !out2.contains("updated    .claude/agents"),
        "{out2}"
    );
    assert!(out2.contains("unchanged  .claude/agents/builder.md"));
}

#[test]
fn the_installer_names_an_old_role_file_it_leaves() {
    let tmp = tempfile::TempDir::new().unwrap();
    let proj = fs::canonicalize(tmp.path()).unwrap().join("proj");
    fs::create_dir_all(proj.join(".claude/agents")).unwrap();
    fs::write(proj.join(".claude/agents/ux-verifier.md"), "old\n").unwrap();
    assert!(Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&proj)
        .status()
        .unwrap()
        .success());
    let o = Command::new("bash")
        .arg(repo("harness/claude/install.sh"))
        .arg(&proj)
        .env("BF_STATE_HOME", tmp.path().join("state-home"))
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(o.status.success(), "{out}");
    assert!(
        out.contains("left       .claude/agents/ux-verifier.md"),
        "{out}"
    );
    assert!(proj.join(".claude/agents/ux-verifier.md").is_file());
}

// ---- R-54: the engine document and the lawyer's file ----

#[test]
fn the_engine_document_gives_the_steps_for_a_kept_record_in_the_engines_order() {
    let doc = read("docs/engine.md");
    let at = doc
        .find("A kept record that must change is moved")
        .expect("the steps");
    let steps = &doc[at..];
    let pos = |needle: &str| {
        steps
            .find(needle)
            .unwrap_or_else(|| panic!("no `{needle}`"))
    };
    let order = [
        "1. `bf amend revise",
        "2. Put the record",
        "3. `bf amend narrow",
        "4. The human's yes",
        "5. `bf amend approve",
    ];
    let at: Vec<usize> = order.iter().map(|n| pos(n)).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
    for want in [
        "every record that changed",
        "A draft of an amendment that is not approved has no force",
        "The hash of the threats",
        "The human's words count for lifting the designer rule only when they were written after the amendment was opened",
    ] {
        assert!(doc.contains(want), "engine.md lacks `{want}`");
    }
}

#[test]
fn the_engine_document_the_skill_and_the_lawyer_name_the_three_commands() {
    let doc = read("docs/engine.md");
    let mut skill = String::new();
    for f in names_in("skills/factory") {
        skill.push_str(&harness(&format!("skills/factory/{f}")));
    }
    let lawyer = harness("agents/lawyer.md");
    for (what, t) in [
        ("engine.md", &doc),
        ("the skill", &skill),
        ("the lawyer", &lawyer),
    ] {
        for cmd in ["bf scan", "bf preset", "bf uiux coverage"] {
            assert!(t.contains(cmd), "{what} lacks `{cmd}`");
        }
    }
    assert!(lawyer.contains("Presets exist"));
    assert!(lawyer.contains("bf preset fill"));
}

// ---- R-56: the notice file ----

#[test]
fn the_notice_names_a_source_for_every_text_it_says_was_copied() {
    let notice = read("NOTICE.md");
    for entry in notice.split("\n## ") {
        for para in entry.split("\n\n") {
            let p = para.to_lowercase();
            let claims_copy = ["copied", "adapted", "derived from", "reproduced"]
                .iter()
                .any(|w| p.contains(w));
            let denies = p.contains("no text was copied")
                || p.contains("no code and no text")
                || p.contains("was copied or adapted");
            if claims_copy && !denies {
                assert!(
                    p.contains("source")
                        && p.contains("licence")
                        && (p.contains("copyright") || p.contains("(c)")),
                    "an entry on copied text names no source, licence and copyright line: {para}"
                );
            }
        }
    }
}

#[test]
fn the_notice_says_the_ideas_of_ai_dlc_v2_were_taken_and_no_code_or_text_was_copied() {
    let n = read("NOTICE.md");
    for want in [
        "AI-DLC v2",
        "`space` and `intent`",
        "the layers, the templates and the learning loop",
        "taken as ideas",
        "No code and no text of AI-DLC v2 was copied",
    ] {
        assert!(n.contains(want), "NOTICE.md lacks `{want}`");
    }
}
