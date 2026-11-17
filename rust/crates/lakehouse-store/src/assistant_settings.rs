//! The deployment's assistant settings, saved from the console (`ADM-9`,
//! migration `0130`): the provider's address, the model and the API key.
//!
//! At most one row exists (`singleton`). No row is not an error: it means
//! nothing was ever saved in the console, and the API supplies the
//! environment's value or the code default (`lakehouse-api`
//! `runtime_settings.rs`). Every field is nullable: `None` means "not set in
//! the console", so the caller falls through to the environment or the
//! default — the merge lives in exactly one place, the resolver, never here
//! (rule 4: one merge rule).
//!
//! # The key is sealed, never read here
//!
//! `llm_key` is `BYTEA`: the key sealed with the installation key
//! (`CONNECTOR_SECRETS_KEY`, `lakehouse-api` `secret_crypto.rs`) under the
//! associated-data name `settings/assistant/key`. This module stores and
//! returns the sealed bytes verbatim and never opens them; only the API's
//! resolver does, and only to build the client. The plaintext never reaches
//! this module's memory, a log or a response.

use crate::{PgPool, StoreError};

/// The saved settings, as stored. Every setting column is `Option`: `None`
/// means "not set in the console".
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
#[allow(
    clippy::struct_field_names,
    reason = "the field names mirror the environment variables the settings \
              come from (`LLM_URL`, `LLM_MODEL`, `LLM_KEY`) and the table's \
              column names, so a reader can match them one to one"
)]
pub struct AssistantSettings {
    /// The provider's base URL, e.g. `https://api.example.com/v1`.
    pub llm_url: Option<String>,
    /// The provider's model name, e.g. `example-model`.
    pub llm_model: Option<String>,
    /// The provider's API key, SEALED (see the module doc comment). `None`
    /// means "not set in the console".
    pub llm_key: Option<Vec<u8>>,
}

/// The saved settings, or `None` when nothing was ever saved in the console.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<AssistantSettings>, StoreError> {
    Ok(
        sqlx::query_as("SELECT llm_url, llm_model, llm_key FROM assistant_settings")
            .fetch_optional(pool)
            .await?,
    )
}

/// Save the settings, replacing any earlier row, and return what is stored.
/// A `None` field clears that setting in the console (falls through to the
/// environment or the default). `updated_by` is the saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn upsert(
    pool: &PgPool,
    settings: &AssistantSettings,
    updated_by: &str,
) -> Result<AssistantSettings, StoreError> {
    Ok(sqlx::query_as(
        "INSERT INTO assistant_settings \
             (singleton, llm_url, llm_model, llm_key, updated_at, updated_by) \
         VALUES (TRUE, $1, $2, $3, now(), $4) \
         ON CONFLICT (singleton) DO UPDATE SET llm_url = EXCLUDED.llm_url, \
              llm_model = EXCLUDED.llm_model, llm_key = EXCLUDED.llm_key, \
              updated_at = now(), updated_by = EXCLUDED.updated_by \
         RETURNING llm_url, llm_model, llm_key",
    )
    .bind(&settings.llm_url)
    .bind(&settings.llm_model)
    .bind(&settings.llm_key)
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
            &AssistantSettings {
                llm_url: Some("https://api.example.com/v1".to_owned()),
                llm_model: Some("example-model".to_owned()),
                llm_key: Some(vec![0x01, 0x02, 0x03]),
            },
            "user-1",
        )
        .await
        .unwrap();

        assert_eq!(saved.llm_url.as_deref(), Some("https://api.example.com/v1"));
        assert_eq!(saved.llm_model.as_deref(), Some("example-model"));
        assert_eq!(saved.llm_key.as_deref(), Some(&[0x01, 0x02, 0x03][..]));

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched, saved);
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clearing_a_field_stores_null(pool: PgPool) -> sqlx::Result<()> {
        upsert(
            &pool,
            &AssistantSettings {
                llm_url: Some("https://api.example.com/v1".to_owned()),
                llm_model: Some("example-model".to_owned()),
                llm_key: Some(vec![0x01]),
            },
            "user-1",
        )
        .await
        .unwrap();

        let cleared = upsert(
            &pool,
            &AssistantSettings {
                llm_url: None,
                llm_model: None,
                llm_key: None,
            },
            "user-1",
        )
        .await
        .unwrap();
        assert_eq!(cleared.llm_url, None);
        assert_eq!(cleared.llm_model, None);
        assert_eq!(cleared.llm_key, None);

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched.llm_key, None);
        Ok(())
    }
}
