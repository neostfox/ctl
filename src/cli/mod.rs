use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
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

#[derive(Parser)]
#[command(name = "control")]
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
    /// Initialize the local task ledger
    Init,
    /// Task lifecycle commands (create through archive)
    Task {
        #[command(subcommand)]
        command: TaskCommands,
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
        /// Required gate template IDs; repeat for multiple entries
        #[arg(long = "gates", required = true)]
        gates: Vec<String>,
        /// Task IDs that must complete before this one (M-d); repeat for multiple
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
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
        /// Required gates (default: cargo_check, cargo_test); repeat for multiple
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
enum ArchitectureCommands {
    Check,
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
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let dry_run = cli.dry_run;
    match &cli.command {
        Commands::Init => cmd_init(dry_run),
        Commands::Task { command } => cmd_task(command, dry_run),
        Commands::Replay { task } => cmd_replay(task.as_deref()),
        Commands::Reconcile => cmd_reconcile(),
        Commands::Validate => cmd_validate(),
        Commands::Doctor => cmd_doctor(),
        Commands::Schema { command } => cmd_schema(command),
        Commands::Boundary { command } => cmd_boundary(command),
        Commands::Gate { command } => cmd_gate(command, dry_run),
        Commands::Context { command } => cmd_context(command, dry_run),
        Commands::Assignment { command } => cmd_assignment(command, dry_run),
        Commands::Audit { id } => cmd_audit(id),
        Commands::Report => cmd_report(),
        Commands::Board { json } => cmd_board(*json),
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
    }
}

fn app_open(dry_run: bool) -> Result<ControlApp> {
    ControlApp::open(&std::env::current_dir()?, dry_run)
}

fn cmd_init(dry_run: bool) -> Result<()> {
    let project_root = std::env::current_dir()?;
    if dry_run {
        println!(
            "[dry-run] Would initialize local task ledger + inject control-plane skills & hooks"
        );
        return Ok(());
    }
    ControlApp::init(&project_root)?;
    println!("Initialized local task ledger.");

    // Write default config if not present
    let config_path = project_root.join(".ctl").join("config.toml");
    if !config_path.exists() {
        let default_config = r#"# ctl Control Plane Configuration
# Customize which decay risks are enabled and their severity.

[risk]
# Production code decay risks (R1-R6). Set false to disable.
R1_cognitive_overload = true
R2_change_propagation = true
R3_knowledge_duplication = true
R4_accidental_complexity = true
R5_dependency_disorder = true
R6_domain_distortion = true

# Test decay risks (T1-T6). Set false to disable.
T1_test_obscurity = true
T2_test_brittleness = true
T3_test_duplication = true
T4_mock_abuse = true
T5_coverage_illusion = true
T6_architecture_mismatch = true

[severity]
# Override severity: "critical", "warning", "suggestion"
# R1 = "warning"

[scope]
# Glob patterns to exclude from analysis
# ignore = ["**/*.generated.*", "**/vendor/**"]
"#;
        std::fs::write(&config_path, default_config)?;
        println!("Created default .ctl/config.toml");
    }

    // Inject control-plane skills, hooks, and OMP settings
    let file_count = crate::infrastructure::skills::inject_all(&project_root)?;
    if file_count > 0 {
        println!(
            "Injected {} file(s) into .omp/ (skills + hooks).",
            file_count
        );
    } else {
        println!("All control-plane files already present in .omp/.");
    }

    println!("Control-plane active: auto-load skill + session hooks configured.");
    Ok(())
}

fn cmd_task(command: &TaskCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        TaskCommands::Create {
            id,
            objective,
            read_scope,
            write_allow,
            write_deny,
            risk_triggers,
            gates,
            depends_on,
        } => {
            let event = app.create_task(
                id,
                CreateTaskInput {
                    objective,
                    read_scope,
                    write_allow,
                    write_deny,
                    risk_triggers,
                    gates,
                    depends_on,
                },
            )?;
            println!("Created task '{}' at seq {}.", id, event.seq);
        }
        TaskCommands::Quick {
            write_allow,
            objective,
            id,
            read_scope,
            gates,
            depends_on,
        } => {
            let task_id = match id {
                Some(i) => i.clone(),
                None => {
                    let secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    format!("quick-{}", secs)
                }
            };
            let read: Vec<String> = if read_scope.is_empty() {
                write_allow.clone()
            } else {
                read_scope.clone()
            };
            let gate_list: Vec<String> = if gates.is_empty() {
                vec!["cargo_check".to_string(), "cargo_test".to_string()]
            } else {
                gates.clone()
            };
            let empty: Vec<String> = Vec::new();
            app.create_task(
                &task_id,
                CreateTaskInput {
                    objective,
                    read_scope: &read,
                    write_allow,
                    write_deny: &empty,
                    risk_triggers: &empty,
                    gates: &gate_list,
                    depends_on,
                },
            )?;
            app.mark_ready(&task_id)?;
            let event = app.start_task(&task_id)?;
            println!(
                "Quick task '{}' created, ready, started at seq {} — write scope: [{}], gates: [{}]",
                task_id,
                event.seq,
                write_allow.join(", "),
                gate_list.join(", ")
            );
        }
        TaskCommands::Revise {
            id,
            objective,
            read_scope,
            write_allow,
            write_deny,
            risk_triggers,
            gates,
            depends_on,
        } => {
            let event = app.revise_task(
                id,
                ReviseTaskInput {
                    objective: objective.as_deref(),
                    read_scope: optional_slice(read_scope),
                    write_allow: optional_slice(write_allow),
                    write_deny: optional_slice(write_deny),
                    risk_triggers: optional_slice(risk_triggers),
                    gates: optional_slice(gates),
                    depends_on: optional_slice(depends_on),
                },
            )?;
            println!("Revised task '{}' at seq {}.", id, event.seq);
        }
        TaskCommands::Ready { id } => {
            let event = app.mark_ready(id)?;
            println!("Marked task '{}' ready at seq {}.", id, event.seq);
        }
        TaskCommands::Status { id, json } => {
            let state = app.get_status(id)?;
            // M6: derived, cross-task view of which declared dependencies are
            // still unfinished. Computed here (not persisted) so the frozen
            // task-view schema / task.json projection is untouched.
            let blocked_by = app.unmet_dependencies(id)?;
            if *json {
                print_task_state(&state, &blocked_by)?;
            } else {
                print_task_human(&state, &blocked_by)?;
            }
        }
        TaskCommands::Start { id } => {
            let event = app.start_task(id)?;
            println!("Started task '{}' at seq {}.", id, event.seq);
        }
        TaskCommands::Submit { id } => {
            let event = app.submit_task(id)?;
            println!("Submitted task '{}' for review at seq {}.", id, event.seq);
        }
        TaskCommands::Reopen { id } => {
            let event = app.reopen_task(id)?;
            println!("Reopened task '{}' at seq {}.", id, event.seq);
        }
        TaskCommands::Finish { id } => {
            let event = app.finish_task(id)?;
            println!("Finished task '{}' at seq {}.", id, event.seq);
        }
        TaskCommands::Cancel { id } => {
            let event = app.cancel_task(id)?;
            println!("Cancelled task '{}' at seq {}.", id, event.seq);
        }
        TaskCommands::Archive { id } => {
            let event = app.archive_task(id)?;
            println!("Archived task '{}' at seq {}.", id, event.seq);
        }
    }
    Ok(())
}

fn optional_slice(values: &[String]) -> Option<&[String]> {
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn cmd_replay(task_id: Option<&str>) -> Result<()> {
    let app = app_open(false)?;
    match task_id {
        Some(id) => {
            app.replay(id)?;
            println!("Replayed task '{}'.", id);
        }
        None => {
            let rebuilt = app.reconcile()?;
            println!("Replayed {} task projection(s).", rebuilt.len());
        }
    }
    Ok(())
}

fn cmd_reconcile() -> Result<()> {
    let app = app_open(false)?;
    let rebuilt = app.reconcile()?;
    println!("Rebuilt {} task projection(s).", rebuilt.len());
    Ok(())
}

fn cmd_validate() -> Result<()> {
    let app = app_open(false)?;
    let issues = app.validate_store()?;
    if issues.is_empty() {
        println!("Task ledger validation passed.");
        return Ok(());
    }
    for issue in &issues {
        println!("VALIDATION ERROR: {}", issue);
    }
    Err(anyhow::anyhow!(
        "Task ledger validation failed with {} issue(s)",
        issues.len()
    ))
}

fn cmd_doctor() -> Result<()> {
    let app = app_open(false)?;
    let results = app.doctor()?;
    let has_error = results
        .iter()
        .any(|result| result.contains("ERROR") || result.contains("REPLAY ERROR"));
    for result in results {
        println!("{}", result);
    }
    if has_error {
        return Err(anyhow::anyhow!("Doctor found task ledger errors"));
    }
    Ok(())
}

fn cmd_audit(id: &str) -> Result<()> {
    let app = app_open(false)?;
    let report = app.generate_audit_report(id)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn cmd_report() -> Result<()> {
    let app = app_open(false)?;
    let reports = app.generate_status_report()?;
    if reports.is_empty() {
        println!("No tasks found.");
    } else {
        for report in &reports {
            let task_id = report
                .get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let phase = report.get("phase").and_then(|v| v.as_str()).unwrap_or("?");
            let objective = report
                .get("objective")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let archived = report
                .get("is_archived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let status = if archived { " (archived)" } else { "" };
            println!("{}: {} [{}]{}", task_id, objective, phase, status);
        }
    }
    Ok(())
}

fn cmd_board(json: bool) -> Result<()> {
    let app = app_open(false)?;
    let board = app.generate_board()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&board)?);
        return Ok(());
    }

    let empty = Vec::new();
    let tasks = board
        .get("tasks")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if tasks.is_empty() {
        println!("No tasks found.");
        return Ok(());
    }

    // Size the TASK column to the widest id (min 4 for the "TASK" header).
    let id_w = tasks
        .iter()
        .filter_map(|t| t.get("task_id").and_then(|v| v.as_str()).map(str::len))
        .max()
        .unwrap_or(4)
        .max(4);

    let s = |t: &Value, k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let b = |t: &Value, k: &str| t.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    let n = |t: &Value, k: &str| t.get(k).and_then(|v| v.as_u64()).unwrap_or(0);

    println!(
        "{:<id_w$}  {:<12}  {:^3} {:^3}  {:<7}  REVIEW",
        "TASK", "PHASE", "H", "A", "GATES"
    );
    for t in tasks {
        let gates = format!("{}/{}", n(t, "gates_passing"), n(t, "gates_total"));
        println!(
            "{:<id_w$}  {:<12}  {:^3} {:^3}  {:<7}  {}",
            s(t, "task_id"),
            s(t, "phase"),
            if b(t, "held") { "*" } else { "-" },
            if b(t, "active") { "*" } else { "-" },
            gates,
            s(t, "review"),
        );
    }

    let g = |k: &str| {
        board
            .get("totals")
            .and_then(|v| v.get(k))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    };
    println!(
        "\n{} tasks · {} active · {} held · {} needs-work · {} completed · {} archived",
        g("tasks"),
        g("active"),
        g("held"),
        g("needs_work"),
        g("completed"),
        g("archived"),
    );
    Ok(())
}

fn cmd_telemetry(command: &TelemetryCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        TelemetryCommands::Add {
            id,
            kind,
            value,
            source,
        } => {
            let source = source.clone().unwrap_or_else(|| {
                std::env::var("CTL_ACTOR")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "human".to_string())
            });
            app.telemetry_add(id, kind, *value, &source)?;
            if !dry_run {
                let known = crate::domain::telemetry::is_known_kind(kind);
                println!(
                    "Recorded telemetry for '{}': {}={}{}",
                    id,
                    kind,
                    value,
                    if known {
                        ""
                    } else {
                        " (unknown kind — drift will fail closed)"
                    }
                );
                println!("Next: ctl drift compute --id {}", id);
            }
        }
    }
    Ok(())
}

fn cmd_drift(command: &DriftCommands) -> Result<()> {
    let app = app_open(false)?;
    match command {
        DriftCommands::Compute { id, json } => {
            let report = app.compute_drift(id)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Task '{}': drift {} (score {})",
                    report.task_id,
                    report.level.as_str(),
                    report.score
                );
                if report.fired_rules.is_empty() {
                    println!("  no rules fired");
                } else {
                    println!("  rules: {}", report.fired_ids().join(", "));
                }
                println!("Next: ctl drift explain --id {}", id);
            }
        }
        DriftCommands::Explain { id, json } => {
            let report = app.compute_drift(id)?;
            let action = app.next_action(id)?;
            if *json {
                let out = serde_json::json!({
                    "drift": report,
                    "next_action": action,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!(
                    "Drift explanation for '{}': {} (score {})",
                    report.task_id,
                    report.level.as_str(),
                    report.score
                );
                if report.fired_rules.is_empty() {
                    println!("  no rules fired — no drift signals present");
                } else {
                    println!("  signals (rule ID · points · evidence):");
                    for r in &report.fired_rules {
                        println!("    {} · +{} · {}", r.id, r.points, r.evidence);
                    }
                }
                println!(
                    "  recommended action: {} — {}",
                    action.action.as_str().to_uppercase(),
                    action.rationale
                );
            }
        }
    }
    Ok(())
}

fn cmd_next_action(id: &str, json: bool) -> Result<()> {
    let app = app_open(false)?;
    let proposal = app.next_action(id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&proposal)?);
        return Ok(());
    }
    println!(
        "Task '{}': {} (drift {}, score {})",
        proposal.task_id,
        proposal.action.as_str().to_uppercase(),
        proposal.level.as_str(),
        proposal.score
    );
    println!("  rationale: {}", proposal.rationale);
    if !proposal.fired_rules.is_empty() {
        println!("  rules: {}", proposal.fired_rules.join(", "));
    }
    if let Some(p) = &proposal.structured_proposal {
        println!(
            "  structured proposal ({}): {}",
            p.proposed_action, p.rationale
        );
        for e in &p.evidence {
            println!("    - {}", e);
        }
        println!("  (advisory only — generates no events, changes no scope, starts no task)");
    }
    println!("  suggested: {}", proposal.suggested_command);
    Ok(())
}

fn cmd_run(command: &RunCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        RunCommands::Ingest {
            id,
            adapter,
            result,
        } => {
            if adapter == "manual" {
                let event = app.ingest_manual_result(id, std::path::Path::new(result))?;
                println!(
                    "Ingested manual result for task '{}' at seq {}.",
                    id, event.seq
                );
            } else if adapter == "omp" {
                let event = app.run_ingest_omp(id, std::path::Path::new(result))?;
                println!(
                    "Ingested OMP result for task '{}' at seq {}.",
                    id, event.seq
                );
            } else {
                return Err(anyhow::anyhow!("Unknown adapter: {}", adapter));
            }
        }
        RunCommands::Start { id, adapter } => {
            let event = app.run_start(id, adapter)?;
            println!(
                "Started {} run for task '{}' at seq {}.",
                adapter, id, event.seq
            );
        }
        RunCommands::Abort { id, reason } => {
            app.run_abort(id, reason)?;
            println!("Aborted run for task '{}'.", id);
        }
    }
    Ok(())
}

fn cmd_workspace(command: &WorkspaceCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        WorkspaceCommands::Create { id } => {
            let event = app.workspace_create(id)?;
            println!("Created workspace for task '{}' at seq {}.", id, event.seq);
        }
        WorkspaceCommands::Diff { id } => {
            let result = app.workspace_diff(id)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        WorkspaceCommands::Apply { id } => {
            let event = app.workspace_apply(id)?;
            println!("Applied workspace for task '{}' at seq {}.", id, event.seq);
        }
        WorkspaceCommands::Cleanup { id } => {
            let event = app.workspace_cleanup(id)?;
            println!("Cleaned workspace for task '{}' at seq {}.", id, event.seq);
        }
        WorkspaceCommands::MergeCandidate { id, json } => {
            let verdict = app.merge_candidate(id)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&verdict)?);
            } else {
                print_merge_candidate(&verdict);
            }
        }
    }
    Ok(())
}

fn print_merge_candidate(v: &Value) {
    let id = v.get("task_id").and_then(|x| x.as_str()).unwrap_or("?");
    let mergeable = v
        .get("mergeable")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let list = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    };
    println!(
        "Task '{}': merge candidate = {}",
        id,
        if mergeable { "MERGEABLE" } else { "BLOCKED" }
    );
    println!("  touched files: {}", list("touched_files"));
    if let Some(reasons) = v.get("blocking_reasons").and_then(|x| x.as_array()) {
        for r in reasons {
            if let Some(s) = r.as_str() {
                println!("  blocked: {}", s);
            }
        }
    }
    if let Some(oos) = v.get("out_of_scope").and_then(|x| x.as_array()) {
        for f in oos {
            if let Some(s) = f.as_str() {
                println!("    out-of-scope: {}", s);
            }
        }
    }
    if let Some(conf) = v.get("cross_task_conflicts").and_then(|x| x.as_array()) {
        for c in conf {
            let p = c.get("path").and_then(|x| x.as_str()).unwrap_or("?");
            let t = c
                .get("conflicting_task")
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            println!("    cross-task conflict: {} (also in task '{}')", p, t);
        }
    }
    if let Some(wc) = v.get("workspace_conflicts").and_then(|x| x.as_array()) {
        for f in wc {
            if let Some(s) = f.as_str() {
                println!("    dirty in main workspace: {}", s);
            }
        }
    }
    if list("requires_approval") > 0 {
        println!(
            "  note: {} high-risk change(s) will need approval at apply time",
            list("requires_approval")
        );
    }
    if mergeable {
        println!("Next: review, then `ctl workspace apply --id {}`", id);
    } else {
        println!("Resolve the blocking reasons above before merging.");
    }
}

fn cmd_apply(id: &str, path: &str, reason: &str, ttl: u64, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    // Model the out-of-scope edit request as a path-scoped approval, so it rides
    // the existing approval ledger/grant flow (schema-free). The grant — issued
    // after a ctl-review mode-A pass — is the recorded reviewer verdict that
    // opens this one path at the gate.
    let scope = serde_json::json!({ "action": "apply", "path": path });
    let event = app.approval_request(id, reason, scope, ttl)?;
    let request_id = event
        .payload
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    println!(
        "Filed out-of-scope edit request for '{}' on task '{}' at seq {} (request {}).\n\
         After a ctl-review (mode A) pass, grant it: ctl approval grant --id {} --request {}",
        path, id, event.seq, request_id, id, request_id
    );
    Ok(())
}

fn cmd_review(command: &ReviewCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        ReviewCommands::Accept { id, note } => {
            let event = app.record_completion_audit(id, true, note.as_deref())?;
            println!(
                "Recorded passing completion audit for task '{}' at seq {}.",
                id, event.seq
            );
        }
        ReviewCommands::Reject { id, note } => {
            let event = app.record_completion_audit(id, false, Some(note))?;
            println!(
                "Recorded FAILING completion audit for task '{}' at seq {}. Finish is blocked until reworked and re-audited.",
                id, event.seq
            );
        }
    }
    Ok(())
}

fn cmd_approval(command: &ApprovalCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        ApprovalCommands::Request {
            id,
            reason,
            action,
            ttl,
        } => {
            let scope = match action {
                Some(a) => serde_json::json!({ "action": a }),
                None => serde_json::json!({}),
            };
            let event = app.approval_request(id, reason, scope, *ttl)?;
            let request_id = event
                .payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            println!(
                "Created approval request '{}' for task '{}' at seq {}.\nGrant with: ctl approval grant --id {} --request {}",
                request_id, id, event.seq, id, request_id
            );
        }
        ApprovalCommands::Grant { id, request } => {
            let event = app.approval_grant(id, request)?;
            println!(
                "Granted approval for task '{}' request '{}' at seq {}.",
                id, request, event.seq
            );
        }
        ApprovalCommands::Deny { id, request } => {
            let event = app.approval_deny(id, request)?;
            println!(
                "Denied approval for task '{}' request '{}' at seq {}.",
                id, request, event.seq
            );
        }
    }
    Ok(())
}

fn cmd_adapter(command: &AdapterCommands) -> Result<()> {
    match command {
        AdapterCommands::Capabilities { adapter } => {
            let app = app_open(false)?;
            let caps = app.adapter_capabilities(adapter)?;
            println!("{}", serde_json::to_string_pretty(&caps)?);
        }
    }
    Ok(())
}

fn print_task_state(state: &TaskState, blocked_by: &[String]) -> Result<()> {
    let gate_results: BTreeMap<_, _> = state.gate_results.iter().collect();
    let active_leases: usize = state
        .leases
        .values()
        .filter(|l| l.status == LeaseStatus::Active)
        .count();
    let pending_approvals_count = state.pending_approvals.len();
    let view = serde_json::json!({
        "schema": "control.task-view.v1",
        "id": state.id,
        "phase": state.phase,
        "is_held": state.is_held,
        "is_archived": state.is_archived,
        "objective": state.objective,
        "read_scope": state.read_scope,
        "write_allow": state.write_allow,
        "write_deny": state.write_deny,
        "risk_triggers": state.risk_triggers,
        "gates": state.gates,
        "depends_on": state.depends_on,
        // M6: derived dependency-gating view — declared deps not yet Completed.
        // Display-only (this richer CLI JSON is already a superset of the frozen
        // persisted task-view); empty array means nothing blocks a start.
        "blocked_by": blocked_by,
        "gate_results": gate_results,
        "active_run": state.active_run,
        "leases_active": active_leases,
        "pending_approvals": pending_approvals_count,
        "last_event_seq": state.last_seq,
    });
    println!("{}", serde_json::to_string_pretty(&view)?);
    Ok(())
}

fn print_task_human(state: &TaskState, blocked_by: &[String]) -> Result<()> {
    println!("Task: {}", state.id);
    println!("Phase: {:?}", state.phase);
    if state.is_held {
        println!("HELD");
    }
    if state.is_archived {
        println!("ARCHIVED");
    }
    if let Some(ref obj) = state.objective {
        println!("Objective: {}", obj);
    }
    if !state.gates.is_empty() {
        println!("Gates:");
        for gate in &state.gates {
            let status = match state.gate_results.get(gate) {
                Some(r) if r.passed => "PASS".to_string(),
                Some(r) => format!("FAIL ({})", r.evidence.chars().take(60).collect::<String>()),
                None => "pending".to_string(),
            };
            println!("  {}: {}", gate, status);
        }
    }
    // M-d: declared dependencies
    if !state.depends_on.is_empty() {
        println!(
            "Depends on: {}",
            state
                .depends_on
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    // M6: dependency-gated start — which declared deps are not yet Completed.
    // Present only when something blocks; a startable task prints nothing here.
    if !blocked_by.is_empty() {
        println!(
            "Blocked by (unfinished dependencies): {}",
            blocked_by.join(", ")
        );
    }
    // M4: Active run
    if let Some(ref run) = state.active_run {
        println!(
            "Run: {} (adapter: {}, lease: {})",
            run.run_id, run.adapter, run.lease_id
        );
    }
    // M4: Leases
    let active_leases: Vec<_> = state
        .leases
        .values()
        .filter(|l| l.status == LeaseStatus::Active)
        .collect();
    if !active_leases.is_empty() {
        println!("Leases: {} active", active_leases.len());
    }
    // M4: Pending approvals
    for approval in state.pending_approvals.values() {
        println!("Approval {}: {:?}", approval.request_id, approval.status);
    }
    println!("Seq: {}", state.last_seq);
    Ok(())
}

fn cmd_schema(command: &SchemaCommands) -> Result<()> {
    match command {
        SchemaCommands::Validate { file } => {
            let content = fs::read_to_string(file)?;
            let instance: Value = serde_json::from_str(&content)?;
            let schema_id = instance
                .get("schema")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Instance missing 'schema' field"))?;
            let validator = SchemaValidator::new("schemas/")?;
            validator.validate_instance(&instance, schema_id)?;
            println!("Validation successful for schema: {}", schema_id);
        }
    }
    Ok(())
}

fn cmd_boundary(command: &BoundaryCommands) -> Result<()> {
    match command {
        BoundaryCommands::Check { path } => {
            let root = std::env::current_dir()?;
            let normalizer = PathNormalizer::new(root);
            match normalizer.normalize(path) {
                Ok(normalized) => {
                    println!("ACCEPT: {}", normalized.display());
                    Ok(())
                }
                Err(e) => {
                    println!("REJECT: {}", e);
                    Err(e)
                }
            }
        }
        BoundaryCommands::Explain { path } => boundary_explain(path),
        BoundaryCommands::CheckById { id } => {
            let app = app_open(false)?;
            let violations = app.boundary_check_and_record(id)?;
            if violations.is_empty() {
                println!("No boundary violations detected for task '{}'.", id);
            } else {
                println!("Boundary violations for task '{}':", id);
                for v in &violations {
                    println!("  {}", v);
                }
                return Err(anyhow::anyhow!(
                    "Task '{}' has {} boundary violation(s)",
                    id,
                    violations.len()
                ));
            }
            Ok(())
        }
    }
}

fn cmd_gate(command: &GateCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        GateCommands::Run { id, gate } => {
            let event = app.run_gate_checked(id, gate)?;
            println!(
                "Gate '{}' for task '{}': {} (seq {})",
                gate,
                id,
                if event
                    .payload
                    .get("passed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    "PASS"
                } else {
                    "FAIL"
                },
                event.seq
            );
        }
        GateCommands::Record {
            id,
            gate,
            passed,
            evidence,
        } => {
            let event = app.record_gate(id, gate, *passed, evidence)?;
            println!(
                "Recorded gate '{}' for task '{}': {} (seq {})",
                gate,
                id,
                if *passed { "PASS" } else { "FAIL" },
                event.seq
            );
        }
    }
    Ok(())
}

fn cmd_context(command: &ContextCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        ContextCommands::Build { id } => {
            let context = app.build_context(id)?;
            let file_count = context
                .get("file_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!(
                "Built context snapshot for task '{}': {} files hashed.",
                id, file_count
            );
        }
    }
    Ok(())
}

fn cmd_assignment(command: &AssignmentCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        AssignmentCommands::Export { id } => {
            let assignment = app.export_assignment(id)?;
            println!("Exported assignment for task '{}'.", id);
            let _ = assignment; // used for side effect of writing file
        }
    }
    Ok(())
}

fn cmd_architecture(command: &ArchitectureCommands) -> Result<()> {
    match command {
        ArchitectureCommands::Check => {
            check_schemas()?;
            check_dependencies()?;
            check_modules()?;
            check_baseline_manifest()?;
            check_state_transitions()?;
            check_milestone_scope()?;
            check_canonical_task_ledger_contract()?;
            check_schema_payload_completeness()?;
            check_schema_counter_examples()?;
            check_fixture_paths_gates()?;
            println!("All architecture checks passed.");
        }
    }
    Ok(())
}

fn boundary_explain(path_str: &str) -> Result<()> {
    let root = std::env::current_dir()?;
    let normalizer = PathNormalizer::new(root);
    println!("Boundary analysis for: {}", path_str);

    if path_str.starts_with("\\\\") || path_str.starts_with("//") {
        println!("  Rule PATH-002: UNC paths are rejected");
    }
    if Path::new(path_str).is_absolute() {
        println!("  Rule PATH-002: absolute paths are rejected");
    }
    if path_str.contains("..") {
        println!("  Rule PATH-002: parent directory traversal is rejected");
    }
    let protected = [
        ".git",
        ".ctl",
        ".control",
        "schemas",
        "Cargo.toml",
        "Cargo.lock",
    ];
    for p in &protected {
        if path_str == *p || path_str.starts_with(&format!("{}/", p)) {
            println!("  Rule PATH-003: protected path '{}' is rejected", p);
        }
    }

    match normalizer.normalize(path_str) {
        Ok(norm) => {
            println!("  Decision: ACCEPT");
            println!("  Normalized: {}", norm.display());
            Ok(())
        }
        Err(e) => {
            println!("  Decision: REJECT");
            println!("  Reason: {}", e);
            Err(e)
        }
    }
}

fn check_schemas() -> Result<()> {
    let schema_dir = Path::new("schemas");
    if !schema_dir.exists() {
        return Err(anyhow::anyhow!("schemas/ directory missing"));
    }

    let allowed_schemas = [
        "control.event-envelope.v1.schema.json",
        "control.task-definition.v1.schema.json",
        "control.task-view.v1.schema.json",
        "control.policy-decision.v1.schema.json",
        "control.run-state.v1.schema.json",
        "control.schedule-plan.v1.schema.json",
    ];

    for entry in fs::read_dir(schema_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !allowed_schemas.contains(&name.as_str()) {
            return Err(anyhow::anyhow!("Unexpected schema file found: {}", name));
        }
    }
    Ok(())
}

fn check_dependencies() -> Result<()> {
    let lock_path = Path::new("Cargo.lock");
    if lock_path.exists() {
        let content = fs::read_to_string(lock_path)?;
        let forbidden = ["tokio", "reqwest", "async-std", "hyper", "actix-web"];
        for dep in &forbidden {
            if content.contains(&format!("name = \"{}\"", dep)) {
                return Err(anyhow::anyhow!("Forbidden dependency detected: {}", dep));
            }
        }
    }

    let content = fs::read_to_string("Cargo.toml").map_err(|_| {
        anyhow::anyhow!("Cargo.toml not found — cannot verify dependency whitelist")
    })?;
    let mut in_deps = false;
    let mut found_deps: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" {
            in_deps = true;
            continue;
        }
        if in_deps && trimmed.starts_with('[') {
            break;
        }
        if in_deps {
            if let Some(name) = trimmed.split('=').next() {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    found_deps.push(name);
                }
            }
        }
    }
    found_deps.sort();
    let mut expected: Vec<&str> = vec!["anyhow", "clap", "serde", "serde_json", "sha2"];
    expected.sort();
    if found_deps != expected {
        return Err(anyhow::anyhow!(
            "Direct dependency mismatch: found {:?}, expected {:?}",
            found_deps,
            expected
        ));
    }

    Ok(())
}

fn check_modules() -> Result<()> {
    let domain_dir = Path::new("src/domain");
    if !domain_dir.exists() {
        return Err(anyhow::anyhow!("src/domain/ missing"));
    }

    // MODULE-001/002: domain/ must stay a pure reducer. Non-test code may not
    // import infrastructure/cli/adapters, nor touch the filesystem, process,
    // network, or wall-clock time. Test modules live under an indented
    // `#[cfg(test)]` block by convention, so module top-level (column-0) lines
    // are the non-test surface we enforce here. Without this scan the guardrail
    // was unenforced — `check_modules` only verified file extensions.
    let forbidden_use = [
        "use crate::infrastructure",
        "use crate::cli",
        "use crate::adapters",
        "use crate::application",
        "use std::fs",
        "use std::io",
        "use std::net",
        "use std::process",
        "use std::time",
    ];
    for entry in fs::read_dir(domain_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") {
            return Err(anyhow::anyhow!(
                "Non-rust file in domain module: {:?}",
                path
            ));
        }
        let content = fs::read_to_string(&path)?;
        for (idx, line) in content.lines().enumerate() {
            // Skip indented lines — these belong to function bodies or the
            // indented `#[cfg(test)]` module, which is exempt.
            if line.starts_with(char::is_whitespace) || !line.starts_with("use ") {
                if !line.starts_with(char::is_whitespace)
                    && (line.contains("SystemTime") || line.contains("Instant"))
                {
                    return Err(anyhow::anyhow!(
                        "Domain purity violation (MODULE-002, wall-clock time) at {:?}:{} — `{}`",
                        path,
                        idx + 1,
                        line.trim()
                    ));
                }
                continue;
            }
            if let Some(pat) = forbidden_use.iter().find(|p| line.starts_with(**p)) {
                return Err(anyhow::anyhow!(
                    "Domain dependency violation (MODULE-001/002) at {:?}:{} — `{}` ({})",
                    path,
                    idx + 1,
                    line.trim(),
                    pat
                ));
            }
        }
    }

    let expected_src_dirs = ["cli", "domain", "infrastructure", "application", "adapters"];

    let mut found_dirs: HashSet<String> = HashSet::new();
    for entry in fs::read_dir("src")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().is_dir() {
            found_dirs.insert(name);
        }
    }

    for expected in &expected_src_dirs {
        if !found_dirs.contains(*expected) {
            return Err(anyhow::anyhow!(
                "Expected src/ module missing: {}",
                expected
            ));
        }
    }

    if !Path::new("src/main.rs").exists() {
        return Err(anyhow::anyhow!("src/main.rs missing"));
    }

    let unexpected: Vec<&String> = found_dirs
        .iter()
        .filter(|d| !expected_src_dirs.contains(&d.as_str()))
        .collect();
    if !unexpected.is_empty() {
        let names: Vec<&str> = unexpected.iter().map(|s| s.as_str()).collect();
        return Err(anyhow::anyhow!(
            "Unexpected src/ subdirectories: {}",
            names.join(", ")
        ));
    }

    Ok(())
}

fn check_baseline_manifest() -> Result<()> {
    let expected_schemas = [
        "control.event-envelope.v1.schema.json",
        "control.task-definition.v1.schema.json",
        "control.task-view.v1.schema.json",
        "control.policy-decision.v1.schema.json",
        "control.run-state.v1.schema.json",
        "control.schedule-plan.v1.schema.json",
    ];
    let mut found_schemas: Vec<String> = Vec::new();
    for entry in fs::read_dir("schemas")? {
        let name = entry?.file_name().to_string_lossy().to_string();
        if name.ends_with(".schema.json") {
            found_schemas.push(name);
        }
    }
    found_schemas.sort();
    let mut expected_sorted = expected_schemas.to_vec();
    expected_sorted.sort();
    if found_schemas != expected_sorted {
        return Err(anyhow::anyhow!(
            "Schema file set mismatch: found {:?}, expected {:?}",
            found_schemas,
            expected_sorted
        ));
    }

    let expected_fixtures = [
        "invalid.json",
        "m5_drift_golden.json",
        "reducer_boundary_violation.jsonl",
        "reducer_hold.jsonl",
        "reducer_lifecycle.jsonl",
        "reducer_m2_lifecycle.jsonl",
        "reducer_m3_lifecycle.jsonl",
        "reducer_m4_lifecycle.jsonl",
        "run_lifecycle.jsonl",
        "reducer_revise.jsonl",
        "reducer_test.jsonl",
        "schema_counter_examples.json",
    ];
    let mut found_fixtures: Vec<String> = Vec::new();
    for entry in fs::read_dir("fixtures")? {
        let name = entry?.file_name().to_string_lossy().to_string();
        if name.ends_with(".jsonl") || name.ends_with(".json") {
            found_fixtures.push(name);
        }
    }
    found_fixtures.sort();
    let mut expected_fixtures_sorted = expected_fixtures.to_vec();
    expected_fixtures_sorted.sort();
    if found_fixtures != expected_fixtures_sorted {
        return Err(anyhow::anyhow!(
            "Fixture file set mismatch: found {:?}, expected {:?}",
            found_fixtures,
            expected_fixtures_sorted
        ));
    }

    Ok(())
}

fn check_state_transitions() -> Result<()> {
    let validator = SchemaValidator::new("schemas/")?;
    let fixture_files = [
        ("fixtures/reducer_test.jsonl", "t1", Phase::InProgress, 3),
        (
            "fixtures/reducer_lifecycle.jsonl",
            "t-lifecycle",
            Phase::Completed,
            10,
        ),
        ("fixtures/reducer_hold.jsonl", "t-hold", Phase::Completed, 8),
        (
            "fixtures/reducer_revise.jsonl",
            "t-revise",
            Phase::InProgress,
            4,
        ),
        (
            "fixtures/reducer_m2_lifecycle.jsonl",
            "t-m2",
            Phase::Completed,
            8,
        ),
        (
            "fixtures/reducer_boundary_violation.jsonl",
            "t-violation",
            Phase::InProgress,
            4,
        ),
        (
            "fixtures/reducer_m4_lifecycle.jsonl",
            "t-m4",
            Phase::Completed,
            18,
        ),
    ];
    for (path, task_id, expected_phase, expected_history) in &fixture_files {
        let content = fs::read_to_string(path)?;
        let mut state = TaskState::new(task_id);
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let json_value: Value = serde_json::from_str(line)?;
            let schema_id = json_value
                .get("schema")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            validator
                .validate_instance(&json_value, schema_id)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Schema validation failed in {} for schema {}: {}",
                        path,
                        schema_id,
                        e
                    )
                })?;
            let event: Event = serde_json::from_str(line)?;
            apply(&mut state, &event).map_err(|e| {
                anyhow::anyhow!(
                    "Reducer error in {} at event seq {}: {}",
                    path,
                    event.seq,
                    e
                )
            })?;
        }
        if state.phase != *expected_phase {
            return Err(anyhow::anyhow!(
                "Fixture {} ended in phase {:?}, expected {:?}",
                path,
                state.phase,
                expected_phase
            ));
        }
        if state.history.len() != *expected_history {
            return Err(anyhow::anyhow!(
                "Fixture {} has {} history entries, expected {}",
                path,
                state.history.len(),
                expected_history
            ));
        }
    }
    Ok(())
}

fn check_milestone_scope() -> Result<()> {
    let command = Cli::command();
    assert_exact_subcommands(
        "top-level CLI",
        command.get_subcommands().map(|cmd| cmd.get_name()),
        [
            "adapter",
            "agent-report",
            "apply",
            "approval",
            "architecture",
            "assignment",
            "audit",
            "board",
            "boundary",
            "context",
            "doctor",
            "drift",
            "gate",
            "hook",
            "init",
            "next-action",
            "reconcile",
            "replay",
            "report",
            "review",
            "run",
            "schedule",
            "schema",
            "task",
            "telemetry",
            "validate",
            "workspace",
        ],
    )?;

    let task_command = command
        .get_subcommands()
        .find(|cmd| cmd.get_name() == "task")
        .ok_or_else(|| anyhow::anyhow!("Missing task command"))?;
    assert_exact_subcommands(
        "task CLI",
        task_command.get_subcommands().map(|cmd| cmd.get_name()),
        [
            "create", "quick", "revise", "ready", "status", "start", "submit", "reopen", "finish",
            "cancel", "archive",
        ],
    )?;

    // M5 nested command surfaces (lock their shape like `task`).
    let telemetry_command = command
        .get_subcommands()
        .find(|cmd| cmd.get_name() == "telemetry")
        .ok_or_else(|| anyhow::anyhow!("Missing telemetry command"))?;
    assert_exact_subcommands(
        "telemetry CLI",
        telemetry_command
            .get_subcommands()
            .map(|cmd| cmd.get_name()),
        ["add"],
    )?;

    let drift_command = command
        .get_subcommands()
        .find(|cmd| cmd.get_name() == "drift")
        .ok_or_else(|| anyhow::anyhow!("Missing drift command"))?;
    assert_exact_subcommands(
        "drift CLI",
        drift_command.get_subcommands().map(|cmd| cmd.get_name()),
        ["compute", "explain"],
    )?;

    Ok(())
}

fn assert_exact_subcommands<'a>(
    label: &str,
    actual: impl Iterator<Item = &'a str>,
    expected: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let mut actual_names: Vec<&str> = actual.collect();
    let mut expected_names: Vec<&str> = expected.into_iter().collect();
    actual_names.sort_unstable();
    expected_names.sort_unstable();
    if actual_names != expected_names {
        return Err(anyhow::anyhow!(
            "{} command surface mismatch: found {:?}, expected {:?}",
            label,
            actual_names,
            expected_names
        ));
    }
    Ok(())
}

fn check_canonical_task_ledger_contract() -> Result<()> {
    let mut command = Cli::command();
    let help = command.render_long_help().to_string();
    if help.contains("--scope") {
        return Err(anyhow::anyhow!(
            "Legacy scope contract exposed by CLI help; use --read-scope/--write-allow"
        ));
    }

    check_files_absent(
        &["src/infrastructure/store/mod.rs", "src/application/mod.rs"],
        &[
            ".control/events.jsonl",
            "control_dir.join(\"events.jsonl\")",
            "Path::new(\".control\").join(\"events.jsonl\")",
        ],
    )?;
    check_files_absent(&["src/main.rs"], &["\"scope\""])?;

    check_files_absent(
        &[
            "schemas/control.task-definition.v1.schema.json",
            "schemas/control.task-view.v1.schema.json",
        ],
        &["\"scope\""],
    )?;
    // event-envelope is excluded because M4 approval events legitimately use "scope"

    for entry in fs::read_dir("fixtures")? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            check_file_content_absent(&path, &["\"scope\""])?;
        }
    }

    check_task_boundary_schema_contract()?;

    Ok(())
}

fn check_task_boundary_schema_contract() -> Result<()> {
    let schema_content = fs::read_to_string("schemas/control.event-envelope.v1.schema.json")?;
    let schema: Value = serde_json::from_str(&schema_content)?;
    for event_type in ["task_created", "task_revised"] {
        let payload_schema = event_payload_schema(&schema, event_type)?;
        let required = payload_schema
            .get("required")
            .and_then(|value| value.as_array())
            .ok_or_else(|| {
                anyhow::anyhow!("{} payload schema missing required list", event_type)
            })?;
        for field in [
            "objective",
            "read_scope",
            "write_allow",
            "write_deny",
            "risk_triggers",
            "gates",
        ] {
            if !required.iter().any(|value| value.as_str() == Some(field)) {
                return Err(anyhow::anyhow!(
                    "{} payload schema missing required field '{}'",
                    event_type,
                    field
                ));
            }
        }

        let properties = payload_schema
            .get("properties")
            .and_then(|value| value.as_object())
            .ok_or_else(|| anyhow::anyhow!("{} payload schema missing properties", event_type))?;
        if properties.contains_key("scope") {
            return Err(anyhow::anyhow!(
                "{} payload schema still exposes legacy 'scope'",
                event_type
            ));
        }
        for field in ["read_scope", "write_allow", "gates"] {
            let min_items = properties
                .get(field)
                .and_then(|value| value.get("minItems"))
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            if min_items == 0 {
                return Err(anyhow::anyhow!(
                    "{} payload field '{}' must require minItems >= 1",
                    event_type,
                    field
                ));
            }
        }
        let objective_min_length = properties
            .get("objective")
            .and_then(|value| value.get("minLength"))
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        if objective_min_length == 0 {
            return Err(anyhow::anyhow!(
                "{} payload field 'objective' must require minLength >= 1",
                event_type
            ));
        }
    }
    Ok(())
}

fn event_payload_schema<'a>(schema: &'a Value, event_type: &str) -> Result<&'a Value> {
    let all_of = schema
        .get("allOf")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow::anyhow!("Event schema missing allOf constraints"))?;
    for item in all_of {
        let item_event_type = item
            .get("if")
            .and_then(|value| value.get("properties"))
            .and_then(|value| value.get("type"))
            .and_then(|value| value.get("const"))
            .and_then(|value| value.as_str());
        if item_event_type == Some(event_type) {
            return item
                .get("then")
                .and_then(|value| value.get("properties"))
                .and_then(|value| value.get("payload"))
                .ok_or_else(|| anyhow::anyhow!("{} missing payload schema", event_type));
        }
    }
    Err(anyhow::anyhow!(
        "Event schema missing payload constraint for {}",
        event_type
    ))
}

fn check_files_absent(paths: &[&str], patterns: &[&str]) -> Result<()> {
    for path in paths {
        check_file_content_absent(Path::new(path), patterns)?;
    }
    Ok(())
}

fn check_file_content_absent(path: &Path, patterns: &[&str]) -> Result<()> {
    let content = fs::read_to_string(path)?;
    for pattern in patterns {
        if content.contains(pattern) {
            return Err(anyhow::anyhow!(
                "{} contains forbidden legacy/canonical-store pattern {}",
                path.display(),
                pattern
            ));
        }
    }
    Ok(())
}

fn check_schema_payload_completeness() -> Result<()> {
    let schema_content = fs::read_to_string("schemas/control.event-envelope.v1.schema.json")?;
    let schema: serde_json::Value = serde_json::from_str(&schema_content)?;

    let event_types = schema
        .pointer("/properties/type/enum")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Schema missing type enum"))?;

    let all_of = schema
        .get("allOf")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Schema missing allOf constraints"))?;

    let mut constrained_types: HashSet<String> = HashSet::new();
    for item in all_of {
        if let Some(event_type) = item
            .get("if")
            .and_then(|i| i.get("properties"))
            .and_then(|p| p.get("type"))
            .and_then(|t| t.get("const"))
            .and_then(|c| c.as_str())
        {
            constrained_types.insert(event_type.to_string());
        }
    }

    for et in event_types {
        let name = et.as_str().unwrap_or("");
        if !constrained_types.contains(name) {
            return Err(anyhow::anyhow!(
                "Schema payload constraint missing for event type '{}'",
                name
            ));
        }
    }

    Ok(())
}

fn check_schema_counter_examples() -> Result<()> {
    let validator = SchemaValidator::new("schemas/")?;
    let content = fs::read_to_string("fixtures/schema_counter_examples.json")?;
    let root: Value = serde_json::from_str(&content)?;
    let groups = root
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("schema_counter_examples.json must be an object"))?;

    for (group_name, group) in groups {
        let examples = group.as_object().ok_or_else(|| {
            anyhow::anyhow!(
                "schema counter-example group '{}' must be an object",
                group_name
            )
        })?;
        for (case_name, instance) in examples {
            let schema_id = instance
                .get("schema")
                .and_then(|v| v.as_str())
                .unwrap_or(group_name);
            if validator.validate_instance(instance, schema_id).is_ok() {
                return Err(anyhow::anyhow!(
                    "Schema counter-example unexpectedly passed: {}.{}",
                    group_name,
                    case_name
                ));
            }
        }
    }

    Ok(())
}

fn check_fixture_paths_gates() -> Result<()> {
    let known_gates: HashSet<&str> = [
        "cargo_fmt_check",
        "cargo_check",
        "cargo_test",
        "cargo_clippy",
    ]
    .into_iter()
    .collect();

    let normalizer = PathNormalizer::new(std::env::current_dir()?);

    let fixture_dir = fs::read_dir("fixtures")?;
    for entry in fixture_dir {
        let entry = entry?;
        let path = entry.path();
        if let Some(ext) = path.extension() {
            if ext == "jsonl" {
                let content = fs::read_to_string(&path)?;
                for (i, line) in content.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let event: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                        anyhow::anyhow!("{}:{}: parse error: {}", path.display(), i + 1, e)
                    })?;

                    for field in ["read_scope", "write_allow", "write_deny"] {
                        if let Some(paths) = event
                            .get("payload")
                            .and_then(|p| p.get(field))
                            .and_then(|s| s.as_array())
                        {
                            for path_val in paths {
                                if let Some(path_str) = path_val.as_str() {
                                    normalizer.normalize(path_str).map_err(|e| {
                                        anyhow::anyhow!(
                                            "{}:{}: illegal {} path '{}': {}",
                                            path.display(),
                                            i + 1,
                                            field,
                                            path_str,
                                            e
                                        )
                                    })?;
                                }
                            }
                        }
                    }

                    if event.get("type").and_then(|t| t.as_str()) == Some("gate_checked") {
                        if let Some(gate_id) = event
                            .get("payload")
                            .and_then(|p| p.get("gate_id"))
                            .and_then(|g| g.as_str())
                        {
                            if !known_gates.contains(gate_id) {
                                return Err(anyhow::anyhow!(
                                    "{}:{}: unknown gate_id '{}' in fixture",
                                    path.display(),
                                    i + 1,
                                    gate_id
                                ));
                            }
                        }
                    }

                    if let Some(gates) = event
                        .get("payload")
                        .and_then(|p| p.get("gates"))
                        .and_then(|g| g.as_array())
                    {
                        for gate_val in gates {
                            if let Some(gate_str) = gate_val.as_str() {
                                if !known_gates.contains(gate_str) {
                                    return Err(anyhow::anyhow!(
                                        "{}:{}: unknown gate '{}' in task gates",
                                        path.display(),
                                        i + 1,
                                        gate_str
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// ── M6: Schedule commands ──

fn cmd_schedule(command: &ScheduleCommands, dry_run: bool) -> Result<()> {
    match command {
        ScheduleCommands::Plan {
            max_concurrent,
            tasks,
        } => cmd_schedule_plan(*max_concurrent, tasks, dry_run),
        ScheduleCommands::Validate { plan } => cmd_schedule_validate(plan),
        ScheduleCommands::Run {
            plan,
            poll_interval,
            timeout,
        } => cmd_schedule_run(plan, *poll_interval, *timeout, dry_run),
    }
}

fn cmd_schedule_plan(max_concurrent: usize, tasks: &[String], dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;

    if tasks.is_empty() {
        return Err(anyhow::anyhow!(
            "No tasks specified. Use --tasks <id1> <id2> ..."
        ));
    }

    // Collect task states + declared dependency edges (M-d).
    let mut task_data = Vec::new();
    let mut deps: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        std::collections::HashMap::new();
    let task_set: std::collections::BTreeSet<&str> = tasks.iter().map(|s| s.as_str()).collect();
    for task_id in tasks {
        let state = app.replay_task(task_id)?;
        if state.phase != Phase::Ready && state.phase != Phase::InProgress {
            return Err(anyhow::anyhow!(
                "Task '{}' is not Ready or InProgress (phase: {})",
                task_id,
                state.phase
            ));
        }
        if state.is_held {
            return Err(anyhow::anyhow!("Task '{}' is held", task_id));
        }
        // Warn about deps on tasks outside this plan (they're assumed satisfied).
        for dep in &state.depends_on {
            if !task_set.contains(dep.as_str()) {
                eprintln!(
                    "Note: task '{}' depends on '{}' which is not in this plan — assumed already satisfied.",
                    task_id, dep
                );
            }
        }
        deps.insert(task_id.clone(), state.depends_on.clone());
        task_data.push((task_id.clone(), state.write_allow.clone()));
    }

    let plan = crate::application::schedule::plan_schedule(&task_data, &deps, max_concurrent)
        .map_err(|errs| anyhow::anyhow!("Cannot plan schedule: {}", errs.join("; ")))?;

    // Output plan as JSON
    let json = serde_json::to_string_pretty(&plan)?;
    println!("{}", json);

    // Write plan to file for later reference
    if !dry_run {
        let plan_path = app
            .project_root
            .join(".ctl")
            .join(format!("plans/{}.json", plan.plan_id));
        std::fs::create_dir_all(plan_path.parent().unwrap())?;
        std::fs::write(&plan_path, &json)?;
        eprintln!("Plan saved to {}", plan_path.display());
    }

    if !plan.conflicts.is_empty() {
        eprintln!("\nWarning: {} conflict(s) detected:", plan.conflicts.len());
        for c in &plan.conflicts {
            eprintln!(
                "  {} <-> {} overlaps: {:?}",
                c.task_a, c.task_b, c.overlapping_paths
            );
        }
    }

    Ok(())
}

/// Read a persisted plan and snapshot the current state of every task it names.
/// Shared by `schedule validate` and `schedule run` (M-c).
fn load_plan_and_states(
    app: &ControlApp,
    plan_id: &str,
) -> Result<(
    crate::application::schedule::SchedulePlan,
    Vec<crate::application::schedule::TaskCurrentState>,
)> {
    let plan_path = app
        .project_root
        .join(".ctl")
        .join("plans")
        .join(format!("{}.json", plan_id));
    if !plan_path.exists() {
        return Err(anyhow::anyhow!(
            "Plan '{}' not found at {} — run `ctl schedule plan` first",
            plan_id,
            plan_path.display()
        ));
    }
    let plan: crate::application::schedule::SchedulePlan =
        serde_json::from_str(&fs::read_to_string(&plan_path)?)?;

    let mut states = Vec::new();
    for group in &plan.groups {
        for task_id in &group.task_ids {
            let state = app.replay_task(task_id)?;
            states.push(crate::application::schedule::TaskCurrentState {
                task_id: task_id.clone(),
                phase: state.phase.as_str().to_string(),
                is_held: state.is_held,
                write_allow: state.write_allow.iter().cloned().collect(),
                depends_on: state.depends_on.iter().cloned().collect(),
            });
        }
    }
    Ok((plan, states))
}

fn cmd_schedule_validate(plan_id: &str) -> Result<()> {
    let app = app_open(false)?;
    let (plan, states) = load_plan_and_states(&app, plan_id)?;

    match crate::application::schedule::validate_plan(&plan, &states) {
        Ok(()) => {
            println!(
                "Schedule plan '{}' is valid: {} group(s), {} task(s), max_concurrent={}.",
                plan_id,
                plan.groups.len(),
                states.len(),
                plan.max_concurrent
            );
            Ok(())
        }
        Err(errors) => {
            for e in &errors {
                eprintln!("INVALID: {}", e);
            }
            Err(anyhow::anyhow!(
                "Schedule plan '{}' failed validation with {} issue(s)",
                plan_id,
                errors.len()
            ))
        }
    }
}

fn cmd_schedule_run(
    plan_id: &str,
    _poll_interval: u64,
    _timeout: u64,
    dry_run: bool,
) -> Result<()> {
    // M6 slice 1: a validated plan's first parallel-safe group is activated as
    // concurrent AgentRun aggregates — each task gets an isolated worktree, a
    // scoped lease, and a prepared OMP manifest. This NEVER spawns an executor:
    // OMP drives each run off its manifest and results are ingested separately.
    // Later groups wait until the current group's runs complete (re-run this
    // command). Crash recovery and merge-conflict recovery remain follow-ups.
    let app = app_open(dry_run)?;
    let (plan, states) = load_plan_and_states(&app, plan_id)?;

    if let Err(errors) = crate::application::schedule::validate_plan(&plan, &states) {
        for e in &errors {
            eprintln!("INVALID: {}", e);
        }
        return Err(anyhow::anyhow!(
            "Refusing to run: plan '{}' failed validation with {} issue(s)",
            plan_id,
            errors.len()
        ));
    }

    println!(
        "Plan '{}' validated: {} group(s), max_concurrent={}.",
        plan_id,
        plan.groups.len(),
        plan.max_concurrent
    );

    let Some(group) = plan.groups.first() else {
        println!("Plan has no groups; nothing to run.");
        return Ok(());
    };

    // Activating a group creates run aggregates + worktrees, which a dry-run
    // must not do. Report the intended activation and stop before any writes.
    if dry_run {
        println!(
            "[dry-run] Would activate group 0 (parallel-safe): {:?} — one OMP run + isolated worktree each, no executor spawned.",
            group.task_ids
        );
        return Ok(());
    }

    println!(
        "Activating group 0 (parallel-safe, non-overlapping write scopes): {:?}",
        group.task_ids
    );
    for task_id in &group.task_ids {
        let run_id = app.create_run(task_id, "omp")?;
        app.start_run(&run_id)
            .map_err(|e| anyhow::anyhow!("schedule run aborted at task '{}': {}", task_id, e))?;
        println!(
            "  started run {} for task '{}' (worktree .ctl/runs/{}/worktree, manifest .ctl/runs/{}/run-manifest.json)",
            run_id, task_id, run_id, run_id
        );
    }

    if plan.groups.len() > 1 {
        println!(
            "\n{} later group(s) pending — re-run `ctl schedule run {}` once the current runs finish.",
            plan.groups.len() - 1,
            plan_id
        );
    }
    println!(
        "\nNo executor was spawned. Drive each run with OMP off its manifest, ingest the result, \
         then `finish` the run to free its write scope."
    );
    Ok(())
}

fn cmd_agent_report() -> Result<()> {
    let app = app_open(false)?;
    let run_store =
        crate::infrastructure::store::run_store::RunEventStore::init(&app.project_root)?;

    let run_ids = run_store.run_ids()?;
    if run_ids.is_empty() {
        println!("No agent runs found.");
        return Ok(());
    }

    println!(
        "{:<20} {:<15} {:<12} {:<10} {:<30}",
        "RUN_ID", "TASK_ID", "ADAPTER", "PHASE", "WORKTREE"
    );
    for run_id in &run_ids {
        let events = run_store.read_for_run(run_id)?;
        let mut state = crate::domain::run::AgentRunState::new(run_id);
        for event in &events {
            if let Err(e) = crate::domain::run::apply_run(&mut state, event) {
                eprintln!("Error replaying run {}: {}", run_id, e);
                break;
            }
        }
        let wt = state.worktree_path.as_deref().unwrap_or("-");
        println!(
            "{:<20} {:<15} {:<12} {:<10} {:<30}",
            state.run_id, state.task_id, state.adapter, state.phase, wt
        );
    }

    Ok(())
}

// ── Hook integration commands ──────────────────────────────────────────

fn cmd_hook(command: &HookCommands) -> Result<()> {
    match command {
        HookCommands::Context => cmd_hook_context(),
        HookCommands::Breadcrumb => cmd_hook_breadcrumb(),
        HookCommands::CheckWrite { path } => cmd_hook_check_write(path),
        HookCommands::Gate {
            tool,
            path,
            command,
            agent_type,
            task,
        } => cmd_hook_gate(
            tool,
            path.as_deref(),
            command.as_deref(),
            agent_type.as_deref(),
            task.as_deref(),
        ),
        HookCommands::RecordDecision { data } => cmd_hook_record_decision(data),
        HookCommands::SpecStatus => cmd_hook_spec_status(),
    }
}

fn cmd_hook_context() -> Result<()> {
    let project_root = std::env::current_dir()?;
    let app = ControlApp::open(&project_root, false)?;
    let reports = app.generate_status_report()?;

    let mut total = 0u32;
    let mut by_phase: BTreeMap<String, u32> = BTreeMap::new();
    let mut active = Vec::new();

    for report in &reports {
        total += 1;
        let phase = report
            .get("phase")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        *by_phase.entry(phase.to_string()).or_default() += 1;
        if phase == "in_progress" {
            let task_id = report.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            // Replay to get full boundary for context injection
            let boundary = app
                .replay_task(task_id)
                .ok()
                .map(|s| {
                    serde_json::json!({
                        "write_allow": s.write_allow,
                        "write_deny": s.write_deny,
                        "read_scope": s.read_scope,
                        "gates": s.gates,
                    })
                })
                .unwrap_or(serde_json::json!({}));
            active.push(serde_json::json!({
                "id": task_id,
                "objective": report.get("objective").and_then(|v| v.as_str()).unwrap_or(""),
                "boundary": boundary,
            }));
        }
    }

    // Spec layers
    let spec_dir = project_root.join(".ctl").join("spec");
    let mut spec_layers = Vec::new();
    if spec_dir.exists() {
        for entry in fs::read_dir(&spec_dir)?.flatten() {
            if entry.file_type()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if spec_dir.join(&name).join("index.md").exists() {
                    spec_layers.push(name);
                }
            }
        }
    }

    let output = serde_json::json!({
        "binary": "ctl",
        "tasks": { "total": total, "by_phase": by_phase },
        "active_tasks": active,
        "spec_layers": spec_layers,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn cmd_hook_breadcrumb() -> Result<()> {
    let project_root = std::env::current_dir()?;
    let tasks_dir = project_root.join(".ctl").join("tasks");
    if !tasks_dir.exists() {
        println!("null");
        return Ok(());
    }

    let mut latest: Option<(String, serde_json::Value, std::time::SystemTime)> = None;
    for entry in fs::read_dir(&tasks_dir)?.flatten() {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let task_path = tasks_dir.join(&id).join("task.json");
        let mtime = entry.metadata()?.modified()?;
        if let Ok(content) = fs::read_to_string(&task_path) {
            if let Ok(task) = serde_json::from_str::<serde_json::Value>(&content) {
                if latest.as_ref().is_none_or(|(_, _, t)| mtime > *t) {
                    latest = Some((id, task, mtime));
                }
            }
        }
    }

    let Some((id, task)) = latest.map(|(i, t, _)| (i, t)) else {
        println!("null");
        return Ok(());
    };

    let next_map = serde_json::json!({
        "Planning": "Revise scope, then `ctl task ready`",
        "Ready": "`ctl task start` to begin work",
        "InProgress": "Implement, then `ctl task submit` (interlock check)",
        "Review": "`ctl task finish` (interlock check)",
        "Completed": "`ctl task archive` to clean up",
        "Cancelled": "`ctl task archive` to clean up",
    });

    let phase = task
        .get("phase")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let hold = task.get("hold").and_then(|v| v.as_bool()).unwrap_or(false);
    let next = next_map
        .get(phase)
        .and_then(|v| v.as_str())
        .unwrap_or("Check with ctl task status");
    let write_allow: Vec<String> = task
        .get("write_allow")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let output = serde_json::json!({
        "task_id": id,
        "phase": phase,
        "next": next,
        "hold": hold,
        "objective": task.get("objective").and_then(|v| v.as_str()).unwrap_or(""),
        "write_allow": write_allow,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
fn cmd_hook_check_write(target_path: &str) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let tasks_dir = project_root.join(".ctl").join("tasks");
    if !tasks_dir.exists() {
        let output = serde_json::json!({ "allowed": true, "reason": "no_tasks_dir" });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Find most recently modified in-progress task
    let mut active: Option<(String, Vec<String>, std::time::SystemTime)> = None;
    for entry in fs::read_dir(&tasks_dir)?.flatten() {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let task_path = tasks_dir.join(&id).join("task.json");
        let mtime = entry.metadata()?.modified()?;
        if let Ok(content) = fs::read_to_string(&task_path) {
            if let Ok(task) = serde_json::from_str::<serde_json::Value>(&content) {
                let phase = task.get("phase").and_then(|v| v.as_str()).unwrap_or("");
                if phase == "in_progress" && active.as_ref().is_none_or(|(_, _, t)| mtime > *t) {
                    let write_allow: Vec<String> = task
                        .get("write_allow")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    active = Some((id, write_allow, mtime));
                }
            }
        }
    }

    let Some((task_id, write_allow)) = active.map(|(i, w, _)| (i, w)) else {
        let output = serde_json::json!({ "allowed": true, "reason": "no_active_in_progress_task" });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    };

    if write_allow.is_empty() {
        let output = serde_json::json!({ "allowed": true, "reason": "empty_write_allow" });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let resolved = if Path::new(target_path).is_relative() {
        project_root.join(target_path)
    } else {
        Path::new(target_path).to_path_buf()
    };

    let in_scope = write_allow
        .iter()
        .any(|allow| resolved.starts_with(project_root.join(allow)));

    let output = serde_json::json!({
        "allowed": in_scope,
        "task_id": task_id,
        "path": target_path,
        "write_allow": write_allow,
        "reason": if in_scope { "in_scope" } else { "out_of_scope" }
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
// ── Governance state machine ──────────────────────────────────────────

/// Governance state derived from the task ledger.
#[derive(Debug)]
enum GovState {
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
struct ActiveTask {
    task_id: String,
    write_allow: Vec<String>,
    is_held: bool,
    approved_actions: Vec<String>,
    /// M-f `ctl apply`: granted out-of-scope edit paths for this task.
    approved_apply_paths: Vec<String>,
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
fn resolve_active_governance(active: &[ActiveTask], bound_task: Option<&str>) -> Option<GovState> {
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

fn compute_gov_state(project_root: &Path, bound_task: Option<&str>) -> Result<GovState> {
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

/// Classify a bash command into an action category.
/// Classify a single (non-compound) command segment.
fn classify_bash_segment(segment: &str) -> &'static str {
    let cmd = segment.trim();
    if cmd.starts_with("git commit") || cmd.starts_with("git add") {
        "git_commit"
    } else if cmd.starts_with("git push") {
        "git_push"
    } else if cmd.starts_with("cargo add") || cmd.starts_with("cargo install") {
        "cargo_deps"
    } else if cmd.starts_with("cargo check")
        || cmd.starts_with("cargo test")
        || cmd.starts_with("cargo build")
        || cmd.starts_with("cargo fmt")
        || cmd.starts_with("cargo clippy")
    {
        "cargo_build"
    } else {
        "bash_other"
    }
}

/// Classify a bash command for gating. Compound commands (`a && b`, `a; b`,
/// `a | b`, `$(a)`) are classified by their MOST restrictive segment — otherwise
/// `cp x y && git push` would slip through as `bash_other` (the loophole the
/// architecture review found). Best-effort only: a caller with shell access can
/// still obscure intent (eval, base64, env-indirection); this discourages and
/// audits casual composition, it is not a hard security boundary.
fn classify_bash(command: &str) -> &'static str {
    let (mut push, mut deps, mut commit, mut build) = (false, false, false, false);
    // Split on shell control/grouping operators so a restricted action inside a
    // compound or substitution (`$(..)`, backticks) becomes its own segment.
    for segment in command.split(|c| matches!(c, ';' | '\n' | '&' | '|' | '(' | ')' | '`')) {
        match classify_bash_segment(segment) {
            "git_push" => push = true,
            "cargo_deps" => deps = true,
            "git_commit" => commit = true,
            "cargo_build" => build = true,
            _ => {}
        }
    }
    // Most restrictive wins: step-up actions first, then commit window, then build.
    if push {
        "git_push"
    } else if deps {
        "cargo_deps"
    } else if commit {
        "git_commit"
    } else if build {
        "cargo_build"
    } else {
        "bash_other"
    }
}

/// Check if a path is within any of the allowed scopes.
fn path_in_scope(project_root: &Path, path: &str, scopes: &[String]) -> bool {
    let resolved = if Path::new(path).is_relative() {
        project_root.join(path)
    } else {
        Path::new(path).to_path_buf()
    };
    scopes
        .iter()
        .any(|s| resolved.starts_with(project_root.join(s)))
}

/// M-c: the first OTHER active task (non-archived, phase `in_progress` or
/// `review`) whose `write_allow` contains `path`. A write landing inside another
/// active task's claimed scope is hard-denied even when it sits within the
/// governing task's own scope — two active tasks must never write the same
/// region. This is the single-path specialization of
/// `schedule::detect_write_scope_overlap`; `ctl schedule validate` applies the
/// set-vs-set form across a whole plan. "Active" matches the `ctl board`
/// definition (in_progress | review), keeping one notion of active across
/// M-a / M-b / M-c.
fn first_overlapping_active_task(
    project_root: &Path,
    path: &str,
    governing_task_id: &str,
) -> Result<Option<String>> {
    let app = ControlApp::open(project_root, false)?;
    let reports = app.generate_status_report()?;
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
        if is_archived || task_id == governing_task_id || !matches!(phase, "in_progress" | "review")
        {
            continue;
        }
        let state = app.replay_task(&task_id)?;
        let scopes: Vec<String> = state.write_allow.iter().cloned().collect();
        if path_in_scope(project_root, path, &scopes) {
            return Ok(Some(task_id));
        }
    }
    Ok(None)
}

/// Check if a path targets the spec directory (always writable for updates).
fn is_spec_path(project_root: &Path, path: &str) -> bool {
    let resolved = if Path::new(path).is_relative() {
        project_root.join(path)
    } else {
        Path::new(path).to_path_buf()
    };
    resolved.starts_with(project_root.join(".ctl").join("spec"))
}
/// Short string for GovState variant (no Debug payload).
fn gov_state_str(state: &GovState) -> &'static str {
    match state {
        GovState::Ungoverned => "ungoverned",
        GovState::Idle => "idle",
        GovState::InProgress { .. } => "in_progress",
        GovState::Review { .. } => "review",
        GovState::Completed { .. } => "completed",
        GovState::MultipleActive { .. } => "multiple_active",
    }
}

fn cmd_hook_gate(
    tool: &str,
    path: Option<&str>,
    command: Option<&str>,
    agent_type: Option<&str>,
    bound_task: Option<&str>,
) -> Result<()> {
    let project_root = std::env::current_dir()?;
    // M-e: dispatch binding. Prefer the explicit `--task` flag; fall back to the
    // `CTL_TASK_ID` env var the dispatcher exports for its subagent. A blank
    // value is treated as absent (no binding).
    let env_task = std::env::var("CTL_TASK_ID").ok();
    let bound = bound_task
        .or(env_task.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let state = compute_gov_state(&project_root, bound)?;

    // UNGOVERNED — no .ctl, allow everything
    if matches!(state, GovState::Ungoverned) {
        let output = serde_json::json!({
            "allowed": true,
            "state": "ungoverned",
            "reason": "no .ctl directory — project not governed"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Read-only tools always allowed
    if matches!(
        tool,
        "read" | "search" | "find" | "ast_grep" | "lsp" | "eval" | "todo"
    ) {
        let output = serde_json::json!({
            "allowed": true,
            "state": gov_state_str(&state),
            "reason": "read-only tool"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // HELD — block everything except reads
    if let GovState::InProgress { is_held: true, .. } = &state {
        let output = serde_json::json!({
            "allowed": false,
            "state": "held",
            "reason": "task is held — resolve hold before proceeding",
            "remedy": "ctl task status --id <id> to see hold reason"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    match tool {
        "write" | "edit" => {
            let target = path.unwrap_or("");

            // Spec path always writable (except when held)
            if is_spec_path(&project_root, target) {
                let output = serde_json::json!({
                    "allowed": true,
                    "state": gov_state_str(&state),
                    "reason": "spec path — always writable"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
                return Ok(());
            }

            match &state {
                GovState::InProgress {
                    task_id,
                    write_allow,
                    approved_apply_paths,
                    ..
                } => {
                    let in_scope = path_in_scope(&project_root, target, write_allow);
                    // M-f `ctl apply`: a write outside write_allow is allowed when a
                    // reviewer has granted this exact out-of-scope path (an audited
                    // exception). M-c overlap still applies below.
                    let applied =
                        !in_scope && path_in_scope(&project_root, target, approved_apply_paths);
                    let allowed_by_scope = in_scope || applied;
                    // M-c: even within our own scope (or a granted apply), a write
                    // must not land inside another *active* task's claimed scope.
                    // Only checked when otherwise allowed (a denied write is moot).
                    let conflict = if allowed_by_scope {
                        first_overlapping_active_task(&project_root, target, task_id)?
                    } else {
                        None
                    };
                    let output = if let Some(other) = conflict {
                        serde_json::json!({
                            "allowed": false,
                            "state": "in_progress",
                            "task_id": task_id,
                            "conflicting_task": other,
                            "reason": format!("path is inside active task '{}' write_allow — cross-task write overlap", other),
                            "remedy": "narrow the write scopes so they don't overlap, or submit/cancel the other task first"
                        })
                    } else {
                        serde_json::json!({
                            "allowed": allowed_by_scope,
                            "state": "in_progress",
                            "task_id": task_id,
                            "reason": if in_scope {
                                "within write_allow"
                            } else if applied {
                                "reviewed out-of-scope exception (ctl apply)"
                            } else {
                                "outside write_allow"
                            },
                            "remedy": if allowed_by_scope { "" } else {
                                "request a reviewed exception: ctl apply --path <p> --reason <why>, then ctl approval grant; or widen scope via ctl task revise"
                            }
                        })
                    };
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                GovState::MultipleActive { task_ids } => {
                    let output = serde_json::json!({
                        "allowed": false,
                        "state": "multiple_active",
                        "task_ids": task_ids,
                        "reason": "multiple in_progress tasks declare write scopes — gateway cannot bind a single write_allow",
                        "remedy": "bind this call to its dispatching task (export CTL_TASK_ID=<id>, or ctl hook gate --task <id>), or leave exactly one task in_progress (ctl task submit --id <id>)"
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                _ => {
                    let output = serde_json::json!({
                        "allowed": false,
                        "state": gov_state_str(&state),
                        "reason": "no active in_progress task — create one first",
                        "remedy": "use control-guard skill to create a task, or user says 'skip control'"
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
            }
        }
        "bash" => {
            let cmd_str = command.unwrap_or("");
            let action = classify_bash(cmd_str);

            match action {
                "git_commit" => match &state {
                    // M-g: the commit window opens at Review and stays open
                    // through Completed. Committing in Review is what lets the
                    // finish interlock require a clean tree without deadlock.
                    GovState::Review {
                        task_id,
                        write_allow,
                    }
                    | GovState::Completed {
                        task_id,
                        write_allow,
                    } => {
                        let output = serde_json::json!({
                            "allowed": true,
                            "state": gov_state_str(&state),
                            "task_id": task_id,
                            "reason": "commit window open (review or completed)",
                            "scope": write_allow
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                    _ => {
                        let output = serde_json::json!({
                            "allowed": false,
                            "state": gov_state_str(&state),
                            "reason": "git commit only allowed in a task's commit window (Review or Completed)",
                            "remedy": "ctl task submit --id <id> to open the commit window, then commit before ctl task finish"
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                },
                "git_push" => {
                    // M-g: push rides the same commit window as commit — open
                    // from Review through Completed.
                    let allowed =
                        matches!(&state, GovState::Review { .. } | GovState::Completed { .. });
                    let output = serde_json::json!({
                        "allowed": allowed,
                        "state": gov_state_str(&state),
                        "reason": if allowed {
                            "push window open (review or completed)"
                        } else {
                            "git push only allowed in a task's commit window (Review or Completed)"
                        },
                        "remedy": if allowed { "" } else {
                            "ctl task submit --id <id> to open the commit window, then commit and push"
                        }
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                "cargo_deps" => {
                    // Dependency changes require a granted step-up approval
                    // (action=deps) on the active in_progress task.
                    let allowed = matches!(
                        &state,
                        GovState::InProgress { approved_actions, .. }
                            if approved_actions.iter().any(|a| a == "deps")
                    );
                    let output = serde_json::json!({
                        "allowed": allowed,
                        "state": gov_state_str(&state),
                        "reason": if allowed {
                            "dependency change approved for active task"
                        } else {
                            "dependency changes require a granted step-up approval (action=deps)"
                        },
                        "remedy": if allowed { "" } else {
                            "ctl approval request --id <id> --action deps --reason <why>, then ctl approval grant --id <id> --request <request_id>"
                        }
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                "cargo_build" => {
                    // cargo check/test/build/fmt are non-mutating verification —
                    // allowed in every governed state, including MultipleActive
                    // (so the agent can still build/test while disambiguating).
                    let allow = matches!(
                        &state,
                        GovState::InProgress { .. }
                            | GovState::Review { .. }
                            | GovState::Completed { .. }
                            | GovState::Idle
                            | GovState::MultipleActive { .. }
                    );
                    let output = serde_json::json!({
                        "allowed": allow,
                        "state": gov_state_str(&state),
                        "reason": if allow { "cargo tool allowed" } else { "not in a build-capable state" }
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                _ => {
                    // bash_other — allow in InProgress/Completed, warn in Idle
                    let allow = !matches!(&state, GovState::Ungoverned);
                    let output = serde_json::json!({
                        "allowed": allow,
                        "state": gov_state_str(&state),
                        "reason": if allow { "bash allowed" } else { "ungoverned" }
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
            }
        }
        "task" => {
            // Spawning subagents — govern based on agent type and state
            let at = agent_type.unwrap_or("task");
            let is_readonly = matches!(at, "explore");

            if is_readonly {
                // Read-only subagents (explore) always allowed
                let output = serde_json::json!({
                    "allowed": true,
                    "state": gov_state_str(&state),
                    "reason": "read-only subagent"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                // Writable subagents inherit governance from task ledger.
                // Block in IDLE/REVIEW/HELD — force parent to have active task.
                let allow = matches!(
                    &state,
                    GovState::Ungoverned | GovState::InProgress { .. } | GovState::Completed { .. }
                );
                let output = serde_json::json!({
                    "allowed": allow,
                    "state": gov_state_str(&state),
                    "agent_type": at,
                    "reason": if allow {
                        "subagent inherits governance from task ledger"
                    } else {
                        "no active task — subagent would operate without governance"
                    },
                    "remedy": if allow { "" } else {
                        "create a ctl task first: ctl task create + ready + start"
                    }
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        }
        _ => {
            // Unknown tool — default allow
            let output = serde_json::json!({
                "allowed": true,
                "state": gov_state_str(&state),
                "reason": "unknown tool — default allow"
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn cmd_hook_record_decision(data: &str) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let decisions_dir = project_root.join(".ctl");
    fs::create_dir_all(&decisions_dir)?;

    let decisions_path = decisions_dir.join("decisions.jsonl");

    // Validate it's valid JSON
    let parsed: serde_json::Value = serde_json::from_str(data)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let mut entry = parsed.as_object().cloned().unwrap_or_default();
    entry.insert("ts".to_string(), serde_json::json!(ts));
    let entry = serde_json::Value::Object(entry);

    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&decisions_path)?;
    writeln!(file, "{}", entry)?;

    let output = serde_json::json!({ "recorded": true });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn cmd_hook_spec_status() -> Result<()> {
    let project_root = std::env::current_dir()?;
    let spec_dir = project_root.join(".ctl").join("spec");
    let src_dir = project_root.join("src");

    if !spec_dir.exists() {
        let output = serde_json::json!({
            "has_specs": false,
            "status": "no_specs",
            "message": "Run /ctl-spec-bootstrap to generate specs"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Find the most recent mtime among spec files
    let mut spec_mtime: Option<std::time::SystemTime> = None;
    let mut spec_count = 0u32;
    if spec_dir.exists() {
        fn scan_dir(
            dir: &Path,
            mtime: &mut Option<std::time::SystemTime>,
            count: &mut u32,
        ) -> Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                if meta.is_dir() {
                    scan_dir(&entry.path(), mtime, count)?;
                } else if entry.path().extension().is_some_and(|e| e == "md") {
                    *count += 1;
                    let t = meta.modified()?;
                    *mtime = Some(mtime.map_or(
                        t,
                        |prev: std::time::SystemTime| if t > prev { t } else { prev },
                    ));
                }
            }
            Ok(())
        }
        scan_dir(&spec_dir, &mut spec_mtime, &mut spec_count)?;
    }

    // Find the most recent mtime among source files
    let mut src_mtime: Option<std::time::SystemTime> = None;
    let mut src_count = 0u32;
    if src_dir.exists() {
        fn scan_src(
            dir: &Path,
            mtime: &mut Option<std::time::SystemTime>,
            count: &mut u32,
        ) -> Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                if meta.is_dir() {
                    scan_src(&entry.path(), mtime, count)?;
                } else {
                    let ext_str = entry
                        .path()
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(String::from);
                    if ext_str.as_ref().is_some_and(|e| {
                        [
                            "rs", "ts", "tsx", "js", "jsx", "java", "go", "py", "vue", "svelte",
                        ]
                        .contains(&e.as_str())
                    }) {
                        *count += 1;
                        let t = meta.modified()?;
                        *mtime = Some(mtime.map_or(
                            t,
                            |prev: std::time::SystemTime| if t > prev { t } else { prev },
                        ));
                    }
                }
            }
            Ok(())
        }
        scan_src(&src_dir, &mut src_mtime, &mut src_count)?;
    }

    // Also check root config files (Cargo.toml, package.json, pom.xml, go.mod, pyproject.toml)
    let config_markers = [
        "Cargo.toml",
        "package.json",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "go.mod",
        "pyproject.toml",
    ];
    for marker in &config_markers {
        let p = project_root.join(marker);
        if p.exists() {
            if let Ok(meta) = p.metadata() {
                if let Ok(t) = meta.modified() {
                    src_mtime = Some(src_mtime.map_or(t, |prev| if t > prev { t } else { prev }));
                }
            }
        }
    }

    let (fresh, drift) = match (spec_mtime, src_mtime) {
        (_, None) => (true, false),       // no source to compare
        (None, Some(_)) => (false, true), // specs missing, source exists
        (Some(s), Some(c)) => (s >= c, c > s),
    };

    let output = serde_json::json!({
        "has_specs": true,
        "spec_files": spec_count,
        "source_files": src_count,
        "fresh": fresh,
        "drift": drift,
        "status": if fresh { "fresh" } else { "stale" },
        "message": if fresh {
            "Specs are up to date"
        } else {
            "Source files changed since last spec refresh. Consider running /ctl-spec-bootstrap"
        }
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{classify_bash, resolve_active_governance, ActiveTask, GovState};

    fn at(id: &str, paths: &[&str], held: bool) -> ActiveTask {
        ActiveTask {
            task_id: id.to_string(),
            write_allow: paths.iter().map(|s| s.to_string()).collect(),
            is_held: held,
            approved_actions: Vec::new(),
            approved_apply_paths: Vec::new(),
        }
    }

    #[test]
    fn no_active_tasks_yields_none() {
        assert!(resolve_active_governance(&[], None).is_none());
        assert!(resolve_active_governance(&[], Some("anything")).is_none());
    }

    #[test]
    fn single_write_task_binds_without_token() {
        let active = vec![at("a", &["src"], false)];
        match resolve_active_governance(&active, None) {
            Some(GovState::InProgress { task_id, .. }) => assert_eq!(task_id, "a"),
            other => panic!("expected InProgress(a), got {other:?}"),
        }
    }

    #[test]
    fn two_write_tasks_without_binding_fail_closed() {
        // M-a: ambiguous write governance → MultipleActive (fail closed).
        let active = vec![at("a", &["src"], false), at("b", &["docs"], false)];
        match resolve_active_governance(&active, None) {
            Some(GovState::MultipleActive { task_ids }) => {
                assert_eq!(task_ids, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected MultipleActive, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_token_binds_amid_multiple_active() {
        // M-e: an explicit binding to one of the active write tasks resolves the
        // ambiguity that would otherwise fail closed.
        let active = vec![at("a", &["src"], false), at("b", &["docs"], false)];
        match resolve_active_governance(&active, Some("b")) {
            Some(GovState::InProgress {
                task_id,
                write_allow,
                ..
            }) => {
                assert_eq!(task_id, "b");
                assert_eq!(write_allow, vec!["docs".to_string()]);
            }
            other => panic!("expected InProgress(b), got {other:?}"),
        }
    }

    #[test]
    fn stale_dispatch_token_is_ignored_not_honored() {
        // A token naming no active task must NOT widen scope; fall back to the
        // unbound M-a scan (here: still ambiguous → fail closed).
        let active = vec![at("a", &["src"], false), at("b", &["docs"], false)];
        match resolve_active_governance(&active, Some("ghost")) {
            Some(GovState::MultipleActive { task_ids }) => assert_eq!(task_ids.len(), 2),
            other => panic!("expected MultipleActive, got {other:?}"),
        }
    }

    #[test]
    fn binding_to_held_task_stays_held() {
        // Binding to a held task surfaces the hold (gate then blocks); the token
        // cannot launder a held task into a writable one.
        let active = vec![at("a", &["src"], false), at("b", &["docs"], true)];
        match resolve_active_governance(&active, Some("b")) {
            Some(GovState::InProgress {
                task_id, is_held, ..
            }) => {
                assert_eq!(task_id, "b");
                assert!(is_held, "bound held task must remain held");
            }
            other => panic!("expected InProgress(b, held), got {other:?}"),
        }
    }

    #[test]
    fn readonly_active_tasks_do_not_create_ambiguity() {
        // Only write-scoped tasks compete for write governance.
        let active = vec![at("w", &["src"], false), at("r", &[], false)];
        match resolve_active_governance(&active, None) {
            Some(GovState::InProgress { task_id, .. }) => assert_eq!(task_id, "w"),
            other => panic!("expected InProgress(w), got {other:?}"),
        }
    }

    #[test]
    fn binding_to_readonly_task_governs_as_readonly() {
        // M-e binds to the *dispatching* task even if it is read-only: its empty
        // write_allow then denies writes, exactly as that task should.
        let active = vec![at("w", &["src"], false), at("r", &[], false)];
        match resolve_active_governance(&active, Some("r")) {
            Some(GovState::InProgress {
                task_id,
                write_allow,
                ..
            }) => {
                assert_eq!(task_id, "r");
                assert!(write_allow.is_empty());
            }
            other => panic!("expected InProgress(r), got {other:?}"),
        }
    }

    #[test]
    fn approved_apply_paths_propagate_through_binding() {
        // M-f ctl apply: the granted out-of-scope paths must survive the
        // collect→resolve→bind step so the gate can honor the exception.
        let mut t = at("a", &["src"], false);
        t.approved_apply_paths = vec!["docs/x.md".to_string()];
        match resolve_active_governance(&[t], None) {
            Some(GovState::InProgress {
                approved_apply_paths,
                ..
            }) => assert_eq!(approved_apply_paths, vec!["docs/x.md".to_string()]),
            other => panic!("expected InProgress with apply paths, got {other:?}"),
        }
    }

    #[test]
    fn simple_commands_classify_directly() {
        assert_eq!(classify_bash("git push origin master"), "git_push");
        assert_eq!(classify_bash("git commit -m x"), "git_commit");
        assert_eq!(classify_bash("cargo add serde"), "cargo_deps");
        assert_eq!(classify_bash("cargo test"), "cargo_build");
        assert_eq!(classify_bash("ls -la"), "bash_other");
    }

    #[test]
    fn compound_commands_use_most_restrictive_segment() {
        // The loophole: a benign prefix must not let a restricted action ride in.
        assert_eq!(
            classify_bash("cp a b && git push origin master"),
            "git_push"
        );
        assert_eq!(classify_bash("echo hi; git commit -m x"), "git_commit");
        assert_eq!(classify_bash("ls | cargo add serde"), "cargo_deps");
        assert_eq!(classify_bash("git commit -m x && git push"), "git_push");
        assert_eq!(classify_bash("true || cargo install foo"), "cargo_deps");
    }

    #[test]
    fn subshell_and_backtick_segments_are_scanned() {
        assert_eq!(classify_bash("echo $(git push)"), "git_push");
        assert_eq!(classify_bash("x=`git push`"), "git_push");
    }
}
