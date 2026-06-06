use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use crate::application::{ControlApp, CreateTaskInput, ReviseTaskInput};
use crate::domain::event::Event;
use crate::domain::task::{apply, Phase, TaskState};
use crate::infrastructure::boundary::normalizer::PathNormalizer;
use crate::infrastructure::schema_validator::SchemaValidator;

#[derive(Parser)]
#[command(name = "control")]
#[command(about = "AI Dev Control Plane CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the local task ledger
    Init,
    /// M1 task lifecycle commands
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
    /// Architecture compliance checks
    Architecture {
        #[command(subcommand)]
        command: ArchitectureCommands,
    },
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
}

#[derive(Subcommand)]
enum ArchitectureCommands {
    Check,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Init => cmd_init(),
        Commands::Task { command } => cmd_task(command),
        Commands::Replay { task } => cmd_replay(task.as_deref()),
        Commands::Validate => cmd_validate(),
        Commands::Doctor => cmd_doctor(),
        Commands::Schema { command } => cmd_schema(command),
        Commands::Boundary { command } => cmd_boundary(command),
        Commands::Architecture { command } => cmd_architecture(command),
    }
}

fn app_open() -> Result<ControlApp> {
    ControlApp::open(&std::env::current_dir()?)
}

fn cmd_init() -> Result<()> {
    ControlApp::init(&std::env::current_dir()?)?;
    println!("Initialized local task ledger.");
    Ok(())
}

fn cmd_task(command: &TaskCommands) -> Result<()> {
    let app = app_open()?;
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
        TaskCommands::Status { id } => {
            let state = app.get_status(id)?;
            print_task_state(&state)?;
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
    let app = app_open()?;
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

fn cmd_validate() -> Result<()> {
    let app = app_open()?;
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
    let app = app_open()?;
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

fn print_task_state(state: &TaskState) -> Result<()> {
    let gate_results: BTreeMap<_, _> = state.gate_results.iter().collect();
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
        "last_event_seq": state.last_seq,
    });
    println!("{}", serde_json::to_string_pretty(&view)?);
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
    }
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
        ".trellis",
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
    let mut expected: Vec<&str> = vec!["anyhow", "clap", "serde", "serde_json"];
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
        "reducer_test.jsonl",
        "reducer_lifecycle.jsonl",
        "reducer_hold.jsonl",
        "reducer_revise.jsonl",
        "schema_counter_examples.json",
        "invalid.json",
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
            "init",
            "task",
            "replay",
            "validate",
            "doctor",
            "schema",
            "boundary",
            "architecture",
        ],
    )?;

    let task_command = command
        .get_subcommands()
        .find(|cmd| cmd.get_name() == "task")
        .ok_or_else(|| anyhow::anyhow!("Missing M1 task command"))?;
    assert_exact_subcommands(
        "task CLI",
        task_command.get_subcommands().map(|cmd| cmd.get_name()),
        ["create", "revise", "ready", "status"],
    )?;

    let forbidden = [
        "context",
        "gate",
        "run",
        "reconcile",
        "assignment",
        "ingest",
        "audit",
        "report",
        "workspace",
        "approval",
        "adapter",
        "telemetry",
        "drift",
        "next-action",
        "nextaction",
        "schedule",
        "agent",
        "start",
        "cancel",
        "submit",
        "finish",
        "archive",
    ];
    let mut names = Vec::new();
    collect_subcommand_names(&command, &mut names);
    for forbidden_cmd in &forbidden {
        if names.iter().any(|name| name == forbidden_cmd) {
            return Err(anyhow::anyhow!(
                "Milestone scope violation: CLI exposes non-M1 command '{}'",
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
            "schemas/control.event-envelope.v1.schema.json",
            "schemas/control.task-definition.v1.schema.json",
            "schemas/control.task-view.v1.schema.json",
        ],
        &["\"scope\""],
    )?;

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
