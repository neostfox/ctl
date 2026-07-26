use super::*;

pub(super) fn cmd_audit(id: &str) -> Result<()> {
    let app = app_open(false)?;
    let report = app.generate_audit_report(id)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub(super) fn cmd_report() -> Result<()> {
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

pub(super) fn cmd_board(
    json: bool,
    table: bool,
    active_only: bool,
    include_archived: bool,
) -> Result<()> {
    let app = app_open(false)?;
    let board = app.generate_board()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&board)?);
        return Ok(());
    }

    let empty = Vec::new();
    let all_tasks = board
        .get("tasks")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if all_tasks.is_empty() {
        println!("No tasks found.");
        return Ok(());
    }

    // Filter: --active hides archived; --include-archived overrides.
    let tasks: Vec<&Value> = all_tasks
        .iter()
        .filter(|t| {
            let archived = t.get("archived").and_then(|v| v.as_bool()).unwrap_or(false);
            if active_only && archived {
                return false;
            }
            if !include_archived && archived {
                return false;
            }
            true
        })
        .collect();

    if tasks.is_empty() {
        println!("No tasks match the filter.");
        return Ok(());
    }

    if table {
        render_board_table(&tasks, &board);
    } else {
        render_board_kanban(&tasks);
    }
    Ok(())
}

/// Group tasks into Kanban columns by phase and render side by side.
pub(super) fn render_board_kanban(tasks: &[&Value]) {
    // Column order mirrors the task lifecycle.
    let columns = [
        ("PLANNING", vec!["planning"]),
        ("READY", vec!["ready"]),
        ("IN PROGRESS", vec!["in_progress"]),
        ("REVIEW", vec!["review"]),
        ("DONE", vec!["completed", "cancelled"]),
    ];

    let phase_of = |t: &Value| -> String {
        t.get("phase")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string()
    };
    let id_of = |t: &Value| -> String {
        t.get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string()
    };
    let held_of = |t: &Value| -> bool { t.get("held").and_then(|v| v.as_bool()).unwrap_or(false) };

    // Group tasks into columns.
    let mut grouped: Vec<Vec<String>> = Vec::with_capacity(columns.len());
    for (_, phases) in &columns {
        grouped.push(
            tasks
                .iter()
                .filter(|t| phases.contains(&phase_of(t).as_str()))
                .map(|t| {
                    let id = id_of(t);
                    if held_of(t) {
                        format!("{id} [HELD]")
                    } else {
                        id
                    }
                })
                .collect(),
        );
    }

    // Column width: max(header_len, widest_entry, 4) + 2 padding.
    let widths: Vec<usize> = columns
        .iter()
        .zip(grouped.iter())
        .map(|((label, _), entries)| {
            let max_entry = entries.iter().map(|e| e.len()).max().unwrap_or(0);
            label.len().max(max_entry).max(4) + 2
        })
        .collect();

    // Header row.
    let mut header = String::new();
    for (i, ((label, _), w)) in columns.iter().zip(widths.iter()).enumerate() {
        if i > 0 {
            header.push_str("│ ");
        }
        header.push_str(&format!("{:<w$}", label, w = w - 2));
    }
    println!("{header}");

    // Separator.
    let mut sep = String::new();
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            sep.push_str("┼─");
        }
        sep.push_str(&"─".repeat(w.saturating_sub(2)));
    }
    println!("{sep}");

    // Body rows.
    let max_rows = grouped.iter().map(|c| c.len()).max().unwrap_or(0);
    for row in 0..max_rows {
        let mut line = String::new();
        for (i, entries) in grouped.iter().enumerate() {
            if i > 0 {
                line.push_str("│ ");
            }
            let cell = entries.get(row).map(|s| s.as_str()).unwrap_or("");
            line.push_str(&format!("{:<w$}", cell, w = widths[i] - 2));
        }
        println!("{line}");
    }

    // Summary line.
    let total: usize = grouped.iter().map(|c| c.len()).sum();
    let archived = tasks
        .iter()
        .filter(|t| t.get("archived").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    println!(
        "\n{} task(s) shown{}",
        total,
        if archived > 0 {
            format!(" · {archived} archived (hidden: use --include-archived)")
        } else {
            String::new()
        }
    );
}

/// Legacy table renderer (used with --table).
pub(super) fn render_board_table(tasks: &[&Value], board: &Value) {
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
}

pub(super) fn cmd_telemetry(command: &TelemetryCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn cmd_drift(command: &DriftCommands) -> Result<()> {
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

pub(super) fn cmd_next_action(id: &str, json: bool) -> Result<()> {
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

pub(super) fn cmd_next_task(json: bool) -> Result<()> {
    let app = app_open(false)?;
    let rec = app.next_task()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&rec)?);
        return Ok(());
    }
    match rec.action {
        "start" | "ready" => {
            let id = rec.task_id.as_deref().unwrap_or("?");
            let obj = rec.objective.as_deref().unwrap_or("");
            println!("Next: ctl task {} --id {}", rec.action, id);
            println!("  task: {} — {}", id, obj);
            println!("  rationale: {}", rec.rationale);
            println!(
                "  candidates: {} ready, {} planning",
                rec.ready_candidates, rec.planning_candidates
            );
        }
        _ => {
            println!("No actionable task found.");
            println!("  {}", rec.rationale);
        }
    }
    Ok(())
}
