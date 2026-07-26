use super::super::*;

pub(crate) fn brainstorm_artifact_recorded(
    state: &mut TaskState,
    event: &Event,
) -> Result<(), String> {
    {
        let brainstorm_id = require_str(&event.payload, "brainstorm_id")?;
        check_trust_level(&event.payload)?;
        let divergence =
            decode_artifact(&event.payload, "divergence_path", "divergence_hash", true)?;
        let convergence = decode_artifact(
            &event.payload,
            "convergence_path",
            "convergence_hash",
            false,
        )?;
        let source_run_id = optional_str(&event.payload, "source_run_id");
        // One brainstorm per task. Re-recording the same id refreshes the
        // originator artifacts (and preserves any critic disposition);
        // binding a different id is rejected.
        let (critic, critic_disposition, skip_reason, skip_decided_by) = match &state.brainstorm_ref
        {
            Some(existing) if existing.id != brainstorm_id => {
                return Err(format!(
                    "brainstorm_artifact_recorded: task already bound to brainstorm '{}'",
                    existing.id
                ));
            }
            Some(existing) => (
                existing.critic.clone(),
                existing.critic_disposition.clone(),
                existing.skip_reason.clone(),
                existing.skip_decided_by.clone(),
            ),
            None => (None, CriticDisposition::Absent, None, None),
        };
        state.brainstorm_ref = Some(BrainstormRef {
            id: brainstorm_id,
            divergence,
            convergence,
            critic,
            critic_disposition,
            critic_independence: CRITIC_INDEPENDENCE_UNATTESTED.to_string(),
            trust_level: BRAINSTORM_TRUST_LEVEL.to_string(),
            source_run_id,
            recorded_by: event.actor.clone(),
            skip_reason,
            skip_decided_by,
        });
    }
    Ok(())
}

pub(crate) fn critic_artifact_attached(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        let brainstorm_id = require_str(&event.payload, "brainstorm_id")?;
        check_trust_level(&event.payload)?;
        // Independence can never be claimed in V1: reject any value other
        // than `unattested`, so the ledger never asserts a critic was
        // independent when no independent orchestrator exists.
        if let Some(indep) = event
            .payload
            .get("critic_independence")
            .and_then(|v| v.as_str())
        {
            if indep != CRITIC_INDEPENDENCE_UNATTESTED {
                return Err(format!(
                    "critic_artifact_attached: critic_independence '{indep}' cannot be \
                         recorded; only '{CRITIC_INDEPENDENCE_UNATTESTED}' is permitted in V1 \
                         (no independent orchestrator)"
                ));
            }
        }
        let critic = decode_artifact(&event.payload, "critic_path", "critic_hash", true)?;
        let reference = state.brainstorm_ref.as_mut().ok_or_else(|| {
            "critic_artifact_attached: no brainstorm recorded for this task".to_string()
        })?;
        if reference.id != brainstorm_id {
            return Err(format!(
                "critic_artifact_attached: brainstorm id '{}' does not match recorded '{}'",
                brainstorm_id, reference.id
            ));
        }
        reference.critic = critic;
        reference.critic_disposition = CriticDisposition::Present;
        reference.critic_independence = CRITIC_INDEPENDENCE_UNATTESTED.to_string();
        // Attaching a critic supersedes any prior skip record.
        reference.skip_reason = None;
        reference.skip_decided_by = None;
    }
    Ok(())
}

pub(crate) fn brainstorm_skipped(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        let brainstorm_id = require_str(&event.payload, "brainstorm_id")?;
        check_trust_level(&event.payload)?;
        let skip_reason = require_str(&event.payload, "skip_reason")?;
        let decided_by = require_str(&event.payload, "decided_by")?;
        let reference = state.brainstorm_ref.as_mut().ok_or_else(|| {
            "brainstorm_skipped: no brainstorm recorded for this task".to_string()
        })?;
        if reference.id != brainstorm_id {
            return Err(format!(
                "brainstorm_skipped: brainstorm id '{}' does not match recorded '{}'",
                brainstorm_id, reference.id
            ));
        }
        reference.critic = None;
        reference.critic_disposition = CriticDisposition::Skipped;
        reference.critic_independence = CRITIC_INDEPENDENCE_UNATTESTED.to_string();
        reference.skip_reason = Some(skip_reason);
        reference.skip_decided_by = Some(decided_by);
    }
    // ── Uncertainty Ledger V1: record-and-disclose unknowns ──
    Ok(())
}

pub(crate) fn uncertainty_recorded(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        let id = require_str(&event.payload, "uncertainty_id")?;
        let statement = require_str(&event.payload, "statement")?;
        check_trust_level(&event.payload)?;
        let source = optional_str(&event.payload, "source");
        if state.uncertainties.iter().any(|u| u.id == id) {
            return Err(format!(
                "uncertainty_recorded: uncertainty '{id}' already recorded"
            ));
        }
        state.uncertainties.push(Uncertainty {
            id,
            statement,
            source,
            status: UncertaintyStatus::Open,
            evidence_ref: None,
            evidence_id: None,
            oracle_kind: None,
            reason: None,
        });
    }
    // ── Oracle V1: record a first-class, oracle-typed evidence object ──
    Ok(())
}

pub(crate) fn evidence_recorded(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        check_trust_level(&event.payload)?;
        let id = require_str(&event.payload, "evidence_id")?;
        let oracle_kind = decode_oracle_kind(&event.payload)?;
        let artifact_ref = decode_artifact(&event.payload, "artifact_path", "artifact_hash", true)?
            .ok_or_else(|| {
                "evidence_recorded: artifact_path and artifact_hash are required".to_string()
            })?;
        let source_ref = optional_str(&event.payload, "source_ref");
        if state.evidences.iter().any(|e| e.id == id) {
            return Err(format!(
                "evidence_recorded: evidence '{id}' already recorded"
            ));
        }
        state.evidences.push(Evidence {
            id,
            oracle_kind,
            source_ref,
            artifact_ref,
            // recorded_by is the envelope actor — an unattested principal, never
            // a separate forgeable payload field that could contradict the actor.
            recorded_by: event.actor.clone(),
        });
    }
    Ok(())
}

pub(crate) fn uncertainty_disposition_recorded(
    state: &mut TaskState,
    event: &Event,
) -> Result<(), String> {
    {
        let id = require_str(&event.payload, "uncertainty_id")?;
        check_trust_level(&event.payload)?;
        let disposition = require_str(&event.payload, "disposition")?;
        // Two evidence shapes: the legacy inline (path+hash) and the Oracle-V1
        // reference (evidence_ref → a recorded evidence id). They are mutually
        // exclusive on a resolve and resolved against state BEFORE the uncertainty
        // is borrowed mutably.
        let inline_evidence =
            decode_artifact(&event.payload, "evidence_path", "evidence_hash", false)?;
        let evidence_id = optional_str(&event.payload, "evidence_ref");
        let reason = optional_str(&event.payload, "reason");
        let has_any_evidence = inline_evidence.is_some() || evidence_id.is_some();
        // Resolve a referenced evidence to owned values up front (disjoint from the
        // mutable uncertainty borrow). Mutual exclusion enforced here.
        let resolved_via_ref = match &evidence_id {
            Some(eid) => {
                if inline_evidence.is_some() {
                    return Err(
                        "uncertainty_disposition_recorded: a 'resolved' must carry \
                             EITHER evidence_ref OR inline evidence_path/evidence_hash, never both"
                            .to_string(),
                    );
                }
                let ev = state
                    .evidences
                    .iter()
                    .find(|e| &e.id == eid)
                    .ok_or_else(|| {
                        format!(
                            "uncertainty_disposition_recorded: evidence_ref '{eid}' does not \
                             reference a recorded evidence in this task"
                        )
                    })?;
                Some((eid.clone(), ev.artifact_ref.clone(), ev.oracle_kind))
            }
            None => None,
        };
        let uncertainty = state
            .uncertainties
            .iter_mut()
            .find(|u| u.id == id)
            .ok_or_else(|| {
                format!("uncertainty_disposition_recorded: unknown uncertainty '{id}'")
            })?;
        // Terminal-is-terminal: a disposed uncertainty cannot be disposed
        // again, so an assumption can never be silently upgraded to resolved.
        if uncertainty.status != UncertaintyStatus::Open {
            return Err(format!(
                "uncertainty_disposition_recorded: uncertainty '{id}' is already '{}'; \
                     a disposition is terminal in V1",
                uncertainty.status.as_str()
            ));
        }
        match disposition.as_str() {
            "resolved" => {
                // resolved is the only disposition closed by external evidence —
                // either a recorded oracle-typed evidence (preferred) or legacy inline.
                //
                // Oracle-resolution semantics: a `model` oracle is advisory and must
                // not resolve an uncertainty (EPISTEMIC_CONTROL §5.1). That rule is
                // enforced at the command layer (record_uncertainty_disposition), the
                // sole path that appends canonical events — NOT here. The reducer stays
                // permissive on purpose: a committed pre-rule stream already resolved an
                // uncertainty via a model evidence_ref, and re-rejecting it on replay
                // would break the append-only ledger. The disclosure keeps such a
                // resolve honest by carrying oracle_kind=model (rendered ADVISORY) below.
                if let Some((eid, artifact_ref, oracle_kind)) = resolved_via_ref {
                    uncertainty.status = UncertaintyStatus::Resolved;
                    uncertainty.evidence_ref = Some(artifact_ref);
                    uncertainty.evidence_id = Some(eid);
                    uncertainty.oracle_kind = Some(oracle_kind);
                    uncertainty.reason = reason;
                } else if let Some(artifact_ref) = inline_evidence {
                    // Legacy inline evidence: oracle kind is unknown (predates Oracle V1).
                    uncertainty.status = UncertaintyStatus::Resolved;
                    uncertainty.evidence_ref = Some(artifact_ref);
                    uncertainty.reason = reason;
                } else {
                    return Err("uncertainty_disposition_recorded: 'resolved' requires \
                             evidence — an evidence_ref to a recorded evidence, or legacy inline \
                             evidence_path + evidence_hash (an unknown closed without external \
                             evidence is not resolved)"
                        .to_string());
                }
            }
            "accepted_as_assumption" => {
                // An assumption must remain visibly unresolved by external evidence.
                if has_any_evidence {
                    return Err("uncertainty_disposition_recorded: \
                             'accepted_as_assumption' must not carry evidence (it remains \
                             unresolved by external evidence); use reason"
                        .to_string());
                }
                uncertainty.status = UncertaintyStatus::AcceptedAsAssumption;
                uncertainty.reason = reason;
            }
            "invalidated" => {
                // "I was wrong / it no longer applies" has no oracle: a reason,
                // never evidence, so it cannot masquerade as a proof.
                if has_any_evidence {
                    return Err("uncertainty_disposition_recorded: 'invalidated' must not \
                             carry evidence; record why in reason"
                        .to_string());
                }
                let reason = reason.ok_or_else(|| {
                    "uncertainty_disposition_recorded: 'invalidated' requires a reason".to_string()
                })?;
                uncertainty.status = UncertaintyStatus::Invalidated;
                uncertainty.reason = Some(reason);
            }
            other => {
                return Err(format!(
                    "uncertainty_disposition_recorded: unknown disposition '{other}'"
                ));
            }
        }
    }
    // ── Research/Spike V1: record a tracked research artifact ──
    Ok(())
}

pub(crate) fn research_artifact_recorded(
    state: &mut TaskState,
    event: &Event,
) -> Result<(), String> {
    {
        check_trust_level(&event.payload)?;
        // Kind binding: only a research task accrues a research footprint. The
        // command layer pre-checks for a friendly message, but the invariant is
        // re-asserted here so it also holds on replay of any historical ledger.
        if state.task_kind != TaskKind::Research {
            return Err(
                "research_artifact_recorded: only a research task may record research \
                     artifacts (task kind is fixed at creation)"
                    .to_string(),
            );
        }
        // Terminal-is-terminal: a completed/cancelled task's disclosed output
        // must not change after the fact.
        if matches!(state.phase, Phase::Completed | Phase::Cancelled) {
            return Err(format!(
                "research_artifact_recorded: task is '{}'; a terminal task cannot record \
                     further research artifacts",
                state.phase.as_str()
            ));
        }
        let artifact_ref = decode_artifact(&event.payload, "artifact_path", "artifact_hash", true)?
            .ok_or_else(|| {
                "research_artifact_recorded: artifact_path and artifact_hash are required"
                    .to_string()
            })?;
        // Scope binding: an artifact must live inside the task's declared
        // write_allow (and outside write_deny) — the same boundary the write
        // gate enforces. The path is already normalized repo-relative, so the
        // test stays pure (no filesystem) and replay-safe.
        if !artifact_within_write_scope(&artifact_ref.path, &state.write_allow, &state.write_deny) {
            return Err(format!(
                "research_artifact_recorded: artifact '{}' is outside the task's write_allow \
                     (or within write_deny)",
                artifact_ref.path
            ));
        }
        let kind = decode_research_artifact_kind(&event.payload)?;
        let source_run_id = optional_str(&event.payload, "source_run_id");
        state.research_artifacts.push(ResearchArtifact {
            artifact_ref,
            kind,
            source_run_id,
        });
    }
    // ── Attestation V1: record a subagent dispatch on the parent task ──
    Ok(())
}

pub(crate) fn subagent_dispatched(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        check_trust_level(&event.payload)?;
        // Terminal-is-terminal: a completed/cancelled task's disclosed record
        // must not change after the fact.
        if matches!(state.phase, Phase::Completed | Phase::Cancelled) {
            return Err(format!(
                "subagent_dispatched: task is '{}'; a terminal task cannot record \
                     further dispatches",
                state.phase.as_str()
            ));
        }
        // role/adapter are host-supplied labels (unattested) — required so a
        // dispatch always names what was dispatched. The three artifacts are
        // optional (record-and-disclose), each a (path, hash) pair with no
        // half pairs.
        let role = require_str(&event.payload, "role")?;
        let adapter = require_str(&event.payload, "adapter")?;
        let parent_run = optional_str(&event.payload, "parent_run");
        let instruction = decode_artifact(
            &event.payload,
            "instruction_path",
            "instruction_hash",
            false,
        )?;
        let context = decode_artifact(&event.payload, "context_path", "context_hash", false)?;
        let output = decode_artifact(&event.payload, "output_path", "output_hash", false)?;
        state.dispatches.push(Dispatch {
            role,
            adapter,
            parent_run,
            instruction,
            context,
            output,
            recorded_by: event.actor.clone(),
        });
    }
    Ok(())
}
