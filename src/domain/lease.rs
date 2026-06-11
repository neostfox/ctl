use serde::{Deserialize, Serialize};

/// Status of a capability lease, projected from events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LeaseStatus {
    Active,
    Revoked,
    Expired,
}

impl std::fmt::Display for LeaseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeaseStatus::Active => write!(f, "Active"),
            LeaseStatus::Revoked => write!(f, "Revoked"),
            LeaseStatus::Expired => write!(f, "Expired"),
        }
    }
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
impl LeaseState {
    /// Check if the lease is currently active.
    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.status == LeaseStatus::Active
    }

    /// Check if the lease has been revoked.
    #[allow(dead_code)]
    pub fn is_revoked(&self) -> bool {
        self.status == LeaseStatus::Revoked
    }

    /// Check if the lease has expired (TTL or max uses exceeded).
    #[allow(dead_code)]
    pub fn is_expired(&self) -> bool {
        self.status == LeaseStatus::Expired
    }
}
