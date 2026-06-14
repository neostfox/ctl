//! Telemetry evidence index (M5).
//!
//! Per the fact model, `telemetry.jsonl` is an **append-only evidence index**
//! that is *separate* from the canonical `events.jsonl`. Telemetry is evidence,
//! not truth: an external executor or human submits it, and the control layer
//! treats it as a signal — never as authority to relax permissions.
//!
//! This module is pure (no IO, no wall-clock). The application layer stamps
//! `recorded_at` and the infrastructure layer reads/appends the JSONL file.

use serde::{Deserialize, Serialize};

/// Schema tag stamped on every telemetry entry.
pub const TELEMETRY_SCHEMA: &str = "control.telemetry.v1";

/// Recognized telemetry signal kinds. An entry whose `kind` is **not** in this
/// set is an *unknown signal*: the drift engine fails closed on it (it never
/// relaxes permissions for a signal it does not understand).
pub const KNOWN_KINDS: &[&str] = &[
    "test_failures",
    "lint_errors",
    "retries",
    "attempts", // alias of retries (iteration budget)
    "unexpected_writes",
];

/// True if `kind` is a recognized telemetry signal.
pub fn is_known_kind(kind: &str) -> bool {
    KNOWN_KINDS.contains(&kind)
}

/// One telemetry evidence record (one line of `telemetry.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelemetryEntry {
    /// Schema tag — always [`TELEMETRY_SCHEMA`].
    pub schema: String,
    /// Task this signal is about.
    pub task_id: String,
    /// Signal kind (see [`KNOWN_KINDS`]); free-form, unknown kinds fail closed.
    pub kind: String,
    /// Numeric magnitude of the signal (e.g. failure count, retry count).
    pub value: i64,
    /// ISO 8601 provenance timestamp, stamped by the application layer when the
    /// evidence was submitted. Not produced here (domain stays time-free).
    pub recorded_at: String,
    /// Provenance: who/what submitted the evidence (actor, adapter name, etc.).
    pub source: String,
}

impl TelemetryEntry {
    /// Construct an entry. `recorded_at` is supplied by the caller (application
    /// layer) so this module performs no wall-clock access.
    pub fn new(task_id: &str, kind: &str, value: i64, recorded_at: &str, source: &str) -> Self {
        Self {
            schema: TELEMETRY_SCHEMA.to_string(),
            task_id: task_id.to_string(),
            kind: kind.to_string(),
            value,
            recorded_at: recorded_at.to_string(),
            source: source.to_string(),
        }
    }
}
