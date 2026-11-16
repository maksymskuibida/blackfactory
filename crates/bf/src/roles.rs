//! `bf roles`: the five new roles. This file belongs to this command group alone; the command line
//! calls its entry point.
//!
//! The roles are the UI/UX verifier, the security expert, the security auditor, the lawyer and the
//! legal auditor. Each has a name, a stage, what it may read, write and run (handoff H-5, which the
//! stage wiring reads), a brief the engine writes, and a one-use token like the existing roles.

use crate::plan;
use crate::refusal::{refuse, Fail, R};
use crate::run;
use crate::store::{s, str_list, Project};
use crate::util;
use serde_json::{json, Value};

mod design;

/// One new role and its limits.
pub struct Role {
    pub name: &'static str,
    /// The stage it works at: `design` or `built`.
    pub stage: &'static str,
    pub summary: &'static str,
    pub reads: &'static [&'static str],
    pub writes: &'static [&'static str],
    pub runs: &'static [&'static str],
    /// A blind role judges the built result against the contract and does not read the source.
    pub blind: bool,
    /// The policy tier it runs at: `builder`, `reviewer` or `verifier`.
    pub tier: &'static str,
    /// The check family whose checks this role's token may settle: `uiux`, `security` or `legal`.
    pub family: &'static str,
    /// A built-stage role that also works at the design stage (the UI/UX verifier judges the screens
    /// before the plan exists).
    pub also_design: bool,
    /// The kinds of review this role judges at the gates; empty for a role that judges none.
    pub kinds: &'static [&'static str],
    /// The kinds of review this role judges inside an amendment.
    pub amendment_kinds: &'static [&'static str],
}

/// The subject of a design-stage brief, token and report: the stage itself, since no unit exists yet.
pub const DESIGN: &str = "design";

impl Role {
    /// Whether the role works at the design stage.
    pub fn at_design(&self) -> bool {
        self.stage == "design" || self.also_design
    }
}

pub const ROLES: &[Role] = &[
    Role {
        name: "uiux-verifier",
        kinds: &[],
        amendment_kinds: &[],
        stage: "built",
        summary: "Judges the screens and the built interface against the contract, blind.",
        reads: &["the brief", "the contract", "the built result and its screens"],
        writes: &[],
        runs: &[
            "bf evidence run|verify|fail|unreachable|attach",
            "bf report",
            "bf ask",
            "bf record get|list",
            "bf status",
        ],
        blind: true,
        tier: "verifier",
        family: "uiux",
        also_design: true,
    },
    Role {
        name: "security-expert",
        kinds: &[],
        amendment_kinds: &[],
        stage: "design",
        summary: "Drafts the threats at the design stage: what is protected, the attack, the mitigation.",
        reads: &["the brief", "the design records", "the architecture"],
        writes: &["threat records (`bf record put threat`)"],
        runs: &[
            "bf record put threat",
            "bf report",
            "bf ask",
            "bf record get|list",
            "bf status",
        ],
        blind: false,
        tier: "reviewer",
        family: "security",
        also_design: false,
    },
    Role {
        name: "security-auditor",
        kinds: &[],
        amendment_kinds: &[],
        stage: "built",
        summary: "Files findings on the built result against the threat model, the diff and the scan output.",
        reads: &["the brief", "the threat records", "the diff", "the scan output"],
        writes: &["findings (`bf record put finding`)"],
        runs: &[
            "bf evidence verify",
            "bf record put finding",
            "bf report",
            "bf ask",
            "bf record get|list",
            "bf status",
        ],
        blind: false,
        tier: "reviewer",
        family: "security",
        also_design: false,
    },
    Role {
        name: "lawyer",
        kinds: &[],
        amendment_kinds: &[],
        stage: "design",
        summary: "Reads the design against the markets the product serves and files what the law needs.",
        reads: &[
            "the brief",
            "the design records",
            "the markets statement",
            "sources fetched on demand",
        ],
        writes: &[
            "legal records (`bf record put legal`)",
            "the markets record (`bf record put markets`)",
        ],
        runs: &[
            "bf record put legal",
            "bf record put markets",
            "bf report",
            "bf ask",
            "bf record get|list",
            "bf status",
        ],
        blind: false,
        tier: "reviewer",
        family: "legal",
        also_design: false,
    },
    Role {
        name: "legal-auditor",
        kinds: &[],
        amendment_kinds: &[],
        stage: "built",
        summary: "Audits the built result against the markets statement and files legal findings.",
        reads: &[
            "the brief",
            "the design records",
            "the markets statement",
            "the diff",
            "sources fetched on demand",
        ],
        writes: &["legal findings (`bf record put legal`)"],
        runs: &[
            "bf evidence verify",
            "bf record put legal",
            "bf report",
            "bf ask",
            "bf record get|list",
            "bf status",
        ],
        blind: false,
        tier: "reviewer",
        family: "legal",
        also_design: false,
    },
    Role {
        name: "design-reviewer",
        kinds: &["idea", "requirements", "architecture", "plan"],
        amendment_kinds: &["amendment", "requirements", "architecture", "legal", "plan"],
        stage: "design",
        summary: "Judges the records of a design stage or of an amendment cold, from the records alone, and files findings.",
        reads: &[
            "the brief",
            "the records of the stage it judges",
            "the approved records of earlier stages they link to",
            "the stage's questions with the human's answers",
        ],
        writes: &["findings (`bf record put finding`)"],
        runs: &[
            "bf record put finding",
            "bf report",
            "bf ask",
            "bf record get|list",
            "bf status",
        ],
        blind: false,
        tier: "reviewer",
        family: "review",
        also_design: false,
    },
    Role {
        name: "uiux-designer",
        kinds: &[],
        amendment_kinds: &[],
        stage: "design",
        summary: "Writes the screens and journeys its brief names.",
        reads: &["the brief", "the design records", "the approved records they link to"],
        writes: &[
            "screens (`bf record put screen`)",
            "journeys (`bf record put journey`)",
        ],
        runs: &[
            "bf record put screen",
            "bf record put journey",
            "bf report",
            "bf ask",
            "bf record get|list",
            "bf status",
        ],
        blind: false,
        tier: "builder",
        family: "uiux",
        also_design: false,
    },
];

/// The role named `name`, if it is one of the seven.
pub fn get(name: &str) -> Option<&'static Role> {
    ROLES.iter().find(|r| r.name == name)
}

/// The roles that write records of `kind` (`threat` or `legal`), by name.
pub fn writers_of(kind: &str) -> Vec<&'static str> {
    let needle = format!("`bf record put {kind}`");
    ROLES
        .iter()
        .filter(|r| r.writes.iter().any(|w| w.contains(&needle)))
        .map(|r| r.name)
        .collect()
}

pub fn is_new_role(name: &str) -> bool {
    get(name).is_some()
}

fn describe(r: &Role) -> Value {
    json!({
        "name": r.name, "stage": r.stage, "also_design": r.also_design, "family": r.family, "summary": r.summary, "blind": r.blind, "tier": r.tier,
        "kinds": r.kinds, "amendment_kinds": r.amendment_kinds,
        "reads": r.reads, "writes": r.writes, "runs": r.runs,
        "may_write_code": false,
    })
}

/// A role works only on a subject at its stage: the design stage (`design`, before any plan exists)
/// for a design-stage role, a unit or a checkpoint of the plan for a built-stage role.
pub fn check_subject(p: &Project, subject: &str, role: &str) -> R<&'static Role> {
    let Some(r) = get(role) else {
        return Err(unknown(role));
    };
    if subject == DESIGN {
        if !r.at_design() {
            return Err(refuse(
                format!("use the role `{role}` at the design stage"),
                format!(
                    "`{role}` works at the {} stage, on a unit or a checkpoint",
                    r.stage
                ),
                &["Use a design-stage role here: `security-expert`, `lawyer` or `uiux-verifier`."],
                "The command prints its result.",
            ));
        }
        return Ok(r);
    }
    let pl = plan::load(p)?;
    let known = pl.unit(subject).is_some() || pl.checkpoints.iter().any(|k| s(k, "id") == subject);
    if !known {
        return Err(refuse(
            format!("use the role `{role}` on {subject}"),
            "no such unit or checkpoint, and it is not the design stage",
            &["Use a unit or checkpoint ID from `bf status`, or `design` for a design-stage role."],
            "The command prints its result.",
        ));
    }
    if r.stage != "built" {
        return Err(refuse(
            format!("use the role `{role}` on {subject}"),
            format!("`{role}` works at the design stage, not on a unit or a checkpoint"),
            &["Use `design` as the subject: `bf roles token design <role>`, `bf brief design --role <role>`."],
            "The command prints its result.",
        ));
    }
    Ok(r)
}

/// The brief of the design stage for a design-stage role: no unit exists, so it lists the design records.
fn design_brief(p: &Project, r: &Role) -> R<std::path::PathBuf> {
    let role = r.name;
    let mut md = format!("# Brief: {DESIGN} as {role}\n\n");
    md.push_str(&format!(
        "## Your role\n\n{} Stage: design. No plan exists yet.\n\n## The design records\n\n",
        r.summary
    ));
    for kind in [
        "requirement",
        "screen",
        "journey",
        "component",
        "decision",
        "target",
        "threat",
        "markets",
    ] {
        for rec in p.list_records(crate::schema::kind_by_name(kind))? {
            md.push_str(&format!(
                "- {} {}: {}\n",
                kind,
                s(&rec, "id"),
                s(&rec, "title")
            ));
        }
    }
    md.push_str("\n## You may\n\nRead:\n");
    for x in r.reads {
        md.push_str(&format!("- {x}\n"));
    }
    md.push_str("\nWrite:\n");
    if r.writes.is_empty() {
        md.push_str("- nothing\n");
    }
    for x in r.writes {
        md.push_str(&format!("- {x}\n"));
    }
    md.push_str("\nRun:\n");
    for x in r.runs {
        md.push_str(&format!("- `{x}`\n"));
    }
    md.push_str(&format!(
        "\n## Limits\n\n- Review rounds: up to {}.\n- Idle limit: {} minutes.\n",
        p.policy.limits.max_rounds, p.policy.limits.idle_minutes
    ));
    md.push_str(&format!(
        "\n## Rules\n\n- Report with `bf report {DESIGN} --result built|blocked --role {role} --token <token>`; the report spends your token and works outside a run.\n- Your token ends when a run starts or stops.\n- Never edit anything under the factory or engine folders.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot go on without the human's decision, ask: `bf ask --title \"<the question>\"`.\n- A check counts only when `bf evidence run` ran it.\n"
    ));
    let path = p.briefs_dir().join(format!("{DESIGN}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// The brief of one unit for one of the new roles, written to the briefs folder as `<unit>-<role>.md`.
pub fn brief(p: &Project, unit: &str, role: &str) -> R<std::path::PathBuf> {
    let r = check_subject(p, unit, role)?;
    if unit == DESIGN {
        return design_brief(p, r);
    }
    let pl = plan::load(p)?;
    let Some(u) = pl.unit(unit) else {
        return Err(refuse(
            format!("brief {unit}"),
            "no such unit",
            &["Use a unit ID from `bf status`."],
            "The command prints a path.",
        ));
    };
    let wc = p.work_dir().join(unit);
    let mut md = format!("# Brief: {unit} as {role}\n\nUnit: {}\n\n", s(u, "title"));
    if !s(u, "text").is_empty() {
        md.push_str(&format!("{}\n\n", s(u, "text")));
    }
    md.push_str(&format!(
        "## Your role\n\n{} Stage: {}.\n\n",
        r.summary, r.stage
    ));
    md.push_str("## What this unit delivers\n\n");
    for id in str_list(u, "delivers") {
        if let Some(rec) = p.get_record(&id)? {
            md.push_str(&format!("### {} {}\n\n", id, s(&rec, "title")));
            if !s(&rec, "text").is_empty() {
                md.push_str(&format!("{}\n\n", s(&rec, "text")));
            }
            for a in str_list(&rec, "acceptance") {
                md.push_str(&format!("- Acceptance: {a}\n"));
            }
        }
    }
    md.push_str("\n## You may\n\nRead:\n");
    for x in r.reads {
        md.push_str(&format!("- {x}\n"));
    }
    md.push_str("\nWrite:\n");
    if r.writes.is_empty() {
        md.push_str("- nothing\n");
    }
    for x in r.writes {
        md.push_str(&format!("- {x}\n"));
    }
    md.push_str("\nRun:\n");
    for x in r.runs {
        md.push_str(&format!("- `{x}`\n"));
    }
    md.push_str("\n## Where you work\n\n");
    if r.blind {
        md.push_str("You are a blind verifier: judge the built result against the contract above. Do not read the source.\n\n");
    } else {
        md.push_str(&format!(
            "Working copy: `{}`\n\nYou write no code and no file in it.\n\n",
            wc.display()
        ));
    }
    md.push_str(&format!(
        "## Limits\n\n- Review rounds: up to {}.\n- Idle limit: {} minutes.\n",
        p.policy.limits.max_rounds, p.policy.limits.idle_minutes
    ));
    md.push_str(&format!(
        "\n## Rules\n\n- Report with `bf report <unit> --result built|blocked --role {role} --token <token>`; the report spends your token and does not move the unit.\n- Never edit anything under the factory or engine folders.\n- Never run `bf gate`, `bf human-turn`, `git push` or `git merge`.\n- If you cannot go on without the human's decision, ask: `bf ask --title \"<the question>\"`.\n- A check counts only when `bf evidence run` ran it.\n"
    ));
    let path = p.briefs_dir().join(format!("{unit}-{role}.md"));
    util::atomic_write(&path, md.as_bytes())?;
    Ok(path)
}

/// Entry point of `bf roles <sub> ...`: `list`, `show <role>`, `token <unit> <role>`.
pub fn run_in(p: &Project, sub: &str, rest: &[String]) -> R<Value> {
    match sub {
        "list" => Ok(json!({"roles": ROLES.iter().map(describe).collect::<Vec<_>>()})),
        "show" => {
            let name = rest.first().map(|x| x.as_str()).unwrap_or("");
            match get(name) {
                Some(r) => Ok(describe(r)),
                None => Err(unknown(name)),
            }
        }
        "token" => {
            let unit = rest.first().map(|x| x.as_str()).unwrap_or("");
            let role = rest.get(1).map(|x| x.as_str()).unwrap_or("");
            check_subject(p, unit, role)?;
            let token = run::issue_token(p, &format!("{unit}:{role}"))?;
            Ok(json!({"unit": unit, "role": role, "token": token}))
        }
        _ => Err(refuse(
            format!("run `bf roles {sub}`"),
            "unknown subcommand",
            &["Use `bf roles list`, `bf roles show <role>` or `bf roles token <unit> <role>`."],
            "The command prints its result.",
        )),
    }
}

fn unknown(name: &str) -> Fail {
    refuse(
        format!("use the role `{name}`"),
        "that is not one of the new roles",
        &["Run `bf roles list` for the names."],
        "The command prints the role.",
    )
}
