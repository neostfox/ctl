//! Drift engine (M5): transparent, deterministic rules over evidence.
//!
//! Drift answers "why continue, pause, or replan?" using a **fixed rule
//! catalog** — never model scoring. The same signals always produce the same
//! score, level, and recommended action (pure integer arithmetic, stable rule
//! ordering). Three invariants from the roadmap are encoded directly here:
//!
//! - **Unknown signals fail closed.** An unrecognized telemetry kind can only
//!   raise drift and force `ASK`; it never relaxes anything.
//! - **Rising drift only pauses or replans.** No branch returns an action that
//!   expands scope or permissions.
//! - **Replan/rescope only propose.** The structured proposal is data the human
//!   acts on; this module performs no side effects (it is pure: no IO, no time).

use crate::domain::task::Phase;
use serde::{Deserialize, Serialize};

// ── Rule catalog ──────────────────────────────────────────────────────────
// Each rule has a stable ID and a fixed point weight. IDs are emitted in
// ascending order so explanations are byte-stable across runs.

pub const DRIFT_BOUNDARY: &str = "DRIFT-001"; // out-of-scope write attempt recorded
pub const DRIFT_GATE_FAIL: &str = "DRIFT-002"; // a required gate's latest result failed
pub const DRIFT_REVIEW: &str = "DRIFT-003"; // unresolved reviewer rejection
pub const DRIFT_TEST_FAIL: &str = "DRIFT-004"; // telemetry: test failures
pub const DRIFT_LINT: &str = "DRIFT-005"; // telemetry: lint errors
pub const DRIFT_RETRIES: &str = "DRIFT-006"; // telemetry: excessive retries
pub const DRIFT_UNEXPECTED_WRITES: &str = "DRIFT-007"; // telemetry: writes outside scope
pub const DRIFT_HELD: &str = "DRIFT-008"; // task is held
pub const DRIFT_UNKNOWN: &str = "DRIFT-009"; // telemetry: unrecognized signal kind (fail closed)

const RETRIES_THRESHOLD: i64 = 3;

// Level thresholds (inclusive lower bounds, integer points).
const LOW_MIN: i64 = 1;
const MEDIUM_MIN: i64 = 20;
const HIGH_MIN: i64 = 50;

/// Signals derived from the event ledger and the telemetry evidence index by
/// the application layer. Plain data — this module never reads them from disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DriftSignals {
    /// Count of `boundary_violation_recorded` events.
    pub boundary_violations: u32,
    /// Count of required gates whose latest result failed.
    pub gate_failures: u32,
    /// An unresolved reviewer rejection exists (review == needs_work).
    pub unresolved_rejections: bool,
    /// Task is currently held.
    pub is_held: bool,
    /// Telemetry: summed reported test failures.
    pub test_failures: i64,
    /// Telemetry: summed reported lint errors.
    pub lint_errors: i64,
    /// Telemetry: summed reported retries/attempts (iteration budget).
    pub retries: i64,
    /// Telemetry: summed reported writes outside the task scope.
    pub unexpected_writes: i64,
    /// Telemetry contained at least one unrecognized signal kind (fail closed).
    pub unknown_signal: bool,
}

/// Drift severity bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftLevel {
    None,
    Low,
    Medium,
    High,
}

impl DriftLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            DriftLevel::None => "none",
            DriftLevel::Low => "low",
            DriftLevel::Medium => "medium",
            DriftLevel::High => "high",
        }
    }

    fn from_score(score: i64) -> Self {
        if score >= HIGH_MIN {
            DriftLevel::High
        } else if score >= MEDIUM_MIN {
            DriftLevel::Medium
        } else if score >= LOW_MIN {
            DriftLevel::Low
        } else {
            DriftLevel::None
        }
    }
}

/// A rule that fired, with the evidence that triggered it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FiredRule {
    pub id: String,
    pub points: i64,
    pub evidence: String,
}

/// Result of evaluating the rule catalog against signals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftReport {
    pub task_id: String,
    pub score: i64,
    pub level: DriftLevel,
    pub fired_rules: Vec<FiredRule>,
    /// Mirrors `DriftSignals::unknown_signal` for the fail-closed decision.
    pub unknown_signal: bool,
}

impl DriftReport {
    fn fired(&self, rule_id: &str) -> bool {
        self.fired_rules.iter().any(|r| r.id == rule_id)
    }

    /// Sorted list of fired rule IDs (stable for explanations/projections).
    pub fn fired_ids(&self) -> Vec<String> {
        self.fired_rules.iter().map(|r| r.id.clone()).collect()
    }
}

/// Recommended next action. Every variant is either a continue or a
/// pause/replan — none expands permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NextActionKind {
    /// Continue the current lifecycle.
    Pass,
    /// Stop and ask a human (also the fail-closed default for unknown signals).
    Ask,
    /// Stop execution; a human must resolve a hold/violation before resuming.
    Stop,
    /// Stop and propose a full replan (structured proposal only).
    Replan,
    /// Stop and propose a scope change (structured proposal only).
    Rescope,
}

impl NextActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NextActionKind::Pass => "pass",
            NextActionKind::Ask => "ask",
            NextActionKind::Stop => "stop",
            NextActionKind::Replan => "replan",
            NextActionKind::Rescope => "rescope",
        }
    }
}

/// A structured proposal accompanying a `Replan`/`Rescope` recommendation. It is
/// **data for a human** — generating it changes no state, approves no lease, and
/// starts no task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredProposal {
    pub proposed_action: String,
    pub rationale: String,
    pub fired_rules: Vec<String>,
    pub evidence: Vec<String>,
}

/// The full next-action decision: signals + rule IDs + evidence are all carried
/// so every decision is self-explaining (roadmap exit condition 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextActionProposal {
    pub task_id: String,
    pub action: NextActionKind,
    pub level: DriftLevel,
    pub score: i64,
    pub rationale: String,
    pub suggested_command: String,
    pub fired_rules: Vec<String>,
    /// Present only for `Replan`/`Rescope`.
    pub structured_proposal: Option<StructuredProposal>,
}

/// Evaluate the fixed rule catalog. Pure and deterministic: rules are checked in
/// ID order and points summed as integers, so identical signals yield an
/// identical report byte-for-byte.
pub fn evaluate(task_id: &str, signals: &DriftSignals) -> DriftReport {
    let mut fired: Vec<FiredRule> = Vec::new();
    let mut fire = |id: &str, points: i64, evidence: String| {
        fired.push(FiredRule {
            id: id.to_string(),
            points,
            evidence,
        });
    };

    if signals.boundary_violations >= 1 {
        fire(
            DRIFT_BOUNDARY,
            30,
            format!(
                "{} boundary violation event(s) recorded",
                signals.boundary_violations
            ),
        );
    }
    if signals.gate_failures >= 1 {
        fire(
            DRIFT_GATE_FAIL,
            20,
            format!("{} required gate(s) failing", signals.gate_failures),
        );
    }
    if signals.unresolved_rejections {
        fire(
            DRIFT_REVIEW,
            25,
            "unresolved reviewer rejection (review needs_work)".to_string(),
        );
    }
    if signals.test_failures >= 1 {
        fire(
            DRIFT_TEST_FAIL,
            15,
            format!("telemetry: {} test failure(s)", signals.test_failures),
        );
    }
    if signals.lint_errors >= 1 {
        fire(
            DRIFT_LINT,
            5,
            format!("telemetry: {} lint error(s)", signals.lint_errors),
        );
    }
    if signals.retries >= RETRIES_THRESHOLD {
        fire(
            DRIFT_RETRIES,
            15,
            format!(
                "telemetry: {} retries (>= {})",
                signals.retries, RETRIES_THRESHOLD
            ),
        );
    }
    if signals.unexpected_writes >= 1 {
        fire(
            DRIFT_UNEXPECTED_WRITES,
            30,
            format!(
                "telemetry: {} write(s) reported outside scope",
                signals.unexpected_writes
            ),
        );
    }
    if signals.is_held {
        fire(DRIFT_HELD, 10, "task is held".to_string());
    }
    if signals.unknown_signal {
        fire(
            DRIFT_UNKNOWN,
            10,
            "telemetry contains an unrecognized signal kind (fail closed)".to_string(),
        );
    }

    let score: i64 = fired.iter().map(|r| r.points).sum();
    DriftReport {
        task_id: task_id.to_string(),
        score,
        level: DriftLevel::from_score(score),
        fired_rules: fired,
        unknown_signal: signals.unknown_signal,
    }
}

/// Map a drift report + task phase to a recommended action. First match wins,
/// so precedence is fixed and deterministic. No branch ever expands permissions.
pub fn next_action(report: &DriftReport, phase: Phase) -> NextActionProposal {
    let id = &report.task_id;
    let fired_ids = report.fired_ids();
    let evidence: Vec<String> = report
        .fired_rules
        .iter()
        .map(|r| format!("{}: {}", r.id, r.evidence))
        .collect();

    let mk = |action: NextActionKind, rationale: &str, suggested: String| NextActionProposal {
        task_id: id.clone(),
        action,
        level: report.level,
        score: report.score,
        rationale: rationale.to_string(),
        suggested_command: suggested,
        fired_rules: fired_ids.clone(),
        structured_proposal: None,
    };

    let mk_proposal = |action: NextActionKind, kind: &str, rationale: &str| NextActionProposal {
        task_id: id.clone(),
        action,
        level: report.level,
        score: report.score,
        rationale: rationale.to_string(),
        suggested_command: format!(
            "review the {} proposal below, then (human) `ctl task revise --id {}`",
            kind, id
        ),
        fired_rules: fired_ids.clone(),
        structured_proposal: Some(StructuredProposal {
            proposed_action: kind.to_string(),
            rationale: rationale.to_string(),
            fired_rules: fired_ids.clone(),
            evidence: evidence.clone(),
        }),
    };

    // 1. Unknown signal → fail closed to ASK, regardless of level.
    if report.unknown_signal {
        return mk(
            NextActionKind::Ask,
            "unrecognized telemetry signal — cannot assess; defaulting to human review (no permission relaxed)",
            format!("ctl drift explain --id {}", id),
        );
    }

    // 2. High drift with an out-of-scope signal → stop, human must resolve.
    if report.level == DriftLevel::High
        && (report.fired(DRIFT_BOUNDARY) || report.fired(DRIFT_UNEXPECTED_WRITES))
    {
        return mk(
            NextActionKind::Stop,
            "high drift with an out-of-scope write signal — execution stopped pending human review",
            format!(
                "resolve the boundary violation, then `ctl drift explain --id {}`",
                id
            ),
        );
    }

    // 3. High drift → propose a replan (proposal only).
    if report.level == DriftLevel::High {
        return mk_proposal(
            NextActionKind::Replan,
            "replan",
            "high drift — current plan is off track; proposing a replan",
        );
    }

    // 4. Medium drift mid-implementation → propose a rescope (proposal only).
    if report.level == DriftLevel::Medium && phase == Phase::InProgress {
        return mk_proposal(
            NextActionKind::Rescope,
            "rescope",
            "medium drift during implementation — proposing a scope adjustment",
        );
    }

    // 5. Medium drift otherwise → ask a human.
    if report.level == DriftLevel::Medium {
        return mk(
            NextActionKind::Ask,
            "medium drift — pausing for human review",
            format!("ctl drift explain --id {}", id),
        );
    }

    // 6. Held (any remaining level) → stop until the hold is resolved.
    if report.fired(DRIFT_HELD) {
        return mk(
            NextActionKind::Stop,
            "task is held — execution stopped until the hold is resolved",
            format!("ctl drift explain --id {}", id),
        );
    }

    // 7. Low / none with no blockers → continue.
    mk(
        NextActionKind::Pass,
        "drift within tolerance — continue the current lifecycle",
        format!("ctl task status --id {}", id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn high_boundary() -> DriftSignals {
        DriftSignals {
            boundary_violations: 1,
            ..Default::default()
        }
    }

    #[test]
    fn empty_signals_are_no_drift() {
        let r = evaluate("t", &DriftSignals::default());
        assert_eq!(r.score, 0);
        assert_eq!(r.level, DriftLevel::None);
        assert!(r.fired_rules.is_empty());
    }

    #[test]
    fn each_rule_fires_with_expected_points() {
        let cases: &[(DriftSignals, &str, i64)] = &[
            (high_boundary(), DRIFT_BOUNDARY, 30),
            (
                DriftSignals {
                    gate_failures: 1,
                    ..Default::default()
                },
                DRIFT_GATE_FAIL,
                20,
            ),
            (
                DriftSignals {
                    unresolved_rejections: true,
                    ..Default::default()
                },
                DRIFT_REVIEW,
                25,
            ),
            (
                DriftSignals {
                    test_failures: 2,
                    ..Default::default()
                },
                DRIFT_TEST_FAIL,
                15,
            ),
            (
                DriftSignals {
                    lint_errors: 1,
                    ..Default::default()
                },
                DRIFT_LINT,
                5,
            ),
            (
                DriftSignals {
                    retries: 3,
                    ..Default::default()
                },
                DRIFT_RETRIES,
                15,
            ),
            (
                DriftSignals {
                    unexpected_writes: 1,
                    ..Default::default()
                },
                DRIFT_UNEXPECTED_WRITES,
                30,
            ),
            (
                DriftSignals {
                    is_held: true,
                    ..Default::default()
                },
                DRIFT_HELD,
                10,
            ),
            (
                DriftSignals {
                    unknown_signal: true,
                    ..Default::default()
                },
                DRIFT_UNKNOWN,
                10,
            ),
        ];
        for (sig, id, pts) in cases {
            let r = evaluate("t", sig);
            assert_eq!(r.fired_rules.len(), 1, "rule {} should fire alone", id);
            assert_eq!(&r.fired_rules[0].id, id);
            assert_eq!(r.fired_rules[0].points, *pts);
            assert_eq!(r.score, *pts);
        }
    }

    #[test]
    fn retries_below_threshold_do_not_fire() {
        let r = evaluate(
            "t",
            &DriftSignals {
                retries: 2,
                ..Default::default()
            },
        );
        assert_eq!(r.level, DriftLevel::None);
    }

    #[test]
    fn level_thresholds_are_exact() {
        // 19 → low, 20 → medium, 49 → medium, 50 → high.
        assert_eq!(DriftLevel::from_score(0), DriftLevel::None);
        assert_eq!(DriftLevel::from_score(1), DriftLevel::Low);
        assert_eq!(DriftLevel::from_score(19), DriftLevel::Low);
        assert_eq!(DriftLevel::from_score(20), DriftLevel::Medium);
        assert_eq!(DriftLevel::from_score(49), DriftLevel::Medium);
        assert_eq!(DriftLevel::from_score(50), DriftLevel::High);
    }

    #[test]
    fn fired_rules_are_emitted_in_id_order() {
        // Fire several rules; IDs must come out ascending (stable explanations).
        let sig = DriftSignals {
            gate_failures: 1,     // 002
            unexpected_writes: 1, // 007
            test_failures: 1,     // 004
            ..Default::default()
        };
        let r = evaluate("t", &sig);
        let ids = r.fired_ids();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn evaluation_is_deterministic() {
        let sig = DriftSignals {
            boundary_violations: 2,
            test_failures: 4,
            retries: 5,
            ..Default::default()
        };
        assert_eq!(evaluate("t", &sig), evaluate("t", &sig));
    }

    #[test]
    fn unknown_signal_forces_ask() {
        let r = evaluate(
            "t",
            &DriftSignals {
                unknown_signal: true,
                boundary_violations: 1, // even with a high signal present
                ..Default::default()
            },
        );
        let a = next_action(&r, Phase::InProgress);
        assert_eq!(a.action, NextActionKind::Ask);
    }

    #[test]
    fn high_with_boundary_stops() {
        let r = evaluate(
            "t",
            &DriftSignals {
                boundary_violations: 1, // 30
                gate_failures: 1,       // 20 → total 50 = high
                ..Default::default()
            },
        );
        assert_eq!(r.level, DriftLevel::High);
        let a = next_action(&r, Phase::InProgress);
        assert_eq!(a.action, NextActionKind::Stop);
    }

    #[test]
    fn high_without_out_of_scope_replans() {
        let r = evaluate(
            "t",
            &DriftSignals {
                unresolved_rejections: true, // 25
                gate_failures: 1,            // 20
                test_failures: 1,            // 15 → 60 = high
                ..Default::default()
            },
        );
        assert_eq!(r.level, DriftLevel::High);
        let a = next_action(&r, Phase::Review);
        assert_eq!(a.action, NextActionKind::Replan);
        assert!(a.structured_proposal.is_some());
    }

    #[test]
    fn medium_in_progress_rescopes() {
        let r = evaluate(
            "t",
            &DriftSignals {
                gate_failures: 1, // 20 = medium
                ..Default::default()
            },
        );
        assert_eq!(r.level, DriftLevel::Medium);
        let a = next_action(&r, Phase::InProgress);
        assert_eq!(a.action, NextActionKind::Rescope);
        assert!(a.structured_proposal.is_some());
    }

    #[test]
    fn medium_outside_in_progress_asks() {
        let r = evaluate(
            "t",
            &DriftSignals {
                unresolved_rejections: true, // 25 = medium
                ..Default::default()
            },
        );
        let a = next_action(&r, Phase::Review);
        assert_eq!(a.action, NextActionKind::Ask);
    }

    #[test]
    fn held_low_drift_stops() {
        let r = evaluate(
            "t",
            &DriftSignals {
                is_held: true, // 10 = low
                ..Default::default()
            },
        );
        assert_eq!(r.level, DriftLevel::Low);
        let a = next_action(&r, Phase::InProgress);
        assert_eq!(a.action, NextActionKind::Stop);
    }

    #[test]
    fn clean_task_passes() {
        let r = evaluate("t", &DriftSignals::default());
        let a = next_action(&r, Phase::InProgress);
        assert_eq!(a.action, NextActionKind::Pass);
        assert!(a.structured_proposal.is_none());
    }

    #[derive(serde::Deserialize)]
    struct GoldenCase {
        name: String,
        signals: DriftSignals,
        phase: Phase,
        expect_level: String,
        expect_score: i64,
        expect_action: String,
        expect_rules: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    struct GoldenFile {
        cases: Vec<GoldenCase>,
    }

    /// Roadmap exit condition: golden fixtures correspond to fixed actions.
    #[test]
    fn golden_fixtures_map_to_fixed_actions() {
        let raw = std::fs::read_to_string("fixtures/m5_drift_golden.json")
            .expect("golden fixture present");
        let file: GoldenFile = serde_json::from_str(&raw).expect("golden fixture parses");
        assert!(!file.cases.is_empty());
        for case in &file.cases {
            let report = evaluate("t", &case.signals);
            assert_eq!(
                report.level.as_str(),
                case.expect_level,
                "level [{}]",
                case.name
            );
            assert_eq!(report.score, case.expect_score, "score [{}]", case.name);
            assert_eq!(
                report.fired_ids(),
                case.expect_rules,
                "rules [{}]",
                case.name
            );
            let action = next_action(&report, case.phase.clone());
            assert_eq!(
                action.action.as_str(),
                case.expect_action,
                "action [{}]",
                case.name
            );
        }
    }

    #[test]
    fn no_action_ever_relaxes_permissions() {
        // Exhaustively spot-check that the only actions produced are the
        // pause/continue set — there is no "expand" variant by construction,
        // but assert it across a spread of inputs as a guard.
        let inputs = [
            DriftSignals::default(),
            high_boundary(),
            DriftSignals {
                unknown_signal: true,
                ..Default::default()
            },
            DriftSignals {
                gate_failures: 1,
                ..Default::default()
            },
            DriftSignals {
                is_held: true,
                ..Default::default()
            },
        ];
        for sig in inputs {
            for phase in [Phase::InProgress, Phase::Review, Phase::Ready] {
                let a = next_action(&evaluate("t", &sig), phase);
                assert!(matches!(
                    a.action,
                    NextActionKind::Pass
                        | NextActionKind::Ask
                        | NextActionKind::Stop
                        | NextActionKind::Replan
                        | NextActionKind::Rescope
                ));
            }
        }
    }
}
