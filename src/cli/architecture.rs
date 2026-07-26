use super::*;
/// The full architecture compliance suite, as a named registry so `check`
/// (fail-fast CI gate) and `review` (full health snapshot) run the exact same
/// set in the exact same order — they can never silently diverge.
pub(super) type ArchCheck = (&'static str, fn() -> Result<()>);

pub(super) fn architecture_checks() -> Vec<ArchCheck> {
    vec![
        ("schemas", check_schemas),
        ("dependencies", check_dependencies),
        ("modules", check_modules),
        ("baseline_manifest", check_baseline_manifest),
        ("state_transitions", check_state_transitions),
        ("milestone_scope", check_milestone_scope),
        (
            "canonical_task_ledger_contract",
            check_canonical_task_ledger_contract,
        ),
        (
            "schema_payload_completeness",
            check_schema_payload_completeness,
        ),
        ("schema_counter_examples", check_schema_counter_examples),
        ("fixture_paths_gates", check_fixture_paths_gates),
    ]
}

pub(super) fn cmd_architecture(command: &ArchitectureCommands) -> Result<()> {
    match command {
        ArchitectureCommands::Check => {
            // Fail-fast: stop at the first violation (unchanged CI-gate behavior).
            for (_name, check) in architecture_checks() {
                check()?;
            }
            println!("All architecture checks passed.");
            Ok(())
        }
        ArchitectureCommands::Review { json } => cmd_architecture_review(*json),
    }
}

/// `ctl architecture review`: run every check (no fail-fast), report each
/// outcome, and exit non-zero if any failed. Built for periodic/scheduled
/// checkups where the full picture matters more than the first failure.
pub(super) fn cmd_architecture_review(json: bool) -> Result<()> {
    let results: Vec<(&str, Option<String>)> = architecture_checks()
        .into_iter()
        .map(|(name, check)| (name, check().err().map(|e| e.to_string())))
        .collect();
    let failed = results.iter().filter(|(_, e)| e.is_some()).count();
    let total = results.len();

    if json {
        let checks: Vec<_> = results
            .iter()
            .map(|(name, err)| {
                serde_json::json!({ "check": name, "passed": err.is_none(), "error": err })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "total": total,
                "passed": total - failed,
                "failed": failed,
                "checks": checks,
            }))?
        );
    } else {
        println!("Architecture review — {total} checks:");
        for (name, err) in &results {
            match err {
                None => println!("  PASS  {name}"),
                Some(e) => println!("  FAIL  {name} — {e}"),
            }
        }
        println!("\n{}/{} passed, {} failed.", total - failed, total, failed);
    }

    if failed > 0 {
        // Non-zero exit for cron/CI; the full report is already on stdout.
        return Err(anyhow::anyhow!(
            "architecture review: {failed}/{total} check(s) failed"
        ));
    }
    Ok(())
}

pub(super) fn check_schemas() -> Result<()> {
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

pub(super) fn check_dependencies() -> Result<()> {
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
        // A section header toggles collection: we govern the portable
        // `[dependencies]` table AND the unix-only target table (where `libc`
        // lives, for process-group signalling). Scanning the target table too
        // keeps the whitelist from being bypassed via a `[target.*]` section.
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]" || trimmed == "[target.'cfg(unix)'.dependencies]";
            continue;
        }
        // Skip blank lines and comments so rationale comments inside a deps
        // table are not mistaken for dependency entries.
        if !in_deps || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(name) = trimmed.split('=').next() {
            let name = name.trim().to_string();
            if !name.is_empty() {
                found_deps.push(name);
            }
        }
    }
    found_deps.sort();
    // `ureq` is the single network client, admitted by ADR 0002 for the
    // `ctl update` self-updater only (native-tls backend, no async runtime).
    // The forbidden scan above still blocks reqwest/tokio/hyper/async-std.
    let mut expected: Vec<&str> = vec![
        "anyhow",
        "clap",
        "libc",
        "serde",
        "serde_json",
        "sha2",
        "ureq",
    ];
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

pub(super) fn check_modules() -> Result<()> {
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

pub(super) fn check_baseline_manifest() -> Result<()> {
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

pub(super) fn check_state_transitions() -> Result<()> {
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

pub(super) fn check_milestone_scope() -> Result<()> {
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
            "brainstorm",
            "context",
            "decisions",
            "dispatch",
            "doctor",
            "drift",
            "gate",
            "handoff",
            "hook",
            "init",
            "next-action",
            "next-task",
            "prd",
            "ralph",
            "reconcile",
            "repair",
            "replay",
            "report",
            "research",
            "review",
            "run",
            "schedule",
            "schema",
            "self-update",
            "skills",
            "spec",
            "task",
            "telemetry",
            "uncertainty",
            "update",
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
            "create", "propose", "quick", "revise", "ready", "approve", "status", "start",
            "submit", "reopen", "finish", "cancel", "archive",
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

pub(super) fn assert_exact_subcommands<'a>(
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

pub(super) fn check_canonical_task_ledger_contract() -> Result<()> {
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

pub(super) fn check_task_boundary_schema_contract() -> Result<()> {
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

pub(super) fn event_payload_schema<'a>(schema: &'a Value, event_type: &str) -> Result<&'a Value> {
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

pub(super) fn check_files_absent(paths: &[&str], patterns: &[&str]) -> Result<()> {
    for path in paths {
        check_file_content_absent(Path::new(path), patterns)?;
    }
    Ok(())
}

pub(super) fn check_file_content_absent(path: &Path, patterns: &[&str]) -> Result<()> {
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

pub(super) fn check_schema_payload_completeness() -> Result<()> {
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

pub(super) fn check_schema_counter_examples() -> Result<()> {
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

pub(super) fn check_fixture_paths_gates() -> Result<()> {
    let known_gates: HashSet<&str> = [
        "cargo_fmt_check",
        "cargo_check",
        "cargo_test",
        "cargo_clippy",
        "tsc_check",
        "eslint_check",
        "vitest_run",
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
