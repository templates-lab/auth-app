//! Postgres adapters for the admin authentication ports.
//!
//! [`PgAdminRepository`] implements [`AdminRepository`] over the `admin_users`
//! table; [`PgIpLockoutStore`] implements [`IpLockoutStore`] over
//! `admin_ip_lockouts`. Both are thin translations between the domain's value
//! objects and rows — the SQL lives here, never in the domain or application.
//!
//! Timestamps cross the boundary as Unix epoch seconds: `SystemTime` becomes a
//! `bigint` written through `to_timestamp()`, and `TIMESTAMPTZ` is read back with
//! `EXTRACT(EPOCH ...)`. That keeps the adapter to sqlx's built-in scalar types
//! (no timezone-typed column binding, no extra feature flag) while the column
//! itself stays a proper `TIMESTAMPTZ`.

use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use domain::{
    AdminAccount, AdminId, AdminRepository, Email, IpLockoutStore, LockoutState, NewAdmin,
    PasswordHash, RepositoryError, Role,
};
use sqlx::postgres::PgPool;
use sqlx::Row;

/// Convert a `SystemTime` to whole Unix epoch seconds, if representable.
///
/// Times before the epoch (not expected for lockout deadlines) collapse to
/// `None`, i.e. "not locked". Shared with [`crate::session_repo`], which
/// stores its timestamps the same way.
pub(crate) fn to_epoch(time: SystemTime) -> Option<i64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Reconstruct a `SystemTime` from Unix epoch seconds.
pub(crate) fn from_epoch(secs: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
}

/// Map an arbitrary sqlx error to a backend [`RepositoryError`].
fn backend(err: sqlx::Error) -> RepositoryError {
    RepositoryError::Backend(err.to_string())
}

/// Map a row from `admin_users` into an [`AdminAccount`].
fn row_to_account(row: &sqlx::postgres::PgRow) -> Result<Option<AdminAccount>, RepositoryError> {
    let id: String = row.try_get("id").map_err(backend)?;
    let email_str: String = row.try_get("email").map_err(backend)?;
    let password_hash: String = row.try_get("password_hash").map_err(backend)?;
    let failed_attempts: i32 = row.try_get("failed_attempts").map_err(backend)?;
    let role: String = row.try_get("role").map_err(backend)?;
    let display_name: Option<String> = row.try_get("display_name").map_err(backend)?;
    let locked_epoch: Option<i64> = row.try_get("locked_until_epoch").map_err(backend)?;

    let email = Email::parse(&email_str)
        .map_err(|e| RepositoryError::Backend(format!("stored email {email_str:?}: {e}")))?;
    let role = Role::parse(&role)
        .map_err(|e| RepositoryError::Backend(format!("stored role {role:?}: {e}")))?;

    Ok(Some(AdminAccount {
        id: AdminId::new(id),
        email,
        password_hash: PasswordHash::from_encoded(password_hash),
        lockout: LockoutState {
            failed_attempts: failed_attempts.max(0) as u32,
            locked_until: locked_epoch.map(from_epoch),
        },
        role,
        display_name,
    }))
}

/// A Postgres-backed [`AdminRepository`].
#[derive(Debug, Clone)]
pub struct PgAdminRepository {
    pool: PgPool,
}

impl PgAdminRepository {
    /// Build the repository over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AdminRepository for PgAdminRepository {
    async fn find_by_email(&self, email: &Email) -> Result<Option<AdminAccount>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id::text AS id, email, password_hash, failed_attempts, role, display_name, \
             EXTRACT(EPOCH FROM locked_until)::bigint AS locked_until_epoch \
             FROM admin_users WHERE email = $1",
        )
        .bind(email.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Ok(None);
        };

        row_to_account(&row)
    }

    async fn find_by_id(&self, id: &AdminId) -> Result<Option<AdminAccount>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id::text AS id, email, password_hash, failed_attempts, role, display_name, \
             EXTRACT(EPOCH FROM locked_until)::bigint AS locked_until_epoch \
             FROM admin_users WHERE id = $1::uuid",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Ok(None);
        };

        row_to_account(&row)
    }

    async fn update_lockout(
        &self,
        id: &AdminId,
        state: &LockoutState,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "UPDATE admin_users \
             SET failed_attempts = $2, locked_until = to_timestamp($3), updated_at = now() \
             WHERE id = $1::uuid",
        )
        .bind(id.as_str())
        .bind(state.failed_attempts as i32)
        .bind(state.locked_until.and_then(to_epoch))
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn count(&self) -> Result<u64, RepositoryError> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM admin_users")
            .fetch_one(&self.pool)
            .await
            .map_err(backend)?;
        let n: i64 = row.try_get("n").map_err(backend)?;
        Ok(n.max(0) as u64)
    }

    async fn insert(&self, admin: &NewAdmin) -> Result<AdminId, RepositoryError> {
        let result = sqlx::query(
            "INSERT INTO admin_users (email, password_hash, role) VALUES ($1, $2, $3) \
             RETURNING id::text AS id",
        )
        .bind(admin.email.as_str())
        .bind(admin.password_hash.as_str())
        .bind(admin.role.as_str())
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(row) => {
                let id: String = row.try_get("id").map_err(backend)?;
                Ok(AdminId::new(id))
            }
            // A duplicate email is a precise, expected outcome — surface it as
            // such so the bootstrap use case can report it cleanly.
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(RepositoryError::EmailTaken)
            }
            Err(e) => Err(backend(e)),
        }
    }
}

/// A Postgres-backed [`IpLockoutStore`].
#[derive(Debug, Clone)]
pub struct PgIpLockoutStore {
    pool: PgPool,
}

impl PgIpLockoutStore {
    /// Build the store over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IpLockoutStore for PgIpLockoutStore {
    async fn get(&self, ip: &str) -> Result<LockoutState, RepositoryError> {
        let row = sqlx::query(
            "SELECT failed_attempts, EXTRACT(EPOCH FROM locked_until)::bigint AS locked_until_epoch \
             FROM admin_ip_lockouts WHERE ip = $1",
        )
        .bind(ip)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        // No row means the IP has never failed: the cleared state.
        let Some(row) = row else {
            return Ok(LockoutState::clear());
        };

        let failed_attempts: i32 = row.try_get("failed_attempts").map_err(backend)?;
        let locked_epoch: Option<i64> = row.try_get("locked_until_epoch").map_err(backend)?;
        Ok(LockoutState {
            failed_attempts: failed_attempts.max(0) as u32,
            locked_until: locked_epoch.map(from_epoch),
        })
    }

    async fn put(&self, ip: &str, state: &LockoutState) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO admin_ip_lockouts (ip, failed_attempts, locked_until, updated_at) \
             VALUES ($1, $2, to_timestamp($3), now()) \
             ON CONFLICT (ip) DO UPDATE SET \
             failed_attempts = EXCLUDED.failed_attempts, \
             locked_until = EXCLUDED.locked_until, \
             updated_at = now()",
        )
        .bind(ip)
        .bind(state.failed_attempts as i32)
        .bind(state.locked_until.and_then(to_epoch))
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_roundtrips_whole_seconds() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_720_000_000);
        let secs = to_epoch(t).unwrap();
        assert_eq!(secs, 1_720_000_000);
        assert_eq!(from_epoch(secs), t);
    }

    #[test]
    fn pre_epoch_time_is_treated_as_unlocked() {
        // A defensive edge: a deadline before the epoch is not representable as
        // positive epoch seconds and should read as "no lock".
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(10);
        assert_eq!(to_epoch(before), None);
    }
}
