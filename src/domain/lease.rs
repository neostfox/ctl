use serde::{Deserialize, Serialize};

/// Status of a capability lease, projected from events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LeaseStatus {
    Active,
    Revoked,
    Expired,
}

/// Lease state tracking write access for a specific run.
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

