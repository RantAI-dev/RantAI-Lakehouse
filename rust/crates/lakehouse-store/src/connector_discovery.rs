//! The wire contract of table discovery, and the store of what the
//! orchestrator discovered (`SRC-13` decisions D1, D9, D10; table
//! `connector_discovery`, migration `0075_connector_discovery.sql` -- read its
//! header for the shape of the row).
//!
//! # Two requests, one answer shape
//!
//! A person asks one of two things of a connector's source:
//!
//! - a **list** ([`Kind::List`]): the names of what exists there (schemas or
//!   tables, files, endpoints, topics, collections), names first and no
//!   columns (D9);
//! - a **preview** ([`Kind::Preview`]): the columns of ONE listed object, with
//!   a type each and whether the type was declared by the source or
//!   inferred from a sample (D6).
//!
//! The API answers either from its own drivers (SQL, object storage, REST) in
//! the same request, or launches the orchestrator's `discover_job` and answers
//! `running`; the job posts its result back and the console polls. Both paths
//! produce an [`Answer`], so the console reads one shape:
//!
//! ```json
//! { "kind": "list", "state": "done", "scope": "public",
//!   "objects": [ { "name": "public.orders", "kind": "table",
//!                  "size": 1024, "modifiedAt": "2026-10-15T09:00:00Z" } ],
//!   "truncated": false }
//! { "kind": "preview", "state": "done", "object": "public.orders",
//!   "columns": [ { "name": "id", "typeName": "integer", "inferred": false } ],
//!   "primaryKey": ["id"] }
//! { "kind": "list", "state": "failed", "errorClass": "unreachable",
//!   "reason": "The source could not be reached." }
//! { "kind": "list", "state": "running" }
//! ```
//!
//! `size`, `modifiedAt`, `primaryKey`, `scope`, `object` and `reason` are left
//! out when there is nothing to say; nothing is ever invented (AGENTS.md
//! principle 2).
//!
//! # Principle 4: no source text, ever
//!
//! A failure is one of the eight words of [`ErrorClass`] (D10's classes, plus
//! `address_refused` and `failed`, see its docs) and the fixed sentence the API
//! renders for it ([`ErrorClass::reason`]). The orchestrator's post is parsed
//! into [`Listing`] or [`Preview`] with unknown fields refused, every string in
//! it is checked (non-blank, bounded, no control characters), and what is
//! stored is the re-serialised typed value. So whatever the orchestrator sends,
//! the row can hold only names the source returned, numbers, times and the
//! closed words.
//!
//! # Bounds
//!
//! At most [`MAX_OBJECTS`] objects, [`MAX_COLUMNS`] columns and
//! [`MAX_RESULT_BYTES`] of JSON. A list cut at the limit says so (`truncated`).
//!
//! # Latest wins, and a stale request
//!
//! One row per connector and kind. [`begin`] overwrites it with a new
//! `running` request and its random `request_id`; [`finish`] stores a result
//! only for that id while the row is still `running`. A `running` row older
//! than [`STALE_AFTER_SECONDS`] is shown as a `timeout` failure
//! ([`Stored::into_answer`]): a job that died never posts, and the console must
//! not wait for it forever.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::schema_change::IsoTime;
use crate::{PgPool, StoreError};

/// The most objects one answer lists (`SRC-13` decision D2, D9).
pub const MAX_OBJECTS: usize = 10_000;

/// The most columns one preview lists.
pub const MAX_COLUMNS: usize = 2_000;

/// The most bytes of JSON one stored result may take. The migration repeats it
/// as a CHECK.
pub const MAX_RESULT_BYTES: usize = 3 * 1024 * 1024;

/// The longest object, column or key name accepted, in characters. The same
/// bound as a schema observation's names.
pub const MAX_NAME_CHARS: usize = 256;

/// The longest type name accepted, in characters.
pub const MAX_TYPE_CHARS: usize = 128;

/// How long a `running` request is believed. The orchestrator's job takes
/// seconds; past this the row is shown as a timeout.
pub const STALE_AFTER_SECONDS: i64 = 300;

/// The fixed sentence that goes with a list cut at [`MAX_OBJECTS`].
pub const TRUNCATED_REASON: &str =
    "Only the first 10,000 are shown. Narrow the search to see the rest.";

/// What a person asks of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The names of what exists at the source.
    List,
    /// The columns of one listed object.
    Preview,
}

impl Kind {
    /// The value stored in `connector_discovery.kind`, sent on the wire and
    /// used as the `{kind}` path segment.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Preview => "preview",
        }
    }

    /// The kind a path segment or a stored value names, or `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "list" => Some(Self::List),
            "preview" => Some(Self::Preview),
            _ => None,
        }
    }
}

/// Where a request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The orchestrator's job was launched and has not posted yet.
    Running,
    /// The answer is here.
    Done,
    /// The request failed; `errorClass` says how.
    Failed,
}

impl State {
    /// The value stored in `connector_discovery.state`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// How a request failed: the fixed words of `SRC-13` decision D10, the only
/// description of a failure that is ever stored or shown. The six words D10
/// names are `unreachable`, `sign_in_refused`, `no_permission`, `timeout`,
/// `too_many_objects` and `not_supported`. Two more are needed to stay honest:
/// `address_refused` (the product's own address check refused the host, an
/// action the operator can take something about) and `failed` (a failure no
/// rule above names, said as such instead of guessed into another class).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// The source or the network cannot be reached.
    Unreachable,
    /// The source refused the connector's credential.
    SignInRefused,
    /// The product's address check refused the host.
    AddressRefused,
    /// The credential works but may not list or read this at the source.
    NoPermission,
    /// The source did not answer in time.
    Timeout,
    /// The source holds more than can be listed or returned.
    TooManyObjects,
    /// Listing is not available for this connector type or setting.
    NotSupported,
    /// A failure none of the other words names.
    Failed,
}

impl ErrorClass {
    /// The value stored in `connector_discovery.error_class`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::SignInRefused => "sign_in_refused",
            Self::AddressRefused => "address_refused",
            Self::NoPermission => "no_permission",
            Self::Timeout => "timeout",
            Self::TooManyObjects => "too_many_objects",
            Self::NotSupported => "not_supported",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [
            Self::Unreachable,
            Self::SignInRefused,
            Self::AddressRefused,
            Self::NoPermission,
            Self::Timeout,
            Self::TooManyObjects,
            Self::NotSupported,
            Self::Failed,
        ]
        .into_iter()
        .find(|class| class.as_str() == text)
    }

    /// The fixed sentence for this class: what the console shows. Never a
    /// driver's or a source's text (AGENTS.md principle 4).
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Unreachable => "The source could not be reached.",
            Self::SignInRefused => "The source refused the connector's credential.",
            Self::AddressRefused => "The source's address is not allowed by this installation.",
            Self::NoPermission => {
                "The connector's account has no permission to read this at the source."
            }
            Self::Timeout => "The source did not answer in time.",
            Self::TooManyObjects => {
                "The source holds more than can be listed at once. Narrow the search."
            }
            Self::NotSupported => "Listing is not supported for this connector type.",
            Self::Failed => "The source could not be read.",
        }
    }
}

/// What a listed object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// A schema (SQL sources, `SRC-13` decision D7).
    Schema,
    /// A base table (decision D8: views are not listed).
    Table,
    /// A file or object in object storage or on an SFTP server.
    File,
    /// An endpoint a REST connector declares.
    Endpoint,
    /// A Kafka topic.
    Topic,
    /// A `MongoDB` collection.
    Collection,
}

/// One listed object. `name` is what a source object's `name` is in the ingest
/// spec: `schema.table` for SQL, the key for files, the path for an endpoint,
/// the topic or collection name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListedObject {
    /// The object's name at the source.
    pub name: String,
    /// What it is.
    pub kind: ObjectKind,
    /// Its size in bytes, for a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    /// When it last changed, RFC 3339, for a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
}

impl ListedObject {
    /// An object with only a name and a kind.
    #[must_use]
    pub fn named(name: impl Into<String>, kind: ObjectKind) -> Self {
        Self {
            name: name.into(),
            kind,
            size: None,
            modified_at: None,
        }
    }
}

/// One column of a preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListedColumn {
    /// The column's name.
    pub name: String,
    /// The type: the source's own spelling when `inferred` is false, a type
    /// guessed from a sample (D6) when it is true.
    pub type_name: String,
    /// Whether `type_name` was inferred from a sample ("inferred from a
    /// sample" in the console) rather than declared by the source.
    pub inferred: bool,
}

/// The result of a [`Kind::List`] request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Listing {
    /// What was listed, at most [`MAX_OBJECTS`].
    pub objects: Vec<ListedObject>,
    /// Whether the source held more than `objects` shows.
    pub truncated: bool,
}

/// The result of a [`Kind::Preview`] request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preview {
    /// The columns, in the source's order, at most [`MAX_COLUMNS`]. Empty when
    /// the source had nothing to infer them from (an empty topic, collection or
    /// file): "no sample" is said as no columns, not as a failure and not as
    /// invented ones (`SRC-13` decision D6; the first load learns them).
    pub columns: Vec<ListedColumn>,
    /// The primary key's columns in key order, when the source declares one;
    /// absent when it declares none or the type has no key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_key: Option<Vec<String>>,
}

/// A name (object, column, key) as accepted: not blank, at most
/// [`MAX_NAME_CHARS`] characters, no control characters.
fn check_name(what: &str, value: &str, max: usize) -> Result<(), StoreError> {
    if value.trim().is_empty() {
        return Err(StoreError::Validation(format!("{what} must not be blank")));
    }
    if value.chars().count() > max {
        return Err(StoreError::Validation(format!(
            "{what} is longer than {max} characters"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(StoreError::Validation(format!(
            "{what} must not contain control characters"
        )));
    }
    Ok(())
}

fn check_size(value: &impl Serialize) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| StoreError::Validation("the result cannot be written".to_owned()))?;
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(StoreError::Validation(format!(
            "the result is larger than {MAX_RESULT_BYTES} bytes"
        )));
    }
    Ok(())
}

impl Listing {
    /// Check every bound of the contract (see the module docs).
    ///
    /// # Errors
    ///
    /// [`StoreError::Validation`] for more than [`MAX_OBJECTS`] objects, a blank,
    /// oversize or control-character name, a negative size, a `modifiedAt` that is
    /// not RFC 3339, or a result past [`MAX_RESULT_BYTES`].
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.objects.len() > MAX_OBJECTS {
            return Err(StoreError::Validation(format!(
                "a list holds at most {MAX_OBJECTS} objects"
            )));
        }
        for object in &self.objects {
            check_name("an object name", &object.name, MAX_NAME_CHARS)?;
            if object.size.is_some_and(|size| size < 0) {
                return Err(StoreError::Validation(
                    "an object size must not be negative".to_owned(),
                ));
            }
            if object
                .modified_at
                .as_deref()
                .is_some_and(|at| at.len() > 40 || OffsetDateTime::parse(at, &Rfc3339).is_err())
            {
                return Err(StoreError::Validation(
                    "modifiedAt must be an RFC 3339 time".to_owned(),
                ));
            }
        }
        check_size(self)
    }
}

impl Preview {
    /// Check every bound of the contract (see the module docs).
    ///
    /// # Errors
    ///
    /// [`StoreError::Validation`] for more than [`MAX_COLUMNS`], a blank,
    /// oversize or control-character name or type, a key longer than the
    /// columns, or a result past [`MAX_RESULT_BYTES`].
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.columns.len() > MAX_COLUMNS {
            return Err(StoreError::Validation(format!(
                "a preview holds at most {MAX_COLUMNS} columns"
            )));
        }
        for column in &self.columns {
            check_name("a column name", &column.name, MAX_NAME_CHARS)?;
            check_name("a type name", &column.type_name, MAX_TYPE_CHARS)?;
        }
        if let Some(key) = &self.primary_key {
            if key.len() > self.columns.len() {
                return Err(StoreError::Validation(
                    "the key is longer than the columns".to_owned(),
                ));
            }
            for name in key {
                check_name("a key column name", name, MAX_NAME_CHARS)?;
            }
        }
        check_size(self)
    }
}

/// What the orchestrator (or the API) ends a request with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A list, for a [`Kind::List`] request.
    Listing(Listing),
    /// Columns, for a [`Kind::Preview`] request.
    Preview(Preview),
    /// The request failed.
    Failed(ErrorClass),
}

/// The response of every discovery route: see the module docs for the JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    /// Which request this answers.
    pub kind: Kind,
    /// Where it stands.
    pub state: State,
    /// What a list was narrowed by (the schema or the glob), when it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// The object a preview is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// A finished list's objects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objects: Option<Vec<ListedObject>>,
    /// Whether a finished list was cut.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// A finished preview's columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<ListedColumn>>,
    /// A finished preview's primary key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_key: Option<Vec<String>>,
    /// A failed request's class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<ErrorClass>,
    /// The fixed sentence for a failure, or for a cut list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// When the request started, for an answer read from the store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<IsoTime>,
    /// When it ended, for an answer read from the store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<IsoTime>,
}

impl Answer {
    fn bare(kind: Kind, state: State) -> Self {
        Self {
            kind,
            state,
            scope: None,
            object: None,
            objects: None,
            truncated: None,
            columns: None,
            primary_key: None,
            error_class: None,
            reason: None,
            started_at: None,
            ended_at: None,
        }
    }

    /// A request whose job is under way.
    #[must_use]
    pub fn running(kind: Kind) -> Self {
        Self::bare(kind, State::Running)
    }

    /// A failed request: the class and its fixed sentence.
    #[must_use]
    pub fn failed(kind: Kind, class: ErrorClass) -> Self {
        let mut answer = Self::bare(kind, State::Failed);
        answer.error_class = Some(class);
        answer.reason = Some(class.reason());
        answer
    }

    /// A finished list. `scope` is what it was narrowed by, `""` for nothing.
    #[must_use]
    pub fn listing(scope: &str, listing: Listing) -> Self {
        let mut answer = Self::bare(Kind::List, State::Done);
        answer.scope = (!scope.is_empty()).then(|| scope.to_owned());
        answer.reason = listing.truncated.then_some(TRUNCATED_REASON);
        answer.truncated = Some(listing.truncated);
        answer.objects = Some(listing.objects);
        answer
    }

    /// A finished preview of `object`.
    #[must_use]
    pub fn preview(object: &str, preview: Preview) -> Self {
        let mut answer = Self::bare(Kind::Preview, State::Done);
        answer.object = Some(object.to_owned());
        answer.columns = Some(preview.columns);
        answer.primary_key = preview.primary_key;
        answer
    }
}

/// A stored request, as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// Which request.
    pub kind: Kind,
    /// Its random id, which the job echoes.
    pub request_id: String,
    /// The object a preview is about, `""` for a list.
    pub object: String,
    /// What a list was narrowed by, `""` for nothing.
    pub scope: String,
    /// Where it stands.
    pub state: State,
    /// The failure's class, when it failed.
    pub error_class: Option<ErrorClass>,
    /// The list, when a list finished.
    pub listing: Option<Listing>,
    /// The columns, when a preview finished.
    pub preview: Option<Preview>,
    /// When it started.
    pub started_at: OffsetDateTime,
    /// When it ended.
    pub ended_at: Option<OffsetDateTime>,
}

impl Stored {
    /// The answer to show at `now`. A `running` request older than
    /// [`STALE_AFTER_SECONDS`] is a `timeout` failure: its job died without
    /// posting, and nobody should wait for it (module docs).
    #[must_use]
    pub fn into_answer(self, now: OffsetDateTime) -> Answer {
        let started = IsoTime::from(self.started_at);
        let ended = self.ended_at.map(IsoTime::from);
        let mut answer = match self.state {
            State::Running if (now - self.started_at).whole_seconds() > STALE_AFTER_SECONDS => {
                Answer::failed(self.kind, ErrorClass::Timeout)
            }
            State::Running => Answer::running(self.kind),
            State::Failed => {
                Answer::failed(self.kind, self.error_class.unwrap_or(ErrorClass::Failed))
            }
            State::Done => match (self.kind, self.listing, self.preview) {
                (Kind::List, Some(listing), _) => Answer::listing(&self.scope, listing),
                (Kind::Preview, _, Some(preview)) => Answer::preview(&self.object, preview),
                // A done row without its result cannot be written (the
                // migration's CHECK); said as a failure, not as an empty list.
                (kind, _, _) => Answer::failed(kind, ErrorClass::Failed),
            },
        };
        if self.kind == Kind::Preview && answer.object.is_none() {
            answer.object = Some(self.object);
        }
        if self.kind == Kind::List && answer.scope.is_none() && !self.scope.is_empty() {
            answer.scope = Some(self.scope);
        }
        answer.started_at = Some(started);
        answer.ended_at = ended;
        answer
    }
}

#[derive(FromRow)]
struct Row {
    kind: String,
    request_id: String,
    object: String,
    scope: String,
    state: String,
    error_class: Option<String>,
    result: Option<serde_json::Value>,
    started_at: OffsetDateTime,
    ended_at: Option<OffsetDateTime>,
}

fn unreadable() -> StoreError {
    StoreError::Validation("a stored discovery row cannot be read".to_owned())
}

impl TryFrom<Row> for Stored {
    type Error = StoreError;

    fn try_from(row: Row) -> Result<Self, StoreError> {
        let kind = Kind::parse(&row.kind).ok_or_else(unreadable)?;
        let state = State::parse(&row.state).ok_or_else(unreadable)?;
        let (mut listing, mut preview) = (None, None);
        if let Some(value) = row.result {
            match kind {
                Kind::List => {
                    listing = Some(serde_json::from_value(value).map_err(|_| unreadable())?);
                }
                Kind::Preview => {
                    preview = Some(serde_json::from_value(value).map_err(|_| unreadable())?);
                }
            }
        }
        Ok(Self {
            kind,
            request_id: row.request_id,
            object: row.object,
            scope: row.scope,
            state,
            error_class: row.error_class.as_deref().and_then(ErrorClass::parse),
            listing,
            preview,
            started_at: row.started_at,
            ended_at: row.ended_at,
        })
    }
}

/// Start a request: write (or overwrite) the connector's row for `kind` as
/// `running`, with `request_id` for the job to echo (module docs, "Latest
/// wins"). `object` is the preview's object (`""` for a list), `scope` what a
/// list is narrowed by.
///
/// # Errors
///
/// [`StoreError::Validation`] for an `object` or `scope` that is too long or
/// holds a control character, or an `object` on a list;
/// [`StoreError::ForeignKeyViolation`] when `connector_id` names no connector;
/// [`StoreError::Database`] otherwise.
pub async fn begin(
    pool: &PgPool,
    connector_id: &str,
    kind: Kind,
    request_id: &str,
    object: &str,
    scope: &str,
    at: OffsetDateTime,
) -> Result<(), StoreError> {
    if kind == Kind::List && !object.is_empty() {
        return Err(StoreError::Validation("a list names no object".to_owned()));
    }
    for (what, value) in [("object", object), ("scope", scope)] {
        if value.chars().count() > MAX_NAME_CHARS || value.chars().any(char::is_control) {
            return Err(StoreError::Validation(format!(
                "{what} is too long or holds a control character"
            )));
        }
    }
    sqlx::query(
        "INSERT INTO connector_discovery \
           (connector_id, kind, request_id, object, scope, state, started_at) \
         VALUES ($1, $2, $3, $4, $5, 'running', $6) \
         ON CONFLICT (connector_id, kind) DO UPDATE SET \
           request_id = EXCLUDED.request_id, object = EXCLUDED.object, \
           scope = EXCLUDED.scope, state = 'running', error_class = NULL, \
           result = NULL, started_at = EXCLUDED.started_at, ended_at = NULL",
    )
    .bind(connector_id)
    .bind(kind.as_str())
    .bind(request_id)
    .bind(object)
    .bind(scope)
    .bind(at)
    .execute(pool)
    .await?;
    Ok(())
}

/// End the request `request_id` with `outcome`. Returns `false` when nothing was
/// stored: the row is no longer that request (a newer one replaced it), or it
/// already ended (a repeated post). The first report wins.
///
/// # Errors
///
/// [`StoreError::Validation`] when `outcome` is a list for a preview (or the
/// reverse) or breaks a bound of [`Listing::validate`] / [`Preview::validate`];
/// [`StoreError::Database`] otherwise.
pub async fn finish(
    pool: &PgPool,
    connector_id: &str,
    kind: Kind,
    request_id: &str,
    outcome: &Outcome,
    at: OffsetDateTime,
) -> Result<bool, StoreError> {
    let (state, class, result): (State, Option<ErrorClass>, Option<serde_json::Value>) =
        match (kind, outcome) {
            (Kind::List, Outcome::Listing(listing)) => {
                listing.validate()?;
                (State::Done, None, Some(to_json(listing)?))
            }
            (Kind::Preview, Outcome::Preview(preview)) => {
                preview.validate()?;
                (State::Done, None, Some(to_json(preview)?))
            }
            (_, Outcome::Failed(class)) => (State::Failed, Some(*class), None),
            _ => {
                return Err(StoreError::Validation(
                    "the result does not match the kind of the request".to_owned(),
                ));
            }
        };
    let stored = sqlx::query(
        "UPDATE connector_discovery \
         SET state = $4, error_class = $5, result = $6, ended_at = $7 \
         WHERE connector_id = $1 AND kind = $2 AND request_id = $3 AND state = 'running'",
    )
    .bind(connector_id)
    .bind(kind.as_str())
    .bind(request_id)
    .bind(state.as_str())
    .bind(class.map(ErrorClass::as_str))
    .bind(result)
    .bind(at)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(stored > 0)
}

fn to_json(value: &impl Serialize) -> Result<serde_json::Value, StoreError> {
    serde_json::to_value(value)
        .map_err(|_| StoreError::Validation("the result cannot be written".to_owned()))
}

/// The connector's latest request of `kind`, if there is one.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails; [`StoreError::Validation`] for a
/// row that cannot be read back (not reachable through [`begin`] and
/// [`finish`], and the migration's CHECKs).
pub async fn get(
    pool: &PgPool,
    connector_id: &str,
    kind: Kind,
) -> Result<Option<Stored>, StoreError> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT kind, request_id, object, scope, state, error_class, result, started_at, ended_at \
         FROM connector_discovery WHERE connector_id = $1 AND kind = $2",
    )
    .bind(connector_id)
    .bind(kind.as_str())
    .fetch_optional(pool)
    .await?;
    row.map(Stored::try_from).transpose()
}
