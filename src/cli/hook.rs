use super::*;

pub(super) fn cmd_hook(command: &HookCommands) -> Result<()> {
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
        HookCommands::WrapupCheck => cmd_hook_wrapup_check(),
    }
}

pub(super) fn cmd_hook_context() -> Result<()> {
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
            let state = app.replay_task(task_id).ok();

            let boundary = state
                .as_ref()
                .map(|s| {
                    serde_json::json!({
                        "write_allow": s.write_allow,
                        "write_deny": s.write_deny,
                        "read_scope": s.read_scope,
                        "gates": s.gates,
                    })
                })
                .unwrap_or(serde_json::json!({}));

            let mut task_obj = serde_json::json!({
                "id": task_id,
                "phase": phase,
                "objective": report.get("objective").and_then(|v| v.as_str()).unwrap_or(""),
                "boundary": boundary,
            });

            // Enrichment — each block is independently fault-tolerant so one
            // failure never kills the whole context injection. These surface
            // deterministic governance signals the model would otherwise have
            // to query manually (drift, blockers, unknowns, provenance).
            if let Some(s) = &state {
                // Drift + recommended next action (M5).
                if let Ok(na) = app.next_action(task_id) {
                    task_obj["next_action"] = serde_json::json!({
                        "action": na.action.as_str(),
                        "rationale": na.rationale,
                    });
                    task_obj["drift_level"] = serde_json::json!(na.level.as_str());
                    task_obj["drift_score"] = serde_json::json!(na.score);
                }

                // Unmet dependencies (M-d) — tasks blocking this one.
                if let Ok(unmet) = app.unmet_dependencies(task_id) {
                    if !unmet.is_empty() {
                        task_obj["blocked_by"] = serde_json::json!(unmet);
                    }
                }

                // Open uncertainties — unknowns this task is carrying.
                if let Some(ledger) = app.uncertainty_ledger_view(s) {
                    let open: Vec<_> = ledger
                        .items
                        .iter()
                        .filter(|u| u.status == "open")
                        .map(|u| {
                            serde_json::json!({
                                "id": u.id,
                                "statement": u.statement,
                            })
                        })
                        .collect();
                    if !open.is_empty() {
                        task_obj["open_uncertainties"] = serde_json::json!(open);
                    }
                }

                // Brainstorm provenance — where this task came from.
                if let Some(prov) = app.brainstorm_provenance_view(s) {
                    task_obj["provenance"] = serde_json::json!({
                        "brainstorm_id": prov.id,
                        "convergence_path": prov
                            .convergence
                            .as_ref()
                            .map(|a| a.path.clone()),
                    });
                }
            }

            active.push(task_obj);
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

    // Knowledge base digest (facts.jsonl) — inject a compact summary so every
    // subsequent session sees accumulated knowledge. Fault-tolerant: missing
    // file → no facts field, never crashes context injection.
    let facts = app.spec_facts_digest().ok();

    let mut output = serde_json::json!({
        "binary": "ctl",
        // Version visibility (B-lite): the governance rules live in this
        // binary, so every session should see WHICH binary answered.
        "ctl_version": env!("CARGO_PKG_VERSION"),
        "tasks": { "total": total, "by_phase": by_phase },
        "active_tasks": active,
        "spec_layers": spec_layers,
    });
    if let Some(digest) = facts {
        output["facts"] = serde_json::json!(digest);
    }

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub(super) fn cmd_hook_breadcrumb() -> Result<()> {
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
pub(super) fn cmd_hook_check_write(target_path: &str) -> Result<()> {
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

    // Reject traversal/UNC/out-of-repo before the scope test, matching the
    // gate proper (`cmd_hook_gate` treats these as Suspicious — hard deny).
    // Without this, `src/../etc/passwd` would match the `src` scope via
    // lexical `Path::starts_with` (component-wise but no ParentDir collapse)
    // and be reported as in_scope.
    if !matches!(
        classify_write_target(&project_root, target_path),
        WriteTarget::InRepo
    ) {
        let output = serde_json::json!({
            "allowed": false,
            "task_id": task_id,
            "path": target_path,
            "write_allow": write_allow,
            "reason": "suspicious or out-of-repo path"
        });
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

pub(super) fn cmd_hook_gate(
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
        "write" | "edit" | "multiedit" => {
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

            // Observe mode: protection and traversal are the write-time hard
            // core; everything else scope-shaped is allowed + recorded + warned.
            let class = classify_write_target(&project_root, target);

            if let WriteTarget::Suspicious(why) = &class {
                let output = serde_json::json!({
                    "allowed": false,
                    "state": gov_state_str(&state),
                    "reason": format!("write target refused classification: {}", why),
                    "remedy": "use a plain repo-relative or absolute path without traversal/UNC components"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
                return Ok(());
            }

            if let WriteTarget::Protected(p) = &class {
                // A granted `ctl apply` exception still authorizes a specific
                // protected path (e.g. Cargo.toml under a deps approval flow).
                if let GovState::InProgress {
                    task_id,
                    approved_apply_paths,
                    ..
                } = &state
                {
                    if path_in_scope(&project_root, target, approved_apply_paths) {
                        let output = serde_json::json!({
                            "allowed": true,
                            "state": "in_progress",
                            "task_id": task_id,
                            "reason": "reviewed out-of-scope exception (ctl apply) on a protected path"
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                        return Ok(());
                    }
                }
                let output = serde_json::json!({
                    "allowed": false,
                    "state": gov_state_str(&state),
                    "reason": format!("protected path: {} — canonical ledgers/schemas/manifests are never writable by default", p),
                    "remedy": "request a reviewed exception: ctl apply --path <p> --reason <why>, then ctl approval grant"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
                return Ok(());
            }

            let out_of_repo = matches!(class, WriteTarget::OutOfRepo);

            match &state {
                GovState::InProgress {
                    task_id,
                    write_allow,
                    approved_apply_paths,
                    ..
                } => {
                    let in_scope =
                        !out_of_repo && path_in_scope(&project_root, target, write_allow);
                    // M-f `ctl apply`: a write outside write_allow is allowed when a
                    // reviewer has granted this exact out-of-scope path (an audited
                    // exception). M-c overlap still applies below.
                    let applied =
                        !in_scope && path_in_scope(&project_root, target, approved_apply_paths);
                    let allowed_by_scope = in_scope || applied;
                    // M-c: even within our own scope (or a granted apply), a write
                    // must not land inside another *active* task's claimed scope.
                    // Checked whenever the write will proceed (which under observe
                    // mode is also the out-of-scope case).
                    let conflict = if !out_of_repo {
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
                    } else if allowed_by_scope {
                        serde_json::json!({
                            "allowed": true,
                            "state": "in_progress",
                            "task_id": task_id,
                            "reason": if in_scope {
                                "within write_allow"
                            } else {
                                "reviewed out-of-scope exception (ctl apply)"
                            }
                        })
                    } else {
                        // Observe: out of scope (or out of repo) under an active
                        // task — allowed, recorded, warned.
                        serde_json::json!({
                            "allowed": true,
                            "state": "in_progress",
                            "task_id": task_id,
                            "record": true,
                            "reason": if out_of_repo {
                                "outside the repository — not governable by task scope (observe mode)"
                            } else {
                                "outside write_allow (observe mode)"
                            },
                            "warning": if out_of_repo {
                                "this write lands outside the repository; it is recorded in .ctl/decisions.jsonl but no task scope can govern it"
                            } else {
                                "this write is outside the active task's write_allow; it is allowed and recorded in .ctl/decisions.jsonl — widen the scope if this belongs to the task"
                            },
                            "remedy": "widen scope via ctl task revise, or request a reviewed exception: ctl apply --path <p> --reason <why>"
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
                    // Observe: no in_progress task (idle/review/completed) —
                    // allowed, recorded, warned. Durable work should still get
                    // a task; the warning says so without blocking.
                    let output = serde_json::json!({
                        "allowed": true,
                        "state": gov_state_str(&state),
                        "record": true,
                        "reason": if out_of_repo {
                            "outside the repository, no active task (observe mode)"
                        } else {
                            "no active in_progress task (observe mode)"
                        },
                        "warning": "no active in_progress task — this ungoverned write is recorded in .ctl/decisions.jsonl; create a ctl task for durable or multi-file work",
                        "remedy": "ctl task quick --write-allow <path> --objective <text>, or continue for trivial edits"
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
            }
        }
        "bash" => {
            let cmd_str = command.unwrap_or("");

            // M6 shared-.git hardening: a destructive git op is denied while any
            // agent run is Running — it could rewrite shared refs/objects or
            // delete a run's worktree, stranding concurrent work. Overlay check:
            // when no run is active the command falls through to normal gating
            // below. If the run store can't be read we fall through too (the
            // shell is never locked out on a transient ctl/store error — same
            // philosophy as the python hook excluding Bash from fail-closed).
            if let Some(op) = detect_shared_git_op(cmd_str) {
                let active = ControlApp::open(&project_root, false)
                    .and_then(|app| app.active_runs())
                    .unwrap_or_default();
                if !active.is_empty() {
                    let run_ids: Vec<&str> = active.iter().map(|r| r.run_id.as_str()).collect();
                    let mut reason = format!(
                        "'{}' is a destructive git operation and {} agent run(s) are active ({}) — \
                         it could rewrite shared refs/objects or delete a run's worktree",
                        op,
                        active.len(),
                        run_ids.join(", ")
                    );
                    let risk =
                        crate::infrastructure::workspace::scan_shared_git_risk(&project_root);
                    if risk.any() {
                        reason.push_str("; a shared .git lock is already present: ");
                        reason.push_str(&risk.descriptions().join("; "));
                    }
                    let output = serde_json::json!({
                        "allowed": false,
                        "state": gov_state_str(&state),
                        "reason": reason,
                        "remedy": "let the run(s) finish, or abort first (ctl run recover --abort <run_id>); inspect with ctl run recover / ctl doctor"
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                    return Ok(());
                }
            }

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
                        // Observe: outside the commit window — allowed, recorded,
                        // warned. The finish interlock still requires the task's
                        // own in-scope work to be committed during Review.
                        let output = serde_json::json!({
                            "allowed": true,
                            "state": gov_state_str(&state),
                            "record": true,
                            "reason": "git commit outside a task's commit window (observe mode)",
                            "warning": "commit outside a Review/Completed window — recorded in .ctl/decisions.jsonl; the canonical flow is ctl task submit → commit → finish",
                            "remedy": "ctl task submit --id <id> opens the commit window for governed work"
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                },
                "git_push" => {
                    // M-g: push rides the same commit window as commit — open
                    // from Review through Completed. Outside it: observe.
                    let in_window =
                        matches!(&state, GovState::Review { .. } | GovState::Completed { .. });
                    let output = if in_window {
                        serde_json::json!({
                            "allowed": true,
                            "state": gov_state_str(&state),
                            "reason": "push window open (review or completed)"
                        })
                    } else {
                        serde_json::json!({
                            "allowed": true,
                            "state": gov_state_str(&state),
                            "record": true,
                            "reason": "git push outside a task's commit window (observe mode)",
                            "warning": "push outside a Review/Completed window — recorded in .ctl/decisions.jsonl; pushes are outward-facing, prefer the governed submit → commit → push flow",
                            "remedy": "ctl task submit --id <id> opens the commit window"
                        })
                    };
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
                "bash_write" => {
                    // gh7 / issue #7 Finding 1: best-effort extract the file
                    // target(s) the shell writes to, and DENY any that are out
                    // of the active task's write_allow or protected — closing
                    // the dogfooded `git checkout --ours <file>` bypass. ctl
                    // cannot parse arbitrary shell: when no target can be
                    // extracted, fall back to observe-mode allow-and-record
                    // (honest: best-effort, not a hard boundary; obfuscation
                    // via eval/env/brace-expansion/$() can still hide a write).
                    // Asymmetric with the write tool, which stays observe-mode
                    // — bash gets the extra deny layer because that is the
                    // channel the agent actually exploited.
                    let targets = extract_bash_write_targets(cmd_str);
                    match &state {
                        GovState::InProgress {
                            task_id,
                            write_allow,
                            approved_apply_paths,
                            ..
                        } => {
                            let mut denied: Vec<String> = Vec::new();
                            for t in &targets {
                                let class = classify_write_target(&project_root, t);
                                let reason = match &class {
                                    WriteTarget::Suspicious(why) => {
                                        Some(format!("refused classification: {}", why))
                                    }
                                    WriteTarget::Protected(p)
                                        if !path_in_scope(
                                            &project_root,
                                            t,
                                            approved_apply_paths,
                                        ) =>
                                    {
                                        Some(format!("protected path: {}", p))
                                    }
                                    WriteTarget::OutOfRepo => None,
                                    _ if path_in_scope(&project_root, t, write_allow) => None,
                                    _ if path_in_scope(&project_root, t, approved_apply_paths) => {
                                        None
                                    }
                                    _ => Some("outside write_allow".to_string()),
                                };
                                if let Some(r) = reason {
                                    denied.push(format!("{} ({})", t, r));
                                }
                            }
                            let output = if !denied.is_empty() {
                                serde_json::json!({
                                    "allowed": false,
                                    "state": "in_progress",
                                    "task_id": task_id,
                                    "record": true,
                                    "reason": format!("bash write target(s) denied: {}", denied.join("; ")),
                                    "remedy": "widen scope via ctl task revise, use the path-scoped Write/Edit tools, or request a reviewed exception: ctl apply --path <p> --reason <why>"
                                })
                            } else if targets.is_empty() {
                                if command_has_opaque_wrapper(cmd_str) {
                                    // gh7 hardening: an opaque wrapper (eval /
                                    // bash -c / sh -c) with no extractable
                                    // target — the classifier CANNOT see
                                    // inside, so it cannot confirm the write is
                                    // in-scope. Fail-closed: deny under an
                                    // active task. Use plain commands (or
                                    // Write/Edit) for governed writes.
                                    serde_json::json!({
                                        "allowed": false,
                                        "state": "in_progress",
                                        "task_id": task_id,
                                        "record": true,
                                        "reason": "opaque bash command (eval/bash -c/sh -c) — write target hidden, cannot verify in-scope (fail-closed, gh7)",
                                        "remedy": "rewrite as a plain command so the write target is visible, or use the path-scoped Write/Edit tools"
                                    })
                                } else {
                                    serde_json::json!({
                                        "allowed": true,
                                        "state": "in_progress",
                                        "task_id": task_id,
                                        "record": true,
                                        "reason": "bash write detected but no file target could be extracted (best-effort classifier) — allowed + recorded, NOT path-scope-checked",
                                        "warning": "ctl's bash classifier is best-effort; obfuscated commands can hide writes. Prefer the path-scoped Write/Edit tools for governed writes."
                                    })
                                }
                            } else {
                                serde_json::json!({
                                    "allowed": true,
                                    "state": "in_progress",
                                    "task_id": task_id,
                                    "record": true,
                                    "reason": format!("bash write target(s) in scope: {}", targets.join(", "))
                                })
                            };
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        _ => {
                            // No active in_progress task — observe-mode warn +
                            // record (scope is undefined without a write_allow).
                            let output = serde_json::json!({
                                "allowed": true,
                                "state": gov_state_str(&state),
                                "record": true,
                                "reason": "bash write with no active in_progress task (observe mode)",
                                "warning": "this shell command appears to write files with no active task — recorded in .ctl/decisions.jsonl; create a ctl task for durable work, or use the path-scoped Write/Edit tools",
                                "remedy": "ctl task quick --write-allow <path> --objective <text>"
                            });
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                    }
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
            // Unknown tool. Host hooks (Claude / opencode / omp) translate
            // their known mutating tools onto `write|edit|bash|task`; an
            // unknown name reaching here is either a host-side translation
            // gap or a direct probe. We default-allow (so legitimate
            // unmapped tools are not locked out) but stamp `record: true`
            // so the call is visible in `.ctl/decisions.jsonl` for review,
            // and surface a warning so operators can spot the gap.
            let output = serde_json::json!({
                "allowed": true,
                "state": gov_state_str(&state),
                "record": true,
                "reason": "unknown tool — default allow (boundary not enforceable for this tool name)",
                "warning": "ctl does not know how to enforce the write boundary for this tool; if it mutates the filesystem, route it through the host hook's write/edit mapping or extend cmd_hook_gate's tool taxonomy",
                "remedy": "if this tool writes files, map it onto `write` in the host hook (see .opencode/plugins/ctl-gate.ts extractGateInput for the pattern)"
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

/// Build one decision-log entry from a host hook's JSON `data`, stamping the
/// wall-clock `ts` and an explicit `canonical: false` label. The label is
/// stamped here (not left to the caller) so every record in `decisions.jsonl`
/// self-identifies as non-canonical advisory evidence — never a task event.
/// Pure (no IO) so the stamping contract is unit-tested directly.
pub(super) fn decision_entry(data: &str, ts: u64) -> Result<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(data)?;
    let mut entry = parsed.as_object().cloned().unwrap_or_default();
    // Stamped last so they always win over any hook-supplied keys of the same name.
    entry.insert("ts".to_string(), serde_json::json!(ts));
    entry.insert("canonical".to_string(), serde_json::json!(false));
    Ok(serde_json::Value::Object(entry))
}

/// Parse a `YYYY-MM-DDTHH:MM:SS[.frac]Z` UTC timestamp to unix seconds.
/// Fractional seconds are ignored; returns None on malformed input. Local to
/// the hook path so the check never depends on event-replay machinery.
pub(super) fn iso8601_utc_to_epoch(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day) = (
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<u32>().ok()?,
        d.next()?.parse::<u32>().ok()?,
    );
    let mut t = time.split(':');
    let (hh, mm) = (
        t.next()?.parse::<u32>().ok()?,
        t.next()?.parse::<u32>().ok()?,
    );
    let ss = t.next()?.split('.').next()?.parse::<u32>().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm), valid for all UTC dates.
    let y_adj = y - i64::from(m <= 2);
    let era = y_adj.div_euclid(400);
    let yoe = y_adj - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + i64::from(hh) * 3_600 + i64::from(mm) * 60 + i64::from(ss);
    u64::try_from(secs).ok()
}

/// Unix epoch of a task's FIRST ledger event — the start of its observation
/// window. Lexical read; None on any error (callers degrade to a total count).
pub(super) fn first_event_epoch(project_root: &Path, task_id: &str) -> Option<u64> {
    let events = project_root
        .join(".ctl")
        .join("tasks")
        .join(task_id)
        .join("events.jsonl");
    let content = std::fs::read_to_string(&events).ok()?;
    let first = content.lines().find(|l| !l.trim().is_empty())?;
    serde_json::from_str::<serde_json::Value>(first)
        .ok()?
        .get("occurred_at")?
        .as_str()
        .and_then(iso8601_utc_to_epoch)
}

/// The most recent `task_completed` event across every task ledger, as
/// `(task_id, unix_epoch)`. Lexical scan — no replay, safe on any ledger.
pub(super) fn latest_completed_task(project_root: &Path) -> Option<(String, u64)> {
    let tasks_dir = project_root.join(".ctl").join("tasks");
    let mut best: Option<(String, u64)> = None;
    for entry in std::fs::read_dir(&tasks_dir).ok()?.flatten() {
        let events = entry.path().join("events.jsonl");
        let Ok(content) = std::fs::read_to_string(&events) else {
            continue;
        };
        for line in content.lines().rev() {
            if !line.contains("\"task_completed\"") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("type").and_then(|t| t.as_str()) != Some("task_completed") {
                continue;
            }
            let ts = v
                .get("occurred_at")
                .and_then(|t| t.as_str())
                .and_then(iso8601_utc_to_epoch);
            let id = v
                .get("task_id")
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_string();
            if let Some(ts) = ts {
                if best.as_ref().is_none_or(|(_, b)| ts > *b) {
                    best = Some((id, ts));
                }
            }
            break; // newest task_completed for this ledger found
        }
    }
    best
}

/// Newest file mtime (unix epoch) under `dir`, recursively. None if the tree
/// is absent or empty.
pub(super) fn newest_mtime_under(dir: &Path) -> Option<u64> {
    let mut newest: Option<u64> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let candidate = if path.is_dir() {
            newest_mtime_under(&path)
        } else {
            entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        };
        if let Some(c) = candidate {
            if newest.is_none_or(|n| c > n) {
                newest = Some(c);
            }
        }
    }
    newest
}

/// Pure wrap-up policy: pending iff a completion exists, no capture write is
/// at-or-after it, and this completion has not already been reminded.
pub(super) fn wrapup_pending(
    finished: u64,
    last_capture: Option<u64>,
    reminded: Option<u64>,
) -> bool {
    let captured = last_capture.is_some_and(|c| c >= finished);
    let already_reminded = reminded == Some(finished);
    !captured && !already_reminded
}

pub(super) fn cmd_hook_wrapup_check() -> Result<()> {
    let project_root = std::env::current_dir()?;
    if !project_root.join(".ctl").exists() {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pending": false, "reason": "no .ctl directory — project not governed"
            }))?
        );
        return Ok(());
    }
    let Some((task_id, finished)) = latest_completed_task(&project_root) else {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pending": false, "reason": "no completed tasks"
            }))?
        );
        return Ok(());
    };
    // Capture targets: project tier (.ctl/spec) + global tier (~/.ctl/memory).
    let mut last_capture = newest_mtime_under(&project_root.join(".ctl").join("spec"));
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let global = Path::new(&home).join(".ctl").join("memory");
        last_capture = match (last_capture, newest_mtime_under(&global)) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
    // Once-guard: non-canonical marker, same trust tier as decisions.jsonl.
    let marker = project_root.join(".ctl").join("wrapup-reminded.json");
    let reminded = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("finished_epoch").and_then(|x| x.as_u64()));
    let pending = wrapup_pending(finished, last_capture, reminded);
    if pending {
        // Auto-mark: emitting a pending report IS the one reminder for this
        // finish. Best-effort — a marker write failure must not fail the hook.
        let _ = std::fs::write(
            &marker,
            serde_json::to_string(&serde_json::json!({
                "finished_epoch": finished, "task_id": task_id, "canonical": false
            }))
            .unwrap_or_default(),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "pending": pending,
            "task_id": task_id,
            "finished_epoch": finished,
            "last_capture_epoch": last_capture,
            "already_reminded": reminded == Some(finished)
        }))?
    );
    Ok(())
}

pub(super) fn cmd_hook_record_decision(data: &str) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let decisions_dir = project_root.join(".ctl");
    fs::create_dir_all(&decisions_dir)?;

    let decisions_path = decisions_dir.join("decisions.jsonl");

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    // `decision_entry` validates that `data` is valid JSON and stamps the label.
    let entry = decision_entry(data, ts)?;

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

pub(super) fn cmd_decisions(limit: usize, json: bool) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let path = project_root.join(".ctl").join("decisions.jsonl");
    let content = fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    println!("{}", format_decisions(&lines, limit, json));
    Ok(())
}

pub(super) fn cmd_hook_spec_status() -> Result<()> {
    let project_root = std::env::current_dir()?;
    let spec_dir = project_root.join(".ctl").join("spec");
    let src_dir = project_root.join("src");

    if !spec_dir.exists() {
        let output = serde_json::json!({
            "has_specs": false,
            "status": "no_specs",
            "message": "Run /ctl-spec to generate specs"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Find the most recent mtime among spec files.
    // spec_dir is guaranteed to exist here — the early return above handles its absence.
    let mut spec_mtime: Option<std::time::SystemTime> = None;
    let mut spec_count = 0u32;
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
            "Source files changed since last spec refresh. Consider running /ctl-spec"
        }
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
