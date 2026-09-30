//! Lineage built from what the platform records about moving data, for
//! `GET /api/governance/lineage`: connector ingest specs and authored
//! pipeline definitions (`lakehouse_store::lineage`). Every edge is a
//! definition a job executes, never a guess from table names — the reason
//! `WS1` removed the naming-convention lineage this route used to derive.
//!
//! - `source → bronze` (`ingest`): `ingest_job` loads a connector's source
//!   object into the Bronze table its `target` names.
//! - `dataset → pipeline` (`read`) and `pipeline → dataset` (`write`):
//!   `authored_pipeline_job` reads the pipeline's source and rebuilds its
//!   target.
//!
//! Jobs defined in the orchestrator's own code are not traced, and the
//! response says so in `note`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use lakehouse_store::lineage::{LineageConnector, LineagePipeline};
use serde_json::{Value, json};

use crate::transform_grammar::{Transform, parse_transform};

/// What the graph covers, shown next to it.
pub const COVERAGE_NOTE: &str = "Traced from connector ingest specs and pipelines built in the \
                                 console. Jobs defined in the orchestrator's code are not traced \
                                 yet.";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    label: String,
    /// `source | bronze | silver | serving | pipeline`, or `dataset` for a
    /// zone this module does not know.
    kind: &'static str,
    sublabel: Option<String>,
    /// What the node opens: a connector id, a pipeline id, or a catalog
    /// asset id.
    reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    from: String,
    to: String,
    kind: &'static str,
}

#[derive(Debug, Default)]
struct Graph {
    nodes: BTreeMap<String, Node>,
    edges: BTreeSet<Edge>,
}

/// The catalog asset id of a Bronze table a connector loaded:
/// `connector_catalog.py` registers it under this slug.
fn bronze_slug(table: &str) -> String {
    table.replace('_', "-")
}

fn dataset_kind(zone: &str) -> &'static str {
    match zone {
        "bronze" => "bronze",
        "silver" => "silver",
        "serving" => "serving",
        _ => "dataset",
    }
}

impl Graph {
    fn from_sources(connectors: &[LineageConnector], pipelines: &[LineagePipeline]) -> Self {
        let mut graph = Self::default();
        for connector in connectors {
            for object in &connector.objects {
                let source = format!("{}:{}", connector.id, object.name);
                graph.nodes.entry(source.clone()).or_insert_with(|| Node {
                    label: object.name.clone(),
                    kind: "source",
                    sublabel: Some(connector.name.clone()),
                    reference: connector.id.clone(),
                });
                let bronze = graph.dataset("bronze", &object.target);
                graph.edge(source, bronze, "ingest");
            }
        }
        for pipeline in pipelines {
            graph.nodes.insert(
                pipeline.id.clone(),
                Node {
                    label: pipeline.name.clone(),
                    kind: "pipeline",
                    sublabel: Some(pipeline.status.clone()),
                    reference: pipeline.id.clone(),
                },
            );
            let def = &pipeline.definition;
            let source = graph.dataset(&def.source_zone, &def.source_table);
            let target = graph.dataset(&def.target_zone, &def.target_table);
            graph.edge(source, pipeline.id.clone(), "read");
            graph.edge(pipeline.id.clone(), target, "write");
        }
        graph
    }

    /// Add the `<zone>.<table>` dataset node if it is new; return its id.
    fn dataset(&mut self, zone: &str, table: &str) -> String {
        let id = format!("{zone}.{table}");
        self.nodes.entry(id.clone()).or_insert_with(|| Node {
            label: id.clone(),
            kind: dataset_kind(zone),
            sublabel: None,
            reference: if zone == "bronze" {
                bronze_slug(table)
            } else {
                id.clone()
            },
        });
        id
    }

    fn edge(&mut self, from: String, to: String, kind: &'static str) {
        self.edges.insert(Edge { from, to, kind });
    }

    /// The node ids `focus` names: a node id, a connector id (all of its
    /// source objects), or a Bronze table's catalog slug.
    fn resolve(&self, focus: &str) -> Vec<String> {
        if self.nodes.contains_key(focus) {
            return vec![focus.to_owned()];
        }
        self.nodes
            .iter()
            .filter(|(_, n)| match n.kind {
                "source" | "bronze" => n.reference == focus,
                _ => false,
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// `starts`, everything upstream of them and everything downstream —
    /// not the other branches of a shared ancestor.
    fn around(&self, starts: &[String]) -> BTreeSet<String> {
        let mut keep: BTreeSet<String> = starts.iter().cloned().collect();
        for downstream in [true, false] {
            let mut queue: VecDeque<String> = starts.iter().cloned().collect();
            let mut seen: BTreeSet<String> = starts.iter().cloned().collect();
            while let Some(id) = queue.pop_front() {
                for edge in &self.edges {
                    let next = match (downstream, edge.from == id, edge.to == id) {
                        (true, true, _) => &edge.to,
                        (false, _, true) => &edge.from,
                        _ => continue,
                    };
                    if seen.insert(next.clone()) {
                        keep.insert(next.clone());
                        queue.push_back(next.clone());
                    }
                }
            }
        }
        keep
    }
}

/// Each kept node's column in a left-to-right layout: the longest path
/// from a node with nothing upstream. A node on a cycle (a pipeline that
/// writes its own source) goes one column past the rest.
fn depths(keep: &BTreeSet<String>, edges: &[&Edge]) -> BTreeMap<String, usize> {
    let mut indegree: BTreeMap<&str, usize> = keep.iter().map(|id| (id.as_str(), 0)).collect();
    for edge in edges {
        if let Some(d) = indegree.get_mut(edge.to.as_str()) {
            *d += 1;
        }
    }
    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    for id in &queue {
        depth.insert((*id).to_owned(), 0);
    }
    while let Some(id) = queue.pop_front() {
        let here = depth.get(id).copied().unwrap_or(0);
        for edge in edges.iter().filter(|e| e.from == id) {
            let next = depth.entry(edge.to.clone()).or_insert(0);
            *next = (*next).max(here + 1);
            if let Some(d) = indegree.get_mut(edge.to.as_str()) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    queue.push_back(edge.to.as_str());
                }
            }
        }
    }
    let past = depth.values().copied().max().map_or(0, |d| d + 1);
    for id in keep {
        depth.entry(id.clone()).or_insert(past);
    }
    depth
}

/// Column mappings for one pipeline, following exactly what
/// `authored_factory._build_select_sql` executes: with a `select`, each
/// selected column is copied, cast and/or renamed; without one every
/// column is copied as is (`rename`/`cast` then do not apply). Row filters
/// and `dedupe` keep or drop rows, not columns, so they map nothing.
fn column_mappings(pipeline: &LineagePipeline) -> Vec<Value> {
    let def = &pipeline.definition;
    let source = format!("{}.{}", def.source_zone, def.source_table);
    let target = format!("{}.{}", def.target_zone, def.target_table);
    let mut columns: Option<Vec<String>> = None;
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut casts: BTreeMap<String, &'static str> = BTreeMap::new();
    for transform in def
        .transforms
        .iter()
        .filter_map(|t| parse_transform(t).ok())
    {
        match transform {
            Transform::Select { columns: cols } => {
                columns = Some(cols.iter().map(|c| c.as_str().to_owned()).collect());
            }
            Transform::Rename { from, to } => {
                renames.insert(from.as_str().to_owned(), to.as_str().to_owned());
            }
            Transform::Cast {
                column,
                target_type,
            } => {
                casts.insert(column.as_str().to_owned(), target_type);
            }
            Transform::Filter { .. } | Transform::Dedupe { .. } => {}
        }
    }
    let Some(columns) = columns else {
        return vec![json!({
            "source": format!("{source}.*"),
            "target": format!("{target}.*"),
            "transform": "all columns, as is",
        })];
    };
    columns
        .iter()
        .map(|column| {
            let new_name = renames.get(column);
            let transform = match (casts.get(column), new_name) {
                (Some(ty), Some(_)) => format!("cast to {ty}, renamed"),
                (Some(ty), None) => format!("cast to {ty}"),
                (None, Some(_)) => "renamed".to_owned(),
                (None, None) => "copied".to_owned(),
            };
            json!({
                "source": format!("{source}.{column}"),
                "target": format!("{target}.{}", new_name.unwrap_or(column)),
                "transform": transform,
            })
        })
        .collect()
}

/// The lineage graph around `focus` (every recorded flow when `focus` is
/// empty), as `LineageGraph` in `contracts/governance.ts`.
///
/// `focusIds` lists the nodes `focus` resolved to, so the page can mark
/// them; an id nothing is recorded for resolves to none and comes back as
/// an empty graph, not an error.
#[must_use]
pub fn graph_around(
    connectors: &[LineageConnector],
    pipelines: &[LineagePipeline],
    focus: &str,
) -> Value {
    let graph = Graph::from_sources(connectors, pipelines);
    let starts = if focus.is_empty() {
        Vec::new()
    } else {
        graph.resolve(focus)
    };
    let keep: BTreeSet<String> = if focus.is_empty() {
        graph.nodes.keys().cloned().collect()
    } else {
        graph.around(&starts)
    };
    let edges: Vec<&Edge> = graph
        .edges
        .iter()
        .filter(|e| keep.contains(&e.from) && keep.contains(&e.to))
        .collect();
    let depth = depths(&keep, &edges);

    let mut nodes: Vec<(&usize, &String, &Node)> = keep
        .iter()
        .filter_map(|id| Some((depth.get(id)?, id, graph.nodes.get(id)?)))
        .collect();
    nodes.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));

    let column_mappings: Vec<Value> = pipelines
        .iter()
        .filter(|p| keep.contains(&p.id))
        .flat_map(column_mappings)
        .collect();

    json!({
        "focus": focus,
        "focusIds": starts,
        "nodes": nodes.iter().map(|(depth, id, node)| json!({
            "id": id,
            "label": node.label,
            "kind": node.kind,
            "sublabel": node.sublabel,
            "ref": node.reference,
            "depth": depth,
        })).collect::<Vec<_>>(),
        "edges": edges.iter().map(|e| json!({
            "id": format!("{}->{}", e.from, e.to),
            "from": e.from,
            "to": e.to,
            "kind": e.kind,
        })).collect::<Vec<_>>(),
        "columnMappings": column_mappings,
        "supported": true,
        "note": COVERAGE_NOTE,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_store::lineage::IngestedObject;
    use lakehouse_store::pipelines::AuthoredDefinition;

    use super::*;

    fn connector() -> LineageConnector {
        LineageConnector {
            id: "conn-nw".to_owned(),
            name: "Northwind".to_owned(),
            objects: vec![
                IngestedObject {
                    name: "public.customers".to_owned(),
                    target: "northwind_customers".to_owned(),
                },
                IngestedObject {
                    name: "public.orders".to_owned(),
                    target: "northwind_orders".to_owned(),
                },
            ],
        }
    }

    fn pipeline(
        id: &str,
        source: (&str, &str),
        target: (&str, &str),
        transforms: &[&str],
    ) -> LineagePipeline {
        LineagePipeline {
            id: id.to_owned(),
            name: id.trim_start_matches("pl-").to_owned(),
            status: "ready".to_owned(),
            definition: AuthoredDefinition {
                source_zone: source.0.to_owned(),
                source_table: source.1.to_owned(),
                incremental_column: None,
                transforms: transforms.iter().map(|t| (*t).to_owned()).collect(),
                fbic_enabled: false,
                target_zone: target.0.to_owned(),
                target_table: target.1.to_owned(),
                connector_id: None,
            },
        }
    }

    fn ids(v: &Value, key: &str) -> Vec<String> {
        v[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_owned())
            .collect()
    }

    fn sample() -> (Vec<LineageConnector>, Vec<LineagePipeline>) {
        (
            vec![connector()],
            vec![
                pipeline(
                    "pl-customers-de",
                    ("bronze", "northwind_customers"),
                    ("silver", "customers_de"),
                    &[
                        "select(customer_id,country)",
                        "rename(country,land)",
                        "filter(country = 'Germany')",
                    ],
                ),
                pipeline(
                    "pl-customers-mart",
                    ("silver", "customers_de"),
                    ("serving", "mart_customers_de"),
                    &[],
                ),
            ],
        )
    }

    #[test]
    fn a_pipeline_is_traced_back_to_its_connector_and_forward_to_its_consumers() {
        let (connectors, pipelines) = sample();
        let v = graph_around(&connectors, &pipelines, "pl-customers-de");
        assert_eq!(
            ids(&v, "nodes"),
            [
                "conn-nw:public.customers",
                "bronze.northwind_customers",
                "pl-customers-de",
                "silver.customers_de",
                "pl-customers-mart",
                "serving.mart_customers_de",
            ],
            "ordered left to right; the connector's other object is not this lineage"
        );
        assert_eq!(v["focusIds"], json!(["pl-customers-de"]));
        let kinds: Vec<&str> = v["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds.iter().filter(|k| **k == "ingest").count(), 1);
        assert_eq!(kinds.len(), 5);
        assert_eq!(v["nodes"][1]["ref"], "northwind-customers");
        assert_eq!(v["supported"], true);
    }

    #[test]
    fn a_bronze_slug_or_a_connector_id_finds_its_nodes() {
        let (connectors, pipelines) = sample();
        let by_slug = graph_around(&connectors, &pipelines, "northwind-customers");
        assert_eq!(by_slug["focusIds"], json!(["bronze.northwind_customers"]));
        let by_connector = graph_around(&connectors, &pipelines, "conn-nw");
        assert_eq!(
            by_connector["focusIds"],
            json!(["conn-nw:public.customers", "conn-nw:public.orders"])
        );
        assert!(ids(&by_connector, "nodes").contains(&"bronze.northwind_orders".to_owned()));
    }

    #[test]
    fn an_id_nothing_is_recorded_for_is_an_empty_graph() {
        let (connectors, pipelines) = sample();
        let v = graph_around(&connectors, &pipelines, "event-pariwisata");
        assert!(ids(&v, "nodes").is_empty());
        assert!(v["focusIds"].as_array().unwrap().is_empty());
    }

    #[test]
    fn no_focus_shows_every_recorded_flow() {
        let (connectors, pipelines) = sample();
        let v = graph_around(&connectors, &pipelines, "");
        assert_eq!(ids(&v, "nodes").len(), 8);
    }

    #[test]
    fn column_mappings_follow_what_the_run_executes() {
        let (_, pipelines) = sample();
        let with_select = column_mappings(&pipelines[0]);
        assert_eq!(
            with_select,
            vec![
                json!({
                    "source": "bronze.northwind_customers.customer_id",
                    "target": "silver.customers_de.customer_id",
                    "transform": "copied",
                }),
                json!({
                    "source": "bronze.northwind_customers.country",
                    "target": "silver.customers_de.land",
                    "transform": "renamed",
                }),
            ]
        );
        let without_select = column_mappings(&pipelines[1]);
        assert_eq!(without_select[0]["source"], "silver.customers_de.*");
        assert_eq!(without_select[0]["transform"], "all columns, as is");

        let cast = pipeline(
            "pl-c",
            ("silver", "a"),
            ("silver", "b"),
            &["select(amount)", "cast(amount,Float64)"],
        );
        assert_eq!(column_mappings(&cast)[0]["transform"], "cast to Float64");
    }

    #[test]
    fn a_pipeline_that_writes_its_own_source_still_lays_out() {
        let looped = pipeline("pl-loop", ("silver", "a"), ("silver", "a"), &[]);
        let v = graph_around(&[], &[looped], "pl-loop");
        assert_eq!(ids(&v, "nodes").len(), 2);
    }
}
