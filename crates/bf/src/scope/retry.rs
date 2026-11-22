//! A-19: the rules a unit's and a checkpoint fix's scope review share when the reviewer answers a
//! file with `retry`, the words a corrected patch must meet. A refusal is `{seq, refusal, files}` as
//! the replay keeps it; each file carries `path` and its own `verdict`: `needed`, `retry`,
//! `retry_spent` or `extension`. One file is judged at most `limit` times for one unit or one fix.

use crate::store::{s, Project};
use serde_json::Value;

/// The policy's review rounds: how many times one file may be judged.
pub fn limit(p: &Project) -> u32 {
    p.policy.limits.max_rounds
}

fn counted(r: &Value) -> bool {
    matches!(s(r, "refusal"), "retry" | "retry_spent")
}

fn file_of<'a>(r: &'a Value, path: &str) -> Option<&'a Value> {
    r["files"].as_array()?.iter().find(|f| s(f, "path") == path)
}

fn is_retry(f: &Value) -> bool {
    matches!(s(f, "verdict"), "retry" | "retry_spent")
}

/// How many rounds of this file the reviewer has used.
pub fn rounds_used(refusals: &[Value], path: &str) -> u32 {
    refusals
        .iter()
        .filter(|r| counted(r) && file_of(r, path).is_some_and(is_retry))
        .count() as u32
}

/// Whether the file may not be asked for again.
pub fn spent(refusals: &[Value], path: &str, limit: u32) -> bool {
    refusals
        .iter()
        .any(|r| file_of(r, path).is_some_and(|f| s(f, "verdict") == "retry_spent"))
        || rounds_used(refusals, path) >= limit
}

/// The word of a `retry` answer for this file: the last allowed judgment is `retry_spent`.
pub fn verdict_word(refusals: &[Value], path: &str, limit: u32) -> &'static str {
    if rounds_used(refusals, path) + 1 >= limit {
        "retry_spent"
    } else {
        "retry"
    }
}

/// The patch of the latest counted refusal in which the file's verdict is `retry`.
pub fn last_patch(refusals: &[Value], path: &str) -> Option<String> {
    refusals
        .iter()
        .rev()
        .filter(|r| counted(r))
        .filter_map(|r| file_of(r, path))
        .find(|f| s(f, "verdict") == "retry")
        .map(|f| s(f, "patch").to_string())
}

/// Whether the file's latest counted judgment was a `retry` (not a `needed`).
pub fn latest_is_retry(refusals: &[Value], path: &str) -> bool {
    refusals
        .iter()
        .rev()
        .filter(|r| counted(r))
        .find_map(|r| file_of(r, path))
        .is_some_and(|f| s(f, "verdict") == "retry")
}

/// The refusals written after a seq.
pub fn since(refusals: &[Value], from_seq: u64) -> Vec<Value> {
    refusals
        .iter()
        .filter(|r| r["seq"].as_u64().unwrap_or(0) > from_seq)
        .cloned()
        .collect()
}

/// One answer of the reviewer: `needed`, `extension` or `retry` with its `must`.
pub fn check_verdict_item(v: &Value) -> Result<(), String> {
    match s(v, "verdict") {
        "needed" | "extension" => Ok(()),
        "retry" => {
            if s(v, "must").trim().is_empty() {
                Err(crate::strings::t("scope.retry.verdict.must", &[]))
            } else {
                Ok(())
            }
        }
        _ => Err(crate::strings::t("scope.retry.verdict.word", &[])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(seq: u64, refusal: &str, verdict: &str) -> Value {
        json!({"seq": seq, "refusal": refusal, "files": [{"path": "a", "verdict": verdict, "patch": format!("p{seq}")}]})
    }

    #[test]
    fn a_file_is_judged_exactly_the_limit_of_times() {
        let mut v = Vec::new();
        assert_eq!(verdict_word(&v, "a", 3), "retry");
        v.push(r(1, "retry", "retry"));
        assert_eq!(verdict_word(&v, "a", 3), "retry");
        v.push(r(2, "retry", "retry"));
        assert_eq!(verdict_word(&v, "a", 3), "retry_spent");
        assert!(!spent(&v, "a", 3));
        v.push(r(3, "retry_spent", "retry_spent"));
        assert!(spent(&v, "a", 3));
        assert_eq!(rounds_used(&v, "a"), 3);
        assert_eq!(since(&v, 2).len(), 1);
    }

    #[test]
    fn an_extension_refusal_counts_no_round() {
        let v = vec![r(1, "extension", "retry"), r(2, "retry", "needed")];
        assert_eq!(rounds_used(&v, "a"), 0);
        assert_eq!(last_patch(&v, "a"), None);
    }
}
