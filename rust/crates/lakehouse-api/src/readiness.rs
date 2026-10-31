//! `GET /health/ready` — the readiness probe a deployment waits on
//! (REL-2 decision D7).
//!
//! # Why this is not `GET /health`
//!
//! `GET /health` is a liveness probe: it answers `200` whenever the process
//! can answer at all, because a deployment with no usable `Postgres` still
//! serves every Phase 1 route, and the ops gates (`ops/g*/`) poll it for a
//! `200` to decide the API is up. It says nothing about whether the stack
//! is ready to serve traffic.
//!
//! This route answers the other question, and it is the one an installer or
//! an updater waits on: `200` when the dependencies this service needs are
//! there and usable, `503` when they are not. `lakehousectl update` (REL-2
//! slice S4) waits for this route to report the NEW version before it
//! calls an update successful.
//!
//! # The three checks, and what each one proves
//!
//! - `postgres`: the database answers AND its schema is at this build's
//!   version — the highest applied migration equals the highest migration
//!   embedded in this binary. A database that is one migration behind is
//!   not ready for this build, and one that is ahead of it is not either
//!   (an old binary must not serve a newer schema).
//! - `clickhouse`: `ClickHouse` answers `SELECT 1`.
//! - `lakekeeper`: the Iceberg REST catalog answers its own config request
//!   (`GET {catalog}/v1/config`), the same call the catalog client makes
//!   when it connects.
//!
//! # The body carries no upstream text
//!
//! Every check is `"ok"` or `"failed"` and nothing else (AGENTS.md rule 4:
//! upstream error text never reaches a response). The reason a check failed
//! is in the log, under the same line the probe wrote, not in the response:
//! a readiness route is public, so its body is read by anything that can
//! reach the port.

use serde::Serialize;

use crate::lakekeeper_token::read_token_file;
use crate::state::AppState;

/// The version this build reports on `/health` and `/health/ready`.
///
/// `CARGO_PKG_VERSION` of this crate — the same string the release
/// pipeline will publish (REL-2 slice S2), so a console can compare what
/// it was built against with what the server runs.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long one readiness check may take before it counts as failed.
///
/// The same bound [`crate::health`]'s service probes use: a readiness
/// route is polled (by `docker compose`'s health check, by an updater
/// waiting for a new version), so a dead dependency must never make the
/// answer wait on a socket timeout.
const PROBE_TIMEOUT: std::time::Duration = crate::health::PROBE_TIMEOUT;

/// The purpose and env var [`read_token_file`] names when the reader token
/// cannot be read — the same pair `routes::lakehouse` uses for the same
/// file, so the operator sees one name for one thing (AGENTS.md rule 4).
const READER_PURPOSE: &str = "lakehouse-api-reader";
const READER_TOKEN_ENV_VAR: &str = "LAKEKEEPER_READ_TOKEN_FILE";

/// One check's outcome.
///
/// Only two values, and neither carries a reason: `"ok"` or `"failed"`.
/// The reason is in the log (AGENTS.md rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Check {
    /// The check ran and the dependency answered.
    Ok,
    /// The check failed, timed out, or could not run at all.
    Failed,
}

impl Check {
    /// Whether this check passed.
    #[must_use]
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// The three dependencies this service needs to be ready.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checks {
    /// The application database, at this build's schema version.
    pub postgres: Check,
    /// The analytical store.
    pub clickhouse: Check,
    /// The Iceberg REST catalog.
    pub lakekeeper: Check,
}

impl Checks {
    /// Whether every check passed — the whole answer of this route.
    #[must_use]
    pub fn all_ok(self) -> bool {
        self.postgres.is_ok() && self.clickhouse.is_ok() && self.lakekeeper.is_ok()
    }
}

/// The body of `GET /health/ready`.
///
/// The same shape for `200` and `503`: a caller that gets `503` still
/// learns which dependency is not ready, without any upstream text.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyBody {
    /// `true` when every check passed.
    pub ready: bool,
    /// This build's version (see [`VERSION`]).
    pub version: &'static str,
    /// The three checks, one per dependency.
    pub checks: Checks,
}

impl ReadyBody {
    /// The body for a finished set of checks.
    #[must_use]
    pub fn new(checks: Checks) -> Self {
        Self {
            ready: checks.all_ok(),
            version: VERSION,
            checks,
        }
    }
}

/// Probe the three dependencies, concurrently.
///
/// Each probe is bounded by [`PROBE_TIMEOUT`], so the whole answer takes
/// about as long as the slowest dependency, not the sum.
pub async fn probe(state: &AppState) -> Checks {
    let http = reqwest::Client::new();
    let (postgres, clickhouse, lakekeeper) = tokio::join!(
        probe_postgres(state),
        probe_clickhouse(state),
        probe_lakekeeper(&http, state),
    );
    Checks {
        postgres,
        clickhouse,
        lakekeeper,
    }
}

/// Is the database's schema version this build's schema version?
///
/// Exactly equal, on purpose: a database one migration BEHIND this build
/// cannot serve this build's routes, and a database one migration AHEAD of
/// it was written by a newer build than this one — an old binary must not
/// claim to be ready for it either.
#[must_use]
pub fn schema_is_current(applied: i64, embedded: i64) -> bool {
    applied == embedded
}

/// Is the database reachable AND at this build's schema version?
///
/// The two are one check because they answer one question: "can this build
/// work against this database?". A database that is behind is not ready
/// for this build, and neither is one that is ahead of it.
///
/// The version comparison is the honest one: this build's expectation comes
/// from the migration set embedded in the binary
/// ([`lakehouse_store::embedded_schema_version`]), and the database's
/// state comes from `sqlx`'s own bookkeeping table
/// ([`lakehouse_store::applied_schema_version`]). Neither side is guessed.
async fn probe_postgres(state: &AppState) -> Check {
    let Some(pool) = state.pg.as_deref() else {
        tracing::warn!("the readiness probe found no Postgres pool: DATABASE_URL did not parse");
        return Check::Failed;
    };
    let embedded = lakehouse_store::embedded_schema_version();
    match tokio::time::timeout(PROBE_TIMEOUT, lakehouse_store::applied_schema_version(pool)).await {
        Ok(Ok(applied)) if schema_is_current(applied, embedded) => Check::Ok,
        Ok(Ok(applied)) => {
            tracing::warn!(
                applied,
                embedded,
                "the readiness probe found the database at another schema version than this build's"
            );
            Check::Failed
        }
        Ok(Err(err)) => {
            tracing::warn!(
                error = ?err,
                "the readiness probe could not read the applied schema version"
            );
            Check::Failed
        }
        Err(_) => {
            tracing::warn!("the readiness probe timed out reading the applied schema version");
            Check::Failed
        }
    }
}

/// Does `ClickHouse` answer `SELECT 1`?
async fn probe_clickhouse(state: &AppState) -> Check {
    match tokio::time::timeout(PROBE_TIMEOUT, state.clickhouse.rows("SELECT 1", None)).await {
        Ok(Ok(_)) => Check::Ok,
        Ok(Err(err)) => {
            tracing::warn!(error = ?err, "the readiness probe could not query ClickHouse");
            Check::Failed
        }
        Err(_) => {
            tracing::warn!("the readiness probe timed out querying ClickHouse");
            Check::Failed
        }
    }
}

/// Does the Iceberg REST catalog answer its config request?
///
/// `GET {catalog_uri}/v1/config` is the call the catalog client makes when
/// it connects (`lakehouse_iceberg::IcebergClient::connect`), so this
/// proves the catalog is reachable AND usable by this service. The reader
/// bearer token is the one the catalog routes use; without it the catalog
/// would refuse, so a missing token file means "not ready" — honestly,
/// because the service cannot read the catalog without it.
async fn probe_lakekeeper(http: &reqwest::Client, state: &AppState) -> Check {
    let token = match read_token_file(
        &state.config.lakekeeper_read_token_file,
        READER_PURPOSE,
        READER_TOKEN_ENV_VAR,
    )
    .await
    {
        Ok(token) => token,
        Err(err) => {
            tracing::warn!(error = ?err, "the readiness probe could not read the catalog token");
            return Check::Failed;
        }
    };
    let url = format!(
        "{}/v1/config",
        state.config.lakekeeper_catalog_uri.trim_end_matches('/')
    );
    match tokio::time::timeout(
        PROBE_TIMEOUT,
        http.get(url).bearer_auth(token.expose_secret()).send(),
    )
    .await
    {
        Ok(Ok(resp)) if resp.status().is_success() => Check::Ok,
        Ok(Ok(resp)) => {
            tracing::warn!(
                status = resp.status().as_u16(),
                "the readiness probe got a non-success answer from the catalog"
            );
            Check::Failed
        }
        Ok(Err(err)) => {
            tracing::warn!(error = ?err, "the readiness probe could not reach the catalog");
            Check::Failed
        }
        Err(_) => {
            tracing::warn!("the readiness probe timed out reaching the catalog");
            Check::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The whole point of the route: three checks that passed mean the
    /// stack is ready, and the body says which version is ready.
    #[test]
    fn health_is_ready_when_every_check_passes() {
        let checks = Checks {
            postgres: Check::Ok,
            clickhouse: Check::Ok,
            lakekeeper: Check::Ok,
        };
        let body = ReadyBody::new(checks);
        assert!(body.ready);
        assert_eq!(body.version, VERSION);
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "ready": true,
                "version": VERSION,
                "checks": {
                    "postgres": "ok",
                    "clickhouse": "ok",
                    "lakekeeper": "ok",
                },
            })
        );
    }

    /// One failed check is enough: the route answers `503` and says which
    /// dependency is not ready.
    #[test]
    fn health_is_not_ready_when_one_check_fails() {
        let body = ReadyBody::new(Checks {
            postgres: Check::Ok,
            clickhouse: Check::Failed,
            lakekeeper: Check::Ok,
        });
        assert!(!body.ready);
        assert_eq!(body.checks.clickhouse, Check::Failed);
        assert_eq!(body.checks.postgres, Check::Ok);
    }

    /// The body carries no upstream text (AGENTS.md rule 4): the only
    /// strings in it are the version and the two check words.
    #[test]
    fn health_carries_no_upstream_text() {
        let body = ReadyBody::new(Checks {
            postgres: Check::Failed,
            clickhouse: Check::Failed,
            lakekeeper: Check::Failed,
        });
        let text = serde_json::to_string(&body).unwrap();
        assert!(text.contains("\"failed\""));
        assert!(!text.contains("error"));
        assert!(!text.contains("password"));
        assert!(!text.contains("psql"));
        // The whole body is the three fields D7 names — no more.
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 3);
        assert!(object.contains_key("ready"));
        assert!(object.contains_key("version"));
        assert!(object.contains_key("checks"));
    }

    /// The check words are exactly `"ok"` and `"failed"` — nothing else
    /// can ever reach the body.
    #[test]
    fn health_check_words_are_ok_and_failed() {
        assert_eq!(serde_json::to_value(Check::Ok).unwrap(), "ok");
        assert_eq!(serde_json::to_value(Check::Failed).unwrap(), "failed");
    }

    /// A database that is not at this build's schema version is not ready:
    /// one migration behind (the case that matters — the build needs a
    /// table this database does not have) and one migration ahead (a newer
    /// build wrote it; this binary must not claim to be ready for it).
    #[test]
    fn health_is_not_ready_when_the_database_is_behind() {
        let embedded = lakehouse_store::embedded_schema_version();
        assert!(embedded > 0, "this build embeds at least one migration");
        assert!(schema_is_current(embedded, embedded));
        assert!(
            !schema_is_current(embedded - 1, embedded),
            "a database one migration behind is not at this build's schema version"
        );
        assert!(
            !schema_is_current(embedded + 1, embedded),
            "a database ahead of this build is not at this build's schema version"
        );
    }

    /// A state whose dependencies are all unreachable: a lazy pool at a
    /// throwaway address this test never dials (the same shape
    /// `routes`'s own route test builds), and no `ClickHouse` and no
    /// catalog token.
    fn test_state() -> AppState {
        let env = std::collections::HashMap::from([(
            "DATABASE_URL".to_owned(),
            "postgres://test:test@127.0.0.1:5432/test".to_owned(),
        )]);
        let config = crate::config::Config::from_map(&env).unwrap();
        AppState::new(config)
    }

    /// The route itself: it answers with the D7 body shape, and the status
    /// follows the body — `503` when a check is not `"ok"`, `200` when
    /// every one is. Which dependency fails depends on what else runs on
    /// this machine, so the assertion is on the shape and on the
    /// agreement between the status and the body, not on which check
    /// failed.
    #[tokio::test]
    async fn health_ready_answers_with_the_d7_shape_and_the_matching_status() {
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let response = crate::routes::router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        // The body is exactly the three fields D7 names.
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 3, "unexpected body: {value}");
        assert_eq!(value["version"], serde_json::json!(VERSION));

        // Every check is one of the two words, and `ready` is exactly
        // "every check passed" — the status follows it.
        let checks = value["checks"].as_object().unwrap();
        assert_eq!(checks.len(), 3, "unexpected checks: {value}");
        let mut all_ok = true;
        for (name, check) in checks {
            let word = check.as_str().unwrap();
            assert!(
                word == "ok" || word == "failed",
                "{name} carries {word}, not a check word"
            );
            all_ok &= word == "ok";
        }
        assert_eq!(value["ready"], serde_json::json!(all_ok));
        assert_eq!(
            status,
            if all_ok {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            }
        );
    }
}
