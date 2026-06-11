//! Event envelope for the control plane event sourcing system.
//!
//! Every state change is recorded as an immutable event following the
//! `control.event-envelope.v1` schema. Events are append-only and form
//! the canonical truth of the task ledger.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Immutable event record in the task event stream.
/// Follows `control.event-envelope.v1` schema.
pub struct Event {
    pub schema: String,
    pub event_id: String,
    pub command_id: String,
    pub task_id: String,
    pub seq: i64,
    pub occurred_at: String, // ISO 8601
    pub actor: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: serde_json::Value,
}

impl Event {
    /// Validate the event envelope structure.
    pub fn is_valid(&self) -> bool {
        self.schema == "control.event-envelope.v1"
            && self.seq > 0
            && !self.event_id.is_empty()
            && !self.task_id.is_empty()
            && !self.event_type.is_empty()
    }

    #[allow(dead_code)]
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    #[allow(dead_code)]
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    #[allow(dead_code)]
    pub fn payload(&self) -> &serde_json::Value {
        &self.payload
    }
}
