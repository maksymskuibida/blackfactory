//! U-57 (C-17, S-11, TH-11, D-59): `bf doctor` prints the line of a check that has waited more than ten
//! minutes for a place of the heavy checks, with the holder named, and it changes no exit code.

mod common;
use common::*;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// A held place and a waiting ticket whose wait began `age` seconds ago. The files stay locked while the
/// returned handles live.
fn tickets(fx: &Fx, age: u64) -> Vec<fs::File> {
    let dir = bf::evidence::places_dir(&fx.project());
    fs::create_dir_all(&dir).unwrap();
    let place = dir.join("place-0");
    fs::write(
        &place,
        r#"{"pid":4242,"intent":"holder-intent","check":"X-HOLD"}"#,
    )
    .unwrap();
    let wait = dir.join("wait-000000000000000000000000000001-4243");
    fs::write(
        &wait,
        format!(r#"{{"check":"X-WAIT","since":{}}}"#, now() - age),
    )
    .unwrap();
    [place, wait]
        .iter()
        .map(|p| {
            let f = fs::OpenOptions::new().read(true).write(true).open(p).unwrap();
            f.lock().unwrap();
            f
        })
        .collect()
}

#[test]
fn doctor_heavy_wait_eleven_minutes_prints_the_line_with_the_holder() {
    let fx = Fx::new();
    let before = fx.bf(&["doctor"]);
    let _h = tickets(&fx, 660);
    let o = fx.bf(&["doctor"]);
    let text = o.all();
    assert!(text.contains("X-WAIT"), "{text}");
    assert!(text.contains("X-HOLD"), "{text}");
    assert!(text.contains("holder-intent"), "{text}");
    assert_eq!(o.code, before.code);
    let j = fx.bf(&["doctor", "--json"]).json();
    let w = j["heavy_waits"].as_array().unwrap();
    assert_eq!(w.len(), 1, "{j}");
    assert!(w[0].as_str().unwrap().contains("X-HOLD"));
}

#[test]
fn doctor_heavy_wait_nine_minutes_prints_none() {
    let fx = Fx::new();
    let before = fx.bf(&["doctor"]);
    let _h = tickets(&fx, 540);
    let o = fx.bf(&["doctor"]);
    assert!(!o.all().contains("X-WAIT"), "{}", o.all());
    assert_eq!(o.code, before.code);
    let j = fx.bf(&["doctor", "--json"]).json();
    assert!(j["heavy_waits"].as_array().unwrap().is_empty(), "{j}");
}
