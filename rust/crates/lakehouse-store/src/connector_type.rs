//! `connector_type` (migration `0035`) — the reference table the console's
//! connector wizard reads to list every type it can offer, whether or not
//! this build can actually dial it yet.
//!
//! A `supported = false` row is listed honestly rather than omitted or
//! faked: the wizard can show a roadmap entry without claiming the type
//! works (AGENTS.md rule 2 — unsupported is `supported: false` with an
//! honest message, never a fabricated success). Every `supported = false`
//! row has `adapter = NULL`, since there is no
//! [`crate::ingest_spec::Dial`] shape yet for a type this build cannot
//! dial.

use serde::Serialize;
use sqlx::FromRow;

use crate::{PgPool, StoreError};

/// One row of `connector_type`. Mirrors `ConnectorType` in
/// `contracts/connectors.ts`.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorType {
    /// The display name shown in the wizard, e.g. `"PostgreSQL CDC"`.
    pub name: String,
    /// One of `sql | cdc | files | rest | sheets` — the
    /// [`crate::ingest_spec::Dial`] shape this type dials, or `None` for
    /// an unsupported type.
    pub adapter: Option<String>,
    /// Whether this build can actually dial this type today.
    pub supported: bool,
    /// A documentation link, when one has actually been published. `None`
    /// rather than a fabricated URL (AGENTS.md rule 2).
    pub docs_url: Option<String>,
}

/// List every connector type, ordered by `name`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_connector_types(pool: &PgPool) -> Result<Vec<ConnectorType>, StoreError> {
    let rows = sqlx::query_as(
        "SELECT name, adapter, supported, docs_url FROM connector_type ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
