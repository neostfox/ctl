pub mod run_store;

use anyhow::{anyhow, Result};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::domain::event::Event;
use crate::domain::telemetry::TelemetryEntry;

/// Max time to wait to acquire a per-task write lock before giving up.
const LOCK_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
/// A lock file older than this is treated as abandoned by a crashed writer and
/// reclaimed. The locked critical section (read-seq → validate → append) is a
/// few milliseconds, so no legitimate holder is ever this old.
const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

/// RAII guard for an exclusive per-task write lock. Dropping it releases the lock
/// (removes the lock file).
pub struct TaskLock {
    path: PathBuf,
}

impl Drop for TaskLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Whether a lock file is old enough to be considered abandoned.
fn lock_is_stale(path: &Path, stale_after: Duration) -> bool {
    match fs::metadata(path).and_then(|m| m.modified()) {
        // `elapsed` errors if mtime is in the future (clock skew) — treat as fresh.
        Ok(mtime) => mtime
            .elapsed()
            .map(|age| age >= stale_after)
            .unwrap_or(false),
        Err(_) => false,
    }
}

pub struct FileEventStore {
    pub tasks_dir: PathBuf,
    /// The `.ctl/` root (parent of `tasks_dir`). Holds cross-task artifacts like
    /// the telemetry evidence index (M5).
    pub ctl_dir: PathBuf,
}

impl FileEventStore {
    /// Create the `.ctl/tasks/` root used by M1 task ledgers.
    pub fn init(project_root: &Path) -> Result<Self> {
        let ctl_dir = project_root.join(".ctl");
        let tasks_dir = ctl_dir.join("tasks");
        fs::create_dir_all(&tasks_dir)?;
        Ok(Self { tasks_dir, ctl_dir })
    }

    /// Open an existing `.ctl/tasks/` task ledger root.
    pub fn open(project_root: &Path) -> Result<Self> {
        let ctl_dir = project_root.join(".ctl");
        let tasks_dir = ctl_dir.join("tasks");
        if !tasks_dir.exists() {
            return Err(anyhow!(".ctl/tasks/ not found. Run 'control init' first."));
        }
        if !tasks_dir.is_dir() {
            return Err(anyhow!(".ctl/tasks exists but is not a directory."));
        }
        Ok(Self { tasks_dir, ctl_dir })
    }

    pub fn task_dir(&self, task_id: &str) -> Result<PathBuf> {
        validate_task_id(task_id)?;
        Ok(self.tasks_dir.join(task_id))
    }

    pub fn events_path(&self, task_id: &str) -> Result<PathBuf> {
        Ok(self.task_dir(task_id)?.join("events.jsonl"))
    }

    /// Acquire an exclusive per-task write lock (cross-process, advisory).
    ///
    /// Held across the read-seq → validate → append critical section so two
    /// concurrent `ctl` processes cannot read the same max sequence and append
    /// conflicting events. Implemented as an atomic create-new lock file (no
    /// external deps); a lock left behind by a crashed writer is reclaimed after
    /// `LOCK_STALE_AFTER`.
    pub fn lock_task(&self, task_id: &str) -> Result<TaskLock> {
        self.lock_task_with(task_id, LOCK_ACQUIRE_TIMEOUT, LOCK_STALE_AFTER)
    }

    /// `lock_task` with explicit timing — for tests.
    fn lock_task_with(
        &self,
        task_id: &str,
        acquire_timeout: Duration,
        stale_after: Duration,
    ) -> Result<TaskLock> {
        let task_dir = self.task_dir(task_id)?;
        fs::create_dir_all(&task_dir)?;
        let path = task_dir.join(".lock");
        let start = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "pid={}", std::process::id());
                    return Ok(TaskLock { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path, stale_after) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() >= acquire_timeout {
                        return Err(anyhow!(
                            "could not acquire write lock for task '{}' within {:?}; \
                             another writer holds {}",
                            task_id,
                            acquire_timeout,
                            path.display()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    return Err(anyhow!(
                        "failed to acquire task lock {}: {}",
                        path.display(),
                        e
                    ))
                }
            }
        }
    }

    /// Append a single event to `.ctl/tasks/<task>/events.jsonl`.
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

    /// Read all task events from `.ctl/tasks/*/events.jsonl`.
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

    /// Write a task view projection to `.ctl/tasks/<task>/task.json`.
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

    // ── M5: telemetry evidence index ──
    //
    // `telemetry.jsonl` is a SEPARATE append-only evidence index (fact model),
    // not part of the canonical event ledger. It lives at the `.ctl/` root so it
    // is cross-task, mirroring `control.json`.

    /// Path to the cross-task telemetry evidence index.
    pub fn telemetry_path(&self) -> PathBuf {
        self.ctl_dir.join("telemetry.jsonl")
    }

    /// Append one telemetry evidence record. Append-only, flushed.
    pub fn append_telemetry(&self, entry: &TelemetryEntry) -> Result<()> {
        fs::create_dir_all(&self.ctl_dir)?;
        let line = serde_json::to_string(entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.telemetry_path())?;
        writeln!(file, "{}", line)?;
        file.flush()?;
        Ok(())
    }

    /// Read all telemetry entries for one task, in append order. Skips blank
    /// lines; parse errors carry the line number.
    pub fn read_telemetry_for_task(&self, task_id: &str) -> Result<Vec<TelemetryEntry>> {
        let path = self.telemetry_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: TelemetryEntry = serde_json::from_str(&line)
                .map_err(|e| anyhow!("{} line {}: parse error: {}", path.display(), i + 1, e))?;
            if entry.task_id == task_id {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
}

fn validate_task_id(task_id: &str) -> Result<()> {
    if task_id.trim().is_empty() {
        return Err(anyhow!("Task id must not be empty"));
    }
    if task_id == "." || task_id == ".." || task_id.contains('/') || task_id.contains('\\') {
        return Err(anyhow!(
            "Task id '{}' must be a single .ctl/tasks child directory",
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
    fn test_telemetry_append_and_read_round_trip() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        // No file yet → empty.
        assert!(store.read_telemetry_for_task("t1").unwrap().is_empty());

        let e1 = TelemetryEntry::new("t1", "test_failures", 2, "2026-06-14T00:00:00Z", "human");
        let e2 = TelemetryEntry::new("t2", "lint_errors", 1, "2026-06-14T00:00:01Z", "human");
        let e3 = TelemetryEntry::new("t1", "retries", 4, "2026-06-14T00:00:02Z", "agent");
        store.append_telemetry(&e1).unwrap();
        store.append_telemetry(&e2).unwrap();
        store.append_telemetry(&e3).unwrap();

        // Filters by task, preserves append order.
        let t1 = store.read_telemetry_for_task("t1").unwrap();
        assert_eq!(t1, vec![e1, e3]);
        let t2 = store.read_telemetry_for_task("t2").unwrap();
        assert_eq!(t2.len(), 1);
        assert_eq!(t2[0].kind, "lint_errors");
    }

    #[test]
    fn test_telemetry_skips_blank_lines() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let entry = TelemetryEntry::new("t1", "test_failures", 1, "2026-06-14T00:00:00Z", "human");
        store.append_telemetry(&entry).unwrap();
        // Inject a blank line.
        let mut f = OpenOptions::new()
            .append(true)
            .open(store.telemetry_path())
            .unwrap();
        writeln!(f).unwrap();
        assert_eq!(store.read_telemetry_for_task("t1").unwrap().len(), 1);
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

    // ── per-task write lock ──

    #[test]
    fn lock_acquire_release_reacquire() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        {
            let _g = store.lock_task("t").unwrap();
            assert!(store.task_dir("t").unwrap().join(".lock").exists());
        } // drop releases
        assert!(!store.task_dir("t").unwrap().join(".lock").exists());
        // Re-acquire succeeds after release.
        let _g2 = store.lock_task("t").unwrap();
    }

    #[test]
    fn lock_times_out_while_held() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let _held = store.lock_task("t").unwrap();
        // A second acquisition cannot succeed while the first is held.
        let r = store.lock_task_with("t", Duration::from_millis(150), Duration::from_secs(30));
        assert!(r.is_err(), "second lock must not acquire while held");
    }

    #[test]
    fn lock_blocks_then_succeeds_after_release() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let held = store.lock_task("t").unwrap();

        let store2 = FileEventStore::init(dir.path()).unwrap();
        let waiter = std::thread::spawn(move || {
            let start = Instant::now();
            let g = store2.lock_task_with("t", Duration::from_secs(5), Duration::from_secs(30));
            (g.is_ok(), start.elapsed())
        });

        std::thread::sleep(Duration::from_millis(200));
        drop(held); // release; waiter should now acquire
        let (ok, waited) = waiter.join().unwrap();
        assert!(ok, "waiter should acquire after release");
        assert!(
            waited >= Duration::from_millis(100),
            "waiter returned too early: {waited:?}"
        );
    }

    #[test]
    fn lock_reclaims_stale_lock() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        // Simulate a lock left by a crashed writer.
        let task_dir = store.task_dir("t").unwrap();
        std::fs::create_dir_all(&task_dir).unwrap();
        std::fs::write(task_dir.join(".lock"), "pid=999999").unwrap();
        // stale_after = 0 → any existing lock is reclaimable immediately.
        let g = store.lock_task_with("t", Duration::from_secs(1), Duration::ZERO);
        assert!(g.is_ok(), "stale lock must be reclaimed");
    }
}
