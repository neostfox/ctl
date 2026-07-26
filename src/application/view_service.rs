use super::*;

impl ControlApp {
    /// Build a context snapshot: hash all files within the task read scope.
    pub fn build_context(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;
        let root = &self.project_root;
        let mut file_hashes = Vec::new();

        for scope_path in &state.read_scope {
            let full_path = root.join(scope_path);
            if full_path.is_dir() {
                collect_file_hashes(&full_path, root, &mut file_hashes)?;
            } else if full_path.is_file() {
                let hash = hash_file(&full_path)?;
                let rel = full_path.strip_prefix(root).unwrap_or(&full_path);
                file_hashes.push(serde_json::json!({
                    "path": path_to_payload_string(rel),
                    "hash": hash,
                }));
            }
        }

        let context = serde_json::json!({
            "task_id": task_id,
            "read_scope": state.read_scope,
            "file_count": file_hashes.len(),
            "files": file_hashes,
            "built_at": now_iso8601(),
        });

        let task_dir = self.store.task_dir(task_id)?;
        let context_path = task_dir.join("context.json");
        if !self.dry_run {
            let temp_path = task_dir.join("context.json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&context)?)?;
            std::fs::rename(&temp_path, &context_path)?;
        }

        Ok(context)
    }

    /// Export a structured assignment JSON for external execution (M3).
    /// Reads task state and optional context.json, writes assignment.json atomically.
    pub fn export_assignment(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;

        let objective = state.objective.clone().unwrap_or_default();
        let read_scope: Vec<&String> = state.read_scope.iter().collect();
        let write_allow: Vec<&String> = state.write_allow.iter().collect();
        let write_deny: Vec<&String> = state.write_deny.iter().collect();
        let risk_triggers: Vec<&String> = state.risk_triggers.iter().collect();
        let gates: Vec<&String> = state.gates.iter().collect();

        // Read context.json if available
        let task_dir = self.store.task_dir(task_id)?;
        let context_path = task_dir.join("context.json");
        let context_snapshot: serde_json::Value = if context_path.exists() {
            let raw = std::fs::read_to_string(&context_path)?;
            serde_json::from_str(&raw)?
        } else {
            serde_json::Value::Null
        };

        let assignment = serde_json::json!({
            "schema": "control.assignment.v1",
            "assignment_id": generate_uuid(),
            "task_id": task_id,
            "adapter": "manual",
            "contract": {
                "type": "manual",
                "input": "assignment.json",
                "output": "agent-output.json",
            },
            "objective": objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": risk_triggers,
            "gates": gates,
            "context_hashes": context_snapshot,
            "required_capabilities": ["file_read", "file_write"],
            "acceptance": {
                "all_gates_must_pass": true,
                "scope_enforcement": true,
            },
            "exported_at": now_iso8601(),
        });

        // Atomic write: temp + rename
        let assignment_path = task_dir.join("assignment.json");
        if assignment_path.exists() && !self.dry_run {
            eprintln!(
                "Warning: Overwriting existing assignment.json for task '{}'",
                task_id
            );
        }
        if !self.dry_run {
            let temp_path = task_dir.join("assignment.json.tmp");
            let json_str = serde_json::to_string_pretty(&assignment)?;
            std::fs::write(&temp_path, &json_str)?;
            std::fs::rename(&temp_path, &assignment_path)?;
        }

        Ok(assignment)
    }

    /// Check workspace modifications against task scope.
    /// Returns list of violations (files modified outside write_allow scope).
    pub fn boundary_check(&self, task_id: &str) -> Result<Vec<String>> {
        let state = self.replay_task(task_id)?;
        let root = &self.project_root;
        let mut violations = Vec::new();

        // Collect all files currently in write scope.
        let mut scope_files: std::collections::HashSet<String> = std::collections::HashSet::new();
        for scope_path in &state.write_allow {
            let full_path = root.join(scope_path);
            if full_path.is_dir() {
                collect_files_recursive(&full_path, root, &mut scope_files)?;
            } else if full_path.is_file() {
                let rel = full_path.strip_prefix(root).unwrap_or(&full_path);
                scope_files.insert(rel.to_string_lossy().to_string());
            }
        }

        // Compare against context snapshot if available
        let context_path = self.store.task_dir(task_id)?.join("context.json");
        if context_path.exists() {
            let context: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&context_path)?)?;
            if let Some(files) = context.get("files").and_then(|f| f.as_array()) {
                let mut baseline_map: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                for entry in files {
                    let path = entry.get("path").and_then(|p| p.as_str()).unwrap_or("");
                    let hash = entry.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                    baseline_map.insert(path.to_string(), hash.to_string());
                }

                // Check each current file against baseline
                for file_path in &scope_files {
                    let full_path = root.join(file_path);
                    if full_path.exists() {
                        let current_hash = hash_file(&full_path)?;
                        if let Some(baseline_hash) = baseline_map.get(file_path) {
                            if &current_hash != baseline_hash {
                                // File was modified — check if it's within write scope
                                violations.push(format!("MODIFIED: {}", file_path));
                            }
                        }
                    }
                }

                // Check for deleted files
                for path in baseline_map.keys() {
                    if !scope_files.contains(path) {
                        let full = root.join(path);
                        if !full.exists() {
                            violations.push(format!("DELETED: {}", path));
                        }
                    }
                }
            }
        } else {
            violations
                .push("No context snapshot found. Run 'control context build' first.".to_string());
        }

        Ok(violations)
    }

    /// Run boundary check and record any violations as canonical events.
    /// Returns the list of violation descriptions.
    /// Per STATE-004 / PATH-004: violations generate `boundary_violation_recorded`
    /// events and the task enters hold.
    pub fn boundary_check_and_record(&self, task_id: &str) -> Result<Vec<String>> {
        let violations = self.boundary_check(task_id)?;
        for violation in &violations {
            let payload = serde_json::json!({
                "violation": violation,
                "detected_at": now_iso8601(),
            });
            let event = self.build_event(task_id, "boundary_violation_recorded", payload)?;
            self.validate_and_append(&event)?;
        }
        if !violations.is_empty() && !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(violations)
    }

    /// Rebuild all task views from events (reconcile).
    pub fn reconcile(&self) -> Result<Vec<String>> {
        let task_ids = self.store.task_ids()?;
        let mut rebuilt = Vec::new();
        for task_id in &task_ids {
            let state = self.replay_task(task_id)?;
            self.store.write_task_view(task_id, &state)?;
            rebuilt.push(task_id.clone());
        }
        // M-b: reconcile also projects the cross-task control view.
        self.project_control()?;
        Ok(rebuilt)
    }

    /// Per-task review verdict derived from the soft-layer verdict→evidence
    /// events (M-b). `evidence_rejected` = reviewer found problems for a file;
    /// a later `evidence_accepted` covering that file resolves it. Mirrors the
    /// finish interlock, so the board reflects what actually blocks completion.
    pub(crate) fn review_status_from_events(events: &[Event]) -> &'static str {
        let mut rejected: HashSet<String> = HashSet::new();
        let mut any_accepted = false;
        for e in events {
            match e.event_type.as_str() {
                "evidence_rejected" => {
                    if let Some(f) = e.payload.get("touched_file").and_then(|v| v.as_str()) {
                        if !f.is_empty() {
                            rejected.insert(f.to_string());
                        }
                    }
                }
                "evidence_accepted" => {
                    any_accepted = true;
                    if let Some(files) = e.payload.get("touched_files").and_then(|v| v.as_array()) {
                        for f in files {
                            if let Some(s) = f.as_str() {
                                rejected.remove(s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !rejected.is_empty() {
            "needs_work"
        } else if any_accepted {
            "passed"
        } else {
            "none"
        }
    }

    /// Build the cross-task control view (M-b): one row per task plus aggregate
    /// totals. A deterministic projection over the event ledger — no wall-clock
    /// field, so repeated reconciles stay byte-identical like `task.json`.
    pub fn generate_board(&self) -> Result<serde_json::Value> {
        let task_ids = self.store.task_ids()?;
        let mut rows = Vec::with_capacity(task_ids.len());
        let (mut active, mut held, mut needs_work, mut completed, mut archived) = (0, 0, 0, 0, 0);

        for task_id in &task_ids {
            let events = self.store.read_for_task(task_id)?;
            let mut state = TaskState::new(task_id);
            for event in &events {
                apply(&mut state, event)
                    .map_err(|e| anyhow!("Reducer error at seq {}: {}", event.seq, e))?;
            }

            // "active" aligns with the gateway/review focus set (M-a): a task in
            // a live working phase that has not been archived.
            let is_active =
                !state.is_archived && matches!(state.phase, Phase::InProgress | Phase::Review);
            let review = Self::review_status_from_events(&events);
            let gates_total = state.gates.len();
            let gates_passing = state
                .gates
                .iter()
                .filter(|g| {
                    state
                        .gate_results
                        .get(g.as_str())
                        .map(|r| r.passed)
                        .unwrap_or(false)
                })
                .count();

            if is_active {
                active += 1;
            }
            if state.is_held {
                held += 1;
            }
            if review == "needs_work" {
                needs_work += 1;
            }
            if state.phase == Phase::Completed {
                completed += 1;
            }
            if state.is_archived {
                archived += 1;
            }

            // M5: deterministic drift projection. Signals derive from events +
            // the telemetry evidence index; the rule engine is pure, so this
            // stays wall-clock-free and reconcile remains byte-identical.
            let telemetry = self.store.read_telemetry_for_task(task_id)?;
            let signals = drift_signals_from(&events, &state, &telemetry);
            let report = crate::domain::drift::evaluate(task_id, &signals);
            let action = crate::domain::drift::next_action(&report, state.phase.clone());

            rows.push(serde_json::json!({
                "task_id": task_id,
                "objective": state.objective,
                "phase": state.phase.as_str(),
                "held": state.is_held,
                "active": is_active,
                "archived": state.is_archived,
                "gates_passing": gates_passing,
                "gates_total": gates_total,
                "review": review,
                "write_scope": state.write_allow.iter().collect::<Vec<_>>(),
                "depends_on": state.depends_on.iter().collect::<Vec<_>>(),
                "drift_level": report.level.as_str(),
                "drift_score": report.score,
                "drift_rules": report.fired_ids(),
                "recommended_action": action.action.as_str(),
            }));
        }

        Ok(serde_json::json!({
            "version": 1,
            "totals": {
                "tasks": rows.len(),
                "active": active,
                "held": held,
                "needs_work": needs_work,
                "completed": completed,
                "archived": archived,
            },
            "tasks": rows,
        }))
    }

    /// Append one verified fact to the knowledge base. ctl assigns the fact id,
    /// stamps the timestamp, and records the actor.
    pub fn spec_fact_add(
        &self,
        statement: &str,
        source: &str,
        category: Option<&str>,
    ) -> Result<crate::application::spec::Fact> {
        if statement.trim().is_empty() {
            return Err(anyhow!("Fact statement must not be empty"));
        }
        if source.trim().is_empty() {
            return Err(anyhow!(
                "Fact source must not be empty — a fact without provenance is \
                 an opinion, not knowledge"
            ));
        }
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        let fact = crate::application::spec::Fact {
            fact_id: crate::application::spec::next_fact_id(&facts),
            statement: statement.trim().to_string(),
            source: source.trim().to_string(),
            category: category.map(|c| c.trim().to_string()),
            recorded_at: now_iso8601(),
            recorded_by: self.actor.clone(),
        };
        let path = crate::application::spec::facts_path(&self.project_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(&fact)?;
        // Open in append mode + fsync, mirroring the telemetry append contract.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        // Self-heal a missing trailing newline (mirrors append_jsonl_line).
        if file.metadata()?.len() > 0 {
            let mut existing = String::new();
            use std::io::Read;
            let mut reader = std::fs::File::open(&path)?;
            reader.read_to_string(&mut existing)?;
            if !existing.ends_with('\n') {
                file.write_all(b"\n")?;
            }
        }
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        if !self.dry_run {
            // No projection to rebuild — facts are evidence, not task state.
        }
        Ok(fact)
    }

    /// List facts, optionally filtered by category and/or a search term.
    pub fn spec_fact_list(
        &self,
        category: Option<&str>,
        search: Option<&str>,
    ) -> Result<Vec<crate::application::spec::Fact>> {
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        let filtered = crate::application::spec::filter_facts(&facts, category, search);
        Ok(filtered.into_iter().cloned().collect())
    }

    /// Promote a fact into a curated spec markdown file by appending a
    /// formatted block. The target is relative to `.ctl/spec/` (e.g.
    /// `backend/infrastructure-layer.md`).
    pub fn spec_fact_promote(&self, fact_id: &str, target: &str) -> Result<std::path::PathBuf> {
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        let fact = facts
            .iter()
            .find(|f| f.fact_id == fact_id)
            .ok_or_else(|| anyhow!("Fact '{}' not found in the knowledge base", fact_id))?;

        let spec_root = self.project_root.join(".ctl").join("spec");
        let target_path = spec_root.join(target);

        // Resolve and boundary-check: the target must stay inside .ctl/spec/.
        let resolved = target_path.canonicalize().map_err(|_| {
            anyhow!(
                "Cannot resolve target spec file '{}.md' — does the file exist \
                 under .ctl/spec/?",
                target.trim_end_matches(".md")
            )
        })?;
        if !resolved.starts_with(spec_root.canonicalize().unwrap_or(spec_root.clone())) {
            return Err(anyhow!(
                "Promote target must be inside .ctl/spec/ — got '{}'",
                target
            ));
        }

        let block = crate::application::spec::format_fact_for_promote(fact);
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target_path)?;
        file.write_all(block.as_bytes())?;
        Ok(target_path)
    }

    /// Build a compact digest of the fact store for context injection.
    pub fn spec_facts_digest(&self) -> Result<crate::application::spec::FactsDigest> {
        let facts = crate::application::spec::read_all_facts(&self.project_root)?;
        Ok(crate::application::spec::facts_digest(&facts, 5))
    }

    /// Write the control view to `.ctl/control.json` (reconcile projection,
    /// M-b / M5+). Atomic temp-file replace, mirroring the task.json projections.
    pub fn project_control(&self) -> Result<PathBuf> {
        let board = self.generate_board()?;
        let ctl_dir = self.project_root.join(".ctl");
        std::fs::create_dir_all(&ctl_dir)?;
        let path = ctl_dir.join("control.json");
        let tmp = ctl_dir.join("control.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&board)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    // ── M5: telemetry + drift + next-action (explainable control loop) ──

    pub fn get_status(&self, task_id: &str) -> Result<TaskState> {
        self.replay_task(task_id)
    }

    pub fn replay(&self, task_id: &str) -> Result<TaskState> {
        let state = self.replay_task(task_id)?;
        self.store.write_task_view(task_id, &state)?;
        Ok(state)
    }

    pub fn validate_store(&self) -> Result<Vec<String>> {
        let events = self.store.read_all()?;
        let mut issues = Vec::new();
        let mut seen_command_ids: HashSet<String> = HashSet::new();
        let mut task_seqs: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();

        for (i, event) in events.iter().enumerate() {
            let line = i + 1;

            // Schema field
            if event.schema != "control.event-envelope.v1" {
                issues.push(format!("Line {}: invalid schema '{}'", line, event.schema));
            }

            // Seq ordering per task
            let prev_seq = task_seqs.get(&event.task_id).copied().unwrap_or(0);
            if event.seq <= prev_seq {
                issues.push(format!(
                    "Line {}: seq {} not strictly increasing for task {} (prev {})",
                    line, event.seq, event.task_id, prev_seq
                ));
            }
            task_seqs.insert(event.task_id.clone(), event.seq);

            // Command id uniqueness
            if !seen_command_ids.insert(event.command_id.clone()) {
                issues.push(format!(
                    "Line {}: duplicate command_id '{}'",
                    line, event.command_id
                ));
            }

            // Schema validation (when schemas/ available)
            if let Some(ref validator) = self.validator {
                let json_val = serde_json::to_value(event)
                    .map_err(|e| anyhow!("Line {}: serialization error: {}", line, e))?;
                if let Err(e) = validator.validate_instance(&json_val, &event.schema) {
                    issues.push(format!("Line {}: schema validation: {}", line, e));
                }
            }
        }

        Ok(issues)
    }

    pub fn doctor(&self) -> Result<Vec<String>> {
        use crate::domain::run::RunPhase;
        use crate::domain::task::Phase;

        let mut results = Vec::new();
        let mut score: i32 = 100;
        let mut task_count = 0u32;
        let mut replay_errors = 0u32;
        let mut inconsistencies = 0u32;

        // ── Task ledgers ──
        // Map of replayed task phases, used for the cross-ledger checks below.
        let mut task_phases: std::collections::HashMap<String, Phase> =
            std::collections::HashMap::new();
        match self.store.read_all() {
            Ok(events) => {
                results.push(format!("events.jsonl: OK ({} events)", events.len()));
                let task_ids = self.store.task_ids()?;
                task_count = task_ids.len() as u32;
                for tid in &task_ids {
                    match self.replay_task(tid) {
                        Ok(state) => {
                            results.push(format!(
                                "Task '{}': {:?} (seq {})",
                                tid, state.phase, state.last_seq
                            ));
                            task_phases.insert(tid.clone(), state.phase);
                        }
                        Err(e) => {
                            replay_errors += 1;
                            score -= 15;
                            results.push(format!("Task '{}': REPLAY ERROR: {}", tid, e));
                        }
                    }
                }
            }
            Err(e) => {
                score -= 30;
                results.push(format!("events.jsonl: ERROR: {}", e));
            }
        }

        // ── Run ledgers + cross-ledger consistency ──
        //
        // Concurrent task/run orchestration is EXPERIMENTAL: a task transition and
        // its run-ledger counterpart are two separate appends (each a single-writer
        // append, but with no transaction spanning both). A crash between them can
        // leave the ledgers disagreeing. ctl never auto-repairs this — doctor only
        // surfaces the facts and the manual recovery step.
        let run_store = self.run_store()?;
        let run_ids = run_store.run_ids()?;
        if !run_ids.is_empty() {
            results.push(String::new());
            results.push(format!("Runs: {} total (orchestration)", run_ids.len()));
            for rid in &run_ids {
                match self.replay_run(rid) {
                    Ok(run) => {
                        results.push(format!(
                            "Run '{}': {:?} (task '{}', seq {})",
                            rid, run.phase, run.task_id, run.last_seq
                        ));
                        // Cross-ledger: the run names a task with no ledger.
                        if !run.task_id.is_empty() && !task_phases.contains_key(&run.task_id) {
                            inconsistencies += 1;
                            score -= 10;
                            results.push(format!(
                                "  INCONSISTENCY: run '{}' references task '{}', which has no \
                                 ledger. Recover by replaying the run's events to confirm intent, \
                                 then cancel the orphan run.",
                                rid, run.task_id
                            ));
                        }
                        // Cross-ledger: a live run whose task is already terminal —
                        // the classic non-atomic window (task closed, run not).
                        if run.phase == RunPhase::Running {
                            if let Some(phase) = task_phases.get(&run.task_id) {
                                if matches!(phase, Phase::Completed | Phase::Cancelled) {
                                    inconsistencies += 1;
                                    score -= 10;
                                    results.push(format!(
                                        "  INCONSISTENCY: run '{}' is Running but its task '{}' is \
                                         {:?}. Recover by aborting the run (ctl run abort).",
                                        rid, run.task_id, phase
                                    ));
                                }
                            }
                            // Worktree: a Running run must have its worktree on disk.
                            if let Some(wt) = &run.worktree_path {
                                if !std::path::Path::new(wt).exists() {
                                    inconsistencies += 1;
                                    score -= 10;
                                    results.push(format!(
                                        "  INCONSISTENCY: run '{}' is Running but its worktree '{}' \
                                         is missing. Recover by aborting the run (ctl run abort).",
                                        rid, wt
                                    ));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        replay_errors += 1;
                        score -= 15;
                        results.push(format!("Run '{}': REPLAY ERROR: {}", rid, e));
                    }
                }
            }
        }

        // ── Shared-.git hazards (M6 shared-state hardening) ──
        // git worktrees share one object store + packed-refs, so a stuck lock
        // blocks or corrupts ref/index operations across every worktree. Facts
        // only — doctor never removes a lock (it may be a legitimately in-flight
        // git op); it flags the hazard and points at recovery.
        let shared_git = crate::infrastructure::workspace::scan_shared_git_risk(&self.project_root);
        if shared_git.any() {
            results.push(String::new());
            for d in shared_git.descriptions() {
                score -= 5;
                results.push(format!("WARNING: shared .git — {}", d));
            }
            results.push(
                "Remove a stale lock only when no git process is running; if an active run \
                 holds it, recover via `ctl run recover`."
                    .to_string(),
            );
        }

        // Health Score deductions
        if score < 0 {
            score = 0;
        }
        results.push(String::new());
        results.push(format!("Health Score: {}/100", score));
        results.push(format!(
            "Tasks: {} total, {} replay errors",
            task_count, replay_errors
        ));
        results.push(format!(
            "Runs: {} total, {} cross-ledger inconsistencies",
            run_ids.len(),
            inconsistencies
        ));
        if replay_errors > 0 {
            results.push(
                "A REPLAY ERROR may be a torn trailing record — run `ctl repair --task <id>` \
                 (or --run <id>) to inspect and truncate it."
                    .to_string(),
            );
        }

        Ok(results)
    }

    // ── Ledger torn-tail repair (explicit, opt-in) ──

    /// Detect (and, when `apply`, truncate) a torn trailing record on one task's
    /// event ledger. Read-only when `apply` is false.
    pub fn repair_task_ledger(
        &self,
        task_id: &str,
        apply: bool,
    ) -> Result<crate::infrastructure::store::TailRepair> {
        self.store.repair_task_ledger(task_id, apply)
    }

    /// Detect (and, when `apply`, truncate) a torn trailing record on one run's
    /// event ledger. Read-only when `apply` is false.
    pub fn repair_run_ledger(
        &self,
        run_id: &str,
        apply: bool,
    ) -> Result<crate::infrastructure::store::TailRepair> {
        self.run_store()?.repair_run_ledger(run_id, apply)
    }

    /// Scan every task and run ledger for a torn trailing record, repairing when
    /// `apply`. Returns `(label, outcome)` per ledger.
    pub fn repair_all_ledgers(
        &self,
        apply: bool,
    ) -> Result<Vec<(String, crate::infrastructure::store::TailRepair)>> {
        let mut out = Vec::new();
        for tid in self.store.task_ids()? {
            out.push((
                format!("task {tid}"),
                self.store.repair_task_ledger(&tid, apply)?,
            ));
        }
        let rs = self.run_store()?;
        for rid in rs.run_ids()? {
            out.push((format!("run {rid}"), rs.repair_run_ledger(&rid, apply)?));
        }
        Ok(out)
    }

    // ── Audit & Reports (M3) ──

    /// Generate a deterministic audit report from events + evidence.
    /// The report is deterministic: same events always produce the same report.
    pub fn generate_audit_report(&self, task_id: &str) -> Result<serde_json::Value> {
        let state = self.replay_task(task_id)?;
        let events = self.store.read_for_task(task_id)?;

        // Collect gate results
        let mut gate_reports = Vec::new();
        for gate_id in &state.gates {
            let result = state.gate_results.get(gate_id);
            gate_reports.push(serde_json::json!({
                "gate_id": gate_id,
                "passed": result.map(|r| r.passed).unwrap_or(false),
                "evidence": result.map(|r| r.evidence.as_str()).unwrap_or("no result"),
                "checked_at": result.map(|r| r.checked_at.as_str()).unwrap_or("never"),
            }));
        }

        // Count evidence events
        let evidence_accepted_count = events
            .iter()
            .filter(|e| e.event_type == "evidence_accepted")
            .count();
        let evidence_rejected_count = events
            .iter()
            .filter(|e| e.event_type == "evidence_rejected")
            .count();

        // Check for violations
        let violation_count = events
            .iter()
            .filter(|e| e.event_type == "boundary_violation_recorded")
            .count();

        // Completion interlock check
        let all_gates_pass = state
            .gates
            .iter()
            .all(|g| state.gate_results.get(g).map(|r| r.passed).unwrap_or(false));
        let interlock_verdict = if state.phase == Phase::Review
            && !state.is_held
            && all_gates_pass
            && evidence_rejected_count == 0
        {
            "allow"
        } else if state.phase == Phase::Completed {
            "completed"
        } else {
            "blocked"
        };

        let report = serde_json::json!({
            "schema": "control.audit-report.v1",
            "task_id": task_id,
            "phase": state.phase.as_str(),
            "is_held": state.is_held,
            "is_archived": state.is_archived,
            "objective": state.objective,
            "total_events": events.len(),
            "gates": gate_reports,
            "all_gates_pass": all_gates_pass,
            "evidence_accepted": evidence_accepted_count,
            "evidence_rejected": evidence_rejected_count,
            "violations": violation_count,
            "completion_interlock": {
                "phase_is_review": state.phase == Phase::Review,
                "no_hold": !state.is_held,
                "all_gates_pass": all_gates_pass,
                "no_rejected_evidence": evidence_rejected_count == 0,
                "verdict": interlock_verdict,
            },
            "write_scope": state.write_allow.iter().collect::<Vec<_>>(),
            "write_deny": state.write_deny.iter().collect::<Vec<_>>(),
            "last_seq": state.last_seq,
        });

        // Write report file
        let task_dir = self.store.task_dir(task_id)?;
        let report_path = task_dir.join("audit-report.json");
        if !self.dry_run {
            let temp_path = task_dir.join("audit-report.json.tmp");
            std::fs::write(&temp_path, serde_json::to_string_pretty(&report)?)?;
            std::fs::rename(&temp_path, &report_path)?;
        }

        Ok(report)
    }

    /// Generate a human-readable summary report.
    pub fn generate_status_report(&self) -> Result<Vec<serde_json::Value>> {
        let task_ids = self.store.task_ids()?;
        let mut reports = Vec::new();
        for task_id in &task_ids {
            let state = self.replay_task(task_id)?;
            reports.push(serde_json::json!({
                "task_id": task_id,
                "phase": state.phase.as_str(),
                "is_held": state.is_held,
                "is_archived": state.is_archived,
                "objective": state.objective,
                "gates_total": state.gates.len(),
                "gates_passing": state.gate_results.values().filter(|r| r.passed).count(),
                "last_seq": state.last_seq,
            }));
        }
        Ok(reports)
    }

    // ── Internal helpers ──

    pub fn replay_task(&self, task_id: &str) -> Result<TaskState> {
        let events = self.store.read_for_task(task_id)?;
        if events.is_empty() {
            return Err(anyhow!("Task '{}' not found", task_id));
        }
        let mut state = TaskState::new(task_id);
        for event in &events {
            apply(&mut state, event)
                .map_err(|e| anyhow!("Reducer error at seq {}: {}", event.seq, e))?;
        }
        Ok(state)
    }

    pub fn approval_request(
        &self,
        task_id: &str,
        reason: &str,
        scope: serde_json::Value,
        ttl_seconds: u64,
    ) -> Result<Event> {
        let request_id = generate_uuid();
        let payload = serde_json::json!({
            "request_id": request_id,
            "reason": reason,
            "scope": scope,
            "ttl_seconds": ttl_seconds,
        });
        let event = self.build_event(task_id, "approval_requested", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn approval_grant(&self, task_id: &str, request_id: &str) -> Result<Event> {
        let payload = serde_json::json!({
            "request_id": request_id,
        });
        let event = self.build_event(task_id, "approval_granted", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn approval_deny(&self, task_id: &str, request_id: &str) -> Result<Event> {
        let payload = serde_json::json!({
            "request_id": request_id,
        });
        let event = self.build_event(task_id, "approval_denied", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    // ── M4: Run lifecycle commands ──

    pub fn adapter_capabilities(&self, adapter_name: &str) -> Result<serde_json::Value> {
        let adapter = adapter_for(adapter_name)?;
        Ok(adapter.capabilities())
    }

    /// adapter-doctor-v1: summarize every registered executor adapter.
    pub fn adapter_list(&self) -> Vec<crate::adapters::AdapterSummary> {
        crate::adapters::adapter_list()
    }

    /// adapter-doctor-v1: diagnose one adapter — the Rust `ExecutorAdapter`
    /// contract clauses PLUS host platform integration (control-guard skill,
    /// managed-protocol drift, plugin/hook files, Bun tests). `verify` opts into
    /// live checks (the opencode Bun suite); without it they stay NOT_TRACKED. An
    /// unknown name yields a single failing contract check (never an Err), so the
    /// caller reports the failure uniformly.
    pub fn adapter_status(
        &self,
        adapter_name: &str,
        verify: bool,
    ) -> crate::adapters::AdapterDiagnostic {
        adapter_status_diagnostic(&self.project_root, adapter_name, verify)
    }

    /// adapter-doctor-v1: diagnose every registered adapter. Factual counts only
    /// (no composite health score).
    pub fn adapter_doctor(&self, verify: bool) -> crate::adapters::AdapterDoctorReport {
        adapter_doctor_report(&self.project_root, verify)
    }

    /// Classify every cross-ledger inconsistency (read-only — replays aggregates
    /// and stats the filesystem, never appends).
    ///
    /// One finding per run, chosen by severity: orphan (no task) > stranded
    /// (terminal task) > missing-worktree, plus partial-start for a Queued run
    /// still holding a lease, and orphaned-worktree for a terminal run whose
    /// isolation dir lingers. Reuses the same "active"/"terminal" notions as
    /// `doctor` and `run recover`, so the three views never disagree. A run whose
    /// own ledger is torn is skipped here — that is `ctl repair --run` territory,
    /// not cross-ledger drift.
    pub fn cross_ledger_findings(&self) -> Result<Vec<CrossLedgerFinding>> {
        use crate::domain::run::RunPhase;
        use crate::domain::task::Phase;

        // Task phase map; a missing entry means the task has no ledger.
        let mut task_phases: std::collections::HashMap<String, Phase> =
            std::collections::HashMap::new();
        for tid in self.store.task_ids()? {
            if let Ok(state) = self.replay_task(&tid) {
                task_phases.insert(tid, state.phase);
            }
        }

        let store = self.run_store()?;
        let mut findings = Vec::new();
        for run_id in store.run_ids()? {
            let run = match self.replay_run(&run_id) {
                Ok(r) => r,
                Err(_) => continue, // torn run ledger — not a cross-ledger concern
            };
            let worktree_on_disk = run
                .worktree_path
                .as_ref()
                .map(|w| Path::new(w).exists())
                .unwrap_or(false);
            let task_id = (!run.task_id.is_empty()).then(|| run.task_id.clone());

            match run.phase {
                RunPhase::Running | RunPhase::Queued => {
                    let no_task =
                        !run.task_id.is_empty() && !task_phases.contains_key(&run.task_id);
                    let task_terminal = matches!(
                        task_phases.get(&run.task_id),
                        Some(Phase::Completed) | Some(Phase::Cancelled)
                    );
                    let (kind, detail) = if no_task {
                        (
                            CrossLedgerKind::OrphanRun,
                            format!(
                                "run '{}' is {:?} but references task '{}', which has no ledger",
                                run_id, run.phase, run.task_id
                            ),
                        )
                    } else if task_terminal {
                        (
                            CrossLedgerKind::StrandedRun,
                            format!(
                                "run '{}' is {:?} but its task '{}' is terminal",
                                run_id, run.phase, run.task_id
                            ),
                        )
                    } else if run.phase == RunPhase::Running && !worktree_on_disk {
                        (
                            CrossLedgerKind::MissingWorktreeRun,
                            format!(
                                "run '{}' is Running but its isolated worktree is missing",
                                run_id
                            ),
                        )
                    } else if run.phase == RunPhase::Queued && run.lease.is_some() {
                        (
                            CrossLedgerKind::PartialStartRun,
                            format!(
                                "run '{}' is Queued holding a lease but never started (crash mid-start)",
                                run_id
                            ),
                        )
                    } else {
                        continue; // consistent (active run, live task, worktree present)
                    };
                    findings.push(CrossLedgerFinding {
                        repair: RepairAction::AbortRun {
                            reason: format!("cross-ledger repair: {}", kind.as_str()),
                        },
                        kind,
                        run_id: run_id.clone(),
                        task_id,
                        detail,
                    });
                }
                RunPhase::Completed | RunPhase::Failed | RunPhase::Aborted => {
                    if worktree_on_disk {
                        let path = run.worktree_path.clone().unwrap_or_default();
                        findings.push(CrossLedgerFinding {
                            kind: CrossLedgerKind::OrphanedWorktree,
                            run_id: run_id.clone(),
                            task_id,
                            detail: format!(
                                "run '{}' is {:?} but its worktree dir still exists at {}",
                                run_id, run.phase, path
                            ),
                            repair: RepairAction::RemoveWorktree { path },
                        });
                    }
                }
            }
        }
        Ok(findings)
    }

    /// Apply one cross-ledger repair. Run aborts append `run_aborted`
    /// (+`lease_revoked`) — the canonical repair evidence; worktree removal is
    /// fs-only (the run ledger is already terminal and correct). Errors are
    /// captured in the outcome, not propagated, so a batch apply continues past a
    /// single failure.
    pub fn apply_cross_ledger_repair(&self, finding: &CrossLedgerFinding) -> RepairOutcome {
        let mut outcome = RepairOutcome {
            run_id: finding.run_id.clone(),
            kind: finding.kind,
            applied: false,
            result: String::new(),
        };
        outcome.result = match &finding.repair {
            RepairAction::AbortRun { reason } => match self.abort_run(&finding.run_id, reason) {
                Ok(ev) => {
                    outcome.applied = true;
                    format!("aborted run (run_aborted at seq {})", ev.seq)
                }
                Err(e) => format!("abort failed: {e}"),
            },
            RepairAction::RemoveWorktree { path } => {
                match crate::infrastructure::workspace::cleanup_worktree(
                    &self.project_root,
                    Path::new(path),
                ) {
                    Ok(()) => {
                        outcome.applied = true;
                        format!("removed leftover worktree {path}")
                    }
                    Err(e) => format!("worktree removal failed: {e}"),
                }
            }
        };
        outcome
    }
}
