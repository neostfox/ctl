mod brainstorm_service;
mod drift_service;
mod event_build_service;
mod handoff_service;
pub mod prd;
mod research_service;
mod run_service;
pub mod schedule;
pub mod spec;
mod task_service;
mod uncertainty_service;
mod view_service;
mod workspace_service;
use anyhow::{anyhow, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::adapter_for;
use crate::domain::event::Event;
use crate::domain::lease::LeaseStatus;
use crate::domain::run::{apply_run, AgentRunState, RunPhase};
use crate::domain::task::{apply, AuditTier, Phase, TaskKind, TaskState};
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
}

/// Truncate to at most `n` bytes on a UTF-8 char boundary (never panics by
/// splitting a multi-byte char), trimming surrounding whitespace and appending
/// "..." when truncated. Used to keep gate-evidence previews bounded.
fn truncate_preview(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

/// Like `truncate_preview` but keeps the **last** `n` bytes (char-boundary
/// safe) and prefixes "…". For streams whose actionable detail lives at the
/// end: cargo writes `test result: FAILED` + the failing-test name + panic at
/// the END of stdout, so a head window buries them under hundreds of `… ok`
/// lines. The tail window surfaces the failure identity.
fn truncate_tail_preview(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.len() <= n {
        return s.to_string();
    }
    let mut start = s.len() - n;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", s[start..].trim_start())
}

/// Format the recorded `evidence` string for a gate result. Pure (no IO) so it
/// is unit-testable in isolation. Failed gates include BOTH a stdout and a
/// stderr preview: cargo writes the failing-test name + panic to **stdout**, so
/// omitting stdout makes the failing test's identity unrecoverable from the
/// recorded evidence (the original bug — a failed cargo_test gate showed only
/// `exit=101 stderr=error: test failed` with no test name).
fn format_gate_evidence(result: &crate::infrastructure::gates::GateRunResult) -> String {
    if result.timed_out {
        // Reaching here means run_gate confirmed the process tree was reaped
        // (containment failure would have returned Err and recorded nothing).
        "exit=timeout termination=process_tree termination_result=confirmed".to_string()
    } else if result.passed {
        format!("exit={} stdout={}B", result.exit_code, result.stdout.len())
    } else {
        format!(
            "exit={} stdout={} stderr={}",
            result.exit_code,
            truncate_tail_preview(&result.stdout, 1024),
            truncate_preview(&result.stderr, 512),
        )
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

fn validate_gate_templates(
    gates: &[String],
    project_root: &std::path::Path,
) -> Result<Vec<String>> {
    let mut validated = Vec::with_capacity(gates.len());
    for gate_id in gates {
        if crate::infrastructure::gates::resolve_gate(gate_id, project_root).is_none() {
            return Err(anyhow!(
                "Unknown gate '{}' — must be a built-in template or a [[gate]] entry in .ctl/config.toml",
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
    // Always Some: SchemaValidator::new loads embedded schemas compiled into
    // the binary as a floor, so it succeeds even when `schemas/` is absent
    // from the working directory (the production case — installed ctl runs
    // from the user's project root, which has no `schemas/` dir). Disk
    // schemas, when present, take precedence for dev iteration. Returning
    // None here would let validate_event silently skip schema checks.
    // Kept as Option for API stability; if construction ever fails we surface
    // the error at the call site rather than degrading to no validation.
    SchemaValidator::new("schemas/").ok()
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
mod tests;

#[cfg(test)]
mod adapter_doctor_tests;
