use crate::domain::approval::{ApprovalState, ApprovalStatus};
use crate::domain::event::Event;
use crate::domain::lease::{LeaseState, LeaseStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Ready,
    InProgress,
    Review,
    Completed,
    Cancelled,
}

impl Phase {
    /// Canonical machine string form (serde `snake_case`), matching the
    /// `phase` field written to `task.json` and the schema enum. This is the
    /// single source of truth for the wire/string form — do NOT derive phase
    /// strings from `format!("{:?}", ..)` (which yields `inprogress`, an
    /// incompatible spelling that silently breaks gate matching).
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Planning => "planning",
            Phase::Ready => "ready",
            Phase::InProgress => "in_progress",
            Phase::Review => "review",
            Phase::Completed => "completed",
            Phase::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Phase::Planning => write!(f, "Planning"),
            Phase::Ready => write!(f, "Ready"),
            Phase::InProgress => write!(f, "In Progress"),
            Phase::Review => write!(f, "Review"),
            Phase::Completed => write!(f, "Completed"),
            Phase::Cancelled => write!(f, "Cancelled"),
        }
    }
}
/// Outcome of running a required gate.
///
/// Frozen protocol: each gate retains only the latest result.
/// The completion interlock requires all gates to have `passed: true`
/// before `task_completed` can be emitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateResult {
    /// Identifier matching a gate in the task definition.
    pub gate_id: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Evidence description (command output summary, hash, etc.).
    pub evidence: String,
    /// ISO 8601 timestamp of when the gate was checked.
    pub checked_at: String,
}

impl fmt::Display for GateResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.passed { "PASS" } else { "FAIL" };
        write!(f, "{}: {} ({})", self.gate_id, status, self.evidence)
    }
}

/// Active run information tracked by the task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunInfo {
    pub run_id: String,
    pub adapter: String,
    pub lease_id: String,
    pub started_at_seq: i64,
}

impl fmt::Display for RunInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Run({}) adapter={} lease={}",
            self.run_id, self.adapter, self.lease_id
        )
    }
}
/// Reference to an active agent run, used by M6 multi-agent scheduling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRef {
    pub run_id: String,
    pub worktree_path: String,
    pub lease_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    pub id: String,
    pub phase: Phase,
    pub is_held: bool,
    pub is_archived: bool,
    pub objective: Option<String>,
    pub read_scope: BTreeSet<String>,
    pub write_allow: BTreeSet<String>,
    pub write_deny: BTreeSet<String>,
    pub risk_triggers: BTreeSet<String>,
    pub gates: BTreeSet<String>,
    /// Latest gate results keyed by gate_id. Each gate retains only the most recent result.
    pub gate_results: HashMap<String, GateResult>,
    /// M4: Active run (at most one per task).
    pub active_run: Option<RunInfo>,
    /// M6: Active agent runs for multi-agent concurrency.
    #[serde(default)]
    pub active_runs: Vec<RunRef>,
    /// M6: Schedule plan ID if this task is part of a planned schedule.
    #[serde(default)]
    pub schedule_plan_id: Option<String>,
    /// M4: Capability leases keyed by lease_id.
    pub leases: HashMap<String, LeaseState>,
    /// M4: Pending/approved/denied approval requests keyed by request_id.
    pub pending_approvals: HashMap<String, ApprovalState>,
    pub history: Vec<String>,
    pub last_seq: i64,
    pub processed_commands: HashSet<String>,
}

impl TaskState {
    #[allow(dead_code)]
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            phase: Phase::Planning,
            is_held: false,
            is_archived: false,
            objective: None,
            read_scope: BTreeSet::new(),
            write_allow: BTreeSet::new(),
            write_deny: BTreeSet::new(),
            risk_triggers: BTreeSet::new(),
            gates: BTreeSet::new(),
            gate_results: HashMap::new(),
            active_run: None,
            active_runs: Vec::new(),
            schedule_plan_id: None,
            leases: HashMap::new(),
            pending_approvals: HashMap::new(),
            history: Vec::new(),
            last_seq: 0,
            processed_commands: HashSet::new(),
        }
    }
}

struct TaskBoundary {
    objective: String,
    read_scope: BTreeSet<String>,
    write_allow: BTreeSet<String>,
    write_deny: BTreeSet<String>,
    risk_triggers: BTreeSet<String>,
    gates: BTreeSet<String>,
}

fn decode_task_boundary(payload: &serde_json::Value) -> Result<TaskBoundary, String> {
    if payload.get("scope").is_some() {
        return Err(
            "Legacy scope is not accepted; use read_scope/write_allow/write_deny/risk_triggers/gates"
                .into(),
        );
    }

    let objective = payload
        .get("objective")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "objective is required and must be a string".to_string())?;
    if objective.is_empty() {
        return Err("objective is required and must not be empty".into());
    }

    Ok(TaskBoundary {
        objective: objective.to_string(),
        read_scope: string_set(payload, "read_scope", true)?,
        write_allow: string_set(payload, "write_allow", true)?,
        write_deny: string_set(payload, "write_deny", false)?,
        risk_triggers: string_set(payload, "risk_triggers", false)?,
        gates: string_set(payload, "gates", true)?,
    })
}

fn string_set(
    payload: &serde_json::Value,
    field: &str,
    require_non_empty: bool,
) -> Result<BTreeSet<String>, String> {
    let values = payload
        .get(field)
        .and_then(|value| value.as_array())
        .ok_or_else(|| format!("{field} is required and must be an array"))?;
    if require_non_empty && values.is_empty() {
        return Err(format!("{field} is required and must not be empty"));
    }

    let mut normalized = BTreeSet::new();
    for value in values {
        let item = value
            .as_str()
            .ok_or_else(|| format!("{field} entries must be strings"))?;
        normalized.insert(item.to_string());
    }
    Ok(normalized)
}

pub fn apply(state: &mut TaskState, event: &Event) -> Result<(), String> {
    // R6: Check task_id BEFORE command_id idempotency (per-task, not global)
    if event.task_id != state.id {
        return Err(format!(
            "Task ID mismatch: event targets {}, state is {}",
            event.task_id, state.id
        ));
    }
    if state.processed_commands.contains(&event.command_id) {
        return Ok(());
    }
    if event.seq <= state.last_seq {
        return Err(format!(
            "Sequence error: received {}, expected > {}",
            event.seq, state.last_seq
        ));
    }
    if state.is_held
        && event.event_type != "hold_exited"
        && event.event_type != "boundary_violation_recorded"
        && event.event_type != "gate_checked"
    {
        return Err(format!("Task {} is held.", state.id));
    }

    match event.event_type.as_str() {
        "task_created" => {
            // R5: Reject duplicate task_created (first event always has last_seq == 0)
            if state.last_seq > 0 {
                return Err("Cannot re-create task: already has events".into());
            }
            let boundary = decode_task_boundary(&event.payload)?;
            state.phase = Phase::Planning;
            state.objective = Some(boundary.objective);
            state.read_scope = boundary.read_scope;
            state.write_allow = boundary.write_allow;
            state.write_deny = boundary.write_deny;
            state.risk_triggers = boundary.risk_triggers;
            state.gates = boundary.gates;
        }
        "task_revised" => {
            if state.phase != Phase::Planning {
                return Err(format!(
                    "Can only revise in Planning, current phase: {:?}",
                    state.phase
                ));
            }
            let boundary = decode_task_boundary(&event.payload)?;
            state.objective = Some(boundary.objective);
            state.read_scope = boundary.read_scope;
            state.write_allow = boundary.write_allow;
            state.write_deny = boundary.write_deny;
            state.risk_triggers = boundary.risk_triggers;
            state.gates = boundary.gates;
        }
        "task_marked_ready" => {
            if state.phase != Phase::Planning {
                return Err("Can only mark ready from Planning".into());
            }
            let missing_objective = state
                .objective
                .as_ref()
                .map(|objective| objective.is_empty())
                .unwrap_or(true);
            if missing_objective
                || state.read_scope.is_empty()
                || state.write_allow.is_empty()
                || state.gates.is_empty()
            {
                return Err(
                    "Missing objective, read_scope, write_allow, or gates for Ready".into(),
                );
            }
            state.phase = Phase::Ready;
        }
        "task_started" => {
            if state.phase != Phase::Ready {
                return Err(format!(
                    "Can only start from Ready, current phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::InProgress;
        }
        "task_submitted_for_review" => {
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "Can only submit for review from InProgress, current phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::Review;
        }
        "task_reopened" => {
            if state.phase != Phase::Review {
                return Err(format!(
                    "Can only reopen from Review, current phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::InProgress;
        }
        "task_completed" => {
            if state.phase != Phase::Review {
                return Err(format!(
                    "Can only complete from Review, current phase: {:?}",
                    state.phase
                ));
            }
            // STATE-012: Completion interlock — all required gates must have
            // a latest passing result before completion.
            for gate_id in &state.gates {
                match state.gate_results.get(gate_id) {
                    Some(result) if result.passed => {}
                    _ => {
                        return Err(format!(
                            "Completion interlock: gate '{}' has no passing result",
                            gate_id
                        ));
                    }
                }
            }
            state.phase = Phase::Completed;
        }
        "task_cancelled" => {
            if state.phase == Phase::Completed || state.phase == Phase::Cancelled {
                return Err(format!(
                    "Cannot cancel from terminal phase: {:?}",
                    state.phase
                ));
            }
            state.phase = Phase::Cancelled;
        }
        "task_archived" => {
            if state.phase != Phase::Completed && state.phase != Phase::Cancelled {
                return Err(format!(
                    "Can only archive from terminal phase, current: {:?}",
                    state.phase
                ));
            }
            state.is_archived = true;
        }
        "hold_entered" => {
            state.is_held = true;
        }
        "hold_exited" => {
            state.is_held = false;
        }
        "boundary_violation_recorded" => {
            state.is_held = true;
        }
        "gate_checked" => {
            // Record a gate execution result. Retains only the latest result per gate_id.
            // Fail-closed: reject missing or empty required fields, reject unknown gate_id.
            let gate_id = event
                .payload
                .get("gate_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if gate_id.is_empty() {
                return Err("gate_checked: gate_id is required and must not be empty".into());
            }
            if !state.gates.contains(gate_id) {
                return Err(format!(
                    "gate_checked: gate '{}' is not declared in task gates",
                    gate_id
                ));
            }
            let passed = event
                .payload
                .get("passed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let evidence = event
                .payload
                .get("evidence")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if evidence.is_empty() {
                return Err(format!(
                    "gate_checked: evidence is required for gate '{}'",
                    gate_id
                ));
            }
            let checked_at = event
                .payload
                .get("checked_at")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if checked_at.is_empty() {
                return Err(format!(
                    "gate_checked: checked_at is required for gate '{}'",
                    gate_id
                ));
            }
            state.gate_results.insert(
                gate_id.to_string(),
                GateResult {
                    gate_id: gate_id.to_string(),
                    passed,
                    evidence: evidence.to_string(),
                    checked_at: checked_at.to_string(),
                },
            );
        }
        "evidence_accepted" => {
            // Validate required fields
            let evidence_id = event
                .payload
                .get("evidence_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if evidence_id.is_empty() {
                return Err("evidence_accepted: evidence_id is required".into());
            }
            let source = event
                .payload
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if source.is_empty() {
                return Err("evidence_accepted: source is required".into());
            }
            // Evidence can be accepted in any phase except terminal states
            if state.phase == Phase::Completed || state.phase == Phase::Cancelled {
                return Err("Cannot accept evidence for terminal task".into());
            }
        }
        "evidence_rejected" => {
            let evidence_id = event
                .payload
                .get("evidence_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if evidence_id.is_empty() {
                return Err("evidence_rejected: evidence_id is required".into());
            }
        }
        // ── M4: Workspace events ──
        "workspace_created" => {
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
        "workspace_cleaned" => {
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
        "workspace_diff_computed" => {
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
        "workspace_applied" => {
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
        "run_started" => {
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
                started_at_seq: event.seq,
            });
        }
        "run_completed" => {
            if state.active_run.is_none() {
                return Err("Cannot complete run: no active run".into());
            }
            state.active_run = None;
        }
        "run_failed" => {
            // run_failed clears active_run regardless of state
            state.active_run = None;
        }
        // ── M4: Lease events ──
        "lease_created" => {
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
            if run_id.is_empty() || resource_path.is_empty() || action.is_empty() {
                return Err("lease_created: run_id, resource_path, and action are required".into());
            }
            if ttl_seconds == 0 || max_uses == 0 {
                return Err("lease_created: ttl_seconds and max_uses must be > 0".into());
            }
            state.leases.insert(
                lease_id.to_string(),
                LeaseState {
                    lease_id: lease_id.to_string(),
                    run_id: run_id.to_string(),
                    resource_path: resource_path.to_string(),
                    action: action.to_string(),
                    ttl_seconds,
                    max_uses,
                    remaining_uses: max_uses,
                    created_at_seq: event.seq,
                    status: LeaseStatus::Active,
                },
            );
        }
        "lease_used" => {
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
            if lease.status != LeaseStatus::Active {
                return Err(format!("Lease '{}' is not active", lease_id));
            }
            if lease.remaining_uses == 0 {
                return Err(format!("Lease '{}' has no remaining uses", lease_id));
            }
            lease.remaining_uses -= 1;
            if lease.remaining_uses == 0 {
                lease.status = LeaseStatus::Expired;
            }
        }
        "lease_expired" => {
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
            lease.status = LeaseStatus::Expired;
        }
        "lease_revoked" => {
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
            lease.status = LeaseStatus::Revoked;
        }
        // ── M4: Approval events ──
        "approval_requested" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_requested: request_id is required".into());
            }
            if state.pending_approvals.contains_key(request_id) {
                return Err(format!("Duplicate approval request_id: {}", request_id));
            }
            let reason = event
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let scope = event
                .payload
                .get("scope")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let ttl_seconds = event
                .payload
                .get("ttl_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if reason.is_empty() {
                return Err("approval_requested: reason is required".into());
            }
            if ttl_seconds == 0 {
                return Err("approval_requested: ttl_seconds must be > 0".into());
            }
            state.pending_approvals.insert(
                request_id.to_string(),
                ApprovalState {
                    request_id: request_id.to_string(),
                    reason: reason.to_string(),
                    scope,
                    ttl_seconds,
                    requested_at_seq: event.seq,
                    granted_at_seq: None,
                    status: ApprovalStatus::Pending,
                },
            );
        }
        "approval_granted" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_granted: request_id is required".into());
            }
            let approval = state
                .pending_approvals
                .get_mut(request_id)
                .ok_or_else(|| format!("Unknown approval request_id: {}", request_id))?;
            if approval.status != ApprovalStatus::Pending {
                return Err(format!(
                    "Approval '{}' is not pending (status: {:?})",
                    request_id, approval.status
                ));
            }
            approval.status = ApprovalStatus::Granted;
            approval.granted_at_seq = Some(event.seq);
        }
        "approval_denied" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_denied: request_id is required".into());
            }
            let approval = state
                .pending_approvals
                .get_mut(request_id)
                .ok_or_else(|| format!("Unknown approval request_id: {}", request_id))?;
            if approval.status != ApprovalStatus::Pending {
                return Err(format!(
                    "Approval '{}' is not pending (status: {:?})",
                    request_id, approval.status
                ));
            }
            approval.status = ApprovalStatus::Denied;
        }
        "approval_expired" => {
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if request_id.is_empty() {
                return Err("approval_expired: request_id is required".into());
            }
            let approval = state
                .pending_approvals
                .get_mut(request_id)
                .ok_or_else(|| format!("Unknown approval request_id: {}", request_id))?;
            approval.status = ApprovalStatus::Expired;
        }
        // ── M6: Multi-agent scheduling events ──
        "run_scheduled" => {
            // Task is assigned to a schedule plan.
            let plan_id = event
                .payload
                .get("plan_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if plan_id.is_empty() {
                return Err("run_scheduled: plan_id is required".into());
            }
            state.schedule_plan_id = Some(plan_id.to_string());
        }
        "run_launched" => {
            // An agent run has been launched for this task.
            if state.phase != Phase::InProgress {
                return Err(format!(
                    "run_launched only valid in InProgress, current: {:?}",
                    state.phase
                ));
            }
            let run_id = event
                .payload
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let worktree_path = event
                .payload
                .get("worktree_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let lease_id = event
                .payload
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if run_id.is_empty() || worktree_path.is_empty() || lease_id.is_empty() {
                return Err(
                    "run_launched: run_id, worktree_path, and lease_id are required".into(),
                );
            }
            // Check for duplicate run_id
            if state.active_runs.iter().any(|r| r.run_id == run_id) {
                return Err(format!("Duplicate run_id in active_runs: {}", run_id));
            }
            state.active_runs.push(RunRef {
                run_id: run_id.to_string(),
                worktree_path: worktree_path.to_string(),
                lease_id: lease_id.to_string(),
            });
        }
        "run_merged" => {
            // A completed run's worktree diff has been applied to main workspace.
            let run_id = event
                .payload
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if run_id.is_empty() {
                return Err("run_merged: run_id is required".into());
            }
            // Remove from active_runs
            let idx = state
                .active_runs
                .iter()
                .position(|r| r.run_id == run_id)
                .ok_or_else(|| {
                    format!("run_merged: run_id '{}' not found in active_runs", run_id)
                })?;
            state.active_runs.remove(idx);
        }
        _ => return Err(format!("Unknown event type: {}", event.event_type)),
    }

    state.last_seq = event.seq;
    state.processed_commands.insert(event.command_id.clone());
    state.history.push(event.event_id.clone());
    Ok(())
}
