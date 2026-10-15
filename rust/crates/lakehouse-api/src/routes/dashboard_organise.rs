//! Finding, keeping and undoing (`BI-18` part C, part 2): version history,
//! the trash, favourites, recent items and search; plus the checks every
//! other dashboard route shares for the personal space and the trash.
//!
//! Plan: `docs/superpowers/plans/2026-10-14-bi-18c-layout.md`; feature page
//! `docs/core/features/dashboard-layout-and-organisation.md`.
//!
//! # Who may do what
//!
//! | Route | Permission | Why |
//! | --- | --- | --- |
//! | `GET /api/dashboard/versions` | `dashboard:read` | reading history is reading the dashboard |
//! | `POST /api/dashboard/versions/revert` | `dashboard:write` | it edits the dashboard |
//! | `GET /api/dashboard/trash` | `dashboard:read` | lists only what the caller may see |
//! | `POST /api/dashboard/trash/restore` | `dashboard:write` | the permission that deletes |
//! | `GET /api/dashboard/shortcuts`, `GET /api/dashboard/search` | `dashboard:read` | read |
//! | `PUT /api/dashboard/favourites`, `POST /api/dashboard/recent` | `dashboard:read` | a preference of the caller's own, never an edit of a dashboard |
//!
//! Every route reads through a [`Viewer`] built from the caller
//! ([`viewer_of`]): another user's personal space and the trash are absent,
//! and an id that names one is answered exactly like an id that never
//! existed.
//!
//! # Going back
//!
//! [`revert`] re-checks the whole stored state against today's rules before it
//! writes a byte: the tabs, the layout, the filters, the parameter controls,
//! and every chart (rebuilt from its stored definition by the same function an
//! edit uses, then run once). The first thing that no longer holds refuses the
//! revert with a sentence naming it, and nothing has changed. Only then are
//! the charts written, the charts added since moved to the trash, and the
//! dashboard's content written back in one save; the result is a new version.

use std::collections::HashSet;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::items::{self, Kind, List};
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{self, BiError, ChartInput, StoredChartSpec};
use lakehouse_bi::versions::{self, Author, BoardState};
use lakehouse_bi::{search, trash};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard_folders::{classify_bi_error, classify_ch_error};
use crate::state::AppState;
use crate::upstream_error;

/// The one sentence for a dashboard the caller may not read or change:
/// unknown, in the trash, or in somebody else's personal space.
const BOARD_NOT_FOUND: &str = "dashboard not found.";

/// The sentence for a chart the caller may not change.
const CHART_NOT_FOUND: &str = "this tile does not exist";

/// Why a check refused. The checks are shared by the HTTP routes (which turn
/// this into an [`ApiError`]) and the assistant's tools (which turn it into
/// their own JSON, with the reference id of a store failure kept), so the
/// store's error is carried, not classified here.
#[derive(Debug)]
pub(crate) enum Gate {
    /// A fixed sentence: the dashboard or chart is unknown, in the trash, or
    /// private to somebody else (the caller cannot tell which).
    Missing(&'static str),
    /// The store could not be read.
    Store(ChError),
}

impl From<Gate> for ApiError {
    fn from(gate: Gate) -> Self {
        match gate {
            Gate::Missing(sentence) => Self::NotFound(sentence.to_owned()),
            Gate::Store(cause) => classify_ch_error(&cause),
        }
    }
}

// ── Shared checks ───────────────────────────────────────────────────────

/// Who is reading, as far as the personal space is concerned. An administrator
/// (`identity:write`) sees every personal space; everybody else sees their own.
#[must_use]
pub(crate) fn viewer_of(principal: &Principal) -> Viewer {
    Viewer::user(
        principal.id.uuid().to_string(),
        principal.has("identity:write"),
    )
}

/// Who made a change, for the version history.
#[must_use]
pub(crate) fn author_of(principal: &Principal) -> Author {
    Author {
        id: principal.id.uuid().to_string(),
        name: principal.display_name.clone(),
    }
}

/// Require that dashboard `id` is live and visible to `viewer`.
///
/// # Errors
///
/// [`Gate::Missing`] with one sentence for every way it is not (unknown, in the
/// trash, somebody else's personal space: the caller cannot tell which);
/// [`Gate::Store`] when the store is unreachable.
pub(crate) async fn require_board(ch: &ChClient, id: &str, viewer: &Viewer) -> Result<(), Gate> {
    match store::board_row_state(ch, id).await.map_err(Gate::Store)? {
        Some((0, owner)) if viewer.sees(&owner) => Ok(()),
        _ => Err(Gate::Missing(BOARD_NOT_FOUND)),
    }
}

/// Require that a chart may be put on dashboard `board` (`None`, blank or the
/// built-in board always pass; so does an id that never existed, as before).
///
/// # Errors
///
/// As [`require_board`] when the dashboard is in the trash or private to
/// somebody else.
pub(crate) async fn require_chart_board(
    ch: &ChClient,
    board: Option<&str>,
    viewer: &Viewer,
) -> Result<(), Gate> {
    let board = board.map(str::trim).unwrap_or_default();
    if board.is_empty() || board == store::DEFAULT_BOARD_ID {
        return Ok(());
    }
    match store::board_row_state(ch, board)
        .await
        .map_err(Gate::Store)?
    {
        Some((2, _)) => Err(Gate::Missing(BOARD_NOT_FOUND)),
        Some((_, owner)) if !viewer.sees(&owner) => Err(Gate::Missing(BOARD_NOT_FOUND)),
        _ => Ok(()),
    }
}

/// Write a version of `board` after a successful change. A history write that
/// fails is logged under a reference and swallowed: the change it describes
/// has already happened and must not be reported as failed.
pub(crate) async fn snapshot(ch: &ChClient, board: &str, principal: &Principal) {
    let _ = snapshot_version(ch, board, Some(principal)).await;
}

/// [`snapshot`] for a change made with no signed-in person attached (the
/// assistant's tools outside a session): recorded as the assistant.
pub(crate) async fn snapshot_as(ch: &ChClient, board: &str, principal: Option<&Principal>) {
    let _ = snapshot_version(ch, board, principal).await;
}

async fn snapshot_version(
    ch: &ChClient,
    board: &str,
    principal: Option<&Principal>,
) -> Option<u32> {
    let author = principal.map_or_else(
        || Author {
            id: String::new(),
            name: "Assistant".to_owned(),
        },
        author_of,
    );
    match versions::record(ch, board, &author).await {
        Ok(number) => number,
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            None
        }
    }
}

/// The board of chart `id` when the chart is live and the viewer may see it;
/// `None` for a chart that does not exist, is in the trash, or is on another
/// user's personal board (which the caller answers like an unknown id).
///
/// # Errors
///
/// [`Gate::Store`] when the store is unreachable.
pub(crate) async fn visible_live_chart_board(
    ch: &ChClient,
    id: &str,
    viewer: &Viewer,
) -> Result<Option<String>, Gate> {
    let Some((state, board)) = store::chart_row_state(ch, id).await.map_err(Gate::Store)? else {
        return Ok(None);
    };
    if state != 0 {
        return Ok(None);
    }
    Ok(require_chart_board(ch, Some(&board), viewer)
        .await
        .ok()
        .map(|()| board))
}

/// Check that an edit may write chart `id`: it is not in the trash and not on a
/// board the viewer may not see. Returns the board it is on now, `None` for a
/// new id (an edit of an unknown id keeps creating one, as before).
///
/// # Errors
///
/// [`Gate::Missing`] for a chart in the trash or on a board the viewer cannot
/// see; [`Gate::Store`] when the store is unreachable.
pub(crate) async fn require_editable_chart(
    ch: &ChClient,
    id: &str,
    viewer: &Viewer,
) -> Result<Option<String>, Gate> {
    let Some((state, board)) = store::chart_row_state(ch, id).await.map_err(Gate::Store)? else {
        return Ok(None);
    };
    match state {
        0 => {
            require_chart_board(ch, Some(&board), viewer).await?;
            Ok(Some(board))
        }
        2 => Err(Gate::Missing(CHART_NOT_FOUND)),
        _ => Ok(None),
    }
}

/// What a "who uses this" check names when something the caller cannot see
/// also uses it. The use still blocks a delete (removing the thing would break
/// that dashboard), but the dashboard is not named: not its title, not its
/// owner, not how many there are.
pub(crate) const HIDDEN_USE: &str = "a dashboard you cannot see";

/// The dashboards and charts a usage check works over, twice: everything live
/// (to decide whether a delete is blocked) and what the caller may see (to name
/// what blocks it).
pub(crate) struct Seen {
    /// Every live chart, whoever's.
    pub all_charts: Vec<StoredChartSpec>,
    /// The charts the caller may see.
    pub charts: Vec<StoredChartSpec>,
    /// Every live dashboard, whoever's.
    pub all_boards: Vec<store::Board>,
    /// The dashboards the caller may see.
    pub boards: Vec<store::Board>,
}

impl Seen {
    /// Read both views.
    ///
    /// # Errors
    ///
    /// Returns [`ChError`] on a `ClickHouse` failure.
    pub(crate) async fn read(ch: &ChClient, viewer: &Viewer) -> Result<Self, ChError> {
        let system = Viewer::system();
        let (all_charts, all_boards) = tokio::try_join!(
            store::list_stored_charts(ch, &system),
            store::list_boards(ch, &system)
        )?;
        if viewer.sees_all() {
            return Ok(Self {
                charts: all_charts.clone(),
                boards: all_boards.clone(),
                all_charts,
                all_boards,
            });
        }
        let (charts, boards) = tokio::try_join!(
            store::list_stored_charts(ch, viewer),
            store::list_boards(ch, viewer)
        )?;
        Ok(Self {
            all_charts,
            charts,
            all_boards,
            boards,
        })
    }

    /// The charts the caller may not see.
    pub(crate) fn hidden_charts(&self) -> Vec<StoredChartSpec> {
        self.all_charts
            .iter()
            .filter(|c| !self.charts.iter().any(|v| v.spec.id == c.spec.id))
            .cloned()
            .collect()
    }

    /// The dashboards the caller may not see.
    pub(crate) fn hidden_boards(&self) -> Vec<store::Board> {
        self.all_boards
            .iter()
            .filter(|b| !self.boards.iter().any(|v| v.id == b.id))
            .cloned()
            .collect()
    }
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("body JSON is invalid".to_owned()))
}

// ── Version history ─────────────────────────────────────────────────────

/// `GET /api/dashboard/versions?board=` query.
#[derive(Debug, Deserialize)]
pub struct VersionsQuery {
    #[serde(default)]
    board: Option<String>,
}

/// `GET /api/dashboard/versions?board=` — the last versions of a dashboard,
/// newest first: who, when, and what changed. The built-in dashboard keeps
/// none.
///
/// # Errors
///
/// 400 without a board, 404 for a dashboard the caller cannot read, 503 when
/// the store is unreachable.
pub async fn versions_list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<VersionsQuery>,
) -> ApiResult<ApiJson<Value>> {
    let id = q.board.as_deref().map(str::trim).unwrap_or_default();
    if id.is_empty() {
        return Err(ApiError::BadRequest("board is required.".to_owned()).into());
    }
    let ch = &state.clickhouse;
    if id == store::DEFAULT_BOARD_ID {
        return Ok(ApiJson(json!({ "versions": [], "keep": versions::KEEP })));
    }
    require_board(ch, id, &viewer_of(&principal)).await?;
    let list = versions::list(ch, id)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({ "versions": list, "keep": versions::KEEP })))
}

/// `POST /api/dashboard/versions/revert` body.
#[derive(Debug, Deserialize)]
pub struct RevertBody {
    #[serde(default)]
    board: String,
    #[serde(default)]
    version: u32,
}

/// The 422 for a version that cannot be restored as it is: one sentence
/// naming the first thing that no longer holds. Nothing has been written.
fn cannot_go_back(version: u32, reason: &str) -> ApiError {
    ApiError::Unprocessable(format!(
        "Cannot go back to version {version}: {reason} Nothing was changed."
    ))
}

/// What [`prepare_revert`] decided, ready to write.
struct RevertPlan {
    /// Charts to write, rebuilt and run once.
    charts: Vec<StoredChartSpec>,
    /// Titles of charts the version had that are not on the dashboard now
    /// (deleted since): they come back only from the trash.
    skipped: Vec<String>,
    /// Ids of charts on the dashboard now that the version did not have.
    added_since: Vec<String>,
    /// The layout and phone layout with the tiles of skipped charts removed.
    layout: store::LayoutMap,
    phone_layout: Vec<lakehouse_bi::layout::PhoneTile>,
    params: Vec<lakehouse_bi::params::BoardParam>,
}

/// The board-level rules of a stored state, against today's.
fn check_state_rules(
    state: &BoardState,
    layout: &store::LayoutMap,
    number: u32,
) -> Result<(), ApiError> {
    let refuse = |reason: String| cannot_go_back(number, &format!("{reason}."));
    lakehouse_bi::layout::validate_tabs(&state.tabs)
        .and_then(|()| lakehouse_bi::layout::check_layout_tabs(layout, &state.tabs))
        .and_then(|()| lakehouse_bi::layout::check_filter_ties(&state.filters, &state.tabs))
        .and_then(|()| lakehouse_bi::layout::validate_phone_layout(&state.phone_layout))
        .and_then(|()| lakehouse_bi::filters::validate_filters(&state.filters))
        .map_err(|reason| refuse(reason.trim_end_matches('.').to_owned()))?;
    if let Some(seconds) = state.refresh_seconds {
        lakehouse_bi::click::validate_refresh_seconds(seconds)
            .map_err(|reason| refuse(reason.trim_end_matches('.').to_owned()))?;
    }
    Ok(())
}

/// A stored chart that is a fixed statement with no definition to rebuild it
/// from (the tiles copied from the built-in dashboard): written back as it
/// was, not rebuilt.
fn is_fixed_statement(input: &ChartInput) -> bool {
    input.mart.is_empty() && input.sql_source.is_none() && !input.kind.is_static()
}

/// Check the whole stored state against today's rules and rebuild its charts,
/// without writing anything.
async fn prepare_revert(
    ch: &ChClient,
    state: &BoardState,
    board: &str,
    number: u32,
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<RevertPlan, ApiError> {
    let live: Vec<StoredChartSpec> = store::list_stored_charts(ch, &Viewer::system())
        .await
        .map_err(|err| classify_ch_error(&err))?
        .into_iter()
        .filter(|c| c.board == board)
        .collect();
    let live_ids: HashSet<&str> = live.iter().map(|c| c.spec.id.as_str()).collect();
    let kept_ids: HashSet<&str> = state
        .charts
        .iter()
        .map(|c| c.id.as_str())
        .filter(|id| live_ids.contains(id))
        .collect();
    let layout: store::LayoutMap = state
        .layout
        .iter()
        .filter(|(id, _)| kept_ids.contains(id.as_str()))
        .map(|(id, tile)| (id.clone(), tile.clone()))
        .collect();
    let phone_layout = state
        .phone_layout
        .iter()
        .filter(|t| kept_ids.contains(t.id.as_str()))
        .cloned()
        .collect();
    check_state_rules(state, &layout, number)?;
    // The parameter controls: a parameter deleted since is a refusal, said
    // with the check's own sentence.
    let params = crate::routes::params::check_board_params_opt(ch, Some(&state.params))
        .await
        .map_err(|reason| match reason {
            ApiError::BadRequest(m) | ApiError::Unprocessable(m) => {
                cannot_go_back(number, &format!("{}.", m.trim_end_matches('.')))
            }
            other => other,
        })?
        .unwrap_or_default();

    let mut charts = Vec::new();
    let mut skipped = Vec::new();
    for chart in &state.charts {
        if !kept_ids.contains(chart.id.as_str()) {
            skipped.push(chart.title.clone());
            continue;
        }
        charts.push(rebuild_chart(ch, chart, board, number, time).await?);
    }
    let added_since = live
        .iter()
        .map(|c| c.spec.id.clone())
        .filter(|id| !state.charts.iter().any(|c| c.id == *id))
        .collect();
    Ok(RevertPlan {
        charts,
        skipped,
        added_since,
        layout,
        phone_layout,
        params,
    })
}

/// One stored chart rebuilt by the function an edit uses, then run once.
async fn rebuild_chart(
    ch: &ChClient,
    chart: &versions::StateChart,
    board: &str,
    number: u32,
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<StoredChartSpec, ApiError> {
    let name = chart.title.as_str();
    let broken = |reason: &str| cannot_go_back(number, &format!("the chart \"{name}\" {reason}."));
    let def: Option<ChartInput> = chart
        .spec
        .get("def")
        .cloned()
        .and_then(|d| serde_json::from_value(d).ok());
    let rebuilt = match def {
        Some(mut input) if !is_fixed_statement(&input) => {
            input.board = Some(board.to_owned());
            let source = if chart.created_by == "ai" {
                ChartSource::Ai
            } else {
                ChartSource::Ui
            };
            match store::spec_from_input(
                ch,
                &input,
                source,
                &chart.created_by,
                Some(chart.id.clone()),
                time,
            )
            .await
            {
                Ok(spec) => spec,
                Err(BiError::Validation(reason)) => {
                    return Err(broken(&format!(
                        "no longer works ({})",
                        reason.trim_end_matches('.')
                    )));
                }
                Err(BiError::Clickhouse(cause)) => return Err(classify_ch_error(&cause)),
            }
        }
        _ => store::stored_from_document(&chart.spec, board, &chart.created_by)
            .ok_or_else(|| broken("cannot be read"))?,
    };
    match store::smoke_test_chart(ch, &rebuilt).await {
        Ok(()) => Ok(rebuilt),
        Err(cause @ ChError::Transport(_)) => Err(classify_ch_error(&cause)),
        Err(cause) => {
            // The engine's own text is for the log, not the response.
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &cause);
            Err(broken("no longer runs"))
        }
    }
}

/// `POST /api/dashboard/versions/revert` — write a stored version back.
///
/// # Errors
///
/// 400 without a board or a version, 404 for a dashboard the caller cannot
/// read or a version that is not kept, 422 with a sentence naming the first
/// thing that no longer holds (nothing is written then), 503 when the store is
/// unreachable.
pub async fn revert(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: RevertBody = parse(&body)?;
    let id = body.board.trim();
    if id.is_empty() || body.version == 0 {
        return Err(ApiError::BadRequest("board and version are required.".to_owned()).into());
    }
    if id == store::DEFAULT_BOARD_ID {
        return Err(
            ApiError::BadRequest("The built-in dashboard keeps no history.".to_owned()).into(),
        );
    }
    let ch = &state.clickhouse;
    require_board(ch, id, &viewer_of(&principal)).await?;
    let Some(stored) = versions::load(ch, id, body.version)
        .await
        .map_err(|err| classify_ch_error(&err))?
    else {
        return Err(ApiError::NotFound("That version is not kept any more.".to_owned()).into());
    };
    let time = crate::routes::settings::time_context(&state).await?;
    let plan = prepare_revert(ch, &stored, id, body.version, &time).await?;
    // Past this line nothing is refused for the content's sake; a failing
    // write is a store failure and the same request can be sent again.
    for chart in &plan.charts {
        store::insert_chart_unchecked(ch, chart)
            .await
            .map_err(|err| classify_ch_error(&err))?;
    }
    for chart in &plan.added_since {
        store::delete_chart(ch, chart, &principal.display_name)
            .await
            .map_err(|err| classify_ch_error(&err))?;
    }
    store::restore_board_fields(
        ch,
        id,
        &store::BoardFields {
            name: &stored.name,
            description: &stored.description,
            layout: &plan.layout,
            filters: &stored.filters,
            refresh_seconds: stored.refresh_seconds,
            grain: &stored.grain,
            params: &plan.params,
            tabs: &stored.tabs,
            phone_layout: &plan.phone_layout,
        },
    )
    .await
    .map_err(|err| classify_bi_error(&err))?;
    let version = snapshot_version(ch, id, Some(&principal)).await;
    Ok(ApiJson(json!({
        "ok": true,
        "restoredFrom": body.version,
        "version": version,
        "chartsRemoved": plan.added_since.len(),
        "chartsNotBrought": plan.skipped,
    })))
}

// ── Trash ───────────────────────────────────────────────────────────────

/// `GET /api/dashboard/trash` — deleted dashboards, and charts deleted on
/// their own, that the caller may see; 30 days each.
///
/// # Errors
///
/// 503 when the store is unreachable.
pub async fn trash_list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let items = trash::list_trash(&state.clickhouse, &viewer_of(&principal))
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(
        json!({ "items": items, "retentionDays": trash::RETENTION_DAYS }),
    ))
}

/// `POST /api/dashboard/trash/restore` body.
#[derive(Debug, Deserialize)]
pub struct RestoreBody {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    id: String,
}

/// `POST /api/dashboard/trash/restore` — bring a dashboard (with the charts its
/// delete took) or a chart back. Needs `dashboard:write`, as deleting does.
///
/// # Errors
///
/// 400 for a bad body, 404 when the item is not in the trash for the caller,
/// 503 when the store is unreachable.
pub async fn trash_restore(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: RestoreBody = parse(&body)?;
    let id = body.id.trim();
    if id.is_empty() {
        return Err(ApiError::BadRequest("id is required.".to_owned()).into());
    }
    let ch = &state.clickhouse;
    let viewer = viewer_of(&principal);
    let outcome = match Kind::parse(body.kind.trim()) {
        Some(Kind::Board) => trash::restore_board(ch, id, &viewer).await,
        Some(Kind::Chart) => trash::restore_chart(ch, id, &viewer).await,
        None => {
            return Err(ApiError::BadRequest("kind must be board or chart.".to_owned()).into());
        }
    }
    .map_err(|err| match err {
        // "Not in the trash" is our own sentence; make it a 404.
        BiError::Validation(message) => ApiError::NotFound(message),
        BiError::Clickhouse(cause) => classify_ch_error(&cause),
    })?;
    // A restored dashboard (or a chart back on a live one) changes what that
    // dashboard holds.
    match outcome.kind {
        "board" => snapshot(ch, id, &principal).await,
        _ => {
            if let Some((_, board)) = store::chart_row_state(ch, id)
                .await
                .map_err(|err| classify_ch_error(&err))?
            {
                snapshot(ch, &board, &principal).await;
            }
        }
    }
    Ok(ApiJson(json!({ "ok": true, "restored": outcome })))
}

// ── Favourites, recent, search ──────────────────────────────────────────

/// `GET /api/dashboard/shortcuts` — the caller's favourites and recent items,
/// resolved to what is live and visible to them.
///
/// # Errors
///
/// 503 when the store is unreachable.
pub async fn shortcuts(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let viewer = viewer_of(&principal);
    let (favourites, recent) = tokio::try_join!(
        items::entries(ch, &viewer, List::Favourite),
        items::entries(ch, &viewer, List::Recent)
    )
    .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(
        json!({ "favourites": favourites, "recent": recent }),
    ))
}

/// `PUT /api/dashboard/favourites` and `POST /api/dashboard/recent` body.
#[derive(Debug, Deserialize)]
pub struct ItemBody {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    id: String,
    /// Favourites only: `true` marks, `false` unmarks.
    #[serde(default)]
    on: Option<bool>,
}

fn item_of(body: &ItemBody) -> Result<(Kind, String), ApiError> {
    let kind = Kind::parse(body.kind.trim())
        .ok_or_else(|| ApiError::BadRequest("kind must be board or chart.".to_owned()))?;
    let id = body.id.trim();
    if id.is_empty() {
        return Err(ApiError::BadRequest("id is required.".to_owned()));
    }
    Ok((kind, id.to_owned()))
}

/// `PUT /api/dashboard/favourites` — mark or unmark a favourite.
///
/// # Errors
///
/// 400 for a bad body or the limit, 404 for an item the caller cannot see, 503
/// when the store is unreachable.
pub async fn favourite_set(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: ItemBody = parse(&body)?;
    let (kind, id) = item_of(&body)?;
    items::set_favourite(
        &state.clickhouse,
        &viewer_of(&principal),
        kind,
        &id,
        body.on.unwrap_or(true),
    )
    .await
    .map_err(item_error)?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// `POST /api/dashboard/recent` — note that the caller opened an item.
///
/// # Errors
///
/// As [`favourite_set`].
pub async fn recent_touch(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: ItemBody = parse(&body)?;
    let (kind, id) = item_of(&body)?;
    items::touch_recent(&state.clickhouse, &viewer_of(&principal), kind, &id)
        .await
        .map_err(item_error)?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// A list write's failure: the "not found" sentence is a 404, the limit a 400.
fn item_error(err: BiError) -> ApiError {
    match err {
        BiError::Validation(message) if message.starts_with("That item") => {
            ApiError::NotFound(message)
        }
        other => classify_bi_error(&other),
    }
}

/// `GET /api/dashboard/search?q=` query.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
}

/// `GET /api/dashboard/search?q=` — dashboards and charts whose name contains
/// the text, across every folder the caller may read.
///
/// # Errors
///
/// 400 for a text that is too long, 503 when the store is unreachable.
pub async fn search_names(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<ApiJson<Value>> {
    let hits = search::search(
        &state.clickhouse,
        &viewer_of(&principal),
        q.q.as_deref().unwrap_or(""),
    )
    .await
    .map_err(|err| classify_bi_error(&err))?;
    Ok(ApiJson(json!({ "hits": hits })))
}
