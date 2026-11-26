//! The human files a finding of their own on a checkpoint, in their quoted words (R-81, D-169, D-170, TH-22).
//! The findings of one filing are one review round with the source `human`, written by one event
//! `checkpoint.finding.human`. This module holds the command, the pick of the message the words come from
//! (which reads and sets no `used` mark of a turn) and the judgment the replay gives an event of this kind.

use crate::ckreview;
use crate::events::{self, State};
use crate::refusal::{refuse_owned, Fail, R};
use crate::store::{s, Project};
use crate::strings::t;
use serde_json::{json, Value};
use std::collections::HashMap;

/// The source of the rounds a filing writes.
pub const SOURCE: &str = "human";

const SEVERITIES: [&str; 4] = ["critical", "high", "medium", "low"];

/// One finding of a filing as the human gave it: the words and the severity, when one was given.
pub type Item = (String, Option<String>);

fn refusal(k: &str, why: String, step: String) -> Fail {
    refuse_owned(
        t("ck_human.what", &[("k", k)]),
        why,
        vec![step],
        t("ck_human.check", &[]),
    )
}

/// The seq of the first `unit.start` of a unit of `k`; none while no unit of it started.
pub fn words_floor(st: &State, k: &str) -> Option<u64> {
    st.ck_first_start.get(k).copied()
}

fn holds_all(text: &str, quotes: &[String]) -> bool {
    quotes.iter().all(|q| text.contains(q.as_str()))
}

/// Whether the message `tseq` may be filed from: a message of the human after the floor, no finding filed
/// from it yet, holding every quote.
pub fn turn_valid(st: &State, k: &str, tseq: u64, quotes: &[String]) -> bool {
    let Some(floor) = words_floor(st, k) else {
        return false;
    };
    st.turns.iter().any(|x| {
        x.seq == tseq
            && x.seq > floor
            && !st.ck_human_filed.contains(&x.seq)
            && !crate::gates::recorded_text_is_host(&x.text)
            && holds_all(&x.text, quotes)
    })
}

/// The newest recorded message of the human after the floor, not filed from, whose text holds every quote.
/// It neither reads nor sets the `used` mark of a turn: a message a decision used may still file.
pub fn pick_words(st: &State, k: &str, quotes: &[String]) -> R<u64> {
    let Some(floor) = words_floor(st, k) else {
        return Err(refusal(
            k,
            t("ck_human.words.no_floor.why", &[("k", k)]),
            t("ck_human.words.no_floor.step", &[("k", k)]),
        ));
    };
    let human = |x: &&events::Turn| !crate::gates::recorded_text_is_host(&x.text);
    let fresh: Vec<&events::Turn> = st
        .turns
        .iter()
        .filter(human)
        .filter(|x| x.seq > floor && !st.ck_human_filed.contains(&x.seq))
        .collect();
    if let Some(x) = fresh.iter().rev().find(|x| holds_all(&x.text, quotes)) {
        return Ok(x.seq);
    }
    let floor_text = floor.to_string();
    if let Some(x) = st
        .turns
        .iter()
        .filter(human)
        .rev()
        .find(|x| x.seq > floor && holds_all(&x.text, quotes))
    {
        return Err(refusal(
            k,
            t("ck_human.words.filed.why", &[("turn", &x.seq.to_string())]),
            t("ck_human.words.filed.step", &[]),
        ));
    }
    if st
        .turns
        .iter()
        .filter(human)
        .any(|x| x.seq <= floor && holds_all(&x.text, quotes))
    {
        return Err(refusal(
            k,
            t(
                "ck_human.words.floor.why",
                &[("k", k), ("floor", &floor_text)],
            ),
            t("ck_human.words.floor.step", &[]),
        ));
    }
    match fresh.last() {
        None => Err(refusal(
            k,
            t(
                "ck_human.words.none.why",
                &[("k", k), ("floor", &floor_text)],
            ),
            t("ck_human.words.none.step", &[]),
        )),
        Some(last) => Err(refusal(
            k,
            t(
                "ck_human.words.absent.why",
                &[
                    ("k", k),
                    ("last", &last.text.chars().take(300).collect::<String>()),
                ],
            ),
            t("ck_human.words.absent.step", &[]),
        )),
    }
}

/// The first sentence of the words, cut at 120 characters.
pub fn first_sentence(words: &str) -> String {
    let chars: Vec<char> = words.trim().chars().collect();
    let mut end = chars.len();
    for (i, c) in chars.iter().enumerate() {
        let stop = match c {
            '\n' => true,
            '.' | '!' | '?' => chars.get(i + 1).is_none_or(|n| n.is_whitespace()),
            _ => false,
        };
        if stop {
            end = if *c == '\n' { i } else { i + 1 };
            break;
        }
    }
    chars[..end]
        .iter()
        .take(120)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The finding a quote becomes.
pub fn finding_of(quote: &str, severity: Option<&str>, turn: u64) -> Value {
    let mut f = json!({"title": first_sentence(quote), "severity": severity.unwrap_or("high"),
        "where": "", "severity_given": severity.is_some(), "by": "human", "turn": turn});
    f["action"] = json!(quote);
    f
}

/// `bf checkpoint finding <K> --human ...`: file the findings, each in the human's quoted words.
pub fn file(p: &Project, k: &str, items: &[Item], fix: bool, fix_quote: &str) -> R<Value> {
    let _lock = crate::lock::engine(p, &format!("file a finding of {k}"))?;
    let st = events::require_chain(p)?;
    let pl = crate::plan::load(p)?;
    if pl.checkpoint(k).is_none() {
        return Err(refusal(
            k,
            t("ck_human.not_checkpoint.why", &[("k", k)]),
            t("ck_human.not_checkpoint.step", &[]),
        ));
    }
    if let Some(c) = st.closed.get(k) {
        return Err(refusal(
            k,
            t(
                "ck_human.closed.why",
                &[("k", k), ("seq", &c.seq.to_string())],
            ),
            t("ck_human.closed.step", &[]),
        ));
    }
    if items.is_empty() || items.iter().any(|(q, _)| q.trim().is_empty()) {
        return Err(refusal(
            k,
            t("ck_human.empty.why", &[]),
            t("ck_human.empty.step", &[]),
        ));
    }
    for (_, sev) in items {
        if let Some(x) = sev {
            if !SEVERITIES.contains(&x.as_str()) {
                return Err(refusal(
                    k,
                    t("ck_human.severity.why", &[("severity", x)]),
                    t("ck_human.severity.step", &[]),
                ));
            }
        }
    }
    if fix == fix_quote.trim().is_empty() {
        return Err(refusal(
            k,
            t("ck_human.fix_quote.why", &[]),
            t("ck_human.fix_quote.step", &[]),
        ));
    }
    let quotes: Vec<String> = items.iter().map(|(q, _)| q.trim().to_string()).collect();
    let mut all = quotes.clone();
    if fix {
        all.push(fix_quote.trim().to_string());
    }
    let turn = pick_words(&st, k, &all)?;
    let findings: Vec<Value> = items
        .iter()
        .map(|(q, sev)| finding_of(q.trim(), sev.as_deref(), turn))
        .collect();
    let rs = ckreview::rounds(p, k)?;
    let round = rs.iter().map(|r| r.round).max().unwrap_or(0) + 1;
    let content = if p.work_dir().join(k).exists() {
        ckreview::cp_content(p, k).unwrap_or_default()
    } else {
        String::new()
    };
    events::append(
        p,
        "checkpoint.finding.human",
        json!({"checkpoint": k, "round": round, "content": content, "findings": findings,
               "fix": fix, "fix_quote": fix_quote.trim(), "turn": turn, "quotes": quotes}),
    )?;
    p.write_guard()?;
    let ids: Vec<String> = (1..=findings.len())
        .map(|i| format!("r{round}.{i}"))
        .collect();
    Ok(json!({"checkpoint": k, "round": round, "ids": ids, "filed": findings.len(), "fix": fix}))
}

/// The line the command prints.
pub fn done_line(k: &str, v: &Value) -> String {
    let ids: Vec<&str> = v["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect();
    t(
        if v["fix"] == true {
            "ck_human.done.fix"
        } else {
            "ck_human.done"
        },
        &[
            ("k", k),
            ("n", &v["filed"].to_string()),
            ("round", &v["round"].to_string()),
            ("ids", &ids.join(", ")),
        ],
    )
}

/// The judgment of the replay on an event `checkpoint.finding.human` at `seq`: the cause it is ignored for,
/// or the message it files from.
pub fn judge(
    st: &State,
    rounds: &HashMap<(String, u32), Vec<Value>>,
    ev: &Value,
    seq: u64,
) -> Result<u64, String> {
    let k = s(ev, "checkpoint");
    let n = ev["round"].as_u64().unwrap_or(0) as u32;
    let sq = seq.to_string();
    if st.closed.contains_key(k) {
        return Err(t("ck_human.ignored.closed", &[("seq", &sq), ("k", k)]));
    }
    if n == 0 || st.ck_round_source.contains_key(&(k.to_string(), n)) {
        return Err(t(
            "ck_human.ignored.round",
            &[("seq", &sq), ("k", k), ("round", &n.to_string())],
        ));
    }
    let findings = ev["findings"].as_array().cloned().unwrap_or_default();
    let quotes = crate::store::str_list(ev, "quotes");
    let tseq = ev["turn"].as_u64().unwrap_or(0);
    let existing = rounds
        .iter()
        .filter(|((c, _), _)| c == k)
        .any(|(_, l)| findings.iter().any(|f| l.contains(f)));
    if existing {
        return Err(t("ck_human.ignored.existing", &[("seq", &sq), ("k", k)]));
    }
    let mut all = quotes.clone();
    if ev["fix"].as_bool().unwrap_or(false) {
        all.push(s(ev, "fix_quote").to_string());
    }
    if !turn_valid(st, k, tseq, &all) {
        return Err(t("ck_human.ignored.turn", &[("seq", &sq), ("k", k)]));
    }
    let shaped = !findings.is_empty()
        && findings.iter().all(|f| {
            s(f, "by") == "human"
                && f["turn"].as_u64() == Some(tseq)
                && quotes.iter().any(|q| q == s(f, "action"))
        });
    if !shaped {
        return Err(t("ck_human.ignored.shape", &[("seq", &sq)]));
    }
    Ok(tseq)
}
