//! U-4: the engine's advice in the design stages (R-46), the questions page of each stage (R-29) and
//! the proof of the threat model (R-49).

mod common;
use common::Fx;
use serde_json::Value;
use std::fs;

const IDEA: &str = "id=\"I-1\"\nkind=\"idea\"\ntitle=\"A greeting\"\n";
const REQ: &str = "id=\"R-1\"\nkind=\"requirement\"\ntitle=\"A greeting exists\"\nacceptance=[\"src/a.txt exists\"]\n";
const THREAT: &str = "id=\"TH-1\"\nkind=\"threat\"\ntitle=\"A visitor changes a text\"\nprotects=\"The texts\"\nattack=\"A visitor edits a file by hand\"\nmitigation=\"The texts are read-only files\"\n";

fn first(v: &Value) -> &Value {
    &v["directives"][0]
}

fn page(fx: &Fx, name: &str) {
    fx.must(&[
        "page",
        "set",
        name,
        &format!("https://pages.example/{name}"),
    ]);
}

fn ask(fx: &Fx, stage: &str, title: &str) {
    fx.must(&["ask", "--stage", stage, "--title", title]);
}

mod design_advice {
    use super::*;

    #[test]
    fn next_before_a_run_names_the_first_gate_with_the_designers_tier() {
        let fx = Fx::new();
        let n = fx.must(&["next", "--json"]).json();
        let d = first(&n);
        assert!(n["run"].is_null(), "{n}");
        assert_eq!(d["action"], "add-records", "{n}");
        assert_eq!(d["gate_kind"], "idea");
        assert_eq!(
            d["tier"], "strong",
            "the designer's tier, not the conductor's"
        );
        assert_eq!(d["effort"], "high");
        assert!(!n.to_string().contains("bf run start"), "{n}");
    }

    #[test]
    fn next_offers_to_open_then_to_decide_the_gate() {
        let fx = Fx::new();
        assert_eq!(fx.put("idea", IDEA).code, 0);
        let n = fx.must(&["next", "--json"]).json();
        assert_eq!(first(&n)["action"], "open-gate", "{n}");
        assert_eq!(first(&n)["gate_kind"], "idea");
        fx.must(&["gate", "open", "idea"]);
        let n = fx.must(&["next", "--json"]).json();
        assert_eq!(first(&n)["action"], "decide-gate", "{n}");
        assert_eq!(first(&n)["gate_kind"], "idea");
    }

    #[test]
    fn an_open_question_comes_before_the_gate_with_its_stage_page() {
        let fx = Fx::new();
        assert_eq!(fx.put("idea", IDEA).code, 0);
        ask(&fx, "idea", "Who is it for?");
        page(&fx, "questions-idea");
        page(&fx, "questions-plan");
        let n = fx.must(&["next", "--json"]).json();
        let d = first(&n);
        assert_eq!(d["action"], "answer-question", "{n}");
        assert_eq!(d["page_url"], "https://pages.example/questions-idea");
        assert!(
            !n.to_string().contains("open-gate"),
            "no directive to open the gate: {n}"
        );
    }

    #[test]
    fn a_page_of_another_stage_is_not_given() {
        let fx = Fx::new();
        assert_eq!(fx.put("idea", IDEA).code, 0);
        ask(&fx, "idea", "Who is it for?");
        page(&fx, "questions-plan");
        let n = fx.must(&["next", "--json"]).json();
        assert_eq!(first(&n)["action"], "answer-question", "{n}");
        assert!(first(&n)["page_url"].is_null(), "{n}");
    }

    #[test]
    fn the_page_of_the_old_name_is_still_found() {
        let fx = Fx::new();
        assert_eq!(fx.put("idea", IDEA).code, 0);
        ask(&fx, "idea", "Who is it for?");
        page(&fx, "questions");
        let n = fx.must(&["next", "--json"]).json();
        assert_eq!(first(&n)["page_url"], "https://pages.example/questions");
    }

    #[test]
    fn a_workers_question_has_a_page_only_when_one_was_recorded_for_its_unit() {
        let fx = Fx::new();
        fx.tiny();
        fx.start_test_run();
        fx.must(&["ask", "--unit", "U-1", "--title", "Which tone?"]);
        page(&fx, "questions-idea");
        let n = fx.must(&["next", "--json"]).json();
        let q = n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["action"] == "answer-question")
            .unwrap()
            .clone();
        assert!(q["page_url"].is_null(), "{n}");
        page(&fx, "questions-U-1");
        let n = fx.must(&["next", "--json"]).json();
        let q = n["directives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["action"] == "answer-question")
            .unwrap()
            .clone();
        assert_eq!(q["page_url"], "https://pages.example/questions-U-1");
    }

    #[test]
    fn doctor_says_no_plan_is_expected_yet() {
        let fx = Fx::new();
        // A requirement with no unit: a problem only once a plan is expected.
        let o = fx.put("requirement", REQ); assert_eq!(o.code, 0, "{}", o.all());
        let d = fx.bf(&["doctor", "--json"]).json();
        let plan = d["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["check"] == "plan")
            .unwrap();
        assert_eq!(plan["ok"], true, "{plan}");
        assert!(
            plan["note"]
                .as_str()
                .unwrap()
                .contains("no plan is expected yet"),
            "{plan}"
        );
    }

    #[test]
    fn the_threat_proof_rests_on_content_and_no_file_time() {
        let fx = Fx::new();
        assert_eq!(fx.put("threat", THREAT).code, 0);
        let p = fx.project();
        assert!(!bf::enforce::threat_model_proven(&p).unwrap());
        fx.must(&[
            "report",
            "design",
            "--result",
            "built",
            "--role",
            "security-expert",
            "--token",
            "x",
        ]);
        assert!(bf::enforce::threat_model_proven(&p).unwrap());
        // A clone or a branch switch changes the times of the files, not their content.
        let f = p.record_path("TH-1").unwrap();
        for t in [
            std::time::SystemTime::now() + std::time::Duration::from_secs(7200),
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000),
        ] {
            let h = fs::OpenOptions::new().write(true).open(&f).unwrap();
            h.set_modified(t).unwrap();
            assert!(bf::enforce::threat_model_proven(&p).unwrap());
        }
        // A change of the content undoes the proof.
        let text = fs::read_to_string(&f).unwrap();
        fs::write(&f, text.replace("read-only", "writable")).unwrap();
        assert!(!bf::enforce::threat_model_proven(&p).unwrap());
    }

    #[test]
    fn the_threat_proof_survives_a_fresh_clone() {
        let fx = Fx::new();
        assert_eq!(fx.put("threat", THREAT).code, 0);
        fx.must(&[
            "report",
            "design",
            "--result",
            "built",
            "--role",
            "security-expert",
            "--token",
            "x",
        ]);
        fx.commit_all("threats");
        let dest = fx.root.parent().unwrap().join("clone");
        common::git(
            fx.root.parent().unwrap(),
            &["clone", fx.root.to_str().unwrap(), dest.to_str().unwrap()],
        );
        assert!(
            dest.join("factory/records").exists(),
            "the clone has the records"
        );
        let mut p = bf::store::Project::discover(&dest).unwrap();
        p.state_base = Some(fx.state_base.clone());
        p.reload_policy().unwrap();
        assert!(bf::enforce::threat_model_proven(&p).unwrap());
    }

    #[test]
    fn the_records_of_an_amendment_carry_the_amenders_tier_and_effort() {
        let fx = Fx::new();
        let pol = fx.root.join("factory/policy.toml");
        let text = fs::read_to_string(&pol).unwrap();
        let text = text
            .replace("amender = \"strong\"", "amender = \"standard\"")
            .replace("amender = \"high\"", "amender = \"low\"");
        fs::write(&pol, text).unwrap();
        fx.human("Yes, I accept the new policy.");
        fx.must(&["policy", "accept", "--quote", "I accept the new policy"]);
        let o = fx.put("requirement", REQ); assert_eq!(o.code, 0, "{}", o.all());
        let n = fx.must(&["next", "--json"]).json();
        assert_eq!(first(&n)["action"], "add-records", "{n}");
        assert_eq!(first(&n)["tier"], "strong", "{n}");
        assert_eq!(first(&n)["effort"], "high", "{n}");
        fx.must(&["amend", "open", "R-1"]);
        let n = fx.must(&["next", "--json"]).json();
        assert_eq!(first(&n)["action"], "add-records", "{n}");
        assert_eq!(first(&n)["tier"], "standard", "{n}");
        assert_eq!(first(&n)["effort"], "low", "{n}");
    }
}
