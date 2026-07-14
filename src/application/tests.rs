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
            depends_on: &[],
        },
    )
    .unwrap();

    assert!(dir
        .path()
        .join(".ctl/tasks/ledger-task/events.jsonl")
        .exists());
    assert!(dir.path().join(".ctl/tasks/ledger-task/task.json").exists());
    assert!(!dir.path().join(".control").join("events.jsonl").exists());
}
#[test]
fn create_task_accepts_protected_path_in_write_allow() {
    // converge-protected-proposal: a protected path may be DECLARED in
    // write_allow at create time. Protection is enforced once, at the
    // runtime gate (deny unless a `ctl apply` exception is granted) — not
    // duplicated as a create-time reject. This unblocks governed tasks that
    // legitimately touch Cargo.toml / schemas / the ledgers.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let write_allow = vec!["src".to_string(), "Cargo.toml".to_string()];
    app.create_task(
        "protected-scope",
        CreateTaskInput {
            objective: "touch a protected manifest",
            read_scope: &["src".to_string()],
            write_allow: &write_allow,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();

    let state = app.replay_task("protected-scope").unwrap();
    let expected: std::collections::BTreeSet<String> = write_allow.iter().cloned().collect();
    assert_eq!(state.write_allow, expected);
}

#[test]
fn revise_task_accepts_protected_path_in_write_allow() {
    // revise_task runs in Planning; widening write_allow to a protected
    // path must also succeed for the same reason as create.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    app.create_task(
        "t",
        CreateTaskInput {
            objective: "x",
            read_scope: &["src".to_string()],
            write_allow: &["src".to_string()],
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
    let widened = vec!["src".to_string(), "Cargo.toml".to_string()];
    app.revise_task(
        "t",
        ReviseTaskInput {
            objective: None,
            read_scope: None,
            write_allow: Some(&widened),
            write_deny: None,
            risk_triggers: None,
            gates: None,
            depends_on: None,
        },
    )
    .unwrap();
    let state = app.replay_task("t").unwrap();
    let expected: std::collections::BTreeSet<String> = widened.iter().cloned().collect();
    assert_eq!(state.write_allow, expected);
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs")
        .status
        .success();
    assert!(ok, "git {:?} failed", args);
}

/// Drive a task to Review with a passing gate but NO completion audit.
fn drive_to_review_bare(app: &ControlApp, id: &str) {
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "interlock test",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
    app.start_task(id).unwrap();
    app.submit_task(id).unwrap();
    app.record_gate(id, "cargo_check", true, "ok").unwrap();
}

/// Record a passing completion audit as a non-implementer reviewer (M6):
/// the implementer (`task_started` actor) is the default "human", so the
/// reviewer acts under a distinct identity.
fn audit_pass(app: &ControlApp, id: &str, note: Option<&str>) {
    ControlApp::open(&app.project_root, false)
        .unwrap()
        .with_actor("reviewer")
        .record_completion_audit(id, true, note)
        .unwrap();
}

fn drive_to_review(app: &ControlApp, id: &str) {
    drive_to_review_bare(app, id);
    // M-f: a fresh passing completion audit is now a finish prerequisite;
    // M6: it must come from a non-implementer reviewer.
    audit_pass(app, id, None);
}

// ── TDD red→green interlock (ctl-tdd-loop-v1) ──

/// Drive a TDD-opted task (cargo_test gate + `tdd-red-green` trigger) to a
/// finishable Review state, optionally recording a RED cargo_test before the
/// green one. Non-git temp dir → tree/commit interlocks are skipped, isolating
/// the TDD check.
fn drive_tdd_to_review(app: &ControlApp, id: &str, record_red: bool) {
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_test".to_string()];
    let triggers = vec![TDD_RED_GREEN_TRIGGER.to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "tdd",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &triggers,
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
    app.start_task(id).unwrap();
    if record_red {
        app.record_gate(id, "cargo_test", false, "red: test fails, impl absent")
            .unwrap();
    }
    app.submit_task(id).unwrap();
    app.record_gate(id, "cargo_test", true, "green: impl done")
        .unwrap();
    audit_pass(app, id, None);
}

#[test]
fn tdd_interlock_blocks_when_test_only_passed() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_tdd_to_review(&app, "tdd-nored", false); // green only, never red
    let err = app.finish_task("tdd-nored").unwrap_err().to_string();
    assert!(err.contains("tdd-red-green"), "got: {err}");
    assert!(err.contains("red→green"), "got: {err}");
}

#[test]
fn tdd_interlock_allows_with_red_before_green() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_tdd_to_review(&app, "tdd-ok", true); // red, then green
    let ev = app.finish_task("tdd-ok").unwrap();
    assert_eq!(ev.event_type, "task_completed");
}

#[test]
fn tdd_interlock_inactive_without_trigger() {
    // A normal task (no trigger) finishes with only a green gate.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review(&app, "normal");
    let ev = app.finish_task("normal").unwrap();
    assert_eq!(ev.event_type, "task_completed");
}

#[test]
fn tdd_interlock_requires_a_test_gate() {
    // Opted into TDD but no cargo_test gate → clear misconfiguration block.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()]; // no cargo_test
    let triggers = vec![TDD_RED_GREEN_TRIGGER.to_string()];
    app.create_task(
        "tdd-misconf",
        CreateTaskInput {
            objective: "x",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &triggers,
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready("tdd-misconf").unwrap();
    app.start_task("tdd-misconf").unwrap();
    app.submit_task("tdd-misconf").unwrap();
    app.record_gate("tdd-misconf", "cargo_check", true, "ok")
        .unwrap();
    audit_pass(&app, "tdd-misconf", None);
    let err = app.finish_task("tdd-misconf").unwrap_err().to_string();
    assert!(err.contains("no 'cargo_test' gate"), "got: {err}");
}

#[test]
fn gate_red_before_green_helper() {
    let mk = |seq: i64, passed: bool| Event {
        schema: "control.event-envelope.v1".to_string(),
        event_id: format!("e{seq}"),
        command_id: format!("c{seq}"),
        task_id: "t".to_string(),
        seq,
        occurred_at: "2026-01-01T00:00:00Z".to_string(),
        actor: "t".to_string(),
        event_type: "gate_checked".to_string(),
        payload: serde_json::json!({"gate_id": "cargo_test", "passed": passed}),
    };
    // pass-only → false
    assert!(!gate_went_red_before_green(&[mk(1, true)], "cargo_test"));
    // fail then pass → true
    assert!(gate_went_red_before_green(
        &[mk(1, false), mk(2, true)],
        "cargo_test"
    ));
    // pass then fail (no later pass) → false
    assert!(!gate_went_red_before_green(
        &[mk(1, true), mk(2, false)],
        "cargo_test"
    ));
    // different gate id → false
    assert!(!gate_went_red_before_green(
        &[mk(1, false), mk(2, true)],
        "cargo_check"
    ));
}

#[test]
fn finish_blocked_without_completion_audit() {
    // No git repo → M-g commit interlock is skipped, isolating the M-f gate.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "noaudit");
    let err = app.finish_task("noaudit").unwrap_err().to_string();
    assert!(
        err.contains("no passing completion audit"),
        "expected M-f review gate, got: {err}"
    );
}

#[test]
fn finish_allowed_with_fresh_completion_audit() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "audited");
    audit_pass(&app, "audited", Some("looks good"));
    let event = app.finish_task("audited").unwrap();
    assert_eq!(event.event_type, "task_completed");
}

#[test]
fn finish_blocked_by_failing_completion_audit() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "failed");
    app.record_completion_audit("failed", false, Some("missing tests"))
        .unwrap();
    let err = app.finish_task("failed").unwrap_err().to_string();
    assert!(
        err.contains("latest completion audit is a FAIL"),
        "expected fail-verdict block, got: {err}"
    );
}

#[test]
fn audit_before_resubmit_is_stale_and_does_not_count() {
    // A pass from a PRIOR review round must not satisfy finish after rework
    // (reopen → resubmit). Freshness is keyed on the last submit's seq.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "rework");
    audit_pass(&app, "rework", None);
    // Rework: back to in_progress, then re-submit. The earlier audit is now
    // before the latest submit and no longer counts.
    app.reopen_task("rework").unwrap();
    app.submit_task("rework").unwrap();
    app.record_gate("rework", "cargo_check", true, "ok")
        .unwrap();
    let err = app.finish_task("rework").unwrap_err().to_string();
    assert!(
        err.contains("no passing completion audit"),
        "stale pre-rework audit must not satisfy finish, got: {err}"
    );
    // A fresh audit after the new submit unblocks it.
    audit_pass(&app, "rework", None);
    assert_eq!(
        app.finish_task("rework").unwrap().event_type,
        "task_completed"
    );
}

#[test]
fn completion_audit_requires_review_phase() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        "early",
        CreateTaskInput {
            objective: "x",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready("early").unwrap();
    app.start_task("early").unwrap();
    // Still in_progress (not submitted) → audit must be rejected.
    let err = app
        .record_completion_audit("early", true, None)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("only be recorded in Review"),
        "expected phase guard, got: {err}"
    );
}

#[test]
fn implementer_cannot_self_approve_completion_audit() {
    // M6: the actor who started/implemented the task may not record its own
    // passing audit. `app` (default actor "human") started the task.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "selfapp");
    let err = app
        .record_completion_audit("selfapp", true, None)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Reviewer-lease binding") && err.contains("human"),
        "implementer self-approval must be blocked, got: {err}"
    );
    // A distinct reviewer can accept it.
    audit_pass(&app, "selfapp", None);
    assert_eq!(
        app.finish_task("selfapp").unwrap().event_type,
        "task_completed"
    );
}

#[test]
fn implementer_may_self_reject_completion_audit() {
    // A FAIL from the implementer (self-flagging a problem) is allowed —
    // only self-approval is the threat.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "selfrej");
    let ev = app
        .record_completion_audit("selfrej", false, Some("found a bug myself"))
        .unwrap();
    assert_eq!(ev.event_type, "evidence_rejected");
}

#[test]
fn event_actor_comes_from_with_actor_override() {
    // M6 foundation: events are stamped with the instance actor, not a
    // hardcoded "human".
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap().with_actor("agent-7");
    let ev = app
        .create_task(
            "act",
            CreateTaskInput {
                objective: "x",
                read_scope: &["src".to_string()],
                write_allow: &["src".to_string()],
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
    assert_eq!(ev.actor, "agent-7");
}

#[test]
fn finish_blocked_by_uncommitted_work_in_scope() {
    let dir = TempDir::new();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);

    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review(&app, "ilk");

    // Uncommitted file inside write scope (src/) → finish must fail closed.
    std::fs::write(dir.path().join("src/work.rs"), "fn w() {}\n").unwrap();
    let err = app.finish_task("ilk").unwrap_err().to_string();
    assert!(
        err.contains("uncommitted changes"),
        "expected commit interlock, got: {err}"
    );

    // Commit the work → tree clean in scope. The gate/audit recorded earlier
    // are now bound to a different (or no) tree, so finish stays closed until
    // they are re-validated against the current committed tree (artifact binding).
    git(dir.path(), &["add", "src/work.rs"]);
    git(dir.path(), &["commit", "-qm", "work"]);
    let stale = app.finish_task("ilk").unwrap_err().to_string();
    assert!(
        stale.contains("completion evidence is stale"),
        "expected artifact binding to block stale evidence, got: {stale}"
    );

    // Re-gate + re-audit on the current tree → finish succeeds.
    app.record_gate("ilk", "cargo_check", true, "ok").unwrap();
    audit_pass(&app, "ilk", None);
    let event = app.finish_task("ilk").unwrap();
    assert_eq!(event.event_type, "task_completed");
}

// ── Artifact binding (tree_hash) interlock ──

/// git repo with one initial commit, so `HEAD^{tree}` exists.
fn git_init_committed(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "init"]);
}

/// Commit a new src/ file so the committed tree advances.
fn commit_src(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join("src").join(name), body).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "change"]);
}

/// create + ready + start + submit a git task scoped to src/ with `gates`.
fn git_task_to_review(app: &ControlApp, id: &str, gates: &[String]) {
    let scope = vec!["src".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "tree binding test",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
    app.start_task(id).unwrap();
    app.submit_task(id).unwrap();
}

// Case 1: gate + audit on the same committed tree → finish succeeds.
#[test]
fn artifact_binding_same_tree_finishes() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(&app, "t", &["cargo_check".to_string()]);
    app.record_gate("t", "cargo_check", true, "ok").unwrap();
    audit_pass(&app, "t", None);
    assert_eq!(app.finish_task("t").unwrap().event_type, "task_completed");
}

// Case 2: gate recorded, then a new commit → gate is stale → finish fails.
#[test]
fn artifact_binding_gate_then_commit_is_stale() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(&app, "t", &["cargo_check".to_string()]);
    app.record_gate("t", "cargo_check", true, "ok").unwrap();
    commit_src(dir.path(), "b.rs", "fn b() {}\n");
    audit_pass(&app, "t", None);
    let err = app.finish_task("t").unwrap_err().to_string();
    assert!(err.contains("completion evidence is stale"), "{err}");
}

// Case 3: audit recorded, then a new commit → evidence is stale → finish fails.
#[test]
fn artifact_binding_audit_then_commit_is_stale() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(&app, "t", &["cargo_check".to_string()]);
    app.record_gate("t", "cargo_check", true, "ok").unwrap();
    audit_pass(&app, "t", None);
    commit_src(dir.path(), "b.rs", "fn b() {}\n");
    let err = app.finish_task("t").unwrap_err().to_string();
    assert!(err.contains("completion evidence is stale"), "{err}");
}

// Case 4: gate re-run on the new tree but audit NOT re-run → audit stale → fail.
#[test]
fn artifact_binding_regate_without_reaudit_is_stale() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(&app, "t", &["cargo_check".to_string()]);
    app.record_gate("t", "cargo_check", true, "ok").unwrap();
    audit_pass(&app, "t", None);
    commit_src(dir.path(), "b.rs", "fn b() {}\n");
    app.record_gate("t", "cargo_check", true, "ok").unwrap(); // gate now fresh
    let err = app.finish_task("t").unwrap_err().to_string();
    assert!(
        err.contains("completion audit is stale"),
        "expected audit-stale, got: {err}"
    );
}

// Case 5: audit re-run on the new tree but a required gate still on old tree → fail.
#[test]
fn artifact_binding_reaudit_with_stale_gate_fails() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(&app, "t", &["cargo_check".to_string()]);
    app.record_gate("t", "cargo_check", true, "ok").unwrap();
    audit_pass(&app, "t", None);
    commit_src(dir.path(), "b.rs", "fn b() {}\n");
    audit_pass(&app, "t", None); // audit now fresh, gate still old
    let err = app.finish_task("t").unwrap_err().to_string();
    assert!(
        err.contains("completion evidence is stale") && err.contains("tree-stale"),
        "expected gate-stale, got: {err}"
    );
}

// Case 6: a legacy gate_checked without tree_hash replays fine but cannot finish.
#[test]
fn artifact_binding_legacy_unbound_gate_replays_but_blocks_finish() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(&app, "t", &["cargo_check".to_string()]);
    // Pre-binding event: no tree_hash. Schema + reducer must accept it (replay ok).
    let ev = app
        .build_event(
            "t",
            "gate_checked",
            serde_json::json!({
                "gate_id": "cargo_check",
                "passed": true,
                "evidence": "ok",
                "checked_at": "2026-01-01T00:00:00Z"
            }),
        )
        .unwrap();
    app.validate_and_append(&ev).unwrap(); // proves replay tolerates missing tree_hash
    audit_pass(&app, "t", None);
    let err = app.finish_task("t").unwrap_err().to_string();
    assert!(err.contains("completion evidence is stale"), "{err}");
}

// Case 7: a FAILED gate on the current tree still cannot finish (passing check first).
#[test]
fn artifact_binding_failed_gate_same_tree_not_passing() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(&app, "t", &["cargo_check".to_string()]);
    app.record_gate("t", "cargo_check", false, "boom").unwrap();
    audit_pass(&app, "t", None);
    let err = app.finish_task("t").unwrap_err().to_string();
    assert!(err.contains("gates not passing"), "{err}");
}

// Case 8: with multiple required gates, one stale gate blocks finish.
#[test]
fn artifact_binding_one_of_many_gates_stale() {
    let dir = TempDir::new();
    git_init_committed(dir.path());
    let app = ControlApp::init(dir.path()).unwrap();
    git_task_to_review(
        &app,
        "t",
        &["cargo_check".to_string(), "cargo_fmt_check".to_string()],
    );
    app.record_gate("t", "cargo_check", true, "ok").unwrap();
    app.record_gate("t", "cargo_fmt_check", true, "ok").unwrap();
    commit_src(dir.path(), "b.rs", "fn b() {}\n");
    app.record_gate("t", "cargo_check", true, "ok").unwrap(); // only this one refreshed
    audit_pass(&app, "t", None);
    let err = app.finish_task("t").unwrap_err().to_string();
    assert!(
        err.contains("completion evidence is stale") && err.contains("cargo_fmt_check"),
        "expected cargo_fmt_check stale, got: {err}"
    );
}

// ── policy_hash interlock ──
//
// Hash *sensitivity* (widen/narrow scope, gate-arg change, gate-set change,
// canonicalization) is covered by unit tests in `domain::policy`. These tests
// cover the finish-time *interlock*: a policy-mismatched (or unbound) gate or
// audit must block completion. Policy cannot change mid-task via the public
// API, so staleness is simulated by recording evidence under a different
// policy hash — exactly what a gate-catalog change across ctl versions yields.

/// Overwrite cargo_check's latest result with an explicit/missing policy_hash.
fn append_gate_policy(app: &ControlApp, id: &str, policy_hash: Option<&str>) {
    let mut p = serde_json::json!({
        "gate_id": "cargo_check", "passed": true,
        "evidence": "ok", "checked_at": "2026-01-01T00:00:00Z"
    });
    if let Some(ph) = policy_hash {
        p["policy_hash"] = serde_json::json!(ph);
    }
    let ev = app.build_event(id, "gate_checked", p).unwrap();
    app.validate_and_append(&ev).unwrap();
}

/// Append a completion audit with an explicit policy_hash, as a non-implementer.
fn append_audit_policy(app: &ControlApp, id: &str, policy_hash: Option<&str>) {
    let reviewer = ControlApp::open(&app.project_root, false)
        .unwrap()
        .with_actor("reviewer");
    let mut p = serde_json::json!({
        "evidence_id": "aud-x", "source": COMPLETION_AUDIT_SOURCE,
        "touched_files": [], "result_file": "", "accepted_at": "2026-01-01T00:00:00Z"
    });
    if let Some(ph) = policy_hash {
        p["policy_hash"] = serde_json::json!(ph);
    }
    let ev = reviewer.build_event(id, "evidence_accepted", p).unwrap();
    reviewer.validate_and_append(&ev).unwrap();
}

// Same policy (non-git) → finish succeeds.
#[test]
fn policy_binding_same_policy_finishes() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "p");
    audit_pass(&app, "p", None);
    assert_eq!(app.finish_task("p").unwrap().event_type, "task_completed");
}

// A gate produced under a different policy → finish fails (policy-stale).
#[test]
fn policy_binding_stale_gate_policy_fails() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "p");
    append_gate_policy(&app, "p", Some("stale-policy-hash"));
    audit_pass(&app, "p", None);
    let err = app.finish_task("p").unwrap_err().to_string();
    assert!(
        err.contains("completion evidence is stale") && err.contains("policy-stale"),
        "{err}"
    );
}

// An audit accepted under a different policy → finish fails.
#[test]
fn policy_binding_stale_audit_policy_fails() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "p");
    append_audit_policy(&app, "p", Some("stale-policy-hash"));
    let err = app.finish_task("p").unwrap_err().to_string();
    assert!(
        err.contains("completion audit is stale") && err.contains("policy"),
        "{err}"
    );
}

// A legacy gate without policy_hash replays but cannot satisfy a new finish.
#[test]
fn policy_binding_legacy_unbound_gate_blocks_finish() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "p");
    append_gate_policy(&app, "p", None); // replay tolerates missing policy_hash
    audit_pass(&app, "p", None);
    let err = app.finish_task("p").unwrap_err().to_string();
    assert!(err.contains("completion evidence is stale"), "{err}");
}

// ── single-writer ledger ──

// Many concurrent writers on one task must never produce a duplicate or
// non-monotonic sequence number; the per-task lock serializes the
// read-seq → validate → append critical section. Losers of a race fail safe
// (reducer "Sequence error") rather than corrupting the ledger.
#[test]
fn concurrent_writers_never_duplicate_seq() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review_bare(&app, "c"); // task in Review with a cargo_check gate
    let root = app.project_root.clone();

    let mut handles = Vec::new();
    for _ in 0..6 {
        let r = root.clone();
        handles.push(std::thread::spawn(move || {
            // Each thread is an independent "process" view of the ledger.
            let a = ControlApp::open(&r, false).unwrap();
            let _ = a.record_gate("c", "cargo_check", true, "ok");
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let app2 = ControlApp::open(dir.path(), false).unwrap();
    let seqs: Vec<i64> = app2
        .store
        .read_for_task("c")
        .unwrap()
        .iter()
        .map(|e| e.seq)
        .collect();
    let mut uniq = seqs.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(
        seqs.len(),
        uniq.len(),
        "duplicate sequence under concurrency: {seqs:?}"
    );
    for w in seqs.windows(2) {
        assert!(w[1] > w[0], "non-monotonic sequence: {seqs:?}");
    }
}

// ── M6: merge-candidate ──

/// git repo + initial commit (tracked src/lib.rs and docs/readme.md), then
/// a started task with an isolated worktree. Returns (app, worktree_path).
fn setup_worktree(dir: &Path, id: &str, write_allow: &[&str]) -> (ControlApp, PathBuf) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.join("docs/readme.md"), "x\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "init"]);

    let app = ControlApp::init(dir).unwrap();
    let scope: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "merge candidate",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
    app.start_task(id).unwrap();
    app.workspace_create(id).unwrap();
    let wt = dir.join(".ctl/tasks").join(id).join("worktree");
    (app, wt)
}

/// Conformance (cross-adapter): ingesting an in-scope result tags the
/// accepted evidence with the *adapter's own* source — for every supported
/// adapter, driven off the registry so a new adapter is covered for free.
#[test]
fn ingest_tags_evidence_source_for_every_adapter() {
    for adapter in crate::adapters::supported_adapters() {
        let dir = TempDir::new();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@t"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "init"]);

        let app = ControlApp::init(dir.path()).unwrap();
        let scope = vec!["src".to_string()];
        app.create_task(
            "t",
            CreateTaskInput {
                objective: "ingest source",
                read_scope: &scope,
                write_allow: &scope,
                write_deny: &[],
                risk_triggers: &[],
                gates: &["cargo_check".to_string()],
                depends_on: &[],
            },
        )
        .unwrap();
        app.mark_ready("t").unwrap();
        app.start_task("t").unwrap();
        app.run_start("t", adapter).unwrap();

        let result_file = dir.path().join("agent-output.json");
        std::fs::write(
            &result_file,
            format!(r#"{{"source":"{adapter}","touched_files":["src/lib.rs"]}}"#),
        )
        .unwrap();
        let evidence = app.run_ingest("t", &result_file, adapter).unwrap();

        assert_eq!(
            evidence.event_type, "evidence_accepted",
            "{adapter}: in-scope ingest should be accepted"
        );
        assert_eq!(
            evidence.payload["source"], *adapter,
            "{adapter}: accepted evidence must be tagged source={adapter}"
        );
    }
}

#[test]
fn merge_candidate_in_scope_is_mergeable() {
    let dir = TempDir::new();
    let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
    // Modify a tracked, in-scope file in the worktree.
    std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();

    let v = app.merge_candidate("mc").unwrap();
    assert_eq!(v["mergeable"], true, "verdict: {v}");
    assert!(v["blocking_reasons"].as_array().unwrap().is_empty());
    assert!(v["touched_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "src/lib.rs"));
}

#[test]
fn merge_candidate_out_of_scope_blocks() {
    let dir = TempDir::new();
    // Task scope is only src/, but the worktree also edits docs/readme.md.
    let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
    std::fs::write(wt.join("docs/readme.md"), "edited\n").unwrap();

    let v = app.merge_candidate("mc").unwrap();
    assert_eq!(v["mergeable"], false, "verdict: {v}");
    assert!(v["out_of_scope"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "docs/readme.md"));
}

#[test]
fn merge_candidate_cross_task_conflict_blocks() {
    let dir = TempDir::new();
    let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
    std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();

    // Another active task claims the same file → cross-task collision.
    let other_scope = vec!["src/lib.rs".to_string()];
    app.create_task(
        "other",
        CreateTaskInput {
            objective: "rival",
            read_scope: &other_scope,
            write_allow: &other_scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready("other").unwrap();
    app.start_task("other").unwrap();

    let v = app.merge_candidate("mc").unwrap();
    assert_eq!(v["mergeable"], false, "verdict: {v}");
    let conflicts = v["cross_task_conflicts"].as_array().unwrap();
    assert!(conflicts
        .iter()
        .any(|c| c["conflicting_task"] == "other" && c["path"] == "src/lib.rs"));
}

#[test]
fn merge_candidate_dirty_main_workspace_blocks() {
    let dir = TempDir::new();
    let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
    std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();
    // The main workspace has its own uncommitted edit to the same file.
    std::fs::write(dir.path().join("src/lib.rs"), "fn a() { /* main */ }\n").unwrap();

    let v = app.merge_candidate("mc").unwrap();
    assert_eq!(v["mergeable"], false, "verdict: {v}");
    assert!(v["workspace_conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "src/lib.rs"));
}

#[test]
fn merge_candidate_emits_no_events() {
    let dir = TempDir::new();
    let (app, wt) = setup_worktree(dir.path(), "mc", &["src"]);
    std::fs::write(wt.join("src/lib.rs"), "fn a() { /* edit */ }\n").unwrap();
    let before = app.store.read_for_task("mc").unwrap().len();
    app.merge_candidate("mc").unwrap();
    assert_eq!(
        app.store.read_for_task("mc").unwrap().len(),
        before,
        "merge_candidate must be read-only"
    );
}

fn create_planning(app: &ControlApp, id: &str) {
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "board test",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
}

#[test]
fn board_aggregates_tasks_by_phase_and_activity() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // "a" → in_progress (active); "b" → stays planning (not active).
    create_planning(&app, "a");
    app.mark_ready("a").unwrap();
    app.start_task("a").unwrap();
    create_planning(&app, "b");

    let board = app.generate_board().unwrap();
    assert_eq!(board["totals"]["tasks"], 2);
    assert_eq!(board["totals"]["active"], 1);
    assert_eq!(board["totals"]["held"], 0);
    assert_eq!(board["totals"]["needs_work"], 0);

    let tasks = board["tasks"].as_array().unwrap();
    let a = tasks.iter().find(|t| t["task_id"] == "a").unwrap();
    assert_eq!(a["phase"], "in_progress");
    assert_eq!(a["active"], true);
    assert_eq!(a["review"], "none");
    let b = tasks.iter().find(|t| t["task_id"] == "b").unwrap();
    assert_eq!(b["active"], false);
}

#[test]
fn reconcile_projects_deterministic_control_json() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    create_planning(&app, "a");
    create_planning(&app, "b");

    app.reconcile().unwrap();
    let path = dir.path().join(".ctl/control.json");
    assert!(path.exists(), "reconcile must project control.json");
    let first = std::fs::read_to_string(&path).unwrap();

    app.reconcile().unwrap();
    let second = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        first, second,
        "control.json must be byte-identical on replay"
    );
}

/// Create + ready + start a simple in-scope task (no review).
fn start_simple(app: &ControlApp, id: &str) {
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "m5 test",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
    app.start_task(id).unwrap();
}

fn event_count(app: &ControlApp, id: &str) -> usize {
    app.store.read_for_task(id).unwrap().len()
}

#[test]
fn telemetry_add_writes_index_and_feeds_drift() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    start_simple(&app, "t");

    // Clean task → no drift.
    assert_eq!(app.compute_drift("t").unwrap().score, 0);

    let before = event_count(&app, "t");
    app.telemetry_add("t", "test_failures", 2, "human").unwrap();
    app.telemetry_add("t", "retries", 4, "human").unwrap();
    assert!(dir.path().join(".ctl/telemetry.jsonl").exists());

    // Telemetry is evidence, NOT a canonical event — the ledger is unchanged.
    assert_eq!(
        event_count(&app, "t"),
        before,
        "telemetry must not append events"
    );

    // 15 (test_failures) + 15 (retries>=3) = 30 = medium.
    let report = app.compute_drift("t").unwrap();
    assert_eq!(report.score, 30);
    assert_eq!(report.level.as_str(), "medium");
    assert_eq!(report.fired_ids(), vec!["DRIFT-004", "DRIFT-006"]);
}

#[test]
fn telemetry_add_dry_run_writes_nothing() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    start_simple(&app, "t");
    let dry = ControlApp::open(&app.project_root, true).unwrap();
    dry.telemetry_add("t", "test_failures", 1, "human").unwrap();
    assert!(!dir.path().join(".ctl/telemetry.jsonl").exists());
}

#[test]
fn unknown_signal_makes_next_action_ask_and_emits_no_events() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    start_simple(&app, "t");
    app.telemetry_add("t", "mystery_signal", 1, "human")
        .unwrap();
    let before = event_count(&app, "t");
    let proposal = app.next_action("t").unwrap();
    assert_eq!(proposal.action.as_str(), "ask");
    assert_eq!(
        event_count(&app, "t"),
        before,
        "next_action must be read-only"
    );
}

#[test]
fn next_action_replan_only_proposes() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    start_simple(&app, "t");
    // Three telemetry signals → high drift, no out-of-scope signal → replan.
    app.telemetry_add("t", "test_failures", 1, "human").unwrap(); // 15
    app.telemetry_add("t", "retries", 3, "human").unwrap(); // 15
                                                            // gate failing (20) pushes to 50 = high.
    app.record_gate("t", "cargo_check", false, "boom").unwrap();
    let before = event_count(&app, "t");
    let proposal = app.next_action("t").unwrap();
    assert_eq!(proposal.action.as_str(), "replan");
    assert!(proposal.structured_proposal.is_some());
    // The proposal is advisory: no scope change, no new events.
    assert_eq!(event_count(&app, "t"), before);
}

#[test]
fn reconcile_with_telemetry_is_byte_identical() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    start_simple(&app, "t");
    app.telemetry_add("t", "test_failures", 2, "human").unwrap();
    app.telemetry_add("t", "unexpected_writes", 1, "human")
        .unwrap();

    app.reconcile().unwrap();
    let path = dir.path().join(".ctl/control.json");
    let first = std::fs::read_to_string(&path).unwrap();
    // Drift fields are present in the projection.
    assert!(first.contains("drift_level"));
    assert!(first.contains("recommended_action"));

    app.reconcile().unwrap();
    let second = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        first, second,
        "control.json with telemetry must be byte-identical on replay"
    );
}

#[test]
fn finish_skips_interlock_outside_git_repo() {
    // Non-git temp dir: tree is unverifiable, so the interlock is skipped
    // and finish falls through to its other checks (here: succeeds).
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_to_review(&app, "nogit");
    std::fs::write(dir.path().join("src/work.rs"), "fn w() {}\n").unwrap();
    let event = app.finish_task("nogit").unwrap();
    assert_eq!(event.event_type, "task_completed");
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

// ── M6: dependency-gated start (serial orchestration) ───────────────────

/// Create + ready a task with the given dependency edges, leaving it Ready
/// (not started) so the start-time dependency gate can be exercised.
fn create_with_deps(app: &ControlApp, id: &str, deps: &[&str]) {
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()];
    let deps: Vec<String> = deps.iter().map(|s| s.to_string()).collect();
    app.create_task(
        id,
        CreateTaskInput {
            objective: "dependency-gated start test",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &deps,
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
}

/// Drive an already-created, Ready task (dependencies satisfied) all the way
/// to Completed. No git repo → the M-g commit interlock is skipped; M-f's
/// audit is supplied by a non-implementer reviewer.
fn finish_ready(app: &ControlApp, id: &str) {
    app.start_task(id).unwrap();
    app.submit_task(id).unwrap();
    app.record_gate(id, "cargo_check", true, "ok").unwrap();
    audit_pass(app, id, None);
    app.finish_task(id).unwrap();
    assert_eq!(app.replay_task(id).unwrap().phase, Phase::Completed);
}

#[test]
fn start_blocked_while_dependency_incomplete() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    create_with_deps(&app, "dep", &[]); // left in Ready, never Completed
    create_with_deps(&app, "dependent", &["dep"]);
    assert_eq!(
        app.unmet_dependencies("dependent").unwrap(),
        vec!["dep".to_string()]
    );
    let err = app.start_task("dependent").unwrap_err().to_string();
    assert!(err.contains("blocked by"), "got: {err}");
    assert!(err.contains("dep"), "error should name the blocker: {err}");
}

#[test]
fn start_allowed_once_dependency_completed() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    create_with_deps(&app, "dep", &[]);
    finish_ready(&app, "dep");
    create_with_deps(&app, "dependent", &["dep"]);
    assert!(app.unmet_dependencies("dependent").unwrap().is_empty());
    let event = app.start_task("dependent").unwrap();
    assert_eq!(event.event_type, "task_started");
}

#[test]
fn start_allowed_when_dependency_archived() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    create_with_deps(&app, "dep", &[]);
    finish_ready(&app, "dep");
    app.archive_task("dep").unwrap();
    // Archiving keeps the phase at Completed, so it still satisfies.
    assert_eq!(app.replay_task("dep").unwrap().phase, Phase::Completed);
    create_with_deps(&app, "dependent", &["dep"]);
    assert!(app.unmet_dependencies("dependent").unwrap().is_empty());
    app.start_task("dependent").unwrap();
}

#[test]
fn start_rejected_for_unknown_dependency() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    create_with_deps(&app, "dependent", &["ghost"]);
    // Missing prerequisite → unmet (fail closed).
    assert_eq!(
        app.unmet_dependencies("dependent").unwrap(),
        vec!["ghost".to_string()]
    );
    let err = app.start_task("dependent").unwrap_err().to_string();
    assert!(err.contains("ghost"), "got: {err}");
}

#[test]
fn dependency_chain_runs_strictly_serial() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    create_with_deps(&app, "a", &[]);
    create_with_deps(&app, "b", &["a"]);
    create_with_deps(&app, "c", &["b"]);
    // While A is unfinished, both B and C are blocked.
    assert!(app.start_task("b").is_err());
    assert!(app.start_task("c").is_err());
    // A complete → B may start; C still blocked on the in-progress B.
    finish_ready(&app, "a");
    app.start_task("b").unwrap();
    assert!(app.start_task("c").is_err());
    // Drive B (already InProgress) to Completed → C may finally start.
    app.submit_task("b").unwrap();
    app.record_gate("b", "cargo_check", true, "ok").unwrap();
    audit_pass(&app, "b", None);
    app.finish_task("b").unwrap();
    let event = app.start_task("c").unwrap();
    assert_eq!(event.event_type, "task_started");
}

#[test]
fn start_unaffected_without_dependencies() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    create_with_deps(&app, "solo", &[]);
    assert!(app.unmet_dependencies("solo").unwrap().is_empty());
    let event = app.start_task("solo").unwrap();
    assert_eq!(event.event_type, "task_started");
}

// ── M6: AgentRun aggregate concurrency (slice 1) ────────────────────────

/// Create + ready + start a write task so runs can be created against it.
fn inprogress_task(app: &ControlApp, id: &str, write_allow: &[&str]) {
    let scope: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "m6 concurrent run test",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
    app.start_task(id).unwrap();
}

/// Seed a Running run aggregate directly via events (no git worktree), so
/// the overlap invariant can be exercised without a real repo. Returns the
/// run_id.
fn seed_running_run(app: &ControlApp, task_id: &str, write_allow: &[&str]) -> String {
    let run_id = generate_uuid();
    let wa: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
    let created = app
        .build_run_event(
            &run_id,
            "run_created",
            serde_json::json!({
                "task_id": task_id,
                "adapter": "omp",
                "write_allow": wa,
                "write_deny": [],
                "gates": ["cargo_check"],
            }),
        )
        .unwrap();
    app.append_run_event(&run_id, created).unwrap();
    let started = app
        .build_run_event(
            &run_id,
            "run_started",
            serde_json::json!({
                "worktree_path": format!(".ctl/runs/{}/worktree", run_id),
                "lease_id": "lease-seed",
            }),
        )
        .unwrap();
    app.append_run_event(&run_id, started).unwrap();
    run_id
}

// ── cross-ledger detect+repair (cross-ledger-detect-repair-v1) ──

fn mk_planning_task(app: &ControlApp, id: &str) {
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "x",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
}

/// Like `seed_running_run` but the worktree is an absolute path that actually
/// exists on disk — so the run looks fully consistent.
fn seed_running_run_with_worktree(
    app: &ControlApp,
    root: &Path,
    task_id: &str,
    write_allow: &[&str],
) -> String {
    let run_id = generate_uuid();
    let wt = root
        .join(".ctl")
        .join("runs")
        .join(&run_id)
        .join("worktree");
    std::fs::create_dir_all(&wt).unwrap();
    let wa: Vec<String> = write_allow.iter().map(|s| s.to_string()).collect();
    let created = app
        .build_run_event(
            &run_id,
            "run_created",
            serde_json::json!({
                "task_id": task_id, "adapter": "omp", "write_allow": wa,
                "write_deny": [], "gates": ["cargo_check"],
            }),
        )
        .unwrap();
    app.append_run_event(&run_id, created).unwrap();
    let started = app
        .build_run_event(
            &run_id,
            "run_started",
            serde_json::json!({
                "worktree_path": wt.to_string_lossy(), "lease_id": "lease-seed",
            }),
        )
        .unwrap();
    app.append_run_event(&run_id, started).unwrap();
    run_id
}

#[test]
fn cross_ledger_detects_orphan_run() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let run_id = seed_running_run(&app, "ghost-task", &["src"]);
    let f = app.cross_ledger_findings().unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].kind, CrossLedgerKind::OrphanRun);
    assert_eq!(f[0].run_id, run_id);
    assert!(matches!(f[0].repair, RepairAction::AbortRun { .. }));
}

#[test]
fn cross_ledger_detects_stranded_run() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.cancel_task("t1").unwrap(); // terminal task
    seed_running_run(&app, "t1", &["src"]);
    let f = app.cross_ledger_findings().unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].kind, CrossLedgerKind::StrandedRun);
}

#[test]
fn cross_ledger_detects_missing_worktree_run() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.mark_ready("t1").unwrap();
    app.start_task("t1").unwrap(); // live, InProgress
    seed_running_run(&app, "t1", &["src"]); // worktree path not on disk
    let f = app.cross_ledger_findings().unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].kind, CrossLedgerKind::MissingWorktreeRun);
}

#[test]
fn cross_ledger_clean_when_run_consistent() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.mark_ready("t1").unwrap();
    app.start_task("t1").unwrap();
    seed_running_run_with_worktree(&app, dir.path(), "t1", &["src"]);
    let f = app.cross_ledger_findings().unwrap();
    assert!(f.is_empty(), "consistent run yields no finding: {f:?}");
}

#[test]
fn cross_ledger_detects_orphaned_worktree() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let run_id = seed_running_run_with_worktree(&app, dir.path(), "ghost", &["src"]);
    // Terminal-ize WITHOUT abort_run so the worktree dir lingers.
    let aborted = app
        .build_run_event(&run_id, "run_aborted", serde_json::json!({"reason": "x"}))
        .unwrap();
    app.append_run_event(&run_id, aborted).unwrap();
    let f = app.cross_ledger_findings().unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].kind, CrossLedgerKind::OrphanedWorktree);
    assert!(matches!(f[0].repair, RepairAction::RemoveWorktree { .. }));
}

#[test]
fn cross_ledger_apply_aborts_run_and_clears_finding() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let run_id = seed_running_run(&app, "ghost-task", &["src"]);
    let findings = app.cross_ledger_findings().unwrap();
    assert_eq!(findings.len(), 1);
    let outcome = app.apply_cross_ledger_repair(&findings[0]);
    assert!(outcome.applied, "repair applied: {}", outcome.result);
    assert_eq!(outcome.run_id, run_id);
    // Run is now Aborted (terminal) → no longer a cross-ledger finding.
    let after = app.cross_ledger_findings().unwrap();
    assert!(after.is_empty(), "finding cleared after repair: {after:?}");
}

#[test]
fn handoff_export_assembles_read_only_artifact() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.mark_ready("t1").unwrap();
    app.start_task("t1").unwrap();

    let h = app.handoff_export("t1").unwrap();
    assert_eq!(h["schema"], "control.handoff.v1");
    assert_eq!(h["task_id"], "t1");
    assert_eq!(h["phase"], "InProgress");
    assert_eq!(h["objective"], "x");
    assert!(h["boundary"]["write_allow"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "src"));
    let gates = h["gate_status"].as_array().unwrap();
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0]["status"], "PENDING"); // gate never run
    assert!(!h["recent_events"].as_array().unwrap().is_empty());

    // Export must be purely read-only — no events appended.
    let before = app.replay_task("t1").unwrap().last_seq;
    let _ = app.handoff_export("t1").unwrap();
    assert_eq!(
        app.replay_task("t1").unwrap().last_seq,
        before,
        "handoff export must not append events"
    );
}

#[test]
fn handoff_export_includes_captured_judgment() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.mark_ready("t1").unwrap();
    app.start_task("t1").unwrap();
    let handoffs = dir.path().join(".ctl/handoffs");
    std::fs::create_dir_all(&handoffs).unwrap();
    std::fs::write(
        handoffs.join("t1.json"),
        r#"{
            "schema": "control.handoff.capture.v1",
            "task_id": "t1",
            "source": "agent_or_human_supplied",
            "next_safe_action": "run the required gate",
            "decisions": ["keep the scope narrow"],
            "uncertainties": ["reviewer availability"]
        }"#,
    )
    .unwrap();

    let h = app.handoff_export("t1").unwrap();
    assert_eq!(h["capture"]["next_safe_action"], "run the required gate");
    assert_eq!(h["capture"]["source"], "agent_or_human_supplied");
    assert_eq!(h["capture"]["decisions"][0], "keep the scope narrow");
}

// ── ralph safety supervisor (ralph-safe-run-v1) ──

#[test]
fn ralph_safety_go_on_clean_active_task() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.mark_ready("t1").unwrap();
    app.start_task("t1").unwrap();
    let v = app.ralph_safety_check("t1").unwrap();
    assert!(v.go, "clean active task is GO, blockers: {:?}", v.blockers);
}

#[test]
fn ralph_safety_nogo_on_terminal_task() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.cancel_task("t1").unwrap();
    let v = app.ralph_safety_check("t1").unwrap();
    assert!(!v.go);
    assert!(
        v.blockers.iter().any(|b| b.contains("terminal")),
        "blockers: {:?}",
        v.blockers
    );
}

#[test]
fn ralph_safety_nogo_on_cross_ledger_drift() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_planning_task(&app, "t1");
    app.mark_ready("t1").unwrap();
    app.start_task("t1").unwrap();
    // A stranded/orphan run anywhere is a global cross-ledger inconsistency.
    seed_running_run(&app, "ghost-task", &["other"]);
    let v = app.ralph_safety_check("t1").unwrap();
    assert!(!v.go);
    assert!(
        v.blockers.iter().any(|b| b.contains("cross-ledger")),
        "blockers: {:?}",
        v.blockers
    );
}

// ── run-lease TTL expiry (capability-lease-ttl-enforce-v1) ──

#[test]
fn ttl_exceeded_is_strictly_greater() {
    assert!(ttl_exceeded(100, 0, 50)); // age 100 > 50
    assert!(!ttl_exceeded(40, 0, 50)); // age 40 < 50
    assert!(!ttl_exceeded(50, 0, 50)); // age 50 == 50 (not strictly greater)
    assert!(!ttl_exceeded(0, 1000, 50)); // now before created → age 0 (saturating)
}

/// Seed a Running run carrying a genuine native lease (lease_created +
/// lease_used + run_started), satisfying the run reducer's binding rules.
fn seed_run_with_native_lease(app: &ControlApp, run_id: &str, task_id: &str, ttl: u64) {
    let ev = |app: &ControlApp, ty: &str, p: serde_json::Value| {
        let e = app.build_run_event(run_id, ty, p).unwrap();
        app.append_run_event(run_id, e).unwrap();
    };
    ev(
        app,
        "run_created",
        serde_json::json!({"task_id": task_id, "adapter": "omp", "write_allow": ["src"],
            "write_deny": [], "gates": ["cargo_check"]}),
    );
    ev(
        app,
        "lease_created",
        serde_json::json!({"lease_id": "L1", "run_id": run_id, "resource_path": "src",
            "action": "write", "ttl_seconds": ttl, "max_uses": 100,
            "task_id": task_id, "adapter": "omp", "scopes": ["src"]}),
    );
    ev(app, "lease_used", serde_json::json!({"lease_id": "L1"}));
    ev(
        app,
        "run_started",
        serde_json::json!({"worktree_path": format!(".ctl/runs/{run_id}/worktree"), "lease_id": "L1"}),
    );
}

const FAR_FUTURE: u64 = 10_000_000_000; // year ~2286 — well past any lease TTL

#[test]
fn expire_lease_records_lease_expired_when_stale() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_run_with_native_lease(&app, "r1", "t1", 3600);
    let report = app.expire_run_lease_at("r1", FAR_FUTURE, true).unwrap();
    assert_eq!(report.outcome, "expired", "{}", report.detail);
    // The lease is now terminally Expired.
    let run = app.replay_run("r1").unwrap();
    assert_eq!(
        run.lease.unwrap().status,
        crate::domain::lease::LeaseStatus::Expired
    );
}

#[test]
fn expire_lease_preview_does_not_mutate() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_run_with_native_lease(&app, "r1", "t1", 3600);
    let before = app.replay_run("r1").unwrap().last_seq;
    let report = app.expire_run_lease_at("r1", FAR_FUTURE, false).unwrap();
    assert_eq!(report.outcome, "would_expire", "{}", report.detail);
    assert_eq!(
        app.replay_run("r1").unwrap().last_seq,
        before,
        "preview must not append"
    );
    assert_eq!(
        app.replay_run("r1").unwrap().lease.unwrap().status,
        crate::domain::lease::LeaseStatus::Active
    );
}

#[test]
fn expire_lease_refuses_within_ttl() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_run_with_native_lease(&app, "r1", "t1", 3600);
    // now before created → age 0 → not stale.
    let report = app.expire_run_lease_at("r1", 1, false).unwrap();
    assert_eq!(report.outcome, "within_ttl", "{}", report.detail);
}

#[test]
fn expire_lease_no_native_lease() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_running_run(&app, "t1", &["src"]); // legacy run, no native lease
    let runs = app.run_store().unwrap().run_ids().unwrap();
    let report = app.expire_run_lease_at(&runs[0], FAR_FUTURE, true).unwrap();
    assert_eq!(report.outcome, "no_lease", "{}", report.detail);
}

#[test]
fn create_run_requires_in_progress_task() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let scope = vec!["src".to_string()];
    let gates = vec!["cargo_check".to_string()];
    app.create_task(
        "planned",
        CreateTaskInput {
            objective: "x",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &gates,
            depends_on: &[],
        },
    )
    .unwrap();
    // Still in Planning → no run may be created.
    let err = app.create_run("planned", "omp").unwrap_err().to_string();
    assert!(err.contains("InProgress"), "got: {err}");
}

#[test]
fn create_run_persists_queued_aggregate() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    inprogress_task(&app, "t1", &["src"]);
    let run_id = app.create_run("t1", "omp").unwrap();
    assert!(dir
        .path()
        .join(".ctl/runs")
        .join(&run_id)
        .join("events.jsonl")
        .exists());
    let run = app.replay_run(&run_id).unwrap();
    assert_eq!(run.phase, RunPhase::Queued);
    assert_eq!(run.task_id, "t1");
    assert!(run.write_allow.contains("src"));
    // Queued is not yet part of the active concurrency set.
    assert!(app.active_runs().unwrap().is_empty());
}

#[test]
fn overlapping_run_start_rejected() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // A already Running on src (seeded, no git needed).
    let a = seed_running_run(&app, "task-a", &["src"]);
    assert_eq!(app.active_runs().unwrap().len(), 1);
    // B is InProgress with an overlapping scope; its run start is refused
    // BEFORE any worktree is created.
    inprogress_task(&app, "task-b", &["src"]);
    let b = app.create_run("task-b", "omp").unwrap();
    let err = app.start_run(&b).unwrap_err().to_string();
    assert!(err.contains("scope conflict"), "got: {err}");
    assert!(err.contains(&a), "should name the conflicting run: {err}");
    assert!(!dir
        .path()
        .join(".ctl/runs")
        .join(&b)
        .join("worktree")
        .exists());
    assert_eq!(app.replay_run(&b).unwrap().phase, RunPhase::Queued);
}

#[test]
fn finishing_run_frees_scope() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let a = seed_running_run(&app, "task-a", &["src"]);
    let scope: BTreeSet<String> = ["src".to_string()].into_iter().collect();
    // While A runs, an overlapping scope is blocked.
    assert!(app.check_run_scope_overlap("other", &scope).is_err());
    // Finish A (seeded worktree path doesn't exist → cleanup skipped, no git).
    app.finish_run(&a).unwrap();
    assert_eq!(app.replay_run(&a).unwrap().phase, RunPhase::Completed);
    assert!(app.active_runs().unwrap().is_empty());
    // Scope is free again.
    assert!(app.check_run_scope_overlap("other", &scope).is_ok());
}

#[test]
fn disjoint_runs_run_concurrently() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_running_run(&app, "task-a", &["src"]);
    seed_running_run(&app, "task-b", &["docs"]);
    assert_eq!(app.active_runs().unwrap().len(), 2);
    // A further disjoint scope is allowed; an overlapping one is not.
    let disjoint: BTreeSet<String> = ["tests".to_string()].into_iter().collect();
    assert!(app.check_run_scope_overlap("c", &disjoint).is_ok());
    let overlap: BTreeSet<String> = ["src".to_string()].into_iter().collect();
    assert!(app.check_run_scope_overlap("c", &overlap).is_err());
}

#[test]
fn run_replay_is_deterministic() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let a = seed_running_run(&app, "task-a", &["src"]);
    let s1 = app.replay_run(&a).unwrap();
    let s2 = app.replay_run(&a).unwrap();
    assert_eq!(s1.phase, s2.phase);
    assert_eq!(s1.write_allow, s2.write_allow);
    assert_eq!(s1.last_seq, s2.last_seq);
}

#[test]
fn concurrent_runs_via_start_run_with_real_worktrees() {
    let dir = TempDir::new();
    // git repo + initial commit so `git worktree add HEAD` succeeds.
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.path().join("docs/readme.md"), "x\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);

    let app = ControlApp::init(dir.path()).unwrap();
    inprogress_task(&app, "task-src", &["src"]);
    inprogress_task(&app, "task-docs", &["docs"]);

    // Two disjoint runs both reach Running with real, distinct worktrees.
    let r_src = app.create_run("task-src", "omp").unwrap();
    app.start_run(&r_src).unwrap();
    let r_docs = app.create_run("task-docs", "omp").unwrap();
    app.start_run(&r_docs).unwrap();
    assert_eq!(app.active_runs().unwrap().len(), 2);
    assert!(dir
        .path()
        .join(".ctl/runs")
        .join(&r_src)
        .join("worktree")
        .exists());
    assert!(dir
        .path()
        .join(".ctl/runs")
        .join(&r_docs)
        .join("run-manifest.json")
        .exists());

    // A third run overlapping task-src's scope is rejected.
    inprogress_task(&app, "task-src2", &["src"]);
    let r_src2 = app.create_run("task-src2", "omp").unwrap();
    assert!(app
        .start_run(&r_src2)
        .unwrap_err()
        .to_string()
        .contains("scope conflict"));

    // Finishing the src run frees the scope; the blocked run can then start.
    app.finish_run(&r_src).unwrap();
    app.start_run(&r_src2).unwrap();
    assert_eq!(app.replay_run(&r_src2).unwrap().phase, RunPhase::Running);
}

// ── M6: run-scoped capability lease wiring (capability-lease-run-wiring-v1) ──

#[test]
fn start_run_grants_and_consumes_native_lease() {
    let dir = TempDir::new();
    let (app, run_id, _wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
    let run = app.replay_run(&run_id).unwrap();
    let lease = run.lease.as_ref().expect("native run lease present");
    assert_eq!(lease.status, crate::domain::lease::LeaseStatus::Active);
    assert_eq!(lease.max_uses, RUN_LEASE_MAX_USES);
    assert_eq!(lease.ttl_seconds, RUN_LEASE_TTL_SECONDS);
    // Start consumes exactly one use.
    assert_eq!(lease.remaining_uses, RUN_LEASE_MAX_USES - 1);
    assert_eq!(lease.task_id, "task-src");
    assert_eq!(lease.adapter, "omp");
    assert_eq!(lease.scopes, run.write_allow);
    assert_eq!(run.lease_id.as_deref(), Some(lease.lease_id.as_str()));

    // Manifest carries the same lease_id.
    let manifest = std::fs::read_to_string(
        dir.path()
            .join(".ctl/runs")
            .join(&run_id)
            .join("run-manifest.json"),
    )
    .unwrap();
    assert!(manifest.contains(&lease.lease_id));

    // run.json projection reports structured (non-prose) lease fields.
    let run_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".ctl/runs").join(&run_id).join("run.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(run_json["lease_status"], "ACTIVE");
    assert_eq!(run_json["lease_compat"], "native");
    assert_eq!(run_json["remaining_uses"], RUN_LEASE_MAX_USES - 1);
}

#[test]
fn overlap_rejected_emits_no_lease_event() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let _a = seed_running_run(&app, "task-a", &["src"]); // Running on src
    inprogress_task(&app, "task-b", &["src"]);
    let b = app.create_run("task-b", "omp").unwrap();
    assert!(app
        .start_run(&b)
        .unwrap_err()
        .to_string()
        .contains("scope conflict"));
    let run = app.replay_run(&b).unwrap();
    assert_eq!(run.phase, RunPhase::Queued);
    assert!(
        run.lease.is_none(),
        "a rejected start must not grant a lease"
    );
    // Only run_created is on the ledger — no lease event leaked.
    let events =
        std::fs::read_to_string(dir.path().join(".ctl/runs").join(&b).join("events.jsonl"))
            .unwrap();
    assert_eq!(
        events.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "rejected start left extra events: {events}"
    );
    assert!(!events.contains("lease_created"));
}

#[test]
fn second_start_run_does_not_double_consume() {
    let dir = TempDir::new();
    let (app, run_id, _wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
    let before = app
        .replay_run(&run_id)
        .unwrap()
        .lease
        .unwrap()
        .remaining_uses;
    assert!(app
        .start_run(&run_id)
        .unwrap_err()
        .to_string()
        .contains("Queued"));
    let after = app
        .replay_run(&run_id)
        .unwrap()
        .lease
        .unwrap()
        .remaining_uses;
    assert_eq!(before, after, "rejected re-start must not consume a use");
    assert_eq!(after, RUN_LEASE_MAX_USES - 1);
}

#[test]
fn finish_revokes_lease_and_unblocks_overlapping_run() {
    let dir = TempDir::new();
    let (app, r_src, _wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
    // An overlapping run is blocked while the first holds the scope.
    inprogress_task(&app, "task-src2", &["src"]);
    let r2 = app.create_run("task-src2", "omp").unwrap();
    assert!(app
        .start_run(&r2)
        .unwrap_err()
        .to_string()
        .contains("scope conflict"));
    // Finishing the first run revokes its lease and frees the scope.
    app.finish_run(&r_src).unwrap();
    let first = app.replay_run(&r_src).unwrap();
    let first_lease = first.lease.clone().unwrap();
    assert_eq!(
        first_lease.status,
        crate::domain::lease::LeaseStatus::Revoked
    );
    // The previously-blocked run can now start and gets its OWN active lease.
    app.start_run(&r2).unwrap();
    let second = app.replay_run(&r2).unwrap();
    assert_eq!(second.phase, RunPhase::Running);
    let l2 = second.lease.unwrap();
    assert_eq!(l2.status, crate::domain::lease::LeaseStatus::Active);
    assert_eq!(l2.remaining_uses, RUN_LEASE_MAX_USES - 1);
    assert_ne!(l2.lease_id, first_lease.lease_id);
}

#[test]
fn recover_reports_unknown_lease_for_legacy_run() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // seed_running_run emits run_started with an opaque lease_id and NO
    // lease_created — a slice-1 (pre-lease) run.
    seed_running_run(&app, "t", &["src"]);
    let report = app.recover_report().unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].lease_status, "UNKNOWN");
    assert_eq!(report[0].lease_compat, "pre_lease_run");
    assert_eq!(report[0].remaining_uses, None);
    assert_eq!(report[0].lease_id.as_deref(), Some("lease-seed"));
    assert!(!report[0].lease_nonactive);
}

#[test]
fn partial_start_run_detected_read_only() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // Hand-build a crash mid-start: run_created + lease_created + lease_used,
    // but no run_started (process died before the start committed).
    let run_id = generate_uuid();
    let created = app
        .build_run_event(
            &run_id,
            "run_created",
            serde_json::json!({
                "task_id": "t", "adapter": "omp",
                "write_allow": ["src"], "write_deny": [], "gates": ["cargo_check"],
            }),
        )
        .unwrap();
    app.append_run_event(&run_id, created).unwrap();
    let lc = app
        .build_run_event(
            &run_id,
            "lease_created",
            serde_json::json!({
                "lease_id": "L", "run_id": run_id.clone(), "resource_path": "src",
                "action": "write", "ttl_seconds": 3600, "max_uses": 100,
                "task_id": "t", "adapter": "omp", "scopes": ["src"],
            }),
        )
        .unwrap();
    app.append_run_event(&run_id, lc).unwrap();
    let lu = app
        .build_run_event(&run_id, "lease_used", serde_json::json!({"lease_id": "L"}))
        .unwrap();
    app.append_run_event(&run_id, lu).unwrap();

    let partials = app.partial_start_runs().unwrap();
    assert_eq!(partials.len(), 1);
    assert_eq!(partials[0]["run_id"].as_str(), Some(run_id.as_str()));
    assert_eq!(partials[0]["lease_status"], "ACTIVE");
    // Still Queued → absent from the Running-only recover report.
    assert!(app.recover_report().unwrap().is_empty());
    // The read-only scan appended nothing.
    assert_eq!(app.replay_run(&run_id).unwrap().last_seq, 3);
}

#[test]
fn running_run_with_revoked_lease_flagged_nonactive() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let run_id = generate_uuid();
    for (etype, payload) in [
        (
            "run_created",
            serde_json::json!({
                "task_id": "t", "adapter": "omp",
                "write_allow": ["src"], "write_deny": [], "gates": ["cargo_check"],
            }),
        ),
        (
            "lease_created",
            serde_json::json!({
                "lease_id": "L", "run_id": run_id.clone(), "resource_path": "src",
                "action": "write", "ttl_seconds": 3600, "max_uses": 100,
                "task_id": "t", "adapter": "omp", "scopes": ["src"],
            }),
        ),
        ("lease_used", serde_json::json!({"lease_id": "L"})),
        (
            "run_started",
            serde_json::json!({
                "worktree_path": format!(".ctl/runs/{}/worktree", run_id),
                "lease_id": "L",
            }),
        ),
        ("lease_revoked", serde_json::json!({"lease_id": "L"})),
    ] {
        let e = app.build_run_event(&run_id, etype, payload).unwrap();
        app.append_run_event(&run_id, e).unwrap();
    }
    let report = app.recover_report().unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].lease_status, "REVOKED");
    assert!(report[0].lease_nonactive);
}

#[test]
fn expire_stale_approvals_records_approval_expired_and_is_idempotent() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    inprogress_task(&app, "t", &["src"]);

    // Request and grant an approval, but backdate the grant far past its TTL
    // so the wall-clock expiry check fires deterministically.
    let req = app
        .approval_request(
            "t",
            "high-risk edit",
            serde_json::json!({ "high_risk_files": ["src/x.rs"] }),
            1,
        )
        .unwrap();
    let request_id = req.payload["request_id"].as_str().unwrap().to_string();

    let mut granted = app
        .build_event(
            "t",
            "approval_granted",
            serde_json::json!({ "request_id": request_id }),
        )
        .unwrap();
    granted.occurred_at = "2020-01-01T00:00:00Z".to_string();
    app.validate_and_append(&granted).unwrap();
    app.rebuild_task_view("t").unwrap();

    // Precondition: the approval reads as Granted before expiry runs.
    let before = app.replay_task("t").unwrap();
    assert_eq!(
        before.pending_approvals[&request_id].status,
        crate::domain::approval::ApprovalStatus::Granted
    );
    let seq_before = before.last_seq;

    // Expiry records an explicit approval_expired event and transitions state.
    app.expire_stale_approvals("t").unwrap();
    let after = app.replay_task("t").unwrap();
    assert_eq!(
        after.pending_approvals[&request_id].status,
        crate::domain::approval::ApprovalStatus::Expired
    );
    assert_eq!(after.last_seq, seq_before + 1);

    // Idempotent: a second pass appends nothing (the approval is no longer granted).
    app.expire_stale_approvals("t").unwrap();
    assert_eq!(app.replay_task("t").unwrap().last_seq, seq_before + 1);
}

// ── M6: crash recovery (slice 2) ────────────────────────────────────────

#[test]
fn recover_report_lists_only_running() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let r = seed_running_run(&app, "t", &["src"]); // Running
    inprogress_task(&app, "tq", &["docs"]);
    app.create_run("tq", "omp").unwrap(); // Queued, never started
    let report = app.recover_report().unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].run_id, r);
    // The seeded run points at a worktree that was never created → flagged
    // as inconsistent (a crash-recovery abort candidate).
    assert!(!report[0].worktree_exists);
}

#[test]
fn recover_abort_frees_scope_and_drops_from_report() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let r = seed_running_run(&app, "t", &["src"]);
    let scope: BTreeSet<String> = ["src".to_string()].into_iter().collect();
    assert!(app.check_run_scope_overlap("x", &scope).is_err());
    app.abort_run(&r, "crash recovery").unwrap();
    assert_eq!(app.replay_run(&r).unwrap().phase, RunPhase::Aborted);
    assert!(app.check_run_scope_overlap("x", &scope).is_ok());
    assert!(app.recover_report().unwrap().is_empty());
    // Aborting an already-terminal run is rejected (no duplicate side effect).
    assert!(app.abort_run(&r, "again").is_err());
}

#[test]
fn finish_drops_run_from_recover_report() {
    // run-finish-emit-v1: the production finish path (now reachable via
    // `ctl run finish`) drives a Running run to Completed and out of the open
    // run / recovery view — the B2 fix (a prod run can finally reach Finished).
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let r = seed_running_run(&app, "t", &["src"]);
    assert_eq!(
        app.recover_report().unwrap().len(),
        1,
        "a Running run shows as open before finish"
    );
    app.finish_run(&r).unwrap();
    assert_eq!(app.replay_run(&r).unwrap().phase, RunPhase::Completed);
    assert!(
        app.recover_report().unwrap().is_empty(),
        "a finished run is no longer open/stranded"
    );
    // Reducer guard: finishing an already-terminal run is rejected.
    assert!(
        app.finish_run(&r).is_err(),
        "only a Running run can be finished"
    );
}

#[test]
fn finish_run_with_provenance_hashes_artifacts_and_records_host_attested_values() {
    // run-attestation-fields-v1: ctl sha256-hashes the supplied artifact and
    // records host-reported fields; absent fields stay unset.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let r = seed_running_run(&app, "t", &["src"]);
    let art = dir.path().join("instruction.txt");
    std::fs::write(&art, b"do the thing").unwrap();
    let prov = RunProvenanceInput {
        model: Some("claude-opus-4-8".into()),
        instruction_artifact: Some(art.to_string_lossy().into_owned()),
        exit_code: Some(0),
        ..Default::default()
    };
    app.finish_run_with_provenance(&r, &prov).unwrap();
    let state = app.replay_run(&r).unwrap();
    assert_eq!(state.phase, RunPhase::Completed);
    assert_eq!(state.model.as_deref(), Some("claude-opus-4-8"));
    assert_eq!(state.exit_code, Some(0));
    // ctl recorded the artifact's sha256 (64 hex chars), never the path.
    let h = state
        .instruction_hash
        .as_deref()
        .expect("instruction hash recorded");
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    // Unsupplied provenance is simply absent.
    assert!(state.provider.is_none());
    assert!(state.context_hash.is_none());
}

#[test]
fn record_subagent_dispatch_records_host_attested_dispatch() {
    // subagent-dispatch-record-v1: the dispatch is appended to the task ledger
    // (passing envelope-schema validation) with the artifact ctl-hashed.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    inprogress_task(&app, "t", &["src"]);
    std::fs::write(dir.path().join("instruction.txt"), b"the instruction").unwrap();
    app.record_subagent_dispatch(
        "t",
        "designer",
        "opencode",
        Some("run-1"),
        Some("instruction.txt"),
        None,
        None,
    )
    .unwrap();
    let state = app.replay_task("t").unwrap();
    assert_eq!(state.dispatches.len(), 1);
    let d = &state.dispatches[0];
    assert_eq!(d.role, "designer");
    assert_eq!(d.adapter, "opencode");
    assert_eq!(d.parent_run.as_deref(), Some("run-1"));
    let instr = d.instruction.as_ref().expect("instruction recorded");
    assert_eq!(instr.hash.len(), 64);
    assert!(instr.hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(d.context.is_none() && d.output.is_none());
}

#[test]
fn orphaned_worktrees_lists_terminal_run_leftover() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let r = seed_running_run(&app, "t", &["src"]);
    app.finish_run(&r).unwrap(); // terminal
                                 // Nothing on disk yet (seeded worktree was never created).
    assert!(app.orphaned_run_worktrees().unwrap().is_empty());
    // Simulate a leftover worktree dir for the now-terminal run.
    let wt = crate::infrastructure::workspace::run_worktree_path(dir.path(), &r);
    std::fs::create_dir_all(&wt).unwrap();
    let orphans = app.orphaned_run_worktrees().unwrap();
    assert!(orphans.iter().any(|o| o.contains(&r)), "got: {orphans:?}");
}

#[test]
fn recover_abort_removes_real_worktree() {
    let dir = TempDir::new();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);

    let app = ControlApp::init(dir.path()).unwrap();
    inprogress_task(&app, "task-src", &["src"]);
    let r = app.create_run("task-src", "omp").unwrap();
    app.start_run(&r).unwrap();
    let wt = crate::infrastructure::workspace::run_worktree_path(dir.path(), &r);
    assert!(wt.exists());
    // A real, in-flight run is reported with a present worktree + manifest.
    let rep = app.recover_report().unwrap();
    assert!(rep
        .iter()
        .any(|s| s.run_id == r && s.worktree_exists && s.manifest_exists));
    // Recovery abort tears it down and frees the scope.
    app.abort_run(&r, "crash recovery").unwrap();
    assert!(!wt.exists());
    assert_eq!(app.replay_run(&r).unwrap().phase, RunPhase::Aborted);
    assert!(app.recover_report().unwrap().is_empty());
}

// ── M6: merge-candidate / recovery (slice 3) ────────────────────────────

/// git repo (tracked src/lib.rs + docs/readme.md) + an InProgress task and a
/// started run with a real worktree. Returns (app, run_id, worktree_path).
fn git_repo_with_started_run(
    dir: &Path,
    task: &str,
    scope: &[&str],
) -> (ControlApp, String, PathBuf) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.join("docs/readme.md"), "x\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "init"]);

    let app = ControlApp::init(dir).unwrap();
    inprogress_task(&app, task, scope);
    let run_id = app.create_run(task, "omp").unwrap();
    app.start_run(&run_id).unwrap();
    let wt = crate::infrastructure::workspace::run_worktree_path(dir, &run_id);
    (app, run_id, wt)
}

#[test]
fn run_merge_candidate_clean_is_mergeable() {
    let dir = TempDir::new();
    let (app, run_id, wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
    // Edit a tracked, in-scope file inside the run's isolated worktree.
    std::fs::write(wt.join("src/lib.rs"), "fn a() { /* run edit */ }\n").unwrap();
    let v = app.run_merge_candidate(&run_id).unwrap();
    assert_eq!(v["mergeable"], true, "verdict: {v}");
    assert!(v["touched_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p == "src/lib.rs"));
    assert!(v["recovery"].as_array().unwrap().is_empty());
}

#[test]
fn run_merge_candidate_dirty_main_blocks_with_recovery() {
    let dir = TempDir::new();
    let (app, run_id, wt) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
    std::fs::write(wt.join("src/lib.rs"), "fn a() { /* run edit */ }\n").unwrap();
    // The main workspace has its own uncommitted edit to the same file.
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "fn a() { /* main edit */ }\n",
    )
    .unwrap();
    let v = app.run_merge_candidate(&run_id).unwrap();
    assert_eq!(v["mergeable"], false, "verdict: {v}");
    assert!(v["workspace_conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p == "src/lib.rs"));
    let rec = v["recovery"].as_array().unwrap();
    assert!(rec
        .iter()
        .any(|r| r["category"] == "dirty_main_workspace" && r["action"].is_string()));
}

#[test]
fn run_merge_candidate_out_of_scope_and_cross_run() {
    let dir = TempDir::new();
    // Run A scoped to src; a concurrent run B scoped to docs.
    let (app, run_a, wt_a) = git_repo_with_started_run(dir.path(), "task-src", &["src"]);
    inprogress_task(&app, "task-docs", &["docs"]);
    let run_b = app.create_run("task-docs", "omp").unwrap();
    app.start_run(&run_b).unwrap();
    // Run A writes into docs/ — outside its own scope AND into B's territory.
    std::fs::write(wt_a.join("docs/readme.md"), "y\n").unwrap();
    let v = app.run_merge_candidate(&run_a).unwrap();
    assert_eq!(v["mergeable"], false, "verdict: {v}");
    assert!(v["out_of_scope"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p == "docs/readme.md"));
    let crc = v["cross_run_conflicts"].as_array().unwrap();
    assert!(crc.iter().any(|c| c["conflicting_run"] == run_b.as_str()));
    let cats: Vec<&str> = v["recovery"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["category"].as_str().unwrap())
        .collect();
    assert!(cats.contains(&"out_of_scope") && cats.contains(&"cross_run_conflict"));
}

#[test]
fn run_merge_candidate_missing_worktree_errors() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // Seeded run points at a worktree that was never created.
    let run_id = seed_running_run(&app, "t", &["src"]);
    let err = app.run_merge_candidate(&run_id).unwrap_err().to_string();
    assert!(err.contains("worktree is missing"), "got: {err}");
    assert!(err.contains("recover"), "should point at recovery: {err}");
}

// ── BS-provenance V1 ──────────────────────────────────────────────────
//
// Record-only brainstorm artifact provenance: hashes, source, skip reason,
// and staleness — without gating or claiming thinking quality/independence.

/// Create a Planning task and write two originator artifacts under the
/// project's `.ctl/brainstorms/<bs>/`. Returns their project-relative paths.
fn seed_brainstorm_task(app: &ControlApp, id: &str, bs: &str) -> (String, String) {
    let scope = vec!["src".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "bs provenance",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
    let dir = app.project_root.join(".ctl").join("brainstorms").join(bs);
    std::fs::create_dir_all(&dir).unwrap();
    let div = format!(".ctl/brainstorms/{bs}/divergence.json");
    let conv = format!(".ctl/brainstorms/{bs}/convergence.json");
    std::fs::write(app.project_root.join(&div), b"{\"candidates\":[1,2,3]}").unwrap();
    std::fs::write(app.project_root.join(&conv), b"{\"proposal\":\"x\"}").unwrap();
    (div, conv)
}

#[test]
fn bs_provenance_records_artifact_hash() {
    // Behavior 1: the recorded reference carries the SHA-256 of the artifact.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let (div, conv) = seed_brainstorm_task(&app, "t", "BS-001");
    app.record_brainstorm_artifacts("t", "BS-001", &div, Some(&conv), None)
        .unwrap();
    let state = app.get_status("t").unwrap();
    let reference = state.brainstorm_ref.expect("reference recorded");
    let expected = hash_file(&app.project_root.join(&div)).unwrap();
    assert_eq!(reference.divergence.unwrap().hash, expected);
    // A recorded reference is L0 content and never asserts independence.
    assert_eq!(reference.trust_level, "content_l0");
    assert_eq!(reference.critic_independence, "unattested");
}

#[test]
fn bs_provenance_detects_stale_artifact() {
    // Behavior 2: editing or deleting an artifact makes the reference stale.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let (div, conv) = seed_brainstorm_task(&app, "t", "BS-002");
    app.record_brainstorm_artifacts("t", "BS-002", &div, Some(&conv), None)
        .unwrap();
    let state = app.get_status("t").unwrap();
    // Fresh immediately after recording.
    let view = app.brainstorm_provenance_view(&state).unwrap();
    assert!(!view.divergence.as_ref().unwrap().stale);
    // Edited on disk → present but stale.
    std::fs::write(app.project_root.join(&div), b"MUTATED").unwrap();
    let edited = app.brainstorm_provenance_view(&state).unwrap();
    let d = edited.divergence.unwrap();
    assert!(
        d.present && d.stale,
        "edited artifact must read present+stale"
    );
    // Deleted → absent from disk and stale.
    std::fs::remove_file(app.project_root.join(&conv)).unwrap();
    let removed = app.brainstorm_provenance_view(&state).unwrap();
    let c = removed.convergence.unwrap();
    assert!(
        !c.present && c.stale,
        "deleted artifact must read missing+stale"
    );
}

#[test]
fn bs_provenance_task_references_brainstorm() {
    // Behavior 3: a task can reference a brainstorm (with an unattested run).
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let (div, conv) = seed_brainstorm_task(&app, "t", "BS-003");
    app.record_brainstorm_artifacts("t", "BS-003", &div, Some(&conv), Some("run-9"))
        .unwrap();
    let state = app.get_status("t").unwrap();
    let reference = state.brainstorm_ref.clone().unwrap();
    assert_eq!(reference.id, "BS-003");
    assert_eq!(reference.source_run_id.as_deref(), Some("run-9"));
    // A recorded source run is a claim, never an attestation.
    let view = app.brainstorm_provenance_view(&state).unwrap();
    assert!(!view.source_run_attested);
}

#[test]
fn bs_provenance_critic_attached_independently() {
    // Behavior 4: a critic artifact can be attached as a separate invocation.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let (div, conv) = seed_brainstorm_task(&app, "t", "BS-004");
    app.record_brainstorm_artifacts("t", "BS-004", &div, Some(&conv), None)
        .unwrap();
    let critic = ".ctl/brainstorms/BS-004/critic.json";
    std::fs::write(app.project_root.join(critic), b"{\"challenge\":\"...\"}").unwrap();
    app.attach_brainstorm_critic("t", "BS-004", critic, None)
        .unwrap();
    let reference = app.get_status("t").unwrap().brainstorm_ref.unwrap();
    assert_eq!(reference.critic_disposition.as_str(), "present");
    assert!(reference.critic.is_some());
    // Even attached, independence is never asserted in V1.
    assert_eq!(reference.critic_independence, "unattested");
}

#[test]
fn bs_provenance_skip_records_reason_and_actor() {
    // Behaviors 5 + 6: with no critic, a skip records reason + decider, and
    // the recording actor is captured in the canonical event.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let (div, conv) = seed_brainstorm_task(&app, "t", "BS-005");
    app.record_brainstorm_artifacts("t", "BS-005", &div, Some(&conv), None)
        .unwrap();
    ControlApp::open(&app.project_root, false)
        .unwrap()
        .with_actor("agent-7")
        .skip_brainstorm_critic("t", "BS-005", "explicit_user_skip", Some("human"), None)
        .unwrap();
    let reference = app.get_status("t").unwrap().brainstorm_ref.unwrap();
    assert_eq!(reference.critic_disposition.as_str(), "skipped");
    assert_eq!(reference.skip_reason.as_deref(), Some("explicit_user_skip"));
    assert_eq!(reference.skip_decided_by.as_deref(), Some("human"));
    // The actor that recorded the skip is in the ledger event itself.
    let events = app.store.read_for_task("t").unwrap();
    let skip = events
        .iter()
        .find(|e| e.event_type == "brainstorm_skipped")
        .unwrap();
    assert_eq!(skip.actor, "agent-7");
}

#[test]
fn bs_provenance_bare_file_is_not_canonical_provenance() {
    // Behavior 10: a brainstorm file on disk is never auto-promoted to
    // canonical provenance — only an explicit recording event creates it,
    // and recording refuses a path that is not actually present.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let (_div, _conv) = seed_brainstorm_task(&app, "t", "BS-010");
    // Artifacts exist on disk, but nothing was recorded.
    let state = app.get_status("t").unwrap();
    assert!(state.brainstorm_ref.is_none());
    assert!(app.brainstorm_provenance_view(&state).is_none());
    // Recording a non-existent artifact is refused.
    let err = app
        .record_brainstorm_artifacts(
            "t",
            "BS-010",
            ".ctl/brainstorms/BS-010/missing.json",
            None,
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("not found"), "got: {err}");
}

// ── Uncertainty Ledger V1 ──
// Record-and-disclose unknowns: open lifecycle, resolved-needs-evidence,
// terminal-is-terminal, and evidence freshness — without gating or verdict.

fn seed_uncertainty_task(app: &ControlApp, id: &str) {
    let scope = vec!["src".to_string()];
    app.create_task(
        id,
        CreateTaskInput {
            objective: "uncertainty ledger",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
}

#[test]
fn uncertainty_cannot_be_recorded_after_terminal() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    // Allowed while the task is live.
    app.record_uncertainty("t", "U-1", "open question", None)
        .unwrap();
    // Drive to a terminal phase.
    app.cancel_task("t").unwrap();
    // The command layer refuses to grow a terminal task's unknown set.
    let err = app
        .record_uncertainty("t", "U-2", "too late", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("terminal task cannot record"), "got: {err}");
    // Reducer stays permissive: a directly-appended post-terminal event still
    // replays (append-only history is never re-rejected on replay).
    let event = app
        .build_event(
            "t",
            "uncertainty_recorded",
            serde_json::json!({
                "uncertainty_id": "U-3",
                "statement": "committed pre-rule post-terminal record",
                "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
            }),
        )
        .unwrap();
    app.validate_and_append(&event).unwrap();
    let state = app.replay_task("t").unwrap();
    assert!(state.uncertainties.iter().any(|u| u.id == "U-3"));
}

#[test]
fn uncertainty_recorded_is_open_l0_content() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "does the gate kill the tree?", Some("review"))
        .unwrap();
    let state = app.get_status("t").unwrap();
    assert_eq!(state.uncertainties.len(), 1);
    assert_eq!(state.uncertainties[0].status.as_str(), "open");
    let view = app.uncertainty_ledger_view(&state).unwrap();
    assert_eq!((view.open, view.resolved), (1, 0));
    // Recording an unknown never raises trust above bare content.
    assert_eq!(view.trust_level, "content_l0");
}

#[test]
fn uncertainty_duplicate_id_rejected() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "first", None).unwrap();
    let err = app
        .record_uncertainty("t", "U-1", "again", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("already recorded"), "got: {err}");
}

#[test]
fn uncertainty_resolved_requires_evidence() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "needs proof", None)
        .unwrap();
    let err = app
        .record_uncertainty_disposition("t", "U-1", "resolved", None, None, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("'resolved' requires evidence"), "got: {err}");
}

#[test]
fn uncertainty_resolved_binds_hashed_evidence_and_tracks_freshness() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "proven?", None).unwrap();
    let ev = app.project_root.join("src").join("ev.txt");
    std::fs::write(&ev, b"PASS exit=0").unwrap();
    app.record_uncertainty_disposition("t", "U-1", "resolved", Some("src/ev.txt"), None, None)
        .unwrap();
    let state = app.get_status("t").unwrap();
    let u = &state.uncertainties[0];
    assert_eq!(u.status.as_str(), "resolved");
    // ctl computes the hash from the path; it equals hashing the file directly.
    assert_eq!(
        u.evidence_ref.as_ref().unwrap().hash,
        hash_file(&ev).unwrap()
    );
    // Fresh immediately; never attested.
    let view = app.uncertainty_ledger_view(&state).unwrap();
    let evidence = view.items[0].evidence.as_ref().unwrap();
    assert_eq!(evidence.freshness.as_str(), "CURRENT");
    assert!(!evidence.attested);
    // Edited → STALE; deleted → ABSENT.
    std::fs::write(&ev, b"MUTATED").unwrap();
    let edited = app.uncertainty_ledger_view(&state).unwrap();
    assert_eq!(
        edited.items[0]
            .evidence
            .as_ref()
            .unwrap()
            .freshness
            .as_str(),
        "STALE"
    );
    std::fs::remove_file(&ev).unwrap();
    let removed = app.uncertainty_ledger_view(&state).unwrap();
    assert_eq!(
        removed.items[0]
            .evidence
            .as_ref()
            .unwrap()
            .freshness
            .as_str(),
        "ABSENT"
    );
}

#[test]
fn uncertainty_assumption_rejects_evidence_stays_unresolved() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "assume it", None)
        .unwrap();
    std::fs::write(app.project_root.join("src").join("ev.txt"), b"x").unwrap();
    let err = app
        .record_uncertainty_disposition(
            "t",
            "U-1",
            "accepted_as_assumption",
            Some("src/ev.txt"),
            None,
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("must not carry evidence"), "got: {err}");
    // Without evidence it stays visibly unresolved by external evidence.
    app.record_uncertainty_disposition(
        "t",
        "U-1",
        "accepted_as_assumption",
        None,
        None,
        Some("ship"),
    )
    .unwrap();
    let state = app.get_status("t").unwrap();
    assert_eq!(
        state.uncertainties[0].status.as_str(),
        "accepted_as_assumption"
    );
    assert!(state.uncertainties[0].evidence_ref.is_none());
}

#[test]
fn uncertainty_invalidated_requires_reason_rejects_evidence() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "moot?", None).unwrap();
    let no_reason = app
        .record_uncertainty_disposition("t", "U-1", "invalidated", None, None, None)
        .unwrap_err()
        .to_string();
    assert!(no_reason.contains("requires a reason"), "got: {no_reason}");
    app.record_uncertainty_disposition(
        "t",
        "U-1",
        "invalidated",
        None,
        None,
        Some("premise gone"),
    )
    .unwrap();
    let state = app.get_status("t").unwrap();
    assert_eq!(state.uncertainties[0].status.as_str(), "invalidated");
    assert_eq!(
        state.uncertainties[0].reason.as_deref(),
        Some("premise gone")
    );
}

#[test]
fn uncertainty_disposition_is_terminal() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "assume then upgrade?", None)
        .unwrap();
    app.record_uncertainty_disposition("t", "U-1", "accepted_as_assumption", None, None, None)
        .unwrap();
    std::fs::write(app.project_root.join("src").join("ev.txt"), b"x").unwrap();
    // Silent upgrade assumption → resolved is impossible: terminal-is-terminal.
    let err = app
        .record_uncertainty_disposition("t", "U-1", "resolved", Some("src/ev.txt"), None, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("is terminal"), "got: {err}");
}

#[test]
fn uncertainty_disposition_unknown_id_rejected() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    let err = app
        .record_uncertainty_disposition("t", "U-X", "accepted_as_assumption", None, None, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown uncertainty"), "got: {err}");
}

// ── Oracle V1: first-class, oracle-typed evidence ──
// Evidence becomes a recorded object carrying oracle_kind; a `resolved` can
// reference it by id. The control layer discloses the oracle kind; it never
// vouches for the claim. `model` is advisory; legacy inline replay is preserved.

#[test]
fn evidence_recorded_then_resolve_via_ref_binds_oracle_kind() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "proven by a test?", None)
        .unwrap();
    let ev = app.project_root.join("src").join("test-log.txt");
    std::fs::write(&ev, b"running 1 test ... ok").unwrap();
    app.record_evidence(
        "t",
        "E-1",
        "test",
        Some("cargo test oracle"),
        "src/test-log.txt",
    )
    .unwrap();
    app.record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-1"), None)
        .unwrap();
    let state = app.get_status("t").unwrap();
    let u = &state.uncertainties[0];
    assert_eq!(u.status.as_str(), "resolved");
    assert_eq!(u.evidence_id.as_deref(), Some("E-1"));
    assert_eq!(u.oracle_kind.unwrap().as_str(), "test");
    // The evidence's artifact is copied onto the uncertainty so freshness resolves
    // uniformly with the legacy inline path.
    assert_eq!(
        u.evidence_ref.as_ref().unwrap().hash,
        hash_file(&ev).unwrap()
    );
    // recorded_by is the envelope actor, never a separate forgeable field.
    assert_eq!(state.evidences[0].recorded_by, app.actor);
    let view = app.uncertainty_ledger_view(&state).unwrap();
    assert_eq!(view.oracle_sources.test, 1);
    assert_eq!(view.items[0].oracle_kind.as_deref(), Some("test"));
    assert!(!view.items[0].advisory);
}

#[test]
fn resolve_via_unknown_evidence_ref_rejected() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "needs proof", None)
        .unwrap();
    let err = app
        .record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-404"), None)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("does not reference a recorded evidence"),
        "got: {err}"
    );
}

#[test]
fn resolve_with_both_evidence_ref_and_inline_rejected() {
    // Critic C1: a resolve must never carry both evidence shapes. The app-layer
    // guard rejects before any hashing; the schema and reducer also forbid it.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "double-bound?", None)
        .unwrap();
    let ev = app.project_root.join("src").join("ev.txt");
    std::fs::write(&ev, b"x").unwrap();
    app.record_evidence("t", "E-1", "deterministic", None, "src/ev.txt")
        .unwrap();
    let err = app
        .record_uncertainty_disposition(
            "t",
            "U-1",
            "resolved",
            Some("src/ev.txt"),
            Some("E-1"),
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("never both"), "got: {err}");
}

#[test]
fn evidence_duplicate_id_rejected() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    let ev = app.project_root.join("src").join("ev.txt");
    std::fs::write(&ev, b"x").unwrap();
    app.record_evidence("t", "E-1", "deterministic", None, "src/ev.txt")
        .unwrap();
    let err = app
        .record_evidence("t", "E-1", "test", None, "src/ev.txt")
        .unwrap_err()
        .to_string();
    assert!(err.contains("already recorded"), "got: {err}");
}

#[test]
fn unknown_oracle_kind_rejected() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    let ev = app.project_root.join("src").join("ev.txt");
    std::fs::write(&ev, b"x").unwrap();
    // Schema rejects a value outside the fixed enum before the reducer runs.
    let err = app
        .record_evidence("t", "E-1", "vibes", None, "src/ev.txt")
        .unwrap_err()
        .to_string();
    assert!(
        err.to_lowercase().contains("oracle_kind") || err.contains("schema"),
        "got: {err}"
    );
}

#[test]
fn model_oracle_cannot_resolve_but_is_disclosed_advisory() {
    // EPISTEMIC_CONTROL §5.1: a model oracle is advisory — never external proof, so
    // it must not RESOLVE an uncertainty. It may still be recorded and discloses on
    // its own ORACLE SOURCES line. The command layer rejects a model-backed resolve.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "did a model say so?", None)
        .unwrap();
    let ev = app.project_root.join("src").join("model-note.md");
    std::fs::write(&ev, b"the model believes X").unwrap();
    app.record_evidence(
        "t",
        "E-1",
        "model",
        Some("BS-UO1 critic"),
        "src/model-note.md",
    )
    .unwrap();
    // The model-backed resolve is rejected at the command layer.
    let err = app
        .record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-1"), None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("model"), "unexpected error: {err}");
    assert!(
        err.contains("advisory") || err.contains("external proof"),
        "unexpected error: {err}"
    );
    // The uncertainty stays open; the model evidence is still recorded + disclosed.
    let state = app.get_status("t").unwrap();
    assert_eq!(state.uncertainties[0].status.as_str(), "open");
    let view = app.uncertainty_ledger_view(&state).unwrap();
    assert_eq!(view.oracle_sources.model_advisory, 1);
    assert!(!view.items[0].advisory);
}

#[test]
fn legacy_model_backed_resolve_still_replays_advisory() {
    // Preserve legacy replay: a pre-rule stream that resolved an uncertainty via a
    // model evidence_ref must still replay byte-identically (the reducer stays
    // permissive). Only the command layer forbids NEW model resolves, so this test
    // appends the resolve directly to simulate a committed pre-rule event. Replay
    // keeps it Resolved and the disclosure marks it ADVISORY (honest, never proof).
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "model-resolved long ago", None)
        .unwrap();
    let ev = app.project_root.join("src").join("model-note.md");
    std::fs::write(&ev, b"the model believes X").unwrap();
    app.record_evidence("t", "E-1", "model", None, "src/model-note.md")
        .unwrap();
    // Bypass the command-layer guard to mimic a stream written before the rule.
    let event = app
        .build_event(
            "t",
            "uncertainty_disposition_recorded",
            serde_json::json!({
                "uncertainty_id": "U-1",
                "disposition": "resolved",
                "evidence_ref": "E-1",
                "trust_level": crate::domain::task::UNCERTAINTY_TRUST_LEVEL,
            }),
        )
        .unwrap();
    app.validate_and_append(&event).unwrap();
    // The reducer accepted it on append AND on full replay.
    let state = app.replay_task("t").unwrap();
    let u = &state.uncertainties[0];
    assert_eq!(u.status.as_str(), "resolved");
    assert_eq!(u.oracle_kind, Some(crate::domain::task::OracleKind::Model));
    let view = app.uncertainty_ledger_view(&state).unwrap();
    assert!(view.items[0].advisory);
    assert_eq!(view.items[0].oracle_kind.as_deref(), Some("model"));
}

#[test]
fn research_artifact_usable_as_evidence_source() {
    // §六: a research/spike artifact can be referenced as the file-backed evidence
    // an uncertainty is resolved against (oracle_kind labels it; content stays L0).
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "what did the spike find?", None)
        .unwrap();
    let findings = app.project_root.join("src").join("findings.md");
    std::fs::write(&findings, b"# findings\nthe API is idempotent").unwrap();
    app.record_evidence(
        "t",
        "E-1",
        "external_authority",
        Some("research-spike findings.md"),
        "src/findings.md",
    )
    .unwrap();
    app.record_uncertainty_disposition("t", "U-1", "resolved", None, Some("E-1"), None)
        .unwrap();
    let state = app.get_status("t").unwrap();
    assert_eq!(state.uncertainties[0].status.as_str(), "resolved");
    assert_eq!(
        state.evidences[0].source_ref.as_deref(),
        Some("research-spike findings.md")
    );
}

#[test]
fn legacy_inline_resolve_still_replays_after_oracle_v1() {
    // Backward compatibility: the legacy inline (evidence_path+evidence_hash)
    // resolve shape — with no evidence_ref and no recorded evidence object — must
    // still replay unchanged. oracle_kind is unknown (None) for legacy resolves.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "legacy?", None).unwrap();
    let ev = app.project_root.join("src").join("legacy.txt");
    std::fs::write(&ev, b"legacy evidence").unwrap();
    app.record_uncertainty_disposition(
        "t",
        "U-1",
        "resolved",
        Some("src/legacy.txt"),
        None,
        None,
    )
    .unwrap();
    // Replay from canonical events rebuilds the same state.
    let state = app.replay_task("t").unwrap();
    let u = &state.uncertainties[0];
    assert_eq!(u.status.as_str(), "resolved");
    assert!(u.evidence_id.is_none());
    assert!(u.oracle_kind.is_none());
    assert!(u.evidence_ref.is_some());
    // No recorded evidence object; ORACLE SOURCES is all-zero.
    assert!(state.evidences.is_empty());
    let view = app.uncertainty_ledger_view(&state).unwrap();
    assert_eq!(
        view.oracle_sources.deterministic + view.oracle_sources.test,
        0
    );
}

#[test]
fn uncertainty_ledger_json_has_no_epistemic_verdict() {
    // §七.10: the JSON disclosure carries raw facts only — no verdict / score /
    // percentage / pass-fail roll-up of the epistemic dimension.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "t");
    app.record_uncertainty("t", "U-1", "open one", None)
        .unwrap();
    let ev = app.project_root.join("src").join("m.md");
    std::fs::write(&ev, b"model says").unwrap();
    app.record_evidence("t", "E-1", "model", None, "src/m.md")
        .unwrap();
    let state = app.get_status("t").unwrap();
    let view = app.uncertainty_ledger_view(&state).unwrap();
    let json = serde_json::to_string(&view).unwrap().to_lowercase();
    assert!(!json.contains("verdict"));
    assert!(!json.contains("score"));
    assert!(!json.contains("confidence"));
    assert!(!json.contains('%'));
    // It does carry the honest texture: the model oracle and its advisory flag.
    assert!(json.contains("model_advisory"));
    assert!(json.contains("\"advisory\""));
}

// ── Research/Spike V1 ──
// A research task completes by producing evidence + uncertainty outcomes, not
// code. Kind is immutable; completion never requires fewer unknowns.

fn seed_research_task(app: &ControlApp, id: &str) {
    let scope = vec!["src".to_string()];
    app.create_task_with_kind(
        id,
        CreateTaskInput {
            objective: "spike",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
        TaskKind::Research,
        AuditTier::Full,
    )
    .unwrap();
}

#[test]
fn task_kind_defaults_implementation_and_is_set_for_research() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "impl"); // uses create_task (default)
    seed_research_task(&app, "res");
    assert_eq!(
        app.get_status("impl").unwrap().task_kind,
        TaskKind::Implementation
    );
    assert_eq!(app.get_status("res").unwrap().task_kind, TaskKind::Research);
}

#[test]
fn task_kind_is_immutable_through_revise() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_research_task(&app, "res");
    app.revise_task(
        "res",
        ReviseTaskInput {
            objective: Some("revised spike"),
            read_scope: None,
            write_allow: None,
            write_deny: None,
            risk_triggers: None,
            gates: None,
            depends_on: None,
        },
    )
    .unwrap();
    // Revise changed the objective but never the kind.
    let state = app.get_status("res").unwrap();
    assert_eq!(state.objective.as_deref(), Some("revised spike"));
    assert_eq!(state.task_kind, TaskKind::Research);
}

#[test]
fn research_artifact_records_hash_kind_and_freshness() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_research_task(&app, "res");
    let path = app.project_root.join("src").join("findings.md");
    std::fs::write(&path, b"# findings").unwrap();
    app.record_research_artifact("res", "src/findings.md", "findings", Some("run-7"))
        .unwrap();
    let state = app.get_status("res").unwrap();
    assert_eq!(state.research_artifacts.len(), 1);
    let a = &state.research_artifacts[0];
    assert_eq!(a.artifact_ref.hash, hash_file(&path).unwrap());
    assert_eq!(a.kind.as_str(), "findings");
    // View: fresh + never attested; mutate → STALE.
    let view = app.research_output_view("res").unwrap().unwrap();
    assert_eq!(view.artifacts_recorded, 1);
    assert_eq!(view.artifacts[0].freshness.as_str(), "CURRENT");
    assert!(!view.artifacts[0].source_run_attested);
    std::fs::write(&path, b"MUTATED").unwrap();
    let stale = app.research_output_view("res").unwrap().unwrap();
    assert_eq!(stale.artifacts[0].freshness.as_str(), "STALE");
}

#[test]
fn research_artifact_unknown_kind_rejected() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_research_task(&app, "res");
    std::fs::write(app.project_root.join("src").join("x.md"), b"x").unwrap();
    let err = app
        .record_research_artifact("res", "src/x.md", "manifesto", None)
        .unwrap_err()
        .to_string();
    // Rejected by the schema enum (defense-in-depth before the reducer's own
    // unknown-kind guard); either way an unknown kind cannot be recorded.
    assert!(err.contains("artifact_kind"), "got: {err}");
}

#[test]
fn research_output_none_for_implementation_and_tags_discovered_items() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "impl");
    assert!(app.research_output_view("impl").unwrap().is_none());

    seed_research_task(&app, "res");
    // Recorded in Planning (before start) → pre-start.
    app.record_uncertainty("res", "U-pre", "known before start", None)
        .unwrap();
    app.mark_ready("res").unwrap();
    app.start_task("res").unwrap();
    // Recorded after start → tagged recorded_after_start.
    app.record_uncertainty("res", "U-post", "surfaced during spike", None)
        .unwrap();
    let view = app.research_output_view("res").unwrap().unwrap();
    assert_eq!(view.uncertainties_opened, 2);
    let pre = view
        .uncertainties
        .iter()
        .find(|u| u.item.id == "U-pre")
        .unwrap();
    let post = view
        .uncertainties
        .iter()
        .find(|u| u.item.id == "U-post")
        .unwrap();
    assert!(
        !pre.recorded_after_start,
        "pre-start uncertainty must not be tagged"
    );
    assert!(
        post.recorded_after_start,
        "post-start uncertainty must be tagged"
    );
}

fn drive_research_to_review(app: &ControlApp, id: &str) {
    seed_research_task(app, id);
    app.mark_ready(id).unwrap();
    app.start_task(id).unwrap();
    app.submit_task(id).unwrap();
    app.record_gate(id, "cargo_check", true, "ok").unwrap();
    ControlApp::open(&app.project_root, false)
        .unwrap()
        .with_actor("reviewer")
        .record_completion_audit(id, true, None)
        .unwrap();
}

#[test]
fn research_finish_requires_artifact_then_uncertainty_then_succeeds() {
    // Non-git temp dir → tree/commit interlocks skipped, isolating the
    // research-specific completion checks (which run after the M-f audit gate).
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_research_to_review(&app, "res");
    // No artifact yet.
    let e1 = app.finish_task("res").unwrap_err().to_string();
    assert!(e1.contains("research artifact"), "got: {e1}");
    // Artifact, but no uncertainty outcome.
    std::fs::write(app.project_root.join("src").join("f.md"), b"f").unwrap();
    app.record_research_artifact("res", "src/f.md", "findings", None)
        .unwrap();
    let e2 = app.finish_task("res").unwrap_err().to_string();
    assert!(e2.contains("uncertainty outcome"), "got: {e2}");
    // One recorded uncertainty satisfies the floor — even though it stays open
    // (completion never requires the open count to fall).
    app.record_uncertainty("res", "U-1", "still open", None)
        .unwrap();
    assert_eq!(app.finish_task("res").unwrap().event_type, "task_completed");
}

#[test]
fn research_artifact_rejected_on_implementation_task() {
    // Kind binding: an implementation task must never accrue a research
    // footprint it never declared. (Checked before any file hashing.)
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_uncertainty_task(&app, "impl"); // implementation kind
    let err = app
        .record_research_artifact("impl", "src/note.md", "findings", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("research task"), "got: {err}");
}

#[test]
fn research_artifact_rejected_out_of_scope() {
    // Scope binding: an artifact must sit inside the task's write_allow — the
    // same boundary the write gate enforces. Here write_allow = ["src"].
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    seed_research_task(&app, "res");
    std::fs::create_dir_all(app.project_root.join("docs")).unwrap();
    std::fs::write(app.project_root.join("docs").join("x.md"), b"x").unwrap();
    let err = app
        .record_research_artifact("res", "docs/x.md", "findings", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("write_allow"), "got: {err}");
}

#[test]
fn research_artifact_rejected_after_terminal() {
    // Terminal-is-terminal: a completed task's disclosed footprint must not
    // change after the fact.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_research_to_review(&app, "res");
    std::fs::write(app.project_root.join("src").join("f.md"), b"f").unwrap();
    app.record_research_artifact("res", "src/f.md", "findings", None)
        .unwrap();
    app.record_uncertainty("res", "U-1", "open", None).unwrap();
    assert_eq!(app.finish_task("res").unwrap().event_type, "task_completed");
    // Now Completed → no further artifacts.
    std::fs::write(app.project_root.join("src").join("g.md"), b"g").unwrap();
    let err = app
        .record_research_artifact("res", "src/g.md", "findings", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("terminal"), "got: {err}");
}

#[test]
fn research_finish_requires_current_artifact() {
    // A finish must point at an artifact that still matches what was recorded;
    // an artifact edited away after recording (STALE) must not satisfy it.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    drive_research_to_review(&app, "res");
    let path = app.project_root.join("src").join("f.md");
    std::fs::write(&path, b"original").unwrap();
    app.record_research_artifact("res", "src/f.md", "findings", None)
        .unwrap();
    app.record_uncertainty("res", "U-1", "open", None).unwrap();
    // Edit the artifact away → STALE → finish blocked on freshness.
    std::fs::write(&path, b"MUTATED").unwrap();
    let err = app.finish_task("res").unwrap_err().to_string();
    assert!(err.contains("CURRENT"), "got: {err}");
    // Re-record against the current file → a CURRENT artifact exists → proceeds.
    app.record_research_artifact("res", "src/f.md", "findings", None)
        .unwrap();
    assert_eq!(app.finish_task("res").unwrap().event_type, "task_completed");
}

// ── Run-ledger single-writer ──

#[test]
fn run_event_seq_is_assigned_authoritatively_under_lock() {
    // build_run_event emits a placeholder seq 0; append_run_event assigns the
    // real seq (max+1) inside the per-run lock. If that assignment regressed,
    // the run reducer would reject seq 0 ("Sequence error") and create_run
    // would fail — so a successful create with last_seq == 1 proves the fix.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let scope = vec!["src".to_string()];
    app.create_task(
        "t",
        CreateTaskInput {
            objective: "runs",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready("t").unwrap();
    app.start_task("t").unwrap();
    let run_id = app.create_run("t", "omp").unwrap();
    assert_eq!(app.replay_run(&run_id).unwrap().last_seq, 1);
}

// ── PRD plan / validate / status (workflow-prd-to-tasks-v1) ──

const CONFIRMED_PRD: &str = "# PRD: Demo\n\n> Status: confirmed\n\n## Objective\n\nShip it.\n\n## Tasks\n\n\
    - id: auth-task\n  objective: add auth boundary\n  write-allow: src/auth\n  gates: cargo_check, cargo_test\n\n\
    - id: config-task\n  objective: parse config\n  write-allow: src/config.rs\n  gates: cargo_check\n";

#[test]
fn prd_validate_clean_prd_has_no_errors() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
    let v = app.prd_validate(&doc).unwrap();
    assert!(v.ok(), "unexpected errors: {:?}", v.errors());
}

#[test]
fn prd_validate_catches_write_overlap() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // config-task writes into src/auth — overlaps auth-task's scope.
    let prd = CONFIRMED_PRD.replace("src/config.rs", "src/auth/sub.rs");
    let doc = crate::application::prd::parse_prd(&prd).unwrap();
    let v = app.prd_validate(&doc).unwrap();
    assert!(!v.ok(), "overlap must be an error");
    assert!(
        v.errors()
            .iter()
            .any(|p| p.message.contains("overlapping write-allow")),
        "{:?}",
        v.errors()
    );
}

#[test]
fn prd_validate_catches_unknown_gate() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let prd = CONFIRMED_PRD.replace("cargo_test", "bogus_gate");
    let doc = crate::application::prd::parse_prd(&prd).unwrap();
    let v = app.prd_validate(&doc).unwrap();
    assert!(!v.ok());
    assert!(
        v.errors()
            .iter()
            .any(|p| p.message.contains("Unknown gate")),
        "{:?}",
        v.errors()
    );
}
#[test]
fn prd_validate_warns_on_protected_write_allow() {
    // A protected path declared in write_allow is allowed (mirrors create/
    // revise) but surfaced as a non-blocking warning: the runtime gate
    // still requires a `ctl apply` exception before the path can be written.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let prd = CONFIRMED_PRD.replace("src/config.rs", "Cargo.toml");
    let doc = crate::application::prd::parse_prd(&prd).unwrap();
    let v = app.prd_validate(&doc).unwrap();
    assert!(
        v.ok(),
        "protected write_allow must not be an error: {:?}",
        v.errors()
    );
    assert!(
        v.warnings()
            .iter()
            .any(|p| { p.message.contains("protected") && p.message.contains("Cargo.toml") }),
        "expected a protected-path warning for Cargo.toml: {:?}",
        v.warnings()
    );
}
#[test]
fn format_gate_evidence_failed_gate_includes_stdout_preview() {
    // cargo writes the failing-test name + panic to STDOUT, not stderr. A
    // failed gate's recorded evidence must carry a stdout preview or the
    // failing test's identity is unrecoverable from the ledger.
    use crate::infrastructure::gates::GateRunResult;
    let r = GateRunResult {
        gate_id: "cargo_test".into(),
        passed: false,
        exit_code: 101,
        stdout: "running 540 tests\n\
                 test foo::bar::adds ... FAILED\n\
                 ---- foo::bar::adds stdout ----\n\
                 panicked at 'assertion failed', src/x.rs:10"
            .into(),
        stderr: "error: test failed, to rerun pass --bin ctl".into(),
        timed_out: false,
    };
    let ev = format_gate_evidence(&r);
    assert!(ev.starts_with("exit=101 "), "lead with exit code: {ev}");
    assert!(
        ev.contains("stdout="),
        "failed-gate evidence must include stdout: {ev}"
    );
    assert!(
        ev.contains("foo::bar"),
        "stdout preview must carry the failing test name: {ev}"
    );
    assert!(ev.contains("stderr="), "stderr must still be present: {ev}");
}

#[test]
fn format_gate_evidence_failed_stdout_keeps_tail_not_head() {
    // Real cargo shape (seen live): a long run of "… ok" lines, then the
    // failure summary + failing-test panic at the END. A head window shows
    // only "running 543 tests" + passing tests and buries the failure; the
    // evidence must keep the TAIL to surface the failing test's identity.
    use crate::infrastructure::gates::GateRunResult;
    let mut stdout = String::from("running 543 tests\n");
    for _ in 0..200 {
        stdout.push_str("test some::long::path::test_case_name ... ok\n");
    }
    stdout.push_str("test result: FAILED. 542 passed; 1 failed;\n");
    stdout.push_str("---- real::failing::test stdout ----\n");
    stdout.push_str("panicked at 'boom', src/real.rs:42\n");
    let r = GateRunResult {
        gate_id: "cargo_test".into(),
        passed: false,
        exit_code: 101,
        stdout,
        stderr: "error: test failed, to rerun pass --bin ctl".into(),
        timed_out: false,
    };
    let ev = format_gate_evidence(&r);
    assert!(
        ev.contains("real::failing::test"),
        "tail window must surface the failing test name: {ev}"
    );
    assert!(
        ev.contains("panicked at"),
        "tail window must include the panic: {ev}"
    );
    assert!(
        !ev.contains("running 543 tests"),
        "head must not crowd out the failure detail: {ev}"
    );
}

#[test]
fn truncate_tail_preview_is_char_boundary_safe() {
    let s = "é".repeat(600); // 1200 bytes; tail window 512 must back off
    let p = truncate_tail_preview(&s, 512);
    assert!(p.starts_with('…'));
    assert!(p.len() <= "…".len() + 512);
    assert_eq!(truncate_tail_preview("  hi  ", 512), "hi");
    assert_eq!(truncate_tail_preview("", 512), "");
}

#[test]
fn truncate_preview_never_splits_a_char_boundary() {
    // A 512-byte cut landing inside a multi-byte char must back off to a
    // boundary instead of panicking.
    let s = "é".repeat(600); // 2 bytes/char → 1200 bytes
    let p = truncate_preview(&s, 512);
    assert!(p.ends_with("..."));
    assert!(p.len() <= 512 + "...".len());
    assert_eq!(truncate_preview("  hi  ", 512), "hi");
    assert_eq!(truncate_preview("", 512), "");
}

#[test]
fn format_gate_evidence_passed_and_timed_out_shapes() {
    use crate::infrastructure::gates::GateRunResult;
    let pass = GateRunResult {
        gate_id: "cargo_check".into(),
        passed: true,
        exit_code: 0,
        stdout: "ignored".into(),
        stderr: String::new(),
        timed_out: false,
    };
    assert_eq!(format_gate_evidence(&pass), "exit=0 stdout=7B");
    let to = GateRunResult {
        gate_id: "cargo_test".into(),
        passed: false,
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
        timed_out: true,
    };
    assert!(format_gate_evidence(&to).contains("exit=timeout"));
}

#[test]
fn prd_plan_draft_refused_without_dry_run() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let prd = CONFIRMED_PRD.replace("confirmed", "draft");
    let doc = crate::application::prd::parse_prd(&prd).unwrap();
    let err = app
        .prd_plan(&doc, None, None, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("confirmed"), "{}", err);
}

#[test]
fn prd_plan_superseded_always_refused() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let prd = CONFIRMED_PRD.replace("confirmed", "superseded");
    let doc = crate::application::prd::parse_prd(&prd).unwrap();
    // Even dry-run refuses a superseded PRD.
    let err = app
        .prd_plan(&doc, None, None, true)
        .unwrap_err()
        .to_string();
    assert!(err.contains("superseded"), "{}", err);
}

#[test]
fn prd_plan_dry_run_creates_nothing() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
    let outcomes = app.prd_plan(&doc, None, None, true).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|o| !o.created));
    // Nothing persisted.
    assert!(app.get_status("auth-task").is_err());
}

#[test]
fn prd_plan_confirmed_creates_tasks_with_correct_boundaries() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
    let outcomes = app.prd_plan(&doc, None, None, false).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|o| o.created));

    let auth = app.get_status("auth-task").unwrap();
    assert_eq!(auth.objective.as_deref(), Some("add auth boundary"));
    assert!(auth.write_allow.contains("src/auth"));
    assert!(auth.gates.contains("cargo_check"));
    assert!(auth.gates.contains("cargo_test"));
    // read-scope defaulted to write-allow (absent in the PRD).
    assert_eq!(auth.read_scope, auth.write_allow);
}

#[test]
fn prd_plan_validation_failure_creates_nothing() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let prd = CONFIRMED_PRD.replace("cargo_test", "bogus_gate");
    let doc = crate::application::prd::parse_prd(&prd).unwrap();
    let err = app
        .prd_plan(&doc, None, None, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("validation failed"), "{}", err);
    // Validate runs before any create → no task exists.
    assert!(app.get_status("auth-task").is_err());
    assert!(app.get_status("config-task").is_err());
}

#[test]
fn prd_plan_wires_depends_on() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let prd = "# PRD: Deps\n\n> Status: confirmed\n\n## Tasks\n\n\
        - id: child\n  objective: depends on parent\n  write-allow: src/child\n  gates: cargo_check\n  depends-on: parent\n\n\
        - id: parent\n  objective: the base\n  write-allow: src/parent\n  gates: cargo_check\n";
    let doc = crate::application::prd::parse_prd(prd).unwrap();
    app.prd_plan(&doc, None, None, false).unwrap();
    let child = app.get_status("child").unwrap();
    assert!(child.depends_on.contains("parent"));
}

#[test]
fn prd_status_view_shows_not_created_then_planning() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();

    // Before planning: all tasks not-yet-created.
    let view = app.prd_status_view(&doc).unwrap();
    assert_eq!(view.total, 2);
    assert_eq!(view.completed, 0);
    assert!(view.rows.iter().all(|r| !r.exists));

    // After planning: tasks exist in Planning.
    app.prd_plan(&doc, None, None, false).unwrap();
    let view = app.prd_status_view(&doc).unwrap();
    assert!(view.rows.iter().all(|r| r.exists));
    assert!(
        view.rows
            .iter()
            .all(|r| r.phase.as_deref() == Some("planning")),
        "{:?}",
        view.rows
    );
}

#[test]
fn prd_plan_records_provenance_when_alignment_given() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // Write the alignment note + PRD file so provenance hashing succeeds.
    std::fs::write(dir.path().join("align.md"), "# alignment\n").unwrap();
    std::fs::write(dir.path().join("demo.md"), CONFIRMED_PRD).unwrap();
    let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
    let outcomes = app
        .prd_plan(&doc, Some("align.md"), Some("demo.md"), false)
        .unwrap();
    assert!(outcomes.iter().all(|o| o.provenance_recorded));

    // Provenance visible via the brainstorm view — convergence = the PRD.
    let state = app.get_status("auth-task").unwrap();
    let prov = app
        .brainstorm_provenance_view(&state)
        .expect("provenance was recorded");
    assert!(prov.convergence.is_some());
}

#[test]
fn prd_plan_without_alignment_skips_provenance() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let doc = crate::application::prd::parse_prd(CONFIRMED_PRD).unwrap();
    let outcomes = app.prd_plan(&doc, None, None, false).unwrap();
    assert!(outcomes.iter().all(|o| !o.provenance_recorded));
    let state = app.get_status("auth-task").unwrap();
    assert!(app.brainstorm_provenance_view(&state).is_none());
}

// ── Rich context injection (the data pipeline cmd_hook_context assembles) ──

#[test]
fn hook_context_enrichment_pipeline_surfaces_blockers_and_uncertainties() {
    // The hook enriches each in_progress task with drift/next-action,
    // blockers, open uncertainties, and provenance. This test exercises
    // the exact data pipeline — if any signal silently stops flowing, the
    // platform hooks render a context-blind model.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();

    // Create a prerequisite (left incomplete) + a dependent task.
    let scope = vec!["src".to_string()];
    app.create_task(
        "prereq",
        CreateTaskInput {
            objective: "prerequisite",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();

    app.create_task(
        "dependent",
        CreateTaskInput {
            objective: "depends on prereq",
            read_scope: &scope,
            write_allow: &scope,
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &["prereq".to_string()],
        },
    )
    .unwrap();

    // Record an open uncertainty on the dependent task.
    app.record_uncertainty("dependent", "U-1", "is the API stable?", None)
        .unwrap();

    let state = app.get_status("dependent").unwrap();

    // Blocker: prereq is not Completed → unmet.
    let unmet = app.unmet_dependencies("dependent").unwrap();
    assert_eq!(unmet, vec!["prereq"]);

    // Open uncertainty is visible in the ledger.
    let ledger = app
        .uncertainty_ledger_view(&state)
        .expect("uncertainty ledger present");
    assert_eq!(ledger.open, 1);
    assert_eq!(ledger.items[0].id, "U-1");
    assert_eq!(ledger.items[0].status, "open");

    // next_action computes (held by unmet-dep gate or drift — either way
    // it returns a valid proposal without error).
    let na = app.next_action("dependent").unwrap();
    assert!(!na.rationale.is_empty());

    // No provenance recorded → None (the hook skips this field gracefully).
    assert!(app.brainstorm_provenance_view(&state).is_none());
}

// ── next-task: deterministic scheduling recommendation ──

fn mk_ready_task(app: &ControlApp, id: &str, scope: &[&str]) {
    app.create_task(
        id,
        CreateTaskInput {
            objective: id,
            read_scope: &scope.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            write_allow: &scope.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
    app.mark_ready(id).unwrap();
}

#[test]
fn next_task_recommends_start_for_ready_unblocked_task() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_ready_task(&app, "alpha", &["src/alpha"]);
    let rec = app.next_task().unwrap();
    assert_eq!(rec.action, "start");
    assert_eq!(rec.task_id.as_deref(), Some("alpha"));
    assert_eq!(rec.ready_candidates, 1);
}

#[test]
fn next_task_picks_lowest_id_on_tie() {
    // Two ready tasks, both drift 0 (no telemetry) → deterministic tie-break
    // by task id ascending.
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_ready_task(&app, "zeta", &["src/zeta"]);
    mk_ready_task(&app, "alpha", &["src/alpha"]);
    let rec = app.next_task().unwrap();
    assert_eq!(rec.task_id.as_deref(), Some("alpha"));
}

#[test]
fn next_task_skips_ready_task_with_unsatisfied_dep() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // prereq stays in planning (never completed) → dep unsatisfied.
    app.create_task(
        "prereq",
        CreateTaskInput {
            objective: "prereq",
            read_scope: &["src".to_string()],
            write_allow: &["src".to_string()],
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
    app.create_task(
        "blocked",
        CreateTaskInput {
            objective: "blocked",
            read_scope: &["src/b".to_string()],
            write_allow: &["src/b".to_string()],
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &["prereq".to_string()],
        },
    )
    .unwrap();
    app.mark_ready("blocked").unwrap();

    let rec = app.next_task().unwrap();
    // blocked is ready but deps unsatisfied → not a start candidate.
    // No actionable ready task → falls back to planning (prereq).
    assert_eq!(rec.action, "ready");
    assert_eq!(rec.task_id.as_deref(), Some("prereq"));
}

#[test]
fn next_task_falls_back_to_planning_when_no_ready() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    // Only a planning task exists.
    app.create_task(
        "seed",
        CreateTaskInput {
            objective: "planning seed",
            read_scope: &["src".to_string()],
            write_allow: &["src".to_string()],
            write_deny: &[],
            risk_triggers: &[],
            gates: &["cargo_check".to_string()],
            depends_on: &[],
        },
    )
    .unwrap();
    let rec = app.next_task().unwrap();
    assert_eq!(rec.action, "ready");
    assert_eq!(rec.task_id.as_deref(), Some("seed"));
}

#[test]
fn next_task_none_when_everything_terminal() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let rec = app.next_task().unwrap();
    assert_eq!(rec.action, "none");
    assert!(rec.task_id.is_none());
}

#[test]
fn next_task_skips_ready_task_conflicting_with_active_scope() {
    // An in_progress task on src/shared blocks a ready task on src/shared
    let dir = TempDir::new();
    std::fs::create_dir_all(dir.path().join("src/shared/sub")).unwrap();
    std::fs::create_dir_all(dir.path().join("src/other")).unwrap();
    let app = ControlApp::init(dir.path()).unwrap();
    mk_ready_task(&app, "active-one", &["src/shared"]);
    app.start_task("active-one").unwrap();
    // This ready task overlaps src/shared → skipped.
    mk_ready_task(&app, "conflicting", &["src/shared/sub"]);
    // This ready task is disjoint → recommended.
    mk_ready_task(&app, "safe", &["src/other"]);
    let rec = app.next_task().unwrap();
    assert_eq!(rec.action, "start");
    assert_eq!(rec.task_id.as_deref(), Some("safe"));
}

// ── Spec fact store (knowledge-accumulation-v1) ──

#[test]
fn spec_fact_add_assigns_sequential_ids_and_persists() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let f1 = app
        .spec_fact_add(
            "normalizer canonicalizes parent",
            "src/norm.rs:77",
            Some("boundary"),
        )
        .unwrap();
    assert_eq!(f1.fact_id, "F-001");
    let f2 = app
        .spec_fact_add("reducer is pure", "src/domain/task.rs:871", Some("domain"))
        .unwrap();
    assert_eq!(f2.fact_id, "F-002");
    // File persists.
    assert!(dir.path().join(".ctl/facts.jsonl").exists());
}

#[test]
fn spec_fact_add_rejects_empty_statement_or_source() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    assert!(app.spec_fact_add("", "src/x", None).is_err());
    assert!(app.spec_fact_add("a fact", "", None).is_err());
}

#[test]
fn spec_fact_list_filters_by_category_and_search() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    app.spec_fact_add(
        "normalizer canonicalizes parent",
        "src/norm.rs",
        Some("boundary"),
    )
    .unwrap();
    app.spec_fact_add("reducer is pure", "src/task.rs", Some("domain"))
        .unwrap();
    app.spec_fact_add("cli uses anyhow full path", "src/cli/mod.rs", Some("cli"))
        .unwrap();

    // By category.
    let boundary = app.spec_fact_list(Some("boundary"), None).unwrap();
    assert_eq!(boundary.len(), 1);
    assert_eq!(boundary[0].fact_id, "F-001");

    // By search.
    let hits = app.spec_fact_list(None, Some("canonicalizes")).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].fact_id, "F-001");

    // No filter → all.
    assert_eq!(app.spec_fact_list(None, None).unwrap().len(), 3);
}

#[test]
fn spec_facts_digest_summarizes_for_context_injection() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    app.spec_fact_add("fact a", "s", Some("boundary")).unwrap();
    app.spec_fact_add("fact b", "s", Some("domain")).unwrap();
    app.spec_fact_add("fact c", "s", Some("boundary")).unwrap();

    let digest = app.spec_facts_digest().unwrap();
    assert_eq!(digest.total, 3);
    assert_eq!(digest.categories.get("boundary"), Some(&2));
    assert_eq!(digest.categories.get("domain"), Some(&1));
    // Most recent first.
    assert_eq!(digest.recent[0].fact_id, "F-003");
}

#[test]
fn spec_fact_promote_appends_to_spec_markdown() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let fact = app
        .spec_fact_add("a gotcha about paths", "src/norm.rs:77", Some("gotcha"))
        .unwrap();

    // Create the target spec file so canonicalize succeeds.
    let spec_dir = dir.path().join(".ctl/spec/backend");
    std::fs::create_dir_all(&spec_dir).unwrap();
    let target = spec_dir.join("error-handling.md");
    std::fs::write(&target, "# Error Handling\n").unwrap();

    let path = app
        .spec_fact_promote(&fact.fact_id, "backend/error-handling.md")
        .unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("Fact F-001"));
    assert!(content.contains("category: gotcha"));
    assert!(content.contains("a gotcha about paths"));
    assert!(content.contains("src/norm.rs:77"));
}

#[test]
fn spec_fact_promote_rejects_unknown_id() {
    let dir = TempDir::new();
    let app = ControlApp::init(dir.path()).unwrap();
    let err = app
        .spec_fact_promote("F-999", "backend/x.md")
        .unwrap_err()
        .to_string();
    assert!(err.contains("not found"), "{}", err);
}
