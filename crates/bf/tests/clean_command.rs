//! U-90 (D-109, D-114): `bf doctor` shows the disk a run holds and warns at two levels, a unit still
//! starts on a low disk, and `bf clean` removes what is already left and lists what it leaves.

mod common;
use common::*;
use std::fs;
use std::path::PathBuf;

/// The tiny project with a second checkpoint of one more unit, so a unit can be running while the first
/// checkpoint closes.
fn fixture() -> Fx {
    let fx = Fx::tiny_project();
    for (k, t) in [
        ("requirement", "id=\"R-3\"\nkind=\"requirement\"\ntitle=\"A third message\"\nacceptance=[\"src/third.txt exists\"]\n"),
        ("unit", "id=\"U-3\"\nkind=\"unit\"\ntitle=\"Third\"\ndelivers=[\"R-3\"]\nfiles=[\"src/third.txt\"]\ncheckpoint=\"K-2\"\nestimate=1\n"),
        ("check", "id=\"X-4\"\nkind=\"check\"\ntitle=\"Third file\"\nhow=\"command\"\ncommand=\"test -f src/third.txt\"\nproves=[\"R-3\"]\nscope=\"unit\"\nunit=\"U-3\"\n"),
        ("check", "id=\"X-5\"\nkind=\"check\"\ntitle=\"Third file again\"\nhow=\"command\"\ncommand=\"test -f src/third.txt\"\nproves=[\"R-3\"]\nscope=\"checkpoint\"\n"),
        ("checkpoint", "id=\"K-2\"\nkind=\"checkpoint\"\nstop_why=\"Test fixture.\"\ntitle=\"Third\"\nafter=[\"U-3\"]\nproves=[\"R-3\"]\nchecks=[\"X-5\"]\n"),
    ] {
        let o = fx.put(k, t);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    fx.commit_all("more records");
    fx.start_test_run();
    fx
}

fn land(fx: &Fx, id: &str, file: &str, check: &str) {
    fx.build_unit(id, file, check, &["test", "-f", file]);
}

/// U-1 and U-2 landed, K-1 closed, U-3 running with a live builder token. The folders of the landed and
/// closed ones are made again with files of known size. Returns the bytes written.
fn left_over(fx: &Fx) -> u64 {
    land(fx, "U-1", "src/hello.txt", "X-1");
    land(fx, "U-2", "src/bye.txt", "X-2");
    let k = fx.work("K-1");
    fx.must_in(
        &k,
        &[
            "evidence",
            "run",
            "X-3",
            "--",
            "ls",
            "src/hello.txt",
            "src/bye.txt",
        ],
    );
    fx.must(&["checkpoint", "close", "K-1"]);
    let o = fx.must(&["unit", "start", "U-3", "--json"]);
    assert!(PathBuf::from(o.json()["working_copy"].as_str().unwrap()).exists());
    // The builder of U-3 holds a token (the fixture runs with tokens off, so the file is written here).
    fs::write(
        fx.state_dir().join("tokens.json"),
        r#"{"U-3:builder": "token-hash"}"#,
    )
    .unwrap();
    let mut total = 0;
    for (name, n) in [("U-1", 100usize), ("U-2", 200), ("K-1", 300)] {
        let d = fx.work(name);
        assert!(
            !d.exists(),
            "{name} was removed by the landing or the close"
        );
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub/blob.bin"), vec![b'x'; n]).unwrap();
        total += n as u64;
    }
    total
}

fn work_paths(v: &serde_json::Value) -> Vec<String> {
    paths(v, "removed")
        .into_iter()
        .filter(|p| p.starts_with(".bf/work/"))
        .collect()
}

fn work_bytes(v: &serde_json::Value) -> u64 {
    v["removed"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["path"].as_str().unwrap().starts_with(".bf/work/"))
        .map(|x| x["bytes"].as_u64().unwrap())
        .sum()
}

fn paths(v: &serde_json::Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn clean_removes_landed_and_closed_folders_and_leaves_the_running_one() {
    let fx = fixture();
    let written = left_over(&fx);
    let o = fx.must(&["clean", "--json"]);
    let v = o.json();
    let mut removed = work_paths(&v);
    removed.sort();
    assert_eq!(
        removed,
        [".bf/work/K-1", ".bf/work/U-1", ".bf/work/U-2"],
        "{v}"
    );
    assert_eq!(work_bytes(&v), written, "{v}");
    let left = v["left"].as_array().unwrap();
    let u3 = left
        .iter()
        .find(|x| x["path"] == ".bf/work/U-3")
        .expect("U-3 is listed");
    assert!(!u3["reason"].as_str().unwrap().is_empty(), "{v}");
    assert!(fx.work("U-3").exists());
    for n in ["U-1", "U-2", "K-1"] {
        assert!(!fx.work(n).exists(), "{n}");
    }
    // A second run removes nothing.
    let v2 = fx.must(&["clean", "--json"]).json();
    assert!(paths(&v2, "removed").is_empty(), "{v2}");
    assert_eq!(v2["freed_bytes"], 0);
    assert!(fx.work("U-3").exists());
}

#[test]
fn dry_run_lists_the_same_and_removes_nothing() {
    let fx = fixture();
    let written = left_over(&fx);
    let v = fx.must(&["clean", "--dry-run", "--json"]).json();
    assert_eq!(v["dry_run"], true);
    assert_eq!(work_paths(&v).len(), 3, "{v}");
    assert_eq!(work_bytes(&v), written);
    for n in ["U-1", "U-2", "K-1", "U-3"] {
        assert!(fx.work(n).exists(), "{n}");
    }
    let text = fx.must(&["clean", "--dry-run"]).stdout;
    assert!(text.contains(".bf/work/U-1"), "{text}");
    assert!(
        !text.contains(&fx.root.to_string_lossy().to_string()),
        "{text}"
    );
}

#[test]
fn doctor_shows_the_total_and_the_first_warning() {
    let fx = fixture();
    fs::create_dir_all(fx.work("U-9")).unwrap();
    fs::write(fx.work("U-9/blob.bin"), vec![b'x'; 1000]).unwrap();
    let quiet = fx.bf(&["doctor", "--json"]).json();
    assert!(quiet["disk"]["warn"].is_null(), "{quiet}");
    assert!(quiet["disk"]["low"].is_null(), "{quiet}");
    assert!(quiet["disk"]["held_bytes"].as_u64().unwrap() >= 1000);
    fx.set_policy("[limits]\n", "[disk]\nwarn_gb = 0\n\n[limits]\n");
    let v = fx.bf(&["doctor", "--json"]).json();
    assert!(
        v["disk"]["warn"].as_str().unwrap().contains("bf clean"),
        "{v}"
    );
    assert!(v["disk"]["low"].is_null(), "{v}");
    let text = fx.bf(&["doctor"]).stdout;
    assert!(text.contains("bf clean"), "{text}");
    assert!(!text.contains(".bf/work"), "{text}");
}

#[test]
fn doctor_gives_the_stronger_warning_and_a_unit_still_starts() {
    let fx = fixture();
    fx.set_policy(
        "[limits]\n",
        "[disk]\nlow_free_gb = 100000000\n\n[limits]\n",
    );
    let v = fx.bf(&["doctor", "--json"]).json();
    assert!(
        v["disk"]["low"].as_str().unwrap().contains("bf clean"),
        "{v}"
    );
    assert!(v["disk"]["warn"].is_null(), "{v}");
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let j = o.json();
    assert!(
        j["disk_warning"].as_str().unwrap().contains("bf clean"),
        "{j}"
    );
    assert!(PathBuf::from(j["working_copy"].as_str().unwrap()).exists());
}

#[test]
fn the_start_of_a_unit_says_the_warning_to_the_human_too() {
    let fx = fixture();
    fx.set_policy(
        "[limits]\n",
        "[disk]\nlow_free_gb = 100000000\n\n[limits]\n",
    );
    let o = fx.must(&["unit", "start", "U-1"]);
    assert!(o.stdout.contains("bf clean"), "{}", o.stdout);
    assert!(fx.work("U-1").exists());
}

#[test]
fn clean_is_refused_from_a_working_copy() {
    let fx = fixture();
    let o = fx.must(&["unit", "start", "U-1", "--json"]);
    let wc = PathBuf::from(o.json()["working_copy"].as_str().unwrap());
    let r = fx.bf_in(&wc, &["clean"]);
    assert_eq!(r.code, 2, "{}", r.all());
    assert_four_parts(&r.stderr);
}

#[test]
fn clean_also_runs_the_sweep_for_the_briefs_of_landed_units() {
    let fx = fixture();
    left_over(&fx);
    let brief = fx.root.join(".bf/briefs/U-1-builder.md");
    let running = fx.root.join(".bf/briefs/U-3-builder.md");
    fs::create_dir_all(brief.parent().unwrap()).unwrap();
    fs::write(&brief, "old brief\n").unwrap();
    fs::write(&running, "live brief\n").unwrap();
    let dry = fx.must(&["clean", "--dry-run", "--json"]).json();
    assert!(
        !paths(&dry, "removed").iter().any(|p| p.contains("briefs")),
        "{dry}"
    );
    assert!(brief.exists());
    let v = fx.must(&["clean", "--json"]).json();
    assert!(
        paths(&v, "removed").contains(&".bf/briefs/U-1-builder.md".to_string()),
        "{v}"
    );
    assert!(!brief.exists());
    assert!(running.exists());
}

fn left_paths(v: &serde_json::Value) -> Vec<String> {
    paths(v, "left")
}

#[test]
fn a_review_copy_of_an_open_checkpoint_has_one_verdict() {
    let fx = fixture();
    left_over(&fx);
    let review = fx.work("K-2-review");
    fs::create_dir_all(&review).unwrap();
    fs::write(review.join("x.bin"), vec![b'x'; 50]).unwrap();
    let dry = fx.must(&["clean", "--dry-run", "--json"]).json();
    assert!(
        left_paths(&dry).contains(&".bf/work/K-2-review".to_string()),
        "{dry}"
    );
    assert!(
        !paths(&dry, "removed").contains(&".bf/work/K-2-review".to_string()),
        "{dry}"
    );
    assert!(dry["note"].as_str().unwrap().contains("partial"), "{dry}");
    let v = fx.must(&["clean", "--json"]).json();
    assert!(
        paths(&v, "removed").contains(&".bf/work/K-2-review".to_string()),
        "{v}"
    );
    assert!(
        !left_paths(&v).contains(&".bf/work/K-2-review".to_string()),
        "{v}"
    );
    assert!(!review.exists());
    assert!(v["note"].is_null());
}

#[test]
fn a_landed_copy_with_a_live_token_stays_in_a_real_run() {
    let fx = fixture();
    left_over(&fx);
    fs::write(
        fx.state_dir().join("tokens.json"),
        r#"{"U-3:builder": "t", "U-1:reviewer": "t"}"#,
    )
    .unwrap();
    let v = fx.must(&["clean", "--json"]).json();
    assert!(left_paths(&v).contains(&".bf/work/U-1".to_string()), "{v}");
    assert!(
        !paths(&v, "removed").contains(&".bf/work/U-1".to_string()),
        "{v}"
    );
    assert!(fx.work("U-1").exists());
    assert!(!fx.work("U-2").exists());
}

#[test]
fn a_landed_copy_with_uncommitted_work_is_listed_once() {
    let fx = fixture();
    left_over(&fx);
    let wc = fx.work("U-1");
    fs::remove_dir_all(&wc).unwrap();
    let branch = fx.project().unit_branch("U-1");
    git(
        &fx.root,
        &["worktree", "add", &wc.to_string_lossy(), &branch],
    );
    fs::write(wc.join("stray.txt"), "mine\n").unwrap();
    let v = fx.must(&["clean", "--json"]).json();
    let n = left_paths(&v)
        .iter()
        .filter(|p| *p == ".bf/work/U-1")
        .count();
    assert_eq!(n, 1, "{v}");
    assert!(wc.join("stray.txt").exists());
}
