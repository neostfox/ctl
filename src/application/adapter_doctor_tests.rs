//! adapter-doctor-v1 platform-integration assembly, exercised against the
//! real repo root (the dogfooding checkout ships every platform file).
use super::{
    adapter_doctor_report, adapter_status_diagnostic, claude_python_tests_check,
    evaluate_pretooluse_matcher,
};
use crate::adapters::{supported_adapters, CheckStatus};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn status_of<'a>(
    diag: &'a crate::adapters::AdapterDiagnostic,
    check: &str,
) -> &'a crate::adapters::AdapterCheck {
    diag.checks
        .iter()
        .find(|c| c.name == check)
        .unwrap_or_else(|| panic!("missing check '{check}'"))
}

#[test]
fn doctor_over_repo_root_has_no_failures_and_is_factual() {
    let report = adapter_doctor_report(&repo_root(), false);
    // The supported executor adapters + the Claude hook platform (the repo
    // wires `.claude/`, so its non-adapter diagnostic is appended).
    assert_eq!(report.total, supported_adapters().len() + 1);
    assert_eq!(
        report.healthy, report.total,
        "no adapter or platform should FAIL in-repo"
    );
    assert_eq!(report.failed, 0);
    // Bun plugin tests must NOT be run by default → at least one NOT_TRACKED.
    assert!(
        report.counts.not_tracked >= 1,
        "opencode Bun tests must be NOT_TRACKED without --verify"
    );
    // Sanity: the aggregate tally equals the sum of per-adapter pass counts.
    let pass_sum: usize = report.adapters.iter().map(|d| d.counts.pass).sum();
    assert_eq!(report.counts.pass, pass_sum);
}

#[test]
fn status_layers_contract_and_platform_checks() {
    let diag = adapter_status_diagnostic(&repo_root(), "opencode", false);
    assert!(diag.resolved);
    let names: Vec<&str> = diag.checks.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.iter().any(|n| n.starts_with("contract.")),
        "must include the pure contract clauses"
    );
    for expected in [
        "platform.skill_present",
        "platform.protocol_in_sync",
        "platform.opencode_plugin_present",
        "platform.opencode_bun_tests",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
    // Skills are in sync in-repo → drift check PASSes (REUSE of CI checker).
    assert_eq!(
        status_of(&diag, "platform.protocol_in_sync").status,
        CheckStatus::Pass
    );
    // Bun tests NOT_TRACKED by default.
    assert_eq!(
        status_of(&diag, "platform.opencode_bun_tests").status,
        CheckStatus::NotTracked
    );
}

#[test]
fn omp_hook_and_config_present_in_repo_pass() {
    let diag = adapter_status_diagnostic(&repo_root(), "omp", false);
    for n in ["platform.omp_hook_present", "platform.omp_config_present"] {
        assert_eq!(
            status_of(&diag, n).status,
            CheckStatus::Pass,
            "{n} ships in-repo"
        );
    }
}

#[test]
fn unknown_adapter_fails_contract_and_reports_unknown_platform() {
    let diag = adapter_status_diagnostic(&repo_root(), "bogus", false);
    assert!(!diag.resolved);
    assert!(diag.has_failures(), "contract.resolves must FAIL");
    // The platform layer says UNKNOWN (no wiring), never a fabricated PASS.
    assert_eq!(
        status_of(&diag, "platform.integration").status,
        CheckStatus::Unknown
    );
}

#[test]
fn verify_attempts_bun_so_it_is_not_not_tracked() {
    // Under --verify the Bun check is actually attempted: PASS, FAIL, or
    // UNKNOWN (bun unavailable) — but never NOT_TRACKED.
    let diag = adapter_status_diagnostic(&repo_root(), "opencode", true);
    assert_ne!(
        status_of(&diag, "platform.opencode_bun_tests").status,
        CheckStatus::NotTracked,
        "--verify must attempt the Bun suite"
    );
}

// ── Claude hook-platform diagnostic (claude-doctor-hookcheck-v1) ──────────

#[test]
fn claude_platform_diagnostic_passes_in_repo() {
    // The repo wires `.claude/`, so the report carries a non-adapter Claude
    // diagnostic: resolved=false, every wiring check PASS, and crucially no
    // FAIL (an optional hook platform must never fail the doctor).
    let report = adapter_doctor_report(&repo_root(), false);
    let claude = report
        .adapters
        .iter()
        .find(|d| d.adapter == "claude")
        .expect("Claude diagnostic present when .claude/ is wired");
    assert!(
        !claude.resolved,
        "Claude is a hook platform, not a resolvable adapter"
    );
    assert!(
        !claude.has_failures(),
        "an optional hook platform never FAILs the report"
    );
    for n in [
        "platform.claude_gate_hook_present",
        "platform.claude_context_hook_present",
        "platform.claude_settings_present",
        "platform.claude_pretooluse_matcher",
    ] {
        assert_eq!(
            status_of(claude, n).status,
            CheckStatus::Pass,
            "{n} holds in-repo"
        );
    }
}

#[test]
fn pretooluse_matcher_pass_when_expected_matcher_registered() {
    let json = r#"{"hooks":{"PreToolUse":[{"matcher":"Write|Edit|MultiEdit|Bash",
        "hooks":[{"type":"command","command":"python x"}]}]}}"#;
    let (status, _) = evaluate_pretooluse_matcher(Some(json));
    assert_eq!(status, CheckStatus::Pass);
}

#[test]
fn pretooluse_matcher_warns_on_wrong_or_absent_matcher() {
    // A different matcher leaves some mutating tools ungated → WARN (visible),
    // not a fabricated PASS.
    let wrong = r#"{"hooks":{"PreToolUse":[{"matcher":"Write|Edit"}]}}"#;
    assert_eq!(
        evaluate_pretooluse_matcher(Some(wrong)).0,
        CheckStatus::Warn
    );
    // No PreToolUse hook at all → WARN.
    let none = r#"{"hooks":{"SessionStart":[]}}"#;
    assert_eq!(evaluate_pretooluse_matcher(Some(none)).0, CheckStatus::Warn);
}

#[test]
fn pretooluse_matcher_unknown_when_unevaluable() {
    // Absent settings or malformed JSON cannot be evaluated → UNKNOWN, never
    // a silent PASS and never a FAIL.
    assert_eq!(evaluate_pretooluse_matcher(None).0, CheckStatus::Unknown);
    assert_eq!(
        evaluate_pretooluse_matcher(Some("{not json")).0,
        CheckStatus::Unknown
    );
}

#[test]
fn claude_hook_tests_not_tracked_without_verify() {
    // The python suite is opt-in: NOT_TRACKED by default (never a silent PASS).
    assert_eq!(
        claude_python_tests_check(&repo_root(), false).status,
        CheckStatus::NotTracked
    );
}

#[test]
fn claude_hook_tests_attempted_under_verify() {
    // Under --verify the suite is actually attempted: PASS / FAIL / UNKNOWN
    // (python unavailable) — but never NOT_TRACKED.
    assert_ne!(
        claude_python_tests_check(&repo_root(), true).status,
        CheckStatus::NotTracked,
        "--verify must attempt the python hook suite"
    );
}
