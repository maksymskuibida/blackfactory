//! U-52: the tests rule of a plan is decided per unit (R-62, D-59, J-9). A plan approved before the
//! rule keeps its units as they were when a later amendment names one unit's tests or adds `PL-1`; a
//! unit is new only when an approval that carries the marker added it or changed its `files`, `text`
//! or `delivers`. Made from the reviewer's probe of K-1 round 3 (receipt rc-00455).

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

    #[allow(dead_code)]
    /// Make the last approval read as one from an engine before the marker: drop `tests_rule` from the
    /// last event and seal the line again with the project's key.
    fn strip_marker(&self) {
        let log = self.root.join("factory/log/events.jsonl");
        let text = fs::read_to_string(&log).unwrap();
        let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        let mut ev: serde_json::Map<String, Value> =
            serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(ev["kind"], "amend.approve");
        assert_eq!(ev.remove("tests_rule"), Some(json!(true)));
        let dir = fs::read_dir(self.base.join("state"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let key = fs::read_to_string(dir.join("key")).unwrap();
        let prev = ev["prev"].as_str().unwrap().to_string();
        let h = bf::events::event_hash(key.trim(), &prev, &ev);
        ev.insert("h".into(), json!(h));
        *lines.last_mut().unwrap() = serde_json::to_string(&Value::Object(ev)).unwrap();
        fs::write(&log, lines.join("\n") + "\n").unwrap();
        let hp = dir.join("head.json");
        let mut head: Value = serde_json::from_str(&fs::read_to_string(&hp).unwrap()).unwrap();
        head["last_hash"] = json!(h);
        fs::write(&hp, head.to_string()).unwrap();
    }

    /// Open a plan repair over `ids`, let `change` put the new records and narrow it. Returns the
    /// amendment's ID.
    fn narrowed_amendment(&self, ids: &[&str], change: impl Fn(&Old)) -> String {
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
        aid
    }

    /// The human says yes and approves `aid`: the exit code and the output.
    fn try_approve(&self, aid: &str) -> (i32, String) {
        self.human("Yes, approve the change.");
        self.run(
            &["amend", "approve", aid, "--quote", "approve the change"],
            "",
        )
    }

    /// Open, change and narrow a repair, and see the engine refuse its approval. Returns the refusal.
    fn refused_amendment(&self, ids: &[&str], change: impl Fn(&Old)) -> String {
        let aid = self.narrowed_amendment(ids, change);
        let (c, out) = self.try_approve(&aid);
        assert_ne!(c, 0, "the approval of {aid} was not refused: {out}");
        out
    }

    /// Seals an approval by hand, with the `tests_rule` marker, so the engine's own refusal does not apply.
    /// Seal an approval of `aid` into the log by hand, as an engine would have written it, with the
    /// tests-rule marker or without it.
    fn seal_approval(&self, aid: &str, marker: bool) {
        let log = self.root.join("factory/log/events.jsonl");
        let text = fs::read_to_string(&log).unwrap();
        let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        let events: Vec<Value> = lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let turn = events
            .iter()
            .rev()
            .find(|e| e["kind"] == "human.turn")
            .map(|e| e["seq"].clone())
            .unwrap();
        let last = events.last().unwrap();
        let prev = last["h"].as_str().unwrap().to_string();
        let seq = last["seq"].as_u64().unwrap() + 1;
        let mut ev = serde_json::Map::new();
        ev.insert("kind".into(), json!("amend.approve"));
        ev.insert("amend".into(), json!(aid));
        ev.insert("turn".into(), turn);
        ev.insert("quote".into(), json!("approve the change"));
        ev.insert("seq".into(), json!(seq));
        ev.insert("at".into(), last["at"].clone());
        ev.insert("prev".into(), json!(prev));
        if marker {
            ev.insert("tests_rule".into(), json!(true));
        }
        let dir = fs::read_dir(self.base.join("state"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let key = fs::read_to_string(dir.join("key")).unwrap();
        let h = bf::events::event_hash(key.trim(), &prev, &ev);
        ev.insert("h".into(), json!(h));
        lines.push(serde_json::to_string(&Value::Object(ev)).unwrap());
        fs::write(&log, lines.join("\n") + "\n").unwrap();
        let hp = dir.join("head.json");
        let mut head: Value = serde_json::from_str(&fs::read_to_string(&hp).unwrap()).unwrap();
        head["last_hash"] = json!(h);
        head["count"] = json!(seq);
        fs::write(&hp, head.to_string()).unwrap();
    }
}

fn with_line(rec: &str, from: &str, to: &str) -> String {
    assert!(rec.contains(from), "{rec} lacks {from}");
    rec.replacen(from, to, 1)
}

fn faulted(out: &str, unit: &str) -> bool {
    out.contains(&format!("{unit} {NEITHER}"))
}

#[test]
fn plan_old_units_the_old_plan_reads_as_today() {
    let p = Old::new();
    let out = p.check();
    assert!(!out.contains(NEITHER), "{out}");
}

#[test]
fn plan_old_units_pl1_added_by_an_amendment_leaves_the_old_units_old() {
    let p = Old::new();
    p.approved_amendment(&["PL-1"], |p| {
        p.put(
            "plan_settings",
            "id=\"PL-1\"\nkind=\"plan_settings\"\ntitle=\"Settings\"\nheavy_limit=1\n",
        );
    });
    let out = p.check();
    assert!(!faulted(&out, "U-1"), "{out}");
    assert!(!faulted(&out, "U-2"), "{out}");
}

#[test]
fn plan_old_units_a_unit_whose_files_a_marked_approval_changed_is_a_problem_alone() {
    let p = Old::new();
    // The approval carries the marker, so U-1, whose files it changes, counts as new: the engine
    // refuses an approval that leaves it with neither `tests` nor `full_suite`, and U-2 is not named.
    let out = p.refused_amendment(&["U-1"], |p| {
        let r = with_line(&p.record("U-1"), "src/hello.txt", "src/hello2.txt");
        p.put("unit", &r);
    });
    assert!(faulted(&out, "U-1"), "{out}");
    assert!(!faulted(&out, "U-2"), "{out}");
    let c = p.check();
    assert!(!faulted(&c, "U-1") && !faulted(&c, "U-2"), "{c}");
}

#[test]
fn plan_old_units_a_change_of_depends_on_alone_leaves_the_unit_old() {
    let p = Old::new();
    // U-1's files change (U-1 is new, and refused for having no tests); the change of U-2's
    // `depends_on` alone leaves U-2 old, so the refusal never names it that way. It may also list the
    // handoff problem that the dropped dependency makes (H-1).
    let out = p.refused_amendment(&["U-1", "U-2"], |p| {
        let r = with_line(&p.record("U-1"), "src/hello.txt", "src/hello2.txt");
        p.put("unit", &r);
        let r = with_line(
            &p.record("U-2"),
            "depends_on = [\"U-1\"]\n",
            "depends_on = []\n",
        );
        p.put("unit", &r);
    });
    assert!(faulted(&out, "U-1"), "{out}");
    assert!(!faulted(&out, "U-2"), "{out}");
}

#[test]
fn plan_old_units_a_change_by_an_approval_without_the_marker_leaves_the_unit_old() {
    // The marker alone makes the difference: the same narrowing, approved once with it and once
    // without it, by an approval sealed into the log by hand.
    for marker in [true, false] {
        let p = Old::new();
        let aid = p.narrowed_amendment(&["U-1"], |p| {
            let r = with_line(&p.record("U-1"), "src/hello.txt", "src/hello2.txt");
            p.put("unit", &r);
        });
        if marker {
            // With the marker the engine itself refuses, naming U-1.
            let (c, out) = p.try_approve(&aid);
            assert_ne!(c, 0, "{out}");
            assert!(faulted(&out, "U-1"), "{out}");
        } else {
            p.human("Yes, approve the change.");
        }
        p.seal_approval(&aid, marker);
        let out = p.check();
        assert_eq!(faulted(&out, "U-1"), marker, "marker {marker}: {out}");
    }
}
