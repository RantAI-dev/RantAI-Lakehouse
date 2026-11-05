//! Repository layer for the write-side halves of the queries domain: saved
//! queries and query-run history.
//!
//! # Postgres vs `ClickHouse`, method by method
//!
//! `src/services/clients/queries.ts` splits `QueryService` into a real half
//! (`run`/`estimate`, `ClickHouse`-backed, ported in Phase 1;
//! `generateSql`, LLM-backed, which asks the copilot's `/api/ai/chat`) and a
//! mock half (`listSaved`, `listHistory`).
//! This module is the Postgres backing for that mock half, plus one
//! addition: `routes::query::run` calls [`record_history`] after a
//! successful `ClickHouse` execution, so [`list_history`] returns *real*
//! past executions instead of fabricated fixtures — see
//! [`record_history`]'s doc comment for how that write is made non-fatal.
//!
//! The console's saved queries started as mock data, backed by seed rows
//! (`0004_queries.sql`) the way `identity`'s `workspace_settings` is backed
//! by a constant. This module now owns their write side too — create,
//! update, delete, folders, versions and trash (migration
//! `0122_saved_query_access.sql`) — with the access rules of
//! `crate::saved_query_access` applied on every read and write.

use serde::Serialize;
use sqlx::{FromRow, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

pub use crate::saved_query_access::SavedQueryViewer;
use crate::saved_query_access::{
    AccessGrant, AccessTarget, FolderMap, FolderNode, Level, ensure_personal_space,
    escape_like_pattern, grants_for_viewer, list_folders, resolve_level,
};
use crate::{PgPool, StoreError};

/// Render a timestamp the way JavaScript's `Date.prototype.toISOString`
/// does. Duplicated from `identity.rs`/`governance.rs` — see either's doc
/// comment for why.
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

// ── Saved queries ───────────────────────────────────────────────────────

/// A saved query. Mirrors `SavedQuery` in `contracts/queries.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    /// `saved_query.id`, as a string.
    pub id: String,
    /// Display title.
    pub title: String,
    /// The saved SQL text.
    pub sql: String,
    /// Who saved this query.
    pub owner: String,
    /// When it was last saved/edited, ISO 8601. Serializes as `updatedAt`.
    pub updated_at: String,
    /// Free-text tags.
    pub tags: Vec<String>,
    /// The folder the query sits in; `None` is the shared space.
    pub folder_id: Option<String>,
    /// The author's user id, when the query has one.
    pub owner_id: Option<String>,
    /// The level the caller holds on it: "view", "edit" or "manage".
    pub level: String,
}

/// A saved query, as the database holds it.
#[derive(Debug, FromRow)]
struct SavedQueryRow {
    id: Uuid,
    title: String,
    sql: String,
    owner: String,
    updated_at: OffsetDateTime,
    tags: Vec<String>,
    folder_id: Option<Uuid>,
    owner_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    deleted_at: Option<OffsetDateTime>,
}

impl SavedQueryRow {
    /// The item the access resolver is asked about.
    fn target(&self) -> AccessTarget {
        AccessTarget {
            id: self.id,
            owner_id: self.owner_id,
            folder_id: self.folder_id,
            tenant_id: self.tenant_id,
        }
    }

    /// The wire shape, at the level the caller holds on it.
    fn into_saved(self, level: Level) -> SavedQuery {
        SavedQuery {
            id: self.id.to_string(),
            title: self.title,
            sql: self.sql,
            owner: self.owner,
            updated_at: iso_millis(self.updated_at),
            tags: self.tags,
            folder_id: self.folder_id.map(|id| id.to_string()),
            owner_id: self.owner_id.map(|id| id.to_string()),
            level: level.as_str().to_owned(),
        }
    }
}

/// Which saved queries a caller may read (`SEC-20`), and who they are.
///
/// A saved query is a statement somebody wrote against their own data, so
/// it belongs to the tenant that wrote it: one tenant's rows are that
/// tenant's alone, and the rows that belong to no tenant are read only by
/// the tenant that owns the shared catalog.
///
/// The scope also carries the **viewer**: who is asking, with the roles
/// they hold and the tenant they act as. The level the viewer holds on a
/// row is decided by [`resolve_level`], so a read path cannot answer
/// "visible" without also answering "at what level" — and a new read path
/// cannot compile without saying who reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedQueryScope {
    /// Every row: a caller who holds every permission, or a deployment
    /// that holds a single tenant — there is no other tenant's row to
    /// hide from anybody.
    All {
        /// Who is asking. The scope's tenant is "every tenant's rows",
        /// so the viewer carries no tenant of its own.
        viewer: SavedQueryViewer,
    },
    /// One tenant's rows, and the rows that belong to no tenant when
    /// `include_tenantless` is set.
    Tenant {
        /// The tenant whose rows are read.
        tenant_id: Uuid,
        /// Whether the rows that belong to no tenant are read as well.
        /// They are the rows written before the column existed, and the
        /// rows whose tenant was deleted: nobody recorded which tenant
        /// wrote them, so they are the shared catalog's own, and the
        /// tenant that owns that catalog is the one that reads them.
        include_tenantless: bool,
        /// Who is asking.
        viewer: SavedQueryViewer,
    },
    /// No row at all: a caller that belongs to no tenant owns no saved
    /// query and reads none.
    None,
}

impl SavedQueryScope {
    /// The tenant whose rows the scope reads: `None` for every row.
    fn tenant_id(&self) -> Option<Uuid> {
        match self {
            Self::All { .. } | Self::None => None,
            Self::Tenant { tenant_id, .. } => Some(*tenant_id),
        }
    }

    /// Whether the rows that belong to no tenant are read as well.
    fn includes_tenantless(&self) -> bool {
        match self {
            Self::Tenant {
                include_tenantless, ..
            } => *include_tenantless,
            Self::All { .. } | Self::None => false,
        }
    }

    /// The viewer, with the scope's own tenant applied.
    ///
    /// The scope's tenant and the viewer's tenant are the same thing: a
    /// scope that reads one tenant's rows is a scope whose viewer acts
    /// as that tenant, and a scope that reads every tenant's rows is a
    /// scope whose viewer acts as none of them. The scope decides, so a
    /// caller cannot widen the reach by handing in a viewer of its own.
    ///
    /// Public because a write path needs the same viewer the read path
    /// used: the store's own writes take a [`SavedQueryViewer`] and check
    /// the tenant boundary against it, so a route that writes has to ask
    /// the scope — never a viewer of its own — who it writes as.
    #[must_use]
    pub fn effective_viewer(&self) -> Option<SavedQueryViewer> {
        match self {
            Self::All { viewer } => {
                let mut viewer = viewer.clone();
                viewer.tenant_id = None;
                Some(viewer)
            }
            Self::Tenant {
                tenant_id, viewer, ..
            } => {
                let mut viewer = viewer.clone();
                viewer.tenant_id = Some(*tenant_id);
                Some(viewer)
            }
            Self::None => None,
        }
    }
}

/// What the caller may see, and at what level: the folders of the tenant
/// the read covers, the rules that name the viewer, and the one resolver
/// that turns both into a level.
struct Visibility {
    folders: FolderMap,
    grants: Vec<AccessGrant>,
    viewer: SavedQueryViewer,
}

impl Visibility {
    /// Read the folders and the rules once, for the whole answer.
    async fn load(pool: &PgPool, viewer: &SavedQueryViewer) -> Result<Self, StoreError> {
        Ok(Self {
            folders: FolderMap::new(list_folders(pool, viewer.tenant_id).await?),
            grants: grants_for_viewer(pool, viewer).await?,
            viewer: viewer.clone(),
        })
    }

    /// The level the viewer holds on the item, or `None` when the item
    /// does not exist for them.
    ///
    /// A query whose folder is not in the map — a folder of another
    /// tenant, or one whose chain cannot be built — has no level at all:
    /// it is invisible, not an error.
    fn level(&self, target: &AccessTarget) -> Option<Level> {
        let chain: Vec<FolderNode> = match target.folder_id {
            Some(folder_id) => self
                .folders
                .chain(folder_id)?
                .into_iter()
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        resolve_level(target, &chain, &self.grants, &self.viewer)
    }
}

/// The saved queries the scope allows, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn list_saved(
    pool: &PgPool,
    scope: SavedQueryScope,
) -> Result<Vec<SavedQuery>, StoreError> {
    list_saved_filtered(pool, &scope, None, None).await
}

/// The saved queries the scope allows whose title or SQL holds `needle`,
/// newest first.
///
/// `%`, `_` and `\` in `needle` are literals, not wildcards: the shared
/// `escape_like_pattern` helper escapes the escape character first, then
/// the two wildcards, and `ILIKE` does the case-insensitive contains-match
/// over the visible rows only.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn list_saved_matching(
    pool: &PgPool,
    scope: &SavedQueryScope,
    needle: Option<&str>,
) -> Result<Vec<SavedQuery>, StoreError> {
    list_saved_filtered(pool, scope, needle, None).await
}

/// Which folder the list is filtered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedQueryFolder {
    /// One folder: the queries that sit in it.
    Folder(Uuid),
    /// The shared space: the queries that sit in no folder.
    Root,
}

/// The saved queries the scope allows, newest first, filtered by the text
/// typed and by the folder asked for.
///
/// Both filters are the same filter as the single-SQL list they replace:
/// one statement, one pass over the visible rows, so a list that is
/// filtered by text and by folder cannot disagree with a list that is
/// filtered by text alone. A folder that does not exist matches no row —
/// the answer is an empty list, not an error, because the caller asked
/// for "what is in this folder", not "does this folder exist".
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn list_saved_filtered(
    pool: &PgPool,
    scope: &SavedQueryScope,
    needle: Option<&str>,
    folder: Option<SavedQueryFolder>,
) -> Result<Vec<SavedQuery>, StoreError> {
    let Some(viewer) = scope.effective_viewer() else {
        // A caller that belongs to no tenant reads no row: the answer is
        // empty, and the database is never asked.
        return Ok(Vec::new());
    };
    let pattern = needle.map(escape_like_pattern);
    // The folder filter is two bound values rather than one, because
    // "the shared space" is the absence of a folder and cannot be spelled
    // as an id.
    let (folder_kind, folder_id): (Option<&str>, Option<Uuid>) = match folder {
        None => (None, None),
        Some(SavedQueryFolder::Root) => (Some("root"), None),
        Some(SavedQueryFolder::Folder(id)) => (Some("folder"), Some(id)),
    };
    let rows: Vec<SavedQueryRow> = sqlx::query_as(
        "SELECT id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at \
         FROM saved_query \
         WHERE deleted_at IS NULL \
           AND ($1::uuid IS NULL OR tenant_id = $1 OR ($2::boolean AND tenant_id IS NULL)) \
           AND ($3::text IS NULL OR title ILIKE $4 OR sql ILIKE $4) \
           AND ($5::text IS NULL \
                OR ($5::text = 'root' AND folder_id IS NULL) \
                OR ($6::uuid IS NOT NULL AND folder_id = $6)) \
         ORDER BY updated_at DESC, title",
    )
    .bind(scope.tenant_id())
    .bind(scope.includes_tenantless())
    .bind(needle)
    .bind(pattern.as_deref())
    .bind(folder_kind)
    .bind(folder_id)
    .fetch_all(pool)
    .await?;
    let visibility = Visibility::load(pool, &viewer).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let level = visibility.level(&row.target())?;
            Some(row.into_saved(level))
        })
        .collect())
}

/// One saved query, when the scope allows the caller to see it.
///
/// A query the caller may not see is `None`, not an error: the caller
/// learns nothing about a row that is not theirs.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn get_saved(
    pool: &PgPool,
    id: &str,
    scope: &SavedQueryScope,
) -> Result<Option<SavedQuery>, StoreError> {
    get_saved_inner(pool, id, scope, false).await
}

/// One saved query, live or waiting in the trash, when the scope allows
/// the caller to see it.
///
/// The trash entry is part of managing a query, so the route that brings
/// one back has to find it the same way the trash list finds it: with the
/// same scope and the same resolver. A live query is found by
/// [`get_saved`] alone.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn get_saved_including_trash(
    pool: &PgPool,
    id: &str,
    scope: &SavedQueryScope,
) -> Result<Option<SavedQuery>, StoreError> {
    get_saved_inner(pool, id, scope, true).await
}

/// The one read behind [`get_saved`] and [`get_saved_including_trash`].
///
/// `include_trash` decides whether a row that waits in the trash is
/// found; everything else — the tenant boundary and the level — is the
/// same rule for both, so the two cannot answer differently about the
/// same row.
async fn get_saved_inner(
    pool: &PgPool,
    id: &str,
    scope: &SavedQueryScope,
    include_trash: bool,
) -> Result<Option<SavedQuery>, StoreError> {
    let Some(viewer) = scope.effective_viewer() else {
        return Ok(None);
    };
    let Ok(query_id) = Uuid::parse_str(id) else {
        return Ok(None);
    };
    let row: Option<SavedQueryRow> = sqlx::query_as(
        "SELECT id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at \
         FROM saved_query \
         WHERE id = $1 AND ($4::boolean OR deleted_at IS NULL) \
           AND ($2::uuid IS NULL OR tenant_id = $2 OR ($3::boolean AND tenant_id IS NULL))",
    )
    .bind(query_id)
    .bind(scope.tenant_id())
    .bind(scope.includes_tenantless())
    .bind(include_trash)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let visibility = Visibility::load(pool, &viewer).await?;
    Ok(visibility
        .level(&row.target())
        .map(|level| row.into_saved(level)))
}

/// One `query_history` row whose text mentions a table name, for working
/// out who queried a catalog asset ([`history_mentioning`]).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct HistoryMention {
    /// `query_history.id`.
    pub id: String,
    /// The SQL that was run.
    pub sql: String,
    /// Who ran it, as displayed.
    pub user_name: String,
    /// The user who ran it, when recorded.
    pub owner_id: Option<Uuid>,
    /// When it ran, ISO 8601.
    pub at: String,
    /// `completed | failed | cancelled | blocked`.
    pub status: String,
    /// How long it took, in milliseconds.
    pub duration_ms: i64,
    /// The audit event recorded for the run, if any.
    pub audit_event_id: Option<String>,
}

/// The most [`history_mentioning`] reads in one call. A table queried more
/// often than this inside the window is under-counted, not mis-reported
/// per row. Public so the asset page can say "based on the newest 5,000
/// queries" when a read came back full (`DATA-15` F2).
pub const HISTORY_MENTION_LIMIT: i64 = 5000;

/// Every query run in the last `days` days, by anyone, whose SQL contains
/// one of `needles` (case-insensitively), newest first.
///
/// A substring match is only a first cut: `orders` also finds
/// `orders_archive` and a string literal. The caller parses each row's SQL
/// to decide which really read the table — `query_history` has no column
/// saying which tables a query touched, and this needs none.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn history_mentioning(
    pool: &PgPool,
    days: i32,
    needles: &[String],
) -> Result<Vec<HistoryMention>, StoreError> {
    if needles.is_empty() {
        return Ok(Vec::new());
    }
    // `%`, `_` and `\` in a table name are literals, not wildcards.
    let patterns: Vec<String> = needles.iter().map(|n| escape_like_pattern(n)).collect();
    let rows: Vec<HistoryMention> = sqlx::query_as(
        "SELECT q.id, q.sql, q.user_name, q.owner_id, \
                to_char(q.at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS at, \
                q.status, q.duration_ms, ae.id AS audit_event_id \
         FROM query_history q \
         LEFT JOIN LATERAL ( \
             SELECT ae.id FROM audit_event ae \
              WHERE ae.resource_kind = 'query_history' AND ae.resource_id = q.id \
              ORDER BY ae.at DESC LIMIT 1 \
         ) ae ON true \
         WHERE q.at >= now() - make_interval(days => $1) \
           AND q.sql ILIKE ANY($2) \
         ORDER BY q.at DESC LIMIT $3",
    )
    .bind(days)
    .bind(&patterns)
    .bind(HISTORY_MENTION_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The SQL text of the newest `limit` queries run in the last `days` days,
/// by anyone, newest first.
///
/// For the catalog's use ranking (`DATA-11` D4): one read per rebuild of
/// the search copy, parsed once in the API, in place of one
/// [`history_mentioning`] per asset. Text only: the ranking needs which
/// tables a query reads, never who ran it, so no user, owner or status
/// leaves this function.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn history_recent(
    pool: &PgPool,
    days: i32,
    limit: i64,
) -> Result<Vec<String>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT sql FROM query_history \
         WHERE at >= now() - make_interval(days => $1) \
         ORDER BY at DESC LIMIT $2",
    )
    .bind(days)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(sql,)| sql).collect())
}

/// Where a new saved query lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedQueryPlacement {
    /// The caller's **personal space** — the folder of the caller,
    /// created on first use.
    PersonalSpace,
    /// The organisation's **shared space**: the root of the tree, where
    /// every member of the organisation sees the query at `view`.
    SharedSpace,
    /// The folder named.
    Folder(Uuid),
}

/// Insert a new saved query in the place the caller asked for.
///
/// The row belongs to the tenant the viewer acts as, and to nobody else
/// (`SEC-20`). The author is the viewer's own id, so the author manages
/// what they wrote — the rule itself, not a row of rules.
///
/// The first version is written in the same transaction as the row, so the
/// history of a query never starts later than the query itself.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn create_saved_query_at(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
    title: &str,
    sql: &str,
    owner: &str,
    tags: &[String],
    placement: SavedQueryPlacement,
) -> Result<SavedQuery, StoreError> {
    let folder_id = match placement {
        SavedQueryPlacement::Folder(folder_id) => Some(folder_id),
        SavedQueryPlacement::SharedSpace => None,
        SavedQueryPlacement::PersonalSpace => match (viewer.id, viewer.tenant_id) {
            (Some(person), Some(tenant_id)) => Some(
                ensure_personal_space(pool, tenant_id, person, owner)
                    .await?
                    .id,
            ),
            // No tenant, with or without a person: the shared space is
            // the only place such a row can live. A personal space hangs
            // off a tenant, so a row that belongs to no tenant has no
            // personal space to land in.
            _ => None,
        },
    };
    let mut tx = pool.begin().await?;
    let row: SavedQueryRow = sqlx::query_as(
        "INSERT INTO saved_query (title, sql, owner, tags, owner_id, tenant_id, folder_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at",
    )
    .bind(title)
    .bind(sql)
    .bind(owner)
    .bind(tags)
    .bind(viewer.id)
    .bind(viewer.tenant_id)
    .bind(folder_id)
    .fetch_one(&mut *tx)
    .await?;
    let author = version_author_name(&mut tx, viewer.id, owner).await?;
    write_version(
        &mut tx, row.id, &row.title, &row.sql, viewer.id, &author, "",
    )
    .await?;
    tx.commit().await?;
    let visibility = Visibility::load(pool, viewer).await?;
    let level = visibility.level(&row.target()).unwrap_or(Level::View);
    Ok(row.into_saved(level))
}

/// Insert a new saved query in the folder named, or in the caller's
/// personal space when no folder is named.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn create_saved_query(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
    title: &str,
    sql: &str,
    owner: &str,
    tags: &[String],
    folder_id: Option<Uuid>,
) -> Result<SavedQuery, StoreError> {
    let placement = folder_id.map_or(
        SavedQueryPlacement::PersonalSpace,
        SavedQueryPlacement::Folder,
    );
    create_saved_query_at(pool, viewer, title, sql, owner, tags, placement).await
}

/// What one update may change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedQueryPatch {
    /// A new title.
    pub title: Option<String>,
    /// New SQL.
    pub sql: Option<String>,
    /// New tags.
    pub tags: Option<Vec<String>>,
    /// The folder the query moves to. `None` leaves it where it is.
    pub folder: Option<FolderMove>,
}

/// Where a query moves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderMove {
    /// Into this folder.
    Into(Uuid),
    /// Back to the shared space, the root.
    Root,
}

/// Change a saved query.
///
/// The caller must hold **edit** on the query: `edit` changes the SQL,
/// the title and the tags, `view` may do neither. A move to another folder
/// (or to the root) needs **manage**. A caller
/// that may not change the row finds no row at all, so the answer never
/// says whether it exists.
///
/// A version is written for every change of the title or the SQL, in the
/// same transaction as the change; a change that leaves both as they
/// were writes none.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] when the caller may not change the
/// query, and when the folder a move names belongs to another tenant than
/// the query's — an unseen folder; [`StoreError::NotAllowed`] when the
/// patch moves the query to a folder other than the one it is in and the
/// caller does not manage the query; and [`StoreError::Database`] if a query fails.
pub async fn update_saved_query(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
    id: &str,
    patch: &SavedQueryPatch,
) -> Result<SavedQuery, StoreError> {
    let Ok(query_id) = Uuid::parse_str(id) else {
        return Err(StoreError::NotFound);
    };
    let Some(row) = load_row(pool, query_id).await? else {
        return Err(StoreError::NotFound);
    };
    // A trashed query is not changed: it waits in the trash until it is
    // restored.
    if row.deleted_at.is_some() || !writable_by(row.tenant_id, viewer) {
        return Err(StoreError::NotFound);
    }
    let visibility = Visibility::load(pool, viewer).await?;
    let Some(level) = visibility.level(&row.target()) else {
        return Err(StoreError::NotFound);
    };
    if level < Level::Edit {
        return Err(StoreError::NotFound);
    }
    let title = patch.title.as_deref().unwrap_or(&row.title);
    let sql = patch.sql.as_deref().unwrap_or(&row.sql);
    let tags = patch.tags.as_deref().unwrap_or(&row.tags);
    // A named move is re-checked here, before any write, the way the
    // edit level above is (`QS-3` review of #382): the folder a query moves into
    // belongs to the query's own tenant, and a move needs manage on the
    // query, wherever it goes. Edit changes what the query says; manage
    // also decides who sees it, so an edit holder can neither publish a
    // private query, hide a shared one, nor raise edit to manage by moving
    // the query into a folder of their own.
    let moved = patch.folder.map(|move_to| match move_to {
        FolderMove::Into(folder_id) => Some(folder_id),
        FolderMove::Root => None,
    });
    if let Some(target) = moved {
        // The same tenant limiting a folder's new parent gets: only the
        // row's own tenant's folders are reachable, so a folder of another
        // tenant is an unseen folder.
        let folders = list_folders(pool, row.tenant_id).await?;
        if let Some(folder_id) = target
            && !folders.iter().any(|folder| folder.id == folder_id)
        {
            return Err(StoreError::NotFound);
        }
        // A patch that names the folder the query is already in is not a
        // move, so an editor who renames with the folder sent again is
        // not refused. The refusal is the store's own fixed sentence,
        // mapped to 403 — never a 404, which would say the query is not
        // there at all.
        if target != row.folder_id && level < Level::Manage {
            return Err(StoreError::NotAllowed);
        }
    }
    let folder_id = moved.unwrap_or(row.folder_id);
    let mut tx = pool.begin().await?;
    let updated: SavedQueryRow = sqlx::query_as(
        "UPDATE saved_query \
         SET title = $2, sql = $3, tags = $4, folder_id = $5, updated_at = now() \
         WHERE id = $1 \
         RETURNING id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at",
    )
    .bind(query_id)
    .bind(title)
    .bind(sql)
    .bind(tags)
    .bind(folder_id)
    .fetch_one(&mut *tx)
    .await?;
    if title != row.title || sql != row.sql {
        let author = version_author_name(&mut tx, viewer.id, &row.owner).await?;
        write_version(&mut tx, query_id, title, sql, viewer.id, &author, "").await?;
    }
    tx.commit().await?;
    Ok(updated.into_saved(level))
}

/// Move a saved query to the trash.
///
/// The caller must hold **edit** on the query: decision 6 (plan 55 D5)
/// puts deleting a query beside changing it. Trashing a query that is
/// already in the trash does nothing.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] when the caller may not delete the
/// query, and [`StoreError::Database`] if a query fails.
pub async fn trash_saved_query(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
    id: &str,
) -> Result<(), StoreError> {
    let Ok(query_id) = Uuid::parse_str(id) else {
        return Err(StoreError::NotFound);
    };
    let Some(row) = load_row(pool, query_id).await? else {
        return Err(StoreError::NotFound);
    };
    if !writable_by(row.tenant_id, viewer) {
        return Err(StoreError::NotFound);
    }
    let visibility = Visibility::load(pool, viewer).await?;
    let Some(level) = visibility.level(&row.target()) else {
        return Err(StoreError::NotFound);
    };
    // Edit may delete a query (decision 6, plan 55 D5).
    if level < Level::Edit {
        return Err(StoreError::NotFound);
    }
    if row.deleted_at.is_some() {
        return Ok(());
    }
    sqlx::query("UPDATE saved_query SET deleted_at = now(), deleted_by = $2 WHERE id = $1")
        .bind(query_id)
        .bind(viewer.id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Bring a saved query back out of the trash, into the folder it was in.
///
/// The caller must hold **edit** on the query: whoever may delete it may
/// bring it back (decision 6, plan 55 D5).
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] when the caller may not restore the
/// query, and [`StoreError::Database`] if a query fails.
pub async fn restore_saved_query(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
    id: &str,
) -> Result<SavedQuery, StoreError> {
    let Ok(query_id) = Uuid::parse_str(id) else {
        return Err(StoreError::NotFound);
    };
    let Some(row) = load_row(pool, query_id).await? else {
        return Err(StoreError::NotFound);
    };
    if !writable_by(row.tenant_id, viewer) {
        return Err(StoreError::NotFound);
    }
    let visibility = Visibility::load(pool, viewer).await?;
    let Some(level) = visibility.level(&row.target()) else {
        return Err(StoreError::NotFound);
    };
    // Edit may restore a query (decision 6, plan 55 D5).
    if level < Level::Edit {
        return Err(StoreError::NotFound);
    }
    let restored: SavedQueryRow = sqlx::query_as(
        "UPDATE saved_query \
         SET deleted_at = NULL, deleted_by = NULL, updated_at = now() \
         WHERE id = $1 \
         RETURNING id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at",
    )
    .bind(query_id)
    .fetch_one(pool)
    .await?;
    Ok(restored.into_saved(level))
}

/// How long a trashed query waits before the sweep takes it away.
///
/// The same window the dashboards' trash keeps (`lakehouse-bi`'s
/// `RETENTION_DAYS`), so a saved query and a dashboard wait the same
/// length of time before they are gone for good.
pub const TRASH_RETENTION_DAYS: i32 = 30;

/// One entry in the trash: the query, the folder it waits in, and the
/// days it has left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryTrashEntry {
    /// The query's id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// The folder it sits in; `None` is the shared space.
    pub folder_id: Option<String>,
    /// When it went to the trash, ISO 8601.
    pub deleted_at: String,
    /// How many days it has left before the sweep takes it away.
    pub days_left: i32,
}

/// The queries in the trash the caller may edit (decision 6, plan 55
/// D5).
///
/// A trashed query waits here until it is restored or swept away; the
/// caller sees the ones it may edit — the same set that may delete and
/// restore a query — so what the list shows is what the caller can act
/// on.
///
/// The days left are counted here rather than by the caller, so the
/// number the console shows and the number the sweep acts on come from
/// the same constant.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn list_saved_trash_entries(
    pool: &PgPool,
    scope: &SavedQueryScope,
) -> Result<Vec<SavedQueryTrashEntry>, StoreError> {
    let Some(viewer) = scope.effective_viewer() else {
        return Ok(Vec::new());
    };
    let rows: Vec<SavedQueryRow> = sqlx::query_as(
        "SELECT id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at \
         FROM saved_query \
         WHERE deleted_at IS NOT NULL \
           AND ($1::uuid IS NULL OR tenant_id = $1 OR ($2::boolean AND tenant_id IS NULL)) \
         ORDER BY deleted_at DESC",
    )
    .bind(scope.tenant_id())
    .bind(scope.includes_tenantless())
    .fetch_all(pool)
    .await?;
    let visibility = Visibility::load(pool, &viewer).await?;
    let now = OffsetDateTime::now_utc();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let level = visibility.level(&row.target())?;
            // The trash lists the queries the caller may edit, the same
            // level delete and restore ask for (decision 6, plan 55 D5).
            if level < Level::Edit {
                return None;
            }
            let deleted_at = row.deleted_at?;
            Some(SavedQueryTrashEntry {
                id: row.id.to_string(),
                title: row.title,
                folder_id: row.folder_id.map(|id| id.to_string()),
                deleted_at: iso_millis(deleted_at),
                days_left: days_left(deleted_at, now),
            })
        })
        .collect())
}

/// The days a trashed query has left, counted from when it went to the
/// trash.
///
/// Whole days, never a fraction: a query trashed a moment ago has 29
/// days left, not 29.99. A query that is past the window has none.
fn days_left(deleted_at: OffsetDateTime, now: OffsetDateTime) -> i32 {
    let deadline = deleted_at + time::Duration::days(i64::from(TRASH_RETENTION_DAYS));
    let seconds = (deadline - now).whole_seconds();
    if seconds <= 0 {
        return 0;
    }
    i32::try_from(seconds / 86_400).unwrap_or(i32::MAX)
}

/// Delete the queries that were trashed before `cutoff`, and answer how
/// many went.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails. Nothing is
/// deleted when it does.
pub async fn purge_trashed_before(
    pool: &PgPool,
    cutoff: OffsetDateTime,
) -> Result<u64, StoreError> {
    let removed =
        sqlx::query("DELETE FROM saved_query WHERE deleted_at IS NOT NULL AND deleted_at < $1")
            .bind(cutoff)
            .execute(pool)
            .await?;
    Ok(removed.rows_affected())
}

/// One version of a saved query: the title and the SQL it had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryVersion {
    /// The version's number: 1 for the first.
    pub version: i32,
    /// The title the query had.
    pub title: String,
    /// The SQL the query had.
    pub sql: String,
    /// The author's user id, when there is one.
    pub author_id: Option<String>,
    /// The author's name, as the history shows it.
    pub author_name: String,
    /// What the history says about this version: "Restored version 3",
    /// or nothing for a change that was only a change.
    pub label: String,
    /// When it was written, ISO 8601.
    pub created_at: String,
}

/// A version, as the database holds it.
#[derive(Debug, FromRow)]
struct SavedQueryVersionRow {
    version: i32,
    title: String,
    sql: String,
    author_id: Option<Uuid>,
    author_name: String,
    label: String,
    created_at: OffsetDateTime,
}

impl From<SavedQueryVersionRow> for SavedQueryVersion {
    fn from(row: SavedQueryVersionRow) -> Self {
        Self {
            version: row.version,
            title: row.title,
            sql: row.sql,
            author_id: row.author_id.map(|id| id.to_string()),
            author_name: row.author_name,
            label: row.label,
            created_at: iso_millis(row.created_at),
        }
    }
}

/// The versions kept per query: the newest [`VERSION_KEEP`] of them.
pub const VERSION_KEEP: usize = 15;

/// The history of one query, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn list_versions(
    pool: &PgPool,
    query_id: Uuid,
) -> Result<Vec<SavedQueryVersion>, StoreError> {
    let rows: Vec<SavedQueryVersionRow> = sqlx::query_as(
        "SELECT version, title, sql, author_id, author_name, label, created_at \
         FROM saved_query_version WHERE query_id = $1 ORDER BY version DESC",
    )
    .bind(query_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(SavedQueryVersion::from).collect())
}

/// Go back to an earlier version of a saved query.
///
/// The revert writes a **new** version, holding the content of the
/// version it went back to, so going back is itself undoable.
///
/// The caller must hold **edit** on the query.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] when the caller may not change the
/// query or when the version does not exist, and [`StoreError::Database`]
/// if a query fails.
pub async fn revert_to_version(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
    query_id: Uuid,
    version: i32,
) -> Result<SavedQuery, StoreError> {
    let Some(row) = load_row(pool, query_id).await? else {
        return Err(StoreError::NotFound);
    };
    if row.deleted_at.is_some() || !writable_by(row.tenant_id, viewer) {
        return Err(StoreError::NotFound);
    }
    let visibility = Visibility::load(pool, viewer).await?;
    let Some(level) = visibility.level(&row.target()) else {
        return Err(StoreError::NotFound);
    };
    if level < Level::Edit {
        return Err(StoreError::NotFound);
    }
    let wanted: Option<(String, String)> = sqlx::query_as(
        "SELECT title, sql FROM saved_query_version WHERE query_id = $1 AND version = $2",
    )
    .bind(query_id)
    .bind(version)
    .fetch_optional(pool)
    .await?;
    let Some((title, sql)) = wanted else {
        return Err(StoreError::NotFound);
    };
    let mut tx = pool.begin().await?;
    let updated: SavedQueryRow = sqlx::query_as(
        "UPDATE saved_query SET title = $2, sql = $3, updated_at = now() \
         WHERE id = $1 \
         RETURNING id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at",
    )
    .bind(query_id)
    .bind(&title)
    .bind(&sql)
    .fetch_one(&mut *tx)
    .await?;
    let author = version_author_name(&mut tx, viewer.id, &row.owner).await?;
    write_version(
        &mut tx,
        query_id,
        &title,
        &sql,
        viewer.id,
        &author,
        &format!("Restored version {version}"),
    )
    .await?;
    tx.commit().await?;
    Ok(updated.into_saved(level))
}

/// One row, with the columns the access rules need.
async fn load_row(pool: &PgPool, query_id: Uuid) -> Result<Option<SavedQueryRow>, StoreError> {
    let row: Option<SavedQueryRow> = sqlx::query_as(
        "SELECT id, title, sql, owner, updated_at, tags, folder_id, owner_id, tenant_id, deleted_at \
         FROM saved_query WHERE id = $1",
    )
    .bind(query_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Whether the caller may write a row of that tenant.
///
/// The tenant is the outer boundary: a caller that acts as one tenant
/// writes that tenant's rows and no others. A caller that acts as no
/// tenant in particular — the scope that reads every tenant's rows —
/// reaches every row, exactly as it reads them.
fn writable_by(tenant_id: Option<Uuid>, viewer: &SavedQueryViewer) -> bool {
    match viewer.tenant_id {
        Some(caller) => tenant_id == Some(caller),
        None => true,
    }
}

/// Write one version of a query and drop the ones that fell out of the
/// history.
///
/// The number is the highest the query has plus one, and the history
/// keeps the newest [`VERSION_KEEP`] versions: the older ones go in the
/// same transaction as the version that pushed them out.
async fn write_version(
    tx: &mut Transaction<'_, Postgres>,
    query_id: Uuid,
    title: &str,
    sql: &str,
    author_id: Option<Uuid>,
    author_name: &str,
    label: &str,
) -> Result<(), StoreError> {
    let next: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM saved_query_version WHERE query_id = $1",
    )
    .bind(query_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO saved_query_version \
         (query_id, version, title, sql, author_id, author_name, label) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(query_id)
    .bind(next)
    .bind(title)
    .bind(sql)
    .bind(author_id)
    .bind(author_name)
    .bind(label)
    .execute(&mut **tx)
    .await?;
    let keep = i32::try_from(VERSION_KEEP).unwrap_or(i32::MAX);
    sqlx::query("DELETE FROM saved_query_version WHERE query_id = $1 AND version <= $2")
        .bind(query_id)
        .bind(next - keep)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The name the history shows for the author: the person's own name in
/// `app_user`, or the display name the row itself carries.
async fn version_author_name(
    tx: &mut Transaction<'_, Postgres>,
    author_id: Option<Uuid>,
    fallback: &str,
) -> Result<String, StoreError> {
    let Some(author_id) = author_id else {
        return Ok(fallback.to_owned());
    };
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM app_user WHERE id = $1")
        .bind(author_id)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(name.unwrap_or_else(|| fallback.to_owned()))
}

// ── Query history (written by `routes::query::run`, read by listHistory) ─

/// One recorded query execution. Mirrors `QueryHistoryItem` in
/// `contracts/queries.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryHistoryItem {
    /// `query_history.id` — the same `q-<epoch_ms>` id the run that
    /// produced this row returned as `QueryResult.id`.
    pub id: String,
    /// The executed SQL text.
    pub sql: String,
    /// Who ran the query, as a display name. Ownership for scoping
    /// [`list_history`] lives in the separate `owner_id` column
    /// (`0046_query_ownership.sql`), not here — this stays a display
    /// string even for a row recorded before that migration, which still
    /// carries the earlier fixed placeholder `"anonymous"`.
    pub user: String,
    /// When the query ran, ISO 8601.
    pub at: String,
    /// `"completed" | "failed" | "cancelled" | "blocked"`.
    pub status: String,
    /// Wall-clock execution time. Serializes as `durationMs`.
    pub duration_ms: i64,
    /// Bytes scanned. Serializes as `scannedBytes`.
    pub scanned_bytes: i64,
    /// Estimated cost units consumed. Serializes as `costUnits`.
    pub cost_units: f64,
    /// The workload classification. Serializes as `workloadClass`.
    pub workload_class: String,
    /// Which storage engine served the query.
    pub engine: String,
    /// Whether the result was served (partly) from cache. Serializes as
    /// `cacheAssisted`.
    pub cache_assisted: bool,
    /// The tables the statement names as written, as
    /// `sql_rewrite::referenced_tables` parsed them when the run was
    /// recorded (`QS-4`, decision 4). Empty for a statement that does not
    /// parse, and for every row recorded before
    /// `0111_query_history_detail.sql`, which backfills nothing.
    /// Serializes as `tablesRead`.
    pub tables_read: Vec<String>,
    /// The exact text the response gave the caller for a failed or refused
    /// run — fixed and classified before it was stored, never the engine's
    /// raw text (principle 4). `None` for a completed run and for every
    /// row recorded before `0111_query_history_detail.sql`. Omitted from
    /// JSON when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The newest real `audit_event` row recorded against this history row
    /// (`resource_kind = 'query_history'`, `resource_id = id`), resolved on
    /// read via a lateral join in [`list_history`] — never stored. `None`
    /// until WS5 starts recording query-run audit events (see the module
    /// doc comment and `list_history`'s doc comment for why no producer
    /// writes this column anymore). Omitted from JSON when absent.
    /// Serializes as `auditEventId`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
    /// The real owner (`0046_query_ownership.sql`), used by
    /// `routes::query::download`'s ownership check. Never serialized: the
    /// `TypeScript` contract has no `ownerId` field, and every row
    /// `list_history` returns already belongs to the caller, so echoing it
    /// back would be redundant at best.
    #[serde(skip)]
    pub owner_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct QueryHistoryRow {
    id: String,
    sql: String,
    user_name: String,
    at: OffsetDateTime,
    status: String,
    duration_ms: i64,
    scanned_bytes: i64,
    cost_units: f64,
    workload_class: String,
    engine: String,
    cache_assisted: bool,
    tables_read: Vec<String>,
    message: Option<String>,
    audit_event_id: Option<String>,
    owner_id: Option<Uuid>,
}

impl From<QueryHistoryRow> for QueryHistoryItem {
    fn from(row: QueryHistoryRow) -> Self {
        Self {
            id: row.id,
            sql: row.sql,
            user: row.user_name,
            at: iso_millis(row.at),
            status: row.status,
            duration_ms: row.duration_ms,
            scanned_bytes: row.scanned_bytes,
            cost_units: row.cost_units,
            workload_class: row.workload_class,
            engine: row.engine,
            cache_assisted: row.cache_assisted,
            tables_read: row.tables_read,
            message: row.message,
            audit_event_id: row.audit_event_id,
            owner_id: row.owner_id,
        }
    }
}

/// The filters the history page sends (`QS-4`, decision 5). The route
/// parses and validates the query string into this; [`list_history`] binds
/// every value, so its SQL is one constant statement. `None` means "no
/// filter on this axis", and the all-`None` default is the side panel's
/// and the page's first view: the newest page of the last 14 days.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryFilter<'a> {
    /// `completed | failed | cancelled | blocked` — the statuses the
    /// table's own check allows. The route rejects anything else with a
    /// 400 before this is built.
    pub status: Option<&'a str>,
    /// `clickhouse | trino` — the engines `run` records. The route
    /// rejects anything else with a 400 before this is built.
    pub engine: Option<&'a str>,
    /// Only runs that took at least this many milliseconds.
    pub min_duration_ms: Option<i64>,
    /// A table the run read: matched against every element of
    /// `tables_read`, case-insensitively, as a contains-match in which
    /// `%` and `_` are literal (decision 7).
    pub table: Option<&'a str>,
    /// The window's start. [`list_history`] clamps it to now minus 14
    /// days (decision 2): the page shows the last fortnight, whatever the
    /// caller asks for.
    pub from: Option<OffsetDateTime>,
    /// The window's end, inclusive.
    pub to: Option<OffsetDateTime>,
    /// The page cursor's `at` half: the `at` of the last row the caller
    /// has already seen. With [`HistoryFilter::before_id`] it pages
    /// without repeats or skips.
    pub before_at: Option<OffsetDateTime>,
    /// The page cursor's `id` half, breaking ties between rows that share
    /// an `at` (rows land in one millisecond more often than a cursor
    /// built on `at` alone could survive).
    pub before_id: Option<&'a str>,
}

/// The page size of [`list_history`]: the most rows any one answer
/// carries. "Load more" asks for the next page with the cursor (decision
/// 5), so no answer grows as `routes::query::run` keeps inserting, and
/// reaching an older row is what the filters are for — the side panel's
/// old first bite of 200 is not kept.
const HISTORY_PAGE_LIMIT: i64 = 50;

/// List one user's recorded query executions, filtered, most recent
/// first, one page at a time.
///
/// History holds the SQL people typed, which is as private as the chat
/// sessions in `console.chat_session`: before `owner_id` existed this
/// listed the whole table, so everyone read everyone else's queries. Rows
/// recorded before then have no owner and are returned to nobody.
///
/// # One constant statement
///
/// Every filter is a `$n IS NULL OR ...` arm, so the SQL never changes
/// shape and every value is bound — `format!` has nothing to do here
/// (decision 5; the repo rule that only constant identifiers may be
/// interpolated). An absent filter costs Postgres one null test per row
/// and buys a statement whose shape Postgres can learn once.
///
/// # The 14-day window (decision 2)
///
/// `GREATEST(COALESCE($6, now() - interval '14 days'), now() - interval
/// '14 days')` floors the window at now minus 14 days: Postgres's
/// `GREATEST` ignores null arguments, so a null `from` falls to the floor
/// and a `from` older than the floor is clamped to it. Nothing is
/// deleted — an older row is still in the table, it is just not listed.
///
/// # The cursor (decision 5)
///
/// `(date_trunc('milliseconds', q.at), q.id) < ($8::timestamptz, $9::text)`
/// under `ORDER BY date_trunc('milliseconds', q.at) DESC, q.id DESC` pages
/// by the `at` *and* `id` of the last row seen: a cursor on `at` alone
/// would repeat or skip every row that shares its millisecond.
///
/// The millisecond is the value the cursor carries: a row's `at` leaves
/// the store cut to milliseconds ([`iso_millis`]), so the console's cursor
/// holds `T.000` for a row at `T.000150`. Comparing that millisecond
/// against the stored microsecond `at` skips every row inside the last
/// page's millisecond — `T.000100` is not below `T.000` — so the order
/// and the comparison both run on `date_trunc('milliseconds', q.at)`, the
/// value the cursor holds.
///
/// # The `table` match (decision 7)
///
/// `%` and `_` in the caller's text are matched literally: the shared
/// `escape_like_pattern` helper escapes the escape character first, then
/// the two wildcards, and `ILIKE` does the case-insensitive contains-match
/// against every element of `tables_read`.
///
/// # `auditEventId` resolution (WS1 task 1.14, judge finding J12)
///
/// `query_history.audit_event_id` is no longer written — `routes::query::run`
/// used to mint a `aud-query-<id>` string that named no `audit_event` row,
/// so every "View audit" link built from it 404ed. This query instead
/// resolves the id through a `LEFT JOIN LATERAL` against `audit_event`,
/// keyed on `resource_kind = 'query_history'` and `resource_id = id`, taking
/// the newest matching event (several can exist for one query run). A bare
/// `LEFT JOIN` would duplicate the history row once per matching event; the
/// lateral subquery's `LIMIT 1` keeps the row count exact. The composite
/// index `audit_event_resource_idx (resource_kind, resource_id)`
/// (`0024_audit_event.sql`) covers this lookup. Nothing inserts a
/// `query_history` audit event yet (WS5), so `auditEventId` is `None` for
/// every row until then — that is honest, not a bug.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_history(
    pool: &PgPool,
    owner_id: Uuid,
    filter: &HistoryFilter<'_>,
) -> Result<Vec<QueryHistoryItem>, StoreError> {
    let rows: Vec<QueryHistoryRow> = sqlx::query_as(
        "SELECT q.id, q.sql, q.user_name, q.at, q.status, q.duration_ms, q.scanned_bytes, \
         q.cost_units, q.workload_class, q.engine, q.cache_assisted, ae.id AS audit_event_id, \
         q.owner_id, q.tables_read, q.message \
         FROM query_history q \
         LEFT JOIN LATERAL ( \
             SELECT ae.id FROM audit_event ae \
              WHERE ae.resource_kind = 'query_history' AND ae.resource_id = q.id \
              ORDER BY ae.at DESC LIMIT 1 \
         ) ae ON true \
         WHERE q.owner_id = $1 \
           AND ($2::text IS NULL OR q.status = $2) \
           AND ($3::text IS NULL OR q.engine = $3) \
           AND ($4::bigint IS NULL OR q.duration_ms >= $4) \
           AND ($5::text IS NULL OR EXISTS ( \
               SELECT 1 FROM unnest(q.tables_read) AS t(t) \
                WHERE t.t ILIKE '%' || replace(replace(replace($5, '\\', '\\\\'), '%', '\\%'), '_', '\\_') || '%')) \
           AND q.at >= GREATEST(COALESCE($6::timestamptz, now() - interval '14 days'), \
                                now() - interval '14 days') \
           AND ($7::timestamptz IS NULL OR q.at <= $7) \
           AND ($8::timestamptz IS NULL OR (date_trunc('milliseconds', q.at), q.id) < ($8::timestamptz, $9::text)) \
         ORDER BY date_trunc('milliseconds', q.at) DESC, q.id DESC \
         LIMIT $10",
    )
    .bind(owner_id)
    .bind(filter.status)
    .bind(filter.engine)
    .bind(filter.min_duration_ms)
    .bind(filter.table)
    .bind(filter.from)
    .bind(filter.to)
    .bind(filter.before_at)
    .bind(filter.before_id)
    .bind(HISTORY_PAGE_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(QueryHistoryItem::from).collect())
}

/// Fetch one query-history row by id — added for an owner-scoped lookup
/// (a later task checks whether the caller downloading a result is the
/// same principal `run` recorded as `user`, now that `user` is a real
/// principal uuid rather than the fixed `"anonymous"` placeholder). Mirrors
/// [`list_history`]'s lateral `audit_event` resolution (see that
/// function's doc comment: `audit_event_id` is never stored, only resolved
/// on read) so a single-row lookup and the list agree on the same value.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_history_item(
    pool: &PgPool,
    id: &str,
) -> Result<Option<QueryHistoryItem>, StoreError> {
    let row: Option<QueryHistoryRow> = sqlx::query_as(
        "SELECT q.id, q.sql, q.user_name, q.at, q.status, q.duration_ms, q.scanned_bytes, \
         q.cost_units, q.workload_class, q.engine, q.cache_assisted, ae.id AS audit_event_id, \
         q.owner_id, q.tables_read, q.message \
         FROM query_history q \
         LEFT JOIN LATERAL ( \
             SELECT ae.id FROM audit_event ae \
              WHERE ae.resource_kind = 'query_history' AND ae.resource_id = q.id \
              ORDER BY ae.at DESC LIMIT 1 \
         ) ae ON true \
         WHERE q.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(QueryHistoryItem::from))
}

/// Everything [`record_history`] needs to write one row.
#[derive(Debug, Clone)]
pub struct RecordHistoryInput<'a> {
    /// The id to record the row under — `routes::query::run` passes the
    /// same `q-<epoch_ms>` id it hands back as `QueryResult.id`, so a
    /// history row and the result that produced it share one identifier.
    pub id: &'a str,
    /// The executed SQL text.
    pub sql: &'a str,
    /// Who ran the query, as a display name.
    pub user: &'a str,
    /// The user the row belongs to. `None` for a run with no authenticated
    /// caller, which [`list_history`] then shows to nobody.
    pub owner_id: Option<Uuid>,
    /// `"completed" | "failed" | "cancelled" | "blocked"`.
    pub status: &'a str,
    /// Wall-clock execution time, in milliseconds.
    pub duration_ms: i64,
    /// Bytes scanned.
    pub scanned_bytes: i64,
    /// Estimated cost units consumed.
    pub cost_units: f64,
    /// The workload classification.
    pub workload_class: &'a str,
    /// Which storage engine served the query.
    pub engine: &'a str,
    /// Whether the result was served (partly) from cache.
    pub cache_assisted: bool,
    /// The tables the statement names as written, as the caller parsed
    /// them. Empty for a statement that does not parse, and for every row
    /// recorded before the column existed (migration
    /// `0111_query_history_detail.sql` backfills nothing).
    pub tables_read: &'a [String],
    /// The exact text the response carried for a failed or refused run —
    /// fixed and classified before it is stored, the same text the
    /// caller's screen showed. `None` where there is nothing to say: a
    /// completed run, and every row recorded before the column existed.
    pub message: Option<&'a str>,
}

/// Record one query execution.
///
/// # Non-fatal by design
///
/// This is called from `routes::query::run` once the run has an answer —
/// a completed run after its result is built, and (since the history
/// page, `QS-4`) a refused or failed run before its error response goes
/// out, carrying the same message that response does. A logging write
/// failing (no pool configured, Postgres down, whatever) must never turn
/// an otherwise-settled run into a different response — so
/// `routes::query::run` calls this, gets a `Result`, and on `Err` logs a
/// warning and still returns the response it had built, success or not.
/// The `Result` is returned rather than swallowed *here* so
/// that decision stays visible at the call site instead of this function
/// silently deciding it for every caller (a test, for instance, very much
/// wants to see the error).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert fails.
pub async fn record_history(
    pool: &PgPool,
    input: &RecordHistoryInput<'_>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO query_history \
         (id, sql, user_name, status, duration_ms, scanned_bytes, cost_units, \
          workload_class, engine, cache_assisted, owner_id, tables_read, message) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(input.id)
    .bind(input.sql)
    .bind(input.user)
    .bind(input.status)
    .bind(input.duration_ms)
    .bind(input.scanned_bytes)
    .bind(input.cost_units)
    .bind(input.workload_class)
    .bind(input.engine)
    .bind(input.cache_assisted)
    .bind(input.owner_id)
    .bind(input.tables_read)
    .bind(input.message)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The wire format is the contract: every key the browser reads must be
    /// the camelCase name `contracts/queries.ts` declares.
    #[test]
    fn serialized_field_names_match_the_typescript_contract() {
        let saved = SavedQuery {
            id: "s".to_owned(),
            title: "t".to_owned(),
            sql: "SELECT 1".to_owned(),
            owner: "o".to_owned(),
            updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
            tags: vec!["finance".to_owned()],
            folder_id: None,
            owner_id: None,
            level: "view".to_owned(),
        };
        let value = serde_json::to_value(&saved).unwrap();
        for key in [
            "id",
            "title",
            "sql",
            "owner",
            "updatedAt",
            "tags",
            "folderId",
            "ownerId",
            "level",
        ] {
            assert!(value.get(key).is_some(), "SavedQuery is missing `{key}`");
        }

        let history = QueryHistoryItem {
            id: "q".to_owned(),
            sql: "SELECT 1".to_owned(),
            user: "u".to_owned(),
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            status: "completed".to_owned(),
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics".to_owned(),
            engine: "hot-store".to_owned(),
            cache_assisted: true,
            tables_read: vec!["marts.retails".to_owned()],
            message: None,
            audit_event_id: Some("aud-1".to_owned()),
            owner_id: None,
        };
        let value = serde_json::to_value(&history).unwrap();
        for key in [
            "id",
            "sql",
            "user",
            "at",
            "status",
            "durationMs",
            "scannedBytes",
            "costUnits",
            "workloadClass",
            "engine",
            "cacheAssisted",
            "auditEventId",
        ] {
            assert!(
                value.get(key).is_some(),
                "QueryHistoryItem is missing `{key}`"
            );
        }
    }

    /// `auditEventId` is optional in the contract; a history row recorded
    /// without one must omit the key, not emit `null`.
    #[test]
    fn history_item_omits_absent_audit_event_id() {
        let item = QueryHistoryItem {
            id: "q".to_owned(),
            sql: "SELECT 1".to_owned(),
            user: "u".to_owned(),
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            status: "completed".to_owned(),
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics".to_owned(),
            engine: "hot-store".to_owned(),
            cache_assisted: false,
            tables_read: Vec::new(),
            message: None,
            audit_event_id: None,
            owner_id: None,
        };
        let value = serde_json::to_value(&item).unwrap();
        assert!(value.get("auditEventId").is_none());
    }
}
