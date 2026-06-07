use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub fn is_valid(&self) -> bool {
        self.schema == "control.event-envelope.v1"
            && self.seq > 0
            && !self.event_id.is_empty()
            && !self.task_id.is_empty()
            && !self.event_type.is_empty()
    }
}
