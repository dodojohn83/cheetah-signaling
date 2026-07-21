//! Tenant aggregate for multi-tenant isolation.

use cheetah_signal_types::{TenantId, UtcTimestamp};
use serde::{Deserialize, Serialize};

/// Maximum length of a tenant display name.
pub const MAX_TENANT_NAME_LEN: usize = 256;

/// A tenant that owns devices, operations and media sessions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Tenant {
    /// Tenant identifier (UUIDv7).
    pub tenant_id: TenantId,
    /// Human-readable display name.
    pub name: String,
    /// Creation timestamp (UTC).
    pub created_at: UtcTimestamp,
    /// Last update timestamp (UTC).
    pub updated_at: UtcTimestamp,
    /// Soft-delete marker.
    pub deleted: bool,
}

impl Tenant {
    /// Creates a new active tenant after validating the name.
    pub fn new(
        tenant_id: TenantId,
        name: impl Into<String>,
        now: UtcTimestamp,
    ) -> Result<Self, crate::DomainError> {
        let name = name.into();
        validate_tenant_name(&name)?;
        if tenant_id.as_uuid().is_nil() {
            return Err(crate::DomainError::invalid_argument(
                "tenant_id must not be nil",
            ));
        }
        Ok(Self {
            tenant_id,
            name,
            created_at: now,
            updated_at: now,
            deleted: false,
        })
    }

    /// Renames the tenant.
    pub fn rename(
        &mut self,
        name: impl Into<String>,
        now: UtcTimestamp,
    ) -> Result<(), crate::DomainError> {
        if self.deleted {
            return Err(crate::DomainError::invalid_argument(
                "cannot rename a deleted tenant",
            ));
        }
        let name = name.into();
        validate_tenant_name(&name)?;
        self.name = name;
        self.updated_at = now;
        Ok(())
    }

    /// Soft-deletes the tenant.
    pub fn soft_delete(&mut self, now: UtcTimestamp) {
        self.deleted = true;
        self.updated_at = now;
    }
}

fn validate_tenant_name(name: &str) -> Result<(), crate::DomainError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(crate::DomainError::invalid_argument(
            "tenant name must not be empty",
        ));
    }
    if name.len() > MAX_TENANT_NAME_LEN {
        return Err(crate::DomainError::invalid_argument(format!(
            "tenant name must not exceed {MAX_TENANT_NAME_LEN} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheetah_signal_types::UtcTimestamp;
    use time::OffsetDateTime;

    fn now() -> UtcTimestamp {
        UtcTimestamp::from_offset(OffsetDateTime::UNIX_EPOCH)
    }

    #[test]
    fn rejects_empty_name() {
        let id = TenantId::from_uuid(uuid::Uuid::now_v7());
        assert!(Tenant::new(id, "  ", now()).is_err());
    }

    #[test]
    fn creates_active_tenant() {
        let id = TenantId::from_uuid(uuid::Uuid::now_v7());
        match Tenant::new(id, "acme", now()) {
            Ok(tenant) => {
                assert_eq!(tenant.name, "acme");
                assert!(!tenant.deleted);
            }
            Err(e) => panic!("valid tenant construction failed: {e}"),
        }
    }
}
