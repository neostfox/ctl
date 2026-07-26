use super::*;

impl ControlApp {
    pub fn run_start(&self, task_id: &str, adapter_name: &str) -> Result<Event> {
        // Expire any stale leases before starting a new run
        let _ = self.expire_stale_leases(task_id);
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::InProgress {
            return Err(anyhow!(
                "Can only start run for InProgress tasks, current: {:?}",
                state.phase
            ));
        }
        if state.active_run.is_some() {
            return Err(anyhow!("Task already has an active run. Rule: RUN-002"));
        }

        // AC4: Cross-task lease write overlap check (ADAPTER-005)
        let write_allow: Vec<String> = state.write_allow.iter().cloned().collect();
        self.check_cross_task_lease_overlap(task_id, &write_allow)?;

        let run_id = generate_uuid();
        let lease_id = generate_uuid();

        // Create worktree
        let worktree_path =
            crate::infrastructure::workspace::create_worktree(&self.project_root, task_id)?;

        // Create lease
        let lease_payload = serde_json::json!({
            "lease_id": lease_id,
            "run_id": run_id,
            "resource_path": state.write_allow.iter().next().unwrap_or(&String::new()),
            "action": "write",
            "ttl_seconds": RUN_LEASE_TTL_SECONDS,
            "max_uses": RUN_LEASE_MAX_USES,
        });
        let lease_event = self.build_event(task_id, "lease_created", lease_payload)?;
        self.validate_and_append(&lease_event)?;

        // Generate run manifest
        let adapter = adapter_for(adapter_name)?;

        let write_deny: Vec<String> = state.write_deny.iter().cloned().collect();
        let gates: Vec<String> = state.gates.iter().cloned().collect();

        let manifest = adapter.prepare_run(
            task_id,
            &run_id,
            &lease_id,
            &worktree_path,
            &write_allow,
            &write_deny,
            &gates,
        )?;

        // Write run manifest atomically
        let task_dir = self.store.task_dir(task_id)?;
        let manifest_path = task_dir.join("run-manifest.json");
        if !self.dry_run {
            let temp_path = task_dir.join("run-manifest.json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&manifest)?)?;
            std::fs::rename(&temp_path, &manifest_path)?;
        }

        // Record workspace_created event
        let ws_payload = serde_json::json!({
            "worktree_path": worktree_path.to_string_lossy(),
            "branch": format!("omp-run-{}", task_id),
        });
        let ws_event = self.build_event(task_id, "workspace_created", ws_payload)?;
        self.validate_and_append(&ws_event)?;

        // Record run_started event
        let payload = serde_json::json!({
            "run_id": run_id,
            "adapter": adapter_name,
            "lease_id": lease_id,
        });
        let event = self.build_event(task_id, "run_started", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Ingest an agent-output result for `adapter_name` ("omp", "opencode", …).
    /// The adapter validates the result shape; evidence is tagged with the
    /// adapter's `source` so the audit trail stays unambiguous across adapters.
    pub fn run_ingest(
        &self,
        task_id: &str,
        result_file: &Path,
        adapter_name: &str,
    ) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.active_run.is_none() {
            return Err(anyhow!("No active run for task '{}'", task_id));
        }

        let content = std::fs::read_to_string(result_file)?;
        let result: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| anyhow!("Invalid result file: {}", e))?;

        // Validate via the selected adapter (source/shape contract).
        adapter_for(adapter_name)?.validate_output(&result)?;

        // Validate touched files against write scope
        let touched_files = result
            .get("touched_files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        for file_entry in &touched_files {
            let file_path = file_entry.as_str().unwrap_or("");
            if file_path.is_empty() {
                continue;
            }
            let normalized = normalizer
                .normalize(file_path)
                .map_err(|e| anyhow!("Invalid touched file '{}': {}", file_path, e))?;
            let normalized_str = normalized.to_string_lossy().replace('\\', "/");
            let in_scope = state.write_allow.iter().any(|scope| {
                crate::domain::task::path_within_scope(&normalized_str, &scope.replace('\\', "/"))
            });
            let in_deny = state.write_deny.iter().any(|scope| {
                crate::domain::task::path_within_scope(&normalized_str, &scope.replace('\\', "/"))
            });
            if !in_scope || in_deny {
                let evidence_id = generate_uuid();
                let payload = serde_json::json!({
                    "evidence_id": evidence_id,
                    "source": adapter_name,
                    "rejection_reason": format!("File '{}' is out of write scope or in deny list", file_path),
                    "touched_file": file_path,
                });
                let event = self.build_event(task_id, "evidence_rejected", payload)?;
                self.validate_and_append(&event)?;
                if !self.dry_run {
                    self.rebuild_task_view(task_id)?;
                }
                return Err(anyhow!(
                    "Evidence rejected: file '{}' is out of write scope or in deny list. Rule: SCOPE-001",
                    file_path
                ));
            }
        }

        // Write agent-output.json
        let evidence_id = generate_uuid();
        let output_path = self.store.task_dir(task_id)?.join("agent-output.json");
        if !self.dry_run {
            let temp_path = output_path.with_extension("json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&result)?)?;
            std::fs::rename(&temp_path, &output_path)?;
        }

        // Record run_completed
        let run_complete_payload = serde_json::json!({
            "run_id": state.active_run.as_ref().unwrap().run_id,
        });
        let rc_event = self.build_event(task_id, "run_completed", run_complete_payload)?;
        self.validate_and_append(&rc_event)?;

        // Revoke the lease now that the run is complete
        let lease_id = state.active_run.as_ref().unwrap().lease_id.clone();
        let revoke_payload = serde_json::json!({ "lease_id": lease_id });
        let revoke_event = self.build_event(task_id, "lease_revoked", revoke_payload)?;
        self.validate_and_append(&revoke_event)?;

        // Cleanup worktree
        let worktree_path = self.get_worktree_path(task_id)?;
        if worktree_path.exists() {
            let _ = crate::infrastructure::workspace::cleanup_worktree(
                &self.project_root,
                &worktree_path,
            );
            let ws_clean_payload = serde_json::json!({
                "worktree_path": worktree_path.to_string_lossy(),
            });
            let ws_clean_event =
                self.build_event(task_id, "workspace_cleaned", ws_clean_payload)?;
            self.validate_and_append(&ws_clean_event)?;
        }

        // Record evidence_accepted
        let payload = serde_json::json!({
            "evidence_id": evidence_id,
            "source": adapter_name,
            "result_file": result_file.to_string_lossy(),
            "touched_files": touched_files,
            "accepted_at": now_iso8601(),
        });
        let event = self.build_event(task_id, "evidence_accepted", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Abort an active run: revoke lease, cleanup worktree, emit run_failed.
    pub fn run_abort(&self, task_id: &str, reason: &str) -> Result<()> {
        let state = self.replay_task(task_id)?;
        let run_info = state
            .active_run
            .as_ref()
            .ok_or_else(|| anyhow!("No active run for task '{}'. Rule: RUN-001", task_id))?
            .clone();

        // Revoke active lease if present
        let lease = state.leases.get(&run_info.lease_id);
        if let Some(lease) = lease {
            if lease.status == LeaseStatus::Active {
                let payload = serde_json::json!({
                    "lease_id": lease.lease_id,
                });
                let event = self.build_event(task_id, "lease_revoked", payload)?;
                self.validate_and_append(&event)?;
            }
        }

        // Cleanup worktree if it exists
        let worktree_path = self
            .project_root
            .join(".ctl")
            .join("tasks")
            .join(task_id)
            .join("worktree");
        if worktree_path.exists() {
            let _ = crate::infrastructure::workspace::cleanup_worktree(
                &self.project_root,
                &worktree_path,
            );
            let payload = serde_json::json!({
                "worktree_path": worktree_path.to_string_lossy(),
            });
            let event = self.build_event(task_id, "workspace_cleaned", payload)?;
            self.validate_and_append(&event)?;
        }

        // Emit run_failed
        let payload = serde_json::json!({
            "run_id": run_info.run_id,
            "reason": reason,
        });
        let event = self.build_event(task_id, "run_failed", payload)?;
        self.validate_and_append(&event)?;

        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(())
    }

    // ── M6: AgentRun aggregate concurrency (slice 1) ──
    //
    // The M4 `run_start` path above is the single-executor flow (one
    // task-embedded `active_run`). These methods activate the independent
    // `AgentRun` aggregate under `.ctl/runs/<run_id>/` so that multiple
    // non-overlapping tasks can have concurrent runs, with a per-run scoped
    // lease whose write scope must be disjoint from every other active run.
    // No executor is ever spawned here — OMP drives execution off the prepared
    // manifest and results are ingested through the existing path.

    /// Lazily open the run-aggregate event store (`.ctl/runs/`). `init` only
    /// ensures the directory exists, so this is cheap and idempotent.
    pub(crate) fn run_store(&self) -> Result<RunEventStore> {
        RunEventStore::init(&self.project_root)
    }

    /// Replay a single AgentRun aggregate from `.ctl/runs/<run_id>/`.
    pub fn replay_run(&self, run_id: &str) -> Result<AgentRunState> {
        let store = self.run_store()?;
        let events = store.read_for_run(run_id)?;
        if events.is_empty() {
            return Err(anyhow!("Run '{}' not found", run_id));
        }
        let mut state = AgentRunState::new(run_id);
        for event in &events {
            apply_run(&mut state, event)
                .map_err(|e| anyhow!("Run reducer error at seq {}: {}", event.seq, e))?;
        }
        Ok(state)
    }

    /// Every run aggregate currently in the `Running` phase — the live
    /// concurrency set used for cross-run scoped-lease overlap rejection.
    pub fn active_runs(&self) -> Result<Vec<AgentRunState>> {
        let store = self.run_store()?;
        let mut active = Vec::new();
        for run_id in store.run_ids()? {
            let state = self.replay_run(&run_id)?;
            if state.phase == RunPhase::Running {
                active.push(state);
            }
        }
        Ok(active)
    }

    /// Build a run-scoped event. The run store keys directories on the event's
    /// `task_id` field, so it carries the run_id (mirroring `RunEventStore`).
    pub(crate) fn build_run_event(
        &self,
        run_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<Event> {
        // seq is a placeholder: the authoritative sequence number is assigned by
        // `append_run_event[_locked]` *inside* the per-run lock, so seq allocation
        // and the append are atomic (no unlocked read-seq race).
        Ok(Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: generate_uuid(),
            command_id: generate_uuid(),
            task_id: run_id.to_string(),
            seq: 0,
            occurred_at: now_iso8601(),
            actor: self.actor.clone(),
            event_type: event_type.to_string(),
            payload,
        })
    }

    /// Dry-run the run reducer over the existing stream + new event, then
    /// persist it and re-project `run.json`. The dry-run replay rejects illegal
    /// transitions before any bytes are written.
    ///
    /// Run-aggregate events are governed by the `apply_run` reducer plus the
    /// structural envelope check (`Event::is_valid`, enforced on read), NOT by
    /// the task-oriented per-type payload conditionals in the envelope JSON
    /// schema: there, `run_started` describes the M4 task-store run pointer
    /// (`run_id`+`adapter`+`lease_id`), which deliberately differs from the M6
    /// run-aggregate shape (`worktree_path`+`lease_id`). Validating run events
    /// against the task conditionals would be a category error.
    /// Append a run event, taking the per-run lock for the whole transaction.
    /// Use this for callers that do not already hold the lock (e.g. `create_run`).
    pub(crate) fn append_run_event(&self, run_id: &str, event: Event) -> Result<Event> {
        let store = self.run_store()?;
        // Single-writer: hold the per-run lock across seq allocation + validate +
        // append, so two processes cannot read the same max seq and append
        // conflicting events. Skipped in dry-run (nothing is persisted).
        let _lock = if self.dry_run {
            None
        } else {
            Some(store.lock_run(run_id)?)
        };
        self.append_run_event_locked(&store, run_id, event)
    }

    /// Locked core of the run-event append: assumes the caller already holds the
    /// per-run lock (e.g. `start_run`/`terminate_run`, which hold it across their
    /// filesystem side-effects too). Assigns the authoritative seq from the
    /// current stream, dry-run-validates via the reducer, then appends + projects.
    pub(crate) fn append_run_event_locked(
        &self,
        store: &RunEventStore,
        run_id: &str,
        mut event: Event,
    ) -> Result<Event> {
        let mut state = AgentRunState::new(run_id);
        let prior = store.read_for_run(run_id)?;
        let mut max_seq = 0;
        for p in &prior {
            apply_run(&mut state, p)
                .map_err(|e| anyhow!("Run reducer error at seq {}: {}", p.seq, e))?;
            if p.seq > max_seq {
                max_seq = p.seq;
            }
        }
        // Authoritative seq, allocated under the lock.
        event.seq = max_seq + 1;
        apply_run(&mut state, &event).map_err(|e| anyhow!("Run reducer rejected: {}", e))?;
        if self.dry_run {
            return Ok(event);
        }
        store.append(&event)?;
        store.write_run_view(run_id, &state)?;
        Ok(event)
    }

    /// Create a queued AgentRun for an InProgress task, returning the run_id.
    /// The run inherits the task's write scope, deny list, and gates; the
    /// adapter drives execution (only `omp` is supported in this slice).
    pub fn create_run(&self, task_id: &str, adapter_name: &str) -> Result<String> {
        let task = self.replay_task(task_id)?;
        if task.phase != Phase::InProgress {
            return Err(anyhow!(
                "Can only create a run for an InProgress task '{}', current: {:?}",
                task_id,
                task.phase
            ));
        }
        if task.write_allow.is_empty() {
            return Err(anyhow!(
                "Task '{}' has an empty write scope; concurrent runs are for write tasks",
                task_id
            ));
        }
        // Validate the adapter is supported (constructs and drops — cheap, ZST).
        adapter_for(adapter_name)?;
        let run_id = generate_uuid();
        let payload = serde_json::json!({
            "task_id": task_id,
            "adapter": adapter_name,
            "write_allow": task.write_allow.iter().collect::<Vec<_>>(),
            "write_deny": task.write_deny.iter().collect::<Vec<_>>(),
            "gates": task.gates.iter().collect::<Vec<_>>(),
        });
        let event = self.build_run_event(&run_id, "run_created", payload)?;
        self.append_run_event(&run_id, event)?;
        Ok(run_id)
    }

    /// M6 core invariant: a starting run's write scope must be disjoint from
    /// every *other* currently-Running run. Returns `Err` naming the first
    /// conflicting run and the overlapping paths. Fails closed.
    pub(crate) fn check_run_scope_overlap(
        &self,
        run_id: &str,
        write_allow: &BTreeSet<String>,
    ) -> Result<()> {
        for other in self.active_runs()? {
            if other.run_id == run_id {
                continue;
            }
            let overlap = crate::application::schedule::detect_write_scope_overlap(
                write_allow,
                &other.write_allow,
            );
            if !overlap.is_empty() {
                return Err(anyhow!(
                    "Run scope conflict: run '{}' (task '{}') is already running with overlapping write scope {:?}. Concurrent runs must have disjoint write scopes.",
                    other.run_id,
                    other.task_id,
                    overlap
                ));
            }
        }
        Ok(())
    }

    /// Start a queued run: enforce the disjoint-scope invariant, create a
    /// per-run isolated worktree, prepare the OMP manifest (no spawn), and
    /// record `run_started`. The overlap check runs *before* any side effect,
    /// so a rejected start leaves no worktree or events behind.
    pub fn start_run(&self, run_id: &str) -> Result<Event> {
        let store = self.run_store()?;
        // Lock order: registry → per-run (deadlock-free; create/terminate take
        // only the per-run lock). The registry lock serializes concurrent starts
        // so the cross-run overlap check + the run_started append are atomic — a
        // second start blocks here, then sees the first run as Running and is
        // rejected by the overlap check. The per-run lock additionally serializes
        // against create/terminate of THIS run and is held across the worktree +
        // manifest side-effects, not merely the append.
        let _registry = if self.dry_run {
            None
        } else {
            Some(store.lock_run_registry()?)
        };
        let _run_lock = if self.dry_run {
            None
        } else {
            Some(store.lock_run(run_id)?)
        };
        let run = self.replay_run(run_id)?;
        if run.phase != RunPhase::Queued {
            return Err(anyhow!(
                "Can only start a run from Queued, current: {:?}",
                run.phase
            ));
        }
        self.check_run_scope_overlap(run_id, &run.write_allow)?;

        let adapter = adapter_for(run.adapter.as_str())?;
        let lease_id = generate_uuid();
        let worktree_path =
            crate::infrastructure::workspace::run_worktree_path(&self.project_root, run_id);
        let write_allow: Vec<String> = run.write_allow.iter().cloned().collect();
        let write_deny: Vec<String> = run.write_deny.iter().cloned().collect();
        let gates: Vec<String> = run.gates.iter().cloned().collect();

        if !self.dry_run {
            // Worktree-per-agent: create only after the overlap check passes.
            crate::infrastructure::workspace::create_run_worktree(&self.project_root, run_id)?;
            let manifest = adapter.prepare_run(
                &run.task_id,
                run_id,
                &lease_id,
                &worktree_path,
                &write_allow,
                &write_deny,
                &gates,
            )?;
            let run_dir = store.run_dir(run_id);
            std::fs::create_dir_all(&run_dir)?;
            let manifest_path = run_dir.join("run-manifest.json");
            let temp_path = run_dir.join("run-manifest.json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&manifest)?)?;
            std::fs::rename(&temp_path, &manifest_path)?;
        }

        // Grant + immediately consume a NEW run-scoped lease, then start — all
        // three events appended under the registry+per-run critical section
        // already held here. start_run does not require any pre-existing lease;
        // it mints one. NOTE: these ledger appends are NOT atomic with the
        // worktree/manifest filesystem side-effects above. A crash between them
        // is surfaced read-only by `ctl run recover` (orphaned worktree, or a
        // Queued run holding a lease), never silently reconciled.
        let resource_path = write_allow.first().cloned().unwrap_or_default();
        let lease_created = self.build_run_event(
            run_id,
            "lease_created",
            serde_json::json!({
                "lease_id": lease_id,
                "run_id": run_id,
                "resource_path": resource_path,
                "action": "write",
                "ttl_seconds": RUN_LEASE_TTL_SECONDS,
                "max_uses": RUN_LEASE_MAX_USES,
                "task_id": run.task_id,
                "adapter": run.adapter,
                "scopes": write_allow, // == run.write_allow exactly (V1)
            }),
        )?;
        self.append_run_event_locked(&store, run_id, lease_created)?;

        let lease_used = self.build_run_event(
            run_id,
            "lease_used",
            serde_json::json!({ "lease_id": lease_id }),
        )?;
        self.append_run_event_locked(&store, run_id, lease_used)?;

        let payload = serde_json::json!({
            "worktree_path": worktree_path.to_string_lossy(),
            "lease_id": lease_id,
        });
        let event = self.build_run_event(run_id, "run_started", payload)?;
        // Lock already held (registry + per-run) — append via the locked core.
        self.append_run_event_locked(&store, run_id, event)
    }

    /// Finish a Running run (→ Completed), freeing its write scope so an
    /// overlapping run may then start. Best-effort worktree cleanup.
    pub fn finish_run(&self, run_id: &str) -> Result<Event> {
        self.finish_run_with_provenance(run_id, &RunProvenanceInput::default())
    }

    /// Finish a run, recording host-attested provenance (run-attestation-fields-v1).
    /// ctl sha256-hashes each supplied artifact file and stores the host-reported
    /// model/provider/timestamps/exit alongside — record-and-disclose, NOT a
    /// verified claim of what ran. Absent fields are simply not recorded.
    pub fn finish_run_with_provenance(
        &self,
        run_id: &str,
        prov: &RunProvenanceInput,
    ) -> Result<Event> {
        let mut payload = serde_json::Map::new();
        let mut put = |k: &str, v: Option<&String>| {
            if let Some(s) = v.filter(|s| !s.is_empty()) {
                payload.insert(k.to_string(), serde_json::json!(s));
            }
        };
        put("model", prov.model.as_ref());
        put("provider", prov.provider.as_ref());
        put("started_at", prov.started_at.as_ref());
        put("ended_at", prov.ended_at.as_ref());
        // Hash the artifacts ctl is given (any readable path — these are the
        // host's transient files; only the digest is recorded, never the path).
        for (key, path) in [
            ("instruction_hash", &prov.instruction_artifact),
            ("context_hash", &prov.context_artifact),
            ("output_hash", &prov.output_artifact),
        ] {
            if let Some(p) = path.as_ref().filter(|s| !s.is_empty()) {
                let hash = hash_file(std::path::Path::new(p))?;
                payload.insert(key.to_string(), serde_json::json!(hash));
            }
        }
        if let Some(code) = prov.exit_code {
            payload.insert("exit_code".to_string(), serde_json::json!(code));
        }
        self.terminate_run(run_id, "run_finished", serde_json::Value::Object(payload))
    }

    /// Mark a run failed (→ Failed) with a reason. Frees its write scope.
    pub fn fail_run(&self, run_id: &str, reason: &str) -> Result<Event> {
        self.terminate_run(
            run_id,
            "run_failed",
            serde_json::json!({ "reason": reason }),
        )
    }

    /// Abort a non-terminal run (→ Aborted) with a reason. Frees its scope.
    pub fn abort_run(&self, run_id: &str, reason: &str) -> Result<Event> {
        self.terminate_run(
            run_id,
            "run_aborted",
            serde_json::json!({ "reason": reason }),
        )
    }

    /// Shared terminal transition: clean up the run's worktree (best-effort)
    /// then record the terminal event. The run reducer enforces which source
    /// phases each terminal type is legal from.
    pub(crate) fn terminate_run(
        &self,
        run_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<Event> {
        let store = self.run_store()?;
        // Hold the per-run lock across the worktree cleanup side-effect AND the
        // append, so a terminate cannot interleave with a concurrent create/start
        // of the same run.
        let _run_lock = if self.dry_run {
            None
        } else {
            Some(store.lock_run(run_id)?)
        };
        let run = self.replay_run(run_id)?;
        if !self.dry_run {
            if let Some(ref wt) = run.worktree_path {
                let wt_path = Path::new(wt);
                if wt_path.exists() {
                    let _ = crate::infrastructure::workspace::cleanup_worktree(
                        &self.project_root,
                        wt_path,
                    );
                }
            }
        }
        // Revoke the run's native lease (if still Active) before the terminal
        // event, mirroring the M4 path. Appended under the per-run lock held here.
        if let Some(ref lease) = run.lease {
            if lease.status == crate::domain::lease::LeaseStatus::Active {
                let revoke = self.build_run_event(
                    run_id,
                    "lease_revoked",
                    serde_json::json!({ "lease_id": lease.lease_id }),
                )?;
                self.append_run_event_locked(&store, run_id, revoke)?;
            }
        }
        let event = self.build_run_event(run_id, event_type, payload)?;
        self.append_run_event_locked(&store, run_id, event)
    }

    /// Explicitly expire a run's lease **iff** it is past its wall-clock TTL
    /// (capability-lease-ttl-enforce-v1). Operator-invoked only — TTL is never
    /// auto-expired in a read path (that would make replay non-deterministic;
    /// it stays report-only in `recover`). This is the explicit, recorded
    /// counterpart: it refuses to touch a within-TTL or non-Active lease, and on
    /// `apply` appends a single `lease_expired` event. It does NOT terminate the
    /// run or any process — winding the run down is a separate `run recover
    /// --abort`. Preview unless `apply`.
    pub fn expire_run_lease(&self, run_id: &str, apply: bool) -> Result<LeaseExpiryReport> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.expire_run_lease_at(run_id, now, apply)
    }

    /// Testable core of [`expire_run_lease`] with an injected `now_epoch`.
    pub(crate) fn expire_run_lease_at(
        &self,
        run_id: &str,
        now_epoch: u64,
        apply: bool,
    ) -> Result<LeaseExpiryReport> {
        let store = self.run_store()?;
        let run = self.replay_run(run_id)?;
        let mut report = LeaseExpiryReport {
            run_id: run_id.to_string(),
            outcome: String::new(),
            age_secs: None,
            ttl_secs: None,
            detail: String::new(),
        };

        let lease = match &run.lease {
            Some(l) => l,
            None => {
                report.outcome = "no_lease".to_string();
                report.detail = "run holds no native lease (legacy or never started)".to_string();
                return Ok(report);
            }
        };
        report.ttl_secs = Some(lease.ttl_seconds);

        if lease.status != crate::domain::lease::LeaseStatus::Active {
            report.outcome = "not_active".to_string();
            report.detail = format!(
                "lease is already {} — nothing to expire",
                lease.status.token()
            );
            return Ok(report);
        }

        // Wall-clock age from the lease_created event's occurred_at in THIS run's
        // stream — the same source `recover_report` uses for `lease_stale`.
        let events = store.read_for_run(run_id)?;
        let created_epoch = event_occurred_at_by_seq(&events, lease.created_at_seq)
            .and_then(|s| parse_iso8601_to_epoch(&s));
        report.age_secs = created_epoch.map(|c| now_epoch.saturating_sub(c));

        let stale = created_epoch
            .map(|c| ttl_exceeded(now_epoch, c, lease.ttl_seconds))
            .unwrap_or(false);
        if !stale {
            report.outcome = "within_ttl".to_string();
            report.detail = format!(
                "lease within TTL (age {}s ≤ ttl {}s) — refusing to expire a fresh lease",
                report.age_secs.unwrap_or(0),
                lease.ttl_seconds
            );
            return Ok(report);
        }

        if !apply {
            report.outcome = "would_expire".to_string();
            report.detail = format!(
                "lease is past TTL (age {}s > ttl {}s) — re-run with --apply to record lease_expired",
                report.age_secs.unwrap_or(0),
                lease.ttl_seconds
            );
            return Ok(report);
        }

        // Apply: append a single lease_expired under the per-run lock.
        let lease_id = lease.lease_id.clone();
        let _lock = if self.dry_run {
            None
        } else {
            Some(store.lock_run(run_id)?)
        };
        let event = self.build_run_event(
            run_id,
            "lease_expired",
            serde_json::json!({ "lease_id": lease_id, "reason": "ttl_exceeded" }),
        )?;
        self.append_run_event_locked(&store, run_id, event)?;
        report.outcome = "expired".to_string();
        report.detail = format!(
            "recorded lease_expired (age {}s > ttl {}s)",
            report.age_secs.unwrap_or(0),
            lease.ttl_seconds
        );
        Ok(report)
    }

    // ── M6: crash recovery (slice 2) — read-only detection + explicit abort ──

    /// Crash-recovery snapshot of every `Running` run: whether its isolated
    /// worktree and prepared manifest are still on disk. A Running run whose
    /// `worktree_exists` is false is inconsistent — the orchestrator likely died
    /// mid-run — and recovery is to `abort_run` it (freeing its write scope)
    /// once a human confirms. Read-only: replays aggregates and stats the
    /// filesystem, never appends.
    pub fn recover_report(&self) -> Result<Vec<RunRecoveryStatus>> {
        let store = self.run_store()?;
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut out = Vec::new();
        for run in self.active_runs()? {
            let manifest_exists = store
                .run_dir(&run.run_id)
                .join("run-manifest.json")
                .exists();
            let worktree_exists = run
                .worktree_path
                .as_ref()
                .map(|wt| Path::new(wt).exists())
                .unwrap_or(false);

            // Structured lease projection + read-only staleness. Staleness is
            // wall-clock TTL computed from the lease_created event's occurred_at
            // in THIS run's stream — it is reported, never auto-expired.
            let (lease_status, lease_compat, remaining_uses, lease_stale, lease_nonactive) =
                match &run.lease {
                    Some(l) => {
                        let active = l.status == crate::domain::lease::LeaseStatus::Active;
                        let stale = if active {
                            let events = store.read_for_run(&run.run_id).unwrap_or_default();
                            event_occurred_at_by_seq(&events, l.created_at_seq)
                                .and_then(|s| parse_iso8601_to_epoch(&s))
                                .map(|created| now_epoch.saturating_sub(created) > l.ttl_seconds)
                                .unwrap_or(false)
                        } else {
                            false
                        };
                        (
                            l.status.token().to_string(),
                            "native".to_string(),
                            Some(l.remaining_uses),
                            stale,
                            !active,
                        )
                    }
                    None => (
                        "UNKNOWN".to_string(),
                        "pre_lease_run".to_string(),
                        None,
                        false,
                        false,
                    ),
                };

            out.push(RunRecoveryStatus {
                run_id: run.run_id.clone(),
                task_id: run.task_id.clone(),
                write_allow: run.write_allow.iter().cloned().collect(),
                worktree_path: run.worktree_path.clone(),
                worktree_exists,
                manifest_exists,
                lease_id: run.lease_id.clone(),
                lease_status,
                lease_compat,
                remaining_uses,
                lease_stale,
                lease_nonactive,
            });
        }
        Ok(out)
    }

    /// Runs that committed a lease but never reached `Running` — i.e. a crash
    /// between lease consumption and `run_started` within `start_run`. Read-only;
    /// the resolution is the existing explicit `ctl run recover --abort`.
    pub fn partial_start_runs(&self) -> Result<Vec<serde_json::Value>> {
        let store = self.run_store()?;
        let mut out = Vec::new();
        for run_id in store.run_ids()? {
            let run = self.replay_run(&run_id)?;
            if run.phase == RunPhase::Queued {
                if let Some(ref lease) = run.lease {
                    out.push(serde_json::json!({
                        "run_id": run.run_id,
                        "task_id": run.task_id,
                        "lease_id": lease.lease_id,
                        "lease_status": lease.status.token(),
                    }));
                }
            }
        }
        Ok(out)
    }

    /// Worktree directories under `.ctl/runs/` whose run is terminal or absent —
    /// leftover isolation dirs safe to prune. Returns their paths (read-only).
    pub fn orphaned_run_worktrees(&self) -> Result<Vec<String>> {
        let store = self.run_store()?;
        let mut orphans = Vec::new();
        for run_id in store.run_ids()? {
            let wt =
                crate::infrastructure::workspace::run_worktree_path(&self.project_root, &run_id);
            if !wt.exists() {
                continue;
            }
            // Worktree on disk but the run is no longer Running → leftover.
            let running = matches!(self.replay_run(&run_id), Ok(s) if s.phase == RunPhase::Running);
            if !running {
                orphans.push(wt.to_string_lossy().to_string());
            }
        }
        Ok(orphans)
    }

    /// M6 slice 3: read-only "can this run's work land, and if not, how do I
    /// recover?" verdict for a run aggregate's isolated worktree. Emits NO
    /// events and never merges. A run is `mergeable` iff every touched file is
    /// inside the run's write scope, none collides with another active run's
    /// scope, and the main workspace is clean in those paths. Each blocker is
    /// classified into a `recovery` entry with a recommended action (commit/stash
    /// the dirty main files, let the other run land first, or abort this run via
    /// `ctl run recover --abort`). High-risk changes are surfaced but do not
    /// themselves block.
    pub fn run_merge_candidate(&self, run_id: &str) -> Result<serde_json::Value> {
        let run = self.replay_run(run_id)?;
        let worktree_path = run.worktree_path.as_ref().ok_or_else(|| {
            anyhow!(
                "Run '{}' has no worktree (not started?) — nothing to merge",
                run_id
            )
        })?;
        let wt = Path::new(worktree_path);
        if !wt.exists() {
            return Err(anyhow!(
                "Run '{}' worktree is missing at {} — recover with `ctl run recover --abort {}`",
                run_id,
                worktree_path,
                run_id
            ));
        }

        let changes = crate::infrastructure::workspace::diff_worktree(&self.project_root, wt)?;
        let touched: Vec<String> = changes
            .iter()
            .flat_map(|c| c.paths().into_iter().map(|s| s.to_string()))
            .collect();
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        let empty_deny = std::collections::BTreeSet::new();

        // (1) Every touched file must be inside this run's write scope.
        let mut out_of_scope = Vec::new();
        for path in &touched {
            if !file_in_write_scope(&normalizer, path, &run.write_allow, &run.write_deny)? {
                out_of_scope.push(path.clone());
            }
        }

        // (2) No touched file may fall into another ACTIVE run's write scope.
        // Slice 1 already keeps active runs disjoint, so this is defense in
        // depth: it catches a run that wrote outside its own scope, into a
        // concurrently-running peer's territory.
        let mut cross_run_conflicts = Vec::new();
        for other in self.active_runs()? {
            if other.run_id == run_id {
                continue;
            }
            for path in &touched {
                if file_in_write_scope(&normalizer, path, &other.write_allow, &empty_deny)? {
                    cross_run_conflicts.push(serde_json::json!({
                        "path": path,
                        "conflicting_run": other.run_id,
                        "conflicting_task": other.task_id,
                    }));
                }
            }
        }

        // (3) The main workspace must be clean in the touched paths, else the
        // merge would clobber concurrent edits. Non-git / unverifiable → no
        // fabricated conflict.
        let workspace_conflicts = if touched.is_empty() {
            Vec::new()
        } else {
            crate::infrastructure::workspace::dirty_paths_in_scope(&self.project_root, &touched)?
                .unwrap_or_default()
        };

        let requires_approval: Vec<String> =
            crate::infrastructure::workspace::detect_high_risk(&changes)
                .iter()
                .map(|(risk, path)| format!("{}: {}", risk, path))
                .collect();

        // Classify each blocker into a recovery action.
        let mut blocking_reasons = Vec::new();
        let mut recovery = Vec::new();
        if !out_of_scope.is_empty() {
            blocking_reasons.push(format!(
                "{} file(s) outside run write scope",
                out_of_scope.len()
            ));
            recovery.push(serde_json::json!({
                "category": "out_of_scope",
                "paths": out_of_scope.clone(),
                "action": format!(
                    "the run wrote outside its scope — abort and re-scope: ctl run recover --abort {}",
                    run_id
                ),
            }));
        }
        if !cross_run_conflicts.is_empty() {
            blocking_reasons.push(format!(
                "{} cross-run scope conflict(s)",
                cross_run_conflicts.len()
            ));
            recovery.push(serde_json::json!({
                "category": "cross_run_conflict",
                "conflicts": cross_run_conflicts.clone(),
                "action": "another active run owns these paths — let it land or abort it first, then re-check",
            }));
        }
        if !workspace_conflicts.is_empty() {
            blocking_reasons.push(format!(
                "{} file(s) dirty in the main workspace",
                workspace_conflicts.len()
            ));
            recovery.push(serde_json::json!({
                "category": "dirty_main_workspace",
                "paths": workspace_conflicts.clone(),
                "action": "commit or stash these files in the main workspace, then re-run `ctl run merge-candidate`",
            }));
        }

        Ok(serde_json::json!({
            "run_id": run_id,
            "task_id": run.task_id,
            "mergeable": blocking_reasons.is_empty(),
            "touched_files": touched,
            "out_of_scope": out_of_scope,
            "cross_run_conflicts": cross_run_conflicts,
            "workspace_conflicts": workspace_conflicts,
            "requires_approval": requires_approval,
            "blocking_reasons": blocking_reasons,
            "recovery": recovery,
        }))
    }

    // ── M4: Helpers ──

    /// AC4: Check that no other task holds an active lease with overlapping write scope.
    /// ADAPTER-005: M6 前禁止多个 agent 并发写入。
    pub(crate) fn check_cross_task_lease_overlap(
        &self,
        current_task_id: &str,
        write_allow: &[String],
    ) -> Result<()> {
        let all_task_ids = self.store.task_ids()?;
        for other_task_id in &all_task_ids {
            if other_task_id == current_task_id {
                continue;
            }
            let other_state = self.replay_task(other_task_id)?;
            for lease in other_state.leases.values() {
                if lease.status != LeaseStatus::Active {
                    continue;
                }
                // Check if the lease's resource_path overlaps with our write_allow
                let lease_resource = lease.resource_path.replace('\\', "/");
                let has_overlap = write_allow.iter().any(|scope| {
                    crate::domain::task::scopes_overlap(&lease_resource, &scope.replace('\\', "/"))
                });
                if has_overlap {
                    return Err(anyhow!(
                        "Cross-task lease conflict: task '{}' holds active lease '{}' on '{}' which overlaps with this task's write scope. Rule: ADAPTER-005",
                        other_task_id, lease.lease_id, lease.resource_path
                    ));
                }
            }
        }
        Ok(())
    }

    /// AUDIT-001: Verify lease is active, not expired, and has remaining uses.
    /// Also checks wall-clock TTL by reading occurred_at from the event stream.
    pub(crate) fn check_lease_valid(&self, task_id: &str, state: &TaskState) -> Result<()> {
        let run_info = state
            .active_run
            .as_ref()
            .ok_or_else(|| anyhow!("No active run — cannot apply without an active lease"))?;
        let lease = state
            .leases
            .get(&run_info.lease_id)
            .ok_or_else(|| anyhow!("Lease '{}' not found", run_info.lease_id))?;
        if lease.status != LeaseStatus::Active {
            return Err(anyhow!(
                "Lease '{}' is not active (status: {:?}). Rule: AUDIT-001",
                lease.lease_id,
                lease.status
            ));
        }
        if lease.remaining_uses == 0 {
            return Err(anyhow!(
                "Lease '{}' has no remaining uses. Rule: AUDIT-001",
                lease.lease_id
            ));
        }
        // TTL wall-clock check at application layer
        let events = self.store.read_for_task(task_id)?;
        if let Some(created_at_str) = event_occurred_at_by_seq(&events, lease.created_at_seq) {
            if let Some(created_epoch) = parse_iso8601_to_epoch(&created_at_str) {
                let now_epoch = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if now_epoch.saturating_sub(created_epoch) > lease.ttl_seconds {
                    return Err(anyhow!(
                        "Lease '{}' TTL exceeded ({}s > {}s). Rule: AUDIT-001",
                        lease.lease_id,
                        now_epoch.saturating_sub(created_epoch),
                        lease.ttl_seconds
                    ));
                }
            }
            // If parsing fails: fail-closed (already checked max_uses above)
        }
        Ok(())
    }

    /// Scan all active leases for a task and emit lease_expired for any that exceeded TTL.
    pub(crate) fn expire_stale_leases(&self, task_id: &str) -> Result<()> {
        let state = self.replay_task(task_id)?;
        let events = self.store.read_for_task(task_id)?;
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        for lease in state.leases.values() {
            if lease.status != LeaseStatus::Active {
                continue;
            }
            if let Some(created_at_str) = event_occurred_at_by_seq(&events, lease.created_at_seq) {
                if let Some(created_epoch) = parse_iso8601_to_epoch(&created_at_str) {
                    if now_epoch.saturating_sub(created_epoch) > lease.ttl_seconds {
                        let payload = serde_json::json!({
                            "lease_id": lease.lease_id,
                            "reason": "ttl_exceeded",
                        });
                        let event = self.build_event(task_id, "lease_expired", payload)?;
                        self.validate_and_append(&event)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Record `approval_expired` for any granted approval whose TTL has elapsed.
    ///
    /// Mirrors `expire_stale_leases`. Without this, an expired approval was only
    /// ever *read* as invalid at the apply gate (lazy invalidation) and the ledger
    /// never recorded the expiry transition — the `approval_expired` event and the
    /// `ApprovalStatus::Expired` state were reachable only via replay/tests.
    /// Idempotent: once expired, `is_granted()` is false, so a subsequent call
    /// skips it (no duplicate event). The schema for `approval_expired` permits
    /// only `request_id`, so no `reason` field is emitted.
    pub(crate) fn expire_stale_approvals(&self, task_id: &str) -> Result<()> {
        let state = self.replay_task(task_id)?;
        let events = self.store.read_for_task(task_id)?;
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        for approval in state.pending_approvals.values() {
            if !approval.is_granted() {
                continue;
            }
            if let Some(granted_seq) = approval.granted_at_seq {
                if let Some(granted_at_str) = event_occurred_at_by_seq(&events, granted_seq) {
                    if let Some(granted_epoch) = parse_iso8601_to_epoch(&granted_at_str) {
                        if now_epoch.saturating_sub(granted_epoch) > approval.ttl_seconds {
                            let payload = serde_json::json!({
                                "request_id": approval.request_id,
                            });
                            let event = self.build_event(task_id, "approval_expired", payload)?;
                            self.validate_and_append(&event)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn ingest_manual_result(&self, task_id: &str, result_file: &Path) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::InProgress && state.phase != Phase::Review {
            return Err(anyhow!(
                "Can only ingest results for InProgress or Review tasks, current: {:?}",
                state.phase
            ));
        }

        // Read and parse the result file
        let content = std::fs::read_to_string(result_file)?;
        let result: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| anyhow!("Invalid result file: {}", e))?;

        // Validate required fields
        let source = result.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if source != "manual" {
            return Err(anyhow!(
                "Result file must have source=\"manual\". Rule: ADAPTER-001"
            ));
        }

        let touched_files = result
            .get("touched_files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Validate all touched files are within write_allow scope
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        for file_entry in &touched_files {
            let file_path = file_entry.as_str().unwrap_or("");
            if file_path.is_empty() {
                continue;
            }
            let normalized = normalizer
                .normalize(file_path)
                .map_err(|e| anyhow!("Invalid touched file '{}': {}", file_path, e))?;
            let normalized_str = normalized.to_string_lossy().replace('\\', "/");
            let in_scope = state.write_allow.iter().any(|scope| {
                crate::domain::task::path_within_scope(&normalized_str, &scope.replace('\\', "/"))
            });
            let in_deny = state.write_deny.iter().any(|scope| {
                crate::domain::task::path_within_scope(&normalized_str, &scope.replace('\\', "/"))
            });
            if !in_scope || in_deny {
                // Reject evidence: file out of scope
                let evidence_id = generate_uuid();
                let payload = serde_json::json!({
                    "evidence_id": evidence_id,
                    "source": "manual",
                    "rejection_reason": format!("File '{}' is out of write scope or in deny list", file_path),
                    "touched_file": file_path,
                });
                let event = self.build_event(task_id, "evidence_rejected", payload)?;
                self.validate_and_append(&event)?;
                if !self.dry_run {
                    self.rebuild_task_view(task_id)?;
                }
                return Err(anyhow!(
                    "Evidence rejected: file '{}' is out of write scope or in deny list. Rule: SCOPE-001",
                    file_path
                ));
            }
        }

        // Generate evidence_id and write agent-output.json
        let evidence_id = generate_uuid();
        let output_path = self.store.task_dir(task_id)?.join("agent-output.json");
        if !self.dry_run {
            let temp_path = output_path.with_extension("json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&result)?)?;
            std::fs::rename(&temp_path, &output_path)?;
        }

        let payload = serde_json::json!({
            "evidence_id": evidence_id,
            "source": "manual",
            "result_file": result_file.to_string_lossy(),
            "touched_files": touched_files,
            "accepted_at": now_iso8601(),
        });
        let event = self.build_event(task_id, "evidence_accepted", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }
}
