use super::*;

pub(super) fn cmd_run(command: &RunCommands, dry_run: bool) -> Result<()> {
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
            } else if crate::adapters::supported_adapters().contains(&adapter.as_str()) {
                let event = app.run_ingest(id, std::path::Path::new(result), adapter)?;
                println!(
                    "Ingested {} result for task '{}' at seq {}.",
                    adapter, id, event.seq
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
        RunCommands::MergeCandidate { run, json } => {
            let verdict = app.run_merge_candidate(run)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&verdict)?);
            } else {
                let mergeable = verdict
                    .get("mergeable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let touched = verdict
                    .get("touched_files")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!(
                    "Run {}: {} ({} touched file(s))",
                    run,
                    if mergeable { "MERGEABLE" } else { "BLOCKED" },
                    touched
                );
                if let Some(reasons) = verdict.get("blocking_reasons").and_then(|v| v.as_array()) {
                    for r in reasons {
                        if let Some(s) = r.as_str() {
                            println!("  - blocker: {}", s);
                        }
                    }
                }
                if let Some(rec) = verdict.get("recovery").and_then(|v| v.as_array()) {
                    for r in rec {
                        if let Some(a) = r.get("action").and_then(|v| v.as_str()) {
                            println!("  -> recover: {}", a);
                        }
                    }
                }
            }
        }
        RunCommands::Recover {
            abort,
            reason,
            json,
        } => {
            if let Some(run_id) = abort {
                app.abort_run(run_id, reason)?;
                println!(
                    "Recovery: aborted run '{}' — write scope freed, worktree cleaned up.",
                    run_id
                );
                return Ok(());
            }
            let report = app.recover_report()?;
            let orphans = app.orphaned_run_worktrees()?;
            let partial_starts = app.partial_start_runs()?;
            // M6 shared-.git hardening: disclose any shared-state lock alongside
            // the run recovery view (facts only — recover never clears a lock).
            let shared_git =
                crate::infrastructure::workspace::scan_shared_git_risk(&std::env::current_dir()?);
            if *json {
                let out = serde_json::json!({
                    "running": report,
                    "orphaned_worktrees": orphans,
                    "partial_starts": partial_starts,
                    "shared_git_risk": shared_git,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            if report.is_empty() {
                println!("No Running runs.");
            } else {
                println!(
                    "{:<38} {:<16} {:<10} {:<10} {:<8} {:<6} WRITE_SCOPE",
                    "RUN_ID", "TASK_ID", "WORKTREE", "MANIFEST", "LEASE", "USES"
                );
                for r in &report {
                    println!(
                        "{:<38} {:<16} {:<10} {:<10} {:<8} {:<6} {}",
                        r.run_id,
                        r.task_id,
                        if r.worktree_exists {
                            "present"
                        } else {
                            "MISSING"
                        },
                        if r.manifest_exists {
                            "present"
                        } else {
                            "missing"
                        },
                        r.lease_status,
                        r.remaining_uses
                            .map(|u| u.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        r.write_allow.join(",")
                    );
                }
                let inconsistent: Vec<&str> = report
                    .iter()
                    .filter(|r| !r.worktree_exists)
                    .map(|r| r.run_id.as_str())
                    .collect();
                if !inconsistent.is_empty() {
                    println!(
                        "\n{} run(s) have a MISSING worktree (likely a crash). Recover with `ctl run recover --abort <run_id>`.",
                        inconsistent.len()
                    );
                }
                let stale: Vec<&str> = report
                    .iter()
                    .filter(|r| r.lease_stale)
                    .map(|r| r.run_id.as_str())
                    .collect();
                if !stale.is_empty() {
                    println!(
                        "{} run(s) hold a lease past its TTL (reported only — not auto-expired): {}",
                        stale.len(),
                        stale.join(", ")
                    );
                }
                let nonactive: Vec<&str> = report
                    .iter()
                    .filter(|r| r.lease_nonactive)
                    .map(|r| r.run_id.as_str())
                    .collect();
                if !nonactive.is_empty() {
                    println!(
                        "{} Running run(s) have a non-active lease (anomaly): {}",
                        nonactive.len(),
                        nonactive.join(", ")
                    );
                }
            }
            if !partial_starts.is_empty() {
                println!(
                    "\n{} Queued run(s) hold a lease but never started (likely a crash mid-start). Recover with `ctl run recover --abort <run_id>`:",
                    partial_starts.len()
                );
                for p in &partial_starts {
                    println!(
                        "  {} (task {})",
                        p.get("run_id").and_then(|v| v.as_str()).unwrap_or("?"),
                        p.get("task_id").and_then(|v| v.as_str()).unwrap_or("?")
                    );
                }
            }
            if !orphans.is_empty() {
                println!("\nOrphaned worktrees (run terminal/absent — safe to remove):");
                for o in &orphans {
                    println!("  {}", o);
                }
            }
            if shared_git.any() {
                println!(
                    "\nShared .git risk (reported only — not auto-cleared; remove a lock \
                     only when no git process is running):"
                );
                for d in shared_git.descriptions() {
                    println!("  - {}", d);
                }
            }
        }
        RunCommands::ExpireLease { run, apply, json } => {
            let report = app.expire_run_lease(run, *apply)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let mark = match report.outcome.as_str() {
                    "expired" => "EXPIRED",
                    "would_expire" => "WOULD EXPIRE (preview)",
                    _ => "NO-OP",
                };
                println!("Run '{}': {} — {}", report.run_id, mark, report.detail);
                if report.outcome == "would_expire" {
                    println!("Re-run with --apply to record lease_expired.");
                }
            }
        }
        RunCommands::Finish {
            run,
            model,
            provider,
            instruction_artifact,
            context_artifact,
            output_artifact,
            started_at,
            ended_at,
            exit_code,
        } => {
            let prov = crate::application::RunProvenanceInput {
                model: model.clone(),
                provider: provider.clone(),
                instruction_artifact: instruction_artifact.clone(),
                context_artifact: context_artifact.clone(),
                output_artifact: output_artifact.clone(),
                started_at: started_at.clone(),
                ended_at: ended_at.clone(),
                exit_code: *exit_code,
            };
            app.finish_run_with_provenance(run, &prov)?;
            println!(
                "Finished run '{}' — phase Completed; recorded host-attested provenance where \
                 supplied (record-and-disclose). Lease revoked and worktree cleaned up if present.",
                run
            );
        }
    }
    Ok(())
}

pub(super) fn cmd_workspace(command: &WorkspaceCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn print_merge_candidate(v: &Value) {
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

pub(super) fn cmd_apply(id: &str, path: &str, reason: &str, ttl: u64, dry_run: bool) -> Result<()> {
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

pub(super) fn cmd_approval(command: &ApprovalCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn cmd_adapter(command: &AdapterCommands) -> Result<()> {
    match command {
        AdapterCommands::Capabilities { adapter } => {
            let app = app_open(false)?;
            let caps = app.adapter_capabilities(adapter)?;
            println!("{}", serde_json::to_string_pretty(&caps)?);
        }
        AdapterCommands::List { json } => {
            let app = app_open(false)?;
            let adapters = app.adapter_list();
            if *json {
                println!("{}", serde_json::to_string_pretty(&adapters)?);
            } else {
                print_adapter_list(&adapters);
            }
        }
        AdapterCommands::Status {
            adapter,
            json,
            verify,
        } => {
            let app = app_open(false)?;
            let diag = app.adapter_status(adapter, *verify);
            if *json {
                println!("{}", serde_json::to_string_pretty(&diag)?);
            } else {
                print_adapter_diagnostic(&diag);
            }
            if diag.has_failures() {
                return Err(anyhow::anyhow!(
                    "Adapter '{}' has {} failing check(s)",
                    adapter,
                    diag.counts.fail
                ));
            }
        }
        AdapterCommands::Doctor { json, verify } => {
            let app = app_open(false)?;
            let report = app.adapter_doctor(*verify);
            if *json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_adapter_doctor(&report);
            }
            if report.failed > 0 {
                return Err(anyhow::anyhow!(
                    "Adapter doctor found {} adapter(s) with failing check(s)",
                    report.failed
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn print_adapter_list(adapters: &[crate::adapters::AdapterSummary]) {
    if adapters.is_empty() {
        println!("No adapters registered.");
        return;
    }
    // Size the ADAPTER/OUTPUT columns to their widest value (min = header width).
    let name_w = adapters
        .iter()
        .map(|a| a.adapter.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let fmt_w = adapters
        .iter()
        .map(|a| a.output_format.len())
        .max()
        .unwrap_or(6)
        .max(6);
    println!("{:<name_w$}  {:<fmt_w$}  CAPABILITIES", "ADAPTER", "OUTPUT");
    for a in adapters {
        println!(
            "{:<name_w$}  {:<fmt_w$}  {}",
            a.adapter,
            a.output_format,
            a.capabilities.join(", ")
        );
    }
    println!("\n{} adapter(s) registered.", adapters.len());
}

/// One factual counts line over a [`StatusTally`] — no composite score.
pub(super) fn tally_line(t: &crate::adapters::StatusTally) -> String {
    format!(
        "{} PASS · {} FAIL · {} WARN · {} UNKNOWN · {} NOT_TRACKED",
        t.pass, t.fail, t.warn, t.unknown, t.not_tracked
    )
}

pub(super) fn print_adapter_diagnostic(diag: &crate::adapters::AdapterDiagnostic) {
    // Factual headline: did any check FAIL? (WARN/UNKNOWN/NOT_TRACKED are not
    // failures.) Deliberately not a "health" verdict or score.
    let headline = if diag.counts.fail > 0 { "FAIL" } else { "OK" };
    println!(
        "Adapter '{}': {} (resolved={})",
        diag.adapter, headline, diag.resolved
    );
    for c in &diag.checks {
        println!("  [{}] {}: {}", c.status.label(), c.name, c.detail);
    }
    println!("  counts: {}", tally_line(&diag.counts));
}

pub(super) fn print_adapter_doctor(report: &crate::adapters::AdapterDoctorReport) {
    for diag in &report.adapters {
        print_adapter_diagnostic(diag);
        println!();
    }
    println!(
        "{} adapters · {} without failures · {} with failures",
        report.total, report.healthy, report.failed
    );
    println!("checks: {}", tally_line(&report.counts));
    if report.failed > 0 {
        println!("Next: ctl adapter status --adapter <name> to inspect a failing adapter");
    }
}

pub(super) fn cmd_agent_report() -> Result<()> {
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
