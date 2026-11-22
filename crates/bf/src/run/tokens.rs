//! The one-use tokens that say which role is speaking.

use super::*;

/// Whether `token` is the current token for `key` (`<unit or checkpoint>:<role>`), as shown by `bf next`.
pub fn holds_token(p: &Project, key: &str, token: Option<&str>) -> bool {
    match (read_tokens(p).get(key).and_then(|t| t.as_str()), token) {
        (Some(h), Some(t)) => h == t.trim(),
        _ => false,
    }
}

/// The one-use token for `key` (`<unit>:<role>`): the one already handed out, or a new one kept in
/// the state folder like the tokens of the existing roles.
pub fn issue_token(p: &Project, key: &str) -> R<String> {
    let _tl = crate::lock::tokens(p)?;
    let mut v = read_tokens(p);
    if let Some(t) = v.get(key).and_then(|t| t.as_str()) {
        return Ok(t.to_string());
    }
    let tok = util::random_hex(12)?;
    v[key] = json!(tok);
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(tok)
}

pub(crate) fn read_tokens(p: &Project) -> Value {
    std::fs::read_to_string(p.tokens_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

/// `bf tokens` are the way a report says which role is speaking. The token of every open worker
/// directive is kept in the state folder, keyed by unit and role, and `bf next` shows it every time it
/// is asked: a conductor that resumes, or a new one, always gets the token of the work already handed
/// out. A token is spent by one successful report; it is dropped when its work is no longer open, and
/// kept while it is, whether or not this read shows the directive.
pub(crate) fn sync_tokens(p: &Project, v: &mut Value) -> R<()> {
    let _tl = crate::lock::tokens(p)?;
    let mut tokens = read_tokens(p);
    // Old format (hash -> {unit, role}) is dropped.
    if tokens
        .as_object()
        .map(|o| o.values().any(|x| x.is_object()))
        .unwrap_or(false)
    {
        tokens = json!({});
    }
    let mut fresh = json!({});
    if let Some(ds) = v["directives"].as_array_mut() {
        for d in ds.iter_mut() {
            let role = d["role"].as_str().unwrap_or("").to_string();
            // A verifier's directive belongs to a checkpoint; the others to a unit.
            let unit = d["unit"]
                .as_str()
                .or_else(|| d["checkpoint"].as_str())
                .unwrap_or("")
                .to_string();
            // While a check of a checkpoint is marked failed or unreachable the human decides; the
            // verifier who is asked to look again still needs its token, so it stays on that directive.
            let (role, field) = if d["action"] == "decide-check" {
                ("verifier".to_string(), "verifier_token")
            } else {
                (role, "token")
            };
            if (matches!(role.as_str(), "builder" | "reviewer" | "verifier")
                || crate::roles::is_new_role(&role))
                && !unit.is_empty()
            {
                let key = format!("{unit}:{role}");
                let tok = match tokens.get(&key).and_then(|t| t.as_str()) {
                    Some(t) => t.to_string(),
                    None => util::random_hex(12)?,
                };
                d[field] = json!(tok);
                fresh[&key] = json!(tok);
            }
        }
    }
    // A unit held by an open question has no directive, but its worker's token stays: the same token
    // is given every time, until a report spends it.
    let held: Vec<String> = p
        .list_records(schema::kind_by_name("question"))?
        .into_iter()
        .filter(|q| !s(q, "unit").is_empty() && q.get("answer").is_none())
        .map(|q| s(&q, "unit").to_string())
        .collect();
    if let Some(o) = tokens.as_object() {
        for (k, t) in o {
            let keep = k
                .split_once(':')
                .map(|(u, r)| {
                    held.iter().any(|h| h == u)
                        && (matches!(r, "builder" | "reviewer" | "verifier")
                            || crate::roles::is_new_role(r))
                })
                .unwrap_or(false);
            if keep && fresh.get(k).is_none() {
                fresh[k] = t.clone();
            }
        }
    }
    // A token whose work is still open stays whether or not this read shows its directive: another
    // hold may hide the directive, and the worker is still working.
    if let Some(o) = tokens.as_object() {
        let open = OpenWork::load(p);
        for (k, t) in o {
            if fresh.get(k).is_none() && open.as_ref().map(|w| w.is_open(p, k)).unwrap_or(false) {
                fresh[k] = t.clone();
            }
        }
    }
    // Tokens handed out by `bf roles token` stay until spent.
    if let Some(o) = tokens.as_object() {
        for (k, t) in o {
            let is_new = k
                .split_once(':')
                .map(|(_, r)| crate::roles::is_new_role(r))
                .unwrap_or(false);
            if is_new && fresh.get(k).is_none() {
                fresh[k] = t.clone();
            }
        }
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&fresh)?.as_bytes())?;
    Ok(())
}

/// The replayed state and the plan, to tell whether the work behind a token is still open.
struct OpenWork {
    st: crate::events::State,
    pl: plan::Plan,
    receipts: Vec<evidence::Receipt>,
}

impl OpenWork {
    fn load(p: &Project) -> Option<OpenWork> {
        Some(OpenWork {
            st: events::replay(p).ok()?,
            pl: plan::load(p).ok()?,
            receipts: evidence::load_receipts(p).ok()?,
        })
    }

    /// Whether the work of `key` (`<unit or checkpoint>:<role>`) is open.
    fn is_open(&self, p: &Project, key: &str) -> bool {
        let Some((id, role)) = key.split_once(':') else {
            return false;
        };
        if self.pl.checkpoints.iter().any(|k| s(k, "id") == id) {
            return self.checkpoint_open(p, id, role);
        }
        let state = self.st.unit_state(id);
        match role {
            "builder" => matches!(state, "running" | "fixing" | "blocked" | "parked"),
            "reviewer" => state == "review",
            _ => false,
        }
    }

    fn checkpoint_open(&self, p: &Project, k: &str, role: &str) -> bool {
        if self.st.closed.contains_key(k) {
            return false;
        }
        let fix_phase = self
            .st
            .ck_fix
            .as_ref()
            .filter(|f| f.checkpoint == k)
            .map(|f| f.phase.as_str());
        let sync_phase = self
            .st
            .sync_open
            .as_ref()
            .filter(|f| f.checkpoint == k)
            .map(|f| f.phase.as_str());
        match role {
            "builder" => {
                matches!(fix_phase, Some("fix" | "scope")) || sync_phase == Some("resolve")
            }
            "reviewer" => {
                fix_phase == Some("review")
                    || sync_phase == Some("review")
                    || crate::ckreview::rounds(p, k)
                        .map(|rs| rs.last().map(|r| r.result.is_none()).unwrap_or(false))
                        .unwrap_or(false)
            }
            "verifier" => self.verifier_open(p, k),
            _ => false,
        }
    }

    fn verifier_open(&self, p: &Project, k: &str) -> bool {
        let Some(cp) = self.pl.checkpoints.iter().find(|c| s(c, "id") == k) else {
            return false;
        };
        if matches!(
            events::hold(&self.st, k, u64::MAX),
            Some(events::Hold::Marked { .. })
        ) {
            return true;
        }
        let after = str_list(cp, "after");
        if after.is_empty() || !after.iter().all(|u| self.st.unit_state(u) == "landed") {
            return false;
        }
        let content = p.content_hash(&p.work_dir().join(k)).unwrap_or_default();
        str_list(cp, "checks").into_iter().any(|c| {
            let rule = self.pl.checks.iter().find(|r| s(r, "id") == c);
            let off = rule
                .map(|r| crate::enforce::is_off(&self.st, r))
                .unwrap_or(false);
            let review = rule
                .map(|r| s(r, "named") == crate::checks::CHECKPOINT_REVIEW)
                .unwrap_or(false);
            !off && !review && evidence::satisfied_at(&self.receipts, &c, &content).is_none()
        })
    }
}

pub(crate) fn check_token(p: &Project, unit: &str, role: &str, token: Option<&str>) -> R<String> {
    let key = format!("{unit}:{role}");
    let have = read_tokens(p)
        .get(&key)
        .and_then(|t| t.as_str())
        .map(|t| t.to_string());
    if let (Some(h), Some(t)) = (&have, token) {
        if h == t.trim() {
            return Ok(key);
        }
    }
    Err(refuse(
        format!("report on {unit} as {role}"),
        format!("a report needs the token of the {role}'s directive for {unit}; none, or the wrong one, was given"),
        &[
            "The conductor runs `bf next --json`; each builder and reviewer directive carries its `token`.",
            "The worker reports with `--token <token>`. A builder cannot report as the reviewer: the reviewer's directive exists only while the unit is in review.",
        ],
        "`bf status` shows the unit in its new state.",
    ))
}

pub(crate) fn spend_token(p: &Project, key: &str) -> R<()> {
    let _tl = crate::lock::tokens(p)?;
    let mut v = read_tokens(p);
    if let Some(o) = v.as_object_mut() {
        o.remove(key);
    }
    util::atomic_write(&p.tokens_path(), serde_json::to_string(&v)?.as_bytes())?;
    Ok(())
}
