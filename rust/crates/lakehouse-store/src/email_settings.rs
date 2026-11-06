//! The deployment's email settings, saved from the console (`ADM-7`,
//! migration `0127`): the mail server, the account, the password and the
//! `From` address, plus the public address the console uses in links.
//!
//! At most one row exists (`singleton`). No row is not an error: it means
//! nothing was ever saved in the console, and the API supplies the
//! environment's value or the code default (`lakehouse-api`
//! `runtime_settings.rs`). Every field is nullable: `None` means "not set in
//! the console", so the caller falls through to the environment or the
//! default — the merge lives in exactly one place, the resolver, never here
//! (rule 4: one merge rule).
//!
//! # The password is sealed, never read here
//!
//! `smtp_pass` is `BYTEA`: the password sealed with the installation key
//! (`CONNECTOR_SECRETS_KEY`, `lakehouse-api` `secret_crypto.rs`) under the
//! associated-data name `settings/email/password`. This module stores and
//! returns the sealed bytes verbatim and never opens them; only the API's
//! resolver does, and only to build the sender. The plaintext never reaches
//! this module's memory, a log or a response.

use crate::{PgPool, StoreError};

/// The saved settings, as stored. Every setting column is `Option`: `None`
/// means "not set in the console".
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct EmailSettings {
    /// The mail server host, e.g. `smtp.example.com`.
    pub smtp_host: Option<String>,
    /// The mail server port, 1 to 65535 (the table's `CHECK`).
    pub smtp_port: Option<i32>,
    /// `"tls"` (implicit TLS) or `"starttls"` — the two modes the sender
    /// implements; the table's `CHECK` closes it to those two.
    pub smtp_security: Option<String>,
    /// The SMTP auth user name. `None` means no auth is sent.
    pub smtp_user: Option<String>,
    /// The SMTP auth password, SEALED (see the module doc comment). `None`
    /// means "not set in the console".
    pub smtp_pass: Option<Vec<u8>>,
    /// The `From` address, e.g. `reports@example.com`.
    pub smtp_from: Option<String>,
    /// The public address of the console, used in emailed links.
    pub console_public_url: Option<String>,
}

/// The saved settings, or `None` when nothing was ever saved in the console.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<EmailSettings>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT smtp_host, smtp_port, smtp_security, smtp_user, smtp_pass, smtp_from, \
             console_public_url FROM email_settings",
    )
    .fetch_optional(pool)
    .await?)
}

/// Save the settings, replacing any earlier row, and return what is stored.
/// A `None` field clears that setting in the console (falls through to the
/// environment or the default). `updated_by` is the saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails, including when the
/// table's `CHECK`s refuse the values.
pub async fn upsert(
    pool: &PgPool,
    settings: &EmailSettings,
    updated_by: &str,
) -> Result<EmailSettings, StoreError> {
    Ok(sqlx::query_as(
        "INSERT INTO email_settings \
             (singleton, smtp_host, smtp_port, smtp_security, smtp_user, smtp_pass, smtp_from, \
              console_public_url, updated_at, updated_by) \
         VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, now(), $8) \
         ON CONFLICT (singleton) DO UPDATE SET smtp_host = EXCLUDED.smtp_host, \
         smtp_port = EXCLUDED.smtp_port, smtp_security = EXCLUDED.smtp_security, \
         smtp_user = EXCLUDED.smtp_user, smtp_pass = EXCLUDED.smtp_pass, \
         smtp_from = EXCLUDED.smtp_from, console_public_url = EXCLUDED.console_public_url, \
         updated_at = now(), updated_by = EXCLUDED.updated_by \
         RETURNING smtp_host, smtp_port, smtp_security, smtp_user, smtp_pass, smtp_from, \
         console_public_url",
    )
    .bind(&settings.smtp_host)
    .bind(settings.smtp_port)
    .bind(&settings.smtp_security)
    .bind(&settings.smtp_user)
    .bind(&settings.smtp_pass)
    .bind(&settings.smtp_from)
    .bind(&settings.console_public_url)
    .bind(updated_by)
    .fetch_one(pool)
    .await?)
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

        let saved = upsert(
            &pool,
            &EmailSettings {
                smtp_host: Some("smtp.example.com".to_owned()),
                smtp_port: Some(465),
                smtp_security: Some("tls".to_owned()),
                smtp_user: Some("reports".to_owned()),
                smtp_pass: Some(vec![0x01, 0x02, 0x03]),
                smtp_from: Some("reports@example.com".to_owned()),
                console_public_url: Some("https://lakehouse.example.com".to_owned()),
            },
            "user-1",
        )
        .await
        .unwrap();

        assert_eq!(saved.smtp_host.as_deref(), Some("smtp.example.com"));
        assert_eq!(saved.smtp_port, Some(465));
        assert_eq!(saved.smtp_security.as_deref(), Some("tls"));
        assert_eq!(saved.smtp_user.as_deref(), Some("reports"));
        assert_eq!(saved.smtp_pass.as_deref(), Some(&[0x01, 0x02, 0x03][..]));
        assert_eq!(saved.smtp_from.as_deref(), Some("reports@example.com"));
        assert_eq!(
            saved.console_public_url.as_deref(),
            Some("https://lakehouse.example.com")
        );

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched, saved);
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clearing_a_field_stores_null(pool: PgPool) -> sqlx::Result<()> {
        upsert(
            &pool,
            &EmailSettings {
                smtp_host: Some("smtp.example.com".to_owned()),
                smtp_port: Some(587),
                smtp_security: None,
                smtp_user: None,
                smtp_pass: None,
                smtp_from: None,
                console_public_url: None,
            },
            "user-1",
        )
        .await
        .unwrap();

        // Clearing the host (the only set field) leaves every column null.
        let cleared = upsert(
            &pool,
            &EmailSettings {
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
                smtp_user: None,
                smtp_pass: None,
                smtp_from: None,
                console_public_url: None,
            },
            "user-1",
        )
        .await
        .unwrap();
        assert_eq!(cleared.smtp_host, None);
        assert_eq!(cleared.smtp_port, None);
        assert_eq!(cleared.smtp_pass, None);

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched.smtp_host, None);
        assert_eq!(fetched.smtp_port, None);
        assert_eq!(fetched.smtp_pass, None);
        Ok(())
    }
}
