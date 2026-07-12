pub mod prd;
pub mod schedule;
pub mod spec;
use anyhow::{anyhow, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::adapter_for;
use crate::domain::event::Event;
use crate::domain::lease::LeaseStatus;
use crate::domain::run::{apply_run, AgentRunState, RunPhase};
use crate::domain::task::{apply, Phase, TaskKind, TaskState};
use crate::infrastructure::schema_validator::SchemaValidator;
use crate::infrastructure::store::run_store::RunEventStore;
use crate::infrastructure::store::FileEventStore;
use std::collections::BTreeSet;

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

/// V1 run-scoped capability-lease defaults, shared so the M4 task-embedded run
/// path and the M6 run-aggregate path grant identical leases. `max_uses` must be
/// at least 2 so the single use consumed at start does not immediately expire
/// (and thus make non-active) a freshly Running run.
pub const RUN_LEASE_TTL_SECONDS: u64 = 3600;
pub const RUN_LEASE_MAX_USES: u64 = 100;

/// Risk-trigger sentinel that opts a task into the TDD red→green completion
/// interlock (ctl-tdd-loop-v1). Carried in `risk_triggers` (an existing
/// free-form, schema-declared field), so enabling it needs no schema or
/// aggregate change; set conveniently via `ctl task create --tdd`.
pub const TDD_RED_GREEN_TRIGGER: &str = "tdd-red-green";

/// The gate whose `gate_checked` history must show red→green for a TDD-enforced
/// task. The canonical test gate.
const TDD_TEST_GATE: &str = "cargo_test";

/// True if `gate_id`'s `gate_checked` history contains a FAILING result at an
/// earlier seq than a PASSING one — i.e. the test demonstrably went red→green.
/// Read-only over the task's event stream.
fn gate_went_red_before_green(events: &[Event], gate_id: &str) -> bool {
    let mut first_fail_seq: Option<i64> = None;
    for e in events {
        if e.event_type != "gate_checked"
            || e.payload.get("gate_id").and_then(|v| v.as_str()) != Some(gate_id)
        {
            continue;
        }
        let passed = e
            .payload
            .get("passed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !passed {
            first_fail_seq.get_or_insert(e.seq);
        } else if first_fail_seq.is_some_and(|fseq| e.seq > fseq) {
            return true; // a pass after a prior fail
        }
    }
    false
}

/// Host-supplied provenance for `ctl run finish` (run-attestation-fields-v1).
/// Record-and-disclose: ctl records these host-attested values and sha256-hashes
/// the artifact files it is given — it does NOT verify what actually ran. Every
/// field is optional; a run may finish with none.
#[derive(Debug, Clone, Default)]
pub struct RunProvenanceInput {
    pub model: Option<String>,
    pub provider: Option<String>,
    /// Path to the instruction artifact; ctl records its sha256.
    pub instruction_artifact: Option<String>,
    /// Path to the context artifact; ctl records its sha256.
    pub context_artifact: Option<String>,
    /// Path to the output artifact; ctl records its sha256.
    pub output_artifact: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub exit_code: Option<i64>,
}

/// M6 crash-recovery snapshot of one `Running` run (see [`ControlApp::recover_report`]).
/// `worktree_exists == false` marks an inconsistent run whose isolation
/// workspace is gone — a recovery-abort candidate.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunRecoveryStatus {
    pub run_id: String,
    pub task_id: String,
    pub write_allow: Vec<String>,
    pub worktree_path: Option<String>,
    pub worktree_exists: bool,
    pub manifest_exists: bool,
    /// The run's lease id (native or legacy opaque), if any.
    pub lease_id: Option<String>,
    /// Structured lease status token: `ACTIVE` / `REVOKED` / `EXPIRED`, or
    /// `UNKNOWN` for a legacy (pre-lease) run.
    pub lease_status: String,
    /// `native` once a `lease_created` is in the run's stream; `pre_lease_run`
    /// for slice-1 runs that predate run-scoped leases.
    pub lease_compat: String,
    /// Remaining lease uses (native leases only).
    pub remaining_uses: Option<u64>,
    /// Wall-clock TTL exceeded for a still-Active lease. Reported only — recover
    /// never appends `lease_expired`.
    pub lease_stale: bool,
    /// A Running run whose native lease is not Active — an anomaly worth a look.
    pub lease_nonactive: bool,
}

/// One task↔run↔registry↔worktree inconsistency, with the explicit repair that
/// `ctl repair --cross-ledger --apply` would perform.
///
/// A task transition and its run-ledger counterpart are two separate appends
/// (each single-writer, but with no transaction spanning both), so a crash
/// between them can leave the ledgers disagreeing. This classifies the
/// disagreement and names a single, conservative repair — it never fabricates a
/// "correct" history, only retires the stale side (abort the live run, or remove
/// a leftover worktree).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrossLedgerFinding {
    pub kind: CrossLedgerKind,
    pub run_id: String,
    pub task_id: Option<String>,
    pub detail: String,
    pub repair: RepairAction,
}

/// The class of cross-ledger drift. One finding per run, chosen by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossLedgerKind {
    /// A non-terminal run whose `task_id` has no task ledger at all.
    OrphanRun,
    /// A Running run whose task is already terminal (Completed/Cancelled) — the
    /// classic non-atomic window (task closed, run not).
    StrandedRun,
    /// A Running run whose isolated worktree is gone (crash mid-run).
    MissingWorktreeRun,
    /// A Queued run holding a lease that never reached `run_started` (crash
    /// mid-start).
    PartialStartRun,
    /// A terminal run whose worktree dir still lingers on disk (leftover
    /// isolation, safe to prune).
    OrphanedWorktree,
}

impl CrossLedgerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CrossLedgerKind::OrphanRun => "orphan_run",
            CrossLedgerKind::StrandedRun => "stranded_run",
            CrossLedgerKind::MissingWorktreeRun => "missing_worktree_run",
            CrossLedgerKind::PartialStartRun => "partial_start_run",
            CrossLedgerKind::OrphanedWorktree => "orphaned_worktree",
        }
    }
}

/// The single repair an inconsistency maps to. Conservative by construction:
/// either retire a stale run (which appends `run_aborted` — the canonical repair
/// evidence) or remove a leftover worktree dir (fs-only; the run ledger is
/// already terminal and correct).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RepairAction {
    /// Abort the run: revoke its lease, clean its worktree, append `run_aborted`.
    AbortRun { reason: String },
    /// Remove the leftover worktree directory (fs-only — no ledger event).
    RemoveWorktree { path: String },
}

impl RepairAction {
    /// One-line preview of what `--apply` would do.
    pub fn preview(&self) -> String {
        match self {
            RepairAction::AbortRun { reason } => {
                format!("abort run (revoke lease, clean worktree, append run_aborted) — {reason}")
            }
            RepairAction::RemoveWorktree { path } => {
                format!("remove leftover worktree dir {path} (fs-only, no ledger event)")
            }
        }
    }
}

/// Outcome of applying one cross-ledger repair.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RepairOutcome {
    pub run_id: String,
    pub kind: CrossLedgerKind,
    pub applied: bool,
    pub result: String,
}

/// GO / NO-GO verdict for the ralph unattended-supervisor loop
/// (ralph-safe-run-v1). `go == true` means it is still safe to continue without
/// a human; otherwise `blockers` lists every reason attention is due. Purely
/// advisory and read-only — it never mutates and never spawns.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RalphVerdict {
    pub go: bool,
    pub blockers: Vec<String>,
}

/// Outcome of an explicit run-lease TTL-expiry attempt
/// (capability-lease-ttl-enforce-v1). `outcome` is one of `expired`,
/// `would_expire` (preview), `within_ttl` (refused — not stale), `not_active`,
/// or `no_lease`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LeaseExpiryReport {
    pub run_id: String,
    pub outcome: String,
    pub age_secs: Option<u64>,
    pub ttl_secs: Option<u64>,
    pub detail: String,
}

/// Deterministic "what should I work on next" recommendation. Ranks Ready
/// tasks by satisfied dependencies + lowest drift + no active scope conflict;
/// falls back to Planning tasks when no Ready task is actionable. Read-only.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NextTaskRecommendation {
    /// "start" (a Ready task is actionable) | "ready" (a Planning task is next)
    /// | "none" (nothing actionable).
    pub action: &'static str,
    pub task_id: Option<String>,
    pub objective: Option<String>,
    pub rationale: String,
    pub ready_candidates: usize,
    pub planning_candidates: usize,
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
        self.create_task_with_kind(id, input, TaskKind::Implementation)
    }

    /// Create a task with an explicit kind (Research/Spike V1). `create_task`
    /// delegates here with `Implementation`. The kind is fixed at creation and
    /// never revised; the field is emitted only for research tasks so
    /// implementation payloads stay byte-identical to pre-feature output.
    pub fn create_task_with_kind(
        &self,
        id: &str,
        input: CreateTaskInput<'_>,
        kind: TaskKind,
    ) -> Result<Event> {
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
        if kind != TaskKind::Implementation {
            payload["task_kind"] = serde_json::json!(kind.as_str());
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

    /// M6 dependency-gated start: the declared `depends_on` task IDs of
    /// `task_id` that are NOT yet satisfied. A dependency counts as satisfied
    /// only when the task exists and has reached `Completed` (archiving keeps
    /// the phase, so an archived-completed prerequisite still satisfies).
    /// Every other case — a missing/unknown task, or a phase of
    /// planning/ready/in_progress/review/cancelled — is unmet. This fails
    /// closed: a dependent never starts ahead of, or alongside, an unfinished
    /// prerequisite. The result is sorted for stable diagnostics.
    ///
    /// This is a cross-task read, so it lives in the app layer (the reducer
    /// stays pure and can only see one task's own log), mirroring the M-a
    /// multiple-active-writer interlock.
    pub fn unmet_dependencies(&self, task_id: &str) -> Result<Vec<String>> {
        let state = self.replay_task(task_id)?;
        let mut unmet: Vec<String> = state
            .depends_on
            .iter()
            .filter(|dep| {
                !matches!(
                    self.replay_task(dep),
                    Ok(dep_state) if dep_state.phase == Phase::Completed
                )
            })
            .cloned()
            .collect();
        unmet.sort();
        Ok(unmet)
    }

    pub fn start_task(&self, task_id: &str) -> Result<Event> {
        // M6 dependency-gated start: refuse while any declared dependency is
        // unfinished, so a dependency chain runs strictly serially.
        let blocked_by = self.unmet_dependencies(task_id)?;
        if !blocked_by.is_empty() {
            return Err(anyhow!(
                "Cannot start '{}': blocked by unfinished dependencies [{}]. \
                 Complete (and archive) each prerequisite first, or drop the edge \
                 with `ctl task revise --id {} --depends-on <remaining ids>`.",
                task_id,
                blocked_by.join(", "),
                task_id
            ));
        }
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

        // Artifact binding (tree_hash): the code being completed must be the code
        // the latest required gate and the accepted completion audit validated.
        // Bound to the committed tree (HEAD^{tree}); skipped outside a git repo,
        // mirroring the M-g commit interlock. `None` here disables the binding
        // checks below so non-git flows behave exactly as before.
        let current_tree = crate::infrastructure::workspace::head_tree_hash(&self.project_root)?;
        // Policy binding: the rules in force now must match the rules the evidence
        // was produced under. Always computable (independent of git).
        let current_policy = self.current_policy_hash(&state);

        // Gate interlock: all required gates must have a latest PASSING result,
        // bound to the current committed tree (git repo only) AND the current
        // policy (always). Unbound (legacy `None`) counts as stale.
        let mut failing_gates = Vec::new();
        let mut tree_stale = Vec::new();
        let mut policy_stale = Vec::new();
        for gate_id in &state.gates {
            match state.gate_results.get(gate_id) {
                Some(result) if result.passed => {
                    if let Some(ref current) = current_tree {
                        if result.tree_hash.as_deref() != Some(current.as_str()) {
                            tree_stale.push(gate_id.as_str());
                        }
                    }
                    if result.policy_hash.as_deref() != Some(current_policy.as_str()) {
                        policy_stale.push(gate_id.as_str());
                    }
                }
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
        if !tree_stale.is_empty() || !policy_stale.is_empty() {
            let tcur = current_tree.as_deref().unwrap_or("n/a (non-git)");
            return Err(anyhow!(
                "Completion interlock: completion evidence is stale.\n\
                 current tree:        {tcur}\n\
                 current policy:      {current_policy}\n\
                 tree-stale gate(s):  {tree_stale:?}\n\
                 policy-stale gate(s): {policy_stale:?}\n\
                 rerun required gates (and re-audit) under the current code and policy"
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

        // TDD red→green interlock (ctl-tdd-loop-v1), opt-in via the
        // `tdd-red-green` risk trigger. A test that only ever passed proves
        // nothing; require the test gate to have FAILED (red) at an earlier
        // point than it PASSED (green) in this task's own gate history —
        // evidence the test can actually fail. Derived from the existing
        // `gate_checked` event stream, so no new event type or schema.
        if state.risk_triggers.contains(TDD_RED_GREEN_TRIGGER) {
            if !state.gates.contains(TDD_TEST_GATE) {
                return Err(anyhow!(
                    "Completion interlock (tdd-red-green): task opted into TDD but has no \
                     '{TDD_TEST_GATE}' gate to prove red→green. Add it to the task's gates."
                ));
            }
            if !gate_went_red_before_green(&events, TDD_TEST_GATE) {
                return Err(anyhow!(
                    "Completion interlock (tdd-red-green): no red→green evidence for \
                     '{TDD_TEST_GATE}'. TDD requires the test to FAIL before it PASSES — run \
                     the gate while the test is red (before implementing), then again once \
                     green. Found no failing '{TDD_TEST_GATE}' result preceding a passing one."
                ));
            }
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
            Some(e) if e.event_type == "evidence_accepted" => {
                // Artifact + policy binding: the passing audit must be bound to the
                // current committed tree (git repo only) AND the current policy.
                let mut stale = Vec::new();
                if let Some(ref current) = current_tree {
                    if e.payload.get("tree_hash").and_then(|v| v.as_str()) != Some(current.as_str())
                    {
                        stale.push("tree");
                    }
                }
                if e.payload.get("policy_hash").and_then(|v| v.as_str())
                    != Some(current_policy.as_str())
                {
                    stale.push("policy");
                }
                if !stale.is_empty() {
                    return Err(anyhow!(
                        "Completion interlock: completion audit is stale ({}); \
                         re-audit under the current code and policy before finishing",
                        stale.join(" + ")
                    ));
                }
            }
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

        // Research/Spike V1: a research task is not exempt from execution
        // integrity (all checks above still applied). It additionally must show a
        // non-degenerate footprint — at least one tracked artifact and at least
        // one uncertainty outcome — so a spike never completes looking identical
        // to an implementation task that produced nothing. This NEVER requires the
        // open-uncertainty count to fall: opening unknowns is a legitimate result.
        if state.task_kind == TaskKind::Research {
            if state.research_artifacts.is_empty() {
                return Err(anyhow!(
                    "Completion interlock: research task requires at least one recorded \
                     research artifact (ctl research record --id {})",
                    task_id
                ));
            }
            // Freshness floor: a finish must point at at least one artifact that
            // still matches what was recorded. An artifact deleted or edited away
            // after recording (STALE/ABSENT) must not satisfy completion — otherwise
            // the disclosed footprint no longer corresponds to anything on disk.
            let has_current = state.research_artifacts.iter().any(|a| {
                self.artifact_freshness(&a.artifact_ref)
                    == crate::domain::task::EvidenceFreshness::Current
            });
            if !has_current {
                return Err(anyhow!(
                    "Completion interlock: research task requires at least one CURRENT research \
                     artifact (every recorded artifact is STALE or ABSENT — re-record against \
                     the current files before finishing)"
                ));
            }
            // "at least one uncertainty outcome" — a recorded uncertainty (a
            // disposition is impossible without a prior record), so this is the
            // real floor.
            if state.uncertainties.is_empty() {
                return Err(anyhow!(
                    "Completion interlock: research task requires at least one recorded \
                     uncertainty outcome (ctl uncertainty record --id {})",
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
            let mut payload = serde_json::json!({
                "evidence_id": evidence_id,
                "source": COMPLETION_AUDIT_SOURCE,
                "touched_files": touched,
                "result_file": note.unwrap_or(""),
                "accepted_at": now_iso8601(),
            });
            // Artifact binding: stamp the committed tree this audit validated.
            if let Some(tree) =
                crate::infrastructure::workspace::head_tree_hash(&self.project_root)?
            {
                payload["tree_hash"] = serde_json::json!(tree);
            }
            // Policy binding: stamp the policy in force when this audit was accepted.
            payload["policy_hash"] = serde_json::json!(self.current_policy_hash(&state));
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

    /// Canonical hash of the task's CURRENT policy (scope + risk triggers +
    /// required-gate *definitions*). Resolves each required gate id to its
    /// template's actual command + args, so a template change (not just a rename)
    /// invalidates prior evidence. Independent of git — always computable.
    fn current_policy_hash(&self, state: &crate::domain::task::TaskState) -> String {
        use crate::domain::policy::{compute_policy_hash, CanonicalGateDefinition};
        let read_scope: Vec<String> = state.read_scope.iter().cloned().collect();
        let write_allow: Vec<String> = state.write_allow.iter().cloned().collect();
        let write_deny: Vec<String> = state.write_deny.iter().cloned().collect();
        let risk_triggers: Vec<String> = state.risk_triggers.iter().cloned().collect();
        let gates: Vec<CanonicalGateDefinition> = state
            .gates
            .iter()
            .map(|g| match crate::infrastructure::gates::find_template(g) {
                Some(t) => CanonicalGateDefinition {
                    gate_id: g.clone(),
                    command: t.command.to_string(),
                    args: t.args.iter().map(|s| s.to_string()).collect(),
                },
                None => CanonicalGateDefinition {
                    gate_id: g.clone(),
                    command: String::new(),
                    args: Vec::new(),
                },
            })
            .collect();
        compute_policy_hash(
            &read_scope,
            &write_allow,
            &write_deny,
            &risk_triggers,
            &gates,
        )
    }

    pub fn record_gate(
        &self,
        task_id: &str,
        gate_id: &str,
        passed: bool,
        evidence: &str,
    ) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        let mut payload = serde_json::json!({
            "gate_id": gate_id,
            "passed": passed,
            "evidence": evidence,
            "checked_at": now_iso8601(),
        });
        // Artifact binding: stamp the committed tree this gate result was validated
        // against. Omitted (not null) outside a git repo so the schema stays valid;
        // unbound results cannot satisfy the finish-time interlock in a git repo.
        if let Some(tree) = crate::infrastructure::workspace::head_tree_hash(&self.project_root)? {
            payload["tree_hash"] = serde_json::json!(tree);
        }
        // Policy binding: stamp the policy in force when this gate ran (always
        // computable; independent of git).
        payload["policy_hash"] = serde_json::json!(self.current_policy_hash(&state));
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
        let evidence = if result.timed_out {
            // Reaching here means run_gate confirmed the process tree was reaped
            // (containment failure would have returned Err and recorded nothing).
            "exit=timeout termination=process_tree termination_result=confirmed".to_string()
        } else if result.passed {
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

    // ── BS-provenance V1: record-only brainstorm artifact provenance ──
    //
    // These emit canonical, task-scoped events binding brainstorm artifacts (by
    // path + SHA-256) to a task. They NEVER gate task creation or completion and
    // make no claim about thinking quality or review independence. Trust is pinned
    // at L0 content and critic independence at `unattested` by the reducer.

    /// Hash a brainstorm artifact, resolving its path against the project root.
    /// The file must exist: a reference is provenance only for content actually
    /// present at record time — a bare path on disk is never auto-provenance.
    fn hash_artifact(&self, path: &str) -> Result<String> {
        let resolved = self.project_root.join(path);
        if !resolved.is_file() {
            return Err(anyhow!("brainstorm artifact not found: {}", path));
        }
        hash_file(&resolved)
    }

    /// Record originator (divergence/convergence) artifacts for a brainstorm.
    pub fn record_brainstorm_artifacts(
        &self,
        task_id: &str,
        brainstorm_id: &str,
        divergence_path: &str,
        convergence_path: Option<&str>,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "brainstorm_id": brainstorm_id,
            "divergence_path": divergence_path,
            "divergence_hash": self.hash_artifact(divergence_path)?,
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(convergence) = convergence_path {
            payload["convergence_path"] = serde_json::json!(convergence);
            payload["convergence_hash"] = serde_json::json!(self.hash_artifact(convergence)?);
        }
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "brainstorm_artifact_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Attach a critic (challenge) artifact to a recorded brainstorm.
    pub fn attach_brainstorm_critic(
        &self,
        task_id: &str,
        brainstorm_id: &str,
        critic_path: &str,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "brainstorm_id": brainstorm_id,
            "critic_path": critic_path,
            "critic_hash": self.hash_artifact(critic_path)?,
            "critic_independence": crate::domain::task::CRITIC_INDEPENDENCE_UNATTESTED,
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "critic_artifact_attached", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record that the critic step was explicitly skipped, with a reason and the
    /// deciding actor (defaults to the recording actor when not supplied).
    pub fn skip_brainstorm_critic(
        &self,
        task_id: &str,
        brainstorm_id: &str,
        reason: &str,
        decided_by: Option<&str>,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "brainstorm_id": brainstorm_id,
            "skip_reason": reason,
            "decided_by": decided_by.unwrap_or(self.actor.as_str()),
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "brainstorm_skipped", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Build a fact-only provenance view, resolving artifact staleness against the
    /// current working tree. Returns None when the task has no recorded brainstorm.
    pub fn brainstorm_provenance_view(
        &self,
        state: &TaskState,
    ) -> Option<crate::domain::task::BrainstormProvenanceView> {
        use crate::domain::task::{ArtifactRef, ArtifactStatus, BrainstormProvenanceView};
        let reference = state.brainstorm_ref.as_ref()?;
        let status = |artifact: &ArtifactRef| -> ArtifactStatus {
            let resolved = self.project_root.join(&artifact.path);
            let present = resolved.is_file();
            // Missing → stale; present but hash drifted → stale; match → fresh.
            let stale = match present.then(|| hash_file(&resolved).ok()).flatten() {
                Some(current) => current != artifact.hash,
                None => true,
            };
            ArtifactStatus {
                path: artifact.path.clone(),
                present,
                stale,
                recorded_hash: artifact.hash.clone(),
            }
        };
        Some(BrainstormProvenanceView {
            id: reference.id.clone(),
            divergence: reference.divergence.as_ref().map(&status),
            convergence: reference.convergence.as_ref().map(&status),
            critic: reference.critic.as_ref().map(&status),
            critic_disposition: reference.critic_disposition.as_str().to_string(),
            critic_independence: reference.critic_independence.clone(),
            trust_level: reference.trust_level.clone(),
            source_run_id: reference.source_run_id.clone(),
            source_run_attested: false,
            recorded_by: reference.recorded_by.clone(),
            skip_reason: reference.skip_reason.clone(),
            skip_decided_by: reference.skip_decided_by.clone(),
        })
    }

    // ── PRD plan / validate / status (workflow-prd-to-tasks-v1) ──
    //
    // Closes the cognitive loop: a confirmed PRD's `## Tasks` section becomes
    // governed tasks in one call. No new event types — reuses create_task +
    // record_brainstorm_artifacts. Pure parsing lives in `application::prd`;
    // these methods add the IO-bound boundary/gate validation and orchestration.

    /// Validate a parsed PRD against format, boundary, gate, and overlap rules.
    /// Read-only — emits no events. Returns every problem found (does not stop
    /// at the first), so the user sees the full picture before planning.
    pub fn prd_validate(
        &self,
        doc: &crate::application::prd::PrdDocument,
    ) -> Result<crate::application::prd::PrdValidation> {
        use crate::application::prd::{overlap_problems, validate_format};

        let mut v = validate_format(doc);

        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );

        for task in &doc.tasks {
            // Boundary normalization catches path escape, protected paths,
            // symlinks/junctions/UNC. Check each path so one bad path doesn't
            // hide the rest.
            for path in &task.write_allow {
                if let Err(e) = normalizer.normalize(path) {
                    v.error(
                        Some(&task.id),
                        format!("write-allow path '{}': {}", path, e),
                    );
                }
            }
            for path in &task.read_scope {
                if let Err(e) = normalizer.normalize(path) {
                    v.error(Some(&task.id), format!("read-scope path '{}': {}", path, e));
                }
            }

            // Gate templates must be known.
            if let Err(e) = validate_gate_templates(&task.gates) {
                v.error(Some(&task.id), format!("{}", e));
            }
        }

        // Cross-task write-allow overlap — each colliding pair is an error.
        for (a, b, overlap) in overlap_problems(doc) {
            v.error(
                None,
                format!(
                    "tasks '{}' and '{}' have overlapping write-allow: {}",
                    a,
                    b,
                    overlap.join(", ")
                ),
            );
        }

        Ok(v)
    }

    /// Plan a confirmed PRD: validate, then create each task (gated) and record
    /// brainstorm provenance. A `draft` PRD is refused unless `dry_run`; a
    /// `superseded` PRD is always refused. In `dry_run`, nothing is persisted —
    /// the returned outcomes describe what would be created.
    pub fn prd_plan(
        &self,
        doc: &crate::application::prd::PrdDocument,
        alignment_path: Option<&str>,
        convergence_path: Option<&str>,
        dry_run: bool,
    ) -> Result<Vec<crate::application::prd::PrdPlanOutcome>> {
        use crate::application::prd::PrdStatus;

        // Status gate — superseded is never plannable, even as a dry run.
        if doc.status == PrdStatus::Superseded {
            return Err(anyhow!(
                "PRD status is 'superseded' — superseded by a later PRD; not plannable"
            ));
        }
        if !dry_run && doc.status != PrdStatus::Confirmed {
            return Err(anyhow!(
                "PRD status is '{}' — set it to 'confirmed' before planning, \
                 or run with --dry-run to preview",
                doc.status.as_str()
            ));
        }

        // Full validation — fail fast, create nothing on any error.
        let validation = self.prd_validate(doc)?;
        if !validation.ok() {
            let mut lines = String::from("PRD validation failed:\n");
            for p in validation.errors() {
                match &p.task_id {
                    Some(tid) => lines.push_str(&format!("  [{}] {}\n", tid, p.message)),
                    None => lines.push_str(&format!("  {}\n", p.message)),
                }
            }
            return Err(anyhow!("{}", lines.trim_end()));
        }

        let bs_id = crate::application::prd::brainstorm_id_for(&doc.title);
        let mut outcomes = Vec::with_capacity(doc.tasks.len());

        for task in &doc.tasks {
            // read-scope defaults to write-allow per the PRD convention.
            let read_scope: Vec<String> = if task.read_scope.is_empty() {
                task.write_allow.clone()
            } else {
                task.read_scope.clone()
            };

            if dry_run {
                outcomes.push(crate::application::prd::PrdPlanOutcome {
                    task_id: task.id.clone(),
                    objective: task.objective.clone(),
                    write_allow: task.write_allow.clone(),
                    gates: task.gates.clone(),
                    depends_on: task.depends_on.clone(),
                    created: false,
                    seq: None,
                    provenance_recorded: false,
                });
                continue;
            }

            let event = self.create_task(
                &task.id,
                CreateTaskInput {
                    objective: &task.objective,
                    read_scope: &read_scope,
                    write_allow: &task.write_allow,
                    write_deny: &[],
                    risk_triggers: &[],
                    gates: &task.gates,
                    depends_on: &task.depends_on,
                },
            )?;

            // Record brainstorm provenance when an alignment (divergence) path is
            // available. The PRD file is the convergence. Without a divergence
            // path, skip — record_brainstorm_artifacts requires one.
            let mut provenance_recorded = false;
            if let Some(divergence) = alignment_path {
                if self
                    .record_brainstorm_artifacts(
                        &task.id,
                        &bs_id,
                        divergence,
                        convergence_path,
                        None,
                    )
                    .is_ok()
                {
                    provenance_recorded = true;
                }
            }

            outcomes.push(crate::application::prd::PrdPlanOutcome {
                task_id: task.id.clone(),
                objective: task.objective.clone(),
                write_allow: task.write_allow.clone(),
                gates: task.gates.clone(),
                depends_on: task.depends_on.clone(),
                created: true,
                seq: Some(event.seq),
                provenance_recorded,
            });
        }

        Ok(outcomes)
    }

    /// Build the observable-loop status view for a parsed PRD: each task's
    /// existence, phase, and brainstorm provenance, plus a completion summary.
    /// Read-only — emits no events. Tasks not yet created show `exists: false`.
    pub fn prd_status_view(
        &self,
        doc: &crate::application::prd::PrdDocument,
    ) -> Result<crate::application::prd::PrdStatusView> {
        let mut rows = Vec::with_capacity(doc.tasks.len());
        let mut completed = 0;

        for task in &doc.tasks {
            match self.get_status(&task.id) {
                Ok(state) => {
                    let phase = format!("{:?}", state.phase).to_ascii_lowercase();
                    if state.phase == Phase::Completed {
                        completed += 1;
                    }
                    let provenance = self.brainstorm_provenance_view(&state);
                    rows.push(crate::application::prd::PrdTaskStatusRow {
                        id: task.id.clone(),
                        exists: true,
                        phase: Some(phase),
                        provenance,
                    });
                }
                Err(_) => {
                    // Task not created yet — it lives only in the PRD.
                    rows.push(crate::application::prd::PrdTaskStatusRow {
                        id: task.id.clone(),
                        exists: false,
                        phase: None,
                        provenance: None,
                    });
                }
            }
        }

        Ok(crate::application::prd::PrdStatusView {
            title: doc.title.clone(),
            status: doc.status,
            total: doc.tasks.len(),
            completed,
            rows,
        })
    }

    // ── Uncertainty Ledger V1: record-and-disclose unknowns ──
    //
    // record_uncertainty + record_uncertainty_disposition emit the two canonical
    // events; the view resolves evidence freshness against the working tree. Never
    // gates, never scores, never renders an aggregate verdict.

    /// Normalize an evidence path (reject `..`, absolute, UNC, symlink escape),
    /// then hash the file ctl-side. Returns `(repo-relative path, sha256)`. The
    /// caller never supplies the hash, so the binding is a faithful record of what
    /// was on disk — not a claim the caller could forge.
    fn hash_evidence(&self, path: &str) -> Result<(String, String)> {
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        let normalized = normalizer
            .normalize(path)
            .map_err(|e| anyhow!("invalid evidence path '{}': {}", path, e))?;
        let rel = path_to_payload_string(&normalized);
        let resolved = self.project_root.join(&rel);
        if !resolved.is_file() {
            return Err(anyhow!("evidence artifact not found: {}", rel));
        }
        let hash = hash_file(&resolved)?;
        Ok((rel, hash))
    }

    /// Record an open uncertainty (an unknown the task carries).
    pub fn record_uncertainty(
        &self,
        task_id: &str,
        uncertainty_id: &str,
        statement: &str,
        source: Option<&str>,
    ) -> Result<Event> {
        // Terminal-is-terminal: a completed/cancelled task's disclosed unknowns
        // must not change after the fact (mirrors research_artifact_recorded).
        // Enforced here at the command layer — the sole canonical-append path —
        // and deliberately NOT in the reducer, so committed pre-rule streams that
        // recorded an uncertainty post-terminal still replay byte-identically.
        let state = self.replay_task(task_id)?;
        if matches!(
            state.phase,
            crate::domain::task::Phase::Completed | crate::domain::task::Phase::Cancelled
        ) {
            return Err(anyhow!(
                "task is '{}'; a terminal task cannot record further uncertainties — \
                 the unknown set of a completed/cancelled task is fixed",
                state.phase.as_str()
            ));
        }
        let mut payload = serde_json::json!({
            "uncertainty_id": uncertainty_id,
            "statement": statement,
            "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
        });
        if let Some(source) = source {
            payload["source"] = serde_json::json!(source);
        }
        let event = self.build_event(task_id, "uncertainty_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record a terminal disposition for an uncertainty. `resolved` requires an
    /// evidence artifact (hashed ctl-side); `accepted_as_assumption` and
    /// `invalidated` must not carry evidence. The reducer enforces terminal-is-
    /// terminal and the disposition-specific evidence/reason rules.
    pub fn record_uncertainty_disposition(
        &self,
        task_id: &str,
        uncertainty_id: &str,
        disposition: &str,
        evidence_path: Option<&str>,
        evidence_ref: Option<&str>,
        reason: Option<&str>,
    ) -> Result<Event> {
        // Mutual exclusion is also enforced by the reducer + schema; reject early
        // here for a clear CLI message before any file hashing happens.
        if evidence_path.is_some() && evidence_ref.is_some() {
            return Err(anyhow!(
                "a 'resolved' must carry either --evidence-ref or --evidence (inline), never both"
            ));
        }
        // Oracle-resolution semantics: a `model` oracle is ADVISORY — never external
        // proof (EPISTEMIC_CONTROL §5.1: a resolve must distinguish "closed by
        // assertion" from "closed by external oracle"). A model-backed evidence may be
        // recorded and disclosed, but it must not *resolve* an uncertainty. This is
        // enforced here at the command layer — the only path that appends canonical
        // events — and deliberately NOT in the reducer, so committed pre-rule streams
        // that already resolved via a model oracle still replay byte-identically.
        if disposition == "resolved" {
            if let Some(eid) = evidence_ref {
                let state = self.replay_task(task_id)?;
                if let Some(ev) = state.evidences.iter().find(|e| e.id == eid) {
                    if ev.oracle_kind.is_advisory() {
                        return Err(anyhow!(
                            "evidence '{}' is a 'model' oracle (advisory, not external proof); \
                             a model oracle cannot resolve an uncertainty — record it as context, \
                             or resolve with a deterministic/test/runtime/human/external_authority \
                             oracle",
                            eid
                        ));
                    }
                }
            }
        }
        let mut payload = serde_json::json!({
            "uncertainty_id": uncertainty_id,
            "disposition": disposition,
            "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
        });
        if let Some(path) = evidence_path {
            let (rel, hash) = self.hash_evidence(path)?;
            payload["evidence_path"] = serde_json::json!(rel);
            payload["evidence_hash"] = serde_json::json!(hash);
        }
        if let Some(eref) = evidence_ref {
            payload["evidence_ref"] = serde_json::json!(eref);
        }
        if let Some(reason) = reason {
            payload["reason"] = serde_json::json!(reason);
        }
        let event = self.build_event(task_id, "uncertainty_disposition_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record a first-class, oracle-typed evidence object (Oracle V1). ctl computes
    /// the artifact hash from a normalized path (the caller never supplies it);
    /// `recorded_by` is the envelope actor, captured by the reducer — not a payload
    /// field. The evidence can later be referenced by a `resolved` disposition.
    pub fn record_evidence(
        &self,
        task_id: &str,
        evidence_id: &str,
        oracle_kind: &str,
        source_ref: Option<&str>,
        artifact_path: &str,
    ) -> Result<Event> {
        let (rel, hash) = self.hash_evidence(artifact_path)?;
        let mut payload = serde_json::json!({
            "evidence_id": evidence_id,
            "oracle_kind": oracle_kind,
            "artifact_path": rel,
            "artifact_hash": hash,
            "trust_level": crate::domain::task::EVIDENCE_TRUST_LEVEL,
        });
        if let Some(source) = source_ref {
            payload["source_ref"] = serde_json::json!(source);
        }
        let event = self.build_event(task_id, "evidence_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Record a subagent dispatch on the parent task (subagent-dispatch-record-v1).
    /// Record-and-disclose: `role`/`adapter` are host-supplied labels and each
    /// supplied artifact is sha256-hashed by ctl (`hash_evidence`); this records
    /// what the host said it dispatched — it never asserts what actually ran.
    /// Absent artifacts are simply not recorded.
    #[allow(clippy::too_many_arguments)]
    pub fn record_subagent_dispatch(
        &self,
        task_id: &str,
        role: &str,
        adapter: &str,
        parent_run: Option<&str>,
        instruction_artifact: Option<&str>,
        context_artifact: Option<&str>,
        output_artifact: Option<&str>,
    ) -> Result<Event> {
        let mut payload = serde_json::json!({
            "role": role,
            "adapter": adapter,
            "trust_level": crate::domain::task::BRAINSTORM_TRUST_LEVEL,
        });
        if let Some(run) = parent_run.filter(|s| !s.is_empty()) {
            payload["parent_run"] = serde_json::json!(run);
        }
        for (path_key, hash_key, artifact) in [
            ("instruction_path", "instruction_hash", instruction_artifact),
            ("context_path", "context_hash", context_artifact),
            ("output_path", "output_hash", output_artifact),
        ] {
            if let Some(p) = artifact.filter(|s| !s.is_empty()) {
                let (rel, hash) = self.hash_evidence(p)?;
                payload[path_key] = serde_json::json!(rel);
                payload[hash_key] = serde_json::json!(hash);
            }
        }
        let event = self.build_event(task_id, "subagent_dispatched", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Resolve evidence/artifact freshness against the working tree: ABSENT if
    /// the file is gone, STALE if its hash drifted, CURRENT if it matches. Never
    /// asserts the content is valid — only whether the file still matches what was
    /// recorded. Shared by evidence and research-artifact disclosure.
    fn artifact_freshness(
        &self,
        artifact: &crate::domain::task::ArtifactRef,
    ) -> crate::domain::task::EvidenceFreshness {
        use crate::domain::task::EvidenceFreshness;
        let resolved = self.project_root.join(&artifact.path);
        if !resolved.is_file() {
            EvidenceFreshness::Absent
        } else if hash_file(&resolved).ok().as_deref() == Some(artifact.hash.as_str()) {
            EvidenceFreshness::Current
        } else {
            EvidenceFreshness::Stale
        }
    }

    /// Build the fact-only view of one uncertainty (shared by the ledger view and
    /// the research-output view).
    fn uncertainty_item_view(
        &self,
        u: &crate::domain::task::Uncertainty,
    ) -> crate::domain::task::UncertaintyItemView {
        use crate::domain::task::{EvidenceView, UncertaintyItemView};
        let evidence = u.evidence_ref.as_ref().map(|ev| EvidenceView {
            path: ev.path.clone(),
            recorded_hash: ev.hash.clone(),
            freshness: self.artifact_freshness(ev),
            attested: false,
        });
        UncertaintyItemView {
            id: u.id.clone(),
            statement: u.statement.clone(),
            status: u.status.as_str().to_string(),
            source: u.source.clone(),
            evidence,
            evidence_id: u.evidence_id.clone(),
            oracle_kind: u.oracle_kind.map(|k| k.as_str().to_string()),
            advisory: u.oracle_kind.map(|k| k.is_advisory()).unwrap_or(false),
            reason: u.reason.clone(),
        }
    }

    /// Aggregate the task's recorded evidence into per-oracle-kind counts. Raw counts
    /// only; `model` is kept on its own `model_advisory` line so it can never be summed
    /// into "external proof".
    fn oracle_sources_view(&self, state: &TaskState) -> crate::domain::task::OracleSourcesView {
        use crate::domain::task::{OracleKind, OracleSourcesView};
        let mut view = OracleSourcesView::default();
        for e in &state.evidences {
            match e.oracle_kind {
                OracleKind::Deterministic => view.deterministic += 1,
                OracleKind::Test => view.test += 1,
                OracleKind::Runtime => view.runtime += 1,
                OracleKind::Human => view.human += 1,
                OracleKind::Model => view.model_advisory += 1,
                OracleKind::ExternalAuthority => view.external_authority += 1,
            }
        }
        view
    }

    /// Build a fact-only uncertainty-ledger view, resolving evidence freshness
    /// against the working tree. Returns None when the task records no uncertainty.
    pub fn uncertainty_ledger_view(
        &self,
        state: &TaskState,
    ) -> Option<crate::domain::task::UncertaintyLedgerView> {
        use crate::domain::task::{
            UncertaintyLedgerView, UncertaintyStatus, UNCERTAINTY_TRUST_LEVEL,
        };
        if state.uncertainties.is_empty() {
            return None;
        }
        let (mut open, mut accepted_as_assumption, mut resolved, mut invalidated) = (0, 0, 0, 0);
        for uncertainty in &state.uncertainties {
            match uncertainty.status {
                UncertaintyStatus::Open => open += 1,
                UncertaintyStatus::AcceptedAsAssumption => accepted_as_assumption += 1,
                UncertaintyStatus::Resolved => resolved += 1,
                UncertaintyStatus::Invalidated => invalidated += 1,
            }
        }
        let items = state
            .uncertainties
            .iter()
            .map(|u| self.uncertainty_item_view(u))
            .collect();
        Some(UncertaintyLedgerView {
            open,
            accepted_as_assumption,
            resolved,
            invalidated,
            trust_level: UNCERTAINTY_TRUST_LEVEL.to_string(),
            oracle_sources: self.oracle_sources_view(state),
            items,
        })
    }

    /// Record a tracked research artifact (Research/Spike V1). ctl computes the
    /// hash from a normalized path; the caller never supplies it.
    pub fn record_research_artifact(
        &self,
        task_id: &str,
        artifact_path: &str,
        artifact_kind: &str,
        source_run_id: Option<&str>,
    ) -> Result<Event> {
        // Pre-check against current state for a clear CLI message before any file
        // hashing. The reducer re-asserts every one of these invariants so they
        // also hold on replay — this layer is for ergonomics, not enforcement.
        let state = self.replay_task(task_id)?;
        if state.task_kind != TaskKind::Research {
            return Err(anyhow!(
                "only a research task may record research artifacts; task '{}' is an \
                 implementation task",
                task_id
            ));
        }
        if matches!(state.phase, Phase::Completed | Phase::Cancelled) {
            return Err(anyhow!(
                "task '{}' is {}; a terminal task cannot record further research artifacts",
                task_id,
                state.phase.as_str()
            ));
        }
        let (rel, hash) = self.hash_evidence(artifact_path)?;
        // Scope binding: the artifact must sit inside the task's write_allow (and
        // outside write_deny) — the same boundary the write gate enforces.
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        if !file_in_write_scope(&normalizer, &rel, &state.write_allow, &state.write_deny)? {
            return Err(anyhow!(
                "research artifact '{}' is outside the task's write_allow (or within write_deny)",
                rel
            ));
        }
        let mut payload = serde_json::json!({
            "artifact_path": rel,
            "artifact_hash": hash,
            "artifact_kind": artifact_kind,
            "trust_level": crate::domain::task::RESEARCH_TRUST_LEVEL,
        });
        if let Some(run) = source_run_id {
            payload["source_run_id"] = serde_json::json!(run);
        }
        let event = self.build_event(task_id, "research_artifact_recorded", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Build a fact-only research-output view for a research task; None for
    /// implementation tasks. Raw per-status counts, artifacts with freshness, and
    /// uncertainty items each tagged `recorded_after_start` (derived from the
    /// single `task_started` seq). Deliberately NO "discovered" scalar and no verdict.
    pub fn research_output_view(
        &self,
        task_id: &str,
    ) -> Result<Option<crate::domain::task::ResearchOutputView>> {
        use crate::domain::task::{
            ResearchArtifactView, ResearchOutputView, ResearchUncertaintyView, UncertaintyStatus,
            RESEARCH_TRUST_LEVEL,
        };
        let state = self.replay_task(task_id)?;
        if state.task_kind != TaskKind::Research {
            return Ok(None);
        }
        let events = self.store.read_for_task(task_id)?;
        // A finishable task has exactly one task_started (reopen emits
        // task_reopened, not a second start). Uncertainties recorded after it are
        // tagged "recorded after start" — a per-item fact, never a rankable count.
        let start_seq = events
            .iter()
            .find(|e| e.event_type == "task_started")
            .map(|e| e.seq);
        let mut recorded_seq: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();
        for e in &events {
            if e.event_type == "uncertainty_recorded" {
                if let Some(id) = e.payload.get("uncertainty_id").and_then(|v| v.as_str()) {
                    recorded_seq.insert(id.to_string(), e.seq);
                }
            }
        }
        let (mut accepted_as_assumptions, mut resolved, mut invalidated) = (0, 0, 0);
        for u in &state.uncertainties {
            match u.status {
                UncertaintyStatus::Resolved => resolved += 1,
                UncertaintyStatus::AcceptedAsAssumption => accepted_as_assumptions += 1,
                UncertaintyStatus::Invalidated => invalidated += 1,
                UncertaintyStatus::Open => {}
            }
        }
        let artifacts = state
            .research_artifacts
            .iter()
            .map(|a| ResearchArtifactView {
                path: a.artifact_ref.path.clone(),
                recorded_hash: a.artifact_ref.hash.clone(),
                kind: a.kind.as_str().to_string(),
                freshness: self.artifact_freshness(&a.artifact_ref),
                source_run_id: a.source_run_id.clone(),
                source_run_attested: false,
            })
            .collect();
        let uncertainties = state
            .uncertainties
            .iter()
            .map(|u| {
                let recorded_after_start = match (start_seq, recorded_seq.get(&u.id)) {
                    (Some(start), Some(&seq)) => seq > start,
                    _ => false,
                };
                ResearchUncertaintyView {
                    item: self.uncertainty_item_view(u),
                    recorded_after_start,
                }
            })
            .collect();
        Ok(Some(ResearchOutputView {
            artifacts_recorded: state.research_artifacts.len(),
            uncertainties_opened: state.uncertainties.len(),
            resolved_with_evidence: resolved,
            accepted_as_assumptions,
            invalidated,
            trust_level: RESEARCH_TRUST_LEVEL.to_string(),
            artifacts,
            uncertainties,
        }))
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

    /// Recommend the next task to advance. Deterministic: among Ready tasks whose
    /// dependencies are all Completed and whose write scope does not overlap any
    /// active in_progress task, pick the lowest drift score (ties broken by task id
    /// for stable output). Falls back to the lowest-drift Planning task when no
    /// Ready task is actionable. Read-only — emits no events.
    pub fn next_task(&self) -> Result<NextTaskRecommendation> {
        use crate::application::schedule::detect_write_scope_overlap;
        use std::collections::HashMap;

        let board = self.generate_board()?;
        let empty = vec![];
        let tasks = board["tasks"].as_array().unwrap_or(&empty);

        // Phase lookup for dependency satisfaction (Completed satisfies).
        let phase_by_id: HashMap<String, String> = tasks
            .iter()
            .filter_map(|t| {
                Some(t["task_id"].as_str()?.to_string()).zip(t["phase"].as_str().map(String::from))
            })
            .collect();

        // Active in_progress write scopes (for overlap detection).
        let active_scopes: Vec<BTreeSet<String>> = tasks
            .iter()
            .filter(|t| {
                t["phase"].as_str() == Some("in_progress")
                    && !t["archived"].as_bool().unwrap_or(false)
            })
            .filter_map(|t| {
                let set: BTreeSet<String> = t["write_scope"]
                    .as_array()?
                    .iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect();
                Some(set)
            })
            .collect();

        let deps_satisfied = |task: &serde_json::Value| -> bool {
            match task["depends_on"].as_array() {
                None => true,
                Some(deps) => deps.iter().all(|d| {
                    let dep_id = d.as_str().unwrap_or("");
                    phase_by_id
                        .get(dep_id)
                        .map(|p| p == "completed")
                        .unwrap_or(false)
                }),
            }
        };

        let no_scope_conflict = |task: &serde_json::Value| -> bool {
            let scopes: BTreeSet<String> = task["write_scope"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            active_scopes
                .iter()
                .all(|active| detect_write_scope_overlap(&scopes, active).is_empty())
        };

        let rank = |a: &serde_json::Value, b: &serde_json::Value| {
            let sa = a["drift_score"].as_i64().unwrap_or(0);
            let sb = b["drift_score"].as_i64().unwrap_or(0);
            sa.cmp(&sb).then_with(|| {
                a["task_id"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["task_id"].as_str().unwrap_or(""))
            })
        };

        let is_ready = |t: &serde_json::Value| {
            t["phase"].as_str() == Some("ready")
                && !t["held"].as_bool().unwrap_or(false)
                && !t["archived"].as_bool().unwrap_or(false)
                && deps_satisfied(t)
                && no_scope_conflict(t)
        };
        let is_planning = |t: &serde_json::Value| {
            t["phase"].as_str() == Some("planning")
                && !t["held"].as_bool().unwrap_or(false)
                && !t["archived"].as_bool().unwrap_or(false)
        };

        let mut ready: Vec<&serde_json::Value> = tasks.iter().filter(|t| is_ready(t)).collect();
        ready.sort_by(|a, b| rank(a, b));

        let mut planning: Vec<&serde_json::Value> =
            tasks.iter().filter(|t| is_planning(t)).collect();
        planning.sort_by(|a, b| rank(a, b));

        let ready_count = ready.len();
        let planning_count = planning.len();

        if let Some(best) = ready.first() {
            let score = best["drift_score"].as_i64().unwrap_or(0);
            return Ok(NextTaskRecommendation {
                action: "start",
                task_id: best["task_id"].as_str().map(String::from),
                objective: best["objective"].as_str().map(String::from),
                rationale: format!(
                    "ready, dependencies satisfied, lowest drift (score {score}), \
                 no active scope conflict"
                ),
                ready_candidates: ready_count,
                planning_candidates: planning_count,
            });
        }

        if let Some(best) = planning.first() {
            return Ok(NextTaskRecommendation {
                action: "ready",
                task_id: best["task_id"].as_str().map(String::from),
                objective: best["objective"].as_str().map(String::from),
                rationale: "no actionable ready task; lowest-drift planning task".to_string(),
                ready_candidates: ready_count,
                planning_candidates: planning_count,
            });
        }

        Ok(NextTaskRecommendation {
            action: "none",
            task_id: None,
            objective: None,
            rationale: "no actionable tasks (all completed, archived, held, or \
            blocked by unsatisfied dependencies)"
                .to_string(),
            ready_candidates: ready_count,
            planning_candidates: planning_count,
        })
    }

    // ── Spec fact store (knowledge-accumulation-v1) ──
    //
    // Atomic verified facts captured during conversations, persisted to
    // `.ctl/facts.jsonl` (append-only evidence, NOT canonical events). Two
    // tiers: raw facts (this store) + curated spec markdown (promote). The
    // digest is injected into `ctl hook context` so every subsequent session
    // sees accumulated knowledge. Record-and-disclose — never gates.

    /// Append one verified fact to the knowledge base. ctl assigns the fact id,
    /// stamps the timestamp, and records the actor.
    pub fn spec_fact_add(
        &self,
        statement: &str,
        source: &str,
        category: Option<&str>,
    ) -> Result<crate::application::spec::Fact> {
        if statement.trim().is_empty() {
            return Err(anyhow!("Fact statement must not be empty"));
        }
        if source.trim().is_empty() {
            return Err(anyhow!(
                "Fact source must not be empty — a fact without provenance is \
                 an opinion, not knowledge"
            ));
        }
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        let fact = crate::application::spec::Fact {
            fact_id: crate::application::spec::next_fact_id(&facts),
            statement: statement.trim().to_string(),
            source: source.trim().to_string(),
            category: category.map(|c| c.trim().to_string()),
            recorded_at: now_iso8601(),
            recorded_by: self.actor.clone(),
        };
        let path = crate::application::spec::facts_path(&self.project_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(&fact)?;
        // Open in append mode + fsync, mirroring the telemetry append contract.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        // Self-heal a missing trailing newline (mirrors append_jsonl_line).
        if file.metadata()?.len() > 0 {
            let mut existing = String::new();
            use std::io::Read;
            let mut reader = std::fs::File::open(&path)?;
            reader.read_to_string(&mut existing)?;
            if !existing.ends_with('\n') {
                file.write_all(b"\n")?;
            }
        }
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        if !self.dry_run {
            // No projection to rebuild — facts are evidence, not task state.
        }
        Ok(fact)
    }

    /// List facts, optionally filtered by category and/or a search term.
    pub fn spec_fact_list(
        &self,
        category: Option<&str>,
        search: Option<&str>,
    ) -> Result<Vec<crate::application::spec::Fact>> {
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        let filtered = crate::application::spec::filter_facts(&facts, category, search);
        Ok(filtered.into_iter().cloned().collect())
    }

    /// Promote a fact into a curated spec markdown file by appending a
    /// formatted block. The target is relative to `.ctl/spec/` (e.g.
    /// `backend/infrastructure-layer.md`).
    pub fn spec_fact_promote(&self, fact_id: &str, target: &str) -> Result<std::path::PathBuf> {
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        let fact = facts
            .iter()
            .find(|f| f.fact_id == fact_id)
            .ok_or_else(|| anyhow!("Fact '{}' not found in the knowledge base", fact_id))?;

        let spec_root = self.project_root.join(".ctl").join("spec");
        let target_path = spec_root.join(target);

        // Resolve and boundary-check: the target must stay inside .ctl/spec/.
        let resolved = target_path.canonicalize().map_err(|_| {
            anyhow!(
                "Cannot resolve target spec file '{}.md' — does the file exist \
                 under .ctl/spec/?",
                target.trim_end_matches(".md")
            )
        })?;
        if !resolved.starts_with(spec_root.canonicalize().unwrap_or(spec_root.clone())) {
            return Err(anyhow!(
                "Promote target must be inside .ctl/spec/ — got '{}'",
                target
            ));
        }

        let block = crate::application::spec::format_fact_for_promote(fact);
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target_path)?;
        file.write_all(block.as_bytes())?;
        Ok(target_path)
    }

    /// Build a compact digest of the fact store for context injection.
    pub fn spec_facts_digest(&self) -> Result<crate::application::spec::FactsDigest> {
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        Ok(crate::application::spec::facts_digest(&facts, 5))
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
    fn read_handoff_capture(&self, task_id: &str) -> Result<Option<serde_json::Value>> {
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

    /// Read-only GO / NO-GO safety evaluation for an unattended (ralph)
    /// supervisor loop: is it still safe to continue without a human? Composes
    /// this session's guards — task hold/terminality, cross-ledger consistency,
    /// shared-`.git` locks, and drift (next-action) — into one verdict. Appends
    /// nothing; spawns nothing. The supervisor halts the moment this returns a
    /// NO-GO; it is the envelope around an external run, never the executor.
    pub fn ralph_safety_check(&self, task_id: &str) -> Result<RalphVerdict> {
        let mut blockers = Vec::new();

        let state = self.replay_task(task_id)?;
        if state.is_held {
            blockers.push("task is held — resolve the hold before resuming".to_string());
        }
        if matches!(state.phase, Phase::Completed | Phase::Cancelled) {
            blockers.push(format!(
                "task is terminal ({:?}) — nothing left to supervise",
                state.phase
            ));
        }

        let cross_ledger = self.cross_ledger_findings()?;
        if !cross_ledger.is_empty() {
            blockers.push(format!(
                "{} cross-ledger inconsistency(ies) — run `ctl repair --cross-ledger`",
                cross_ledger.len()
            ));
        }

        let risk = crate::infrastructure::workspace::scan_shared_git_risk(&self.project_root);
        if risk.any() {
            blockers.push(format!(
                "shared .git lock present: {}",
                risk.descriptions().join("; ")
            ));
        }

        // Drift: anything other than Pass means a human decision is due.
        let na = self.next_action(task_id)?;
        if !matches!(na.action, crate::domain::drift::NextActionKind::Pass) {
            blockers.push(format!(
                "drift next-action is {} ({})",
                na.action.as_str(),
                na.rationale
            ));
        }

        Ok(RalphVerdict {
            go: blockers.is_empty(),
            blockers,
        })
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
        use crate::domain::run::RunPhase;
        use crate::domain::task::Phase;

        let mut results = Vec::new();
        let mut score: i32 = 100;
        let mut task_count = 0u32;
        let mut replay_errors = 0u32;
        let mut inconsistencies = 0u32;

        // ── Task ledgers ──
        // Map of replayed task phases, used for the cross-ledger checks below.
        let mut task_phases: std::collections::HashMap<String, Phase> =
            std::collections::HashMap::new();
        match self.store.read_all() {
            Ok(events) => {
                results.push(format!("events.jsonl: OK ({} events)", events.len()));
                let task_ids = self.store.task_ids()?;
                task_count = task_ids.len() as u32;
                for tid in &task_ids {
                    match self.replay_task(tid) {
                        Ok(state) => {
                            results.push(format!(
                                "Task '{}': {:?} (seq {})",
                                tid, state.phase, state.last_seq
                            ));
                            task_phases.insert(tid.clone(), state.phase);
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

        // ── Run ledgers + cross-ledger consistency ──
        //
        // Concurrent task/run orchestration is EXPERIMENTAL: a task transition and
        // its run-ledger counterpart are two separate appends (each a single-writer
        // append, but with no transaction spanning both). A crash between them can
        // leave the ledgers disagreeing. ctl never auto-repairs this — doctor only
        // surfaces the facts and the manual recovery step.
        let run_store = self.run_store()?;
        let run_ids = run_store.run_ids()?;
        if !run_ids.is_empty() {
            results.push(String::new());
            results.push(format!("Runs: {} total (orchestration)", run_ids.len()));
            for rid in &run_ids {
                match self.replay_run(rid) {
                    Ok(run) => {
                        results.push(format!(
                            "Run '{}': {:?} (task '{}', seq {})",
                            rid, run.phase, run.task_id, run.last_seq
                        ));
                        // Cross-ledger: the run names a task with no ledger.
                        if !run.task_id.is_empty() && !task_phases.contains_key(&run.task_id) {
                            inconsistencies += 1;
                            score -= 10;
                            results.push(format!(
                                "  INCONSISTENCY: run '{}' references task '{}', which has no \
                                 ledger. Recover by replaying the run's events to confirm intent, \
                                 then cancel the orphan run.",
                                rid, run.task_id
                            ));
                        }
                        // Cross-ledger: a live run whose task is already terminal —
                        // the classic non-atomic window (task closed, run not).
                        if run.phase == RunPhase::Running {
                            if let Some(phase) = task_phases.get(&run.task_id) {
                                if matches!(phase, Phase::Completed | Phase::Cancelled) {
                                    inconsistencies += 1;
                                    score -= 10;
                                    results.push(format!(
                                        "  INCONSISTENCY: run '{}' is Running but its task '{}' is \
                                         {:?}. Recover by aborting the run (ctl run abort).",
                                        rid, run.task_id, phase
                                    ));
                                }
                            }
                            // Worktree: a Running run must have its worktree on disk.
                            if let Some(wt) = &run.worktree_path {
                                if !std::path::Path::new(wt).exists() {
                                    inconsistencies += 1;
                                    score -= 10;
                                    results.push(format!(
                                        "  INCONSISTENCY: run '{}' is Running but its worktree '{}' \
                                         is missing. Recover by aborting the run (ctl run abort).",
                                        rid, wt
                                    ));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        replay_errors += 1;
                        score -= 15;
                        results.push(format!("Run '{}': REPLAY ERROR: {}", rid, e));
                    }
                }
            }
        }

        // ── Shared-.git hazards (M6 shared-state hardening) ──
        // git worktrees share one object store + packed-refs, so a stuck lock
        // blocks or corrupts ref/index operations across every worktree. Facts
        // only — doctor never removes a lock (it may be a legitimately in-flight
        // git op); it flags the hazard and points at recovery.
        let shared_git = crate::infrastructure::workspace::scan_shared_git_risk(&self.project_root);
        if shared_git.any() {
            results.push(String::new());
            for d in shared_git.descriptions() {
                score -= 5;
                results.push(format!("WARNING: shared .git — {}", d));
            }
            results.push(
                "Remove a stale lock only when no git process is running; if an active run \
                 holds it, recover via `ctl run recover`."
                    .to_string(),
            );
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
        results.push(format!(
            "Runs: {} total, {} cross-ledger inconsistencies",
            run_ids.len(),
            inconsistencies
        ));
        if replay_errors > 0 {
            results.push(
                "A REPLAY ERROR may be a torn trailing record — run `ctl repair --task <id>` \
                 (or --run <id>) to inspect and truncate it."
                    .to_string(),
            );
        }

        Ok(results)
    }

    // ── Ledger torn-tail repair (explicit, opt-in) ──

    /// Detect (and, when `apply`, truncate) a torn trailing record on one task's
    /// event ledger. Read-only when `apply` is false.
    pub fn repair_task_ledger(
        &self,
        task_id: &str,
        apply: bool,
    ) -> Result<crate::infrastructure::store::TailRepair> {
        self.store.repair_task_ledger(task_id, apply)
    }

    /// Detect (and, when `apply`, truncate) a torn trailing record on one run's
    /// event ledger. Read-only when `apply` is false.
    pub fn repair_run_ledger(
        &self,
        run_id: &str,
        apply: bool,
    ) -> Result<crate::infrastructure::store::TailRepair> {
        self.run_store()?.repair_run_ledger(run_id, apply)
    }

    /// Scan every task and run ledger for a torn trailing record, repairing when
    /// `apply`. Returns `(label, outcome)` per ledger.
    pub fn repair_all_ledgers(
        &self,
        apply: bool,
    ) -> Result<Vec<(String, crate::infrastructure::store::TailRepair)>> {
        let mut out = Vec::new();
        for tid in self.store.task_ids()? {
            out.push((
                format!("task {tid}"),
                self.store.repair_task_ledger(&tid, apply)?,
            ));
        }
        let rs = self.run_store()?;
        for rid in rs.run_ids()? {
            out.push((format!("run {rid}"), rs.repair_run_ledger(&rid, apply)?));
        }
        Ok(out)
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

    pub fn adapter_capabilities(&self, adapter_name: &str) -> Result<serde_json::Value> {
        let adapter = adapter_for(adapter_name)?;
        Ok(adapter.capabilities())
    }

    /// adapter-doctor-v1: summarize every registered executor adapter.
    pub fn adapter_list(&self) -> Vec<crate::adapters::AdapterSummary> {
        crate::adapters::adapter_list()
    }

    /// adapter-doctor-v1: diagnose one adapter — the Rust `ExecutorAdapter`
    /// contract clauses PLUS host platform integration (control-guard skill,
    /// managed-protocol drift, plugin/hook files, Bun tests). `verify` opts into
    /// live checks (the opencode Bun suite); without it they stay NOT_TRACKED. An
    /// unknown name yields a single failing contract check (never an Err), so the
    /// caller reports the failure uniformly.
    pub fn adapter_status(
        &self,
        adapter_name: &str,
        verify: bool,
    ) -> crate::adapters::AdapterDiagnostic {
        adapter_status_diagnostic(&self.project_root, adapter_name, verify)
    }

    /// adapter-doctor-v1: diagnose every registered adapter. Factual counts only
    /// (no composite health score).
    pub fn adapter_doctor(&self, verify: bool) -> crate::adapters::AdapterDoctorReport {
        adapter_doctor_report(&self.project_root, verify)
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
    fn run_store(&self) -> Result<RunEventStore> {
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
    fn build_run_event(
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
    fn append_run_event(&self, run_id: &str, event: Event) -> Result<Event> {
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
    fn append_run_event_locked(
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
    fn check_run_scope_overlap(&self, run_id: &str, write_allow: &BTreeSet<String>) -> Result<()> {
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
    fn terminate_run(
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
    fn expire_run_lease_at(
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

    /// Classify every cross-ledger inconsistency (read-only — replays aggregates
    /// and stats the filesystem, never appends).
    ///
    /// One finding per run, chosen by severity: orphan (no task) > stranded
    /// (terminal task) > missing-worktree, plus partial-start for a Queued run
    /// still holding a lease, and orphaned-worktree for a terminal run whose
    /// isolation dir lingers. Reuses the same "active"/"terminal" notions as
    /// `doctor` and `run recover`, so the three views never disagree. A run whose
    /// own ledger is torn is skipped here — that is `ctl repair --run` territory,
    /// not cross-ledger drift.
    pub fn cross_ledger_findings(&self) -> Result<Vec<CrossLedgerFinding>> {
        use crate::domain::run::RunPhase;
        use crate::domain::task::Phase;

        // Task phase map; a missing entry means the task has no ledger.
        let mut task_phases: std::collections::HashMap<String, Phase> =
            std::collections::HashMap::new();
        for tid in self.store.task_ids()? {
            if let Ok(state) = self.replay_task(&tid) {
                task_phases.insert(tid, state.phase);
            }
        }

        let store = self.run_store()?;
        let mut findings = Vec::new();
        for run_id in store.run_ids()? {
            let run = match self.replay_run(&run_id) {
                Ok(r) => r,
                Err(_) => continue, // torn run ledger — not a cross-ledger concern
            };
            let worktree_on_disk = run
                .worktree_path
                .as_ref()
                .map(|w| Path::new(w).exists())
                .unwrap_or(false);
            let task_id = (!run.task_id.is_empty()).then(|| run.task_id.clone());

            match run.phase {
                RunPhase::Running | RunPhase::Queued => {
                    let no_task =
                        !run.task_id.is_empty() && !task_phases.contains_key(&run.task_id);
                    let task_terminal = matches!(
                        task_phases.get(&run.task_id),
                        Some(Phase::Completed) | Some(Phase::Cancelled)
                    );
                    let (kind, detail) = if no_task {
                        (
                            CrossLedgerKind::OrphanRun,
                            format!(
                                "run '{}' is {:?} but references task '{}', which has no ledger",
                                run_id, run.phase, run.task_id
                            ),
                        )
                    } else if task_terminal {
                        (
                            CrossLedgerKind::StrandedRun,
                            format!(
                                "run '{}' is {:?} but its task '{}' is terminal",
                                run_id, run.phase, run.task_id
                            ),
                        )
                    } else if run.phase == RunPhase::Running && !worktree_on_disk {
                        (
                            CrossLedgerKind::MissingWorktreeRun,
                            format!(
                                "run '{}' is Running but its isolated worktree is missing",
                                run_id
                            ),
                        )
                    } else if run.phase == RunPhase::Queued && run.lease.is_some() {
                        (
                            CrossLedgerKind::PartialStartRun,
                            format!(
                                "run '{}' is Queued holding a lease but never started (crash mid-start)",
                                run_id
                            ),
                        )
                    } else {
                        continue; // consistent (active run, live task, worktree present)
                    };
                    findings.push(CrossLedgerFinding {
                        repair: RepairAction::AbortRun {
                            reason: format!("cross-ledger repair: {}", kind.as_str()),
                        },
                        kind,
                        run_id: run_id.clone(),
                        task_id,
                        detail,
                    });
                }
                RunPhase::Completed | RunPhase::Failed | RunPhase::Aborted => {
                    if worktree_on_disk {
                        let path = run.worktree_path.clone().unwrap_or_default();
                        findings.push(CrossLedgerFinding {
                            kind: CrossLedgerKind::OrphanedWorktree,
                            run_id: run_id.clone(),
                            task_id,
                            detail: format!(
                                "run '{}' is {:?} but its worktree dir still exists at {}",
                                run_id, run.phase, path
                            ),
                            repair: RepairAction::RemoveWorktree { path },
                        });
                    }
                }
            }
        }
        Ok(findings)
    }

    /// Apply one cross-ledger repair. Run aborts append `run_aborted`
    /// (+`lease_revoked`) — the canonical repair evidence; worktree removal is
    /// fs-only (the run ledger is already terminal and correct). Errors are
    /// captured in the outcome, not propagated, so a batch apply continues past a
    /// single failure.
    pub fn apply_cross_ledger_repair(&self, finding: &CrossLedgerFinding) -> RepairOutcome {
        let mut outcome = RepairOutcome {
            run_id: finding.run_id.clone(),
            kind: finding.kind,
            applied: false,
            result: String::new(),
        };
        outcome.result = match &finding.repair {
            RepairAction::AbortRun { reason } => match self.abort_run(&finding.run_id, reason) {
                Ok(ev) => {
                    outcome.applied = true;
                    format!("aborted run (run_aborted at seq {})", ev.seq)
                }
                Err(e) => format!("abort failed: {e}"),
            },
            RepairAction::RemoveWorktree { path } => {
                match crate::infrastructure::workspace::cleanup_worktree(
                    &self.project_root,
                    Path::new(path),
                ) {
                    Ok(()) => {
                        outcome.applied = true;
                        format!("removed leftover worktree {path}")
                    }
                    Err(e) => format!("worktree removal failed: {e}"),
                }
            }
        };
        outcome
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

    /// Record `approval_expired` for any granted approval whose TTL has elapsed.
    ///
    /// Mirrors `expire_stale_leases`. Without this, an expired approval was only
    /// ever *read* as invalid at the apply gate (lazy invalidation) and the ledger
    /// never recorded the expiry transition — the `approval_expired` event and the
    /// `ApprovalStatus::Expired` state were reachable only via replay/tests.
    /// Idempotent: once expired, `is_granted()` is false, so a subsequent call
    /// skips it (no duplicate event). The schema for `approval_expired` permits
    /// only `request_id`, so no `reason` field is emitted.
    fn expire_stale_approvals(&self, task_id: &str) -> Result<()> {
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
fn validate_handoff_capture(value: &serde_json::Value, task_id: &str) -> Result<()> {
    if value.get("schema").and_then(|v| v.as_str()) != Some("control.handoff.capture.v1") {
        return Err(anyhow!(
            "handoff capture schema must be control.handoff.capture.v1"
        ));
    }
    if value.get("task_id").and_then(|v| v.as_str()) != Some(task_id) {
        return Err(anyhow!(
            "handoff capture task_id does not match '{task_id}'"
        ));
    }
    if value.get("source").and_then(|v| v.as_str()) != Some("agent_or_human_supplied") {
        return Err(anyhow!(
            "handoff capture source must be agent_or_human_supplied"
        ));
    }
    if value
        .get("next_safe_action")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err(anyhow!(
            "handoff capture requires a non-empty next_safe_action"
        ));
    }
    Ok(())
}

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

/// Decide whether a worktree-relative `path` is writable under a task boundary:
/// inside some `write_allow` scope and not shadowed by a `write_deny` scope,
/// after normalization. Shared by `workspace_apply` (which errors on the first
/// out-of-scope file) and `merge_candidate` (which collects them).
fn file_in_write_scope(
    normalizer: &crate::infrastructure::boundary::normalizer::PathNormalizer,
    path: &str,
    write_allow: &std::collections::BTreeSet<String>,
    write_deny: &std::collections::BTreeSet<String>,
) -> Result<bool> {
    let normalized = normalizer
        .normalize(path)
        .map_err(|e| anyhow!("Invalid path '{}': {}", path, e))?;
    let normalized_str = normalized.to_string_lossy().replace('\\', "/");
    let matches = |scope: &String| {
        crate::domain::task::path_within_scope(&normalized_str, &scope.replace('\\', "/"))
    };
    Ok(write_allow.iter().any(matches) && !write_deny.iter().any(matches))
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

// ── adapter-doctor-v1: platform-integration diagnostics ─────────────────────
//
// The Rust `ExecutorAdapter` contract clauses are checked purely in
// `crate::adapters`. Here we add the host-integration checks that need the
// filesystem and the managed-protocol drift checker, then fold both into one
// `AdapterDiagnostic`. We report FACTS — presence checks are PASS/FAIL/WARN/
// UNKNOWN; live verification (the opencode Bun suite) is NOT_TRACKED unless
// `--verify`, and UNKNOWN when the tool is unavailable — never a silent PASS.

/// Diagnose a single adapter: contract clauses (pure) + platform integration.
pub fn adapter_status_diagnostic(
    project_root: &std::path::Path,
    adapter_name: &str,
    verify: bool,
) -> crate::adapters::AdapterDiagnostic {
    let resolved = adapter_for(adapter_name).is_ok();
    let mut checks = crate::adapters::adapter_contract_checks(adapter_name);
    checks.extend(adapter_platform_checks(project_root, adapter_name, verify));
    crate::adapters::AdapterDiagnostic::new(adapter_name, resolved, checks)
}

/// Diagnose every registered adapter (registry order), plus the Claude Code hook
/// platform when this project wires it. Factual report only.
pub fn adapter_doctor_report(
    project_root: &std::path::Path,
    verify: bool,
) -> crate::adapters::AdapterDoctorReport {
    let mut adapters: Vec<crate::adapters::AdapterDiagnostic> =
        crate::adapters::supported_adapters()
            .iter()
            .map(|name| adapter_status_diagnostic(project_root, name, verify))
            .collect();
    // Claude Code is a hook/skills platform, not an executor adapter
    // (`adapter: None`), so it is absent from `supported_adapters()` and the loop
    // above never reaches it — yet it carries the most runtime wiring (gate +
    // context hooks + a PreToolUse matcher). When this project wires Claude
    // (`.claude/` present) surface that wiring as a non-adapter diagnostic so the
    // runtime gaps the drift tests (skill TEXT only) cannot observe (D2) become
    // visible. Absent `.claude/` → Claude is not this project's platform, so the
    // report is left exactly as before.
    if project_root.join(".claude").is_dir() {
        adapters.push(claude_platform_diagnostic(project_root, verify));
    }
    crate::adapters::AdapterDoctorReport::new(adapters)
}

/// Presence check: PASS if `rel` exists under `root`, else the `missing` status.
fn presence_check(
    root: &std::path::Path,
    name: &str,
    rel: &str,
    missing: crate::adapters::CheckStatus,
) -> crate::adapters::AdapterCheck {
    use crate::adapters::{AdapterCheck, CheckStatus};
    if root.join(rel).exists() {
        AdapterCheck::new(name, CheckStatus::Pass, format!("present: {rel}"))
    } else {
        AdapterCheck::new(name, missing, format!("missing: {rel}"))
    }
}

/// The PreToolUse matcher the Claude gate must register — exactly the mutating
/// tools `ctl-gate.py` claims to govern. (Bash fails open and Task is unmatched
/// by design; those are platform boundaries, not matcher gaps — see
/// `.claude/subagent-dispatch.md`.)
const CLAUDE_PRETOOLUSE_MATCHER: &str = "Write|Edit|MultiEdit|Bash";

/// Evaluate the `.claude/settings.json` PreToolUse matcher from its raw content.
/// Pure (string in, status + detail out) so the parse/verdict logic is unit
/// tested without touching the filesystem.
///
/// PASS when a PreToolUse hook group registers exactly the expected matcher;
/// WARN when settings exist but no matching PreToolUse hook is wired (mutating
/// tools may be ungated); UNKNOWN when settings are absent or unparseable
/// (cannot evaluate). Never FAIL — Claude is an optional hook platform, so a
/// wiring gap is surfaced, not made fatal.
fn evaluate_pretooluse_matcher(
    settings_json: Option<&str>,
) -> (crate::adapters::CheckStatus, String) {
    use crate::adapters::CheckStatus;
    let content = match settings_json {
        Some(c) => c,
        None => {
            return (
                CheckStatus::Unknown,
                "missing: .claude/settings.json — cannot evaluate PreToolUse matcher".to_string(),
            )
        }
    };
    let json: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => {
            return (
                CheckStatus::Unknown,
                ".claude/settings.json is not valid JSON — cannot evaluate matcher".to_string(),
            )
        }
    };
    let matchers: Vec<String> = json
        .get("hooks")
        .and_then(|h| h.get("PreToolUse"))
        .and_then(|p| p.as_array())
        .map(|groups| {
            groups
                .iter()
                .filter_map(|g| g.get("matcher").and_then(|m| m.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if matchers.iter().any(|m| m == CLAUDE_PRETOOLUSE_MATCHER) {
        (
            CheckStatus::Pass,
            format!("PreToolUse gates {CLAUDE_PRETOOLUSE_MATCHER}"),
        )
    } else if matchers.is_empty() {
        (
            CheckStatus::Warn,
            "no PreToolUse hook registered in .claude/settings.json — mutating tools may be ungated"
                .to_string(),
        )
    } else {
        (
            CheckStatus::Warn,
            format!(
                "PreToolUse matcher(s) {matchers:?} do not equal {CLAUDE_PRETOOLUSE_MATCHER:?} — some mutating tools may be ungated"
            ),
        )
    }
}

/// Read `.claude/settings.json` and check its PreToolUse matcher.
fn claude_pretooluse_matcher_check(root: &std::path::Path) -> crate::adapters::AdapterCheck {
    let content = std::fs::read_to_string(root.join(".claude").join("settings.json")).ok();
    let (status, detail) = evaluate_pretooluse_matcher(content.as_deref());
    crate::adapters::AdapterCheck::new("platform.claude_pretooluse_matcher", status, detail)
}

/// Diagnose the Claude Code hook platform: gate/context hooks + settings present
/// and the PreToolUse matcher correct. `resolved = false` — Claude hosts a
/// control-guard but is NOT a resolvable executor adapter, so this keeps
/// `adapter: None` intact (Claude never enters `supported_adapters()` or
/// `adapter_for`). Missing files are WARN, never FAIL: a project may legitimately
/// not wire Claude.
fn claude_platform_diagnostic(
    root: &std::path::Path,
    verify: bool,
) -> crate::adapters::AdapterDiagnostic {
    use crate::adapters::{AdapterDiagnostic, CheckStatus};
    // Source the gate-hook path from the single platform registry (it is keyed by
    // label since Claude's `adapter` is None); fall back to the canonical path so
    // the check still runs if the row is ever renamed.
    let gate_hook = crate::infrastructure::skills::platform_skill_by_label("Claude Code")
        .map(|ps| ps.entry_point)
        .unwrap_or(".claude/hooks/ctl-gate.py");
    let checks = vec![
        presence_check(
            root,
            "platform.claude_gate_hook_present",
            gate_hook,
            CheckStatus::Warn,
        ),
        presence_check(
            root,
            "platform.claude_context_hook_present",
            ".claude/hooks/ctl-context.py",
            CheckStatus::Warn,
        ),
        presence_check(
            root,
            "platform.claude_settings_present",
            ".claude/settings.json",
            CheckStatus::Warn,
        ),
        claude_pretooluse_matcher_check(root),
        // The python hook test suite: NOT_TRACKED by default, run under --verify
        // (mirrors the opencode Bun check). Pins the per-tool gate contract.
        claude_python_tests_check(root, verify),
    ];
    AdapterDiagnostic::new("claude", false, checks)
}

/// The Claude python hook test suite (`.claude/hooks/test_*.py`). NOT_TRACKED
/// unless `verify`; under `--verify` it is actually run (UNKNOWN if the test
/// files are absent or Python is unavailable — never a silent PASS).
fn claude_python_tests_check(
    project_root: &std::path::Path,
    verify: bool,
) -> crate::adapters::AdapterCheck {
    use crate::adapters::{AdapterCheck, CheckStatus};
    const TEST_FILE: &str = ".claude/hooks/test_ctl_gate.py";
    let name = "platform.claude_hook_tests";
    if !project_root.join(TEST_FILE).exists() {
        return AdapterCheck::new(name, CheckStatus::Unknown, format!("missing: {TEST_FILE}"));
    }
    if !verify {
        return AdapterCheck::new(
            name,
            CheckStatus::NotTracked,
            "python hook tests not run by default; pass --verify to execute",
        );
    }
    match run_claude_python_tests(project_root) {
        Ok(true) => AdapterCheck::new(name, CheckStatus::Pass, "python hook tests passed"),
        Ok(false) => AdapterCheck::new(
            name,
            CheckStatus::Fail,
            "python hook tests reported failures",
        ),
        Err(e) => AdapterCheck::new(
            name,
            CheckStatus::Unknown,
            format!("python unavailable: {e}"),
        ),
    }
}

/// Run `python -m unittest discover` over `.claude/hooks` — a FIXED command (not
/// arbitrary shell). Returns whether the suite passed; errors only if Python
/// cannot be launched.
fn run_claude_python_tests(project_root: &std::path::Path) -> Result<bool> {
    let output = std::process::Command::new("python")
        .args([
            "-m",
            "unittest",
            "discover",
            "-s",
            ".claude/hooks",
            "-p",
            "test_*.py",
        ])
        .current_dir(project_root)
        .output()
        .map_err(|e| anyhow!("failed to launch python: {e}"))?;
    Ok(output.status.success())
}

/// Platform-integration checks for one adapter. An adapter with no registered
/// platform wiring yields a single UNKNOWN check (it gets contract-only
/// coverage, and we say so rather than implying a pass).
fn adapter_platform_checks(
    project_root: &std::path::Path,
    adapter_name: &str,
    verify: bool,
) -> Vec<crate::adapters::AdapterCheck> {
    use crate::adapters::{AdapterCheck, CheckStatus};
    use crate::infrastructure::skills::{
        evaluate_protocol_drift, platform_skill_for, DriftStatus, CANONICAL_PROTOCOL_PATH,
    };

    let ps = match platform_skill_for(adapter_name) {
        Some(ps) => ps,
        None => {
            return vec![AdapterCheck::new(
                "platform.integration",
                CheckStatus::Unknown,
                format!("no platform integration registered for adapter '{adapter_name}'"),
            )];
        }
    };

    let mut checks = Vec::new();

    // 1. control-guard skill must exist for this adapter.
    checks.push(presence_check(
        project_root,
        "platform.skill_present",
        ps.skill_path,
        CheckStatus::Fail,
    ));

    // 2. managed-protocol marker/version/core drift — REUSES the CI checker.
    checks.push(match evaluate_protocol_drift(project_root, ps.skill_path) {
        DriftStatus::InSync(v) => AdapterCheck::new(
            "platform.protocol_in_sync",
            CheckStatus::Pass,
            format!("managed core v{v} matches {CANONICAL_PROTOCOL_PATH}"),
        ),
        DriftStatus::Drift(why) => {
            AdapterCheck::new("platform.protocol_in_sync", CheckStatus::Fail, why)
        }
        DriftStatus::Missing => AdapterCheck::new(
            "platform.protocol_in_sync",
            CheckStatus::Unknown,
            "skill absent; cannot evaluate drift",
        ),
    });

    // 3 + 4. adapter-specific host wiring.
    match adapter_name {
        "omp" => {
            // OMP hook/config presence is checked WHEN DETECTABLE: absence is a
            // WARN/UNKNOWN, not a hard FAIL (a checkout may legitimately not wire
            // OMP), and live hook behavior is never asserted here.
            checks.push(presence_check(
                project_root,
                "platform.omp_hook_present",
                ps.entry_point,
                CheckStatus::Warn,
            ));
            checks.push(presence_check(
                project_root,
                "platform.omp_config_present",
                ".omp/settings.json",
                CheckStatus::Unknown,
            ));
        }
        "opencode" => {
            // The plugin file is the hard requirement for the opencode gate.
            checks.push(presence_check(
                project_root,
                "platform.opencode_plugin_present",
                ps.entry_point,
                CheckStatus::Fail,
            ));
            // Bun plugin tests: NOT_TRACKED by default; run only under --verify.
            checks.push(opencode_bun_tests_check(project_root, verify));
        }
        _ => {}
    }

    checks
}

/// The opencode plugin's Bun test suite. NOT_TRACKED unless `verify`; under
/// `--verify` it is actually run (UNKNOWN if the test file is absent or Bun is
/// unavailable — never a silent PASS).
fn opencode_bun_tests_check(
    project_root: &std::path::Path,
    verify: bool,
) -> crate::adapters::AdapterCheck {
    use crate::adapters::{AdapterCheck, CheckStatus};
    const TEST_FILE: &str = ".opencode/plugins/ctl-gate.test.ts";
    let name = "platform.opencode_bun_tests";
    if !project_root.join(TEST_FILE).exists() {
        return AdapterCheck::new(name, CheckStatus::Unknown, format!("missing: {TEST_FILE}"));
    }
    if !verify {
        return AdapterCheck::new(
            name,
            CheckStatus::NotTracked,
            "Bun plugin tests not run by default; pass --verify to execute",
        );
    }
    match run_bun_opencode_tests(project_root) {
        Ok(true) => AdapterCheck::new(name, CheckStatus::Pass, "bun test passed"),
        Ok(false) => AdapterCheck::new(name, CheckStatus::Fail, "bun test reported failures"),
        Err(e) => AdapterCheck::new(name, CheckStatus::Unknown, format!("bun unavailable: {e}")),
    }
}

/// Run `bun test` in `.opencode` — a FIXED command (not arbitrary shell). Returns
/// whether the suite passed; errors only if Bun cannot be launched.
fn run_bun_opencode_tests(project_root: &std::path::Path) -> Result<bool> {
    let dir = project_root.join(".opencode");
    let output = std::process::Command::new("bun")
        .arg("test")
        .current_dir(&dir)
        .output()
        .map_err(|e| anyhow!("failed to launch bun: {e}"))?;
    Ok(output.status.success())
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

/// True iff a lease created at `created_epoch` is past its `ttl` at `now_epoch`.
/// Pure (the wall clock is read by the caller) so it is unit-testable; mirrors
/// the staleness check in `recover_report` (strictly greater than the TTL).
fn ttl_exceeded(now_epoch: u64, created_epoch: u64, ttl: u64) -> bool {
    now_epoch.saturating_sub(created_epoch) > ttl
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

    // ── TDD red→green interlock (ctl-tdd-loop-v1) ──

    /// Drive a TDD-opted task (cargo_test gate + `tdd-red-green` trigger) to a
    /// finishable Review state, optionally recording a RED cargo_test before the
    /// green one. Non-git temp dir → tree/commit interlocks are skipped, isolating
    /// the TDD check.
    fn drive_tdd_to_review(app: &ControlApp, id: &str, record_red: bool) {
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_test".to_string()];
        let triggers = vec![TDD_RED_GREEN_TRIGGER.to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "tdd",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &triggers,
                gates: &gates,
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready(id).unwrap();
        app.start_task(id).unwrap();
        if record_red {
            app.record_gate(id, "cargo_test", false, "red: test fails, impl absent")
                .unwrap();
        }
        app.submit_task(id).unwrap();
        app.record_gate(id, "cargo_test", true, "green: impl done")
            .unwrap();
        audit_pass(app, id, None);
    }

    #[test]
    fn tdd_interlock_blocks_when_test_only_passed() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_tdd_to_review(&app, "tdd-nored", false); // green only, never red
        let err = app.finish_task("tdd-nored").unwrap_err().to_string();
        assert!(err.contains("tdd-red-green"), "got: {err}");
        assert!(err.contains("red→green"), "got: {err}");
    }

    #[test]
    fn tdd_interlock_allows_with_red_before_green() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_tdd_to_review(&app, "tdd-ok", true); // red, then green
        let ev = app.finish_task("tdd-ok").unwrap();
        assert_eq!(ev.event_type, "task_completed");
    }

    #[test]
    fn tdd_interlock_inactive_without_trigger() {
        // A normal task (no trigger) finishes with only a green gate.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review(&app, "normal");
        let ev = app.finish_task("normal").unwrap();
        assert_eq!(ev.event_type, "task_completed");
    }

    #[test]
    fn tdd_interlock_requires_a_test_gate() {
        // Opted into TDD but no cargo_test gate → clear misconfiguration block.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()]; // no cargo_test
        let triggers = vec![TDD_RED_GREEN_TRIGGER.to_string()];
        app.create_task(
            "tdd-misconf",
            CreateTaskInput {
                objective: "x",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &triggers,
                gates: &gates,
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready("tdd-misconf").unwrap();
        app.start_task("tdd-misconf").unwrap();
        app.submit_task("tdd-misconf").unwrap();
        app.record_gate("tdd-misconf", "cargo_check", true, "ok")
            .unwrap();
        audit_pass(&app, "tdd-misconf", None);
        let err = app.finish_task("tdd-misconf").unwrap_err().to_string();
        assert!(err.contains("no 'cargo_test' gate"), "got: {err}");
    }

    #[test]
    fn gate_red_before_green_helper() {
        let mk = |seq: i64, passed: bool| Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: format!("e{seq}"),
            command_id: format!("c{seq}"),
            task_id: "t".to_string(),
            seq,
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
            actor: "t".to_string(),
            event_type: "gate_checked".to_string(),
            payload: serde_json::json!({"gate_id": "cargo_test", "passed": passed}),
        };
        // pass-only → false
        assert!(!gate_went_red_before_green(&[mk(1, true)], "cargo_test"));
        // fail then pass → true
        assert!(gate_went_red_before_green(
            &[mk(1, false), mk(2, true)],
            "cargo_test"
        ));
        // pass then fail (no later pass) → false
        assert!(!gate_went_red_before_green(
            &[mk(1, true), mk(2, false)],
            "cargo_test"
        ));
        // different gate id → false
        assert!(!gate_went_red_before_green(
            &[mk(1, false), mk(2, true)],
            "cargo_check"
        ));
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

        // Commit the work → tree clean in scope. The gate/audit recorded earlier
        // are now bound to a different (or no) tree, so finish stays closed until
        // they are re-validated against the current committed tree (artifact binding).
        git(dir.path(), &["add", "src/work.rs"]);
        git(dir.path(), &["commit", "-qm", "work"]);
        let stale = app.finish_task("ilk").unwrap_err().to_string();
        assert!(
            stale.contains("completion evidence is stale"),
            "expected artifact binding to block stale evidence, got: {stale}"
        );

        // Re-gate + re-audit on the current tree → finish succeeds.
        app.record_gate("ilk", "cargo_check", true, "ok").unwrap();
        audit_pass(&app, "ilk", None);
        let event = app.finish_task("ilk").unwrap();
        assert_eq!(event.event_type, "task_completed");
    }

    // ── Artifact binding (tree_hash) interlock ──

    /// git repo with one initial commit, so `HEAD^{tree}` exists.
    fn git_init_committed(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", "init"]);
    }

    /// Commit a new src/ file so the committed tree advances.
    fn commit_src(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join("src").join(name), body).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", "change"]);
    }

    /// create + ready + start + submit a git task scoped to src/ with `gates`.
    fn git_task_to_review(app: &ControlApp, id: &str, gates: &[String]) {
        let scope = vec!["src".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "tree binding test",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates,
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready(id).unwrap();
        app.start_task(id).unwrap();
        app.submit_task(id).unwrap();
    }

    // Case 1: gate + audit on the same committed tree → finish succeeds.
    #[test]
    fn artifact_binding_same_tree_finishes() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(&app, "t", &["cargo_check".to_string()]);
        app.record_gate("t", "cargo_check", true, "ok").unwrap();
        audit_pass(&app, "t", None);
        assert_eq!(app.finish_task("t").unwrap().event_type, "task_completed");
    }

    // Case 2: gate recorded, then a new commit → gate is stale → finish fails.
    #[test]
    fn artifact_binding_gate_then_commit_is_stale() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(&app, "t", &["cargo_check".to_string()]);
        app.record_gate("t", "cargo_check", true, "ok").unwrap();
        commit_src(dir.path(), "b.rs", "fn b() {}\n");
        audit_pass(&app, "t", None);
        let err = app.finish_task("t").unwrap_err().to_string();
        assert!(err.contains("completion evidence is stale"), "{err}");
    }

    // Case 3: audit recorded, then a new commit → evidence is stale → finish fails.
    #[test]
    fn artifact_binding_audit_then_commit_is_stale() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(&app, "t", &["cargo_check".to_string()]);
        app.record_gate("t", "cargo_check", true, "ok").unwrap();
        audit_pass(&app, "t", None);
        commit_src(dir.path(), "b.rs", "fn b() {}\n");
        let err = app.finish_task("t").unwrap_err().to_string();
        assert!(err.contains("completion evidence is stale"), "{err}");
    }

    // Case 4: gate re-run on the new tree but audit NOT re-run → audit stale → fail.
    #[test]
    fn artifact_binding_regate_without_reaudit_is_stale() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(&app, "t", &["cargo_check".to_string()]);
        app.record_gate("t", "cargo_check", true, "ok").unwrap();
        audit_pass(&app, "t", None);
        commit_src(dir.path(), "b.rs", "fn b() {}\n");
        app.record_gate("t", "cargo_check", true, "ok").unwrap(); // gate now fresh
        let err = app.finish_task("t").unwrap_err().to_string();
        assert!(
            err.contains("completion audit is stale"),
            "expected audit-stale, got: {err}"
        );
    }

    // Case 5: audit re-run on the new tree but a required gate still on old tree → fail.
    #[test]
    fn artifact_binding_reaudit_with_stale_gate_fails() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(&app, "t", &["cargo_check".to_string()]);
        app.record_gate("t", "cargo_check", true, "ok").unwrap();
        audit_pass(&app, "t", None);
        commit_src(dir.path(), "b.rs", "fn b() {}\n");
        audit_pass(&app, "t", None); // audit now fresh, gate still old
        let err = app.finish_task("t").unwrap_err().to_string();
        assert!(
            err.contains("completion evidence is stale") && err.contains("tree-stale"),
            "expected gate-stale, got: {err}"
        );
    }

    // Case 6: a legacy gate_checked without tree_hash replays fine but cannot finish.
    #[test]
    fn artifact_binding_legacy_unbound_gate_replays_but_blocks_finish() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(&app, "t", &["cargo_check".to_string()]);
        // Pre-binding event: no tree_hash. Schema + reducer must accept it (replay ok).
        let ev = app
            .build_event(
                "t",
                "gate_checked",
                serde_json::json!({
                    "gate_id": "cargo_check",
                    "passed": true,
                    "evidence": "ok",
                    "checked_at": "2026-01-01T00:00:00Z"
                }),
            )
            .unwrap();
        app.validate_and_append(&ev).unwrap(); // proves replay tolerates missing tree_hash
        audit_pass(&app, "t", None);
        let err = app.finish_task("t").unwrap_err().to_string();
        assert!(err.contains("completion evidence is stale"), "{err}");
    }

    // Case 7: a FAILED gate on the current tree still cannot finish (passing check first).
    #[test]
    fn artifact_binding_failed_gate_same_tree_not_passing() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(&app, "t", &["cargo_check".to_string()]);
        app.record_gate("t", "cargo_check", false, "boom").unwrap();
        audit_pass(&app, "t", None);
        let err = app.finish_task("t").unwrap_err().to_string();
        assert!(err.contains("gates not passing"), "{err}");
    }

    // Case 8: with multiple required gates, one stale gate blocks finish.
    #[test]
    fn artifact_binding_one_of_many_gates_stale() {
        let dir = TempDir::new();
        git_init_committed(dir.path());
        let app = ControlApp::init(dir.path()).unwrap();
        git_task_to_review(
            &app,
            "t",
            &["cargo_check".to_string(), "cargo_fmt_check".to_string()],
        );
        app.record_gate("t", "cargo_check", true, "ok").unwrap();
        app.record_gate("t", "cargo_fmt_check", true, "ok").unwrap();
        commit_src(dir.path(), "b.rs", "fn b() {}\n");
        app.record_gate("t", "cargo_check", true, "ok").unwrap(); // only this one refreshed
        audit_pass(&app, "t", None);
        let err = app.finish_task("t").unwrap_err().to_string();
        assert!(
            err.contains("completion evidence is stale") && err.contains("cargo_fmt_check"),
            "expected cargo_fmt_check stale, got: {err}"
        );
    }

    // ── policy_hash interlock ──
    //
    // Hash *sensitivity* (widen/narrow scope, gate-arg change, gate-set change,
    // canonicalization) is covered by unit tests in `domain::policy`. These tests
    // cover the finish-time *interlock*: a policy-mismatched (or unbound) gate or
    // audit must block completion. Policy cannot change mid-task via the public
    // API, so staleness is simulated by recording evidence under a different
    // policy hash — exactly what a gate-catalog change across ctl versions yields.

    /// Overwrite cargo_check's latest result with an explicit/missing policy_hash.
    fn append_gate_policy(app: &ControlApp, id: &str, policy_hash: Option<&str>) {
        let mut p = serde_json::json!({
            "gate_id": "cargo_check", "passed": true,
            "evidence": "ok", "checked_at": "2026-01-01T00:00:00Z"
        });
        if let Some(ph) = policy_hash {
            p["policy_hash"] = serde_json::json!(ph);
        }
        let ev = app.build_event(id, "gate_checked", p).unwrap();
        app.validate_and_append(&ev).unwrap();
    }

    /// Append a completion audit with an explicit policy_hash, as a non-implementer.
    fn append_audit_policy(app: &ControlApp, id: &str, policy_hash: Option<&str>) {
        let reviewer = ControlApp::open(&app.project_root, false)
            .unwrap()
            .with_actor("reviewer");
        let mut p = serde_json::json!({
            "evidence_id": "aud-x", "source": COMPLETION_AUDIT_SOURCE,
            "touched_files": [], "result_file": "", "accepted_at": "2026-01-01T00:00:00Z"
        });
        if let Some(ph) = policy_hash {
            p["policy_hash"] = serde_json::json!(ph);
        }
        let ev = reviewer.build_event(id, "evidence_accepted", p).unwrap();
        reviewer.validate_and_append(&ev).unwrap();
    }

    // Same policy (non-git) → finish succeeds.
    #[test]
    fn policy_binding_same_policy_finishes() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "p");
        audit_pass(&app, "p", None);
        assert_eq!(app.finish_task("p").unwrap().event_type, "task_completed");
    }

    // A gate produced under a different policy → finish fails (policy-stale).
    #[test]
    fn policy_binding_stale_gate_policy_fails() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "p");
        append_gate_policy(&app, "p", Some("stale-policy-hash"));
        audit_pass(&app, "p", None);
        let err = app.finish_task("p").unwrap_err().to_string();
        assert!(
            err.contains("completion evidence is stale") && err.contains("policy-stale"),
            "{err}"
        );
    }

    // An audit accepted under a different policy → finish fails.
    #[test]
    fn policy_binding_stale_audit_policy_fails() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "p");
        append_audit_policy(&app, "p", Some("stale-policy-hash"));
        let err = app.finish_task("p").unwrap_err().to_string();
        assert!(
            err.contains("completion audit is stale") && err.contains("policy"),
            "{err}"
        );
    }

    // A legacy gate without policy_hash replays but cannot satisfy a new finish.
    #[test]
    fn policy_binding_legacy_unbound_gate_blocks_finish() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "p");
        append_gate_policy(&app, "p", None); // replay tolerates missing policy_hash
        audit_pass(&app, "p", None);
        let err = app.finish_task("p").unwrap_err().to_string();
        assert!(err.contains("completion evidence is stale"), "{err}");
    }

    // ── single-writer ledger ──

    // Many concurrent writers on one task must never produce a duplicate or
    // non-monotonic sequence number; the per-task lock serializes the
    // read-seq → validate → append critical section. Losers of a race fail safe
    // (reducer "Sequence error") rather than corrupting the ledger.
    #[test]
    fn concurrent_writers_never_duplicate_seq() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_to_review_bare(&app, "c"); // task in Review with a cargo_check gate
        let root = app.project_root.clone();

        let mut handles = Vec::new();
        for _ in 0..6 {
            let r = root.clone();
            handles.push(std::thread::spawn(move || {
                // Each thread is an independent "process" view of the ledger.
                let a = ControlApp::open(&r, false).unwrap();
                let _ = a.record_gate("c", "cargo_check", true, "ok");
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let app2 = ControlApp::open(dir.path(), false).unwrap();
        let seqs: Vec<i64> = app2
            .store
            .read_for_task("c")
            .unwrap()
            .iter()
            .map(|e| e.seq)
            .collect();
        let mut uniq = seqs.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(
            seqs.len(),
            uniq.len(),
            "duplicate sequence under concurrency: {seqs:?}"
        );
        for w in seqs.windows(2) {
            assert!(w[1] > w[0], "non-monotonic sequence: {seqs:?}");
        }
    }

    // ── M6: merge-candidate ──

    /// git repo + initial commit (tracked src/lib.rs and docs/readme.md), then
    /// a started task with an isolated worktree. Returns (app, worktree_path).
    fn setup_worktree(dir: &Path, id: &str, write_allow: &[&str]) -> (ControlApp, PathBuf) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.join("docs/readme.md"), "x\n").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", "init"]);

        let app = ControlApp::init(dir).unwrap();
        let scope: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "merge candidate",
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
        app.workspace_create(id).unwrap();
        let wt = dir.join(".ctl/tasks").join(id).join("worktree");
        (app, wt)
    }

    /// Conformance (cross-adapter): ingesting an in-scope result tags the
    /// accepted evidence with the *adapter's own* source — for every supported
    /// adapter, driven off the registry so a new adapter is covered for free.
    #[test]
    fn ingest_tags_evidence_source_for_every_adapter() {
        for adapter in crate::adapters::supported_adapters() {
            let dir = TempDir::new();
            git(dir.path(), &["init", "-q"]);
            git(dir.path(), &["config", "user.email", "t@t"]);
            git(dir.path(), &["config", "user.name", "t"]);
            std::fs::create_dir_all(dir.path().join("src")).unwrap();
            std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
            git(dir.path(), &["add", "-A"]);
            git(dir.path(), &["commit", "-qm", "init"]);

            let app = ControlApp::init(dir.path()).unwrap();
            let scope = vec!["src".to_string()];
            app.create_task(
                "t",
                CreateTaskInput {
                    objective: "ingest source",
                    read_scope: &scope,
                    write_allow: &scope,
                    write_deny: &[],
                    risk_triggers: &[],
                    gates: &["cargo_check".to_string()],
                    depends_on: &[],
                },
            )
            .unwrap();
            app.mark_ready("t").unwrap();
            app.start_task("t").unwrap();
            app.run_start("t", adapter).unwrap();

            let result_file = dir.path().join("agent-output.json");
            std::fs::write(
                &result_file,
                format!(r#"{{"source":"{adapter}","touched_files":["src/lib.rs"]}}"#),
            )
            .unwrap();
            let evidence = app.run_ingest("t", &result_file, adapter).unwrap();

            assert_eq!(
                evidence.event_type, "evidence_accepted",
                "{adapter}: in-scope ingest should be accepted"
            );
            assert_eq!(
                evidence.payload["source"], *adapter,
                "{adapter}: accepted evidence must be tagged source={adapter}"
            );
        }
    }

    #[test]
    fn merge_candidate_in_scope_is_mergeable() {
        let dir = TempDir::new();
        let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
        // Modify a tracked, in-scope file in the worktree.
        std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();

        let v = app.merge_candidate("mc").unwrap();
        assert_eq!(v["mergeable"], true, "verdict: {v}");
        assert!(v["blocking_reasons"].as_array().unwrap().is_empty());
        assert!(v["touched_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "src/lib.rs"));
    }

    #[test]
    fn merge_candidate_out_of_scope_blocks() {
        let dir = TempDir::new();
        // Task scope is only src/, but the worktree also edits docs/readme.md.
        let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
        std::fs::write(wt.join("docs/readme.md"), "edited\n").unwrap();

        let v = app.merge_candidate("mc").unwrap();
        assert_eq!(v["mergeable"], false, "verdict: {v}");
        assert!(v["out_of_scope"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "docs/readme.md"));
    }

    #[test]
    fn merge_candidate_cross_task_conflict_blocks() {
        let dir = TempDir::new();
        let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
        std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();

        // Another active task claims the same file → cross-task collision.
        let other_scope = vec!["src/lib.rs".to_string()];
        app.create_task(
            "other",
            CreateTaskInput {
                objective: "rival",
                read_scope: &other_scope,
                write_allow: &other_scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready("other").unwrap();
        app.start_task("other").unwrap();

        let v = app.merge_candidate("mc").unwrap();
        assert_eq!(v["mergeable"], false, "verdict: {v}");
        let conflicts = v["cross_task_conflicts"].as_array().unwrap();
        assert!(conflicts
            .iter()
            .any(|c| c["conflicting_task"] == "other" && c["path"] == "src/lib.rs"));
    }

    #[test]
    fn merge_candidate_dirty_main_workspace_blocks() {
        let dir = TempDir::new();
        let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
        std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();
        // The main workspace has its own uncommitted edit to the same file.
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() { /* main */ }\n").unwrap();

        let v = app.merge_candidate("mc").unwrap();
        assert_eq!(v["mergeable"], false, "verdict: {v}");
        assert!(v["workspace_conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "src/lib.rs"));
    }

    #[test]
    fn merge_candidate_emits_no_events() {
        let dir = TempDir::new();
        let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
        std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();
        let before = app.store.read_for_task("mc").unwrap().len();
        app.merge_candidate("mc").unwrap();
        assert_eq!(
            app.store.read_for_task("mc").unwrap().len(),
            before,
            "merge_candidate must be read-only"
        );
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

    // ── M6: dependency-gated start (serial orchestration) ───────────────────

    /// Create + ready a task with the given dependency edges, leaving it Ready
    /// (not started) so the start-time dependency gate can be exercised.
    fn create_with_deps(app: &ControlApp, id: &str, deps: &[&str]) {
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()];
        let deps: Vec<String> = deps.iter().map(|s| s.to_string()).collect();
        app.create_task(
            id,
            CreateTaskInput {
                objective: "dependency-gated start test",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &gates,
                depends_on: &deps,
            },
        )
        .unwrap();
        app.mark_ready(id).unwrap();
    }

    /// Drive an already-created, Ready task (dependencies satisfied) all the way
    /// to Completed. No git repo → the M-g commit interlock is skipped; M-f's
    /// audit is supplied by a non-implementer reviewer.
    fn finish_ready(app: &ControlApp, id: &str) {
        app.start_task(id).unwrap();
        app.submit_task(id).unwrap();
        app.record_gate(id, "cargo_check", true, "ok").unwrap();
        audit_pass(app, id, None);
        app.finish_task(id).unwrap();
        assert_eq!(app.replay_task(id).unwrap().phase, Phase::Completed);
    }

    #[test]
    fn start_blocked_while_dependency_incomplete() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        create_with_deps(&app, "dep", &[]); // left in Ready, never Completed
        create_with_deps(&app, "dependent", &["dep"]);
        assert_eq!(
            app.unmet_dependencies("dependent").unwrap(),
            vec!["dep".to_string()]
        );
        let err = app.start_task("dependent").unwrap_err().to_string();
        assert!(err.contains("blocked by"), "got: {err}");
        assert!(err.contains("dep"), "error should name the blocker: {err}");
    }

    #[test]
    fn start_allowed_once_dependency_completed() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        create_with_deps(&app, "dep", &[]);
        finish_ready(&app, "dep");
        create_with_deps(&app, "dependent", &["dep"]);
        assert!(app.unmet_dependencies("dependent").unwrap().is_empty());
        let event = app.start_task("dependent").unwrap();
        assert_eq!(event.event_type, "task_started");
    }

    #[test]
    fn start_allowed_when_dependency_archived() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        create_with_deps(&app, "dep", &[]);
        finish_ready(&app, "dep");
        app.archive_task("dep").unwrap();
        // Archiving keeps the phase at Completed, so it still satisfies.
        assert_eq!(app.replay_task("dep").unwrap().phase, Phase::Completed);
        create_with_deps(&app, "dependent", &["dep"]);
        assert!(app.unmet_dependencies("dependent").unwrap().is_empty());
        app.start_task("dependent").unwrap();
    }

    #[test]
    fn start_rejected_for_unknown_dependency() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        create_with_deps(&app, "dependent", &["ghost"]);
        // Missing prerequisite → unmet (fail closed).
        assert_eq!(
            app.unmet_dependencies("dependent").unwrap(),
            vec!["ghost".to_string()]
        );
        let err = app.start_task("dependent").unwrap_err().to_string();
        assert!(err.contains("ghost"), "got: {err}");
    }

    #[test]
    fn dependency_chain_runs_strictly_serial() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        create_with_deps(&app, "a", &[]);
        create_with_deps(&app, "b", &["a"]);
        create_with_deps(&app, "c", &["b"]);
        // While A is unfinished, both B and C are blocked.
        assert!(app.start_task("b").is_err());
        assert!(app.start_task("c").is_err());
        // A complete → B may start; C still blocked on the in-progress B.
        finish_ready(&app, "a");
        app.start_task("b").unwrap();
        assert!(app.start_task("c").is_err());
        // Drive B (already InProgress) to Completed → C may finally start.
        app.submit_task("b").unwrap();
        app.record_gate("b", "cargo_check", true, "ok").unwrap();
        audit_pass(&app, "b", None);
        app.finish_task("b").unwrap();
        let event = app.start_task("c").unwrap();
        assert_eq!(event.event_type, "task_started");
    }

    #[test]
    fn start_unaffected_without_dependencies() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        create_with_deps(&app, "solo", &[]);
        assert!(app.unmet_dependencies("solo").unwrap().is_empty());
        let event = app.start_task("solo").unwrap();
        assert_eq!(event.event_type, "task_started");
    }

    // ── M6: AgentRun aggregate concurrency (slice 1) ────────────────────────

    /// Create + ready + start a write task so runs can be created against it.
    fn inprogress_task(app: &ControlApp, id: &str, write_allow: &[&str]) {
        let scope: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "m6 concurrent run test",
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

    /// Seed a Running run aggregate directly via events (no git worktree), so
    /// the overlap invariant can be exercised without a real repo. Returns the
    /// run_id.
    fn seed_running_run(app: &ControlApp, task_id: &str, write_allow: &[&str]) -> String {
        let run_id = generate_uuid();
        let wa: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
        let created = app
            .build_run_event(
                &run_id,
                "run_created",
                serde_json::json!({
                    "task_id": task_id,
                    "adapter": "omp",
                    "write_allow": wa,
                    "write_deny": [],
                    "gates": ["cargo_check"],
                }),
            )
            .unwrap();
        app.append_run_event(&run_id, created).unwrap();
        let started = app
            .build_run_event(
                &run_id,
                "run_started",
                serde_json::json!({
                    "worktree_path": format!(".ctl/runs/{}/worktree", run_id),
                    "lease_id": "lease-seed",
                }),
            )
            .unwrap();
        app.append_run_event(&run_id, started).unwrap();
        run_id
    }

    // ── cross-ledger detect+repair (cross-ledger-detect-repair-v1) ──

    fn mk_planning_task(app: &ControlApp, id: &str) {
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            id,
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
    }

    /// Like `seed_running_run` but the worktree is an absolute path that actually
    /// exists on disk — so the run looks fully consistent.
    fn seed_running_run_with_worktree(
        app: &ControlApp,
        root: &Path,
        task_id: &str,
        write_allow: &[&str],
    ) -> String {
        let run_id = generate_uuid();
        let wt = root
            .join(".ctl")
            .join("runs")
            .join(&run_id)
            .join("worktree");
        std::fs::create_dir_all(&wt).unwrap();
        let wa: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
        let created = app
            .build_run_event(
                &run_id,
                "run_created",
                serde_json::json!({
                    "task_id": task_id, "adapter": "omp", "write_allow": wa,
                    "write_deny": [], "gates": ["cargo_check"],
                }),
            )
            .unwrap();
        app.append_run_event(&run_id, created).unwrap();
        let started = app
            .build_run_event(
                &run_id,
                "run_started",
                serde_json::json!({
                    "worktree_path": wt.to_string_lossy(), "lease_id": "lease-seed",
                }),
            )
            .unwrap();
        app.append_run_event(&run_id, started).unwrap();
        run_id
    }

    #[test]
    fn cross_ledger_detects_orphan_run() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let run_id = seed_running_run(&app, "ghost-task", &["src"]);
        let f = app.cross_ledger_findings().unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, CrossLedgerKind::OrphanRun);
        assert_eq!(f[0].run_id, run_id);
        assert!(matches!(f[0].repair, RepairAction::AbortRun { .. }));
    }

    #[test]
    fn cross_ledger_detects_stranded_run() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.cancel_task("t1").unwrap(); // terminal task
        seed_running_run(&app, "t1", &["src"]);
        let f = app.cross_ledger_findings().unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, CrossLedgerKind::StrandedRun);
    }

    #[test]
    fn cross_ledger_detects_missing_worktree_run() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.mark_ready("t1").unwrap();
        app.start_task("t1").unwrap(); // live, InProgress
        seed_running_run(&app, "t1", &["src"]); // worktree path not on disk
        let f = app.cross_ledger_findings().unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, CrossLedgerKind::MissingWorktreeRun);
    }

    #[test]
    fn cross_ledger_clean_when_run_consistent() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.mark_ready("t1").unwrap();
        app.start_task("t1").unwrap();
        seed_running_run_with_worktree(&app, dir.path(), "t1", &["src"]);
        let f = app.cross_ledger_findings().unwrap();
        assert!(f.is_empty(), "consistent run yields no finding: {f:?}");
    }

    #[test]
    fn cross_ledger_detects_orphaned_worktree() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let run_id = seed_running_run_with_worktree(&app, dir.path(), "ghost", &["src"]);
        // Terminal-ize WITHOUT abort_run so the worktree dir lingers.
        let aborted = app
            .build_run_event(&run_id, "run_aborted", serde_json::json!({"reason": "x"}))
            .unwrap();
        app.append_run_event(&run_id, aborted).unwrap();
        let f = app.cross_ledger_findings().unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, CrossLedgerKind::OrphanedWorktree);
        assert!(matches!(f[0].repair, RepairAction::RemoveWorktree { .. }));
    }

    #[test]
    fn cross_ledger_apply_aborts_run_and_clears_finding() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let run_id = seed_running_run(&app, "ghost-task", &["src"]);
        let findings = app.cross_ledger_findings().unwrap();
        assert_eq!(findings.len(), 1);
        let outcome = app.apply_cross_ledger_repair(&findings[0]);
        assert!(outcome.applied, "repair applied: {}", outcome.result);
        assert_eq!(outcome.run_id, run_id);
        // Run is now Aborted (terminal) → no longer a cross-ledger finding.
        let after = app.cross_ledger_findings().unwrap();
        assert!(after.is_empty(), "finding cleared after repair: {after:?}");
    }

    #[test]
    fn handoff_export_assembles_read_only_artifact() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.mark_ready("t1").unwrap();
        app.start_task("t1").unwrap();

        let h = app.handoff_export("t1").unwrap();
        assert_eq!(h["schema"], "control.handoff.v1");
        assert_eq!(h["task_id"], "t1");
        assert_eq!(h["phase"], "InProgress");
        assert_eq!(h["objective"], "x");
        assert!(h["boundary"]["write_allow"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "src"));
        let gates = h["gate_status"].as_array().unwrap();
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0]["status"], "PENDING"); // gate never run
        assert!(!h["recent_events"].as_array().unwrap().is_empty());

        // Export must be purely read-only — no events appended.
        let before = app.replay_task("t1").unwrap().last_seq;
        let _ = app.handoff_export("t1").unwrap();
        assert_eq!(
            app.replay_task("t1").unwrap().last_seq,
            before,
            "handoff export must not append events"
        );
    }

    #[test]
    fn handoff_export_includes_captured_judgment() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.mark_ready("t1").unwrap();
        app.start_task("t1").unwrap();
        let handoffs = dir.path().join(".ctl/handoffs");
        std::fs::create_dir_all(&handoffs).unwrap();
        std::fs::write(
            handoffs.join("t1.json"),
            r#"{
                "schema": "control.handoff.capture.v1",
                "task_id": "t1",
                "source": "agent_or_human_supplied",
                "next_safe_action": "run the required gate",
                "decisions": ["keep the scope narrow"],
                "uncertainties": ["reviewer availability"]
            }"#,
        )
        .unwrap();

        let h = app.handoff_export("t1").unwrap();
        assert_eq!(h["capture"]["next_safe_action"], "run the required gate");
        assert_eq!(h["capture"]["source"], "agent_or_human_supplied");
        assert_eq!(h["capture"]["decisions"][0], "keep the scope narrow");
    }

    // ── ralph safety supervisor (ralph-safe-run-v1) ──

    #[test]
    fn ralph_safety_go_on_clean_active_task() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.mark_ready("t1").unwrap();
        app.start_task("t1").unwrap();
        let v = app.ralph_safety_check("t1").unwrap();
        assert!(v.go, "clean active task is GO, blockers: {:?}", v.blockers);
    }

    #[test]
    fn ralph_safety_nogo_on_terminal_task() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.cancel_task("t1").unwrap();
        let v = app.ralph_safety_check("t1").unwrap();
        assert!(!v.go);
        assert!(
            v.blockers.iter().any(|b| b.contains("terminal")),
            "blockers: {:?}",
            v.blockers
        );
    }

    #[test]
    fn ralph_safety_nogo_on_cross_ledger_drift() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_planning_task(&app, "t1");
        app.mark_ready("t1").unwrap();
        app.start_task("t1").unwrap();
        // A stranded/orphan run anywhere is a global cross-ledger inconsistency.
        seed_running_run(&app, "ghost-task", &["other"]);
        let v = app.ralph_safety_check("t1").unwrap();
        assert!(!v.go);
        assert!(
            v.blockers.iter().any(|b| b.contains("cross-ledger")),
            "blockers: {:?}",
            v.blockers
        );
    }

    // ── run-lease TTL expiry (capability-lease-ttl-enforce-v1) ──

    #[test]
    fn ttl_exceeded_is_strictly_greater() {
        assert!(ttl_exceeded(100, 0, 50)); // age 100 > 50
        assert!(!ttl_exceeded(40, 0, 50)); // age 40 < 50
        assert!(!ttl_exceeded(50, 0, 50)); // age 50 == 50 (not strictly greater)
        assert!(!ttl_exceeded(0, 1000, 50)); // now before created → age 0 (saturating)
    }

    /// Seed a Running run carrying a genuine native lease (lease_created +
    /// lease_used + run_started), satisfying the run reducer's binding rules.
    fn seed_run_with_native_lease(app: &ControlApp, run_id: &str, task_id: &str, ttl: u64) {
        let ev = |app: &ControlApp, ty: &str, p: serde_json::Value| {
            let e = app.build_run_event(run_id, ty, p).unwrap();
            app.append_run_event(run_id, e).unwrap();
        };
        ev(
            app,
            "run_created",
            serde_json::json!({"task_id": task_id, "adapter": "omp", "write_allow": ["src"],
                "write_deny": [], "gates": ["cargo_check"]}),
        );
        ev(
            app,
            "lease_created",
            serde_json::json!({"lease_id": "L1", "run_id": run_id, "resource_path": "src",
                "action": "write", "ttl_seconds": ttl, "max_uses": 100,
                "task_id": task_id, "adapter": "omp", "scopes": ["src"]}),
        );
        ev(app, "lease_used", serde_json::json!({"lease_id": "L1"}));
        ev(
            app,
            "run_started",
            serde_json::json!({"worktree_path": format!(".ctl/runs/{run_id}/worktree"), "lease_id": "L1"}),
        );
    }

    const FAR_FUTURE: u64 = 10_000_000_000; // year ~2286 — well past any lease TTL

    #[test]
    fn expire_lease_records_lease_expired_when_stale() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_run_with_native_lease(&app, "r1", "t1", 3600);
        let report = app.expire_run_lease_at("r1", FAR_FUTURE, true).unwrap();
        assert_eq!(report.outcome, "expired", "{}", report.detail);
        // The lease is now terminally Expired.
        let run = app.replay_run("r1").unwrap();
        assert_eq!(
            run.lease.unwrap().status,
            crate::domain::lease::LeaseStatus::Expired
        );
    }

    #[test]
    fn expire_lease_preview_does_not_mutate() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_run_with_native_lease(&app, "r1", "t1", 3600);
        let before = app.replay_run("r1").unwrap().last_seq;
        let report = app.expire_run_lease_at("r1", FAR_FUTURE, false).unwrap();
        assert_eq!(report.outcome, "would_expire", "{}", report.detail);
        assert_eq!(
            app.replay_run("r1").unwrap().last_seq,
            before,
            "preview must not append"
        );
        assert_eq!(
            app.replay_run("r1").unwrap().lease.unwrap().status,
            crate::domain::lease::LeaseStatus::Active
        );
    }

    #[test]
    fn expire_lease_refuses_within_ttl() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_run_with_native_lease(&app, "r1", "t1", 3600);
        // now before created → age 0 → not stale.
        let report = app.expire_run_lease_at("r1", 1, false).unwrap();
        assert_eq!(report.outcome, "within_ttl", "{}", report.detail);
    }

    #[test]
    fn expire_lease_no_native_lease() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_running_run(&app, "t1", &["src"]); // legacy run, no native lease
        let runs = app.run_store().unwrap().run_ids().unwrap();
        let report = app.expire_run_lease_at(&runs[0], FAR_FUTURE, true).unwrap();
        assert_eq!(report.outcome, "no_lease", "{}", report.detail);
    }

    #[test]
    fn create_run_requires_in_progress_task() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let scope = vec!["src".to_string()];
        let gates = vec!["cargo_check".to_string()];
        app.create_task(
            "planned",
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
        // Still in Planning → no run may be created.
        let err = app.create_run("planned", "omp").unwrap_err().to_string();
        assert!(err.contains("InProgress"), "got: {err}");
    }

    #[test]
    fn create_run_persists_queued_aggregate() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        inprogress_task(&app, "t1", &["src"]);
        let run_id = app.create_run("t1", "omp").unwrap();
        assert!(dir
            .path()
            .join(".ctl/runs")
            .join(&run_id)
            .join("events.jsonl")
            .exists());
        let run = app.replay_run(&run_id).unwrap();
        assert_eq!(run.phase, RunPhase::Queued);
        assert_eq!(run.task_id, "t1");
        assert!(run.write_allow.contains("src"));
        // Queued is not yet part of the active concurrency set.
        assert!(app.active_runs().unwrap().is_empty());
    }

    #[test]
    fn overlapping_run_start_rejected() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // A already Running on src (seeded, no git needed).
        let a = seed_running_run(&app, "task-a", &["src"]);
        assert_eq!(app.active_runs().unwrap().len(), 1);
        // B is InProgress with an overlapping scope; its run start is refused
        // BEFORE any worktree is created.
        inprogress_task(&app, "task-b", &["src"]);
        let b = app.create_run("task-b", "omp").unwrap();
        let err = app.start_run(&b).unwrap_err().to_string();
        assert!(err.contains("scope conflict"), "got: {err}");
        assert!(err.contains(&a), "should name the conflicting run: {err}");
        assert!(!dir
            .path()
            .join(".ctl/runs")
            .join(&b)
            .join("worktree")
            .exists());
        assert_eq!(app.replay_run(&b).unwrap().phase, RunPhase::Queued);
    }

    #[test]
    fn finishing_run_frees_scope() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let a = seed_running_run(&app, "task-a", &["src"]);
        let scope: BTreeSet<String> = ["src".to_string()].into_iter().collect();
        // While A runs, an overlapping scope is blocked.
        assert!(app.check_run_scope_overlap("other", &scope).is_err());
        // Finish A (seeded worktree path doesn't exist → cleanup skipped, no git).
        app.finish_run(&a).unwrap();
        assert_eq!(app.replay_run(&a).unwrap().phase, RunPhase::Completed);
        assert!(app.active_runs().unwrap().is_empty());
        // Scope is free again.
        assert!(app.check_run_scope_overlap("other", &scope).is_ok());
    }

    #[test]
    fn disjoint_runs_run_concurrently() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_running_run(&app, "task-a", &["src"]);
        seed_running_run(&app, "task-b", &["docs"]);
        assert_eq!(app.active_runs().unwrap().len(), 2);
        // A further disjoint scope is allowed; an overlapping one is not.
        let disjoint: BTreeSet<String> = ["tests".to_string()].into_iter().collect();
        assert!(app.check_run_scope_overlap("c", &disjoint).is_ok());
        let overlap: BTreeSet<String> = ["src".to_string()].into_iter().collect();
        assert!(app.check_run_scope_overlap("c", &overlap).is_err());
    }

    #[test]
    fn run_replay_is_deterministic() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let a = seed_running_run(&app, "task-a", &["src"]);
        let s1 = app.replay_run(&a).unwrap();
        let s2 = app.replay_run(&a).unwrap();
        assert_eq!(s1.phase, s2.phase);
        assert_eq!(s1.write_allow, s2.write_allow);
        assert_eq!(s1.last_seq, s2.last_seq);
    }

    #[test]
    fn concurrent_runs_via_start_run_with_real_worktrees() {
        let dir = TempDir::new();
        // git repo + initial commit so `git worktree add HEAD` succeeds.
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@t"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("docs/readme.md"), "x\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "init"]);

        let app = ControlApp::init(dir.path()).unwrap();
        inprogress_task(&app, "task-src", &["src"]);
        inprogress_task(&app, "task-docs", &["docs"]);

        // Two disjoint runs both reach Running with real, distinct worktrees.
        let r_src = app.create_run("task-src", "omp").unwrap();
        app.start_run(&r_src).unwrap();
        let r_docs = app.create_run("task-docs", "omp").unwrap();
        app.start_run(&r_docs).unwrap();
        assert_eq!(app.active_runs().unwrap().len(), 2);
        assert!(dir
            .path()
            .join(".ctl/runs")
            .join(&r_src)
            .join("worktree")
            .exists());
        assert!(dir
            .path()
            .join(".ctl/runs")
            .join(&r_docs)
            .join("run-manifest.json")
            .exists());

        // A third run overlapping task-src's scope is rejected.
        inprogress_task(&app, "task-src2", &["src"]);
        let r_src2 = app.create_run("task-src2", "omp").unwrap();
        assert!(app
            .start_run(&r_src2)
            .unwrap_err()
            .to_string()
            .contains("scope conflict"));

        // Finishing the src run frees the scope; the blocked run can then start.
        app.finish_run(&r_src).unwrap();
        app.start_run(&r_src2).unwrap();
        assert_eq!(app.replay_run(&r_src2).unwrap().phase, RunPhase::Running);
    }

    // ── M6: run-scoped capability lease wiring (capability-lease-run-wiring-v1) ──

    #[test]
    fn start_run_grants_and_consumes_native_lease() {
        let dir = TempDir::new();
        let (app, run_id, _wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
        let run = app.replay_run(&run_id).unwrap();
        let lease = run.lease.as_ref().expect("native run lease present");
        assert_eq!(lease.status, crate::domain::lease::LeaseStatus::Active);
        assert_eq!(lease.max_uses, RUN_LEASE_MAX_USES);
        assert_eq!(lease.ttl_seconds, RUN_LEASE_TTL_SECONDS);
        // Start consumes exactly one use.
        assert_eq!(lease.remaining_uses, RUN_LEASE_MAX_USES - 1);
        assert_eq!(lease.task_id, "task-src");
        assert_eq!(lease.adapter, "omp");
        assert_eq!(lease.scopes, run.write_allow);
        assert_eq!(run.lease_id.as_deref(), Some(lease.lease_id.as_str()));

        // Manifest carries the same lease_id.
        let manifest = std::fs::read_to_string(
            dir.path()
                .join(".ctl/runs")
                .join(&run_id)
                .join("run-manifest.json"),
        )
        .unwrap();
        assert!(manifest.contains(&lease.lease_id));

        // run.json projection reports structured (non-prose) lease fields.
        let run_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".ctl/runs").join(&run_id).join("run.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(run_json["lease_status"], "ACTIVE");
        assert_eq!(run_json["lease_compat"], "native");
        assert_eq!(run_json["remaining_uses"], RUN_LEASE_MAX_USES - 1);
    }

    #[test]
    fn overlap_rejected_emits_no_lease_event() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let _a = seed_running_run(&app, "task-a", &["src"]); // Running on src
        inprogress_task(&app, "task-b", &["src"]);
        let b = app.create_run("task-b", "omp").unwrap();
        assert!(app
            .start_run(&b)
            .unwrap_err()
            .to_string()
            .contains("scope conflict"));
        let run = app.replay_run(&b).unwrap();
        assert_eq!(run.phase, RunPhase::Queued);
        assert!(
            run.lease.is_none(),
            "a rejected start must not grant a lease"
        );
        // Only run_created is on the ledger — no lease event leaked.
        let events =
            std::fs::read_to_string(dir.path().join(".ctl/runs").join(&b).join("events.jsonl"))
                .unwrap();
        assert_eq!(
            events.lines().filter(|l| !l.trim().is_empty()).count(),
            1,
            "rejected start left extra events: {events}"
        );
        assert!(!events.contains("lease_created"));
    }

    #[test]
    fn second_start_run_does_not_double_consume() {
        let dir = TempDir::new();
        let (app, run_id, _wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
        let before = app
            .replay_run(&run_id)
            .unwrap()
            .lease
            .unwrap()
            .remaining_uses;
        assert!(app
            .start_run(&run_id)
            .unwrap_err()
            .to_string()
            .contains("Queued"));
        let after = app
            .replay_run(&run_id)
            .unwrap()
            .lease
            .unwrap()
            .remaining_uses;
        assert_eq!(before, after, "rejected re-start must not consume a use");
        assert_eq!(after, RUN_LEASE_MAX_USES - 1);
    }

    #[test]
    fn finish_revokes_lease_and_unblocks_overlapping_run() {
        let dir = TempDir::new();
        let (app, r_src, _wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
        // An overlapping run is blocked while the first holds the scope.
        inprogress_task(&app, "task-src2", &["src"]);
        let r2 = app.create_run("task-src2", "omp").unwrap();
        assert!(app
            .start_run(&r2)
            .unwrap_err()
            .to_string()
            .contains("scope conflict"));
        // Finishing the first run revokes its lease and frees the scope.
        app.finish_run(&r_src).unwrap();
        let first = app.replay_run(&r_src).unwrap();
        let first_lease = first.lease.clone().unwrap();
        assert_eq!(
            first_lease.status,
            crate::domain::lease::LeaseStatus::Revoked
        );
        // The previously-blocked run can now start and gets its OWN active lease.
        app.start_run(&r2).unwrap();
        let second = app.replay_run(&r2).unwrap();
        assert_eq!(second.phase, RunPhase::Running);
        let l2 = second.lease.unwrap();
        assert_eq!(l2.status, crate::domain::lease::LeaseStatus::Active);
        assert_eq!(l2.remaining_uses, RUN_LEASE_MAX_USES - 1);
        assert_ne!(l2.lease_id, first_lease.lease_id);
    }

    #[test]
    fn recover_reports_unknown_lease_for_legacy_run() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // seed_running_run emits run_started with an opaque lease_id and NO
        // lease_created — a slice-1 (pre-lease) run.
        seed_running_run(&app, "t", &["src"]);
        let report = app.recover_report().unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].lease_status, "UNKNOWN");
        assert_eq!(report[0].lease_compat, "pre_lease_run");
        assert_eq!(report[0].remaining_uses, None);
        assert_eq!(report[0].lease_id.as_deref(), Some("lease-seed"));
        assert!(!report[0].lease_nonactive);
    }

    #[test]
    fn partial_start_run_detected_read_only() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // Hand-build a crash mid-start: run_created + lease_created + lease_used,
        // but no run_started (process died before the start committed).
        let run_id = generate_uuid();
        let created = app
            .build_run_event(
                &run_id,
                "run_created",
                serde_json::json!({
                    "task_id": "t", "adapter": "omp",
                    "write_allow": ["src"], "write_deny": [], "gates": ["cargo_check"],
                }),
            )
            .unwrap();
        app.append_run_event(&run_id, created).unwrap();
        let lc = app
            .build_run_event(
                &run_id,
                "lease_created",
                serde_json::json!({
                    "lease_id": "L", "run_id": run_id.clone(), "resource_path": "src",
                    "action": "write", "ttl_seconds": 3600, "max_uses": 100,
                    "task_id": "t", "adapter": "omp", "scopes": ["src"],
                }),
            )
            .unwrap();
        app.append_run_event(&run_id, lc).unwrap();
        let lu = app
            .build_run_event(&run_id, "lease_used", serde_json::json!({"lease_id": "L"}))
            .unwrap();
        app.append_run_event(&run_id, lu).unwrap();

        let partials = app.partial_start_runs().unwrap();
        assert_eq!(partials.len(), 1);
        assert_eq!(partials[0]["run_id"].as_str(), Some(run_id.as_str()));
        assert_eq!(partials[0]["lease_status"], "ACTIVE");
        // Still Queued → absent from the Running-only recover report.
        assert!(app.recover_report().unwrap().is_empty());
        // The read-only scan appended nothing.
        assert_eq!(app.replay_run(&run_id).unwrap().last_seq, 3);
    }

    #[test]
    fn running_run_with_revoked_lease_flagged_nonactive() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let run_id = generate_uuid();
        for (etype, payload) in [
            (
                "run_created",
                serde_json::json!({
                    "task_id": "t", "adapter": "omp",
                    "write_allow": ["src"], "write_deny": [], "gates": ["cargo_check"],
                }),
            ),
            (
                "lease_created",
                serde_json::json!({
                    "lease_id": "L", "run_id": run_id.clone(), "resource_path": "src",
                    "action": "write", "ttl_seconds": 3600, "max_uses": 100,
                    "task_id": "t", "adapter": "omp", "scopes": ["src"],
                }),
            ),
            ("lease_used", serde_json::json!({"lease_id": "L"})),
            (
                "run_started",
                serde_json::json!({
                    "worktree_path": format!(".ctl/runs/{}/worktree", run_id),
                    "lease_id": "L",
                }),
            ),
            ("lease_revoked", serde_json::json!({"lease_id": "L"})),
        ] {
            let e = app.build_run_event(&run_id, etype, payload).unwrap();
            app.append_run_event(&run_id, e).unwrap();
        }
        let report = app.recover_report().unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].lease_status, "REVOKED");
        assert!(report[0].lease_nonactive);
    }

    #[test]
    fn expire_stale_approvals_records_approval_expired_and_is_idempotent() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        inprogress_task(&app, "t", &["src"]);

        // Request and grant an approval, but backdate the grant far past its TTL
        // so the wall-clock expiry check fires deterministically.
        let req = app
            .approval_request(
                "t",
                "high-risk edit",
                serde_json::json!({ "high_risk_files": ["src/x.rs"] }),
                1,
            )
            .unwrap();
        let request_id = req.payload["request_id"].as_str().unwrap().to_string();

        let mut granted = app
            .build_event(
                "t",
                "approval_granted",
                serde_json::json!({ "request_id": request_id }),
            )
            .unwrap();
        granted.occurred_at = "2020-01-01T00:00:00Z".to_string();
        app.validate_and_append(&granted).unwrap();
        app.rebuild_task_view("t").unwrap();

        // Precondition: the approval reads as Granted before expiry runs.
        let before = app.replay_task("t").unwrap();
        assert_eq!(
            before.pending_approvals[&request_id].status,
            crate::domain::approval::ApprovalStatus::Granted
        );
        let seq_before = before.last_seq;

        // Expiry records an explicit approval_expired event and transitions state.
        app.expire_stale_approvals("t").unwrap();
        let after = app.replay_task("t").unwrap();
        assert_eq!(
            after.pending_approvals[&request_id].status,
            crate::domain::approval::ApprovalStatus::Expired
        );
        assert_eq!(after.last_seq, seq_before + 1);

        // Idempotent: a second pass appends nothing (the approval is no longer granted).
        app.expire_stale_approvals("t").unwrap();
        assert_eq!(app.replay_task("t").unwrap().last_seq, seq_before + 1);
    }

    // ── M6: crash recovery (slice 2) ────────────────────────────────────────

    #[test]
    fn recover_report_lists_only_running() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let r = seed_running_run(&app, "t", &["src"]); // Running
        inprogress_task(&app, "tq", &["docs"]);
        app.create_run("tq", "omp").unwrap(); // Queued, never started
        let report = app.recover_report().unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].run_id, r);
        // The seeded run points at a worktree that was never created → flagged
        // as inconsistent (a crash-recovery abort candidate).
        assert!(!report[0].worktree_exists);
    }

    #[test]
    fn recover_abort_frees_scope_and_drops_from_report() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let r = seed_running_run(&app, "t", &["src"]);
        let scope: BTreeSet<String> = ["src".to_string()].into_iter().collect();
        assert!(app.check_run_scope_overlap("x", &scope).is_err());
        app.abort_run(&r, "crash recovery").unwrap();
        assert_eq!(app.replay_run(&r).unwrap().phase, RunPhase::Aborted);
        assert!(app.check_run_scope_overlap("x", &scope).is_ok());
        assert!(app.recover_report().unwrap().is_empty());
        // Aborting an already-terminal run is rejected (no duplicate side effect).
        assert!(app.abort_run(&r, "again").is_err());
    }

    #[test]
    fn finish_drops_run_from_recover_report() {
        // run-finish-emit-v1: the production finish path (now reachable via
        // `ctl run finish`) drives a Running run to Completed and out of the open
        // run / recovery view — the B2 fix (a prod run can finally reach Finished).
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let r = seed_running_run(&app, "t", &["src"]);
        assert_eq!(
            app.recover_report().unwrap().len(),
            1,
            "a Running run shows as open before finish"
        );
        app.finish_run(&r).unwrap();
        assert_eq!(app.replay_run(&r).unwrap().phase, RunPhase::Completed);
        assert!(
            app.recover_report().unwrap().is_empty(),
            "a finished run is no longer open/stranded"
        );
        // Reducer guard: finishing an already-terminal run is rejected.
        assert!(
            app.finish_run(&r).is_err(),
            "only a Running run can be finished"
        );
    }

    #[test]
    fn finish_run_with_provenance_hashes_artifacts_and_records_host_attested_values() {
        // run-attestation-fields-v1: ctl sha256-hashes the supplied artifact and
        // records host-reported fields; absent fields stay unset.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let r = seed_running_run(&app, "t", &["src"]);
        let art = dir.path().join("instruction.txt");
        std::fs::write(&art, b"do the thing").unwrap();
        let prov = RunProvenanceInput {
            model: Some("claude-opus-4-8".into()),
            instruction_artifact: Some(art.to_string_lossy().into_owned()),
            exit_code: Some(0),
            ..Default::default()
        };
        app.finish_run_with_provenance(&r, &prov).unwrap();
        let state = app.replay_run(&r).unwrap();
        assert_eq!(state.phase, RunPhase::Completed);
        assert_eq!(state.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(state.exit_code, Some(0));
        // ctl recorded the artifact's sha256 (64 hex chars), never the path.
        let h = state
            .instruction_hash
            .as_deref()
            .expect("instruction hash recorded");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        // Unsupplied provenance is simply absent.
        assert!(state.provider.is_none());
        assert!(state.context_hash.is_none());
    }

    #[test]
    fn record_subagent_dispatch_records_host_attested_dispatch() {
        // subagent-dispatch-record-v1: the dispatch is appended to the task ledger
        // (passing envelope-schema validation) with the artifact ctl-hashed.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        inprogress_task(&app, "t", &["src"]);
        std::fs::write(dir.path().join("instruction.txt"), b"the instruction").unwrap();
        app.record_subagent_dispatch(
            "t",
            "designer",
            "opencode",
            Some("run-1"),
            Some("instruction.txt"),
            None,
            None,
        )
        .unwrap();
        let state = app.replay_task("t").unwrap();
        assert_eq!(state.dispatches.len(), 1);
        let d = &state.dispatches[0];
        assert_eq!(d.role, "designer");
        assert_eq!(d.adapter, "opencode");
        assert_eq!(d.parent_run.as_deref(), Some("run-1"));
        let instr = d.instruction.as_ref().expect("instruction recorded");
        assert_eq!(instr.hash.len(), 64);
        assert!(instr.hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(d.context.is_none() && d.output.is_none());
    }

    #[test]
    fn orphaned_worktrees_lists_terminal_run_leftover() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let r = seed_running_run(&app, "t", &["src"]);
        app.finish_run(&r).unwrap(); // terminal
                                     // Nothing on disk yet (seeded worktree was never created).
        assert!(app.orphaned_run_worktrees().unwrap().is_empty());
        // Simulate a leftover worktree dir for the now-terminal run.
        let wt = crate::infrastructure::workspace::run_worktree_path(dir.path(), &r);
        std::fs::create_dir_all(&wt).unwrap();
        let orphans = app.orphaned_run_worktrees().unwrap();
        assert!(orphans.iter().any(|o| o.contains(&r)), "got: {orphans:?}");
    }

    #[test]
    fn recover_abort_removes_real_worktree() {
        let dir = TempDir::new();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@t"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "init"]);

        let app = ControlApp::init(dir.path()).unwrap();
        inprogress_task(&app, "task-src", &["src"]);
        let r = app.create_run("task-src", "omp").unwrap();
        app.start_run(&r).unwrap();
        let wt = crate::infrastructure::workspace::run_worktree_path(dir.path(), &r);
        assert!(wt.exists());
        // A real, in-flight run is reported with a present worktree + manifest.
        let rep = app.recover_report().unwrap();
        assert!(rep
            .iter()
            .any(|s| s.run_id == r && s.worktree_exists && s.manifest_exists));
        // Recovery abort tears it down and frees the scope.
        app.abort_run(&r, "crash recovery").unwrap();
        assert!(!wt.exists());
        assert_eq!(app.replay_run(&r).unwrap().phase, RunPhase::Aborted);
        assert!(app.recover_report().unwrap().is_empty());
    }

    // ── M6: merge-candidate / recovery (slice 3) ────────────────────────────

    /// git repo (tracked src/lib.rs + docs/readme.md) + an InProgress task and a
    /// started run with a real worktree. Returns (app, run_id, worktree_path).
    fn git_repo_with_started_run(
        dir: &Path,
        task: &str,
        scope: &[&str],
    ) -> (ControlApp, String, PathBuf) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.join("docs/readme.md"), "x\n").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", "init"]);

        let app = ControlApp::init(dir).unwrap();
        inprogress_task(&app, task, scope);
        let run_id = app.create_run(task, "omp").unwrap();
        app.start_run(&run_id).unwrap();
        let wt = crate::infrastructure::workspace::run_worktree_path(dir, &run_id);
        (app, run_id, wt)
    }

    #[test]
    fn run_merge_candidate_clean_is_mergeable() {
        let dir = TempDir::new();
        let (app, run_id, wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
        // Edit a tracked, in-scope file inside the run's isolated worktree.
        std::fs::write(wt.join("src/lib.rs"), "fn a() { /* run edit */ }\n").unwrap();
        let v = app.run_merge_candidate(&run_id).unwrap();
        assert_eq!(v["mergeable"], true, "verdict: {v}");
        assert!(v["touched_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "src/lib.rs"));
        assert!(v["recovery"].as_array().unwrap().is_empty());
    }

    #[test]
    fn run_merge_candidate_dirty_main_blocks_with_recovery() {
        let dir = TempDir::new();
        let (app, run_id, wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
        std::fs::write(wt.join("src/lib.rs"), "fn a() { /* run edit */ }\n").unwrap();
        // The main workspace has its own uncommitted edit to the same file.
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "fn a() { /* main edit */ }\n",
        )
        .unwrap();
        let v = app.run_merge_candidate(&run_id).unwrap();
        assert_eq!(v["mergeable"], false, "verdict: {v}");
        assert!(v["workspace_conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "src/lib.rs"));
        let rec = v["recovery"].as_array().unwrap();
        assert!(rec
            .iter()
            .any(|r| r["category"] == "dirty_main_workspace" && r["action"].is_string()));
    }

    #[test]
    fn run_merge_candidate_out_of_scope_and_cross_run() {
        let dir = TempDir::new();
        // Run A scoped to src; a concurrent run B scoped to docs.
        let (app, run_a, wt_a) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
        inprogress_task(&app, "task-docs", &["docs"]);
        let run_b = app.create_run("task-docs", "omp").unwrap();
        app.start_run(&run_b).unwrap();
        // Run A writes into docs/ — outside its own scope AND into B's territory.
        std::fs::write(wt_a.join("docs/readme.md"), "y\n").unwrap();
        let v = app.run_merge_candidate(&run_a).unwrap();
        assert_eq!(v["mergeable"], false, "verdict: {v}");
        assert!(v["out_of_scope"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "docs/readme.md"));
        let crc = v["cross_run_conflicts"].as_array().unwrap();
        assert!(crc.iter().any(|c| c["conflicting_run"] == run_b.as_str()));
        let cats: Vec<&str> = v["recovery"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["category"].as_str().unwrap())
            .collect();
        assert!(cats.contains(&"out_of_scope") && cats.contains(&"cross_run_conflict"));
    }

    #[test]
    fn run_merge_candidate_missing_worktree_errors() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // Seeded run points at a worktree that was never created.
        let run_id = seed_running_run(&app, "t", &["src"]);
        let err = app.run_merge_candidate(&run_id).unwrap_err().to_string();
        assert!(err.contains("worktree is missing"), "got: {err}");
        assert!(err.contains("recover"), "should point at recovery: {err}");
    }

    // ── BS-provenance V1 ──────────────────────────────────────────────────
    //
    // Record-only brainstorm artifact provenance: hashes, source, skip reason,
    // and staleness — without gating or claiming thinking quality/independence.

    /// Create a Planning task and write two originator artifacts under the
    /// project's `.ctl/brainstorms/<bs>/`. Returns their project-relative paths.
    fn seed_brainstorm_task(app: &ControlApp, id: &str, bs: &str) -> (String, String) {
        let scope = vec!["src".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "bs provenance",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
        let dir = app.project_root.join(".ctl").join("brainstorms").join(bs);
        std::fs::create_dir_all(&dir).unwrap();
        let div = format!(".ctl/brainstorms/{bs}/divergence.json");
        let conv = format!(".ctl/brainstorms/{bs}/convergence.json");
        std::fs::write(app.project_root.join(&div), b"{\"candidates\":[1,2,3]}").unwrap();
        std::fs::write(app.project_root.join(&conv), b"{\"proposal\":\"x\"}").unwrap();
        (div, conv)
    }

    #[test]
    fn bs_provenance_records_artifact_hash() {
        // Behavior 1: the recorded reference carries the SHA-256 of the artifact.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let (div, conv) = seed_brainstorm_task(&app, "t", "BS-001");
        app.record_brainstorm_artifacts("t", "BS-001", &div, Some(&conv), None)
            .unwrap();
        let state = app.get_status("t").unwrap();
        let reference = state.brainstorm_ref.expect("reference recorded");
        let expected = hash_file(&app.project_root.join(&div)).unwrap();
        assert_eq!(reference.divergence.unwrap().hash, expected);
        // A recorded reference is L0 content and never asserts independence.
        assert_eq!(reference.trust_level, "content_l0");
        assert_eq!(reference.critic_independence, "unattested");
    }

    #[test]
    fn bs_provenance_detects_stale_artifact() {
        // Behavior 2: editing or deleting an artifact makes the reference stale.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let (div, conv) = seed_brainstorm_task(&app, "t", "BS-002");
        app.record_brainstorm_artifacts("t", "BS-002", &div, Some(&conv), None)
            .unwrap();
        let state = app.get_status("t").unwrap();
        // Fresh immediately after recording.
        let view = app.brainstorm_provenance_view(&state).unwrap();
        assert!(!view.divergence.as_ref().unwrap().stale);
        // Edited on disk → present but stale.
        std::fs::write(app.project_root.join(&div), b"MUTATED").unwrap();
        let edited = app.brainstorm_provenance_view(&state).unwrap();
        let d = edited.divergence.unwrap();
        assert!(
            d.present && d.stale,
            "edited artifact must read present+stale"
        );
        // Deleted → absent from disk and stale.
        std::fs::remove_file(app.project_root.join(&conv)).unwrap();
        let removed = app.brainstorm_provenance_view(&state).unwrap();
        let c = removed.convergence.unwrap();
        assert!(
            !c.present && c.stale,
            "deleted artifact must read missing+stale"
        );
    }

    #[test]
    fn bs_provenance_task_references_brainstorm() {
        // Behavior 3: a task can reference a brainstorm (with an unattested run).
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let (div, conv) = seed_brainstorm_task(&app, "t", "BS-003");
        app.record_brainstorm_artifacts("t", "BS-003", &div, Some(&conv), Some("run-9"))
            .unwrap();
        let state = app.get_status("t").unwrap();
        let reference = state.brainstorm_ref.clone().unwrap();
        assert_eq!(reference.id, "BS-003");
        assert_eq!(reference.source_run_id.as_deref(), Some("run-9"));
        // A recorded source run is a claim, never an attestation.
        let view = app.brainstorm_provenance_view(&state).unwrap();
        assert!(!view.source_run_attested);
    }

    #[test]
    fn bs_provenance_critic_attached_independently() {
        // Behavior 4: a critic artifact can be attached as a separate invocation.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let (div, conv) = seed_brainstorm_task(&app, "t", "BS-004");
        app.record_brainstorm_artifacts("t", "BS-004", &div, Some(&conv), None)
            .unwrap();
        let critic = ".ctl/brainstorms/BS-004/critic.json";
        std::fs::write(app.project_root.join(critic), b"{\"challenge\":\"...\"}").unwrap();
        app.attach_brainstorm_critic("t", "BS-004", critic, None)
            .unwrap();
        let reference = app.get_status("t").unwrap().brainstorm_ref.unwrap();
        assert_eq!(reference.critic_disposition.as_str(), "present");
        assert!(reference.critic.is_some());
        // Even attached, independence is never asserted in V1.
        assert_eq!(reference.critic_independence, "unattested");
    }

    #[test]
    fn bs_provenance_skip_records_reason_and_actor() {
        // Behaviors 5 + 6: with no critic, a skip records reason + decider, and
        // the recording actor is captured in the canonical event.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let (div, conv) = seed_brainstorm_task(&app, "t", "BS-005");
        app.record_brainstorm_artifacts("t", "BS-005", &div, Some(&conv), None)
            .unwrap();
        ControlApp::open(&app.project_root, false)
            .unwrap()
            .with_actor("agent-7")
            .skip_brainstorm_critic("t", "BS-005", "explicit_user_skip", Some("human"), None)
            .unwrap();
        let reference = app.get_status("t").unwrap().brainstorm_ref.unwrap();
        assert_eq!(reference.critic_disposition.as_str(), "skipped");
        assert_eq!(reference.skip_reason.as_deref(), Some("explicit_user_skip"));
        assert_eq!(reference.skip_decided_by.as_deref(), Some("human"));
        // The actor that recorded the skip is in the ledger event itself.
        let events = app.store.read_for_task("t").unwrap();
        let skip = events
            .iter()
            .find(|e| e.event_type == "brainstorm_skipped")
            .unwrap();
        assert_eq!(skip.actor, "agent-7");
    }

    #[test]
    fn bs_provenance_bare_file_is_not_canonical_provenance() {
        // Behavior 10: a brainstorm file on disk is never auto-promoted to
        // canonical provenance — only an explicit recording event creates it,
        // and recording refuses a path that is not actually present.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let (_div, _conv) = seed_brainstorm_task(&app, "t", "BS-010");
        // Artifacts exist on disk, but nothing was recorded.
        let state = app.get_status("t").unwrap();
        assert!(state.brainstorm_ref.is_none());
        assert!(app.brainstorm_provenance_view(&state).is_none());
        // Recording a non-existent artifact is refused.
        let err = app
            .record_brainstorm_artifacts(
                "t",
                "BS-010",
                ".ctl/brainstorms/BS-010/missing.json",
                None,
                None,
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found"), "got: {err}");
    }

    // ── Uncertainty Ledger V1 ──
    // Record-and-disclose unknowns: open lifecycle, resolved-needs-evidence,
    // terminal-is-terminal, and evidence freshness — without gating or verdict.

    fn seed_uncertainty_task(app: &ControlApp, id: &str) {
        let scope = vec!["src".to_string()];
        app.create_task(
            id,
            CreateTaskInput {
                objective: "uncertainty ledger",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
    }

    #[test]
    fn uncertainty_cannot_be_recorded_after_terminal() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        // Allowed while the task is live.
        app.record_uncertainty("t", "U-1", "open question", None)
            .unwrap();
        // Drive to a terminal phase.
        app.cancel_task("t").unwrap();
        // The command layer refuses to grow a terminal task's unknown set.
        let err = app
            .record_uncertainty("t", "U-2", "too late", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("terminal task cannot record"), "got: {err}");
        // Reducer stays permissive: a directly-appended post-terminal event still
        // replays (append-only history is never re-rejected on replay).
        let event = app
            .build_event(
                "t",
                "uncertainty_recorded",
                serde_json::json!({
                    "uncertainty_id": "U-3",
                    "statement": "committed pre-rule post-terminal record",
                    "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
                }),
            )
            .unwrap();
        app.validate_and_append(&event).unwrap();
        let state = app.replay_task("t").unwrap();
        assert!(state.uncertainties.iter().any(|u| u.id == "U-3"));
    }

    #[test]
    fn uncertainty_recorded_is_open_l0_content() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "does the gate kill the tree?", Some("review"))
            .unwrap();
        let state = app.get_status("t").unwrap();
        assert_eq!(state.uncertainties.len(), 1);
        assert_eq!(state.uncertainties[0].status.as_str(), "open");
        let view = app.uncertainty_ledger_view(&state).unwrap();
        assert_eq!((view.open, view.resolved), (1, 0));
        // Recording an unknown never raises trust above bare content.
        assert_eq!(view.trust_level, "content_l0");
    }

    #[test]
    fn uncertainty_duplicate_id_rejected() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "first", None).unwrap();
        let err = app
            .record_uncertainty("t", "U-1", "again", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already recorded"), "got: {err}");
    }

    #[test]
    fn uncertainty_resolved_requires_evidence() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "needs proof", None)
            .unwrap();
        let err = app
            .record_uncertainty_disposition("t", "U-1", "resolved", None, None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("'resolved' requires evidence"), "got: {err}");
    }

    #[test]
    fn uncertainty_resolved_binds_hashed_evidence_and_tracks_freshness() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "proven?", None).unwrap();
        let ev = app.project_root.join("src").join("ev.txt");
        std::fs::write(&ev, b"PASS exit=0").unwrap();
        app.record_uncertainty_disposition("t", "U-1", "resolved", Some("src/ev.txt"), None, None)
            .unwrap();
        let state = app.get_status("t").unwrap();
        let u = &state.uncertainties[0];
        assert_eq!(u.status.as_str(), "resolved");
        // ctl computes the hash from the path; it equals hashing the file directly.
        assert_eq!(
            u.evidence_ref.as_ref().unwrap().hash,
            hash_file(&ev).unwrap()
        );
        // Fresh immediately; never attested.
        let view = app.uncertainty_ledger_view(&state).unwrap();
        let evidence = view.items[0].evidence.as_ref().unwrap();
        assert_eq!(evidence.freshness.as_str(), "CURRENT");
        assert!(!evidence.attested);
        // Edited → STALE; deleted → ABSENT.
        std::fs::write(&ev, b"MUTATED").unwrap();
        let edited = app.uncertainty_ledger_view(&state).unwrap();
        assert_eq!(
            edited.items[0]
                .evidence
                .as_ref()
                .unwrap()
                .freshness
                .as_str(),
            "STALE"
        );
        std::fs::remove_file(&ev).unwrap();
        let removed = app.uncertainty_ledger_view(&state).unwrap();
        assert_eq!(
            removed.items[0]
                .evidence
                .as_ref()
                .unwrap()
                .freshness
                .as_str(),
            "ABSENT"
        );
    }

    #[test]
    fn uncertainty_assumption_rejects_evidence_stays_unresolved() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "assume it", None)
            .unwrap();
        std::fs::write(app.project_root.join("src").join("ev.txt"), b"x").unwrap();
        let err = app
            .record_uncertainty_disposition(
                "t",
                "U-1",
                "accepted_as_assumption",
                Some("src/ev.txt"),
                None,
                None,
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("must not carry evidence"), "got: {err}");
        // Without evidence it stays visibly unresolved by external evidence.
        app.record_uncertainty_disposition(
            "t",
            "U-1",
            "accepted_as_assumption",
            None,
            None,
            Some("ship"),
        )
        .unwrap();
        let state = app.get_status("t").unwrap();
        assert_eq!(
            state.uncertainties[0].status.as_str(),
            "accepted_as_assumption"
        );
        assert!(state.uncertainties[0].evidence_ref.is_none());
    }

    #[test]
    fn uncertainty_invalidated_requires_reason_rejects_evidence() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "moot?", None).unwrap();
        let no_reason = app
            .record_uncertainty_disposition("t", "U-1", "invalidated", None, None, None)
            .unwrap_err()
            .to_string();
        assert!(no_reason.contains("requires a reason"), "got: {no_reason}");
        app.record_uncertainty_disposition(
            "t",
            "U-1",
            "invalidated",
            None,
            None,
            Some("premise gone"),
        )
        .unwrap();
        let state = app.get_status("t").unwrap();
        assert_eq!(state.uncertainties[0].status.as_str(), "invalidated");
        assert_eq!(
            state.uncertainties[0].reason.as_deref(),
            Some("premise gone")
        );
    }

    #[test]
    fn uncertainty_disposition_is_terminal() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "assume then upgrade?", None)
            .unwrap();
        app.record_uncertainty_disposition("t", "U-1", "accepted_as_assumption", None, None, None)
            .unwrap();
        std::fs::write(app.project_root.join("src").join("ev.txt"), b"x").unwrap();
        // Silent upgrade assumption → resolved is impossible: terminal-is-terminal.
        let err = app
            .record_uncertainty_disposition("t", "U-1", "resolved", Some("src/ev.txt"), None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("is terminal"), "got: {err}");
    }

    #[test]
    fn uncertainty_disposition_unknown_id_rejected() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        let err = app
            .record_uncertainty_disposition("t", "U-X", "accepted_as_assumption", None, None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown uncertainty"), "got: {err}");
    }

    // ── Oracle V1: first-class, oracle-typed evidence ──
    // Evidence becomes a recorded object carrying oracle_kind; a `resolved` can
    // reference it by id. The control layer discloses the oracle kind; it never
    // vouches for the claim. `model` is advisory; legacy inline replay is preserved.

    #[test]
    fn evidence_recorded_then_resolve_via_ref_binds_oracle_kind() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "proven by a test?", None)
            .unwrap();
        let ev = app.project_root.join("src").join("test-log.txt");
        std::fs::write(&ev, b"running 1 test ... ok").unwrap();
        app.record_evidence(
            "t",
            "E-1",
            "test",
            Some("cargo test oracle"),
            "src/test-log.txt",
        )
        .unwrap();
        app.record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-1"), None)
            .unwrap();
        let state = app.get_status("t").unwrap();
        let u = &state.uncertainties[0];
        assert_eq!(u.status.as_str(), "resolved");
        assert_eq!(u.evidence_id.as_deref(), Some("E-1"));
        assert_eq!(u.oracle_kind.unwrap().as_str(), "test");
        // The evidence's artifact is copied onto the uncertainty so freshness resolves
        // uniformly with the legacy inline path.
        assert_eq!(
            u.evidence_ref.as_ref().unwrap().hash,
            hash_file(&ev).unwrap()
        );
        // recorded_by is the envelope actor, never a separate forgeable field.
        assert_eq!(state.evidences[0].recorded_by, app.actor);
        let view = app.uncertainty_ledger_view(&state).unwrap();
        assert_eq!(view.oracle_sources.test, 1);
        assert_eq!(view.items[0].oracle_kind.as_deref(), Some("test"));
        assert!(!view.items[0].advisory);
    }

    #[test]
    fn resolve_via_unknown_evidence_ref_rejected() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "needs proof", None)
            .unwrap();
        let err = app
            .record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-404"), None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("does not reference a recorded evidence"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_with_both_evidence_ref_and_inline_rejected() {
        // Critic C1: a resolve must never carry both evidence shapes. The app-layer
        // guard rejects before any hashing; the schema and reducer also forbid it.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "double-bound?", None)
            .unwrap();
        let ev = app.project_root.join("src").join("ev.txt");
        std::fs::write(&ev, b"x").unwrap();
        app.record_evidence("t", "E-1", "deterministic", None, "src/ev.txt")
            .unwrap();
        let err = app
            .record_uncertainty_disposition(
                "t",
                "U-1",
                "resolved",
                Some("src/ev.txt"),
                Some("E-1"),
                None,
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("never both"), "got: {err}");
    }

    #[test]
    fn evidence_duplicate_id_rejected() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        let ev = app.project_root.join("src").join("ev.txt");
        std::fs::write(&ev, b"x").unwrap();
        app.record_evidence("t", "E-1", "deterministic", None, "src/ev.txt")
            .unwrap();
        let err = app
            .record_evidence("t", "E-1", "test", None, "src/ev.txt")
            .unwrap_err()
            .to_string();
        assert!(err.contains("already recorded"), "got: {err}");
    }

    #[test]
    fn unknown_oracle_kind_rejected() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        let ev = app.project_root.join("src").join("ev.txt");
        std::fs::write(&ev, b"x").unwrap();
        // Schema rejects a value outside the fixed enum before the reducer runs.
        let err = app
            .record_evidence("t", "E-1", "vibes", None, "src/ev.txt")
            .unwrap_err()
            .to_string();
        assert!(
            err.to_lowercase().contains("oracle_kind") || err.contains("schema"),
            "got: {err}"
        );
    }

    #[test]
    fn model_oracle_cannot_resolve_but_is_disclosed_advisory() {
        // EPISTEMIC_CONTROL §5.1: a model oracle is advisory — never external proof, so
        // it must not RESOLVE an uncertainty. It may still be recorded and discloses on
        // its own ORACLE SOURCES line. The command layer rejects a model-backed resolve.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "did a model say so?", None)
            .unwrap();
        let ev = app.project_root.join("src").join("model-note.md");
        std::fs::write(&ev, b"the model believes X").unwrap();
        app.record_evidence(
            "t",
            "E-1",
            "model",
            Some("BS-UO1 critic"),
            "src/model-note.md",
        )
        .unwrap();
        // The model-backed resolve is rejected at the command layer.
        let err = app
            .record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-1"), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("model"), "unexpected error: {err}");
        assert!(
            err.contains("advisory") || err.contains("external proof"),
            "unexpected error: {err}"
        );
        // The uncertainty stays open; the model evidence is still recorded + disclosed.
        let state = app.get_status("t").unwrap();
        assert_eq!(state.uncertainties[0].status.as_str(), "open");
        let view = app.uncertainty_ledger_view(&state).unwrap();
        assert_eq!(view.oracle_sources.model_advisory, 1);
        assert!(!view.items[0].advisory);
    }

    #[test]
    fn legacy_model_backed_resolve_still_replays_advisory() {
        // Preserve legacy replay: a pre-rule stream that resolved an uncertainty via a
        // model evidence_ref must still replay byte-identically (the reducer stays
        // permissive). Only the command layer forbids NEW model resolves, so this test
        // appends the resolve directly to simulate a committed pre-rule event. Replay
        // keeps it Resolved and the disclosure marks it ADVISORY (honest, never proof).
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "model-resolved long ago", None)
            .unwrap();
        let ev = app.project_root.join("src").join("model-note.md");
        std::fs::write(&ev, b"the model believes X").unwrap();
        app.record_evidence("t", "E-1", "model", None, "src/model-note.md")
            .unwrap();
        // Bypass the command-layer guard to mimic a stream written before the rule.
        let event = app
            .build_event(
                "t",
                "uncertainty_disposition_recorded",
                serde_json::json!({
                    "uncertainty_id": "U-1",
                    "disposition": "resolved",
                    "evidence_ref": "E-1",
                    "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
                }),
            )
            .unwrap();
        app.validate_and_append(&event).unwrap();
        // The reducer accepted it on append AND on full replay.
        let state = app.replay_task("t").unwrap();
        let u = &state.uncertainties[0];
        assert_eq!(u.status.as_str(), "resolved");
        assert_eq!(u.oracle_kind, Some(crate::domain::task::OracleKind::Model));
        let view = app.uncertainty_ledger_view(&state).unwrap();
        assert!(view.items[0].advisory);
        assert_eq!(view.items[0].oracle_kind.as_deref(), Some("model"));
    }

    #[test]
    fn research_artifact_usable_as_evidence_source() {
        // §六: a research/spike artifact can be referenced as the file-backed evidence
        // an uncertainty is resolved against (oracle_kind labels it; content stays L0).
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "what did the spike find?", None)
            .unwrap();
        let findings = app.project_root.join("src").join("findings.md");
        std::fs::write(&findings, b"# findings\nthe API is idempotent").unwrap();
        app.record_evidence(
            "t",
            "E-1",
            "external_authority",
            Some("research-spike findings.md"),
            "src/findings.md",
        )
        .unwrap();
        app.record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-1"), None)
            .unwrap();
        let state = app.get_status("t").unwrap();
        assert_eq!(state.uncertainties[0].status.as_str(), "resolved");
        assert_eq!(
            state.evidences[0].source_ref.as_deref(),
            Some("research-spike findings.md")
        );
    }

    #[test]
    fn legacy_inline_resolve_still_replays_after_oracle_v1() {
        // Backward compatibility: the legacy inline (evidence_path+evidence_hash)
        // resolve shape — with no evidence_ref and no recorded evidence object — must
        // still replay unchanged. oracle_kind is unknown (None) for legacy resolves.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "legacy?", None).unwrap();
        let ev = app.project_root.join("src").join("legacy.txt");
        std::fs::write(&ev, b"legacy evidence").unwrap();
        app.record_uncertainty_disposition(
            "t",
            "U-1",
            "resolved",
            Some("src/legacy.txt"),
            None,
            None,
        )
        .unwrap();
        // Replay from canonical events rebuilds the same state.
        let state = app.replay_task("t").unwrap();
        let u = &state.uncertainties[0];
        assert_eq!(u.status.as_str(), "resolved");
        assert!(u.evidence_id.is_none());
        assert!(u.oracle_kind.is_none());
        assert!(u.evidence_ref.is_some());
        // No recorded evidence object; ORACLE SOURCES is all-zero.
        assert!(state.evidences.is_empty());
        let view = app.uncertainty_ledger_view(&state).unwrap();
        assert_eq!(
            view.oracle_sources.deterministic + view.oracle_sources.test,
            0
        );
    }

    #[test]
    fn uncertainty_ledger_json_has_no_epistemic_verdict() {
        // §七.10: the JSON disclosure carries raw facts only — no verdict / score /
        // percentage / pass-fail roll-up of the epistemic dimension.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "t");
        app.record_uncertainty("t", "U-1", "open one", None)
            .unwrap();
        let ev = app.project_root.join("src").join("m.md");
        std::fs::write(&ev, b"model says").unwrap();
        app.record_evidence("t", "E-1", "model", None, "src/m.md")
            .unwrap();
        let state = app.get_status("t").unwrap();
        let view = app.uncertainty_ledger_view(&state).unwrap();
        let json = serde_json::to_string(&view).unwrap().to_lowercase();
        assert!(!json.contains("verdict"));
        assert!(!json.contains("score"));
        assert!(!json.contains("confidence"));
        assert!(!json.contains('%'));
        // It does carry the honest texture: the model oracle and its advisory flag.
        assert!(json.contains("model_advisory"));
        assert!(json.contains("\"advisory\""));
    }

    // ── Research/Spike V1 ──
    // A research task completes by producing evidence + uncertainty outcomes, not
    // code. Kind is immutable; completion never requires fewer unknowns.

    fn seed_research_task(app: &ControlApp, id: &str) {
        let scope = vec!["src".to_string()];
        app.create_task_with_kind(
            id,
            CreateTaskInput {
                objective: "spike",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
            TaskKind::Research,
        )
        .unwrap();
    }

    #[test]
    fn task_kind_defaults_implementation_and_is_set_for_research() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "impl"); // uses create_task (default)
        seed_research_task(&app, "res");
        assert_eq!(
            app.get_status("impl").unwrap().task_kind,
            TaskKind::Implementation
        );
        assert_eq!(app.get_status("res").unwrap().task_kind, TaskKind::Research);
    }

    #[test]
    fn task_kind_is_immutable_through_revise() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_research_task(&app, "res");
        app.revise_task(
            "res",
            ReviseTaskInput {
                objective: Some("revised spike"),
                read_scope: None,
                write_allow: None,
                write_deny: None,
                risk_triggers: None,
                gates: None,
                depends_on: None,
            },
        )
        .unwrap();
        // Revise changed the objective but never the kind.
        let state = app.get_status("res").unwrap();
        assert_eq!(state.objective.as_deref(), Some("revised spike"));
        assert_eq!(state.task_kind, TaskKind::Research);
    }

    #[test]
    fn research_artifact_records_hash_kind_and_freshness() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_research_task(&app, "res");
        let path = app.project_root.join("src").join("findings.md");
        std::fs::write(&path, b"# findings").unwrap();
        app.record_research_artifact("res", "src/findings.md", "findings", Some("run-7"))
            .unwrap();
        let state = app.get_status("res").unwrap();
        assert_eq!(state.research_artifacts.len(), 1);
        let a = &state.research_artifacts[0];
        assert_eq!(a.artifact_ref.hash, hash_file(&path).unwrap());
        assert_eq!(a.kind.as_str(), "findings");
        // View: fresh + never attested; mutate → STALE.
        let view = app.research_output_view("res").unwrap().unwrap();
        assert_eq!(view.artifacts_recorded, 1);
        assert_eq!(view.artifacts[0].freshness.as_str(), "CURRENT");
        assert!(!view.artifacts[0].source_run_attested);
        std::fs::write(&path, b"MUTATED").unwrap();
        let stale = app.research_output_view("res").unwrap().unwrap();
        assert_eq!(stale.artifacts[0].freshness.as_str(), "STALE");
    }

    #[test]
    fn research_artifact_unknown_kind_rejected() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_research_task(&app, "res");
        std::fs::write(app.project_root.join("src").join("x.md"), b"x").unwrap();
        let err = app
            .record_research_artifact("res", "src/x.md", "manifesto", None)
            .unwrap_err()
            .to_string();
        // Rejected by the schema enum (defense-in-depth before the reducer's own
        // unknown-kind guard); either way an unknown kind cannot be recorded.
        assert!(err.contains("artifact_kind"), "got: {err}");
    }

    #[test]
    fn research_output_none_for_implementation_and_tags_discovered_items() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "impl");
        assert!(app.research_output_view("impl").unwrap().is_none());

        seed_research_task(&app, "res");
        // Recorded in Planning (before start) → pre-start.
        app.record_uncertainty("res", "U-pre", "known before start", None)
            .unwrap();
        app.mark_ready("res").unwrap();
        app.start_task("res").unwrap();
        // Recorded after start → tagged recorded_after_start.
        app.record_uncertainty("res", "U-post", "surfaced during spike", None)
            .unwrap();
        let view = app.research_output_view("res").unwrap().unwrap();
        assert_eq!(view.uncertainties_opened, 2);
        let pre = view
            .uncertainties
            .iter()
            .find(|u| u.item.id == "U-pre")
            .unwrap();
        let post = view
            .uncertainties
            .iter()
            .find(|u| u.item.id == "U-post")
            .unwrap();
        assert!(
            !pre.recorded_after_start,
            "pre-start uncertainty must not be tagged"
        );
        assert!(
            post.recorded_after_start,
            "post-start uncertainty must be tagged"
        );
    }

    fn drive_research_to_review(app: &ControlApp, id: &str) {
        seed_research_task(app, id);
        app.mark_ready(id).unwrap();
        app.start_task(id).unwrap();
        app.submit_task(id).unwrap();
        app.record_gate(id, "cargo_check", true, "ok").unwrap();
        ControlApp::open(&app.project_root, false)
            .unwrap()
            .with_actor("reviewer")
            .record_completion_audit(id, true, None)
            .unwrap();
    }

    #[test]
    fn research_finish_requires_artifact_then_uncertainty_then_succeeds() {
        // Non-git temp dir → tree/commit interlocks skipped, isolating the
        // research-specific completion checks (which run after the M-f audit gate).
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_research_to_review(&app, "res");
        // No artifact yet.
        let e1 = app.finish_task("res").unwrap_err().to_string();
        assert!(e1.contains("research artifact"), "got: {e1}");
        // Artifact, but no uncertainty outcome.
        std::fs::write(app.project_root.join("src").join("f.md"), b"f").unwrap();
        app.record_research_artifact("res", "src/f.md", "findings", None)
            .unwrap();
        let e2 = app.finish_task("res").unwrap_err().to_string();
        assert!(e2.contains("uncertainty outcome"), "got: {e2}");
        // One recorded uncertainty satisfies the floor — even though it stays open
        // (completion never requires the open count to fall).
        app.record_uncertainty("res", "U-1", "still open", None)
            .unwrap();
        assert_eq!(app.finish_task("res").unwrap().event_type, "task_completed");
    }

    #[test]
    fn research_artifact_rejected_on_implementation_task() {
        // Kind binding: an implementation task must never accrue a research
        // footprint it never declared. (Checked before any file hashing.)
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_uncertainty_task(&app, "impl"); // implementation kind
        let err = app
            .record_research_artifact("impl", "src/note.md", "findings", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("research task"), "got: {err}");
    }

    #[test]
    fn research_artifact_rejected_out_of_scope() {
        // Scope binding: an artifact must sit inside the task's write_allow — the
        // same boundary the write gate enforces. Here write_allow = ["src"].
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        seed_research_task(&app, "res");
        std::fs::create_dir_all(app.project_root.join("docs")).unwrap();
        std::fs::write(app.project_root.join("docs").join("x.md"), b"x").unwrap();
        let err = app
            .record_research_artifact("res", "docs/x.md", "findings", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("write_allow"), "got: {err}");
    }

    #[test]
    fn research_artifact_rejected_after_terminal() {
        // Terminal-is-terminal: a completed task's disclosed footprint must not
        // change after the fact.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_research_to_review(&app, "res");
        std::fs::write(app.project_root.join("src").join("f.md"), b"f").unwrap();
        app.record_research_artifact("res", "src/f.md", "findings", None)
            .unwrap();
        app.record_uncertainty("res", "U-1", "open", None).unwrap();
        assert_eq!(app.finish_task("res").unwrap().event_type, "task_completed");
        // Now Completed → no further artifacts.
        std::fs::write(app.project_root.join("src").join("g.md"), b"g").unwrap();
        let err = app
            .record_research_artifact("res", "src/g.md", "findings", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("terminal"), "got: {err}");
    }

    #[test]
    fn research_finish_requires_current_artifact() {
        // A finish must point at an artifact that still matches what was recorded;
        // an artifact edited away after recording (STALE) must not satisfy it.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        drive_research_to_review(&app, "res");
        let path = app.project_root.join("src").join("f.md");
        std::fs::write(&path, b"original").unwrap();
        app.record_research_artifact("res", "src/f.md", "findings", None)
            .unwrap();
        app.record_uncertainty("res", "U-1", "open", None).unwrap();
        // Edit the artifact away → STALE → finish blocked on freshness.
        std::fs::write(&path, b"MUTATED").unwrap();
        let err = app.finish_task("res").unwrap_err().to_string();
        assert!(err.contains("CURRENT"), "got: {err}");
        // Re-record against the current file → a CURRENT artifact exists → proceeds.
        app.record_research_artifact("res", "src/f.md", "findings", None)
            .unwrap();
        assert_eq!(app.finish_task("res").unwrap().event_type, "task_completed");
    }

    // ── Run-ledger single-writer ──

    #[test]
    fn run_event_seq_is_assigned_authoritatively_under_lock() {
        // build_run_event emits a placeholder seq 0; append_run_event assigns the
        // real seq (max+1) inside the per-run lock. If that assignment regressed,
        // the run reducer would reject seq 0 ("Sequence error") and create_run
        // would fail — so a successful create with last_seq == 1 proves the fix.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let scope = vec!["src".to_string()];
        app.create_task(
            "t",
            CreateTaskInput {
                objective: "runs",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready("t").unwrap();
        app.start_task("t").unwrap();
        let run_id = app.create_run("t", "omp").unwrap();
        assert_eq!(app.replay_run(&run_id).unwrap().last_seq, 1);
    }

    // ── PRD plan / validate / status (workflow-prd-to-tasks-v1) ──

    const CONFIRMED_PRD: &str = "# PRD: Demo\n\n> Status: confirmed\n\n## Objective\n\nShip it.\n\n## Tasks\n\n\
        - id: auth-task\n  objective: add auth boundary\n  write-allow: src/auth\n  gates: cargo_check, cargo_test\n\n\
        - id: config-task\n  objective: parse config\n  write-allow: src/config.rs\n  gates: cargo_check\n";

    #[test]
    fn prd_validate_clean_prd_has_no_errors() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
        let v = app.prd_validate(&doc).unwrap();
        assert!(v.ok(), "unexpected errors: {:?}", v.errors());
    }

    #[test]
    fn prd_validate_catches_write_overlap() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // config-task writes into src/auth — overlaps auth-task's scope.
        let prd = CONFIRMED_PRD.replace("src/config.rs", "src/auth/sub.rs");
        let doc = crate::application::prd::parse_prd(&prd).unwrap();
        let v = app.prd_validate(&doc).unwrap();
        assert!(!v.ok(), "overlap must be an error");
        assert!(
            v.errors()
                .iter()
                .any(|p| p.message.contains("overlapping write-allow")),
            "{:?}",
            v.errors()
        );
    }

    #[test]
    fn prd_validate_catches_unknown_gate() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let prd = CONFIRMED_PRD.replace("cargo_test", "bogus_gate");
        let doc = crate::application::prd::parse_prd(&prd).unwrap();
        let v = app.prd_validate(&doc).unwrap();
        assert!(!v.ok());
        assert!(
            v.errors()
                .iter()
                .any(|p| p.message.contains("Unknown gate")),
            "{:?}",
            v.errors()
        );
    }

    #[test]
    fn prd_plan_draft_refused_without_dry_run() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let prd = CONFIRMED_PRD.replace("confirmed", "draft");
        let doc = crate::application::prd::parse_prd(&prd).unwrap();
        let err = app
            .prd_plan(&doc, None, None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("confirmed"), "{}", err);
    }

    #[test]
    fn prd_plan_superseded_always_refused() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let prd = CONFIRMED_PRD.replace("confirmed", "superseded");
        let doc = crate::application::prd::parse_prd(&prd).unwrap();
        // Even dry-run refuses a superseded PRD.
        let err = app
            .prd_plan(&doc, None, None, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("superseded"), "{}", err);
    }

    #[test]
    fn prd_plan_dry_run_creates_nothing() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
        let outcomes = app.prd_plan(&doc, None, None, true).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|o| !o.created));
        // Nothing persisted.
        assert!(app.get_status("auth-task").is_err());
    }

    #[test]
    fn prd_plan_confirmed_creates_tasks_with_correct_boundaries() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
        let outcomes = app.prd_plan(&doc, None, None, false).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|o| o.created));

        let auth = app.get_status("auth-task").unwrap();
        assert_eq!(auth.objective.as_deref(), Some("add auth boundary"));
        assert!(auth.write_allow.contains("src/auth"));
        assert!(auth.gates.contains("cargo_check"));
        assert!(auth.gates.contains("cargo_test"));
        // read-scope defaulted to write-allow (absent in the PRD).
        assert_eq!(auth.read_scope, auth.write_allow);
    }

    #[test]
    fn prd_plan_validation_failure_creates_nothing() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let prd = CONFIRMED_PRD.replace("cargo_test", "bogus_gate");
        let doc = crate::application::prd::parse_prd(&prd).unwrap();
        let err = app
            .prd_plan(&doc, None, None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("validation failed"), "{}", err);
        // Validate runs before any create → no task exists.
        assert!(app.get_status("auth-task").is_err());
        assert!(app.get_status("config-task").is_err());
    }

    #[test]
    fn prd_plan_wires_depends_on() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let prd = "# PRD: Deps\n\n> Status: confirmed\n\n## Tasks\n\n\
            - id: child\n  objective: depends on parent\n  write-allow: src/child\n  gates: cargo_check\n  depends-on: parent\n\n\
            - id: parent\n  objective: the base\n  write-allow: src/parent\n  gates: cargo_check\n";
        let doc = crate::application::prd::parse_prd(prd).unwrap();
        app.prd_plan(&doc, None, None, false).unwrap();
        let child = app.get_status("child").unwrap();
        assert!(child.depends_on.contains("parent"));
    }

    #[test]
    fn prd_status_view_shows_not_created_then_planning() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();

        // Before planning: all tasks not-yet-created.
        let view = app.prd_status_view(&doc).unwrap();
        assert_eq!(view.total, 2);
        assert_eq!(view.completed, 0);
        assert!(view.rows.iter().all(|r| !r.exists));

        // After planning: tasks exist in Planning.
        app.prd_plan(&doc, None, None, false).unwrap();
        let view = app.prd_status_view(&doc).unwrap();
        assert!(view.rows.iter().all(|r| r.exists));
        assert!(
            view.rows
                .iter()
                .all(|r| r.phase.as_deref() == Some("planning")),
            "{:?}",
            view.rows
        );
    }

    #[test]
    fn prd_plan_records_provenance_when_alignment_given() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // Write the alignment note + PRD file so provenance hashing succeeds.
        std::fs::write(dir.path().join("align.md"), "# alignment\n").unwrap();
        std::fs::write(dir.path().join("demo.md"), CONFIRMED_PRD).unwrap();
        let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
        let outcomes = app
            .prd_plan(&doc, Some("align.md"), Some("demo.md"), false)
            .unwrap();
        assert!(outcomes.iter().all(|o| o.provenance_recorded));

        // Provenance visible via the brainstorm view — convergence = the PRD.
        let state = app.get_status("auth-task").unwrap();
        let prov = app
            .brainstorm_provenance_view(&state)
            .expect("provenance was recorded");
        assert!(prov.convergence.is_some());
    }

    #[test]
    fn prd_plan_without_alignment_skips_provenance() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
        let outcomes = app.prd_plan(&doc, None, None, false).unwrap();
        assert!(outcomes.iter().all(|o| !o.provenance_recorded));
        let state = app.get_status("auth-task").unwrap();
        assert!(app.brainstorm_provenance_view(&state).is_none());
    }

    // ── Rich context injection (the data pipeline cmd_hook_context assembles) ──

    #[test]
    fn hook_context_enrichment_pipeline_surfaces_blockers_and_uncertainties() {
        // The hook enriches each in_progress task with drift/next-action,
        // blockers, open uncertainties, and provenance. This test exercises
        // the exact data pipeline — if any signal silently stops flowing, the
        // platform hooks render a context-blind model.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();

        // Create a prerequisite (left incomplete) + a dependent task.
        let scope = vec!["src".to_string()];
        app.create_task(
            "prereq",
            CreateTaskInput {
                objective: "prerequisite",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();

        app.create_task(
            "dependent",
            CreateTaskInput {
                objective: "depends on prereq",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &["prereq".to_string()],
            },
        )
        .unwrap();

        // Record an open uncertainty on the dependent task.
        app.record_uncertainty("dependent", "U-1", "is the API stable?", None)
            .unwrap();

        let state = app.get_status("dependent").unwrap();

        // Blocker: prereq is not Completed → unmet.
        let unmet = app.unmet_dependencies("dependent").unwrap();
        assert_eq!(unmet, vec!["prereq"]);

        // Open uncertainty is visible in the ledger.
        let ledger = app
            .uncertainty_ledger_view(&state)
            .expect("uncertainty ledger present");
        assert_eq!(ledger.open, 1);
        assert_eq!(ledger.items[0].id, "U-1");
        assert_eq!(ledger.items[0].status, "open");

        // next_action computes (held by unmet-dep gate or drift — either way
        // it returns a valid proposal without error).
        let na = app.next_action("dependent").unwrap();
        assert!(!na.rationale.is_empty());

        // No provenance recorded → None (the hook skips this field gracefully).
        assert!(app.brainstorm_provenance_view(&state).is_none());
    }

    // ── next-task: deterministic scheduling recommendation ──

    fn mk_ready_task(app: &ControlApp, id: &str, scope: &[&str]) {
        app.create_task(
            id,
            CreateTaskInput {
                objective: id,
                read_scope: &scope.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                write_allow: &scope.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready(id).unwrap();
    }

    #[test]
    fn next_task_recommends_start_for_ready_unblocked_task() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_ready_task(&app, "alpha", &["src/alpha"]);
        let rec = app.next_task().unwrap();
        assert_eq!(rec.action, "start");
        assert_eq!(rec.task_id.as_deref(), Some("alpha"));
        assert_eq!(rec.ready_candidates, 1);
    }

    #[test]
    fn next_task_picks_lowest_id_on_tie() {
        // Two ready tasks, both drift 0 (no telemetry) → deterministic tie-break
        // by task id ascending.
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_ready_task(&app, "zeta", &["src/zeta"]);
        mk_ready_task(&app, "alpha", &["src/alpha"]);
        let rec = app.next_task().unwrap();
        assert_eq!(rec.task_id.as_deref(), Some("alpha"));
    }

    #[test]
    fn next_task_skips_ready_task_with_unsatisfied_dep() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // prereq stays in planning (never completed) → dep unsatisfied.
        app.create_task(
            "prereq",
            CreateTaskInput {
                objective: "prereq",
                read_scope: &["src".to_string()],
                write_allow: &["src".to_string()],
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
        app.create_task(
            "blocked",
            CreateTaskInput {
                objective: "blocked",
                read_scope: &["src/b".to_string()],
                write_allow: &["src/b".to_string()],
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &["prereq".to_string()],
            },
        )
        .unwrap();
        app.mark_ready("blocked").unwrap();

        let rec = app.next_task().unwrap();
        // blocked is ready but deps unsatisfied → not a start candidate.
        // No actionable ready task → falls back to planning (prereq).
        assert_eq!(rec.action, "ready");
        assert_eq!(rec.task_id.as_deref(), Some("prereq"));
    }

    #[test]
    fn next_task_falls_back_to_planning_when_no_ready() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        // Only a planning task exists.
        app.create_task(
            "seed",
            CreateTaskInput {
                objective: "planning seed",
                read_scope: &["src".to_string()],
                write_allow: &["src".to_string()],
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
        let rec = app.next_task().unwrap();
        assert_eq!(rec.action, "ready");
        assert_eq!(rec.task_id.as_deref(), Some("seed"));
    }

    #[test]
    fn next_task_none_when_everything_terminal() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let rec = app.next_task().unwrap();
        assert_eq!(rec.action, "none");
        assert!(rec.task_id.is_none());
    }

    #[test]
    fn next_task_skips_ready_task_conflicting_with_active_scope() {
        // An in_progress task on src/shared blocks a ready task on src/shared
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.path().join("src/shared/sub")).unwrap();
        std::fs::create_dir_all(dir.path().join("src/other")).unwrap();
        let app = ControlApp::init(dir.path()).unwrap();
        mk_ready_task(&app, "active-one", &["src/shared"]);
        app.start_task("active-one").unwrap();
        // This ready task overlaps src/shared → skipped.
        mk_ready_task(&app, "conflicting", &["src/shared/sub"]);
        // This ready task is disjoint → recommended.
        mk_ready_task(&app, "safe", &["src/other"]);
        let rec = app.next_task().unwrap();
        assert_eq!(rec.action, "start");
        assert_eq!(rec.task_id.as_deref(), Some("safe"));
    }

    // ── Spec fact store (knowledge-accumulation-v1) ──

    #[test]
    fn spec_fact_add_assigns_sequential_ids_and_persists() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let f1 = app
            .spec_fact_add(
                "normalizer canonicalizes parent",
                "src/norm.rs:77",
                Some("boundary"),
            )
            .unwrap();
        assert_eq!(f1.fact_id, "F-001");
        let f2 = app
            .spec_fact_add("reducer is pure", "src/domain/task.rs:871", Some("domain"))
            .unwrap();
        assert_eq!(f2.fact_id, "F-002");
        // File persists.
        assert!(dir.path().join(".ctl/facts.jsonl").exists());
    }

    #[test]
    fn spec_fact_add_rejects_empty_statement_or_source() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        assert!(app.spec_fact_add("", "src/x", None).is_err());
        assert!(app.spec_fact_add("a fact", "", None).is_err());
    }

    #[test]
    fn spec_fact_list_filters_by_category_and_search() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        app.spec_fact_add(
            "normalizer canonicalizes parent",
            "src/norm.rs",
            Some("boundary"),
        )
        .unwrap();
        app.spec_fact_add("reducer is pure", "src/task.rs", Some("domain"))
            .unwrap();
        app.spec_fact_add("cli uses anyhow full path", "src/cli/mod.rs", Some("cli"))
            .unwrap();

        // By category.
        let boundary = app.spec_fact_list(Some("boundary"), None).unwrap();
        assert_eq!(boundary.len(), 1);
        assert_eq!(boundary[0].fact_id, "F-001");

        // By search.
        let hits = app.spec_fact_list(None, Some("canonicalizes")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].fact_id, "F-001");

        // No filter → all.
        assert_eq!(app.spec_fact_list(None, None).unwrap().len(), 3);
    }

    #[test]
    fn spec_facts_digest_summarizes_for_context_injection() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        app.spec_fact_add("fact a", "s", Some("boundary")).unwrap();
        app.spec_fact_add("fact b", "s", Some("domain")).unwrap();
        app.spec_fact_add("fact c", "s", Some("boundary")).unwrap();

        let digest = app.spec_facts_digest().unwrap();
        assert_eq!(digest.total, 3);
        assert_eq!(digest.categories.get("boundary"), Some(&2));
        assert_eq!(digest.categories.get("domain"), Some(&1));
        // Most recent first.
        assert_eq!(digest.recent[0].fact_id, "F-003");
    }

    #[test]
    fn spec_fact_promote_appends_to_spec_markdown() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let fact = app
            .spec_fact_add("a gotcha about paths", "src/norm.rs:77", Some("gotcha"))
            .unwrap();

        // Create the target spec file so canonicalize succeeds.
        let spec_dir = dir.path().join(".ctl/spec/backend");
        std::fs::create_dir_all(&spec_dir).unwrap();
        let target = spec_dir.join("error-handling.md");
        std::fs::write(&target, "# Error Handling\n").unwrap();

        let path = app
            .spec_fact_promote(&fact.fact_id, "backend/error-handling.md")
            .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Fact F-001"));
        assert!(content.contains("category: gotcha"));
        assert!(content.contains("a gotcha about paths"));
        assert!(content.contains("src/norm.rs:77"));
    }

    #[test]
    fn spec_fact_promote_rejects_unknown_id() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let err = app
            .spec_fact_promote("F-999", "backend/x.md")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found"), "{}", err);
    }
}

#[cfg(test)]
mod adapter_doctor_tests {
    //! adapter-doctor-v1 platform-integration assembly, exercised against the
    //! real repo root (the dogfooding checkout ships every platform file).
    use super::{
        adapter_doctor_report, adapter_status_diagnostic, claude_python_tests_check,
        evaluate_pretooluse_matcher,
    };
    use crate::adapters::{supported_adapters, CheckStatus};
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn status_of<'a>(
        diag: &'a crate::adapters::AdapterDiagnostic,
        check: &str,
    ) -> &'a crate::adapters::AdapterCheck {
        diag.checks
            .iter()
            .find(|c| c.name == check)
            .unwrap_or_else(|| panic!("missing check '{check}'"))
    }

    #[test]
    fn doctor_over_repo_root_has_no_failures_and_is_factual() {
        let report = adapter_doctor_report(&repo_root(), false);
        // The supported executor adapters + the Claude hook platform (the repo
        // wires `.claude/`, so its non-adapter diagnostic is appended).
        assert_eq!(report.total, supported_adapters().len() + 1);
        assert_eq!(
            report.healthy, report.total,
            "no adapter or platform should FAIL in-repo"
        );
        assert_eq!(report.failed, 0);
        // Bun plugin tests must NOT be run by default → at least one NOT_TRACKED.
        assert!(
            report.counts.not_tracked >= 1,
            "opencode Bun tests must be NOT_TRACKED without --verify"
        );
        // Sanity: the aggregate tally equals the sum of per-adapter pass counts.
        let pass_sum: usize = report.adapters.iter().map(|d| d.counts.pass).sum();
        assert_eq!(report.counts.pass, pass_sum);
    }

    #[test]
    fn status_layers_contract_and_platform_checks() {
        let diag = adapter_status_diagnostic(&repo_root(), "opencode", false);
        assert!(diag.resolved);
        let names: Vec<&str> = diag.checks.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.iter().any(|n| n.starts_with("contract.")),
            "must include the pure contract clauses"
        );
        for expected in [
            "platform.skill_present",
            "platform.protocol_in_sync",
            "platform.opencode_plugin_present",
            "platform.opencode_bun_tests",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        // Skills are in sync in-repo → drift check PASSes (REUSE of CI checker).
        assert_eq!(
            status_of(&diag, "platform.protocol_in_sync").status,
            CheckStatus::Pass
        );
        // Bun tests NOT_TRACKED by default.
        assert_eq!(
            status_of(&diag, "platform.opencode_bun_tests").status,
            CheckStatus::NotTracked
        );
    }

    #[test]
    fn omp_hook_and_config_present_in_repo_pass() {
        let diag = adapter_status_diagnostic(&repo_root(), "omp", false);
        for n in ["platform.omp_hook_present", "platform.omp_config_present"] {
            assert_eq!(
                status_of(&diag, n).status,
                CheckStatus::Pass,
                "{n} ships in-repo"
            );
        }
    }

    #[test]
    fn unknown_adapter_fails_contract_and_reports_unknown_platform() {
        let diag = adapter_status_diagnostic(&repo_root(), "bogus", false);
        assert!(!diag.resolved);
        assert!(diag.has_failures(), "contract.resolves must FAIL");
        // The platform layer says UNKNOWN (no wiring), never a fabricated PASS.
        assert_eq!(
            status_of(&diag, "platform.integration").status,
            CheckStatus::Unknown
        );
    }

    #[test]
    fn verify_attempts_bun_so_it_is_not_not_tracked() {
        // Under --verify the Bun check is actually attempted: PASS, FAIL, or
        // UNKNOWN (bun unavailable) — but never NOT_TRACKED.
        let diag = adapter_status_diagnostic(&repo_root(), "opencode", true);
        assert_ne!(
            status_of(&diag, "platform.opencode_bun_tests").status,
            CheckStatus::NotTracked,
            "--verify must attempt the Bun suite"
        );
    }

    // ── Claude hook-platform diagnostic (claude-doctor-hookcheck-v1) ──────────

    #[test]
    fn claude_platform_diagnostic_passes_in_repo() {
        // The repo wires `.claude/`, so the report carries a non-adapter Claude
        // diagnostic: resolved=false, every wiring check PASS, and crucially no
        // FAIL (an optional hook platform must never fail the doctor).
        let report = adapter_doctor_report(&repo_root(), false);
        let claude = report
            .adapters
            .iter()
            .find(|d| d.adapter == "claude")
            .expect("Claude diagnostic present when .claude/ is wired");
        assert!(
            !claude.resolved,
            "Claude is a hook platform, not a resolvable adapter"
        );
        assert!(
            !claude.has_failures(),
            "an optional hook platform never FAILs the report"
        );
        for n in [
            "platform.claude_gate_hook_present",
            "platform.claude_context_hook_present",
            "platform.claude_settings_present",
            "platform.claude_pretooluse_matcher",
        ] {
            assert_eq!(
                status_of(claude, n).status,
                CheckStatus::Pass,
                "{n} holds in-repo"
            );
        }
    }

    #[test]
    fn pretooluse_matcher_pass_when_expected_matcher_registered() {
        let json = r#"{"hooks":{"PreToolUse":[{"matcher":"Write|Edit|MultiEdit|Bash",
            "hooks":[{"type":"command","command":"python x"}]}]}}"#;
        let (status, _) = evaluate_pretooluse_matcher(Some(json));
        assert_eq!(status, CheckStatus::Pass);
    }

    #[test]
    fn pretooluse_matcher_warns_on_wrong_or_absent_matcher() {
        // A different matcher leaves some mutating tools ungated → WARN (visible),
        // not a fabricated PASS.
        let wrong = r#"{"hooks":{"PreToolUse":[{"matcher":"Write|Edit"}]}}"#;
        assert_eq!(
            evaluate_pretooluse_matcher(Some(wrong)).0,
            CheckStatus::Warn
        );
        // No PreToolUse hook at all → WARN.
        let none = r#"{"hooks":{"SessionStart":[]}}"#;
        assert_eq!(evaluate_pretooluse_matcher(Some(none)).0, CheckStatus::Warn);
    }

    #[test]
    fn pretooluse_matcher_unknown_when_unevaluable() {
        // Absent settings or malformed JSON cannot be evaluated → UNKNOWN, never
        // a silent PASS and never a FAIL.
        assert_eq!(evaluate_pretooluse_matcher(None).0, CheckStatus::Unknown);
        assert_eq!(
            evaluate_pretooluse_matcher(Some("{not json")).0,
            CheckStatus::Unknown
        );
    }

    #[test]
    fn claude_hook_tests_not_tracked_without_verify() {
        // The python suite is opt-in: NOT_TRACKED by default (never a silent PASS).
        assert_eq!(
            claude_python_tests_check(&repo_root(), false).status,
            CheckStatus::NotTracked
        );
    }

    #[test]
    fn claude_hook_tests_attempted_under_verify() {
        // Under --verify the suite is actually attempted: PASS / FAIL / UNKNOWN
        // (python unavailable) — but never NOT_TRACKED.
        assert_ne!(
            claude_python_tests_check(&repo_root(), true).status,
            CheckStatus::NotTracked,
            "--verify must attempt the python hook suite"
        );
    }
}
