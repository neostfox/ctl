use super::*;

pub(super) fn print_brainstorm_provenance(view: &crate::domain::task::BrainstormProvenanceView) {
    print!("{}", format_brainstorm_provenance(view));
}

pub(super) fn cmd_brainstorm(command: &BrainstormCommands) -> Result<()> {
    match command {
        BrainstormCommands::Record {
            id,
            brainstorm,
            divergence,
            convergence,
            source_run,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event = app.record_brainstorm_artifacts(
                id,
                brainstorm,
                divergence,
                convergence.as_deref(),
                source_run.as_deref(),
            )?;
            println!(
                "Recorded brainstorm '{}' artifacts for task '{}' at seq {}.",
                brainstorm, id, event.seq
            );
        }
        BrainstormCommands::AttachCritic {
            id,
            brainstorm,
            critic,
            source_run,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event =
                app.attach_brainstorm_critic(id, brainstorm, critic, source_run.as_deref())?;
            println!(
                "Attached critic artifact to brainstorm '{}' on task '{}' at seq {} \
                 (independence: unattested).",
                brainstorm, id, event.seq
            );
        }
        BrainstormCommands::SkipCritic {
            id,
            brainstorm,
            reason,
            decided_by,
            source_run,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event = app.skip_brainstorm_critic(
                id,
                brainstorm,
                reason,
                decided_by.as_deref(),
                source_run.as_deref(),
            )?;
            println!(
                "Recorded critic skip for brainstorm '{}' on task '{}' at seq {}.",
                brainstorm, id, event.seq
            );
        }
        BrainstormCommands::Show { id, json } => {
            let app = app_open(false)?;
            let state = app.get_status(id)?;
            let provenance = app.brainstorm_provenance_view(&state);
            match provenance {
                Some(view) if *json => {
                    println!("{}", serde_json::to_string_pretty(&view)?);
                }
                Some(view) => print_brainstorm_provenance(&view),
                None if *json => println!("null"),
                None => println!("No brainstorm provenance recorded for task '{}'.", id),
            }
        }
    }
    Ok(())
}

pub(super) fn cmd_uncertainty(command: &UncertaintyCommands) -> Result<()> {
    match command {
        UncertaintyCommands::Record {
            id,
            uncertainty,
            statement,
            source,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event = app.record_uncertainty(id, uncertainty, statement, source.as_deref())?;
            println!(
                "Recorded uncertainty '{}' on task '{}' at seq {}.",
                uncertainty, id, event.seq
            );
        }
        UncertaintyCommands::Evidence {
            id,
            evidence,
            oracle_kind,
            source,
            artifact,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event = app.record_evidence(
                id,
                evidence,
                oracle_kind.as_payload(),
                source.as_deref(),
                artifact,
            )?;
            println!(
                "Recorded {} evidence '{}' on task '{}' at seq {}.",
                oracle_kind.as_payload(),
                evidence,
                id,
                event.seq
            );
        }
        UncertaintyCommands::Dispose {
            id,
            uncertainty,
            disposition,
            evidence_ref,
            evidence,
            reason,
            dry_run,
        } => {
            let app = app_open(*dry_run)?;
            let event = app.record_uncertainty_disposition(
                id,
                uncertainty,
                disposition.as_payload(),
                evidence.as_deref(),
                evidence_ref.as_deref(),
                reason.as_deref(),
            )?;
            println!(
                "Recorded disposition '{}' for uncertainty '{}' on task '{}' at seq {}.",
                disposition.as_payload(),
                uncertainty,
                id,
                event.seq
            );
        }
        UncertaintyCommands::Status { id, json } => {
            let app = app_open(false)?;
            let state = app.get_status(id)?;
            let ledger = app.uncertainty_ledger_view(&state);
            match ledger {
                Some(view) if *json => println!("{}", serde_json::to_string_pretty(&view)?),
                Some(view) => print!("{}", format_uncertainty_ledger(&view)),
                None if *json => println!("null"),
                None => println!("No uncertainties recorded for task '{}'.", id),
            }
        }
    }
    Ok(())
}
