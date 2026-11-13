//! Record kinds, ID prefixes and field validation.

use crate::refusal::{refuse_owned, R};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Str,
    Int,
    Bool,
    StrList,
    /// A list of IDs; the string names the allowed prefixes ("" = any designed kind).
    Ids(&'static str),
    /// One ID with the allowed prefix.
    Id(&'static str),
    Tables,
    Any,
}

pub struct Field {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub allowed: &'static [&'static str],
}

const fn f(name: &'static str, ty: Ty, required: bool) -> Field {
    Field {
        name,
        ty,
        required,
        allowed: &[],
    }
}
const fn fe(name: &'static str, ty: Ty, required: bool, allowed: &'static [&'static str]) -> Field {
    Field {
        name,
        ty,
        required,
        allowed,
    }
}

pub const LABELS: &[&str] = &["critical", "high", "medium", "low", "core", "main", "minor"];

pub struct Kind {
    pub name: &'static str,
    pub prefix: &'static str,
    pub designed: bool,
    pub fields: &'static [Field],
}

macro_rules! design_fields {
    ($($extra:expr),*) => {
        &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("title", Ty::Str, true),
            f("text", Ty::Str, false),
            fe("label", Ty::Str, false, LABELS),
            f("links", Ty::Ids(""), false),
            $($extra),*
        ]
    };
}

pub static KINDS: &[Kind] = &[
    Kind {
        name: "requirement",
        prefix: "R",
        designed: true,
        fields: design_fields!(f("acceptance", Ty::StrList, false)),
    },
    Kind {
        name: "component",
        prefix: "C",
        designed: true,
        fields: design_fields!(),
    },
    Kind {
        name: "screen",
        prefix: "S",
        designed: true,
        fields: design_fields!(),
    },
    Kind {
        name: "journey",
        prefix: "J",
        designed: true,
        fields: design_fields!(),
    },
    Kind {
        name: "target",
        prefix: "T",
        designed: true,
        fields: design_fields!(f("measure", Ty::Str, false)),
    },
    Kind {
        name: "threat",
        prefix: "TH",
        designed: true,
        fields: design_fields!(f("mitigation", Ty::Str, false)),
    },
    Kind {
        name: "decision",
        prefix: "D",
        designed: true,
        fields: design_fields!(f("chosen", Ty::Str, false), f("rejected", Ty::Str, false)),
    },
    Kind {
        name: "idea",
        prefix: "I",
        designed: false,
        fields: &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("title", Ty::Str, true),
            f("text", Ty::Str, false),
            f("links", Ty::Ids(""), false),
        ],
    },
    Kind {
        name: "unit",
        prefix: "U",
        designed: false,
        fields: &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("title", Ty::Str, true),
            f("text", Ty::Str, false),
            f("delivers", Ty::Ids(""), true),
            f("depends_on", Ty::Ids("U"), false),
            f("files", Ty::StrList, true),
            f("checkpoint", Ty::Id("K"), true),
            f("estimate", Ty::Int, false),
            fe("tier", Ty::Str, false, &["cheap", "standard", "strong"]),
        ],
    },
    Kind {
        name: "handoff",
        prefix: "H",
        designed: false,
        fields: &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("from", Ty::Id("U"), true),
            f("to", Ty::Id("U"), true),
            f("thing", Ty::Str, true),
            f("contract", Ty::Str, false),
        ],
    },
    Kind {
        name: "checkpoint",
        prefix: "K",
        designed: false,
        fields: &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("title", Ty::Str, true),
            f("text", Ty::Str, false),
            f("after", Ty::Ids("U"), true),
            f("proves", Ty::Ids("+H"), true),
            f("checks", Ty::Ids("X"), false),
            f("human_step", Ty::Str, false),
            fe("on_human_step", Ty::Str, false, &["stop", "carry-on"]),
            f("pull_request", Ty::Bool, false),
        ],
    },
    Kind {
        name: "check",
        prefix: "X",
        designed: false,
        fields: &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("title", Ty::Str, true),
            f("text", Ty::Str, false),
            fe("how", Ty::Str, true, &["command", "agent", "human"]),
            f("command", Ty::Any, false),
            f("proves", Ty::Ids("+H"), true),
            fe("scope", Ty::Str, true, &["unit", "checkpoint"]),
            f("unit", Ty::Id("U"), false),
        ],
    },
    Kind {
        name: "finding",
        prefix: "F",
        designed: false,
        fields: &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("title", Ty::Str, true),
            f("text", Ty::Str, false),
            f("unit", Ty::Id("U"), true),
            fe(
                "severity",
                Ty::Str,
                false,
                &["critical", "high", "medium", "low"],
            ),
            f("where", Ty::Str, false),
            f("action", Ty::Str, false),
            fe("outcome", Ty::Str, false, &["open", "fixed", "accepted"]),
        ],
    },
    Kind {
        name: "question",
        prefix: "Q",
        designed: false,
        fields: &[
            f("id", Ty::Str, true),
            f("kind", Ty::Str, true),
            f("title", Ty::Str, true),
            f("context", Ty::Str, false),
            fe("answer_kind", Ty::Str, true, &["single", "multi", "text"]),
            f("options", Ty::Tables, false),
            f("stage", Ty::Str, false),
            f("answer", Ty::Any, false),
            f("answered_at", Ty::Str, false),
            f("answered_turn", Ty::Int, false),
        ],
    },
];

pub fn kind_by_name(name: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.name == name)
}

pub fn kind_by_prefix(prefix: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.prefix == prefix)
}

/// Split "TH-12" into ("TH", 12). None when the shape is wrong.
pub fn parse_id(id: &str) -> Option<(&str, u32)> {
    let (p, n) = id.split_once('-')?;
    if p.is_empty() || !p.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    if n.is_empty()
        || n.len() > 9
        || !n.chars().all(|c| c.is_ascii_digit())
        || (n.len() > 1 && n.starts_with('0'))
    {
        return None;
    }
    Some((p, n.parse().ok()?))
}

pub fn kind_of_id(id: &str) -> Option<&'static Kind> {
    parse_id(id).and_then(|(p, _)| kind_by_prefix(p))
}

pub fn is_designed_id(id: &str) -> bool {
    kind_of_id(id).map(|k| k.designed).unwrap_or(false)
}

fn id_ok(id: &str, allowed: &str) -> bool {
    match parse_id(id) {
        None => false,
        Some((p, _)) => {
            if allowed.is_empty() {
                kind_by_prefix(p).map(|k| k.designed).unwrap_or(false)
            } else if allowed == "+H" {
                // Designed parts, and the handoffs between units.
                p == "H" || kind_by_prefix(p).map(|k| k.designed).unwrap_or(false)
            } else {
                p == allowed
            }
        }
    }
}

/// Validate a record against its kind. Collects every problem.
pub fn validate(kind: &Kind, rec: &Value) -> Vec<String> {
    let mut bad = Vec::new();
    let Some(obj) = rec.as_object() else {
        return vec!["the record is not a table".into()];
    };
    for fld in kind.fields {
        match obj.get(fld.name) {
            None => {
                if fld.required {
                    bad.push(format!("missing required field `{}`", fld.name));
                }
            }
            Some(v) => check_field(fld, v, &mut bad),
        }
    }
    for k in obj.keys() {
        if !kind.fields.iter().any(|fl| fl.name == k) {
            bad.push(format!("unknown field `{k}` for a {}", kind.name));
        }
    }
    if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
        match parse_id(id) {
            Some((p, _)) if p == kind.prefix => {}
            _ => bad.push(format!("id `{id}` must look like {}-<number>", kind.prefix)),
        }
    }
    if let Some(k) = obj.get("kind").and_then(|v| v.as_str()) {
        if k != kind.name {
            bad.push(format!(
                "`kind` is `{k}` but the record is put as `{}`",
                kind.name
            ));
        }
    }
    if kind.name == "check" {
        let how = obj.get("how").and_then(|v| v.as_str()).unwrap_or("");
        if how == "command" && !obj.contains_key("command") {
            bad.push("a check with how = \"command\" needs a `command`".into());
        }
        if obj.get("scope").and_then(|v| v.as_str()) == Some("unit") && !obj.contains_key("unit") {
            bad.push("a check with scope = \"unit\" needs a `unit`".into());
        }
    }
    if kind.name == "unit" {
        if let Some(a) = obj.get("delivers").and_then(|v| v.as_array()) {
            if a.is_empty() {
                bad.push("`delivers` must name at least one ID".into());
            }
        }
    }
    bad
}

fn check_field(fld: &Field, v: &Value, bad: &mut Vec<String>) {
    let n = fld.name;
    match fld.ty {
        Ty::Str => match v.as_str() {
            Some(s) if !s.trim().is_empty() => {
                if !fld.allowed.is_empty() && !fld.allowed.contains(&s) {
                    bad.push(format!("`{n}` must be one of {}", fld.allowed.join(", ")));
                }
            }
            _ => bad.push(format!("`{n}` must be a non-empty string")),
        },
        Ty::Int => {
            if v.as_i64().is_none() {
                bad.push(format!("`{n}` must be an integer"));
            }
        }
        Ty::Bool => {
            if !v.is_boolean() {
                bad.push(format!("`{n}` must be true or false"));
            }
        }
        Ty::StrList => {
            let ok = v
                .as_array()
                .map(|a| a.iter().all(|x| x.is_string()))
                .unwrap_or(false);
            if !ok {
                bad.push(format!("`{n}` must be a list of strings"));
            }
        }
        Ty::Ids(p) => match v.as_array() {
            Some(a) => {
                for x in a {
                    match x.as_str() {
                        Some(s) if id_ok(s, p) => {}
                        _ => bad.push(format!(
                            "`{n}` holds {} which is not a valid {} ID",
                            x,
                            if p.is_empty() {
                                "designed-part".to_string()
                            } else if p == "+H" {
                                "designed-part or H-<number>".to_string()
                            } else {
                                format!("{p}-<number>")
                            }
                        )),
                    }
                }
            }
            None => bad.push(format!("`{n}` must be a list of IDs")),
        },
        Ty::Id(p) => match v.as_str() {
            Some(s) if id_ok(s, p) => {}
            _ => bad.push(format!("`{n}` must be one {p}-<number> ID")),
        },
        Ty::Tables => {
            let ok = v
                .as_array()
                .map(|a| a.iter().all(|x| x.is_object()))
                .unwrap_or(false);
            if !ok {
                bad.push(format!("`{n}` must be a list of tables"));
            }
        }
        Ty::Any => {}
    }
}

/// Validate or return a refusal naming every problem.
pub fn validate_or_refuse(kind: &Kind, rec: &Value) -> R<()> {
    let bad = validate(kind, rec);
    if bad.is_empty() {
        return Ok(());
    }
    let mut steps: Vec<String> = bad.iter().map(|b| format!("Fix: {b}.")).collect();
    steps.push("Run the same `bf record put` command again.".into());
    Err(refuse_owned(
        format!("record put {}", kind.name),
        format!(
            "the record does not match the {} schema ({} problem(s))",
            kind.name,
            bad.len()
        ),
        steps,
        "`bf record get <id>` prints the record back.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids() {
        assert_eq!(parse_id("TH-12"), Some(("TH", 12)));
        assert_eq!(parse_id("R-1"), Some(("R", 1)));
        assert_eq!(parse_id("r-1"), None);
        assert_eq!(parse_id("R-"), None);
        assert_eq!(parse_id("R-1x"), None);
        assert_eq!(kind_of_id("U-3").unwrap().name, "unit");
        assert!(is_designed_id("D-1") && !is_designed_id("U-1"));
    }

    #[test]
    fn requirement_validates() {
        let k = kind_by_name("requirement").unwrap();
        let ok = json!({"id":"R-1","kind":"requirement","title":"Does a thing","acceptance":["a"]});
        assert!(validate(k, &ok).is_empty());
        let bad = json!({"id":"U-1","kind":"requirement","title":"","extra":1});
        let p = validate(k, &bad);
        assert!(p.len() >= 3, "{p:?}");
    }

    #[test]
    fn unit_needs_scope_and_delivery() {
        let k = kind_by_name("unit").unwrap();
        let bad = json!({"id":"U-1","kind":"unit","title":"x","delivers":[],"files":["a"],"checkpoint":"K-1"});
        assert!(validate(k, &bad).iter().any(|p| p.contains("delivers")));
        let bad2 = json!({"id":"U-1","kind":"unit","title":"x","delivers":["U-2"],"files":["a"],"checkpoint":"K-1"});
        assert!(!validate(k, &bad2).is_empty());
    }

    #[test]
    fn check_rules() {
        let k = kind_by_name("check").unwrap();
        let bad = json!({"id":"X-1","kind":"check","title":"t","how":"command","proves":["R-1"],"scope":"unit"});
        let p = validate(k, &bad);
        assert!(
            p.iter().any(|x| x.contains("command")) && p.iter().any(|x| x.contains("unit")),
            "{p:?}"
        );
    }
}
