use super::*;

impl ControlApp {
    pub(crate) fn build_event(
        &self,
        task_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<Event> {
        let seq = self.store.next_seq_for_task(task_id)?;
        Ok(Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: generate_uuid(),
            command_id: generate_uuid(),
            task_id: task_id.to_string(),
            seq,
            occurred_at: now_iso8601(),
            actor: self.actor.clone(),
            event_type: event_type.to_string(),
            payload,
        })
    }

    /// Normalize boundary-scope paths (read_scope / write_allow / write_deny)
    /// against structural boundary rules: rejects escape (`..`), absolute,
    /// UNC, drive prefixes, symlinks/junctions, and root escapes.
    ///
    /// Protected paths are NOT rejected here. Protection is enforced once, at
    /// the runtime gate (`classify_write_target` → `is_protected`): a protected
    /// path declared in write_allow is accepted at create/revise but still
    /// requires a `ctl apply` exception before it can actually be written. This
    /// converges protected handling with gate-observe-mode — the create-time
    /// reject was a redundant early-deny that blocked governed work on
    /// protected paths (Cargo.toml, schemas, ledgers).
    pub(crate) fn normalize_boundary_paths(
        &self,
        field: &str,
        paths: &[String],
    ) -> Result<Vec<String>> {
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        let mut normalized = Vec::with_capacity(paths.len());
        for path in paths {
            let np = normalizer
                .normalize(path)
                .map_err(|e| anyhow!("Invalid {} path '{}': {}", field, path, e))?;
            normalized.push(path_to_payload_string(&np));
        }
        Ok(normalized)
    }

    pub(crate) fn validate_event(&self, event: &Event) -> Result<()> {
        if matches!(event.event_type.as_str(), "task_created" | "task_revised")
            && event.payload.get("scope").is_some()
        {
            return Err(anyhow!(
                "Legacy task boundary field 'scope' is not accepted in M1 events"
            ));
        }

        // 1. Schema validation (when schemas/ available)
        if let Some(ref validator) = self.validator {
            let json_val = serde_json::to_value(event)?;
            validator
                .validate_instance(&json_val, &event.schema)
                .map_err(|e| anyhow!("Schema validation failed: {}", e))?;
        }

        // 2. Dry-run reducer against the existing canonical stream.
        let mut state = TaskState::new(&event.task_id);
        for prior in self.store.read_for_task(&event.task_id)? {
            apply(&mut state, &prior)
                .map_err(|e| anyhow!("Reducer error at seq {}: {}", prior.seq, e))?;
        }
        apply(&mut state, event).map_err(|e| anyhow!("Reducer rejected: {}", e))
    }

    pub(crate) fn validate_and_append(&self, event: &Event) -> Result<()> {
        if self.dry_run {
            self.validate_event(event)?;
            println!(
                "[dry-run] Would append event: type={}, task={}, seq={}",
                event.event_type, event.task_id, event.seq
            );
            return Ok(());
        }
        // Single-writer: hold a per-task lock across validate + append so the
        // sequence read inside `validate_event` and the append are atomic across
        // processes. A concurrent writer that built the same seq will, once it
        // acquires the lock, re-read the now-longer stream and be rejected by the
        // reducer's "Sequence error" rather than appending a duplicate.
        let _lock = self.store.lock_task(&event.task_id)?;
        self.validate_event(event)?;
        self.store.append(event)?;
        Ok(())
    }

    pub(crate) fn rebuild_task_view(&self, task_id: &str) -> Result<()> {
        let state = self.replay_task(task_id)?;
        self.store.write_task_view(task_id, &state)?;
        Ok(())
    }

    // ── M4: Workspace commands ──
}
