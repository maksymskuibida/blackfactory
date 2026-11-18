//! Questions asked at the same moment each get their own number (U-?), and the questions page of each
//! design stage (U-4: R-29, R-53, D-36, S-5).

mod common;
use common::*;
use serde_json::Value;
use std::process::Command;

#[allow(clippy::module_inception)]
mod questions {
    use super::*;

    #[test]
    fn eight_questions_asked_together_are_all_kept() {
        let fx = Fx::new();
        fx.tiny();
        let mut handles = Vec::new();
        for i in 0..8 {
            let root = fx.root.clone();
            let sb = fx.state_base.clone();
            handles.push(std::thread::spawn(move || {
                Command::new(bin())
                    .current_dir(&root)
                    .args([
                        "ask",
                        "--stage",
                        "idea",
                        "--title",
                        &format!("Question {i}?"),
                    ])
                    .env("BF_STATE_HOME", &sb)
                    .output()
                    .map(|o| {
                        (
                            o.status.code(),
                            String::from_utf8_lossy(&o.stderr).to_string(),
                        )
                    })
                    .unwrap()
            }));
        }
        for h in handles {
            let (code, stderr) = h.join().unwrap();
            assert_eq!(code, Some(0), "{stderr}");
        }
        let q = fx
            .bf(&["record", "list", "--kind", "question", "--json"])
            .json();
        let list = q.as_array().unwrap();
        assert_eq!(list.len(), 8, "{q}");
        let mut titles: Vec<String> = list
            .iter()
            .map(|r| r["title"].as_str().unwrap().to_string())
            .collect();
        titles.sort();
        titles.dedup();
        assert_eq!(titles.len(), 8, "{titles:?}");
        let d = fx.bf(&["doctor"]);
        assert!(!d.stdout.contains("PROBLEM"), "{}", d.all());
    }

    fn answer(fx: &Fx, id: &str) {
        fx.human("My answer is A.");
        let f = fx.root.join("ans.json");
        std::fs::write(&f, "{\"answer\": \"fine\"}").unwrap();
        fx.must(&["answer", id, "--file", f.to_str().unwrap()]);
        std::fs::remove_file(&f).unwrap();
    }

    fn view(fx: &Fx, kind: &str, arg: &str) -> Value {
        fx.must(&["view", kind, arg, "--json"]).json()
    }

    #[test]
    fn the_page_of_a_stage_names_its_stage_and_holds_its_questions_only() {
        let fx = Fx::new();
        fx.must(&["ask", "--stage", "idea", "--title", "Who is it for?"]);
        fx.must(&["ask", "--stage", "plan", "--title", "How big?"]);
        let v = view(&fx, "questions", "idea");
        assert_eq!(v["id"], "questions-idea", "{v}");
        let title = v["page"]["title"].as_str().unwrap();
        assert!(title.contains("idea"), "{title}");
        assert!(title.contains(v["project"].as_str().unwrap()), "{title}");
        let crumbs = v["page"]["crumbs"].as_array().unwrap();
        assert_eq!(crumbs.last().unwrap(), "idea");
        let qs = v["questions"].as_array().unwrap();
        assert_eq!(qs.len(), 1, "{v}");
        assert_eq!(qs[0]["title"], "Who is it for?");
    }

    #[test]
    fn an_unanswered_question_shows_no_time_and_an_answered_one_the_recorded_time() {
        let fx = Fx::new();
        fx.must(&["ask", "--stage", "idea", "--title", "Who is it for?"]);
        let v = view(&fx, "questions", "idea");
        assert!(v.get("built_at").is_none(), "{v}");
        assert!(v["questions"][0].get("answered_at").is_none(), "{v}");
        let text = v.to_string();
        assert!(
            !text.contains("T") || !regex_like_time(&text),
            "no time anywhere in the page data: {text}"
        );
        answer(&fx, "Q-1");
        let rec = fx.must(&["record", "get", "Q-1", "--json"]).json();
        let at = rec["answered_at"].as_str().unwrap().to_string();
        let v = view(&fx, "questions", "idea");
        assert_eq!(v["questions"][0]["answered_at"], at.as_str(), "{v}");
    }

    /// A date-time such as `2026-01-01T10:00` anywhere in the text.
    fn regex_like_time(t: &str) -> bool {
        let b = t.as_bytes();
        (0..b.len().saturating_sub(15)).any(|i| {
            b[i..i + 4].iter().all(u8::is_ascii_digit)
                && b[i + 4] == b'-'
                && b[i + 7] == b'-'
                && b[i + 10] == b'T'
                && b[i + 11].is_ascii_digit()
                && b[i + 13] == b':'
        })
    }

    #[test]
    fn the_index_has_one_row_for_each_stage_that_has_questions() {
        let fx = Fx::new();
        fx.must(&["ask", "--stage", "idea", "--title", "Who is it for?"]);
        fx.must(&["ask", "--stage", "plan", "--title", "How big?"]);
        answer(&fx, "Q-2");
        let v = view(&fx, "index", "");
        let sec = v["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "questions")
            .unwrap_or_else(|| panic!("{v}"));
        let hrefs: Vec<&str> = sec["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["href"].as_str().unwrap())
            .collect();
        assert_eq!(hrefs, ["questions-idea.html", "questions-plan.html"]);
        let rows = sec["rows"].as_array().unwrap();
        assert_eq!(rows[0]["handled"]["text"], "open");
        assert_eq!(rows[1]["handled"]["text"], "answered");
        // The stage that waits is in the first section too.
        assert!(
            v["sections"][0]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["href"] == "questions-idea.html"),
            "{v}"
        );
    }

    #[test]
    fn with_only_the_old_page_the_index_has_one_row_without_a_stage() {
        let fx = Fx::new();
        fx.must(&["ask", "--stage", "idea", "--title", "Who is it for?"]);
        fx.must(&[
            "page",
            "set",
            "questions",
            "https://pages.example/questions",
        ]);
        let v = view(&fx, "index", "");
        let sec = v["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "questions")
            .unwrap();
        let rows = sec["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{sec}");
        assert_eq!(rows[0]["href"], "questions.html");
    }

    #[test]
    fn a_units_page_does_not_hide_the_old_page_in_the_index() {
        let fx = Fx::new();
        fx.must(&["ask", "--stage", "idea", "--title", "Who is it for?"]);
        fx.must(&["page", "set", "questions", "https://pages.example/questions"]);
        fx.must(&["page", "set", "questions-U-1", "https://pages.example/u1"]);
        let v = view(&fx, "index", "");
        let sec = v["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "questions")
            .unwrap();
        let rows = sec["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{sec}");
        assert_eq!(rows[0]["href"], "questions.html", "{sec}");
    }

    #[test]
    fn the_conductors_instructions_name_the_stage_page_and_write_no_time() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/claude/skills/factory/asking.md");
        let text = std::fs::read_to_string(&dir)
            .unwrap_or_else(|e| panic!("the asking instructions {dir:?} cannot be read: {e}"));
        assert!(text.contains("questions-<stage>.html"), "render name missing");
        assert!(text.contains("bf page set questions-<stage>"), "record name missing");
        assert!(
            !text.contains("--out .bf/pages/questions.html"),
            "the old page name is still rendered"
        );
        // No step writes a time into the page.
        for bad in ["answered_at", "date -u", "date +", "\"time\":"] {
            assert!(!text.contains(bad), "{bad} writes a time");
        }
    }
}
