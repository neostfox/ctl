use super::{
    classify_bash, classify_write_target, decision_entry, detect_shared_git_op, ellipsize,
    format_brainstorm_provenance, format_decision_line, format_decisions, format_research_output,
    format_uncertainty_ledger, is_cargo_target_build, iso8601_utc_to_epoch, omp_agent_env_file,
    parse_project_default_gates, resolve_active_governance, resolve_ctl_for_hook, upsert_env_line,
    wrapup_pending, ActiveTask, CtlProbe, CtlReach, GovState, WriteTarget,
};
use std::path::{Path, PathBuf};

// ── wrap-up check (Stop-hook reminder) ──

#[test]
fn iso8601_parse_matches_known_epochs() {
    assert_eq!(iso8601_utc_to_epoch("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(
        iso8601_utc_to_epoch("2026-07-04T05:05:35Z"),
        Some(1_783_141_535)
    );
    // Fractional seconds tolerated, malformed rejected.
    assert_eq!(iso8601_utc_to_epoch("1970-01-01T00:00:01.500Z"), Some(1));
    assert_eq!(iso8601_utc_to_epoch("not a date"), None);
    assert_eq!(iso8601_utc_to_epoch("2026-13-01T00:00:00Z"), None);
}

#[test]
fn wrapup_pending_policy() {
    // No capture ever → pending.
    assert!(wrapup_pending(100, None, None));
    // Capture before the finish → still pending.
    assert!(wrapup_pending(100, Some(99), None));
    // Capture at-or-after the finish → cleared.
    assert!(!wrapup_pending(100, Some(100), None));
    assert!(!wrapup_pending(100, Some(101), None));
    // Already reminded for THIS finish → never pending twice.
    assert!(!wrapup_pending(100, None, Some(100)));
    // A reminder for an older finish does not suppress a newer one.
    assert!(wrapup_pending(200, Some(150), Some(100)));
}

// ── classify_write_target (gate observe mode) ──
// Purely lexical, so no files are created; the root just needs to be an
// absolute path that exists as a string on every platform.

fn classify_root() -> PathBuf {
    std::env::temp_dir().join("ctl_gate_classify_root")
}

#[test]
fn write_target_in_repo_relative_and_absolute() {
    let root = classify_root();
    assert!(matches!(
        classify_write_target(&root, "src/cli/mod.rs"),
        WriteTarget::InRepo
    ));
    let abs = root.join("src").join("cli").join("mod.rs");
    assert!(matches!(
        classify_write_target(&root, &abs.to_string_lossy()),
        WriteTarget::InRepo
    ));
}

#[test]
fn write_target_out_of_repo_is_classified_not_denied() {
    let root = classify_root();
    // temp_dir itself is absolute and NOT under the (deeper) root.
    let outside = std::env::temp_dir().join("elsewhere").join("CLAUDE.md");
    assert!(matches!(
        classify_write_target(&root, &outside.to_string_lossy()),
        WriteTarget::OutOfRepo
    ));
}

#[test]
fn write_target_protected_paths_are_flagged() {
    let root = classify_root();
    for p in [
        "Cargo.toml",
        "Cargo.lock",
        "schemas/x.json",
        ".git/config",
        ".ctl/tasks/t/events.jsonl",
    ] {
        assert!(
            matches!(classify_write_target(&root, p), WriteTarget::Protected(_)),
            "{p} must classify as protected"
        );
    }
    // Absolute form of a protected path is protected too.
    let abs = root.join("Cargo.toml");
    assert!(matches!(
        classify_write_target(&root, &abs.to_string_lossy()),
        WriteTarget::Protected(_)
    ));
}

#[test]
fn write_target_ctl_carveouts_stay_writable() {
    // The normalizer carve-outs from the blanket `.ctl` protection must
    // survive the gate-side check (spec is exempted earlier, config here).
    let root = classify_root();
    assert!(matches!(
        classify_write_target(&root, ".ctl/config.toml"),
        WriteTarget::InRepo
    ));
}

#[test]
fn write_target_traversal_and_unc_are_suspicious() {
    let root = classify_root();
    assert!(matches!(
        classify_write_target(&root, "../outside.txt"),
        WriteTarget::Suspicious(_)
    ));
    assert!(matches!(
        classify_write_target(&root, "//server/share/x"),
        WriteTarget::Suspicious(_)
    ));
    assert!(matches!(
        classify_write_target(&root, ""),
        WriteTarget::Suspicious(_)
    ));
}

fn at(id: &str, paths: &[&str], held: bool) -> ActiveTask {
    ActiveTask {
        task_id: id.to_string(),
        write_allow: paths.iter().map(|s| s.to_string()).collect(),
        is_held: held,
        approved_actions: Vec::new(),
        approved_apply_paths: Vec::new(),
    }
}

#[test]
fn no_active_tasks_yields_none() {
    assert!(resolve_active_governance(&[], None).is_none());
    assert!(resolve_active_governance(&[], Some("anything")).is_none());
}

#[test]
fn single_write_task_binds_without_token() {
    let active = vec![at("a", &["src"], false)];
    match resolve_active_governance(&active, None) {
        Some(GovState::InProgress { task_id, .. }) => assert_eq!(task_id, "a"),
        other => panic!("expected InProgress(a), got {other:?}"),
    }
}

#[test]
fn two_write_tasks_without_binding_fail_closed() {
    // M-a: ambiguous write governance → MultipleActive (fail closed).
    let active = vec![at("a", &["src"], false), at("b", &["docs"], false)];
    match resolve_active_governance(&active, None) {
        Some(GovState::MultipleActive { task_ids }) => {
            assert_eq!(task_ids, vec!["a".to_string(), "b".to_string()]);
        }
        other => panic!("expected MultipleActive, got {other:?}"),
    }
}

#[test]
fn dispatch_token_binds_amid_multiple_active() {
    // M-e: an explicit binding to one of the active write tasks resolves the
    // ambiguity that would otherwise fail closed.
    let active = vec![at("a", &["src"], false), at("b", &["docs"], false)];
    match resolve_active_governance(&active, Some("b")) {
        Some(GovState::InProgress {
            task_id,
            write_allow,
            ..
        }) => {
            assert_eq!(task_id, "b");
            assert_eq!(write_allow, vec!["docs".to_string()]);
        }
        other => panic!("expected InProgress(b), got {other:?}"),
    }
}

#[test]
fn stale_dispatch_token_is_ignored_not_honored() {
    // A token naming no active task must NOT widen scope; fall back to the
    // unbound M-a scan (here: still ambiguous → fail closed).
    let active = vec![at("a", &["src"], false), at("b", &["docs"], false)];
    match resolve_active_governance(&active, Some("ghost")) {
        Some(GovState::MultipleActive { task_ids }) => assert_eq!(task_ids.len(), 2),
        other => panic!("expected MultipleActive, got {other:?}"),
    }
}

#[test]
fn binding_to_held_task_stays_held() {
    // Binding to a held task surfaces the hold (gate then blocks); the token
    // cannot launder a held task into a writable one.
    let active = vec![at("a", &["src"], false), at("b", &["docs"], true)];
    match resolve_active_governance(&active, Some("b")) {
        Some(GovState::InProgress {
            task_id, is_held, ..
        }) => {
            assert_eq!(task_id, "b");
            assert!(is_held, "bound held task must remain held");
        }
        other => panic!("expected InProgress(b, held), got {other:?}"),
    }
}

#[test]
fn readonly_active_tasks_do_not_create_ambiguity() {
    // Only write-scoped tasks compete for write governance.
    let active = vec![at("w", &["src"], false), at("r", &[], false)];
    match resolve_active_governance(&active, None) {
        Some(GovState::InProgress { task_id, .. }) => assert_eq!(task_id, "w"),
        other => panic!("expected InProgress(w), got {other:?}"),
    }
}

#[test]
fn binding_to_readonly_task_governs_as_readonly() {
    // M-e binds to the *dispatching* task even if it is read-only: its empty
    // write_allow then denies writes, exactly as that task should.
    let active = vec![at("w", &["src"], false), at("r", &[], false)];
    match resolve_active_governance(&active, Some("r")) {
        Some(GovState::InProgress {
            task_id,
            write_allow,
            ..
        }) => {
            assert_eq!(task_id, "r");
            assert!(write_allow.is_empty());
        }
        other => panic!("expected InProgress(r), got {other:?}"),
    }
}

#[test]
fn approved_apply_paths_propagate_through_binding() {
    // M-f ctl apply: the granted out-of-scope paths must survive the
    // collect→resolve→bind step so the gate can honor the exception.
    let mut t = at("a", &["src"], false);
    t.approved_apply_paths = vec!["docs/x.md".to_string()];
    match resolve_active_governance(&[t], None) {
        Some(GovState::InProgress {
            approved_apply_paths,
            ..
        }) => assert_eq!(approved_apply_paths, vec!["docs/x.md".to_string()]),
        other => panic!("expected InProgress with apply paths, got {other:?}"),
    }
}

#[test]
fn simple_commands_classify_directly() {
    assert_eq!(classify_bash("git push origin master"), "git_push");
    assert_eq!(classify_bash("git commit -m x"), "git_commit");
    assert_eq!(classify_bash("cargo add serde"), "cargo_deps");
    // Self-install of the local crate is build-tier, not a dep change;
    // registry installs remain deps (supply chain).
    assert_eq!(
        classify_bash("cargo install --path . --force"),
        "cargo_build"
    );
    assert_eq!(classify_bash("cargo install ripgrep"), "cargo_deps");
    // Quoted spans are data: the live false positive — a commit MESSAGE
    // containing "(cargo install /" — must classify as the commit it is,
    // not as a dependency change.
    assert_eq!(
        classify_bash(
            r#"git add -A && git commit -m "install story (cargo install / GitHub release).""#
        ),
        "git_commit"
    );
    assert_eq!(classify_bash("echo 'git push origin'"), "bash_other");
    // Unquoted substitution still classifies (casual composition caught).
    assert_eq!(classify_bash("echo $(git push)"), "git_push");
    assert_eq!(classify_bash("cargo test"), "cargo_build");
    assert_eq!(classify_bash("ls -la"), "bash_other");
}

#[test]
fn compound_commands_use_most_restrictive_segment() {
    // The loophole: a benign prefix must not let a restricted action ride in.
    assert_eq!(
        classify_bash("cp a b && git push origin master"),
        "git_push"
    );
    assert_eq!(classify_bash("echo hi; git commit -m x"), "git_commit");
    assert_eq!(classify_bash("ls | cargo add serde"), "cargo_deps");
    assert_eq!(classify_bash("git commit -m x && git push"), "git_push");
    assert_eq!(classify_bash("true || cargo install foo"), "cargo_deps");
}

#[test]
fn subshell_and_backtick_segments_are_scanned() {
    assert_eq!(classify_bash("echo $(git push)"), "git_push");
    assert_eq!(classify_bash("x=`git push`"), "git_push");
}

#[test]
fn write_commands_classify_as_bash_write() {
    // Output redirection to a file, and common file-mutating commands, are
    // reclassified so the gate can require an active task. Best-effort only.
    assert_eq!(classify_bash("echo hi > src/x.rs"), "bash_write");
    assert_eq!(classify_bash("cat a >> b.txt"), "bash_write");
    assert_eq!(classify_bash("tee out.txt"), "bash_write");
    assert_eq!(classify_bash("cp a b"), "bash_write");
    assert_eq!(classify_bash("mv a b"), "bash_write");
    assert_eq!(classify_bash("sed -i s/a/b/ f"), "bash_write");
    assert_eq!(classify_bash("dd if=a of=b"), "bash_write");
    assert_eq!(classify_bash("ln -s a b"), "bash_write");
}

#[test]
fn read_only_bash_is_not_a_write() {
    // No file redirect and a non-mutating verb stays bash_other; fd-duplication
    // (`2>&1`) targets no file and must not be mistaken for a write.
    assert_eq!(classify_bash("grep foo bar"), "bash_other");
    assert_eq!(classify_bash("cat src/x.rs"), "bash_other");
    assert_eq!(classify_bash("ls -la 2>&1"), "bash_other");
    assert_eq!(classify_bash("echo hello"), "bash_other");
}

#[test]
fn restricted_verbs_outrank_bash_write_but_write_outranks_build() {
    // A write composed with a stricter action keeps the stricter class…
    assert_eq!(classify_bash("cp a b && git push"), "git_push");
    assert_eq!(classify_bash("echo x > f && cargo add serde"), "cargo_deps");
    assert_eq!(classify_bash("echo x > f ; git commit -m y"), "git_commit");
    // …but a write outranks a plain build (a write needs an active task; a
    // build is allowed even when idle).
    assert_eq!(classify_bash("cp a b && cargo test"), "bash_write");
    // A redirect appended to a cargo/git command is NOT reclassified (kept
    // conservative so `cargo test > log` stays buildable when idle).
    assert_eq!(classify_bash("cargo test > log.txt"), "cargo_build");
}

#[test]
fn detect_shared_git_op_flags_destructive_verbs() {
    assert_eq!(
        detect_shared_git_op("git checkout main"),
        Some("git checkout")
    );
    assert_eq!(
        detect_shared_git_op("git switch feature"),
        Some("git switch")
    );
    assert_eq!(
        detect_shared_git_op("git reset --hard HEAD~1"),
        Some("git reset")
    );
    assert_eq!(detect_shared_git_op("git rebase main"), Some("git rebase"));
    assert_eq!(detect_shared_git_op("git clean -dfx"), Some("git clean"));
}

#[test]
fn detect_shared_git_op_branch_only_on_destructive_flags() {
    // List / create are safe; delete / move / force rewrite shared refs.
    assert_eq!(detect_shared_git_op("git branch"), None);
    assert_eq!(detect_shared_git_op("git branch new-feature"), None);
    assert_eq!(
        detect_shared_git_op("git branch -D old"),
        Some("git branch -D")
    );
    assert_eq!(
        detect_shared_git_op("git branch --delete old"),
        Some("git branch -D")
    );
    assert_eq!(
        detect_shared_git_op("git branch -M renamed"),
        Some("git branch -D")
    );
}

#[test]
fn detect_shared_git_op_ignores_safe_git_and_non_git() {
    assert_eq!(detect_shared_git_op("git status"), None);
    assert_eq!(detect_shared_git_op("git log --oneline"), None);
    assert_eq!(detect_shared_git_op("git commit -m x"), None);
    assert_eq!(detect_shared_git_op("git push origin main"), None);
    assert_eq!(detect_shared_git_op("ls -la"), None);
    // A substring is not a verb: `checkout` must be the git subcommand.
    assert_eq!(detect_shared_git_op("echo git checkout is dangerous"), None);
}

#[test]
fn detect_shared_git_op_scans_compound_and_substitution() {
    // A destructive verb buried in a compound / substitution is still caught.
    assert_eq!(
        detect_shared_git_op("cp a b && git reset --hard"),
        Some("git reset")
    );
    assert_eq!(
        detect_shared_git_op("echo hi; git clean -f"),
        Some("git clean")
    );
    assert_eq!(
        detect_shared_git_op("x=$(git checkout main)"),
        Some("git checkout")
    );
    assert_eq!(
        detect_shared_git_op("y=`git rebase main`"),
        Some("git rebase")
    );
}

#[test]
fn prd_template_holds_the_parseable_convention() {
    // A later `prd plan` parser depends on these section headers and the
    // per-task field keys — pin them so the template can't silently drift.
    let t = super::PRD_TEMPLATE.replace("{title}", "Demo");
    assert!(t.starts_with("# PRD: Demo"), "title substituted");
    for needle in [
        "## Objective",
        "## Context",
        "## Tasks",
        "- id:",
        "objective:",
        "write-allow:",
        "gates:",
        "read-scope:",
        "depends-on:",
        "Status: draft",
    ] {
        assert!(t.contains(needle), "template must contain {needle:?}");
    }
    assert!(
        !t.contains("{title}"),
        "no unsubstituted placeholder remains"
    );
}

#[test]
fn architecture_check_registry_is_well_formed() {
    // `check` (fail-fast) and `review` (run-all) share this registry, so its
    // shape is the contract: a fixed, uniquely-named set of checks.
    let checks = super::architecture_checks();
    assert_eq!(checks.len(), 10, "all architecture checks registered");
    let mut names: Vec<&str> = checks.iter().map(|(n, _)| *n).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 10, "check names are unique");
}

#[test]
fn brainstorm_provenance_never_renders_as_independent() {
    // Behavior 8: the status block discloses `unattested` and never renders
    // it as an independence claim or a pass/green marker.
    use crate::domain::task::{ArtifactStatus, BrainstormProvenanceView};
    let view = BrainstormProvenanceView {
        id: "BS-001".into(),
        divergence: Some(ArtifactStatus {
            path: "d".into(),
            present: true,
            stale: false,
            recorded_hash: "h".into(),
        }),
        convergence: Some(ArtifactStatus {
            path: "c".into(),
            present: true,
            stale: true,
            recorded_hash: "h".into(),
        }),
        critic: None,
        critic_disposition: "absent".into(),
        critic_independence: "unattested".into(),
        trust_level: "content_l0".into(),
        source_run_id: Some("run-1".into()),
        source_run_attested: false,
        recorded_by: "human".into(),
        skip_reason: None,
        skip_decided_by: None,
    };
    let out = format_brainstorm_provenance(&view);
    // Discloses unattested independence verbatim...
    assert!(out.contains("critic independence: unattested"));
    // ...and never as an independence claim or a pass/green marker.
    assert!(!out.to_lowercase().contains("independent"));
    assert!(!out.contains("PASS"));
    assert!(!out.contains('✅'));
    // Staleness is surfaced; source-run attestation disclosed unavailable.
    assert!(out.contains("convergence artifact: present (STALE"));
    assert!(out.contains("attestation: unavailable"));
    // L0 content trust disclosed, never elevated.
    assert!(out.contains("trust level: content_l0"));
}

#[test]
fn uncertainty_ledger_discloses_facts_never_a_verdict() {
    // The ledger shows raw per-status counts and freshness, never a roll-up
    // verdict, score, percentage, or green marker.
    use crate::domain::task::{
        EvidenceFreshness, EvidenceView, OracleSourcesView, UncertaintyItemView,
        UncertaintyLedgerView,
    };
    let view = UncertaintyLedgerView {
        open: 2,
        accepted_as_assumption: 1,
        resolved: 1,
        invalidated: 1,
        trust_level: "content_l0".into(),
        oracle_sources: OracleSourcesView {
            model_advisory: 1,
            ..Default::default()
        },
        items: vec![UncertaintyItemView {
            id: "U-1".into(),
            statement: "is the evidence external?".into(),
            status: "resolved".into(),
            source: Some("review".into()),
            evidence: Some(EvidenceView {
                path: "tests/x.rs".into(),
                recorded_hash: "abc123".into(),
                freshness: EvidenceFreshness::Stale,
                attested: false,
            }),
            evidence_id: Some("E-1".into()),
            oracle_kind: Some("model".into()),
            advisory: true,
            reason: None,
        }],
    };
    let out = format_uncertainty_ledger(&view);
    // Raw counts disclosed verbatim...
    assert!(out.contains("open: 2"));
    assert!(out.contains("resolved with evidence: 1"));
    // ...the oracle-source breakdown keeps a model on its own advisory line...
    assert!(out.contains("ORACLE SOURCES"));
    assert!(out.contains("model advisory: 1"));
    // ...a model-backed resolve is explicitly marked advisory, not external proof...
    assert!(out.contains("evidence_ref: E-1"));
    assert!(out.contains("ADVISORY (not external proof)"));
    // ...freshness is file-consistency only, attestation unavailable...
    assert!(out.contains("freshness: STALE"));
    assert!(out.contains("attestation: unavailable"));
    assert!(out.contains("evidence: unattested"));
    assert!(out.contains("trust level: content_l0"));
    // ...and never a verdict, score, percentage, or green marker.
    assert!(!out.contains('✅'));
    assert!(!out.contains('%'));
    assert!(!out.to_uppercase().contains("PASS"));
    assert!(!out.to_lowercase().contains("verdict"));
}

#[test]
fn research_output_discloses_facts_no_verdict_no_discovered_scalar() {
    // RESEARCH OUTPUT shows raw counts, artifact freshness, and a per-item
    // recorded-after-start tag — never a verdict, score, or "discovered" count.
    use crate::domain::task::{
        EvidenceFreshness, ResearchArtifactView, ResearchOutputView, ResearchUncertaintyView,
        UncertaintyItemView,
    };
    let view = ResearchOutputView {
        artifacts_recorded: 1,
        uncertainties_opened: 2,
        resolved_with_evidence: 0,
        accepted_as_assumptions: 0,
        invalidated: 0,
        trust_level: "content_l0".into(),
        artifacts: vec![ResearchArtifactView {
            path: "research/x/findings.md".into(),
            recorded_hash: "abc".into(),
            kind: "findings".into(),
            freshness: EvidenceFreshness::Current,
            source_run_id: Some("run-1".into()),
            source_run_attested: false,
        }],
        uncertainties: vec![
            ResearchUncertaintyView {
                item: UncertaintyItemView {
                    id: "U-1".into(),
                    statement: "surfaced".into(),
                    status: "open".into(),
                    source: None,
                    evidence: None,
                    evidence_id: None,
                    oracle_kind: None,
                    advisory: false,
                    reason: None,
                },
                recorded_after_start: true,
            },
            ResearchUncertaintyView {
                item: UncertaintyItemView {
                    id: "U-0".into(),
                    statement: "known before".into(),
                    status: "open".into(),
                    source: None,
                    evidence: None,
                    evidence_id: None,
                    oracle_kind: None,
                    advisory: false,
                    reason: None,
                },
                recorded_after_start: false,
            },
        ],
    };
    let out = format_research_output(&view);
    assert!(out.contains("artifacts recorded: 1"));
    assert!(out.contains("uncertainties opened: 2"));
    assert!(out.contains("freshness: CURRENT"));
    assert!(out.contains("[recorded after start]"));
    assert!(out.contains("[pre-start]"));
    assert!(out.contains("attestation: unavailable"));
    // No discovered scalar, no verdict/score/green marker.
    assert!(!out.to_lowercase().contains("discovered"));
    assert!(!out.contains('✅'));
    assert!(!out.contains('%'));
    assert!(!out.to_uppercase().contains("PASS"));
    assert!(!out.to_lowercase().contains("verdict"));
}

// ── project default gate floor parsing ──

#[test]
fn project_gates_single_line() {
    let cfg = "[project]\ndefault_gates = [\"cargo_check\", \"cargo_test\", \"cargo_clippy\"]\n";
    assert_eq!(
        parse_project_default_gates(cfg),
        vec!["cargo_check", "cargo_test", "cargo_clippy"]
    );
}

#[test]
fn project_gates_multi_line() {
    let cfg =
        "[project]\ntype = \"rust\"\ndefault_gates = [\n  \"cargo_check\",\n  \"cargo_test\",\n]\n";
    assert_eq!(
        parse_project_default_gates(cfg),
        vec!["cargo_check", "cargo_test"]
    );
}

#[test]
fn project_gates_trailing_comment_and_no_spaces() {
    let cfg = "[project]\ndefault_gates=[\"cargo_check\"] # the floor\n";
    assert_eq!(parse_project_default_gates(cfg), vec!["cargo_check"]);
}

#[test]
fn project_gates_absent_when_no_section() {
    let cfg = "[risk]\nR1_cognitive_overload = true\n";
    assert!(parse_project_default_gates(cfg).is_empty());
}

#[test]
fn project_gates_absent_when_key_missing() {
    let cfg = "[project]\ntype = \"rust\"\n";
    assert!(parse_project_default_gates(cfg).is_empty());
}

#[test]
fn project_gates_empty_array_yields_none() {
    let cfg = "[project]\ndefault_gates = []\n";
    assert!(parse_project_default_gates(cfg).is_empty());
}

#[test]
fn project_gates_only_read_from_project_table() {
    // A `default_gates` under a different table must be ignored.
    let cfg = "[other]\ndefault_gates = [\"cargo_test\"]\n\n[project]\ntype = \"rust\"\n";
    assert!(parse_project_default_gates(cfg).is_empty());
}

#[test]
fn project_gates_stop_at_next_table() {
    let cfg = "[project]\ndefault_gates = [\"cargo_check\"]\n\n[severity]\nR1 = \"warning\"\n";
    assert_eq!(parse_project_default_gates(cfg), vec!["cargo_check"]);
}

// ── decision log (.ctl/decisions.jsonl) — non-canonical gate evidence ──────

#[test]
fn decision_entry_stamps_non_canonical_label_and_ts() {
    // Every record must self-identify as non-canonical and carry a timestamp,
    // while preserving the hook-supplied fields verbatim.
    let v = decision_entry(
        r#"{"source":"claude","tool":"write","allowed":false,"reason":"outside write_allow"}"#,
        1700,
    )
    .expect("valid json");
    assert_eq!(v.get("canonical").and_then(|x| x.as_bool()), Some(false));
    assert_eq!(v.get("ts").and_then(|x| x.as_u64()), Some(1700));
    assert_eq!(v.get("source").and_then(|x| x.as_str()), Some("claude"));
    assert_eq!(v.get("allowed").and_then(|x| x.as_bool()), Some(false));
}

#[test]
fn decision_entry_label_cannot_be_forged_by_the_hook() {
    // A hook that tries to mark its record canonical:true must not win — the
    // stamp is applied last so the log can never claim canonical status.
    let v = decision_entry(r#"{"canonical":true,"ts":1}"#, 9000).expect("valid json");
    assert_eq!(v.get("canonical").and_then(|x| x.as_bool()), Some(false));
    assert_eq!(v.get("ts").and_then(|x| x.as_u64()), Some(9000));
}

#[test]
fn decision_entry_rejects_invalid_json() {
    assert!(decision_entry("not json", 1).is_err());
}

#[test]
fn format_decisions_always_labels_non_canonical() {
    let lines = vec![
        r#"{"source":"claude","tool":"bash","allowed":false,"reason":"git commit only in commit window","ts":1}"#.to_string(),
    ];
    let out = format_decisions(&lines, 50, false);
    assert!(out.contains("NON-CANONICAL"));
    assert!(out.contains("not covered by `ctl validate`"));
    assert!(out.contains("[DENY ]"));
    assert!(out.contains("claude/bash"));
}

#[test]
fn format_decisions_empty_log_is_stated_not_blank() {
    let out = format_decisions(&[], 50, false);
    assert!(out.contains("NON-CANONICAL"));
    assert!(out.contains("(no decisions recorded yet)"));
}

#[test]
fn format_decisions_limit_shows_most_recent() {
    let lines: Vec<String> = (0..5)
        .map(|i| format!(r#"{{"tool":"bash","allowed":true,"reason":"r{i}","ts":{i}}}"#))
        .collect();
    let out = format_decisions(&lines, 2, false);
    assert!(out.contains("showing 2 of 5 record(s)"));
    // The two most-recent (ts=3, ts=4) are shown; the oldest is not.
    assert!(out.contains("ts=4") && out.contains("ts=3"));
    assert!(!out.contains("ts=0"));
}

#[test]
fn format_decisions_json_mode_is_raw_passthrough_no_banner() {
    let lines = vec![
        r#"{"tool":"bash","allowed":true,"ts":1}"#.to_string(),
        r#"{"tool":"write","allowed":false,"ts":2}"#.to_string(),
    ];
    let out = format_decisions(&lines, 0, true);
    assert!(!out.contains("NON-CANONICAL"));
    assert_eq!(out.lines().count(), 2);
    assert!(out.contains(r#""tool":"write""#));
}

#[test]
fn format_decision_line_keeps_unparseable_records_visible() {
    // A malformed advisory record must be shown (flagged), never dropped.
    let out = format_decision_line("{ broken");
    assert!(out.contains("?unparseable"));
    assert!(out.contains("{ broken"));
}

#[test]
fn ellipsize_marks_truncation_only_when_needed() {
    assert_eq!(ellipsize("short", 10), "short");
    let long = ellipsize("abcdefghij", 5);
    assert!(long.ends_with('…'));
    assert_eq!(long.chars().count(), 5);
}

// ── ctl-init reachability self-check ──
// Mirrors the ONE blessed hook resolution chain (B-lite):
// CTL_BIN → ~/.cargo/bin → real exe on PATH.

#[test]
fn ctl_bin_override_resolves_first() {
    let exists = |_p: &Path| true;
    let p = CtlProbe {
        windows: true,
        bin_name: "ctl.exe",
        ctl_bin: Some("  C:/x/ctl.exe  ".into()),
        home: None,
        path_dirs: vec![],
        exists: &exists,
    };
    assert_eq!(
        resolve_ctl_for_hook(&p),
        CtlReach::Resolved { how: "CTL_BIN" }
    );
}

#[test]
fn blank_ctl_bin_is_ignored() {
    let exists = |_p: &Path| false;
    let p = CtlProbe {
        windows: true,
        bin_name: "ctl.exe",
        ctl_bin: Some("   ".into()),
        home: None,
        path_dirs: vec![],
        exists: &exists,
    };
    assert_eq!(resolve_ctl_for_hook(&p), CtlReach::NotFound);
}

#[test]
fn cargo_install_resolves_before_path() {
    let cargo = Path::new("/home/u").join(".cargo").join("bin").join("ctl");
    let exists = move |q: &Path| q == cargo || q == Path::new("/tools").join("ctl");
    let p = CtlProbe {
        windows: false,
        bin_name: "ctl",
        ctl_bin: None,
        home: Some("/home/u".into()),
        path_dirs: vec!["/tools".into()],
        exists: &exists,
    };
    assert_eq!(
        resolve_ctl_for_hook(&p),
        CtlReach::Resolved { how: "cargo" }
    );
}

#[test]
fn windows_only_shim_on_path_is_flagged_as_only_shim() {
    // Only a bare-name shim (ctl.cmd) is on PATH — e.g. a stale npm-era
    // leftover; no real ctl.exe anywhere. execFile/subprocess (no shell)
    // cannot run it → the gate fails closed.
    let shim = Path::new("C:/npm").join("ctl.cmd");
    let exists = move |q: &Path| q == shim;
    let p = CtlProbe {
        windows: true,
        bin_name: "ctl.exe",
        ctl_bin: None,
        home: None,
        path_dirs: vec!["C:/npm".into()],
        exists: &exists,
    };
    assert_eq!(resolve_ctl_for_hook(&p), CtlReach::OnlyShim);
}

#[test]
fn real_exe_on_path_resolves() {
    let exe = Path::new("C:/tools").join("ctl.exe");
    let exists = move |q: &Path| q == exe;
    let p = CtlProbe {
        windows: true,
        bin_name: "ctl.exe",
        ctl_bin: None,
        home: None,
        path_dirs: vec!["C:/tools".into()],
        exists: &exists,
    };
    assert_eq!(resolve_ctl_for_hook(&p), CtlReach::Resolved { how: "PATH" });
}

#[test]
fn nothing_anywhere_is_not_found() {
    let exists = |_q: &Path| false;
    let p = CtlProbe {
        windows: true,
        bin_name: "ctl.exe",
        ctl_bin: None,
        home: None,
        path_dirs: vec!["C:/x".into()],
        exists: &exists,
    };
    assert_eq!(resolve_ctl_for_hook(&p), CtlReach::NotFound);
}

// ── OMP agent .env CTL_BIN pin (ctl init --platform omp|all) ──

#[test]
fn omp_env_file_defaults_to_home_omp_agent() {
    let f = omp_agent_env_file(None, None, Path::new("/home/u"));
    assert_eq!(
        f,
        Path::new("/home/u").join(".omp").join("agent").join(".env")
    );
}

#[test]
fn omp_env_file_agent_dir_override_wins() {
    let f = omp_agent_env_file(
        Some(" /custom/agent "),
        Some("/ignored"),
        Path::new("/home/u"),
    );
    assert_eq!(f, Path::new("/custom/agent").join(".env"));
}

#[test]
fn omp_env_file_relative_config_dir_joins_home() {
    let f = omp_agent_env_file(None, Some(".omp-alt"), Path::new("/home/u"));
    assert_eq!(
        f,
        Path::new("/home/u")
            .join(".omp-alt")
            .join("agent")
            .join(".env")
    );
}

#[test]
fn upsert_appends_to_empty_and_preserves_other_lines() {
    let (out, old) = upsert_env_line("", "CTL_BIN", "/x/ctl");
    assert_eq!(out, "CTL_BIN=\"/x/ctl\"\n");
    assert_eq!(old, None);

    let (out, old) = upsert_env_line("# comment\nFOO=bar\n", "CTL_BIN", "/x/ctl");
    assert_eq!(out, "# comment\nFOO=bar\nCTL_BIN=\"/x/ctl\"\n");
    assert_eq!(old, None);
}

#[test]
fn upsert_replaces_existing_key_and_reports_old_value() {
    // Key matching mirrors OMP parseEnvFile: trimmed key before first '='.
    let (out, old) = upsert_env_line(
        "FOO=bar\nCTL_BIN = \"C:\\old\\ctl.exe\"\r\nBAZ=1\n",
        "CTL_BIN",
        "C:\\new\\ctl.exe",
    );
    assert_eq!(out, "FOO=bar\nCTL_BIN=\"C:\\new\\ctl.exe\"\nBAZ=1\n");
    assert_eq!(old.as_deref(), Some("C:\\old\\ctl.exe"));
}

#[test]
fn upsert_collapses_duplicate_keys_and_reports_last_wins_value() {
    // OMP's parseEnvFile is last-wins: a duplicate CTL_BIN below the pin
    // would silently override it, so all occurrences collapse into one.
    let (out, old) = upsert_env_line(
        "CTL_BIN=/first/ctl\nFOO=bar\nCTL_BIN=\"/second/ctl\"\n",
        "CTL_BIN",
        "/new/ctl",
    );
    assert_eq!(out, "CTL_BIN=\"/new/ctl\"\nFOO=bar\n");
    assert_eq!(old.as_deref(), Some("/second/ctl"));
}

#[test]
fn upsert_is_idempotent_and_skips_comments() {
    let content = "# CTL_BIN=commented\nCTL_BIN=\"/x/ctl\"\n";
    let (out, old) = upsert_env_line(content, "CTL_BIN", "/x/ctl");
    assert_eq!(out, content);
    assert_eq!(old.as_deref(), Some("/x/ctl"));
}

#[test]
fn cargo_target_builds_are_detected() {
    assert!(is_cargo_target_build(Path::new(
        "C:\\repo\\target\\debug\\ctl.exe"
    )));
    assert!(is_cargo_target_build(Path::new("/repo/target/release/ctl")));
    assert!(!is_cargo_target_build(Path::new(
        "C:\\Users\\u\\.cargo\\bin\\ctl.exe"
    )));
    assert!(!is_cargo_target_build(Path::new(
        "/tools/targeted/debug/ctl"
    )));
}
#[test]
fn init_platform_selection_supports_multiple_platforms() {
    let selection = super::resolve_platform_selection(
        &[super::PlatformArg::Claude, super::PlatformArg::Opencode],
        false,
        false,
        false,
        false,
    )
    .unwrap();
    assert!(selection.claude);
    assert!(selection.opencode);
    assert!(!selection.omp);
}
