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
}

#[derive(Subcommand)]
enum TaskCommands {
    /// Create a Planning task with a structured M1 boundary
    Create {
        /// Stable task identifier; maps to .trellis/tasks/<id>/
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
        Commands::Run { command } => cmd_run(command, dry_run),
        Commands::Workspace { command } => cmd_workspace(command, dry_run),
        Commands::Approval { command } => cmd_approval(command, dry_run),
        Commands::Adapter { command } => cmd_adapter(command),
        Commands::Architecture { command } => cmd_architecture(command),
        Commands::Schedule { command } => cmd_schedule(command, dry_run),
        Commands::Hook { command } => cmd_hook(command),
        Commands::AgentReport => cmd_agent_report(),
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
                },
            )?;
            println!("Created task '{}' at seq {}.", id, event.seq);
        }
        TaskCommands::Revise {
            id,
            objective,
            read_scope,
            write_allow,
            write_deny,
            risk_triggers,
            gates,
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
            if *json {
                print_task_state(&state)?;
            } else {
                print_task_human(&state)?;
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
    }
    Ok(())
}

fn cmd_approval(command: &ApprovalCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        ApprovalCommands::Request { id, reason, ttl } => {
            let scope = serde_json::json!({});
            let event = app.approval_request(id, reason, scope, *ttl)?;
            println!(
                "Created approval request for task '{}' at seq {}.",
                id, event.seq
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

fn print_task_state(state: &TaskState) -> Result<()> {
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
        "gate_results": gate_results,
        "active_run": state.active_run,
        "leases_active": active_leases,
        "pending_approvals": pending_approvals_count,
        "last_event_seq": state.last_seq,
    });
    println!("{}", serde_json::to_string_pretty(&view)?);
    Ok(())
}

fn print_task_human(state: &TaskState) -> Result<()> {
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

    for entry in fs::read_dir(domain_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().is_none_or(|e| e != "rs") {
            return Err(anyhow::anyhow!(
                "Non-rust file in domain module: {:?}",
                path
            ));
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
            "approval",
            "architecture",
            "assignment",
            "audit",
            "boundary",
            "context",
            "doctor",
            "gate",
            "init",
            "reconcile",
            "replay",
            "report",
            "run",
            "schedule",
            "schema",
            "task",
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
            "create", "revise", "ready", "status", "start", "submit", "reopen", "finish", "cancel",
            "archive",
        ],
    )?;

    // Post-M4 commands that must not appear yet (M5 items)
    let forbidden = ["telemetry", "drift", "next-action", "nextaction"];
    let mut names = Vec::new();
    collect_subcommand_names(&command, &mut names);
    for forbidden_cmd in &forbidden {
        if names.iter().any(|name| name == forbidden_cmd) {
            return Err(anyhow::anyhow!(
                "Milestone scope violation: CLI exposes post-M4 command '{}'",
                forbidden_cmd
            ));
        }
    }

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

fn collect_subcommand_names(command: &clap::Command, names: &mut Vec<String>) {
    for subcommand in command.get_subcommands() {
        names.push(subcommand.get_name().to_string());
        collect_subcommand_names(subcommand, names);
    }
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

    // Collect task states
    let mut task_data = Vec::new();
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
        task_data.push((task_id.clone(), state.write_allow.clone()));
    }

    let plan = crate::application::schedule::plan_schedule(&task_data, max_concurrent);

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

fn cmd_schedule_validate(_plan: &str) -> Result<()> {
    // TODO: Full validation requires reading plan file and re-checking task states
    eprintln!("Schedule validation not yet implemented (requires plan persistence)");
    Ok(())
}

fn cmd_schedule_run(_plan: &str, _poll_interval: u64, _timeout: u64, _dry_run: bool) -> Result<()> {
    // TODO: Full execution engine requires file locking and process management
    eprintln!(
        "Schedule execution not yet implemented (requires file locking + process supervision)"
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
        } => cmd_hook_gate(
            tool,
            path.as_deref(),
            command.as_deref(),
            agent_type.as_deref(),
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
        if phase == "inprogress" {
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
        "InProgress": "Implement in worktree, then `/ctl-apply` → `/ctl-close`",
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
                if phase == "InProgress" && active.as_ref().is_none_or(|(_, _, t)| mtime > *t) {
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
    },
    /// A task is in review
    Review,
    /// A task is completed but not yet archived (commit window)
    Completed {
        task_id: String,
        write_allow: Vec<String>,
    },
}

fn compute_gov_state(project_root: &Path) -> Result<GovState> {
    let tasks_dir = project_root.join(".ctl").join("tasks");
    if !tasks_dir.exists() {
        return Ok(GovState::Ungoverned);
    }

    let app = ControlApp::open(project_root, false)?;
    let reports = app.generate_status_report()?;

    // Priority: Held > InProgress > Completed > Review > Idle
    for report in &reports {
        let phase = report.get("phase").and_then(|v| v.as_str()).unwrap_or("");
        let is_held = report
            .get("is_held")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let is_archived = report
            .get("is_archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let task_id = report
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if is_archived {
            continue;
        }

        if phase == "inprogress" {
            let state = app.replay_task(&task_id)?;
            return Ok(GovState::InProgress {
                task_id,
                write_allow: state.write_allow.iter().cloned().collect(),
                is_held,
            });
        }
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
            return Ok(GovState::Review);
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
fn classify_bash(command: &str) -> &'static str {
    let cmd = command.trim();
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
        GovState::Review => "review",
        GovState::Completed { .. } => "completed",
    }
}

fn cmd_hook_gate(
    tool: &str,
    path: Option<&str>,
    command: Option<&str>,
    agent_type: Option<&str>,
) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let state = compute_gov_state(&project_root)?;

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
                    ..
                } => {
                    let in_scope = path_in_scope(&project_root, target, write_allow);
                    let output = serde_json::json!({
                        "allowed": in_scope,
                        "state": "in_progress",
                        "task_id": task_id,
                        "reason": if in_scope { "within write_allow" } else { "outside write_allow" },
                        "remedy": if in_scope { "" } else { "widen scope via ctl task revise or use /ctl-apply" }
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
                    GovState::Completed {
                        task_id,
                        write_allow,
                    } => {
                        let output = serde_json::json!({
                            "allowed": true,
                            "state": "completed",
                            "task_id": task_id,
                            "reason": "commit window open for completed task",
                            "scope": write_allow
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                    _ => {
                        let output = serde_json::json!({
                            "allowed": false,
                            "state": gov_state_str(&state),
                            "reason": "git commit only allowed when task is completed",
                            "remedy": "ctl task submit --id <id> && ctl task finish --id <id>"
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                },
                "git_push" => {
                    let output = serde_json::json!({
                        "allowed": false,
                        "state": gov_state_str(&state),
                        "reason": "git push requires step-up approval",
                        "remedy": "ctl approval request --action push"
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                "cargo_deps" => {
                    let output = serde_json::json!({
                        "allowed": false,
                        "state": gov_state_str(&state),
                        "reason": "dependency changes require step-up approval",
                        "remedy": "ctl approval request --action deps"
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                "cargo_build" => {
                    // cargo check/test/build/fmt allowed in InProgress, Review, Completed
                    let allow = matches!(
                        &state,
                        GovState::InProgress { .. }
                            | GovState::Review
                            | GovState::Completed { .. }
                            | GovState::Idle
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
