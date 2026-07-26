use super::super::*;

pub(crate) fn workspace_created(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::InProgress {
            return Err(format!(
                "Can only create workspace in InProgress, current: {:?}",
                state.phase
            ));
        }
        let worktree_path = event
            .payload
            .get("worktree_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if worktree_path.is_empty() {
            return Err("workspace_created: worktree_path is required".into());
        }
    }
    Ok(())
}

pub(crate) fn workspace_cleaned(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::InProgress {
            return Err(format!(
                "workspace_cleaned only valid in InProgress, current: {:?}",
                state.phase
            ));
        }
        let worktree_path = event
            .payload
            .get("worktree_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if worktree_path.is_empty() {
            return Err("workspace_cleaned: worktree_path is required".into());
        }
    }
    Ok(())
}

pub(crate) fn workspace_diff_computed(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::InProgress {
            return Err(format!(
                "workspace_diff_computed only valid in InProgress, current: {:?}",
                state.phase
            ));
        }
        // Diff computed is informational; no state mutation.
        // Validate required arrays exist.
        for field in [
            "files_added",
            "files_modified",
            "files_deleted",
            "high_risk",
        ] {
            if event
                .payload
                .get(field)
                .and_then(|v| v.as_array())
                .is_none()
            {
                return Err(format!(
                    "workspace_diff_computed: '{}' must be an array",
                    field
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn workspace_applied(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::InProgress {
            return Err(format!(
                "workspace_applied only valid in InProgress, current: {:?}",
                state.phase
            ));
        }
        let files = event
            .payload
            .get("files_applied")
            .and_then(|v| v.as_array());
        if files.is_none() {
            return Err("workspace_applied: files_applied must be an array".into());
        }
    }
    // ── M4: Run lifecycle events ──
    Ok(())
}

pub(crate) fn run_started(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::InProgress {
            return Err(format!(
                "run_started only valid in InProgress, current phase: {:?}",
                state.phase
            ));
        }
        if state.active_run.is_some() {
            return Err("Cannot start run: another run is already active".into());
        }
        let run_id = event
            .payload
            .get("run_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let adapter = event
            .payload
            .get("adapter")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let lease_id = event
            .payload
            .get("lease_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if run_id.is_empty() || adapter.is_empty() || lease_id.is_empty() {
            return Err("run_started: run_id, adapter, and lease_id are required".into());
        }
        state.active_run = Some(RunInfo {
            run_id: run_id.to_string(),
            adapter: adapter.to_string(),
            lease_id: lease_id.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn run_completed(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        if state.active_run.is_none() {
            return Err("Cannot complete run: no active run".into());
        }
        state.active_run = None;
    }
    Ok(())
}

pub(crate) fn run_failed(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        // run_failed clears active_run regardless of state
        state.active_run = None;
    }
    // ── M4: Lease events ──
    Ok(())
}

pub(crate) fn lease_created(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        let lease_id = event
            .payload
            .get("lease_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if lease_id.is_empty() {
            return Err("lease_created: lease_id is required".into());
        }
        if state.leases.contains_key(lease_id) {
            return Err(format!("Duplicate lease_id: {}", lease_id));
        }
        let run_id = event
            .payload
            .get("run_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let resource_path = event
            .payload
            .get("resource_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let action = event
            .payload
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ttl_seconds = event
            .payload
            .get("ttl_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let max_uses = event
            .payload
            .get("max_uses")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        // Delegate to the typed transition; map its errors back to this
        // aggregate's long-standing messages (byte-identical to pre-refactor).
        let lease = LeaseState::grant(LeaseGrant {
            lease_id: lease_id.to_string(),
            run_id: run_id.to_string(),
            resource_path: resource_path.to_string(),
            action: action.to_string(),
            ttl_seconds,
            max_uses,
            created_at_seq: event.seq,
            // Task-aggregate leases carry no run-binding fields.
            task_id: String::new(),
            adapter: String::new(),
            scopes: std::collections::BTreeSet::new(),
        })
        .map_err(|e| match e {
            LeaseError::InvalidRunId | LeaseError::InvalidResource | LeaseError::InvalidAction => {
                "lease_created: run_id, resource_path, and action are required".to_string()
            }
            LeaseError::InvalidTtl | LeaseError::InvalidMaxUses => {
                "lease_created: ttl_seconds and max_uses must be > 0".to_string()
            }
            other => format!("lease_created: {}", other),
        })?;
        state.leases.insert(lease_id.to_string(), lease);
    }
    Ok(())
}

pub(crate) fn lease_used(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        let lease_id = event
            .payload
            .get("lease_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if lease_id.is_empty() {
            return Err("lease_used: lease_id is required".into());
        }
        let lease = state
            .leases
            .get_mut(lease_id)
            .ok_or_else(|| format!("Unknown lease_id: {}", lease_id))?;
        lease.consume().map_err(|e| match e {
            LeaseError::NotActive => format!("Lease '{}' is not active", lease_id),
            LeaseError::NoRemainingUses => {
                format!("Lease '{}' has no remaining uses", lease_id)
            }
            other => format!("Lease '{}': {}", lease_id, other),
        })?;
    }
    Ok(())
}

pub(crate) fn lease_expired(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        let lease_id = event
            .payload
            .get("lease_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if lease_id.is_empty() {
            return Err("lease_expired: lease_id is required".into());
        }
        let lease = state
            .leases
            .get_mut(lease_id)
            .ok_or_else(|| format!("Unknown lease_id: {}", lease_id))?;
        lease.expire();
    }
    Ok(())
}

pub(crate) fn lease_revoked(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        let lease_id = event
            .payload
            .get("lease_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if lease_id.is_empty() {
            return Err("lease_revoked: lease_id is required".into());
        }
        let lease = state
            .leases
            .get_mut(lease_id)
            .ok_or_else(|| format!("Unknown lease_id: {}", lease_id))?;
        lease.revoke();
    }
    // ── M4: Approval events ──
    Ok(())
}
