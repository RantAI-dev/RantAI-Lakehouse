//! The deployment's single sign-on (OIDC) settings, saved from the console
//! (`ADM-12`, migration `0134`; `SEC-65`, migration `0136`): the issuer,
//! client ID, client secret, provider name, groups claim, role map, JIT
//! provisioning toggle, and "trust this provider's emails" switch.
//!
//! At most one row exists in `sso_settings` (`singleton`). No row means nothing
//! was saved in the console, and the API supplies the environment's values or
//! code defaults (`lakehouse-api` `runtime_settings.rs`). Every field is
//! nullable: `None` means "not set in the console", so the caller falls
//! through to the environment or defaults (rule 4: one merge rule).
//!
//! # The client secret is sealed, never read here
//!
//! `oidc_client_secret` and `sso_test_run.draft_secret` are `BYTEA`: the secret
//! sealed with the installation key (`CONNECTOR_SECRETS_KEY`, `lakehouse-api`
//! `secret_crypto.rs`) under the associated-data name
//! `settings/sso/client_secret`. This module stores and returns the sealed bytes
//! verbatim and never opens them; only the API's resolver does. Plaintext never
//! reaches this module's memory, a log or a response.

use crate::{PgPool, StoreError};
use time::OffsetDateTime;
use uuid::Uuid;

/// The saved single sign-on settings, as stored. Every setting column is
/// `Option`: `None` means "not set in the console".
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
#[allow(
    clippy::struct_field_names,
    reason = "the field names mirror the environment variables the settings \
              come from (`OIDC_ISSUER`, etc.) and the table's column names, \
              so a reader can match them one to one"
)]
pub struct SsoSettings {
    /// The OIDC provider issuer URL, e.g. `https://accounts.google.com`.
    pub oidc_issuer: Option<String>,
    /// The application client id registered with the OIDC provider.
    pub oidc_client_id: Option<String>,
    /// The OIDC client secret, SEALED with the installation key.
    pub oidc_client_secret: Option<Vec<u8>>,
    /// Operator-chosen provider label, e.g. `okta`. Default `default`.
    pub oidc_provider_name: Option<String>,
    /// Claim carrying user groups/roles, e.g. `groups`. Default `groups`.
    pub oidc_groups_claim: Option<String>,
    /// Role map JSON array of `{group, role}` objects.
    pub oidc_role_map: Option<serde_json::Value>,
    /// Whether unrecognized OIDC subjects are allowed to provision new users.
    pub oidc_jit_provisioning: Option<bool>,
    /// Whether this provider's emails are trusted without the
    /// `email_verified` claim (`SEC-65` D3). `None` means "not set in the
    /// console", which resolves to the environment or `false`.
    pub oidc_trust_email: Option<bool>,
}

/// A sign-in test run against a draft configuration.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SsoTestRun {
    /// Unique ID of the test run.
    pub id: Uuid,
    /// Principal ID who started the test run.
    pub created_by: Uuid,
    /// The draft configuration JSON (excluding client secret).
    pub draft: serde_json::Value,
    /// The draft client secret, SEALED with the installation key.
    pub draft_secret: Option<Vec<u8>>,
    /// The test result JSON (subject, email, groups, mapped roles, or error).
    pub result: Option<serde_json::Value>,
    /// Expiration timestamp (10 minutes after creation).
    pub expires_at: OffsetDateTime,
}

/// Input parameters to create a new [`SsoTestRun`].
#[derive(Debug, Clone)]
pub struct NewSsoTestRun {
    /// Explicit ID of the test run.
    pub id: Uuid,
    /// Principal ID who created the test run.
    pub created_by: Uuid,
    /// The draft configuration JSON.
    pub draft: serde_json::Value,
    /// The sealed client secret, if provided.
    pub draft_secret: Option<Vec<u8>>,
    /// Expiration timestamp.
    pub expires_at: OffsetDateTime,
}

/// The saved SSO settings, or `None` when nothing was ever saved in the console.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<SsoSettings>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT oidc_issuer, oidc_client_id, oidc_client_secret, oidc_provider_name, \
                oidc_groups_claim, oidc_role_map, oidc_jit_provisioning, \
                oidc_trust_email \
         FROM sso_settings",
    )
    .fetch_optional(pool)
    .await?)
}

/// Save the settings, replacing any earlier row, and return what is stored.
/// A `None` field clears that setting in the console. `updated_by` is the
/// saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn upsert(
    pool: &PgPool,
    settings: &SsoSettings,
    updated_by: &str,
) -> Result<SsoSettings, StoreError> {
    Ok(sqlx::query_as(
        "INSERT INTO sso_settings \
             (singleton, oidc_issuer, oidc_client_id, oidc_client_secret, \
              oidc_provider_name, oidc_groups_claim, oidc_role_map, \
              oidc_jit_provisioning, oidc_trust_email, updated_at, updated_by) \
         VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, now(), $9) \
         ON CONFLICT (singleton) DO UPDATE SET \
              oidc_issuer = EXCLUDED.oidc_issuer, \
              oidc_client_id = EXCLUDED.oidc_client_id, \
              oidc_client_secret = EXCLUDED.oidc_client_secret, \
              oidc_provider_name = EXCLUDED.oidc_provider_name, \
              oidc_groups_claim = EXCLUDED.oidc_groups_claim, \
              oidc_role_map = EXCLUDED.oidc_role_map, \
              oidc_jit_provisioning = EXCLUDED.oidc_jit_provisioning, \
              oidc_trust_email = EXCLUDED.oidc_trust_email, \
              updated_at = now(), \
              updated_by = EXCLUDED.updated_by \
         RETURNING oidc_issuer, oidc_client_id, oidc_client_secret, \
                   oidc_provider_name, oidc_groups_claim, oidc_role_map, \
                   oidc_jit_provisioning, oidc_trust_email",
    )
    .bind(&settings.oidc_issuer)
    .bind(&settings.oidc_client_id)
    .bind(&settings.oidc_client_secret)
    .bind(&settings.oidc_provider_name)
    .bind(&settings.oidc_groups_claim)
    .bind(&settings.oidc_role_map)
    .bind(settings.oidc_jit_provisioning)
    .bind(settings.oidc_trust_email)
    .bind(updated_by)
    .fetch_one(pool)
    .await?)
}

/// Insert a new test run, cleaning up any expired test runs in the same call
/// (plan D8: expired rows deleted when the next test is created).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn create_test_run(pool: &PgPool, run: &NewSsoTestRun) -> Result<SsoTestRun, StoreError> {
    // Clean up expired runs first.
    sqlx::query("DELETE FROM sso_test_run WHERE expires_at < now()")
        .execute(pool)
        .await?;

    Ok(sqlx::query_as(
        "INSERT INTO sso_test_run (id, created_by, draft, draft_secret, result, expires_at) \
         VALUES ($1, $2, $3, $4, NULL, $5) \
         RETURNING id, created_by, draft, draft_secret, result, expires_at",
    )
    .bind(run.id)
    .bind(run.created_by)
    .bind(&run.draft)
    .bind(&run.draft_secret)
    .bind(run.expires_at)
    .fetch_one(pool)
    .await?)
}

/// Read a test run by its ID.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_test_run(pool: &PgPool, id: Uuid) -> Result<Option<SsoTestRun>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT id, created_by, draft, draft_secret, result, expires_at \
         FROM sso_test_run WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// Update the result of a test run after callback execution.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the update fails.
pub async fn update_test_run_result(
    pool: &PgPool,
    id: Uuid,
    result: &serde_json::Value,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE sso_test_run SET result = $1 WHERE id = $2")
        .bind(result)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_upsert_round_trips_and_null_columns_stay_null(pool: PgPool) -> sqlx::Result<()> {
        assert!(
            get(&pool).await.unwrap().is_none(),
            "no row before the first save"
        );

        let role_map = serde_json::json!([
            {"group": "admins", "role": "Platform Admin"},
            {"group": "viewers", "role": "Viewer"}
        ]);

        let saved = upsert(
            &pool,
            &SsoSettings {
                oidc_issuer: Some("https://idp.example.com".to_owned()),
                oidc_client_id: Some("client-123".to_owned()),
                oidc_client_secret: Some(vec![0x01, 0x02, 0x03]),
                oidc_provider_name: Some("okta".to_owned()),
                oidc_groups_claim: Some("custom_groups".to_owned()),
                oidc_role_map: Some(role_map.clone()),
                oidc_jit_provisioning: Some(true),
                oidc_trust_email: Some(true),
            },
            "user-1",
        )
        .await
        .unwrap();

        assert_eq!(
            saved.oidc_issuer.as_deref(),
            Some("https://idp.example.com")
        );
        assert_eq!(saved.oidc_client_id.as_deref(), Some("client-123"));
        assert_eq!(
            saved.oidc_client_secret.as_deref(),
            Some(&[0x01, 0x02, 0x03][..])
        );
        assert_eq!(saved.oidc_provider_name.as_deref(), Some("okta"));
        assert_eq!(saved.oidc_groups_claim.as_deref(), Some("custom_groups"));
        assert_eq!(saved.oidc_role_map.as_ref(), Some(&role_map));
        assert_eq!(saved.oidc_jit_provisioning, Some(true));

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched, saved);
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clearing_a_field_stores_null(pool: PgPool) -> sqlx::Result<()> {
        upsert(
            &pool,
            &SsoSettings {
                oidc_issuer: Some("https://idp.example.com".to_owned()),
                oidc_client_id: Some("client-123".to_owned()),
                oidc_client_secret: None,
                oidc_provider_name: None,
                oidc_groups_claim: None,
                oidc_role_map: None,
                oidc_jit_provisioning: None,
                oidc_trust_email: None,
            },
            "user-1",
        )
        .await
        .unwrap();

        let cleared = upsert(
            &pool,
            &SsoSettings {
                oidc_issuer: None,
                oidc_client_id: None,
                oidc_client_secret: None,
                oidc_provider_name: None,
                oidc_groups_claim: None,
                oidc_role_map: None,
                oidc_jit_provisioning: None,
                oidc_trust_email: None,
            },
            "user-1",
        )
        .await
        .unwrap();

        assert_eq!(cleared.oidc_issuer, None);
        assert_eq!(cleared.oidc_client_id, None);
        assert_eq!(cleared.oidc_client_secret, None);

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched.oidc_issuer, None);
        assert_eq!(fetched.oidc_client_id, None);
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn test_run_creation_and_expiration(pool: PgPool) -> sqlx::Result<()> {
        let user_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let draft = serde_json::json!({
            "issuer": "https://idp.example.com",
            "clientId": "client-123"
        });

        // Insert an expired test run directly to test cleanup
        let expired_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO sso_test_run (id, created_by, draft, expires_at) \
             VALUES ($1, $2, $3, now() - INTERVAL '1 hour')",
        )
        .bind(expired_id)
        .bind(user_id)
        .bind(&draft)
        .execute(&pool)
        .await?;

        // Creating a new test run should clean up the expired one
        let new_run = NewSsoTestRun {
            id: run_id,
            created_by: user_id,
            draft: draft.clone(),
            draft_secret: Some(vec![0xaa, 0xbb]),
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(10),
        };

        let created = create_test_run(&pool, &new_run).await.unwrap();
        assert_eq!(created.id, run_id);
        assert_eq!(created.created_by, user_id);
        assert_eq!(created.draft, draft);
        assert_eq!(created.draft_secret.as_deref(), Some(&[0xaa, 0xbb][..]));
        assert_eq!(created.result, None);

        // Expired one is gone
        assert!(get_test_run(&pool, expired_id).await.unwrap().is_none());

        // Fetched matches
        let fetched = get_test_run(&pool, run_id).await.unwrap().unwrap();
        assert_eq!(fetched.id, run_id);

        // Update result
        let result = serde_json::json!({
            "subject": "sub-1",
            "email": "user@example.com"
        });
        update_test_run_result(&pool, run_id, &result)
            .await
            .unwrap();

        let updated = get_test_run(&pool, run_id).await.unwrap().unwrap();
        assert_eq!(updated.result.as_ref(), Some(&result));

        Ok(())
    }
}
