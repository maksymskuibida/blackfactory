//! `bf ask`, `bf answer` and `bf questions`.

use super::{out, project, read_data_in, real_zone, Args};
use crate::refusal::{refuse, R};
use crate::store::{s, Project};
use crate::{events, schema, util};
use serde_json::{json, Value};

pub(crate) fn cmd_ask(args: &Args) -> R<()> {
    let p = project(args)?;
    let f = match (args.flag("file"), args.flag("title")) {
        (Some(file), _) => read_data_in(&p, file)?,
        (None, Some(title)) => {
            // A short question without a file: `--title`, `--context`, `--kind`, `--options "A=text;B=text"`.
            let mut q = json!({"title": title});
            if let Some(c) = args.flag("context") {
                q["context"] = json!(c);
            }
            if let Some(k) = args.flag("kind") {
                q["kind"] = json!(k);
            }
            if let Some(o) = args.flag("options") {
                q["options"] = json!(o
                    .split(';')
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| {
                        let (k, t) = x.split_once('=').unwrap_or(("", x));
                        json!({"key": k.trim(), "text": t.trim()})
                    })
                    .collect::<Vec<_>>());
                if q.get("kind").is_none() {
                    q["kind"] = json!("single");
                }
            } else if q.get("kind").is_none() {
                q["kind"] = json!("text");
            }
            q
        }
        (None, None) => {
            return Err(refuse(
                "ask a question",
                "it needs a question file (`--file`) or a title (`--title`)",
                &["Write the question as `bf ask --title \"...\" [--context \"...\"] [--options \"A=first;B=second\"]`, or give a JSON file with `--file`."],
                "The command prints the new question ID.",
            ))
        }
    };
    // A question belongs to a stage (the gate waits for it) or to a unit (a worker's question to
    // the human; that unit waits for the answer, the others go on).
    let zone = real_zone(&p);
    let unit_tag: Option<String> = match (&zone, args.flag("unit")) {
        (Some((_, id)), _) => Some(id.clone()),
        (None, Some(u)) => Some(u.to_string()),
        _ => None,
    };
    let items: Vec<Value> = match &f {
        Value::Array(a) => a.clone(),
        o => match o.get("questions").and_then(|q| q.as_array()) {
            Some(a) => a.clone(),
            None => vec![o.clone()],
        },
    };
    if items.is_empty() {
        return Err(refuse(
            "ask questions",
            "the file holds no question",
            &["Put a list of questions in the file: [{\"title\": \"...\", \"kind\": \"single\", \"options\": [...]}]."],
            "The command prints the new question IDs.",
        ));
    }
    let kind = schema::kind_by_name("question").unwrap();
    let mut made: Vec<Value> = Vec::new();
    let mut recs: Vec<Value> = Vec::new();
    for f in &items {
        let mut rec = json!({
            "id": f.get("id").cloned().unwrap_or_else(|| json!(format!("{}-{}", kind.prefix, next_q(&p, &recs)))),
            "kind": "question",
            "title": f.get("title").cloned().unwrap_or(Value::Null),
            "answer_kind": f.get("kind").or_else(|| f.get("answer_kind")).cloned().unwrap_or(json!("single")),
        });
        for k in ["context", "options", "stage"] {
            if let Some(v) = f.get(k) {
                rec[k] = v.clone();
            }
        }
        // `other` is the reserved key of the Other answer the engine adds to every choice question.
        if let Some(opts) = rec.get("options").and_then(|o| o.as_array()) {
            if opts
                .iter()
                .any(|o| s(o, "key").eq_ignore_ascii_case("other"))
            {
                return Err(refuse(
                    format!("ask {}", f.get("title").and_then(|t| t.as_str()).unwrap_or("a question")),
                    "an option uses the key `other`, which is reserved: every choice question already takes an Other answer in the human's own words",
                    &["Give that option another key (A, B, C ...), or drop it.", "To switch the Other answer off for this question, put `\"other\": false` in it."],
                    "`bf questions` lists the question.",
                ));
            }
        }
        if let Some(b) = f.get("other").and_then(|v| v.as_bool()) {
            rec["other"] = json!(b);
        }
        if rec.get("stage").is_none() {
            if let Some(st) = args.flag("stage") {
                rec["stage"] = json!(st);
            }
        }
        if let Some(u) = &unit_tag {
            rec["unit"] = json!(u);
            rec.as_object_mut().map(|o| o.remove("stage"));
        }
        if rec.get("stage").is_none() && rec.get("unit").is_none() {
            return Err(refuse(
                "ask a question that blocks nothing",
                "the question is not tagged: it names no stage and no unit, so no gate and no unit would wait for the answer",
                &[
                    "A design question: add `--stage idea|requirements|ui|architecture|plan` (or `stage` in the file); that gate will not open until it is answered.",
                    "A question about a unit's work: add `--unit U-<n>` (a worker asks from its working copy and is tagged by itself); that unit waits for the answer while the others go on.",
                ],
                "`bf questions --open` lists the question with its stage or unit.",
            ));
        }
        schema::validate_or_refuse(kind, &rec)?;
        recs.push(rec);
    }
    // Validate everything first, then write, so a bad list writes nothing.
    // An ID the engine numbered is taken with `create_new` and a retry on the next number, so
    // questions asked at the same moment each keep their own file.
    for (i, rec) in recs.iter_mut().enumerate() {
        if items[i].get("id").is_some() {
            p.write_record(rec)?;
        } else {
            let mut tries = 0;
            while !p.write_record_new(rec)? {
                tries += 1;
                if tries > 1000 {
                    return Err(crate::refusal::err("cannot find a free question number"));
                }
                let n = schema::parse_id(s(rec, "id")).map(|x| x.1).unwrap_or(0);
                let next = next_q(&p, &[]).max(n + 1);
                rec["id"] = json!(format!("{}-{}", kind.prefix, next));
            }
        }
        events::append(&p, "question.asked", json!({"question": rec["id"]}))?;
        made.push(rec.clone());
    }
    p.write_guard()?;
    if let Some(n) = events::replay(&p)?.human_turns_note() {
        eprintln!("note: {n}");
    }
    out(args, &json!(made), || {
        made.iter()
            .map(|r| s(r, "id").to_string())
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(())
}

pub(crate) fn cmd_answer(args: &Args) -> R<()> {
    let p = project(args)?;
    let id = args.pos_at(1, "a question ID")?;
    let f = read_data_in(&p, args.need("file", "an answer file (JSON or TOML)")?)?;
    let mut q = p.require_record(id)?;
    if s(&q, "kind") != "question" {
        return Err(refuse(
            format!("answer {id}"),
            "that ID is not a question",
            &["Use a Q-<number> ID."],
            "`bf questions` lists it answered.",
        ));
    }
    // An answer is the human's. It needs a human turn recorded after the question was asked: the
    // engine cannot read the answer's source, but it can see that the human wrote since.
    let log = events::load_log(&p)?;
    let asked_seq = log
        .events
        .iter()
        .find(|e| e["kind"] == "question.asked" && e["question"] == id)
        .and_then(|e| e["seq"].as_u64())
        .unwrap_or(0);
    let st = events::replay(&p)?;
    let Some(turn) = st.turns.iter().rev().find(|t| t.seq > asked_seq) else {
        return Err(refuse(
            format!("answer {id}"),
            "no human turn was recorded after the question was asked, so nothing shows that the human answered",
            &[
                "Show the human the question form and wait for them to say they are done.",
                "The hook records their message; then run `bf answer` with what they chose.",
            ],
            "`bf questions` lists the question as answered.",
        ));
    };
    let turn_seq = turn.seq;
    let mut ans = f.get("answer").cloned().unwrap_or(f);
    // An Other answer is the human's own words: the key `other` chosen needs the text.
    if let Value::Object(o) = &mut ans {
        let chose_other = o.get("choice").and_then(|v| v.as_str()) == Some("other")
            || o.get("choices")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().any(|c| c.as_str() == Some("other")))
                .unwrap_or(false);
        if chose_other {
            if !crate::store::question_takes_other(&q) {
                return Err(refuse(
                    format!("answer {id} with Other"),
                    "this question does not take an Other answer",
                    &["Answer with one of its options."],
                    "`bf questions` lists the question as answered.",
                ));
            }
            let text = o
                .get("other")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                return Err(refuse(
                    format!("answer {id} with Other"),
                    "the answer chooses `other` but gives no text; Other means the human's own answer",
                    &["Put the human's own words in `other`: `{\"choice\": \"other\", \"other\": \"<their answer>\"}` (for a multi question `{\"choices\": [\"A\", \"other\"], \"other\": \"<text>\"}`).", "Run `bf answer` again."],
                    "`bf questions` lists the question as answered, with their text.",
                ));
            }
            o.insert("other".into(), json!(text));
        }
    }
    q["answer"] = ans;
    q["answered_at"] = json!(util::now());
    q["answered_turn"] = json!(turn_seq);
    p.write_record(&q)?;
    events::append(
        &p,
        "question.answered",
        json!({"question": id, "turn": turn_seq}),
    )?;
    p.write_guard()?;
    out(args, &q, || format!("answered {id}"));
    Ok(())
}

pub(crate) fn cmd_questions(args: &Args) -> R<()> {
    let p = project(args)?;
    let qs = p.list_records(schema::kind_by_name("question"))?;
    let sel: Vec<&Value> = qs
        .iter()
        .filter(|q| !args.has("open") || q.get("answer").is_none())
        .collect();
    out(args, &json!(sel), || {
        sel.iter()
            .map(|q| {
                format!(
                    "{} {} [{}]",
                    s(q, "id"),
                    s(q, "title"),
                    if q.get("answer").is_some() {
                        format!("answered: {}", crate::store::answer_text(q))
                    } else {
                        "open".to_string()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(())
}

/// The next free question number, counting the questions not yet written.
fn next_q(p: &Project, pending: &[Value]) -> u32 {
    let base = p
        .next_id(schema::kind_by_name("question").unwrap())
        .ok()
        .and_then(|id| schema::parse_id(&id).map(|x| x.1))
        .unwrap_or(1);
    let extra = pending
        .iter()
        .filter_map(|r| schema::parse_id(s(r, "id")).map(|x| x.1))
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);
    base.max(extra)
}
