use crate::domain::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
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

#[allow(dead_code)]
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
        _ => return Err(format!("Unknown event type: {}", event.event_type)),
    }

    state.last_seq = event.seq;
    state.processed_commands.insert(event.command_id.clone());
    state.history.push(event.event_id.clone());
    Ok(())
}
