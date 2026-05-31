use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

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
    Schema {
        #[command(subcommand)]
        command: SchemaCommands,
    },
    Boundary {
        #[command(subcommand)]
        command: BoundaryCommands,
    },
    Architecture {
        #[command(subcommand)]
        command: ArchitectureCommands,
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
        Commands::Schema { command } => match command {
            SchemaCommands::Validate { file } => {
                let content = fs::read_to_string(file)?;
                let instance: Value = serde_json::from_str(&content)?;
                let schema_id = instance
                    .get("schema")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let validator = SchemaValidator::new("schemas/")?;
                validator.validate_instance(&instance, schema_id)?;
                println!("Validation successful for schema: {}", schema_id);
            }
        },
        Commands::Boundary { command } => match command {
            BoundaryCommands::Check { path } => {
                let root = std::env::current_dir()?;
                let normalizer = PathNormalizer::new(root);
                match normalizer.normalize(path) {
                    Ok(normalized) => {
                        println!("PASS: {}", normalized.display());
                    }
                    Err(e) => {
                        println!("REJECT: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            BoundaryCommands::Explain { path } => {
                boundary_explain(path)?;
            }
        },
        Commands::Architecture { command } => match command {
            ArchitectureCommands::Check => {
                println!("Running architecture checks...");
                check_schemas()?;
                check_dependencies()?;
                check_modules()?;
                check_baseline_manifest()?;
                check_state_transitions()?;
                println!("All architecture checks passed.");
            }
        },
    }
    Ok(())
}

fn boundary_explain(path_str: &str) -> Result<()> {
    let root = std::env::current_dir()?;
    let normalizer = PathNormalizer::new(root.clone());

    println!("Boundary explain for: {}", path_str);
    println!("Root: {}", root.display());
    println!("---");

    // Run each check individually and report
    if path_str.starts_with("\\\\") || path_str.starts_with("//") {
        println!("[REJECT] UNC path detected");
        return Ok(());
    }
    println!("[PASS]  Not a UNC path");

    let path = Path::new(path_str);
    if path.is_absolute() {
        println!("[REJECT] Absolute path");
        return Ok(());
    }
    println!("[PASS]  Not an absolute path");

    let has_parent_dir = path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir));
    if has_parent_dir {
        println!("[REJECT] Contains '..' component");
        return Ok(());
    }
    println!("[PASS]  No '..' components");

    // Check if normalize succeeds end-to-end
    match normalizer.normalize(path_str) {
        Ok(normalized) => {
            println!("[PASS]  Not a protected path");
            println!("[PASS]  No symlink/junction in ancestry");
            println!("[PASS]  Does not escape root");
            println!("---");
            println!("RESULT: ACCEPTED -> {}", normalized.display());
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("protected") {
                println!("[REJECT] Protected path: {}", msg);
            } else if msg.contains("Symlink") || msg.contains("Junction") {
                println!("[PASS]  Not a protected path");
                println!("[REJECT] Symlink/Junction: {}", msg);
            } else if msg.contains("escapes") {
                println!("[PASS]  Not a protected path");
                println!("[PASS]  No symlink/junction in ancestry");
                println!("[REJECT] Root escape: {}", msg);
            } else {
                println!("[REJECT] {}", msg);
            }
            println!("---");
            println!("RESULT: REJECTED");
        }
    }

    Ok(())
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
    // Blacklist: check Cargo.lock for forbidden transitive deps
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

    // Whitelist: direct dependencies in Cargo.toml must match M0 allowed set
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

    // main.rs is also required
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
    // Schema files — exact set match
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

    // Fixture files — exact set match
    let expected_fixtures = [
        "reducer_test.jsonl",
        "reducer_lifecycle.jsonl",
        "reducer_hold.jsonl",
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
            9,
        ),
        ("fixtures/reducer_hold.jsonl", "t-hold", Phase::Completed, 8),
    ];
    for (path, task_id, expected_phase, expected_history) in &fixture_files {
        let content = fs::read_to_string(path)?;
        let mut state = TaskState::new(task_id);
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            // Schema validation: verify protocol conformance before reducer
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
            // Reducer replay: verify state machine behavior
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
