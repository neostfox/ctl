use serde::{Deserialize, Serialize};

/// Capability lease state, projected from events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LeaseStatus {
    Active,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LeaseState {
    pub lease_id: String,
    pub run_id: String,
    pub resource_path: String,
    pub action: String,
    pub ttl_seconds: u64,
    pub max_uses: u64,
    pub remaining_uses: u64,
    pub created_at_seq: i64,
    pub status: LeaseStatus,
}

impl LeaseState {
    #[allow(dead_code)]
    pub fn is_valid(&self) -> bool {
        self.status == LeaseStatus::Active && self.remaining_uses > 0
    }
}
