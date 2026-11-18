//! The UI/UX verifier role, the contracts and the four driver guides are in the harness, call no
//! skill, and name their tools.

use std::fs;
use std::path::Path;

fn read(file: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(file);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn has(file: &str, needles: &[&str]) {
    let t = read(file);
    for n in needles {
        assert!(t.contains(n), "{file} should contain `{n}`");
    }
}

const ROLE: &str = "harness/claude/agents/uiux-verifier.md";
const DRIVERS: [(&str, &[&str]); 4] = [
    ("web", &["playwright", "node", "curl"]),
    ("ios", &["xcrun", "simctl", "xcodebuild"]),
    ("android", &["adb", "emulator", "uiautomator"]),
    ("terminal", &["bash", "tmux", "exit"]),
];
const SECTIONS: [&str; 4] = ["## Tools needed", "## Start the product", "## Act", "## Capture evidence"];

#[test]
fn the_role_file_has_its_name_and_limits() {
    let t = read(ROLE);
    assert!(t.starts_with("---\nname: uiux-verifier\n"));
    has(
        ROLE,
        &[
            "model: sonnet",
            "command-line tools only",
            "unreachable",
            "Never turn unreachable into met",
            "bf evidence attach",
            "Never approve a gate or an amendment",
        ],
    );
}

#[test]
fn the_role_file_has_the_amendment_stage_paragraph() {
    has(
        ROLE,
        &[
            "UI/UX stage review of an amendment",
            "judge only the screens and journeys the amendment names",
            "File each problem as a finding of the amendment",
            "stage token",
            "--outcome met|failed",
        ],
    );
}

#[test]
fn the_role_file_names_every_driver_and_contract() {
    let mut needles = vec![
        "harness/claude/ux/journey-contract.md",
        "harness/claude/ux/report-contract.md",
        "harness/claude/ux/accessibility-checklist.md",
    ];
    for (d, _) in DRIVERS {
        needles.push(Box::leak(format!("drivers/{d}.md").into_boxed_str()));
    }
    has(ROLE, &needles);
}

#[test]
fn each_driver_tells_start_act_capture_and_tools() {
    for (d, tools) in DRIVERS {
        let f = format!("harness/claude/ux/drivers/{d}.md");
        let t = read(&f);
        for s in SECTIONS {
            assert!(t.contains(s), "{f} should contain `{s}`");
        }
        for tool in tools {
            assert!(t.contains(tool), "{f} should name `{tool}`");
        }
        assert!(t.contains("bf evidence unreachable"), "{f}: a missing tool is unreachable");
        assert!(t.lines().count() <= 200, "{f} is over 200 lines");
    }
}

#[test]
fn the_contracts_exist_and_stay_within_the_cap() {
    for f in ["journey-contract", "report-contract", "accessibility-checklist"] {
        let file = format!("harness/claude/ux/{f}.md");
        let t = read(&file);
        assert!(t.lines().count() <= 200 && t.len() > 200, "{file}");
    }
    has("harness/claude/ux/journey-contract.md", &["--says", "J-id> step", "unreachable"]);
    has("harness/claude/ux/report-contract.md", &["met", "failed", "unreachable"]);
    has("harness/claude/ux/accessibility-checklist.md", &["WCAG 2.2 level AA", "Perceivable"]);
}

#[test]
fn no_file_calls_a_skill() {
    let mut files = vec![ROLE.to_string()];
    for f in ["journey-contract", "report-contract", "accessibility-checklist"] {
        files.push(format!("harness/claude/ux/{f}.md"));
    }
    for (d, _) in DRIVERS {
        files.push(format!("harness/claude/ux/drivers/{d}.md"));
    }
    for f in files {
        let t = read(&f).to_lowercase();
        for bad in ["skill tool", "/skill", "invoke the", "use the skill", "superpowers", "frontend-design"] {
            assert!(!t.contains(bad), "{f} should not mention `{bad}`");
        }
        assert!(!t.contains("call a skill") || f == ROLE, "{f}");
    }
}

#[test]
fn the_notice_says_the_ux_files_were_written_new() {
    has(
        "NOTICE.md",
        &["harness/claude/ux/", "journey-contract.md", "report-contract.md", "accessibility-checklist.md", "written new", "WCAG 2.2"],
    );
}

#[test]
fn the_notice_makes_no_unattributed_mit_claim() {
    let n = read("NOTICE.md");
    let i = n.find("## UI/UX mode contracts").unwrap();
    assert!(!n[i..].contains("MIT"), "no MIT claim without a named source");
}

#[test]
fn the_driver_and_role_fixes_stay_pinned() {
    has("harness/claude/ux/drivers/web.md", &["npx --no-install playwright --version"]);
    has(
        ROLE,
        &["the bounded start-up waits the driver guides describe (a fixed number of tries) are allowed"],
    );
}
