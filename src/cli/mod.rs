use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use crate::application::{ControlApp, CreateTaskInput, ReviseTaskInput};
use crate::domain::event::Event;
use crate::domain::lease::LeaseStatus;
use crate::domain::task::{apply, Phase, TaskState};
use crate::infrastructure::boundary::normalizer::PathNormalizer;
use crate::infrastructure::schema_validator::SchemaValidator;
mod governance;
use governance::*;
mod render;
use render::*;
mod init;
use init::*;
mod task;
use task::*;
mod board;
use board::*;
mod spec;
use spec::*;
mod handoff;
use handoff::*;
mod research;
use research::*;
mod run;
use run::*;
mod brainstorm;
use brainstorm::*;
mod gate;
use gate::*;
mod architecture;
use architecture::*;
mod schedule;
use schedule::*;
mod hook;
use hook::*;
mod write_gate;
use write_gate::*;

#[derive(Parser)]
#[command(name = "ctl")]
#[command(version)]
#[command(about = "AI Dev Control Plane CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Validate and show what would happen, but do not persist changes
    #[arg(long, global = true)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize ctl and configure one or more agent platforms.
    Init {
        /// Platform to configure; repeat for multiple platforms. Omit to choose interactively.
        #[arg(long = "platform", value_enum)]
        platform: Vec<PlatformArg>,
        /// Configure Claude Code (.claude/).
        #[arg(long)]
        claude: bool,
        /// Configure opencode (.opencode/).
        #[arg(long)]
        opencode: bool,
        /// Configure OMP (.omp/).
        #[arg(long)]
        omp: bool,
        /// Configure all supported platforms.
        #[arg(long)]
        all: bool,
        /// Skip prompts; use explicit platforms, detected integrations, or all on a fresh project.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Task lifecycle commands (create through archive)
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },
    /// Generate the workflow skills from their single source
    Skills {
        #[command(subcommand)]
        command: SkillsCommands,
    },
    /// Rebuild task.json projection(s) from canonical task events
    Replay {
        /// Replay only this task; omit to replay every task
        #[arg(long)]
        task: Option<String>,
    },
    /// Rebuild all task views from canonical events
    Reconcile,
    /// Validate canonical task event logs
    Validate,
    /// Diagnose local task ledger health
    Doctor,
    /// Update ctl-managed project files. Use --merge for conflict-safe template sync.
    /// Without --merge, this retains the legacy binary self-update behavior.
    Update {
        /// Install a specific binary release tag (legacy self-update mode).
        #[arg(long)]
        version: Option<String>,
        /// Report whether a newer binary exists (legacy self-update mode).
        #[arg(long)]
        check: bool,
        /// Merge embedded workflow, hook, and skill templates into configured platforms.
        #[arg(long)]
        merge: bool,
        /// Overwrite locally modified managed files in --merge mode.
        #[arg(long)]
        force: bool,
        /// Leave locally modified managed files untouched in --merge mode.
        #[arg(long)]
        skip: bool,
    },
    /// Update the ctl binary in place to the latest GitHub release (ADR 0002).
    /// This is the only command that performs network I/O.
    SelfUpdate {
        /// Install a specific release tag instead of the latest.
        #[arg(long)]
        version: Option<String>,
        /// Report whether a newer version exists without installing anything.
        #[arg(long)]
        check: bool,
    },
    /// Truncate a torn trailing record from a ledger (explicit crash-recovery),
    /// or with --cross-ledger, detect and repair task↔run inconsistencies
    Repair {
        /// Repair this task's event ledger
        #[arg(long)]
        task: Option<String>,
        /// Repair this run's event ledger
        #[arg(long)]
        run: Option<String>,
        /// Scan every task and run ledger
        #[arg(long)]
        all: bool,
        /// Detect cross-ledger task↔run↔worktree inconsistencies (orphan/stranded/
        /// missing-worktree/partial-start runs, orphaned worktrees). Preview by
        /// default; with --apply, retires the stale side (abort run / prune
        /// worktree). Takes no target selector.
        #[arg(long)]
        cross_ledger: bool,
        /// Actually perform the repair; without this it only previews (dry-run)
        #[arg(long)]
        apply: bool,
        /// Emit JSON (cross-ledger mode)
        #[arg(long)]
        json: bool,
    },
    /// Schema validation commands
    Schema {
        #[command(subcommand)]
        command: SchemaCommands,
    },
    /// Boundary validation commands
    Boundary {
        #[command(subcommand)]
        command: BoundaryCommands,
    },
    /// Gate execution and recording (M2)
    Gate {
        #[command(subcommand)]
        command: GateCommands,
    },
    /// Context snapshot commands (M2)
    Context {
        #[command(subcommand)]
        command: ContextCommands,
    },
    /// Assignment export commands (M3)
    Assignment {
        #[command(subcommand)]
        command: AssignmentCommands,
    },
    /// Architecture compliance checks
    Architecture {
        #[command(subcommand)]
        command: ArchitectureCommands,
    },
    /// Generate audit report for a task (M3)
    Audit {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
    /// Show summary report of all tasks (M3)
    Report,
    /// Cross-task control board: phase / hold / active / gate / review per task,
    /// plus aggregate totals. Reads the same projection `reconcile` writes to
    /// `.ctl/control.json` (M-b).
    Board {
        /// Output as JSON (default is a human-readable table)
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Show the legacy table format instead of Kanban columns
        #[arg(long, default_value_t = false)]
        table: bool,
        /// Show only active (non-archived) tasks
        #[arg(long, default_value_t = false)]
        active: bool,
        /// Include archived tasks (hidden by default)
        #[arg(long, default_value_t = false)]
        include_archived: bool,
    },
    /// View the NON-CANONICAL gate-decision log (`.ctl/decisions.jsonl`):
    /// advisory records of blocked/flagged tool calls written by the host gate
    /// hooks. These are evidence, NOT canonical task events — not hash-chained
    /// and not covered by `ctl validate`.
    Decisions {
        /// Show at most the N most-recent records (0 = all)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Output the raw JSONL records instead of a formatted table
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Run commands (M3 manual + M4 OMP)
    Run {
        #[command(subcommand)]
        command: RunCommands,
    },
    /// Workspace isolation commands (M4)
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommands,
    },
    /// Approval commands (M4)
    Approval {
        #[command(subcommand)]
        command: ApprovalCommands,
    },
    /// Completion-audit review verdicts (M-f hard review gate)
    Review {
        #[command(subcommand)]
        command: ReviewCommands,
    },
    /// Request a reviewed out-of-scope edit exception (M-f). Files a path-scoped
    /// approval; once granted (after a ctl-review mode-A pass) the gate allows
    /// writes to that one path outside the task's write_allow.
    Apply {
        /// Task identifier (the active in_progress task)
        #[arg(long)]
        id: String,
        /// The out-of-scope path to request edit access to
        #[arg(long)]
        path: String,
        /// Why the out-of-scope edit is needed
        #[arg(long)]
        reason: String,
        /// TTL in seconds (default 86400)
        #[arg(long, default_value_t = 86400)]
        ttl: u64,
    },
    /// Adapter capability queries (M4)
    Adapter {
        #[command(subcommand)]
        command: AdapterCommands,
    },
    /// Schedule concurrent execution of multiple tasks (M6)
    Schedule {
        #[command(subcommand)]
        command: ScheduleCommands,
    },
    /// Hook integration commands (called by OMP hooks)
    Hook {
        #[command(subcommand)]
        command: HookCommands,
    },
    /// Agent run status report (M6)
    AgentReport,
    /// Telemetry evidence index (M5): submit signals for drift analysis
    Telemetry {
        #[command(subcommand)]
        command: TelemetryCommands,
    },
    /// Drift analysis (M5): transparent, deterministic rules over evidence
    Drift {
        #[command(subcommand)]
        command: DriftCommands,
    },
    /// Recommend the next action (M5): pass / ask / stop / replan / rescope,
    /// derived from drift. Read-only and advisory — emits no events.
    NextAction {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Output as JSON (default is human-readable)
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Recommend the next task to ready/start, ranked by satisfied deps +
    /// lowest drift + no active scope conflict. Read-only and advisory.
    NextTask {
        /// Output as JSON (default is human-readable)
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Read-only handoff artifacts (ctl-handoff-v1): export a portable snapshot
    /// of a task for another session or human to pick up. Emits no events.
    Handoff {
        #[command(subcommand)]
        command: HandoffCommands,
    },
    /// PRD workflow (workflow-prd-to-tasks-v1): scaffold a PRD whose structured
    /// `## Tasks` section a later `prd plan` step can turn into ctl tasks.
    Prd {
        #[command(subcommand)]
        command: PrdCommands,
    },
    /// Spec fact store (knowledge-accumulation-v1): capture and retrieve atomic
    /// verified facts. `.ctl/facts.jsonl` is an append-only evidence index;
    /// promote copies a fact into a curated spec markdown file.
    Spec {
        #[command(subcommand)]
        command: SpecCommands,
    },
    /// Bounded safety supervisor for unattended runs (ralph-safe-run-v1). A
    /// read-only dead-man's-switch around an external run — it NEVER spawns an
    /// executor or writes code; it halts the moment human attention is due.
    Ralph {
        #[command(subcommand)]
        command: RalphCommands,
    },
    /// Brainstorm provenance (V1): record which cognitive artifacts a task
    /// derived from. Record-only — never gates create/finish, never claims
    /// thinking quality or review independence.
    Brainstorm {
        #[command(subcommand)]
        command: BrainstormCommands,
    },
    /// Uncertainty ledger (V1): record-and-disclose the unknowns a task carries.
    /// Record-only — never gates, never scores, never renders a verdict.
    Uncertainty {
        #[command(subcommand)]
        command: UncertaintyCommands,
    },
    /// Research/Spike (V1): record tracked research artifacts. A research task
    /// completes by producing evidence + uncertainty outcomes, not code.
    Research {
        #[command(subcommand)]
        command: ResearchCommands,
    },
    /// Attestation (V1): record subagent dispatches on the parent task ledger.
    Dispatch {
        #[command(subcommand)]
        command: DispatchCommands,
    },
}

#[derive(Subcommand)]
enum BrainstormCommands {
    /// Record originator (divergence/convergence) artifacts for a brainstorm.
    Record {
        /// Task identifier to bind the brainstorm to
        #[arg(long)]
        id: String,
        /// Logical brainstorm identifier (e.g. BS-001)
        #[arg(long = "brainstorm")]
        brainstorm: String,
        /// Path to the divergence (candidate-directions) artifact
        #[arg(long)]
        divergence: String,
        /// Path to the convergence (task-proposal) artifact, if any
        #[arg(long)]
        convergence: Option<String>,
        /// Claimed originating run id, if any (never attested in V1)
        #[arg(long = "source-run")]
        source_run: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Attach a critic (challenge) artifact to a recorded brainstorm.
    AttachCritic {
        #[arg(long)]
        id: String,
        #[arg(long = "brainstorm")]
        brainstorm: String,
        /// Path to the critic artifact
        #[arg(long)]
        critic: String,
        #[arg(long = "source-run")]
        source_run: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Record that the critic step was explicitly skipped, with a reason.
    SkipCritic {
        #[arg(long)]
        id: String,
        #[arg(long = "brainstorm")]
        brainstorm: String,
        /// Why the critic step was skipped
        #[arg(long)]
        reason: String,
        /// Who decided to skip (defaults to the recording actor)
        #[arg(long = "decided-by")]
        decided_by: Option<String>,
        #[arg(long = "source-run")]
        source_run: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show a task's brainstorm provenance (fact-only; staleness resolved).
    Show {
        #[arg(long)]
        id: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

/// Task kind argument (Research/Spike V1). Maps to the domain `TaskKind`.
#[derive(Clone, Copy, ValueEnum)]
enum TaskKindArg {
    Implementation,
    Research,
}

/// Completion-audit depth argument (ceremony scheme 6). `light` skips the decay
/// rubric; default `full`.
#[derive(Clone, Copy, ValueEnum)]
enum AuditTierArg {
    Full,
    Light,
}

#[derive(Subcommand)]
enum SkillsCommands {
    /// Generate every platform's SKILL.md from `.agent/skills/<skill>/source.md`
    /// AND the `@velo-ai/omp` plugin package under `npm-omp/` from `.omp/`.
    /// With --check, verify the on-disk files match (a CI gate) without writing.
    Sync {
        /// Verify only: exit non-zero if any generated file is out of date.
        #[arg(long)]
        check: bool,
    },
}

/// Which agent platform `ctl init` wires up.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PlatformArg {
    /// Claude Code — governance hooks, settings + workflow skills under `.claude/`
    Claude,
    /// opencode — gate plugin, subagent roles, skill mirror under `.opencode/`
    Opencode,
    /// OMP — skills, hooks, settings under `.omp/`
    Omp,
    /// All of the above
    All,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PlatformSelection {
    claude: bool,
    opencode: bool,
    omp: bool,
}

impl PlatformSelection {
    fn all() -> Self {
        Self {
            claude: true,
            opencode: true,
            omp: true,
        }
    }

    fn any(self) -> bool {
        self.claude || self.opencode || self.omp
    }
}

fn resolve_platform_selection(
    explicit: &[PlatformArg],
    claude: bool,
    opencode: bool,
    omp: bool,
    all: bool,
) -> Result<PlatformSelection> {
    let mut selection = PlatformSelection {
        claude,
        opencode,
        omp,
    };
    if all {
        selection = PlatformSelection::all();
    }
    for platform in explicit {
        match platform {
            PlatformArg::Claude => selection.claude = true,
            PlatformArg::Opencode => selection.opencode = true,
            PlatformArg::Omp => selection.omp = true,
            PlatformArg::All => selection = PlatformSelection::all(),
        }
    }
    if selection.any() {
        Ok(selection)
    } else {
        Err(anyhow::anyhow!("no platform selected"))
    }
}

fn detected_platform_selection(project_root: &Path) -> PlatformSelection {
    PlatformSelection {
        claude: project_root.join(".claude").exists(),
        opencode: project_root.join(".opencode").exists(),
        omp: project_root.join(".omp").exists(),
    }
}

impl TaskKindArg {
    fn to_domain(self) -> crate::domain::task::TaskKind {
        match self {
            TaskKindArg::Implementation => crate::domain::task::TaskKind::Implementation,
            TaskKindArg::Research => crate::domain::task::TaskKind::Research,
        }
    }
}

impl AuditTierArg {
    fn to_domain(self) -> crate::domain::task::AuditTier {
        match self {
            AuditTierArg::Full => crate::domain::task::AuditTier::Full,
            AuditTierArg::Light => crate::domain::task::AuditTier::Light,
        }
    }
}

/// Research artifact kind argument. Rendered kebab-case on the CLI; mapped to the
/// canonical snake_case payload value.
#[derive(Clone, Copy, ValueEnum)]
enum ResearchKindArg {
    Findings,
    Experiment,
    Recommendation,
    DesignDraft,
}

impl ResearchKindArg {
    fn as_payload(&self) -> &'static str {
        match self {
            ResearchKindArg::Findings => "findings",
            ResearchKindArg::Experiment => "experiment",
            ResearchKindArg::Recommendation => "recommendation",
            ResearchKindArg::DesignDraft => "design_draft",
        }
    }
}

/// Terminal disposition of an uncertainty (V1). Rendered kebab-case on the CLI;
/// mapped to the canonical snake_case payload value.
#[derive(Clone, Copy, ValueEnum)]
enum DispositionArg {
    /// Closed by external evidence (requires --evidence).
    Resolved,
    /// Proceeding on faith; stays visibly unresolved by external evidence.
    AcceptedAsAssumption,
    /// No longer applies (requires --reason; never carries evidence).
    Invalidated,
}

impl DispositionArg {
    fn as_payload(&self) -> &'static str {
        match self {
            DispositionArg::Resolved => "resolved",
            DispositionArg::AcceptedAsAssumption => "accepted_as_assumption",
            DispositionArg::Invalidated => "invalidated",
        }
    }
}

/// Oracle kind argument (Oracle V1). Rendered kebab-case on the CLI; mapped to the
/// canonical snake_case payload value. `model` is advisory; `human` is not an
/// authenticated principal.
#[derive(Clone, Copy, ValueEnum)]
enum OracleKindArg {
    Deterministic,
    Test,
    Runtime,
    Human,
    Model,
    ExternalAuthority,
}

impl OracleKindArg {
    fn as_payload(&self) -> &'static str {
        match self {
            OracleKindArg::Deterministic => "deterministic",
            OracleKindArg::Test => "test",
            OracleKindArg::Runtime => "runtime",
            OracleKindArg::Human => "human",
            OracleKindArg::Model => "model",
            OracleKindArg::ExternalAuthority => "external_authority",
        }
    }
}

#[derive(Subcommand)]
enum UncertaintyCommands {
    /// Record an open uncertainty (an unknown the task carries).
    Record {
        /// Task identifier to bind the uncertainty to
        #[arg(long)]
        id: String,
        /// Stable uncertainty identifier (e.g. U-001), unique within the task
        #[arg(long = "uncertainty")]
        uncertainty: String,
        /// The unknown, in plain text
        #[arg(long)]
        statement: String,
        /// Free-text note on where it came from (unattested)
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Record a first-class, oracle-typed evidence object (Oracle V1) that a
    /// `resolved` disposition can later reference by id. ctl computes the hash.
    Evidence {
        /// Task identifier to bind the evidence to
        #[arg(long)]
        id: String,
        /// Stable evidence identifier (e.g. E-001), unique within the task
        #[arg(long = "evidence")]
        evidence: String,
        /// What kind of oracle produced it. `model` is advisory; `human` is not an
        /// authenticated principal.
        #[arg(long = "oracle-kind", value_enum)]
        oracle_kind: OracleKindArg,
        /// Free-text locator for the source (command, test name, URL); unattested
        #[arg(long)]
        source: Option<String>,
        /// Path to the file-backed evidence artifact (normalized + hashed by ctl)
        #[arg(long)]
        artifact: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Record a terminal disposition for an uncertainty.
    Dispose {
        #[arg(long)]
        id: String,
        #[arg(long = "uncertainty")]
        uncertainty: String,
        /// How the uncertainty was disposed
        #[arg(long, value_enum)]
        disposition: DispositionArg,
        /// Oracle V1: id of a recorded evidence (from `uncertainty evidence`) that
        /// resolves this uncertainty. Resolved only. Mutually exclusive with --evidence.
        #[arg(long = "evidence-ref")]
        evidence_ref: Option<String>,
        /// Legacy inline evidence: path to the artifact (resolved only; hashed by ctl).
        /// Mutually exclusive with --evidence-ref.
        #[arg(long)]
        evidence: Option<String>,
        /// Why it was disposed (required for invalidated)
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show a task's uncertainty ledger (fact-only; freshness resolved).
    Status {
        #[arg(long)]
        id: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ResearchCommands {
    /// Record a tracked research artifact (hash computed by ctl).
    Record {
        /// Task identifier (must be a research task)
        #[arg(long)]
        id: String,
        /// Artifact kind
        #[arg(long, value_enum)]
        kind: ResearchKindArg,
        /// Path to the artifact (normalized + hashed by ctl)
        #[arg(long)]
        artifact: String,
        /// Claimed originating run id, if any (never attested in V1)
        #[arg(long = "source-run")]
        source_run: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show a research task's fact-only output (artifacts + uncertainty outcomes).
    Status {
        #[arg(long)]
        id: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DispatchCommands {
    /// Record a subagent dispatch on the parent task. Host-attested: ctl hashes
    /// any supplied artifact (`sha2`) and records the role/adapter labels — it
    /// records what it was told was dispatched, never verifies what ran.
    Record {
        /// Parent task id the dispatch belongs to.
        #[arg(long)]
        task: String,
        /// Host-supplied subagent role/type label (unattested).
        #[arg(long)]
        role: String,
        /// Host-supplied adapter label (unattested).
        #[arg(long)]
        adapter: String,
        /// Claimed originating run id, if any (never attested in V1).
        #[arg(long)]
        run: Option<String>,
        /// Path to the instruction artifact; ctl records its sha256.
        #[arg(long)]
        instruction_artifact: Option<String>,
        /// Path to the context artifact; ctl records its sha256.
        #[arg(long)]
        context_artifact: Option<String>,
        /// Path to the output artifact; ctl records its sha256.
        #[arg(long)]
        output_artifact: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// List the subagent dispatches recorded on a task (fact-only disclosure).
    List {
        /// Task id whose dispatches to show.
        #[arg(long)]
        task: String,
    },
}

#[derive(Subcommand)]
enum TelemetryCommands {
    /// Append one telemetry evidence record to the index (M5)
    Add {
        /// Task identifier the signal is about
        #[arg(long)]
        id: String,
        /// Signal kind (e.g. test_failures, lint_errors, retries,
        /// unexpected_writes). Unknown kinds are accepted but fail closed.
        #[arg(long)]
        kind: String,
        /// Numeric magnitude of the signal
        #[arg(long, default_value_t = 1)]
        value: i64,
        /// Provenance of the evidence (default: the CTL_ACTOR identity)
        #[arg(long)]
        source: Option<String>,
    },
}

#[derive(Subcommand)]
enum DriftCommands {
    /// Compute the drift level/score for a task (M5)
    Compute {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Output as JSON (default is human-readable)
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Explain a drift decision: signals, rule IDs, and evidence (M5)
    Explain {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Output as JSON (default is human-readable)
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TaskCommands {
    /// Create a Planning task with a structured M1 boundary
    Create {
        /// Stable task identifier; maps to .ctl/tasks/<id>/
        #[arg(long)]
        id: String,
        /// Non-empty task objective
        #[arg(long)]
        objective: String,
        /// Paths the agent may read; repeat for multiple entries
        #[arg(long = "read-scope", required = true)]
        read_scope: Vec<String>,
        /// Paths the agent may write; repeat for multiple entries
        #[arg(long = "write-allow", required = true)]
        write_allow: Vec<String>,
        /// Additional paths the agent must not write; repeat for multiple entries
        #[arg(long = "write-deny")]
        write_deny: Vec<String>,
        /// Review/hold triggers; repeat for multiple entries
        #[arg(long = "risk-triggers")]
        risk_triggers: Vec<String>,
        /// Enforce the TDD red→green interlock: finish is blocked unless the
        /// cargo_test gate history shows a FAIL before a PASS. Adds the
        /// `tdd-red-green` risk trigger; requires a cargo_test gate.
        #[arg(long)]
        tdd: bool,
        /// Gate template IDs. If omitted, the gates are derived from the project
        /// default floor recorded in `.ctl/config.toml` (`[project].default_gates`,
        /// set by /ctl-spec). Repeat for multiple entries.
        #[arg(long = "gates")]
        gates: Vec<String>,
        /// Task IDs that must complete before this one (M-d); repeat for multiple
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        /// Task kind: implementation (default) or research. Immutable after create.
        #[arg(long, value_enum, default_value_t = TaskKindArg::Implementation)]
        kind: TaskKindArg,
        /// Completion-audit depth: `full` (default, full decay rubric) or `light`
        /// (closure checklist only — reviewer-isolated, skips R1-R6/T1-T6).
        #[arg(long = "audit-tier", value_enum, default_value_t = AuditTierArg::Full)]
        audit_tier: AuditTierArg,
    },
    /// Propose a task (gh6 full proposal-mode): the model authors the boundary;
    /// the task lands in `Proposed` and CANNOT be started until a human approves
    /// it (`ctl task approve`). Same args as `create`.
    Propose {
        /// Stable task identifier; maps to .ctl/tasks/<id>/
        #[arg(long)]
        id: String,
        /// Non-empty task objective
        #[arg(long)]
        objective: String,
        /// Paths the agent may read; repeat for multiple entries
        #[arg(long = "read-scope", required = true)]
        read_scope: Vec<String>,
        /// Paths the agent may write; repeat for multiple entries
        #[arg(long = "write-allow", required = true)]
        write_allow: Vec<String>,
        /// Additional paths the agent must not write; repeat for multiple entries
        #[arg(long = "write-deny")]
        write_deny: Vec<String>,
        /// Review/hold triggers; repeat for multiple entries
        #[arg(long = "risk-triggers")]
        risk_triggers: Vec<String>,
        /// Enforce the TDD red→green interlock: finish is blocked unless the
        /// cargo_test gate history shows a FAIL before a PASS. Adds the
        /// `tdd-red-green` risk trigger; requires a cargo_test gate.
        #[arg(long)]
        tdd: bool,
        /// Gate template IDs. If omitted, derived from the project default
        /// floor (`.ctl/config.toml [project].default_gates`). Repeat for multiple
        #[arg(long = "gates")]
        gates: Vec<String>,
        /// Task IDs that must complete before this one (M-d); repeat for multiple
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        /// Task kind: implementation (default) or research. Immutable after create.
        #[arg(long, value_enum, default_value_t = TaskKindArg::Implementation)]
        kind: TaskKindArg,
        /// Completion-audit depth: `full` (default, full decay rubric) or `light`
        /// (closure checklist only — reviewer-isolated, skips R1-R6/T1-T6).
        #[arg(long = "audit-tier", value_enum, default_value_t = AuditTierArg::Full)]
        audit_tier: AuditTierArg,
    },
    /// Fuse create + ready + start into one command with sensible defaults.
    /// Keeps the write boundary explicit (`--write-allow` required) but removes
    /// the three-step ceremony for small changes.
    Quick {
        /// Paths the agent may write — the task boundary; repeat for multiple
        #[arg(long = "write-allow", required = true)]
        write_allow: Vec<String>,
        /// Task objective
        #[arg(long, default_value = "quick change")]
        objective: String,
        /// Task id (default: quick-<unix_timestamp>)
        #[arg(long)]
        id: Option<String>,
        /// Read scope (default: same as --write-allow); repeat for multiple
        #[arg(long = "read-scope")]
        read_scope: Vec<String>,
        /// Gates; if omitted, derived from the project default floor
        /// (`.ctl/config.toml [project].default_gates`). Repeat for multiple
        #[arg(long = "gates")]
        gates: Vec<String>,
        /// Task IDs that must complete before this one (M-d); repeat for multiple
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
    },
    /// Revise a Planning task boundary; omitted fields keep current values
    Revise {
        /// Stable task identifier
        #[arg(long)]
        id: String,
        /// Replacement task objective
        #[arg(long)]
        objective: Option<String>,
        /// Replacement read scope; repeat for multiple entries
        #[arg(long = "read-scope")]
        read_scope: Vec<String>,
        /// Replacement write allowlist; repeat for multiple entries
        #[arg(long = "write-allow")]
        write_allow: Vec<String>,
        /// Replacement write denylist; repeat for multiple entries
        #[arg(long = "write-deny")]
        write_deny: Vec<String>,
        /// Replacement risk triggers; repeat for multiple entries
        #[arg(long = "risk-triggers")]
        risk_triggers: Vec<String>,
        /// Replacement gate template IDs; repeat for multiple entries
        #[arg(long = "gates")]
        gates: Vec<String>,
        /// Replacement dependency task IDs (M-d); repeat for multiple entries
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
    },
    /// Mark a Planning task ready
    Ready {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
    /// Approve a proposed task (proposal-mode, gh6 / issue #6): a human-only
    /// verb for `ready`. Errors when the acting identity (CTL_ACTOR) is not
    /// "human" — the model proposes (create); only a human approves. `ready`
    /// applies the same check; `approve` is the proposal-mode verb. The model
    /// proposes via `ctl task create` (propose is a synonym, not a separate
    /// command — identical args and behavior).
    Approve {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
    /// Print the current task projection
    Status {
        /// Stable task identifier
        #[arg(long)]
        id: String,
        /// Output as JSON (default is human-readable)
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Start a Ready task (transition to InProgress)
    Start {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
    /// Submit an InProgress task for review
    Submit {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
    /// Reopen a Review task back to InProgress
    Reopen {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
    /// Finish a Review task (completion interlock: all gates must pass)
    Finish {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
    /// Cancel a non-terminal task
    Cancel {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
    /// Archive a completed or cancelled task
    Archive {
        /// Stable task identifier
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum SchemaCommands {
    Validate {
        #[arg(short, long)]
        file: String,
    },
}

#[derive(Subcommand)]
enum BoundaryCommands {
    /// Validate a path against boundary rules
    Check {
        /// Path to validate
        #[arg(short, long)]
        path: String,
    },
    /// Explain why a path is accepted or rejected
    Explain {
        /// Path to explain
        #[arg(short, long)]
        path: String,
    },
    /// Check a task's workspace against its declared write scope
    CheckById {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum GateCommands {
    /// Execute a gate and record the result as a canonical event
    Run {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Gate template ID to execute
        #[arg(long)]
        gate: String,
    },
    /// Record an externally-verified gate result
    Record {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Gate template ID
        #[arg(long)]
        gate: String,
        /// Whether the gate passed
        #[arg(long)]
        passed: bool,
        /// Evidence description
        #[arg(long)]
        evidence: String,
    },
}

#[derive(Subcommand)]
enum ContextCommands {
    /// Build a context snapshot (hash all files in read scope)
    Build {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum AssignmentCommands {
    /// Export a structured assignment JSON for external execution
    Export {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum HandoffCommands {
    /// Export a portable, read-only handoff snapshot for a task
    Export {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Output as JSON (default is a human-readable digest)
        #[arg(long)]
        json: bool,
    },
    /// Capture explicit agent/human judgment for the next session.
    Capture {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// JSON file containing decisions, uncertainties, hazards, and next_safe_action.
        #[arg(long)]
        file: String,
        /// Print the captured artifact as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum PrdCommands {
    /// Print a structured PRD template to stdout (redirect to a file, e.g.
    /// `ctl prd init > prd.md`), ready for the grill/LLM step to fill in.
    Init {
        /// Title to put in the PRD heading
        #[arg(long, default_value = "<title>")]
        title: String,
    },
    /// Validate a filled PRD's `## Tasks` section: format, boundaries,
    /// protected paths, gate templates, and cross-task write overlap. Read-only.
    Validate {
        /// Path to the PRD markdown file
        #[arg(long)]
        file: String,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Turn a confirmed PRD's `## Tasks` section into governed ctl tasks. Each
    /// task still goes through the normal PreToolUse gate. A `draft` PRD is
    /// refused unless `--dry-run`; `superseded` is always refused.
    Plan {
        /// Path to the PRD markdown file
        #[arg(long)]
        file: String,
        /// Preview what would be created without persisting anything
        #[arg(long)]
        dry_run: bool,
        /// Path to the alignment note (divergence provenance); the PRD file is
        /// recorded as convergence. Optional — omit to skip provenance recording.
        #[arg(long)]
        alignment: Option<String>,
    },
    /// Show a PRD's observable-loop status: each task's existence, phase, and
    /// brainstorm provenance, plus a completion summary. Read-only.
    Status {
        /// Path to the PRD markdown file
        #[arg(long)]
        file: String,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum SpecCommands {
    /// Atomic verified facts — the project knowledge base.
    Fact {
        #[command(subcommand)]
        command: FactCommands,
    },
    /// Scan `.ctl/spec/**/*.md` for stale code-path references — backtick-quoted
    /// file paths that no longer exist on disk. Read-only: reports spec rot,
    /// never edits. [ROADMAP #2/S]
    Doctor {
        /// Print the findings as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum FactCommands {
    /// Record a verified fact. The statement + source persist to
    /// `.ctl/facts.jsonl` and surface in every subsequent session's context.
    Add {
        /// The fact statement (what was verified)
        #[arg(long)]
        statement: String,
        /// Where it was verified (file:line, command, URL) — required provenance
        #[arg(long)]
        source: String,
        /// Free-text category for filtering (e.g. "boundary", "gotcha")
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// List or search facts. Read-only.
    List {
        /// Filter by category (case-insensitive)
        #[arg(long)]
        category: Option<String>,
        /// Search statement + source (case-insensitive substring)
        #[arg(long)]
        search: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Append a fact as a formatted block into a curated spec markdown file
    /// under `.ctl/spec/`. The fact stays in the raw store; this copies it
    /// into processed knowledge.
    Promote {
        /// Fact id (e.g. F-003)
        #[arg(long)]
        id: String,
        /// Target spec file, relative to `.ctl/spec/` (e.g. backend/error-handling.md)
        #[arg(long)]
        to: String,
    },
}

#[derive(Subcommand)]
enum RalphCommands {
    /// Supervise an unattended run: loop a read-only safety check (GO/NO-GO)
    /// with hard stops, halting the instant a human is needed. Spawns nothing.
    Run {
        /// Task to supervise
        #[arg(long)]
        id: String,
        /// Maximum safety cycles before stopping (hard bound)
        #[arg(long, default_value_t = 100)]
        max_iters: u64,
        /// Wall-clock deadline in seconds (0 = no deadline)
        #[arg(long, default_value_t = 0)]
        max_secs: u64,
        /// Seconds to sleep between cycles (0 = no sleep)
        #[arg(long, default_value_t = 0)]
        interval_secs: u64,
        /// Kill-switch file: if it exists, stop immediately
        #[arg(long, default_value = ".ctl/STOP")]
        kill_switch: String,
    },
}

#[derive(Subcommand)]
enum ArchitectureCommands {
    /// Run compliance checks, failing fast on the first violation (CI gate)
    Check,
    /// Run every compliance check and report each pass/fail — a full
    /// architecture health snapshot for periodic/scheduled checkups. Exits
    /// non-zero if any check fails; `--json` for machine consumption.
    Review {
        /// Emit a structured JSON report
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RunCommands {
    /// Ingest a manual execution result as evidence
    Ingest {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Adapter type ("manual" or "omp")
        #[arg(long, default_value = "manual")]
        adapter: String,
        /// Path to the result file
        #[arg(long)]
        result: String,
    },
    /// Start an OMP adapter run with worktree isolation (M4)
    Start {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Adapter type (must be "omp")
        #[arg(long, default_value = "omp")]
        adapter: String,
    },
    /// Abort an active run after OMP crash or manual intervention (M4)
    Abort {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Reason for aborting the run
        #[arg(long)]
        reason: String,
    },
    /// M6: read-only verdict — can this run's worktree land, and if not, how to
    /// recover? Classifies blockers (out-of-scope / cross-run / dirty main) with
    /// recommended recovery actions. Never merges.
    MergeCandidate {
        /// Run identifier (aggregate under .ctl/runs/<run_id>/).
        #[arg(long)]
        run: String,
        /// Emit JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Crash recovery (M6): report Running run aggregates + orphaned worktrees;
    /// with --abort, tear down one run (free its scope, remove its worktree).
    Recover {
        /// Abort this run_id: record run_aborted, clean up its worktree, and
        /// free its write scope. Omit for a read-only recovery report.
        #[arg(long)]
        abort: Option<String>,
        /// Reason recorded on the abort.
        #[arg(long, default_value = "crash-recovery abort")]
        reason: String,
        /// Emit JSON instead of a human table.
        #[arg(long)]
        json: bool,
    },
    /// Explicitly expire a run's lease IF it is past its wall-clock TTL
    /// (capability-lease-ttl-enforce-v1). Preview by default; `--apply` records
    /// a `lease_expired` event. Refuses a within-TTL or non-active lease. Does
    /// not terminate the run (use `run recover --abort` for that).
    ExpireLease {
        /// Run id whose lease to check/expire.
        #[arg(long)]
        run: String,
        /// Actually record `lease_expired`; without this it only previews.
        #[arg(long)]
        apply: bool,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Finish a Running run aggregate (M6): record `run_finished` so the run
    /// reaches Completed on its ledger and stops showing as an open/stranded run
    /// in recovery. Revokes the lease and cleans up the worktree if still present.
    /// The reducer enforces the guard: only a Running run can be finished. This is
    /// the production caller `run_finished` previously lacked (the symmetric
    /// counterpart to `run recover --abort`).
    Finish {
        /// Run id to finish.
        #[arg(long)]
        run: String,
        /// Host-reported model id (record-and-disclose; host-attested).
        #[arg(long)]
        model: Option<String>,
        /// Host-reported provider (host-attested).
        #[arg(long)]
        provider: Option<String>,
        /// Path to the instruction artifact; ctl records its sha256 (host-attested).
        #[arg(long)]
        instruction_artifact: Option<String>,
        /// Path to the context artifact; ctl records its sha256 (host-attested).
        #[arg(long)]
        context_artifact: Option<String>,
        /// Path to the output artifact; ctl records its sha256 (host-attested).
        #[arg(long)]
        output_artifact: Option<String>,
        /// Host-reported run start timestamp, ISO 8601 (host-attested).
        #[arg(long)]
        started_at: Option<String>,
        /// Host-reported run end timestamp, ISO 8601 (host-attested).
        #[arg(long)]
        ended_at: Option<String>,
        /// Host-reported process exit code (host-attested).
        #[arg(long)]
        exit_code: Option<i64>,
    },
}

#[derive(Subcommand)]
enum WorkspaceCommands {
    /// Create an isolated git worktree for a task (M4)
    Create {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
    /// Compute diff between worktree and HEAD (M4)
    Diff {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
    /// Apply verified worktree changes to main workspace (M4)
    Apply {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
    /// Remove a worktree (M4)
    Cleanup {
        /// Task identifier
        #[arg(long)]
        id: String,
    },
    /// Read-only clean-merge verdict for a task's worktree (M6). Reports
    /// whether the changes can be merged: in scope, no cross-task collision,
    /// main workspace clean. Emits no events and never merges — a human
    /// confirms, then runs `workspace apply`.
    MergeCandidate {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Output as JSON (default is human-readable)
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ApprovalCommands {
    /// Create an approval request (M4)
    Request {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Reason for the approval request
        #[arg(long)]
        reason: String,
        /// Action this approval authorizes for the active task (e.g. `deps`).
        /// Recorded in the approval scope and read by the governance gate.
        #[arg(long)]
        action: Option<String>,
        /// TTL in seconds (default 86400)
        #[arg(long, default_value_t = 86400)]
        ttl: u64,
    },
    /// Grant an approval request (M4)
    Grant {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Approval request ID
        #[arg(long)]
        request: String,
    },
    /// Deny an approval request (M4)
    Deny {
        /// Task identifier
        #[arg(long)]
        id: String,
        /// Approval request ID
        #[arg(long)]
        request: String,
    },
}

#[derive(Subcommand)]
enum ReviewCommands {
    /// Record a PASSING completion audit (M-f). A fresh pass (after the last
    /// submit) is required before `ctl task finish`.
    Accept {
        /// Task identifier (must be in Review)
        #[arg(long)]
        id: String,
        /// Optional reviewer note / audit summary
        #[arg(long)]
        note: Option<String>,
    },
    /// Record a FAILING completion audit (M-f) — blocks finish until the work is
    /// reworked and a passing audit is recorded.
    Reject {
        /// Task identifier (must be in Review)
        #[arg(long)]
        id: String,
        /// Reason the audit failed
        #[arg(long)]
        note: String,
    },
}

#[derive(Subcommand)]
enum AdapterCommands {
    /// Report adapter capabilities (M4)
    Capabilities {
        /// Adapter name (e.g., "omp")
        #[arg(long)]
        adapter: String,
    },
    /// List every registered executor adapter and its declared capabilities
    List {
        /// Emit JSON instead of a table
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show contract + platform-integration status of a single adapter
    Status {
        /// Adapter name (e.g., "omp")
        #[arg(long)]
        adapter: String,
        /// Emit JSON instead of human-readable output
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Run live checks too (e.g. the opencode Bun plugin tests). Off by
        /// default — live checks otherwise report NOT_TRACKED.
        #[arg(long, default_value_t = false)]
        verify: bool,
    },
    /// Diagnose every registered adapter: Rust contract + platform integration
    Doctor {
        /// Emit JSON instead of human-readable output
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Run live checks too (e.g. the opencode Bun plugin tests). Off by
        /// default — live checks otherwise report NOT_TRACKED.
        #[arg(long, default_value_t = false)]
        verify: bool,
    },
}

#[derive(Subcommand)]
enum ScheduleCommands {
    /// Plan concurrent execution of tasks with non-overlapping write scopes (M6)
    Plan {
        /// Maximum concurrent agents
        #[arg(long, default_value = "4")]
        max_concurrent: usize,
        /// Task IDs to schedule (space-separated)
        #[arg(long, num_args = 1..)]
        tasks: Vec<String>,
    },
    /// Validate a schedule plan against current task states (M6)
    Validate {
        /// Schedule plan ID
        #[arg(long)]
        plan: String,
    },
    /// Execute a validated schedule plan (M6)
    Run {
        /// Schedule plan ID
        #[arg(long)]
        plan: String,
        /// Poll interval in seconds
        #[arg(long, default_value = "5")]
        poll_interval: u64,
        /// Timeout per run in seconds
        #[arg(long, default_value = "1800")]
        timeout: u64,
    },
}

#[derive(Subcommand)]
enum HookCommands {
    /// Output session context as JSON for OMP hooks
    Context,
    /// Output active task breadcrumb as JSON for OMP hooks
    Breadcrumb,
    /// Check if a path is within write_allow for the active task
    CheckWrite {
        /// Path to check
        #[arg(long)]
        path: String,
    },
    /// Unified governance gate: check action against task state machine
    Gate {
        /// Tool name: write, edit, bash, read, search, find, task, other
        #[arg(long)]
        tool: String,
        /// Target path (for write/edit)
        #[arg(long)]
        path: Option<String>,
        /// Command string (for bash)
        #[arg(long)]
        command: Option<String>,
        /// Subagent type (for task tool): explore, task, oracle, etc.
        #[arg(long)]
        agent_type: Option<String>,
        /// M-e: dispatch binding — the task that dispatched this call. Binds
        /// governance to that task's write_allow even amid multiple active
        /// tasks. Falls back to the CTL_TASK_ID env var when omitted.
        #[arg(long)]
        task: Option<String>,
    },
    /// Append a decision record to decisions.jsonl
    RecordDecision {
        /// JSON object to record
        #[arg(long)]
        data: String,
    },
    /// Check if .ctl/spec/ is fresh relative to source changes
    SpecStatus,
    /// Wrap-up check for session-stop hooks: did the most recent task
    /// completion get a knowledge capture afterwards (project tier
    /// `.ctl/spec/`, global tier `~/.ctl/memory/`)? Reports `pending` and
    /// auto-marks a non-canonical once-guard (`.ctl/wrapup-reminded.json`) so
    /// the same finish is never reported pending twice.
    WrapupCheck,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let dry_run = cli.dry_run;
    match &cli.command {
        Commands::Init {
            platform,
            claude,
            opencode,
            omp,
            all,
            yes,
        } => cmd_init(platform, *claude, *opencode, *omp, *all, *yes, dry_run),
        Commands::Skills { command } => cmd_skills(command),
        Commands::Task { command } => cmd_task(command, dry_run),
        Commands::Replay { task } => cmd_replay(task.as_deref()),
        Commands::Reconcile => cmd_reconcile(),
        Commands::Validate => cmd_validate(),
        Commands::Doctor => cmd_doctor(),
        Commands::Update {
            version,
            check,
            merge,
            force,
            skip,
        } => {
            if !merge && (*force || *skip) {
                return Err(anyhow::anyhow!(
                    "ctl update --force/--skip require --merge; use `ctl self-update` for the binary."
                ));
            }
            if *merge {
                if version.is_some() || *check {
                    return Err(anyhow::anyhow!(
                        "ctl update --merge cannot be combined with --version or --check; use `ctl self-update` for the binary."
                    ));
                }
                if *force && *skip {
                    return Err(anyhow::anyhow!(
                        "ctl update --merge --force and --skip are mutually exclusive."
                    ));
                }
                cmd_project_update(*force, *skip, dry_run)
            } else {
                crate::infrastructure::self_update::run(version.clone(), *check)
            }
        }
        Commands::SelfUpdate { version, check } => {
            crate::infrastructure::self_update::run(version.clone(), *check)
        }
        Commands::Repair {
            task,
            run,
            all,
            cross_ledger,
            apply,
            json,
        } => cmd_repair(
            task.as_deref(),
            run.as_deref(),
            *all,
            *cross_ledger,
            *apply,
            *json,
        ),
        Commands::Schema { command } => cmd_schema(command),
        Commands::Boundary { command } => cmd_boundary(command),
        Commands::Gate { command } => cmd_gate(command, dry_run),
        Commands::Context { command } => cmd_context(command, dry_run),
        Commands::Assignment { command } => cmd_assignment(command, dry_run),
        Commands::Audit { id } => cmd_audit(id),
        Commands::Report => cmd_report(),
        Commands::Board {
            json,
            table,
            active,
            include_archived,
        } => cmd_board(*json, *table, *active, *include_archived),
        Commands::Decisions { limit, json } => cmd_decisions(*limit, *json),
        Commands::Run { command } => cmd_run(command, dry_run),
        Commands::Workspace { command } => cmd_workspace(command, dry_run),
        Commands::Approval { command } => cmd_approval(command, dry_run),
        Commands::Review { command } => cmd_review(command, dry_run),
        Commands::Apply {
            id,
            path,
            reason,
            ttl,
        } => cmd_apply(id, path, reason, *ttl, dry_run),
        Commands::Adapter { command } => cmd_adapter(command),
        Commands::Architecture { command } => cmd_architecture(command),
        Commands::Schedule { command } => cmd_schedule(command, dry_run),
        Commands::Hook { command } => cmd_hook(command),
        Commands::AgentReport => cmd_agent_report(),
        Commands::Telemetry { command } => cmd_telemetry(command, dry_run),
        Commands::Drift { command } => cmd_drift(command),
        Commands::NextAction { id, json } => cmd_next_action(id, *json),
        Commands::NextTask { json } => cmd_next_task(*json),
        Commands::Handoff { command } => cmd_handoff(command),
        Commands::Prd { command } => cmd_prd(command, dry_run),
        Commands::Ralph { command } => cmd_ralph(command),
        Commands::Brainstorm { command } => cmd_brainstorm(command),
        Commands::Uncertainty { command } => cmd_uncertainty(command),
        Commands::Research { command } => cmd_research(command),
        Commands::Dispatch { command } => cmd_dispatch(command),
        Commands::Spec { command } => cmd_spec(command, dry_run),
    }
}

fn app_open(dry_run: bool) -> Result<ControlApp> {
    ControlApp::open(&std::env::current_dir()?, dry_run)
}

#[cfg(test)]
mod tests;
