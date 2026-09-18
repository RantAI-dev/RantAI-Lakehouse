//! `GET /api/catalog`, `GET /api/catalog/{id}` — the dataset registry.
//!
//! Ports `src/app/api/catalog/route.ts` and
//! `src/app/api/catalog/[id]/route.ts`. Bronze/raw assets come from the
//! `bronze_meta`/`bronze_meta_sec` registry tables in `lake`; Silver/Gold
//! assets are read directly off `ClickHouse`'s `system.tables` /
//! `system.columns` / `system.parts`.
//!
//! # `GET`/`PUT /api/catalog/{id}/annotation` — bounded console metadata
//!
//! `asset_annotation` (migration `0032`) holds console-only metadata a
//! `catalog:write` holder can attach to any asset id: owner, steward, tags,
//! description. `PUT` reuses the already-seeded `catalog:write` permission
//! (WS2 plan review W8) — no new permission string. Because any
//! `catalog:write` holder can call it, the body is bounded on both sides —
//! validated here, returning `400` naming the offending field, and mirrored
//! as `CHECK` constraints in `0032_asset_annotation.sql` as defense in
//! depth:
//! - `owner`/`steward`: at most 128 characters each;
//! - `description`: at most 4,000 characters;
//! - `tags`: at most 20 entries, each 1-64 characters matching
//!   `^[a-z0-9][a-z0-9_-]*$`;
//! - the `{id}` path segment itself: at most 200 characters.
//!
//! The stored key is `asset_id`, the full catalog id as this route's own
//! `{id}` path parameter provides it — a bare slug for Bronze
//! (`bronze_catalog_row`), `"silver.<name>"` for Silver
//! (`silver_catalog_row`), `"serving.<name>"` for Gold (`gold_catalog_row`)
//! — never a re-derived table name.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_core::ident::SqlLiteral;
use lakehouse_iceberg::IcebergClient;
use lakehouse_iceberg::rest;
use lakehouse_store::PgPool;
use lakehouse_store::annotation::AnnotationRow;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use crate::bounded;
use crate::bronze_stats_cache::{BronzeStatsCache, CachedTableStats};
use crate::error::{ApiRejection, ApiResult};
use crate::json::ApiJson;
use crate::lakehouse_catalog::{self, CatalogAccessError};
use crate::routes::support::{js_error, js_string, num_or_zero, prettify, str_col};
use crate::state::AppState;

use crate::tenant::{
    NAMESPACE_META, NAMESPACE_PRIMER, NAMESPACE_SEKUNDER, TENANT_DOMAIN, TENANT_OWNER,
    TENANT_RESIDENCY, is_curated_bronze,
};

/// Query parameters accepted by `GET /api/catalog`. `q` is the free-text
/// search term the command palette sends (WS2 §13) — moved here from
/// `src/services/clients/assets.ts`'s browser-side filter so there is one
/// implementation of the term match, not two (`AGENTS.md` rule 4).
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    q: Option<String>,
}

/// `GET /api/catalog` — the full asset registry, grouped into namespaces,
/// optionally narrowed by `?q=` to assets whose `name`/`description`/`id`
/// (or, when Postgres is configured, whose annotation `description`/`tags`)
/// contain the term.
pub async fn list(State(state): State<AppState>, Query(params): Query<ListQuery>) -> Response {
    match list_body(&state.clickhouse).await {
        Ok((mut body, bronze_pairs)) => {
            enrich_bronze_assets(&state, &mut body, bronze_pairs).await;
            if let Some(q) = params.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
                let annotations = match state.pg.as_deref() {
                    Some(pool) => match lakehouse_store::annotation::list_all(pool).await {
                        Ok(rows) => rows,
                        Err(err) => {
                            // Annotations widen the search; they never gate
                            // it. A store failure here degrades to
                            // name/description/id matching only, not a
                            // failed request.
                            tracing::warn!(%err, "asset_annotation lookup failed during catalog search");
                            Vec::new()
                        }
                    },
                    None => Vec::new(),
                };
                if let Some(assets) = body.get("assets").and_then(Value::as_array) {
                    let filtered = filter_assets_by_query(assets, q, &annotations);
                    body["assets"] = Value::Array(filtered);
                }
            }
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        // `catch (e) { return NextResponse.json({ error: String(e), assets:
        // [], namespaces: [] }, { status: 503 }); }` in `catalog/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err), "assets": [], "namespaces": [] })),
        )
            .into_response(),
    }
}

/// Case-insensitive substring match over each asset's `name`, `description`,
/// and `id` — the SAME fields and case rule
/// `src/services/clients/assets.ts`'s `clickhouseAssetService.listAssets`
/// used to apply in the browser, moved here so there is one implementation
/// (WS2 §13). Case folding uses `to_lowercase()`, not `to_ascii_lowercase()`,
/// because the browser's JavaScript `toLowerCase()` is Unicode-aware and a
/// faithful port must match it for non-ASCII text in descriptions and
/// annotations. Widened by `annotations`: an asset also matches when its own
/// annotation row's `description` or any `tags` entry contains the term. An
/// empty `q` matches everything.
fn filter_assets_by_query(assets: &[Value], q: &str, annotations: &[AnnotationRow]) -> Vec<Value> {
    let needle = q.trim().to_lowercase();
    if needle.is_empty() {
        return assets.to_vec();
    }
    let by_id: HashMap<&str, &AnnotationRow> = annotations
        .iter()
        .map(|row| (row.asset_id.as_str(), row))
        .collect();
    assets
        .iter()
        .filter(|asset| {
            let id = asset["id"].as_str().unwrap_or_default();
            let name = asset["name"].as_str().unwrap_or_default().to_lowercase();
            let description = asset["description"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase();
            if name.contains(&needle)
                || description.contains(&needle)
                || id.to_lowercase().contains(&needle)
            {
                return true;
            }
            by_id.get(id).is_some_and(|row| {
                row.description
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&needle))
                    || row.tags.iter().any(|t| t.to_lowercase().contains(&needle))
            })
        })
        .cloned()
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "one straight-line port of a single TS handler; splitting it up \
              would scatter the query→merge→group pipeline across helpers \
              with no independent reuse, hurting rather than helping \
              readability of the port"
)]
/// Returns the catalog body plus every Bronze row's `(slug, table_name)`
/// pair, straight from the `cat` registry rows — `list` hands these to
/// [`enrich_bronze_assets`] rather than re-deriving slugs from table names.
async fn list_body(ch: &ChClient) -> Result<(Value, Vec<(String, String)>), ChError> {
    let cat = ch
        .rows(
            "SELECT slug, title, description, tier, updated_at, table_name FROM lake.`bronze_meta.dataset_catalog`
       UNION ALL
       SELECT slug, title, description, tier, updated_at, table_name FROM lake.`bronze_meta_sec.dataset_catalog`",
            None,
        )
        .await?;
    let sync_rows = ch
        .rows(
            "SELECT slug, toString(total) total, author, frekuensi FROM lake.`bronze_meta.dataset_sync`
       UNION ALL SELECT slug, toString(total) total, author, frekuensi FROM lake.`bronze_meta_sec.dataset_sync`",
            None,
        )
        .await?;
    let col_rows = ch
        .rows(
            "SELECT slug, toString(count()) n FROM lake.`bronze_meta.dataset_column` GROUP BY slug
       UNION ALL SELECT slug, toString(count()) n FROM lake.`bronze_meta_sec.dataset_column` GROUP BY slug",
            None,
        )
        .await?;

    let total_of = |slug: &str| -> i64 {
        sync_rows
            .iter()
            .find(|s| str_col(s, "slug") == slug)
            .map_or(0, |s| num_or_zero(Some(s), "total"))
    };
    let author_of = |slug: &str| -> String {
        sync_rows
            .iter()
            .find(|s| str_col(s, "slug") == slug)
            .map(|s| str_col(s, "author").to_owned())
            .unwrap_or_default()
    };
    let col_of = |slug: &str| -> i64 {
        col_rows
            .iter()
            .find(|c| str_col(c, "slug") == slug)
            .map_or(0, |c| num_or_zero(Some(c), "n"))
    };

    let mut assets: Vec<Value> = cat
        .iter()
        .map(|c| {
            let slug = str_col(c, "slug");
            let rows = total_of(slug);
            let sekunder = str_col(c, "tier") == "sekunder";
            let owner = author_of(slug);
            let description = str_col(c, "description");
            let updated_at = str_col(c, "updated_at");
            bronze_catalog_row(
                slug,
                str_col(c, "title"),
                sekunder,
                owner.as_str(),
                description,
                rows,
                col_of(slug),
                updated_at,
            )
        })
        .collect();

    let bronze_table_names: HashSet<&str> = cat.iter().map(|c| str_col(c, "table_name")).collect();
    // Handed back to `list` for `enrich_bronze_assets` — taken from `cat`
    // directly rather than re-derived from the `assets` rows above, so a
    // future change to `bronze_catalog_row`'s shape can never silently
    // break the slug/table_name pairing this depends on.
    let bronze_pairs: Vec<(String, String)> = cat
        .iter()
        .map(|c| {
            (
                str_col(c, "slug").to_owned(),
                str_col(c, "table_name").to_owned(),
            )
        })
        .collect();

    let (tbl_rows, col_count_rows, part_rows) = tokio::try_join!(
        ch.rows(
            "SELECT database db, name, engine FROM system.tables
         WHERE database IN ('silver','serving') ORDER BY name",
            None,
        ),
        ch.rows(
            "SELECT database db, table, toString(count()) n FROM system.columns
         WHERE database IN ('silver','serving') GROUP BY database, table",
            None,
        ),
        ch.rows(
            "SELECT table, toString(sum(rows)) r FROM system.parts
         WHERE database='serving' AND active GROUP BY table",
            None,
        ),
    )?;

    let col_count_of = |db: &str, table: &str| -> i64 {
        col_count_rows
            .iter()
            .find(|c| str_col(c, "db") == db && str_col(c, "table") == table)
            .map_or(0, |c| num_or_zero(Some(c), "n"))
    };
    let gold_rows_of = |table: &str| -> i64 {
        part_rows
            .iter()
            .find(|p| str_col(p, "table") == table)
            .map_or(0, |p| num_or_zero(Some(p), "r"))
    };

    for t in &tbl_rows {
        let db = str_col(t, "db");
        let name = str_col(t, "name");
        let engine = str_col(t, "engine");
        if db == "silver" {
            if bronze_table_names.contains(name) {
                continue;
            }
            assets.push(silver_catalog_row(
                name,
                engine,
                col_count_of("silver", name),
            ));
        } else {
            if name.ends_with("_baru") {
                continue;
            }
            let rows = gold_rows_of(name);
            assets.push(gold_catalog_row(
                name,
                engine,
                rows,
                col_count_of("serving", name),
            ));
        }
    }

    let namespaces = build_namespaces(&assets);
    Ok((
        json!({ "assets": assets, "namespaces": namespaces }),
        bronze_pairs,
    ))
}

// WS2 — filling Bronze `sizeBytes`/`freshnessLagSeconds` from Iceberg.
//
// `dataset_catalog.table_name` (read by `list_body` above as the second
// element of each `bronze_pairs` entry) is written by two different
// producers, with two different meanings, and no registry column records
// which one wrote a given row:
//
// - The demo seed (`demo/clickhouse/04_registry.sql:64-91`) sets
//   `table_name` to a plain `ClickHouse` table living in database `lake`
//   (e.g. `'commerce_orders'`) — confirmed by the same file's `total`/
//   column backfill, which joins `system.parts`/`system.columns` `WHERE
//   database = 'lake'` on that exact name (`:125-128`, `:160-164`). No
//   Bronze Iceberg table by that name exists in Lakekeeper for these rows.
// - The Dagster/dlt ingest path
//   (`dagster/dispar_orchestrate/assets.py:44-53` calling
//   `bronze_catalog.py::register_bronze_table` at `:311-323`) sets
//   `table_name` to `summary["bronze_table_name"]`, the exact string dlt
//   uses as the destination table name (`dlt_pipeline.py:261`,
//   `resource.apply_hints(table_name=cfg.bronze_table_name)`) when writing
//   through Lakekeeper's REST catalog into the flat `bronze` namespace
//   (`dlt_pipeline.py:281-286`, `dataset_name="bronze"` — the same flat
//   namespace `docs/adr/0004-bronze-naming-partitioning-retention.md`
//   establishes). For these rows, `table_name` genuinely is the Bronze
//   Iceberg table name.
//
// Because the registry cannot tell the two apart, enrichment never trusts
// `table_name` on its own: `iceberg_candidates` only keeps a pair whose
// `table_name` is confirmed present in a real listing of Lakekeeper's
// `bronze` namespace (`rest::list_table_idents`, cached alongside the
// per-table stats — see `crate::bronze_stats_cache`). A demo-seed row's
// `table_name` (a `lake.*` table) is excluded and its Bronze fields stay
// `null`, never guessed at.
//
// Known limitation: if a demo-seed `table_name` ever collides with a real
// Bronze Iceberg table name (a future Dagster ingest genuinely creating
// `bronze.commerce_orders`, say), that unrelated row would receive that
// table's stats. This is judged acceptable because the collision requires
// an exact match against a table that actually exists, and the far more
// common failure mode — no match — is handled correctly by never
// fabricating a value for it.

/// Per-request budget for enriching the catalog LIST from Iceberg —
/// deliberately short: this route is on the page-load path, and a slow or
/// unreachable Lakekeeper must degrade to `null` Bronze fields, never a
/// slow page. Covers the connect, the (possibly cached) namespace listing,
/// and the per-table loads together, not each separately.
const ICEBERG_ENRICHMENT_BUDGET: Duration = Duration::from_millis(300);
/// At most this many concurrent Lakekeeper `load_table` calls at once.
const ICEBERG_ENRICHMENT_MAX_CONCURRENT: usize = 8;

/// Keeps only the `(slug, table_name)` pairs whose `table_name` is
/// confirmed present in `bronze_tables` — see the module comment above for
/// why `dataset_catalog.table_name` cannot be trusted without this check.
fn iceberg_candidates(
    pairs: &[(String, String)],
    bronze_tables: &HashSet<String>,
) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter(|(_, table_name)| bronze_tables.contains(table_name))
        .cloned()
        .collect()
}

/// Sets `sizeBytes`/`freshnessLagSeconds` on every Bronze row (`"type":
/// "iceberg-table"`) whose `id` (slug) is a key in `by_slug`. Every other
/// row, and every other field on a matching row, is untouched — `health`
/// stays `"unknown"` (WS2 does not measure health; see the module comment
/// above [`bronze_catalog_row`]).
///
/// `freshnessLagSeconds` is `(now_ms - last_updated_ms) / 1000`, computed
/// here at response time (never cached, never `0` as a stand-in for
/// "unknown") — `null` when there is no snapshot timestamp, or when the
/// computed lag would be negative (clock skew between this service and
/// whatever wrote the snapshot), never a fabricated non-negative number.
fn apply_iceberg_enrichment(
    assets: &mut [Value],
    by_slug: &HashMap<String, CachedTableStats>,
    now_ms: i64,
) {
    for asset in assets.iter_mut() {
        let Some(obj) = asset.as_object_mut() else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("iceberg-table") {
            continue;
        }
        let Some(slug) = obj.get("id").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let Some(stats) = by_slug.get(&slug) else {
            continue;
        };
        obj.insert(
            "sizeBytes".to_owned(),
            stats.total_bytes.map_or(Value::Null, |bytes| json!(bytes)),
        );
        let lag_seconds = stats.last_updated_ms.and_then(|updated_ms| {
            let lag_ms = now_ms - updated_ms;
            if lag_ms < 0 {
                None
            } else {
                Some(lag_ms / 1000)
            }
        });
        obj.insert(
            "freshnessLagSeconds".to_owned(),
            lag_seconds.map_or(Value::Null, |lag| json!(lag)),
        );
    }
}

/// Current time in epoch milliseconds, for [`apply_iceberg_enrichment`]'s
/// freshness computation. Falls back to `0` (a maximal, honestly-wrong-in-a-
/// safe-direction lag) only if the system clock is somehow before the Unix
/// epoch — never panics.
fn now_millis() -> i64 {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    i64::try_from(millis).unwrap_or(i64::MAX)
}

/// The bounded, budgeted, cache-aware core of Bronze Iceberg enrichment.
///
/// `connect` is the one injectable seam: production passes
/// `lakehouse_catalog::client`, and the budget test below passes a future
/// that never resolves, proving the whole pipeline degrades to "no
/// enrichment" within `budget` rather than hanging the response — without
/// a live Lakekeeper. Once connected, this function ALWAYS talks to the
/// real `lakehouse_iceberg::rest` surface directly (never
/// `lakehouse_catalog::call`, never a retry) — see
/// `crate::lakehouse_catalog::call`'s own doc comment on why a per-item
/// fan-out call must not go through it.
///
/// # Errors
/// Never returns an error: a connect timeout/failure, a listing
/// timeout/failure, or an unfinished per-table load all degrade to an
/// empty (or partial) map rather than failing the request. The cause is
/// logged once per request at the point it happens.
async fn enrich_bronze_stats<ConnectFut>(
    pairs: Vec<(String, String)>,
    cache: &BronzeStatsCache,
    budget: Duration,
    max_concurrent: usize,
    bronze_ns: &NamespaceIdent,
    connect: impl FnOnce() -> ConnectFut,
) -> HashMap<String, CachedTableStats>
where
    ConnectFut: Future<Output = Result<Arc<IcebergClient>, CatalogAccessError>>,
{
    if pairs.is_empty() {
        return HashMap::new();
    }

    let start = Instant::now();
    let client = match tokio::time::timeout(budget, connect()).await {
        Ok(Ok(client)) => client,
        Ok(Err(err)) => {
            tracing::warn!(
                ?err,
                "Iceberg enrichment: could not connect to Lakekeeper; the catalog list is unchanged"
            );
            return HashMap::new();
        }
        Err(_elapsed) => {
            tracing::warn!(
                "Iceberg enrichment: connecting to Lakekeeper exceeded the request budget; \
                 the catalog list is unchanged"
            );
            return HashMap::new();
        }
    };

    let bronze_tables = if let Some(names) = cache.get_bronze_table_names(Instant::now()) {
        names
    } else {
        let remaining = budget.saturating_sub(Instant::now().saturating_duration_since(start));
        match tokio::time::timeout(remaining, rest::list_table_idents(&client, bronze_ns)).await {
            Ok(Ok(idents)) => {
                let names: HashSet<String> = idents
                    .into_iter()
                    .map(|ident| ident.name().to_owned())
                    .collect();
                cache.put_bronze_table_names(names.clone(), Instant::now());
                names
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    ?err,
                    "Iceberg enrichment: could not list the bronze namespace; the catalog list is unchanged"
                );
                return HashMap::new();
            }
            Err(_elapsed) => {
                tracing::warn!(
                    "Iceberg enrichment: listing the bronze namespace exceeded the request \
                     budget; the catalog list is unchanged"
                );
                return HashMap::new();
            }
        }
    };

    let candidates = iceberg_candidates(&pairs, &bronze_tables);
    if candidates.is_empty() {
        return HashMap::new();
    }

    let now = Instant::now();
    let mut by_slug = HashMap::new();
    let mut miss_table_names = Vec::new();
    let mut slug_by_table: HashMap<String, String> = HashMap::new();
    for (slug, table_name) in candidates {
        if let Some(stats) = cache.get_stats(&table_name, now) {
            by_slug.insert(slug, stats);
        } else {
            slug_by_table.insert(table_name.clone(), slug);
            miss_table_names.push(table_name);
        }
    }
    if miss_table_names.is_empty() {
        return by_slug;
    }

    let remaining = budget.saturating_sub(Instant::now().saturating_duration_since(start));
    let loaded = bounded::run_with_budget(miss_table_names, remaining, max_concurrent, {
        let client = client.clone();
        let bronze_ns = bronze_ns.clone();
        move |table_name: String| {
            let client = client.clone();
            let ident = TableIdent::new(bronze_ns.clone(), table_name);
            async move {
                rest::load_table_summary(&client, &ident)
                    .await
                    .ok()
                    .map(|summary| CachedTableStats {
                        total_bytes: summary.stats.total_bytes,
                        last_updated_ms: summary.last_updated_ms,
                    })
            }
        }
    })
    .await;

    let write_now = Instant::now();
    for (table_name, stats) in loaded {
        cache.put_stats(table_name.clone(), stats, write_now);
        if let Some(slug) = slug_by_table.get(&table_name) {
            by_slug.insert(slug.clone(), stats);
        }
    }
    by_slug
}

/// Wires [`enrich_bronze_stats`] to production: the real cache on
/// `AppState`, the real budget/concurrency constants, and
/// `lakehouse_catalog::client` as the connect seam. Mutates `body`'s
/// `assets` array in place; a body with no `assets` array (never happens
/// in practice — `list_body` always sets it) is left untouched.
async fn enrich_bronze_assets(
    state: &AppState,
    body: &mut Value,
    bronze_pairs: Vec<(String, String)>,
) {
    if bronze_pairs.is_empty() {
        return;
    }
    let Ok(bronze_ns) = NamespaceIdent::from_strs(["bronze"]) else {
        return;
    };
    let by_slug = enrich_bronze_stats(
        bronze_pairs,
        &state.bronze_stats_cache,
        ICEBERG_ENRICHMENT_BUDGET,
        ICEBERG_ENRICHMENT_MAX_CONCURRENT,
        &bronze_ns,
        || lakehouse_catalog::client(state),
    )
    .await;
    if by_slug.is_empty() {
        return;
    }
    if let Some(assets) = body.get_mut("assets").and_then(Value::as_array_mut) {
        apply_iceberg_enrichment(assets, &by_slug, now_millis());
    }
}

// WS1 task 1.9 — `sizeBytes` and `freshnessLagSeconds` are `null` and
// `health` is `"unknown"` on every row below. `sizeBytes` used to be
// `rows * 220`, an invented per-row byte constant (the same one already
// removed from `ops.rs`/`overview.rs`), which made "sort by size" the same
// as "sort by row count" while presenting itself as a real measurement.
// `freshnessLagSeconds` used to be a literal `0`, which `FreshnessIndicator`
// renders as "Fresh" for every asset regardless of actual staleness.
// `health` used to be derived from whether rows/columns came back, which
// measures neither freshness, failed loads, nor data quality. WS2 fills
// `sizeBytes` from Iceberg snapshot summaries (`total-files-size`) and
// `freshnessLagSeconds` from Iceberg snapshot timestamps — but only for
// Bronze rows whose `table_name` is confirmed present in Lakekeeper's
// `bronze` namespace listing, via `list`'s enrichment (see the module
// comment above `iceberg_candidates`); nothing yet measures asset health.

/// One Bronze/Iceberg asset row for the catalog list. `rows` and
/// `last_updated` are real (from the `dataset_sync`/`dataset_catalog`
/// registry); `size_bytes` and `freshness_lag_seconds` are `null` in this
/// row builder and filled in afterward by `list`'s Iceberg enrichment when
/// the row's `table_name` names a real Bronze Iceberg table; `health` is
/// not measured at all — see the module comment above.
fn bronze_catalog_row(
    slug: &str,
    title: &str,
    sekunder: bool,
    owner: &str,
    description: &str,
    rows: i64,
    col_count: i64,
    last_updated: &str,
) -> Value {
    json!({
        "id": slug,
        "name": title,
        "namespace": if sekunder { NAMESPACE_SEKUNDER } else { NAMESPACE_PRIMER },
        "type": "iceberg-table",
        "layer": if is_curated_bronze(slug) { "bronze" } else { "raw" },
        "tier": "warm",
        "classification": "internal",
        "owner": if owner.is_empty() { TENANT_OWNER.as_str() } else { owner },
        "domain": TENANT_DOMAIN.as_str(),
        "description": description,
        "format": "Apache Iceberg (Parquet)",
        "engine": "hot-store",
        "rows": rows,
        "sizeBytes": Value::Null,
        "columnCount": col_count,
        "freshnessLagSeconds": Value::Null,
        "lastUpdated": last_updated,
        "health": "unknown",
        "residency": TENANT_RESIDENCY.as_str(),
    })
}

/// One Silver/`ClickHouse` view/table row for the catalog list. Silver row
/// counts are not queried (an unfilled gap, not a measured zero), so `rows`
/// and `sizeBytes` are `null` rather than the literal zeros they used to be;
/// `lastUpdated` is `null` in place of the empty-string placeholder for
/// "unknown".
fn silver_catalog_row(name: &str, engine: &str, col_count: i64) -> Value {
    json!({
        "id": format!("silver.{name}"),
        "name": prettify(name),
        "namespace": "silver",
        "type": if engine == "View" { "view" } else { "table" },
        "layer": "silver",
        "tier": "warm",
        "classification": "internal",
        "owner": TENANT_OWNER.as_str(),
        "domain": TENANT_DOMAIN.as_str(),
        "description": "Model Silver terkurasi (bersih & terkonform) di ClickHouse.",
        "format": if engine == "View" { "ClickHouse View".to_owned() } else { format!("ClickHouse {engine}") },
        "engine": "hot-store",
        "rows": Value::Null,
        "sizeBytes": Value::Null,
        "columnCount": col_count,
        "freshnessLagSeconds": Value::Null,
        "lastUpdated": Value::Null,
        "health": "unknown",
        "residency": TENANT_RESIDENCY.as_str(),
    })
}

/// One Gold/`ClickHouse` mart row for the catalog list. `rows` is real (from
/// `system.parts`); `sizeBytes`, `freshnessLagSeconds`, and `health` are
/// not measured, and `lastUpdated` is `null` in place of the empty-string
/// placeholder for "unknown".
fn gold_catalog_row(name: &str, engine: &str, rows: i64, col_count: i64) -> Value {
    json!({
        "id": format!("serving.{name}"),
        "name": prettify(name),
        "namespace": "serving",
        "type": "table",
        "layer": "gold",
        "tier": "hot",
        "classification": "internal",
        "owner": TENANT_OWNER.as_str(),
        "domain": TENANT_DOMAIN.as_str(),
        "description": "Mart Gold penyaji dashboard (agregat siap pakai).",
        "format": format!("ClickHouse {engine}"),
        "engine": "hot-store",
        "rows": rows,
        "sizeBytes": Value::Null,
        "columnCount": col_count,
        "freshnessLagSeconds": Value::Null,
        "lastUpdated": Value::Null,
        "health": "unknown",
        "residency": TENANT_RESIDENCY.as_str(),
    })
}

/// Namespace metadata: display name and description. Overridable per
/// deployment via `CATALOG_NAMESPACE_META` — see `crate::tenant`.
/// Unlisted namespaces fall back to `(name, "")`.
fn ns_meta(name: &str) -> (String, String) {
    NAMESPACE_META
        .get(name)
        .map(|m| (m.name.clone(), m.description.clone()))
        .unwrap_or_default()
}

/// Group `assets` by `namespace`, counting each, in first-seen order —
/// mirroring JavaScript `Map` insertion-order iteration in
/// `catalog/route.ts`.
fn build_namespaces(assets: &[Value]) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for a in assets {
        let ns = a.get("namespace").and_then(Value::as_str).unwrap_or("");
        if !counts.contains_key(ns) {
            order.push(ns.to_owned());
        }
        *counts.entry(ns.to_owned()).or_insert(0) += 1;
    }
    order
        .into_iter()
        .map(|name| {
            let (meta_name, description) = ns_meta(&name);
            // Computed before `json!`, because `"id": name` moves `name`.
            let display = if meta_name.is_empty() {
                name.clone()
            } else {
                meta_name
            };
            let asset_count = counts[&name];
            json!({
                "id": name,
                "name": display,
                "description": description,
                "assetCount": asset_count,
                "owner": TENANT_OWNER.as_str(),
                "residency": TENANT_RESIDENCY.as_str(),
                "sourceEngine": "ClickHouse + Iceberg",
            })
        })
        .collect()
}

/// `GET /api/catalog/{id}` — one asset's metadata, schema, and a data
/// sample.
pub async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Response> {
    if id.starts_with("silver.") || id.starts_with("serving.") {
        return clickhouse_asset_detail(&state.clickhouse, &id).await;
    }
    bronze_asset_detail(&state.clickhouse, &id).await
}

/// `[db, table]` parsed from a `silver.*`/`serving.*` id, matching
/// `id.split(".")` → `[db, ...rest]`, `rest.join(".").replace(/[^a-zA-Z0-9_]/g,
/// "")` in `catalog/[id]/route.ts`. Note the replace strips *all*
/// non-word characters from `rest.join(".")`, including the dots the join
/// just inserted — so a multi-segment id like `silver.a.b` yields table
/// `"ab"`, not `"a.b"`. Reproduced here for fidelity, not because it's
/// intentional upstream.
fn split_db_table(id: &str) -> (String, String) {
    let mut parts = id.split('.');
    let db = parts.next().unwrap_or("").to_owned();
    let rest = parts.collect::<Vec<_>>().join(".");
    let table: String = rest
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (db, table)
}

async fn clickhouse_asset_detail(ch: &ChClient, id: &str) -> ApiResult<Response> {
    let (db, table) = split_db_table(id);
    if (db != "silver" && db != "serving") || table.is_empty() {
        return Err(not_found());
    }
    let is_gold = db == "serving";

    let query = format!("SELECT * FROM {db}.`{table}` LIMIT 5");
    let Ok(result) = ch.query(&query, None).await else {
        return Err(not_found());
    };
    let schema: Vec<Value> = result
        .meta
        .iter()
        .filter(|m| !m.name.starts_with('_'))
        .map(|m| json!({ "name": m.name, "dataType": m.ty }))
        .collect();
    let sample: Vec<Value> = result
        .data
        .iter()
        .map(|row| {
            let mut o = Map::new();
            for m in &result.meta {
                if !m.name.starts_with('_') {
                    o.insert(m.name.clone(), Value::String(js_string(row.get(&m.name))));
                }
            }
            Value::Object(o)
        })
        .collect();

    // `SELECT ... WHERE database='{db}' AND table='{table}' AND active` —
    // db/table are already constrained to a fixed set / alphanumeric-only
    // above, so this is safe despite the raw interpolation, matching the
    // TypeScript exactly.
    let rows_sql = format!(
        "SELECT toString(sum(rows)) r FROM system.parts WHERE database='{db}' AND table='{table}' AND active"
    );
    let rows = match ch.rows(&rows_sql, None).await {
        Ok(rr) => num_or_zero(rr.first(), "r"),
        Err(_) => 0, // "view: no parts" — swallowed in the TypeScript too.
    };

    let body = clickhouse_detail_body(id, &table, &db, is_gold, rows, &schema, &sample);
    Ok((StatusCode::OK, ApiJson(body)).into_response())
}

async fn bronze_asset_detail(ch: &ChClient, id: &str) -> ApiResult<Response> {
    match bronze_asset_detail_body(ch, id).await {
        Ok(Some(body)) => Ok((StatusCode::OK, ApiJson(body)).into_response()),
        Ok(None) => Err(not_found()),
        // `catch (e) { return NextResponse.json({ error: String(e) }, {
        // status: 503 }); }` in `catalog/[id]/route.ts`.
        Err(err) => Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response()),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one straight-line port of a single TS handler; splitting it up \
              would scatter the query→merge pipeline across helpers with no \
              independent reuse, hurting rather than helping readability of \
              the port"
)]
async fn bronze_asset_detail_body(ch: &ChClient, id: &str) -> Result<Option<Value>, ChError> {
    let escaped_id = SqlLiteral::from(id);
    let sync_sql = format!(
        "SELECT slug, title, description, tier, table_name, toString(total) total,
                author, frekuensi, satuan, klasifikasi, updated_at FROM (
           SELECT slug,title,description,'primer' tier,table_name,total,author,frekuensi,satuan,klasifikasi,'' updated_at FROM lake.`bronze_meta.dataset_sync` s
           UNION ALL
           SELECT slug,title,description,'sekunder' tier,table_name,total,author,frekuensi,satuan,klasifikasi,'' updated_at FROM lake.`bronze_meta_sec.dataset_sync`
         ) WHERE slug = {escaped_id} LIMIT 1"
    );
    let sync_rows = ch.rows(&sync_sql, None).await?;
    let Some(sync) = sync_rows.first() else {
        return Ok(None);
    };

    let cols_sql = format!(
        "SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta.dataset_column` WHERE slug={escaped_id}
       UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta_sec.dataset_column` WHERE slug={escaped_id}"
    );
    let cols = ch.rows(&cols_sql, None).await?;

    let table = str_col(sync, "table_name");
    let (sample, type_of): (Vec<Value>, Map<String, Value>) = match ch
        .query(&format!("SELECT * FROM silver.`{table}` LIMIT 5"), None)
        .await
    {
        Ok(r) => {
            let sample = r
                .data
                .iter()
                .map(|row| {
                    let mut o = Map::new();
                    for m in &r.meta {
                        if !m.name.starts_with('_') {
                            o.insert(m.name.clone(), Value::String(js_string(row.get(&m.name))));
                        }
                    }
                    Value::Object(o)
                })
                .collect();
            let mut type_of = Map::new();
            for m in &r.meta {
                type_of.insert(m.name.clone(), Value::String(m.ty.clone()));
            }
            (sample, type_of)
        }
        Err(_) => (Vec::new(), Map::new()), // "silver not yet available" — swallowed in the TypeScript too.
    };

    let schema: Vec<Value> = cols
        .iter()
        .map(|c| {
            let key_asli = str_col(c, "key_asli");
            let tipe = str_col(c, "tipe");
            let deskripsi = str_col(c, "deskripsi");
            // `typeOf.get(c.key_asli) ?? c.tipe ?? "String"` — nullish
            // coalescing, not a falsy check: an empty-but-present `tipe`
            // would NOT fall through to `"String"`. `tipe` is a `ClickHouse`
            // `String` column and is never actually null in practice (only
            // absent-from-`system.columns` triggers the missing-key case
            // handled by `type_of.get`), so the `"String"` literal fallback
            // is unreachable in real data and intentionally omitted here.
            let data_type = type_of
                .get(key_asli)
                .and_then(Value::as_str)
                .unwrap_or(tipe);
            let mut o = Map::new();
            o.insert("name".to_owned(), json!(key_asli));
            o.insert("dataType".to_owned(), json!(data_type));
            if !deskripsi.is_empty() {
                o.insert("description".to_owned(), json!(deskripsi));
            }
            Value::Object(o)
        })
        .collect();

    let rows = num_or_zero(Some(sync), "total");
    let sekunder = str_col(sync, "tier") == "sekunder";
    let slug = str_col(sync, "slug");
    let owner = str_col(sync, "author");
    let description = str_col(sync, "description");
    let updated_at = str_col(sync, "updated_at");

    let downstream = if sekunder {
        vec![]
    } else {
        vec![json!({ "id": format!("silver.{table}"), "name": format!("silver.{table}") })]
    };

    let col_count = cols.len();
    let body = bronze_detail_body(
        slug,
        str_col(sync, "title"),
        sekunder,
        owner,
        description,
        rows,
        col_count,
        updated_at,
        &schema,
        &sample,
        &downstream,
        str_col(sync, "frekuensi"),
        str_col(sync, "satuan"),
        str_col(sync, "klasifikasi"),
        table,
    );
    Ok(Some(body))
}

/// Detail body for a `silver.*`/`serving.*` asset. `rows` is real (from
/// `system.parts`); `sizeBytes`, `freshnessLagSeconds`, `health`, and `usage`
/// are not measured, `lastUpdated` is `null` in place of the empty-string
/// placeholder for "unknown", and `lifecyclePolicy` is no longer emitted
/// (it named a policy that exists nowhere) — see the WS1 task 1.9 comment
/// above `bronze_catalog_row`.
fn clickhouse_detail_body(
    id: &str,
    table: &str,
    db: &str,
    is_gold: bool,
    rows: i64,
    schema: &[Value],
    sample: &[Value],
) -> Value {
    json!({
        "id": id,
        "name": prettify(table),
        "namespace": db,
        "type": if is_gold { "table" } else { "view" },
        "layer": if is_gold { "gold" } else { "silver" },
        "tier": if is_gold { "hot" } else { "warm" },
        "classification": "internal",
        "owner": TENANT_OWNER.as_str(),
        "domain": TENANT_DOMAIN.as_str(),
        "description": if is_gold {
            "Mart Gold penyaji dashboard (agregat siap pakai)."
        } else {
            "Model Silver terkurasi (bersih & terkonform) di ClickHouse."
        },
        "format": if is_gold { "ClickHouse MergeTree" } else { "ClickHouse View" },
        "engine": "hot-store",
        "rows": rows,
        "sizeBytes": Value::Null,
        "columnCount": schema.len(),
        "freshnessLagSeconds": Value::Null,
        "lastUpdated": Value::Null,
        "health": "unknown",
        "residency": TENANT_RESIDENCY.as_str(),
        "schema": schema,
        "sample": sample,
        "qualityChecks": [],
        "policySummary": [],
        "usage": Value::Null,
        "recentQueries": [],
        "dependents": [],
        "changeHistory": [],
        "snapshots": [],
        "schemaVersions": [],
        "upstream": [],
        "downstream": [],
    })
}

/// Detail body for a Bronze/Iceberg asset. `rows` and `last_updated` are
/// real (from the `dataset_sync`/`dataset_catalog` registry); `sizeBytes`,
/// `freshnessLagSeconds`, `health`, and `usage` are not measured, and
/// `lifecyclePolicy` is no longer emitted — see the WS1 task 1.9 comment
/// above `bronze_catalog_row`.
#[allow(
    clippy::too_many_arguments,
    reason = "one-to-one port of the original inline `json!` body; every \
              argument is a distinct already-fetched value with no natural \
              grouping, so a wrapper struct would just move the same fields \
              one level up without reducing coupling"
)]
fn bronze_detail_body(
    slug: &str,
    title: &str,
    sekunder: bool,
    owner: &str,
    description: &str,
    rows: i64,
    col_count: usize,
    last_updated: &str,
    schema: &[Value],
    sample: &[Value],
    downstream: &[Value],
    frekuensi: &str,
    satuan: &str,
    klasifikasi: &str,
    table_name: &str,
) -> Value {
    json!({
        "id": slug,
        "name": title,
        "namespace": if sekunder { NAMESPACE_SEKUNDER } else { NAMESPACE_PRIMER },
        "type": "iceberg-table",
        "layer": if is_curated_bronze(slug) { "bronze" } else { "raw" },
        "tier": "warm",
        "classification": "internal",
        "owner": if owner.is_empty() { TENANT_OWNER.as_str() } else { owner },
        "domain": TENANT_DOMAIN.as_str(),
        "description": description,
        "format": "Apache Iceberg (Parquet)",
        "engine": "hot-store",
        "rows": rows,
        "sizeBytes": Value::Null,
        "columnCount": col_count,
        "freshnessLagSeconds": Value::Null,
        "lastUpdated": last_updated,
        "health": "unknown",
        "residency": TENANT_RESIDENCY.as_str(),
        "schema": schema,
        "sample": sample,
        "qualityChecks": [],
        "policySummary": [],
        "usage": Value::Null,
        "recentQueries": [],
        "dependents": [],
        "changeHistory": [],
        "snapshots": [],
        "schemaVersions": [],
        "upstream": [],
        "downstream": downstream,
        // The registry's own table_name — a true registry fact, not an
        // Iceberg-verified one (see the module comment above
        // `iceberg_candidates` for why this string may or may not name a
        // real Bronze Iceberg table). The frontend uses it to try loading
        // Iceberg snapshots for this asset, and handles a 404 quietly.
        "tableName": if table_name.is_empty() { Value::Null } else { json!(table_name) },
        "_meta": {
            "frekuensi": frekuensi,
            "satuan": satuan,
            "klasifikasi": klasifikasi,
        },
    })
}

fn not_found() -> ApiRejection {
    ApiError::NotFound("Asset not found".to_owned()).into()
}

// WS2 §13 — GET/PUT /api/catalog/{id}/annotation. See the module doc above
// for the bounds this validates and why the key is `asset_id`, not a
// table name.

/// Body accepted by `PUT /api/catalog/{id}/annotation`. Every field is
/// optional so a caller can send only what changed; a missing `tags` is
/// treated as an empty list, not "leave tags unchanged" — this is an
/// upsert-replace, matching `upsert_annotation`'s `ON CONFLICT DO UPDATE`.
#[derive(Debug, Deserialize)]
struct AnnotationBody {
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    steward: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

const ANNOTATION_ID_MAX_LEN: usize = 200;
const ANNOTATION_OWNER_MAX_LEN: usize = 128;
const ANNOTATION_STEWARD_MAX_LEN: usize = 128;
const ANNOTATION_DESCRIPTION_MAX_LEN: usize = 4000;
const ANNOTATION_TAGS_MAX_COUNT: usize = 20;
const ANNOTATION_TAG_MAX_LEN: usize = 64;

fn annotation_pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "annotation store unavailable: no Postgres pool is configured".to_owned(),
        )
    })
}

/// `true` if `tag` is 1-64 characters matching `^[a-z0-9][a-z0-9_-]*$` —
/// mirrored as `asset_annotation_tags_are_valid` in
/// `0032_asset_annotation.sql`.
fn tag_is_valid(tag: &str) -> bool {
    if tag.is_empty() || tag.len() > ANNOTATION_TAG_MAX_LEN {
        return false;
    }
    let mut chars = tag.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Bounds the `{id}` path segment. Returns `400` naming the field.
///
/// # Errors
/// [`ApiError::BadRequest`] if `id` exceeds [`ANNOTATION_ID_MAX_LEN`]
/// characters.
fn validate_annotation_id(id: &str) -> Result<(), ApiError> {
    if id.chars().count() > ANNOTATION_ID_MAX_LEN {
        return Err(ApiError::BadRequest(format!(
            "id must be at most {ANNOTATION_ID_MAX_LEN} characters"
        )));
    }
    Ok(())
}

/// Bounds every field of `body`. Returns `400` naming the offending field
/// — see the module doc above for the exact limits.
///
/// # Errors
/// [`ApiError::BadRequest`] on the first bound violated.
fn validate_annotation_body(body: &AnnotationBody) -> Result<(), ApiError> {
    if let Some(owner) = &body.owner
        && owner.chars().count() > ANNOTATION_OWNER_MAX_LEN
    {
        return Err(ApiError::BadRequest(format!(
            "owner must be at most {ANNOTATION_OWNER_MAX_LEN} characters"
        )));
    }
    if let Some(steward) = &body.steward
        && steward.chars().count() > ANNOTATION_STEWARD_MAX_LEN
    {
        return Err(ApiError::BadRequest(format!(
            "steward must be at most {ANNOTATION_STEWARD_MAX_LEN} characters"
        )));
    }
    if let Some(description) = &body.description
        && description.chars().count() > ANNOTATION_DESCRIPTION_MAX_LEN
    {
        return Err(ApiError::BadRequest(format!(
            "description must be at most {ANNOTATION_DESCRIPTION_MAX_LEN} characters"
        )));
    }
    if body.tags.len() > ANNOTATION_TAGS_MAX_COUNT {
        return Err(ApiError::BadRequest(format!(
            "tags must have at most {ANNOTATION_TAGS_MAX_COUNT} entries"
        )));
    }
    for tag in &body.tags {
        if !tag_is_valid(tag) {
            return Err(ApiError::BadRequest(format!(
                "tags: \"{tag}\" must be 1-{ANNOTATION_TAG_MAX_LEN} characters matching ^[a-z0-9][a-z0-9_-]*$"
            )));
        }
    }
    Ok(())
}

/// `GET /api/catalog/{id}/annotation` — one asset's console-only metadata.
/// An asset with no annotation row returns the empty shape rather than
/// `404`, matching how the rest of this module nulls out unmeasured
/// fields instead of failing.
///
/// # Errors
/// [`ApiError::Unavailable`] if no Postgres pool is configured; a
/// classified [`lakehouse_store::StoreError`] on any other database
/// failure.
pub async fn get_annotation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let pool = annotation_pool(&state)?;
    let row = lakehouse_store::annotation::get_annotation(pool, &id).await?;
    Ok(ApiJson(row.map_or_else(
        || {
            json!({
                "owner": Value::Null,
                "steward": Value::Null,
                "tags": Value::Array(vec![]),
                "description": Value::Null,
            })
        },
        |r| {
            json!({
                "owner": r.owner,
                "steward": r.steward,
                "tags": r.tags,
                "description": r.description,
            })
        },
    )))
}

/// `PUT /api/catalog/{id}/annotation` — create or replace one asset's
/// console-only metadata. Reuses the already-seeded `catalog:write`
/// permission (`0002_seed_identity.sql`'s Data Engineer role) — no new
/// permission string, no grant migration (WS2 plan review W8).
///
/// # Errors
/// `400` if the body is not JSON, or if `id`/`owner`/`steward`/
/// `description`/any `tags` entry exceeds its bound — see the module doc
/// above. [`ApiError::Unavailable`] if no Postgres pool is configured; a
/// classified [`lakehouse_store::StoreError`] on any other database
/// failure (including a `CHECK` constraint the validation above should
/// have already caught).
pub async fn put_annotation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    validate_annotation_id(&id)?;
    let parsed: AnnotationBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("body must be JSON".to_owned()))?;
    validate_annotation_body(&parsed)?;
    let pool = annotation_pool(&state)?;
    lakehouse_store::annotation::upsert_annotation(
        pool,
        &lakehouse_store::annotation::AnnotationInput {
            asset_id: id,
            owner: parsed.owner,
            steward: parsed.steward,
            tags: parsed.tags,
            description: parsed.description,
        },
    )
    .await?;
    Ok(ApiJson(json!({ "ok": true })))
}

// ── POST /api/catalog/{id}/access-request (WS7 item E2) ────────────────

/// `{permission, reason}` — the `POST /api/catalog/{id}/access-request`
/// body shape.
#[derive(Debug, Deserialize)]
struct AccessRequestBody {
    permission: String,
    #[serde(default)]
    reason: String,
}

/// `POST /api/catalog/{id}/access-request` — a principal who can already
/// SEE a catalog entry (`catalog:read`, held by the seeded Analyst role)
/// asks for a permission they do not already hold on it. Creates a
/// `pending`, `kind = "access"` `approval_item` (WS7 item E1's migration)
/// — the request itself carries no expiry; only the eventual GRANT does
/// (WS7 item E3).
///
/// # Errors
///
/// - `400` [`ApiError::BadRequest`] if the body is not JSON, `permission`
///   is not a real `resource:action`-shaped token (validated the same way
///   `identity::create_role`'s `PermissionSet::parse` already validates a
///   permission string's shape), or the CALLING principal already holds
///   `permission` — requesting a permission you already have is not a
///   pending request.
/// - `503` [`ApiError::Unavailable`] if no Postgres pool is configured; a
///   classified [`lakehouse_store::StoreError`] on any other database
///   failure.
pub async fn access_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<lakehouse_auth::Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: AccessRequestBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("body must be JSON".to_owned()))?;
    let permission = parsed.permission.trim();
    if lakehouse_auth::PermissionSet::parse(permission).is_empty() {
        return Err(ApiError::BadRequest(
            "permission must be a real \"resource:action\" token".to_owned(),
        )
        .into());
    }
    if principal.has(permission) {
        return Err(ApiError::BadRequest(
            "principal already holds the requested permission".to_owned(),
        )
        .into());
    }
    let pool = annotation_pool(&state)?;
    let req = lakehouse_store::agents::create_access_request(
        pool,
        &lakehouse_store::agents::NewAccessRequest {
            requested_by_user_id: principal.id.uuid(),
            catalog_id: &id,
            permission,
            reason: parsed.reason.trim(),
        },
    )
    .await?;
    // Best-effort, matching WS5 item D1's own posture: an audit-write
    // failure never fails the request that triggered it.
    let _ = lakehouse_store::audit::insert(
        pool,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: Some(principal.id.uuid().to_string()),
            principal_kind: Some("user".to_owned()),
            actor_label: Some(principal.display_name.clone()),
            action: "catalog.access_request".to_owned(),
            resource_kind: Some("catalog".to_owned()),
            resource_id: Some(id.clone()),
            outcome: "needs_approval".to_owned(),
            approval_id: Some(req.id.clone()),
            ..Default::default()
        },
    )
    .await;
    Ok(ApiJson(json!({
        "ok": true,
        "approvalId": req.id,
        "status": req.status,
    })))
}

// ── POST /api/catalog/access-requests/{id}/decide (WS7 item E3) ────────

/// `POST /api/catalog/access-requests/{id}/decide` — a dedicated,
/// `access:approve`-gated route that decides an access request. **Never**
/// reused for a `kind = "tool_call"` approval — see the module-level
/// discussion in the WS7 plan (M4/N1): `access:approve` (minted by WS7
/// item E1's migration, held only by Governance Admin) is a distinct
/// governance authority from `agent:approve`, and the two decisions stay
/// on two separate routes so `POLICY_TABLE`/`route_auth.rs`'s own
/// table-driven loop proves the right permission for each, rather than a
/// single fan-out handler whose in-handler `kind` branch a future edit
/// could silently skip.
///
/// `GRANT_DAYS`: how long an approved access request's [`AccessGrant`]
/// lasts. Fixed rather than caller-supplied (no field in the request or
/// decide body names it) — a requester/approver cannot negotiate their
/// own grant's lifetime through this route; the WS7 plan does not specify
/// a value, so this uses the same 30-day window its own store-layer test
/// (`lakehouse_store::agents::decide_access_request_approved_creates_a_time_bounded_grant`)
/// exercises.
const GRANT_DAYS: i64 = 30;

/// See [`DecideAccessRequestBody`]'s doc comment — reuses
/// `routes::agents::DecideApprovalBody`'s exact shape (WS7 item E3, N1),
/// never a redefined struct.
type DecideAccessRequestBody = crate::routes::agents::DecideApprovalBody;

/// # Errors
///
/// - `400` [`ApiError::BadRequest`] on an unparseable body or a `decision`
///   other than `"approved"`/`"rejected"`.
/// - `404` [`ApiError::NotFound`] if `id` is unknown, OR names a
///   `kind = "tool_call"` approval — this route can only decide `kind =
///   "access"` rows (see this function's own doc comment).
/// - `403` [`ApiError::PermissionDenied`] if the deciding principal is the SAME
///   person who requested this access — self-approval is refused, proven
///   by a real test (WS7 plan's own acceptance criterion for this task).
/// - `409` [`ApiError::Conflict`] if the request has already been decided.
/// - `503`/`500` as [`lakehouse_store::StoreError`] classifies.
pub async fn decide_access_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<lakehouse_auth::Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: DecideAccessRequestBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("body must be JSON".to_owned()))?;
    let decision = match body.decision.as_str() {
        "approved" => lakehouse_store::agents::Decision::Approved,
        "rejected" => lakehouse_store::agents::Decision::Rejected,
        other => {
            return Err(ApiError::BadRequest(format!(
                "decision must be \"approved\" or \"rejected\", got {other:?}"
            ))
            .into());
        }
    };
    let pool = annotation_pool(&state)?;

    // N1: 404, not 403 — see this function's own doc comment.
    let existing = lakehouse_store::agents::get_approval(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Access request {id} not found")))?;
    if existing.kind != "access" {
        return Err(ApiError::NotFound(format!("Access request {id} not found")).into());
    }
    // Self-approval refused: the deciding principal must not be the same
    // human who requested this access — the SERVER-SIDE enforcement of
    // Hard Requirement 3, not merely a client-side hint.
    if existing.requested_by_user_id == Some(principal.id.uuid()) {
        return Err(ApiError::PermissionDenied(
            "tidak boleh menyetujui permintaan akses milik sendiri".to_owned(),
        )
        .into());
    }

    let grant = match lakehouse_store::agents::decide_access_request(
        pool,
        &id,
        decision,
        body.comment.as_deref(),
        GRANT_DAYS,
    )
    .await
    {
        Ok(grant) => grant,
        Err(lakehouse_store::StoreError::NotFound) => {
            return Err(ApiError::NotFound(format!("Access request {id} not found")).into());
        }
        Err(lakehouse_store::StoreError::Conflict) => {
            return Err(ApiError::Conflict(format!(
                "Access request {id} has already been decided"
            ))
            .into());
        }
        Err(err) => return Err(ApiError::from(err).into()),
    };

    let decide_outcome = match decision {
        lakehouse_store::agents::Decision::Approved => "approved",
        lakehouse_store::agents::Decision::Rejected => "rejected",
    };
    // Best-effort, matching WS5 item D1's own posture.
    let _ = lakehouse_store::audit::insert(
        pool,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: Some(principal.id.uuid().to_string()),
            principal_kind: Some("user".to_owned()),
            actor_label: Some(principal.display_name.clone()),
            action: existing.action.clone(),
            resource_kind: Some("approval".to_owned()),
            resource_id: Some(id.clone()),
            args: Some(json!({ "comment": body.comment })),
            outcome: decide_outcome.to_owned(),
            approval_id: Some(id.clone()),
            ..Default::default()
        },
    )
    .await;

    Ok(ApiJson(json!({
        "ok": true,
        "status": decide_outcome,
        "grant": grant,
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn split_db_table_parses_silver_id() {
        assert_eq!(
            split_db_table("silver.mart_wisman"),
            ("silver".to_owned(), "mart_wisman".to_owned())
        );
    }

    #[test]
    fn split_db_table_strips_dots_from_multi_segment_rest() {
        // Reproduces the TS quirk: rest.join(".") re-inserts dots, then the
        // regex strips them again along with any other non-word char.
        assert_eq!(
            split_db_table("silver.a.b"),
            ("silver".to_owned(), "ab".to_owned())
        );
    }

    #[test]
    fn split_db_table_rejects_unknown_db() {
        let (db, table) = split_db_table("gold.mart_wisman");
        assert!(db != "silver" && db != "serving");
        assert_eq!(table, "mart_wisman");
    }

    #[test]
    fn split_db_table_empty_table_for_bare_db() {
        let (db, table) = split_db_table("silver");
        assert_eq!(db, "silver");
        assert!(table.is_empty());
    }

    #[test]
    fn split_db_table_strips_sql_metacharacters() {
        let (_, table) = split_db_table("silver.mart'; DROP TABLE x --");
        assert_eq!(table, "martDROPTABLEx");
    }

    #[test]
    fn filter_assets_by_query_matches_name_description_or_id_case_insensitively() {
        let assets = vec![
            json!({ "id": "bronze.orders", "name": "Orders", "description": "Order events" }),
            json!({ "id": "bronze.users", "name": "Users", "description": "Signup ledger" }),
        ];
        let filtered = filter_assets_by_query(&assets, "ORDER", &[]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["id"], json!("bronze.orders"));

        let by_id = filter_assets_by_query(&assets, "bronze.users", &[]);
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0]["id"], json!("bronze.users"));
    }

    #[test]
    fn filter_assets_by_query_matches_via_an_annotation_tag() {
        let assets = vec![
            json!({ "id": "bronze.orders", "name": "Orders", "description": "Order events" }),
            json!({ "id": "bronze.users", "name": "Users", "description": "Signup ledger" }),
        ];
        let annotations = vec![AnnotationRow {
            asset_id: "bronze.users".to_owned(),
            owner: None,
            steward: None,
            tags: vec!["pii".to_owned()],
            description: None,
        }];
        let filtered = filter_assets_by_query(&assets, "pii", &annotations);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["id"], json!("bronze.users"));
    }

    #[test]
    fn filter_assets_by_query_empty_q_returns_everything() {
        let assets = vec![
            json!({ "id": "bronze.orders", "name": "Orders", "description": "Order events" }),
            json!({ "id": "bronze.users", "name": "Users", "description": "Signup ledger" }),
        ];
        assert_eq!(filter_assets_by_query(&assets, "", &[]).len(), 2);
        assert_eq!(filter_assets_by_query(&assets, "   ", &[]).len(), 2);
    }

    #[test]
    fn filter_assets_by_query_matches_non_ascii_case_unicode_aware() {
        // `Ö`/`ö` are distinct bytes under `to_ascii_lowercase` (ASCII
        // lowercasing only touches `A`-`Z`, so a non-ASCII byte never
        // changes and the two never compare equal) but fold to the same
        // code point under `to_lowercase`, matching the browser's
        // Unicode-aware JavaScript `toLowerCase()`.
        let assets = vec![json!({
            "id": "bronze.orders",
            "name": "Orders",
            "description": "Umsatz nach Übersee"
        })];
        let filtered = filter_assets_by_query(&assets, "übersee", &[]);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn ns_meta_falls_back_to_empty_for_unknown_namespace() {
        assert_eq!(ns_meta("mystery"), (String::new(), String::new()));
    }

    #[test]
    fn ns_meta_known_namespace() {
        assert_eq!(ns_meta("silver").0, "Silver (kurasi)");
    }

    // WS1 task 1.9 — `sizeBytes` was `rows * 220` (an invented constant, the
    // same one already removed from `ops.rs`/`overview.rs`), `freshnessLagSeconds`
    // was a literal `0` (which `FreshnessIndicator` renders as "Fresh" for
    // every asset), and `health` was derived from row/column presence, which
    // measures neither freshness nor failed loads. WS2 owns real sizes
    // (`Iceberg` manifests) and freshness (`Iceberg` snapshot timestamps).
    #[test]
    fn catalog_rows_never_invent_size_freshness_or_health() {
        let bronze = bronze_catalog_row(
            "slug-1",
            "Title",
            false,
            "owner",
            "desc",
            42,
            3,
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(bronze["rows"], json!(42));
        assert_eq!(bronze["sizeBytes"], Value::Null);
        assert_eq!(bronze["freshnessLagSeconds"], Value::Null);
        assert_eq!(bronze["health"], json!("unknown"));
        assert_eq!(bronze["lastUpdated"], json!("2026-01-01T00:00:00Z"));

        let silver = silver_catalog_row("mart_wisman", "View", 5);
        assert_eq!(silver["rows"], Value::Null);
        assert_eq!(silver["sizeBytes"], Value::Null);
        assert_eq!(silver["freshnessLagSeconds"], Value::Null);
        assert_eq!(silver["health"], json!("unknown"));
        assert_eq!(silver["lastUpdated"], Value::Null);

        let gold = gold_catalog_row("mart_wisman", "MergeTree", 99, 5);
        assert_eq!(gold["rows"], json!(99));
        assert_eq!(gold["sizeBytes"], Value::Null);
        assert_eq!(gold["freshnessLagSeconds"], Value::Null);
        assert_eq!(gold["health"], json!("unknown"));
        assert_eq!(gold["lastUpdated"], Value::Null);
    }

    // WS1 task 1.9 — usage was three hardcoded zeros (nothing counts
    // per-asset queries or users) and `lifecyclePolicy` named a policy that
    // exists nowhere. Both detail bodies must stop asserting them.
    #[test]
    fn catalog_detail_reports_usage_as_unmeasured_and_no_lifecycle_policy() {
        let ch_detail = clickhouse_detail_body(
            "silver.mart_wisman",
            "mart_wisman",
            "silver",
            false,
            10,
            &[],
            &[],
        );
        assert_eq!(ch_detail["usage"], Value::Null);
        assert!(ch_detail.get("lifecyclePolicy").is_none());
        assert_eq!(ch_detail["sizeBytes"], Value::Null);
        assert_eq!(ch_detail["freshnessLagSeconds"], Value::Null);
        assert_eq!(ch_detail["health"], json!("unknown"));
        // Silver/Gold detail never emits tableName — only the Bronze
        // registry carries a table_name to report.
        assert!(ch_detail.get("tableName").is_none());

        let bronze_detail = bronze_detail_body(
            "slug-1",
            "Title",
            false,
            "owner",
            "desc",
            42,
            3,
            "2026-01-01T00:00:00Z",
            &[],
            &[],
            &[],
            "harian",
            "orang",
            "primer",
            "commerce_orders",
        );
        assert_eq!(bronze_detail["usage"], Value::Null);
        assert!(bronze_detail.get("lifecyclePolicy").is_none());
        assert_eq!(bronze_detail["sizeBytes"], Value::Null);
        assert_eq!(bronze_detail["freshnessLagSeconds"], Value::Null);
        assert_eq!(bronze_detail["health"], json!("unknown"));
        assert_eq!(bronze_detail["tableName"], json!("commerce_orders"));
    }

    // WS2 §4 — the Bronze asset detail exposes the registry's table_name so
    // the frontend can link an asset to its Iceberg table (Snapshots tab);
    // an empty registry value must render as `null`, never an empty string.
    #[test]
    fn bronze_detail_body_reports_null_table_name_when_registry_value_is_empty() {
        let bronze_detail = bronze_detail_body(
            "slug-1",
            "Title",
            false,
            "owner",
            "desc",
            1,
            0,
            "2026-01-01T00:00:00Z",
            &[],
            &[],
            &[],
            "harian",
            "orang",
            "primer",
            "",
        );
        assert_eq!(bronze_detail["tableName"], Value::Null);
    }

    #[test]
    fn build_namespaces_counts_and_preserves_first_seen_order() {
        let assets = vec![
            json!({"namespace": NAMESPACE_PRIMER}),
            json!({"namespace": "silver"}),
            json!({"namespace": NAMESPACE_PRIMER}),
        ];
        let namespaces = build_namespaces(&assets);
        assert_eq!(namespaces.len(), 2);
        assert_eq!(namespaces[0]["id"], NAMESPACE_PRIMER);
        assert_eq!(namespaces[0]["assetCount"], 2);
        assert_eq!(namespaces[1]["id"], "silver");
        assert_eq!(namespaces[1]["assetCount"], 1);
    }

    // WS2 §4 — `dataset_catalog.table_name` is written by two producers
    // with two different meanings (see the module comment above
    // `iceberg_candidates`); a pair must only survive when its
    // `table_name` is confirmed present in Lakekeeper's real `bronze`
    // listing.
    #[test]
    fn iceberg_candidates_excludes_a_seed_like_name_absent_from_the_listing() {
        let pairs = vec![("commerce-orders".to_owned(), "commerce_orders".to_owned())];
        let bronze_tables: HashSet<String> = HashSet::new();
        assert!(iceberg_candidates(&pairs, &bronze_tables).is_empty());
    }

    #[test]
    fn iceberg_candidates_includes_a_dlt_like_name_present_in_the_listing() {
        let pairs = vec![("orders".to_owned(), "orders".to_owned())];
        let bronze_tables: HashSet<String> = ["orders".to_owned()].into_iter().collect();
        assert_eq!(
            iceberg_candidates(&pairs, &bronze_tables),
            vec![("orders".to_owned(), "orders".to_owned())]
        );
    }

    #[test]
    fn iceberg_candidates_is_empty_when_the_listing_is_empty() {
        let pairs = vec![
            ("a".to_owned(), "a".to_owned()),
            ("b".to_owned(), "b".to_owned()),
        ];
        assert!(iceberg_candidates(&pairs, &HashSet::new()).is_empty());
    }

    fn bronze_row_with_id(slug: &str) -> Value {
        bronze_catalog_row(
            slug,
            "Title",
            false,
            "owner",
            "desc",
            1,
            1,
            "2026-01-01T00:00:00Z",
        )
    }

    #[test]
    fn apply_iceberg_enrichment_fills_a_matching_bronze_row() {
        let mut assets = vec![bronze_row_with_id("orders")];
        let mut by_slug = HashMap::new();
        by_slug.insert(
            "orders".to_owned(),
            CachedTableStats {
                total_bytes: Some(1_000),
                last_updated_ms: Some(500),
            },
        );
        // now_ms - last_updated_ms = 60_500 ms = 60s.
        apply_iceberg_enrichment(&mut assets, &by_slug, 61_000);
        assert_eq!(assets[0]["sizeBytes"], json!(1_000));
        assert_eq!(assets[0]["freshnessLagSeconds"], json!(60));
        assert_eq!(assets[0]["health"], json!("unknown"));
    }

    #[test]
    fn apply_iceberg_enrichment_leaves_a_non_bronze_row_untouched() {
        let mut assets = vec![silver_catalog_row("mart_wisman", "View", 1)];
        let mut by_slug = HashMap::new();
        by_slug.insert(
            "silver.mart_wisman".to_owned(),
            CachedTableStats {
                total_bytes: Some(1),
                last_updated_ms: Some(1),
            },
        );
        apply_iceberg_enrichment(&mut assets, &by_slug, 1_000);
        assert_eq!(assets[0]["sizeBytes"], Value::Null);
        assert_eq!(assets[0]["freshnessLagSeconds"], Value::Null);
    }

    #[test]
    fn apply_iceberg_enrichment_null_size_when_total_bytes_is_none() {
        let mut assets = vec![bronze_row_with_id("orders")];
        let mut by_slug = HashMap::new();
        by_slug.insert(
            "orders".to_owned(),
            CachedTableStats {
                total_bytes: None,
                last_updated_ms: Some(1),
            },
        );
        apply_iceberg_enrichment(&mut assets, &by_slug, 1_000);
        assert_eq!(assets[0]["sizeBytes"], Value::Null);
    }

    #[test]
    fn apply_iceberg_enrichment_null_lag_for_a_future_timestamp() {
        let mut assets = vec![bronze_row_with_id("orders")];
        let mut by_slug = HashMap::new();
        by_slug.insert(
            "orders".to_owned(),
            CachedTableStats {
                total_bytes: Some(1),
                // last_updated_ms is AFTER now_ms — clock skew, not a
                // negative-but-real lag.
                last_updated_ms: Some(2_000),
            },
        );
        apply_iceberg_enrichment(&mut assets, &by_slug, 1_000);
        assert_eq!(
            assets[0]["freshnessLagSeconds"],
            Value::Null,
            "a future last_updated_ms must never render as a negative lag"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn enrich_bronze_stats_leaves_the_list_unchanged_when_the_connect_never_resolves() {
        let cache = BronzeStatsCache::new();
        let bronze_ns = NamespaceIdent::from_strs(["bronze"]).expect("valid namespace");
        let pairs = vec![("orders".to_owned(), "orders".to_owned())];

        let result = enrich_bronze_stats(
            pairs,
            &cache,
            Duration::from_millis(100),
            8,
            &bronze_ns,
            // Never resolves — proves the timeout, not the connector, is
            // what bounds this function. No live Lakekeeper is reachable
            // from a unit test, so the type is named without ever
            // constructing a real value.
            std::future::pending::<Result<Arc<IcebergClient>, CatalogAccessError>>,
        )
        .await;

        assert!(
            result.is_empty(),
            "a connect that never resolves must leave enrichment empty, not hang"
        );
    }

    // WS2 §13 — annotation bounds. Pure-function tests, matching this
    // module's own idiom of unit-testing the extracted validation/JSON
    // builders directly rather than a full `AppState`.

    fn valid_body() -> AnnotationBody {
        AnnotationBody {
            owner: Some("data-eng".to_owned()),
            steward: Some("bayu".to_owned()),
            tags: vec!["pii".to_owned()],
            description: Some("Order events".to_owned()),
        }
    }

    #[test]
    fn validate_annotation_id_accepts_a_bronze_slug_and_a_silver_prefixed_id() {
        assert!(validate_annotation_id("commerce_orders").is_ok());
        assert!(validate_annotation_id("silver.mart_orders").is_ok());
    }

    #[test]
    fn validate_annotation_id_rejects_over_200_chars() {
        let id = "a".repeat(201);
        let err = validate_annotation_id(&id).expect_err("must reject");
        assert!(matches!(err, ApiError::BadRequest(msg) if msg.contains("id")));
    }

    #[test]
    fn validate_annotation_body_accepts_a_fully_populated_body() {
        assert!(validate_annotation_body(&valid_body()).is_ok());
    }

    #[test]
    fn validate_annotation_body_accepts_every_field_absent() {
        let body = AnnotationBody {
            owner: None,
            steward: None,
            tags: vec![],
            description: None,
        };
        assert!(validate_annotation_body(&body).is_ok());
    }

    #[test]
    fn validate_annotation_body_rejects_owner_over_128_chars() {
        let mut body = valid_body();
        body.owner = Some("o".repeat(129));
        let err = validate_annotation_body(&body).expect_err("must reject");
        assert!(matches!(err, ApiError::BadRequest(msg) if msg.contains("owner")));
    }

    #[test]
    fn validate_annotation_body_rejects_steward_over_128_chars() {
        let mut body = valid_body();
        body.steward = Some("s".repeat(129));
        let err = validate_annotation_body(&body).expect_err("must reject");
        assert!(matches!(err, ApiError::BadRequest(msg) if msg.contains("steward")));
    }

    #[test]
    fn validate_annotation_body_rejects_description_over_4000_chars() {
        let mut body = valid_body();
        body.description = Some("d".repeat(4001));
        let err = validate_annotation_body(&body).expect_err("must reject");
        assert!(matches!(err, ApiError::BadRequest(msg) if msg.contains("description")));
    }

    #[test]
    fn validate_annotation_body_rejects_more_than_20_tags() {
        let mut body = valid_body();
        body.tags = (0..21).map(|i| format!("tag-{i}")).collect();
        let err = validate_annotation_body(&body).expect_err("must reject");
        assert!(matches!(err, ApiError::BadRequest(msg) if msg.contains("tags")));
    }

    #[test]
    fn validate_annotation_body_accepts_exactly_20_tags() {
        let mut body = valid_body();
        body.tags = (0..20).map(|i| format!("tag-{i}")).collect();
        assert!(validate_annotation_body(&body).is_ok());
    }

    #[test]
    fn validate_annotation_body_rejects_a_tag_over_64_chars() {
        let mut body = valid_body();
        body.tags = vec!["a".repeat(65)];
        let err = validate_annotation_body(&body).expect_err("must reject");
        assert!(matches!(err, ApiError::BadRequest(msg) if msg.contains("tags")));
    }

    #[test]
    fn validate_annotation_body_rejects_an_uppercase_tag() {
        let mut body = valid_body();
        body.tags = vec!["PII".to_owned()];
        assert!(validate_annotation_body(&body).is_err());
    }

    #[test]
    fn validate_annotation_body_rejects_a_tag_starting_with_a_hyphen() {
        let mut body = valid_body();
        body.tags = vec!["-pii".to_owned()];
        assert!(validate_annotation_body(&body).is_err());
    }

    #[test]
    fn validate_annotation_body_accepts_a_single_char_tag() {
        let mut body = valid_body();
        body.tags = vec!["a".to_owned()];
        assert!(validate_annotation_body(&body).is_ok());
    }

    #[test]
    fn put_annotation_rejects_a_non_json_body_in_english() {
        let err = serde_json::from_slice::<AnnotationBody>(b"not json")
            .map_err(|_| ApiError::BadRequest("body must be JSON".to_owned()))
            .expect_err("must reject");
        assert!(matches!(err, ApiError::BadRequest(msg) if msg == "body must be JSON"));
    }
}
