//! The API's client for `Kafka Connect`'s REST API (`SRC-4` slice 2, decision
//! S2-b, ADR 0016).
//!
//! The API is `Kafka Connect`'s only client: it creates, updates, pauses,
//! resumes and deletes the one connector each change-capture registry
//! connector maps to, and reads its state. Nothing else talks to the worker.
//!
//! # Fail closed
//!
//! `KAFKA_CONNECT_URL` unset (or not an `http(s)` address) means
//! [`KafkaConnectClient::from_config`] returns `None`, and the caller answers
//! `supported: false` with [`NOT_CONFIGURED`]. The API never dials a guessed
//! address.
//!
//! # Allowlist
//!
//! Only a connector class in [`ALLOWED_CONNECTOR_CLASSES`] is ever sent: the
//! class is data the registry row decides, and `Kafka Connect` will load any
//! plugin class on its path. `PostgreSQL`, `MySQL`, `SQL Server`, `MongoDB` and `Oracle` now; the other
//! `Debezium` classes are added by the slice that builds each source.
//!
//! # Upstream text never reaches a response
//!
//! A `Kafka Connect` answer can echo the connector configuration, host names
//! and driver messages. [`ConnectError`] is a fixed class with a fixed
//! sentence; the raw status goes to the log as a class only, never the body
//! (`AGENTS.md` principle 4).
//!
//! # Why this dials without `resolve_checked`
//!
//! The usual rule for a dialer is the allowlisted resolver and
//! `resolve_checked`, because a person controls the address. Here no person
//! does: the address is the operator's `KAFKA_CONNECT_URL`, an internal
//! compose service the guard exists to refuse. The same posture as the
//! `Lakekeeper` and `ClickHouse` clients, which are also dialed from
//! operator configuration.

use std::collections::BTreeMap;
use std::time::Duration;

use lakehouse_core::ApiError;
use lakehouse_store::cdc::{
    MONGODB_CONNECTOR_CLASS, MYSQL_CONNECTOR_CLASS, ORACLE_CONNECTOR_CLASS,
    POSTGRES_CONNECTOR_CLASS, SQLSERVER_CONNECTOR_CLASS,
};
use serde_json::Value;

use crate::config::Config;

/// The `connector.class` values this build may start. See the module docs.
pub const ALLOWED_CONNECTOR_CLASSES: [&str; 5] = [
    POSTGRES_CONNECTOR_CLASS,
    MYSQL_CONNECTOR_CLASS,
    SQLSERVER_CONNECTOR_CLASS,
    MONGODB_CONNECTOR_CLASS,
    ORACLE_CONNECTOR_CLASS,
];

/// What a change-capture connector answers while `KAFKA_CONNECT_URL` is unset.
pub const NOT_CONFIGURED: &str = "Change capture is not available on this deployment: it needs \
    Kafka Connect, which is not configured (the cdc profile and KAFKA_CONNECT_URL).";

/// A request is a few small JSON documents; ten seconds is generous for a
/// healthy worker and short enough that a stuck one fails the request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest answer read. `Kafka Connect`'s answers for one connector are a
/// few hundred bytes; a larger one is not read.
const MAX_ANSWER_BYTES: usize = 256 * 1024;

/// Why a `Kafka Connect` call did not succeed, as a class with a fixed
/// sentence. Never carries upstream text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    /// The connector class is not one this build starts.
    #[error("this connector's source type is not allowed to start change capture")]
    ClassNotAllowed,
    /// The worker could not be reached, timed out, or is rebalancing.
    #[error("Kafka Connect is not reachable right now")]
    Unavailable,
    /// The worker refused the connector configuration.
    #[error("Kafka Connect refused the connector configuration")]
    Rejected,
    /// There is no such connector in the worker.
    #[error("the connector is not in Kafka Connect")]
    NotFound,
}

impl From<ConnectError> for ApiError {
    fn from(err: ConnectError) -> Self {
        match err {
            ConnectError::ClassNotAllowed | ConnectError::Rejected => {
                ApiError::Unprocessable(err.to_string())
            }
            ConnectError::Unavailable => ApiError::Unavailable(err.to_string()),
            ConnectError::NotFound => ApiError::NotFound(err.to_string()),
        }
    }
}

/// A connector's state in `Kafka Connect`, in the four fixed words the console
/// shows (`SRC-4` decision S2-h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectState {
    /// The connector and every task run.
    Running,
    /// Paused: the connector and its tasks are stopped, the position kept.
    Paused,
    /// The connector or a task failed.
    Failed,
    /// Neither: created and not yet assigned, restarting, or a state this
    /// build does not know. Not reported as running (principle 2).
    Starting,
}

impl ConnectState {
    /// The word the console shows.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Failed => "failed",
            Self::Starting => "starting",
        }
    }
}

/// Read the state out of `GET /connectors/<name>/status`. Any failed part is
/// `failed`; running needs the connector and every task running, with at least
/// one task (a connector with no task assigned yet has captured nothing).
#[must_use]
pub fn classify_status(status: &Value) -> ConnectState {
    let connector = status
        .pointer("/connector/state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tasks: Vec<&str> = status
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|task| {
            task.get("state")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
        .collect();
    if connector == "FAILED" || tasks.contains(&"FAILED") {
        ConnectState::Failed
    } else if connector == "PAUSED" {
        ConnectState::Paused
    } else if connector == "RUNNING" && !tasks.is_empty() && tasks.iter().all(|t| *t == "RUNNING") {
        ConnectState::Running
    } else {
        ConnectState::Starting
    }
}

/// What `Kafka Connect` answered to a configuration validation (`SRC-4`
/// slice 6, decision S10-a): how many settings it found wrong and the texts it
/// gave. The texts can name hosts, users and driver messages, so they exist only
/// to be classified by the caller (`crate::cdc_oracle`): this type has no
/// `Debug` and no accessor that hands them out for display.
pub struct ConfigValidation {
    error_count: u64,
    messages: Vec<String>,
}

impl ConfigValidation {
    /// Read `PUT /connector-plugins/<class>/config/validate`'s answer: its
    /// `error_count` and every `configs[].value.errors[]` text. An answer
    /// without an `error_count` is not a validation: `None`.
    #[must_use]
    pub fn from_answer(answer: &Value) -> Option<Self> {
        let error_count = answer.get("error_count").and_then(Value::as_u64)?;
        let messages = answer
            .get("configs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|config| config.pointer("/value/errors").and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        Some(Self {
            error_count,
            messages,
        })
    }

    /// Whether `Kafka Connect` found nothing wrong.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.error_count == 0
    }

    /// The error texts, for matching against fixed patterns only.
    #[must_use]
    pub fn messages_for_classification(&self) -> &[String] {
        &self.messages
    }
}

/// The client. Cheap to build: one per use, from [`Config`].
pub struct KafkaConnectClient {
    base: String,
    http: reqwest::Client,
}

impl KafkaConnectClient {
    /// The client for `config`, or `None` when `KAFKA_CONNECT_URL` is unset or
    /// is not an `http://` or `https://` address (fail closed).
    #[must_use]
    pub fn from_config(config: &Config) -> Option<Self> {
        Self::new(config.kafka_connect_url.as_deref()?)
    }

    fn new(url: &str) -> Option<Self> {
        let base = url.trim().trim_end_matches('/');
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            tracing::warn!("KAFKA_CONNECT_URL is not an http(s) address; change capture is off");
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // A redirect would send the connector configuration to wherever the
            // worker is told to point; the worker never redirects.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Self {
            base: base.to_owned(),
            http,
        })
    }

    /// Create or update connector `name` with `config` (`PUT
    /// /connectors/<name>/config`, which does both). `name` is the output of
    /// `lakehouse_store::cdc::connect_connector_name`.
    ///
    /// # Errors
    ///
    /// [`ConnectError::ClassNotAllowed`] before anything is sent when
    /// `config`'s `connector.class` is not in [`ALLOWED_CONNECTOR_CLASSES`];
    /// otherwise the class of the failure.
    pub async fn put_config(
        &self,
        name: &str,
        config: &BTreeMap<String, String>,
    ) -> Result<(), ConnectError> {
        let class = config.get("connector.class").map(String::as_str);
        if !class.is_some_and(|c| ALLOWED_CONNECTOR_CLASSES.contains(&c)) {
            return Err(ConnectError::ClassNotAllowed);
        }
        let request = self
            .http
            .put(format!("{}/connectors/{name}/config", self.base))
            .json(config);
        self.send(request).await.map(|_| ())
    }

    /// Ask the connector plugin to validate `config` without creating anything
    /// (`PUT /connector-plugins/<class>/config/validate`, `SRC-4` slice 6,
    /// decision S10-a). The plugin opens the connection the connector would,
    /// with the same driver and settings, and reports what failed. `config`
    /// holds the password as a `${dir:...}` reference, never a value: the worker
    /// resolves config providers before it validates. The same class allowlist
    /// as [`Self::put_config`].
    ///
    /// # Errors
    ///
    /// [`ConnectError::ClassNotAllowed`] before anything is sent;
    /// [`ConnectError::Unavailable`] for an unreachable worker or an answer
    /// that is not a validation; otherwise the class of the failure.
    pub async fn validate_config(
        &self,
        config: &BTreeMap<String, String>,
    ) -> Result<ConfigValidation, ConnectError> {
        let class = config.get("connector.class").map(String::as_str);
        let Some(class) = class.filter(|c| ALLOWED_CONNECTOR_CLASSES.contains(c)) else {
            return Err(ConnectError::ClassNotAllowed);
        };
        let request = self
            .http
            .put(format!(
                "{}/connector-plugins/{class}/config/validate",
                self.base
            ))
            .json(config);
        let body = self.send(request).await?;
        let answer: Value = serde_json::from_slice(&body).map_err(|_| ConnectError::Unavailable)?;
        ConfigValidation::from_answer(&answer).ok_or(ConnectError::Unavailable)
    }

    /// The failure text of connector `name`: the connector's own trace and its
    /// tasks', from `GET /connectors/<name>/status`. `None` when nothing failed.
    /// For matching against fixed patterns only (`crate::cdc_oracle_failure`);
    /// the text is never shown (principle 4).
    ///
    /// # Errors
    ///
    /// The class of the failure; [`ConnectError::NotFound`] if it is not there.
    pub async fn failure_trace(&self, name: &str) -> Result<Option<String>, ConnectError> {
        let request = self
            .http
            .get(format!("{}/connectors/{name}/status", self.base));
        let body = self.send(request).await?;
        let status: Value = serde_json::from_slice(&body).map_err(|_| ConnectError::Unavailable)?;
        Ok(trace_of(&status))
    }

    /// Pause connector `name` (`PUT /connectors/<name>/pause`).
    ///
    /// # Errors
    ///
    /// The class of the failure; [`ConnectError::NotFound`] if it is not there.
    pub async fn pause(&self, name: &str) -> Result<(), ConnectError> {
        let request = self
            .http
            .put(format!("{}/connectors/{name}/pause", self.base));
        self.send(request).await.map(|_| ())
    }

    /// Resume connector `name` (`PUT /connectors/<name>/resume`).
    ///
    /// # Errors
    ///
    /// The class of the failure; [`ConnectError::NotFound`] if it is not there.
    pub async fn resume(&self, name: &str) -> Result<(), ConnectError> {
        let request = self
            .http
            .put(format!("{}/connectors/{name}/resume", self.base));
        self.send(request).await.map(|_| ())
    }

    /// Restart connector `name` and its tasks (`POST
    /// /connectors/<name>/restart`), so the tasks read a changed password file
    /// (the worker reads it when a task starts).
    ///
    /// # Errors
    ///
    /// The class of the failure; [`ConnectError::NotFound`] if it is not there.
    pub async fn restart(&self, name: &str) -> Result<(), ConnectError> {
        let request = self.http.post(format!(
            "{}/connectors/{name}/restart?includeTasks=true&onlyFailed=false",
            self.base
        ));
        self.send(request).await.map(|_| ())
    }

    /// Delete connector `name`. A connector that is not there is success: the
    /// call is made again after a half-finished delete.
    ///
    /// # Errors
    ///
    /// The class of the failure, except not-found.
    pub async fn delete(&self, name: &str) -> Result<(), ConnectError> {
        let request = self.http.delete(format!("{}/connectors/{name}", self.base));
        match self.send(request).await {
            Ok(_) | Err(ConnectError::NotFound) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// The connector's state (`GET /connectors/<name>/status`).
    ///
    /// # Errors
    ///
    /// The class of the failure; [`ConnectError::NotFound`] if it is not there.
    pub async fn state(&self, name: &str) -> Result<ConnectState, ConnectError> {
        let request = self
            .http
            .get(format!("{}/connectors/{name}/status", self.base));
        let body = self.send(request).await?;
        let status: Value = serde_json::from_slice(&body).map_err(|_| ConnectError::Unavailable)?;
        Ok(classify_status(&status))
    }

    /// Send `request`, map the outcome to a [`ConnectError`] class, and return
    /// the body of a success.
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Vec<u8>, ConnectError> {
        let mut response = request.send().await.map_err(|err| {
            // The class only: a `reqwest` error's text can carry the address.
            tracing::warn!(
                connect = err.is_connect(),
                timeout = err.is_timeout(),
                "Kafka Connect request failed"
            );
            ConnectError::Unavailable
        })?;
        let status = response.status();
        if !status.is_success() {
            tracing::warn!(status = status.as_u16(), "Kafka Connect refused a request");
            return Err(classify_http_status(status.as_u16()));
        }
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() + chunk.len() > MAX_ANSWER_BYTES {
                        return Err(ConnectError::Unavailable);
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => return Ok(body),
                Err(_) => return Err(ConnectError::Unavailable),
            }
        }
    }
}

/// The traces of the parts of `status` that failed, joined; `None` if none did.
fn trace_of(status: &Value) -> Option<String> {
    let connector = status.pointer("/connector");
    let tasks = status
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let traces: Vec<&str> = connector
        .into_iter()
        .chain(tasks)
        .filter(|part| part.get("state").and_then(Value::as_str) == Some("FAILED"))
        .filter_map(|part| part.get("trace").and_then(Value::as_str))
        .collect();
    (!traces.is_empty()).then(|| traces.join("\n"))
}

/// The class of a non-success HTTP status. 404 is a missing connector, 400 and
/// 422 are a configuration the worker refuses; everything else, including 409
/// (a rebalance in progress) and 5xx, is the worker not being ready, which a
/// retry fixes.
fn classify_http_status(status: u16) -> ConnectError {
    match status {
        404 => ConnectError::NotFound,
        400 | 422 => ConnectError::Rejected,
        _ => ConnectError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn config(class: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("connector.class".to_owned(), class.to_owned()),
            ("tasks.max".to_owned(), "1".to_owned()),
        ])
    }

    #[test]
    fn an_unset_or_non_http_address_gives_no_client() {
        let unset = Config::from_map(&std::collections::HashMap::new()).unwrap();
        assert!(KafkaConnectClient::from_config(&unset).is_none());
        assert!(KafkaConnectClient::new("kafka-connect:8083").is_none());
        assert!(KafkaConnectClient::new("ftp://kafka-connect").is_none());
        assert!(KafkaConnectClient::new("http://kafka-connect:8083/").is_some());
    }

    #[test]
    fn the_state_words_follow_the_connector_and_every_task() {
        let state = |connector: &str, tasks: &[&str]| {
            classify_status(&json!({
                "connector": {"state": connector},
                "tasks": tasks.iter().map(|t| json!({"state": t})).collect::<Vec<_>>(),
            }))
        };
        assert_eq!(state("RUNNING", &["RUNNING"]), ConnectState::Running);
        assert_eq!(state("PAUSED", &["PAUSED"]), ConnectState::Paused);
        assert_eq!(state("RUNNING", &["FAILED"]), ConnectState::Failed);
        assert_eq!(state("FAILED", &[]), ConnectState::Failed);
        // No task assigned yet, or one still starting: not "running".
        assert_eq!(state("RUNNING", &[]), ConnectState::Starting);
        assert_eq!(state("RUNNING", &["UNASSIGNED"]), ConnectState::Starting);
        assert_eq!(classify_status(&json!({})), ConnectState::Starting);
        assert_eq!(ConnectState::Running.word(), "running");
    }

    #[test]
    fn http_statuses_map_to_classes() {
        assert_eq!(classify_http_status(404), ConnectError::NotFound);
        assert_eq!(classify_http_status(400), ConnectError::Rejected);
        assert_eq!(classify_http_status(409), ConnectError::Unavailable);
        assert_eq!(classify_http_status(500), ConnectError::Unavailable);
    }

    #[tokio::test]
    async fn a_class_outside_the_allowlist_is_never_sent() {
        let server = MockServer::start().await;
        let client = KafkaConnectClient::new(&server.uri()).unwrap();
        let err = client
            .put_config("lakehouse-a", &config("org.example.EvilConnector"))
            .await
            .unwrap_err();
        assert_eq!(err, ConnectError::ClassNotAllowed);
        assert!(server.received_requests().await.unwrap().is_empty());
        let err = client
            .put_config("lakehouse-a", &BTreeMap::new())
            .await
            .unwrap_err();
        assert_eq!(err, ConnectError::ClassNotAllowed);
    }

    #[tokio::test]
    async fn validate_asks_the_plugin_and_reads_its_error_count() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(format!(
                "/connector-plugins/{ORACLE_CONNECTOR_CLASS}/config/validate"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "x", "error_count": 1,
                "configs": [
                    {"definition": {"name": "a"}, "value": {"name": "a", "errors": []}},
                    {"definition": {"name": "database.hostname"},
                     "value": {"name": "database.hostname", "errors": ["ORA-01017: invalid"]}}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = KafkaConnectClient::new(&server.uri()).unwrap();
        let validation = client
            .validate_config(&config(ORACLE_CONNECTOR_CLASS))
            .await
            .unwrap();
        assert!(!validation.is_valid());
        assert_eq!(
            validation.messages_for_classification(),
            ["ORA-01017: invalid"]
        );
    }

    #[tokio::test]
    async fn validate_never_sends_a_class_outside_the_allowlist_and_needs_an_error_count() {
        let server = MockServer::start().await;
        let client = KafkaConnectClient::new(&server.uri()).unwrap();
        assert!(matches!(
            client
                .validate_config(&config("org.example.EvilConnector"))
                .await,
            Err(ConnectError::ClassNotAllowed)
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"unexpected": 1})))
            .mount(&server)
            .await;
        assert!(matches!(
            client
                .validate_config(&config(ORACLE_CONNECTOR_CLASS))
                .await,
            Err(ConnectError::Unavailable)
        ));
    }

    #[test]
    fn the_failure_trace_joins_the_failed_parts_only() {
        let status = json!({
            "connector": {"state": "RUNNING"},
            "tasks": [
                {"id": 0, "state": "FAILED", "trace": "boom one"},
                {"id": 1, "state": "RUNNING", "trace": "not a failure"}
            ]
        });
        assert_eq!(trace_of(&status).as_deref(), Some("boom one"));
        assert_eq!(trace_of(&json!({"connector": {"state": "RUNNING"}})), None);
    }

    #[tokio::test]
    async fn an_allowed_class_is_put_to_the_config_path() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/connectors/lakehouse-a/config"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        let client = KafkaConnectClient::new(&server.uri()).unwrap();
        client
            .put_config("lakehouse-a", &config(POSTGRES_CONNECTOR_CLASS))
            .await
            .unwrap();
    }

    /// `SRC-4` slices 5 and 6: `MongoDB`'s and `Oracle`'s classes are allowed, and the list is
    /// exactly the five classes this build starts (a sixth needs its slice and this test).
    #[test]
    fn the_allowlist_is_the_five_classes_this_build_starts() {
        let mut classes = ALLOWED_CONNECTOR_CLASSES.to_vec();
        classes.sort_unstable();
        assert_eq!(
            classes,
            [
                "io.debezium.connector.mongodb.MongoDbConnector",
                "io.debezium.connector.mysql.MySqlConnector",
                "io.debezium.connector.oracle.OracleConnector",
                "io.debezium.connector.postgresql.PostgresConnector",
                "io.debezium.connector.sqlserver.SqlServerConnector",
            ]
        );
    }

    #[tokio::test]
    async fn upstream_text_is_classified_and_never_carried() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("password authentication failed for user secret-user"),
            )
            .mount(&server)
            .await;
        let client = KafkaConnectClient::new(&server.uri()).unwrap();
        let err = client.pause("lakehouse-a").await.unwrap_err();
        assert_eq!(err, ConnectError::Rejected);
        assert!(!err.to_string().contains("secret-user"));
        let api: ApiError = err.into();
        assert!(!format!("{api:?}").contains("secret-user"));
    }

    #[tokio::test]
    async fn restart_asks_for_the_tasks_too() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/connectors/lakehouse-a/restart"))
            .and(wiremock::matchers::query_param("includeTasks", "true"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        let client = KafkaConnectClient::new(&server.uri()).unwrap();
        client.restart("lakehouse-a").await.unwrap();
    }

    #[tokio::test]
    async fn deleting_a_missing_connector_is_success_and_state_reads_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/connectors/lakehouse-a/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "connector": {"state": "RUNNING"},
                "tasks": [{"state": "RUNNING"}],
            })))
            .mount(&server)
            .await;
        let client = KafkaConnectClient::new(&server.uri()).unwrap();
        client.delete("lakehouse-a").await.unwrap();
        assert_eq!(
            client.state("lakehouse-a").await.unwrap(),
            ConnectState::Running
        );
    }
}
