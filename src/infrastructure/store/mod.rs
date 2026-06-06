use anyhow::{anyhow, Result};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::domain::event::Event;

pub struct FileEventStore {
    pub tasks_dir: PathBuf,
}

impl FileEventStore {
    /// Create the `.trellis/tasks/` root used by M1 task ledgers.
    pub fn init(project_root: &Path) -> Result<Self> {
        let tasks_dir = project_root.join(".trellis").join("tasks");
        fs::create_dir_all(&tasks_dir)?;
        Ok(Self { tasks_dir })
    }

    /// Open an existing `.trellis/tasks/` task ledger root.
    pub fn open(project_root: &Path) -> Result<Self> {
        let tasks_dir = project_root.join(".trellis").join("tasks");
        if !tasks_dir.exists() {
            return Err(anyhow!(
                ".trellis/tasks/ not found. Run 'control init' first."
            ));
        }
        if !tasks_dir.is_dir() {
            return Err(anyhow!(".trellis/tasks exists but is not a directory."));
        }
        Ok(Self { tasks_dir })
    }

    pub fn task_dir(&self, task_id: &str) -> Result<PathBuf> {
        validate_task_id(task_id)?;
        Ok(self.tasks_dir.join(task_id))
    }

    pub fn events_path(&self, task_id: &str) -> Result<PathBuf> {
        Ok(self.task_dir(task_id)?.join("events.jsonl"))
    }

    /// Append a single event to `.trellis/tasks/<task>/events.jsonl`.
    pub fn append(&self, event: &Event) -> Result<()> {
        let task_dir = self.task_dir(&event.task_id)?;
        fs::create_dir_all(&task_dir)?;
        let events_path = task_dir.join("events.jsonl");
        let line = serde_json::to_string(event)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(events_path)?;
        writeln!(file, "{}", line)?;
        file.flush()?;
        Ok(())
    }

    /// Read all task events from `.trellis/tasks/*/events.jsonl`.
    /// Skips blank lines. Returns errors for malformed JSON.
    pub fn read_all(&self) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        for task_id in self.task_ids()? {
            events.extend(self.read_for_task(&task_id)?);
        }
        Ok(events)
    }

    /// Read events for a specific task.
    pub fn read_for_task(&self, task_id: &str) -> Result<Vec<Event>> {
        let events_path = self.events_path(task_id)?;
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
            if event.task_id != task_id {
                return Err(anyhow!(
                    "{} line {}: event task_id '{}' does not match task directory '{}'",
                    events_path.display(),
                    i + 1,
                    event.task_id,
                    task_id
                ));
            }
            events.push(event);
        }
        Ok(events)
    }

    /// Get the next sequence number for a task.
    pub fn next_seq_for_task(&self, task_id: &str) -> Result<i64> {
        let events = self.read_for_task(task_id)?;
        let max_seq = events.iter().map(|e| e.seq).max().unwrap_or(0);
        Ok(max_seq + 1)
    }

    /// Collect all unique task IDs with canonical event logs.
    pub fn task_ids(&self) -> Result<Vec<String>> {
        if !self.tasks_dir.exists() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.tasks_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let task_id = entry.file_name().to_string_lossy().into_owned();
            validate_task_id(&task_id)?;
            if entry.path().join("events.jsonl").exists() {
                ids.push(task_id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Write a task view projection to `.trellis/tasks/<task>/task.json`.
    /// Uses atomic write (temp + rename) to avoid corruption.
    pub fn write_task_view(
        &self,
        task_id: &str,
        state: &crate::domain::task::TaskState,
    ) -> Result<()> {
        let view = serde_json::json!({
            "schema": "control.task-view.v1",
            "id": state.id,
            "phase": serde_json::to_value(&state.phase)?,
            "is_held": state.is_held,
            "is_archived": state.is_archived,
            "objective": state.objective,
            "read_scope": state.read_scope,
            "write_allow": state.write_allow,
            "write_deny": state.write_deny,
            "risk_triggers": state.risk_triggers,
            "gates": state.gates.iter().collect::<Vec<_>>(),
            "last_event_seq": state.last_seq,
        });
        let task_dir = self.task_dir(task_id)?;
        fs::create_dir_all(&task_dir)?;
        let task_path = task_dir.join("task.json");
        let temp_path = task_dir.join("task.json.tmp");
        let json_str = serde_json::to_string_pretty(&view)?;
        fs::write(&temp_path, &json_str)?;
        fs::rename(&temp_path, &task_path)?;
        Ok(())
    }
}

fn validate_task_id(task_id: &str) -> Result<()> {
    if task_id.trim().is_empty() {
        return Err(anyhow!("Task id must not be empty"));
    }
    if task_id == "." || task_id == ".." || task_id.contains('/') || task_id.contains('\\') {
        return Err(anyhow!(
            "Task id '{}' must be a single .trellis/tasks child directory",
            task_id
        ));
    }
    if Path::new(task_id).is_absolute() {
        return Err(anyhow!("Task id '{}' must be relative", task_id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::event::Event;
    use crate::domain::task::{apply, TaskState};
    use serde_json::json;
    use std::path::PathBuf;

    /// Helper: create a unique temp dir under system temp, cleaned up on drop.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let base = std::env::temp_dir();
            let name = format!(
                "control-test-{}-{}",
                std::process::id(),
                generate_test_counter()
            );
            let path = base.join(name);
            std::fs::create_dir_all(&path).unwrap();
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

    static TEST_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn generate_test_counter() -> u64 {
        TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    #[allow(dead_code)]
    fn make_event(task_id: &str, seq: i64, event_type: &str) -> Event {
        Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: format!("{:08x}-0000-4000-8000-000000000000", seq),
            command_id: format!("cmd-{}", seq),
            task_id: task_id.to_string(),
            seq,
            occurred_at: "2026-06-03T12:00:00Z".to_string(),
            actor: "human".to_string(),
            event_type: event_type.to_string(),
            payload: json!({}),
        }
    }

    fn make_create_event(task_id: &str, seq: i64) -> Event {
        Event {
            schema: "control.event-envelope.v1".to_string(),
            event_id: format!("{:08x}-0000-4000-8000-000000000000", seq),
            command_id: format!("cmd-{}", seq),
            task_id: task_id.to_string(),
            seq,
            occurred_at: "2026-06-03T12:00:00Z".to_string(),
            actor: "human".to_string(),
            event_type: "task_created".to_string(),
            payload: json!({
                "objective": "Test task",
                "read_scope": ["src"],
                "write_allow": ["src"],
                "write_deny": [],
                "risk_triggers": [],
                "gates": ["cargo_check"]
            }),
        }
    }

    #[test]
    fn test_init_creates_trellis_tasks_root_without_control_store() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        assert!(store.tasks_dir.exists());
        assert!(!dir.path().join(".control").exists());
    }

    #[test]
    fn test_init_is_idempotent_for_existing_trellis_tasks_root() {
        let dir = TempDir::new();
        FileEventStore::init(dir.path()).unwrap();
        assert!(FileEventStore::init(dir.path()).is_ok());
    }

    #[test]
    fn test_append_and_read() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let event = make_create_event("t1", 1);
        store.append(&event).unwrap();
        assert!(store.events_path("t1").unwrap().exists());

        let events = store.read_all().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].task_id, "t1");
        assert_eq!(events[0].seq, 1);
    }

    #[test]
    fn test_read_for_task_filters() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        store.append(&make_create_event("t1", 1)).unwrap();
        store.append(&make_create_event("t2", 1)).unwrap();

        let t1 = store.read_for_task("t1").unwrap();
        assert_eq!(t1.len(), 1);
        assert_eq!(t1[0].task_id, "t1");
    }

    #[test]
    fn test_next_seq_for_task() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        assert_eq!(store.next_seq_for_task("t1").unwrap(), 1);
        store.append(&make_create_event("t1", 1)).unwrap();
        assert_eq!(store.next_seq_for_task("t1").unwrap(), 2);
    }

    #[test]
    fn test_write_task_view() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let mut state = TaskState::new("t1");
        apply(&mut state, &make_create_event("t1", 1)).unwrap();

        store.write_task_view("t1", &state).unwrap();

        let view_path = store.task_dir("t1").unwrap().join("task.json");
        assert!(view_path.exists());
        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(view_path).unwrap()).unwrap();
        assert_eq!(content["schema"], "control.task-view.v1");
        assert_eq!(content["id"], "t1");
        assert_eq!(content["is_archived"], false);
    }
}
