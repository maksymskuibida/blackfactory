//! One shared refusal type. Every refusal has four parts: what was refused,
//! why, numbered steps to fix it, and how to tell it worked.

use serde_json::{json, Value};
use std::fmt;

#[derive(Debug, Clone)]
pub struct Refusal {
    pub refused: String,
    pub because: String,
    pub do_this: Vec<String>,
    pub worked_when: String,
}

#[derive(Debug)]
pub enum Fail {
    /// Exit code 2: the engine understood the request and said no.
    Refused(Refusal),
    /// Exit code 1: something broke.
    Error(String),
}

pub type R<T> = Result<T, Fail>;

impl Refusal {
    pub fn to_json(&self) -> Value {
        json!({
            "ok": false,
            "refused": self.refused,
            "because": self.because,
            "do_this": self.do_this,
            "it_worked_when": self.worked_when,
        })
    }

    /// Plain-text form, the same four parts in the same order.
    pub fn text(&self) -> String {
        let mut s = format!(
            "refused: {}\nbecause: {}\ndo this:\n",
            self.refused, self.because
        );
        for (i, step) in self.do_this.iter().enumerate() {
            s.push_str(&format!("  {}. {}\n", i + 1, step));
        }
        s.push_str(&format!("it worked when: {}\n", self.worked_when));
        s
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Refused(r) => write!(f, "{r}"),
            Fail::Error(e) => write!(f, "error: {e}"),
        }
    }
}

/// Build a refusal. All four parts are required; empty parts are a bug.
pub fn refuse(
    refused: impl Into<String>,
    because: impl Into<String>,
    steps: &[&str],
    worked_when: impl Into<String>,
) -> Fail {
    let r = Refusal {
        refused: refused.into(),
        because: because.into(),
        do_this: steps.iter().map(|s| s.to_string()).collect(),
        worked_when: worked_when.into(),
    };
    debug_assert!(!r.refused.is_empty() && !r.because.is_empty() && !r.worked_when.is_empty());
    debug_assert!(!r.do_this.is_empty());
    Fail::Refused(r)
}

pub fn refuse_owned(
    refused: impl Into<String>,
    because: impl Into<String>,
    steps: Vec<String>,
    worked_when: impl Into<String>,
) -> Fail {
    let r = Refusal {
        refused: refused.into(),
        because: because.into(),
        do_this: steps,
        worked_when: worked_when.into(),
    };
    debug_assert!(!r.do_this.is_empty());
    Fail::Refused(r)
}

pub fn err(msg: impl Into<String>) -> Fail {
    Fail::Error(msg.into())
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Self {
        Fail::Error(format!("io: {e}"))
    }
}
impl From<serde_json::Error> for Fail {
    fn from(e: serde_json::Error) -> Self {
        Fail::Error(format!("json: {e}"))
    }
}
impl From<toml::de::Error> for Fail {
    fn from(e: toml::de::Error) -> Self {
        Fail::Error(format!("toml: {e}"))
    }
}
impl From<toml::ser::Error> for Fail {
    fn from(e: toml::ser::Error) -> Self {
        Fail::Error(format!("toml: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_parts_in_text_and_json() {
        let Fail::Refused(r) = refuse(
            "a thing",
            "a reason",
            &["step one", "step two"],
            "it passes",
        ) else {
            panic!()
        };
        let t = r.text();
        for part in [
            "refused: ",
            "because: ",
            "do this:",
            "  1. step one",
            "  2. step two",
            "it worked when: ",
        ] {
            assert!(t.contains(part), "{part}");
        }
        let j = r.to_json();
        for k in ["refused", "because", "do_this", "it_worked_when"] {
            assert!(j.get(k).is_some());
        }
    }
}
