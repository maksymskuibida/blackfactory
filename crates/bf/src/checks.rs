//! The check catalogue (C-1): the three families of named checks, each with its stage, how it is
//! proven and whether it is required by default. Compiled into the engine (D-16); every other part
//! reads the named checks from here.

use crate::refusal::{refuse, R};
use serde_json::{json, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Uiux,
    Security,
    Legal,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Uiux => "uiux",
            Family::Security => "security",
            Family::Legal => "legal",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Design,
    Built,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Design => "design",
            Stage::Built => "built",
        }
    }
}

/// How a named check is proven; the same words as the `how` field of a check record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proof {
    Command,
    Agent,
    Human,
}

impl Proof {
    pub fn name(self) -> &'static str {
        match self {
            Proof::Command => "command",
            Proof::Agent => "agent",
            Proof::Human => "human",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NamedCheck {
    pub family: Family,
    pub name: &'static str,
    pub stage: Stage,
    pub proof: Proof,
    pub required: bool,
}

const fn c(family: Family, name: &'static str, stage: Stage, proof: Proof) -> NamedCheck {
    NamedCheck {
        family,
        name,
        stage,
        proof,
        required: true,
    }
}

pub static CATALOGUE: &[NamedCheck] = &[
    c(
        Family::Uiux,
        "design-uiux-verification",
        Stage::Design,
        Proof::Agent,
    ),
    c(
        Family::Uiux,
        "built-uiux-verification",
        Stage::Built,
        Proof::Agent,
    ),
    c(Family::Uiux, "accessibility", Stage::Built, Proof::Command),
    c(
        Family::Security,
        "threat-model",
        Stage::Design,
        Proof::Agent,
    ),
    c(
        Family::Security,
        "security-audit",
        Stage::Built,
        Proof::Agent,
    ),
    c(
        Family::Security,
        "secret-scan",
        Stage::Built,
        Proof::Command,
    ),
    c(
        Family::Legal,
        "dependency-licence-check",
        Stage::Built,
        Proof::Command,
    ),
    c(
        Family::Legal,
        "markets-statement",
        Stage::Design,
        Proof::Human,
    ),
    c(Family::Legal, "legal-audit", Stage::Built, Proof::Agent),
];

/// Handoff H-1: every named check with its family, stage and default.
pub fn all() -> &'static [NamedCheck] {
    CATALOGUE
}

pub fn by_name(name: &str) -> Option<&'static NamedCheck> {
    CATALOGUE.iter().find(|x| x.name == name)
}

pub fn in_family(family: Family) -> Vec<&'static NamedCheck> {
    CATALOGUE.iter().filter(|x| x.family == family).collect()
}

/// The checks required on a project that sets nothing.
pub fn required_by_default() -> Vec<&'static NamedCheck> {
    CATALOGUE.iter().filter(|x| x.required).collect()
}

pub fn to_json(x: &NamedCheck) -> Value {
    json!({
        "family": x.family.name(),
        "name": x.name,
        "stage": x.stage.name(),
        "how": x.proof.name(),
        "required": x.required,
    })
}

/// Entry point of `bf checks`: `bf checks [list]`.
pub fn run(sub: &str, _rest: &[String]) -> R<Value> {
    match sub {
        "" | "list" => Ok(json!({"checks": all().iter().map(to_json).collect::<Vec<_>>()})),
        _ => Err(refuse(
            "run `bf checks`",
            format!("unknown subcommand `{sub}`"),
            &["Run `bf checks list`."],
            "The command prints the catalogue.",
        )),
    }
}

pub fn text(v: &Value) -> String {
    v["checks"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    format!(
                        "{:<9} {:<28} {:<7} {:<8} {}",
                        x["family"].as_str().unwrap_or(""),
                        x["name"].as_str().unwrap_or(""),
                        x["stage"].as_str().unwrap_or(""),
                        x["how"].as_str().unwrap_or(""),
                        if x["required"] == true {
                            "required"
                        } else {
                            "optional"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        for (i, a) in CATALOGUE.iter().enumerate() {
            assert!(CATALOGUE[i + 1..].iter().all(|b| b.name != a.name));
        }
    }
}
