pub mod run_store;

use anyhow::{anyhow, Result};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::domain::event::Event;
use crate::domain::telemetry::TelemetryEntry;

/// Max time to wait to acquire a per-entity write lock before giving up.
pub(crate) const LOCK_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

/// Monotonic counter for lock ownership nonces (unique per process).
static LOCK_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn new_lock_nonce() -> String {
    let n = LOCK_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{}", std::process::id(), n)
}

/// RAII guard for an exclusive per-directory write lock (used for both the
/// per-task and the per-run ledgers, and the run registry).
///
/// Mutual exclusion is unconditional: the lock is acquired by atomically creating
/// the lock file (`create_new`), and nothing ever removes a lock it does not own.
/// In particular there is NO time-based "stale reclaim" — a slow-but-live holder
/// can never have its lock stolen (which would let two writers proceed at once),
/// and a crashed holder's lock is recovered explicitly, not by a heuristic. (A
/// race-free automatic reclaim is not achievable with plain lock files; it needs
/// OS advisory locks — flock/LockFileEx — which would add a platform dependency.)
pub struct DirLock {
    path: PathBuf,
    /// Ownership token written into the lock file at creation. Drop removes the
    /// file ONLY if it still carries this nonce, so a guard can never delete a
    /// lock now held by someone else.
    nonce: String,
}

impl Drop for DirLock {
    fn drop(&mut self) {
        // Remove only if we still own it (first line == our nonce).
        if let Ok(content) = fs::read_to_string(&self.path) {
            if content.lines().next() == Some(self.nonce.as_str()) {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

/// Acquire an exclusive lock on `<dir>/.lock` (cross-process, advisory). Shared by
/// the task ledger, run ledger, and run registry. `kind`/`id` only shape the
/// error message ("task"/"run"/"run-registry"). Creates `dir` first, so a lock
/// can be taken before the entity's directory exists (e.g. a new run). Mutual
/// exclusion is unconditional: a live holder's lock is never stolen; a crashed
/// holder's lock is recovered explicitly (the timeout error reports its path/pid).
pub(crate) fn lock_dir(
    dir: &Path,
    kind: &str,
    id: &str,
    acquire_timeout: Duration,
) -> Result<DirLock> {
    fs::create_dir_all(dir)?;
    let path = dir.join(".lock");
    let nonce = new_lock_nonce();
    let start = Instant::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                // Line 1: ownership nonce (checked on Drop). Line 2: holder pid
                // (diagnostic only — never used to steal a live lock).
                let _ = writeln!(f, "{}", nonce);
                let _ = writeln!(f, "pid={}", std::process::id());
                return Ok(DirLock { path, nonce });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if start.elapsed() >= acquire_timeout {
                    let holder = fs::read_to_string(&path).unwrap_or_default();
                    let holder_pid = holder.lines().nth(1).unwrap_or("pid=unknown");
                    return Err(anyhow!(
                        "could not acquire write lock for {} '{}' within {:?} (held by {}). \
                         If no ctl process is running, the holder crashed — remove the stale \
                         lock file to recover: {}",
                        kind,
                        id,
                        acquire_timeout,
                        holder_pid,
                        path.display()
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                return Err(anyhow!(
                    "failed to acquire {} lock {}: {}",
                    kind,
                    path.display(),
                    e
                ))
            }
        }
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
    /// external deps). Mutual exclusion is unconditional: a live holder's lock is
    /// never stolen. A lock left by a crashed writer must be removed explicitly
    /// (the acquire-timeout error reports its path and the holder pid).
    pub fn lock_task(&self, task_id: &str) -> Result<DirLock> {
        self.lock_task_with(task_id, LOCK_ACQUIRE_TIMEOUT)
    }

    /// `lock_task` with an explicit acquire timeout — for tests. Delegates to the
    /// shared `lock_dir` primitive; `task_dir` validates the id.
    fn lock_task_with(&self, task_id: &str, acquire_timeout: Duration) -> Result<DirLock> {
        let task_dir = self.task_dir(task_id)?;
        lock_dir(&task_dir, "task", task_id, acquire_timeout)
    }

    /// Append a single event to `.ctl/tasks/<task>/events.jsonl`.
    pub fn append(&self, event: &Event) -> Result<()> {
        let task_dir = self.task_dir(&event.task_id)?;
        fs::create_dir_all(&task_dir)?;
        let events_path = task_dir.join("events.jsonl");
        append_jsonl_line(&events_path, &serde_json::to_string(event)?)
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

        let content = fs::read_to_string(&events_path)?;
        let ends_with_newline = content.ends_with('\n');
        let lines: Vec<&str> = content.lines().collect();
        let last_idx = lines.len().saturating_sub(1);
        let mut events = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let line = *line;
            if line.trim().is_empty() {
                continue;
            }
            let event: Event = serde_json::from_str(line).map_err(|e| {
                torn_tail_or_parse_error(
                    &events_path,
                    i,
                    last_idx,
                    ends_with_newline,
                    e,
                    &format!("--task {task_id}"),
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

    /// Append one telemetry evidence record. Append-only, durably synced.
    pub fn append_telemetry(&self, entry: &TelemetryEntry) -> Result<()> {
        fs::create_dir_all(&self.ctl_dir)?;
        append_jsonl_line(&self.telemetry_path(), &serde_json::to_string(entry)?)
    }

    /// Detect (and, when `apply`, truncate) a torn trailing record on this task's
    /// event ledger. See [`repair_torn_tail`] for the exact recoverable shape.
    /// Read-only when `apply` is false. Returns the (possibly no-op) outcome.
    pub fn repair_task_ledger(&self, task_id: &str, apply: bool) -> Result<TailRepair> {
        repair_torn_tail(&self.events_path(task_id)?, apply)
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

/// Append one JSON line to a JSONL log and fsync it.
///
/// The record and its terminating newline are written in a single `write_all`,
/// so a concurrent reader never observes a half-written line and a crash cannot
/// split content from its terminator within this call. The trailing `sync_all`
/// upgrades the OS-buffer flush to a durable on-disk commit, so an acknowledged
/// append survives a power loss (crash-consistency for the canonical ledger).
pub(crate) fn append_jsonl_line(path: &Path, line: &str) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut buf = String::with_capacity(line.len() + 1);
    buf.push_str(line);
    buf.push('\n');
    file.write_all(buf.as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// Classify a JSONL parse failure: a *torn trailing record* (recoverable via
/// `ctl repair`) versus unrecoverable corruption.
///
/// A torn tail is the final line of a file that does NOT end in a newline — a
/// crash between writing a record's bytes and its terminator. That is the only
/// shape `ctl repair` will truncate; every other parse failure (mid-file, or a
/// newline-terminated final line) is corruption that must be inspected by hand.
pub(crate) fn torn_tail_or_parse_error(
    path: &Path,
    idx: usize,
    last_idx: usize,
    ends_with_newline: bool,
    err: serde_json::Error,
    repair_selector: &str,
) -> anyhow::Error {
    if idx == last_idx && !ends_with_newline {
        anyhow!(
            "{}: torn trailing record at line {} (incomplete append, missing newline \
             terminator): {}. Run `ctl repair {}` to truncate the partial record \
             (it backs up the ledger first).",
            path.display(),
            idx + 1,
            err,
            repair_selector
        )
    } else {
        anyhow!("{} line {}: parse error: {}", path.display(), idx + 1, err)
    }
}

/// Outcome of a torn-tail repair check on a JSONL ledger.
pub struct TailRepair {
    /// A torn trailing record was found (and truncated when `apply` was true).
    pub repaired: bool,
    /// Bytes that were (or, in a dry run, would be) removed.
    pub removed_bytes: usize,
    /// Backup written before truncation (only when a repair was applied).
    pub backup: Option<PathBuf>,
    /// Human-readable summary of what was found.
    pub detail: String,
}

/// Detect and optionally repair a *torn trailing record* — the single crash-
/// recoverable corruption shape on an append-only JSONL ledger.
///
/// A torn tail is the bytes after the last newline that fail to parse as an
/// `Event`, on a file that does not end in a newline (a crash between a record
/// and its terminator). Only that exact shape is touched: an empty/absent file,
/// a file ending in a newline, or a final line that parses (a complete record
/// missing only its newline) is left intact. Mid-file corruption is NEVER auto-
/// truncated — it must be reviewed by hand. When `apply` is true, the entire
/// ledger is first backed up to a `.corrupt[-N]` sibling, then the partial record
/// is truncated and the file fsynced.
pub(crate) fn repair_torn_tail(path: &Path, apply: bool) -> Result<TailRepair> {
    let noop = |detail: &str| TailRepair {
        repaired: false,
        removed_bytes: 0,
        backup: None,
        detail: detail.to_string(),
    };
    if !path.exists() {
        return Ok(noop("no ledger file"));
    }
    let bytes = fs::read(path)?;
    if bytes.is_empty() {
        return Ok(noop("empty ledger"));
    }
    if *bytes.last().unwrap() == b'\n' {
        return Ok(noop("ledger ends with a newline — no torn trailing record"));
    }
    // No terminating newline: everything after the last newline is the final,
    // possibly-incomplete record.
    let cut = bytes
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let tail = &bytes[cut..];
    // A final record that still parses is a complete write missing only its
    // newline — harmless (reads tolerate it). Leave it intact.
    if let Ok(s) = std::str::from_utf8(tail) {
        if !s.trim().is_empty() && serde_json::from_str::<Event>(s).is_ok() {
            return Ok(noop(
                "final record parses (missing only a newline) — left intact",
            ));
        }
    }
    let removed_bytes = tail.len();
    let mut backup = None;
    if apply {
        let bp = backup_path(path);
        fs::write(&bp, &bytes)?;
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_len(cut as u64)?;
        file.sync_all()?;
        backup = Some(bp);
    }
    Ok(TailRepair {
        repaired: true,
        removed_bytes,
        backup,
        detail: format!("torn trailing record of {removed_bytes} bytes"),
    })
}

/// First free `<file>.corrupt`, `<file>.corrupt-1`, … sibling, so a repair never
/// clobbers an earlier backup.
fn backup_path(path: &Path) -> PathBuf {
    let suffixed = |suffix: &str| {
        let mut name = path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(suffix);
        path.with_file_name(name)
    };
    let base = suffixed(".corrupt");
    if !base.exists() {
        return base;
    }
    for n in 1.. {
        let cand = suffixed(&format!(".corrupt-{n}"));
        if !cand.exists() {
            return cand;
        }
    }
    unreachable!()
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
        let r = store.lock_task_with("t", Duration::from_millis(150));
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
            let g = store2.lock_task_with("t", Duration::from_secs(5));
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

    // Safety: a LIVE holder's lock is never stolen (no time-based reclaim), so two
    // writers can never proceed at once. (Regression for the broken stale-reclaim.)
    #[test]
    fn live_lock_is_never_stolen() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let _held = store.lock_task("t").unwrap(); // LIVE holder (this process)
        let stolen = store
            .lock_task_with("t", Duration::from_millis(200))
            .is_ok();
        assert!(!stolen, "live lock was stolen — mutual exclusion broken");
    }

    // Safety: Drop removes the lock ONLY if it still owns it. If the file has been
    // replaced by another holder, Drop must leave it intact.
    #[test]
    fn drop_does_not_delete_foreign_lock() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let path = store.task_dir("t").unwrap().join(".lock");
        let a = store.lock_task("t").unwrap();
        std::fs::write(&path, "someone-else\npid=999").unwrap(); // now another holder's
        drop(a);
        assert!(path.exists(), "Drop deleted a lock it no longer owns");
        let _ = std::fs::remove_file(&path);
    }

    // A crashed holder's lock can be recovered by removing the file; a fresh
    // acquire then succeeds (the supported explicit recovery path).
    #[test]
    fn explicit_recovery_after_stale_lock() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let task_dir = store.task_dir("t").unwrap();
        std::fs::create_dir_all(&task_dir).unwrap();
        let lock_path = task_dir.join(".lock");
        std::fs::write(&lock_path, "dead-holder\npid=999999").unwrap();
        // Not reclaimed automatically:
        assert!(store
            .lock_task_with("t", Duration::from_millis(100))
            .is_err());
        // Explicit recovery:
        std::fs::remove_file(&lock_path).unwrap();
        assert!(store.lock_task("t").is_ok());
    }

    // ── torn-tail crash recovery ──

    #[test]
    fn torn_trailing_record_detected_and_repaired() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        store.append(&make_create_event("t1", 1)).unwrap();
        store.append(&make_create_event("t1", 2)).unwrap();
        // Simulate a crash mid-append: a partial record with no trailing newline.
        let path = store.events_path("t1").unwrap();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"schema\":\"control.event-envelope.v1\",\"seq\":3")
            .unwrap();
        drop(f);

        // Read surfaces it as a *torn trailing record* (recoverable), pointing at
        // `ctl repair` — not an opaque parse error.
        let err = store.read_for_task("t1").unwrap_err().to_string();
        assert!(err.contains("torn trailing record"), "got: {err}");
        assert!(err.contains("ctl repair"), "got: {err}");

        // Dry run detects without mutating.
        let dry = store.repair_task_ledger("t1", false).unwrap();
        assert!(dry.repaired && dry.removed_bytes > 0 && dry.backup.is_none());
        assert!(
            store.read_for_task("t1").is_err(),
            "dry run must not mutate"
        );

        // Apply truncates the partial record (after backing up the ledger).
        let done = store.repair_task_ledger("t1", true).unwrap();
        assert!(done.repaired);
        assert!(done.backup.as_ref().unwrap().exists());
        let events = store.read_for_task("t1").unwrap();
        assert_eq!(events.len(), 2, "the two intact records survive");
    }

    #[test]
    fn complete_final_record_missing_newline_is_left_intact() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        store.append(&make_create_event("t1", 1)).unwrap();
        // A COMPLETE record written without its terminator (crash after content,
        // before the newline) — harmless, not a torn tail.
        let path = store.events_path("t1").unwrap();
        let line = serde_json::to_string(&make_create_event("t1", 2)).unwrap();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(line.as_bytes()).unwrap();
        drop(f);
        // Reads fine, and repair recognizes it as intact (no truncation).
        assert_eq!(store.read_for_task("t1").unwrap().len(), 2);
        let r = store.repair_task_ledger("t1", true).unwrap();
        assert!(!r.repaired, "{}", r.detail);
    }

    #[test]
    fn mid_file_corruption_is_not_treated_as_torn_tail() {
        let dir = TempDir::new();
        let store = FileEventStore::init(dir.path()).unwrap();
        let good = serde_json::to_string(&make_create_event("t1", 1)).unwrap();
        let path = store.task_dir("t1").unwrap().join("events.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Broken FIRST line, good last line, file terminated by a newline.
        fs::write(&path, format!("{{not valid json\n{good}\n")).unwrap();
        // Read fails as an ordinary parse error — NOT a recoverable torn tail.
        let err = store.read_for_task("t1").unwrap_err().to_string();
        assert!(err.contains("parse error"), "got: {err}");
        assert!(!err.contains("torn trailing record"), "got: {err}");
        // Repair refuses: the file ends with a newline, so there is no torn tail.
        let r = store.repair_task_ledger("t1", true).unwrap();
        assert!(
            !r.repaired,
            "mid-file corruption must never be auto-truncated"
        );
    }
}
