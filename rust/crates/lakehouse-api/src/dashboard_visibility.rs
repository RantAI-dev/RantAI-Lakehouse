//! GOV-17: one API-owned resolver for "does the caller's CURRENT table grants
//! let them see this whole dashboard?".
//!
//! # The invariant
//!
//! A person restricted to some tables must not see a dashboard that depends on
//! any other table. Hiding only the tile's values while leaving its title,
//! chart labels, counts or error text visible does not meet that requirement.
//! So the resolver returns the set of board ids whose data dependency is not
//! readable, and the built-in board's own visibility, and the API narrows the
//! request's [`Viewer`] with them BEFORE any tile query is dispatched. The
//! store's existing ACL, personal-space and tenant predicates still decide
//! what is otherwise visible: this overlay only ever removes.
//!
//! # How dependencies are decided
//!
//! Dependencies come from metadata only — no chart data SQL is executed. The
//! stored chart's final SQL is parsed with the SAME
//! [`crate::sql_rewrite::referenced_tables`] the policy engine uses (no second
//! regex parser), and its structured definition (mart, SQL source, joins,
//! secondary/combined layers, SQL sources of those layers) is read as well so
//! a stale or missing final-SQL field cannot drop a dependency. SQL sources are
//! read in one batched metadata query. The built-in "Main" tiles from
//! `BUILTIN_DASHBOARD_SPEC` are parsed the same way.
//!
//! # Failing closed
//!
//! An unparseable statement, a missing SQL source, a store outage or an
//! oversized deny-list never widen access: the board is hidden or the request
//! fails with a classified error. There is no fallback to `Viewer::system`,
//! no empty-dependency shortcut and no cache that survives a grant change —
//! grants are read from Postgres on every request.

use std::collections::HashMap;

use lakehouse_auth::Principal;
use lakehouse_bi::access::{MAX_SCOPE_IDS, TooManyIds, Viewer};
use lakehouse_bi::sources::{self, SqlSource};
use lakehouse_bi::specs::ChartKind;
use lakehouse_bi::store::{self, ChartInput, StoredChartSpec};
use sqlparser::dialect::ClickHouseDialect;

use crate::state::AppState;
use crate::table_access::{self, TableAccess};
use crate::upstream_error;
use lakehouse_core::ApiError;

/// The table-dependent part of a caller's dashboard visibility. Empty lists
/// and `builtin_visible = true` mean "no narrowing" (the unrestricted case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableVisibility {
    /// Board ids whose required data dependency is unreadable.
    pub(crate) hidden_boards: Vec<String>,
    /// Whether the built-in "Main" board's dependencies are all readable.
    pub(crate) builtin_visible: bool,
    /// GOV-17 review R2: saved SQL source ids whose statement reads a table
    /// this caller may not read (or that cannot be resolved). Empty for an
    /// unrestricted caller. A source is a standalone resource too: its SQL and
    /// columns must not be listable merely because its folder is visible.
    pub(crate) hidden_sources: Vec<String>,
}

impl TableVisibility {
    pub(crate) fn unrestricted() -> Self {
        Self {
            hidden_boards: Vec::new(),
            builtin_visible: true,
            hidden_sources: Vec::new(),
        }
    }
}

/// Resolve the caller's effective visibility from a FRESH read of their table
/// grants and a batched metadata read of boards, charts and SQL sources.
///
/// # Errors
///
/// A classified store error (never upstream text) when the grants, charts or
/// sources cannot be read, or `Unavailable` with the fixed `TooManyIds`
/// sentence when the deny-list would exceed [`MAX_SCOPE_IDS`]. Either outcome
/// hides the dashboard; neither widens access.
pub(crate) async fn resolve_visibility(
    state: &AppState,
    principal: &Principal,
) -> Result<TableVisibility, ApiError> {
    resolve_visibility_for_roles(state, &principal.role_names).await
}

/// [`resolve_visibility`] for a read whose effective identity is a fixed set of
/// role names rather than a signed-in principal — a public link or a signed
/// embed reads as the Dashboard Viewer role (`embed::EMBED_VIEWER_ROLE`). No
/// new, unrestricted principal is invented here.
///
/// # Errors
///
/// As [`resolve_visibility`].
pub(crate) async fn resolve_visibility_for_roles(
    state: &AppState,
    roles: &[String],
) -> Result<TableVisibility, ApiError> {
    let access = table_access::for_roles(state.pg.as_deref(), roles)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    // The common, unrestricted case pays no ClickHouse metadata read at all.
    // A genuine `*:*`/`All`/no-role principal keeps established access.
    if access.is_all() {
        return Ok(TableVisibility::unrestricted());
    }

    // GOV-17: `system()` reads META-DATA only — chart/source rows are used to
    // collect dependency keys, never returned to a caller and never executed
    // as chart data SQL. No request handler sees these unfiltered rows.
    let system = Viewer::system();
    let (stored, sources) = tokio::try_join!(
        store::list_stored_charts(&state.clickhouse, &system),
        sources::list_sources(&state.clickhouse),
    )
    .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    let sources: HashMap<String, SqlSource> = sources
        .into_iter()
        .map(|source| (source.id.clone(), source))
        .collect();

    let mut hidden: Vec<String> = Vec::new();
    let mut builtin_visible = true;

    for chart in &stored {
        let Ok(keys) = stored_chart_tables(chart, &sources) else {
            hide_board(&mut hidden, &mut builtin_visible, &chart.board);
            continue;
        };
        if !tables_allowed(&access, &keys) {
            hide_board(&mut hidden, &mut builtin_visible, &chart.board);
        }
    }

    for kpi in lakehouse_bi::specs::kpis() {
        if !builtin_tables_allowed(&access, &kpi.sql, &kpi.mart) {
            builtin_visible = false;
        }
    }
    for chart in lakehouse_bi::specs::charts() {
        if !builtin_tables_allowed(&access, &chart.sql, &chart.mart) {
            builtin_visible = false;
        }
    }

    // GOV-17 review R2: a saved SQL source is a governed resource of its own.
    // A narrowed caller must not list/detail/preview a ROOT-folder source whose
    // SQL reads a denied table (or whose SQL no longer parses) merely because
    // its folder is visible. Reuse the same parser and canonical keys as the
    // chart dependency check; fail closed when a statement cannot be resolved.
    let mut hidden_sources: Vec<String> = Vec::new();
    for source in sources.values() {
        if !source_readable(&access, &source.sql) {
            hidden_sources.push(source.id.clone());
        }
    }
    hidden_sources.sort();
    hidden_sources.dedup();
    if hidden_sources.len() > MAX_SCOPE_IDS {
        // Fail closed: a truncated deny-list could SHOW a denied source's SQL.
        return Err(ApiError::Unavailable(TooManyIds::MESSAGE.to_owned()));
    }

    // GOV-17 review R1: when Main is hidden for ANY reason — a denied built-in
    // tile or a denied stored chart filed on it — its STORED tiles must also
    // disappear from `list_stored_charts`, which `/api/dashboard/specs`,
    // assistant list_charts, search and file export read directly without
    // `get_body`'s built-in filter. `chart_owner_clause` filters the raw stored
    // board column, so the `builtin_visible` flag alone cannot reach that SQL
    // clause (or the in-memory `sees_board` predicate). Add both legacy
    // spellings: they match no real board row and are deduplicated below.
    if !builtin_visible {
        hidden.push(String::new());
        hidden.push(store::DEFAULT_BOARD_ID.to_owned());
    }

    hidden.sort();
    hidden.dedup();
    if hidden.len() > MAX_SCOPE_IDS {
        // Fail closed: a truncated deny-list could SHOW a hidden dashboard.
        return Err(ApiError::Unavailable(TooManyIds::MESSAGE.to_owned()));
    }
    Ok(TableVisibility {
        hidden_boards: hidden,
        builtin_visible,
        hidden_sources,
    })
}

/// Attach a resolved visibility decision to a request viewer. Called by the
/// one resolver in `dashboard_access` so every HTTP and assistant read that
/// goes through `viewer_for`/`scope_for_write` is narrowed consistently.
pub(crate) fn narrow(viewer: Viewer, visibility: &TableVisibility) -> Result<Viewer, ApiError> {
    if visibility.hidden_boards.is_empty()
        && visibility.builtin_visible
        && visibility.hidden_sources.is_empty()
    {
        return Ok(viewer); // the unrestricted fast path is a true no-op
    }
    if visibility.hidden_boards.len() > MAX_SCOPE_IDS
        || visibility.hidden_sources.len() > MAX_SCOPE_IDS
    {
        return Err(ApiError::Unavailable(TooManyIds::MESSAGE.to_owned()));
    }
    Ok(viewer
        .narrowed_by_tables(visibility.hidden_boards.clone(), visibility.builtin_visible)
        .with_hidden_sources(visibility.hidden_sources.clone()))
}

/// A chart filed with an empty board id belongs to the built-in "Main" board.
/// `list_stored_charts` normalizes that empty id to `DEFAULT_BOARD_ID`, so both
/// spellings mean Main here.
fn hide_board(hidden: &mut Vec<String>, builtin_visible: &mut bool, board: &str) {
    if board.trim().is_empty() || board == store::DEFAULT_BOARD_ID {
        // Its tiles live on Main, so hiding the whole board means hiding Main
        // (and, on the `all` aggregate, excluding the filed-on-Main tiles).
        *builtin_visible = false;
    } else {
        hidden.push(board.to_owned());
    }
}

/// A mart name as the canonical `"serving.<mart>"` key the grants hold. An
/// absent/blank mart is not a dependency of its own — the final SQL is.
fn mart_key(mart: &str) -> Option<String> {
    let name = mart.trim().strip_prefix("serving.").unwrap_or(mart.trim());
    (!name.is_empty()).then(|| format!("serving.{name}"))
}

/// Every canonical table key a saved chart needs, from its final SQL AND its
/// structured definition. `Err(())` means a required dependency could not be
/// resolved (unparseable SQL, missing SQL source, or a data chart with no SQL)
/// and the caller must fail closed.
fn stored_chart_tables(
    chart: &StoredChartSpec,
    sources: &HashMap<String, SqlSource>,
) -> Result<Vec<String>, ()> {
    let mut keys = Vec::new();
    if is_static(chart.spec.kind) {
        // A text/heading/image/web tile genuinely reads no data. There is no
        // dependency to prove, so it cannot hide a board on its own.
        return Ok(keys);
    }
    // A chart built on a saved SQL source can have empty stored SQL: its
    // statement is assembled at read time from the source. Follow the source
    // instead of mistaking that for "no dependency" and hiding the board.
    if !chart.spec.sql.trim().is_empty() {
        keys.extend(sql_tables(&chart.spec.sql)?);
    }
    if let Some(key) = mart_key(&chart.spec.mart) {
        keys.push(key);
    }
    definition_tables(&chart.def, sources, &mut keys)?;
    if keys.is_empty() {
        // Neither final SQL, mart nor a source proved a table: cannot show
        // that this data chart reads no governed table (fail closed).
        return Err(());
    }
    Ok(keys)
}

/// The dependencies a `ChartInput` adds beyond the final SQL: its own source
/// and joins, and every secondary/combined layer's source and joins.
fn definition_tables(
    def: &ChartInput,
    sources: &HashMap<String, SqlSource>,
    keys: &mut Vec<String>,
) -> Result<(), ()> {
    if let Some(id) = def.sql_source.as_deref() {
        source_tables(id, sources, keys)?;
    }
    if let Some(key) = mart_key(&def.mart) {
        keys.push(key);
    }
    for join in def.shape.joins_list() {
        if let Some(key) = mart_key(&join.mart) {
            keys.push(key);
        }
    }
    for layer in def.shape.layers_list() {
        if let Some(key) = mart_key(&layer.mart) {
            keys.push(key);
        }
        if let Some(id) = layer.sql_source.as_deref() {
            source_tables(id, sources, keys)?;
        }
        for join in layer.joins.iter().flatten() {
            if let Some(key) = mart_key(&join.mart) {
                keys.push(key);
            }
        }
    }
    Ok(())
}

/// Follow one saved SQL source id to the tables its statement reads. A source
/// that is missing or whose SQL no longer parses is unresolvable (fail closed).
fn source_tables(
    id: &str,
    sources: &HashMap<String, SqlSource>,
    keys: &mut Vec<String>,
) -> Result<(), ()> {
    let id = id.trim();
    if id.is_empty() {
        return Ok(());
    }
    let source = sources.get(id).ok_or(())?;
    keys.extend(sql_tables(&source.sql)?);
    Ok(())
}

/// Parse a statement with the ONE parser the policy engine uses; `None` is
/// "cannot prove this reads no governed table", i.e. fail closed.
fn sql_tables(sql: &str) -> Result<Vec<String>, ()> {
    crate::sql_rewrite::referenced_tables(sql, &ClickHouseDialect {}).ok_or(())
}

/// Whether every required table key is readable. A key that does not
/// canonicalize to a qualified `schema.table` is refused for a narrowed
/// caller, exactly as `TableAccess::allows_key` does.
fn tables_allowed(access: &TableAccess, keys: &[String]) -> bool {
    keys.iter().all(|key| access.allows_key(key))
}

/// GOV-17 review R2: whether a saved SQL source's statement reads only tables
/// the caller may read. Reuses the one parser and canonical keys as the chart
/// check; unparseable SQL is unresolvable and therefore hidden (fail closed).
/// A genuinely data-free statement reads no governed table and stays visible.
fn source_readable(access: &TableAccess, sql: &str) -> bool {
    sql_tables(sql).is_ok_and(|keys| tables_allowed(access, &keys))
}

fn builtin_tables_allowed(access: &TableAccess, sql: &str, mart: &str) -> bool {
    let Ok(mut keys) = sql_tables(sql) else {
        return false;
    };
    if let Some(key) = mart_key(mart) {
        keys.push(key);
    }
    tables_allowed(access, &keys)
}

/// Kinds that render without reading any table (GOV-17's "genuinely static"
/// exception). Everything else is a data chart whose empty SQL is a broken
/// dependency, not a static tile.
fn is_static(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Text | ChartKind::Heading | ChartKind::Image | ChartKind::Web
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use lakehouse_bi::shape::MeasureList;
    use lakehouse_bi::store::ChartSpec;
    use std::collections::HashSet;

    fn listed(tables: &[(&str, &str)]) -> TableAccess {
        let t = tables
            .iter()
            .map(|(s, t)| ((*s).to_owned(), (*t).to_owned()))
            .collect::<HashSet<_>>();
        TableAccess::Listed(t, HashSet::new())
    }

    fn source(id: &str, sql: &str) -> SqlSource {
        SqlSource {
            id: id.to_owned(),
            title: id.to_owned(),
            sql: sql.to_owned(),
            columns: Vec::new(),
            folder_id: String::new(),
            created_by: String::new(),
            updated_at: None,
        }
    }

    fn chart(sql: &str, mart: &str, kind: ChartKind) -> StoredChartSpec {
        StoredChartSpec {
            spec: ChartSpec {
                id: "c".to_owned(),
                title: "c".to_owned(),
                subtitle: None,
                kind,
                mart: mart.to_owned(),
                sql_source: None,
                sql: sql.to_owned(),
                x: String::new(),
                y: lakehouse_bi::specs::ChartY::Single("v".to_owned()),
                series: None,
                map: None,
                lat: None,
                lon: None,
                format: None,
                span: None,
                text: None,
                caption: None,
                target: None,
                click: None,
            },
            source: lakehouse_bi::specs::ChartSource::Ui,
            board: String::new(),
            def: ChartInput {
                title: "c".to_owned(),
                subtitle: None,
                mart: mart.to_owned(),
                sql_source: None,
                kind,
                dimension: String::new(),
                measures: MeasureList::default(),
                breakdown: None,
                map: None,
                lat: None,
                lon: None,
                aggregate: None,
                limit: None,
                order: None,
                span: None,
                board: None,
                text: None,
                caption: None,
                target: None,
                click: None,
                grain: None,
                tables: lakehouse_bi::tables::TableFields::default(),
                shape: lakehouse_bi::shape::ShapeFields::default(),
                style: lakehouse_bi::style::StyleFields::default(),
                types: Box::default(),
            },
            has_year: false,
            created_by: None,
            created_at: None,
        }
    }

    #[test]
    fn final_sql_and_mart_are_both_required_dependencies() {
        let keys = stored_chart_tables(
            &chart("SELECT * FROM serving.allowed", "allowed", ChartKind::Kpi),
            &HashMap::new(),
        )
        .unwrap();
        assert!(keys.iter().any(|k| k == "serving.allowed"));
        let access = listed(&[("serving", "allowed")]);
        assert!(tables_allowed(&access, &keys));
        let narrowed = listed(&[("serving", "other")]);
        assert!(!tables_allowed(&narrowed, &keys));
    }

    #[test]
    fn a_joined_layer_table_is_a_dependency_even_if_final_sql_is_stale() {
        let mut c = chart("SELECT * FROM serving.base", "base", ChartKind::Bar);
        c.def.shape.layers = Some(vec![lakehouse_bi::layers::LayerDef {
            mart: "layer_mart".to_owned(),
            ..Default::default()
        }]);
        let access = listed(&[("serving", "base")]);
        let keys = stored_chart_tables(&c, &HashMap::new()).unwrap();
        assert!(
            !tables_allowed(&access, &keys),
            "the layer's mart is required"
        );
        let access = listed(&[("serving", "base"), ("serving", "layer_mart")]);
        assert!(tables_allowed(&access, &keys));
    }

    #[test]
    fn a_missing_or_broken_sql_source_is_unresolvable() {
        let mut c = chart("", "", ChartKind::Bar);
        c.def.sql_source = Some("s_missing".to_owned());
        assert!(stored_chart_tables(&c, &HashMap::new()).is_err());
        c.def.sql_source = Some("s_broken".to_owned());
        let sources = HashMap::from([(
            "s_broken".to_owned(),
            source("s_broken", "NOT SQL AT ALL ]]"),
        )]);
        assert!(stored_chart_tables(&c, &sources).is_err());
    }

    #[test]
    fn a_sql_sources_tables_are_followed() {
        let mut c = chart("", "", ChartKind::Bar);
        c.def.sql_source = Some("s_1".to_owned());
        let sources = HashMap::from([(
            "s_1".to_owned(),
            source("s_1", "SELECT * FROM serving.source_mart"),
        )]);
        let keys = stored_chart_tables(&c, &sources).unwrap();
        assert!(keys.iter().any(|k| k == "serving.source_mart"));
    }

    /// GOV-17 review R2: a saved SQL source that reads a denied (or unqualified)
    /// table is hidden; a source on an allowed table, a data-free source and a
    /// caller with unrestricted access are not. Unparseable SQL fails closed.
    #[test]
    fn a_sql_source_is_hidden_only_when_its_tables_are_unreadable() {
        let narrowed = listed(&[("serving", "allowed")]);
        assert!(source_readable(&narrowed, "SELECT * FROM serving.allowed"));
        assert!(!source_readable(&narrowed, "SELECT * FROM serving.denied"));
        assert!(!source_readable(&narrowed, "SELECT * FROM unqualified"));
        assert!(!source_readable(&narrowed, "not SQL ]]"));
        assert!(source_readable(&narrowed, "SELECT 1 AS n"));
        let all = TableAccess::All;
        assert!(source_readable(&all, "SELECT * FROM serving.denied"));
    }

    #[test]
    fn a_text_tile_has_no_dependency_and_cannot_hide_a_board() {
        let keys = stored_chart_tables(&chart("", "", ChartKind::Text), &HashMap::new()).unwrap();
        assert_eq!(keys, Vec::<String>::new());
    }

    #[test]
    fn unparseable_final_sql_is_unresolvable() {
        assert!(
            stored_chart_tables(&chart("]] broken", "m", ChartKind::Kpi), &HashMap::new()).is_err()
        );
    }

    #[test]
    fn an_unparseable_builtin_tile_hides_main() {
        let access = listed(&[("serving", "mart_x")]);
        assert!(!builtin_tables_allowed(&access, "]]", "mart_x"));
        assert!(builtin_tables_allowed(
            &access,
            "SELECT 1 FROM serving.mart_x",
            "mart_x"
        ));
        assert!(!builtin_tables_allowed(
            &access,
            "SELECT 1 FROM serving.denied_mart",
            "denied_mart"
        ));
    }

    #[test]
    fn hiding_a_filed_on_main_chart_hides_main_not_a_named_board() {
        let mut hidden = Vec::new();
        let mut builtin = true;
        hide_board(&mut hidden, &mut builtin, "");
        assert_eq!(hidden, Vec::<String>::new());
        assert!(!builtin);
        // `list_stored_charts` normalizes an empty board id to `default`.
        hide_board(&mut hidden, &mut builtin, "default");
        assert!(!builtin);
        hide_board(&mut hidden, &mut builtin, "b1");
        assert_eq!(hidden, ["b1".to_owned()]);
    }

    /// The resolver's output must narrow an `all` viewer too: identity:write
    /// alone cannot bypass a denied table dependency (GOV-17 decision 3).
    #[test]
    fn a_resolved_visibility_narrows_an_all_viewer() {
        let visibility = TableVisibility {
            hidden_boards: vec!["b".to_owned()],
            builtin_visible: false,
            hidden_sources: Vec::new(),
        };
        let viewer = narrow(Viewer::user("u", true), &visibility).unwrap();
        assert!(!viewer.sees_board("b", ""));
        assert!(!viewer.builtin_visible());
    }
}
