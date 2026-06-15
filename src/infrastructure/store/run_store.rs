//! Run event store for M6 multi-agent concurrency.
//!
//! Mirrors `FileEventStore` but operates under `.ctl/runs/`.
//! Each run has its own directory with `events.jsonl` and `run.json`.

use anyhow::{anyhow, Result};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{lock_dir, DirLock, LOCK_ACQUIRE_TIMEOUT};
use crate::domain::event::Event;
use crate::domain::run::AgentRunState;
pub struct RunEventStore {
    pub runs_dir: PathBuf,
}

impl RunEventStore {
    /// Create the `.ctl/runs/` root directory.
    pub fn init(project_root: &Path) -> Result<Self> {
        let runs_dir = project_root.join(".ctl").join("runs");
        fs::create_dir_all(&runs_dir)?;
        Ok(Self { runs_dir })
    }

    /// Path to a specific run directory: `.ctl/runs/<run_id>/`
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir.join(run_id)
    }

    /// Path to a run's event log: `.ctl/runs/<run_id>/events.jsonl`
    pub fn events_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("events.jsonl")
    }

    /// Append a single event to `.ctl/runs/<run_id>/events.jsonl`.
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

    /// Acquire the exclusive per-run write lock, held across the run ledger's
    /// read-seq → validate → append critical section (mirrors `lock_task`). The
    /// run dir need not exist yet — `lock_dir` creates it.
    pub fn lock_run(&self, run_id: &str) -> Result<DirLock> {
        self.lock_run_with(run_id, LOCK_ACQUIRE_TIMEOUT)
    }

    /// `lock_run` with an explicit acquire timeout — for tests.
    fn lock_run_with(&self, run_id: &str, acquire_timeout: Duration) -> Result<DirLock> {
        validate_run_id(run_id)?;
        lock_dir(&self.run_dir(run_id), "run", run_id, acquire_timeout)
    }

    /// Acquire the coarse run-registry lock (`.ctl/runs/.lock`). Serializes
    /// concurrent run *starts* so the cross-run scope-overlap check and the
    /// `run_started` append are atomic across runs — two starts cannot both pass
    /// the disjoint-scope check against a snapshot that excludes each other.
    /// Always acquired BEFORE any per-run lock (registry → per-run) to stay
    /// deadlock-free. The `.lock` file is skipped by `run_ids` (it is not a dir).
    pub fn lock_run_registry(&self) -> Result<DirLock> {
        lock_dir(
            &self.runs_dir,
            "run-registry",
            "__registry__",
            LOCK_ACQUIRE_TIMEOUT,
        )
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

    /// Write a run view projection to `.ctl/runs/<run_id>/run.json`.
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
        // `generate_uuid` mixes a nanosecond clock with a process-global atomic
        // counter, so the path is unique across runs even when the OS recycles a
        // pid. The old `pid + counter` scheme reused paths between runs; with no
        // cleanup that meant a fresh test could open a stale `events.jsonl`/`.lock`
        // left by an earlier run and flake (read back >1 events, or a stale lock
        // that never releases).
        let base = std::env::temp_dir().join(format!(
            "control-run-test-{}",
            crate::application::generate_uuid()
        ));
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

    // ── per-run write lock (mirrors the task-lock suite in the parent module) ──

    #[test]
    fn lock_run_acquire_release_reacquire() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        {
            let _g = store.lock_run("run-a").unwrap();
            assert!(store.run_dir("run-a").join(".lock").exists());
        } // drop releases
        assert!(!store.run_dir("run-a").join(".lock").exists());
        let _g2 = store.lock_run("run-a").unwrap(); // re-acquire after release
    }

    #[test]
    fn lock_run_times_out_while_held() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        let _held = store.lock_run("run-a").unwrap();
        let r = store.lock_run_with("run-a", Duration::from_millis(150));
        assert!(r.is_err(), "second run lock must not acquire while held");
    }

    #[test]
    fn lock_run_blocks_then_succeeds_after_release() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        let held = store.lock_run("run-a").unwrap();

        let store2 = RunEventStore::init(&tmp).unwrap();
        let waiter = std::thread::spawn(move || {
            let g = store2.lock_run_with("run-a", Duration::from_secs(5));
            g.is_ok()
        });
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
        assert!(
            waiter.join().unwrap(),
            "waiter should acquire after release"
        );
    }

    #[test]
    fn run_live_lock_is_never_stolen() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        let _held = store.lock_run("run-a").unwrap();
        let stolen = store
            .lock_run_with("run-a", Duration::from_millis(200))
            .is_ok();
        assert!(
            !stolen,
            "live run lock was stolen — mutual exclusion broken"
        );
    }

    #[test]
    fn run_drop_does_not_delete_foreign_lock() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        let path = store.run_dir("run-a").join(".lock");
        let a = store.lock_run("run-a").unwrap();
        std::fs::write(&path, "someone-else\npid=999").unwrap();
        drop(a);
        assert!(path.exists(), "Drop deleted a run lock it no longer owns");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_explicit_recovery_after_stale_lock() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        let run_dir = store.run_dir("run-a");
        fs::create_dir_all(&run_dir).unwrap();
        let lock_path = run_dir.join(".lock");
        std::fs::write(&lock_path, "dead-holder\npid=999999").unwrap();
        assert!(store
            .lock_run_with("run-a", Duration::from_millis(100))
            .is_err());
        std::fs::remove_file(&lock_path).unwrap();
        assert!(store.lock_run("run-a").is_ok());
    }

    #[test]
    fn run_registry_lock_acquires_and_releases() {
        let tmp = make_tmp_dir();
        let store = RunEventStore::init(&tmp).unwrap();
        {
            let _g = store.lock_run_registry().unwrap();
            assert!(store.runs_dir.join(".lock").exists());
        }
        assert!(!store.runs_dir.join(".lock").exists());
    }

    // The crux of single-writer: concurrent writers that each acquire lock_run,
    // read the next seq, and append cannot collide — the resulting stream has
    // contiguous, unique seqs (no duplicate seq, no gap). Deterministic assertion
    // (no timing): without the lock, the read-seq/append race would drop or
    // duplicate events.
    #[test]
    fn concurrent_locked_appends_get_contiguous_seqs() {
        let tmp = make_tmp_dir();
        let n_threads = 4;
        let per_thread = 5;
        let threads: Vec<_> = (0..n_threads)
            .map(|t| {
                let tmp = tmp.clone();
                std::thread::spawn(move || {
                    let store = RunEventStore::init(&tmp).unwrap();
                    for i in 0..per_thread {
                        let _lock = store.lock_run("run-x").unwrap();
                        let seq = store.next_seq_for_run("run-x").unwrap();
                        let event = Event {
                            schema: "control.event-envelope.v1".to_string(),
                            event_id: format!("evt-{}-{}", t, i),
                            command_id: format!("cmd-{}-{}", t, i),
                            task_id: "run-x".to_string(),
                            seq,
                            occurred_at: "2026-06-15T00:00:00Z".to_string(),
                            actor: "agent".to_string(),
                            event_type: "run_started".to_string(),
                            payload: serde_json::json!({}),
                        };
                        store.append(&event).unwrap();
                        // _lock drops here, releasing before the next iteration.
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let events = RunEventStore::init(&tmp)
            .unwrap()
            .read_for_run("run-x")
            .unwrap();
        let mut seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
        seqs.sort_unstable();
        let expected: Vec<i64> = (1..=(n_threads * per_thread) as i64).collect();
        assert_eq!(
            seqs, expected,
            "per-run lock must serialize seq allocation: no duplicate seq, no gap"
        );
    }
}
