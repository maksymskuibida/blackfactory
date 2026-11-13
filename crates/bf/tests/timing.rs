//! Timing tests. T-1: queries and state changes on 1,000 records. T-11: gate pages.
//! T-12: the guard hook. They print medians and fail only on gross regressions:
//! the thresholds are several times the design targets because the machine may be busy.
//! Run with `cargo test --test timing -- --nocapture --test-threads=1` to see the numbers.

mod common;
use common::*;
use std::fs;
use std::time::Instant;

fn stats(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (
        v[v.len() / 2],
        v[((v.len() as f64 - 1.0) * 0.99).round() as usize],
    )
}

fn time_cmd(fx: &Fx, args: &[&str], n: usize) -> (f64, f64) {
    let mut v = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        let o = fx.bf(args);
        v.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(o.code == 0 || o.code == 2, "{args:?}: {}", o.all());
    }
    stats(&mut v)
}

/// 1,000 records: 500 requirements, 200 components, 100 decisions, 100 units, 100 checks, 1 checkpoint.
fn big(fx: &Fx) {
    let dir = fx.root.join("factory/records");
    fs::create_dir_all(&dir).unwrap();
    let w = |id: &str, body: String| fs::write(dir.join(format!("{id}.toml")), body).unwrap();
    for i in 1..=500 {
        w(&format!("R-{i}"), format!("id=\"R-{i}\"\nkind=\"requirement\"\ntitle=\"Requirement number {i} says one clear thing\"\ntext=\"Some detail for requirement {i}.\"\nacceptance=[\"it works {i}\"]\n"));
    }
    for i in 1..=200 {
        w(
            &format!("C-{i}"),
            format!("id=\"C-{i}\"\nkind=\"component\"\ntitle=\"Component {i}\"\n"),
        );
    }
    for i in 1..=100 {
        w(&format!("D-{i}"), format!("id=\"D-{i}\"\nkind=\"decision\"\ntitle=\"Decision {i}\"\nchosen=\"A\"\nrejected=\"B\"\n"));
    }
    let mut proves = Vec::new();
    for u in 1..=100 {
        let mut del: Vec<String> = (0..5)
            .map(|k| format!("\"R-{}\"", (u - 1) * 5 + k + 1))
            .collect();
        del.extend((0..2).map(|k| format!("\"C-{}\"", (u - 1) * 2 + k + 1)));
        del.push(format!("\"D-{u}\""));
        let del = del.join(",");
        proves.push(del.clone());
        w(&format!("U-{u}"), format!("id=\"U-{u}\"\nkind=\"unit\"\ntitle=\"Unit {u}\"\ndelivers=[{del}]\nfiles=[\"src/u{u}/\"]\ncheckpoint=\"K-1\"\n"));
        w(&format!("X-{u}"), format!("id=\"X-{u}\"\nkind=\"check\"\ntitle=\"Check {u}\"\nhow=\"command\"\ncommand=\"true\"\nproves=[{del}]\nscope=\"unit\"\nunit=\"U-{u}\"\n"));
    }
    let after: Vec<String> = (1..=100).map(|u| format!("\"U-{u}\"")).collect();
    let checks: Vec<String> = (1..=100).map(|u| format!("\"X-{u}\"")).collect();
    w("K-1", format!("id=\"K-1\"\nkind=\"checkpoint\"\ntitle=\"All\"\nafter=[{}]\nproves=[{}]\nchecks=[{}]\n", after.join(","), proves.join(","), checks.join(",")));
}

#[test]
fn t1_queries_and_state_changes_on_1000_records() {
    let fx = Fx::new();
    big(&fx);
    let n = fs::read_dir(fx.root.join("factory/records"))
        .unwrap()
        .count();
    assert_eq!(n, 1001);
    fx.must(&["plan", "check"]);

    // Queries: design target median 50 ms, p99 200 ms. Failing thresholds: 5x and 5x.
    let queries: Vec<(&str, Vec<&str>)> = vec![
        ("record get", vec!["record", "get", "R-250"]),
        (
            "record list --kind unit",
            vec!["record", "list", "--kind", "unit"],
        ),
        ("id new", vec!["id", "new", "requirement"]),
        ("plan check", vec!["plan", "check"]),
        ("ledger check --plan", vec!["ledger", "check", "--plan"]),
        ("ledger impact", vec!["ledger", "impact", "R-7"]),
        ("status", vec!["status"]),
        ("view coverage", vec!["view", "coverage"]),
    ];
    println!("\nT-1 queries, 1,001 records (target median 50 ms, p99 200 ms):");
    for (name, a) in &queries {
        let (m, p99) = time_cmd(&fx, a, 15);
        println!("  {name:<26} median {m:>7.1} ms   p99 {p99:>7.1} ms");
        assert!(m < 250.0, "{name}: median {m:.1} ms is a gross regression");
        assert!(
            p99 < 1000.0,
            "{name}: p99 {p99:.1} ms is a gross regression"
        );
    }

    // State changes: design target median 150 ms, p99 500 ms. Failing thresholds: 5x.
    println!("T-1 state changes (target median 150 ms, p99 500 ms):");
    let mut put = Vec::new();
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..15 {
        let f = tmp.path().join(format!("n{i}.toml"));
        fs::write(
            &f,
            format!(
                "id=\"R-{}\"\nkind=\"requirement\"\ntitle=\"New {i}\"\n",
                1000 + i
            ),
        )
        .unwrap();
        let t = Instant::now();
        let o = fx.bf(&[
            "record",
            "put",
            "requirement",
            "--file",
            f.to_str().unwrap(),
        ]);
        put.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(o.code, 0, "{}", o.all());
    }
    let (m, p99) = stats(&mut put);
    println!(
        "  {:<26} median {m:>7.1} ms   p99 {p99:>7.1} ms",
        "record put"
    );
    assert!(m < 750.0 && p99 < 2500.0, "record put: {m:.1} / {p99:.1}");
    let mut turn = Vec::new();
    for i in 0..15 {
        let t = Instant::now();
        let o = fx.human(&format!("message {i}"));
        turn.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(o.code, 0);
    }
    let (m, p99) = stats(&mut turn);
    println!(
        "  {:<26} median {m:>7.1} ms   p99 {p99:>7.1} ms",
        "human-turn"
    );
    assert!(m < 750.0 && p99 < 2500.0, "human-turn: {m:.1} / {p99:.1}");
    let t = Instant::now();
    fx.must(&["gate", "open", "architecture"]);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "  {:<26} once         {ms:>7.1} ms (snapshots 300 records)",
        "gate open"
    );
    assert!(ms < 2500.0);
}

#[test]
fn t11_gate_pages_are_built_in_under_a_second() {
    let fx = Fx::new();
    big(&fx);
    println!("\nT-11 gate pages on 1,001 records (target < 1 s):");
    for gate in ["requirements", "architecture", "plan"] {
        let mut v = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let o = fx.must(&["view", "gate", gate]);
            v.push(t.elapsed().as_secs_f64() * 1000.0);
            assert!(o.stdout.contains("\"contract\": 1"));
        }
        let (m, _) = stats(&mut v);
        println!("  gate {gate:<14} median {m:>7.1} ms");
        assert!(m < 1000.0, "gate {gate}: {m:.1} ms");
    }
}

#[test]
fn t12_the_guard_hook_is_fast() {
    let fx = Fx::new();
    big(&fx);
    fx.must(&["status"]); // any state change rewrites the table; the status call does not, so put one record
    fx.put(
        "requirement",
        "id=\"R-900\"\nkind=\"requirement\"\ntitle=\"x\"\n",
    );
    let calls = [
        serde_json::json!({"tool_name": "Write", "tool_input": {"file_path": fx.root.join("src/a.rs").to_string_lossy()}, "cwd": fx.root.to_string_lossy()}),
        serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "cargo test && git status | head"}, "cwd": fx.root.to_string_lossy()}),
        serde_json::json!({"tool_name": "Write", "tool_input": {"file_path": fx.root.join("factory/policy.toml").to_string_lossy()}, "cwd": fx.root.to_string_lossy()}),
    ];
    println!("\nT-12 bf guard, one child process per call (target median 30 ms, p99 100 ms):");
    for (i, c) in calls.iter().enumerate() {
        let payload = c.to_string();
        let mut v = Vec::new();
        for _ in 0..60 {
            let t = Instant::now();
            let o = fx.bf_stdin(&fx.root, &["guard"], &payload);
            v.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(o.code, 0);
        }
        let (m, p99) = stats(&mut v);
        println!(
            "  call {i} ({})   median {m:>6.1} ms   p99 {p99:>6.1} ms",
            c["tool_name"]
        );
        assert!(
            m < 100.0,
            "guard median {m:.1} ms is a gross regression (target 30)"
        );
        assert!(
            p99 < 400.0,
            "guard p99 {p99:.1} ms is a gross regression (target 100)"
        );
    }
}

#[test]
fn t12_the_guard_needs_only_stdin_and_one_file() {
    // A directory with nothing but .bf/guard.json: no git repository, no factory folder.
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    fs::create_dir_all(root.join(".bf")).unwrap();
    let table = serde_json::json!({"v": 1, "root": root.to_string_lossy(), "protected": ["factory/"], "deny_commands": [], "worktrees": [], "worker_bf_allow": []});
    fs::write(root.join(".bf/guard.json"), table.to_string()).unwrap();
    let payload = serde_json::json!({"tool_name": "Write", "tool_input": {"file_path": root.join("factory/x").to_string_lossy()}, "cwd": root.to_string_lossy()});
    let fx = Fx::new();
    let o = fx.bf_stdin(&root, &["guard"], &payload.to_string());
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(o.json()["hookSpecificOutput"]["permissionDecision"], "deny");
}

/// Append `n` well-formed, correctly chained events straight to the log and move the head with them.
fn long_log(fx: &Fx, n: usize) {
    use serde_json::{json, Map, Value};
    let p = fx.project();
    let key = p.read_key().unwrap();
    let log = p.log_path();
    let text = fs::read_to_string(&log).unwrap();
    let mut count = text.lines().count();
    let mut prev = text
        .lines()
        .last()
        .map(|l| {
            serde_json::from_str::<Value>(l).unwrap()["h"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .unwrap_or_default();
    let mut out = text;
    for _ in 0..n {
        count += 1;
        let mut m = Map::new();
        m.insert("seq".into(), json!(count));
        m.insert("at".into(), json!("2026-01-01T00:00:00Z"));
        m.insert("kind".into(), json!("note"));
        m.insert("prev".into(), json!(prev));
        m.insert("text".into(), json!(format!("filler event {count}")));
        let h = bf::events::event_hash(&key, &prev, &m);
        m.insert("h".into(), json!(h));
        prev = h;
        out.push_str(&serde_json::to_string(&Value::Object(m)).unwrap());
        out.push('\n');
    }
    fs::write(&log, out).unwrap();
    let mut head = bf::events::read_head(&p).unwrap_or_default();
    head.count = count as u64;
    head.last_hash = prev;
    bf::events::write_head(&p, &head).unwrap();
}

#[test]
fn t13_status_on_a_long_log_stays_quick() {
    let fx = Fx::new();
    long_log(&fx, 20_000);
    assert_eq!(fx.bf(&["log", "--verify"]).code, 0);
    let (med, p99) = time_cmd(&fx, &["status"], 5);
    println!("status on a 20,000-event log: median {med:.0} ms, p99 {p99:.0} ms");
    assert!(med < 1500.0, "status took {med:.0} ms");
    let (med, _) = time_cmd(&fx, &["next"], 3);
    println!("next on a 20,000-event log: median {med:.0} ms");
    assert!(med < 1500.0, "next took {med:.0} ms");
}
