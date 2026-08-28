use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    owner: String,
    expires_ms: u64,
}

impl Lease {
    pub fn acquire(owner: impl Into<String>, now_ms: u64, ttl: Duration) -> Self {
        Self {
            owner: owner.into(),
            expires_ms: now_ms.saturating_add(ttl.as_millis().min(u128::from(u64::MAX)) as u64),
        }
    }
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn is_valid_for(&self, owner: &str, now_ms: u64) -> bool {
        self.owner == owner && now_ms < self.expires_ms
    }
    pub fn renew(&mut self, owner: &str, now_ms: u64, ttl: Duration) -> Result<(), LeaseError> {
        if !self.is_valid_for(owner, now_ms) {
            return Err(LeaseError::NotOwner);
        }
        self.expires_ms = now_ms.saturating_add(ttl.as_millis().min(u128::from(u64::MAX)) as u64);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("lease is expired or belongs to another owner")]
    NotOwner,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_can_renew_until_expiry() {
        let mut lease = Lease::acquire("rest", 100, Duration::from_millis(50));
        assert!(lease.is_valid_for("rest", 149));
        assert_eq!(
            lease.renew("mcp", 120, Duration::from_secs(1)),
            Err(LeaseError::NotOwner)
        );
        assert!(lease.renew("rest", 120, Duration::from_millis(50)).is_ok());
        assert!(!lease.is_valid_for("rest", 170));
    }
}
