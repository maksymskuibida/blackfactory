//! The human files a finding of their own on a checkpoint, in their quoted words (R-81, D-169, D-170, TH-22).
//! The findings of one filing are one review round with the source `human`, written by one event
//! `checkpoint.finding.human`. This module holds the command, the pick of the message the words come from
//! (which reads and sets no `used` mark of a turn) and the judgment the replay gives an event of this kind.

use crate::ckreview;
use crate::events::{self, CkFixOpen, State};
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

/// `bf checkpoint finding <K> --human ...`: file the findings, each in the human's quoted words. Filed with
/// a fix, they join the open fix of the checkpoint when it waits for them, or open one at once when nothing
/// holds it (R-81).
pub fn file(p: &Project, k: &str, items: &[Item], fix: bool, fix_quote: &str) -> R<Value> {
    let (v, filing) = file_locked(p, k, items, fix, fix_quote)?;
    if fix {
        let ids: Vec<(u32, u32)> = (1..=v["filed"].as_u64().unwrap_or(0) as u32)
            .map(|i| (v["round"].as_u64().unwrap_or(0) as u32, i))
            .collect();
        join_waiting(p, k, &ids, filing)?;
        crate::ckfix::maybe_open(p)?;
    }
    Ok(v)
}

fn file_locked(
    p: &Project,
    k: &str,
    items: &[Item],
    fix: bool,
    fix_quote: &str,
) -> R<(Value, u64)> {
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
    let written = events::append(
        p,
        "checkpoint.finding.human",
        json!({"checkpoint": k, "round": round, "content": content, "findings": findings,
               "fix": fix, "fix_quote": fix_quote.trim(), "turn": turn, "quotes": quotes}),
    )?;
    p.write_guard()?;
    let ids: Vec<String> = (1..=findings.len())
        .map(|i| format!("r{round}.{i}"))
        .collect();
    Ok((
        json!({"checkpoint": k, "round": round, "ids": ids, "filed": findings.len(), "fix": fix}),
        written["seq"].as_u64().unwrap_or(0),
    ))
}

/// The human findings of `k` that wait for a fix, as the replay keeps them.
pub fn waiting(st: &State, k: &str) -> Vec<(u32, u32)> {
    st.ck_human_wait.get(k).cloned().unwrap_or_default()
}

/// Findings with the (round, number) of each.
pub type Waiting = (Vec<Value>, Vec<(u32, u32)>);

/// The waiting human findings of `k` with their values, as the review rounds hold them.
pub fn waiting_findings(p: &Project, st: &State, k: &str) -> R<Waiting> {
    let rs = ckreview::rounds(p, k)?;
    let (mut findings, mut ids) = (Vec::new(), Vec::new());
    for (r, i) in waiting(st, k) {
        let found = rs
            .iter()
            .find(|x| x.round == r && ckreview::is_human(&x.source))
            .and_then(|x| x.findings.get((i as usize).wrapping_sub(1)));
        if let Some(f) = found.filter(|f| ckreview::fate(st, k, r, i as usize - 1, f).is_none()) {
            findings.push(f.clone());
            ids.push((r, i));
        }
    }
    Ok((findings, ids))
}

/// Why the human findings of `k` that wait for a fix wait, as the key of the reason (`round`, `fix_round`, `sync`,
/// `other_fix`, `units`, `run`, `plan_gate`, `budget`, `check`); none when nothing holds them and the next
/// `bf next` opens their fix. `bf next` and the page both read it.
pub fn waits_for(
    p: &Project,
    st: &State,
    pl: &crate::plan::Plan,
    k: &str,
) -> R<Option<&'static str>> {
    if waiting(st, k).is_empty() {
        return Ok(None);
    }
    if st.active_run().is_none() {
        return Ok(Some("run"));
    }
    if !st.gate_passed("plan", None) {
        return Ok(Some("plan_gate"));
    }
    if p.policy.budget.cap > 0.0 && st.cost >= p.policy.budget.cap {
        return Ok(Some("budget"));
    }
    if st.sync_open.is_some() {
        return Ok(Some("sync"));
    }
    if let Some(f) = &st.ck_fix {
        return Ok(Some(if f.checkpoint == k {
            "fix_round"
        } else {
            "other_fix"
        }));
    }
    let after = pl
        .checkpoint(k)
        .map(|c| crate::store::str_list(c, "after"))
        .unwrap_or_default();
    if after.is_empty() || !after.iter().all(|u| st.unit_state(u) == "landed") {
        return Ok(Some("units"));
    }
    let rs = ckreview::rounds(p, k)?;
    if ckreview::open_round(&rs, "").is_some() || crate::ckaudit::round_open(p, k)? {
        return Ok(Some("round"));
    }
    if let Some(c) = pl.checkpoint(k) {
        let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
        let receipts = crate::evidence::load_receipts(p)?;
        if crate::ckfix::failing_checks(pl, st, &receipts, c, &content).1 {
            return Ok(Some("check"));
        }
    }
    Ok(None)
}

/// The reason as a human reads it.
pub fn waits_text(key: &str, k: &str) -> String {
    match key {
        "round" => t("human_fix.waits.round", &[("k", k)]),
        "fix_round" => t("human_fix.waits.fix_round", &[("k", k)]),
        "sync" => t("human_fix.waits.sync", &[("k", k)]),
        "other_fix" => t("human_fix.waits.other_fix", &[("k", k)]),
        "units" => t("human_fix.waits.units", &[("k", k)]),
        "run" => t("human_fix.waits.run", &[("k", k)]),
        "plan_gate" => t("human_fix.waits.plan_gate", &[("k", k)]),
        "budget" => t("human_fix.waits.budget", &[("k", k)]),
        _ => t("human_fix.waits.check", &[("k", k)]),
    }
}

/// The information entries of `bf next`: for each checkpoint with human findings that wait for a fix, what
/// each waits for.
pub fn next_entries(p: &Project, st: &State, pl: &crate::plan::Plan) -> R<Vec<Value>> {
    let mut out = Vec::new();
    for c in &pl.checkpoints {
        let k = s(c, "id");
        let wait = waiting(st, k);
        if wait.is_empty() || st.closed.contains_key(k) {
            continue;
        }
        let Some(why) = waits_for(p, st, pl, k)? else {
            continue;
        };
        let reason = waits_text(why, k);
        let rounds = ckreview::rounds(p, k)?;
        let list: Vec<Value> = wait
            .iter()
            .map(|(r, i)| {
                let title = rounds
                    .iter()
                    .find(|x| x.round == *r)
                    .and_then(|x| x.findings.get(*i as usize - 1))
                    .map(|f| s(f, "title").to_string())
                    .unwrap_or_default();
                json!({"id": format!("r{r}.{i}"), "title": title, "waits_for": why, "waits_text": reason})
            })
            .collect();
        let ids: Vec<String> = wait.iter().map(|(r, i)| format!("r{r}.{i}")).collect();
        out.push(json!({"role": "human", "action": "see-waiting-human-findings", "info": true,
            "checkpoint": k, "findings": list,
            "note": t("human_fix.next.note", &[("k", k), ("ids", &ids.join(", ")), ("reason", &reason)])}));
    }
    Ok(out)
}

fn pairs_json(ids: &[(u32, u32)]) -> Value {
    json!(ids
        .iter()
        .map(|(r, i)| json!({"round": r, "index": i}))
        .collect::<Vec<_>>())
}

/// The filing a round of human findings was written by, as the replay counted it.
fn filing_seq(st: &State, k: &str, round: u32) -> u64 {
    st.ck_round_done
        .iter()
        .rev()
        .find(|(c, r, _)| c == k && *r == round)
        .map(|x| x.2)
        .unwrap_or(0)
}

/// A filing with a fix while a fix of the checkpoint waits for the human (`low`, `parked`) or its design is
/// being made: the waiting findings are added to it at once (R-81). A fix at work takes them at its next round.
fn join_waiting(p: &Project, k: &str, ids: &[(u32, u32)], filing: u64) -> R<()> {
    let _lock = crate::lock::engine(p, &format!("add the human findings to the fix of {k}"))?;
    let st = events::require_chain(p)?;
    let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == k) else {
        return Ok(());
    };
    let step = if f.phase == "low" {
        "low"
    } else if f.phase == "parked" {
        "parked"
    } else if crate::ckfix::in_design(f) {
        "design"
    } else {
        return Ok(());
    };
    let wait = waiting(&st, k);
    let ids: Vec<(u32, u32)> = ids.iter().copied().filter(|x| wait.contains(x)).collect();
    if ids.is_empty() {
        return Ok(());
    }
    events::append(
        p,
        "checkpoint.fix.add",
        json!({"checkpoint": k, "human": true, "step": step, "finding_ids": pairs_json(&ids), "filing": filing}),
    )?;
    crate::ckfix::write_overlay(p)?;
    p.write_guard()?;
    Ok(())
}

/// The fix of `k` at work had its review reported: the waiting human findings join its next round (`review`),
/// or stay with it parked at its round limit (`park`). The caller holds the engine lock. Whether any joined.
pub(crate) fn join_after_review(p: &Project, k: &str, step: &str) -> R<bool> {
    let st = events::require_chain(p)?;
    let ids = waiting_findings(p, &st, k)?.1;
    let Some(first) = ids.first() else {
        return Ok(false);
    };
    events::append(
        p,
        "checkpoint.fix.add",
        json!({"checkpoint": k, "human": true, "step": step, "finding_ids": pairs_json(&ids),
               "filing": filing_seq(&st, k, first.0)}),
    )?;
    crate::ckfix::write_overlay(p)?;
    p.write_guard()?;
    Ok(true)
}

/// The fix of `k` has just gone from `gap` back to `fix`: the waiting human findings join it (step `gap`).
/// The caller holds the engine lock. Whether any joined.
#[allow(dead_code)]
pub(crate) fn join_after_gap(p: &Project, k: &str) -> R<bool> {
    join_after_review(p, k, "gap")
}

/// The judgment of the replay on a `checkpoint.fix.add` with `human = true`: the waiting human findings join
/// the open fix only at the steps the engine writes them at, and any other add is ignored with its cause (TH-22).
pub fn replay_add(
    st: &mut State,
    rounds: &HashMap<(String, u32), Vec<Value>>,
    ev: &Value,
    seq: u64,
    prev: Option<&Value>,
) -> Result<(), String> {
    let k = s(ev, "checkpoint").to_string();
    let step = s(ev, "step").to_string();
    let sq = seq.to_string();
    let ignored = |key: &str| t(key, &[("seq", &sq), ("k", &k), ("step", &step)]);
    let pairs = events::id_pairs(&ev["finding_ids"]);
    let wait = waiting(st, &k);
    if pairs.is_empty() || pairs.iter().any(|x| !wait.contains(x)) {
        return Err(ignored("human_fix.ignored.add.none"));
    }
    let Some(f) = st.ck_fix.as_ref().filter(|f| f.checkpoint == k) else {
        return Err(ignored("human_fix.ignored.add.nofix"));
    };
    let reviewed = prev.filter(|e| {
        e["kind"].as_str() == Some("checkpoint.fix.reviewed") && s(e, "checkpoint") == k
    });
    let fits = match step.as_str() {
        "low" => f.phase == "low",
        "parked" => f.phase == "parked",
        "design" => crate::ckfix::in_design(f),
        "review" => reviewed.is_some_and(|e| {
            s(e, "to") != "parked"
                && if s(e, "result") == "clean" {
                    f.phase == "review"
                } else {
                    matches!(f.phase.as_str(), "fix" | "low")
                }
        }),
        "park" => reviewed.is_some_and(|e| s(e, "to") == "parked") && f.phase == "parked",
        "gap" => {
            f.phase == "fix"
                && f.gap_left != 0
                && prev.is_some_and(|e| e["seq"].as_u64() == Some(f.gap_left))
        }
        _ => false,
    };
    if !fits {
        return Err(ignored("human_fix.ignored.add.step"));
    }
    let mut items: Vec<(String, Value)> = Vec::new();
    for (r, i) in &pairs {
        match rounds
            .get(&(k.clone(), *r))
            .and_then(|l| l.get((*i as usize).wrapping_sub(1)))
        {
            Some(v)
                if st.ck_round_source.get(&(k.clone(), *r)).map(|x| x.as_str()) == Some(SOURCE)
                    && ckreview::fate(st, &k, *r, *i as usize - 1, v).is_none() =>
            {
                items.push((format!("r{r}.{i}"), v.clone()))
            }
            _ => return Err(ignored("human_fix.ignored.add.none")),
        }
    }
    let clean = reviewed.is_some_and(|e| s(e, "result") == "clean");
    if let Some(w) = st.ck_human_wait.get_mut(&k) {
        w.retain(|x| !pairs.contains(x));
    }
    if let Some(o) = st.ck_fix.as_mut() {
        join(o, &step, clean, items);
    }
    Ok(())
}

fn join(o: &mut CkFixOpen, step: &str, clean: bool, items: Vec<(String, Value)>) {
    if clean {
        o.findings.clear();
    }
    for (id, _) in &items {
        if !o.outside_design.contains(id) {
            o.outside_design.push(id.clone());
        }
    }
    crate::ckfix::take_in(o, items);
    match step {
        "low" | "parked" | "review" | "gap" => {
            // A round entered only for the human findings counts against the limit only when the review
            // filed a critical, high or medium finding (a fix that waits on low findings counted it).
            if step == "review" && o.phase == "low" {
                o.rounds = o.rounds.saturating_sub(1);
            }
            o.phase = "fix".into();
            o.parked_in_review = false;
            o.reason.clear();
        }
        _ => {}
    }
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
