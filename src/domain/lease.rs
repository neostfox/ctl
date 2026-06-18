use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Status of a capability lease, projected from events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LeaseStatus {
    Active,
    Revoked,
    Expired,
}

impl LeaseStatus {
    /// Structured, uppercase wire token (never a prose string).
    pub fn token(&self) -> &'static str {
        match self {
            LeaseStatus::Active => "ACTIVE",
            LeaseStatus::Revoked => "REVOKED",
            LeaseStatus::Expired => "EXPIRED",
        }
    }
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

/// Typed errors for lease state transitions. Each reducer maps these to its own
/// caller-facing message, so the transition logic stays decoupled from any
/// particular event payload or wording.
#[derive(Debug, Clone, PartialEq)]
pub enum LeaseError {
    InvalidLeaseId,
    InvalidRunId,
    InvalidResource,
    InvalidAction,
    InvalidTtl,
    InvalidMaxUses,
    NotActive,
    NoRemainingUses,
}

impl fmt::Display for LeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LeaseError::InvalidLeaseId => write!(f, "lease_id is required"),
            LeaseError::InvalidRunId => write!(f, "run_id is required"),
            LeaseError::InvalidResource => write!(f, "resource_path is required"),
            LeaseError::InvalidAction => write!(f, "action is required"),
            LeaseError::InvalidTtl => write!(f, "ttl_seconds must be > 0"),
            LeaseError::InvalidMaxUses => write!(f, "max_uses must be > 0"),
            LeaseError::NotActive => write!(f, "lease is not active"),
            LeaseError::NoRemainingUses => write!(f, "lease has no remaining uses"),
        }
    }
}

/// A typed, validated grant request. Reducers parse their event payload into
/// this struct, then call [`LeaseState::grant`] — the transition logic never
/// touches a raw `Event`.
#[derive(Debug, Clone)]
pub struct LeaseGrant {
    pub lease_id: String,
    pub run_id: String,
    pub resource_path: String,
    pub action: String,
    pub ttl_seconds: u64,
    pub max_uses: u64,
    pub created_at_seq: i64,
    /// Binding fields (default-empty for task-aggregate leases).
    pub task_id: String,
    pub adapter: String,
    pub scopes: BTreeSet<String>,
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
    /// Parent task this lease is bound to (run-scoped leases set this; legacy
    /// task-aggregate leases leave it empty). `#[serde(default)]` keeps existing
    /// projected state replaying unchanged.
    #[serde(default)]
    pub task_id: String,
    /// Adapter the lease authorizes (run-scoped leases set this).
    #[serde(default)]
    pub adapter: String,
    /// Write scope the lease authorizes. For run-scoped leases this equals the
    /// run's `write_allow` exactly (V1).
    #[serde(default)]
    pub scopes: BTreeSet<String>,
}

impl LeaseState {
    /// Grant a new, Active lease from a validated request. Generic validation
    /// only (non-empty ids/resource/action, ttl > 0, max_uses > 0); aggregate-
    /// specific rules (e.g. the run path's `max_uses >= 2` and scope equality)
    /// are enforced by the calling reducer.
    pub fn grant(g: LeaseGrant) -> Result<Self, LeaseError> {
        if g.lease_id.is_empty() {
            return Err(LeaseError::InvalidLeaseId);
        }
        if g.run_id.is_empty() {
            return Err(LeaseError::InvalidRunId);
        }
        if g.resource_path.is_empty() {
            return Err(LeaseError::InvalidResource);
        }
        if g.action.is_empty() {
            return Err(LeaseError::InvalidAction);
        }
        if g.ttl_seconds == 0 {
            return Err(LeaseError::InvalidTtl);
        }
        if g.max_uses == 0 {
            return Err(LeaseError::InvalidMaxUses);
        }
        Ok(LeaseState {
            lease_id: g.lease_id,
            run_id: g.run_id,
            resource_path: g.resource_path,
            action: g.action,
            ttl_seconds: g.ttl_seconds,
            max_uses: g.max_uses,
            remaining_uses: g.max_uses,
            created_at_seq: g.created_at_seq,
            status: LeaseStatus::Active,
            task_id: g.task_id,
            adapter: g.adapter,
            scopes: g.scopes,
        })
    }

    /// Consume one use. Requires the lease be Active with uses remaining; the
    /// lease auto-expires when the last use is consumed.
    pub fn consume(&mut self) -> Result<(), LeaseError> {
        if self.status != LeaseStatus::Active {
            return Err(LeaseError::NotActive);
        }
        if self.remaining_uses == 0 {
            return Err(LeaseError::NoRemainingUses);
        }
        self.remaining_uses -= 1;
        if self.remaining_uses == 0 {
            self.status = LeaseStatus::Expired;
        }
        Ok(())
    }

    /// Revoke the lease (idempotent terminal transition).
    pub fn revoke(&mut self) {
        self.status = LeaseStatus::Revoked;
    }

    /// Expire the lease (idempotent terminal transition).
    pub fn expire(&mut self) {
        self.status = LeaseStatus::Expired;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_grant() -> LeaseGrant {
        LeaseGrant {
            lease_id: "l1".into(),
            run_id: "r1".into(),
            resource_path: "src/foo/".into(),
            action: "write".into(),
            ttl_seconds: 3600,
            max_uses: 100,
            created_at_seq: 2,
            task_id: "t1".into(),
            adapter: "omp".into(),
            scopes: BTreeSet::from(["src/foo/".to_string()]),
        }
    }

    #[test]
    fn grant_sets_active_and_full_uses() {
        let l = LeaseState::grant(sample_grant()).unwrap();
        assert_eq!(l.status, LeaseStatus::Active);
        assert_eq!(l.remaining_uses, 100);
        assert_eq!(l.task_id, "t1");
        assert_eq!(l.adapter, "omp");
        assert!(l.scopes.contains("src/foo/"));
    }

    #[test]
    fn grant_rejects_zero_ttl_and_uses() {
        let mut g = sample_grant();
        g.ttl_seconds = 0;
        assert_eq!(LeaseState::grant(g).unwrap_err(), LeaseError::InvalidTtl);
        let mut g = sample_grant();
        g.max_uses = 0;
        assert_eq!(
            LeaseState::grant(g).unwrap_err(),
            LeaseError::InvalidMaxUses
        );
    }

    #[test]
    fn consume_decrements_and_expires_at_zero() {
        let mut g = sample_grant();
        g.max_uses = 2;
        let mut l = LeaseState::grant(g).unwrap();
        l.consume().unwrap();
        assert_eq!(l.remaining_uses, 1);
        assert_eq!(l.status, LeaseStatus::Active);
        l.consume().unwrap();
        assert_eq!(l.remaining_uses, 0);
        assert_eq!(l.status, LeaseStatus::Expired);
    }

    #[test]
    fn consume_after_expiry_rejected_as_not_active() {
        let mut g = sample_grant();
        g.max_uses = 1;
        let mut l = LeaseState::grant(g).unwrap();
        l.consume().unwrap(); // → Expired
        assert_eq!(l.consume().unwrap_err(), LeaseError::NotActive);
    }

    #[test]
    fn consume_after_revoke_rejected() {
        let mut l = LeaseState::grant(sample_grant()).unwrap();
        l.revoke();
        assert_eq!(l.status, LeaseStatus::Revoked);
        assert_eq!(l.consume().unwrap_err(), LeaseError::NotActive);
    }
}
