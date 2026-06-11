//! AgentRun aggregate for M6 multi-agent concurrency.
//!
//! Each run is an independent aggregate with its own event stream.
//! A run represents a single agent execution within a task boundary.
//!
//! Storage: `.trellis/runs/<run_id>/events.jsonl`
//!
//! Run lifecycle:
//!   Queued → Running → Completed | Failed | Aborted
//!
//! The run reducer is pure — no filesystem, network, time, or process access.

use crate::domain::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

/// Phase of an agent run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Queued,
    Running,
    Completed,
    Failed,
    Aborted,
}

impl fmt::Display for RunPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunPhase::Queued => write!(f, "Queued"),
            RunPhase::Running => write!(f, "Running"),
            RunPhase::Completed => write!(f, "Completed"),
            RunPhase::Failed => write!(f, "Failed"),
            RunPhase::Aborted => write!(f, "Aborted"),
        }
    }
}

/// Outcome of running a gate within an agent run.
/// Mirrors task-level GateResult but scoped to a run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunGateResult {
    pub gate_id: String,
    pub passed: bool,
    pub evidence: String,
    pub checked_at: String,
}

impl fmt::Display for RunGateResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.passed { "PASS" } else { "FAIL" };
        write!(f, "{}: {} ({})", self.gate_id, status, self.evidence)
    }
}

/// State of an agent run, projected from its event stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunState {
    /// Unique run identifier.
    pub run_id: String,
    /// Parent task identifier.
    pub task_id: String,
    /// Adapter name (e.g., "omp", "manual").
    pub adapter: String,
    /// Current run phase.
    pub phase: RunPhase,
    /// Path to the isolated worktree (set when run starts).
    pub worktree_path: Option<String>,
    /// Active lease ID for this run.
    pub lease_id: Option<String>,
    /// Write-allowed paths (inherited from task).
    pub write_allow: BTreeSet<String>,
    /// Write-denied paths (inherited from task).
    pub write_deny: BTreeSet<String>,
    /// Required gates (inherited from task).
    pub gates: BTreeSet<String>,
    /// Latest gate results keyed by gate_id.
    pub gate_results: HashMap<String, RunGateResult>,
    /// Files touched by this run (accumulated from evidence).
    pub touched_files: BTreeSet<String>,
    /// Ordered event history (event_ids).
    pub history: Vec<String>,
    /// Last processed sequence number.
    pub last_seq: i64,
    /// Idempotency: processed command_ids.
    pub processed_commands: HashSet<String>,
}

impl AgentRunState {
    pub fn new(run_id: &str) -> Self {
        Self {
            run_id: run_id.to_string(),
            task_id: String::new(),
            adapter: String::new(),
            phase: RunPhase::Queued,
            worktree_path: None,
            lease_id: None,
            write_allow: BTreeSet::new(),
            write_deny: BTreeSet::new(),
            gates: BTreeSet::new(),
            gate_results: HashMap::new(),
            touched_files: BTreeSet::new(),
            history: Vec::new(),
            last_seq: 0,
            processed_commands: HashSet::new(),
        }
    }
}

/// Pure reducer for AgentRun events.
/// Follows the same pattern as task::apply — no side effects.
pub fn apply_run(state: &mut AgentRunState, event: &Event) -> Result<(), String> {
    // Run ID must match
    if event.task_id != state.run_id {
        return Err(format!(
            "Run ID mismatch: event targets {}, state is {}",
            event.task_id, state.run_id
        ));
    }

    // Idempotency: skip already-processed commands
    if state.processed_commands.contains(&event.command_id) {
        return Ok(());
    }

    // Sequence must be strictly ascending
    if event.seq <= state.last_seq {
        return Err(format!(
            "Sequence error: received {}, expected > {}",
            event.seq, state.last_seq
        ));
    }

    match event.event_type.as_str() {
        "run_created" => {
            if state.last_seq > 0 {
                return Err("Cannot re-create run: already has events".into());
            }
            let task_id = event
                .payload
                .get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let adapter = event
                .payload
                .get("adapter")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if task_id.is_empty() || adapter.is_empty() {
                return Err("run_created: task_id and adapter are required".into());
            }
            state.task_id = task_id.to_string();
            state.adapter = adapter.to_string();

            // Inherit scope from payload (set by scheduler)
            if let Some(arr) = event.payload.get("write_allow").and_then(|v| v.as_array()) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        state.write_allow.insert(s.to_string());
                    }
                }
            }
            if let Some(arr) = event.payload.get("write_deny").and_then(|v| v.as_array()) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        state.write_deny.insert(s.to_string());
                    }
                }
            }
            if let Some(arr) = event.payload.get("gates").and_then(|v| v.as_array()) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        state.gates.insert(s.to_string());
                    }
                }
            }

            state.phase = RunPhase::Queued;
        }
        "run_started" => {
            if state.phase != RunPhase::Queued {
                return Err(format!(
                    "Can only start from Queued, current: {:?}",
                    state.phase
                ));
            }
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
            if worktree_path.is_empty() || lease_id.is_empty() {
                return Err("run_started: worktree_path and lease_id are required".into());
            }
            state.phase = RunPhase::Running;
            state.worktree_path = Some(worktree_path.to_string());
            state.lease_id = Some(lease_id.to_string());
        }
        "run_finished" => {
            if state.phase != RunPhase::Running {
                return Err(format!(
                    "Can only finish from Running, current: {:?}",
                    state.phase
                ));
            }
            state.phase = RunPhase::Completed;
        }
        "run_failed" => {
            // run_failed can occur from Running or Queued
            if state.phase != RunPhase::Running && state.phase != RunPhase::Queued {
                return Err(format!(
                    "run_failed only valid from Running or Queued, current: {:?}",
                    state.phase
                ));
            }
            let reason = event
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if reason.is_empty() {
                return Err("run_failed: reason is required".into());
            }
            state.phase = RunPhase::Failed;
        }
        "run_aborted" => {
            // run_aborted can occur from any non-terminal phase
            if state.phase == RunPhase::Completed
                || state.phase == RunPhase::Failed
                || state.phase == RunPhase::Aborted
            {
                return Err(format!(
                    "Cannot abort from terminal phase: {:?}",
                    state.phase
                ));
            }
            let reason = event
                .payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if reason.is_empty() {
                return Err("run_aborted: reason is required".into());
            }
            state.phase = RunPhase::Aborted;
        }
        "evidence_accepted" => {
            if state.phase == RunPhase::Completed
                || state.phase == RunPhase::Failed
                || state.phase == RunPhase::Aborted
            {
                return Err("Cannot accept evidence for terminal run".into());
            }
            let evidence_id = event
                .payload
                .get("evidence_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if evidence_id.is_empty() {
                return Err("evidence_accepted: evidence_id is required".into());
            }
            // Accumulate touched files from evidence
            if let Some(arr) = event
                .payload
                .get("touched_files")
                .and_then(|v| v.as_array())
            {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        state.touched_files.insert(s.to_string());
                    }
                }
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
        "gate_checked" => {
            let gate_id = event
                .payload
                .get("gate_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if gate_id.is_empty() {
                return Err("gate_checked: gate_id is required".into());
            }
            if !state.gates.contains(gate_id) {
                return Err(format!(
                    "gate_checked: gate '{}' is not declared in run gates",
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
                RunGateResult {
                    gate_id: gate_id.to_string(),
                    passed,
                    evidence: evidence.to_string(),
                    checked_at: checked_at.to_string(),
                },
            );
        }
        _ => return Err(format!("Unknown run event type: {}", event.event_type)),
    }

    state.last_seq = event.seq;
    state.processed_commands.insert(event.command_id.clone());
    state.history.push(event.event_id.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_event(run_id: &str, seq: i64, event_type: &str, payload: serde_json::Value) -> Event {
        Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: format!("evt-{}", seq),
            command_id: format!("cmd-{}", seq),
            task_id: run_id.to_string(),
            seq,
            occurred_at: "2026-06-11T10:00:00Z".to_string(),
            actor: "scheduler".to_string(),
            event_type: event_type.to_string(),
            payload,
        }
    }

    #[test]
    fn test_run_lifecycle() {
        let mut state = AgentRunState::new("r1");

        // run_created
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({
                "task_id": "t1",
                "adapter": "omp",
                "write_allow": ["src/foo/"],
                "write_deny": [],
                "gates": ["cargo_check"]
            }),
        );
        apply_run(&mut state, &e).unwrap();
        assert_eq!(state.phase, RunPhase::Queued);
        assert_eq!(state.task_id, "t1");
        assert_eq!(state.adapter, "omp");
        assert!(state.write_allow.contains("src/foo/"));
        assert!(state.gates.contains("cargo_check"));

        // run_started
        let e = make_event(
            "r1",
            2,
            "run_started",
            json!({
                "worktree_path": ".trellis/runs/r1/worktree",
                "lease_id": "lease-1"
            }),
        );
        apply_run(&mut state, &e).unwrap();
        assert_eq!(state.phase, RunPhase::Running);
        assert_eq!(
            state.worktree_path,
            Some(".trellis/runs/r1/worktree".to_string())
        );
        assert_eq!(state.lease_id, Some("lease-1".to_string()));

        // gate_checked
        let e = make_event(
            "r1",
            3,
            "gate_checked",
            json!({
                "gate_id": "cargo_check",
                "passed": true,
                "evidence": "cargo check succeeded",
                "checked_at": "2026-06-11T10:05:00Z"
            }),
        );
        apply_run(&mut state, &e).unwrap();
        assert!(state.gate_results.get("cargo_check").unwrap().passed);

        // evidence_accepted
        let e = make_event(
            "r1",
            4,
            "evidence_accepted",
            json!({
                "evidence_id": "ev-1",
                "touched_files": ["src/foo/mod.rs", "src/foo/bar.rs"]
            }),
        );
        apply_run(&mut state, &e).unwrap();
        assert!(state.touched_files.contains("src/foo/mod.rs"));
        assert!(state.touched_files.contains("src/foo/bar.rs"));

        // run_finished
        let e = make_event("r1", 5, "run_finished", json!({}));
        apply_run(&mut state, &e).unwrap();
        assert_eq!(state.phase, RunPhase::Completed);
    }

    #[test]
    fn test_run_id_mismatch_rejected() {
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r2",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        let err = apply_run(&mut state, &e).unwrap_err();
        assert!(err.contains("Run ID mismatch"));
    }

    #[test]
    fn test_duplicate_command_idempotent() {
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        apply_run(&mut state, &e).unwrap();
        // Same command_id → idempotent skip
        let result = apply_run(&mut state, &e);
        assert!(result.is_ok());
    }

    #[test]
    fn test_out_of_order_seq_rejected() {
        let mut state = AgentRunState::new("r1");
        // First event with seq=1 succeeds
        let e1 = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        apply_run(&mut state, &e1).unwrap();
        // Second event with seq=1 again (not strictly ascending) → rejected
        let mut e2 = make_event(
            "r1",
            1,
            "run_started",
            json!({"worktree_path": "/tmp/w", "lease_id": "l1"}),
        );
        e2.command_id = "cmd-1-dup".to_string(); // different command_id to bypass idempotency
        let err = apply_run(&mut state, &e2).unwrap_err();
        assert!(err.contains("Sequence error"));
    }

    #[test]
    fn test_phase_transition_enforcement() {
        // run_started on fresh state (phase=Queued but no task_id/adapter) succeeds
        // because the reducer only checks phase, not initialization.
        // This is by design: run_created + run_started is the normal path.
        // The real enforcement is that run_finished requires Running:
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        apply_run(&mut state, &e).unwrap();
        // Try to finish from Queued (without start)
        let e = make_event("r1", 2, "run_finished", json!({}));
        let err = apply_run(&mut state, &e).unwrap_err();
        assert!(err.contains("Can only finish from Running"));

        // Try to abort from terminal phase
        let e = make_event(
            "r1",
            2,
            "run_started",
            json!({"worktree_path": "/tmp/w", "lease_id": "l1"}),
        );
        apply_run(&mut state, &e).unwrap();
        let e = make_event("r1", 3, "run_finished", json!({}));
        apply_run(&mut state, &e).unwrap();
        let e = make_event("r1", 4, "run_aborted", json!({"reason": "too late"}));
        let err = apply_run(&mut state, &e).unwrap_err();
        assert!(err.contains("Cannot abort from terminal phase"));
    }

    #[test]
    fn test_run_failed_from_running() {
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        apply_run(&mut state, &e).unwrap();
        let e = make_event(
            "r1",
            2,
            "run_started",
            json!({"worktree_path": "/tmp/w", "lease_id": "l1"}),
        );
        apply_run(&mut state, &e).unwrap();

        let e = make_event("r1", 3, "run_failed", json!({"reason": "OOM"}));
        apply_run(&mut state, &e).unwrap();
        assert_eq!(state.phase, RunPhase::Failed);
    }

    #[test]
    fn test_run_aborted() {
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        apply_run(&mut state, &e).unwrap();

        let e = make_event("r1", 2, "run_aborted", json!({"reason": "human cancel"}));
        apply_run(&mut state, &e).unwrap();
        assert_eq!(state.phase, RunPhase::Aborted);
    }

    #[test]
    fn test_terminal_phase_rejects_events() {
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        apply_run(&mut state, &e).unwrap();
        let e = make_event(
            "r1",
            2,
            "run_started",
            json!({"worktree_path": "/tmp/w", "lease_id": "l1"}),
        );
        apply_run(&mut state, &e).unwrap();
        let e = make_event("r1", 3, "run_finished", json!({}));
        apply_run(&mut state, &e).unwrap();

        // Can't abort from completed
        let e = make_event("r1", 4, "run_aborted", json!({"reason": "too late"}));
        let err = apply_run(&mut state, &e).unwrap_err();
        assert!(err.contains("Cannot abort from terminal phase"));

        // Can't accept evidence for completed run
        let e = make_event(
            "r1",
            5,
            "evidence_accepted",
            json!({"evidence_id": "ev-late"}),
        );
        let err = apply_run(&mut state, &e).unwrap_err();
        assert!(err.contains("terminal run"));
    }

    #[test]
    fn test_gate_not_in_run_gates_rejected() {
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp", "gates": ["cargo_check"]}),
        );
        apply_run(&mut state, &e).unwrap();

        let e = make_event(
            "r1",
            2,
            "gate_checked",
            json!({
                "gate_id": "cargo_test",
                "passed": true,
                "evidence": "ok",
                "checked_at": "2026-06-11T10:00:00Z"
            }),
        );
        let err = apply_run(&mut state, &e).unwrap_err();
        assert!(err.contains("not declared in run gates"));
    }

    #[test]
    fn test_evidence_rejected_requires_id() {
        let mut state = AgentRunState::new("r1");
        let e = make_event(
            "r1",
            1,
            "run_created",
            json!({"task_id": "t1", "adapter": "omp"}),
        );
        apply_run(&mut state, &e).unwrap();

        let e = make_event("r1", 2, "evidence_rejected", json!({}));
        let err = apply_run(&mut state, &e).unwrap_err();
        assert!(err.contains("evidence_rejected: evidence_id is required"));
    }

    #[test]
    fn test_run_fixture_replay() {
        let content = std::fs::read_to_string("fixtures/run_lifecycle.jsonl").unwrap();
        let mut state = AgentRunState::new("r1");

        for line in content.lines() {
            let event: Event = serde_json::from_str(line).unwrap();
            apply_run(&mut state, &event).unwrap();
        }

        assert_eq!(state.phase, RunPhase::Completed);
        assert_eq!(state.task_id, "t-concurrent-a");
        assert_eq!(state.adapter, "omp");
        assert!(state.worktree_path.is_some());
        assert!(state.lease_id.is_some());
        assert!(state.gate_results.get("cargo_check").unwrap().passed);
        assert!(state.touched_files.contains("src/foo/mod.rs"));
        assert_eq!(state.history.len(), 5);
    }
}
