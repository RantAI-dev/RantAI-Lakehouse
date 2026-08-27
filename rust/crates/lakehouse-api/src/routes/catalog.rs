//! `GET /api/catalog`, `GET /api/catalog/{id}` — the dataset registry.
//!
//! Ports `src/app/api/catalog/route.ts` and
//! `src/app/api/catalog/[id]/route.ts`. Bronze/raw assets come from the
//! `bronze_meta`/`bronze_meta_sec` registry tables in `lake`; Silver/Gold
//! assets are read directly off `ClickHouse`'s `system.tables` /
//! `system.columns` / `system.parts`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde_json::{Map, Value, json};

use crate::error::{ApiRejection, ApiResult};
use crate::routes::support::{js_error, js_string, num_or_zero, prettify, str_col};
use crate::state::AppState;

/// Dataset slugs whose Bronze registry entry is curated (SDI-derived and
/// promoted a layer), everything else in the registry is raw. Ported
/// verbatim from the `BRONZE_CURATED` set in both TypeScript route files.
const BRONZE_CURATED: &[&str] = &[
    "wisman-jakarta-per-bulan",
    "wisman-jakarta-per-negara",
    "wisman-jakarta-per-pintu-masuk",
    "jumlah-pengunjung-event-2026",
];

const DEFAULT_OWNER: &str = "Dinas Pariwisata & Ekraf DKI Jakarta";

/// `GET /api/catalog` — the full asset registry, grouped into namespaces.
pub async fn list(State(state): State<AppState>) -> Response {
    match list_body(&state.clickhouse).await {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        // `catch (e) { return NextResponse.json({ error: String(e), assets:
        // [], namespaces: [] }, { status: 503 }); }` in `catalog/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": js_error(err), "assets": [], "namespaces": [] })),
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
            json!({
                "id": slug,
                "name": str_col(c, "title"),
                "namespace": if sekunder { "sekunder" } else { "sdi-primer" },
                "type": "iceberg-table",
                "layer": if BRONZE_CURATED.contains(&slug) { "bronze" } else { "raw" },
                "tier": "warm",
                "classification": "internal",
                "owner": if owner.is_empty() { DEFAULT_OWNER } else { owner.as_str() },
                "domain": "pariwisata",
                "description": description,
                "format": "Apache Iceberg (Parquet)",
                "engine": "hot-store",
                "rows": rows,
                "sizeBytes": rows * 220,
                "columnCount": col_of(slug),
                "freshnessLagSeconds": 0,
                "lastUpdated": updated_at,
                "health": if rows > 0 { "healthy" } else { "degraded" },
                "residency": "id-jakarta",
            })
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
            assets.push(json!({
                "id": format!("silver.{name}"),
                "name": prettify(name),
                "namespace": "silver",
                "type": if engine == "View" { "view" } else { "table" },
                "layer": "silver",
                "tier": "warm",
                "classification": "internal",
                "owner": DEFAULT_OWNER,
                "domain": "pariwisata",
                "description": "Model Silver terkurasi (bersih & terkonform) di ClickHouse.",
                "format": if engine == "View" { "ClickHouse View".to_owned() } else { format!("ClickHouse {engine}") },
                "engine": "hot-store",
                "rows": 0,
                "sizeBytes": 0,
                "columnCount": col_count_of("silver", name),
                "freshnessLagSeconds": 0,
                "lastUpdated": "",
                "health": "healthy",
                "residency": "id-jakarta",
            }));
        } else {
            if name.ends_with("_baru") {
                continue;
            }
            let rows = gold_rows_of(name);
            assets.push(json!({
                "id": format!("serving.{name}"),
                "name": prettify(name),
                "namespace": "serving",
                "type": "table",
                "layer": "gold",
                "tier": "hot",
                "classification": "internal",
                "owner": DEFAULT_OWNER,
                "domain": "pariwisata",
                "description": "Mart Gold penyaji dashboard (agregat siap pakai).",
                "format": format!("ClickHouse {engine}"),
                "engine": "hot-store",
                "rows": rows,
                "sizeBytes": rows * 220,
                "columnCount": col_count_of("serving", name),
                "freshnessLagSeconds": 0,
                "lastUpdated": "",
                "health": if rows > 0 { "healthy" } else { "degraded" },
                "residency": "id-jakarta",
            }));
        }
    }

    let namespaces = build_namespaces(&assets);
    Ok(json!({ "assets": assets, "namespaces": namespaces }))
}

/// Namespace metadata: display name and description. Ported from `NS_META`
/// in `catalog/route.ts`; unlisted namespaces fall back to `(name, "")`.
fn ns_meta(name: &str) -> (&'static str, &'static str) {
    match name {
        "sdi-primer" => (
            "SDI Primer (Satu Data Jakarta)",
            "Dataset primer ditarik dari Satu Data Jakarta ke Bronze/Iceberg.",
        ),
        "sekunder" => (
            "Data Sekunder (olahan)",
            "Dataset sekunder olahan (wisman bersih, TripAdvisor, halal, dll).",
        ),
        "silver" => (
            "Silver (kurasi)",
            "Model bersih & terkonform di ClickHouse — dimensi, wisman, restoran, event, dst.",
        ),
        "serving" => (
            "Gold (mart penyaji)",
            "Mart agregat penyaji dashboard — mart_wisman, mart_kuliner, mart_event, dll.",
        ),
        _ => ("", ""),
    }
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
            json!({
                "id": name,
                "name": if meta_name.is_empty() { name.clone() } else { meta_name.to_owned() },
                "description": description,
                "assetCount": counts[&name],
                "owner": DEFAULT_OWNER,
                "residency": "id-jakarta",
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

    let body = json!({
        "id": id,
        "name": prettify(&table),
        "namespace": db,
        "type": if is_gold { "table" } else { "view" },
        "layer": if is_gold { "gold" } else { "silver" },
        "tier": if is_gold { "hot" } else { "warm" },
        "classification": "internal",
        "owner": DEFAULT_OWNER,
        "domain": "pariwisata",
        "description": if is_gold {
            "Mart Gold penyaji dashboard (agregat siap pakai)."
        } else {
            "Model Silver terkurasi (bersih & terkonform) di ClickHouse."
        },
        "format": if is_gold { "ClickHouse MergeTree" } else { "ClickHouse View" },
        "engine": "hot-store",
        "rows": rows,
        "sizeBytes": rows * 220,
        "columnCount": schema.len(),
        "freshnessLagSeconds": 0,
        "lastUpdated": "",
        "health": if schema.is_empty() { "degraded" } else { "healthy" },
        "residency": "id-jakarta",
        "schema": schema,
        "sample": sample,
        "qualityChecks": [],
        "policySummary": [],
        "usage": { "queries7d": 0, "users7d": 0, "avgLatencyMs": 0 },
        "recentQueries": [],
        "dependents": [],
        "changeHistory": [],
        "snapshots": [],
        "schemaVersions": [],
        "upstream": [],
        "downstream": [],
        "lifecyclePolicy": "default",
    });
    Ok((StatusCode::OK, Json(body)).into_response())
}

async fn bronze_asset_detail(ch: &ChClient, id: &str) -> ApiResult<Response> {
    match bronze_asset_detail_body(ch, id).await {
        Ok(Some(body)) => Ok((StatusCode::OK, Json(body)).into_response()),
        Ok(None) => Err(not_found()),
        // `catch (e) { return NextResponse.json({ error: String(e) }, {
        // status: 503 }); }` in `catalog/[id]/route.ts`.
        Err(err) => Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": js_error(err) })),
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
    let escaped_id = id.replace('\'', "");
    let sync_sql = format!(
        "SELECT slug, title, description, tier, table_name, toString(total) total,
                author, frekuensi, satuan, klasifikasi, updated_at FROM (
           SELECT slug,title,description,'primer' tier,table_name,total,author,frekuensi,satuan,klasifikasi,'' updated_at FROM lake.`bronze_meta.dataset_sync` s
           UNION ALL
           SELECT slug,title,description,'sekunder' tier,table_name,total,author,frekuensi,satuan,klasifikasi,'' updated_at FROM lake.`bronze_meta_sec.dataset_sync`
         ) WHERE slug = '{escaped_id}' LIMIT 1"
    );
    let sync_rows = ch.rows(&sync_sql, None).await?;
    let Some(sync) = sync_rows.first() else {
        return Ok(None);
    };

    let cols_sql = format!(
        "SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta.dataset_column` WHERE slug='{escaped_id}'
       UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta_sec.dataset_column` WHERE slug='{escaped_id}'"
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

    let body = json!({
        "id": slug,
        "name": str_col(sync, "title"),
        "namespace": if sekunder { "sekunder" } else { "sdi-primer" },
        "type": "iceberg-table",
        "layer": if BRONZE_CURATED.contains(&slug) { "bronze" } else { "raw" },
        "tier": "warm",
        "classification": "internal",
        "owner": if owner.is_empty() { DEFAULT_OWNER } else { owner },
        "domain": "pariwisata",
        "description": description,
        "format": "Apache Iceberg (Parquet)",
        "engine": "hot-store",
        "rows": rows,
        "sizeBytes": rows * 220,
        "columnCount": cols.len(),
        "freshnessLagSeconds": 0,
        "lastUpdated": updated_at,
        "health": if rows > 0 { "healthy" } else { "degraded" },
        "residency": "id-jakarta",
        "schema": schema,
        "sample": sample,
        "qualityChecks": [],
        "policySummary": [],
        "usage": { "queries7d": 0, "users7d": 0, "avgLatencyMs": 0 },
        "recentQueries": [],
        "dependents": [],
        "changeHistory": [],
        "snapshots": [],
        "schemaVersions": [],
        "upstream": [],
        "downstream": downstream,
        "lifecyclePolicy": "default",
        "_meta": {
            "frekuensi": str_col(sync, "frekuensi"),
            "satuan": str_col(sync, "satuan"),
            "klasifikasi": str_col(sync, "klasifikasi"),
        },
    });
    Ok(Some(body))
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
        assert_eq!(ns_meta("mystery"), ("", ""));
    }

    #[test]
    fn ns_meta_known_namespace() {
        assert_eq!(ns_meta("silver").0, "Silver (kurasi)");
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
