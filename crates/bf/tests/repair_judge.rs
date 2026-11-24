//! U-58: the repair judge (R-69, D-88, D-90): the token, the verdict, the limits of a go, the switch and
//! the replay. The guard's pass is written by the guard itself: a test gets one by sending the hook the
//! payload of a subagent of type `repair-judge` that runs the judge's command.

mod common;
use common::*;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const NEITHER: &str = "neither names its `tests`";

struct Old {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    base: PathBuf,
}

impl Old {
    fn new() -> Old {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let src = fixture("old_project");
        for part in ["project", "state"] {
            let st = Command::new("cp")
                .arg("-R")
                .arg(src.join(part))
                .arg(base.join(part))
                .status()
                .unwrap();
            assert!(st.success());
        }
        Old {
            root: base.join("project"),
            base,
            _tmp: tmp,
        }
    }

    fn run(&self, args: &[&str], stdin: &str) -> (i32, String) {
        use std::io::Write;
        let mut c = Command::new(bin())
            .env("BF_STATE_HOME", self.base.join("state"))
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
        let o = c.wait_with_output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )
    }

    fn must(&self, args: &[&str]) -> String {
        let (c, out) = self.run(args, "");
        assert_eq!(c, 0, "{args:?}: {out}");
        out
    }

    fn human(&self, text: &str) {
        let payload = json!({"session_id": "0b6f1c3e-aaaa-bbbb-cccc-1234567890ab",
            "cwd": self.root.to_string_lossy(), "permission_mode": "default",
            "hook_event_name": "UserPromptSubmit", "prompt": text});
        let (c, out) = self.run(&["human-turn"], &payload.to_string());
        assert_eq!(c, 0, "{out}");
    }

    /// The pass of a judge's turn: the guard writes it for a `repair-judge` payload; the installed role
    /// file is the harness copy.
    fn pass(&self, aid: &str, tok: &str) {
        let d = self.root.join(".claude/agents");
        fs::create_dir_all(&d).unwrap();
        fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../harness/claude/agents/repair-judge.md"),
            d.join("repair-judge.md"),
        )
        .unwrap();
        let payload = json!({"tool_name": "Bash", "cwd": self.root.to_string_lossy(),
            "tool_input": {"command": format!("bf amend judge {aid} --file - --token {tok} --model m")},
            "session_id": "s", "agent_id": "a", "agent_type": "repair-judge"});
        let (c, out) = self.run(&["guard"], &payload.to_string());
        assert_eq!(c, 0, "{out}");
    }

    fn check(&self) -> String {
        self.run(&["plan", "check"], "").1
    }

    fn record(&self, id: &str) -> String {
        fs::read_to_string(self.root.join(format!("factory/records/{id}.toml"))).unwrap()
    }

    fn put(&self, kind: &str, text: &str) {
        let f = self.root.join("put.toml");
        fs::write(&f, text).unwrap();
        self.must(&[
            "record",
            "put",
            kind,
            "--file",
            f.to_str().unwrap(),
            "--replace",
        ]);
        fs::remove_file(f).unwrap();
    }

    /// Open a plan repair over `ids`, let `change` put the new records, narrow and approve it with the
    /// human's words. Returns the approval's amendment ID.
    fn approved_amendment(&self, ids: &[&str], change: impl Fn(&Old)) {
        let mut args = vec!["amend", "open", "--repair"];
        args.extend(ids);
        args.push("--json");
        let opened: Value = serde_json::from_str(&self.must(&args)).unwrap();
        change(self);
        let keep: Vec<String> = opened["impact_ids"]
            .as_array()
            .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
            .unwrap_or_default();
        let aid = opened["amend"]
            .as_str()
            .or_else(|| opened["id"].as_str())
            .unwrap_or("A-1")
            .to_string();
        let f = self.root.join("narrow.json");
        fs::write(
            &f,
            json!({"amend": aid, "keep": keep, "waive": []}).to_string(),
        )
        .unwrap();
        self.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
        fs::remove_file(f).unwrap();
        self.human("Yes, approve the change.");
        self.must(&["amend", "approve", &aid, "--quote", "approve the change"]);
    }

}

fn with_line(rec: &str, from: &str, to: &str) -> String {
    assert!(rec.contains(from), "{rec} lacks {from}");
    rec.replacen(from, to, 1)
}

impl Old {
    fn log(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("factory/log/events.jsonl"))
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
}

#[test]
fn repair_judge_the_a8_repair_is_a_go_on_an_old_project_and_leaves_no_problem() {
    let p = Old::new();
    // An earlier repair of the human adds an independent unit U-3.
    p.approved_amendment(&["U-3", "X-4", "K-1"], |p| {
        p.put("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Third\"\ndelivers=[\"R-2\"]\nfiles=[\"src/c.txt\"]\ncheckpoint=\"K-1\"\nestimate=1\nfull_suite=\"fixture\"\n");
        p.put("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"Third file\"\nhow=\"command\"\ncommand=\"test -f src/c.txt\"\nproves=[\"R-2\"]\nscope=\"unit\"\nunit=\"U-3\"\n");
        let k = with_line(&p.record("K-1"), "\"U-2\",\n", "\"U-2\",\n    \"U-3\",\n");
        p.put("checkpoint", &k);
    });
    // The A-8 repair: the new unit after U-2.
    let opened: Value =
        serde_json::from_str(&p.must(&["amend", "open", "--repair", "U-3", "--json"])).unwrap();
    let r = with_line(
        &p.record("U-3"),
        "estimate = 1\n",
        "depends_on = [\"U-2\"]\nestimate = 1\n",
    );
    p.put("unit", &r);
    let f = p.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-2", "keep": opened["impact_ids"], "waive": []}).to_string(),
    )
    .unwrap();
    p.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let tok: Value =
        serde_json::from_str(&p.must(&["roles", "token", "A-2", "judge", "--json"])).unwrap();
    p.pass("A-2", tok["token"].as_str().unwrap());
    let (c, out) = p.run(
        &[
            "amend", "judge", "A-2", "--file", "-", "--token",
            tok["token"].as_str().unwrap(), "--model", "m",
        ],
        &json!({"verdict": "go", "reasons": "U-5 waits for U-2: one entry added."}).to_string(),
    );
    assert_eq!(c, 0, "{out}");
    let log = p.log();
    let ap = log.iter().rfind(|e| e["kind"] == "amend.approve").unwrap();
    assert_eq!(ap["by"], "judge", "{ap}");
    assert!(ap.get("turn").is_none() && ap.get("tests_rule").is_none(), "{ap}");
    let check = p.check();
    assert!(!check.contains(NEITHER), "{check}");
    assert!(!check.contains("problem"), "{check}");
    assert!(!check.contains("U-1 neither") && !check.contains("U-2 neither"), "{check}");
    let st: Value = serde_json::from_str(&p.must(&["status", "--json"])).unwrap();
    let a2 = st["amendments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "A-2")
        .unwrap();
    assert_eq!(a2["status"], "approved", "{a2}");
    // No human message came after the narrowing of A-2.
    let nar = log.iter().rposition(|e| e["kind"] == "amend.narrow").unwrap();
    let apr = log.iter().rposition(|e| e["kind"] == "amend.approve").unwrap();
    assert!(
        !log[nar..apr].iter().any(|e| e["kind"] == "human.turn"),
        "a human turn came between"
    );
}

#[test]
fn repair_judge_a_human_approval_that_adds_a_plan_problem_is_refused() {
    let p = Old::new();
    let opened: Value =
        serde_json::from_str(&p.must(&["amend", "open", "--repair", "U-1", "--json"])).unwrap();
    let r = with_line(&p.record("U-1"), "src/hello.txt", "src/hello2.txt");
    p.put("unit", &r);
    let f = p.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}).to_string(),
    )
    .unwrap();
    p.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    p.human("Yes, approve the change.");
    let (c, out) = p.run(&["amend", "approve", "A-1", "--quote", "approve the change"], "");
    assert_eq!(c, 2, "{out}");
    assert!(out.contains("U-1") && out.contains(NEITHER), "{out}");
}

#[test]
fn repair_judge_a_human_approval_that_adds_a_plan_problem_through_a_handoff_is_refused() {
    let p = Old::new();
    let opened: Value =
        serde_json::from_str(&p.must(&["amend", "open", "--repair", "H-1", "--json"])).unwrap();
    // The handoff now hands over from a unit to itself: a plan problem that only the handoff causes.
    let h = with_line(&p.record("H-1"), "to = \"U-2\"", "to = \"U-1\"");
    p.put("handoff", &h);
    let f = p.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}).to_string(),
    )
    .unwrap();
    p.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    p.human("Yes, approve the change.");
    let (c, out) = p.run(&["amend", "approve", "A-1", "--quote", "approve the change"], "");
    assert_eq!(c, 2, "{out}");
    assert!(out.contains("H-1") && out.contains("itself"), "{out}");
}

// ---- a project with a plan the human approved ----

fn unit(id: &str, cp: &str, file: &str, deps: &[&str], est: u32) -> String {
    format!(
        "id=\"{id}\"\nkind=\"unit\"\ntitle=\"Unit {id}\"\ndelivers=[\"R-1\",\"C-1\"]\nfiles=[\"{file}\"]\ndepends_on={}\ncheckpoint=\"{cp}\"\nestimate={est}\nfull_suite=\"fixture\"\n",
        json!(deps)
    )
}

fn plan(fx: &Fx) {
    plan_holding(fx, "");
}

/// The plan, with U-3 also holding `held` (a file another unit may be given by a moved entry).
fn plan_holding(fx: &Fx, held: &str) {
    for (k, t) in [
        ("requirement", "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"Things exist\"\nacceptance=[\"they exist\"]\n".to_string()),
        ("component", "id=\"C-1\"\nkind=\"component\"\ntitle=\"Things\"\n".to_string()),
    ] {
        assert_eq!(fx.put(k, &t).code, 0);
    }
    for i in 1..=6 {
        let file = if i == 3 && !held.is_empty() {
            format!("src/f3.txt\", \"{held}")
        } else {
            format!("src/f{i}.txt")
        };
        let t = unit(&format!("U-{i}"), "K-1", &file, &[], 5);
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

fn pass(fx: &Fx, aid: &str) {
    pass_for(fx, aid, "t");
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
    pass(fx, aid);
    pass_for(fx, aid, &t);
    judge(fx, aid, &t, verdict)
}

fn events(fx: &Fx) -> Vec<Value> {
    fs::read_to_string(fx.root.join("factory/log/events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn kinds(fx: &Fx, kind: &str) -> usize {
    events(fx).iter().filter(|e| e["kind"] == kind).count()
}

fn status_of(fx: &Fx, aid: &str) -> String {
    let st = fx.must(&["status", "--json"]).json();
    st["amendments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == aid)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn refused_naming(fx: &Fx, aid: &str, verdict: &str, words: &[&str]) {
    let o = turn(fx, aid, verdict);
    assert_eq!(o.code, 2, "{}", o.all());
    for w in words {
        assert!(o.stderr.contains(w), "{w} missing: {}", o.stderr);
    }
    assert_eq!(status_of(fx, aid), "open");
    let failed = events(fx).into_iter().rfind(|e| e["kind"] == "amend.judge.failed").unwrap();
    assert_eq!(failed["amend"], aid);
}

#[test]
fn repair_judge_a_depends_on_repair_is_approved_by_a_go_with_no_human() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[(
        "unit",
        unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5),
    )]);
    let o = turn(&fx, &aid, "go");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(status_of(&fx, &aid), "approved");
    let ev = events(&fx);
    let ap = ev.iter().rfind(|e| e["kind"] == "amend.approve").unwrap();
    assert_eq!(ap["by"], "judge");
    assert!(ap.get("turn").is_none() && ap.get("tests_rule").is_none(), "{ap}");
    let j = ev.iter().rfind(|e| e["kind"] == "amend.judge").unwrap();
    assert_eq!(ap["judge_seq"], j["seq"]);
    assert_eq!(j["model"], "strong-model");
    assert_eq!(j["session"], "s1");
    assert_eq!(j["measures"]["K-1"]["chain_after"], 1, "{j}");
    fx.must(&["plan", "check"]);
}

#[test]
fn repair_judge_a_second_token_and_a_token_after_a_verdict_are_refused() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    token(&fx, &aid);
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(o.stderr.contains("already issued"), "{}", o.stderr);
    // After a verdict on the latest narrowing a token is refused too.
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    let o = turn(&fx, &aid, "ask");
    assert_eq!(o.code, 0, "{}", o.all());
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(!o.stderr.is_empty(), "{}", o.stderr);
    assert_eq!(kinds(&fx, "amend.judge.token"), 1);
}

#[test]
fn repair_judge_the_switch_off_refuses_the_token() {
    let fx = Fx::tiny_project();
    fx.set_policy("repair_judge = true", "repair_judge = false");
    fx.start_test_run();
    let opened = fx.must(&["amend", "open", "--repair", "U-2", "--json"]).json();
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let o = fx.refused(&["roles", "token", "A-1", "judge"]);
    assert!(o.stderr.contains("repair_judge"), "{}", o.stderr);
    assert_eq!(kinds(&fx, "amend.judge.token"), 0);
}

#[test]
fn repair_judge_a_repair_from_a_design_and_an_amendment_with_a_design_part_get_no_token() {
    let fx = Fx::new();
    plan(&fx);
    let o = fx.must(&["amend", "open", "--repair", "--from-design", "K-1", "U-2", "--json"]).json();
    assert_eq!(o["from_design"], "K-1", "{o}");
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": "A-1", "keep": o["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let t = fx.refused(&["roles", "token", "A-1", "judge"]);
    assert!(t.stderr.contains("the human's for good"), "{}", t.stderr);
    fx.must(&["amend", "withdraw", "A-1", "--reason", "try"]);
    // Not a repair, and a design part.
    let o = fx.must(&["amend", "open", "R-1", "--json"]).json();
    fs::write(&f, json!({"amend": "A-2", "keep": o["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let t = fx.refused(&["roles", "token", "A-2", "judge"]);
    assert!(t.stderr.contains("not a plan repair"), "{}", t.stderr);
}

#[test]
fn repair_judge_a_missing_pass_writes_a_failure_and_no_second_token() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    let t = token(&fx, &aid);
    let o = judge(&fx, &aid, &t, "go");
    assert_eq!(o.code, 2, "{}", o.all());
    assert!(o.stderr.contains("no fresh pass"), "{}", o.stderr);
    assert_eq!(kinds(&fx, "amend.judge.failed"), 1);
    assert_eq!(status_of(&fx, &aid), "open");
    let o = fx.refused(&["roles", "token", &aid, "judge"]);
    assert!(o.stderr.contains("the human's"), "{}", o.stderr);
}

#[test]
fn repair_judge_an_unreadable_verdict_fails_the_turn() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    let t = token(&fx, &aid);
    pass(&fx, &aid);
    pass_for(&fx, &aid, &t);
    let o = fx.bf_stdin(
        &fx.root,
        &["amend", "judge", &aid, "--file", "-", "--token", &t, "--model", "m"],
        "not json",
    );
    assert_eq!(o.code, 2, "{}", o.all());
    assert_eq!(kinds(&fx, "amend.judge.failed"), 1);
}

#[test]
fn repair_judge_an_ask_waits_for_the_human_and_binds_its_records_after_a_withdrawal() {
    let fx = Fx::new();
    plan(&fx);
    let edit = unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5);
    let aid = repair(&fx, &["U-2"], &[("unit", edit.clone())]);
    let o = turn(&fx, &aid, "ask");
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(status_of(&fx, &aid), "open");
    // The human approves it as before.
    fx.must(&["amend", "withdraw", &aid, "--reason", "smaller"]);
    // The same change as a new repair gets no judge token.
    let aid2 = repair(&fx, &["U-2"], &[("unit", edit)]);
    let t = fx.refused(&["roles", "token", &aid2, "judge"]);
    assert!(t.stderr.contains("U-2"), "{}", t.stderr);
}

#[test]
fn repair_judge_an_ask_is_approved_by_the_human_as_before() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    turn(&fx, &aid, "ask");
    fx.human("Yes, approve it.");
    fx.must(&["amend", "approve", &aid, "--quote", "approve it"]);
    assert_eq!(status_of(&fx, &aid), "approved");
}

#[test]
fn repair_judge_an_approval_written_by_hand_does_not_count() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    bf::events::append(
        &fx.project(),
        "amend.approve",
        json!({"amend": aid, "by": "judge", "judge_seq": 1, "content": {}}),
    )
    .unwrap();
    assert_eq!(status_of(&fx, &aid), "open");
    // One that names a verdict `ask` does not count either.
    let t = token(&fx, &aid);
    pass(&fx, &aid);
    pass_for(&fx, &aid, &t);
    let o = judge(&fx, &aid, &t, "ask");
    assert_eq!(o.code, 0, "{}", o.all());
    let ask_seq = events(&fx)
        .iter()
        .rfind(|e| e["kind"] == "amend.judge")
        .unwrap()["seq"]
        .clone();
    bf::events::append(
        &fx.project(),
        "amend.approve",
        json!({"amend": aid, "by": "judge", "judge_seq": ask_seq, "content": {}}),
    )
    .unwrap();
    assert_eq!(status_of(&fx, &aid), "open");
    // A go written by hand with no judge token behind it does not count either.
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    let go = bf::events::append(
        &fx.project(),
        "amend.judge",
        json!({"amend": aid, "verdict": "go", "reasons": "x", "model": "m", "fingerprint": "f", "measures": {}}),
    )
    .unwrap();
    let _ = go;
    let go_seq = events(&fx)
        .iter()
        .rfind(|e| e["kind"] == "amend.judge")
        .unwrap()["seq"]
        .clone();
    bf::events::append(
        &fx.project(),
        "amend.approve",
        json!({"amend": aid, "by": "judge", "judge_seq": go_seq, "content": {}}),
    )
    .unwrap();
    assert_eq!(status_of(&fx, &aid), "open");
}

#[test]
fn repair_judge_a_go_outside_the_limits_is_refused_naming_each_limit() {
    // A changed check command.
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["X-1"], &[(
        "check",
        "id=\"X-1\"\nkind=\"check\"\ntitle=\"First\"\nhow=\"command\"\ncommand=\"false\"\nproves=[\"R-1\",\"C-1\"]\nscope=\"checkpoint\"\n".into(),
    )]);
    refused_naming(&fx, &aid, "go", &["X-1"]);
    // A moved checkpoint.
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-2", "src/f2.txt", &[], 5))]);
    refused_naming(&fx, &aid, "go", &["`checkpoint`"]);
    // A removed unit check.
    let fx = Fx::new();
    plan(&fx);
    let o = fx.must(&["amend", "open", "--repair", "U-2", "--json"]).json();
    fs::remove_file(fx.root.join("factory/records/X-12.toml")).unwrap();
    let aid = o["amend"].as_str().unwrap().to_string();
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": aid, "keep": o["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    refused_naming(&fx, &aid, "go", &["X-12"]);
    // A checkpoint operation: U-2 and its check go onto a new checkpoint.
    let fx = Fx::tiny_run();
    let o = fx.must(&["amend", "open", "--repair", "K-1", "--json"]).json();
    fx.must(&["amend", "checkpoint", "after", "K-1", "U-2"]);
    let aid = o["amend"].as_str().unwrap().to_string();
    let f = fx.root.join("narrow.json");
    fs::write(&f, json!({"amend": aid, "keep": o["impact_ids"], "waive": []}).to_string()).unwrap();
    fx.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    refused_naming(&fx, &aid, "go", &["checkpoint operation"]);
}

#[test]
fn repair_judge_a_protected_path_a_later_checkpoint_and_a_removed_dependency_are_refused() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt\", \"crates/bf/src/judge.rs", &[], 5))]);
    refused_naming(&fx, &aid, "go", &["crates/bf/src/judge.rs", "U-2"]);
    fx.must(&["amend", "withdraw", &aid, "--reason", "no"]);
    // A depends_on entry that names a unit of a later checkpoint.
    let aid = repair(&fx, &["U-3"], &[("unit", unit("U-3", "K-1", "src/f3.txt", &["U-7"], 5))]);
    refused_naming(&fx, &aid, "go", &["U-3", "U-7", "K-2"]);
}

#[test]
fn repair_judge_a_removed_depends_on_entry_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    let a1 = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5))]);
    turn(&fx, &a1, "go");
    assert_eq!(status_of(&fx, &a1), "approved");
    let a2 = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &[], 5))]);
    refused_naming(&fx, &a2, "go", &["U-1", "depends_on"]);
}

#[test]
fn repair_judge_a_fourth_one_unit_wave_in_a_row_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(
        &fx,
        &["U-2", "U-3", "U-4", "U-5"],
        &[
            ("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5)),
            ("unit", unit("U-3", "K-1", "src/f3.txt", &["U-2"], 5)),
            ("unit", unit("U-4", "K-1", "src/f4.txt", &["U-3"], 5)),
            ("unit", unit("U-5", "K-1", "src/f5.txt", &["U-4"], 5)),
        ],
    );
    refused_naming(&fx, &aid, "go", &["one-unit waves", "K-1"]);
}

#[test]
fn repair_judge_three_one_unit_waves_are_allowed() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(
        &fx,
        &["U-2", "U-3", "U-4"],
        &[
            ("unit", unit("U-2", "K-1", "src/f2.txt", &["U-1"], 5)),
            ("unit", unit("U-3", "K-1", "src/f3.txt", &["U-2"], 5)),
            ("unit", unit("U-4", "K-1", "src/f4.txt", &["U-3"], 5)),
        ],
    );
    let o = turn(&fx, &aid, "go");
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn repair_judge_a_scope_change_of_forty_percent_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-1"], &[("unit", unit("U-1", "K-1", "src/f1.txt", &[], 12))]);
    refused_naming(&fx, &aid, "go", &["scope of K-1", "40 percent"]);
}

#[test]
fn repair_judge_a_second_repair_of_twenty_percent_counts_the_first_until_it_is_seen() {
    let fx = Fx::new();
    plan(&fx);
    let a1 = repair(&fx, &["U-1"], &[("unit", unit("U-1", "K-1", "src/f1.txt", &[], 6))]);
    let o = turn(&fx, &a1, "go");
    assert_eq!(o.code, 0, "{}", o.all());
    let a2 = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt", &[], 6))]);
    refused_naming(&fx, &a2, "go", &["scope of K-1", "40 percent"]);
}

#[test]
fn repair_judge_a_moved_file_counts_both_units_whole() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(
        &fx,
        &["U-1", "U-2"],
        &[
            ("unit", unit("U-1", "K-1", "src/f1.txt\", \"src/f2.txt", &[], 5)),
            ("unit", unit("U-2", "K-1", "src/f9.txt", &[], 5)),
        ],
    );
    // f2 moved to U-1, and f9 is a new file of no tests folder: refused for both.
    refused_naming(&fx, &aid, "go", &["src/f9.txt"]);
}

#[test]
fn repair_judge_a_new_test_file_is_allowed() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-1"], &[("unit", unit("U-1", "K-1", "src/f1.txt\", \"crates/bf/tests/new_one.rs", &[], 5))]);
    let o = turn(&fx, &aid, "go");
    assert_eq!(o.code, 0, "{}", o.all());
}

#[test]
fn repair_judge_an_existing_test_file_given_to_a_unit_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    fs::create_dir_all(fx.root.join("crates/bf/tests")).unwrap();
    fs::write(fx.root.join("crates/bf/tests/existing_guard.rs"), "#[test]\nfn holds() {}\n").unwrap();
    fx.commit_all("an existing test file");
    let aid = repair(&fx, &["U-1"], &[("unit", unit("U-1", "K-1", "src/f1.txt\", \"crates/bf/tests/existing_guard.rs", &[], 5))]);
    refused_naming(&fx, &aid, "go", &["crates/bf/tests/existing_guard.rs", "U-1"]);
}

#[test]
fn repair_judge_a_glob_or_a_folder_under_a_tests_folder_is_refused() {
    for path in ["crates/bf/tests/*.rs", "crates/bf/tests/more/"] {
        let fx = Fx::new();
        plan(&fx);
        let file = format!("src/f1.txt\", \"{path}");
        let aid = repair(&fx, &["U-1"], &[("unit", unit("U-1", "K-1", &file, &[], 5))]);
        refused_naming(&fx, &aid, "go", &[path, "U-1"]);
    }
}

#[test]
fn repair_judge_the_token_guard_roles_and_plan_code_are_protected() {
    for path in [
        "crates/bf/src/run/tokens.rs",
        "crates/bf/src/run/roles.rs",
        "crates/bf/src/cli/guard.rs",
        "crates/bf/src/plan/targets.rs",
    ] {
        // The entry is moved from U-3 to U-2, which the move rule allows; only the protected path stops it.
        let fx = Fx::new();
        plan_holding(&fx, path);
        let file = format!("src/f2.txt\", \"{path}");
        let aid = repair(
            &fx,
            &["U-2", "U-3"],
            &[
                ("unit", unit("U-2", "K-1", &file, &[], 5)),
                ("unit", unit("U-3", "K-1", "src/f3.txt", &[], 5)),
            ],
        );
        refused_naming(&fx, &aid, "go", &["overlaps the protected path", path, "U-2"]);
    }
}

#[test]
fn repair_judge_a_glob_under_a_nested_tests_folder_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    let aid = repair(&fx, &["U-1"], &[("unit", unit("U-1", "K-1", "src/f1.txt\", \"crates/bf/tests/sub/*.rs", &[], 5))]);
    refused_naming(&fx, &aid, "go", &["crates/bf/tests/sub/*.rs", "not a new test file"]);
}

#[test]
fn repair_judge_a_test_file_that_exists_only_on_the_checkpoint_branch_is_refused() {
    let fx = Fx::new();
    plan(&fx);
    // The checkpoint branch holds a test file that main does not.
    fx.must(&["unit", "start", "U-1"]);
    let list = git(&fx.root, &["worktree", "list", "--porcelain"]);
    let ck = list
        .split("\n\n")
        .find(|b| b.contains("refs/heads/bf/K-1"))
        .and_then(|b| b.lines().next())
        .map(|l| std::path::PathBuf::from(l.trim_start_matches("worktree ")))
        .expect("the checkpoint worktree");
    fs::create_dir_all(ck.join("crates/bf/tests")).unwrap();
    fs::write(ck.join("crates/bf/tests/only_on_branch.rs"), "#[test]\nfn holds() {}\n").unwrap();
    git(&ck, &["add", "-A"]);
    git(&ck, &["commit", "-q", "-m", "a test file on the checkpoint branch"]);
    assert!(!fx.root.join("crates/bf/tests/only_on_branch.rs").exists());
    let aid = repair(&fx, &["U-2"], &[("unit", unit("U-2", "K-1", "src/f2.txt\", \"crates/bf/tests/only_on_branch.rs", &[], 5))]);
    refused_naming(&fx, &aid, "go", &["crates/bf/tests/only_on_branch.rs", "U-2"]);
}

#[test]
fn repair_judge_a_landed_units_record_is_refused() {
    let p = Old::new();
    let opened: Value =
        serde_json::from_str(&p.must(&["amend", "open", "--repair", "U-1", "--json"])).unwrap();
    let r = with_line(&p.record("U-1"), "estimate = 2", "estimate = 3");
    p.put("unit", &r);
    let f = p.root.join("narrow.json");
    fs::write(
        &f,
        json!({"amend": "A-1", "keep": opened["impact_ids"], "waive": []}).to_string(),
    )
    .unwrap();
    p.must(&["amend", "narrow", "--file", f.to_str().unwrap()]);
    let tok: Value =
        serde_json::from_str(&p.must(&["roles", "token", "A-1", "judge", "--json"])).unwrap();
    p.pass("A-1", tok["token"].as_str().unwrap());
    let (c, out) = p.run(
        &[
            "amend", "judge", "A-1", "--file", "-", "--token",
            tok["token"].as_str().unwrap(), "--model", "m",
        ],
        &json!({"verdict": "go", "reasons": "small."}).to_string(),
    );
    assert_eq!(c, 2, "{out}");
    assert!(out.contains("U-1") && out.contains("has landed"), "{out}");
    assert!(p.log().iter().any(|e| e["kind"] == "amend.judge.failed"));
}
