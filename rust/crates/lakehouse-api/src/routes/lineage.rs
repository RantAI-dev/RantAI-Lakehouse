//! Recorded lineage: the graph behind `GET /api/governance/lineage` and the
//! copilot's `get_lineage` tool.
//!
//! # Only edges the platform itself recorded
//!
//! `WS1` item 1.5 removed an earlier lineage view because it drew arrows
//! and transforms nobody had captured, and a lineage graph is exactly the
//! surface a reader takes as authoritative. This module draws an edge only
//! where the platform holds a record of it, and every edge names that
//! record in its `evidence`:
//!
//! | Edge | Record |
//! | --- | --- |
//! | publisher -> dataset | `bronze_meta.dataset_sync.author` |
//! | dataset -> Gold table | `bronze_meta.dataset_catalog.table_name` (the catalog registry) |
//! | connector -> Bronze table | the connector's ingest spec (`source_objects[].target`) |
//! | source -> target of an authored pipeline | `pipeline_definition.source`/`target` |
//! | table -> view | the view's own `SELECT` in `system.tables.as_select` |
//! | Gold table -> Gold Iceberg table | a successful row in `console.gold_export_run` |
//!
//! What the platform does not record is not drawn: a Silver or Gold table
//! loaded by a job outside the platform has no edge into it, and the
//! response says so in `coverage` rather than guessing from table names.
//!
//! # Tenancy
//!
//! Connectors and authored pipelines are read for the caller's own tenant
//! only (`tenant_scope::resolve`), exactly like their list routes. The
//! catalog, the `ClickHouse` tables and the Gold export history are one per
//! deployment with no tenant column; they are included only when
//! `catalog::catalog_tenant_refusal` allows this caller to read the shared
//! catalog, the same single rule the catalog route applies.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use serde_json::{Map, Value, json};

use crate::state::AppState;

/// Most nodes one response carries, so a focus on a hub table cannot
/// return the whole warehouse.
const MAX_NODES: usize = 150;

#[derive(Default)]
struct Graph {
    nodes: BTreeMap<String, (String, &'static str)>,
    edges: Vec<(String, String, &'static str, String)>,
    coverage: Vec<String>,
}

impl Graph {
    fn node(&mut self, id: &str, label: &str, kind: &'static str) {
        self.nodes
            .entry(id.to_owned())
            .or_insert_with(|| (label.to_owned(), kind));
    }

    fn edge(&mut self, from: &str, to: &str, kind: &'static str, evidence: &str) {
        if from == to {
            return;
        }
        self.edges
            .push((from.to_owned(), to.to_owned(), kind, evidence.to_owned()));
    }
}

fn text(row: &Map<String, Value>, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The node id of a `ClickHouse` table.
fn table_id(db: &str, table: &str) -> String {
    format!("table:{db}.{table}")
}

/// The node for an authored pipeline's `"zone.table"` end: `bronze` is an
/// Iceberg Bronze table, `silver` is `ClickHouse` `silver`, and `gold` /
/// `serving` is `ClickHouse` `serving`.
fn zone_node(zone_table: &str) -> Option<(String, String, &'static str)> {
    let (zone, table) = zone_table.split_once('.')?;
    match zone {
        "bronze" => Some((
            format!("bronze:{table}"),
            format!("bronze.{table}"),
            "bronze",
        )),
        "silver" => Some((
            table_id("silver", table),
            format!("silver.{table}"),
            "silver",
        )),
        "gold" | "serving" => Some((
            table_id("serving", table),
            format!("serving.{table}"),
            "gold",
        )),
        _ => None,
    }
}

/// `db.table` references in a view's `SELECT`, matched against the tables
/// that exist (`known`), with or without backtick quoting. A name that is
/// not an existing table is never an edge.
fn referenced_tables(sql: &str, known: &BTreeSet<String>) -> BTreeSet<String> {
    let cleaned = sql.replace('`', "");
    let mut out = BTreeSet::new();
    for token in cleaned.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')) {
        if known.contains(token) {
            out.insert(token.to_owned());
        }
    }
    out
}

/// Every shared, one-per-deployment record (see the module doc).
async fn add_shared(state: &AppState, g: &mut Graph) {
    let ch = &state.clickhouse;
    add_tables_and_views(ch, g).await;
    add_catalog(ch, g).await;
    add_exports(ch, g).await;
}

/// `silver`/`serving` tables, and each view's edges from the tables its
/// `SELECT` reads.
async fn add_tables_and_views(ch: &lakehouse_clickhouse::ChClient, g: &mut Graph) {
    let tables = ch
        .rows(
            "SELECT database, name, engine, as_select FROM system.tables \
             WHERE database IN ('silver', 'serving') AND NOT is_temporary",
            None,
        )
        .await
        .unwrap_or_default();
    let mut known: BTreeSet<String> = BTreeSet::new();
    for row in &tables {
        let (db, name) = (text(row, "database"), text(row, "name"));
        let kind = if db == "serving" { "gold" } else { "silver" };
        g.node(&table_id(&db, &name), &format!("{db}.{name}"), kind);
        known.insert(format!("{db}.{name}"));
    }
    // `lake` holds the Bronze registry tables as `bronze_meta.<name>`; a
    // view reading one of them references `lake.bronze_meta.<name>`.
    let lake = ch
        .rows(
            "SELECT name FROM system.tables WHERE database = 'lake'",
            None,
        )
        .await
        .unwrap_or_default();
    for row in &lake {
        known.insert(format!("lake.{}", text(row, "name")));
    }
    for row in &tables {
        let engine = text(row, "engine");
        if engine != "View" && engine != "MaterializedView" {
            continue;
        }
        let view = format!("{}.{}", text(row, "database"), text(row, "name"));
        for source in referenced_tables(&text(row, "as_select"), &known) {
            if source == view {
                continue;
            }
            let from = if let Some(rest) = source.strip_prefix("lake.") {
                let id = format!("lake:{rest}");
                g.node(&id, &format!("lake.{rest}"), "bronze");
                id
            } else {
                let (db, name) = source.split_once('.').unwrap_or(("", &source));
                table_id(db, name)
            };
            let (vdb, vname) = view.split_once('.').unwrap_or(("", &view));
            g.edge(
                &from,
                &table_id(vdb, vname),
                "view",
                "view definition (system.tables.as_select)",
            );
        }
    }
}

/// Datasets from the catalog registry, the Gold table each is served
/// from, and each dataset's publisher.
async fn add_catalog(ch: &lakehouse_clickhouse::ChClient, g: &mut Graph) {
    let catalog = ch
        .rows(
            &format!(
                "SELECT slug, title, table_name FROM {}",
                crate::routes::ai::tools::data::CATALOG_UNION
            ),
            None,
        )
        .await;
    match catalog {
        Ok(rows) => {
            for row in &rows {
                let (slug, title, table) = (
                    text(row, "slug"),
                    text(row, "title"),
                    text(row, "table_name"),
                );
                let dataset = format!("dataset:{slug}");
                g.node(
                    &dataset,
                    if title.is_empty() { &slug } else { &title },
                    "dataset",
                );
                if !table.is_empty() {
                    let gold = table_id("serving", &table);
                    g.node(&gold, &format!("serving.{table}"), "gold");
                    g.edge(
                        &dataset,
                        &gold,
                        "catalog",
                        "catalog registry (bronze_meta.dataset_catalog.table_name)",
                    );
                }
            }
        }
        Err(_) => g.coverage.push(
            "the Bronze catalog registry could not be read, so datasets are not shown".to_owned(),
        ),
    }
    let publishers = ch
        .rows(
            "SELECT slug, author FROM lake.`bronze_meta.dataset_sync` \
             UNION ALL SELECT slug, author FROM lake.`bronze_meta_sec.dataset_sync`",
            None,
        )
        .await
        .unwrap_or_default();
    for row in &publishers {
        let (slug, author) = (text(row, "slug"), text(row, "author"));
        if author.is_empty() {
            continue;
        }
        let source = format!("source:{author}");
        g.node(&source, &author, "source");
        g.edge(
            &source,
            &format!("dataset:{slug}"),
            "publisher",
            "dataset sync record (bronze_meta.dataset_sync.author)",
        );
    }
}

/// Gold tables exported to Iceberg by a successful Gold export.
async fn add_exports(ch: &lakehouse_clickhouse::ChClient, g: &mut Graph) {
    let exports = ch
        .rows(
            "SELECT DISTINCT mart FROM console.gold_export_run WHERE status = 'success'",
            None,
        )
        .await
        .unwrap_or_default();
    for row in &exports {
        let mart = text(row, "mart");
        let iceberg = format!("iceberg:gold.{mart}");
        g.node(&iceberg, &format!("gold.{mart} (Iceberg)"), "iceberg");
        g.edge(
            &table_id("serving", &mart),
            &iceberg,
            "export",
            "successful Gold export (console.gold_export_run)",
        );
    }
}

async fn add_tenant_owned(state: &AppState, tenant: Option<uuid::Uuid>, g: &mut Graph) {
    let Some(pool) = state.pg.as_deref() else {
        g.coverage.push(
            "no Postgres pool: connector ingest and authored pipelines are not shown".to_owned(),
        );
        return;
    };
    let connectors = lakehouse_store::connectors::list_connectors(
        pool,
        &lakehouse_store::connectors::ConnectorFilter { tenant_id: tenant },
    )
    .await
    .unwrap_or_default();
    let names: HashMap<String, String> = connectors
        .iter()
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect();
    let ingestible = lakehouse_store::connectors::list_ingestible_connectors(pool)
        .await
        .unwrap_or_default();
    for connector in ingestible.iter().filter(|c| names.contains_key(&c.id)) {
        let id = format!("connector:{}", connector.id);
        g.node(
            &id,
            names
                .get(&connector.id)
                .map_or(connector.id.as_str(), String::as_str),
            "connector",
        );
        for object in connector.source_objects.as_array().into_iter().flatten() {
            let Some(target) = object.get("target").and_then(Value::as_str) else {
                continue;
            };
            let bronze = format!("bronze:{target}");
            g.node(&bronze, &format!("bronze.{target}"), "bronze");
            g.edge(
                &id,
                &bronze,
                "ingest",
                &format!("ingest spec ({} adapter)", connector.adapter),
            );
        }
    }
    let pipelines = lakehouse_store::pipelines::list_pipelines(
        pool,
        &lakehouse_store::pipelines::PipelineFilter {
            tenant_id: tenant,
            all_tenants: false,
        },
    )
    .await
    .unwrap_or_default();
    for pipeline in &pipelines {
        let (Some((from, from_label, from_kind)), Some((to, to_label, to_kind))) =
            (zone_node(&pipeline.source), zone_node(&pipeline.target))
        else {
            continue;
        };
        g.node(&from, &from_label, from_kind);
        g.node(&to, &to_label, to_kind);
        g.edge(
            &from,
            &to,
            "pipeline",
            &format!("authored pipeline {} ({})", pipeline.name, pipeline.id),
        );
    }
}

/// Node ids matching `focus`: an exact id, a dataset slug, a qualified
/// table name (`serving.x`, `gold.x` for an Iceberg copy), a bare table
/// name (warehouse tables only), or a label.
fn matches_focus(g: &Graph, focus: &str) -> Vec<String> {
    let f = focus.trim().to_lowercase();
    g.nodes
        .iter()
        .filter(|(id, (label, _))| {
            let id = id.to_lowercase();
            let label = label.to_lowercase();
            id == f
                || id.split_once(':').is_some_and(|(_, rest)| rest == f)
                || label == f
                // A bare table name resolves to a warehouse table only; the
                // Iceberg copy of an exported table shares its name, sits
                // downstream of it, and is reached by walking the graph.
                || (!id.starts_with("iceberg:") && id.rsplit('.').next() == Some(f.as_str()))
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// Everything upstream and downstream of `start`: incoming edges walked
/// back, outgoing edges walked forward, never sideways (a sibling that
/// shares a parent is not in the focus's lineage).
fn reachable(g: &Graph, start: &[String]) -> BTreeSet<String> {
    let mut keep: BTreeSet<String> = start.iter().cloned().collect();
    for forward in [true, false] {
        let mut frontier: Vec<String> = start.to_vec();
        while let Some(node) = frontier.pop() {
            if keep.len() >= MAX_NODES {
                break;
            }
            for (from, to, _, _) in &g.edges {
                let next = if forward && *from == node {
                    to
                } else if !forward && *to == node {
                    from
                } else {
                    continue;
                };
                if keep.insert(next.clone()) {
                    frontier.push(next.clone());
                }
            }
        }
    }
    keep
}

/// The recorded lineage around `focus` for `principal` (see the module
/// doc). An empty or unknown focus returns an empty graph with a note, so
/// the response never depends on which data a deployment holds.
///
/// # Errors
///
/// [`ApiError::NotFound`] when `X-Tenant` names a tenant the caller does
/// not belong to, and [`ApiError::Unavailable`] when the shared-catalog
/// rule cannot be evaluated: both from the same helpers the catalog and
/// connector routes use.
pub(crate) async fn build(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
    focus: &str,
) -> Result<Value, ApiError> {
    let tenant = crate::tenant_scope::resolve(principal, headers)?;
    let mut g = Graph::default();
    match crate::routes::catalog::catalog_tenant_refusal(state, principal, headers).await? {
        None => add_shared(state, &mut g).await,
        Some(reason) => g.coverage.push(format!(
            "shared catalog, ClickHouse tables and Gold exports are not shown: {reason}"
        )),
    }
    add_tenant_owned(state, tenant, &mut g).await;
    g.coverage.push(
        "only edges the platform recorded are drawn; a table loaded by a job outside the \
         platform has no incoming edge"
            .to_owned(),
    );

    let base = json!({ "focus": focus, "columnMappings": [], "supported": true });
    let empty = |note: String| {
        let mut body = base.clone();
        body["nodes"] = json!([]);
        body["edges"] = json!([]);
        body["note"] = json!(note);
        body
    };
    if focus.trim().is_empty() {
        return Ok(empty(
            "pass ?focus= with a dataset slug or a table name (e.g. serving.<table>)".to_owned(),
        ));
    }
    let start = matches_focus(&g, focus);
    if start.is_empty() {
        return Ok(empty(format!("no recorded lineage mentions {focus}")));
    }
    let keep = reachable(&g, &start);
    let nodes: Vec<Value> = g
        .nodes
        .iter()
        .filter(|(id, _)| keep.contains(*id))
        .map(|(id, (label, kind))| {
            let kind = if start.contains(id) { "focus" } else { kind };
            json!({ "id": id, "label": label, "kind": kind })
        })
        .collect();
    let edges: Vec<Value> = g
        .edges
        .iter()
        .enumerate()
        .filter(|(_, (from, to, _, _))| keep.contains(from) && keep.contains(to))
        .map(|(i, (from, to, kind, evidence))| {
            json!({ "id": format!("e{i}"), "from": from, "to": to, "kind": kind, "evidence": evidence })
        })
        .collect();
    let mut body = base;
    body["nodes"] = json!(nodes);
    body["edges"] = json!(edges);
    body["coverage"] = json!(g.coverage);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Graph {
        let mut g = Graph::default();
        g.node("source:Pub", "Pub", "source");
        g.node("dataset:visits", "Visits", "dataset");
        g.node("table:serving.mart_visits", "serving.mart_visits", "gold");
        g.node(
            "iceberg:gold.mart_visits",
            "gold.mart_visits (Iceberg)",
            "iceberg",
        );
        g.node("dataset:events", "Events", "dataset");
        g.node("table:serving.mart_events", "serving.mart_events", "gold");
        g.edge("source:Pub", "dataset:visits", "publisher", "x");
        g.edge("source:Pub", "dataset:events", "publisher", "x");
        g.edge(
            "dataset:visits",
            "table:serving.mart_visits",
            "catalog",
            "x",
        );
        g.edge(
            "dataset:events",
            "table:serving.mart_events",
            "catalog",
            "x",
        );
        g.edge(
            "table:serving.mart_visits",
            "iceberg:gold.mart_visits",
            "export",
            "x",
        );
        g
    }

    #[test]
    fn a_focus_matches_by_slug_table_name_or_qualified_name() {
        let g = graph();
        assert_eq!(
            matches_focus(&g, "visits"),
            vec!["dataset:visits".to_owned()]
        );
        assert_eq!(
            matches_focus(&g, "mart_visits"),
            vec!["table:serving.mart_visits".to_owned()]
        );
        assert_eq!(
            matches_focus(&g, "serving.mart_visits"),
            vec!["table:serving.mart_visits".to_owned()]
        );
        assert_eq!(
            matches_focus(&g, "gold.mart_visits"),
            vec!["iceberg:gold.mart_visits".to_owned()]
        );
        assert!(matches_focus(&g, "nothing").is_empty());
    }

    #[test]
    fn lineage_walks_up_and_down_but_never_to_siblings() {
        let g = graph();
        let keep = reachable(&g, &["table:serving.mart_visits".to_owned()]);
        assert!(keep.contains("dataset:visits"));
        assert!(keep.contains("source:Pub"));
        assert!(keep.contains("iceberg:gold.mart_visits"));
        // The publisher's other dataset is a sibling, not lineage.
        assert!(!keep.contains("dataset:events"));
        assert!(!keep.contains("table:serving.mart_events"));
    }

    #[test]
    fn a_view_references_only_tables_that_exist() {
        let known: BTreeSet<String> = ["lake.bronze_meta.replication_slot", "serving.mart_x"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let refs = referenced_tables(
            "SELECT a FROM lake.`bronze_meta.replication_slot` JOIN serving.mart_x USING a WHERE b.c = 1",
            &known,
        );
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn a_pipeline_zone_maps_to_the_right_layer() {
        assert_eq!(zone_node("bronze.orders").map(|n| n.2), Some("bronze"));
        assert_eq!(
            zone_node("silver.orders").map(|n| n.0),
            Some("table:silver.orders".to_owned())
        );
        assert_eq!(
            zone_node("gold.m").map(|n| n.0),
            Some("table:serving.m".to_owned())
        );
        assert_eq!(zone_node("orders"), None);
    }
}
