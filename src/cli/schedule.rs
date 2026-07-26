use super::*;

pub(super) fn cmd_schedule(command: &ScheduleCommands, dry_run: bool) -> Result<()> {
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

pub(super) fn cmd_schedule_plan(
    max_concurrent: usize,
    tasks: &[String],
    dry_run: bool,
) -> Result<()> {
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
pub(super) fn load_plan_and_states(
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

pub(super) fn cmd_schedule_validate(plan_id: &str) -> Result<()> {
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

pub(super) fn cmd_schedule_run(
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
