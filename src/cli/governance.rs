use super::*;

/// Governance state derived from the task ledger.
#[derive(Debug)]
pub(super) enum GovState {
    /// No .ctl directory — not a governed project
    Ungoverned,
    /// Tasks exist but none active
    Idle,
    /// A task is in_progress (with optional hold)
    InProgress {
        task_id: String,
        write_allow: Vec<String>,
        is_held: bool,
        /// Actions authorized by granted approvals on this task (e.g. `deps`),
        /// read by the step-up gate. Derived from `pending_approvals`.
        approved_actions: Vec<String>,
        /// M-f `ctl apply`: out-of-scope paths a reviewer has granted for this
        /// task (granted approvals with `scope.action == "apply"`). A write
        /// landing under one of these is allowed as a reviewed exception even
        /// though it sits outside `write_allow`.
        approved_apply_paths: Vec<String>,
    },
    /// A task is in review. The commit window opens here (M-g): `git commit`
    /// and `git push` are allowed in Review as well as Completed.
    Review {
        task_id: String,
        write_allow: Vec<String>,
    },
    /// A task is completed but not yet archived (commit window)
    Completed {
        task_id: String,
        write_allow: Vec<String>,
    },
    /// More than one in_progress task declares a write scope (M-a). The gateway
    /// cannot bind a single `write_allow`, so it fails closed rather than
    /// silently governing only the first task. Resolve down to one active
    /// write task (submit/hold the others) — or bind the call to one of them
    /// with a dispatch token (M-e) — to restore write governance.
    MultipleActive { task_ids: Vec<String> },
}

/// One non-archived `in_progress` task as seen by the gateway (M-a).
pub(super) struct ActiveTask {
    pub(super) task_id: String,
    pub(super) write_allow: Vec<String>,
    pub(super) is_held: bool,
    pub(super) approved_actions: Vec<String>,
    /// M-f `ctl apply`: granted out-of-scope edit paths for this task.
    pub(super) approved_apply_paths: Vec<String>,
}

/// Resolve the set of active `in_progress` tasks into a single governing state
/// (M-a write-ambiguity + M-e dispatch binding). Pure: no IO, so it is unit
/// tested directly without `.ctl` fixtures.
///
/// `bound_task` is the dispatch binding (M-e): when it names one of the active
/// tasks, governance binds to **that** task even if other active write tasks
/// exist — the dispatching task is stated explicitly, so there is no ambiguity
/// to fail closed on. A binding that matches no active task is treated as stale
/// and ignored (a bad token must never widen scope), falling through to the
/// unbound M-a scan: ≥2 active write tasks → `MultipleActive` (fail closed),
/// otherwise bind the sole governing task (held > write task > read-only).
///
/// Returns `None` when there are no active tasks (caller continues to the
/// review/completed/idle checks).
pub(super) fn resolve_active_governance(
    active: &[ActiveTask],
    bound_task: Option<&str>,
) -> Option<GovState> {
    if active.is_empty() {
        return None;
    }

    let bind_to = |t: &ActiveTask| GovState::InProgress {
        task_id: t.task_id.clone(),
        write_allow: t.write_allow.clone(),
        is_held: t.is_held,
        approved_actions: t.approved_actions.clone(),
        approved_apply_paths: t.approved_apply_paths.clone(),
    };

    // M-e: explicit dispatch binding to a specific active task wins outright.
    if let Some(want) = bound_task {
        if let Some(t) = active.iter().find(|t| t.task_id == want) {
            return Some(bind_to(t));
        }
        // else: stale/bogus token — ignore and fall through to the M-a scan.
    }

    // M-a: tasks with a non-empty write scope compete for write governance.
    // Read-only in_progress tasks (empty write_allow) never write, so they do
    // not create write ambiguity. Two or more write tasks → fail closed.
    let write_task_ids: Vec<String> = active
        .iter()
        .filter(|t| !t.write_allow.is_empty())
        .map(|t| t.task_id.clone())
        .collect();
    if write_task_ids.len() >= 2 {
        return Some(GovState::MultipleActive {
            task_ids: write_task_ids,
        });
    }

    // Bind to a single task. Prefer a held one (held fails closed across every
    // tool), then the sole write task, then the first active task (all
    // read-only). Priority: Held > the write task > read-only.
    let chosen = active
        .iter()
        .find(|t| t.is_held)
        .or_else(|| active.iter().find(|t| !t.write_allow.is_empty()))
        .unwrap_or(&active[0]);
    Some(bind_to(chosen))
}

pub(super) fn compute_gov_state(project_root: &Path, bound_task: Option<&str>) -> Result<GovState> {
    let tasks_dir = project_root.join(".ctl").join("tasks");
    if !tasks_dir.exists() {
        return Ok(GovState::Ungoverned);
    }

    let app = ControlApp::open(project_root, false)?;
    let reports = app.generate_status_report()?;

    // ── Active in_progress tasks (M-a) ──────────────────────────────────
    // Collect EVERY non-archived in_progress task, not just the first one.
    // Pre-M-a this loop early-returned on the first in_progress task, so when
    // several were active simultaneously (e.g. concurrent sub-agent reviews)
    // the rest were silently ungoverned at the gateway. The collected set is
    // resolved to a single governing state by `resolve_active_governance`
    // (M-a fail-closed + M-e dispatch binding).
    let mut active: Vec<ActiveTask> = Vec::new();
    for report in &reports {
        let phase = report.get("phase").and_then(|v| v.as_str()).unwrap_or("");
        let is_archived = report
            .get("is_archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_archived || phase != "in_progress" {
            continue;
        }
        let task_id = report
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let is_held = report
            .get("is_held")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let state = app.replay_task(&task_id)?;
        // Collect actions authorized by granted approvals on this task.
        let approved_actions: Vec<String> = state
            .pending_approvals
            .values()
            .filter(|a| a.is_granted())
            .filter_map(|a| {
                a.scope
                    .get("action")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();
        // M-f `ctl apply`: out-of-scope paths granted via approvals with
        // scope.action == "apply".
        let approved_apply_paths: Vec<String> = state
            .pending_approvals
            .values()
            .filter(|a| a.is_granted())
            .filter(|a| a.scope.get("action").and_then(|v| v.as_str()) == Some("apply"))
            .filter_map(|a| {
                a.scope
                    .get("path")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from)
            })
            .collect();
        active.push(ActiveTask {
            task_id,
            write_allow: state.write_allow.iter().cloned().collect(),
            is_held,
            approved_actions,
            approved_apply_paths,
        });
    }

    if let Some(state) = resolve_active_governance(&active, bound_task) {
        return Ok(state);
    }

    // Check for completed (not archived) — commit window
    for report in &reports {
        let phase = report.get("phase").and_then(|v| v.as_str()).unwrap_or("");
        let is_archived = report
            .get("is_archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let task_id = report
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !is_archived && phase == "completed" {
            let state = app.replay_task(&task_id)?;
            return Ok(GovState::Completed {
                task_id,
                write_allow: state.write_allow.iter().cloned().collect(),
            });
        }
    }

    // Check for review
    for report in &reports {
        let phase = report.get("phase").and_then(|v| v.as_str()).unwrap_or("");
        let is_archived = report
            .get("is_archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !is_archived && phase == "review" {
            let task_id = report
                .get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let state = app.replay_task(&task_id)?;
            return Ok(GovState::Review {
                task_id,
                write_allow: state.write_allow.iter().cloned().collect(),
            });
        }
    }

    // Any non-archived tasks at all?
    let has_active = reports.iter().any(|r| {
        !r.get("is_archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    });
    if has_active {
        Ok(GovState::Idle)
    } else {
        Ok(GovState::Ungoverned)
    }
}

/// Short string for GovState variant (no Debug payload).
pub(super) fn gov_state_str(state: &GovState) -> &'static str {
    match state {
        GovState::Ungoverned => "ungoverned",
        GovState::Idle => "idle",
        GovState::InProgress { .. } => "in_progress",
        GovState::Review { .. } => "review",
        GovState::Completed { .. } => "completed",
        GovState::MultipleActive { .. } => "multiple_active",
    }
}
