//! `list_maps` (`BI-16` part C2, plan decision D10): the maps a chart can be
//! drawn on, so the assistant can set a map chart's `map` to an id that exists
//! instead of guessing one.
//!
//! The bundled maps are owned by the console (`MAP_CATALOGUE` in
//! `echarts-maps.ts`; the server only checks the shape of a map id), so the
//! fixed ones are listed here by hand and the per-country ones by the rule
//! that makes their id. The uploaded maps come from Postgres. Reading needs
//! only `dashboard:read` (a map is visible to everyone who can make charts).

use serde_json::{Value, json};

use crate::state::AppState;
use crate::upstream_error;

/// Whether `name` is `list_maps` (one guard arm in `run_tool`, which is at
/// clippy's line limit).
pub(super) fn handles(name: &str) -> bool {
    name == "list_maps"
}

/// The maps shipped with the console, by id and label. Kept next to the
/// console's catalogue by name; a test pins the ids.
const BUILT_IN: [(&str, &str, &str); 4] = [
    ("dki-jakarta", "Jakarta - cities", "Jakarta"),
    ("id-provinces", "Indonesia - provinces", "Indonesia"),
    ("id-regencies", "Indonesia - kabupaten/kota", "Indonesia"),
    ("world", "World - countries", "the world"),
];

/// List the built-in and uploaded maps.
pub(super) async fn list_maps(state: &AppState) -> Value {
    let mut maps: Vec<Value> = BUILT_IN
        .iter()
        .map(|(id, label, covers)| json!({ "id": id, "label": label, "source": "built-in", "covers": covers }))
        .collect();
    if let Some(pool) = state.pg.as_deref() {
        match lakehouse_store::geo_map::list(pool).await {
            Ok(rows) => maps.extend(rows.into_iter().map(|m| {
                json!({ "id": m.id, "label": m.name, "source": "uploaded", "regions": m.region_count })
            })),
            Err(err) => {
                let failed = upstream_error::internal_error(&upstream_error::STORE, &err);
                return json!({ "error": failed.to_string() });
            }
        }
    }
    json!({
        "maps": maps,
        "countryMaps": "The states or provinces of any country are the map `adm1-` followed by its ISO 3166-1 alpha-3 code in lower case, for example adm1-usa or adm1-bra; Indonesia's provinces are id-provinces.",
        "use": "Set `map` of a geomap, pointmap or geoheat chart to an id above. A region chart joins its dimension to the map's region names (a country map also by ISO code).",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_ids_are_the_consoles() {
        let ids: Vec<&str> = BUILT_IN.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(
            ids,
            ["dki-jakarta", "id-provinces", "id-regencies", "world"]
        );
        assert!(handles("list_maps") && !handles("list_charts"));
    }
}
