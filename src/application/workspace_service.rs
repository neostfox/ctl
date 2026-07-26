use super::*;

impl ControlApp {
    pub fn workspace_create(&self, task_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::InProgress {
            return Err(anyhow!(
                "Can only create workspace for InProgress tasks, current: {:?}",
                state.phase
            ));
        }

        let worktree_path =
            crate::infrastructure::workspace::create_worktree(&self.project_root, task_id)?;
        let branch = format!("omp-run-{}", task_id);

        let payload = serde_json::json!({
            "worktree_path": worktree_path.to_string_lossy(),
            "branch": branch,
        });
        let event = self.build_event(task_id, "workspace_created", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn workspace_diff(&self, task_id: &str) -> Result<serde_json::Value> {
        let _state = self.replay_task(task_id)?;
        let worktree_path = self.get_worktree_path(task_id)?;

        let changes =
            crate::infrastructure::workspace::diff_worktree(&self.project_root, &worktree_path)?;

        let high_risks = crate::infrastructure::workspace::detect_high_risk(&changes);

        let mut files_added = Vec::new();
        let mut files_modified = Vec::new();
        let mut files_deleted = Vec::new();

        use crate::infrastructure::workspace::Change;
        for change in &changes {
            match change {
                Change::Add(p) => files_added.push(p.clone()),
                Change::Modify(p) => files_modified.push(p.clone()),
                Change::Delete(p) => files_deleted.push(p.clone()),
                // A rename is a delete of the old path + add of the new one.
                Change::Rename { from, to } => {
                    files_deleted.push(from.clone());
                    files_added.push(to.clone());
                }
            }
        }

        // Auto-create approval requests for high-risk changes
        let high_risk_descriptions: Vec<String> = high_risks
            .iter()
            .map(|(risk_type, path)| format!("{}: {}", risk_type, path))
            .collect();

        if !high_risks.is_empty() {
            let scope = serde_json::json!({
                "high_risk_files": high_risks.iter().map(|(_, p)| p).collect::<Vec<_>>(),
                "diff_summary": {
                    "added": files_added.len(),
                    "modified": files_modified.len(),
                    "deleted": files_deleted.len(),
                },
            });
            let request_id = generate_uuid();
            let approval_payload = serde_json::json!({
                "request_id": request_id,
                "reason": format!("High-risk changes detected: {} file(s)", high_risks.len()),
                "scope": scope,
                "ttl_seconds": 86400,
            });
            let event = self.build_event(task_id, "approval_requested", approval_payload)?;
            self.validate_and_append(&event)?;
        }

        // Record diff_computed event
        let payload = serde_json::json!({
            "files_added": files_added,
            "files_modified": files_modified,
            "files_deleted": files_deleted,
            "high_risk": high_risk_descriptions,
        });
        let event = self.build_event(task_id, "workspace_diff_computed", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }

        Ok(serde_json::json!({
            "task_id": task_id,
            "files_added": files_added,
            "files_modified": files_modified,
            "files_deleted": files_deleted,
            "high_risk": high_risk_descriptions,
        }))
    }

    pub fn workspace_apply(&self, task_id: &str) -> Result<Event> {
        // Expire any stale leases before applying
        let _ = self.expire_stale_leases(task_id);
        // Record expiry of any stale approvals before the approval gate below, so
        // the ledger reflects the transition rather than the gate lazily reading a
        // still-"granted" approval as invalid (mirrors lease expiry above).
        let _ = self.expire_stale_approvals(task_id);
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::InProgress {
            return Err(anyhow!(
                "Can only apply workspace for InProgress tasks, current: {:?}",
                state.phase
            ));
        }

        // AUDIT-001: Verify active lease before applying writes
        self.check_lease_valid(task_id, &state)?;

        let worktree_path = self.get_worktree_path(task_id)?;
        let changes =
            crate::infrastructure::workspace::diff_worktree(&self.project_root, &worktree_path)?;
        let high_risks = crate::infrastructure::workspace::detect_high_risk(&changes);

        // Check all touched paths are within write_allow. For a rename this
        // covers both the removed and the created path.
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        for change in &changes {
            for path in change.paths() {
                if !file_in_write_scope(&normalizer, path, &state.write_allow, &state.write_deny)? {
                    return Err(anyhow!(
                        "File '{}' is out of write scope or in deny list. Rule: scope_enforcement",
                        path
                    ));
                }
                // Protected-path hard deny: match the runtime write gate's
                // single enforcement point. Previously this path only
                // consulted `detect_high_risk`, whose prefix list
                // (`.omp/`, `.ctl/spec/`, `schemas/`, `Cargo.{toml,lock}`) is
                // a strict subset of `PathNormalizer::is_protected` — so a
                // changeset touching the canonical ledger
                // (`.ctl/tasks/<id>/events.jsonl`), `.git/config`, or
                // anything under `.control/` was applied silently with no
                // approval. Reject here; a reviewed exception for a protected
                // path goes through `ctl apply` (audited per-path), not
                // `ctl workspace apply`.
                match normalizer.normalize(path) {
                    Ok(np) if normalizer.is_protected(&np) => {
                        return Err(anyhow!(
                            "File '{}' is a protected path (canonical ledgers, \
                             manifests, schemas, .git, .control are never writable \
                             via workspace apply). Rule: PROTECTED-001. Use \
                             `ctl apply --path {} --reason <why>` to request a \
                             reviewed per-path exception.",
                            path,
                            path
                        ));
                    }
                    _ => {}
                }
            }
        }

        // Check high-risk changes have approval (with TTL check)
        let all_events = self.store.read_for_task(task_id)?;
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for (risk_type, path) in &high_risks {
            let has_valid_approval = state.pending_approvals.values().any(|a| {
                if !a.is_granted() {
                    return false;
                }
                // Check the file is in scope
                let in_scope = a
                    .scope
                    .get("high_risk_files")
                    .and_then(|v| v.as_array())
                    .is_some_and(|files| files.iter().any(|f| f.as_str() == Some(path)));
                if !in_scope {
                    return false;
                }
                // TTL check: granted_at must not be older than ttl_seconds
                if let Some(granted_seq) = a.granted_at_seq {
                    if let Some(granted_at_str) = event_occurred_at_by_seq(&all_events, granted_seq)
                    {
                        if let Some(granted_epoch) = parse_iso8601_to_epoch(&granted_at_str) {
                            return now_epoch.saturating_sub(granted_epoch) <= a.ttl_seconds;
                        }
                    }
                }
                // If we can't determine grant time, fail-closed
                false
            });
            if !has_valid_approval {
                return Err(anyhow!(
                    "High-risk change '{}' on '{}' requires valid approval (not expired). Rule: APPROVAL-001. Grant with: ctl approval grant --id {} --request <request_id>",
                    risk_type, path, task_id
                ));
            }
        }

        // Emit lease_used event (consumes one lease use)
        let lease_id = state.active_run.as_ref().unwrap().lease_id.clone();
        let lease_used_payload = serde_json::json!({
            "lease_id": lease_id,
        });
        let lease_used_event = self.build_event(task_id, "lease_used", lease_used_payload)?;
        self.validate_and_append(&lease_used_event)?;

        // Apply the changeset: creates/modifies/deletes/renames in the main
        // workspace per each change's kind (no longer copy-only).
        crate::infrastructure::workspace::apply_changes(
            &self.project_root,
            &worktree_path,
            &changes,
        )?;

        // Record every path touched in the main workspace (a rename touches the
        // old and new path; a delete records the removed path).
        let files_applied: Vec<String> = changes
            .iter()
            .flat_map(|c| c.paths().into_iter().map(|s| s.to_string()))
            .collect();
        let payload = serde_json::json!({
            "files_applied": files_applied,
        });
        let event = self.build_event(task_id, "workspace_applied", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn workspace_cleanup(&self, task_id: &str) -> Result<Event> {
        let worktree_path = self.get_worktree_path(task_id)?;
        crate::infrastructure::workspace::cleanup_worktree(&self.project_root, &worktree_path)?;

        let payload = serde_json::json!({
            "worktree_path": worktree_path.to_string_lossy(),
        });
        let event = self.build_event(task_id, "workspace_cleaned", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// M6: Read-only "is this worktree a clean merge candidate?" verdict.
    ///
    /// Emits NO events and never merges — the human reviews this, then runs
    /// `workspace apply` to actually merge. A candidate is `mergeable` iff all
    /// touched files are in the task's write scope, none collide with another
    /// active task's write scope, and the main workspace has no conflicting
    /// dirty state in those paths. High-risk changes are surfaced for human
    /// attention but do not by themselves block the candidate (the apply path
    /// still gates them via approval).
    pub fn merge_candidate(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;
        let worktree_path = self.get_worktree_path(task_id)?;
        let changes =
            crate::infrastructure::workspace::diff_worktree(&self.project_root, &worktree_path)?;
        let touched: Vec<String> = changes
            .iter()
            .flat_map(|c| c.paths().into_iter().map(|s| s.to_string()))
            .collect();

        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );

        // (1) Every touched file must be inside this task's write scope.
        let mut out_of_scope = Vec::new();
        for path in &touched {
            if !file_in_write_scope(&normalizer, path, &state.write_allow, &state.write_deny)? {
                out_of_scope.push(path.clone());
            }
        }

        // (2) No touched file may fall into another ACTIVE task's write scope
        // (in_progress | review, non-archived) — that would be a concurrent-write
        // collision. Mirrors the gateway's cross-task overlap rule (M-c).
        let mut cross_task_conflicts = Vec::new();
        let empty_deny = std::collections::BTreeSet::new();
        for report in &self.generate_status_report()? {
            let other_id = report.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let phase = report.get("phase").and_then(|v| v.as_str()).unwrap_or("");
            let archived = report
                .get("is_archived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if other_id == task_id || archived || !matches!(phase, "in_progress" | "review") {
                continue;
            }
            let other = self.replay_task(other_id)?;
            for path in &touched {
                if file_in_write_scope(&normalizer, path, &other.write_allow, &empty_deny)? {
                    cross_task_conflicts.push(serde_json::json!({
                        "path": path,
                        "conflicting_task": other_id,
                    }));
                }
            }
        }

        // (3) The main workspace must be clean in the touched paths, else the
        // merge would clobber concurrent edits. Non-git / unverifiable → no
        // fabricated conflict (Ok(None)).
        let workspace_conflicts = if touched.is_empty() {
            Vec::new()
        } else {
            crate::infrastructure::workspace::dirty_paths_in_scope(&self.project_root, &touched)?
                .unwrap_or_default()
        };

        // High-risk changes are informational (apply still gates them).
        let requires_approval: Vec<String> =
            crate::infrastructure::workspace::detect_high_risk(&changes)
                .iter()
                .map(|(risk, path)| format!("{}: {}", risk, path))
                .collect();

        let mut blocking_reasons = Vec::new();
        if !out_of_scope.is_empty() {
            blocking_reasons.push(format!(
                "{} file(s) outside write scope",
                out_of_scope.len()
            ));
        }
        if !cross_task_conflicts.is_empty() {
            blocking_reasons.push(format!(
                "{} cross-task scope conflict(s)",
                cross_task_conflicts.len()
            ));
        }
        if !workspace_conflicts.is_empty() {
            blocking_reasons.push(format!(
                "{} file(s) dirty in the main workspace",
                workspace_conflicts.len()
            ));
        }

        Ok(serde_json::json!({
            "task_id": task_id,
            "mergeable": blocking_reasons.is_empty(),
            "touched_files": touched,
            "out_of_scope": out_of_scope,
            "cross_task_conflicts": cross_task_conflicts,
            "workspace_conflicts": workspace_conflicts,
            "requires_approval": requires_approval,
            "blocking_reasons": blocking_reasons,
        }))
    }

    // ── M4: Approval commands ──

    pub(crate) fn get_worktree_path(&self, task_id: &str) -> Result<PathBuf> {
        let worktree_path = self
            .project_root
            .join(".ctl")
            .join("tasks")
            .join(task_id)
            .join("worktree");
        if !worktree_path.exists() {
            return Err(anyhow!("Worktree not found for task '{}'", task_id));
        }
        Ok(worktree_path)
    }
}
