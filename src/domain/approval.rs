use serde::{Deserialize, Serialize};

/// Approval request state, projected from events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApprovalStatus {
    Pending,
    Granted,
    Denied,
    Expired,
}

impl std::fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalStatus::Pending => write!(f, "Pending"),
            ApprovalStatus::Granted => write!(f, "Granted"),
            ApprovalStatus::Denied => write!(f, "Denied"),
            ApprovalStatus::Expired => write!(f, "Expired"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalState {
    pub request_id: String,
    pub reason: String,
    pub scope: serde_json::Value,
    pub ttl_seconds: u64,
    pub requested_at_seq: i64,
    #[serde(default)]
    pub granted_at_seq: Option<i64>,
    pub status: ApprovalStatus,
}

impl ApprovalState {
    pub fn is_granted(&self) -> bool {
        self.status == ApprovalStatus::Granted
    }
}
