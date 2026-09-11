//! `GET /api/catalog`, `GET /api/catalog/{id}` — the dataset registry.
//!
//! Ports `src/app/api/catalog/route.ts` and
//! `src/app/api/catalog/[id]/route.ts`. Bronze/raw assets come from the
//! `bronze_meta`/`bronze_meta_sec` registry tables in `lake`; Silver/Gold
//! assets are read directly off `ClickHouse`'s `system.tables` /
//! `system.columns` / `system.parts`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_core::ident::SqlLiteral;
use serde_json::{Map, Value, json};

use crate::error::{ApiRejection, ApiResult};
use crate::json::ApiJson;
use crate::routes::support::{js_error, js_string, num_or_zero, prettify, str_col};
use crate::state::AppState;

use crate::tenant::{
    NAMESPACE_META, TENANT_DOMAIN, TENANT_OWNER, TENANT_RESIDENCY, is_curated_bronze,
};

/// `GET /api/catalog` — the full asset registry, grouped into namespaces.
pub async fn list(State(state): State<AppState>) -> Response {
    match list_body(&state.clickhouse).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        // `catch (e) { return NextResponse.json({ error: String(e), assets:
        // [], namespaces: [] }, { status: 503 }); }` in `catalog/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err), "assets": [], "namespaces": [] })),
        )
            .into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one straight-line port of a single TS handler; splitting it up \
              would scatter the query→merge→group pipeline across helpers \
              with no independent reuse, hurting rather than helping \
              readability of the port"
)]
async fn list_body(ch: &ChClient) -> Result<Value, ChError> {
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

    let bronze_table_names: std::collections::HashSet<&str> =
        cat.iter().map(|c| str_col(c, "table_name")).collect();

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
    Ok(json!({ "assets": assets, "namespaces": namespaces }))
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
// real sizes from `Iceberg` manifests and freshness from `Iceberg` snapshot
// timestamps; nothing yet measures asset health.

/// One Bronze/Iceberg asset row for the catalog list. `rows` and
/// `last_updated` are real (from the `dataset_sync`/`dataset_catalog`
/// registry); `size_bytes`, `freshness_lag_seconds`, and `health` are not
/// measured — see the module comment above.
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
        "namespace": if sekunder { "sekunder" } else { "sdi-primer" },
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
            // Dihitung sebelum `json!`, karena `"id": name` memindahkan `name`.
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
        Err(_) => 0, // "view: tak ada parts" — swallowed in the TypeScript too.
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
        Err(_) => (Vec::new(), Map::new()), // "silver belum ada" — swallowed in the TypeScript too.
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
) -> Value {
    json!({
        "id": slug,
        "name": title,
        "namespace": if sekunder { "sekunder" } else { "sdi-primer" },
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
        "_meta": {
            "frekuensi": frekuensi,
            "satuan": satuan,
            "klasifikasi": klasifikasi,
        },
    })
}

fn not_found() -> ApiRejection {
    ApiError::NotFound("Aset tidak ditemukan".to_owned()).into()
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
        );
        assert_eq!(bronze_detail["usage"], Value::Null);
        assert!(bronze_detail.get("lifecyclePolicy").is_none());
        assert_eq!(bronze_detail["sizeBytes"], Value::Null);
        assert_eq!(bronze_detail["freshnessLagSeconds"], Value::Null);
        assert_eq!(bronze_detail["health"], json!("unknown"));
    }

    #[test]
    fn build_namespaces_counts_and_preserves_first_seen_order() {
        let assets = vec![
            json!({"namespace": "sdi-primer"}),
            json!({"namespace": "silver"}),
            json!({"namespace": "sdi-primer"}),
        ];
        let namespaces = build_namespaces(&assets);
        assert_eq!(namespaces.len(), 2);
        assert_eq!(namespaces[0]["id"], "sdi-primer");
        assert_eq!(namespaces[0]["assetCount"], 2);
        assert_eq!(namespaces[1]["id"], "silver");
        assert_eq!(namespaces[1]["assetCount"], 1);
    }
}
