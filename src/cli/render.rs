/// Render a brainstorm-provenance view as fact-only disclosure. Deliberately uses
/// neutral words ("present", "absent", "STALE", "unattested", "unavailable") and
/// NEVER a pass/green marker — and never the word "independent" as a positive
/// claim. It discloses what was recorded; it does not evaluate it.
pub(super) fn format_brainstorm_provenance(
    view: &crate::domain::task::BrainstormProvenanceView,
) -> String {
    fn artifact_line(label: &str, status: Option<&crate::domain::task::ArtifactStatus>) -> String {
        match status {
            None => format!("  {label} artifact: absent\n"),
            Some(s) if s.stale => format!(
                "  {label} artifact: present (STALE — artifact changed or missing since recording)\n"
            ),
            Some(_) => format!("  {label} artifact: present\n"),
        }
    }
    let mut out = String::new();
    out.push_str("BRAINSTORM PROVENANCE\n");
    out.push_str(&format!("  brainstorm id: {}\n", view.id));
    out.push_str(&artifact_line("divergence", view.divergence.as_ref()));
    out.push_str(&artifact_line("convergence", view.convergence.as_ref()));
    out.push_str(&artifact_line("critic", view.critic.as_ref()));
    out.push_str(&format!(
        "  critic disposition: {}\n",
        view.critic_disposition
    ));
    // Always "unattested" in V1; a bare disclosure, never rendered as "independent".
    out.push_str(&format!(
        "  critic independence: {}\n",
        view.critic_independence
    ));
    match &view.skip_reason {
        Some(reason) => {
            let by = view.skip_decided_by.as_deref().unwrap_or("unknown");
            out.push_str(&format!("  skip reason: {reason} (decided by: {by})\n"));
        }
        None => out.push_str("  skip reason: none\n"),
    }
    match &view.source_run_id {
        Some(run) => out.push_str(&format!("  source run: {run} (attestation: unavailable)\n")),
        None => out.push_str("  source run attestation: unavailable\n"),
    }
    out.push_str(&format!(
        "  trust level: {} (untrusted content)\n",
        view.trust_level
    ));
    out.push_str(&format!("  recorded by: {}\n", view.recorded_by));
    out
}

/// Render an uncertainty ledger as fact-only disclosure: raw per-status counts
/// and the items, each with source and (for resolved) evidence freshness. NEVER
/// emits an aggregate verdict, score, ratio, percentage, or green marker — and
/// always discloses that the content is unverified and evidence unattested.
pub(super) fn format_uncertainty_ledger(
    view: &crate::domain::task::UncertaintyLedgerView,
) -> String {
    let mut out = String::new();
    out.push_str("UNCERTAINTIES  (content: unverified; evidence: unattested)\n");
    out.push_str(&format!("  open: {}\n", view.open));
    out.push_str(&format!(
        "  accepted as assumptions: {}\n",
        view.accepted_as_assumption
    ));
    out.push_str(&format!("  resolved with evidence: {}\n", view.resolved));
    out.push_str(&format!("  invalidated: {}\n", view.invalidated));
    out.push_str(&format!(
        "  trust level: {} (untrusted content)\n",
        view.trust_level
    ));
    out.push_str(&format_oracle_sources(&view.oracle_sources));
    for item in &view.items {
        out.push('\n');
        out.push_str(&format!("  {}  {}\n", item.id, item.status.to_uppercase()));
        out.push_str(&format!("    statement: {}\n", item.statement));
        if let Some(source) = &item.source {
            out.push_str(&format!("    source: {source} (unattested)\n"));
        }
        out.push_str(&format_item_oracle(item));
        if let Some(evidence) = &item.evidence {
            out.push_str(&format!(
                "    evidence: {} @ {}\n",
                evidence.path, evidence.recorded_hash
            ));
            out.push_str(&format!(
                "    freshness: {} (file consistency only; attestation: unavailable)\n",
                evidence.freshness.as_str()
            ));
        }
        if let Some(reason) = &item.reason {
            out.push_str(&format!("    reason: {reason}\n"));
        }
    }
    out
}

/// Render the per-oracle-kind breakdown. Raw counts only — never a score/ratio/
/// verdict. `model` is kept on its own `model advisory` line so it can never read as
/// external proof.
pub(super) fn format_oracle_sources(o: &crate::domain::task::OracleSourcesView) -> String {
    let mut out = String::new();
    out.push_str("  ORACLE SOURCES\n");
    out.push_str(&format!(
        "    deterministic/test: {}\n",
        o.deterministic + o.test
    ));
    out.push_str(&format!("    runtime: {}\n", o.runtime));
    out.push_str(&format!("    human decisions: {}\n", o.human));
    out.push_str(&format!("    model advisory: {}\n", o.model_advisory));
    out.push_str(&format!(
        "    external authority: {}\n",
        o.external_authority
    ));
    out
}

/// Per-item oracle disclosure for a resolved-via-evidence-ref uncertainty. A `model`
/// oracle is explicitly marked ADVISORY — never external proof. Legacy inline resolves
/// (no recorded oracle) print nothing here.
pub(super) fn format_item_oracle(item: &crate::domain::task::UncertaintyItemView) -> String {
    let mut out = String::new();
    if let Some(eid) = &item.evidence_id {
        out.push_str(&format!("    evidence_ref: {eid}\n"));
    }
    if let Some(kind) = &item.oracle_kind {
        if item.advisory {
            out.push_str(&format!(
                "    oracle: {kind} — ADVISORY (not external proof)\n"
            ));
        } else {
            out.push_str(&format!("    oracle: {kind} (unattested)\n"));
        }
    }
    out
}

/// Render a research-output view as fact-only disclosure: raw per-status counts,
/// produced artifacts (with freshness), and uncertainty items each tagged with
/// whether it was recorded after start. NEVER emits a verdict, score, ratio, or
/// a "discovered" count, and discloses unverified content / unattested evidence.
pub(super) fn format_research_output(view: &crate::domain::task::ResearchOutputView) -> String {
    let mut out = String::new();
    out.push_str("RESEARCH OUTPUT  (content: unverified; evidence: unattested)\n");
    out.push_str(&format!(
        "  artifacts recorded: {}\n",
        view.artifacts_recorded
    ));
    out.push_str(&format!(
        "  uncertainties opened: {}\n",
        view.uncertainties_opened
    ));
    out.push_str(&format!(
        "  resolved with evidence: {}\n",
        view.resolved_with_evidence
    ));
    out.push_str(&format!(
        "  accepted as assumptions: {}\n",
        view.accepted_as_assumptions
    ));
    out.push_str(&format!("  invalidated: {}\n", view.invalidated));
    out.push_str(&format!(
        "  trust level: {} (untrusted content)\n",
        view.trust_level
    ));
    if !view.artifacts.is_empty() {
        out.push_str("\n  ARTIFACTS\n");
        for a in &view.artifacts {
            out.push_str(&format!(
                "    {}  {} @ {}\n",
                a.kind, a.path, a.recorded_hash
            ));
            out.push_str(&format!(
                "      freshness: {} (file consistency only; attestation: unavailable)\n",
                a.freshness.as_str()
            ));
            if let Some(run) = &a.source_run_id {
                out.push_str(&format!(
                    "      source run: {run} (attestation: unavailable)\n"
                ));
            }
        }
    }
    if !view.uncertainties.is_empty() {
        out.push_str("\n  UNCERTAINTIES\n");
        for u in &view.uncertainties {
            let tag = if u.recorded_after_start {
                "[recorded after start]"
            } else {
                "[pre-start]"
            };
            out.push_str(&format!(
                "    {}  {}  {}\n",
                u.item.id,
                u.item.status.to_uppercase(),
                tag
            ));
            if let Some(source) = &u.item.source {
                out.push_str(&format!("      source: {source} (unattested)\n"));
            }
            if let Some(evidence) = &u.item.evidence {
                out.push_str(&format!(
                    "      evidence: {} @ {} freshness: {} (attestation: unavailable)\n",
                    evidence.path,
                    evidence.recorded_hash,
                    evidence.freshness.as_str()
                ));
            }
            if let Some(reason) = &u.item.reason {
                out.push_str(&format!("      reason: {reason}\n"));
            }
        }
    }
    out
}

/// Shorten `s` to at most `max` characters, marking truncation with an ellipsis.
pub(super) fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Format one decision record as a single human-readable line. A record that
/// does not parse as JSON is shown verbatim (flagged), never dropped — a
/// malformed advisory record must stay visible rather than silently disappear.
pub(super) fn format_decision_line(line: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return format!("  [?unparseable] {line}"),
    };
    let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let mark = match v.get("allowed").and_then(|x| x.as_bool()) {
        Some(true) => "ALLOW",
        Some(false) => "DENY ",
        None => "?    ",
    };
    let ts = v.get("ts").and_then(|x| x.as_u64()).unwrap_or(0);
    let source = get("source");
    let tool = get("tool");
    let path = get("path");
    let target = if path.is_empty() {
        ellipsize(&get("command"), 60)
    } else {
        path
    };
    let reason = ellipsize(&get("reason"), 80);
    format!("  [{mark}] ts={ts} {source}/{tool}  {target}  — {reason}")
}

/// Render the gate-decision log for `ctl decisions`. Pure (lines in, string out)
/// so selection + formatting are unit-tested without IO. `lines` are the raw
/// JSONL records oldest-first; the `limit` most-recent are shown (0 = all). The
/// non-canonical banner is emitted on every (non-JSON) invocation so a reader
/// can never mistake this log for canonical truth.
pub(super) fn format_decisions(lines: &[String], limit: usize, json: bool) -> String {
    let records: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    let start = if limit == 0 {
        0
    } else {
        records.len().saturating_sub(limit)
    };
    let shown = &records[start..];

    if json {
        // Raw JSONL passthrough of the selected window (still the non-canonical
        // records — each line already carries `"canonical": false`).
        return shown
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut out = String::new();
    out.push_str("⚠ NON-CANONICAL gate-decision log (.ctl/decisions.jsonl)\n");
    out.push_str("  Advisory records of blocked/flagged tool calls from the host gate hooks.\n");
    out.push_str(
        "  Evidence, NOT canonical task events · not hash-chained · not covered by `ctl validate`.\n",
    );

    if records.is_empty() {
        out.push_str("\n  (no decisions recorded yet)");
        return out;
    }

    out.push_str(&format!(
        "\n  showing {} of {} record(s):\n\n",
        shown.len(),
        records.len()
    ));
    for line in shown {
        out.push_str(&format_decision_line(line));
        out.push('\n');
    }
    out.truncate(out.trim_end().len());
    out
}
