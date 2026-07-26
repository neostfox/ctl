use super::*;

impl ControlApp {
    /// Normalize an evidence path (reject `..`, absolute, UNC, symlink escape),
    /// then hash the file ctl-side. Returns `(repo-relative path, sha256)`. The
    /// caller never supplies the hash, so the binding is a faithful record of what
    /// was on disk — not a claim the caller could forge.
    pub(crate) fn hash_evidence(&self, path: &str) -> Result<(String, String)> {
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        let normalized = normalizer
            .normalize(path)
            .map_err(|e| anyhow!("invalid evidence path '{}': {}", path, e))?;
        let rel = path_to_payload_string(&normalized);
        let resolved = self.project_root.join(&rel);
        if !resolved.is_file() {
            return Err(anyhow!("evidence artifact not found: {}", rel));
        }
        let hash = hash_file(&resolved)?;
        Ok((rel, hash))
    }

    /// Record an open uncertainty (an unknown the task carries).
    pub fn record_uncertainty(
        &self,
        task_id: &str,
        uncertainty_id: &str,
        statement: &str,
        source: Option<&str>,
    ) -> Result<Event> {
        // Terminal-is-terminal: a completed/cancelled task's disclosed unknowns
        // must not change after the fact (mirrors research_artifact_recorded).
        // Enforced here at the command layer — the sole canonical-append path —
        // and deliberately NOT in the reducer, so committed pre-rule streams that
        // recorded an uncertainty post-terminal still replay byte-identically.
        let state = self.replay_task(task_id)?;
        if matches!(
            state.phase,
            crate::domain::task::Phase::Completed | crate::domain::task::Phase::Cancelled
        ) {
            return Err(anyhow!(
                "task is '{}'; a terminal task cannot record further uncertainties — \
                 the unknown set of a completed/cancelled task is fixed",
                state.phase.as_str()
            ));
        }
        let mut payload = serde_json::json!({
            "uncertainty_id": uncertainty_id,
            "statement": statement,
            "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
        });
        if let Some(source) = source {
            payload["source"] = serde_json::json!(source);
        }
        let event = self.build_event(task_id, "uncertainty_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record a terminal disposition for an uncertainty. `resolved` requires an
    /// evidence artifact (hashed ctl-side); `accepted_as_assumption` and
    /// `invalidated` must not carry evidence. The reducer enforces terminal-is-
    /// terminal and the disposition-specific evidence/reason rules.
    pub fn record_uncertainty_disposition(
        &self,
        task_id: &str,
        uncertainty_id: &str,
        disposition: &str,
        evidence_path: Option<&str>,
        evidence_ref: Option<&str>,
        reason: Option<&str>,
    ) -> Result<Event> {
        // Mutual exclusion is also enforced by the reducer + schema; reject early
        // here for a clear CLI message before any file hashing happens.
        if evidence_path.is_some() && evidence_ref.is_some() {
            return Err(anyhow!(
                "a 'resolved' must carry either --evidence-ref or --evidence (inline), never both"
            ));
        }
        // Oracle-resolution semantics: a `model` oracle is ADVISORY — never external
        // proof (EPISTEMIC_CONTROL §5.1: a resolve must distinguish "closed by
        // assertion" from "closed by external oracle"). A model-backed evidence may be
        // recorded and disclosed, but it must not *resolve* an uncertainty. This is
        // enforced here at the command layer — the only path that appends canonical
        // events — and deliberately NOT in the reducer, so committed pre-rule streams
        // that already resolved via a model oracle still replay byte-identically.
        if disposition == "resolved" {
            if let Some(eid) = evidence_ref {
                let state = self.replay_task(task_id)?;
                if let Some(ev) = state.evidences.iter().find(|e| e.id == eid) {
                    if ev.oracle_kind.is_advisory() {
                        return Err(anyhow!(
                            "evidence '{}' is a 'model' oracle (advisory, not external proof); \
                             a model oracle cannot resolve an uncertainty — record it as context, \
                             or resolve with a deterministic/test/runtime/human/external_authority \
                             oracle",
                            eid
                        ));
                    }
                }
            }
        }
        let mut payload = serde_json::json!({
            "uncertainty_id": uncertainty_id,
            "disposition": disposition,
            "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
        });
        if let Some(path) = evidence_path {
            let (rel, hash) = self.hash_evidence(path)?;
            payload["evidence_path"] = serde_json::json!(rel);
            payload["evidence_hash"] = serde_json::json!(hash);
        }
        if let Some(eref) = evidence_ref {
            payload["evidence_ref"] = serde_json::json!(eref);
        }
        if let Some(reason) = reason {
            payload["reason"] = serde_json::json!(reason);
        }
        let event = self.build_event(task_id, "uncertainty_disposition_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record a first-class, oracle-typed evidence object (Oracle V1). ctl computes
    /// the artifact hash from a normalized path (the caller never supplies it);
    /// `recorded_by` is the envelope actor, captured by the reducer — not a payload
    /// field. The evidence can later be referenced by a `resolved` disposition.
    pub fn record_evidence(
        &self,
        task_id: &str,
        evidence_id: &str,
        oracle_kind: &str,
        source_ref: Option<&str>,
        artifact_path: &str,
    ) -> Result<Event> {
        let (rel, hash) = self.hash_evidence(artifact_path)?;
        let mut payload = serde_json::json!({
            "evidence_id": evidence_id,
            "oracle_kind": oracle_kind,
            "artifact_path": rel,
            "artifact_hash": hash,
            "trust_level": crate::domain::task::EVIDENCE_TRUST_LEVEL,
        });
        if let Some(source) = source_ref {
            payload["source_ref"] = serde_json::json!(source);
        }
        let event = self.build_event(task_id, "evidence_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record a subagent dispatch on the parent task (subagent-dispatch-record-v1).
    /// Record-and-disclose: `role`/`adapter` are host-supplied labels and each
    /// supplied artifact is sha256-hashed by ctl (`hash_evidence`); this records
    /// what the host said it dispatched — it never asserts what actually ran.
    /// Absent artifacts are simply not recorded.
    #[allow(clippy::too_many_arguments)]
    pub fn record_subagent_dispatch(
        &self,
        task_id: &str,
        role: &str,
        adapter: &str,
        parent_run: Option<&str>,
        instruction_artifact: Option<&str>,
        context_artifact: Option<&str>,
        output_artifact: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "role": role,
            "adapter": adapter,
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(run) = parent_run.filter(|s| !s.is_empty()) {
            payload["parent_run"] = serde_json::json!(run);
        }
        for (path_key, hash_key, artifact) in [
            ("instruction_path", "instruction_hash", instruction_artifact),
            ("context_path", "context_hash", context_artifact),
            ("output_path", "output_hash", output_artifact),
        ] {
            if let Some(p) = artifact.filter(|s| !s.is_empty()) {
                let (rel, hash) = self.hash_evidence(p)?;
                payload[path_key] = serde_json::json!(rel);
                payload[hash_key] = serde_json::json!(hash);
            }
        }
        let event = self.build_event(task_id, "subagent_dispatched", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Resolve evidence/artifact freshness against the working tree: ABSENT if
    /// the file is gone, STALE if its hash drifted, CURRENT if it matches. Never
    /// asserts the content is valid — only whether the file still matches what was
    /// recorded. Shared by evidence and research-artifact disclosure.
    pub(crate) fn artifact_freshness(
        &self,
        artifact: &crate::domain::task::ArtifactRef,
    ) -> crate::domain::task::EvidenceFreshness {
        use crate::domain::task::EvidenceFreshness;
        let resolved = self.project_root.join(&artifact.path);
        if !resolved.is_file() {
            EvidenceFreshness::Absent
        } else if hash_file(&resolved).ok().as_deref() == Some(artifact.hash.as_str()) {
            EvidenceFreshness::Current
        } else {
            EvidenceFreshness::Stale
        }
    }

    /// Build the fact-only view of one uncertainty (shared by the ledger view and
    /// the research-output view).
    pub(crate) fn uncertainty_item_view(
        &self,
        u: &crate::domain::task::Uncertainty,
    ) -> crate::domain::task::UncertaintyItemView {
        use crate::domain::task::{EvidenceView, UncertaintyItemView};
        let evidence = u.evidence_ref.as_ref().map(|ev| EvidenceView {
            path: ev.path.clone(),
            recorded_hash: ev.hash.clone(),
            freshness: self.artifact_freshness(ev),
            attested: false,
        });
        UncertaintyItemView {
            id: u.id.clone(),
            statement: u.statement.clone(),
            status: u.status.as_str().to_string(),
            source: u.source.clone(),
            evidence,
            evidence_id: u.evidence_id.clone(),
            oracle_kind: u.oracle_kind.map(|k| k.as_str().to_string()),
            advisory: u.oracle_kind.map(|k| k.is_advisory()).unwrap_or(false),
            reason: u.reason.clone(),
        }
    }

    /// Aggregate the task's recorded evidence into per-oracle-kind counts. Raw counts
    /// only; `model` is kept on its own `model_advisory` line so it can never be summed
    /// into "external proof".
    pub(crate) fn oracle_sources_view(
        &self,
        state: &TaskState,
    ) -> crate::domain::task::OracleSourcesView {
        use crate::domain::task::{OracleKind, OracleSourcesView};
        let mut view = OracleSourcesView::default();
        for e in &state.evidences {
            match e.oracle_kind {
                OracleKind::Deterministic => view.deterministic += 1,
                OracleKind::Test => view.test += 1,
                OracleKind::Runtime => view.runtime += 1,
                OracleKind::Human => view.human += 1,
                OracleKind::Model => view.model_advisory += 1,
                OracleKind::ExternalAuthority => view.external_authority += 1,
            }
        }
        view
    }

    /// Build a fact-only uncertainty-ledger view, resolving evidence freshness
    /// against the working tree. Returns None when the task records no uncertainty.
    pub fn uncertainty_ledger_view(
        &self,
        state: &TaskState,
    ) -> Option<crate::domain::task::UncertaintyLedgerView> {
        use crate::domain::task::{
            UncertaintyLedgerView, UncertaintyStatus, UNCERTAINTY_TRUST_LEVEL,
        };
        if state.uncertainties.is_empty() {
            return None;
        }
        let (mut open, mut accepted_as_assumption, mut resolved, mut invalidated) = (0, 0, 0, 0);
        for uncertainty in &state.uncertainties {
            match uncertainty.status {
                UncertaintyStatus::Open => open += 1,
                UncertaintyStatus::AcceptedAsAssumption => accepted_as_assumption += 1,
                UncertaintyStatus::Resolved => resolved += 1,
                UncertaintyStatus::Invalidated => invalidated += 1,
            }
        }
        let items = state
            .uncertainties
            .iter()
            .map(|u| self.uncertainty_item_view(u))
            .collect();
        Some(UncertaintyLedgerView {
            open,
            accepted_as_assumption,
            resolved,
            invalidated,
            trust_level: UNCERTAINTY_TRUST_LEVEL.to_string(),
            oracle_sources: self.oracle_sources_view(state),
            items,
        })
    }
}
