//! Comments on dashboards (`BI-19` part E; plan
//! `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`, decision D15; feature
//! page `docs/core/features/sharing-and-delivery.md`, E1 to E3).
//!
//! # Routes
//!
//! | Route | Policy | What |
//! | --- | --- | --- |
//! | `GET /api/dashboard/comments?board=` | signed in, `view` on the dashboard | the dashboard's comments, oldest first |
//! | `POST /api/dashboard/comments` | signed in, `view` | post one, with the people it mentions |
//! | `PUT /api/dashboard/comments/{id}` | signed in, `view`, the author | change its text and mentions |
//! | `DELETE /api/dashboard/comments/{id}` | signed in, `view`, the author or `manage` | remove it |
//! | `GET /api/dashboard/comments/people?board=&q=` | signed in, `view` | the @ picker: people who can see the dashboard |
//! | `POST /api/dashboard/comments/mentions/read` | signed in | mark the caller's mentions read (the bell) |
//!
//! The router asks only "signed in": the level is a property of the dashboard,
//! so the handlers work it out with the same [`Scope`] the other BI-19 routes
//! use (`dashboard_access::scope_for_write`).
//!
//! # What a stranger can tell
//!
//! A comment is as visible as its dashboard and no more. A dashboard the caller
//! cannot see, one in the trash and one that never existed all answer
//! `404 dashboard not found.`; a comment id the caller cannot reach (unknown,
//! deleted, or on a dashboard they cannot see) answers `404 comment not found.`
//! for all three. Comments follow the board: while it is in the trash every
//! route above is a 404 for them, and when it is restored they are back.
//!
//! # Mentions (decision D15, feature E2)
//!
//! Every mentioned person must be able to see the dashboard **as themselves**
//! (their own roles, rules and the restrictions), be active, and share a tenant
//! with the author, or the comment is refused with one sentence that does not
//! say which of those failed and does not name the person. The author already
//! sees the dashboard, so the sentence tells them nothing about it; the person
//! mentioned is told nothing at all. The check runs when the comment is posted
//! (and for people added by an edit); access removed later is handled when the
//! mention is read (the bell re-checks), not by rewriting the comment.
//!
//! # What leaves the console
//!
//! The email (when `SMTP_HOST` is set) says who, which dashboard and the link.
//! It never carries the comment's text, a chart or a value (feature E3), and
//! goes only to people who passed the check above. The bell lists a mention as
//! "`<name>` mentioned you on `<dashboard>`", built from the live dashboard when
//! the bell is read and left out when the reader can no longer see it.
//!
//! # Plain text
//!
//! The text is stored and returned exactly as typed (trimmed). Nothing here
//! interprets it, and the console shows it as text, never as HTML or Markdown.
//! Control characters other than a newline and a tab are refused (a NUL would
//! also be refused by `Postgres`, as a 500).
//!
//! # Errors
//!
//! SEC-11: every sentence is ours; store and `ClickHouse` text goes to the log
//! under a reference.

use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_bi::access::Level;
use lakehouse_bi::store::{self, Board};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::dashboard_comments::{self as comments, Comment};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard_access::{Scope, require_board_level, scope_for_write};
use crate::state::AppState;
use crate::upstream_error;

const NEEDS_DATABASE: &str = "Comments need the database.";
const COMMENT_NOT_FOUND: &str = "comment not found.";
const BODY_SHAPE: &str =
    "body must be {\"board\": string, \"body\": string, \"mentions\"?: [string]}";
const EDIT_SHAPE: &str = "body must be {\"body\": string, \"mentions\"?: [string]}";
const EMPTY: &str = "A comment cannot be empty.";
const TOO_LONG: &str = "A comment can have at most 2000 characters.";
const CONTROL_CHARS: &str = "A comment cannot contain control characters.";
const MENTION_REFUSED: &str = "One of the people mentioned cannot see this dashboard.";
const TOO_MANY_MENTIONS: &str = "A comment can mention at most 20 people.";
/// The one sentence a service identity (a run token or API key) gets on every
/// comment route. `pub` so the route tests assert the exact text; comments are
/// a human conversation, and a service identity has no name to show on one.
pub const SERVICE_REFUSED: &str =
    "Comments are for people; a service identity cannot post, edit, delete or look people up.";
const NOT_THE_AUTHOR: &str = "Only the author can change a comment.";
const NOT_AUTHOR_OR_MANAGER: &str =
    "Only the author or someone who manages the dashboard can delete a comment.";
const MAX_MENTIONS: usize = 20;
/// People the @ picker offers at most, and how many candidates it checks to
/// find them (each check reads the candidate's own access).
const PICKER_SHOWN: usize = 10;
const PICKER_CANDIDATES: i64 = 40;

fn pool_of(state: &AppState) -> Result<&PgPool, ApiError> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable(NEEDS_DATABASE.to_owned()))
}

fn store_failure(err: &lakehouse_store::StoreError) -> ApiError {
    upstream_error::internal_error(&upstream_error::STORE, err)
}

/// A dashboard the caller may see, as the comment routes need it.
struct Target {
    /// What `board_id` holds: the row's id, or the built-in dashboard's.
    key: String,
    /// `None` for the built-in dashboard.
    board: Option<Board>,
    /// Shared so a caller that resolves many dashboards (the bell) resolves the
    /// caller's scope once (`BI-19` E review E-3).
    scope: Arc<Scope>,
}

impl Target {
    /// Whether the caller manages the dashboard (may delete others' comments).
    fn manageable(&self) -> bool {
        self.scope
            .require_known(self.board.as_ref(), Level::Manage)
            .is_ok()
    }

    fn name(&self) -> String {
        self.board
            .as_ref()
            .map_or_else(|| "the default dashboard".to_owned(), |b| b.name.clone())
    }
}

/// `BI-19` E review E-5: a service identity has no `app_user` row, so its
/// comment would fail the author foreign key as a 500. Refuse it up front with
/// a sentence instead.
fn refuse_service(principal: &Principal) -> Result<(), ApiError> {
    match principal.id {
        PrincipalId::User(_) => Ok(()),
        PrincipalId::Service(_) => Err(ApiError::PermissionDenied(SERVICE_REFUSED.to_owned())),
    }
}

/// Resolve `board` at `view` for a caller whose `scope` is already worked out:
/// the one place that decides a comment route may touch a dashboard.
async fn target_with(state: &AppState, scope: Arc<Scope>, board: &str) -> Result<Target, ApiError> {
    let id = board.trim();
    let board = require_board_level(state, &scope, id, Level::View).await?;
    let key = board
        .as_ref()
        .map_or_else(|| store::DEFAULT_BOARD_ID.to_owned(), |b| b.id.clone());
    Ok(Target { key, board, scope })
}

/// [`target_with`] after resolving `principal`'s scope.
async fn target_for(
    state: &AppState,
    principal: &Principal,
    board: &str,
) -> Result<Target, ApiError> {
    pool_of(state)?;
    let scope = Arc::new(scope_for_write(state, principal).await?);
    target_with(state, scope, board).await
}

/// [`target_for`] with a comment's own not-found sentence, for the routes that
/// start from a comment id: a comment on a dashboard the caller cannot see is
/// the same answer as an unknown comment.
async fn target_of_comment(
    state: &AppState,
    principal: &Principal,
    comment: &Comment,
) -> Result<Target, ApiError> {
    match target_for(state, principal, &comment.board_id).await {
        Err(ApiError::NotFound(_)) => Err(ApiError::NotFound(COMMENT_NOT_FOUND.to_owned())),
        other => other,
    }
}

/// The text as stored: trimmed, 1 to 2000 characters, no control characters.
fn checked_text(raw: &str) -> Result<String, ApiError> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(ApiError::BadRequest(EMPTY.to_owned()));
    }
    if text.chars().count() > comments::MAX_CHARS {
        return Err(ApiError::BadRequest(TOO_LONG.to_owned()));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(ApiError::BadRequest(CONTROL_CHARS.to_owned()));
    }
    Ok(text.to_owned())
}

/// The mentioned ids as a set: parsed, without duplicates, without the author.
fn parsed_mentions(raw: &[String], author: Uuid) -> Result<Vec<Uuid>, ApiError> {
    let mut ids: Vec<Uuid> = Vec::new();
    for item in raw {
        // A malformed id gets the mention sentence: it is no more informative
        // than a person who does not exist.
        let id = item
            .trim()
            .parse::<Uuid>()
            .map_err(|_| ApiError::BadRequest(MENTION_REFUSED.to_owned()))?;
        if id != author && !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() > MAX_MENTIONS {
        return Err(ApiError::BadRequest(TOO_MANY_MENTIONS.to_owned()));
    }
    Ok(ids)
}

/// Whether person `user` can see the target dashboard as themselves.
/// Fail closed: only a definite yes is true; a disabled or unknown person, a
/// tenant that does not overlap the author's, or a person who cannot see it is
/// false; a store failure is an error, never a yes.
async fn can_see(
    state: &AppState,
    author: &Principal,
    target: &Target,
    user: Uuid,
) -> Result<bool, ApiError> {
    let person = match crate::routes::render::principal_of(state, &user.to_string()).await {
        Ok(Some(person)) => person,
        Ok(None) => return Ok(false),
        // `principal_of` has logged the failure under a reference.
        Err(_logged) => return Err(ApiError::Unavailable(NEEDS_DATABASE.to_owned())),
    };
    if !crate::routes::reports::shares_a_tenant(&author.tenant_ids, &person.tenant_ids) {
        return Ok(false);
    }
    // `BI-19` E review E-3: the person's scope once, judged against the board
    // the caller already fetched (no second `ClickHouse` read per candidate).
    let scope = match scope_for_write(state, &person).await {
        Ok(scope) => scope,
        Err(ApiError::NotFound(_) | ApiError::PermissionDenied(_)) => return Ok(false),
        Err(other) => return Err(other),
    };
    Ok(scope
        .require_known(target.board.as_ref(), Level::View)
        .is_ok())
}

/// Every mentioned person must see the dashboard (D15).
async fn require_all_see(
    state: &AppState,
    author: &Principal,
    target: &Target,
    mentions: &[Uuid],
) -> Result<(), ApiError> {
    for user in mentions {
        if !can_see(state, author, target, *user).await? {
            return Err(ApiError::BadRequest(MENTION_REFUSED.to_owned()));
        }
    }
    Ok(())
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap_or_default()
}

fn comment_json(
    comment: &Comment,
    mentions: &[(Uuid, String)],
    caller: Uuid,
    manages: bool,
) -> Value {
    let mine = comment.author_id == Some(caller);
    json!({
        "id": comment.id,
        "board": comment.board_id,
        "author": comment.author_id.map(|id| json!({
            "id": id,
            "name": comment.author_name.clone().unwrap_or_default(),
        })),
        "body": comment.body,
        "createdAt": rfc3339(comment.created_at),
        "editedAt": comment.edited_at.map(rfc3339),
        "mentions": mentions.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
        "canEdit": mine,
        "canDelete": mine || manages,
    })
}

async fn mentions_for(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<(Uuid, Uuid, String)>, ApiError> {
    comments::mentions_of(pool, ids)
        .await
        .map_err(|e| store_failure(&e))
}

/// One comment as the caller sees it.
async fn one_json(
    state: &AppState,
    principal: &Principal,
    target: &Target,
    id: Uuid,
) -> Result<Value, ApiError> {
    let pool = pool_of(state)?;
    let comment = comments::get(pool, id)
        .await
        .map_err(|e| store_failure(&e))?
        .ok_or_else(|| ApiError::NotFound(COMMENT_NOT_FOUND.to_owned()))?;
    let named: Vec<(Uuid, String)> = mentions_for(pool, &[id])
        .await?
        .into_iter()
        .map(|(_, user, name)| (user, name))
        .collect();
    Ok(comment_json(
        &comment,
        &named,
        principal.id.uuid(),
        target.manageable(),
    ))
}

/// Query string of `GET /api/dashboard/comments`. `pub` only because the
/// handler that takes it is routed from `routes/mod.rs`; the board id is
/// optional here so a missing one is our `400`, not the extractor's text.
#[derive(Deserialize)]
pub struct BoardQuery {
    board: Option<String>,
}

/// `GET /api/dashboard/comments?board=` — the dashboard's comments (the newest
/// 200, oldest first; `truncated` says older ones are left out).
///
/// # Errors
///
/// `400` without a board id, `404 dashboard not found.` for a dashboard the
/// caller cannot see (unknown, trashed and hidden alike), `403` for a caller
/// with no access to dashboards at all, `503` without a database.
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<BoardQuery>,
) -> ApiResult<ApiJson<Value>> {
    let board = q.board.as_deref().map(str::trim).unwrap_or_default();
    if board.is_empty() {
        return Err(ApiError::BadRequest("board is required".to_owned()).into());
    }
    let target = target_for(&state, &principal, board).await?;
    let pool = pool_of(&state)?;
    let (rows, truncated) = comments::list(pool, &target.key)
        .await
        .map_err(|e| store_failure(&e))?;
    let ids: Vec<Uuid> = rows.iter().map(|c| c.id).collect();
    let named = mentions_for(pool, &ids).await?;
    let manages = target.manageable();
    let caller = principal.id.uuid();
    let items: Vec<Value> = rows
        .iter()
        .map(|c| {
            let who: Vec<(Uuid, String)> = named
                .iter()
                .filter(|(comment, _, _)| *comment == c.id)
                .map(|(_, user, name)| (*user, name.clone()))
                .collect();
            comment_json(c, &who, caller, manages)
        })
        .collect();
    Ok(ApiJson(
        json!({ "comments": items, "truncated": truncated }),
    ))
}

#[derive(Deserialize)]
struct PostBody {
    board: String,
    body: String,
    #[serde(default)]
    mentions: Vec<String>,
}

#[derive(Deserialize)]
struct EditBody {
    body: String,
    #[serde(default)]
    mentions: Option<Vec<String>>,
}

/// `POST /api/dashboard/comments`.
///
/// # Errors
///
/// As [`list`]; also `400` for a body of another shape, an empty, oversized or
/// control-character text, or a mention of somebody who cannot see the
/// dashboard (one sentence for every reason).
pub async fn post(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    refuse_service(&principal)?;
    let parsed: PostBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    // Access before the body is judged: a caller who cannot see the dashboard
    // learns nothing from the validation sentences.
    let target = target_for(&state, &principal, &parsed.board).await?;
    let text = checked_text(&parsed.body)?;
    let author = principal.id.uuid();
    let mentions = parsed_mentions(&parsed.mentions, author)?;
    require_all_see(&state, &principal, &target, &mentions).await?;
    let pool = pool_of(&state)?;
    let (id, to_email) = comments::insert(pool, &target.key, author, &text, &mentions)
        .await
        .map_err(|e| store_failure(&e))?;
    notify(&state, &principal, &target, &to_email);
    Ok(ApiJson(one_json(&state, &principal, &target, id).await?))
}

/// `PUT /api/dashboard/comments/{id}` — the author changes the text and,
/// when `mentions` is present, the people mentioned. Only people newly
/// mentioned (and with no other unread mention here) are emailed.
///
/// # Errors
///
/// `404 comment not found.` for a comment the caller cannot reach; `403` for
/// somebody who can see it and did not write it; `400` as [`post`].
pub async fn edit(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    refuse_service(&principal)?;
    let parsed: EditBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(EDIT_SHAPE.to_owned()))?;
    let pool = pool_of(&state)?;
    let comment = comments::get(pool, id)
        .await
        .map_err(|e| store_failure(&e))?
        .ok_or_else(|| ApiError::NotFound(COMMENT_NOT_FOUND.to_owned()))?;
    let target = target_of_comment(&state, &principal, &comment).await?;
    let author = principal.id.uuid();
    // Even a manager edits nobody else's words (D15: author to edit).
    if comment.author_id != Some(author) {
        return Err(ApiError::PermissionDenied(NOT_THE_AUTHOR.to_owned()).into());
    }
    let text = checked_text(&parsed.body)?;
    let already: Vec<Uuid> = mentions_for(pool, &[id])
        .await?
        .into_iter()
        .map(|(_, user, _)| user)
        .collect();
    let mentions = match &parsed.mentions {
        Some(raw) => parsed_mentions(raw, author)?,
        None => already.clone(),
    };
    // Only people not already mentioned need the check; the others passed it
    // when they were added.
    let fresh: Vec<Uuid> = mentions
        .iter()
        .copied()
        .filter(|user| !already.contains(user))
        .collect();
    require_all_see(&state, &principal, &target, &fresh).await?;
    let to_email = comments::update(pool, id, &text, &mentions)
        .await
        .map_err(|e| store_failure(&e))?
        .ok_or_else(|| ApiError::NotFound(COMMENT_NOT_FOUND.to_owned()))?;
    notify(&state, &principal, &target, &to_email);
    Ok(ApiJson(one_json(&state, &principal, &target, id).await?))
}

/// `DELETE /api/dashboard/comments/{id}` — the author, or somebody who manages
/// the dashboard.
///
/// # Errors
///
/// `404 comment not found.` for a comment the caller cannot reach; `403` for
/// somebody who can see it and may not delete it.
pub async fn remove(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> ApiResult<ApiJson<Value>> {
    refuse_service(&principal)?;
    let pool = pool_of(&state)?;
    let comment = comments::get(pool, id)
        .await
        .map_err(|e| store_failure(&e))?
        .ok_or_else(|| ApiError::NotFound(COMMENT_NOT_FOUND.to_owned()))?;
    let target = target_of_comment(&state, &principal, &comment).await?;
    let author = principal.id.uuid();
    if comment.author_id != Some(author) && !target.manageable() {
        return Err(ApiError::PermissionDenied(NOT_AUTHOR_OR_MANAGER.to_owned()).into());
    }
    comments::soft_delete(pool, id, author)
        .await
        .map_err(|e| store_failure(&e))?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// Query string of `GET /api/dashboard/comments/people`: the dashboard and the
/// typed prefix. `pub` for the same reason as [`BoardQuery`]; both are optional
/// so a missing one is our `400`, not the extractor's text.
#[derive(Deserialize)]
pub struct PeopleQuery {
    board: Option<String>,
    q: Option<String>,
}

/// `GET /api/dashboard/comments/people?board=&q=` — the @ picker: active people
/// whose **name** contains `q` and whose own access reaches the dashboard, at
/// most 10 (`BI-19` E review E-2: an address, or a piece of one, matches nobody,
/// so the picker is no way to look up who has which address). Somebody who cannot see
/// the dashboard is not offered, so the picker never offers a mention the post
/// would refuse, and it is no way to list who can see a dashboard they are
/// not on (the caller must see it themselves first).
///
/// # Errors
///
/// As [`list`].
pub async fn people(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<PeopleQuery>,
) -> ApiResult<ApiJson<Value>> {
    refuse_service(&principal)?;
    let board = q.board.as_deref().map(str::trim).unwrap_or_default();
    if board.is_empty() {
        return Err(ApiError::BadRequest("board is required".to_owned()).into());
    }
    let target = target_for(&state, &principal, board).await?;
    let pool = pool_of(&state)?;
    let found = lakehouse_store::report_subscriptions::search_active_user_names(
        pool,
        q.q.as_deref().unwrap_or("").trim(),
        PICKER_CANDIDATES,
        &principal.tenant_ids,
    )
    .await
    .map_err(|e| store_failure(&e))?;
    let me = principal.id.uuid();
    let mut shown: Vec<Value> = Vec::new();
    for (id, name) in found {
        if shown.len() >= PICKER_SHOWN {
            break;
        }
        if id != me && can_see(&state, &principal, &target, id).await? {
            // Name only, matched on the name only (E-2).
            shown.push(json!({ "id": id, "name": name }));
        }
    }
    Ok(ApiJson(json!({ "items": shown })))
}

/// `POST /api/dashboard/comments/mentions/read` — the caller's mentions are
/// read (what the bell's "mark as read" calls).
///
/// # Errors
///
/// `503` without a database.
pub async fn mark_read(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let pool = pool_of(&state)?;
    let marked = comments::mark_mentions_read(pool, principal.id.uuid())
        .await
        .map_err(|e| store_failure(&e))?;
    Ok(ApiJson(json!({ "marked": marked })))
}

/// The caller's unread mentions as bell lines, for `GET /api/notifications`.
/// Built from the live dashboard and left out when the reader can no longer see
/// it; the comment's text is never in a line.
///
/// # Errors
///
/// The classified store error.
pub(crate) async fn bell_lines(
    state: &AppState,
    principal: &Principal,
) -> Result<Vec<Value>, ApiError> {
    let pool = pool_of(state)?;
    let unread = comments::unread_mentions(pool, principal.id.uuid())
        .await
        .map_err(|e| store_failure(&e))?;
    if unread.is_empty() {
        return Ok(Vec::new());
    }
    // `BI-19` E review E-3: the reader's scope once for the whole bell, not once
    // per dashboard.
    let scope = Arc::new(scope_for_write(state, principal).await?);
    let mut by_board: std::collections::HashMap<String, Option<Target>> =
        std::collections::HashMap::new();
    let mut lines = Vec::new();
    for mention in unread {
        if !by_board.contains_key(&mention.board_id) {
            let target = match target_with(state, Arc::clone(&scope), &mention.board_id).await {
                Ok(target) => Some(target),
                Err(ApiError::NotFound(_) | ApiError::PermissionDenied(_)) => None,
                Err(other) => return Err(other),
            };
            by_board.insert(mention.board_id.clone(), target);
        }
        let Some(Some(target)) = by_board.get(&mention.board_id) else {
            continue;
        };
        let who = mention
            .author_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or("Someone");
        lines.push(json!({
            "id": mention.comment_id,
            "board": target.key,
            "title": format!("{who} mentioned you on {}", target.name()),
            "at": rfc3339(mention.created_at),
        }));
    }
    Ok(lines)
}

/// The email a mention sends: who, which dashboard, the link. The signature is
/// the guarantee (`BI-19` E review E-4): it is given no comment text, chart or
/// value (feature E3), and every name is escaped as text.
fn mention_email(author: &str, board: &str, link: Option<&str>) -> String {
    use crate::routes::report_runner::html_text;
    match link {
        Some(link) => format!(
            "<p>{} mentioned you in a comment on the dashboard {}.</p><p><a href=\"{2}\">{2}</a></p>",
            html_text(author),
            html_text(board),
            html_text(link)
        ),
        None => format!(
            "<p>{} mentioned you in a comment on the dashboard {}.</p>",
            html_text(author),
            html_text(board)
        ),
    }
}

/// Email the people the store chose (when `SMTP_HOST` is set): newly mentioned
/// and with no other unread mention on this dashboard (`BI-19` E review E-1).
/// Not the comment's text, not a chart or a value (E3).
/// Runs in the background and is best effort: a failed send never fails or
/// delays the comment, and is logged without the address.
fn notify(state: &AppState, author: &Principal, target: &Target, people: &[Uuid]) {
    if people.is_empty() || state.config.smtp_host.is_none() {
        return;
    }
    let Some(pool) = state.pg.clone() else {
        return;
    };
    let sender =
        lakehouse_notify::EmailSender::new(crate::routes::alerts::smtp_config(&state.config));
    let link = crate::routes::report_runner::dashboard_link(state, &target.key);
    let html = mention_email(&author.display_name, &target.name(), link.as_deref());
    let people = people.to_vec();
    tokio::spawn(async move {
        let subject = "You were mentioned in a comment";
        for user in people {
            let Ok(person) = lakehouse_store::identity::get_user(&pool, &user.to_string()).await
            else {
                continue;
            };
            let sent = sender.send(person.email.trim(), subject, &html).await;
            if !sent.ok {
                tracing::warn!("a mention email was not sent");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// `BI-19` E review E-4: names are escaped, and the email is exactly the
    /// fixed sentences around the names and the link (no place for a comment's
    /// text, which `mention_email` is never given).
    #[test]
    fn the_mention_email_escapes_names_and_carries_no_comment_text() {
        let html = mention_email(
            "<script>alert(1)</script>",
            "Q3 \"board\" & <b>co</b>",
            Some("https://console.example/d?x=1&y=2"),
        );
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html}"
        );
        assert!(
            html.contains("Q3 &quot;board&quot; &amp; &lt;b&gt;co&lt;/b&gt;"),
            "{html}"
        );
        assert!(html.contains("x=1&amp;y=2"), "{html}");
        assert_eq!(
            mention_email("Ann", "Sales", None),
            "<p>Ann mentioned you in a comment on the dashboard Sales.</p>"
        );
        assert_eq!(
            mention_email("Ann", "Sales", Some("https://c.example/d")),
            "<p>Ann mentioned you in a comment on the dashboard Sales.</p>\
             <p><a href=\"https://c.example/d\">https://c.example/d</a></p>"
        );
    }

    /// `BI-19` E review E-5.
    #[test]
    fn a_service_identity_is_refused_with_a_sentence() {
        let mut principal = Principal {
            id: PrincipalId::User(Uuid::new_v4()),
            tenant_ids: Vec::new(),
            display_name: "t".to_owned(),
            permissions: lakehouse_auth::PermissionSet::default(),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        assert!(refuse_service(&principal).is_ok());
        principal.id = PrincipalId::Service(Uuid::new_v4());
        assert!(
            matches!(refuse_service(&principal), Err(ApiError::PermissionDenied(m)) if m == SERVICE_REFUSED)
        );
    }

    #[test]
    fn text_is_trimmed_and_bounded() {
        assert_eq!(checked_text("  hi \n").unwrap(), "hi");
        assert!(matches!(checked_text(" \n "), Err(ApiError::BadRequest(m)) if m == EMPTY));
        assert!(checked_text(&"x".repeat(2000)).is_ok());
        assert!(
            matches!(checked_text(&"x".repeat(2001)), Err(ApiError::BadRequest(m)) if m == TOO_LONG)
        );
        // Characters, not bytes.
        assert!(checked_text(&"é".repeat(2000)).is_ok());
    }

    #[test]
    fn control_characters_are_refused_but_a_newline_and_a_tab_are_not() {
        assert!(checked_text("a\nb\tc").is_ok());
        for bad in ["a\0b", "a\u{7}b", "a\u{1b}[31m"] {
            assert!(
                matches!(checked_text(bad), Err(ApiError::BadRequest(m)) if m == CONTROL_CHARS),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn markup_is_kept_as_typed_for_the_console_to_show_as_text() {
        let raw = "<script>alert(1)</script> **bold** [x](javascript:alert(1))";
        assert_eq!(checked_text(raw).unwrap(), raw);
    }

    #[test]
    fn mentions_drop_the_author_and_duplicates_and_refuse_a_bad_id() {
        let author = Uuid::new_v4();
        let other = Uuid::new_v4();
        let ids = parsed_mentions(
            &[author.to_string(), other.to_string(), other.to_string()],
            author,
        )
        .unwrap();
        assert_eq!(ids, [other]);
        assert!(
            matches!(parsed_mentions(&["nope".to_owned()], author), Err(ApiError::BadRequest(m)) if m == MENTION_REFUSED)
        );
        let many: Vec<String> = (0..21).map(|_| Uuid::new_v4().to_string()).collect();
        assert!(
            matches!(parsed_mentions(&many, author), Err(ApiError::BadRequest(m)) if m == TOO_MANY_MENTIONS)
        );
    }
}
