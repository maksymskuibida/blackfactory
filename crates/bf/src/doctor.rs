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
    for r in &p.repos {
        if r.configured != r.branch {
            add(
                "main branch",
                false,
                format!(
                    "the project files say `{}` for the repository `{}` but the engine watches `{}`, the branch it recorded at the start. If the change is on purpose, the human says so in chat and `bf head accept --quote \"<their words>\"` takes it",
                    r.configured, r.name, r.branch
                ),
            );
        }
    }
    add(
        "human turns",
        true,
        match st.human_turns_note() {
            Some(n) => format!("note: {n}"),
            None => format!(
                "{} message(s) of the human recorded or rejected",
                st.human_events
            ),
        },
    );
    if p.workspace && !p.nested_repo_lines().is_empty() {
        let missing = p.missing_excludes();
        if missing.is_empty() {
            add(
                "nested repositories",
                true,
                "the code repositories inside the workspace are in .git/info/exclude".to_string(),
            );
        } else {
            // Put back (a fresh clone of the workspace has no exclude file), then say so.
            let put = p.ensure_excludes().unwrap_or_default();
            add(
                "nested repositories",
                !put.is_empty(),
                if put.is_empty() {
                    format!(
                        "{} missing from .git/info/exclude and could not be added",
                        missing.join(", ")
                    )
                } else {
                    format!(
                        "{} was missing from .git/info/exclude and has been put back",
                        put.join(", ")
                    )
                },
            );
        }
    }
    let notes = crate::evidence::path_notes(p);
    add(
        "check path",
        true,
        if notes.is_empty() {
            "the folders on the check PATH are system folders".to_string()
        } else {
            format!("for information: {}", notes.join("; "))
        },
    );
    add(
        "log head",
        st.head_error.is_none(),
        st.head_error
            .clone()
            .unwrap_or_else(|| "the log matches the state head".into()),
    );
    let mirror_path = p.factory().join("checks.json");
    let mirror = std::fs::read_to_string(&mirror_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let (mirror_ok, mirror_note) = match mirror {
        None if !mirror_path.exists() => {
            let decided = st
                .checks_proposals
                .iter()
                .any(|x| x.used && (x.scope == "default" || x.scope == "both"));
            if decided || !st.checks_default.is_empty() {
                (false, "the log has a project default for the checks but the mirror file is missing: write it again from the log with `bf checks mirror`".to_string())
            } else {
                (
                    true,
                    "no check settings are recorded and no mirror file exists".to_string(),
                )
            }
        }
        None => (
            false,
            "the mirror file of the check default cannot be read as JSON".to_string(),
        ),
        Some(v) => {
            let file: std::collections::BTreeMap<String, String> = v["off"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, r)| (k.clone(), r.as_str().unwrap_or("").to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if file == st.checks_default {
                (
                    true,
                    "the mirror file matches the project default in the log".to_string(),
                )
            } else {
                (false, "the mirror file of the check default differs from the project default in the log: the log is the truth; write the file again from the log with `bf checks mirror`".to_string())
            }
        }
    };
    add("check settings", mirror_ok, mirror_note);
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
    // Before the plan gate is approved no plan is expected: a part without a unit is no problem yet.
    if st.gate_passed("plan", None) {
        add(
            "plan",
            pl.problems.is_empty(),
            crate::strings::t(
                "cli.design.doctor.plan_problems",
                &[("n", &pl.problems.len().to_string())],
            ),
        );
    } else {
        add(
            "plan",
            true,
            crate::strings::t("cli.design.doctor.plan_not_expected", &[]),
        );
    }
    // A checkpoint's working copy with changes nobody committed: a landing that stopped half way.
    let mut dirty: Vec<String> = Vec::new();
    for c in &pl.checkpoints {
        let k = c["id"].as_str().unwrap_or("");
        if k.is_empty() || st.closed.contains_key(k) {
            continue;
        }
        if st
            .sync_open
            .as_ref()
            .map(|s| s.checkpoint == k)
            .unwrap_or(false)
        {
            continue;
        }
        let top = p.work_dir().join(k);
        let mut wcs = vec![top.clone()];
        if let Ok(rd) = std::fs::read_dir(&top) {
            wcs.extend(rd.flatten().map(|e| e.path()));
        }
        for wc in wcs.into_iter().filter(|w| w.join(".git").exists()) {
            if crate::gitx::has_uncommitted(&wc) {
                dirty.push(format!("{k}: changes not committed"));
            }
        }
    }
    add(
        "checkpoint working copies",
        dirty.is_empty(),
        if dirty.is_empty() {
            "every open checkpoint's working copy is clean".to_string()
        } else {
            format!(
                "{}; a landing or sync stopped half way: put the working copy back with `git reset --hard` and `git merge --abort` in it (the human's step), then run the command again",
                dirty.join("; ")
            )
        },
    );
    // The disk the run holds: information and a warning, never a problem (D-109).
    let look = crate::run::cleanup::disk::look(p);
    add("disk", true, look.line());
    // A check that waited over ten minutes for a place of the heavy checks: information, no problem.
    let heavy_waits = evidence::long_waits(p);
    let disk = json!({
        "held_bytes": look.held,
        "free_bytes": look.free,
        "warn": look.warn,
        "low": look.low,
    });
    Ok(json!({"ok": ok, "checks": checks, "disk": disk, "heavy_waits": heavy_waits}))
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
