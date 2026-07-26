use super::*;

pub(super) fn cmd_ralph(command: &RalphCommands) -> Result<()> {
    match command {
        RalphCommands::Run {
            id,
            max_iters,
            max_secs,
            interval_secs,
            kill_switch,
        } => cmd_ralph_run(id, *max_iters, *max_secs, *interval_secs, kill_switch),
    }
}

/// Bounded, read-only safety supervisor (ralph-safe-run-v1). Loops a GO/NO-GO
/// safety check around an unattended run with three independent hard stops —
/// kill-switch file, wall-clock deadline, and a max-cycle cap — and halts the
/// instant the check reports NO-GO. This is the governance envelope, NOT an
/// executor: it spawns nothing and writes no code (ctl never spawns).
pub(super) fn cmd_ralph_run(
    id: &str,
    max_iters: u64,
    max_secs: u64,
    interval_secs: u64,
    kill_switch: &str,
) -> Result<()> {
    let app = app_open(false)?;
    let kill = std::path::Path::new(kill_switch);
    let start = std::time::Instant::now();

    println!(
        "ralph supervisor for '{id}' — max_iters={max_iters}, max_secs={}, interval={interval_secs}s, kill-switch={kill_switch}",
        if max_secs == 0 { "none".to_string() } else { max_secs.to_string() }
    );
    println!("(read-only watchdog — spawns no executor, writes no code)");

    let mut iter: u64 = 0;
    let stop_reason = loop {
        // Hard stops, checked before each cycle.
        if kill.exists() {
            break format!("kill-switch present ({kill_switch})");
        }
        if max_secs > 0 && start.elapsed().as_secs() >= max_secs {
            break format!("deadline reached ({max_secs}s)");
        }
        if iter >= max_iters {
            break format!("max iterations reached ({max_iters})");
        }
        iter += 1;

        let verdict = app.ralph_safety_check(id)?;
        if verdict.go {
            println!("  cycle {iter}: GO — safe to continue");
        } else {
            println!("  cycle {iter}: NO-GO — human attention needed:");
            for b in &verdict.blockers {
                println!("      - {b}");
            }
            break format!("safety NO-GO at cycle {iter}");
        }

        if interval_secs > 0 {
            std::thread::sleep(std::time::Duration::from_secs(interval_secs));
        }
    };

    println!("STOP: {stop_reason}. Supervised {iter} cycle(s).");
    Ok(())
}

pub(super) fn cmd_research(command: &ResearchCommands) -> Result<()> {
    match command {
        ResearchCommands::Record {
            id,
            kind,
            artifact,
            source_run,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event = app.record_research_artifact(
                id,
                artifact,
                kind.as_payload(),
                source_run.as_deref(),
            )?;
            println!(
                "Recorded {} artifact for task '{}' at seq {}.",
                kind.as_payload(),
                id,
                event.seq
            );
        }
        ResearchCommands::Status { id, json } => {
            let app = app_open(false)?;
            let view = app.research_output_view(id)?;
            match view {
                Some(view) if *json => println!("{}", serde_json::to_string_pretty(&view)?),
                Some(view) => print!("{}", format_research_output(&view)),
                None if *json => println!("null"),
                None => println!("Task '{}' is not a research task (no research output).", id),
            }
        }
    }
    Ok(())
}

pub(super) fn cmd_dispatch(command: &DispatchCommands) -> Result<()> {
    match command {
        DispatchCommands::Record {
            task,
            role,
            adapter,
            run,
            instruction_artifact,
            context_artifact,
            output_artifact,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event = app.record_subagent_dispatch(
                task,
                role,
                adapter,
                run.as_deref(),
                instruction_artifact.as_deref(),
                context_artifact.as_deref(),
                output_artifact.as_deref(),
            )?;
            println!(
                "Recorded subagent dispatch on task '{}' at seq {} — role={}, adapter={} \
                 (host-attested: ctl records what it was told was dispatched, not what ran).",
                task, event.seq, role, adapter
            );
        }
        DispatchCommands::List { task } => {
            let app = app_open(false)?;
            let state = app.replay_task(task)?;
            if state.dispatches.is_empty() {
                println!("No subagent dispatches recorded on task '{}'.", task);
                return Ok(());
            }
            println!(
                "Subagent dispatches on task '{}' (host-attested — ctl records what it was \
                 told, never verifies what ran):",
                task
            );
            for (i, d) in state.dispatches.iter().enumerate() {
                let run = d
                    .parent_run
                    .as_ref()
                    .map(|r| format!(" run={r}"))
                    .unwrap_or_default();
                println!("  {}. role={} adapter={}{}", i + 1, d.role, d.adapter, run);
                for (label, art) in [
                    ("instruction", &d.instruction),
                    ("context", &d.context),
                    ("output", &d.output),
                ] {
                    if let Some(a) = art {
                        println!("       {label}: {} @ {}", a.path, a.hash);
                    }
                }
                println!("       recorded_by={} (unattested label)", d.recorded_by);
            }
        }
    }
    Ok(())
}
