pub mod schedule;
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::omp::OmpAdapter;
use crate::adapters::ExecutorAdapter;
use crate::domain::event::Event;
use crate::domain::lease::LeaseStatus;
use crate::domain::task::{apply, Phase, TaskState};
use crate::infrastructure::schema_validator::SchemaValidator;
use crate::infrastructure::store::FileEventStore;

/// Evidence `source` that marks a reviewer's dedicated completion audit (M-f),
/// distinct from implementer/adapter output evidence. The finish interlock
/// requires a fresh PASS with this source; using a distinguished source keeps
/// the canonical event schema unchanged.
pub const COMPLETION_AUDIT_SOURCE: &str = "completion_audit";

pub struct ControlApp {
    pub project_root: PathBuf,
    store: FileEventStore,
    validator: Option<SchemaValidator>,
    dry_run: bool,
    /// Identity stamped on every event this instance appends (M6). Defaults to
    /// `"human"`; set from the `CTL_ACTOR` env var so a reviewer sub-agent and
    /// the implementer act under distinct identities. Read by the reviewer ≠
    /// implementer interlock.
    actor: String,
}

/// Resolve the acting identity from the environment (M6). Blank/unset → the
/// unattributed default `"human"`.
fn actor_from_env() -> String {
    std::env::var("CTL_ACTOR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "human".to_string())
}

pub struct CreateTaskInput<'a> {
    pub objective: &'a str,
    pub read_scope: &'a [String],
    pub write_allow: &'a [String],
    pub write_deny: &'a [String],
    pub risk_triggers: &'a [String],
    pub gates: &'a [String],
    /// M-d: task IDs that must complete before this one runs.
    pub depends_on: &'a [String],
}

pub struct ReviseTaskInput<'a> {
    pub objective: Option<&'a str>,
    pub read_scope: Option<&'a [String]>,
    pub write_allow: Option<&'a [String]>,
    pub write_deny: Option<&'a [String]>,
    pub risk_triggers: Option<&'a [String]>,
    pub gates: Option<&'a [String]>,
    pub depends_on: Option<&'a [String]>,
}

impl ControlApp {
    pub fn init(project_root: &Path) -> Result<Self> {
        let store = FileEventStore::init(project_root)?;
        let validator = new_validator_if_available();
        Ok(Self {
            project_root: project_root.to_path_buf(),
            store,
            validator,
            dry_run: false,
            actor: actor_from_env(),
        })
    }

    pub fn open(project_root: &Path, dry_run: bool) -> Result<Self> {
        let store = FileEventStore::open(project_root)?;
        let validator = new_validator_if_available();
        Ok(Self {
            project_root: project_root.to_path_buf(),
            store,
            validator,
            dry_run,
            actor: actor_from_env(),
        })
    }

    /// Override the acting identity (M6). Used where the actor is known
    /// explicitly rather than via `CTL_ACTOR` — e.g. tests separating an
    /// implementer from a reviewer.
    pub fn with_actor(mut self, actor: &str) -> Self {
        self.actor = actor.to_string();
        self
    }

    // ── Commands ──

    pub fn create_task(&self, id: &str, input: CreateTaskInput<'_>) -> Result<Event> {
        let existing = self.store.read_for_task(id)?;
        if !existing.is_empty() {
            return Err(anyhow!("Task '{}' already exists", id));
        }

        let read_scope = self.normalize_boundary_paths("read_scope", input.read_scope, false)?;
        let write_allow = self.normalize_boundary_paths("write_allow", input.write_allow, true)?;
        let write_deny = self.normalize_boundary_paths("write_deny", input.write_deny, true)?;
        let gates = validate_gate_templates(input.gates)?;
        validate_task_definition(input.objective, &read_scope, &write_allow, &gates)?;

        let mut payload = serde_json::json!({
            "objective": input.objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": input.risk_triggers,
            "gates": gates,
        });
        // M-d: only emit depends_on when non-empty, keeping payloads minimal and
        // dependency-free events byte-identical to pre-M-d output.
        if !input.depends_on.is_empty() {
            payload["depends_on"] = serde_json::json!(input.depends_on);
        }
        let event = self.build_event(id, "task_created", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(id)?;
        }
        Ok(event)
    }

    pub fn revise_task(&self, task_id: &str, input: ReviseTaskInput<'_>) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::Planning {
            return Err(anyhow!(
                "Can only revise in Planning phase, current: {:?}",
                state.phase
            ));
        }

        let objective = input
            .objective
            .map(String::from)
            .or_else(|| state.objective.clone())
            .unwrap_or_default();
        let read_scope = match input.read_scope {
            Some(paths) => self.normalize_boundary_paths("read_scope", paths, false)?,
            None => state.read_scope.iter().cloned().collect(),
        };
        let write_allow = match input.write_allow {
            Some(paths) => self.normalize_boundary_paths("write_allow", paths, true)?,
            None => state.write_allow.iter().cloned().collect(),
        };
        let write_deny = match input.write_deny {
            Some(paths) => self.normalize_boundary_paths("write_deny", paths, true)?,
            None => state.write_deny.iter().cloned().collect(),
        };
        let risk_triggers = input
            .risk_triggers
            .map(|triggers| triggers.to_vec())
            .unwrap_or_else(|| state.risk_triggers.iter().cloned().collect());
        let gates = match input.gates {
            Some(gates) => validate_gate_templates(gates)?,
            None => state.gates.iter().cloned().collect(),
        };
        let depends_on: Vec<String> = match input.depends_on {
            Some(deps) => deps.to_vec(),
            None => state.depends_on.iter().cloned().collect(),
        };
        validate_task_definition(&objective, &read_scope, &write_allow, &gates)?;

        let mut payload = serde_json::json!({
            "objective": objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": risk_triggers,
            "gates": gates,
        });
        if !depends_on.is_empty() {
            payload["depends_on"] = serde_json::json!(depends_on);
        }
        let event = self.build_event(task_id, "task_revised", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn mark_ready(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_marked_ready", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn start_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_started", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_cancelled", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    // ── Post-M0 lifecycle helpers (not exposed by the M0 CLI) ──

    pub fn submit_task(&self, task_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.is_held {
            return Err(anyhow!("Cannot submit: task is held"));
        }
        if state.phase != Phase::InProgress {
            return Err(anyhow!(
                "Can only submit for review from InProgress, current: {:?}",
                state.phase
            ));
        }
        // Check for any boundary violations recorded since start
        let events = self.store.read_for_task(task_id)?;
        let has_violations = events
            .iter()
            .any(|e| e.event_type == "boundary_violation_recorded");
        if has_violations {
            return Err(anyhow!("Cannot submit: task has boundary violations"));
        }
        let event =
            self.build_event(task_id, "task_submitted_for_review", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn reopen_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_reopened", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Completion interlock: phase must be Review, not held, all gates passing,
    /// and no rejected evidence.
    pub fn finish_task(&self, task_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;

        // Phase check
        if state.phase != Phase::Review {
            return Err(anyhow!(
                "Can only finish from Review, current: {:?}",
                state.phase
            ));
        }

        // Hold check
        if state.is_held {
            return Err(anyhow!("Cannot finish: task is held"));
        }

        // Gate interlock: all required gates must have latest passing result
        let mut failing_gates = Vec::new();
        for gate_id in &state.gates {
            match state.gate_results.get(gate_id) {
                Some(result) if result.passed => {}
                _ => {
                    failing_gates.push(gate_id.as_str());
                }
            }
        }
        if !failing_gates.is_empty() {
            return Err(anyhow!(
                "Completion interlock: gates not passing: {:?}",
                failing_gates
            ));
        }

        // Check for rejected evidence that hasn't been superseded by accepted evidence.
        // A rejection for a file is resolved if a later evidence_accepted covers it.
        let events = self.store.read_for_task(task_id)?;
        let mut rejected_files: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for e in &events {
            match e.event_type.as_str() {
                "evidence_rejected" => {
                    if let Some(f) = e.payload.get("touched_file").and_then(|v| v.as_str()) {
                        if !f.is_empty() {
                            rejected_files.insert(f.to_string());
                        }
                    }
                }
                "evidence_accepted" => {
                    if let Some(files) = e.payload.get("touched_files").and_then(|v| v.as_array()) {
                        for f in files {
                            if let Some(s) = f.as_str() {
                                rejected_files.remove(s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !rejected_files.is_empty() {
            return Err(anyhow!(
                "Completion interlock: rejected evidence unresolved for: {:?}",
                rejected_files
            ));
        }

        // Commit interlock (M-g): a task cannot complete with uncommitted work
        // in its write scope. The commit window opens at Review, so by the time
        // finish runs the agent must already have committed. Scoped to the
        // task's write_allow and git-tracked paths (`.ctl/` is gitignored and
        // thus excluded). Skipped for read-only tasks (empty write_allow) and
        // outside a git repository, where there is nothing to commit / no way
        // to verify.
        let scope: Vec<String> = state.write_allow.iter().cloned().collect();
        if !scope.is_empty() {
            if let Some(dirty) =
                crate::infrastructure::workspace::dirty_paths_in_scope(&self.project_root, &scope)?
            {
                if !dirty.is_empty() {
                    return Err(anyhow!(
                        "Completion interlock: uncommitted changes in write scope: {:?}. \
                         Commit (and optionally push) within Review before finishing.",
                        dirty
                    ));
                }
            }
        }

        // Hard review gate (M-f): completion requires a FRESH passing completion
        // audit. A verdict counts only if recorded after the last submit — rework
        // re-submits and invalidates a prior round's audit. The latest such
        // verdict must be a PASS; a FAIL (or no audit at all) blocks finish. This
        // upgrades review from convention (soft-layer ctl-review subagents) to a
        // gateway interlock. `events` is already loaded above.
        let last_submit_seq = events
            .iter()
            .filter(|e| e.event_type == "task_submitted_for_review")
            .map(|e| e.seq)
            .max();
        let latest_audit = events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type.as_str(),
                    "evidence_accepted" | "evidence_rejected"
                )
            })
            .filter(|e| {
                e.payload.get("source").and_then(|v| v.as_str())
                    == Some(crate::application::COMPLETION_AUDIT_SOURCE)
            })
            .filter(|e| last_submit_seq.is_none_or(|s| e.seq > s))
            .max_by_key(|e| e.seq);
        match latest_audit {
            Some(e) if e.event_type == "evidence_accepted" => {}
            Some(_) => {
                return Err(anyhow!(
                    "Completion interlock: the latest completion audit is a FAIL. \
                     Rework, then record a passing audit (ctl review accept --id {}) before finishing.",
                    task_id
                ));
            }
            None => {
                return Err(anyhow!(
                    "Completion interlock: no passing completion audit since submit. \
                     A reviewer must record one: ctl review accept --id {}",
                    task_id
                ));
            }
        }

        let event = self.build_event(task_id, "task_completed", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn archive_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_archived", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// The actors who performed implementation work on a task (M6). The reviewer
    /// who records a passing completion audit must not be one of them. Implementer
    /// signals: who `task_started` the task, and who produced non-audit work
    /// evidence (`evidence_accepted` with a `source` other than the completion
    /// audit, i.e. adapter/manual output).
    fn implementer_actors(events: &[Event]) -> HashSet<String> {
        let mut actors = HashSet::new();
        for e in events {
            match e.event_type.as_str() {
                "task_started" => {
                    actors.insert(e.actor.clone());
                }
                "evidence_accepted" => {
                    let source = e.payload.get("source").and_then(|v| v.as_str());
                    if source != Some(COMPLETION_AUDIT_SOURCE) {
                        actors.insert(e.actor.clone());
                    }
                }
                _ => {}
            }
        }
        actors
    }

    /// M-f: record a reviewer's completion-audit verdict on a submitted task.
    ///
    /// A PASS is the hard prerequisite the finish interlock requires; a FAIL
    /// blocks completion until the work is reworked and re-audited. Modeled on
    /// the existing evidence events with a distinguished `source`
    /// ([`COMPLETION_AUDIT_SOURCE`]) so it needs no canonical-schema change; the
    /// reviewer identity is the event `actor` (M6 — set via `CTL_ACTOR`).
    /// Recorded only in Review — the post-submit audit window.
    ///
    /// M6 reviewer-lease binding: a PASS may **not** be recorded by an
    /// implementer of the task (no self-approval). A FAIL is always allowed —
    /// an implementer self-flagging a problem is healthy; only self-certifying
    /// completion is the threat.
    pub fn record_completion_audit(
        &self,
        task_id: &str,
        pass: bool,
        note: Option<&str>,
    ) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::Review {
            return Err(anyhow!(
                "Completion audit can only be recorded in Review (task is {:?}); submit the task first",
                state.phase
            ));
        }
        let events = self.store.read_for_task(task_id)?;
        if pass && Self::implementer_actors(&events).contains(&self.actor) {
            return Err(anyhow!(
                "Reviewer-lease binding: actor '{}' implemented this task and cannot record its own \
                 passing completion audit. A different reviewer must accept it (set CTL_ACTOR to the \
                 reviewer's identity).",
                self.actor
            ));
        }
        let evidence_id = generate_uuid();
        let event = if pass {
            let touched: Vec<String> = state.write_allow.iter().cloned().collect();
            let payload = serde_json::json!({
                "evidence_id": evidence_id,
                "source": COMPLETION_AUDIT_SOURCE,
                "touched_files": touched,
                "result_file": note.unwrap_or(""),
                "accepted_at": now_iso8601(),
            });
            self.build_event(task_id, "evidence_accepted", payload)?
        } else {
            let payload = serde_json::json!({
                "evidence_id": evidence_id,
                "source": COMPLETION_AUDIT_SOURCE,
                "rejection_reason": note.unwrap_or("completion audit failed"),
                // Empty: the generic rejected-evidence interlock keys on a
                // per-file rejection; the completion-audit verdict is task-level
                // and enforced by the dedicated M-f interlock instead.
                "touched_file": "",
            });
            self.build_event(task_id, "evidence_rejected", payload)?
        };
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn record_gate(
        &self,
        task_id: &str,
        gate_id: &str,
        passed: bool,
        evidence: &str,
    ) -> Result<Event> {
        let payload = serde_json::json!({
            "gate_id": gate_id,
            "passed": passed,
            "evidence": evidence,
            "checked_at": now_iso8601(),
        });
        let event = self.build_event(task_id, "gate_checked", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Execute a gate through the EXEC-002 runner and record the result
    /// as a canonical `gate_checked` event.
    pub fn run_gate_checked(&self, task_id: &str, gate_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if !state.gates.contains(gate_id) {
            return Err(anyhow!(
                "Gate '{}' is not declared in task gates: {:?}",
                gate_id,
                state.gates
            ));
        }

        let result = crate::infrastructure::gates::run_gate(gate_id, &self.project_root)?;
        let evidence = if result.passed {
            format!("exit={} stdout={}B", result.exit_code, result.stdout.len())
        } else {
            // Include stderr for failed gates (truncated for evidence field)
            let stderr_preview = if result.stderr.len() > 512 {
                format!("{}...", &result.stderr[..512])
            } else {
                result.stderr.clone()
            };
            format!("exit={} stderr={}", result.exit_code, stderr_preview)
        };

        self.record_gate(task_id, gate_id, result.passed, &evidence)
    }

    /// Build a context snapshot: hash all files within the task read scope.
    pub fn build_context(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;
        let root = &self.project_root;
        let mut file_hashes = Vec::new();

        for scope_path in &state.read_scope {
            let full_path = root.join(scope_path);
            if full_path.is_dir() {
                collect_file_hashes(&full_path, root, &mut file_hashes)?;
            } else if full_path.is_file() {
                let hash = hash_file(&full_path)?;
                let rel = full_path.strip_prefix(root).unwrap_or(&full_path);
                file_hashes.push(serde_json::json!({
                    "path": path_to_payload_string(rel),
                    "hash": hash,
                }));
            }
        }

        let context = serde_json::json!({
            "task_id": task_id,
            "read_scope": state.read_scope,
            "file_count": file_hashes.len(),
            "files": file_hashes,
            "built_at": now_iso8601(),
        });

        let task_dir = self.store.task_dir(task_id)?;
        let context_path = task_dir.join("context.json");
        if !self.dry_run {
            let temp_path = task_dir.join("context.json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&context)?)?;
            std::fs::rename(&temp_path, &context_path)?;
        }

        Ok(context)
    }

    /// Export a structured assignment JSON for external execution (M3).
    /// Reads task state and optional context.json, writes assignment.json atomically.
    pub fn export_assignment(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;

        let objective = state.objective.clone().unwrap_or_default();
        let read_scope: Vec<&String> = state.read_scope.iter().collect();
        let write_allow: Vec<&String> = state.write_allow.iter().collect();
        let write_deny: Vec<&String> = state.write_deny.iter().collect();
        let risk_triggers: Vec<&String> = state.risk_triggers.iter().collect();
        let gates: Vec<&String> = state.gates.iter().collect();

        // Read context.json if available
        let task_dir = self.store.task_dir(task_id)?;
        let context_path = task_dir.join("context.json");
        let context_snapshot: serde_json::Value = if context_path.exists() {
            let raw = std::fs::read_to_string(&context_path)?;
            serde_json::from_str(&raw)?
        } else {
            serde_json::Value::Null
        };

        let assignment = serde_json::json!({
            "schema": "control.assignment.v1",
            "assignment_id": generate_uuid(),
            "task_id": task_id,
            "adapter": "manual",
            "contract": {
                "type": "manual",
                "input": "assignment.json",
                "output": "agent-output.json",
            },
            "objective": objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": risk_triggers,
            "gates": gates,
            "context_hashes": context_snapshot,
            "required_capabilities": ["file_read", "file_write"],
            "acceptance": {
                "all_gates_must_pass": true,
                "scope_enforcement": true,
            },
            "exported_at": now_iso8601(),
        });

        // Atomic write: temp + rename
        let assignment_path = task_dir.join("assignment.json");
        if assignment_path.exists() && !self.dry_run {
            eprintln!(
                "Warning: Overwriting existing assignment.json for task '{}'",
                task_id
            );
        }
        if !self.dry_run {
            let temp_path = task_dir.join("assignment.json.tmp");
            let json_str = serde_json::to_string_pretty(&assignment)?;
            std::fs::write(&temp_path, &json_str)?;
            std::fs::rename(&temp_path, &assignment_path)?;
        }

        Ok(assignment)
    }

    /// Check workspace modifications against task scope.
    /// Returns list of violations (files modified outside write_allow scope).
    pub fn boundary_check(&self, task_id: &str) -> Result<Vec<String>> {
        let state = self.replay_task(task_id)?;
        let root = &self.project_root;
        let mut violations = Vec::new();

        // Collect all files currently in write scope.
        let mut scope_files: std::collections::HashSet<String> = std::collections::HashSet::new();
        for scope_path in &state.write_allow {
            let full_path = root.join(scope_path);
            if full_path.is_dir() {
                collect_files_recursive(&full_path, root, &mut scope_files)?;
            } else if full_path.is_file() {
                let rel = full_path.strip_prefix(root).unwrap_or(&full_path);
                scope_files.insert(rel.to_string_lossy().to_string());
            }
        }

        // Compare against context snapshot if available
        let context_path = self.store.task_dir(task_id)?.join("context.json");
        if context_path.exists() {
            let context: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&context_path)?)?;
            if let Some(files) = context.get("files").and_then(|f| f.as_array()) {
                let mut baseline_map: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                for entry in files {
                    let path = entry.get("path").and_then(|p| p.as_str()).unwrap_or("");
                    let hash = entry.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                    baseline_map.insert(path.to_string(), hash.to_string());
                }

                // Check each current file against baseline
                for file_path in &scope_files {
                    let full_path = root.join(file_path);
                    if full_path.exists() {
                        let current_hash = hash_file(&full_path)?;
                        if let Some(baseline_hash) = baseline_map.get(file_path) {
                            if &current_hash != baseline_hash {
                                // File was modified — check if it's within write scope
                                violations.push(format!("MODIFIED: {}", file_path));
                            }
                        }
                    }
                }

                // Check for deleted files
                for path in baseline_map.keys() {
                    if !scope_files.contains(path) {
                        let full = root.join(path);
                        if !full.exists() {
                            violations.push(format!("DELETED: {}", path));
                        }
                    }
                }
            }
        } else {
            violations
                .push("No context snapshot found. Run 'control context build' first.".to_string());
        }

        Ok(violations)
    }

    /// Run boundary check and record any violations as canonical events.
    /// Returns the list of violation descriptions.
    /// Per STATE-004 / PATH-004: violations generate `boundary_violation_recorded`
    /// events and the task enters hold.
    pub fn boundary_check_and_record(&self, task_id: &str) -> Result<Vec<String>> {
        let violations = self.boundary_check(task_id)?;
        for violation in &violations {
            let payload = serde_json::json!({
                "violation": violation,
                "detected_at": now_iso8601(),
            });
            let event = self.build_event(task_id, "boundary_violation_recorded", payload)?;
            self.validate_and_append(&event)?;
        }
        if !violations.is_empty() && !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(violations)
    }

    /// Rebuild all task views from events (reconcile).
    pub fn reconcile(&self) -> Result<Vec<String>> {
        let task_ids = self.store.task_ids()?;
        let mut rebuilt = Vec::new();
        for task_id in &task_ids {
            let state = self.replay_task(task_id)?;
            self.store.write_task_view(task_id, &state)?;
            rebuilt.push(task_id.clone());
        }
        // M-b: reconcile also projects the cross-task control view.
        self.project_control()?;
        Ok(rebuilt)
    }

    /// Per-task review verdict derived from the soft-layer verdict→evidence
    /// events (M-b). `evidence_rejected` = reviewer found problems for a file;
    /// a later `evidence_accepted` covering that file resolves it. Mirrors the
    /// finish interlock, so the board reflects what actually blocks completion.
    fn review_status_from_events(events: &[Event]) -> &'static str {
        let mut rejected: HashSet<String> = HashSet::new();
        let mut any_accepted = false;
        for e in events {
            match e.event_type.as_str() {
                "evidence_rejected" => {
                    if let Some(f) = e.payload.get("touched_file").and_then(|v| v.as_str()) {
                        if !f.is_empty() {
                            rejected.insert(f.to_string());
                        }
                    }
                }
                "evidence_accepted" => {
                    any_accepted = true;
                    if let Some(files) = e.payload.get("touched_files").and_then(|v| v.as_array()) {
                        for f in files {
                            if let Some(s) = f.as_str() {
                                rejected.remove(s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !rejected.is_empty() {
            "needs_work"
        } else if any_accepted {
            "passed"
        } else {
            "none"
        }
    }

    /// Build the cross-task control view (M-b): one row per task plus aggregate
    /// totals. A deterministic projection over the event ledger — no wall-clock
    /// field, so repeated reconciles stay byte-identical like `task.json`.
    pub fn generate_board(&self) -> Result<serde_json::Value> {
        let task_ids = self.store.task_ids()?;
        let mut rows = Vec::with_capacity(task_ids.len());
        let (mut active, mut held, mut needs_work, mut completed, mut archived) = (0, 0, 0, 0, 0);

        for task_id in &task_ids {
            let events = self.store.read_for_task(task_id)?;
            let mut state = TaskState::new(task_id);
            for event in &events {
                apply(&mut state, event)
                    .map_err(|e| anyhow!("Reducer error at seq {}: {}", event.seq, e))?;
            }

            // "active" aligns with the gateway/review focus set (M-a): a task in
            // a live working phase that has not been archived.
            let is_active =
                !state.is_archived && matches!(state.phase, Phase::InProgress | Phase::Review);
            let review = Self::review_status_from_events(&events);
            let gates_total = state.gates.len();
            let gates_passing = state
                .gates
                .iter()
                .filter(|g| {
                    state
                        .gate_results
                        .get(g.as_str())
                        .map(|r| r.passed)
                        .unwrap_or(false)
                })
                .count();

            if is_active {
                active += 1;
            }
            if state.is_held {
                held += 1;
            }
            if review == "needs_work" {
                needs_work += 1;
            }
            if state.phase == Phase::Completed {
                completed += 1;
            }
            if state.is_archived {
                archived += 1;
            }

            // M5: deterministic drift projection. Signals derive from events +
            // the telemetry evidence index; the rule engine is pure, so this
            // stays wall-clock-free and reconcile remains byte-identical.
            let telemetry = self.store.read_telemetry_for_task(task_id)?;
            let signals = drift_signals_from(&events, &state, &telemetry);
            let report = crate::domain::drift::evaluate(task_id, &signals);
            let action = crate::domain::drift::next_action(&report, state.phase.clone());

            rows.push(serde_json::json!({
                "task_id": task_id,
                "objective": state.objective,
                "phase": state.phase.as_str(),
                "held": state.is_held,
                "active": is_active,
                "archived": state.is_archived,
                "gates_passing": gates_passing,
                "gates_total": gates_total,
                "review": review,
                "write_scope": state.write_allow.iter().collect::<Vec<_>>(),
                "depends_on": state.depends_on.iter().collect::<Vec<_>>(),
                "drift_level": report.level.as_str(),
                "drift_score": report.score,
                "drift_rules": report.fired_ids(),
                "recommended_action": action.action.as_str(),
            }));
        }

        Ok(serde_json::json!({
            "version": 1,
            "totals": {
                "tasks": rows.len(),
                "active": active,
                "held": held,
                "needs_work": needs_work,
                "completed": completed,
                "archived": archived,
            },
            "tasks": rows,
        }))
    }

    /// Write the control view to `.ctl/control.json` (reconcile projection,
    /// M-b / M5+). Atomic temp-file replace, mirroring the task.json projections.
    pub fn project_control(&self) -> Result<PathBuf> {
        let board = self.generate_board()?;
        let ctl_dir = self.project_root.join(".ctl");
        std::fs::create_dir_all(&ctl_dir)?;
        let path = ctl_dir.join("control.json");
        let tmp = ctl_dir.join("control.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&board)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    // ── M5: telemetry + drift + next-action (explainable control loop) ──

    /// Append one telemetry evidence record to the evidence index (M5). This is
    /// the only M5 write op; drift/next-action are read-only projections. The
    /// `recorded_at` provenance timestamp is stamped here (the domain stays
    /// time-free). Unknown `kind`s are accepted as evidence but the drift engine
    /// fails closed on them.
    pub fn telemetry_add(
        &self,
        task_id: &str,
        kind: &str,
        value: i64,
        source: &str,
    ) -> Result<crate::domain::telemetry::TelemetryEntry> {
        // The task must exist so telemetry is always attributable.
        if self.store.read_for_task(task_id)?.is_empty() {
            return Err(anyhow!("Task '{}' does not exist", task_id));
        }
        let entry = crate::domain::telemetry::TelemetryEntry::new(
            task_id,
            kind,
            value,
            &now_iso8601(),
            source,
        );
        if self.dry_run {
            println!(
                "[dry-run] Would append telemetry: task={}, kind={}, value={}",
                task_id, kind, value
            );
            return Ok(entry);
        }
        self.store.append_telemetry(&entry)?;
        Ok(entry)
    }

    /// Derive the drift signals for a task from the event ledger and the
    /// telemetry evidence index. Pure projection — emits no events.
    fn collect_drift_signals(
        &self,
        task_id: &str,
    ) -> Result<(crate::domain::drift::DriftSignals, Phase)> {
        let events = self.store.read_for_task(task_id)?;
        if events.is_empty() {
            return Err(anyhow!("Task '{}' does not exist", task_id));
        }
        let mut state = TaskState::new(task_id);
        for event in &events {
            apply(&mut state, event)
                .map_err(|e| anyhow!("Reducer error at seq {}: {}", event.seq, e))?;
        }
        let telemetry = self.store.read_telemetry_for_task(task_id)?;
        let signals = drift_signals_from(&events, &state, &telemetry);
        Ok((signals, state.phase))
    }

    /// Compute the drift report for a task (M5). Read-only.
    pub fn compute_drift(&self, task_id: &str) -> Result<crate::domain::drift::DriftReport> {
        let (signals, _phase) = self.collect_drift_signals(task_id)?;
        Ok(crate::domain::drift::evaluate(task_id, &signals))
    }

    /// Recommend the next action for a task (M5). Read-only and advisory — it
    /// emits no events and, for replan/rescope, only returns a structured
    /// proposal for a human to act on.
    pub fn next_action(&self, task_id: &str) -> Result<crate::domain::drift::NextActionProposal> {
        let (signals, phase) = self.collect_drift_signals(task_id)?;
        let report = crate::domain::drift::evaluate(task_id, &signals);
        Ok(crate::domain::drift::next_action(&report, phase))
    }

    // ── Queries ──

    pub fn get_status(&self, task_id: &str) -> Result<TaskState> {
        self.replay_task(task_id)
    }

    pub fn replay(&self, task_id: &str) -> Result<TaskState> {
        let state = self.replay_task(task_id)?;
        self.store.write_task_view(task_id, &state)?;
        Ok(state)
    }

    pub fn validate_store(&self) -> Result<Vec<String>> {
        let events = self.store.read_all()?;
        let mut issues = Vec::new();
        let mut seen_command_ids: HashSet<String> = HashSet::new();
        let mut task_seqs: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();

        for (i, event) in events.iter().enumerate() {
            let line = i + 1;

            // Schema field
            if event.schema != "control.event-envelope.v1" {
                issues.push(format!("Line {}: invalid schema '{}'", line, event.schema));
            }

            // Seq ordering per task
            let prev_seq = task_seqs.get(&event.task_id).copied().unwrap_or(0);
            if event.seq <= prev_seq {
                issues.push(format!(
                    "Line {}: seq {} not strictly increasing for task {} (prev {})",
                    line, event.seq, event.task_id, prev_seq
                ));
            }
            task_seqs.insert(event.task_id.clone(), event.seq);

            // Command id uniqueness
            if !seen_command_ids.insert(event.command_id.clone()) {
                issues.push(format!(
                    "Line {}: duplicate command_id '{}'",
                    line, event.command_id
                ));
            }

            // Schema validation (when schemas/ available)
            if let Some(ref validator) = self.validator {
                let json_val = serde_json::to_value(event)
                    .map_err(|e| anyhow!("Line {}: serialization error: {}", line, e))?;
                if let Err(e) = validator.validate_instance(&json_val, &event.schema) {
                    issues.push(format!("Line {}: schema validation: {}", line, e));
                }
            }
        }

        Ok(issues)
    }

    pub fn doctor(&self) -> Result<Vec<String>> {
        let mut results = Vec::new();
        let mut score: i32 = 100;
        let mut task_count = 0u32;
        let mut replay_errors = 0u32;

        // Check events.jsonl readable
        match self.store.read_all() {
            Ok(events) => {
                results.push(format!("events.jsonl: OK ({} events)", events.len()));

                // Try to replay each task
                let task_ids = self.store.task_ids()?;
                task_count = task_ids.len() as u32;
                for tid in &task_ids {
                    match self.replay_task(tid) {
                        Ok(state) => {
                            results.push(format!(
                                "Task '{}': {:?} (seq {})",
                                tid, state.phase, state.last_seq
                            ));
                        }
                        Err(e) => {
                            replay_errors += 1;
                            score -= 15;
                            results.push(format!("Task '{}': REPLAY ERROR: {}", tid, e));
                        }
                    }
                }
            }
            Err(e) => {
                score -= 30;
                results.push(format!("events.jsonl: ERROR: {}", e));
            }
        }

        // Health Score deductions
        if score < 0 {
            score = 0;
        }
        results.push(String::new());
        results.push(format!("Health Score: {}/100", score));
        results.push(format!(
            "Tasks: {} total, {} replay errors",
            task_count, replay_errors
        ));

        Ok(results)
    }

    // ── Audit & Reports (M3) ──

    /// Generate a deterministic audit report from events + evidence.
    /// The report is deterministic: same events always produce the same report.
    pub fn generate_audit_report(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;
        let events = self.store.read_for_task(task_id)?;

        // Collect gate results
        let mut gate_reports = Vec::new();
        for gate_id in &state.gates {
            let result = state.gate_results.get(gate_id);
            gate_reports.push(serde_json::json!({
                "gate_id": gate_id,
                "passed": result.map(|r| r.passed).unwrap_or(false),
                "evidence": result.map(|r| r.evidence.as_str()).unwrap_or("no result"),
                "checked_at": result.map(|r| r.checked_at.as_str()).unwrap_or("never"),
            }));
        }

        // Count evidence events
        let evidence_accepted_count = events
            .iter()
            .filter(|e| e.event_type == "evidence_accepted")
            .count();
        let evidence_rejected_count = events
            .iter()
            .filter(|e| e.event_type == "evidence_rejected")
            .count();

        // Check for violations
        let violation_count = events
            .iter()
            .filter(|e| e.event_type == "boundary_violation_recorded")
            .count();

        // Completion interlock check
        let all_gates_pass = state
            .gates
            .iter()
            .all(|g| state.gate_results.get(g).map(|r| r.passed).unwrap_or(false));
        let interlock_verdict = if state.phase == Phase::Review
            && !state.is_held
            && all_gates_pass
            && evidence_rejected_count == 0
        {
            "allow"
        } else if state.phase == Phase::Completed {
            "completed"
        } else {
            "blocked"
        };

        let report = serde_json::json!({
            "schema": "control.audit-report.v1",
            "task_id": task_id,
            "phase": state.phase.as_str(),
            "is_held": state.is_held,
            "is_archived": state.is_archived,
            "objective": state.objective,
            "total_events": events.len(),
            "gates": gate_reports,
            "all_gates_pass": all_gates_pass,
            "evidence_accepted": evidence_accepted_count,
            "evidence_rejected": evidence_rejected_count,
            "violations": violation_count,
            "completion_interlock": {
                "phase_is_review": state.phase == Phase::Review,
                "no_hold": !state.is_held,
                "all_gates_pass": all_gates_pass,
                "no_rejected_evidence": evidence_rejected_count == 0,
                "verdict": interlock_verdict,
            },
            "write_scope": state.write_allow.iter().collect::<Vec<_>>(),
            "write_deny": state.write_deny.iter().collect::<Vec<_>>(),
            "last_seq": state.last_seq,
        });

        // Write report file
        let task_dir = self.store.task_dir(task_id)?;
        let report_path = task_dir.join("audit-report.json");
        if !self.dry_run {
            let temp_path = task_dir.join("audit-report.json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&report)?)?;
            std::fs::rename(&temp_path, &report_path)?;
        }

        Ok(report)
    }

    /// Generate a human-readable summary report.
    pub fn generate_status_report(&self) -> Result<Vec<serde_json::Value>> {
        let task_ids = self.store.task_ids()?;
        let mut reports = Vec::new();
        for task_id in &task_ids {
            let state = self.replay_task(task_id)?;
            reports.push(serde_json::json!({
                "task_id": task_id,
                "phase": state.phase.as_str(),
                "is_held": state.is_held,
                "is_archived": state.is_archived,
                "objective": state.objective,
                "gates_total": state.gates.len(),
                "gates_passing": state.gate_results.values().filter(|r| r.passed).count(),
                "last_seq": state.last_seq,
            }));
        }
        Ok(reports)
    }

    // ── Internal helpers ──

    pub fn replay_task(&self, task_id: &str) -> Result<TaskState> {
        let events = self.store.read_for_task(task_id)?;
        if events.is_empty() {
            return Err(anyhow!("Task '{}' not found", task_id));
        }
        let mut state = TaskState::new(task_id);
        for event in &events {
            apply(&mut state, event)
                .map_err(|e| anyhow!("Reducer error at seq {}: {}", event.seq, e))?;
        }
        Ok(state)
    }

    fn build_event(
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

    fn normalize_boundary_paths(
        &self,
        field: &str,
        paths: &[String],
        write: bool,
    ) -> Result<Vec<String>> {
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        let mut normalized = Vec::with_capacity(paths.len());
        for path in paths {
            let path = if write {
                normalizer.normalize_write(path)
            } else {
                normalizer.normalize(path)
            }
            .map_err(|e| anyhow!("Invalid {} path '{}': {}", field, path, e))?;
            normalized.push(path_to_payload_string(&path));
        }
        Ok(normalized)
    }

    fn validate_event(&self, event: &Event) -> Result<()> {
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

    fn validate_and_append(&self, event: &Event) -> Result<()> {
        self.validate_event(event)?;
        if self.dry_run {
            println!(
                "[dry-run] Would append event: type={}, task={}, seq={}",
                event.event_type, event.task_id, event.seq
            );
            return Ok(());
        }
        self.store.append(event)?;
        Ok(())
    }

    fn rebuild_task_view(&self, task_id: &str) -> Result<()> {
        let state = self.replay_task(task_id)?;
        self.store.write_task_view(task_id, &state)?;
        Ok(())
    }

    // ── M4: Workspace commands ──

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

        let diff_files =
            crate::infrastructure::workspace::diff_worktree(&self.project_root, &worktree_path)?;

        let high_risks = crate::infrastructure::workspace::detect_high_risk(&diff_files);

        let mut files_added = Vec::new();
        let mut files_modified = Vec::new();
        let mut files_deleted = Vec::new();

        for (status, path) in &diff_files {
            match status.as_str() {
                "A" => files_added.push(path.clone()),
                "D" => files_deleted.push(path.clone()),
                _ => files_modified.push(path.clone()),
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
        let diff_files =
            crate::infrastructure::workspace::diff_worktree(&self.project_root, &worktree_path)?;
        let high_risks = crate::infrastructure::workspace::detect_high_risk(&diff_files);

        // Check all files are within write_allow
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        let mut files_to_apply = Vec::new();
        for (status, path) in &diff_files {
            let normalized = normalizer
                .normalize(path)
                .map_err(|e| anyhow!("Invalid path '{}': {}", path, e))?;
            let normalized_str = normalized.to_string_lossy().replace('\\', "/");
            let in_scope = state.write_allow.iter().any(|scope| {
                let scope_norm = scope.replace('\\', "/");
                normalized_str.starts_with(scope_norm.as_str())
                    || normalized_str == scope_norm.as_str()
            });
            let in_deny = state.write_deny.iter().any(|scope| {
                let scope_norm = scope.replace('\\', "/");
                normalized_str.starts_with(scope_norm.as_str())
                    || normalized_str == scope_norm.as_str()
            });
            if !in_scope || in_deny {
                return Err(anyhow!(
                    "File '{}' is out of write scope or in deny list. Rule: scope_enforcement",
                    path
                ));
            }
            if status != "D" {
                files_to_apply.push(path.clone());
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
                    "High-risk change '{}' on '{}' requires valid approval (not expired). Rule: APPROVAL-001. Grant with: control approval grant --id {} --request <request_id>",
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

        // Apply files
        crate::infrastructure::workspace::apply_files(
            &self.project_root,
            &worktree_path,
            &files_to_apply,
        )?;

        let payload = serde_json::json!({
            "files_applied": files_to_apply,
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

    // ── M4: Approval commands ──

    pub fn approval_request(
        &self,
        task_id: &str,
        reason: &str,
        scope: serde_json::Value,
        ttl_seconds: u64,
    ) -> Result<Event> {
        let request_id = generate_uuid();
        let payload = serde_json::json!({
            "request_id": request_id,
            "reason": reason,
            "scope": scope,
            "ttl_seconds": ttl_seconds,
        });
        let event = self.build_event(task_id, "approval_requested", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn approval_grant(&self, task_id: &str, request_id: &str) -> Result<Event> {
        let payload = serde_json::json!({
            "request_id": request_id,
        });
        let event = self.build_event(task_id, "approval_granted", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn approval_deny(&self, task_id: &str, request_id: &str) -> Result<Event> {
        let payload = serde_json::json!({
            "request_id": request_id,
        });
        let event = self.build_event(task_id, "approval_denied", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    // ── M4: Run lifecycle commands ──

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
            "ttl_seconds": 3600,
            "max_uses": 100,
        });
        let lease_event = self.build_event(task_id, "lease_created", lease_payload)?;
        self.validate_and_append(&lease_event)?;

        // Generate run manifest
        let adapter: Box<dyn ExecutorAdapter> = match adapter_name {
            "omp" => Box::new(OmpAdapter),
            _ => return Err(anyhow!("Unknown adapter: {}", adapter_name)),
        };

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

    pub fn run_ingest_omp(&self, task_id: &str, result_file: &Path) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.active_run.is_none() {
            return Err(anyhow!("No active run for task '{}'", task_id));
        }

        let content = std::fs::read_to_string(result_file)?;
        let result: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| anyhow!("Invalid result file: {}", e))?;

        // Validate via OMP adapter
        let omp = OmpAdapter;
        omp.validate_output(&result)?;

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
                let scope_norm = scope.replace('\\', "/");
                normalized_str.starts_with(scope_norm.as_str())
                    || normalized_str == scope_norm.as_str()
            });
            let in_deny = state.write_deny.iter().any(|scope| {
                let scope_norm = scope.replace('\\', "/");
                normalized_str.starts_with(scope_norm.as_str())
                    || normalized_str == scope_norm.as_str()
            });
            if !in_scope || in_deny {
                let evidence_id = generate_uuid();
                let payload = serde_json::json!({
                    "evidence_id": evidence_id,
                    "source": "omp",
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
            "source": "omp",
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

    pub fn adapter_capabilities(&self, adapter_name: &str) -> Result<serde_json::Value> {
        let adapter: Box<dyn ExecutorAdapter> = match adapter_name {
            "omp" => Box::new(OmpAdapter),
            _ => return Err(anyhow!("Unknown adapter: {}", adapter_name)),
        };
        Ok(adapter.capabilities())
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

    // ── M4: Helpers ──

    fn get_worktree_path(&self, task_id: &str) -> Result<PathBuf> {
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

    /// AC4: Check that no other task holds an active lease with overlapping write scope.
    /// ADAPTER-005: M6 前禁止多个 agent 并发写入。
    fn check_cross_task_lease_overlap(
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
                    let scope_norm = scope.replace('\\', "/");
                    lease_resource.starts_with(scope_norm.as_str())
                        || scope_norm.starts_with(lease_resource.as_str())
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
    fn check_lease_valid(&self, task_id: &str, state: &TaskState) -> Result<()> {
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
    fn expire_stale_leases(&self, task_id: &str) -> Result<()> {
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
                let scope_norm = scope.replace('\\', "/");
                normalized_str.starts_with(scope_norm.as_str())
                    || normalized_str == scope_norm.as_str()
            });
            let in_deny = state.write_deny.iter().any(|scope| {
                let scope_norm = scope.replace('\\', "/");
                normalized_str.starts_with(scope_norm.as_str())
                    || normalized_str == scope_norm.as_str()
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

fn validate_task_definition(
    objective: &str,
    read_scope: &[String],
    write_allow: &[String],
    gates: &[String],
) -> Result<()> {
    if objective.trim().is_empty() {
        return Err(anyhow!("Task objective must not be empty"));
    }
    if read_scope.is_empty() {
        return Err(anyhow!("Task read_scope must not be empty"));
    }
    if write_allow.is_empty() {
        return Err(anyhow!("Task write_allow must not be empty"));
    }
    if gates.is_empty() {
        return Err(anyhow!("Task gates must not be empty"));
    }
    Ok(())
}

fn validate_gate_templates(gates: &[String]) -> Result<Vec<String>> {
    let mut validated = Vec::with_capacity(gates.len());
    for gate_id in gates {
        if crate::infrastructure::gates::find_template(gate_id).is_none() {
            return Err(anyhow!(
                "Unknown gate '{}' — only known gate templates are allowed",
                gate_id
            ));
        }
        validated.push(gate_id.clone());
    }
    Ok(validated)
}

fn path_to_payload_string(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        if let std::path::Component::Normal(part) = component {
            parts.push(part.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

// ── File hashing helpers ──

fn collect_file_hashes(
    dir: &std::path::Path,
    root: &std::path::Path,
    results: &mut Vec<serde_json::Value>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden dirs and target.
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            collect_file_hashes(&path, root, results)?;
        } else if path.is_file() {
            let hash = hash_file(&path)?;
            let rel = path.strip_prefix(root).unwrap_or(&path);
            results.push(serde_json::json!({
                "path": path_to_payload_string(rel),
                "hash": hash,
            }));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn collect_files_recursive(
    dir: &std::path::Path,
    root: &std::path::Path,
    results: &mut std::collections::HashSet<String>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            collect_files_recursive(&path, root, results)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            results.insert(path_to_payload_string(rel));
        }
    }
    Ok(())
}

fn hash_file(path: &std::path::Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn new_validator_if_available() -> Option<SchemaValidator> {
    if std::path::Path::new("schemas").exists() {
        SchemaValidator::new("schemas/").ok()
    } else {
        None
    }
}

// ── M5: drift signal derivation (pure over already-loaded data) ──

/// Build the drift signals from a task's events, its reduced state, and its
/// telemetry entries. Kept free-standing so both `collect_drift_signals` and
/// the `control.json` board projection derive signals identically.
fn drift_signals_from(
    events: &[Event],
    state: &TaskState,
    telemetry: &[crate::domain::telemetry::TelemetryEntry],
) -> crate::domain::drift::DriftSignals {
    let boundary_violations = events
        .iter()
        .filter(|e| e.event_type == "boundary_violation_recorded")
        .count() as u32;
    let gate_failures = state
        .gates
        .iter()
        .filter(|g| {
            state
                .gate_results
                .get(g.as_str())
                .map(|r| !r.passed)
                .unwrap_or(false)
        })
        .count() as u32;
    let unresolved_rejections = ControlApp::review_status_from_events(events) == "needs_work";

    // Saturating sums: a pathological flood of large values can't overflow the
    // accumulator (the rule checks only care about thresholds, not exact totals).
    let (mut test_failures, mut lint_errors, mut retries, mut unexpected_writes) =
        (0i64, 0i64, 0i64, 0i64);
    let mut unknown_signal = false;
    for entry in telemetry {
        match entry.kind.as_str() {
            "test_failures" => test_failures = test_failures.saturating_add(entry.value),
            "lint_errors" => lint_errors = lint_errors.saturating_add(entry.value),
            "retries" | "attempts" => retries = retries.saturating_add(entry.value),
            "unexpected_writes" => {
                unexpected_writes = unexpected_writes.saturating_add(entry.value)
            }
            _ => unknown_signal = true,
        }
    }

    crate::domain::drift::DriftSignals {
        boundary_violations,
        gate_failures,
        unresolved_rejections,
        is_held: state.is_held,
        test_failures,
        lint_errors,
        retries,
        unexpected_writes,
        unknown_signal,
    }
}

// ── UUID generation (no external crate) ──

static UUID_COUNTER: AtomicU64 = AtomicU64::new(0);
pub fn generate_uuid() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c = UUID_COUNTER.fetch_add(1, Ordering::Relaxed);

    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:08x}{:04x}",
        (ts.wrapping_add(c)) as u32,
        ((ts >> 16) ^ c) as u16,
        (ts >> 32) as u16 & 0x0FFF,
        (c >> 4) as u16 & 0x0FFF,
        (ts >> 8) as u32,
        (c & 0xFFFF) as u16,
    )
}

// ── ISO 8601 timestamp (no external crate) ──

pub fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d, h, mi, s) = epoch_to_datetime(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, mi, s)
}

/// Convert Unix epoch seconds to (year, month, day, hour, minute, second).
/// Based on Howard Hinnant's algorithm.
fn epoch_to_datetime(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs / 86400;
    let time_secs = secs % 86400;

    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    (
        y as u64,
        m as u64,
        d as u64,
        time_secs / 3600,
        (time_secs % 3600) / 60,
        time_secs % 60,
    )
}

/// Parse a simple ISO 8601 UTC string (YYYY-MM-DDTHH:MM:SSZ) to Unix epoch seconds.
/// Returns None if parsing fails (fail-closed).
fn parse_iso8601_to_epoch(s: &str) -> Option<u64> {
    // Expected format: "2026-06-07T10:00:00Z" (len 20)
    if s.len() < 19 {
        return None;
    }
    let year: u64 = s.get(0..4)?.parse().ok()?;
    let month: u64 = s.get(5..7)?.parse().ok()?;
    let day: u64 = s.get(8..10)?.parse().ok()?;
    let hour: u64 = s.get(11..13)?.parse().ok()?;
    let minute: u64 = s.get(14..16)?.parse().ok()?;
    let second: u64 = s.get(17..19)?.parse().ok()?;

    // Days from year 0 using civil_from_days approach
    let m = month;
    let y = if m <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146097 + doe - 719468;

    Some(days_since_epoch * 86400 + hour * 3600 + minute * 60 + second)
}

/// Get the occurred_at timestamp for a given event seq in a task's event stream.
fn event_occurred_at_by_seq(events: &[Event], seq: i64) -> Option<String> {
    events
        .iter()
        .find(|e| e.seq == seq)
        .map(|e| e.occurred_at.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("control-app-test-{}", generate_uuid()));
            std::fs::create_dir_all(path.join("src")).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn create_task_writes_canonical_trellis_task_ledger_and_projection() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let read_scope = vec!["src".to_string()];
        let write_allow = vec!["src".to_string()];
        let write_deny = Vec::new();
        let risk_triggers = Vec::new();
        let gates = vec!["cargo_check".to_string()];

        app.create_task(
            "ledger-task",
            CreateTaskInput {
                objective: "Implement ledger",
                read_scope: &read_scope,
                write_allow: &write_allow,
                write_deny: &write_deny,
                risk_triggers: &risk_triggers,
                gates: &gates,
                depends_on: &[],
            },
        )
        .unwrap();

        assert!(dir
            .path()
            .join(".ctl/tasks/ledger-task/events.jsonl")
            .exists());
        assert!(dir.path().join(".ctl/tasks/ledger-task/task.json").exists());
        assert!(!dir.path().join(".control").join("events.jsonl").exists());
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {:?} failed", args);
    }

    /// Drive a task to Review with a passing gate but NO completion audit.
    fn drive_to_review_bare(app: &ControlApp, id: &str) {
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "interlock test",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &gates,
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready(id).unwrap();
        app.start_task(id).unwrap();
        app.submit_task(id).unwrap();
        app.record_gate(id, "cargo_check", true, "ok").unwrap();
    }

    /// Record a passing completion audit as a non-implementer reviewer (M6):
    /// the implementer (`task_started` actor) is the default "human", so the
    /// reviewer acts under a distinct identity.
    fn audit_pass(app: &ControlApp, id: &str, note: Option<&str>) {
        ControlApp::open(&app.project_root, false)
            .unwrap()
            .with_actor("reviewer")
            .record_completion_audit(id, true, note)
            .unwrap();
    }

    fn drive_to_review(app: &ControlApp, id: &str) {
        drive_to_review_bare(app, id);
        // M-f: a fresh passing completion audit is now a finish prerequisite;
        // M6: it must come from a non-implementer reviewer.
        audit_pass(app, id, None);
    }

    #[test]
    fn finish_blocked_without_completion_audit() {
        // No git repo → M-g commit interlock is skipped, isolating the M-f gate.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "noaudit");
        let err = app.finish_task("noaudit").unwrap_err().to_string();
        assert!(
            err.contains("no passing completion audit"),
            "expected M-f review gate, got: {err}"
        );
    }

    #[test]
    fn finish_allowed_with_fresh_completion_audit() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "audited");
        audit_pass(&app, "audited", Some("looks good"));
        let event = app.finish_task("audited").unwrap();
        assert_eq!(event.event_type, "task_completed");
    }

    #[test]
    fn finish_blocked_by_failing_completion_audit() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "failed");
        app.record_completion_audit("failed", false, Some("missing tests"))
            .unwrap();
        let err = app.finish_task("failed").unwrap_err().to_string();
        assert!(
            err.contains("latest completion audit is a FAIL"),
            "expected fail-verdict block, got: {err}"
        );
    }

    #[test]
    fn audit_before_resubmit_is_stale_and_does_not_count() {
        // A pass from a PRIOR review round must not satisfy finish after rework
        // (reopen → resubmit). Freshness is keyed on the last submit's seq.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "rework");
        audit_pass(&app, "rework", None);
        // Rework: back to in_progress, then re-submit. The earlier audit is now
        // before the latest submit and no longer counts.
        app.reopen_task("rework").unwrap();
        app.submit_task("rework").unwrap();
        app.record_gate("rework", "cargo_check", true, "ok")
            .unwrap();
        let err = app.finish_task("rework").unwrap_err().to_string();
        assert!(
            err.contains("no passing completion audit"),
            "stale pre-rework audit must not satisfy finish, got: {err}"
        );
        // A fresh audit after the new submit unblocks it.
        audit_pass(&app, "rework", None);
        assert_eq!(
            app.finish_task("rework").unwrap().event_type,
            "task_completed"
        );
    }

    #[test]
    fn completion_audit_requires_review_phase() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            "early",
            CreateTaskInput {
                objective: "x",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &gates,
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready("early").unwrap();
        app.start_task("early").unwrap();
        // Still in_progress (not submitted) → audit must be rejected.
        let err = app
            .record_completion_audit("early", true, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("only be recorded in Review"),
            "expected phase guard, got: {err}"
        );
    }

    #[test]
    fn implementer_cannot_self_approve_completion_audit() {
        // M6: the actor who started/implemented the task may not record its own
        // passing audit. `app` (default actor "human") started the task.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "selfapp");
        let err = app
            .record_completion_audit("selfapp", true, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Reviewer-lease binding") && err.contains("human"),
            "implementer self-approval must be blocked, got: {err}"
        );
        // A distinct reviewer can accept it.
        audit_pass(&app, "selfapp", None);
        assert_eq!(
            app.finish_task("selfapp").unwrap().event_type,
            "task_completed"
        );
    }

    #[test]
    fn implementer_may_self_reject_completion_audit() {
        // A FAIL from the implementer (self-flagging a problem) is allowed —
        // only self-approval is the threat.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "selfrej");
        let ev = app
            .record_completion_audit("selfrej", false, Some("found a bug myself"))
            .unwrap();
        assert_eq!(ev.event_type, "evidence_rejected");
    }

    #[test]
    fn event_actor_comes_from_with_actor_override() {
        // M6 foundation: events are stamped with the instance actor, not a
        // hardcoded "human".
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap().with_actor("agent-7");
        let ev = app
            .create_task(
                "act",
                CreateTaskInput {
                    objective: "x",
                    read_scope: &["src".to_string()],
                    write_allow: &["src".to_string()],
                    write_deny: &[],
                    risk_triggers: &[],
                    gates: &["cargo_check".to_string()],
                    depends_on: &[],
                },
            )
            .unwrap();
        assert_eq!(ev.actor, "agent-7");
    }

    #[test]
    fn finish_blocked_by_uncommitted_work_in_scope() {
        let dir = TempDir::new();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@t"]);
        git(dir.path(), &["config", "user.name", "t"]);

        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review(&app, "ilk");

        // Uncommitted file inside write scope (src/) → finish must fail closed.
        std::fs::write(dir.path().join("src/work.rs"), "fn w() {}\n").unwrap();
        let err = app.finish_task("ilk").unwrap_err().to_string();
        assert!(
            err.contains("uncommitted changes"),
            "expected commit interlock, got: {err}"
        );

        // Commit the work → tree clean in scope → finish succeeds.
        git(dir.path(), &["add", "src/work.rs"]);
        git(dir.path(), &["commit", "-qm", "work"]);
        let event = app.finish_task("ilk").unwrap();
        assert_eq!(event.event_type, "task_completed");
    }

    fn create_planning(app: &ControlApp, id: &str) {
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "board test",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &gates,
                depends_on: &[],
            },
        )
        .unwrap();
    }

    #[test]
    fn board_aggregates_tasks_by_phase_and_activity() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // "a" → in_progress (active); "b" → stays planning (not active).
        create_planning(&app, "a");
        app.mark_ready("a").unwrap();
        app.start_task("a").unwrap();
        create_planning(&app, "b");

        let board = app.generate_board().unwrap();
        assert_eq!(board["totals"]["tasks"], 2);
        assert_eq!(board["totals"]["active"], 1);
        assert_eq!(board["totals"]["held"], 0);
        assert_eq!(board["totals"]["needs_work"], 0);

        let tasks = board["tasks"].as_array().unwrap();
        let a = tasks.iter().find(|t| t["task_id"] == "a").unwrap();
        assert_eq!(a["phase"], "in_progress");
        assert_eq!(a["active"], true);
        assert_eq!(a["review"], "none");
        let b = tasks.iter().find(|t| t["task_id"] == "b").unwrap();
        assert_eq!(b["active"], false);
    }

    #[test]
    fn reconcile_projects_deterministic_control_json() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        create_planning(&app, "a");
        create_planning(&app, "b");

        app.reconcile().unwrap();
        let path = dir.path().join(".ctl/control.json");
        assert!(path.exists(), "reconcile must project control.json");
        let first = std::fs::read_to_string(&path).unwrap();

        app.reconcile().unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            first, second,
            "control.json must be byte-identical on replay"
        );
    }

    /// Create + ready + start a simple in-scope task (no review).
    fn start_simple(app: &ControlApp, id: &str) {
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "m5 test",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &gates,
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready(id).unwrap();
        app.start_task(id).unwrap();
    }

    fn event_count(app: &ControlApp, id: &str) -> usize {
        app.store.read_for_task(id).unwrap().len()
    }

    #[test]
    fn telemetry_add_writes_index_and_feeds_drift() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        start_simple(&app, "t");

        // Clean task → no drift.
        assert_eq!(app.compute_drift("t").unwrap().score, 0);

        let before = event_count(&app, "t");
        app.telemetry_add("t", "test_failures", 2, "human").unwrap();
        app.telemetry_add("t", "retries", 4, "human").unwrap();
        assert!(dir.path().join(".ctl/telemetry.jsonl").exists());

        // Telemetry is evidence, NOT a canonical event — the ledger is unchanged.
        assert_eq!(
            event_count(&app, "t"),
            before,
            "telemetry must not append events"
        );

        // 15 (test_failures) + 15 (retries>=3) = 30 = medium.
        let report = app.compute_drift("t").unwrap();
        assert_eq!(report.score, 30);
        assert_eq!(report.level.as_str(), "medium");
        assert_eq!(report.fired_ids(), vec!["DRIFT-004", "DRIFT-006"]);
    }

    #[test]
    fn telemetry_add_dry_run_writes_nothing() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        start_simple(&app, "t");
        let dry = ControlApp::open(&app.project_root, true).unwrap();
        dry.telemetry_add("t", "test_failures", 1, "human").unwrap();
        assert!(!dir.path().join(".ctl/telemetry.jsonl").exists());
    }

    #[test]
    fn unknown_signal_makes_next_action_ask_and_emits_no_events() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        start_simple(&app, "t");
        app.telemetry_add("t", "mystery_signal", 1, "human")
            .unwrap();
        let before = event_count(&app, "t");
        let proposal = app.next_action("t").unwrap();
        assert_eq!(proposal.action.as_str(), "ask");
        assert_eq!(
            event_count(&app, "t"),
            before,
            "next_action must be read-only"
        );
    }

    #[test]
    fn next_action_replan_only_proposes() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        start_simple(&app, "t");
        // Three telemetry signals → high drift, no out-of-scope signal → replan.
        app.telemetry_add("t", "test_failures", 1, "human").unwrap(); // 15
        app.telemetry_add("t", "retries", 3, "human").unwrap(); // 15
                                                                // gate failing (20) pushes to 50 = high.
        app.record_gate("t", "cargo_check", false, "boom").unwrap();
        let before = event_count(&app, "t");
        let proposal = app.next_action("t").unwrap();
        assert_eq!(proposal.action.as_str(), "replan");
        assert!(proposal.structured_proposal.is_some());
        // The proposal is advisory: no scope change, no new events.
        assert_eq!(event_count(&app, "t"), before);
    }

    #[test]
    fn reconcile_with_telemetry_is_byte_identical() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        start_simple(&app, "t");
        app.telemetry_add("t", "test_failures", 2, "human").unwrap();
        app.telemetry_add("t", "unexpected_writes", 1, "human")
            .unwrap();

        app.reconcile().unwrap();
        let path = dir.path().join(".ctl/control.json");
        let first = std::fs::read_to_string(&path).unwrap();
        // Drift fields are present in the projection.
        assert!(first.contains("drift_level"));
        assert!(first.contains("recommended_action"));

        app.reconcile().unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            first, second,
            "control.json with telemetry must be byte-identical on replay"
        );
    }

    #[test]
    fn finish_skips_interlock_outside_git_repo() {
        // Non-git temp dir: tree is unverifiable, so the interlock is skipped
        // and finish falls through to its other checks (here: succeeds).
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review(&app, "nogit");
        std::fs::write(dir.path().join("src/work.rs"), "fn w() {}\n").unwrap();
        let event = app.finish_task("nogit").unwrap();
        assert_eq!(event.event_type, "task_completed");
    }

    #[test]
    fn test_generate_uuid_format() {
        let uuid = generate_uuid();
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        assert!(uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn test_generate_uuid_unique() {
        let a = generate_uuid();
        let b = generate_uuid();
        assert_ne!(a, b);
    }

    #[test]
    fn test_now_iso8601_format() {
        let ts = now_iso8601();
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }

    #[test]
    fn test_epoch_to_datetime() {
        // 2026-01-01T00:00:00Z = 1767225600
        let (y, m, d, h, mi, s) = epoch_to_datetime(1767225600);
        assert_eq!(y, 2026);
        assert_eq!(m, 1);
        assert_eq!(d, 1);
        assert_eq!(h, 0);
        assert_eq!(mi, 0);
        assert_eq!(s, 0);
    }
}
