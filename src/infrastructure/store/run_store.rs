//! Run event store for M6 multi-agent concurrency.
//!
//! Mirrors `FileEventStore` but operates under `.trellis/runs/`.
//! Each run has its own directory with `events.jsonl` and `run.json`.

use anyhow::{anyhow, Result};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::domain::event::Event;
use crate::domain::run::AgentRunState;
pub struct RunEventStore {
    pub runs_dir: PathBuf,
}

impl RunEventStore {
    /// Create the `.trellis/runs/` root directory.
    pub fn init(project_root: &Path) -> Result<Self> {
        let runs_dir = project_root.join(".trellis").join("runs");
        fs::create_dir_all(&runs_dir)?;
        Ok(Self { runs_dir })
    }

    /// Path to a specific run directory: `.trellis/runs/<run_id>/`
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir.join(run_id)
    }

    /// Path to a run's event log: `.trellis/runs/<run_id>/events.jsonl`
    pub fn events_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("events.jsonl")
    }

    /// Append a single event to `.trellis/runs/<run_id>/events.jsonl`.
    ///
    /// Uses the event's `task_id` field as the run directory key
    /// (consistent with how FileEventStore::append uses `event.task_id`).
    /// Callers should set `event.task_id` to the run_id when appending
    /// run-scoped events, or use the `run_id`-keyed overload as needed.
    pub fn append(&self, event: &Event) -> Result<()> {
        // For run events, we rely on a run_id derived from context.
        // The Event struct carries `task_id`; callers must ensure this
        // is set to the run_id for run-scoped events. We validate it.
        let run_id = &event.task_id;
        validate_run_id(run_id)?;
        let run_dir = self.run_dir(run_id);
        fs::create_dir_all(&run_dir)?;
        let events_path = run_dir.join("events.jsonl");
        let line = serde_json::to_string(event)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(events_path)?;
        writeln!(file, "{}", line)?;
        file.flush()?;
        Ok(())
    }

    /// Read all events for a specific run.
    /// Skips blank lines. Returns errors for malformed JSON.
    pub fn read_for_run(&self, run_id: &str) -> Result<Vec<Event>> {
        validate_run_id(run_id)?;
        let events_path = self.events_path(run_id);
        if !events_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&events_path)?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: Event = serde_json::from_str(&line).map_err(|e| {
                anyhow!(
                    "{} line {}: parse error: {}",
                    events_path.display(),
                    i + 1,
                    e
                )
            })?;
            if !event.is_valid() {
                return Err(anyhow!(
                    "{} line {}: event failed validation: schema={}, seq={}",
                    events_path.display(),
                    i + 1,
                    event.schema,
                    event.seq
                ));
            }
            events.push(event);
        }
        Ok(events)
    }

    /// Get the next sequence number for a run.
    pub fn next_seq_for_run(&self, run_id: &str) -> Result<i64> {
        let events = self.read_for_run(run_id)?;
        let max_seq = events.iter().map(|e| e.seq).max().unwrap_or(0);
        Ok(max_seq + 1)
    }

    /// Collect all run IDs with canonical event logs.
    pub fn run_ids(&self) -> Result<Vec<String>> {
        if !self.runs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.runs_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let run_id = entry.file_name().to_string_lossy().into_owned();
            validate_run_id(&run_id)?;
            if entry.path().join("events.jsonl").exists() {
                ids.push(run_id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Write a run view projection to `.trellis/runs/<run_id>/run.json`.
    /// Uses atomic write (temp + rename) to avoid corruption.
    pub fn write_run_view(&self, run_id: &str, state: &AgentRunState) -> Result<()> {
        validate_run_id(run_id)?;
        let view = serde_json::json!({
            "schema": "control.run-state.v1",
            "run_id": state.run_id,
            "task_id": state.task_id,
            "adapter": state.adapter,
            "phase": serde_json::to_value(&state.phase)?,
            "worktree_path": state.worktree_path,
            "lease_id": state.lease_id,
            "write_allow": state.write_allow,
            "write_deny": state.write_deny,
            "gates": state.gates,
            "gate_results": state.gate_results,
            "touched_files": state.touched_files,
            "last_seq": state.last_seq,
        });
        let run_dir = self.run_dir(run_id);
        fs::create_dir_all(&run_dir)?;
        let run_path = run_dir.join("run.json");
        let temp_path = run_dir.join("run.json.tmp");
        let json_str = serde_json::to_string_pretty(&view)?;
        fs::write(&temp_path, &json_str)?;
        fs::rename(&temp_path, &run_path)?;
        Ok(())
    }
}

/// Validate a run_id: non-empty, alphanumeric + dashes only.
fn validate_run_id(run_id: &str) -> Result<()> {
    if run_id.trim().is_empty() {
        return Err(anyhow!("Run id must not be empty"));
    }
    if !run_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(anyhow!(
            "Run id '{}' must contain only alphanumeric characters and dashes",
            run_id
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::run::RunPhase;
    use std::collections::{BTreeSet, HashMap, HashSet};

    fn make_tmp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("control-run-test-{}-{}", std::process::id(), id));
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn validate_run_id_accepts_valid() {
        assert!(validate_run_id("abc-123").is_ok());
        assert!(validate_run_id("run1").is_ok());
        assert!(validate_run_id("a").is_ok());
        assert!(validate_run_id("A-B-C-123").is_ok());
    }

    #[test]
    fn validate_run_id_rejects_empty() {
        assert!(validate_run_id("").is_err());
        assert!(validate_run_id("  ").is_err());
    }

    #[test]
    fn validate_run_id_rejects_special_chars() {
        assert!(validate_run_id("run/id").is_err());
        assert!(validate_run_id("run.id").is_err());
        assert!(validate_run_id("run id").is_err());
        assert!(validate_run_id("run_id").is_err());
    }

    #[test]
    fn init_creates_runs_dir() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        assert!(store.runs_dir.exists());
        assert!(store.runs_dir.ends_with("runs"));
    }

    #[test]
    fn run_ids_empty_when_no_runs() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        let ids = store.run_ids().unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn append_and_read_roundtrip() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();

        let event = Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: "evt-1".to_string(),
            command_id: "cmd-1".to_string(),
            task_id: "run-abc".to_string(),
            seq: 1,
            occurred_at: "2026-06-11T00:00:00Z".to_string(),
            actor: "agent".to_string(),
            event_type: "run_started".to_string(),
            payload: serde_json::json!({}),
        };

        store.append(&event).unwrap();
        let events = store.read_for_run("run-abc").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "evt-1");
    }

    #[test]
    fn next_seq_starts_at_one() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        assert_eq!(store.next_seq_for_run("run-new").unwrap(), 1);
    }

    #[test]
    fn next_seq_increments() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();

        for i in 1..=3 {
            let event = Event {
                schema: "control.event-envelope.v1".to_string(),
                event_id: format!("evt-{}", i),
                command_id: format!("cmd-{}", i),
                task_id: "run-seq".to_string(),
                seq: i,
                occurred_at: "2026-06-11T00:00:00Z".to_string(),
                actor: "agent".to_string(),
                event_type: "run_started".to_string(),
                payload: serde_json::json!({}),
            };
            store.append(&event).unwrap();
        }

        assert_eq!(store.next_seq_for_run("run-seq").unwrap(), 4);
    }

    #[test]
    fn run_ids_lists_sorted() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();

        for rid in &["run-b", "run-a", "run-c"] {
            let event = Event {
                schema: "control.event-envelope.v1".to_string(),
                event_id: format!("evt-{}", rid),
                command_id: format!("cmd-{}", rid),
                task_id: rid.to_string(),
                seq: 1,
                occurred_at: "2026-06-11T00:00:00Z".to_string(),
                actor: "agent".to_string(),
                event_type: "run_started".to_string(),
                payload: serde_json::json!({}),
            };
            store.append(&event).unwrap();
        }

        let ids = store.run_ids().unwrap();
        assert_eq!(ids, vec!["run-a", "run-b", "run-c"]);
    }

    #[test]
    fn write_run_view_atomic() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();

        let state = AgentRunState {
            run_id: "run-view".to_string(),
            task_id: "task-1".to_string(),
            adapter: "test".to_string(),
            phase: RunPhase::Running,
            worktree_path: Some("/tmp/wt".to_string()),
            lease_id: Some("lease-1".to_string()),
            write_allow: BTreeSet::new(),
            write_deny: BTreeSet::new(),
            gates: BTreeSet::new(),
            gate_results: HashMap::new(),
            touched_files: BTreeSet::new(),
            history: Vec::new(),
            last_seq: 5,
            processed_commands: HashSet::new(),
        };

        store.write_run_view("run-view", &state).unwrap();

        let path = store.run_dir("run-view").join("run.json");
        assert!(path.exists());
        // No temp file left behind
        assert!(!store.run_dir("run-view").join("run.json.tmp").exists());

        let content = fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["schema"], "control.run-state.v1");
        assert_eq!(v["run_id"], "run-view");
        assert_eq!(v["phase"], "running");
        assert_eq!(v["last_seq"], 5);
    }
}
