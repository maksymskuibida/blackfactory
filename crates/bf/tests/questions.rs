//! Questions asked at the same moment each get their own number.

mod common;
use common::*;
use std::process::Command;

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
