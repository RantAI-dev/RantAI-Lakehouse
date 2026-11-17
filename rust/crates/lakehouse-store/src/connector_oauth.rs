//! The pending sign-ins of OAuth connectors (`SRC-12` part C, decisions 6 and
//! 12; table `connector_oauth_state`, migration
//! `0080_connector_oauth_state.sql` -- read its header for the row).
//!
//! A sign-in is started by one user for one connector ([`begin`]) and finished
//! by the provider's redirect to the callback ([`consume`]). The two calls
//! carry the properties the callback depends on:
//!
//! - the row is found by the SHA-256 of the `state`, never the `state` itself;
//! - [`consume`] deletes the row in the same statement that reads it, so a
//!   `state` works once whatever follows (a replay finds nothing, and a
//!   callback refused for another reason has still burned it);
//! - the expiry is the database's clock: 10 minutes after [`begin`];
//! - the PKCE verifier is returned to the callback and nowhere else.

use crate::{PgPool, StoreError};

/// How long a sign-in may take, in seconds (10 minutes; `SRC-12` decision 6).
pub const STATE_TTL_SECONDS: i32 = 600;

/// What the callback gets back for a live `state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The connector the sign-in was started for.
    pub connector_id: String,
    /// The user who started it.
    pub principal_id: String,
    /// The PKCE verifier. A secret for the length of the sign-in: the caller
    /// sends it to the token endpoint and drops it.
    pub code_verifier: String,
}

/// Record a sign-in. Expired rows, and an earlier sign-in of the same user for
/// the same connector, are removed first (the new one replaces it).
///
/// `state_hash` is 64 lower-case hex characters (the table checks it) and
/// `code_verifier` 43 to 128 characters (RFC 7636).
///
/// # Errors
///
/// [`StoreError::NotFound`] (a foreign-key violation) for an unknown
/// connector, [`StoreError::Conflict`] for a `state_hash` that already exists,
/// [`StoreError::Validation`] for a value the table would refuse, and
/// [`StoreError::Database`] otherwise.
pub async fn begin(
    pool: &PgPool,
    state_hash: &str,
    connector_id: &str,
    principal_id: &str,
    code_verifier: &str,
) -> Result<(), StoreError> {
    let hash_ok = state_hash.len() == 64
        && state_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !hash_ok || !(43..=128).contains(&code_verifier.len()) {
        return Err(StoreError::Validation(
            "a sign-in needs a hashed state and a PKCE verifier of 43 to 128 characters".to_owned(),
        ));
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM connector_oauth_state \
         WHERE expires_at <= now() OR (connector_id = $1 AND principal_id = $2)",
    )
    .bind(connector_id)
    .bind(principal_id)
    .execute(&mut *tx)
    .await?;
    let inserted = sqlx::query(
        "INSERT INTO connector_oauth_state \
           (state_hash, connector_id, principal_id, code_verifier, expires_at) \
         VALUES ($1, $2, $3, $4, now() + ($5::int * interval '1 second'))",
    )
    .bind(state_hash)
    .bind(connector_id)
    .bind(principal_id)
    .bind(code_verifier)
    .bind(STATE_TTL_SECONDS)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(StoreError::NotFound);
        }
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            return Err(StoreError::Conflict);
        }
        Err(err) => return Err(err.into()),
    }
    tx.commit().await?;
    Ok(())
}

/// Take a sign-in out of the table: `Some` for a `state` that exists and has
/// not expired, `None` for an unknown, expired or already-used one. The row is
/// deleted either way.
///
/// # Errors
///
/// [`StoreError::Database`] when the statement fails.
pub async fn consume(pool: &PgPool, state_hash: &str) -> Result<Option<Pending>, StoreError> {
    let row: Option<(String, String, String, bool)> = sqlx::query_as(
        "DELETE FROM connector_oauth_state WHERE state_hash = $1 \
         RETURNING connector_id, principal_id, code_verifier, expires_at > now()",
    )
    .bind(state_hash)
    .fetch_optional(pool)
    .await?;
    Ok(
        row.and_then(|(connector_id, principal_id, code_verifier, live)| {
            live.then_some(Pending {
                connector_id,
                principal_id,
                code_verifier,
            })
        }),
    )
}
