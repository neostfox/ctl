use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::event::Event;
use crate::domain::task::{apply, Phase, TaskState};
use crate::infrastructure::schema_validator::SchemaValidator;
use crate::infrastructure::store::FileEventStore;

pub struct ControlApp {
    project_root: PathBuf,
    store: FileEventStore,
    validator: Option<SchemaValidator>,
}

pub struct CreateTaskInput<'a> {
    pub objective: &'a str,
    pub read_scope: &'a [String],
    pub write_allow: &'a [String],
    pub write_deny: &'a [String],
    pub risk_triggers: &'a [String],
    pub gates: &'a [String],
}

pub struct ReviseTaskInput<'a> {
    pub objective: Option<&'a str>,
    pub read_scope: Option<&'a [String]>,
    pub write_allow: Option<&'a [String]>,
    pub write_deny: Option<&'a [String]>,
    pub risk_triggers: Option<&'a [String]>,
    pub gates: Option<&'a [String]>,
}

impl ControlApp {
    pub fn init(project_root: &Path) -> Result<Self> {
        let store = FileEventStore::init(project_root)?;
        let project_root = std::fs::canonicalize(project_root)?;
        let validator = new_validator_if_available();
        Ok(Self {
            project_root,
            store,
            validator,
        })
    }

    pub fn open(project_root: &Path) -> Result<Self> {
        let store = FileEventStore::open(project_root)?;
        let validator = new_validator_if_available();
        let project_root = std::fs::canonicalize(project_root)?;
        Ok(Self {
            project_root,
            store,
            validator,
        })
    }

    // ── Commands ──

    pub fn create_task(&self, id: &str, input: CreateTaskInput<'_>) -> Result<Event> {
        let existing = self.store.read_for_task(id)?;
        if !existing.is_empty() {
            return Err(anyhow!("Task '{}' already exists", id));
        }

        let read_scope = self.normalize_boundary_paths("read_scope", input.read_scope)?;
        let write_allow = self.normalize_boundary_paths("write_allow", input.write_allow)?;
        let write_deny = self.normalize_boundary_paths("write_deny", input.write_deny)?;
        let gates = validate_gate_templates(input.gates)?;
        validate_task_definition(input.objective, &read_scope, &write_allow, &gates)?;

        let payload = serde_json::json!({
            "objective": input.objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": input.risk_triggers,
            "gates": gates,
        });
        let event = self.build_event(id, "task_created", payload)?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(id)?;
        Ok(event)
    }

    pub fn revise_task(&self, task_id: &str, input: ReviseTaskInput<'_>) -> Result<Event> {
        let state = self.replay_task(task_id)?;
        if state.phase != Phase::Planning {
            return Err(anyhow!(
                "Can only revise in Planning phase, current: {:?}",
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
            Some(gates) => validate_gate_templates(gates)?,
            None => state.gates.iter().cloned().collect(),
        };
        validate_task_definition(&objective, &read_scope, &write_allow, &gates)?;

        let payload = serde_json::json!({
            "objective": objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": write_deny,
            "risk_triggers": risk_triggers,
            "gates": gates,
        });
        let event = self.build_event(task_id, "task_revised", payload)?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    pub fn mark_ready(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_marked_ready", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    pub fn start_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_started", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_cancelled", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    // ── Post-M0 lifecycle helpers (not exposed by the M0 CLI) ──

    pub fn submit_task(&self, task_id: &str) -> Result<Event> {
        let event =
            self.build_event(task_id, "task_submitted_for_review", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    pub fn reopen_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_reopened", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    /// # UNVERIFIED — completion interlock
    /// This method does NOT yet verify:
    /// - agent output presence and hash integrity
    /// - scope diff against baseline context
    /// - pending approval requests
    /// - evidence hash chain
    ///   This helper is intentionally not exposed by the M0 CLI.
    pub fn finish_task(&self, task_id: &str) -> Result<Event> {
        // Completion interlock: check gates, hold, scope
        let state = self.replay_task(task_id)?;
        if state.is_held {
            return Err(anyhow!("Cannot finish: task is held"));
        }
        // All required gates must have latest passing result
        for gate_id in &state.gates {
            match state.gate_results.get(gate_id) {
                Some(result) if result.passed => {}
                _ => {
                    return Err(anyhow!(
                        "Completion interlock: gate '{}' has no passing result",
                        gate_id
                    ));
                }
            }
        }
        let event = self.build_event(task_id, "task_completed", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    pub fn archive_task(&self, task_id: &str) -> Result<Event> {
        let event = self.build_event(task_id, "task_archived", serde_json::json!({}))?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

    pub fn record_gate(
        &self,
        task_id: &str,
        gate_id: &str,
        passed: bool,
        evidence: &str,
    ) -> Result<Event> {
        let payload = serde_json::json!({
            "gate_id": gate_id,
            "passed": passed,
            "evidence": evidence,
            "checked_at": now_iso8601(),
        });
        let event = self.build_event(task_id, "gate_checked", payload)?;
        self.validate_and_append(&event)?;
        self.rebuild_task_view(task_id)?;
        Ok(event)
    }

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
                    "path": rel.to_string_lossy(),
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
        let temp_path = task_dir.join("context.json.tmp");
        std::fs::write(&temp_path, serde_json::to_string_pretty(&context)?)?;
        std::fs::rename(&temp_path, &context_path)?;

        Ok(context)
    }

    /// Check workspace modifications against task scope.
    /// Returns list of violations (files modified outside write_allow scope).
    #[allow(dead_code)]
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

    /// Rebuild all task views from events (reconcile).
    pub fn reconcile(&self) -> Result<Vec<String>> {
        let task_ids = self.store.task_ids()?;
        let mut rebuilt = Vec::new();
        for task_id in &task_ids {
            let state = self.replay_task(task_id)?;
            self.store.write_task_view(task_id, &state)?;
            rebuilt.push(task_id.clone());
        }
        Ok(rebuilt)
    }

    // ── Queries ──

    pub fn get_status(&self, task_id: &str) -> Result<TaskState> {
        self.replay_task(task_id)
    }

    pub fn replay(&self, task_id: &str) -> Result<TaskState> {
        let state = self.replay_task(task_id)?;
        self.store.write_task_view(task_id, &state)?;
        Ok(state)
    }

    #[allow(dead_code)]
    pub fn list_tasks(&self) -> Result<Vec<String>> {
        self.store.task_ids()
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
        let mut results = Vec::new();

        // Check events.jsonl readable
        match self.store.read_all() {
            Ok(events) => {
                results.push(format!("events.jsonl: OK ({} events)", events.len()));

                // Try to replay each task
                let task_ids = self.store.task_ids()?;
                for tid in &task_ids {
                    match self.replay_task(tid) {
                        Ok(state) => {
                            results.push(format!(
                                "Task '{}': {:?} (seq {})",
                                tid, state.phase, state.last_seq
                            ));
                        }
                        Err(e) => {
                            results.push(format!("Task '{}': REPLAY ERROR: {}", tid, e));
                        }
                    }
                }
            }
            Err(e) => {
                results.push(format!("events.jsonl: ERROR: {}", e));
            }
        }

        Ok(results)
    }

    // ── Internal helpers ──

    fn replay_task(&self, task_id: &str) -> Result<TaskState> {
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

    fn build_event(
        &self,
        task_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<Event> {
        let seq = self.store.next_seq_for_task(task_id)?;
        Ok(Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: generate_uuid(),
            command_id: generate_uuid(),
            task_id: task_id.to_string(),
            seq,
            occurred_at: now_iso8601(),
            actor: "human".to_string(),
            event_type: event_type.to_string(),
            payload,
        })
    }

    fn normalize_boundary_paths(&self, field: &str, paths: &[String]) -> Result<Vec<String>> {
        let normalizer = crate::infrastructure::boundary::normalizer::PathNormalizer::new(
            self.project_root.clone(),
        );
        let mut normalized = Vec::with_capacity(paths.len());
        for path in paths {
            let path = normalizer
                .normalize(path)
                .map_err(|e| anyhow!("Invalid {} path '{}': {}", field, path, e))?;
            normalized.push(path_to_payload_string(&path));
        }
        Ok(normalized)
    }

    fn validate_and_append(&self, event: &Event) -> Result<()> {
        if matches!(event.event_type.as_str(), "task_created" | "task_revised")
            && event.payload.get("scope").is_some()
        {
            return Err(anyhow!(
                "Legacy task boundary field 'scope' is not accepted in M1 events"
            ));
        }

        // 1. Schema validation (when schemas/ available)
        if let Some(ref validator) = self.validator {
            let json_val = serde_json::to_value(event)?;
            validator
                .validate_instance(&json_val, &event.schema)
                .map_err(|e| anyhow!("Schema validation failed: {}", e))?;
        }

        // 2. Dry-run reducer against the existing canonical stream.
        let mut state = TaskState::new(&event.task_id);
        for prior in self.store.read_for_task(&event.task_id)? {
            apply(&mut state, &prior)
                .map_err(|e| anyhow!("Reducer error at seq {}: {}", prior.seq, e))?;
        }
        apply(&mut state, event).map_err(|e| anyhow!("Reducer rejected: {}", e))?;

        // 3. Persist
        self.store.append(event)?;
        Ok(())
    }

    fn rebuild_task_view(&self, task_id: &str) -> Result<()> {
        let state = self.replay_task(task_id)?;
        self.store.write_task_view(task_id, &state)?;
        Ok(())
    }
}

fn validate_task_definition(
    objective: &str,
    read_scope: &[String],
    write_allow: &[String],
    gates: &[String],
) -> Result<()> {
    if objective.trim().is_empty() {
        return Err(anyhow!("Task objective must not be empty"));
    }
    if read_scope.is_empty() {
        return Err(anyhow!("Task read_scope must not be empty"));
    }
    if write_allow.is_empty() {
        return Err(anyhow!("Task write_allow must not be empty"));
    }
    if gates.is_empty() {
        return Err(anyhow!("Task gates must not be empty"));
    }
    Ok(())
}

fn validate_gate_templates(gates: &[String]) -> Result<Vec<String>> {
    let mut validated = Vec::with_capacity(gates.len());
    for gate_id in gates {
        if crate::infrastructure::gates::find_template(gate_id).is_none() {
            return Err(anyhow!(
                "Unknown gate '{}' — only known gate templates are allowed",
                gate_id
            ));
        }
        validated.push(gate_id.clone());
    }
    Ok(validated)
}

fn path_to_payload_string(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        if let std::path::Component::Normal(part) = component {
            parts.push(part.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

// ── File hashing helpers ──

fn collect_file_hashes(
    dir: &std::path::Path,
    root: &std::path::Path,
    results: &mut Vec<serde_json::Value>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden dirs and target.
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            collect_file_hashes(&path, root, results)?;
        } else if path.is_file() {
            let hash = hash_file(&path)?;
            let rel = path.strip_prefix(root).unwrap_or(&path);
            results.push(serde_json::json!({
                "path": rel.to_string_lossy(),
                "hash": hash,
            }));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn collect_files_recursive(
    dir: &std::path::Path,
    root: &std::path::Path,
    results: &mut std::collections::HashSet<String>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            collect_files_recursive(&path, root, results)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            results.insert(rel.to_string_lossy().to_string());
        }
    }
    Ok(())
}

/// # UNVERIFIED — EVIDENCE-001
/// This hash is a 16-byte XOR fold, NOT cryptographically secure.
/// It must NOT be used for evidence integrity verification.
/// Replace with SHA-256 before this is used as evidence integrity data (requires DEP-001/004 review).
fn hash_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    // Simple hash: XOR-fold of byte values — NOT cryptographic
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 8192];
    let mut hash: [u8; 16] = [0; 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for (i, &byte) in buf[..n].iter().enumerate() {
            hash[i % 16] ^= byte;
        }
    }
    Ok(hash.iter().map(|b| format!("{:02x}", b)).collect())
}

fn new_validator_if_available() -> Option<SchemaValidator> {
    if std::path::Path::new("schemas").exists() {
        SchemaValidator::new("schemas/").ok()
    } else {
        None
    }
}

// ── UUID generation (no external crate) ──

static UUID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_uuid() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c = UUID_COUNTER.fetch_add(1, Ordering::Relaxed);

    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:08x}{:04x}",
        (ts.wrapping_add(c)) as u32,
        ((ts >> 16) ^ c) as u16,
        (ts >> 32) as u16 & 0x0FFF,
        (c >> 4) as u16 & 0x0FFF,
        (ts >> 8) as u32,
        (c & 0xFFFF) as u16,
    )
}

// ── ISO 8601 timestamp (no external crate) ──

fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d, h, mi, s) = epoch_to_datetime(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, mi, s)
}

/// Convert Unix epoch seconds to (year, month, day, hour, minute, second).
/// Based on Howard Hinnant's algorithm.
fn epoch_to_datetime(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs / 86400;
    let time_secs = secs % 86400;

    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    (
        y as u64,
        m as u64,
        d as u64,
        time_secs / 3600,
        (time_secs % 3600) / 60,
        time_secs % 60,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("control-app-test-{}", generate_uuid()));
            std::fs::create_dir_all(path.join("src")).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn create_task_writes_canonical_trellis_task_ledger_and_projection() {
        let dir = TempDir::new();
        let app = ControlApp::init(dir.path()).unwrap();
        let read_scope = vec!["src".to_string()];
        let write_allow = vec!["src".to_string()];
        let write_deny = Vec::new();
        let risk_triggers = Vec::new();
        let gates = vec!["cargo_check".to_string()];

        app.create_task(
            "ledger-task",
            CreateTaskInput {
                objective: "Implement ledger",
                read_scope: &read_scope,
                write_allow: &write_allow,
                write_deny: &write_deny,
                risk_triggers: &risk_triggers,
                gates: &gates,
            },
        )
        .unwrap();

        assert!(dir
            .path()
            .join(".trellis/tasks/ledger-task/events.jsonl")
            .exists());
        assert!(dir
            .path()
            .join(".trellis/tasks/ledger-task/task.json")
            .exists());
        assert!(!dir.path().join(".control").join("events.jsonl").exists());
    }

    #[test]
    fn test_generate_uuid_format() {
        let uuid = generate_uuid();
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        assert!(uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn test_generate_uuid_unique() {
        let a = generate_uuid();
        let b = generate_uuid();
        assert_ne!(a, b);
    }

    #[test]
    fn test_now_iso8601_format() {
        let ts = now_iso8601();
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }

    #[test]
    fn test_epoch_to_datetime() {
        // 2026-01-01T00:00:00Z = 1767225600
        let (y, m, d, h, mi, s) = epoch_to_datetime(1767225600);
        assert_eq!(y, 2026);
        assert_eq!(m, 1);
        assert_eq!(d, 1);
        assert_eq!(h, 0);
        assert_eq!(mi, 0);
        assert_eq!(s, 0);
    }
}
