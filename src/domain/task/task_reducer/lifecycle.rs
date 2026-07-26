use super::super::*;

pub(crate) fn task_created(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        // R5: Reject duplicate task_created (first event always has last_seq == 0)
        if state.last_seq > 0 {
            return Err("Cannot re-create task: already has events".into());
        }
        let boundary = decode_task_boundary(&event.payload)?;
        state.phase = Phase::Planning;
        state.objective = Some(boundary.objective);
        state.read_scope = boundary.read_scope;
        state.write_allow = boundary.write_allow;
        state.write_deny = boundary.write_deny;
        state.risk_triggers = boundary.risk_triggers;
        state.gates = boundary.gates;
        state.depends_on = boundary.depends_on;
        // Research/Spike V1: kind is fixed at creation and never revised.
        state.task_kind = decode_task_kind(&event.payload)?;
        state.audit_tier = decode_audit_tier(&event.payload)?;
    }
    Ok(())
}

pub(crate) fn task_proposed(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        // gh6 full proposal-mode: a model-proposed task lands in Proposed
        // (not Planning); it cannot be started until a human approves it.
        if state.last_seq > 0 {
            return Err("Cannot propose task: already has events".into());
        }
        let boundary = decode_task_boundary(&event.payload)?;
        state.phase = Phase::Proposed;
        state.objective = Some(boundary.objective);
        state.read_scope = boundary.read_scope;
        state.write_allow = boundary.write_allow;
        state.write_deny = boundary.write_deny;
        state.risk_triggers = boundary.risk_triggers;
        state.gates = boundary.gates;
        state.depends_on = boundary.depends_on;
        state.task_kind = decode_task_kind(&event.payload)?;
        state.audit_tier = decode_audit_tier(&event.payload)?;
    }
    Ok(())
}

pub(crate) fn task_revised(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::Planning && state.phase != Phase::Proposed {
            return Err(format!(
                "Can only revise in Planning or Proposed, current phase: {:?}",
                state.phase
            ));
        }
        let boundary = decode_task_boundary(&event.payload)?;
        state.objective = Some(boundary.objective);
        state.read_scope = boundary.read_scope;
        state.write_allow = boundary.write_allow;
        state.write_deny = boundary.write_deny;
        state.risk_triggers = boundary.risk_triggers;
        state.gates = boundary.gates;
        state.depends_on = boundary.depends_on;
    }
    Ok(())
}

pub(crate) fn task_marked_ready(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::Planning {
            return Err("Can only mark ready from Planning".into());
        }
        let missing_objective = state
            .objective
            .as_ref()
            .map(|objective| objective.is_empty())
            .unwrap_or(true);
        if missing_objective
            || state.read_scope.is_empty()
            || state.write_allow.is_empty()
            || state.gates.is_empty()
        {
            return Err("Missing objective, read_scope, write_allow, or gates for Ready".into());
        }
        state.phase = Phase::Ready;
    }
    Ok(())
}

pub(crate) fn task_approved(state: &mut TaskState, event: &Event) -> Result<(), String> {
    {
        // gh6 full proposal-mode: human-only (actor=human enforced at the
        // REDUCER — stronger than the application-layer check; a non-human
        // approval event fails replay). Proposed -> Ready.
        if state.phase != Phase::Proposed {
            return Err(format!(
                "Can only approve from Proposed, current phase: {:?}",
                state.phase
            ));
        }
        if event.actor != "human" {
            return Err(format!(
                    "task_approved requires actor=human; got '{}' — the model proposes, a human approves",
                    event.actor
                ));
        }
        state.phase = Phase::Ready;
    }
    Ok(())
}

pub(crate) fn task_started(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::Ready {
            return Err(format!(
                "Can only start from Ready, current phase: {:?}",
                state.phase
            ));
        }
        state.phase = Phase::InProgress;
    }
    Ok(())
}

pub(crate) fn task_submitted_for_review(
    state: &mut TaskState,
    _event: &Event,
) -> Result<(), String> {
    {
        if state.phase != Phase::InProgress {
            return Err(format!(
                "Can only submit for review from InProgress, current phase: {:?}",
                state.phase
            ));
        }
        state.phase = Phase::Review;
    }
    Ok(())
}

pub(crate) fn task_reopened(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::Review {
            return Err(format!(
                "Can only reopen from Review, current phase: {:?}",
                state.phase
            ));
        }
        state.phase = Phase::InProgress;
    }
    Ok(())
}

pub(crate) fn task_completed(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::Review {
            return Err(format!(
                "Can only complete from Review, current phase: {:?}",
                state.phase
            ));
        }
        // STATE-012: Completion interlock — all required gates must have
        // a latest passing result before completion.
        for gate_id in &state.gates {
            match state.gate_results.get(gate_id) {
                Some(result) if result.passed => {}
                _ => {
                    return Err(format!(
                        "Completion interlock: gate '{}' has no passing result",
                        gate_id
                    ));
                }
            }
        }
        state.phase = Phase::Completed;
    }
    Ok(())
}

pub(crate) fn task_cancelled(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        if state.phase == Phase::Completed || state.phase == Phase::Cancelled {
            return Err(format!(
                "Cannot cancel from terminal phase: {:?}",
                state.phase
            ));
        }
        state.phase = Phase::Cancelled;
    }
    Ok(())
}

pub(crate) fn task_archived(state: &mut TaskState, _event: &Event) -> Result<(), String> {
    {
        if state.phase != Phase::Completed && state.phase != Phase::Cancelled {
            return Err(format!(
                "Can only archive from terminal phase, current: {:?}",
                state.phase
            ));
        }
        state.is_archived = true;
    }
    Ok(())
}
