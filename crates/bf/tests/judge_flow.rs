//! U-66: the flow of the repair judge (R-69, D-90): the directive for the conductor, the human's step for
//! an `ask` or a failed judge, the units an open amendment holds, and the verdict in the log.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;

// ---- a project with a plan the human approved ----

fn unit(id: &str, cp: &str, file: &str, deps: &[&str], est: u32) -> String {
    format!(
        "id=\"{id}\"\nkind=\"unit\"\ntitle=\"Unit {id}\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"{file}\"]\ndepends_on={}\ncheckpoint=\"{cp}\"\nestimate={est}\nfull_suite=\"fixture\"\n",
        json!(deps)
    )
}

fn plan(fx: &Fx) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"Things exist\"\nacceptance=[\"they exist\"]\n".to_string()),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Things\"\n".to_string()),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    for i in 1..=6 {
        let t = unit(&format!("U-{i}"), "K-1", &format!("src/f{i}.txt"), &[], 5);
        let o = fx.put("unit", &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let o = fx.put("unit", &unit("U-7", "K-2", "src/f7.txt", &[], 1));
    assert_eq!(o.code, 0, "{}", o.all());
    for i in 1..=7 {
        let t = format!("id=\"X-{}\"\nkind=\"check\"\ntitle=\"Unit {i}\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-{i}\"\n", 10 + i);
        let o = fx.put("check", &t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    for (k, t) in [
        ("check", "id=\"X-1\"\nkind=\"check\"\ntitle=\"First\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"checkpoint\"\n"),
        ("check", "id=\"X-2\"\nkind=\"check\"\ntitle=\"Second\"\nhow=\"command\"\ncommand=\"true\"\nproves=[\"R-1\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-1\"\nkind=\"checkpoint\"\nstop_why=\"Test.\"\ntitle=\"One\"\nafter=[\"U-1\",\"U-2\",\"U-3\",\"U-4\",\"U-5\",\"U-6\"]\nproves=[\"R-1\",\"C-1\"]\nchecks=[\"X-1\"]\n"),
        ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test.\"\ntitle=\"Two\"\nafter=[\"U-7\"]\nproves=[\"R-1\"]\nchecks=[\"X-2\"]\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("plan");
    fx.start_test_run();
}

fn put_replace(fx: &Fx, kind: &str, text: &str) {
    let f = fx.root.join("edit.toml");
    fs::write(&f, text).unwrap();
    fx.must(&["record", "put", kind, "--file", f.to_str().unwrap(), "--replace"]);
    fs::remove_file(f).unwrap();
}

/// Open a plan repair over `ids`, put the edits, narrow it. Returns its ID.
fn repair(fx: &Fx, ids: &[&str], edits: &[(&str, String)]) -> String {
    let mut a = vec!["amend", "open", "--repair"];
    a.extend(ids);
    a.push("--json");
    let o = fx.must(&a).json();
    for (k, t) in edits {
        put_replace(fx, k, t);
    }
    let aid = o["amend"].as_str().unwrap().to_string();
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": aid, "keep": o["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    aid
}

fn token(fx: &Fx, aid: &str) -> String {
    fx.must(&["roles", "token", aid, "judge", "--json"]).json()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The pass of a judge's turn for the call that carries the token `tok`.
fn pass_for(fx: &Fx, aid: &str, tok: &str) {
    let d = fx.root.join(".claude/agents");
    fs::create_dir_all(&d).unwrap();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/claude/agents/repair-judge.md"),
        d.join("repair-judge.md"),
    )
    .unwrap();
    let payload = json!({"tool_name": "Bash", "cwd": fx.root.to_string_lossy(),
        "tool_input": {"command": format!("bf amend judge {aid} --file - --token {tok} --model m")},
        "session_id": "s1", "agent_id": "g1", "agent_type": "repair-judge"});
    let o = fx.bf_stdin(&fx.root, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

fn judge(fx: &Fx, aid: &str, tok: &str, verdict: &str) -> Out {
    fx.bf_stdin(
        &fx.root,
        &["amend", "judge", aid, "--file", "-", "--token", tok, "--model", "strong-model"],
        &json!({"verdict": verdict, "reasons": "plain words"}).to_string(),
    )
}

/// The whole turn of a judge with a pass: token, pass, verdict.
fn turn(fx: &Fx, aid: &str, verdict: &str) -> Out {
    let t = token(fx, aid);
    pass_for(fx, aid, &t);
    judge(fx, aid, &t, verdict)
}

fn next(fx: &Fx) -> Value {
    fx.must(&["next", "--json"]).json()
}

fn directives(n: &Value, action: &str) -> Vec<Value> {
    n["directives"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["action"] == action)
        .cloned()
        .collect()
}

fn starts(n: &Value, unit: &str) -> bool {
    directives(n, "start-unit").iter().any(|d| d["unit"] == unit)
}

/// A repair of U-2 that puts it after U-1 (one `depends_on` entry), narrowed.
fn narrowed(fx: &Fx) -> String {
    repair(fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))])
}

#[test]
fn judge_flow_the_conductor_gets_judge_repair_once_for_a_narrowed_repair() {
    let fx = Fx::new();
    plan(&fx);
    assert!(directives(&next(&fx), "judge-repair").is_empty());
    let aid = narrowed(&fx);
    let n = next(&fx);
    let d = directives(&n, "judge-repair");
    assert_eq!(d.len(), 1, "{n}");
    assert_eq!(d[0]["role"], "conductor");
    assert_eq!(d[0]["work"], "amender");
    assert_eq!(d[0]["amendment"], aid.as_str());
    assert_eq!(d[0]["tier"], "strong", "{}", d[0]);
    assert!(d[0]["note"].as_str().unwrap().contains("repair-judge"), "{}", d[0]);
    // Once the token is out, the conductor is not sent again.
    token(&fx, &aid);
    assert!(directives(&next(&fx), "judge-repair").is_empty());
}

#[test]
fn judge_flow_no_directive_with_the_switch_off() {
    let fx = Fx::new();
    plan(&fx);
    narrowed(&fx);
    fx.set_policy("repair_judge = true", "repair_judge = false");
    let n = next(&fx);
    assert!(directives(&n, "judge-repair").is_empty(), "{n}");
}

#[test]
fn judge_flow_an_ask_is_the_humans_step_and_holds_only_its_units() {
    let fx = Fx::new();
    plan(&fx);
    let aid = narrowed(&fx);
    let o = turn(&fx, &aid, "ask");
    assert_eq!(o.code, 0, "{}", o.all());
    let n = next(&fx);
    assert!(directives(&n, "judge-repair").is_empty(), "{n}");
    let step = directives(&n, "decide-repair");
    assert_eq!(step.len(), 1, "{n}");
    assert_eq!(step[0]["role"], "human");
    assert!(step[0]["note"].as_str().unwrap().contains("plain words"), "{}", step[0]);
    // A unit outside its set is started; the unit in its set and the one in its kept impact are not.
    assert!(starts(&n, "U-3"), "{n}");
    assert!(!starts(&n, "U-2"), "{n}");
    let o = fx.refused(&["unit", "start", "U-2"]);
    assert!(o.stderr.contains(&aid) && o.stderr.contains("U-2"), "{}", o.stderr);
    fx.must(&["unit", "start", "U-3"]);
}

#[test]
fn judge_flow_a_unit_in_the_kept_impact_of_an_open_amendment_is_held() {
    let fx = Fx::new();
    plan(&fx);
    let opened = fx.must(&["amend", "open", "--repair", "U-2", "--json"]).json();
    let aid = opened["amend"].as_str().unwrap().to_string();
    let n = next(&fx);
    assert!(!starts(&n, "U-2"), "{n}");
    assert!(starts(&n, "U-1"), "{n}");
    let o = fx.refused(&["unit", "start", "U-2"]);
    assert!(o.stderr.contains(&aid), "{}", o.stderr);
    // Withdrawn, it holds nothing.
    fx.must(&["amend", "withdraw", &aid, "--reason", "not needed"]);
    assert!(starts(&next(&fx), "U-2"));
}

#[test]
fn judge_flow_a_judge_with_no_pass_costs_one_turn_and_the_repair_goes_to_the_human() {
    let fx = Fx::new();
    plan(&fx);
    let aid = narrowed(&fx);
    // The guard wrote no pass: the payload named no agent type.
    let t = token(&fx, &aid);
    let o = judge(&fx, &aid, &t, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    let n = next(&fx);
    assert!(directives(&n, "judge-repair").is_empty(), "{n}");
    let step = directives(&n, "decide-repair");
    assert_eq!(step.len(), 1, "{n}");
    assert_eq!(step[0]["role"], "human");
    assert!(step[0]["reason"].as_str().unwrap().contains("pass"), "{}", step[0]);
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(o.stderr.contains(&aid), "{}", o.stderr);
}

#[test]
fn judge_flow_a_withdrawn_ask_stays_listed_for_the_human() {
    let fx = Fx::new();
    plan(&fx);
    let aid = narrowed(&fx);
    assert_eq!(turn(&fx, &aid, "ask").code, 0);
    fx.must(&["amend", "withdraw", &aid, "--reason", "not needed"]);
    let n = next(&fx);
    let seen = directives(&n, "see-judged-repair");
    assert_eq!(seen.len(), 1, "{n}");
    assert_eq!(seen[0]["amendment"], aid.as_str());
    assert_eq!(seen[0]["info"], true);
    let note = seen[0]["note"].as_str().unwrap();
    assert!(note.contains("plain words") && note.contains("withdrawn"), "{note}");
    // Information is not a step: nothing waits on the human for it.
    assert_eq!(n["human_needed"], false, "{n}");
}

#[test]
fn judge_flow_a_repair_the_judge_approved_is_listed_for_the_human() {
    let fx = Fx::new();
    plan(&fx);
    let aid = narrowed(&fx);
    let o = turn(&fx, &aid, "go");
    assert_eq!(o.code, 0, "{}", o.all());
    let n = next(&fx);
    let seen = directives(&n, "see-judged-repair");
    assert_eq!(seen.len(), 1, "{n}");
    assert!(seen[0]["note"].as_str().unwrap().contains("approved"), "{}", seen[0]);
    assert!(starts(&n, "U-3"), "{n}");
}

#[test]
fn judge_flow_the_log_prints_the_verdict_with_its_reasons_and_no_measures() {
    let fx = Fx::new();
    plan(&fx);
    let aid = narrowed(&fx);
    assert_eq!(turn(&fx, &aid, "ask").code, 0);
    let out = fx.must(&["log"]).stdout;
    let line = out.lines().find(|l| l.contains("amend.judge\"")).unwrap_or_else(|| panic!("{out}"));
    assert!(line.contains("\"verdict\":\"ask\"") && line.contains("plain words"), "{line}");
    assert!(line.contains("strong-model"), "{line}");
    for hidden in ["measures", "fingerprint", "session"] {
        assert!(!line.contains(hidden), "{hidden} in {line}");
    }
    assert!(line.contains("[contents left out]"), "{line}");
    assert!(out.contains("--whole"), "{out}");
}

#[test]
fn judge_flow_a_token_left_from_an_earlier_narrowing_gives_the_human_a_step() {
    let fx = Fx::new();
    plan(&fx);
    let aid = narrowed(&fx);
    // A judge token for the first narrowing, then no verdict; the repair is revised and narrowed again.
    token(&fx, &aid);
    let o = fx.must(&["amend", "revise", "U-2", "--json"]).json();
    put_replace(&fx, "unit", &unit("U-2", "K-1", "src/f2.txt", &["U-1"], 6));
    let f = fx.root.join("narrow2.json");
    fs::write(&f, json!({"amend": aid, "keep": o["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let n = next(&fx);
    assert!(directives(&n, "judge-repair").is_empty(), "{n}");
    let step = directives(&n, "decide-repair");
    assert_eq!(step.len(), 1, "{n}");
    assert_eq!(step[0]["role"], "human");
    assert_eq!(step[0]["judge"], "failed");
}

#[test]
fn judge_flow_status_shows_a_held_unit_as_not_ready() {
    let fx = Fx::new();
    plan(&fx);
    let aid = fx.must(&["amend", "open", "--repair", "U-1", "--json"]).json()["amend"]
        .as_str()
        .unwrap()
        .to_string();
    let st = fx.must(&["status", "--json"]).json();
    let find = |id: &str| {
        st["units"].as_array().unwrap().iter().find(|u| u["id"] == id).cloned().unwrap()
    };
    let u1 = find("U-1");
    assert_ne!(u1["state"], "ready", "{u1}");
    assert!(u1.to_string().contains(&aid), "{u1}");
    assert_eq!(find("U-3")["state"], "ready");
}

#[test]
fn judge_flow_the_run_page_events_print_the_verdict_with_no_measures() {
    let fx = Fx::new();
    plan(&fx);
    let aid = narrowed(&fx);
    assert_eq!(turn(&fx, &aid, "ask").code, 0);
    let page = fx.must(&["view", "run", "--json"]).json().to_string();
    // The judge's event on the page: the text of the line that holds its verdict.
    let at = page.find("\\\"verdict\\\":\\\"ask").unwrap_or_else(|| panic!("{page}"));
    let end = page[at..].find("]\"").map(|n| at + n).unwrap_or(page.len());
    let start = page[..at].rfind("\"text\"").unwrap_or(0);
    let line = &page[start..end];
    assert!(line.contains("plain words") && line.contains("strong-model"), "{line}");
    assert!(line.contains("contents left out"), "{line}");
    for hidden in ["measures", "fingerprint", "session"] {
        assert!(!line.contains(hidden), "{hidden} in {line}");
    }
}
