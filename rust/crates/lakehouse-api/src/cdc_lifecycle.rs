//! The life of a change-capture connector in `Kafka Connect` (`SRC-4` slice 2,
//! decision S2-e, ADR 0016): provision it, keep its pause state in step with
//! the registry, read its state, remove it.
//!
//! # The registry is the truth
//!
//! The registry row decides whether a connector is paused (`paused_at`). The
//! connector in `Kafka Connect` follows it: [`sync_pause_state`] reads the row
//! and pauses or resumes the worker's connector to match. It is called after
//! every change of the row that this API makes (a person's pause or resume,
//! an auto-disable, a schema-change pause or its approval) and by
//! [`provision`], so a call that failed once is healed by the next one. While
//! it has not run, the applier already skips a paused connector (it reads the
//! registry), so nothing is loaded; the worker only keeps reading the source
//! into Kafka, which retains it.
//!
//! # Nothing here fails a registry change
//!
//! Pausing and resuming in `Kafka Connect` is best effort after the registry
//! row changed: a worker that cannot be reached is logged (a class, never its
//! text) and the answer of the route that changed the row is unchanged. The
//! two routes that exist to start or stop capture ([`provision`], and a
//! person's pause and resume in `routes::connector_runs`) report the failure,
//! as a fixed sentence.

use lakehouse_core::ApiError;
use lakehouse_store::cdc::{
    connect_connector_name, render_connect_config, render_mongodb_connect_config,
};
use lakehouse_store::connectors;
use lakehouse_store::ingest_spec::Dial;
use lakehouse_store::schema_change;
use sqlx::PgPool;

use crate::cdc_secrets::{CdcSecretError, CdcSecretStore, WriteOutcome};
use crate::kafka_connect::{ConnectError, ConnectState, KafkaConnectClient};
use crate::state::AppState;

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "connector store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn secret_error(err: CdcSecretError) -> ApiError {
    match err {
        CdcSecretError::InvalidId | CdcSecretError::Unresolved => {
            ApiError::Unprocessable(err.to_string())
        }
        CdcSecretError::Io => ApiError::Unavailable(err.to_string()),
    }
}

/// What [`provision`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provisioned {
    /// The connector is created or updated in `Kafka Connect`.
    Started,
    /// `KAFKA_CONNECT_URL` is not set, or the connector is `MySQL` or `SQL Server`
    /// and `CDC_KAFKA_BOOTSTRAP` is not: nothing was started (fail closed).
    NotConfigured,
    /// The connector is not a change-capture connector, or it names no table
    /// yet: there is nothing to capture.
    NothingToCapture,
}

/// The tables a change-capture connector's source objects name.
pub(crate) fn table_names(source_objects: &serde_json::Value) -> Vec<String> {
    source_objects
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|object| object.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// Create or update connector `id` in `Kafka Connect` from its registry row:
/// write its password file, render the connector JSON, `PUT` it, restart it if
/// the password changed, and bring its pause state in line. Idempotent.
///
/// # Errors
///
/// 422 with a fixed sentence when the connector's settings cannot be rendered
/// or its credential does not resolve; 503 when the secrets directory or
/// `Kafka Connect` is not usable; 404 for an unknown connector.
pub async fn provision(state: &AppState, id: &str) -> Result<Provisioned, ApiError> {
    let Some(client) = KafkaConnectClient::from_config(&state.config) else {
        return Ok(Provisioned::NotConfigured);
    };
    let pool = pool(state)?;
    let info = connectors::get_connector_dial_info(pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    if info.adapter.as_deref() != Some("cdc") {
        return Ok(Provisioned::NothingToCapture);
    }
    let spec = connectors::get_ingest_spec(pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    let tables = table_names(&spec.source_objects);
    if tables.is_empty() {
        return Ok(Provisioned::NothingToCapture);
    }
    let parsed =
        Dial::parse("cdc", &info.dial).map_err(|err| ApiError::Unprocessable(err.to_string()))?;
    let table_refs: Vec<&str> = tables.iter().map(String::as_str).collect();
    let config = if let Some(mongo) = parsed.as_cdc_mongo() {
        // `SRC-4` slice 5: a `MongoDB` connector keeps no schema history (a document
        // has no schema to remember), so it needs no broker address of its own.
        render_mongodb_connect_config(id, mongo, &table_refs, &state.config.cdc_secrets_dir)
            .map_err(|err| ApiError::Unprocessable(err.to_string()))?
    } else {
        let Some(dial) = parsed.as_cdc() else {
            return Ok(Provisioned::NothingToCapture);
        };
        // `MySQL` and `SQL Server` keep their schema history in a Kafka topic and
        // cannot start without the broker's address: unset is the same fail-closed
        // answer as an unset `KAFKA_CONNECT_URL` (`SRC-4` decision S3-b).
        let bootstrap = state.config.cdc_kafka_bootstrap.as_deref();
        if bootstrap.is_none() && dial.driver != lakehouse_store::ingest_spec::SqlDriver::Postgres {
            return Ok(Provisioned::NotConfigured);
        }
        render_connect_config(
            id,
            dial,
            &table_refs,
            &state.config.cdc_secrets_dir,
            bootstrap,
        )
        .map_err(|err| ApiError::Unprocessable(err.to_string()))?
    };
    let name =
        connect_connector_name(id).map_err(|err| ApiError::Unprocessable(err.to_string()))?;

    // The file before the connector: a task that starts reads it at once.
    let written = CdcSecretStore::new(&state.config.cdc_secrets_dir)
        .materialise(
            state.connector_secret_resolver.as_ref(),
            id,
            &info.secret_ref,
        )
        .await
        .map_err(secret_error)?;
    client.put_config(&name, &config).await?;
    if written == WriteOutcome::Changed {
        // The worker reads the password when a task starts; a rotated one needs
        // a restart. A connector just created has nothing to restart.
        match client.restart(&name).await {
            Ok(()) | Err(ConnectError::NotFound) => {}
            Err(err) => return Err(err.into()),
        }
    }
    sync_pause_state(state, id).await;
    Ok(Provisioned::Started)
}

/// The error a save answers when the settings were stored but the connector
/// could not be started: the same status, a sentence that says the save held.
#[must_use]
pub fn after_save(err: ApiError) -> ApiError {
    const SAVED: &str = "The settings were saved, but change capture did not start";
    match err {
        ApiError::Unprocessable(why) => ApiError::Unprocessable(format!("{SAVED}: {why}")),
        ApiError::Unavailable(why) => ApiError::Unavailable(format!("{SAVED}: {why}")),
        other => other,
    }
}

/// Bring connector `id`'s state in `Kafka Connect` in line with the registry
/// row's pause: paused there when paused here, resumed otherwise. Best effort,
/// see the module docs; does nothing for a connector that is not
/// change-capture or when `Kafka Connect` is not configured.
pub async fn sync_pause_state(state: &AppState, id: &str) {
    let Some(client) = KafkaConnectClient::from_config(&state.config) else {
        return;
    };
    let Ok(pool) = pool(state) else {
        return;
    };
    let is_cdc = matches!(
        connectors::get_connector_dial_info(pool, id).await,
        Ok(Some(info)) if info.adapter.as_deref() == Some("cdc")
    );
    if !is_cdc {
        return;
    }
    let Ok(Some(paused)) = schema_change::is_paused(pool, id).await else {
        return;
    };
    let Ok(name) = connect_connector_name(id) else {
        return;
    };
    let outcome = if paused {
        client.pause(&name).await
    } else {
        client.resume(&name).await
    };
    match outcome {
        // Not created yet (nothing to capture so far): nothing to pause.
        Ok(()) | Err(ConnectError::NotFound) => {}
        Err(err) => tracing::warn!(
            connector_id = %id,
            paused,
            error = %err,
            "could not bring the Kafka Connect connector's pause in line with the registry"
        ),
    }
}

/// Whether connector `id` is a change-capture connector (`adapter = cdc`); a
/// route then also acts on its `Kafka Connect` connector, and otherwise does
/// only its registry work.
///
/// # Errors
///
/// 404 for an unknown connector; 503 as the store reports.
pub async fn is_change_capture(state: &AppState, id: &str) -> Result<bool, ApiError> {
    let info = connectors::get_connector_dial_info(pool(state)?, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    Ok(info.adapter.as_deref() == Some("cdc"))
}

/// Pause connector `id` in `Kafka Connect` for a person's pause, reporting a
/// failure (unlike [`sync_pause_state`]). A connector that is not created yet,
/// or no `Kafka Connect` configured, is success: there is nothing to pause.
///
/// # Errors
///
/// The class of the `Kafka Connect` failure, as an [`ApiError`].
pub async fn pause_in_connect(state: &AppState, id: &str) -> Result<(), ApiError> {
    let Some(client) = KafkaConnectClient::from_config(&state.config) else {
        return Ok(());
    };
    let name =
        connect_connector_name(id).map_err(|err| ApiError::Unprocessable(err.to_string()))?;
    match client.pause(&name).await {
        Ok(()) | Err(ConnectError::NotFound) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// [`pause_in_connect`]'s counterpart for a person's resume.
///
/// # Errors
///
/// The class of the `Kafka Connect` failure, as an [`ApiError`].
pub async fn resume_in_connect(state: &AppState, id: &str) -> Result<(), ApiError> {
    let Some(client) = KafkaConnectClient::from_config(&state.config) else {
        return Ok(());
    };
    let name =
        connect_connector_name(id).map_err(|err| ApiError::Unprocessable(err.to_string()))?;
    match client.resume(&name).await {
        Ok(()) | Err(ConnectError::NotFound) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Remove connector `id` from `Kafka Connect`, before its slot is dropped (the
/// slot cannot be dropped while the worker reads it). A connector that is not
/// there is success. Does nothing when `Kafka Connect` is not configured: then
/// none was ever started.
///
/// # Errors
///
/// The class of the `Kafka Connect` failure, as an [`ApiError`].
pub async fn remove_from_connect(state: &AppState, id: &str) -> Result<(), ApiError> {
    let Some(client) = KafkaConnectClient::from_config(&state.config) else {
        return Ok(());
    };
    let name =
        connect_connector_name(id).map_err(|err| ApiError::Unprocessable(err.to_string()))?;
    client.delete(&name).await?;
    Ok(())
}

/// Remove connector `id`'s password file. Best effort: the registry row is
/// gone already, and the file is memory only.
pub fn remove_secret(state: &AppState, id: &str) {
    if let Err(err) = CdcSecretStore::new(&state.config.cdc_secrets_dir).remove(id) {
        tracing::warn!(connector_id = %id, error = %err, "could not remove a change-capture password file");
    }
}

/// The fixed state words of `GET /api/connectors/{id}/change-capture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureState {
    /// `Kafka Connect` is not configured on this deployment.
    NotConfigured,
    /// Configured, and the connector is not in the worker (never provisioned,
    /// or it names no table).
    NotCreated,
    /// The connector's state.
    State(ConnectState),
    /// `Kafka Connect` could not be asked right now.
    Unreachable,
}

/// Read connector `id`'s state in `Kafka Connect`.
///
/// # Errors
///
/// 404 for an unknown connector; 503/500 as the store reports.
pub async fn capture_state(state: &AppState, id: &str) -> Result<CaptureState, ApiError> {
    let Some(client) = KafkaConnectClient::from_config(&state.config) else {
        return Ok(CaptureState::NotConfigured);
    };
    let name =
        connect_connector_name(id).map_err(|err| ApiError::Unprocessable(err.to_string()))?;
    Ok(match client.state(&name).await {
        Ok(found) => CaptureState::State(found),
        Err(ConnectError::NotFound) => CaptureState::NotCreated,
        Err(_) => CaptureState::Unreachable,
    })
}

/// At start: for every change-capture connector, write its password file
/// again (`tmpfs` empties on a restart) and, if a file was missing or
/// different, restart its connector so the tasks read it. Then bring each
/// connector's pause in line. Retried a few times, because `Kafka Connect`
/// may still be starting; logs and gives up afterwards (the next provisioning
/// call heals it). Without `KAFKA_CONNECT_URL` it writes nothing.
pub async fn reconcile_at_start(state: &AppState) {
    const ATTEMPTS: u32 = 6;
    const WAIT: std::time::Duration = std::time::Duration::from_secs(10);
    if KafkaConnectClient::from_config(&state.config).is_none() {
        return;
    }
    for attempt in 1..=ATTEMPTS {
        match reconcile_once(state).await {
            Ok(()) => return,
            Err(err) if attempt < ATTEMPTS => {
                tracing::info!(attempt, error = %err, "change-capture start-up pass will retry");
                tokio::time::sleep(WAIT).await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "change-capture start-up pass gave up; saving a change-capture connector again heals it");
            }
        }
    }
}

async fn reconcile_once(state: &AppState) -> Result<(), ApiError> {
    let pool = pool(state)?;
    let ids: Vec<String> = connectors::list_ingestible_connectors(pool)
        .await?
        .into_iter()
        .filter(|c| c.adapter == "cdc")
        .map(|c| c.id)
        .collect();
    for id in ids {
        match provision(state, &id).await {
            Ok(_) => {}
            // The worker or the directory is not ready: the whole pass retries.
            Err(err @ ApiError::Unavailable(_)) => return Err(err),
            // This connector's own settings are unusable; the others go on.
            Err(err) => {
                tracing::warn!(connector_id = %id, error = %err, "change-capture connector not started");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn tables_are_the_source_object_names_in_order() {
        let objects = json!([
            {"name": "public.orders", "target": "a"},
            {"target": "no name"},
            {"name": "sales.items", "target": "b"},
        ]);
        assert_eq!(table_names(&objects), ["public.orders", "sales.items"]);
        assert_eq!(table_names(&json!({})), Vec::<String>::new());
        assert_eq!(table_names(&json!([])), Vec::<String>::new());
    }
}
