//! `bf doctor`: health checks and `--perf` timings.

use crate::events;
use crate::evidence;
use crate::guard;
use crate::plan;
use crate::refusal::R;
use crate::store::Project;
use crate::util;
use serde_json::{json, Value};
use std::time::Instant;

pub fn doctor(p: &Project) -> R<Value> {
    let st = events::replay(p)?;
    let mut checks = Vec::new();
    let mut ok = true;
    let mut add = |name: &str, good: bool, note: String| {
        if !good {
            ok = false;
        }
        checks.push(json!({"check": name, "ok": good, "note": note}));
    };
    add(
        "key",
        evidence::read_key(p).is_some(),
        "the sealing key in the state folder exists".into(),
    );
    add(
        "policy",
        p.policy_state == crate::store::PolicyState::Accepted,
        format!(
            "{:?}: the file on disk is{} the accepted policy",
            p.policy_state,
            if p.policy_state == crate::store::PolicyState::Accepted {
                ""
            } else {
                " not"
            }
        ),
    );
    let sb = p.policy.exec.sandbox && crate::sandbox::available();
    add(
        "sandbox",
        true,
        if sb {
            "checks and git run under sandbox-exec: no state folder, no network, no writes to factory/ or the other working copies".to_string()
        } else {
            "checks and git run WITHOUT a sandbox (not available here, or switched off in the policy); receipts say `sandbox = \"none\"`".to_string()
        },
    );
    add(
        "check path",
        crate::evidence::check_exec_path(p).is_ok(),
        crate::evidence::check_exec_path(p)
            .err()
            .map(|e| e.to_string().replace('\n', " "))
            .unwrap_or_else(|| "the folders on the check PATH are the accepted ones".into()),
    );
    add(
        "log head",
        st.head_error.is_none(),
        st.head_error
            .clone()
            .unwrap_or_else(|| "the log matches the state head".into()),
    );
    let drift = crate::gates::drift(p, &st)?;
    add(
        "frozen records",
        drift.is_empty(),
        if drift.is_empty() {
            "the approved gates' records are as approved".into()
        } else {
            drift.join("; ")
        },
    );
    add(
        "log chain",
        st.chain_ok,
        st.chain_error
            .clone()
            .unwrap_or_else(|| format!("{} events", st.event_count)),
    );
    let table = std::fs::read_to_string(p.guard_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    add(
        "guard table",
        table.is_some(),
        "readable .bf/guard.json".into(),
    );
    let gi = std::fs::read_to_string(p.root.join(".gitignore")).unwrap_or_default();
    add(
        ".gitignore",
        gi.lines().any(|l| l.trim() == ".bf/" || l.trim() == ".bf"),
        ".bf/ is ignored by git".into(),
    );
    add(
        "ignored events",
        st.ignored.is_empty(),
        if st.ignored.is_empty() {
            "none".into()
        } else {
            st.ignored.join("; ")
        },
    );
    let pl = plan::load(p)?;
    add(
        "record files",
        pl.bad.is_empty(),
        if pl.bad.is_empty() {
            "every record file can be read".to_string()
        } else {
            pl.bad
                .iter()
                .map(|(f, e)| format!("{f}: {e}"))
                .collect::<Vec<_>>()
                .join("; ")
        },
    );
    add(
        "plan",
        pl.problems.is_empty(),
        format!("{} problem(s)", pl.problems.len()),
    );
    Ok(json!({"ok": ok, "checks": checks}))
}

fn time_ms<F: FnMut()>(n: usize, mut f: F) -> (f64, f64) {
    let mut v = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        f();
        v.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    (util::median(&mut v.clone()), util::percentile(&mut v, 0.99))
}

/// Median and 99th percentile of each class of command, run in this process,
/// plus `bf guard` as a real child process.
pub fn perf(p: &Project, exe: &std::path::Path) -> R<Value> {
    let mut rows = Vec::new();
    let mut add = |name: &str, (m, p99): (f64, f64)| {
        rows.push(json!({"command": name, "median_ms": (m * 100.0).round() / 100.0, "p99_ms": (p99 * 100.0).round() / 100.0}))
    };
    add(
        "record list",
        time_ms(20, || {
            let _ = p.list_records(None);
        }),
    );
    add(
        "ledger check",
        time_ms(10, || {
            if let Ok(pl) = plan::load(p) {
                let _ = crate::ledger::check(p, &pl, false);
            }
        }),
    );
    add(
        "plan check",
        time_ms(10, || {
            let _ = plan::load(p);
        }),
    );
    add(
        "view index",
        time_ms(5, || {
            let _ = crate::view::build(p, "index", None);
        }),
    );
    add(
        "status",
        time_ms(5, || {
            let _ = crate::run::status(p);
        }),
    );
    let payload = json!({"tool_name": "Write", "tool_input": {"file_path": "src/x.rs"}, "cwd": p.root.to_string_lossy()}).to_string();
    add(
        "guard (in process)",
        time_ms(200, || {
            let _ = guard::run_hook(&payload, &p.root, None);
        }),
    );
    let mut v = Vec::new();
    for _ in 0..30 {
        let t = Instant::now();
        let mut c = std::process::Command::new(exe);
        c.arg("guard")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null());
        if let Ok(mut child) = c.spawn() {
            if let Some(mut si) = child.stdin.take() {
                use std::io::Write;
                let _ = si.write_all(payload.as_bytes());
            }
            let _ = child.wait();
        }
        v.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    add(
        "guard (child process)",
        (util::median(&mut v.clone()), util::percentile(&mut v, 0.99)),
    );
    Ok(json!({"rows": rows, "note": "Times are wall clock on this machine, in this project."}))
}
