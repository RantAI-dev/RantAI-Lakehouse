//! The connection test of an Oracle change-capture connector (`SRC-4` slice 6,
//! decision S10-a).
//!
//! # Why `Kafka Connect` runs the test
//!
//! The connector reaches Oracle with Oracle's JDBC driver, which can require
//! native network encryption (AES256, SHA256 checksum: decision S6-c). The API
//! has no Oracle client, and the orchestrator's `python-oracledb` thin mode has
//! no native network encryption at all (`oracledb` 2.5.1 and 26.1.0), so a test
//! there would connect unencrypted and not show what the connector will do. So
//! the test asks `Kafka Connect` to validate the connector configuration the
//! API would create (`PUT /connector-plugins/<class>/config/validate`): the same
//! renderer, the same driver and the same encryption keys.
//!
//! # Same order as every other dial
//!
//! [`crate::connector_probe::resolve_checked`] runs first, before the credential
//! is resolved or anything dials. The worker resolves the host again itself
//! (the same gap the create path has: the worker dials a name the API checked).
//! The password reaches the worker only as a `${dir:...}` reference to the file
//! the create path writes ([`crate::cdc_secrets`]); it is never in the request.
//!
//! # Upstream text never reaches a response
//!
//! `Kafka Connect` answers with the driver's own messages. They are matched
//! here against fixed patterns ([`classify`]) and the answer is one of the
//! fixed sentences of [`OracleTestFailure`]; the class goes to the log, never
//! the text (principle 4).

use std::time::Instant;

use lakehouse_store::cdc::{connect_connector_name, render_connect_config};
use lakehouse_store::connectors::ConnectorDialInfo;
use lakehouse_store::ingest_spec::{Dial, SqlDriver};

use crate::connector_probe::{Outcome, resolve_checked};
use crate::kafka_connect::{ConfigValidation, ConnectError, KafkaConnectClient, NOT_CONFIGURED};
use crate::state::AppState;

/// Said in the test result when the orchestrator also reads Oracle's setup facts (`SRC-4` Amendment
/// 11, S11-c): that read is the owner-signed exception to `SEC-14`'s encryption rule, limited to the
/// data dictionary. Fixed text, shown as is.
pub const UNENCRYPTED_SETUP_READ: &str = "Setup checks are read over an unencrypted connection; \
    the password is not sent in clear text, the session is not encrypted.";

/// A table the test names when the connector selects none yet. The connector
/// is only validated, never created, so it captures nothing; the renderer needs
/// one name, and `Debezium`'s validation does not look for the table.
const PLACEHOLDER_TABLE: &str = "LAKEHOUSE_TEST.NO_TABLE";

/// Why the validation did not pass, as a class with a fixed sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OracleTestFailure {
    /// `ORA-01017`: the login was refused.
    #[error("Oracle connection failed: authentication failed")]
    WrongLogin,
    /// `ORA-12541`, `ORA-12514`, `ORA-12505`, a refused or timed-out connection.
    #[error("Oracle connection failed: the server or the service could not be reached")]
    Unreachable,
    /// `ORA-12660`: the server does not accept the encryption the connector
    /// requires.
    #[error(
        "Oracle connection failed: the server does not accept the required network encryption \
         (AES256 with a SHA256 checksum). Allow it in the server's sqlnet.ora, or set the \
         connector's TLS mode to disable."
    )]
    EncryptionRefused,
    /// Anything else.
    #[error("Oracle connection test failed")]
    Failed,
}

/// Match `Kafka Connect`'s error texts against Oracle's documented error codes
/// and the driver's own connection phrases. Encryption first: `ORA-12660` can
/// arrive wrapped in a generic connection failure.
#[must_use]
pub fn classify(validation: &ConfigValidation) -> OracleTestFailure {
    let texts = validation.messages_for_classification();
    let has = |needles: &[&str]| {
        texts.iter().any(|text| {
            let lower = text.to_ascii_lowercase();
            needles.iter().any(|needle| lower.contains(needle))
        })
    };
    if has(&["ora-12660", "ora-12650", "ora-12649"]) {
        OracleTestFailure::EncryptionRefused
    } else if has(&["ora-01017"]) {
        OracleTestFailure::WrongLogin
    } else if has(&[
        "ora-12541",
        "ora-12514",
        "ora-12505",
        "ora-12170",
        "connection refused",
        "timed out",
        "unknownhostexception",
        "network adapter could not establish",
    ]) {
        OracleTestFailure::Unreachable
    } else {
        OracleTestFailure::Failed
    }
}

/// Whether `info` is an Oracle change-capture connector.
#[must_use]
pub fn is_oracle_cdc(info: &ConnectorDialInfo) -> bool {
    info.adapter.as_deref() == Some("cdc")
        && matches!(
            Dial::parse("cdc", &info.dial),
            Ok(Dial::Cdc(dial)) if dial.driver == SqlDriver::Oracle
        )
}

fn outcome(ok: bool, supported: bool, elapsed_ms: Option<i64>, message: &str) -> Outcome {
    Outcome {
        ok,
        supported,
        latency_ms: elapsed_ms,
        message: message.to_owned(),
        rotated_refresh_token: None,
    }
}

/// Test Oracle change-capture connector `id` through `Kafka Connect`, for the
/// `tables` its spec selects (none: a placeholder); `None` when `info` is not
/// one (the caller then probes as usual). Never an error: a
/// failure to set up is a failed or unsupported [`Outcome`] with a fixed
/// sentence.
pub async fn test(
    state: &AppState,
    info: &ConnectorDialInfo,
    id: &str,
    tables: &[String],
) -> Option<Outcome> {
    if !is_oracle_cdc(info) {
        return None;
    }
    let (Some(client), Some(bootstrap)) = (
        KafkaConnectClient::from_config(&state.config),
        state.config.cdc_kafka_bootstrap.as_deref(),
    ) else {
        return Some(outcome(false, false, None, NOT_CONFIGURED));
    };
    let Ok(Dial::Cdc(dial)) = Dial::parse("cdc", &info.dial) else {
        return None;
    };
    // The SSRF check first, before the credential is touched.
    if let Err(message) = resolve_checked(
        &dial.host,
        dial.port,
        &state.config.connector_internal_hosts(),
    )
    .await
    {
        return Some(outcome(false, true, None, &message));
    }
    let table_refs: Vec<&str> = if tables.is_empty() {
        vec![PLACEHOLDER_TABLE]
    } else {
        tables.iter().map(String::as_str).collect()
    };
    let Ok(config) = render_connect_config(
        id,
        &dial,
        &table_refs,
        &state.config.cdc_secrets_dir,
        Some(bootstrap),
    ) else {
        return Some(outcome(
            false,
            true,
            None,
            "The connector's settings cannot be turned into a change-capture configuration.",
        ));
    };
    if crate::cdc_secrets::CdcSecretStore::new(&state.config.cdc_secrets_dir)
        .materialise(
            state.connector_secret_resolver.as_ref(),
            id,
            &info.secret_ref,
        )
        .await
        .is_err()
    {
        return Some(outcome(
            false,
            true,
            None,
            "could not resolve the connector's credential",
        ));
    }
    let started = Instant::now();
    let validated = match connect_connector_name(id) {
        Ok(name) => client.validate_config(&name, &config).await,
        Err(_) => Err(ConnectError::Rejected),
    };
    let elapsed_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    Some(match validated {
        Ok(validation) if validation.is_valid() => outcome(
            true,
            true,
            Some(elapsed_ms),
            "Connected through Kafka Connect with the connector's own driver and encryption.",
        ),
        Ok(validation) => {
            let failure = classify(&validation);
            tracing::info!(
                connector_id = %id,
                ?failure,
                fields = ?validation.failed_fields(),
                "the Oracle change-capture test failed"
            );
            outcome(false, true, Some(elapsed_ms), &failure.to_string())
        }
        Err(ConnectError::ClassNotAllowed | ConnectError::Rejected) => outcome(
            false,
            true,
            None,
            "Kafka Connect does not accept this connector's configuration.",
        ),
        Err(ConnectError::Unavailable | ConnectError::NotFound) => outcome(
            false,
            true,
            None,
            "Kafka Connect is not reachable right now.",
        ),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn failed_with(text: &str) -> ConfigValidation {
        ConfigValidation::from_answer(&json!({
            "error_count": 1,
            "configs": [{"value": {"errors": [text]}}]
        }))
        .unwrap()
    }

    #[test]
    fn oracle_error_codes_become_fixed_classes() {
        for (text, want) in [
            (
                "Unable to connect: ORA-01017: invalid username/password; logon denied",
                OracleTestFailure::WrongLogin,
            ),
            (
                "Unable to connect: ORA-12514: Listener does not currently know of service",
                OracleTestFailure::Unreachable,
            ),
            (
                "Unable to connect: ORA-12541: Cannot connect. No listener",
                OracleTestFailure::Unreachable,
            ),
            (
                "Unable to connect: IO Error: Connection refused",
                OracleTestFailure::Unreachable,
            ),
            (
                "Unable to connect: ORA-12660: Encryption or crypto-checksumming parameters incompatible",
                OracleTestFailure::EncryptionRefused,
            ),
            // Encryption wins over a generic wrapper that also says "refused".
            (
                "IO Error: Connection refused (ORA-12660)",
                OracleTestFailure::EncryptionRefused,
            ),
            ("something unexpected", OracleTestFailure::Failed),
        ] {
            assert_eq!(classify(&failed_with(text)), want, "{text}");
        }
        let nothing = ConfigValidation::from_answer(&json!({"error_count": 1})).unwrap();
        assert_eq!(classify(&nothing), OracleTestFailure::Failed);
    }

    #[test]
    fn a_fixed_sentence_never_carries_the_upstream_text() {
        let text = "ORA-01017: invalid username/password for user SECRET_USER at db.internal";
        let sentence = classify(&failed_with(text)).to_string();
        assert!(!sentence.contains("SECRET_USER") && !sentence.contains("db.internal"));
    }

    #[test]
    fn only_an_oracle_cdc_connector_is_tested_here() {
        let mut row = ConnectorDialInfo {
            kind: "Oracle CDC".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            secret_ref_secondary: None,
            adapter: Some("cdc".to_owned()),
            dial: json!({"driver": "oracle", "host": "db.invalid", "port": 1521,
                         "database": "FREE", "pdbName": "FREEPDB1", "user": "u"}),
        };
        assert!(is_oracle_cdc(&row));
        row.dial = json!({"driver": "mssql", "host": "db.invalid", "port": 1433,
                          "database": "d", "user": "u"});
        assert!(!is_oracle_cdc(&row));
        row.adapter = Some("sql".to_owned());
        row.dial = json!({"driver": "oracle", "host": "db.invalid", "port": 1521,
                          "database": "FREE", "user": "u"});
        assert!(!is_oracle_cdc(&row));
    }
}
