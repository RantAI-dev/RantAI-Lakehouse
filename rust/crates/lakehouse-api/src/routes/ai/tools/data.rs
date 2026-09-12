//! Read-only lakehouse-data tools: `run_sql`, `list_datasets`,
//! `describe_dataset`, `get_lineage`, `get_quality`, `describe_mart`.
//! Moved out of `ai.rs` unchanged (T0.1 registry refactor).

use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::SqlLiteral;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::routes::support::{is_numeric_type, strip_non_ident};

const CATALOG_UNION: &str = "(SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta.dataset_catalog` \
     UNION ALL SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta_sec.dataset_catalog`)";

pub(super) async fn run_sql(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let sql = arg_str(args, "sql");
    if !crate::routes::agent::is_read_only_sql(&sql) {
        return json!({ "error": "Hanya SELECT diizinkan." });
    }
    match ch.query(&sql, None).await {
        Ok(r) => {
            let columns: Vec<String> = r.meta.iter().map(|m| m.name.clone()).collect();
            let rows: Vec<Value> = r
                .data
                .iter()
                .take(50)
                .map(|row| Value::Object(row.clone()))
                .collect();
            json!({ "columns": columns, "rows": rows, "rowCount": r.rows })
        }
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn list_datasets(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let rows = match ch
        .rows(
            &format!("SELECT slug, title, tier FROM {CATALOG_UNION} LIMIT 500"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let term = arg_str(args, "search").to_lowercase();
    let tier = arg_str(args, "tier");
    let hits: Vec<&Map<String, Value>> = rows
        .iter()
        .filter(|r| {
            let row_tier = r.get("tier").and_then(Value::as_str).unwrap_or("");
            if !tier.is_empty() && row_tier != tier {
                return false;
            }
            if term.is_empty() {
                return true;
            }
            let title = r.get("title").and_then(Value::as_str).unwrap_or("");
            let slug = r.get("slug").and_then(Value::as_str).unwrap_or("");
            format!("{title} {slug}").to_lowercase().contains(&term)
        })
        .take(40)
        .collect();
    json!({ "total": hits.len(), "datasets": hits })
}

pub(super) async fn describe_dataset(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let slug = SqlLiteral::from(arg_str(args, "slug"));
    let meta_rows = match ch
        .rows(
            &format!(
                "SELECT title, table_name, tier FROM {CATALOG_UNION} WHERE slug={slug} LIMIT 1"
            ),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let Some(meta) = meta_rows.first() else {
        return json!({ "error": "dataset tidak ditemukan" });
    };
    let table = meta.get("table_name").and_then(Value::as_str).unwrap_or("");
    let cols = match ch
        .rows(
            &format!(
                "SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta.dataset_column` WHERE slug={slug} \
                 UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta_sec.dataset_column` WHERE slug={slug}"
            ),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let rows = ch
        .rows(
            &format!("SELECT toString(count()) n FROM silver.`{table}`"),
            None,
        )
        .await
        .ok()
        .and_then(|r| {
            r.first()
                .and_then(|row| row.get("n").and_then(Value::as_str))
                .and_then(|s| s.parse::<i64>().ok())
        })
        .unwrap_or(0);
    json!({
        "title": meta.get("title"),
        "tier": meta.get("tier"),
        "table": table,
        "rows": rows,
        "columns": cols,
    })
}

pub(super) async fn get_lineage(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let slug = SqlLiteral::from(arg_str(args, "slug"));
    let meta_rows = match ch
        .rows(
            &format!("SELECT table_name, tier FROM {CATALOG_UNION} WHERE slug={slug} LIMIT 1"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let Some(meta) = meta_rows.first() else {
        return json!({ "error": "dataset tidak ditemukan" });
    };
    let table = meta
        .get("table_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let sekunder = meta.get("tier").and_then(Value::as_str) == Some("sekunder");
    let escaped_table = SqlLiteral::from(table.as_str());
    let cols = match ch
        .rows(
            &format!("SELECT kolom, tipe FROM _silver_meta.kolom_tipe WHERE tabel={escaped_table} LIMIT 100"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let source_label = if sekunder {
        "Sumber sekunder"
    } else {
        "Satu Data Jakarta"
    };
    let mappings: Vec<String> = cols
        .iter()
        .map(|c| {
            format!(
                "{} → {}",
                c.get("kolom").and_then(Value::as_str).unwrap_or(""),
                c.get("tipe").and_then(Value::as_str).unwrap_or("")
            )
        })
        .collect();
    json!({
        "chain": format!("{source_label} → bronze.{table} → silver.{table}"),
        "columnMappings": mappings,
    })
}

pub(super) async fn get_quality(ch: &ChClient) -> Value {
    match ch
        .rows(
            "SELECT verdict, toString(count()) n FROM ( \
               SELECT tabel, cek, argMax(verdict, dibuat_pada) verdict FROM _silver_meta.quality GROUP BY tabel, cek \
             ) GROUP BY verdict",
            None,
        )
        .await
    {
        Ok(rows) => json!({ "summary": rows }),
        Err(err) => json!({ "error": format!("quality belum tersedia: {err}") }),
    }
}

pub(super) async fn describe_mart(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let mart = strip_non_ident(&arg_str(args, "mart"));
    if mart.is_empty() {
        let rows = match ch
            .rows(
                "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
                 WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
                None,
            )
            .await
        {
            Ok(r) => r,
            Err(err) => return json!({ "error": err.to_string() }),
        };
        let marts: Vec<Value> = rows
            .iter()
            .map(|r| {
                let rows_n: i64 = r
                    .get("total_rows")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                json!({ "mart": r.get("name"), "rows": rows_n })
            })
            .collect();
        return json!({ "marts": marts });
    }
    let cols = match ch
        .rows(
            &format!("SELECT name, type FROM system.columns WHERE database='serving' AND table='{mart}' ORDER BY position"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    if cols.is_empty() {
        return json!({ "error": format!("mart '{mart}' tidak ditemukan di serving.") });
    }
    let dimensions: Vec<&str> = cols
        .iter()
        .filter(|c| !is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();
    let measures: Vec<&str> = cols
        .iter()
        .filter(|c| is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();
    json!({ "mart": mart, "dimensions": dimensions, "measures": measures })
}
