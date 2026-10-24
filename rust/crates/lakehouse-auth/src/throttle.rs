//! Per-account login throttling (SEC-2, login-protection-and-session-cleanup
//! plan). Keyed by a SHA-256 hash of the submitted email — no email is ever
//! stored for accounts that do not exist.
//!
//! # Non-enumeration
//!
//! [`key_for`] always produces a hash, whether or not the account exists,
//! and the caller is responsible for computing it the same way in both
//! branches. A locked key never reaches password verification, so the
//! Argon2id cost is saved on a locked attempt.

use sha2::Digest as _;
use time::Duration;
use time::OffsetDateTime;

use crate::{AuthError, PgPool};

/// The lockout defaults from the plan: 5 failures within 900 s lock the
/// key for 300 s. Each is an env setting; zero/negative/unparseable falls
/// back to the default (enforced by [`crate::Config`] — this struct is
/// constructed from already-validated values and carries no fallback
/// logic of its own).
/// How many failed attempts are tolerated within [`Self::window`] before
/// [`Self::lockout`] is applied to the key.
#[derive(Debug, Clone)]
pub struct ThrottlePolicy {
    /// Failed attempts tolerated within [`Self::window`] before the key
    /// locks.
    pub max_failures: u32,
    /// How long a failure counts toward [`Self::max_failures`]; failures
    /// older than this stop counting (the count restarts at 1 on the next
    /// failure).
    pub window: Duration,
    /// How long a key stays locked once [`Self::max_failures`] is
    /// reached.
    pub lockout: Duration,
}

/// SHA-256 hex of the trimmed, lower-cased email. Always produces a
/// 64-character hex string whether or not the account exists, so a locked
/// key never reveals which branch the caller is on.
///
/// Does not reuse `crate::token::hash_token` — `hash_token` takes
/// `&Secret` (a wrapper with `Debug`-redaction guarantees) and is designed
/// for opaque bearer tokens, not for one-shot hashing of a caller-provided
/// plaintext string.
#[must_use]
pub fn key_for(email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    let digest = sha2::Sha256::digest(normalized.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{byte:02x}"));
    }
    out
}

/// `Some(t)` while `locked_until > now()`, otherwise `None` — including
/// when the key has no row at all or the lock has passed.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn locked_until(pool: &PgPool, key: &str) -> Result<Option<OffsetDateTime>, AuthError> {
    let row: Option<(Option<OffsetDateTime>,)> =
        sqlx::query_as("SELECT locked_until FROM login_throttle WHERE key_hash = $1")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    let Some((Some(locked),)) = row else {
        return Ok(None);
    };
    let now = OffsetDateTime::now_utc();
    if locked <= now {
        return Ok(None);
    }
    Ok(Some(locked))
}

/// Record a failed login attempt for `key`. One atomic SQL statement:
/// `INSERT … ON CONFLICT (key_hash) DO UPDATE` — two concurrent failures
/// both increment the count.
///
/// The count restarts at 1 in two cases: the row's window has passed, or
/// the row was locked and that lock has expired (review SHOULD-FIX 1,
/// 2026-10-03 — without this, one typo after a lock expires immediately
/// re-locks the key for the full lockout again). `locked_until < now`
/// covers the expired lock; `NULL` (never locked) yields `NULL`, which a
/// `CASE WHEN` treats as false.
///
/// A reset must also CLEAR the stale lock time (`locked_until = NULL`,
/// review blocker 5, 2026-10-03): an expired `locked_until` left on the
/// row satisfies `locked_until < now` on every later failure, so the
/// count would reset to 1 forever and the key could never lock again —
/// one lockout, then unlimited guessing. With the clear, the expired-lock
/// clause fires exactly once, and the fresh window alone governs until it
/// lapses or the key reaches the limit again.
///
/// Returns `Some(locked_until)` if this call just set the lock,
/// `None` otherwise.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn record_failure(
    pool: &PgPool,
    key: &str,
    policy: &ThrottlePolicy,
) -> Result<Option<OffsetDateTime>, AuthError> {
    let now = OffsetDateTime::now_utc();
    let window_interval = format!("{} seconds", policy.window.whole_seconds());
    let lockout_interval = format!("{} seconds", policy.lockout.whole_seconds());
    // One SQL statement: if the old window has passed OR the previous
    // lock has expired, start a new window at 1; otherwise increment.
    // When the resulting count >= max_failures, set locked_until =
    // now() + lockout. The reset predicate appears once per assignment
    // because ON CONFLICT DO UPDATE assignments only see the pre-update
    // row — they cannot reference each other.
    let row: Option<(i32, Option<OffsetDateTime>)> = sqlx::query_as(
        "INSERT INTO login_throttle (key_hash, failures, window_started_at, locked_until) \
         VALUES ($1, 1, $2, NULL) \
         ON CONFLICT (key_hash) DO UPDATE SET \
           failures = CASE \
             WHEN login_throttle.window_started_at < $2 - $3::interval \
               OR login_throttle.locked_until < $2 \
               THEN 1 \
             ELSE login_throttle.failures + 1 \
           END, \
           window_started_at = CASE \
             WHEN login_throttle.window_started_at < $2 - $3::interval \
               OR login_throttle.locked_until < $2 \
               THEN $2 \
             ELSE login_throttle.window_started_at \
           END, \
           locked_until = CASE \
             WHEN (CASE \
               WHEN login_throttle.window_started_at < $2 - $3::interval \
                 OR login_throttle.locked_until < $2 \
                 THEN 1 \
               ELSE login_throttle.failures + 1 \
             END) >= $4 \
               THEN $2 + $5::interval \
             WHEN login_throttle.window_started_at < $2 - $3::interval \
               OR login_throttle.locked_until < $2 \
               THEN NULL \
             ELSE login_throttle.locked_until \
           END \
         RETURNING login_throttle.failures, login_throttle.locked_until",
    )
    .bind(key)
    .bind(now)
    .bind(&window_interval)
    .bind(i32::try_from(policy.max_failures).unwrap_or(i32::MAX))
    .bind(&lockout_interval)
    .fetch_optional(pool)
    .await?;
    let Some((failures, locked)) = row else {
        return Ok(None);
    };
    let max: i32 = i32::try_from(policy.max_failures).unwrap_or(i32::MAX);
    if failures >= max {
        Ok(locked)
    } else {
        Ok(None)
    }
}

/// Count an attempt BEFORE it is checked, and say which attempt it is and until
/// when the key is locked, if it is (one atomic UPSERT ... RETURNING)
/// (`BI-19` part A review fix (BLOCKER) R1).
///
/// [`locked_until`] then [`record_failure`] counts only after the (slow)
/// verification, so any number of simultaneous guesses all pass the lock check
/// before the first failure is written, and each runs a full hash. This is the
/// other order: one atomic statement adds the attempt and returns the count, and
/// the caller verifies only while the count is at most
/// [`ThrottlePolicy::max_failures`]. Whatever number of requests arrive at once,
/// at most `max_failures` of them reach the verifier within a window.
///
/// The count restarts at 1 when the window has passed with no lock, or when a
/// lock has expired. The attempt that reaches `max_failures` sets the lock for
/// [`ThrottlePolicy::lockout`]; every later attempt, refused or not, keeps it
/// and is counted. The caller calls [`clear`] on success, so a person who types
/// the right password never counts toward a lock.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn record_attempt(
    pool: &PgPool,
    key: &str,
    policy: &ThrottlePolicy,
) -> Result<(u32, Option<OffsetDateTime>), AuthError> {
    let now = OffsetDateTime::now_utc();
    let window_interval = format!("{} seconds", policy.window.whole_seconds());
    let lockout_interval = format!("{} seconds", policy.lockout.whole_seconds());
    let max = i32::try_from(policy.max_failures).unwrap_or(i32::MAX);
    // `reset`: the window passed with no lock, or the lock has expired. It is
    // written out in each assignment because ON CONFLICT DO UPDATE assignments
    // only see the pre-update row.
    let (count, locked): (i32, Option<OffsetDateTime>) = sqlx::query_as(
        "INSERT INTO login_throttle (key_hash, failures, window_started_at, locked_until) \
         VALUES ($1, 1, $2, CASE WHEN 1 >= $4 THEN $2 + $5::interval END) \
         ON CONFLICT (key_hash) DO UPDATE SET \
           failures = CASE \
             WHEN COALESCE(login_throttle.locked_until < $2, \
                           login_throttle.window_started_at < $2 - $3::interval) \
               THEN 1 ELSE login_throttle.failures + 1 END, \
           window_started_at = CASE \
             WHEN COALESCE(login_throttle.locked_until < $2, \
                           login_throttle.window_started_at < $2 - $3::interval) \
               THEN $2 ELSE login_throttle.window_started_at END, \
           locked_until = CASE \
             WHEN COALESCE(login_throttle.locked_until < $2, \
                           login_throttle.window_started_at < $2 - $3::interval) \
               THEN CASE WHEN 1 >= $4 THEN $2 + $5::interval END \
             WHEN login_throttle.failures + 1 >= $4 \
               THEN COALESCE(login_throttle.locked_until, $2 + $5::interval) \
             ELSE login_throttle.locked_until END \
         RETURNING login_throttle.failures, login_throttle.locked_until",
    )
    .bind(key)
    .bind(now)
    .bind(&window_interval)
    .bind(max)
    .bind(&lockout_interval)
    .fetch_one(pool)
    .await?;
    Ok((u32::try_from(count).unwrap_or(u32::MAX), locked))
}

/// Clear the throttle row for `key` — called on a successful login so the
/// count resets.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn clear(pool: &PgPool, key: &str) -> Result<(), AuthError> {
    sqlx::query("DELETE FROM login_throttle WHERE key_hash = $1")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn key_for_is_deterministic() {
        assert_eq!(key_for("User@Example.Com"), key_for("  user@example.com  "));
    }

    #[test]
    fn key_for_is_64_hex_characters() {
        let key = key_for("test@example.com");
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn key_for_different_emails_produce_different_keys() {
        assert_ne!(key_for("a@b.com"), key_for("c@d.com"));
    }
}
