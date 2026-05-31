use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn is_valid(&self) -> bool {
        self.schema == "control.event-envelope.v1" && self.seq > 0
    }
}
