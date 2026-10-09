//! What a connector's source tables looked like at the last run, and the
//! changes found since (`SRC-8`; tables `connector_source_schema`,
//! `connector_schema_change`, `connector_inactive_column`, migration
//! `0063_connector_schema_changes.sql` -- read its header for the shape
//! of each).
//!
//! # Why this lives in the store and not in a handler
//!
//! The API decides, the orchestrator observes (plan section 2). The
//! comparison and the policy are pure (`schema_diff.rs`); this module owns
//! the state they work on and the two writes that must be atomic:
//!
//! - [`ObservationTx::record`] writes the change rows, moves the baseline
//!   when the changes were applied, keeps the last OBSERVED shape of a
//!   table that waits, and sets the connector pause, in ONE transaction. A
//!   pending change never moves the baseline: the table keeps being
//!   compared against what the Bronze table was last told to hold, so a
//!   removed column keeps being "removed" until a person decides.
//! - [`approve_object`] accepts everything that waits for one table, moves
//!   the baseline to what was last observed, marks removed columns
//!   inactive and lifts the connector pause when nothing else waits.
//!
//! Both take an advisory lock on `(connector, table)` first, so two runs
//! observing the same table at once cannot both insert the same change.
//!
//! # Identity of a pending change (idempotence)
//!
//! A pending change is identified by `(connector, table, kind, column)`
//! (`column` is the empty string for `primary_key_changed` and
//! `table_added`); the partial unique index in the migration enforces it.
//! Seeing the same change again on the next run updates its `after` value
//! and `breaking` flag in place and reports it as NOT new, so the caller
//! raises no second alert. A pending change the source no longer shows
//! (the column came back) is withdrawn: its row is deleted, because it
//! never took effect and `status` has no value for "withdrawn".
//!
//! Nothing here holds source data: only table names, column names and type
//! names (`AGENTS.md` principle 4).

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, types::Json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

/// The `paused_reason` the `pause` policy writes, and the only one
/// [`approve_object`] lifts: a pause set by something else (`SRC-11`'s
/// auto-disable) is not this module's to clear.
pub const SCHEMA_CHANGE_PAUSE_REASON: &str =
    "Paused by the schema-change policy until a change is approved";

/// The four per-connector policies (`SRC-8` decision D1). A breaking
/// change waits under every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaChangePolicy {
    /// Apply a non-breaking change, list it, alert. The default.
    ApplyNonBreaking,
    /// As above, and add tables that appear in a schema already loaded.
    ApplyAll,
    /// List and alert, do not apply; the table keeps loading its known columns.
    AskFirst,
    /// The whole connector stops loading until a change is approved.
    Pause,
}

impl SchemaChangePolicy {
    /// The value stored in `connector.schema_change_policy` and sent on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApplyNonBreaking => "apply_non_breaking",
            Self::ApplyAll => "apply_all",
            Self::AskFirst => "ask_first",
            Self::Pause => "pause",
        }
    }

    /// Parse a stored or wire value; `None` for anything else.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "apply_non_breaking" => Some(Self::ApplyNonBreaking),
            "apply_all" => Some(Self::ApplyAll),
            "ask_first" => Some(Self::AskFirst),
            "pause" => Some(Self::Pause),
            _ => None,
        }
    }
}

/// One column of a source table, as the orchestrator reflected it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedColumn {
    /// Column name, compared exactly (case-sensitive).
    pub name: String,
    /// The source's own type string, e.g. `varchar(40)`.
    pub type_name: String,
    /// Whether the source allows NULL. Recorded; a nullability change is
    /// not a schema change in `SRC-8`.
    pub nullable: bool,
}

/// What kind of change was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A column the last accepted shape did not have.
    ColumnAdded,
    /// A column the last accepted shape had and the source no longer shows.
    ColumnRemoved,
    /// The same column with another type.
    TypeChanged,
    /// The primary key's column list differs.
    PrimaryKeyChanged,
    /// A table that appeared in a schema the connector already loads from.
    TableAdded,
}

impl ChangeKind {
    /// The value stored in `connector_schema_change.kind`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ColumnAdded => "column_added",
            Self::ColumnRemoved => "column_removed",
            Self::TypeChanged => "type_changed",
            Self::PrimaryKeyChanged => "primary_key_changed",
            Self::TableAdded => "table_added",
        }
    }
}

/// Where a change stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    /// Took effect when detected.
    Applied,
    /// Waits for a person.
    Pending,
    /// A person accepted it.
    Approved,
}

impl ChangeStatus {
    /// The value stored in `connector_schema_change.status`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Pending => "pending",
            Self::Approved => "approved",
        }
    }
}

/// The last accepted shape of one source table, plus the last observed one
/// when the table waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedSchema {
    /// Source table (or object) name.
    pub object_name: String,
    /// The accepted columns.
    pub columns: Vec<ObservedColumn>,
    /// The accepted primary key.
    pub primary_key: Vec<String>,
    /// The columns last observed while a change waits; `None` when nothing waits.
    pub waiting_columns: Option<Vec<ObservedColumn>>,
    /// The primary key last observed while a change waits.
    pub waiting_primary_key: Option<Vec<String>>,
}

/// A change to write. Built from the pure comparison's output
/// (`schema_diff.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChange {
    /// What kind of change.
    pub kind: ChangeKind,
    /// The column; empty for `primary_key_changed` and `table_added`.
    pub column_name: String,
    /// The value before, as text (a type name, a key list); `None` when none.
    pub before_value: Option<String>,
    /// The value after, as text.
    pub after_value: Option<String>,
    /// Whether the change is breaking.
    pub breaking: bool,
    /// `Applied` or `Pending`.
    pub status: ChangeStatus,
}

/// A stored change, as the routes return it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SchemaChange {
    /// `connector_schema_change.id`.
    pub id: String,
    /// Source table.
    pub object_name: String,
    /// `column_added | column_removed | type_changed | primary_key_changed | table_added`.
    pub kind: String,
    /// The column; empty for table-level kinds.
    pub column_name: String,
    /// The value before.
    pub before_value: Option<String>,
    /// The value after.
    pub after_value: Option<String>,
    /// Whether the change is breaking.
    pub breaking: bool,
    /// `applied | pending | approved`.
    pub status: String,
    /// The run that detected it, when the orchestrator sent one.
    pub run_id: Option<String>,
    /// When it was detected, ISO 8601.
    pub detected_at: IsoTime,
    /// Who approved it (a user id), when approved.
    pub decided_by: Option<String>,
    /// When it was approved, ISO 8601.
    pub decided_at: Option<IsoTime>,
}

/// A time that serializes as ISO 8601 with milliseconds, like every other
/// time this crate returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoTime(String);

impl From<OffsetDateTime> for IsoTime {
    fn from(at: OffsetDateTime) -> Self {
        Self(iso_millis(at))
    }
}

impl Serialize for IsoTime {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl IsoTime {
    /// The ISO 8601 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A column the connector marked inactive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InactiveColumn {
    /// Source table.
    pub object_name: String,
    /// The column.
    pub column_name: String,
    /// When it became inactive, ISO 8601.
    pub inactive_since: IsoTime,
}

/// One change as [`ObservationTx::record`] left it, and whether this call
/// created it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedChange {
    /// The stored row.
    pub change: SchemaChange,
    /// `true` when this call inserted the row; `false` for a still-pending
    /// change seen again. Only new rows raise an alert.
    pub is_new: bool,
}

/// Everything [`ObservationTx::record`] writes besides the changes.
#[derive(Debug, Clone)]
pub struct ObservationWrite<'a> {
    /// The columns the orchestrator observed.
    pub observed_columns: &'a [ObservedColumn],
    /// The primary key it observed.
    pub observed_primary_key: &'a [String],
    /// The changes to store, `Applied` or `Pending`.
    pub changes: &'a [NewChange],
    /// `true` when the baseline becomes the observed shape: a first
    /// observation, or changes that took effect. `false` when a change
    /// waits (the baseline stays) or nothing changed.
    pub accept_observed: bool,
    /// Pause the connector (policy `pause`).
    pub pause_connector: bool,
    /// The run that observed, when sent.
    pub run_id: Option<&'a str>,
    /// The observation time.
    pub now: OffsetDateTime,
}

/// An open observation of one `(connector, table)`: the advisory lock is
/// held until [`Self::commit`] or drop.
pub struct ObservationTx<'p> {
    tx: sqlx::Transaction<'p, sqlx::Postgres>,
    connector_id: String,
    object_name: String,
}

type AcceptedRow = (
    Json<Vec<ObservedColumn>>,
    Json<Vec<String>>,
    Option<Json<Vec<ObservedColumn>>>,
    Option<Json<Vec<String>>>,
);

async fn lock(
    conn: &mut sqlx::PgConnection,
    connector_id: &str,
    object_name: &str,
) -> Result<(), StoreError> {
    // `hashtext` is a 32-bit hash: two tables may share a lock, which only
    // serialises them needlessly; it can never let two observers of the
    // SAME table run at once.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!(
            "schema-change\u{1f}{connector_id}\u{1f}{object_name}"
        ))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

impl<'p> ObservationTx<'p> {
    /// Open a transaction and take the `(connector, table)` lock.
    ///
    /// # Errors
    ///
    /// [`StoreError::Database`] if the transaction or the lock fails.
    pub async fn begin(
        pool: &'p PgPool,
        connector_id: &str,
        object_name: &str,
    ) -> Result<Self, StoreError> {
        let mut tx = pool.begin().await?;
        lock(&mut tx, connector_id, object_name).await?;
        Ok(Self {
            tx,
            connector_id: connector_id.to_owned(),
            object_name: object_name.to_owned(),
        })
    }

    /// The last accepted shape of the table; `None` before the first observation.
    ///
    /// # Errors
    ///
    /// [`StoreError::Database`] if the query fails.
    pub async fn accepted(&mut self) -> Result<Option<AcceptedSchema>, StoreError> {
        let row: Option<AcceptedRow> = sqlx::query_as(
            "SELECT columns, primary_key, waiting_columns, waiting_primary_key \
             FROM connector_source_schema WHERE connector_id = $1 AND object_name = $2",
        )
        .bind(&self.connector_id)
        .bind(&self.object_name)
        .fetch_optional(&mut *self.tx)
        .await?;
        Ok(row.map(|(columns, primary_key, wc, wp)| AcceptedSchema {
            object_name: self.object_name.clone(),
            columns: columns.0,
            primary_key: primary_key.0,
            waiting_columns: wc.map(|j| j.0),
            waiting_primary_key: wp.map(|j| j.0),
        }))
    }

    /// Write the changes, the baseline and the pause (see the module doc).
    ///
    /// # Errors
    ///
    /// [`StoreError::Database`] if any statement fails; nothing is
    /// written then, because the caller drops the transaction.
    pub async fn record(
        &mut self,
        write: &ObservationWrite<'_>,
    ) -> Result<Vec<RecordedChange>, StoreError> {
        let mut recorded = Vec::with_capacity(write.changes.len());
        for change in write.changes {
            let id = format!("chg-{}", Uuid::new_v4());
            let row: (SchemaChange, bool) = self.insert_change(&id, change, write).await?;
            recorded.push(RecordedChange {
                change: row.0,
                is_new: row.1,
            });
        }

        // Pending changes the source no longer shows are withdrawn.
        let (kinds, columns): (Vec<&str>, Vec<&str>) = write
            .changes
            .iter()
            .filter(|c| c.status == ChangeStatus::Pending)
            .map(|c| (c.kind.as_str(), c.column_name.as_str()))
            .unzip();
        sqlx::query(
            "DELETE FROM connector_schema_change \
             WHERE connector_id = $1 AND object_name = $2 AND status = 'pending' \
               AND (kind, column_name) NOT IN (SELECT * FROM unnest($3::text[], $4::text[]))",
        )
        .bind(&self.connector_id)
        .bind(&self.object_name)
        .bind(&kinds)
        .bind(&columns)
        .execute(&mut *self.tx)
        .await?;

        let waits = write
            .changes
            .iter()
            .any(|c| c.status == ChangeStatus::Pending);
        self.write_baseline(write, waits).await?;

        if write.pause_connector {
            sqlx::query(
                "UPDATE connector SET paused_reason = COALESCE(paused_reason, $2), \
                 paused_at = COALESCE(paused_at, $3) WHERE id = $1",
            )
            .bind(&self.connector_id)
            .bind(SCHEMA_CHANGE_PAUSE_REASON)
            .bind(write.now)
            .execute(&mut *self.tx)
            .await?;
        }
        Ok(recorded)
    }

    async fn insert_change(
        &mut self,
        id: &str,
        change: &NewChange,
        write: &ObservationWrite<'_>,
    ) -> Result<(SchemaChange, bool), StoreError> {
        insert_change_row(
            &mut self.tx,
            &self.connector_id,
            &self.object_name,
            id,
            change,
            write.run_id,
            write.now,
        )
        .await
    }

    async fn write_baseline(
        &mut self,
        write: &ObservationWrite<'_>,
        waits: bool,
    ) -> Result<(), StoreError> {
        if write.accept_observed {
            // `WHERE ... IS DISTINCT FROM` keeps a repeated identical
            // observation from rewriting the row.
            sqlx::query(
                "INSERT INTO connector_source_schema \
                   (connector_id, object_name, columns, primary_key, observed_at) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (connector_id, object_name) DO UPDATE SET \
                   columns = EXCLUDED.columns, primary_key = EXCLUDED.primary_key, \
                   observed_at = EXCLUDED.observed_at, \
                   waiting_columns = NULL, waiting_primary_key = NULL \
                 WHERE connector_source_schema.columns IS DISTINCT FROM EXCLUDED.columns \
                    OR connector_source_schema.primary_key IS DISTINCT FROM EXCLUDED.primary_key \
                    OR connector_source_schema.waiting_columns IS NOT NULL",
            )
            .bind(&self.connector_id)
            .bind(&self.object_name)
            .bind(Json(write.observed_columns))
            .bind(Json(write.observed_primary_key))
            .bind(write.now)
            .execute(&mut *self.tx)
            .await?;
            // A column accepted again is no longer inactive.
            let names: Vec<&str> = write
                .observed_columns
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            sqlx::query(
                "DELETE FROM connector_inactive_column \
                 WHERE connector_id = $1 AND object_name = $2 AND column_name = ANY($3)",
            )
            .bind(&self.connector_id)
            .bind(&self.object_name)
            .bind(&names)
            .execute(&mut *self.tx)
            .await?;
        } else {
            let (columns, key) = if waits {
                (
                    Some(Json(write.observed_columns)),
                    Some(Json(write.observed_primary_key)),
                )
            } else {
                (None, None)
            };
            sqlx::query(
                "UPDATE connector_source_schema SET waiting_columns = $3, waiting_primary_key = $4 \
                 WHERE connector_id = $1 AND object_name = $2 \
                   AND (waiting_columns IS DISTINCT FROM $3 \
                        OR waiting_primary_key IS DISTINCT FROM $4)",
            )
            .bind(&self.connector_id)
            .bind(&self.object_name)
            .bind(columns)
            .bind(key)
            .execute(&mut *self.tx)
            .await?;
        }
        Ok(())
    }

    /// Commit the transaction and release the lock.
    ///
    /// # Errors
    ///
    /// [`StoreError::Database`] if the commit fails.
    pub async fn commit(self) -> Result<(), StoreError> {
        self.tx.commit().await?;
        Ok(())
    }
}

/// Insert one change row, or update the same pending one in place; the bool
/// is `true` for a row this call inserted.
async fn insert_change_row(
    conn: &mut sqlx::PgConnection,
    connector_id: &str,
    object_name: &str,
    id: &str,
    change: &NewChange,
    run_id: Option<&str>,
    now: OffsetDateTime,
) -> Result<(SchemaChange, bool), StoreError> {
    const COLUMNS: &str = "id, object_name, kind, column_name, before_value, after_value, \
                           breaking, status, run_id, detected_at, decided_by, decided_at";
    // The conflict target is the pending-identity index; an `applied`
    // row never conflicts with it. `xmax = 0` is true only for a row
    // this statement inserted, false for the pending row it updated.
    let sql = format!(
        "INSERT INTO connector_schema_change \
           (id, connector_id, object_name, kind, column_name, before_value, after_value, \
            breaking, status, run_id, detected_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         ON CONFLICT (connector_id, object_name, kind, column_name) WHERE status = 'pending' \
         DO UPDATE SET after_value = EXCLUDED.after_value, breaking = EXCLUDED.breaking \
         RETURNING {COLUMNS}, (xmax = 0) AS inserted"
    );
    let row: SchemaChangeInserted = sqlx::query_as(&sql)
        .bind(id)
        .bind(connector_id)
        .bind(object_name)
        .bind(change.kind.as_str())
        .bind(&change.column_name)
        .bind(change.before_value.as_deref())
        .bind(change.after_value.as_deref())
        .bind(change.breaking)
        .bind(change.status.as_str())
        .bind(run_id)
        .bind(now)
        .fetch_one(conn)
        .await?;
    Ok((row.change, row.inserted))
}

/// Why a table that appeared at the source was not added to the connector
/// (`SRC-8` task 8). The texts are fixed: they are stored as the change's
/// `after_value`, returned by the API and shown by the console, and never
/// carry a name from the source or from an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableRefusal {
    /// Another connector or another table of this connector lands in the
    /// Bronze table the new one would get.
    TargetTaken,
    /// Uploaded files reserved the Bronze table name (ADR 0014, decision 5).
    ReservedForUploads,
}

impl TableRefusal {
    /// The fixed text.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::TargetTaken => {
                "Not added: its Bronze table name is already used by another table. \
                 Add it by hand with a different target."
            }
            Self::ReservedForUploads => {
                "Not added: its Bronze table name is reserved for uploaded files. \
                 Add it by hand with a different target."
            }
        }
    }
}

/// A table that appeared at the source, with the Bronze target it would
/// get and any reason it must not be added that the caller already found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCandidate {
    /// `<schema>.<table>`.
    pub name: String,
    /// The derived Bronze table.
    pub target: String,
    /// Refused before the store was asked (a target another connector uses,
    /// or one uploads reserved).
    pub refusal: Option<TableRefusal>,
}

/// What [`record_table_additions`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableAdditions {
    /// Names appended to the connector's tables, in request order.
    pub added: Vec<String>,
    /// Names refused, with the reason, in request order.
    pub not_added: Vec<(String, TableRefusal)>,
    /// The `table_added` rows written or seen again; `is_new` marks those
    /// this call inserted.
    pub changes: Vec<RecordedChange>,
}

/// Add the new tables of a schema the connector already loads from
/// (`SRC-8` decision D2, policy `apply_all`) and list each as a
/// `table_added` change, in ONE transaction.
///
/// - Appended: the connector's `source_objects` gains `{name, target,
///   loadMode: "replace"}` through `connectors::append_source_objects_in`, and
///   the change is `applied` with `after_value` = the target.
/// - Refused (a candidate's own `refusal`, or a target another table of the
///   connector took under the row lock): not appended; the change is
///   `pending` with `after_value` = [`TableRefusal::reason`]. Seeing the same
///   refusal again updates that pending row in place and reports it as NOT
///   new (identity: connector, table, kind, empty column), so a schedule that
///   asks every run raises one alert, not one per run.
/// - A name the connector already selects: skipped, nothing recorded, which
///   is what makes a second identical request add nothing.
///
/// # Errors
///
/// As `connectors::append_source_objects_in`, plus
/// [`StoreError::Database`]; nothing is written then.
pub async fn record_table_additions(
    pool: &PgPool,
    connector_id: &str,
    candidates: &[TableCandidate],
    run_id: Option<&str>,
    now: OffsetDateTime,
) -> Result<TableAdditions, StoreError> {
    use crate::connectors::{AdditionOutcome, SourceObjectAddition, append_source_objects_in};

    let mut tx = pool.begin().await?;
    let eligible: Vec<SourceObjectAddition> = candidates
        .iter()
        .filter(|c| c.refusal.is_none())
        .map(|c| SourceObjectAddition {
            name: c.name.clone(),
            target: c.target.clone(),
        })
        .collect();
    let mut outcomes = if eligible.is_empty() {
        Vec::new()
    } else {
        append_source_objects_in(&mut tx, connector_id, &eligible).await?
    }
    .into_iter();

    let mut result = TableAdditions {
        added: Vec::new(),
        not_added: Vec::new(),
        changes: Vec::new(),
    };
    for candidate in candidates {
        let (status, after, refusal) = match candidate.refusal {
            Some(refusal) => (
                ChangeStatus::Pending,
                refusal.reason().to_owned(),
                Some(refusal),
            ),
            None => match outcomes.next() {
                Some(AdditionOutcome::Added) => {
                    (ChangeStatus::Applied, candidate.target.clone(), None)
                }
                Some(AdditionOutcome::TargetTaken) => (
                    ChangeStatus::Pending,
                    TableRefusal::TargetTaken.reason().to_owned(),
                    Some(TableRefusal::TargetTaken),
                ),
                // Already selected: nothing to record. `None` cannot happen
                // (one outcome per eligible candidate) and is skipped too.
                Some(AdditionOutcome::AlreadySelected) | None => continue,
            },
        };
        let change = NewChange {
            kind: ChangeKind::TableAdded,
            column_name: String::new(),
            before_value: None,
            after_value: Some(after),
            breaking: false,
            status,
        };
        let id = format!("chg-{}", Uuid::new_v4());
        let (row, is_new) = insert_change_row(
            &mut tx,
            connector_id,
            &candidate.name,
            &id,
            &change,
            run_id,
            now,
        )
        .await?;
        result.changes.push(RecordedChange {
            change: row,
            is_new,
        });
        match refusal {
            Some(refusal) => result.not_added.push((candidate.name.clone(), refusal)),
            None => result.added.push(candidate.name.clone()),
        }
    }
    tx.commit().await?;
    Ok(result)
}

/// A [`SchemaChange`] row plus the `inserted` flag, decoded together.
struct SchemaChangeInserted {
    change: SchemaChange,
    inserted: bool,
}

impl<'r> FromRow<'r, sqlx::postgres::PgRow> for SchemaChangeInserted {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            change: SchemaChange::from_row(row)?,
            inserted: row.try_get("inserted")?,
        })
    }
}

const CHANGE_COLUMNS: &str = "id, object_name, kind, column_name, before_value, after_value, \
                              breaking, status, run_id, detected_at, decided_by, decided_at";

/// The most pending changes [`list_pending`] returns. A connector with more
/// waiting than this is far past what a person can review on one page; the
/// approval still takes every change of a table.
pub const PENDING_LIST_LIMIT: i64 = 500;

/// The pending changes of a connector, oldest first, at most
/// [`PENDING_LIST_LIMIT`].
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_pending(
    pool: &PgPool,
    connector_id: &str,
) -> Result<Vec<SchemaChange>, StoreError> {
    let sql = format!(
        "SELECT {CHANGE_COLUMNS} FROM connector_schema_change \
         WHERE connector_id = $1 AND status = 'pending' ORDER BY detected_at, id LIMIT $2"
    );
    Ok(sqlx::query_as(&sql)
        .bind(connector_id)
        .bind(PENDING_LIST_LIMIT)
        .fetch_all(pool)
        .await?)
}

/// The newest `limit` changes that no longer wait (applied or approved),
/// newest first.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_recent(
    pool: &PgPool,
    connector_id: &str,
    limit: i64,
) -> Result<Vec<SchemaChange>, StoreError> {
    let sql = format!(
        "SELECT {CHANGE_COLUMNS} FROM connector_schema_change \
         WHERE connector_id = $1 AND status <> 'pending' \
         ORDER BY detected_at DESC, id DESC LIMIT $2"
    );
    Ok(sqlx::query_as(&sql)
        .bind(connector_id)
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

/// Every column of a connector marked inactive, by table then name.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_inactive_columns(
    pool: &PgPool,
    connector_id: &str,
) -> Result<Vec<InactiveColumn>, StoreError> {
    let rows: Vec<(String, String, OffsetDateTime)> = sqlx::query_as(
        "SELECT object_name, column_name, inactive_since FROM connector_inactive_column \
         WHERE connector_id = $1 ORDER BY object_name, column_name",
    )
    .bind(connector_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(object_name, column_name, at)| InactiveColumn {
            object_name,
            column_name,
            inactive_since: at.into(),
        })
        .collect())
}

/// Whether the connector is paused (`SRC-8` F6): `None` for an unknown
/// connector, so a caller can answer 404 before 409.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn is_paused(pool: &PgPool, connector_id: &str) -> Result<Option<bool>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT paused_at IS NOT NULL FROM connector WHERE id = $1")
            .bind(connector_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// What [`approve_object`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    /// The changes now `approved`.
    pub approved: Vec<SchemaChange>,
    /// Whether the connector's pause was lifted (no change waits on the
    /// connector any more and the pause was the schema-change one).
    pub pause_lifted: bool,
}

/// Approve every pending change of one table (`SRC-8` decision D5): they
/// become `approved`, the baseline becomes the last OBSERVED shape, removed
/// columns become inactive, and the connector's schema-change pause is
/// lifted when nothing else waits. `Ok(None)` when nothing waits for the
/// table.
///
/// # Errors
///
/// [`StoreError::Database`] if any statement fails (nothing is written).
pub async fn approve_object(
    pool: &PgPool,
    connector_id: &str,
    object_name: &str,
    decided_by: Option<&str>,
    now: OffsetDateTime,
) -> Result<Option<Approval>, StoreError> {
    let mut tx = pool.begin().await?;
    lock(&mut tx, connector_id, object_name).await?;
    let sql = format!(
        "UPDATE connector_schema_change SET status = 'approved', decided_by = $3, decided_at = $4 \
         WHERE connector_id = $1 AND object_name = $2 AND status = 'pending' \
         RETURNING {CHANGE_COLUMNS}"
    );
    let mut approved: Vec<SchemaChange> = sqlx::query_as(&sql)
        .bind(connector_id)
        .bind(object_name)
        .bind(decided_by)
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;
    if approved.is_empty() {
        return Ok(None);
    }
    approved.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.column_name.cmp(&b.column_name)));

    // The waiting shape is the last one observed; without it (it is always
    // set together with a pending row) the baseline simply stays.
    sqlx::query(
        "UPDATE connector_source_schema SET columns = waiting_columns, \
           primary_key = COALESCE(waiting_primary_key, primary_key), observed_at = $3, \
           waiting_columns = NULL, waiting_primary_key = NULL \
         WHERE connector_id = $1 AND object_name = $2 AND waiting_columns IS NOT NULL",
    )
    .bind(connector_id)
    .bind(object_name)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    for change in approved.iter().filter(|c| c.kind == "column_removed") {
        sqlx::query(
            "INSERT INTO connector_inactive_column \
               (connector_id, object_name, column_name, inactive_since) \
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(connector_id)
        .bind(object_name)
        .bind(&change.column_name)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    // An approved addition accepts the column again: not inactive.
    let added: Vec<&str> = approved
        .iter()
        .filter(|c| c.kind == "column_added")
        .map(|c| c.column_name.as_str())
        .collect();
    sqlx::query(
        "DELETE FROM connector_inactive_column \
         WHERE connector_id = $1 AND object_name = $2 AND column_name = ANY($3)",
    )
    .bind(connector_id)
    .bind(object_name)
    .bind(&added)
    .execute(&mut *tx)
    .await?;

    let (others,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM connector_schema_change \
         WHERE connector_id = $1 AND status = 'pending')",
    )
    .bind(connector_id)
    .fetch_one(&mut *tx)
    .await?;
    let pause_lifted = if others {
        false
    } else {
        sqlx::query(
            "UPDATE connector SET paused_reason = NULL, paused_at = NULL \
             WHERE id = $1 AND paused_reason = $2",
        )
        .bind(connector_id)
        .bind(SCHEMA_CHANGE_PAUSE_REASON)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0
    };
    tx.commit().await?;
    Ok(Some(Approval {
        approved,
        pause_lifted,
    }))
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for IsoTime {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        Ok(<OffsetDateTime as sqlx::Decode<sqlx::Postgres>>::decode(value)?.into())
    }
}

impl sqlx::Type<sqlx::Postgres> for IsoTime {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <OffsetDateTime as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}
