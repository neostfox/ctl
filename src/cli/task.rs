use super::*;

pub(super) fn cmd_task(command: &TaskCommands, dry_run: bool) -> Result<()> {
    let app = app_open(dry_run)?;
    match command {
        TaskCommands::Create {
            id,
            objective,
            read_scope,
            write_allow,
            write_deny,
            risk_triggers,
            tdd,
            gates,
            depends_on,
            kind,
            audit_tier,
        } => {
            // `--tdd` is sugar for adding the opt-in risk trigger.
            let mut triggers = risk_triggers.clone();
            if *tdd
                && !triggers
                    .iter()
                    .any(|t| t == crate::application::TDD_RED_GREEN_TRIGGER)
            {
                triggers.push(crate::application::TDD_RED_GREEN_TRIGGER.to_string());
            }
            // Gates: explicit `--gates` win; otherwise derive the project default
            // floor recorded by /ctl-spec. The app still requires a
            // non-empty gate set, so a project with no floor must pass `--gates`.
            let gates = resolve_task_gates(&app.project_root, gates)?;
            let event = app.create_task_with_kind(
                id,
                CreateTaskInput {
                    objective,
                    read_scope,
                    write_allow,
                    write_deny,
                    risk_triggers: &triggers,
                    gates: &gates,
                    depends_on,
                },
                kind.to_domain(),
                audit_tier.to_domain(),
            )?;
            println!(
                "Created {} task '{}' at seq {}.",
                kind.to_domain().as_str(),
                id,
                event.seq
            );
            // Pipeline nudge (non-blocking, record-only provenance): remind the
            // creator to link the task back to its alignment artifact.
            println!(
                "hint: no alignment provenance attached — if this task derived from an \
                 alignment note (.ctl/spec/alignment/), record it: ctl brainstorm record \
                 --id {} --brainstorm BS-xxx --divergence <note-path>",
                id
            );
        }
        TaskCommands::Propose {
            id,
            objective,
            read_scope,
            write_allow,
            write_deny,
            risk_triggers,
            tdd,
            gates,
            depends_on,
            kind,
            audit_tier,
        } => {
            let mut triggers = risk_triggers.clone();
            if *tdd
                && !triggers
                    .iter()
                    .any(|t| t == crate::application::TDD_RED_GREEN_TRIGGER)
            {
                triggers.push(crate::application::TDD_RED_GREEN_TRIGGER.to_string());
            }
            let gates = resolve_task_gates(&app.project_root, gates)?;
            let event = app.propose_task_with_kind(
                id,
                CreateTaskInput {
                    objective,
                    read_scope,
                    write_allow,
                    write_deny,
                    risk_triggers: &triggers,
                    gates: &gates,
                    depends_on,
                },
                kind.to_domain(),
                audit_tier.to_domain(),
            )?;
            println!(
                "Proposed {} task '{}' at seq {} — awaits human approval (ctl task approve --id {}).",
                kind.to_domain().as_str(),
                id,
                event.seq,
                id
            );
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
            // Same gate resolution as `create`: explicit `--gates` win, else the
            // project default floor (no hardcoded list — set by /ctl-spec).
            let gate_list = resolve_task_gates(&app.project_root, gates)?;
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
            // Pipeline nudge (non-blocking, record-only provenance): same as
            // `create` — quick tasks that came from an alignment note should
            // still carry the link.
            println!(
                "hint: no alignment provenance attached — if this task derived from an \
                 alignment note (.ctl/spec/alignment/), record it: ctl brainstorm record \
                 --id {} --brainstorm BS-xxx --divergence <note-path>",
                task_id
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
        TaskCommands::Approve { id } => {
            let event = app.approve_task(id)?;
            println!(
                "Approved task '{}' (Proposed → Ready) at seq {}.",
                id, event.seq
            );
        }
        TaskCommands::Status { id, json } => {
            let state = app.get_status(id)?;
            // M6: derived, cross-task view of which declared dependencies are
            // still unfinished. Computed here (not persisted) so the frozen
            // task-view schema / task.json projection is untouched.
            let blocked_by = app.unmet_dependencies(id)?;
            // BS-provenance V1: fact-only provenance block, staleness resolved
            // against the working tree. None when no brainstorm was recorded.
            let provenance = app.brainstorm_provenance_view(&state);
            // Oracle V1: fact-only uncertainty ledger + oracle-source disclosure,
            // surfaced in task status for every kind (None when none recorded).
            let uncertainty = app.uncertainty_ledger_view(&state);
            if *json {
                print_task_state(
                    &state,
                    &blocked_by,
                    provenance.as_ref(),
                    uncertainty.as_ref(),
                )?;
            } else {
                print_task_human(
                    &state,
                    &blocked_by,
                    provenance.as_ref(),
                    uncertainty.as_ref(),
                )?;
                // Research/Spike V1: append the fact-only research output block.
                if state.task_kind == crate::domain::task::TaskKind::Research {
                    if let Some(view) = app.research_output_view(id)? {
                        print!("{}", format_research_output(&view));
                    }
                }
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
            // Observe-mode consumer: recorded ungoverned writes are only worth
            // recording if someone reads them — surface the log at the one
            // moment a human is already reviewing. Windowed to this task
            // (records since its first event) so the number is actionable;
            // best-effort: never fail the finish on a log-read error.
            let decisions_path = app.project_root.join(".ctl").join("decisions.jsonl");
            if let Ok(content) = std::fs::read_to_string(&decisions_path) {
                let records: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
                if !records.is_empty() {
                    let since = first_event_epoch(&app.project_root, id);
                    let windowed = since.map(|s| {
                        records
                            .iter()
                            .filter(|l| {
                                serde_json::from_str::<serde_json::Value>(l)
                                    .ok()
                                    .and_then(|v| v.get("ts").and_then(|t| t.as_u64()))
                                    .is_some_and(|ts| ts >= s)
                            })
                            .count()
                    });
                    match windowed {
                        Some(w) => println!(
                            "observation log: {} record(s) in this task's window ({} total, non-canonical) — review with: ctl decisions",
                            w,
                            records.len()
                        ),
                        None => println!(
                            "observation log: {} non-canonical record(s) in .ctl/decisions.jsonl — review with: ctl decisions",
                            records.len()
                        ),
                    }
                }
            }
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

pub(super) fn optional_slice(values: &[String]) -> Option<&[String]> {
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

pub(super) fn cmd_replay(task_id: Option<&str>) -> Result<()> {
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

pub(super) fn cmd_reconcile() -> Result<()> {
    let app = app_open(false)?;
    let rebuilt = app.reconcile()?;
    println!("Rebuilt {} task projection(s).", rebuilt.len());
    Ok(())
}

pub(super) fn cmd_validate() -> Result<()> {
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

pub(super) fn cmd_doctor() -> Result<()> {
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

pub(super) fn cmd_repair(
    task: Option<&str>,
    run: Option<&str>,
    all: bool,
    cross_ledger: bool,
    apply: bool,
    json: bool,
) -> Result<()> {
    if cross_ledger {
        if task.is_some() || run.is_some() || all {
            return Err(anyhow::anyhow!(
                "--cross-ledger takes no target selector (it scans all task↔run ledgers)"
            ));
        }
        return cmd_repair_cross_ledger(apply, json);
    }
    let selectors = task.is_some() as u8 + run.is_some() as u8 + all as u8;
    if selectors != 1 {
        return Err(anyhow::anyhow!(
            "specify exactly one of --task <id>, --run <id>, --all, or --cross-ledger"
        ));
    }
    let app = app_open(false)?;
    let outcomes = if let Some(t) = task {
        vec![(format!("task {t}"), app.repair_task_ledger(t, apply)?)]
    } else if let Some(r) = run {
        vec![(format!("run {r}"), app.repair_run_ledger(r, apply)?)]
    } else {
        app.repair_all_ledgers(apply)?
    };

    let mut repaired = 0u32;
    for (label, outcome) in &outcomes {
        if outcome.repaired {
            repaired += 1;
            if apply {
                let backup = outcome
                    .backup
                    .as_ref()
                    .map(|p| format!(" (backed up to {})", p.display()))
                    .unwrap_or_default();
                println!(
                    "{label}: REPAIRED — removed {} bytes{backup}",
                    outcome.removed_bytes
                );
            } else {
                println!(
                    "{label}: torn trailing record found ({} bytes) — re-run with --apply to truncate",
                    outcome.removed_bytes
                );
            }
        } else {
            println!("{label}: {}", outcome.detail);
        }
    }
    if repaired == 0 {
        println!("No torn trailing records found.");
    } else if !apply {
        println!(
            "\n{repaired} ledger(s) have a torn tail. This is a dry run — re-run with --apply to repair."
        );
    }
    Ok(())
}

/// `ctl repair --cross-ledger`: detect task↔run↔worktree inconsistencies and,
/// with `--apply`, retire the stale side. Preview is read-only; apply executes
/// each repair (run aborts append `run_aborted` — the audit trail) and continues
/// past individual failures.
pub(super) fn cmd_repair_cross_ledger(apply: bool, json: bool) -> Result<()> {
    let app = app_open(false)?;
    let findings = app.cross_ledger_findings()?;

    if !apply {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "mode": "preview",
                    "inconsistencies": findings,
                }))?
            );
            return Ok(());
        }
        if findings.is_empty() {
            println!("No cross-ledger inconsistencies found.");
            return Ok(());
        }
        println!(
            "{} cross-ledger inconsistency(ies) found (preview — no ledgers modified):",
            findings.len()
        );
        for f in &findings {
            println!("  [{}] {}", f.kind.as_str(), f.detail);
            println!("      -> repair: {}", f.repair.preview());
        }
        println!("\nThis is a preview. Re-run with --apply to execute these repairs.");
        return Ok(());
    }

    // --apply
    let outcomes: Vec<_> = findings
        .iter()
        .map(|f| app.apply_cross_ledger_repair(f))
        .collect();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "mode": "apply",
                "outcomes": outcomes,
            }))?
        );
        return Ok(());
    }
    if outcomes.is_empty() {
        println!("No cross-ledger inconsistencies found.");
        return Ok(());
    }
    for o in &outcomes {
        let mark = if o.applied { "REPAIRED" } else { "FAILED" };
        println!(
            "  [{}] run {} — {} ({})",
            o.kind.as_str(),
            o.run_id,
            o.result,
            mark
        );
    }
    let applied = outcomes.iter().filter(|o| o.applied).count();
    println!("\nApplied {}/{} repair(s).", applied, outcomes.len());
    Ok(())
}

pub(super) fn cmd_review(command: &ReviewCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn print_task_state(
    state: &TaskState,
    blocked_by: &[String],
    provenance: Option<&crate::domain::task::BrainstormProvenanceView>,
    uncertainty: Option<&crate::domain::task::UncertaintyLedgerView>,
) -> Result<()> {
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
        "audit_tier": state.audit_tier,
        // M6: derived dependency-gating view — declared deps not yet Completed.
        // Display-only (this richer CLI JSON is already a superset of the frozen
        // persisted task-view); empty array means nothing blocks a start.
        "blocked_by": blocked_by,
        "gate_results": gate_results,
        "active_run": state.active_run,
        "leases_active": active_leases,
        "pending_approvals": pending_approvals_count,
        // BS-provenance V1: fact-only, display-only (null when none recorded).
        "brainstorm_provenance": provenance,
        // Oracle V1: fact-only uncertainty ledger + oracle sources (null when none).
        // Display-only superset of the frozen persisted task-view; no verdict/score.
        "uncertainties": uncertainty,
        "last_event_seq": state.last_seq,
    });
    println!("{}", serde_json::to_string_pretty(&view)?);
    Ok(())
}

pub(super) fn print_task_human(
    state: &TaskState,
    blocked_by: &[String],
    provenance: Option<&crate::domain::task::BrainstormProvenanceView>,
    uncertainty: Option<&crate::domain::task::UncertaintyLedgerView>,
) -> Result<()> {
    println!("Task: {}", state.id);
    println!("Phase: {:?}", state.phase);
    if state.task_kind == crate::domain::task::TaskKind::Research {
        println!("Kind: research");
    }
    if state.audit_tier == crate::domain::task::AuditTier::Light {
        println!("Audit tier: light");
    }
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
    // BS-provenance V1: fact-only disclosure (omitted when none recorded).
    if let Some(view) = provenance {
        print_brainstorm_provenance(view);
    }
    // Oracle V1: fact-only uncertainty ledger + oracle sources (omitted when none).
    if let Some(view) = uncertainty {
        print!("{}", format_uncertainty_ledger(view));
    }
    println!("Seq: {}", state.last_seq);
    Ok(())
}
