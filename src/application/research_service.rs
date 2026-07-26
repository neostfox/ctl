use super::*;

impl ControlApp {
    /// Record a tracked research artifact (Research/Spike V1). ctl computes the
    /// hash from a normalized path; the caller never supplies it.
    pub fn record_research_artifact(
        &self,
        task_id: &str,
        artifact_path: &str,
        artifact_kind: &str,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        // Pre-check against current state for a clear CLI message before any file
        // hashing. The reducer re-asserts every one of these invariants so they
        // also hold on replay — this layer is for ergonomics, not enforcement.
        let state = self.replay_task(task_id)?;
        if state.task_kind != TaskKind::Research {
            return Err(anyhow!(
                "only a research task may record research artifacts; task '{}' is an \
                 implementation task",
                task_id
            ));
        }
        if matches!(state.phase, Phase::Completed | Phase::Cancelled) {
            return Err(anyhow!(
                "task '{}' is {}; a terminal task cannot record further research artifacts",
                task_id,
                state.phase.as_str()
            ));
        }
        let (rel, hash) = self.hash_evidence(artifact_path)?;
        // Scope binding: the artifact must sit inside the task's write_allow (and
        // outside write_deny) — the same boundary the write gate enforces.
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        if !file_in_write_scope(&normalizer, &rel, &state.write_allow, &state.write_deny)? {
            return Err(anyhow!(
                "research artifact '{}' is outside the task's write_allow (or within write_deny)",
                rel
            ));
        }
        let mut payload = serde_json::json!({
            "artifact_path": rel,
            "artifact_hash": hash,
            "artifact_kind": artifact_kind,
            "trust_level": crate::domain::task::RESEARCH_TRUST_LEVEL,
        });
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "research_artifact_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Build a fact-only research-output view for a research task; None for
    /// implementation tasks. Raw per-status counts, artifacts with freshness, and
    /// uncertainty items each tagged `recorded_after_start` (derived from the
    /// single `task_started` seq). Deliberately NO "discovered" scalar and no verdict.
    pub fn research_output_view(
        &self,
        task_id: &str,
    ) -> Result<Option<crate::domain::task::ResearchOutputView>> {
        use crate::domain::task::{
            ResearchArtifactView, ResearchOutputView, ResearchUncertaintyView, UncertaintyStatus,
            RESEARCH_TRUST_LEVEL,
        };
        let state = self.replay_task(task_id)?;
        if state.task_kind != TaskKind::Research {
            return Ok(None);
        }
        let events = self.store.read_for_task(task_id)?;
        // A finishable task has exactly one task_started (reopen emits
        // task_reopened, not a second start). Uncertainties recorded after it are
        // tagged "recorded after start" — a per-item fact, never a rankable count.
        let start_seq = events
            .iter()
            .find(|e| e.event_type == "task_started")
            .map(|e| e.seq);
        let mut recorded_seq: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();
        for e in &events {
            if e.event_type == "uncertainty_recorded" {
                if let Some(id) = e.payload.get("uncertainty_id").and_then(|v| v.as_str()) {
                    recorded_seq.insert(id.to_string(), e.seq);
                }
            }
        }
        let (mut accepted_as_assumptions, mut resolved, mut invalidated) = (0, 0, 0);
        for u in &state.uncertainties {
            match u.status {
                UncertaintyStatus::Resolved => resolved += 1,
                UncertaintyStatus::AcceptedAsAssumption => accepted_as_assumptions += 1,
                UncertaintyStatus::Invalidated => invalidated += 1,
                UncertaintyStatus::Open => {}
            }
        }
        let artifacts = state
            .research_artifacts
            .iter()
            .map(|a| ResearchArtifactView {
                path: a.artifact_ref.path.clone(),
                recorded_hash: a.artifact_ref.hash.clone(),
                kind: a.kind.as_str().to_string(),
                freshness: self.artifact_freshness(&a.artifact_ref),
                source_run_id: a.source_run_id.clone(),
                source_run_attested: false,
            })
            .collect();
        let uncertainties = state
            .uncertainties
            .iter()
            .map(|u| {
                let recorded_after_start = match (start_seq, recorded_seq.get(&u.id)) {
                    (Some(start), Some(&seq)) => seq > start,
                    _ => false,
                };
                ResearchUncertaintyView {
                    item: self.uncertainty_item_view(u),
                    recorded_after_start,
                }
            })
            .collect();
        Ok(Some(ResearchOutputView {
            artifacts_recorded: state.research_artifacts.len(),
            uncertainties_opened: state.uncertainties.len(),
            resolved_with_evidence: resolved,
            accepted_as_assumptions,
            invalidated,
            trust_level: RESEARCH_TRUST_LEVEL.to_string(),
            artifacts,
            uncertainties,
        }))
    }

    /// Read-only GO / NO-GO safety evaluation for an unattended (ralph)
    /// supervisor loop: is it still safe to continue without a human? Composes
    /// this session's guards — task hold/terminality, cross-ledger consistency,
    /// shared-`.git` locks, and drift (next-action) — into one verdict. Appends
    /// nothing; spawns nothing. The supervisor halts the moment this returns a
    /// NO-GO; it is the envelope around an external run, never the executor.
    pub fn ralph_safety_check(&self, task_id: &str) -> Result<RalphVerdict> {
        let mut blockers = Vec::new();

        let state = self.replay_task(task_id)?;
        if state.is_held {
            blockers.push("task is held — resolve the hold before resuming".to_string());
        }
        if matches!(state.phase, Phase::Completed | Phase::Cancelled) {
            blockers.push(format!(
                "task is terminal ({:?}) — nothing left to supervise",
                state.phase
            ));
        }

        let cross_ledger = self.cross_ledger_findings()?;
        if !cross_ledger.is_empty() {
            blockers.push(format!(
                "{} cross-ledger inconsistency(ies) — run `ctl repair --cross-ledger`",
                cross_ledger.len()
            ));
        }

        let risk = crate::infrastructure::workspace::scan_shared_git_risk(&self.project_root);
        if risk.any() {
            blockers.push(format!(
                "shared .git lock present: {}",
                risk.descriptions().join("; ")
            ));
        }

        // Drift: anything other than Pass means a human decision is due.
        let na = self.next_action(task_id)?;
        if !matches!(na.action, crate::domain::drift::NextActionKind::Pass) {
            blockers.push(format!(
                "drift next-action is {} ({})",
                na.action.as_str(),
                na.rationale
            ));
        }

        Ok(RalphVerdict {
            go: blockers.is_empty(),
            blockers,
        })
    }

    // ── Queries ──
}
