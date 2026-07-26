use super::*;

impl ControlApp {
    /// Record originator (divergence/convergence) artifacts for a brainstorm.
    pub fn record_brainstorm_artifacts(
        &self,
        task_id: &str,
        brainstorm_id: &str,
        divergence_path: &str,
        convergence_path: Option<&str>,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "brainstorm_id": brainstorm_id,
            "divergence_path": divergence_path,
            "divergence_hash": self.hash_artifact(divergence_path)?,
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(convergence) = convergence_path {
            payload["convergence_path"] = serde_json::json!(convergence);
            payload["convergence_hash"] = serde_json::json!(self.hash_artifact(convergence)?);
        }
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "brainstorm_artifact_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Attach a critic (challenge) artifact to a recorded brainstorm.
    pub fn attach_brainstorm_critic(
        &self,
        task_id: &str,
        brainstorm_id: &str,
        critic_path: &str,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "brainstorm_id": brainstorm_id,
            "critic_path": critic_path,
            "critic_hash": self.hash_artifact(critic_path)?,
            "critic_independence": crate::domain::task::CRITIC_INDEPENDENCE_UNATTESTED,
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "critic_artifact_attached", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record that the critic step was explicitly skipped, with a reason and the
    /// deciding actor (defaults to the recording actor when not supplied).
    pub fn skip_brainstorm_critic(
        &self,
        task_id: &str,
        brainstorm_id: &str,
        reason: &str,
        decided_by: Option<&str>,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "brainstorm_id": brainstorm_id,
            "skip_reason": reason,
            "decided_by": decided_by.unwrap_or(self.actor.as_str()),
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "brainstorm_skipped", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Build a fact-only provenance view, resolving artifact staleness against the
    /// current working tree. Returns None when the task has no recorded brainstorm.
    pub fn brainstorm_provenance_view(
        &self,
        state: &TaskState,
    ) -> Option<crate::domain::task::BrainstormProvenanceView> {
        use crate::domain::task::{ArtifactRef, ArtifactStatus, BrainstormProvenanceView};
        let reference = state.brainstorm_ref.as_ref()?;
        let status = |artifact: &ArtifactRef| -> ArtifactStatus {
            let resolved = self.project_root.join(&artifact.path);
            let present = resolved.is_file();
            // Missing → stale; present but hash drifted → stale; match → fresh.
            let stale = match present.then(|| hash_file(&resolved).ok()).flatten() {
                Some(current) => current != artifact.hash,
                None => true,
            };
            ArtifactStatus {
                path: artifact.path.clone(),
                present,
                stale,
                recorded_hash: artifact.hash.clone(),
            }
        };
        Some(BrainstormProvenanceView {
            id: reference.id.clone(),
            divergence: reference.divergence.as_ref().map(&status),
            convergence: reference.convergence.as_ref().map(&status),
            critic: reference.critic.as_ref().map(&status),
            critic_disposition: reference.critic_disposition.as_str().to_string(),
            critic_independence: reference.critic_independence.clone(),
            trust_level: reference.trust_level.clone(),
            source_run_id: reference.source_run_id.clone(),
            source_run_attested: false,
            recorded_by: reference.recorded_by.clone(),
            skip_reason: reference.skip_reason.clone(),
            skip_decided_by: reference.skip_decided_by.clone(),
        })
    }

    // ── PRD plan / validate / status (workflow-prd-to-tasks-v1) ──
    //
    // Closes the cognitive loop: a confirmed PRD's `## Tasks` section becomes
    // governed tasks in one call. No new event types — reuses create_task +
    // record_brainstorm_artifacts. Pure parsing lives in `application::prd`;
    // these methods add the IO-bound boundary/gate validation and orchestration.

    /// Validate a parsed PRD against format, boundary, gate, and overlap rules.
    /// Read-only — emits no events. Returns every problem found (does not stop
    /// at the first), so the user sees the full picture before planning.
    pub fn prd_validate(
        &self,
        doc: &crate::application::prd::PrdDocument,
    ) -> Result<crate::application::prd::PrdValidation> {
        use crate::application::prd::{overlap_problems, validate_format};

        let mut v = validate_format(doc);

        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );

        for task in &doc.tasks {
            // Structural boundary check: path escape, symlinks/junctions/UNC.
            // Protected paths are NOT errors here (they mirror create/revise:
            // a protected path may be declared in write_allow) — but a write to
            // one still requires a `ctl apply` exception at the runtime gate, so
            // surface it as a non-blocking warning (record-and-disclose).
            for path in &task.write_allow {
                match normalizer.normalize(path) {
                    Ok(normalized) => {
                        if normalizer.is_protected(&normalized) {
                            v.warning(
                                Some(&task.id),
                                format!(
                                    "write-allow path '{}' is protected — the runtime gate requires a reviewed `ctl apply` exception before it can be written",
                                    path
                                ),
                            );
                        }
                    }
                    Err(e) => v.error(
                        Some(&task.id),
                        format!("write-allow path '{}': {}", path, e),
                    ),
                }
            }
            for path in &task.read_scope {
                if let Err(e) = normalizer.normalize(path) {
                    v.error(Some(&task.id), format!("read-scope path '{}': {}", path, e));
                }
            }

            // Gate templates must be known.
            if let Err(e) = validate_gate_templates(&task.gates, &self.project_root) {
                v.error(Some(&task.id), format!("{}", e));
            }
        }

        // Cross-task write-allow overlap — each colliding pair is an error.
        for (a, b, overlap) in overlap_problems(doc) {
            v.error(
                None,
                format!(
                    "tasks '{}' and '{}' have overlapping write-allow: {}",
                    a,
                    b,
                    overlap.join(", ")
                ),
            );
        }

        Ok(v)
    }

    /// Plan a confirmed PRD: validate, then create each task (gated) and record
    /// brainstorm provenance. A `draft` PRD is refused unless `dry_run`; a
    /// `superseded` PRD is always refused. In `dry_run`, nothing is persisted —
    /// the returned outcomes describe what would be created.
    pub fn prd_plan(
        &self,
        doc: &crate::application::prd::PrdDocument,
        alignment_path: Option<&str>,
        convergence_path: Option<&str>,
        dry_run: bool,
    ) -> Result<Vec<crate::application::prd::PrdPlanOutcome>> {
        use crate::application::prd::PrdStatus;

        // Status gate — superseded is never plannable, even as a dry run.
        if doc.status == PrdStatus::Superseded {
            return Err(anyhow!(
                "PRD status is 'superseded' — superseded by a later PRD; not plannable"
            ));
        }
        if !dry_run && doc.status != PrdStatus::Confirmed {
            return Err(anyhow!(
                "PRD status is '{}' — set it to 'confirmed' before planning, \
                 or run with --dry-run to preview",
                doc.status.as_str()
            ));
        }

        // Full validation — fail fast, create nothing on any error.
        let validation = self.prd_validate(doc)?;
        if !validation.ok() {
            let mut lines = String::from("PRD validation failed:\n");
            for p in validation.errors() {
                match &p.task_id {
                    Some(tid) => lines.push_str(&format!("  [{}] {}\n", tid, p.message)),
                    None => lines.push_str(&format!("  {}\n", p.message)),
                }
            }
            return Err(anyhow!("{}", lines.trim_end()));
        }

        let bs_id = crate::application::prd::brainstorm_id_for(&doc.title);
        let mut outcomes = Vec::with_capacity(doc.tasks.len());

        for task in &doc.tasks {
            // read-scope defaults to write-allow per the PRD convention.
            let read_scope: Vec<String> = if task.read_scope.is_empty() {
                task.write_allow.clone()
            } else {
                task.read_scope.clone()
            };

            if dry_run {
                outcomes.push(crate::application::prd::PrdPlanOutcome {
                    task_id: task.id.clone(),
                    objective: task.objective.clone(),
                    write_allow: task.write_allow.clone(),
                    gates: task.gates.clone(),
                    depends_on: task.depends_on.clone(),
                    created: false,
                    seq: None,
                    provenance_recorded: false,
                });
                continue;
            }

            let event = self.create_task(
                &task.id,
                CreateTaskInput {
                    objective: &task.objective,
                    read_scope: &read_scope,
                    write_allow: &task.write_allow,
                    write_deny: &[],
                    risk_triggers: &[],
                    gates: &task.gates,
                    depends_on: &task.depends_on,
                },
            )?;

            // Record brainstorm provenance when an alignment (divergence) path is
            // available. The PRD file is the convergence. Without a divergence
            // path, skip — record_brainstorm_artifacts requires one.
            let mut provenance_recorded = false;
            if let Some(divergence) = alignment_path {
                if self
                    .record_brainstorm_artifacts(
                        &task.id,
                        &bs_id,
                        divergence,
                        convergence_path,
                        None,
                    )
                    .is_ok()
                {
                    provenance_recorded = true;
                }
            }

            outcomes.push(crate::application::prd::PrdPlanOutcome {
                task_id: task.id.clone(),
                objective: task.objective.clone(),
                write_allow: task.write_allow.clone(),
                gates: task.gates.clone(),
                depends_on: task.depends_on.clone(),
                created: true,
                seq: Some(event.seq),
                provenance_recorded,
            });
        }

        Ok(outcomes)
    }

    /// Build the observable-loop status view for a parsed PRD: each task's
    /// existence, phase, and brainstorm provenance, plus a completion summary.
    /// Read-only — emits no events. Tasks not yet created show `exists: false`.
    pub fn prd_status_view(
        &self,
        doc: &crate::application::prd::PrdDocument,
    ) -> Result<crate::application::prd::PrdStatusView> {
        let mut rows = Vec::with_capacity(doc.tasks.len());
        let mut completed = 0;

        for task in &doc.tasks {
            match self.get_status(&task.id) {
                Ok(state) => {
                    let phase = format!("{:?}", state.phase).to_ascii_lowercase();
                    if state.phase == Phase::Completed {
                        completed += 1;
                    }
                    let provenance = self.brainstorm_provenance_view(&state);
                    rows.push(crate::application::prd::PrdTaskStatusRow {
                        id: task.id.clone(),
                        exists: true,
                        phase: Some(phase),
                        provenance,
                    });
                }
                Err(_) => {
                    // Task not created yet — it lives only in the PRD.
                    rows.push(crate::application::prd::PrdTaskStatusRow {
                        id: task.id.clone(),
                        exists: false,
                        phase: None,
                        provenance: None,
                    });
                }
            }
        }

        Ok(crate::application::prd::PrdStatusView {
            title: doc.title.clone(),
            status: doc.status,
            total: doc.tasks.len(),
            completed,
            rows,
        })
    }

    // ── Uncertainty Ledger V1: record-and-disclose unknowns ──
    //
    // record_uncertainty + record_uncertainty_disposition emit the two canonical
    // events; the view resolves evidence freshness against the working tree. Never
    // gates, never scores, never renders an aggregate verdict.
}
