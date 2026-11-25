//! U-70: the repair judge under the free rules of the human-absent flag (R-73, D-95): the mode fixed with
//! the token, the three turns, `withdraw-repair`, the one open place and `bf amend seen` for several.

use crate::repair_judge::common;
use crate::repair_judge::{
    events, judge, kinds, pass, pass_for, planned, repair, status_of, token, unit,
};
use common::*;
use serde_json::{json, Value};
use std::fs;

const CMD: &str = "/factory:human-absent";

fn say(fx: &Fx, session: &str, text: &str) {
    let payload = json!({"session_id": session, "cwd": fx.root.to_string_lossy(),
        "permission_mode": "default", "hook_event_name": "UserPromptSubmit", "prompt": text});
    let o = fx.bf_stdin(&fx.root, &["human-turn"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
}

fn set_flag(fx: &Fx) {
    say(fx, "c1", CMD);
}

fn release(fx: &Fx) {
    say(fx, "c1", "hello again");
    say(fx, "c1", "yes");
}

/// A unit record with a title of its own.
fn titled(id: &str, title: &str) -> String {
    unit(id, "K-1", &format!("src/f{}.txt", &id[2..]), &[], 5).replace(
        &format!("Unit {id}"),
        title,
    )
}

/// A repair that rewrites the title of `U-2`: the field list of the usual mode refuses it.
fn retitle(fx: &Fx, title: &str) -> String {
    repair(fx, &["U-2"], &[("unit", titled("U-2", title))])
}

/// The turn of a judge with the verdict, as `turn` of the judge's own tests, returning the output.
fn give(fx: &Fx, aid: &str, verdict: &str) -> Out {
    let t = token(fx, aid);
    pass(fx, aid);
    pass_for(fx, aid, &t);
    judge(fx, aid, &t, verdict)
}

fn next_directives(fx: &Fx, action: &str) -> Vec<Value> {
    fx.must(&["next", "--json"]).json()["directives"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d["action"] == action)
        .collect()
}

#[test]
fn absent_judge_the_token_carries_the_mode_of_the_flag() {
    let fx = planned();
    let aid = retitle(&fx, "Reworded");
    let o = fx.must(&["roles", "token", &aid, "judge", "--json"]).json();
    assert_eq!(o["mode"], "present", "{o}");
    let ev = events(&fx);
    let t = ev.iter().rfind(|e| e["kind"] == "amend.judge.token").unwrap();
    assert_eq!(t["mode"], "");
    let fx = planned();
    set_flag(&fx);
    let aid = retitle(&fx, "Reworded");
    let o = fx.must(&["roles", "token", &aid, "judge", "--json"]).json();
    assert_eq!(o["mode"], "absent", "{o}");
    let t = events(&fx).into_iter().rfind(|e| e["kind"] == "amend.judge.token").unwrap();
    assert_eq!(t["mode"], "absent");
    assert_eq!(next_directives(&fx, "judge-repair").len(), 0, "the token is issued");
}

#[test]
fn absent_judge_a_reworded_title_is_refused_in_the_usual_mode_and_approved_in_mode_absent() {
    let fx = planned();
    let aid = retitle(&fx, "Reworded");
    let o = give(&fx, &aid, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    let fx = planned();
    set_flag(&fx);
    let aid = retitle(&fx, "Reworded");
    let o = give(&fx, &aid, "go");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(status_of(&fx, &aid), "approved");
    let ev = events(&fx);
    let ap = ev.iter().rfind(|e| e["kind"] == "amend.approve").unwrap();
    assert_eq!(ap["by"], "judge");
    assert_eq!(ap["mode"], "absent");
    assert!(ap.get("tests_rule").is_none() && ap.get("turn").is_none(), "{ap}");
    let j = ev.iter().rfind(|e| e["kind"] == "amend.judge").unwrap();
    assert_eq!(j["mode"], "absent");
    assert_eq!(j["measures"]["reworded"], json!(["U-2"]));
    fx.must(&["plan", "check"]);
}

#[test]
fn absent_judge_a_changed_check_command_is_refused_in_both_modes() {
    for flag in [false, true] {
        let fx = planned();
        if flag {
            set_flag(&fx);
        }
        let check = "id=\"X-12\"\nkind=\"check\"\ntitle=\"Unit 2\"\nhow=\"command\"\ncommand=\"false\"\nproves=[\"R-1\"]\nscope=\"unit\"\nunit=\"U-2\"\n";
        let aid = repair(&fx, &["X-12"], &[("check", check.to_string())]);
        let o = give(&fx, &aid, "go");
        assert_eq!(o.code, 2, "flag {flag}: {}", o.all());
    }
}

/// Three turns on one amendment: ask, revise, narrow, ask, revise, narrow, ask. The flag is set.
fn three_asks(fx: &Fx) -> String {
    set_flag(fx);
    let aid = retitle(fx, "One");
    assert_eq!(give(fx, &aid, "ask").code, 0);
    for title in ["Two", "Three"] {
        let rev = fx.must(&["amend", "revise", "U-2", "--json"]).json();
        let f = fx.root.join("edit.toml");
        fs::write(&f, titled("U-2", title)).unwrap();
        fx.must(&["record", "put", "unit", "--file", f.to_str().unwrap(), "--replace"]);
        let n = fx.root.join("narrow.json");
        fs::write(&n, json!({"amend": aid, "keep": rev["impact_ids"], "waive": []}).to_string()).unwrap();
        fx.must(&["amend", "narrow", "--file", n.to_str().unwrap()]);
        assert_eq!(give(fx, &aid, "ask").code, 0);
    }
    aid
}

#[test]
fn absent_judge_an_ask_leaves_two_more_turns_and_the_third_makes_it_the_humans() {
    let fx = planned();
    let aid = three_asks(&fx);
    assert_eq!(kinds(&fx, "amend.judge.token"), 3);
    // Three turns with no go: the repair is the human's, and no fourth token is given.
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(!o.stderr.is_empty());
    let d = next_directives(&fx, "withdraw-repair");
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["amendment"], json!(aid));
}

#[test]
fn absent_judge_the_usual_mode_has_one_turn_and_no_withdraw_directive() {
    let fx = planned();
    let aid = retitle(&fx, "First");
    assert_eq!(give(&fx, &aid, "ask").code, 0);
    assert!(next_directives(&fx, "withdraw-repair").is_empty());
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(!o.stderr.is_empty());
}

#[test]
fn absent_judge_a_verdict_after_the_release_is_not_taken() {
    let fx = planned();
    set_flag(&fx);
    let aid = retitle(&fx, "Reworded");
    let t = token(&fx, &aid);
    pass(&fx, &aid);
    pass_for(&fx, &aid, &t);
    release(&fx);
    let before = events(&fx).len();
    let o = judge(&fx, &aid, &t, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("usual mode"), "{}", o.stderr);
    assert_eq!(events(&fx).len(), before, "nothing was written");
    assert_eq!(status_of(&fx, &aid), "open");
    // The repair is judged again in the usual mode, and the voided turn counts none.
    let o = fx.must(&["roles", "token", &aid, "judge", "--json"]).json();
    assert_eq!(o["mode"], "present", "{o}");
}

#[test]
fn absent_judge_open_without_repair_or_from_design_is_refused_while_the_flag_is_on() {
    let fx = planned();
    set_flag(&fx);
    let o = fx.refused(&["amend", "open", "R-1"]);
    assert!(o.stderr.contains("bf ask"), "{}", o.stderr);
    let o = fx.refused(&["amend", "open", "--repair", "--from-design", "K-1", "U-2"]);
    assert!(o.stderr.contains("bf ask"), "{}", o.stderr);
    fx.must(&["amend", "open", "--repair", "U-2"]);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "x"]);
    release(&fx);
    fx.must(&["amend", "open", "R-1"]);
}

#[test]
fn absent_judge_a_go_written_by_hand_with_no_judge_event_does_not_count() {
    let fx = planned();
    set_flag(&fx);
    let aid = retitle(&fx, "Reworded");
    let p = fx.project();
    bf::events::append(
        &p,
        "amend.approve",
        json!({"amend": aid, "by": "judge", "judge_seq": 1, "mode": "absent"}),
    )
    .unwrap();
    assert_eq!(status_of(&fx, &aid), "open");
}

#[test]
fn absent_judge_seen_takes_several_amendments_or_all_on_one_quote() {
    let fx = planned();
    set_flag(&fx);
    let a1 = retitle(&fx, "One");
    assert_eq!(give(&fx, &a1, "go").code, 0);
    let a2 = repair(&fx, &["U-3"], &[("unit", titled("U-3", "Two"))]);
    assert_eq!(give(&fx, &a2, "go").code, 0);
    say(&fx, "c1", "I saw both repairs");
    fx.must(&["amend", "seen", &a1, &a2, "--quote", "I saw both repairs"]);
    assert_eq!(kinds(&fx, "amend.judge.seen"), 1);
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.judged_unseen.is_empty(), "{:?}", st.judged_unseen);
    // `--all` takes every one not yet seen.
    let a3 = repair(&fx, &["U-4"], &[("unit", titled("U-4", "Three"))]);
    assert_eq!(give(&fx, &a3, "go").code, 0);
    say(&fx, "c1", "I saw it all");
    fx.must(&["amend", "seen", "--all", "--quote", "I saw it all"]);
    let st = bf::events::replay(&fx.project()).unwrap();
    assert!(st.judged_unseen.is_empty());
    assert!(st.judged_absent.contains(&a3));
}

#[test]
fn absent_judge_a_repair_that_can_get_no_token_is_told_to_withdraw() {
    let fx = planned();
    let first = three_asks(&fx);
    fx.must(&["amend", "withdraw", &first, "--reason", "the human's"]);
    // A fourth repair that changes a record of the three failed ones gets no token.
    let aid = retitle(&fx, "Fourth");
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(o.stderr.contains("not seen") || o.stderr.contains("unseen") || !o.stderr.is_empty());
    let d = next_directives(&fx, "withdraw-repair");
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["amendment"], json!(aid));
    assert!(d[0]["reason"].as_str().unwrap().contains("U-2"), "{d:?}");
}

#[test]
fn absent_judge_a_token_voided_by_a_release_stays_void_when_the_flag_is_set_again() {
    let fx = planned();
    set_flag(&fx);
    let aid = retitle(&fx, "Reworded");
    let t = token(&fx, &aid);
    release(&fx);
    set_flag(&fx);
    pass(&fx, &aid);
    pass_for(&fx, &aid, &t);
    let o = judge(&fx, &aid, &t, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(status_of(&fx, &aid), "open");
    assert_eq!(kinds(&fx, "amend.judge"), 0);
}

#[test]
fn absent_judge_a_running_third_turn_is_not_told_to_withdraw() {
    let fx = planned();
    set_flag(&fx);
    let aid = retitle(&fx, "One");
    assert_eq!(give(&fx, &aid, "ask").code, 0);
    for title in ["Two", "Three"] {
        let rev = fx.must(&["amend", "revise", "U-2", "--json"]).json();
        let f = fx.root.join("edit.toml");
        fs::write(&f, titled("U-2", title)).unwrap();
        fx.must(&["record", "put", "unit", "--file", f.to_str().unwrap(), "--replace"]);
        let n = fx.root.join("narrow.json");
        fs::write(&n, json!({"amend": aid, "keep": rev["impact_ids"], "waive": []}).to_string()).unwrap();
        fx.must(&["amend", "narrow", "--file", n.to_str().unwrap()]);
        if title == "Two" {
            assert_eq!(give(&fx, &aid, "ask").code, 0);
        }
    }
    // The third token is issued and its judge is running, within the idle limit.
    let t = token(&fx, &aid);
    assert_eq!(kinds(&fx, "amend.judge.token"), 3);
    assert!(next_directives(&fx, "withdraw-repair").is_empty());
    // Its verdict may still be a go.
    pass(&fx, &aid);
    pass_for(&fx, &aid, &t);
    let o = judge(&fx, &aid, &t, "go");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(status_of(&fx, &aid), "approved");
}
