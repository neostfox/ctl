#[allow(dead_code)]
mod adapters;
#[allow(dead_code)]
mod application;
mod cli;
mod domain;
mod infrastructure;

fn main() -> anyhow::Result<()> {
    // The clap-derived CLI builds a deep command tree (~40 top-level
    // subcommands, several with their own nested subcommands). That recursive
    // construction overflows the OS default main-thread stack on Windows in
    // *debug* builds — every subcommand exits 253 with no output, including
    // `--version`. Release builds (smaller frames) and Linux (8 MB main stack)
    // are unaffected, so CI never caught it. Run the whole CLI on a worker
    // thread with a Linux-sized stack so the binary behaves identically across
    // debug/release and platforms. (`std::thread` is permitted — `gates/` and
    // `store/` already use it; it is not an async runtime.)
    let handle = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(cli::run)
        .map_err(|e| anyhow::anyhow!("failed to spawn ctl worker thread: {e}"))?;
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("ctl worker thread panicked"))?
}

#[cfg(test)]
mod tests {
    use crate::domain::event::Event;
    use crate::domain::task::{apply, Phase, TaskState};
    use crate::infrastructure::schema_validator::SchemaValidator;
    use serde_json::json;
    use std::fs;

    #[test]
    fn test_reducer() {
        let content = fs::read_to_string("fixtures/reducer_test.jsonl").unwrap();
        let mut state = TaskState::new("t1");

        for line in content.lines() {
            let event: Event = serde_json::from_str(line).unwrap();
            apply(&mut state, &event).unwrap();
        }

        assert_eq!(state.phase, Phase::InProgress);
        assert_eq!(state.history.len(), 3);
    }

    #[test]
    fn test_schema_validation() {
        let validator = SchemaValidator::new("schemas/").unwrap();

        let valid = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1",
            "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human",
            "type": "task_created",
            "payload": {
                "objective": "Test task",
                "read_scope": ["src/"],
                "write_allow": ["src/"],
                "write_deny": [],
                "risk_triggers": [],
                "gates": ["cargo_check"]
            }
        });
        assert!(validator
            .validate_instance(&valid, "control.event-envelope.v1")
            .is_ok());

        let missing_field = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1",
            "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "type": "task_created",
            "payload": {}
        });
        assert!(validator
            .validate_instance(&missing_field, "control.event-envelope.v1")
            .is_err());

        let bad_uuid = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "not-a-uuid",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1",
            "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human",
            "type": "task_created",
            "payload": {}
        });
        assert!(validator
            .validate_instance(&bad_uuid, "control.event-envelope.v1")
            .is_err());
    }
}
