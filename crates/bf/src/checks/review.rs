//! The review checks' view of the records (C-24): which records each kind of review judges, and the
//! hash of them that a review's receipt is bound to. A change to any record of a kind needs that
//! review again; a change to a record of another kind does not.

use crate::refusal::R;
use crate::store::Project;
use crate::util;
use serde_json::Value;

/// What a design review judges: one stage, or the whole of an amendment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewKind {
    Idea,
    Requirements,
    Architecture,
    /// The legal and markets records, a stage of an amendment.
    Legal,
    Plan,
    /// The whole of an amendment: the records its set names and its stage decisions.
    Amendment,
}

/// Every kind of review, in the order of the gates.
pub const REVIEW_KINDS: [ReviewKind; 6] = [
    ReviewKind::Idea,
    ReviewKind::Requirements,
    ReviewKind::Architecture,
    ReviewKind::Legal,
    ReviewKind::Plan,
    ReviewKind::Amendment,
];

impl ReviewKind {
    pub fn name(self) -> &'static str {
        match self {
            ReviewKind::Idea => "idea",
            ReviewKind::Requirements => "requirements",
            ReviewKind::Architecture => "architecture",
            ReviewKind::Legal => "legal",
            ReviewKind::Plan => "plan",
            ReviewKind::Amendment => "amendment",
        }
    }

    pub fn parse(name: &str) -> Option<ReviewKind> {
        REVIEW_KINDS.iter().copied().find(|k| k.name() == name)
    }

    /// The stage a review of this kind belongs to, as a decision names it.
    fn stage(self) -> &'static str {
        match self {
            ReviewKind::Idea => "idea",
            ReviewKind::Requirements => "requirements",
            ReviewKind::Architecture | ReviewKind::Legal => "architecture",
            ReviewKind::Plan => "plan",
            ReviewKind::Amendment => "",
        }
    }

    /// The record kinds this review judges. At a gate the architecture holds the markets and legal
    /// records too; inside an amendment the legal stage is its own.
    fn record_kinds(self, scoped: bool) -> &'static [&'static str] {
        match (self, scoped) {
            (ReviewKind::Idea, _) => &["idea"],
            (ReviewKind::Requirements, _) => &["requirement", "journey"],
            (ReviewKind::Architecture, false) => &[
                "component",
                "decision",
                "target",
                "threat",
                "markets",
                "legal",
            ],
            (ReviewKind::Architecture, true) => &["component", "decision", "target", "threat"],
            (ReviewKind::Legal, _) => &["markets", "legal"],
            (ReviewKind::Plan, _) => &["unit", "handoff", "check", "checkpoint"],
            (ReviewKind::Amendment, _) => &[],
        }
    }
}

/// The stage a record kind belongs to.
fn stage_of_kind(kind: &str) -> &'static str {
    match kind {
        "idea" => "idea",
        "requirement" | "journey" => "requirements",
        "screen" => "ui",
        "component" | "decision" | "target" | "threat" | "markets" | "legal" => "architecture",
        _ => "plan",
    }
}

fn is_review_check(rec: &Value) -> bool {
    let n = rec["named"].as_str().unwrap_or("");
    super::in_family(super::Family::Review)
        .iter()
        .any(|c| c.name == n)
}

/// The records a review of `kind` judges, as `(name, content)` sorted by name. `set` is the list of IDs
/// an amendment names: for the amendment kind it picks the records, for a stage kind it narrows the
/// stage to the records of the set. Without a set, a stage kind covers the whole stage.
/// The idea also holds the text of `factory/idea.md`.
pub fn kind_records(
    p: &Project,
    kind: ReviewKind,
    set: Option<&[String]>,
) -> R<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    let in_set = |id: &str| set.map(|s| s.iter().any(|x| x == id)).unwrap_or(true);
    let mut push = |id: &str, rec: &Value| {
        out.push((
            id.to_string(),
            serde_json::to_string(rec).unwrap_or_default(),
        ));
    };
    let all = p.list_records(None)?;
    match kind {
        ReviewKind::Amendment => {
            let ids = set.unwrap_or(&[]);
            let mut stages: Vec<&str> = Vec::new();
            for rec in &all {
                let id = rec["id"].as_str().unwrap_or("");
                if ids.iter().any(|x| x == id) {
                    stages.push(stage_of_kind(rec["kind"].as_str().unwrap_or("")));
                    push(id, rec);
                }
            }
            // The stage decisions of the stages the amendment touches.
            for rec in &all {
                let id = rec["id"].as_str().unwrap_or("");
                if rec["kind"] == "decision"
                    && !ids.iter().any(|x| x == id)
                    && stages.contains(&rec["stage"].as_str().unwrap_or(""))
                {
                    push(id, rec);
                }
            }
        }
        _ => {
            let kinds = kind.record_kinds(set.is_some());
            for rec in &all {
                let id = rec["id"].as_str().unwrap_or("");
                let k = rec["kind"].as_str().unwrap_or("");
                if !kinds.contains(&k) || is_review_check(rec) {
                    continue;
                }
                // A decision belongs to the stage it names; one that names none is architecture's.
                if k == "decision" && set.is_some() {
                    let st = rec["stage"].as_str().unwrap_or("");
                    if !in_set(id) && st != kind.stage() {
                        continue;
                    }
                } else if !in_set(id) {
                    continue;
                }
                push(id, rec);
            }
        }
    }
    if kind == ReviewKind::Idea {
        if let Ok(text) = std::fs::read_to_string(p.factory().join("idea.md")) {
            out.push(("factory/idea.md".to_string(), text));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The hash a review's receipt carries: of the records the review of `kind` judges. It changes with
/// any of them and with no record of another kind.
pub fn records_hash(p: &Project, kind: ReviewKind, set: Option<&[String]>) -> R<String> {
    let mut buf = String::new();
    buf.push_str(kind.name());
    buf.push('\n');
    for (name, content) in kind_records(p, kind, set)? {
        buf.push_str(&name);
        buf.push('\0');
        buf.push_str(&content);
        buf.push('\n');
    }
    Ok(util::sha256_hex(buf.as_bytes()))
}

// ---------------------------------------------------------------------------------------------
// The review rules (R-97, R-98, D-72): rounds and the park, the holds of a gate and of an
// amendment, the human's answer and the page data. The hooks of the evidence, record, amend and
// review commands call these functions; nothing here appends to the log.
//
// A subject is the name of a gate (`idea`, `requirements`, `architecture`, `plan`) or an amendment
// ID (`A-3`). Events this part reads, all with a `subject` field:
//   design-review.park     {subject, round}
//   design-review.release  {subject, turn, quote}
//   review.finding.accept  {subject, ids, turn, quote}
//   amend.split            {amendment, stage | stages}      (written by the split command)
// A review receipt carries `review_kind`, `review_amendment` (empty at a gate) and `records_hash`.
// ---------------------------------------------------------------------------------------------

use crate::events::{self, Decide, State};
use crate::refusal::{refuse, refuse_owned};
use crate::store::{s, str_list};
use serde_json::json;
use std::collections::HashMap;

pub const PARK_EVENT: &str = "design-review.park";
pub const RELEASE_EVENT: &str = "design-review.release";
pub const ACCEPT_EVENT: &str = "review.finding.accept";
pub const SPLIT_EVENT: &str = "amend.split";
/// The stage of an amendment that the UI/UX verifier reviews.
pub const UI_STAGE: &str = "ui";

/// The gates whose records a design reviewer judges.
pub const GATES: [&str; 4] = ["idea", "requirements", "architecture", "plan"];
const STAGES: [&str; 5] = ["requirements", "ui", "architecture", "legal", "plan"];

fn is_amendment(subject: &str) -> bool {
    subject.starts_with("A-")
}

/// The named check a review of this subject is on: the gate's own, or the amendment review.
pub fn check_name(subject: &str) -> String {
    if is_amendment(subject) {
        super::AMENDMENT_REVIEW.to_string()
    } else {
        format!("{subject}-review")
    }
}

/// Whether the named check is on now.
pub fn is_on(st: &State, name: &str) -> bool {
    super::required_now(st).iter().any(|c| c.name == name)
}

/// The key of the token of the reviewer of `kind` for `subject` (`design` for a gate).
pub fn token_key(subject: &str, kind: &str) -> String {
    if is_amendment(subject) {
        if kind == UI_STAGE {
            format!("{subject}:uiux-verifier:{UI_STAGE}")
        } else {
            format!("{subject}:design-reviewer:{kind}")
        }
    } else {
        format!("{}:design-reviewer:{kind}", crate::roles::DESIGN)
    }
}

/// The hash a review of `kind` is bound to; `ui` is the screens of the set.
pub fn hash_for(p: &Project, kind: &str, set: Option<&[String]>) -> R<String> {
    if kind == UI_STAGE {
        let mut buf = String::from("ui\n");
        let mut rows: Vec<(String, String)> = Vec::new();
        for rec in p.list_records(None)? {
            let id = rec["id"].as_str().unwrap_or("").to_string();
            if rec["kind"] == "screen" && set.map(|x| x.contains(&id)).unwrap_or(true) {
                rows.push((id, serde_json::to_string(&rec).unwrap_or_default()));
            }
        }
        rows.sort();
        for (a, b) in rows {
            buf.push_str(&format!("{a}\0{b}\n"));
        }
        return Ok(util::sha256_hex(buf.as_bytes()));
    }
    match ReviewKind::parse(kind) {
        Some(k) => records_hash(p, k, set),
        None => Err(refuse(
            format!("hash the records of a review of kind `{kind}`"),
            "that is not a kind of review",
            &["Use idea, requirements, architecture, legal, plan, amendment or ui."],
            "The command prints a hash.",
        )),
    }
}

/// One review receipt, read from the receipts and the log.
#[derive(Clone, Debug)]
pub struct Review {
    pub receipt: String,
    pub subject: String,
    pub kind: String,
    pub hash: String,
    /// `met`, `failed` or `unreachable`.
    pub outcome: String,
    pub says: String,
    pub file: String,
    pub at: String,
    pub seq: u64,
    num: u64,
}

impl Review {
    fn order(&self) -> (u64, u64) {
        (self.seq, self.num)
    }
    fn counts(&self) -> bool {
        self.outcome != "unreachable"
    }
}

fn events_of(p: &Project) -> R<Vec<Value>> {
    Ok(events::load_log(p)?.events)
}

fn last_seq(evs: &[Value], kind: &str, subject: &str) -> u64 {
    evs.iter()
        .filter(|e| e["kind"] == kind && e["subject"] == subject)
        .filter_map(|e| e["seq"].as_u64())
        .max()
        .unwrap_or(0)
}

/// Every sealed review receipt of the subject, oldest first.
pub fn reviews(p: &Project, subject: &str) -> R<Vec<Review>> {
    let evs = events_of(p)?;
    reviews_in(p, &evs, subject)
}

fn reviews_in(p: &Project, evs: &[Value], subject: &str) -> R<Vec<Review>> {
    let mut seqs: HashMap<String, u64> = HashMap::new();
    for e in evs {
        if matches!(
            e["kind"].as_str(),
            Some("evidence.attach" | "evidence.outcome")
        ) {
            if let (Some(r), Some(q)) = (e["receipt"].as_str(), e["seq"].as_u64()) {
                seqs.insert(r.to_string(), q);
            }
        }
    }
    let mut out = Vec::new();
    for r in crate::evidence::load_receipts(p)? {
        if !r.seal_ok || r.stale {
            continue;
        }
        let kind = s(&r.v, "review_kind");
        if kind.is_empty() {
            continue;
        }
        let amend = s(&r.v, "review_amendment");
        let mine = if is_amendment(subject) {
            amend == subject
        } else {
            amend.is_empty() && kind == subject
        };
        if !mine {
            continue;
        }
        let outcome = match s(&r.v, "type") {
            "attach" if r.passes() => "met",
            "fail" => "failed",
            "unreachable" => "unreachable",
            _ => continue,
        };
        let id = r.id().to_string();
        out.push(Review {
            seq: seqs.get(&id).copied().unwrap_or(0),
            num: id.trim_start_matches("rc-").parse().unwrap_or(0),
            receipt: id,
            subject: subject.to_string(),
            kind: kind.to_string(),
            hash: s(&r.v, "records_hash").to_string(),
            outcome: outcome.to_string(),
            says: s(&r.v, "reason").to_string(),
            file: s(&r.v, "file_name").to_string(),
            at: s(&r.v, "at").to_string(),
        });
    }
    out.sort_by_key(|r| r.order());
    Ok(out)
}

/// The stages the general designer split an amendment into.
pub fn split_stages(evs: &[Value], amendment: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in evs
        .iter()
        .filter(|e| e["kind"] == SPLIT_EVENT && e["amendment"] == amendment)
    {
        let mut add = |x: &str| {
            if STAGES.contains(&x) && !out.iter().any(|y| y == x) {
                out.push(x.to_string());
            }
        };
        if let Some(x) = e["stage"].as_str() {
            add(x);
        }
        match &e["stages"] {
            Value::Object(m) => m.keys().for_each(|k| add(k)),
            Value::Array(a) => a.iter().filter_map(|x| x.as_str()).for_each(&mut add),
            _ => {}
        }
    }
    out
}

/// One round of a review, with the reviews that belong to it.
#[derive(Clone, Debug)]
pub struct Round {
    pub n: u32,
    pub reviews: Vec<Review>,
}

fn ends_round(subject: &str, r: &Review) -> bool {
    r.counts() && (!is_amendment(subject) || r.kind == "amendment")
}

/// The rounds since the latest release, and how many of them ended. The last round may be open.
fn rounds_in(subject: &str, evs: &[Value], all: &[Review]) -> (Vec<Round>, u32) {
    let released = last_seq(evs, RELEASE_EVENT, subject);
    let mine: Vec<&Review> = all.iter().filter(|r| r.seq > released).collect();
    let split = is_amendment(subject) && !split_stages(evs, subject).is_empty();
    let mut rounds: Vec<Round> = Vec::new();
    let mut pending: Vec<Review> = Vec::new();
    let mut ended = 0u32;
    for r in mine {
        if ends_round(subject, r) {
            ended += 1;
            let mut rs = std::mem::take(&mut pending);
            rs.push(r.clone());
            rounds.push(Round {
                n: ended,
                reviews: rs,
            });
        } else if !split && is_amendment(subject) {
            // A solo review's requested stage reviews answer the round before them. One with no
            // round before it since the release answers the round that parked: it is not a
            // round of the fresh set.
            if let Some(last) = rounds.last_mut() {
                last.reviews.push(r.clone());
            }
        } else {
            pending.push(r.clone());
        }
    }
    if !pending.is_empty() {
        rounds.push(Round {
            n: ended + 1,
            reviews: pending,
        });
    }
    (rounds, ended)
}

/// The stage reviews run after a release to answer the round that parked: they are no round of
/// the fresh set, so they are listed apart. Only a solo amendment has them.
fn answering_parked(subject: &str, evs: &[Value], all: &[Review]) -> Vec<Review> {
    if !is_amendment(subject) || !split_stages(evs, subject).is_empty() {
        return Vec::new();
    }
    let released = last_seq(evs, RELEASE_EVENT, subject);
    all.iter()
        .filter(|r| r.seq > released)
        .take_while(|r| !ends_round(subject, r))
        .cloned()
        .collect()
}

/// The rounds of the subject since its latest release, and the number that ended.
pub fn rounds(p: &Project, subject: &str) -> R<(Vec<Round>, u32)> {
    let evs = events_of(p)?;
    let all = reviews_in(p, &evs, subject)?;
    Ok(rounds_in(subject, &evs, &all))
}

/// Whether the subject is parked: a park later than its latest release.
pub fn parked(p: &Project, subject: &str) -> R<bool> {
    let evs = events_of(p)?;
    let park = last_seq(&evs, PARK_EVENT, subject);
    Ok(park > 0 && park > last_seq(&evs, RELEASE_EVENT, subject))
}

/// A finding of a gate or an amendment, as the rules read it.
#[derive(Clone, Debug)]
pub struct Finding {
    pub id: String,
    pub subject: String,
    /// The kind of review that filed it: the gate, a stage, or `amendment`.
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub action: String,
    /// `open`, `fixed` or `accepted`. A recorded `accepted` that no accept event backs reads as open.
    pub outcome: String,
    pub asks: String,
    /// The human's words, for an accepted one.
    pub words: String,
}

impl Finding {
    /// A finding with no severity is judged as a medium one.
    pub fn blocking(&self) -> bool {
        matches!(self.severity.as_str(), "" | "critical" | "high" | "medium")
    }
}

fn findings_in(p: &Project, evs: &[Value], subject: &str) -> R<Vec<Finding>> {
    let mut words: HashMap<String, String> = HashMap::new();
    for e in evs.iter().filter(|e| e["kind"] == ACCEPT_EVENT) {
        for id in str_list(e, "ids") {
            words.insert(id, e["quote"].as_str().unwrap_or("").to_string());
        }
    }
    let mut out = Vec::new();
    let kind = crate::schema::kind_by_name("finding");
    for f in p.list_records(kind)? {
        let amend = s(&f, "amendment");
        let gate = s(&f, "gate");
        let (mine, fk) = if is_amendment(subject) {
            (
                amend == subject,
                if gate.is_empty() { "amendment" } else { gate },
            )
        } else {
            (amend.is_empty() && gate == subject, gate)
        };
        if !mine {
            continue;
        }
        let id = s(&f, "id").to_string();
        let outcome = match s(&f, "outcome") {
            "fixed" => "fixed",
            "accepted" if words.contains_key(&id) => "accepted",
            _ => "open",
        };
        out.push(Finding {
            words: if outcome == "accepted" {
                words.get(&id).cloned().unwrap_or_default()
            } else {
                String::new()
            },
            id,
            subject: subject.to_string(),
            kind: fk.to_string(),
            severity: s(&f, "severity").to_string(),
            title: s(&f, "title").to_string(),
            action: s(&f, "action").to_string(),
            outcome: outcome.to_string(),
            asks: s(&f, "asks").to_string(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// The findings of a subject.
pub fn findings(p: &Project, subject: &str) -> R<Vec<Finding>> {
    findings_in(p, &events_of(p)?, subject)
}

struct AmendCtx {
    set: Vec<String>,
    repair: bool,
    open: bool,
    split: Vec<String>,
}

fn amend_ctx(st: &State, evs: &[Value], id: &str) -> Option<AmendCtx> {
    st.amends.iter().find(|a| a.id == id).map(|a| AmendCtx {
        set: a.ids.clone(),
        repair: a.repair,
        open: matches!(a.status.as_str(), "open" | "narrowed"),
        split: split_stages(evs, id),
    })
}

fn current(rs: &[Review], kind: &str, hash: &str) -> Option<Review> {
    rs.iter()
        .filter(|r| r.kind == kind && r.hash == hash && r.counts())
        .max_by_key(|r| r.order())
        .cloned()
}

/// A finding that asks for a stage review counts as fixed once that review exists at the current hash.
fn ask_met(p: &Project, ctx: &AmendCtx, rs: &[Review], f: &Finding) -> R<bool> {
    if f.asks.is_empty() {
        return Ok(false);
    }
    let h = hash_for(p, &f.asks, Some(&ctx.set))?;
    Ok(current(rs, &f.asks, &h).is_some())
}

fn holding_lines(fs: &[Finding], skip: &dyn Fn(&Finding) -> bool) -> Vec<String> {
    fs.iter()
        .filter(|f| f.outcome == "open" && f.blocking() && !skip(f))
        .map(|f| {
            format!(
                "{} ({}): {} - fix it, or the human accepts it in their own words",
                f.id,
                if f.severity.is_empty() {
                    "no severity, read as medium"
                } else {
                    &f.severity
                },
                f.title
            )
        })
        .collect()
}

/// What holds a design gate: one line each. Empty when nothing holds it, the gate has no review
/// (ui and legal) or its review check is off.
pub fn gate_blocker(p: &Project, st: &State, gate: &str) -> R<Vec<String>> {
    if !GATES.contains(&gate) || !is_on(st, &check_name(gate)) {
        return Ok(vec![]);
    }
    let evs = events_of(p)?;
    let rs = reviews_in(p, &evs, gate)?;
    let h = hash_for(p, gate, None)?;
    let mut out = Vec::new();
    if !rs
        .iter()
        .any(|r| r.counts() && r.hash == h && r.kind == gate)
    {
        out.push(format!(
            "no review of the {gate} records as they are now: a change to any of them needs the review again (check {})",
            check_name(gate)
        ));
    }
    if parked(p, gate)? {
        out.push(format!("the {gate} review is parked: the human releases it with `bf review release {gate} --quote \"<their words>\"`"));
    }
    out.extend(holding_lines(&findings_in(p, &evs, gate)?, &|_| false));
    Ok(out)
}

/// What holds the approval of an amendment: one line each. Empty for a plan repair, when the
/// amendment review is off, or when nothing holds it.
pub fn amendment_blocker(p: &Project, st: &State, id: &str) -> R<Vec<String>> {
    let evs = events_of(p)?;
    let Some(ctx) = amend_ctx(st, &evs, id) else {
        return Ok(vec![]);
    };
    if ctx.repair || !is_on(st, super::AMENDMENT_REVIEW) {
        return Ok(vec![]);
    }
    let rs = reviews_in(p, &evs, id)?;
    let fs = findings_in(p, &evs, id)?;
    let set = Some(ctx.set.as_slice());
    let mut out = Vec::new();
    let mut need: Vec<String> = Vec::new();
    let mut newest_stage = (0u64, 0u64);
    if ctx.split.is_empty() {
        for f in fs
            .iter()
            .filter(|f| !f.asks.is_empty() && f.outcome != "accepted")
        {
            if !need.contains(&f.asks) {
                need.push(f.asks.clone());
            }
        }
    } else {
        need = ctx.split.clone();
    }
    for st_name in &need {
        let h = hash_for(p, st_name, set)?;
        match current(&rs, st_name, &h) {
            Some(r) => newest_stage = newest_stage.max(r.order()),
            None => out.push(format!(
                "no review of the {st_name} stage of {id} as its records are now"
            )),
        }
    }
    let h = hash_for(p, "amendment", set)?;
    match current(&rs, "amendment", &h) {
        None => out.push(format!(
            "no review of kind amendment of {id} as its records are now"
        )),
        Some(r) if !ctx.split.is_empty() && r.order() <= newest_stage => out.push(format!(
            "the review of kind amendment of {id} is older than a stage review: the stages must be reviewed first, then the review that they agree"
        )),
        _ => {}
    }
    if parked(p, id)? {
        out.push(format!("the review of {id} is parked: the human releases it with `bf review release {id} --quote \"<their words>\"`"));
    }
    let mut skip_ids: Vec<String> = Vec::new();
    for f in &fs {
        if ask_met(p, &ctx, &rs, f)? {
            skip_ids.push(f.id.clone());
        }
    }
    out.extend(holding_lines(&fs, &|f| skip_ids.contains(&f.id)));
    Ok(out)
}

/// What on_attach says about a review receipt: the fields it carries and whether to park.
#[derive(Clone, Debug)]
pub struct Attach {
    /// The gate or the amendment ID.
    pub subject: String,
    pub kind: String,
    pub amendment: Option<String>,
    pub records_hash: String,
    /// The round the receipt belongs to.
    pub round: u32,
    /// Append `design-review.park` for the subject.
    pub park: bool,
}

impl Attach {
    /// The fields the receipt carries.
    pub fn fields(&self) -> Value {
        json!({
            "review_kind": self.kind,
            "review_amendment": self.amendment.clone().unwrap_or_default(),
            "records_hash": self.records_hash,
            "review_round": self.round,
        })
    }
}

fn claim_from_token(p: &Project, token: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(m) = crate::run::read_tokens(p).as_object() {
        for (k, v) in m {
            if v.as_str() != Some(token.trim()) {
                continue;
            }
            let parts: Vec<&str> = k.split(':').collect();
            match parts.as_slice() {
                [subj, "design-reviewer", kind] if *subj == crate::roles::DESIGN => {
                    out.push((kind.to_string(), kind.to_string()))
                }
                [a, "design-reviewer", kind] if is_amendment(a) => {
                    out.push((a.to_string(), kind.to_string()))
                }
                [a, "uiux-verifier", "ui"] if is_amendment(a) => {
                    out.push((a.to_string(), UI_STAGE.to_string()))
                }
                _ => {}
            }
        }
    }
    out
}

fn refuse_outcome(why: String, steps: &[&str]) -> crate::refusal::Fail {
    refuse(
        "give a design review an outcome",
        why,
        steps,
        "`bf evidence verify` lists the receipt with its kind and hash.",
    )
}

/// The rules for an outcome on a review check. Returns `None` for a check that is not a design
/// review. `amendment` and `kind` name what the outcome is for when no token says it (tokens off).
pub fn on_attach(
    p: &Project,
    st: &State,
    check: &Value,
    outcome: &str,
    token: Option<&str>,
    amendment: Option<&str>,
    kind: Option<&str>,
) -> R<Option<Attach>> {
    let named = s(check, "named");
    let is_design = super::in_family(super::Family::Review)
        .iter()
        .any(|c| c.name == named && c.name != super::CHECKPOINT_REVIEW);
    let uiux_name = named == "design-uiux-verification" && amendment.is_some();
    if !is_design && !uiux_name {
        return Ok(None);
    }
    let gate_of_check = named.strip_suffix("-review").filter(|g| GATES.contains(g));
    // Who and what the outcome is for.
    let (subject, kind) = if p.policy.roles.tokens {
        let tok = token.unwrap_or("");
        let mut found = claim_from_token(p, tok);
        if tok.trim().is_empty() {
            found.clear();
        }
        found.retain(|(sj, k)| {
            amendment.map(|a| a == sj).unwrap_or(true) && kind.map(|x| x == k).unwrap_or(true)
        });
        found.into_iter().next().ok_or_else(|| {
            refuse_outcome(
                "an outcome on a design review is accepted only with the design reviewer's token of that kind (the UI/UX verifier's for the ui stage of an amendment); none, or the wrong one, was given".into(),
                &["Run `bf review token <kind> [--subject A-n]` for the reviewer's token.", "The reviewer passes it with `--token`."],
            )
        })?
    } else if let Some(a) = amendment {
        let Some(k) = kind else {
            return Err(refuse_outcome(
                "the kind of the review is missing".into(),
                &["Name the kind with the outcome."],
            ));
        };
        (a.to_string(), k.to_string())
    } else if let Some(g) = gate_of_check {
        (g.to_string(), g.to_string())
    } else {
        return Err(refuse_outcome(
            "the amendment the review is for is missing".into(),
            &["Name the amendment with the outcome."],
        ));
    };
    let amend_id = is_amendment(&subject).then(|| subject.clone());
    if amend_id.is_none() {
        if gate_of_check != Some(subject.as_str()) || kind != subject {
            return Err(refuse_outcome(
                format!("the token is for a review of {kind}, not for check {named}"),
                &["Use the check that matches the kind of the token."],
            ));
        }
    } else {
        let ok = named == super::AMENDMENT_REVIEW
            || (kind == UI_STAGE && uiux_name)
            || gate_of_check == Some(kind.as_str());
        if !ok || !(kind == "amendment" || STAGES.contains(&kind.as_str())) {
            return Err(refuse_outcome(
                format!("a review of kind {kind} of an amendment is not given on check {named}"),
                &["Use the check amendment-review."],
            ));
        }
        let evs = events_of(p)?;
        if !amend_ctx(st, &evs, &subject)
            .map(|c| c.open)
            .unwrap_or(false)
        {
            return Err(refuse_outcome(
                format!("{subject} is not an open amendment"),
                &["`bf status` lists the amendments."],
            ));
        }
    }
    if !matches!(outcome, "met" | "failed" | "unreachable") {
        return Err(refuse_outcome(
            format!("`{outcome}` is not an outcome"),
            &["Use met, failed or unreachable."],
        ));
    }
    if parked(p, &subject)? {
        return Err(refuse_outcome(
            format!("the review of {subject} is parked after {} rounds without a clean result", p.policy.limits.max_rounds),
            &["The human releases it in their own words: `bf review release <gate or A-n> --quote \"<their words>\"`."],
        ));
    }
    let evs = events_of(p)?;
    let ctx = amend_ctx(st, &evs, &subject);
    let set: Option<&[String]> = ctx.as_ref().map(|c| c.set.as_slice());
    let hash = hash_for(p, &kind, set)?;
    let fs = findings_in(p, &evs, &subject)?;
    let rs = reviews_in(p, &evs, &subject)?;
    let mut skip_ids: Vec<String> = Vec::new();
    if let Some(c) = &ctx {
        for f in &fs {
            if ask_met(p, c, &rs, f)? {
                skip_ids.push(f.id.clone());
            }
        }
    }
    let held = holding_lines(
        &fs.iter()
            .filter(|f| f.kind == kind)
            .cloned()
            .collect::<Vec<_>>(),
        &|f| skip_ids.contains(&f.id),
    );
    if outcome == "met" && !held.is_empty() {
        return Err(refuse_owned(
            "give a design review the outcome met",
            "a critical, high or medium finding of this review is open",
            {
                let mut v = held;
                v.push("Fix them and file them fixed, or record the outcome failed.".into());
                v
            },
            "`bf evidence verify` lists the receipt.",
        ));
    }
    let (_, done) = rounds_in(&subject, &evs, &rs);
    let probe = Review {
        receipt: String::new(),
        subject: subject.clone(),
        kind: kind.clone(),
        hash: hash.clone(),
        outcome: outcome.to_string(),
        says: String::new(),
        file: String::new(),
        at: String::new(),
        seq: u64::MAX,
        num: u64::MAX,
    };
    let ends = ends_round(&subject, &probe);
    let split = ctx.as_ref().map(|c| !c.split.is_empty()).unwrap_or(false);
    let round = if ends || split || done == 0 {
        done + 1
    } else {
        done
    };
    let any_open = fs
        .iter()
        .any(|f| f.outcome == "open" && f.blocking() && !skip_ids.contains(&f.id));
    Ok(Some(Attach {
        // An ask finding of the solo amendment reviewer in the last round parks with that round
        // (it is open, and holds): the stage review it asked for is refused as parked until the
        // human releases, and then runs; it does not count as a round of the fresh set.
        park: ends && round == p.policy.limits.max_rounds && any_open && outcome != "met",
        subject,
        kind,
        amendment: amend_id,
        records_hash: hash,
        round,
    }))
}

/// The ways a design finding may be put. `existing` is the record already on file, if any.
pub fn may_put_finding(
    p: &Project,
    st: &State,
    rec: &Value,
    existing: Option<&Value>,
    token: Option<&str>,
) -> R<()> {
    let gate = s(rec, "gate");
    let amend = s(rec, "amendment");
    if gate.is_empty() && amend.is_empty() {
        return Ok(());
    }
    let refused = format!("put finding {}", s(rec, "id"));
    let no = |why: String, steps: &[&str]| {
        refuse(
            refused.clone(),
            why,
            steps,
            "`bf record get` shows the finding.",
        )
    };
    let was = existing.map(|e| s(e, "outcome")).unwrap_or("");
    if s(rec, "outcome") == "accepted" && was != "accepted" {
        return Err(no(
            "a finding is never accepted by a put; the human accepts it in their own words".into(),
            &["The human says so in chat; then `bf review finding <gate or A-n> --accept <F-ids> --quote \"<their words>\"`."],
        ));
    }
    let (subject, kind) = if amend.is_empty() {
        (gate.to_string(), gate.to_string())
    } else {
        (
            amend.to_string(),
            if gate.is_empty() {
                "amendment".into()
            } else {
                gate.to_string()
            },
        )
    };
    let reviewed = if amend.is_empty() {
        GATES.contains(&gate)
    } else {
        true
    };
    if reviewed && p.policy.roles.tokens {
        let key = token_key(&subject, &kind);
        if !crate::run::holds_token(p, &key, token) {
            return Err(no(
                format!("a finding of {subject} is put only with the token of its kind ({kind}); none, or the wrong one, was given"),
                &["The design reviewer passes the token `bf review token` issued for its kind."],
            ));
        }
    }
    if !s(rec, "asks").is_empty() {
        let evs = events_of(p)?;
        let split = !amend.is_empty() && !split_stages(&evs, amend).is_empty();
        if amend.is_empty() || kind != "amendment" || split {
            return Err(no(
                "a request for a stage review comes only from the solo amendment reviewer of an amendment that was not split".into(),
                &["Leave `asks` out."],
            ));
        }
        if p.policy.roles.tokens
            && !crate::run::holds_token(p, &token_key(&subject, "amendment"), token)
        {
            return Err(no(
                "a request for a stage review needs the solo amendment reviewer's token".into(),
                &["Use the token of kind amendment."],
            ));
        }
        let asked = s(rec, "asks");
        let other = p
            .list_records(crate::schema::kind_by_name("finding"))?
            .into_iter()
            .any(|f| {
                s(&f, "amendment") == amend && s(&f, "asks") == asked && s(&f, "id") != s(rec, "id")
            });
        if other {
            return Err(no(format!("{amend} already asks for a review of the {asked} stage: at most one finding per stage"), &["Leave this one out."]));
        }
    }
    let _ = st;
    Ok(())
}

fn marked_state(p: &Project, evs: &[Value]) -> R<State> {
    let mut st = events::replay(p)?;
    for e in evs
        .iter()
        .filter(|e| matches!(e["kind"].as_str(), Some(RELEASE_EVENT | ACCEPT_EVENT)))
    {
        if let Some(t) = e["turn"].as_u64() {
            if let Some(x) = st.turns.iter_mut().find(|x| x.seq == t) {
                x.used = true;
            }
        }
    }
    Ok(st)
}

/// The human message that accepts findings: written after the latest review of the findings' kind,
/// unused, taken once. Returns the log position of the message; the caller appends
/// `review.finding.accept` with it and sets the findings' outcome.
pub fn check_accept(p: &Project, ids: &[String], quote: &str) -> R<u64> {
    let refused = "accept findings of a design review";
    if ids.is_empty() {
        return Err(refuse(
            refused,
            "no finding was named",
            &["Name the findings: `--accept F-1,F-2`."],
            "The command prints the accepted findings.",
        ));
    }
    let evs = events_of(p)?;
    let mut bound = 0u64;
    for id in ids {
        let rec = p.get_record(id)?.filter(|r| r["kind"] == "finding");
        let Some(rec) = rec else {
            return Err(refuse(
                refused,
                format!("{id} is not a finding"),
                &["Use the ID of a finding."],
                "The command prints the accepted findings.",
            ));
        };
        let (gate, amend) = (s(&rec, "gate"), s(&rec, "amendment"));
        let (subject, kind) = if amend.is_empty() {
            (gate.to_string(), gate.to_string())
        } else {
            (
                amend.to_string(),
                if gate.is_empty() {
                    "amendment".to_string()
                } else {
                    gate.to_string()
                },
            )
        };
        if subject.is_empty() {
            return Err(refuse(
                refused,
                format!("{id} belongs to a unit, not to a gate or an amendment"),
                &["Use the finding of a gate or an amendment."],
                "The command prints the accepted findings.",
            ));
        }
        let f = findings_in(p, &evs, &subject)?
            .into_iter()
            .find(|f| &f.id == id);
        if f.map(|f| f.outcome != "open").unwrap_or(true) {
            return Err(refuse(
                refused,
                format!("{id} is not open"),
                &["Only an open finding is accepted."],
                "The command prints the accepted findings.",
            ));
        }
        let latest = reviews_in(p, &evs, &subject)?
            .into_iter()
            .filter(|r| r.kind == kind && r.counts())
            .map(|r| r.order().0)
            .max();
        let Some(latest) = latest else {
            return Err(refuse(
                refused,
                format!("no review of kind {kind} of {subject} exists to answer"),
                &["The review comes first."],
                "The command prints the accepted findings.",
            ));
        };
        bound = bound.max(latest);
    }
    let st = marked_state(p, &evs)?;
    Ok(events::pick_turn(&st, bound, Decide::Approve, quote)?.0)
}

/// The human message that releases a parked review: written after the park, unused, taken once.
/// The caller appends `design-review.release` with the position it returns.
pub fn check_release(p: &Project, subject: &str, quote: &str) -> R<u64> {
    let evs = events_of(p)?;
    let park = last_seq(&evs, PARK_EVENT, subject);
    if park == 0 || park < last_seq(&evs, RELEASE_EVENT, subject) {
        return Err(refuse(
            format!("release the review of {subject}"),
            "the review is not parked",
            &["`bf status` shows the review."],
            "`bf status` shows the review.",
        ));
    }
    let st = marked_state(p, &evs)?;
    Ok(events::pick_turn(&st, park, Decide::Approve, quote)?.0)
}

fn review_json(r: &Review) -> Value {
    json!({"receipt": r.receipt, "kind": r.kind, "outcome": r.outcome, "says": r.says,
           "file": r.file, "at": r.at})
}

fn finding_json(f: &Finding) -> Value {
    // A finding's own action text, not a directive: set apart from the `json!` so the directive scan skips it.
    let mut v = json!({"id": f.id, "severity": f.severity, "title": f.title,
                       "outcome": f.outcome, "asks": f.asks, "words": f.words});
    v["action"] = json!(f.action);
    v
}

/// The review's data for a gate page or an amendment page.
pub fn page_section(p: &Project, st: &State, subject: &str) -> R<Value> {
    let evs = events_of(p)?;
    let all = reviews_in(p, &evs, subject)?;
    let (rs, done) = rounds_in(subject, &evs, &all);
    let fs = findings_in(p, &evs, subject)?;
    let park = last_seq(&evs, PARK_EVENT, subject);
    let released = last_seq(&evs, RELEASE_EVENT, subject);
    let words = evs
        .iter()
        .rev()
        .find(|e| e["kind"] == RELEASE_EVENT && e["subject"] == subject)
        .and_then(|e| e["quote"].as_str())
        .unwrap_or("");
    let (hold, low): (Vec<&Finding>, Vec<&Finding>) = fs.iter().partition(|f| f.blocking());
    Ok(json!({
        "check": check_name(subject),
        "on": is_on(st, &check_name(subject)),
        "rounds": rs.iter().map(|r| json!({"round": r.n, "reviews": r.reviews.iter().map(review_json).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "answering_parked": answering_parked(subject, &evs, &all).iter().map(review_json).collect::<Vec<_>>(),
        "round": done,
        "max_rounds": p.policy.limits.max_rounds,
        "parked": park > 0 && park > released,
        "release_words": words,
        "findings": {
            "holding": hold.into_iter().map(finding_json).collect::<Vec<_>>(),
            "low": low.into_iter().map(finding_json).collect::<Vec<_>>(),
        },
    }))
}
