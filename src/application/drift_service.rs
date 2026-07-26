use super::*;

impl ControlApp {
    /// Recommend the next task to advance. Deterministic: among Ready tasks whose
    /// dependencies are all Completed and whose write scope does not overlap any
    /// active in_progress task, pick the lowest drift score (ties broken by task id
    /// for stable output). Falls back to the lowest-drift Planning task when no
    /// Ready task is actionable. Read-only — emits no events.
    pub fn next_task(&self) -> Result<NextTaskRecommendation> {
        use crate::application::schedule::detect_write_scope_overlap;
        use std::collections::HashMap;

        let board = self.generate_board()?;
        let empty = vec![];
        let tasks = board["tasks"].as_array().unwrap_or(&empty);

        // Phase lookup for dependency satisfaction (Completed satisfies).
        let phase_by_id: HashMap<String, String> = tasks
            .iter()
            .filter_map(|t| {
                Some(t["task_id"].as_str()?.to_string()).zip(t["phase"].as_str().map(String::from))
            })
            .collect();

        // Active in_progress write scopes (for overlap detection).
        let active_scopes: Vec<BTreeSet<String>> = tasks
            .iter()
            .filter(|t| {
                t["phase"].as_str() == Some("in_progress")
                    && !t["archived"].as_bool().unwrap_or(false)
            })
            .filter_map(|t| {
                let set: BTreeSet<String> = t["write_scope"]
                    .as_array()?
                    .iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect();
                Some(set)
            })
            .collect();

        let deps_satisfied = |task: &serde_json::Value| -> bool {
            match task["depends_on"].as_array() {
                None => true,
                Some(deps) => deps.iter().all(|d| {
                    let dep_id = d.as_str().unwrap_or("");
                    phase_by_id
                        .get(dep_id)
                        .map(|p| p == "completed")
                        .unwrap_or(false)
                }),
            }
        };

        let no_scope_conflict = |task: &serde_json::Value| -> bool {
            let scopes: BTreeSet<String> = task["write_scope"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            active_scopes
                .iter()
                .all(|active| detect_write_scope_overlap(&scopes, active).is_empty())
        };

        let rank = |a: &serde_json::Value, b: &serde_json::Value| {
            let sa = a["drift_score"].as_i64().unwrap_or(0);
            let sb = b["drift_score"].as_i64().unwrap_or(0);
            sa.cmp(&sb).then_with(|| {
                a["task_id"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["task_id"].as_str().unwrap_or(""))
            })
        };

        let is_ready = |t: &serde_json::Value| {
            t["phase"].as_str() == Some("ready")
                && !t["held"].as_bool().unwrap_or(false)
                && !t["archived"].as_bool().unwrap_or(false)
                && deps_satisfied(t)
                && no_scope_conflict(t)
        };
        let is_planning = |t: &serde_json::Value| {
            t["phase"].as_str() == Some("planning")
                && !t["held"].as_bool().unwrap_or(false)
                && !t["archived"].as_bool().unwrap_or(false)
        };

        let mut ready: Vec<&serde_json::Value> = tasks.iter().filter(|t| is_ready(t)).collect();
        ready.sort_by(|a, b| rank(a, b));

        let mut planning: Vec<&serde_json::Value> =
            tasks.iter().filter(|t| is_planning(t)).collect();
        planning.sort_by(|a, b| rank(a, b));

        let ready_count = ready.len();
        let planning_count = planning.len();

        if let Some(best) = ready.first() {
            let score = best["drift_score"].as_i64().unwrap_or(0);
            return Ok(NextTaskRecommendation {
                action: "start",
                task_id: best["task_id"].as_str().map(String::from),
                objective: best["objective"].as_str().map(String::from),
                rationale: format!(
                    "ready, dependencies satisfied, lowest drift (score {score}), \
                 no active scope conflict"
                ),
                ready_candidates: ready_count,
                planning_candidates: planning_count,
            });
        }

        if let Some(best) = planning.first() {
            return Ok(NextTaskRecommendation {
                action: "ready",
                task_id: best["task_id"].as_str().map(String::from),
                objective: best["objective"].as_str().map(String::from),
                rationale: "no actionable ready task; lowest-drift planning task".to_string(),
                ready_candidates: ready_count,
                planning_candidates: planning_count,
            });
        }

        Ok(NextTaskRecommendation {
            action: "none",
            task_id: None,
            objective: None,
            rationale: "no actionable tasks (all completed, archived, held, or \
            blocked by unsatisfied dependencies)"
                .to_string(),
            ready_candidates: ready_count,
            planning_candidates: planning_count,
        })
    }

    // ── Spec fact store (knowledge-accumulation-v1) ──
    //
    // Atomic verified facts captured during conversations, persisted to
    // `.ctl/facts.jsonl` (append-only evidence, NOT canonical events). Two
    // tiers: raw facts (this store) + curated spec markdown (promote). The
    // digest is injected into `ctl hook context` so every subsequent session
    // sees accumulated knowledge. Record-and-disclose — never gates.

    /// Append one telemetry evidence record to the evidence index (M5). This is
    /// the only M5 write op; drift/next-action are read-only projections. The
    /// `recorded_at` provenance timestamp is stamped here (the domain stays
    /// time-free). Unknown `kind`s are accepted as evidence but the drift engine
    /// fails closed on them.
    pub fn telemetry_add(
        &self,
        task_id: &str,
        kind: &str,
        value: i64,
        source: &str,
    ) -> Result<crate::domain::telemetry::TelemetryEntry> {
        // The task must exist so telemetry is always attributable.
        if self.store.read_for_task(task_id)?.is_empty() {
            return Err(anyhow!("Task '{}' does not exist", task_id));
        }
        let entry = crate::domain::telemetry::TelemetryEntry::new(
            task_id,
            kind,
            value,
            &now_iso8601(),
            source,
        );
        if self.dry_run {
            println!(
                "[dry-run] Would append telemetry: task={}, kind={}, value={}",
                task_id, kind, value
            );
            return Ok(entry);
        }
        self.store.append_telemetry(&entry)?;
        Ok(entry)
    }

    /// Derive the drift signals for a task from the event ledger and the
    /// telemetry evidence index. Pure projection — emits no events.
    pub(crate) fn collect_drift_signals(
        &self,
        task_id: &str,
    ) -> Result<(crate::domain::drift::DriftSignals, Phase)> {
        let events = self.store.read_for_task(task_id)?;
        if events.is_empty() {
            return Err(anyhow!("Task '{}' does not exist", task_id));
        }
        let mut state = TaskState::new(task_id);
        for event in &events {
            apply(&mut state, event)
                .map_err(|e| anyhow!("Reducer error at seq {}: {}", event.seq, e))?;
        }
        let telemetry = self.store.read_telemetry_for_task(task_id)?;
        let signals = drift_signals_from(&events, &state, &telemetry);
        Ok((signals, state.phase))
    }

    /// Compute the drift report for a task (M5). Read-only.
    pub fn compute_drift(&self, task_id: &str) -> Result<crate::domain::drift::DriftReport> {
        let (signals, _phase) = self.collect_drift_signals(task_id)?;
        Ok(crate::domain::drift::evaluate(task_id, &signals))
    }

    /// Recommend the next action for a task (M5). Read-only and advisory — it
    /// emits no events and, for replan/rescope, only returns a structured
    /// proposal for a human to act on.
    pub fn next_action(&self, task_id: &str) -> Result<crate::domain::drift::NextActionProposal> {
        let (signals, phase) = self.collect_drift_signals(task_id)?;
        let report = crate::domain::drift::evaluate(task_id, &signals);
        Ok(crate::domain::drift::next_action(&report, phase))
    }
}
