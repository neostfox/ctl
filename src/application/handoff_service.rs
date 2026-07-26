use super::*;

impl ControlApp {
    /// Read-only handoff artifact for a task (ctl-handoff-v1): a portable
    /// snapshot another session or human can pick up from — objective +
    /// boundary, per-gate status, the completion-interlock verdict, the
    /// drift-derived next action, the uncommitted files inside the task's write
    /// scope, and the recent event tail. Appends nothing, mutates nothing; every
    /// field comes from an existing read-only query.
    pub fn handoff_export(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;
        let events = self.store.read_for_task(task_id)?;

        let gate_status: Vec<_> = state
            .gates
            .iter()
            .map(|g| {
                let r = state.gate_results.get(g);
                serde_json::json!({
                    "gate": g,
                    "status": match r {
                        Some(r) if r.passed => "PASS",
                        Some(_) => "FAIL",
                        None => "PENDING",
                    },
                    "checked_at": r.map(|r| r.checked_at.clone()),
                })
            })
            .collect();

        // Completion-interlock verdict — works in any phase ("block" outside
        // Review); omitted only if the audit projection itself errors.
        let interlock = self
            .generate_audit_report(task_id)
            .ok()
            .and_then(|a| a.get("completion_interlock").cloned());

        // Drift-derived recommended next action (read-only).
        let next_action = self.next_action(task_id).ok().map(|p| {
            serde_json::json!({
                "action": format!("{:?}", p.action),
                "level": format!("{:?}", p.level),
                "rationale": p.rationale,
                "suggested_command": p.suggested_command,
            })
        });

        // Uncommitted files inside the task's write scope (None if non-git).
        let write_allow: Vec<String> = state.write_allow.iter().cloned().collect();
        let uncommitted = crate::infrastructure::workspace::dirty_paths_in_scope(
            &self.project_root,
            &write_allow,
        )?;

        // Recent event tail (chronological).
        let start = events.len().saturating_sub(10);
        let recent_events: Vec<_> = events[start..]
            .iter()
            .map(|e| {
                serde_json::json!({
                    "seq": e.seq,
                    "type": e.event_type,
                    "at": e.occurred_at,
                    "actor": e.actor,
                })
            })
            .collect();
        let capture = self.read_handoff_capture(task_id)?;

        Ok(serde_json::json!({
            "schema": "control.handoff.v1",
            "task_id": task_id,
            "phase": format!("{:?}", state.phase),
            "is_held": state.is_held,
            "objective": state.objective,
            "boundary": {
                "read_scope": state.read_scope,
                "write_allow": state.write_allow,
                "write_deny": state.write_deny,
                "gates": state.gates,
            },
            "gate_status": gate_status,
            "interlock": interlock,
            "next_action": next_action,
            "uncommitted_in_scope": uncommitted,
            "recent_events": recent_events,
            "capture": capture,
        }))
    }

    /// Read a captured, non-canonical handoff judgment if one exists.
    pub(crate) fn read_handoff_capture(&self, task_id: &str) -> Result<Option<serde_json::Value>> {
        let path = self
            .project_root
            .join(".ctl")
            .join("handoffs")
            .join(format!("{task_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Invalid handoff capture {}: {e}", path.display()))?;
        validate_handoff_capture(&value, task_id)?;
        Ok(Some(value))
    }

    /// Persist explicit agent/human judgment beside, but outside, canonical task state.
    pub fn capture_handoff(&self, task_id: &str, input_path: &Path) -> Result<serde_json::Value> {
        self.replay_task(task_id)?;
        let normalized = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        )
        .normalize(&input_path.to_string_lossy())?;
        let source_path = self.project_root.join(normalized);
        let content = std::fs::read_to_string(&source_path)
            .with_context(|| format!("reading handoff capture {}", source_path.display()))?;
        let mut value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Invalid handoff capture input: {e}"))?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| anyhow!("Handoff capture input must be a JSON object"))?;
        object.insert(
            "schema".to_string(),
            serde_json::json!("control.handoff.capture.v1"),
        );
        object.insert("task_id".to_string(), serde_json::json!(task_id));
        object.insert(
            "source".to_string(),
            serde_json::json!("agent_or_human_supplied"),
        );
        object.insert("captured_at".to_string(), serde_json::json!(now_iso8601()));
        validate_handoff_capture(&value, task_id)?;

        let dir = self.project_root.join(".ctl").join("handoffs");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{task_id}.json"));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&value)?)?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(value)
    }
}
