//! A reviewer asks for a security audit of a unit in review (D-117): a finding with `asks = "security"`.
//! `bf next` then offers the security auditor that work in place of the review, and the auditor's report
//! (built or blocked) closes the ask and gives the review back.

use super::*;

/// The value of `asks` that names the security audit.
pub const ASK: &str = "security";
/// The role that does the audit.
pub const ROLE: &str = "security-auditor";

/// Every open asking finding of the unit, oldest first.
pub fn open_asks(p: &Project, unit: &str) -> R<Vec<Value>> {
    let mut v: Vec<Value> = p
        .list_records(schema::kind_by_name("finding"))?
        .into_iter()
        .filter(|f| s(f, "unit") == unit && s(f, "asks") == ASK && is_open(f))
        .collect();
    v.sort_by_key(|f| {
        schema::parse_id(s(f, "id"))
            .map(|(_, n)| n)
            .unwrap_or(u32::MAX)
    });
    Ok(v)
}

/// The open ask of a unit in `review`: its oldest open asking finding. A second one filed while it is open
/// is answered with it.
pub fn asked(p: &Project, st: &State, unit: &str) -> R<Option<Value>> {
    if st.unit_state(unit) != "review" {
        return Ok(None);
    }
    Ok(open_asks(p, unit)?.into_iter().next())
}

/// How long a unit may be idle before it is parked: a unit in review with an open ask is allowed one limit
/// for the review and one for the audit.
pub fn park_limit(limit: u64, asked: bool) -> u64 {
    if asked {
        limit.saturating_mul(2)
    } else {
        limit
    }
}

/// The directive that offers the audit.
pub(crate) fn directive(
    unit: &str,
    ask: &Value,
    wc: &str,
    brief: &str,
    limits: Value,
    tier: &str,
) -> Value {
    json!({"role": ROLE, "action": "security-audit", "unit": unit, "finding": s(ask, "id"),
        "tier": tier, "working_copy": wc, "brief": brief, "limits": limits,
        "note": crate::strings::t("run.expert.note", &[("unit", unit), ("finding", s(ask, "id"))]),
        "commands": [format!("bf brief {unit} --role {ROLE}")]})
}

/// The auditor's report ends the ask: every open asking finding of the unit is `fixed`, and its `action`
/// says the audit answered it, or was not done and why (`reason` is `Some` for a blocked report).
pub(crate) fn close(p: &Project, unit: &str, reason: Option<&str>) -> R<()> {
    let asks = open_asks(p, unit)?;
    if asks.is_empty() {
        return Ok(());
    }
    let sentence = match reason {
        None => crate::strings::t("run.expert.answered", &[]),
        Some(r) if !r.trim().is_empty() => {
            crate::strings::t("run.expert.not_done", &[("reason", r.trim())])
        }
        Some(_) => crate::strings::t("run.expert.not_done_no_reason", &[]),
    };
    for mut f in asks {
        let was = s(&f, "action").trim().to_string();
        f["action"] = json!(if was.is_empty() {
            sentence.clone()
        } else {
            format!("{was} {sentence}")
        });
        f["outcome"] = json!("fixed");
        p.write_record(&f)?;
    }
    p.write_guard()?;
    Ok(())
}
