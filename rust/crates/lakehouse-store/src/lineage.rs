//! What a tenant's lineage is built from: the recorded definitions that
//! decide what moves where, not anything inferred from table names.
//!
//! - A connector's ingest spec (`connector.source_objects`): `ingest_job`
//!   loads each source object into the Bronze table its `target` names.
//! - An authored pipeline's definition (`pipeline_definition`):
//!   `authored_pipeline_job` reads its source table and rebuilds its target
//!   table from exactly this.
//!
//! `lakehouse-api`'s `lineage` module turns these into a graph.

use serde::Deserialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::pipelines::AuthoredDefinition;
use crate::{PgPool, StoreError};

/// A connector with an ingest spec, and where each of its objects lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageConnector {
    /// `connector.id`.
    pub id: String,
    /// `connector.name`.
    pub name: String,
    /// Each ingested object, in the spec's order.
    pub objects: Vec<IngestedObject>,
}

/// One source object of a connector and the Bronze table it is loaded into.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IngestedObject {
    /// The object in the source system, e.g. `"public.customers"`.
    pub name: String,
    /// The Bronze table it lands in (`bronze.<target>`).
    pub target: String,
}

/// An authored pipeline and the definition its runs execute.
#[derive(Debug, Clone)]
pub struct LineagePipeline {
    /// `pipeline_definition.id`.
    pub id: String,
    /// Pipeline name.
    pub name: String,
    /// Lifecycle status (`draft`, `ready`, `paused`, ...).
    pub status: String,
    /// What it reads, transforms and writes.
    pub definition: AuthoredDefinition,
}

#[derive(Debug, FromRow)]
struct ConnectorRow {
    id: String,
    name: String,
    source_objects: serde_json::Value,
}

#[derive(Debug, FromRow)]
struct PipelineRow {
    id: String,
    name: String,
    status: String,
    #[sqlx(flatten)]
    definition: crate::pipelines::DefinitionRow,
}

/// Every connector with an ingest spec and every authored pipeline in
/// `tenant_id`, oldest first. Another tenant's rows, and unassigned ones,
/// are left out.
///
/// A source object whose JSON does not carry a `name` and a `target` is
/// skipped rather than failing the whole read: `set_ingest_spec` validates
/// both, so only a hand-edited row could lack them.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn lineage_sources(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<(Vec<LineageConnector>, Vec<LineagePipeline>), StoreError> {
    let connectors: Vec<ConnectorRow> = sqlx::query_as(
        "SELECT id, name, source_objects FROM connector \
         WHERE adapter IS NOT NULL AND tenant_id = $1 ORDER BY created_at",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    let pipelines: Vec<PipelineRow> = sqlx::query_as(
        "SELECT id, name, status, source, target, incremental_column, transforms, \
         fbic_enabled, connector_id FROM pipeline_definition \
         WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok((
        connectors
            .into_iter()
            .map(|row| LineageConnector {
                id: row.id,
                name: row.name,
                objects: ingested_objects(row.source_objects),
            })
            .collect(),
        pipelines
            .into_iter()
            .map(|row| LineagePipeline {
                id: row.id,
                name: row.name,
                status: row.status,
                definition: row.definition.into(),
            })
            .collect(),
    ))
}

/// The well-formed entries of a `source_objects` JSON array.
fn ingested_objects(source_objects: serde_json::Value) -> Vec<IngestedObject> {
    match source_objects {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .filter(|o: &IngestedObject| !o.target.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn keeps_each_well_formed_object_and_skips_the_rest() {
        let objects = ingested_objects(json!([
            { "name": "public.customers", "incrementalKey": "updated_at", "target": "customers" },
            { "name": "public.orders" },
            { "name": "public.empty", "target": "" },
            "not an object",
        ]));
        assert_eq!(
            objects,
            vec![IngestedObject {
                name: "public.customers".to_owned(),
                target: "customers".to_owned(),
            }]
        );
        assert!(ingested_objects(json!({})).is_empty());
    }
}
