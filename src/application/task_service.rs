use super::*;

impl ControlApp {
    pub fn create_task(&self, id: &str, input: CreateTaskInput<'_>) -> Result<Event> {
        self.create_task_with_kind(id, input, TaskKind::Implementation, AuditTier::Full)
    }

    /// Create a task with an explicit kind (Research/Spike V1). `create_task`
    /// delegates here with `Implementation`. The kind is fixed at creation and
    /// never revised; the field is emitted only for research tasks so
    /// implementation payloads stay byte-identical to pre-feature output.
    pub fn create_task_with_kind(
        &self,
        id: &str,
        input: CreateTaskInput<'_>,
        kind: TaskKind,
        audit_tier: AuditTier,
    ) -> Result<Event> {
        let existing = self.store.read_for_task(id)?;
        if !existing.is_empty() {
            return Err(anyhow!("Task '{}' already exists", id));
        }

        let read_scope = self.normalize_boundary_paths("read_scope", input.read_scope)?;
        let write_allow = self.normalize_boundary_paths("write_allow", input.write_allow)?;
        let write_deny = self.normalize_boundary_paths("write_deny", input.write_deny)?;
        let gates = validate_gate_templates(input.gates, &self.project_root)?;
        validate_task_definition(input.objective, &read_scope, &write_allow, &gates)?;

        let mut payload = serde_json::json!({
            "objective": input.objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": input.risk_triggers,
            "gates": gates,
        });
        // M-d: only emit depends_on when non-empty, keeping payloads minimal and
        // dependency-free events byte-identical to pre-M-d output.
        if !input.depends_on.is_empty() {
            payload["depends_on"] = serde_json::json!(input.depends_on);
        }
        if kind != TaskKind::Implementation {
            payload["task_kind"] = serde_json::json!(kind.as_str());
        }
        if audit_tier != AuditTier::Full {
            payload["audit_tier"] = serde_json::json!(audit_tier.as_str());
        }
        let event = self.build_event(id, "task_created", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(id)?;
        }
        Ok(event)
    }

    /// gh6 full proposal-mode: a model-proposed task lands in Proposed (not
    /// Planning); it cannot be started until a human approves it via
    /// [`approve_task`]. Mirrors [`create_task_with_kind`] but emits
    /// `task_proposed` so the reducer sets phase = Proposed.
    pub fn propose_task_with_kind(
        &self,
        id: &str,
        input: CreateTaskInput<'_>,
        kind: TaskKind,
        audit_tier: AuditTier,
    ) -> Result<Event> {
        let existing = self.store.read_for_task(id)?;
        if !existing.is_empty() {
            return Err(anyhow!("Task '{}' already exists", id));
        }
        let read_scope = self.normalize_boundary_paths("read_scope", input.read_scope)?;
        let write_allow = self.normalize_boundary_paths("write_allow", input.write_allow)?;
        let write_deny = self.normalize_boundary_paths("write_deny", input.write_deny)?;
        let gates = validate_gate_templates(input.gates, &self.project_root)?;
        validate_task_definition(input.objective, &read_scope, &write_allow, &gates)?;
        let mut payload = serde_json::json!({
            "objective": input.objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": input.risk_triggers,
            "gates": gates,
        });
        if !input.depends_on.is_empty() {
            payload["depends_on"] = serde_json::json!(input.depends_on);
        }
        if kind != TaskKind::Implementation {
            payload["task_kind"] = serde_json::json!(kind.as_str());
        }
        if audit_tier != AuditTier::Full {
            payload["audit_tier"] = serde_json::json!(audit_tier.as_str());
        }
        let event = self.build_event(id, "task_proposed", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(id)?;
        }
        Ok(event)
    }

    pub fn propose_task(&self, id: &str, input: CreateTaskInput<'_>) -> Result<Event> {
        self.propose_task_with_kind(id, input, TaskKind::Implementation, AuditTier::Full)
    }

    pub fn revise_task(&self, task_id: &str, input: ReviseTaskInput<'_>) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::Planning && state.phase != Phase::Proposed {
            return Err(anyhow!(
                "Can only revise in Planning or Proposed phase, current: {:?}",
                state.phase
            ));
        }

        let objective = input
            .objective
            .map(String::from)
            .or_else(|| state.objective.clone())
            .unwrap_or_default();
        let read_scope = match input.read_scope {
            Some(paths) => self.normalize_boundary_paths("read_scope", paths)?,
            None => state.read_scope.iter().cloned().collect(),
        };
        let write_allow = match input.write_allow {
            Some(paths) => self.normalize_boundary_paths("write_allow", paths)?,
            None => state.write_allow.iter().cloned().collect(),
        };
        let write_deny = match input.write_deny {
            Some(paths) => self.normalize_boundary_paths("write_deny", paths)?,
            None => state.write_deny.iter().cloned().collect(),
        };
        let risk_triggers = input
            .risk_triggers
            .map(|triggers| triggers.to_vec())
            .unwrap_or_else(|| state.risk_triggers.iter().cloned().collect());
        let gates = match input.gates {
            Some(gates) => validate_gate_templates(gates, &self.project_root)?,
            None => state.gates.iter().cloned().collect(),
        };
        let depends_on: Vec<String> = match input.depends_on {
            Some(deps) => deps.to_vec(),
            None => state.depends_on.iter().cloned().collect(),
        };
        validate_task_definition(&objective, &read_scope, &write_allow, &gates)?;

        let mut payload = serde_json::json!({
            "objective": objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": risk_triggers,
            "gates": gates,
        });
        if !depends_on.is_empty() {
            payload["depends_on"] = serde_json::json!(depends_on);
        }
        let event = self.build_event(task_id, "task_revised", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn mark_ready(&self, task_id: &str) -> Result<Event> {
        // gh6 / issue #6 proposal-mode: only a human actor can ready (approve)
        // a task. The model proposes via `ctl task create`; a human approves via
        // `ctl task approve` (or `ctl task ready`). ctl's actor is a label, not
        // a crypto identity (honest disclosure): in a real OMP session the hook
        // sets CTL_ACTOR to the model label (real enforcement); in a raw shell
        // the model could unset CTL_ACTOR to pass (honor-system, same posture as
        // the reviewer-≠-implementer interlock). `ctl task quick` fuses
        // create+ready+start, so under proposal-mode it is effectively
        // human-only too (the embedded ready hits this check).
        if self.actor != "human" {
            return Err(anyhow!(
                "Task '{}' can only be approved (readied) by a human actor; current actor is \
                 '{}'. The model proposes (ctl task create); a human approves (ctl task approve).",
                task_id,
                self.actor,
            ));
        }
        let event = self.build_event(task_id, "task_marked_ready", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// gh6 full proposal-mode: human-only approval. Transitions a Proposed task
    /// to Ready via a `task_approved` event. The actor must be "human" (enforced
    /// here AND at the reducer — a non-human approval event fails replay, so
    /// the invariant holds even against a forged event).
    pub fn approve_task(&self, task_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::Proposed {
            return Err(anyhow!(
                "Can only approve a Proposed task, current phase: {:?}",
                state.phase
            ));
        }
        if self.actor != "human" {
            return Err(anyhow!(
                "Task '{}' can only be approved by a human actor; current actor is '{}'. \
                 The model proposes (ctl task propose); a human approves (ctl task approve).",
                task_id,
                self.actor,
            ));
        }
        let event = self.build_event(task_id, "task_approved", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// M6 dependency-gated start: the declared `depends_on` task IDs of
    /// `task_id` that are NOT yet satisfied. A dependency counts as satisfied
    /// only when the task exists and has reached `Completed` (archiving keeps
    /// the phase, so an archived-completed prerequisite still satisfies).
    /// Every other case — a missing/unknown task, or a phase of
    /// planning/ready/in_progress/review/cancelled — is unmet. This fails
    /// closed: a dependent never starts ahead of, or alongside, an unfinished
    /// prerequisite. The result is sorted for stable diagnostics.
    ///
    /// This is a cross-task read, so it lives in the app layer (the reducer
    /// stays pure and can only see one task's own log), mirroring the M-a
    /// multiple-active-writer interlock.
    pub fn unmet_dependencies(&self, task_id: &str) -> Result<Vec<String>> {
        let state = self.replay_task(task_id)?;
        let mut unmet: Vec<String> = state
            .depends_on
            .iter()
            .filter(|dep| {
                !matches!(
                    self.replay_task(dep),
                    Ok(dep_state) if dep_state.phase == Phase::Completed
                )
            })
            .cloned()
            .collect();
        unmet.sort();
        Ok(unmet)
    }

    pub fn start_task(&self, task_id: &str) -> Result<Event> {
        // M6 dependency-gated start: refuse while any declared dependency is
        // unfinished, so a dependency chain runs strictly serially.
        let blocked_by = self.unmet_dependencies(task_id)?;
        if !blocked_by.is_empty() {
            return Err(anyhow!(
                "Cannot start '{}': blocked by unfinished dependencies [{}]. \
                 Complete (and archive) each prerequisite first, or drop the edge \
                 with `ctl task revise --id {} --depends-on <remaining ids>`.",
                task_id,
                blocked_by.join(", "),
                task_id
            ));
        }
        let event = self.build_event(task_id, "task_started", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_cancelled", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    // ── Post-M0 lifecycle helpers (not exposed by the M0 CLI) ──

    pub fn submit_task(&self, task_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.is_held {
            return Err(anyhow!("Cannot submit: task is held"));
        }
        if state.phase != Phase::InProgress {
            return Err(anyhow!(
                "Can only submit for review from InProgress, current: {:?}",
                state.phase
            ));
        }
        // Check for any boundary violations recorded since start
        let events = self.store.read_for_task(task_id)?;
        let has_violations = events
            .iter()
            .any(|e| e.event_type == "boundary_violation_recorded");
        if has_violations {
            return Err(anyhow!("Cannot submit: task has boundary violations"));
        }
        let event =
            self.build_event(task_id, "task_submitted_for_review", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn reopen_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_reopened", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// A release task took a granted apply-approval on a release manifest
    /// (Cargo.toml / Cargo.lock) — i.e., a version bump. [`finish_task`]
    /// requires the FULL CI verify set (fmt+clippy+test+arch) for these, not
    /// just declared gates, closing the "declared < CI verify" gap.
    pub(crate) fn is_release_task(state: &TaskState) -> bool {
        state.pending_approvals.values().any(|a| {
            a.is_granted()
                && a.scope.get("action").and_then(|v| v.as_str()) == Some("apply")
                && matches!(
                    a.scope.get("path").and_then(|v| v.as_str()),
                    Some("Cargo.toml") | Some("Cargo.lock")
                )
        })
    }

    /// Completion interlock: phase must be Review, not held, all gates passing,
    /// and no rejected evidence.
    pub fn finish_task(&self, task_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;

        // Phase check
        if state.phase != Phase::Review {
            return Err(anyhow!(
                "Can only finish from Review, current: {:?}",
                state.phase
            ));
        }

        // Hold check
        if state.is_held {
            return Err(anyhow!("Cannot finish: task is held"));
        }

        // Artifact binding (tree_hash): the code being completed must be the code
        // the latest required gate and the accepted completion audit validated.
        // Bound to the committed tree (HEAD^{tree}); skipped outside a git repo,
        // mirroring the M-g commit interlock. `None` here disables the binding
        // checks below so non-git flows behave exactly as before.
        let current_tree = crate::infrastructure::workspace::head_tree_hash(&self.project_root)?;
        // Policy binding: the rules in force now must match the rules the evidence
        // was produced under. Always computable (independent of git).
        let current_policy = self.current_policy_hash(&state);

        // Gate interlock: all required gates must have a latest PASSING result,
        // bound to the current committed tree (git repo only) AND the current
        // policy (always). Unbound (legacy `None`) counts as stale.
        let mut failing_gates = Vec::new();
        let mut tree_stale = Vec::new();
        let mut policy_stale = Vec::new();
        for gate_id in &state.gates {
            match state.gate_results.get(gate_id) {
                Some(result) if result.passed => {
                    if let Some(ref current) = current_tree {
                        if result.tree_hash.as_deref() != Some(current.as_str()) {
                            tree_stale.push(gate_id.as_str());
                        }
                    }
                    if result.policy_hash.as_deref() != Some(current_policy.as_str()) {
                        policy_stale.push(gate_id.as_str());
                    }
                }
                _ => {
                    failing_gates.push(gate_id.as_str());
                }
            }
        }
        // ── Release-task verify floor (mirrors CI verify): a task that took a
        // granted apply-approval on a release manifest (Cargo.toml / Cargo.lock)
        // is a version-bump release. Its finish must pass the FULL CI verify set
        // — fmt + clippy + test + architecture — not just the gates it happened
        // to declare via --gates. Closes the "declared gates < CI verify" gap
        // that once let a rustfmt-drift pass finish and fail only on push.
        let is_release = Self::is_release_task(&state);
        if is_release {
            for baseline in [
                "cargo_fmt_check",
                "cargo_clippy",
                "cargo_test",
                "architecture_check",
            ] {
                if state.gates.contains(baseline) {
                    continue; // already enforced by the declared-gate loop above
                }
                match state.gate_results.get(baseline) {
                    Some(result) if result.passed => {
                        if let Some(current) = &current_tree {
                            if result.tree_hash.as_deref() != Some(current.as_str()) {
                                tree_stale.push(baseline);
                            }
                        }
                        if result.policy_hash.as_deref() != Some(current_policy.as_str()) {
                            policy_stale.push(baseline);
                        }
                    }
                    _ => failing_gates.push(baseline),
                }
            }
        }
        if !failing_gates.is_empty() {
            return Err(anyhow!(
                "Completion interlock: gates not passing: {:?}",
                failing_gates
            ));
        }
        if !tree_stale.is_empty() || !policy_stale.is_empty() {
            let tcur = current_tree.as_deref().unwrap_or("n/a (non-git)");
            let scope: Vec<String> = state.write_allow.iter().cloned().collect();
            let hint = match current_tree.as_deref() {
                Some(cur) => {
                    // A stale gate with no recorded tree (legacy unbound) is
                    // incomputable — finish counts it stale, but it cannot be
                    // diffed, so suppress the no-rework hint for the whole set.
                    let has_unbound = tree_stale
                        .iter()
                        .filter_map(|g| state.gate_results.get(*g))
                        .any(|r| r.tree_hash.is_none());
                    let evidence_trees: Vec<&str> = tree_stale
                        .iter()
                        .filter_map(|g| state.gate_results.get(*g))
                        .filter_map(|r| r.tree_hash.as_deref())
                        .filter(|et| *et != cur)
                        .collect();
                    if evidence_trees.is_empty() {
                        String::new()
                    } else {
                        let mut all_changes: Vec<String> = Vec::new();
                        let mut all_computed = !has_unbound;
                        for et in &evidence_trees {
                            match crate::infrastructure::workspace::scoped_tree_diff(
                                &self.project_root,
                                et,
                                cur,
                                &scope,
                            ) {
                                Some(c) => all_changes.extend(c),
                                None => all_computed = false,
                            }
                        }
                        all_changes.sort();
                        all_changes.dedup();
                        if !all_computed {
                            String::new()
                        } else if all_changes.is_empty() {
                            "\n  write-scope unchanged across all stale gate evidence trees: rerun gates + re-audit under HEAD (no rework)".to_string()
                        } else {
                            format!("\n  write-scope changed since stale evidence: {all_changes:?} — rework, then rerun gates + re-audit")
                        }
                    }
                }
                None => String::new(),
            };
            return Err(anyhow!(
                "Completion interlock: completion evidence is stale.\n\
                 current tree:        {tcur}\n\
                 current policy:      {current_policy}\n\
                 tree-stale gate(s):  {tree_stale:?}\n\
                 policy-stale gate(s): {policy_stale:?}\n\
                 canonical order: submit → commit → gate run → review accept → finish{hint}"
            ));
        }

        // Check for rejected evidence that hasn't been superseded by accepted evidence.
        // A rejection for a file is resolved if a later evidence_accepted covers it.
        let events = self.store.read_for_task(task_id)?;
        let mut rejected_files: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for e in &events {
            match e.event_type.as_str() {
                "evidence_rejected" => {
                    if let Some(f) = e.payload.get("touched_file").and_then(|v| v.as_str()) {
                        if !f.is_empty() {
                            rejected_files.insert(f.to_string());
                        }
                    }
                }
                "evidence_accepted" => {
                    if let Some(files) = e.payload.get("touched_files").and_then(|v| v.as_array()) {
                        for f in files {
                            if let Some(s) = f.as_str() {
                                rejected_files.remove(s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !rejected_files.is_empty() {
            return Err(anyhow!(
                "Completion interlock: rejected evidence unresolved for: {:?}",
                rejected_files
            ));
        }

        // TDD red→green interlock (ctl-tdd-loop-v1), opt-in via the
        // `tdd-red-green` risk trigger. A test that only ever passed proves
        // nothing; require the test gate to have FAILED (red) at an earlier
        // point than it PASSED (green) in this task's own gate history —
        // evidence the test can actually fail. Derived from the existing
        // `gate_checked` event stream, so no new event type or schema.
        if state.risk_triggers.contains(TDD_RED_GREEN_TRIGGER) {
            if !state.gates.contains(TDD_TEST_GATE) {
                return Err(anyhow!(
                    "Completion interlock (tdd-red-green): task opted into TDD but has no \
                     '{TDD_TEST_GATE}' gate to prove red→green. Add it to the task's gates."
                ));
            }
            if !gate_went_red_before_green(&events, TDD_TEST_GATE) {
                return Err(anyhow!(
                    "Completion interlock (tdd-red-green): no red→green evidence for \
                     '{TDD_TEST_GATE}'. TDD requires the test to FAIL before it PASSES — run \
                     the gate while the test is red (before implementing), then again once \
                     green. Found no failing '{TDD_TEST_GATE}' result preceding a passing one."
                ));
            }
        }

        // Commit interlock (M-g): a task cannot complete with uncommitted work
        // in its write scope. The commit window opens at Review, so by the time
        // finish runs the agent must already have committed. Scoped to the
        // task's write_allow and git-tracked paths (`.ctl/` is gitignored and
        // thus excluded). Skipped for read-only tasks (empty write_allow) and
        // outside a git repository, where there is nothing to commit / no way
        // to verify.
        let scope: Vec<String> = state.write_allow.iter().cloned().collect();
        if !scope.is_empty() {
            if let Some(dirty) =
                crate::infrastructure::workspace::dirty_paths_in_scope(&self.project_root, &scope)?
            {
                if !dirty.is_empty() {
                    return Err(anyhow!(
                        "Completion interlock: uncommitted changes in write scope: {:?}. \
                         Commit (and optionally push) within Review before finishing.",
                        dirty
                    ));
                }
            }
        }

        // Hard review gate (M-f): completion requires a FRESH passing completion
        // audit. A verdict counts only if recorded after the last submit — rework
        // re-submits and invalidates a prior round's audit. The latest such
        // verdict must be a PASS; a FAIL (or no audit at all) blocks finish. This
        // upgrades review from convention (soft-layer ctl-review subagents) to a
        // gateway interlock. `events` is already loaded above.
        let last_submit_seq = events
            .iter()
            .filter(|e| e.event_type == "task_submitted_for_review")
            .map(|e| e.seq)
            .max();
        let latest_audit = events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type.as_str(),
                    "evidence_accepted" | "evidence_rejected"
                )
            })
            .filter(|e| {
                e.payload.get("source").and_then(|v| v.as_str())
                    == Some(crate::application::COMPLETION_AUDIT_SOURCE)
            })
            .filter(|e| last_submit_seq.is_none_or(|s| e.seq > s))
            .max_by_key(|e| e.seq);
        match latest_audit {
            Some(e) if e.event_type == "evidence_accepted" => {
                // Artifact + policy binding: the passing audit must be bound to the
                // current committed tree (git repo only) AND the current policy.
                let mut stale = Vec::new();
                if let Some(ref current) = current_tree {
                    if e.payload.get("tree_hash").and_then(|v| v.as_str()) != Some(current.as_str())
                    {
                        stale.push("tree");
                    }
                }
                if e.payload.get("policy_hash").and_then(|v| v.as_str())
                    != Some(current_policy.as_str())
                {
                    stale.push("policy");
                }
                if !stale.is_empty() {
                    let scope: Vec<String> = state.write_allow.iter().cloned().collect();
                    let audit_tree = e.payload.get("tree_hash").and_then(|v| v.as_str());
                    let hint = match (audit_tree, current_tree.as_deref()) {
                        (Some(et), Some(cur)) if et != cur => {
                            match crate::infrastructure::workspace::scoped_tree_diff(&self.project_root, et, cur, &scope) {
                                Some(c) if c.is_empty() => format!("\n  write-scope unchanged since audit tree {et}: re-audit under HEAD (no rework)"),
                                Some(c) => format!("\n  write-scope changed since audit tree {et}: {c:?} — rework, then re-audit"),
                                None => String::new(),
                            }
                        }
                        _ => String::new(),
                    };
                    return Err(anyhow!(
                        "Completion interlock: completion audit is stale ({}); \
                         re-audit under the current code and policy before finishing.{hint}\n\
                         canonical order: submit → commit → gate run → review accept → finish",
                        stale.join(" + ")
                    ));
                }
            }
            Some(_) => {
                return Err(anyhow!(
                    "Completion interlock: the latest completion audit is a FAIL. \
                     Rework, then record a passing audit (ctl review accept --id {}) before finishing.",
                    task_id
                ));
            }
            None => {
                return Err(anyhow!(
                    "Completion interlock: no passing completion audit since submit. \
                     A reviewer must record one: ctl review accept --id {}",
                    task_id
                ));
            }
        }

        // Research/Spike V1: a research task is not exempt from execution
        // integrity (all checks above still applied). It additionally must show a
        // non-degenerate footprint — at least one tracked artifact and at least
        // one uncertainty outcome — so a spike never completes looking identical
        // to an implementation task that produced nothing. This NEVER requires the
        // open-uncertainty count to fall: opening unknowns is a legitimate result.
        if state.task_kind == TaskKind::Research {
            if state.research_artifacts.is_empty() {
                return Err(anyhow!(
                    "Completion interlock: research task requires at least one recorded \
                     research artifact (ctl research record --id {})",
                    task_id
                ));
            }
            // Freshness floor: a finish must point at at least one artifact that
            // still matches what was recorded. An artifact deleted or edited away
            // after recording (STALE/ABSENT) must not satisfy completion — otherwise
            // the disclosed footprint no longer corresponds to anything on disk.
            let has_current = state.research_artifacts.iter().any(|a| {
                self.artifact_freshness(&a.artifact_ref)
                    == crate::domain::task::EvidenceFreshness::Current
            });
            if !has_current {
                return Err(anyhow!(
                    "Completion interlock: research task requires at least one CURRENT research \
                     artifact (every recorded artifact is STALE or ABSENT — re-record against \
                     the current files before finishing)"
                ));
            }
            // "at least one uncertainty outcome" — a recorded uncertainty (a
            // disposition is impossible without a prior record), so this is the
            // real floor.
            if state.uncertainties.is_empty() {
                return Err(anyhow!(
                    "Completion interlock: research task requires at least one recorded \
                     uncertainty outcome (ctl uncertainty record --id {})",
                    task_id
                ));
            }
        }

        let event = self.build_event(task_id, "task_completed", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    pub fn archive_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_archived", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// The actors who performed implementation work on a task (M6). The reviewer
    /// who records a passing completion audit must not be one of them. Implementer
    /// signals: who `task_started` the task, and who produced non-audit work
    /// evidence (`evidence_accepted` with a `source` other than the completion
    /// audit, i.e. adapter/manual output).
    pub(crate) fn implementer_actors(events: &[Event]) -> HashSet<String> {
        let mut actors = HashSet::new();
        for e in events {
            match e.event_type.as_str() {
                "task_started" => {
                    actors.insert(e.actor.clone());
                }
                "evidence_accepted" => {
                    let source = e.payload.get("source").and_then(|v| v.as_str());
                    if source != Some(COMPLETION_AUDIT_SOURCE) {
                        actors.insert(e.actor.clone());
                    }
                }
                _ => {}
            }
        }
        actors
    }

    /// M-f: record a reviewer's completion-audit verdict on a submitted task.
    ///
    /// A PASS is the hard prerequisite the finish interlock requires; a FAIL
    /// blocks completion until the work is reworked and re-audited. Modeled on
    /// the existing evidence events with a distinguished `source`
    /// ([`COMPLETION_AUDIT_SOURCE`]) so it needs no canonical-schema change; the
    /// reviewer identity is the event `actor` (M6 — set via `CTL_ACTOR`).
    /// Recorded only in Review — the post-submit audit window.
    ///
    /// M6 reviewer-lease binding: a PASS may **not** be recorded by an
    /// implementer of the task (no self-approval). A FAIL is always allowed —
    /// an implementer self-flagging a problem is healthy; only self-certifying
    /// completion is the threat.
    pub fn record_completion_audit(
        &self,
        task_id: &str,
        pass: bool,
        note: Option<&str>,
    ) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::Review {
            return Err(anyhow!(
                "Completion audit can only be recorded in Review (task is {:?}); submit the task first",
                state.phase
            ));
        }
        let events = self.store.read_for_task(task_id)?;
        if pass && Self::implementer_actors(&events).contains(&self.actor) {
            return Err(anyhow!(
                "Reviewer-lease binding: actor '{}' implemented this task and cannot record its own \
                 passing completion audit. A different reviewer must accept it (set CTL_ACTOR to the \
                 reviewer's identity).",
                self.actor
            ));
        }
        let evidence_id = generate_uuid();
        let event = if pass {
            let touched: Vec<String> = state.write_allow.iter().cloned().collect();
            let mut payload = serde_json::json!({
                "evidence_id": evidence_id,
                "source": COMPLETION_AUDIT_SOURCE,
                "touched_files": touched,
                "result_file": note.unwrap_or(""),
                "accepted_at": now_iso8601(),
            });
            // Artifact binding: stamp the committed tree this audit validated.
            if let Some(tree) =
                crate::infrastructure::workspace::head_tree_hash(&self.project_root)?
            {
                payload["tree_hash"] = serde_json::json!(tree);
            }
            // Policy binding: stamp the policy in force when this audit was accepted.
            payload["policy_hash"] = serde_json::json!(self.current_policy_hash(&state));
            self.build_event(task_id, "evidence_accepted", payload)?
        } else {
            let payload = serde_json::json!({
                "evidence_id": evidence_id,
                "source": COMPLETION_AUDIT_SOURCE,
                "rejection_reason": note.unwrap_or("completion audit failed"),
                // Empty: the generic rejected-evidence interlock keys on a
                // per-file rejection; the completion-audit verdict is task-level
                // and enforced by the dedicated M-f interlock instead.
                "touched_file": "",
            });
            self.build_event(task_id, "evidence_rejected", payload)?
        };
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Canonical hash of the task's CURRENT policy (scope + risk triggers +
    /// required-gate *definitions*). Resolves each required gate id to its
    /// template's actual command + args, so a template change (not just a rename)
    /// invalidates prior evidence. Independent of git — always computable.
    pub(crate) fn current_policy_hash(&self, state: &crate::domain::task::TaskState) -> String {
        use crate::domain::policy::{compute_policy_hash, CanonicalGateDefinition};
        let read_scope: Vec<String> = state.read_scope.iter().cloned().collect();
        let write_allow: Vec<String> = state.write_allow.iter().cloned().collect();
        let write_deny: Vec<String> = state.write_deny.iter().cloned().collect();
        let risk_triggers: Vec<String> = state.risk_triggers.iter().cloned().collect();
        let gates: Vec<CanonicalGateDefinition> = state
            .gates
            .iter()
            .map(
                |g| match crate::infrastructure::gates::resolve_gate(g, &self.project_root) {
                    Some(t) => CanonicalGateDefinition {
                        gate_id: g.clone(),
                        command: t.command().to_string(),
                        args: t.args(),
                    },
                    None => CanonicalGateDefinition {
                        gate_id: g.clone(),
                        command: String::new(),
                        args: Vec::new(),
                    },
                },
            )
            .collect();
        compute_policy_hash(
            &read_scope,
            &write_allow,
            &write_deny,
            &risk_triggers,
            &gates,
        )
    }

    pub fn record_gate(
        &self,
        task_id: &str,
        gate_id: &str,
        passed: bool,
        evidence: &str,
    ) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        let mut payload = serde_json::json!({
            "gate_id": gate_id,
            "passed": passed,
            "evidence": evidence,
            "checked_at": now_iso8601(),
        });
        // Artifact binding: stamp the committed tree this gate result was validated
        // against. Omitted (not null) outside a git repo so the schema stays valid;
        // unbound results cannot satisfy the finish-time interlock in a git repo.
        if let Some(tree) = crate::infrastructure::workspace::head_tree_hash(&self.project_root)? {
            payload["tree_hash"] = serde_json::json!(tree);
        }
        // Policy binding: stamp the policy in force when this gate ran (always
        // computable; independent of git).
        payload["policy_hash"] = serde_json::json!(self.current_policy_hash(&state));
        let event = self.build_event(task_id, "gate_checked", payload)?;
        self.validate_and_append(&event)?;
        if !self.dry_run {
            self.rebuild_task_view(task_id)?;
        }
        Ok(event)
    }

    /// Execute a gate through the EXEC-002 runner and record the result
    /// as a canonical `gate_checked` event.
    pub fn run_gate_checked(&self, task_id: &str, gate_id: &str) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if !state.gates.contains(gate_id) {
            return Err(anyhow!(
                "Gate '{}' is not declared in task gates: {:?}",
                gate_id,
                state.gates
            ));
        }

        let result = crate::infrastructure::gates::run_gate(gate_id, &self.project_root)?;
        let evidence = format_gate_evidence(&result);

        self.record_gate(task_id, gate_id, result.passed, &evidence)
    }

    // ── BS-provenance V1: record-only brainstorm artifact provenance ──
    //
    // These emit canonical, task-scoped events binding brainstorm artifacts (by
    // path + SHA-256) to a task. They NEVER gate task creation or completion and
    // make no claim about thinking quality or review independence. Trust is pinned
    // at L0 content and critic independence at `unattested` by the reducer.

    /// Hash a brainstorm artifact, resolving its path against the project root.
    /// The file must exist: a reference is provenance only for content actually
    /// present at record time — a bare path on disk is never auto-provenance.
    pub(crate) fn hash_artifact(&self, path: &str) -> Result<String> {
        let resolved = self.project_root.join(path);
        if !resolved.is_file() {
            return Err(anyhow!("brainstorm artifact not found: {}", path));
        }
        hash_file(&resolved)
    }
}
