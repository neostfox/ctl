use super::*;

pub(super) fn cmd_schema(command: &SchemaCommands) -> Result<()> {
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

pub(super) fn cmd_boundary(command: &BoundaryCommands) -> Result<()> {
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

pub(super) fn cmd_gate(command: &GateCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn cmd_context(command: &ContextCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn cmd_assignment(command: &AssignmentCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn boundary_explain(path_str: &str) -> Result<()> {
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
            println!("  Rule PATH-003: path '{}' is protected — allowed in write_allow, but the runtime gate denies writes unless a `ctl apply` exception is granted", p);
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
