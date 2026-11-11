//! `list_maps` (`BI-16` part C2, plan decision D10): the maps a chart can be
//! drawn on, so the assistant can set a map chart's `map` to an id that exists
//! instead of guessing one.
//!
//! The bundled maps are owned by the console (`MAP_CATALOGUE` in
//! `echarts-maps.ts`; the server only checks the shape of a map id), so the
//! fixed ones are listed here by hand and the per-country ones by the rule
//! that makes their id. The uploaded maps come from Postgres. Reading needs
//! only `dashboard:read` (a map is visible to everyone who can make charts).
//!
//! This module is also the ONE place that knows which maps exist: `list_maps`
//! shows them and [`require_known_map`] refuses a chart call that names a map
//! that does not exist (`AI-6`), so the list and the check cannot drift apart.

use serde_json::{Map, Value, json};

use lakehouse_store::geo_map::GeoMap;

use crate::state::AppState;
use crate::upstream_error;

/// Whether `name` is `list_maps` (one guard arm in `run_tool`, which is at
/// clippy's line limit).
pub(super) fn handles(name: &str) -> bool {
    name == "list_maps"
}

/// The maps shipped with the console, by id and label. Kept next to the
/// console's catalogue by name; a test pins the ids. This is the ONE list of
/// the built-in ids: `list_maps` shows them and [`require_known_map`] refuses
/// a `map` that is not one of them.
const BUILT_IN: [(&str, &str, &str); 4] = [
    ("dki-jakarta", "Jakarta - cities", "Jakarta"),
    ("id-provinces", "Indonesia - provinces", "Indonesia"),
    ("id-regencies", "Indonesia - kabupaten/kota", "Indonesia"),
    ("world", "World - countries", "the world"),
];

/// How many uploaded maps the refusal sentence may name. Past that the
/// sentence would be unreadable, so it names none of them and points at
/// `list_maps` instead.
const MAX_NAMED_UPLOADED: usize = 10;

/// The longest id the refusal echoes, in characters. The shape the server
/// accepts is at most that long (`lakehouse_bi`'s `map_id_is_well_formed`),
/// so a well-formed id is echoed whole and a longer one is cut.
const MAX_ECHOED_CHARS: usize = 40;

/// The uploaded maps, oldest first (the order the console lists them), or the
/// classified store error: the store's own text never reaches the caller
/// (`AGENTS.md` principle 4, `SEC-11`).
///
/// # Errors
///
/// The classified store error (`upstream_error::internal_error`), never the store's own text.
async fn uploaded_maps(state: &AppState) -> Result<Vec<GeoMap>, Value> {
    let Some(pool) = state.pg.as_deref() else {
        // No Postgres: no uploaded map is known, and no store is asked.
        return Ok(Vec::new());
    };
    match lakehouse_store::geo_map::list(pool).await {
        Ok(rows) => Ok(rows),
        Err(err) => {
            let failed = upstream_error::internal_error(&upstream_error::STORE, &err);
            Err(json!({ "error": failed.to_string() }))
        }
    }
}

/// List the built-in and uploaded maps.
pub(super) async fn list_maps(state: &AppState) -> Value {
    let mut maps: Vec<Value> = BUILT_IN
        .iter()
        .map(|(id, label, covers)| json!({ "id": id, "label": label, "source": "built-in", "covers": covers }))
        .collect();
    match uploaded_maps(state).await {
        Ok(rows) => maps.extend(rows.into_iter().map(|m| {
            json!({ "id": m.id, "label": m.name, "source": "uploaded", "regions": m.region_count })
        })),
        Err(error) => return error,
    }
    json!({
        "maps": maps,
        "countryMaps": "The states or provinces of any country are the map `adm1-` followed by its ISO 3166-1 alpha-3 code in lower case, for example adm1-usa or adm1-bra; Indonesia's provinces are id-provinces.",
        "use": "Set `map` of a geomap, pointmap or geoheat chart to an id above. A region chart joins its dimension to the map's region names (a country map also by ISO code).",
    })
}

/// Refuse a chart call whose `map` names no map that exists (`AI-6`).
///
/// The model is told which maps it may use, so it does not save a chart on a
/// map the console cannot draw (the tile would say "Map not available"). The
/// ids are the ones this module owns: a built-in one, an uploaded one, or a
/// country's map (`adm1-` and its ISO 3166-1 alpha-3 code in lower case).
///
/// A call that names no `map` (an `update_chart` that keeps the stored one),
/// a `null` map and a blank map are not checked: `lakehouse_bi`'s
/// `geo_fields` stores nothing for a blank one.
///
/// # Errors
///
/// The refusal the model reads, or the classified store error.
pub(super) async fn require_known_map(
    state: &AppState,
    name: &str,
    args: &Map<String, Value>,
) -> Result<(), Value> {
    if name != "create_chart" && name != "update_chart" {
        return Ok(());
    }
    let map = super::arg_str(args, "map").trim().to_owned();
    if map.is_empty() {
        return Ok(());
    }
    let uploaded = uploaded_maps(state).await?;
    if is_known(&map, &uploaded) {
        return Ok(());
    }
    Err(json!({ "error": refusal_sentence(&map, &uploaded) }))
}

/// Whether `id` names a map that exists: a built-in one, an uploaded one, or
/// a country's map.
fn is_known(id: &str, uploaded: &[GeoMap]) -> bool {
    BUILT_IN.iter().any(|(built_in, _, _)| *built_in == id)
        || uploaded.iter().any(|m| m.id == id)
        || is_country_map(id)
}

/// Whether `id` is a country's map: `adm1-` and exactly three lower-case
/// ASCII letters. The server holds no country list, so the shape is all there
/// is to check — the rule `list_maps` describes in its `countryMaps` text.
fn is_country_map(id: &str) -> bool {
    let Some(code) = id.strip_prefix("adm1-") else {
        return false;
    };
    code.len() == 3 && code.bytes().all(|b| b.is_ascii_lowercase())
}

/// The sentence a chart call is refused with when its `map` names no map that
/// exists: the id the model used and the maps it may use.
fn refusal_sentence(id: &str, uploaded: &[GeoMap]) -> String {
    let mut choices: Vec<&str> = BUILT_IN.iter().map(|(built_in, _, _)| *built_in).collect();
    if uploaded.len() <= MAX_NAMED_UPLOADED {
        choices.extend(uploaded.iter().map(|m| m.id.as_str()));
    }
    choices.push("adm1-<iso3> for a country's provinces");
    format!(
        "map '{id}' does not exist. Use one of: {choices}, or an uploaded map from list_maps.",
        id = echoed(id),
        choices = choices.join(", ")
    )
}

/// The id as the refusal shows it: at most [`MAX_ECHOED_CHARS`] characters, so
/// a model cannot put a long text in the reply. A well-formed id (at most
/// that long) is echoed whole.
fn echoed(id: &str) -> String {
    id.chars().take(MAX_ECHOED_CHARS).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;
    use lakehouse_auth::Principal;

    #[test]
    fn the_built_in_ids_are_the_consoles() {
        let ids: Vec<&str> = BUILT_IN.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(
            ids,
            ["dki-jakarta", "id-provinces", "id-regencies", "world"]
        );
        assert!(handles("list_maps") && !handles("list_charts"));
    }

    /// The per-country rule is the shape `list_maps` describes in its
    /// `countryMaps` text: `adm1-` and the country's ISO 3166-1 alpha-3 code
    /// in lower case. The server holds no country list, so a code that is not
    /// a real one cannot be told apart here.
    #[test]
    fn a_country_map_is_adm1_and_three_lower_case_letters() {
        assert!(is_country_map("adm1-usa"));
        assert!(is_country_map("adm1-bra"));
        assert!(!is_country_map("adm1-us"));
        assert!(!is_country_map("adm1-usaa"));
        assert!(!is_country_map("adm1-USA"));
        assert!(!is_country_map("adm1-us1"));
        assert!(!is_country_map("id-provinces"));
        assert!(!is_country_map("adm1-"));
    }

    /// The refusal names the maps the model may use, and the id it used: the
    /// built-in ones, the per-country rule and the uploaded maps (there are
    /// few enough of them to name).
    #[test]
    fn the_refusal_names_the_maps_the_model_may_use() {
        let sentence = refusal_sentence("indonesia", &[]);
        assert_eq!(
            sentence,
            "map 'indonesia' does not exist. Use one of: dki-jakarta, id-provinces, id-regencies, world, adm1-<iso3> for a country's provinces, or an uploaded map from list_maps."
        );
        assert!(sentence.contains("id-provinces"), "{sentence}");
    }

    /// An id the model made up is echoed, but only its first 40 characters:
    /// the shape the server accepts is at most that long.
    #[test]
    fn the_refusal_echoes_at_most_forty_characters_of_the_id() {
        let long = "a".repeat(60);
        let sentence = refusal_sentence(&long, &[]);
        assert!(sentence.contains(&format!("map '{}' does not exist", "a".repeat(40))));
        assert!(!sentence.contains(&"a".repeat(41)), "{sentence}");
    }

    /// An uploaded map row for the sentence tests: only the id is read.
    fn fixture_map(id: &str) -> GeoMap {
        GeoMap {
            id: id.to_owned(),
            name: "Fixture".to_owned(),
            name_property: "name".to_owned(),
            region_count: 2,
            created_by: "tester".to_owned(),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            region_lookup: false,
        }
    }

    /// The uploaded maps sit in the "one of" list after the built-in ones and
    /// before the per-country rule, when there are few enough to name.
    #[test]
    fn a_refusal_names_the_uploaded_maps_between_the_built_ins_and_the_country_rule() {
        let uploaded = vec![fixture_map("m_0123456789ab")];
        let sentence = refusal_sentence("indonesia", &uploaded);
        assert!(
            sentence.contains("world, m_0123456789ab, adm1-<iso3> for a country's provinces"),
            "{sentence}"
        );
    }

    /// Past ten uploaded maps the refusal names none of them: the sentence
    /// would be unreadable, so it points at `list_maps` instead.
    #[test]
    fn a_refusal_names_no_uploaded_map_when_there_are_more_than_ten() {
        let uploaded: Vec<GeoMap> = (0..11)
            .map(|i| fixture_map(&format!("m_{i:012x}")))
            .collect();
        let sentence = refusal_sentence("indonesia", &uploaded);
        assert!(!sentence.contains("m_000000000000"), "{sentence}");
        assert!(
            sentence.contains("world, adm1-<iso3> for a country's provinces"),
            "{sentence}"
        );
    }

    /// A state whose Postgres is the test pool and whose `ClickHouse` is
    /// unreachable: the map check needs no dashboard and no engine, so a call
    /// that names a map that does not exist is refused before either is
    /// reached.
    fn state_over_pg(pool: &sqlx::PgPool) -> AppState {
        let options = pool.connect_options();
        let url = format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database")
        );
        let env = HashMap::from([
            ("DATABASE_URL".to_owned(), url),
            ("CH_URL".to_owned(), "http://127.0.0.1:1".to_owned()),
        ]);
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    /// A caller who may make charts (`dashboard:write`), as the write tools'
    /// gate reads them.
    fn writer() -> Principal {
        Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::from_u128(9)),
            tenant_ids: Vec::new(),
            display_name: "Rina".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("dashboard:read, dashboard:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// `AI-6`: an uploaded map is a map that exists, so a chart may name it;
    /// an uploaded id the store does not hold is refused, and the refusal
    /// names the uploaded map the store does hold.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_uploaded_map_may_be_named_and_an_uploaded_id_the_store_does_not_hold_may_not(
        pool: sqlx::PgPool,
    ) {
        let state = state_over_pg(&pool);
        lakehouse_store::geo_map::insert(
            &pool,
            &lakehouse_store::geo_map::NewGeoMap {
                id: "m_0123456789ab",
                name: "Fixture provinces",
                name_property: "name",
                region_count: 2,
                geojson: b"{}",
                created_by: "tester",
            },
        )
        .await
        .expect("the map row");

        let mut args = Map::new();
        args.insert("map".to_owned(), json!("m_0123456789ab"));
        let saved = super::super::run_tool(&state, Some(&writer()), "create_chart", &args).await;
        assert!(
            !saved["error"]
                .as_str()
                .unwrap_or_default()
                .contains("does not exist. Use one of:"),
            "an uploaded map may be named: {saved}"
        );

        args.insert("map".to_owned(), json!("m_ffffffffffff"));
        let saved = super::super::run_tool(&state, Some(&writer()), "create_chart", &args).await;
        let sentence = saved["error"].as_str().unwrap_or_default();
        assert!(
            sentence.starts_with("map 'm_ffffffffffff' does not exist."),
            "{saved}"
        );
        assert!(sentence.contains("m_0123456789ab"), "{saved}");
    }
}
