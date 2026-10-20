//! Quality incidents (`DATA-14`, F2): what is wrong now, by table.
//!
//! A failed run of an authored rule opens an incident
//! ([`open_for_failed_run`]); it stays open until a person resolves it
//! with a reason (`POST /api/governance/quality/incidents/{id}/resolve`,
//! `governance:write`, audited as `quality.incident_resolve`). A passing
//! run does not close one. Deleting a rule resolves its open incidents
//! ([`resolve_for_deleted_rule`]) with the resolution "rule deleted" and
//! the deleting user. The records live in Postgres
//! (`lakehouse_store::quality`, migration `0081`).
//!
//! The legacy rows a quality job recorded in `_silver_meta.quality` have
//! no stable id and are not rules, so they open no incident; only the
//! authored rules and the learned anomalies do.
//!
//! Bookkeeping here never fails the event that caused it: a run whose
//! incident could not be saved is still a saved run, and the next failed
//! run tries again.

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::governance::QualityRule;
use lakehouse_store::quality::{self, IncidentKind, Resolution, StatusFilter};
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::quality::pool;
use crate::state::AppState;

/// The resolution recorded when the rule an incident belongs to is deleted.
pub(crate) const RULE_DELETED: &str = "rule deleted";

/// Opens (or moves the `last_seen_at` of) the incident for a rule whose
/// run just failed. The detail is what the run measured and the threshold
/// it was held to, so the list says what is wrong without opening the rule.
pub(crate) async fn open_for_failed_run(state: &AppState, rule: &QualityRule, measured: &str) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    // PR #106 review R2 BLOCKER 9: commit incident first (never lost when rule store is down)
    let detail = format!("{} — {measured} (threshold: {})", rule.name, rule.threshold);
    let recorded = match quality::record_failure(
        pg,
        &rule.asset,
        Some(&rule.id),
        IncidentKind::Check,
        &detail,
    )
    .await
    {
        Ok(rec) => rec,
        Err(err) => {
            tracing::warn!(?err, rule = %rule.name, "quality: the incident could not be recorded");
            return;
        }
    };
    tracing::debug!(rule = %rule.name, opened = recorded.opened, "quality: incident recorded");
    deliver_incident_alert(state, pg, &recorded.id, &rule.asset, &detail).await;
}

// PR #106 review R2 BLOCKER 9: retriable incident alert delivery (SRC-17 F2).
// Rules are read first; if the rule store is down, the incident is already open
// in Postgres, leaving the handover marker absent so a later pass/retry delivers.
// When rules are read, the marker is written to pipeline_run_event and delivery is awaited.
async fn deliver_incident_alert(
    state: &AppState,
    pool: &lakehouse_store::PgPool,
    incident_id: &str,
    asset: &str,
    detail: &str,
) {
    let rules = match crate::routes::load_alerts::read_rules(state).await {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(?err, %asset, incident = incident_id, "quality: alert rules unavailable, leaving incident open for later delivery");
            return;
        }
    };
    let handed_over = match lakehouse_store::pipelines::record_pipeline_run_event(
        pool,
        incident_id,
        asset,
        "quality_incident_alerted",
    )
    .await
    {
        Ok(inserted) => inserted,
        Err(err) => {
            tracing::warn!(?err, %asset, incident = incident_id, "quality: could not record incident alert marker");
            return;
        }
    };
    if !handed_over {
        tracing::debug!(
            incident = incident_id,
            "quality: incident already alerted; skipping delivery"
        );
        return;
    }
    let text = format!("{asset}: {detail}. View: /governance/data-quality");
    crate::routes::load_alerts::deliver_quality_incident_with_rules(
        state, pool, &rules, asset, &text,
    )
    .await;
}

/// Opens the incident of an anomaly (`DATA-14`, F5) for `asset`, or moves the
/// `last_seen_at` of the one already open. Returns the incident's id when
/// this call opened it, `None` when it was already open or could not be
/// saved (logged). Bookkeeping after a load or a tick, so it never fails.
pub(crate) async fn open_anomaly(
    state: &AppState,
    asset: &str,
    kind: IncidentKind,
    detail: &str,
) -> Option<String> {
    let pg = state.pg.as_deref()?;
    // PR #106 review R2 BLOCKER 9: commit incident first (never lost when rule store is down)
    let recorded = match quality::record_failure(pg, asset, None, kind, detail).await {
        Ok(rec) => rec,
        Err(err) => {
            tracing::warn!(?err, %asset, kind = kind.as_str(), "quality: the anomaly could not be recorded");
            return None;
        }
    };
    deliver_incident_alert(state, pg, &recorded.id, asset, detail).await;
    recorded.opened.then_some(recorded.id)
}

/// Resolves the open incidents of a rule that was just deleted.
pub(crate) async fn resolve_for_deleted_rule(
    state: &AppState,
    rule: &QualityRule,
    principal: &Principal,
) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    if let Err(err) =
        quality::resolve_for_rule(pg, &rule.id, principal.id.uuid(), RULE_DELETED).await
    {
        tracing::warn!(?err, rule = %rule.name, "quality: incidents of the deleted rule were not resolved");
    }
}

// PR #106 review R1 SHOULD-FIX DATA-14 3 / D2: a passing run marks any open incident for this rule stale
pub(crate) async fn mark_stale_for_passed_run(state: &AppState, rule: &QualityRule) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    if let Err(err) = quality::mark_stale_for_rule(pg, &rule.id).await {
        tracing::warn!(?err, rule = %rule.name, "quality: could not mark open incident stale");
    }
}

/// The `GET /api/governance/quality/incidents` query.
#[derive(Debug, serde::Deserialize)]
pub struct IncidentsQuery {
    /// `open` (the default), `resolved` or `all`.
    #[serde(default)]
    pub status: Option<String>,
    /// Narrow to one table.
    #[serde(default)]
    pub asset: Option<String>,
}

/// `GET /api/governance/quality/incidents?status=&asset=` — incidents,
/// newest first, at most 500. Readable by any login, like the quality
/// rule list it belongs beside.
///
/// # Errors
///
/// `400` for a `status` that is not `open`, `resolved` or `all`;
/// `503`/`500` on a store failure.
pub async fn list_incidents(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<IncidentsQuery>,
) -> ApiResult<ApiJson<Value>> {
    // PR #106 review R1 SHOULD-FIX DATA-14 1 / D4
    if let Some(reason) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await?
    {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    let status = match query.status.as_deref() {
        None | Some("") => StatusFilter::Open,
        Some(other) => StatusFilter::parse(other).ok_or_else(|| {
            ApiError::BadRequest("status must be open, resolved or all.".to_owned())
        })?,
    };
    let asset = query.asset.as_deref().filter(|a| !a.trim().is_empty());
    let incidents = quality::list(pool(&state)?, status, asset).await?;
    Ok(ApiJson(json!({ "incidents": incidents })))
}

/// The `POST …/incidents/{id}/resolve` body.
#[derive(Debug, serde::Deserialize)]
pub struct ResolveBody {
    resolution: String,
}

/// `POST /api/governance/quality/incidents/{id}/resolve` — close an open
/// incident with the reason a person gives. Audited as
/// `quality.incident_resolve` on the incident's table.
///
/// # Errors
///
/// `400` when the body is malformed or the reason is empty or too long;
/// `404` when no incident has that id; `409` when it was resolved before;
/// `503`/`500` on a store failure.
pub async fn resolve_incident(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: ResolveBody = serde_json::from_slice(&body)
        // SEC-11 (CI run on DATA-14): a fixed sentence, not serde's text.
        .map_err(|_| ApiError::BadRequest("Body must be JSON {resolution}.".to_owned()))?;
    let incident = match quality::resolve(pool(&state)?, &id, principal.id.uuid(), &body.resolution)
        .await?
    {
        Resolution::Resolved(incident) => incident,
        Resolution::AlreadyResolved => {
            return Err(ApiError::Conflict("That incident is already resolved.".to_owned()).into());
        }
        Resolution::NotFound => {
            return Err(ApiError::NotFound("Quality incident not found".to_owned()).into());
        }
    };
    crate::routes::governance::audit_table_change(
        &state,
        Some(&principal),
        &incident.asset,
        "quality.incident_resolve",
        json!({
            "incidentId": incident.id,
            "kind": incident.kind,
            "ruleId": incident.rule_id,
            "resolution": incident.resolution,
        }),
    )
    .await;
    Ok(ApiJson(json!(incident)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use std::time::Duration;

    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    struct Harness {
        state: AppState,
        webhook: MockServer,
        _clickhouse: MockServer,
    }

    /// Postgres plus mocked `ClickHouse` (the rule store) and webhook servers.
    /// `rules` are `(id, asset, enabled)` quality incident rules; each rule's
    /// webhook is `/<id>` on the webhook mock.
    async fn harness(pool: &sqlx::PgPool, rules: &[(&str, &str, bool)]) -> Harness {
        let ch = MockServer::start().await;
        let webhook = MockServer::start().await;
        let options = pool.connect_options();
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_owned(),
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options.get_database().expect("a named test database"),
            ),
        );
        env.insert("CH_URL".to_owned(), ch.uri());
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        // The webhook mock listens on loopback, which the real address policy
        // refuses (`SEC-10`); the policy has its own tests.
        state.webhook_sender_override = Some(lakehouse_notify::MappedResolver::new().into_sender());
        let data: Vec<Value> = rules
            .iter()
            .map(|(id, asset, enabled)| {
                json!({
                    "id": id, "name": format!("Rule {id}"), "type": "quality_incident",
                    "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "0",
                    "board": "", "channel": "webhook",
                    "target": format!("{}/{id}", webhook.uri()),
                    "enabled": if *enabled { "1" } else { "0" },
                    "created_at": "", "severity": "high", "pipeline": "", "connector": "",
                    "asset": asset,
                })
            })
            .collect();
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "rows": data.len(), "data": data })),
            )
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&webhook)
            .await;
        Harness {
            state,
            webhook,
            _clickhouse: ch,
        }
    }

    impl Harness {
        async fn posts_to(&self, rule_id: &str) -> Vec<String> {
            self.webhook
                .received_requests()
                .await
                .expect("request log")
                .iter()
                .filter(|r| r.url.path() == format!("/{rule_id}"))
                .map(|r| String::from_utf8_lossy(&r.body).into_owned())
                .collect()
        }

        /// The delivery runs in the background: wait for `count` posts.
        async fn wait_for_posts(&self, rule_id: &str, count: usize) {
            for _ in 0..100 {
                if self.posts_to(rule_id).await.len() >= count {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("{rule_id} never reached {count} posts");
        }

        /// Gives a background delivery that must NOT happen time to have happened.
        async fn settle() {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn rule(asset: &str) -> QualityRule {
        QualityRule {
            id: "r1".to_owned(),
            name: "email_filled".to_owned(),
            asset: asset.to_owned(),
            dimension: "completeness".to_owned(),
            threshold: "email not null >= 90%".to_owned(),
            severity: "high".to_owned(),
            last_status: None,
            last_run_at: None,
        }
    }

    /// `DATA-14` F6: an incident that opens is delivered to the rules of its
    /// table and to `*` rules, once; the same incident seen again delivers
    /// nothing; another table's rule and a disabled rule never deliver.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_opened_incident_is_delivered_once_to_the_rules_of_its_table(pool: sqlx::PgPool) {
        let h = harness(
            &pool,
            &[
                ("one", "silver.orders", true),
                ("all", "*", true),
                ("other", "silver.customers", true),
                ("off", "silver.orders", false),
            ],
        )
        .await;
        let failing = rule("silver.orders");

        open_for_failed_run(&h.state, &failing, "75.0% not null").await;
        h.wait_for_posts("one", 1).await;
        h.wait_for_posts("all", 1).await;
        Harness::settle().await;
        assert_eq!(h.posts_to("other").await.len(), 0);
        assert_eq!(h.posts_to("off").await.len(), 0);

        // Failing again moves `last_seen_at`; nobody is told twice.
        open_for_failed_run(&h.state, &failing, "74.0% not null").await;
        Harness::settle().await;
        assert_eq!(h.posts_to("one").await.len(), 1);
        assert_eq!(h.posts_to("all").await.len(), 1);

        // The message names the table, the rule, what was measured and the
        // threshold, and links to the console — nothing else.
        let body = &h.posts_to("one").await[0];
        for needle in [
            "silver.orders",
            "email_filled",
            "75.0% not null",
            "email not null >= 90%",
            "/governance/data-quality",
        ] {
            assert!(body.contains(needle), "{needle} missing from {body}");
        }
    }

    /// Each delivery is also listed on the Alerts page, under its own source.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_delivered_quality_alert_is_listed_as_an_instance(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("one", "silver.orders", true)]).await;
        open_for_failed_run(&h.state, &rule("silver.orders"), "75.0% not null").await;
        h.wait_for_posts("one", 1).await;

        let mut listed: i64 = 0;
        for _ in 0..100 {
            listed = sqlx::query_scalar(
                "SELECT count(*) FROM alert_instance WHERE source = 'Data quality'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if listed > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(listed, 1);
    }

    /// An anomaly opens an incident and is delivered the same way, once.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_opened_anomaly_is_delivered_once(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("all", "*", true)]).await;
        let detail = "The latest load added 100 rows, 10% of the usual";

        let opened = open_anomaly(
            &h.state,
            "silver.orders",
            IncidentKind::VolumeAnomaly,
            detail,
        )
        .await;
        assert!(opened.is_some());
        h.wait_for_posts("all", 1).await;
        assert!(h.posts_to("all").await[0].contains(detail));

        let again = open_anomaly(
            &h.state,
            "silver.orders",
            IncidentKind::VolumeAnomaly,
            detail,
        )
        .await;
        assert_eq!(again, None);
        Harness::settle().await;
        assert_eq!(h.posts_to("all").await.len(), 1);
    }

    /// With no rule for the table, opening an incident delivers nothing and
    /// still succeeds.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_incident_with_no_matching_rule_is_still_opened(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("other", "silver.customers", true)]).await;
        open_for_failed_run(&h.state, &rule("silver.orders"), "75.0% not null").await;
        Harness::settle().await;
        assert_eq!(h.posts_to("other").await.len(), 0);
        let open = quality::list(&pool, StatusFilter::Open, None)
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
    }

    // PR #106 review R2 BLOCKER 9: rule-store failure commits incident row and leaves marker absent for retry
    // PR #106 review R3 SHOULD-FIX 16: prove retry on re-sighting when rule store recovers
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rule_store_failure_commits_incident_row_and_retries_on_next_pass(
        pool: sqlx::PgPool,
    ) {
        let ch = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("clickhouse down"))
            .mount(&ch)
            .await;
        let options = pool.connect_options();
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_owned(),
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options.get_database().expect("a named test database"),
            ),
        );
        env.insert("CH_URL".to_owned(), ch.uri());
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        state.webhook_sender_override = Some(lakehouse_notify::MappedResolver::new().into_sender());

        open_for_failed_run(&state, &rule("silver.orders"), "90%").await;

        let open_incidents = quality::list(&pool, StatusFilter::Open, None)
            .await
            .unwrap();
        assert_eq!(
            open_incidents.len(),
            1,
            "incident row must be committed even when reading alert rules fails"
        );
        let alerted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline_run_event WHERE kind = 'quality_incident_alerted'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            alerted, 0,
            "handover marker must not be written when reading alert rules fails"
        );

        // Second pass: ClickHouse recovers; re-sighting the failure delivers and records marker.
        ch.reset().await;
        let webhook = MockServer::start().await;
        let data = vec![json!({
            "id": "r1", "name": "Rule r1", "type": "quality_incident",
            "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "0",
            "board": "", "channel": "webhook",
            "target": format!("{}/r1", webhook.uri()),
            "enabled": "1",
            "created_at": "", "severity": "high", "pipeline": "", "connector": "",
            "asset": "silver.orders",
        })];
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "rows": data.len(), "data": data })),
            )
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&webhook)
            .await;

        open_for_failed_run(&state, &rule("silver.orders"), "90%").await;

        let alerted_retry: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline_run_event WHERE kind = 'quality_incident_alerted'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            alerted_retry, 1,
            "handover marker must be written when reading alert rules succeeds on retry"
        );

        let requests = webhook.received_requests().await.expect("request log");
        assert_eq!(requests.len(), 1, "webhook must receive alert on retry");
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("silver.orders"));
    }
}
