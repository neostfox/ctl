//! M0 audit matrix: comprehensive tests for schema validation, reducer invariants,
//! replay determinism, hold mechanics, and baseline regression.

#[cfg(test)]
mod tests {
    use crate::domain::event::Event;
    use crate::domain::task::{apply, Phase, TaskState};
    use crate::infrastructure::schema_validator::SchemaValidator;
    use serde_json::json;
    use std::fs;

    // ============================================================
    // Schema counter-examples
    // ============================================================

    #[test]
    fn schema_valid_instance() {
        let validator = SchemaValidator::new("schemas/").unwrap();
        let valid = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "task_created", "payload": task_payload(
                "Test",
                &["src/"],
                &["src/"],
                &["cargo_check"]
            )
        });
        assert!(validator
            .validate_instance(&valid, "control.event-envelope.v1")
            .is_ok());
    }

    #[test]
    fn schema_rejects_empty_boundary_arrays() {
        let validator = SchemaValidator::new("schemas/").unwrap();
        let empty_read_scope = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "task_created", "payload": task_payload(
                "Test",
                &[],
                &["src/"],
                &["cargo_check"]
            )
        });
        assert!(validator
            .validate_instance(&empty_read_scope, "control.event-envelope.v1")
            .is_err());

        let empty_write_allow = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "task_created", "payload": task_payload(
                "Test",
                &["src/"],
                &[],
                &["cargo_check"]
            )
        });
        assert!(validator
            .validate_instance(&empty_write_allow, "control.event-envelope.v1")
            .is_err());

        let empty_gates = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "task_created", "payload": task_payload(
                "Test",
                &["src/"],
                &["src/"],
                &[]
            )
        });
        assert!(validator
            .validate_instance(&empty_gates, "control.event-envelope.v1")
            .is_err());
    }

    #[test]
    fn schema_rejects_missing_required_field() {
        let validator = SchemaValidator::new("schemas/").unwrap();
        let bad = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "type": "task_created", "payload": {}
        });
        assert!(validator
            .validate_instance(&bad, "control.event-envelope.v1")
            .is_err());
    }

    #[test]
    fn schema_rejects_bad_uuid() {
        let validator = SchemaValidator::new("schemas/").unwrap();
        let bad = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "not-a-uuid",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "task_created", "payload": {}
        });
        assert!(validator
            .validate_instance(&bad, "control.event-envelope.v1")
            .is_err());
    }

    #[test]
    fn schema_rejects_zero_seq() {
        let validator = SchemaValidator::new("schemas/").unwrap();
        let bad = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 0,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "task_created", "payload": {}
        });
        assert!(validator
            .validate_instance(&bad, "control.event-envelope.v1")
            .is_err());
    }

    #[test]
    fn schema_rejects_unknown_event_type() {
        let validator = SchemaValidator::new("schemas/").unwrap();
        let bad = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "unknown_event", "payload": {}
        });
        assert!(validator
            .validate_instance(&bad, "control.event-envelope.v1")
            .is_err());
    }

    #[test]
    fn schema_rejects_extra_field() {
        let validator = SchemaValidator::new("schemas/").unwrap();
        let bad = json!({
            "schema": "control.event-envelope.v1",
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "command_id": "550e8400-e29b-41d4-a716-446655440001",
            "task_id": "t1", "seq": 1,
            "occurred_at": "2026-05-30T10:00:00Z",
            "actor": "human", "type": "task_created", "payload": {},
            "unexpected_field": "nope"
        });
        assert!(validator
            .validate_instance(&bad, "control.event-envelope.v1")
            .is_err());
    }

    #[test]
    fn schema_counter_examples_from_fixture() {
        let content = fs::read_to_string("fixtures/schema_counter_examples.json").unwrap();
        let fixtures: serde_json::Value = serde_json::from_str(&content).unwrap();
        let validator = SchemaValidator::new("schemas/").unwrap();
        if let Some(cases) = fixtures.get("event_envelope").and_then(|v| v.as_object()) {
            for (name, instance) in cases {
                assert!(
                    validator
                        .validate_instance(instance, "control.event-envelope.v1")
                        .is_err(),
                    "Counter-example '{}' should fail",
                    name
                );
            }
        }
        if let Some(cases) = fixtures.get("task_definition").and_then(|v| v.as_object()) {
            for (name, instance) in cases {
                assert!(
                    validator
                        .validate_instance(instance, "control.task-definition.v1")
                        .is_err(),
                    "Counter-example '{}' should fail",
                    name
                );
            }
        }
    }

    // ============================================================
    // Reducer: fixture replay
    // ============================================================

    #[test]
    fn reducer_original_fixture() {
        let content = fs::read_to_string("fixtures/reducer_test.jsonl").unwrap();
        let mut state = TaskState::new("t1");
        for line in content.lines() {
            apply(&mut state, &serde_json::from_str::<Event>(line).unwrap()).unwrap();
        }
        assert_eq!(state.phase, Phase::InProgress);
        assert_eq!(state.history.len(), 3);
    }

    #[test]
    fn reducer_full_lifecycle() {
        let state = replay("t-lifecycle", "fixtures/reducer_lifecycle.jsonl");
        assert_eq!(state.phase, Phase::Completed);
        assert!(state.is_archived);
        // 10 events: 6 lifecycle + 2 gate_checked + task_completed + task_archived
        assert_eq!(state.history.len(), 10);
        assert_eq!(state.last_seq, 10);
    }

    #[test]
    fn reducer_hold_blocks_transitions() {
        let state = replay("t-hold", "fixtures/reducer_hold.jsonl");
        assert_eq!(state.phase, Phase::Completed);
        // 8 events: 6 hold transitions + gate_checked + task_completed
        assert_eq!(state.history.len(), 8);
    }

    #[test]
    fn reducer_rejects_legacy_scope_boundary() {
        let mut state = TaskState::new("t-legacy");
        let result = apply(
            &mut state,
            &ev(
                1,
                "t-legacy",
                "task_created",
                json!({
                    "objective": "legacy",
                    "scope": ["src/"],
                    "read_scope": ["src/"],
                    "write_allow": ["src/"],
                    "write_deny": [],
                    "risk_triggers": [],
                    "gates": ["cargo_check"]
                }),
            ),
        );
        assert!(
            result.is_err(),
            "legacy scope must not enter canonical state"
        );
    }

    #[test]
    fn reducer_stores_boundary_fields_in_deterministic_order() {
        let mut state = TaskState::new("t-boundary");
        apply(
            &mut state,
            &ev(
                1,
                "t-boundary",
                "task_created",
                json!({
                    "objective": "ordered",
                    "read_scope": ["z/", "a/", "z/"],
                    "write_allow": ["src/b.rs", "src/a.rs"],
                    "write_deny": ["target/", ".git/"],
                    "risk_triggers": ["deps", "schema"],
                    "gates": ["cargo_test", "cargo_check"]
                }),
            ),
        )
        .unwrap();

        assert_eq!(
            state
                .read_scope
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["a/", "z/"]
        );
        assert_eq!(
            state
                .write_allow
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["src/a.rs", "src/b.rs"]
        );
        assert_eq!(
            state
                .write_deny
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![".git/", "target/"]
        );
        assert_eq!(
            state
                .risk_triggers
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["deps", "schema"]
        );
        assert_eq!(
            state.gates.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["cargo_check", "cargo_test"]
        );
    }

    // ============================================================
    // Reducer: illegal state transitions
    // ============================================================

    #[test]
    fn reject_start_from_planning() {
        let mut s = TaskState::new("t-sp");
        apply(
            &mut s,
            &ev(
                1,
                "t-sp",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        assert!(apply(&mut s, &ev(2, "t-sp", "task_started", json!({}))).is_err());
    }

    #[test]
    fn reject_start_from_in_progress() {
        let mut s = TaskState::new("t-sip");
        apply(
            &mut s,
            &ev(
                1,
                "t-sip",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-sip", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-sip", "task_started", json!({}))).unwrap();
        assert!(apply(&mut s, &ev(4, "t-sip", "task_started", json!({}))).is_err());
    }

    #[test]
    fn reject_submit_from_planning() {
        let mut s = TaskState::new("t-spl");
        apply(
            &mut s,
            &ev(
                1,
                "t-spl",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        assert!(apply(
            &mut s,
            &ev(2, "t-spl", "task_submitted_for_review", json!({}))
        )
        .is_err());
    }

    #[test]
    fn reject_submit_from_ready() {
        let mut s = TaskState::new("t-sr");
        apply(
            &mut s,
            &ev(
                1,
                "t-sr",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-sr", "task_marked_ready", json!({}))).unwrap();
        assert!(apply(
            &mut s,
            &ev(3, "t-sr", "task_submitted_for_review", json!({}))
        )
        .is_err());
    }

    #[test]
    fn reject_complete_from_planning() {
        let mut s = TaskState::new("t-cp");
        apply(
            &mut s,
            &ev(
                1,
                "t-cp",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        assert!(apply(&mut s, &ev(2, "t-cp", "task_completed", json!({}))).is_err());
    }

    #[test]
    fn reject_complete_from_in_progress() {
        let mut s = TaskState::new("t-cip");
        apply(
            &mut s,
            &ev(
                1,
                "t-cip",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-cip", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-cip", "task_started", json!({}))).unwrap();
        assert!(apply(&mut s, &ev(4, "t-cip", "task_completed", json!({}))).is_err());
    }

    #[test]
    fn reject_reopen_from_planning() {
        let mut s = TaskState::new("t-rp");
        apply(
            &mut s,
            &ev(
                1,
                "t-rp",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        assert!(apply(&mut s, &ev(2, "t-rp", "task_reopened", json!({}))).is_err());
    }

    #[test]
    fn reject_cancel_from_completed() {
        let mut s = TaskState::new("t-cc");
        apply(
            &mut s,
            &ev(
                1,
                "t-cc",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-cc", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-cc", "task_started", json!({}))).unwrap();
        apply(
            &mut s,
            &ev(4, "t-cc", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        apply(&mut s, &ev(5, "t-cc", "gate_checked", json!({"gate_id":"cargo_check","passed":true,"evidence":"ok","checked_at":"2026-05-30T12:00:00Z"}))).unwrap();
        apply(&mut s, &ev(6, "t-cc", "task_completed", json!({}))).unwrap();
        assert!(apply(&mut s, &ev(7, "t-cc", "task_cancelled", json!({}))).is_err());
    }

    #[test]
    fn reject_cancel_from_cancelled() {
        let mut s = TaskState::new("t-canc");
        apply(
            &mut s,
            &ev(
                1,
                "t-canc",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-canc", "task_cancelled", json!({}))).unwrap();
        assert!(apply(&mut s, &ev(3, "t-canc", "task_cancelled", json!({}))).is_err());
    }

    #[test]
    fn reject_ready_without_objective() {
        let mut s = TaskState::new("t-no");
        s.read_scope.insert("src/".into());
        s.write_allow.insert("src/".into());
        s.gates.insert("cargo_check".into());
        assert!(apply(&mut s, &ev(1, "t-no", "task_marked_ready", json!({}))).is_err());
    }

    #[test]
    fn reject_ready_without_gates() {
        let mut s = TaskState::new("t-ng");
        s.objective = Some("t".into());
        s.read_scope.insert("src/".into());
        s.write_allow.insert("src/".into());
        assert!(apply(&mut s, &ev(1, "t-ng", "task_marked_ready", json!({}))).is_err());
    }

    #[test]
    fn reject_ready_without_read_scope() {
        let mut s = TaskState::new("t-nrs");
        s.objective = Some("t".into());
        s.write_allow.insert("src/".into());
        s.gates.insert("cargo_check".into());
        assert!(apply(&mut s, &ev(1, "t-nrs", "task_marked_ready", json!({}))).is_err());
    }

    #[test]
    fn reject_ready_without_write_allow() {
        let mut s = TaskState::new("t-nwa");
        s.objective = Some("t".into());
        s.read_scope.insert("src/".into());
        s.gates.insert("cargo_check".into());
        assert!(apply(&mut s, &ev(1, "t-nwa", "task_marked_ready", json!({}))).is_err());
    }

    // ============================================================
    // Reducer: idempotency and seq validation
    // ============================================================

    #[test]
    fn reducer_idempotent_command() {
        let mut s = TaskState::new("t-idem");
        let e = ev(
            1,
            "t-idem",
            "task_created",
            task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
        );
        apply(&mut s, &e).unwrap();
        apply(&mut s, &e).unwrap();
        assert_eq!(s.history.len(), 1);
    }

    #[test]
    fn reducer_rejects_seq_not_increasing() {
        let mut s = TaskState::new("t-seq");
        apply(
            &mut s,
            &ev(
                1,
                "t-seq",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        let dup = Event {
            schema: "control.event-envelope.v1".into(),
            event_id: "e-dup".into(),
            command_id: "c-dup".into(),
            task_id: "t-seq".into(),
            seq: 1,
            occurred_at: "2026-05-30T10:00:00Z".into(),
            actor: "human".into(),
            event_type: "task_marked_ready".into(),
            payload: json!({}),
        };
        assert!(apply(&mut s, &dup).is_err());
    }

    #[test]
    fn reducer_rejects_task_id_mismatch() {
        let mut s = TaskState::new("t-mine");
        apply(
            &mut s,
            &ev(
                1,
                "t-mine",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        assert!(apply(&mut s, &ev(2, "t-other", "task_marked_ready", json!({}))).is_err());
    }

    #[test]
    fn reducer_rejects_unknown_event_type() {
        let mut s = TaskState::new("t-unk");
        apply(
            &mut s,
            &ev(
                1,
                "t-unk",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        assert!(apply(&mut s, &ev(2, "t-unk", "nonexistent_event", json!({}))).is_err());
    }

    #[test]
    fn reject_duplicate_task_created() {
        let mut s = TaskState::new("t-dup");
        apply(
            &mut s,
            &ev(
                1,
                "t-dup",
                "task_created",
                task_payload("first", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        assert!(apply(
            &mut s,
            &ev(
                2,
                "t-dup",
                "task_created",
                task_payload("second", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .is_err());
    }

    // ============================================================
    // Hold + boundary_violation
    // ============================================================

    #[test]
    fn hold_prevents_start() {
        let mut s = TaskState::new("t-hs");
        apply(
            &mut s,
            &ev(
                1,
                "t-hs",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-hs", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-hs", "hold_entered", json!({}))).unwrap();
        assert!(apply(&mut s, &ev(4, "t-hs", "task_started", json!({}))).is_err());
    }

    #[test]
    fn hold_prevents_submit() {
        let mut s = TaskState::new("t-hsub");
        apply(
            &mut s,
            &ev(
                1,
                "t-hsub",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-hsub", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-hsub", "task_started", json!({}))).unwrap();
        apply(&mut s, &ev(4, "t-hsub", "hold_entered", json!({}))).unwrap();
        assert!(apply(
            &mut s,
            &ev(5, "t-hsub", "task_submitted_for_review", json!({}))
        )
        .is_err());
    }

    #[test]
    fn boundary_violation_enters_hold() {
        let mut s = TaskState::new("t-bv");
        apply(
            &mut s,
            &ev(
                1,
                "t-bv",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-bv", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-bv", "task_started", json!({}))).unwrap();
        apply(
            &mut s,
            &ev(4, "t-bv", "boundary_violation_recorded", json!({})),
        )
        .unwrap();
        assert!(s.is_held);
        assert!(apply(
            &mut s,
            &ev(5, "t-bv", "task_submitted_for_review", json!({}))
        )
        .is_err());
    }

    #[test]
    fn hold_exited_allows_progress() {
        let mut s = TaskState::new("t-he");
        apply(
            &mut s,
            &ev(
                1,
                "t-he",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-he", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-he", "task_started", json!({}))).unwrap();
        apply(&mut s, &ev(4, "t-he", "hold_entered", json!({}))).unwrap();
        assert!(s.is_held);
        apply(&mut s, &ev(5, "t-he", "hold_exited", json!({}))).unwrap();
        assert!(!s.is_held);
        apply(
            &mut s,
            &ev(6, "t-he", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        assert_eq!(s.phase, Phase::Review);
    }

    // ============================================================
    // Replay determinism
    // ============================================================

    #[test]
    fn replay_determinism_lifecycle() {
        let s1 = replay("t-lifecycle", "fixtures/reducer_lifecycle.jsonl");
        let s2 = replay("t-lifecycle", "fixtures/reducer_lifecycle.jsonl");
        assert_eq!(s1.phase, s2.phase);
        assert_eq!(s1.last_seq, s2.last_seq);
        assert_eq!(s1.history, s2.history);
        assert_eq!(s1.processed_commands, s2.processed_commands);
        assert_eq!(s1.is_held, s2.is_held);
    }

    #[test]
    fn replay_determinism_hold() {
        let s1 = replay("t-hold", "fixtures/reducer_hold.jsonl");
        let s2 = replay("t-hold", "fixtures/reducer_hold.jsonl");
        assert_eq!(s1.phase, s2.phase);
        assert_eq!(s1.last_seq, s2.last_seq);
        assert_eq!(s1.history, s2.history);
    }

    // ============================================================
    // Completion interlock (STATE-012)
    // ============================================================
    #[test]
    fn completion_interlock_rejects_without_gate_results() {
        let mut s = TaskState::new("t-ci1");
        apply(
            &mut s,
            &ev(
                1,
                "t-ci1",
                "task_created",
                task_payload(
                    "t",
                    &["src/"],
                    &["src/"],
                    &["cargo_fmt_check", "cargo_test"],
                ),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-ci1", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-ci1", "task_started", json!({}))).unwrap();
        apply(
            &mut s,
            &ev(4, "t-ci1", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        // No gate_checked events — completion must be blocked
        let result = apply(&mut s, &ev(5, "t-ci1", "task_completed", json!({})));
        assert!(
            result.is_err(),
            "Must reject completion without gate results"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("Completion interlock"),
            "Error should mention interlock, got: {}",
            err
        );
    }
    #[test]
    fn completion_interlock_rejects_partial_gate_pass() {
        let mut s = TaskState::new("t-ci2");
        apply(
            &mut s,
            &ev(
                1,
                "t-ci2",
                "task_created",
                task_payload(
                    "t",
                    &["src/"],
                    &["src/"],
                    &["cargo_fmt_check", "cargo_test"],
                ),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-ci2", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-ci2", "task_started", json!({}))).unwrap();
        apply(
            &mut s,
            &ev(4, "t-ci2", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        // Only one gate checked and passed
        apply(
            &mut s,
            &ev(
                5,
                "t-ci2",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":true,"evidence":"ok","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        )
        .unwrap();
        let result = apply(&mut s, &ev(6, "t-ci2", "task_completed", json!({})));
        assert!(
            result.is_err(),
            "Must reject when not all gates have results"
        );
    }
    #[test]
    fn completion_interlock_rejects_failed_gate() {
        let mut s = TaskState::new("t-ci3");
        apply(
            &mut s,
            &ev(
                1,
                "t-ci3",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_fmt_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-ci3", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-ci3", "task_started", json!({}))).unwrap();
        apply(
            &mut s,
            &ev(4, "t-ci3", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        // Gate checked but failed
        apply(
            &mut s,
            &ev(
                5,
                "t-ci3",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":false,"evidence":"errors found","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        )
        .unwrap();
        let result = apply(&mut s, &ev(6, "t-ci3", "task_completed", json!({})));
        assert!(result.is_err(), "Must reject when gate failed");
    }
    #[test]
    fn completion_interlock_allows_when_all_gates_pass() {
        let mut s = TaskState::new("t-ci4");
        apply(
            &mut s,
            &ev(
                1,
                "t-ci4",
                "task_created",
                task_payload(
                    "t",
                    &["src/"],
                    &["src/"],
                    &["cargo_fmt_check", "cargo_test"],
                ),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-ci4", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-ci4", "task_started", json!({}))).unwrap();
        apply(
            &mut s,
            &ev(4, "t-ci4", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        apply(
            &mut s,
            &ev(
                5,
                "t-ci4",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":true,"evidence":"clean","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        )
        .unwrap();
        apply(
            &mut s,
            &ev(
                6,
                "t-ci4",
                "gate_checked",
                json!({"gate_id":"cargo_test","passed":true,"evidence":"56 passed","checked_at":"2026-05-30T12:01:00Z"}),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(7, "t-ci4", "task_completed", json!({}))).unwrap();
        assert_eq!(s.phase, Phase::Completed);
    }
    #[test]
    fn gate_checked_retains_latest_result() {
        let mut s = TaskState::new("t-gr");
        apply(
            &mut s,
            &ev(
                1,
                "t-gr",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_fmt_check"]),
            ),
        )
        .unwrap();
        // First check: failed
        apply(
            &mut s,
            &ev(
                2,
                "t-gr",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":false,"evidence":"errors","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        )
        .unwrap();
        assert!(!s.gate_results.get("cargo_fmt_check").unwrap().passed);
        // Second check: passed — overwrites
        apply(
            &mut s,
            &ev(
                3,
                "t-gr",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":true,"evidence":"clean","checked_at":"2026-05-30T12:01:00Z"}),
            ),
        )
        .unwrap();
        assert!(s.gate_results.get("cargo_fmt_check").unwrap().passed);
        assert_eq!(
            s.gate_results.get("cargo_fmt_check").unwrap().evidence,
            "clean"
        );
    }
    #[test]
    fn cancel_from_planning_succeeds() {
        let mut s = TaskState::new("t-canp");
        apply(
            &mut s,
            &ev(
                1,
                "t-canp",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-canp", "task_cancelled", json!({}))).unwrap();
        assert_eq!(s.phase, Phase::Cancelled);
    }
    #[test]
    fn gate_checked_allowed_during_hold() {
        // Per ARCHITECTURE_GUARDRAILS.md line 186: audit_hold allows deterministic offline gates
        let mut s = TaskState::new("t-gch");
        apply(
            &mut s,
            &ev(
                1,
                "t-gch",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_fmt_check"]),
            ),
        )
        .unwrap();
        apply(&mut s, &ev(2, "t-gch", "task_marked_ready", json!({}))).unwrap();
        apply(&mut s, &ev(3, "t-gch", "task_started", json!({}))).unwrap();
        apply(&mut s, &ev(4, "t-gch", "hold_entered", json!({}))).unwrap();
        assert!(s.is_held);
        // gate_checked must succeed during hold
        apply(
            &mut s,
            &ev(
                5,
                "t-gch",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":true,"evidence":"clean","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        )
        .unwrap();
        assert!(s.gate_results.get("cargo_fmt_check").unwrap().passed);
    }
    #[test]
    fn gate_checked_rejects_empty_gate_id() {
        let mut s = TaskState::new("t-egid");
        apply(
            &mut s,
            &ev(
                1,
                "t-egid",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_fmt_check"]),
            ),
        )
        .unwrap();
        let result = apply(
            &mut s,
            &ev(
                2,
                "t-egid",
                "gate_checked",
                json!({"gate_id":"","passed":true,"evidence":"ok","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        );
        assert!(result.is_err(), "Must reject empty gate_id");
    }
    #[test]
    fn gate_checked_rejects_unknown_gate_id() {
        let mut s = TaskState::new("t-ugid");
        apply(
            &mut s,
            &ev(
                1,
                "t-ugid",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_fmt_check"]),
            ),
        )
        .unwrap();
        let result = apply(
            &mut s,
            &ev(
                2,
                "t-ugid",
                "gate_checked",
                json!({"gate_id":"nonexistent","passed":true,"evidence":"ok","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        );
        assert!(result.is_err(), "Must reject gate_id not in task gates");
    }
    #[test]
    fn gate_checked_rejects_empty_evidence() {
        let mut s = TaskState::new("t-eev");
        apply(
            &mut s,
            &ev(
                1,
                "t-eev",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_fmt_check"]),
            ),
        )
        .unwrap();
        let result = apply(
            &mut s,
            &ev(
                2,
                "t-eev",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":true,"evidence":"","checked_at":"2026-05-30T12:00:00Z"}),
            ),
        );
        assert!(result.is_err(), "Must reject empty evidence");
    }
    #[test]
    fn gate_checked_rejects_empty_checked_at() {
        let mut s = TaskState::new("t-eca");
        apply(
            &mut s,
            &ev(
                1,
                "t-eca",
                "task_created",
                task_payload("t", &["src/"], &["src/"], &["cargo_fmt_check"]),
            ),
        )
        .unwrap();
        let result = apply(
            &mut s,
            &ev(
                2,
                "t-eca",
                "gate_checked",
                json!({"gate_id":"cargo_fmt_check","passed":true,"evidence":"ok","checked_at":""}),
            ),
        );
        assert!(result.is_err(), "Must reject empty checked_at");
    }
    // ============================================================
    // Baseline manifest (AUDIT-005: fixed-set regression)
    // ============================================================
    /// Audit matrix version — bump when test structure changes.
    const AUDIT_MATRIX_VERSION: u32 = 6;
    const BASELINE_SCHEMA_FILES: &[&str] = &[
        "control.event-envelope.v1.schema.json",
        "control.task-definition.v1.schema.json",
        "control.task-view.v1.schema.json",
        "control.policy-decision.v1.schema.json",
    ];
    const BASELINE_FIXTURE_FILES: &[&str] = &[
        "invalid.json",
        "reducer_boundary_violation.jsonl",
        "reducer_hold.jsonl",
        "reducer_lifecycle.jsonl",
        "reducer_m2_lifecycle.jsonl",
        "reducer_m3_lifecycle.jsonl",
        "reducer_m4_lifecycle.jsonl",
        "reducer_revise.jsonl",
        "reducer_test.jsonl",
        "schema_counter_examples.json",
    ];
    const BASELINE_REQUIRED_GATES: &[&str] = &[
        "cargo_fmt_check",
        "cargo_check",
        "cargo_test",
        "cargo_clippy",
    ];
    #[test]
    fn baseline_audit_matrix_version() {
        assert_eq!(
            AUDIT_MATRIX_VERSION, 6,
            "Audit matrix version must be explicitly bumped on structural changes"
        );
    }
    #[test]
    fn baseline_schema_exact_set() {
        let mut found: Vec<String> = Vec::new();
        for entry in fs::read_dir("schemas/").unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            if name.ends_with(".schema.json") {
                found.push(name);
            }
        }
        found.sort();
        let mut expected: Vec<&str> = BASELINE_SCHEMA_FILES.to_vec();
        expected.sort();
        assert_eq!(
            found, expected,
            "Schema file set must match baseline exactly"
        );
    }
    #[test]
    fn baseline_fixture_exact_set() {
        let mut found: Vec<String> = Vec::new();
        for entry in fs::read_dir("fixtures/").unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            if name.ends_with(".jsonl") || name.ends_with(".json") {
                found.push(name);
            }
        }
        found.sort();
        let mut expected: Vec<&str> = BASELINE_FIXTURE_FILES.to_vec();
        expected.sort();
        assert_eq!(
            found, expected,
            "Fixture file set must match baseline exactly"
        );
    }
    #[test]
    fn baseline_required_gates_pinned() {
        // Ensures the required gate set is explicitly declared.
        // Adding or removing gates is a baseline regression until this const is updated.
        assert!(
            !BASELINE_REQUIRED_GATES.is_empty(),
            "Required gates must not be empty"
        );
        assert_eq!(
            BASELINE_REQUIRED_GATES.len(),
            4,
            "Required gate count changed — update this test if intentional"
        );
    }

    // ============================================================
    // M3 evidence events
    // ============================================================

    #[test]
    fn evidence_accepted_in_progress() {
        let mut state = TaskState::new("t1");
        apply(
            &mut state,
            &ev(
                1,
                "t1",
                "task_created",
                task_payload("T", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut state, &ev(2, "t1", "task_marked_ready", json!({}))).unwrap();
        apply(&mut state, &ev(3, "t1", "task_started", json!({}))).unwrap();
        apply(
            &mut state,
            &ev(
                4,
                "t1",
                "evidence_accepted",
                json!({
                    "evidence_id": "ev-1",
                    "source": "manual",
                }),
            ),
        )
        .unwrap();
        assert_eq!(state.phase, Phase::InProgress);
    }

    #[test]
    fn evidence_accepted_in_review() {
        let mut state = TaskState::new("t1");
        apply(
            &mut state,
            &ev(
                1,
                "t1",
                "task_created",
                task_payload("T", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut state, &ev(2, "t1", "task_marked_ready", json!({}))).unwrap();
        apply(&mut state, &ev(3, "t1", "task_started", json!({}))).unwrap();
        apply(
            &mut state,
            &ev(4, "t1", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        apply(
            &mut state,
            &ev(
                5,
                "t1",
                "evidence_accepted",
                json!({
                    "evidence_id": "ev-1",
                    "source": "manual",
                }),
            ),
        )
        .unwrap();
        assert_eq!(state.phase, Phase::Review);
    }

    #[test]
    fn evidence_rejected_in_completed() {
        let mut state = TaskState::new("t1");
        apply(
            &mut state,
            &ev(
                1,
                "t1",
                "task_created",
                task_payload("T", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut state, &ev(2, "t1", "task_marked_ready", json!({}))).unwrap();
        apply(&mut state, &ev(3, "t1", "task_started", json!({}))).unwrap();
        apply(&mut state, &ev(4, "t1", "gate_checked", json!({"gate_id":"cargo_check","passed":true,"evidence":"ok","checked_at":"2026-06-06T10:00:00Z"}))).unwrap();
        apply(
            &mut state,
            &ev(5, "t1", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        apply(&mut state, &ev(6, "t1", "task_completed", json!({}))).unwrap();
        let result = apply(
            &mut state,
            &ev(
                7,
                "t1",
                "evidence_accepted",
                json!({
                    "evidence_id": "ev-1",
                    "source": "manual",
                }),
            ),
        );
        assert!(result.is_err());
    }

    #[test]
    fn evidence_accepted_rejects_empty_evidence_id() {
        let mut state = TaskState::new("t1");
        apply(
            &mut state,
            &ev(
                1,
                "t1",
                "task_created",
                task_payload("T", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut state, &ev(2, "t1", "task_marked_ready", json!({}))).unwrap();
        apply(&mut state, &ev(3, "t1", "task_started", json!({}))).unwrap();
        let result = apply(
            &mut state,
            &ev(
                4,
                "t1",
                "evidence_accepted",
                json!({
                    "evidence_id": "",
                    "source": "manual",
                }),
            ),
        );
        assert!(result.is_err());
    }

    #[test]
    fn evidence_accepted_rejects_empty_source() {
        let mut state = TaskState::new("t1");
        apply(
            &mut state,
            &ev(
                1,
                "t1",
                "task_created",
                task_payload("T", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut state, &ev(2, "t1", "task_marked_ready", json!({}))).unwrap();
        apply(&mut state, &ev(3, "t1", "task_started", json!({}))).unwrap();
        let result = apply(
            &mut state,
            &ev(
                4,
                "t1",
                "evidence_accepted",
                json!({
                    "evidence_id": "ev-1",
                    "source": "",
                }),
            ),
        );
        assert!(result.is_err());
    }

    #[test]
    fn evidence_rejected_records_even_for_completed() {
        let mut state = TaskState::new("t1");
        apply(
            &mut state,
            &ev(
                1,
                "t1",
                "task_created",
                task_payload("T", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        apply(&mut state, &ev(2, "t1", "task_marked_ready", json!({}))).unwrap();
        apply(&mut state, &ev(3, "t1", "task_started", json!({}))).unwrap();
        apply(&mut state, &ev(4, "t1", "gate_checked", json!({"gate_id":"cargo_check","passed":true,"evidence":"ok","checked_at":"2026-06-06T10:00:00Z"}))).unwrap();
        apply(
            &mut state,
            &ev(5, "t1", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        apply(&mut state, &ev(6, "t1", "task_completed", json!({}))).unwrap();
        // evidence_rejected should succeed even on completed tasks (it's a record, not a state change)
        apply(
            &mut state,
            &ev(
                7,
                "t1",
                "evidence_rejected",
                json!({
                    "evidence_id": "ev-1",
                }),
            ),
        )
        .unwrap();
    }

    #[test]
    fn evidence_rejected_rejects_empty_evidence_id() {
        let mut state = TaskState::new("t1");
        apply(
            &mut state,
            &ev(
                1,
                "t1",
                "task_created",
                task_payload("T", &["src/"], &["src/"], &["cargo_check"]),
            ),
        )
        .unwrap();
        let result = apply(
            &mut state,
            &ev(
                2,
                "t1",
                "evidence_rejected",
                json!({
                    "evidence_id": "",
                }),
            ),
        );
        assert!(result.is_err());
    }

    #[test]
    fn m3_full_lifecycle_with_evidence() {
        let mut state = TaskState::new("t-m3");
        apply(
            &mut state,
            &ev(
                1,
                "t-m3",
                "task_created",
                task_payload(
                    "M3 lifecycle",
                    &["src/"],
                    &["src/"],
                    &["cargo_check", "cargo_test"],
                ),
            ),
        )
        .unwrap();
        apply(&mut state, &ev(2, "t-m3", "task_marked_ready", json!({}))).unwrap();
        apply(&mut state, &ev(3, "t-m3", "task_started", json!({}))).unwrap();
        // Accept evidence during implementation
        apply(
            &mut state,
            &ev(
                4,
                "t-m3",
                "evidence_accepted",
                json!({
                    "evidence_id": "ev-m3-1",
                    "source": "manual",
                    "touched_files": ["src/main.rs"],
                }),
            ),
        )
        .unwrap();
        // Run gates
        apply(
            &mut state,
            &ev(
                5,
                "t-m3",
                "gate_checked",
                json!({
                    "gate_id": "cargo_check",
                    "passed": true,
                    "evidence": "exit=0",
                    "checked_at": "2026-06-06T10:06:00Z"
                }),
            ),
        )
        .unwrap();
        apply(
            &mut state,
            &ev(
                6,
                "t-m3",
                "gate_checked",
                json!({
                    "gate_id": "cargo_test",
                    "passed": true,
                    "evidence": "exit=0",
                    "checked_at": "2026-06-06T10:07:00Z"
                }),
            ),
        )
        .unwrap();
        // Submit
        apply(
            &mut state,
            &ev(7, "t-m3", "task_submitted_for_review", json!({})),
        )
        .unwrap();
        assert_eq!(state.phase, Phase::Review);
        // Complete
        apply(&mut state, &ev(8, "t-m3", "task_completed", json!({}))).unwrap();
        assert_eq!(state.phase, Phase::Completed);
        // Archive
        apply(&mut state, &ev(9, "t-m3", "task_archived", json!({}))).unwrap();
        assert!(state.is_archived);
    }

    // ============================================================
    // Helpers
    // ============================================================

    fn ev(seq: i64, task_id: &str, event_type: &str, payload: serde_json::Value) -> Event {
        Event {
            schema: "control.event-envelope.v1".into(),
            event_id: format!("e-{}-{}", task_id, seq),
            command_id: format!("c-{}-{}", task_id, seq),
            task_id: task_id.into(),
            seq,
            occurred_at: "2026-05-30T10:00:00Z".into(),
            actor: "human".into(),
            event_type: event_type.into(),
            payload,
        }
    }

    fn task_payload(
        objective: &str,
        read_scope: &[&str],
        write_allow: &[&str],
        gates: &[&str],
    ) -> serde_json::Value {
        json!({
            "objective": objective,
            "read_scope": read_scope,
            "write_allow": write_allow,
            "write_deny": [],
            "risk_triggers": [],
            "gates": gates
        })
    }

    fn replay(task_id: &str, path: &str) -> TaskState {
        let content = fs::read_to_string(path).unwrap();
        let mut state = TaskState::new(task_id);
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let event: Event = serde_json::from_str(line).unwrap();
            apply(&mut state, &event).unwrap();
        }
        state
    }
}
