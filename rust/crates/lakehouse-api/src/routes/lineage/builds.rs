//! What each `silver`/`serving` table was last built from: one more record
//! for the lineage graph, kept in a module of its own so `lineage.rs` stays
//! close to the file it is upstream.
//!
//! | Edge | Record |
//! | --- | --- |
//! | source table -> the table built from it | the statement that last wrote the table, in `ClickHouse`'s `system.query_log` |
//!
//! This is what answers "what is this Gold table built from". A Silver or
//! Gold table is filled by a `CREATE TABLE … AS SELECT` or an `INSERT …
//! SELECT`, whoever runs it — a job in the orchestrator's code, an authored
//! pipeline — and `ClickHouse` logs the tables that statement read. The
//! edge is observed, not inferred from table names, and its `evidence`
//! names the statement and when it ran, like every other edge.
//!
//! A table filled some other way, or last written before the query log's
//! retention, has no such edge; the response's `coverage` says so.

use serde_json::{Map, Value};

use super::{Graph, table_id, text};

/// Per `silver`/`serving` table, the statement that last wrote it while
/// reading at least one other table: the tables it named, when it ran and
/// whether it created the table or inserted into it. `read` is one table
/// per line, as `system.query_log.tables` spells them. Bounded to half a
/// year so the scan stays small on a log nobody trims.
const BUILDS_SQL: &str = "\
SELECT target, arrayStringConcat(argMax(tables, event_time), '\\n') AS read, \
       formatDateTime(max(event_time), '%Y-%m-%d %H:%i', 'UTC') AS at, \
       argMax(query_kind, event_time) AS kind \
FROM ( \
    SELECT replaceAll(extract(query, '(?i)^\\\\s*(?:CREATE\\\\s+(?:OR\\\\s+REPLACE\\\\s+)?TABLE(?:\\\\s+IF\\\\s+NOT\\\\s+EXISTS)?|INSERT\\\\s+INTO(?:\\\\s+TABLE)?)\\\\s+([`\\\\w.]+)'), '`', '') AS target, \
           tables, event_time, query_kind \
    FROM system.query_log \
    WHERE type = 'QueryFinish' AND query_kind IN ('Create', 'Insert') AND length(tables) > 1 \
      AND hasAny(databases, ['silver', 'serving']) AND event_date >= today() - 180 \
) \
WHERE startsWith(target, 'silver.') OR startsWith(target, 'serving.') \
GROUP BY target";

/// The edge kind of a table build.
const BUILD: &str = "build";

/// The node a table named in `system.query_log.tables` stands for:
/// ``<catalog db>.`bronze.<table>` `` is a Bronze Iceberg table read
/// through a `DataLakeCatalog` database, and `silver.x` / `serving.x` are
/// warehouse tables. Anything else — the registry in `lake`, the console's
/// own tables — is not lineage of a dataset and is left out.
fn source(name: &str) -> Option<(String, String, &'static str)> {
    let name = name.replace('`', "");
    let (db, rest) = name.split_once('.')?;
    if let Some(table) = rest.strip_prefix("bronze.") {
        return Some((
            format!("bronze:{table}"),
            format!("bronze.{table}"),
            "bronze",
        ));
    }
    match db {
        "silver" => Some((table_id("silver", rest), name.clone(), "silver")),
        "serving" => Some((table_id("serving", rest), name.clone(), "gold")),
        _ => None,
    }
}

/// Draws [`BUILDS_SQL`]'s rows: an edge into each table that still exists
/// from every table its last build read.
fn add_rows(g: &mut Graph, rows: &[Map<String, Value>]) {
    for row in rows {
        let target = text(row, "target");
        let Some((db, table)) = target.split_once('.') else {
            continue;
        };
        let to = table_id(db, table);
        if !g.nodes.contains_key(&to) {
            continue;
        }
        let statement = if text(row, "kind") == "Insert" {
            "INSERT … SELECT"
        } else {
            "CREATE TABLE … AS SELECT"
        };
        let evidence = format!(
            "built by {statement} on {} UTC (system.query_log)",
            text(row, "at")
        );
        for name in text(row, "read").lines() {
            let Some((from, label, kind)) = source(name) else {
                continue;
            };
            g.node(&from, &label, kind);
            g.edge(&from, &to, BUILD, &evidence);
        }
    }
}

/// What each `silver`/`serving` table was last built from, as `ClickHouse`
/// itself logged it. Shared and one per deployment, like the tables.
pub(super) async fn add(ch: &lakehouse_clickhouse::ChClient, g: &mut Graph) {
    match ch.rows(BUILDS_SQL, None).await {
        Ok(rows) => add_rows(g, &rows),
        Err(err) => {
            tracing::warn!(?err, "lineage: the query log could not be read");
            g.coverage.push(
                "what each table is built from is not shown: ClickHouse's query log could not be read"
                    .to_owned(),
            );
        }
    }
}

/// Drops a build edge between two nodes some other record already
/// connects: an authored pipeline's run is such a build, and the pipeline's
/// own edge says more about it. Called once every other source is in.
pub(super) fn drop_repeats(g: &mut Graph) {
    let recorded: Vec<(String, String)> = g
        .edges
        .iter()
        .filter(|(_, _, kind, _)| *kind != BUILD)
        .map(|(from, to, _, _)| (from.clone(), to.clone()))
        .collect();
    g.edges.retain(|(from, to, kind, _)| {
        *kind != BUILD || !recorded.iter().any(|(f, t)| f == from && t == to)
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::reachable;
    use super::*;

    fn row(target: &str, read: &[&str], kind: &str) -> Map<String, Value> {
        let Value::Object(row) = json!({
            "target": target, "read": read.join("\n"), "at": "2026-09-22 10:03", "kind": kind,
        }) else {
            unreachable!("a JSON object literal")
        };
        row
    }

    /// A Gold table's lineage reaches back through Silver to Bronze from
    /// what `ClickHouse` logged of each build — the answer to "what is
    /// this mart built from", which used to be "nothing recorded".
    #[test]
    fn a_table_build_draws_an_edge_from_each_table_it_read() {
        let mut g = Graph::default();
        g.node("table:silver.materials", "silver.materials", "silver");
        g.node(
            "table:serving.mart_by_group",
            "serving.mart_by_group",
            "gold",
        );
        add_rows(
            &mut g,
            &[
                row(
                    "serving.mart_by_group",
                    &["serving.mart_by_group", "silver.materials"],
                    "Create",
                ),
                row(
                    "silver.materials",
                    &[
                        "icecat_sap.`bronze.materials`",
                        "silver.materials",
                        "lake.`bronze_meta.dataset_sync`",
                    ],
                    "Insert",
                ),
                // A table that no longer exists is not drawn into.
                row(
                    "silver.dropped",
                    &["silver.dropped", "silver.materials"],
                    "Create",
                ),
            ],
        );

        let edges: Vec<(&str, &str, &str)> = g
            .edges
            .iter()
            .map(|(f, t, k, _)| (f.as_str(), t.as_str(), *k))
            .collect();
        assert_eq!(
            edges,
            vec![
                (
                    "table:silver.materials",
                    "table:serving.mart_by_group",
                    "build"
                ),
                ("bronze:materials", "table:silver.materials", "build"),
            ]
        );
        // The Bronze table became a node; the registry table did not.
        assert_eq!(g.nodes.get("bronze:materials").map(|n| n.1), Some("bronze"));
        assert!(!g.nodes.keys().any(|id| id.contains("dataset_sync")));
        // Each edge cites the statement and when it ran.
        assert_eq!(
            g.edges[0].3,
            "built by CREATE TABLE … AS SELECT on 2026-09-22 10:03 UTC (system.query_log)"
        );
        assert!(g.edges[1].3.starts_with("built by INSERT … SELECT"));

        // And the mart's lineage now walks back to Bronze.
        let keep = reachable(&g, &["table:serving.mart_by_group".to_owned()]);
        assert!(keep.contains("bronze:materials"));
    }

    /// A pair an authored pipeline connects stays one edge: the
    /// pipeline's, whichever of the two was added first.
    #[test]
    fn a_build_a_pipeline_already_accounts_for_is_not_drawn_twice() {
        let mut g = Graph::default();
        g.node("bronze:customers", "bronze.customers", "bronze");
        g.node("table:silver.customers_de", "silver.customers_de", "silver");
        g.node(
            "table:serving.mart_customers",
            "serving.mart_customers",
            "gold",
        );
        add_rows(
            &mut g,
            &[
                row(
                    "silver.customers_de",
                    &["icecat_pipelines.`bronze.customers`", "silver.customers_de"],
                    "Create",
                ),
                row(
                    "serving.mart_customers",
                    &["serving.mart_customers", "silver.customers_de"],
                    "Create",
                ),
            ],
        );
        g.edge(
            "bronze:customers",
            "table:silver.customers_de",
            "pipeline",
            "authored pipeline customers_de (pl-1)",
        );
        drop_repeats(&mut g);

        let edges: Vec<(&str, &str)> = g
            .edges
            .iter()
            .map(|(_, to, kind, _)| (to.as_str(), *kind))
            .collect();
        assert_eq!(
            edges,
            vec![
                ("table:serving.mart_customers", "build"),
                ("table:silver.customers_de", "pipeline"),
            ]
        );
    }
}
