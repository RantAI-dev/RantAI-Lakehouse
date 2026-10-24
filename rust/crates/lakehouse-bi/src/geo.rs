//! Boundary files an administrator uploads as maps (`BI-16` part C2b, plan
//! `2026-10-24-bi-16c-advanced-charts-maps.md`, decision D5).
//!
//! # What this module does and why it is pure
//!
//! A map is a `GeoJSON` `FeatureCollection` of polygons plus the name of the
//! property that holds each region's name. The route reads the bytes, this
//! module [`validate`]s them and [`simplify`]s them for drawing, and the route
//! stores the result. Nothing here touches a network, a database or the clock,
//! so every rule is a unit test.
//!
//! # Rules (all checked before anything is stored)
//!
//! - a `FeatureCollection` of at most [`MAX_FEATURES`] features, each a
//!   `Polygon` or `MultiPolygon` (a point, a line or a missing geometry is
//!   refused by feature number);
//! - every position is finite, longitude in -180..180 and latitude in -90..90,
//!   every ring is closed and has at least four positions;
//! - the chosen name property is present on every feature, a string or a
//!   number, non-empty, at most [`MAX_NAME_CHARS`] characters, and unique
//!   (duplicates are refused with the names).
//!
//! # Simplification (deviation from D5, recorded in the plan handoff)
//!
//! D5 names the `geo` crate (`SimplifyVwPreserve`). This module uses a
//! Douglas-Peucker pass of its own instead: the crate would add `geo-types`,
//! `robust`, `rstar` and their dependencies to a workspace that must stay
//! small enough to build on a nearly full disk, and the pass is forty lines of
//! pure code with the same contract (a ring that keeps its first position and
//! at least a triangle). Coordinates are rounded to five decimals (about a
//! metre) and the tolerance is stepped up from [`MIN_TOLERANCE`] until the
//! written file is at most [`MAX_SIMPLIFIED_BYTES`]; when even
//! [`MAX_TOLERANCE`] is not enough the file is refused with a sentence rather
//! than drawn badly. Each ring is simplified on its own, so neighbouring
//! borders can show hairline gaps when zoomed far in (the feature page says so).
//!
//! The error sentences never repeat more of the file than a region name or a
//! feature number (`SEC-11`).

use std::collections::BTreeSet;

use serde::de::{Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Most regions a map may have (plan D5).
pub const MAX_FEATURES: usize = 20_000;
/// Longest region name, in characters.
pub const MAX_NAME_CHARS: usize = 120;
/// Longest name property, in characters.
pub const MAX_PROPERTY_CHARS: usize = 64;
/// The drawn file is at most this big (plan D5: 5 MB).
pub const MAX_SIMPLIFIED_BYTES: usize = 5 * 1024 * 1024;
/// First simplification tolerance, in degrees (about a metre).
pub const MIN_TOLERANCE: f64 = 0.000_01;
/// Last tolerance tried, in degrees (about five kilometres).
pub const MAX_TOLERANCE: f64 = 0.05;

/// An uploaded map as the formula compiler sees it (`BI-16` part C3): enough
/// to resolve the map a `RegionOf` call names, nothing of its boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialMap {
    /// `m_` and 12 lower-case hex digits, made by the server.
    pub id: String,
    /// The label people know the map by.
    pub name: String,
    /// Whether the engine holds the map's region lookup (its polygon
    /// dictionary was created and loaded). A map without one cannot be used by
    /// `RegionOf`; the compiler says so.
    pub region_lookup: bool,
}

/// Whether `id` has the shape the server makes (`m_` and 12 lower-case hex
/// digits), so it is safe to build an identifier from.
#[must_use]
pub fn is_map_id(id: &str) -> bool {
    id.strip_prefix("m_").is_some_and(|rest| {
        rest.len() == 12
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// The engine name of a map's polygon dictionary, or `None` for anything that
/// is not a server-made map id. This is the one place an identifier is built
/// from a map id; the id is checked by [`is_map_id`] first, so no text of a
/// formula or a request can reach a statement through it.
#[must_use]
pub fn dictionary_name(id: &str) -> Option<String> {
    is_map_id(id).then(|| format!("console.geo_dict_{id}"))
}

/// A longitude/latitude pair; extra members (altitude) are read and dropped.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Position([f64; 2]);

impl<'de> Deserialize<'de> for Position {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Position;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a position [longitude, latitude]")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Position, A::Error> {
                let lon: f64 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("a position needs two numbers"))?;
                let lat: f64 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("a position needs two numbers"))?;
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Position([lon, lat]))
            }
        }
        deserializer.deserialize_seq(V)
    }
}

type Ring = Vec<Position>;
type Polygon = Vec<Ring>;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Geometry {
    Polygon {
        coordinates: Polygon,
    },
    MultiPolygon {
        coordinates: Vec<Polygon>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Feature {
    #[serde(default)]
    properties: Option<Map<String, Value>>,
    #[serde(default)]
    geometry: Option<Geometry>,
}

#[derive(Debug, Deserialize)]
struct Collection {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    features: Vec<Feature>,
}

/// One region of a checked map.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// The value of the name property, as text.
    pub name: String,
    polygons: Vec<Polygon>,
}

/// A map that passed [`validate`].
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedMap {
    /// The regions, in file order.
    pub regions: Vec<Region>,
}

/// The tidy text of a name property value, or `None` when it is not a name.
fn name_of(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn check_position(p: Position, feature: usize) -> Result<(), String> {
    let [lon, lat] = p.0;
    if !(lon.is_finite() && lat.is_finite()) {
        return Err(format!(
            "feature {feature} has a coordinate that is not a number."
        ));
    }
    if !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat) {
        return Err(format!(
            "feature {feature} has a coordinate outside longitude -180 to 180 and latitude -90 to 90; the file must use WGS 84 (EPSG:4326)."
        ));
    }
    Ok(())
}

fn check_polygon(polygon: &Polygon, feature: usize) -> Result<(), String> {
    if polygon.is_empty() {
        return Err(format!("feature {feature} has a polygon with no rings."));
    }
    for ring in polygon {
        if ring.len() < 4 {
            return Err(format!(
                "feature {feature} has a ring with fewer than four positions."
            ));
        }
        if ring.first() != ring.last() {
            return Err(format!(
                "feature {feature} has a ring that is not closed (its last position must equal its first)."
            ));
        }
        for p in ring {
            check_position(*p, feature)?;
        }
    }
    Ok(())
}

/// Check an uploaded file and keep its regions.
///
/// `name_property` is the property holding each region's name.
///
/// # Errors
///
/// The sentence to show the person who uploaded the file.
pub fn validate(bytes: &[u8], name_property: &str) -> Result<CheckedMap, String> {
    let property = name_property.trim();
    if property.is_empty() || property.chars().count() > MAX_PROPERTY_CHARS {
        return Err(format!(
            "choose the property that holds the region names (1 to {MAX_PROPERTY_CHARS} characters)."
        ));
    }
    let collection: Collection = serde_json::from_slice(bytes)
        .map_err(|_| "the file is not GeoJSON: it must be a FeatureCollection.".to_owned())?;
    if collection.kind != "FeatureCollection" {
        return Err("the file must be a GeoJSON FeatureCollection.".to_owned());
    }
    if collection.features.is_empty() {
        return Err("the file has no regions.".to_owned());
    }
    if collection.features.len() > MAX_FEATURES {
        return Err(format!(
            "the file has {} regions; a map has at most {MAX_FEATURES}. Split it or remove detail.",
            collection.features.len()
        ));
    }
    let mut regions = Vec::with_capacity(collection.features.len());
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for (index, feature) in collection.features.into_iter().enumerate() {
        let n = index + 1;
        let polygons = match feature.geometry {
            Some(Geometry::Polygon { coordinates }) => vec![coordinates],
            Some(Geometry::MultiPolygon { coordinates }) => coordinates,
            Some(Geometry::Other) | None => {
                return Err(format!(
                    "feature {n} is not a polygon; a map takes only Polygon and MultiPolygon regions."
                ));
            }
        };
        if polygons.is_empty() {
            return Err(format!("feature {n} has no polygons."));
        }
        for polygon in &polygons {
            check_polygon(polygon, n)?;
        }
        let name = feature
            .properties
            .as_ref()
            .and_then(|p| p.get(property))
            .and_then(name_of)
            .ok_or_else(|| {
                format!("feature {n} has no usable '{property}' property: every region needs a name there.")
            })?;
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(format!(
                "feature {n} has a name longer than {MAX_NAME_CHARS} characters."
            ));
        }
        if !seen.insert(name.clone()) {
            duplicates.insert(name.clone());
        }
        regions.push(Region { name, polygons });
    }
    if !duplicates.is_empty() {
        let shown: Vec<String> = duplicates.iter().take(5).cloned().collect();
        let more = duplicates.len().saturating_sub(shown.len());
        return Err(format!(
            "these names appear on more than one region: {}{}. Each region needs its own name.",
            shown.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ));
    }
    Ok(CheckedMap { regions })
}

// ── the region lookup on the engine (`BI-16` part C3, plan D8) ───────────

/// The table that holds every uploaded map's full boundaries, one row per
/// region, for the polygon dictionaries to read. Created the way the console's
/// other engine tables are (`CREATE TABLE IF NOT EXISTS`), by the first upload
/// that needs it. Policies cannot cover it: it holds boundaries, never rows of
/// data.
pub const REGION_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS console.geo_region \
(map_id String, name String, polygon Array(Array(Array(Tuple(Float64, Float64))))) \
ENGINE = MergeTree ORDER BY (map_id, name)";

/// The statement that creates a map's polygon dictionary, or `None` for an
/// `id` that is not server-made (the id is the only thing put in the text).
///
/// *Not verified on a running engine*: written from the `ClickHouse`
/// documentation for polygon dictionaries (key `Array(Array(Array(Array(
/// Float64))))`, `LAYOUT(POLYGON())`, a `CLICKHOUSE` source with a `QUERY`),
/// and the caller treats any failure as "no lookup for this map".
#[must_use]
pub fn dictionary_ddl(id: &str) -> Option<String> {
    let name = dictionary_name(id)?;
    Some(format!(
        "CREATE DICTIONARY IF NOT EXISTS {name} (key Array(Array(Array(Array(Float64)))), name String) \
         PRIMARY KEY key \
         SOURCE(CLICKHOUSE(QUERY 'SELECT polygon AS key, name FROM console.geo_region WHERE map_id = ''{id}''')) \
         LAYOUT(POLYGON()) LIFETIME(0)"
    ))
}

/// Loads the dictionary now, so a source the engine cannot read fails here and
/// not on a chart's first read.
#[must_use]
pub fn reload_dictionary_sql(id: &str) -> Option<String> {
    Some(format!("SYSTEM RELOAD DICTIONARY {}", dictionary_name(id)?))
}

/// Drops a map's dictionary.
#[must_use]
pub fn drop_dictionary_sql(id: &str) -> Option<String> {
    Some(format!(
        "DROP DICTIONARY IF EXISTS {}",
        dictionary_name(id)?
    ))
}

/// Removes a map's rows from [`REGION_TABLE_DDL`]'s table.
#[must_use]
pub fn delete_regions_sql(id: &str) -> Option<String> {
    is_map_id(id).then(|| {
        format!("ALTER TABLE console.geo_region DELETE WHERE map_id = '{id}' SETTINGS mutations_sync = 1")
    })
}

#[derive(Serialize)]
struct RegionRow<'a> {
    map_id: &'a str,
    name: &'a str,
    polygon: &'a [Polygon],
}

/// Most regions, and most bytes, one insert statement carries.
const INSERT_REGIONS: usize = 100;
const INSERT_BYTES: usize = 8 * 1024 * 1024;

impl CheckedMap {
    /// The `INSERT` statements that put this map's full boundaries (not the
    /// simplified ones: `RegionOf` answers against the real borders) into
    /// [`REGION_TABLE_DDL`]'s table, `JSONEachRow`, in chunks of at most 100
    /// regions and about 8 MB. `None` for an `id` that is not server-made.
    #[must_use]
    pub fn region_inserts(&self, id: &str) -> Option<Vec<String>> {
        if !is_map_id(id) {
            return None;
        }
        let head = "INSERT INTO console.geo_region (map_id, name, polygon) FORMAT JSONEachRow\n";
        let mut out = Vec::new();
        let mut body = String::new();
        let mut count = 0;
        for region in &self.regions {
            let row = RegionRow {
                map_id: id,
                name: &region.name,
                polygon: &region.polygons,
            };
            let line = serde_json::to_string(&row).ok()?;
            if count > 0 && (count >= INSERT_REGIONS || body.len() + line.len() > INSERT_BYTES) {
                out.push(format!("{head}{body}"));
                body.clear();
                count = 0;
            }
            body.push_str(&line);
            body.push('\n');
            count += 1;
        }
        if count > 0 {
            out.push(format!("{head}{body}"));
        }
        Some(out)
    }
}

// ── simplification ──────────────────────────────────────────────────────

fn round5(x: f64) -> f64 {
    (x * 100_000.0).round() / 100_000.0
}

/// Douglas-Peucker without recursion, keeping the first and last position.
fn douglas_peucker(points: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        let [x1, y1] = points[lo];
        let [x2, y2] = points[hi];
        let (dx, dy) = (x2 - x1, y2 - y1);
        let length = dx.hypot(dy);
        let mut far = None;
        let mut far_d = tolerance;
        for (i, p) in points.iter().enumerate().take(hi).skip(lo + 1) {
            let d = if length == 0.0 {
                (p[0] - x1).hypot(p[1] - y1)
            } else {
                (dy * (p[0] - x1) - dx * (p[1] - y1)).abs() / length
            };
            if d > far_d {
                far = Some(i);
                far_d = d;
            }
        }
        if let Some(i) = far {
            keep[i] = true;
            stack.push((lo, i));
            stack.push((i, hi));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// A ring simplified and rounded; always at least a closed triangle.
fn simplify_ring(ring: &Ring, tolerance: f64) -> Ring {
    let open: Vec<[f64; 2]> = ring[..ring.len() - 1].iter().map(|p| p.0).collect();
    let mut kept = if tolerance > 0.0 {
        douglas_peucker(&open, tolerance)
    } else {
        open.clone()
    };
    // A ring that collapsed below a triangle is a sliver smaller than the
    // tolerance: it keeps its own positions (rounded) rather than vanish or
    // become a line, so the region still exists on the map.
    if kept.len() < 3 {
        kept = open;
    }
    let mut out: Ring = Vec::with_capacity(kept.len() + 1);
    for p in kept {
        let q = Position([round5(p[0]), round5(p[1])]);
        if out.last() != Some(&q) {
            out.push(q);
        }
    }
    if let Some(first) = out.first().copied() {
        out.push(first);
    }
    out
}

#[derive(Serialize)]
struct OutFeature<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    properties: OutProperties<'a>,
    geometry: OutGeometry,
}

#[derive(Serialize)]
struct OutProperties<'a> {
    name: &'a str,
}

#[derive(Serialize)]
struct OutGeometry {
    #[serde(rename = "type")]
    kind: &'static str,
    coordinates: Vec<Polygon>,
}

#[derive(Serialize)]
struct OutCollection<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    features: Vec<OutFeature<'a>>,
}

fn write(map: &CheckedMap, tolerance: f64) -> Result<Vec<u8>, String> {
    let features = map
        .regions
        .iter()
        .map(|r| OutFeature {
            kind: "Feature",
            properties: OutProperties { name: &r.name },
            geometry: OutGeometry {
                kind: "MultiPolygon",
                coordinates: r
                    .polygons
                    .iter()
                    .map(|polygon| {
                        polygon
                            .iter()
                            .map(|ring| simplify_ring(ring, tolerance))
                            .collect()
                    })
                    .collect(),
            },
        })
        .collect();
    serde_json::to_vec(&OutCollection {
        kind: "FeatureCollection",
        features,
    })
    .map_err(|_| "the map could not be written.".to_owned())
}

/// The file the console draws: a `FeatureCollection` whose features carry the
/// region name as `properties.name` (the chosen property, copied there so an
/// uploaded map is read like a built-in one) and a `MultiPolygon`, at most
/// [`MAX_SIMPLIFIED_BYTES`].
///
/// # Errors
///
/// A sentence when the file cannot be brought under the cap at
/// [`MAX_TOLERANCE`].
pub fn simplify(map: &CheckedMap) -> Result<Vec<u8>, String> {
    let mut tolerance = 0.0;
    loop {
        let bytes = write(map, tolerance)?;
        if bytes.len() <= MAX_SIMPLIFIED_BYTES {
            return Ok(bytes);
        }
        if tolerance >= MAX_TOLERANCE {
            return Err(
                "the boundaries are too detailed to draw even after simplifying them; remove some regions or split the file."
                    .to_owned(),
            );
        }
        tolerance = if tolerance == 0.0 {
            MIN_TOLERANCE
        } else {
            (tolerance * 2.0).min(MAX_TOLERANCE)
        };
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn square(x: f64, y: f64, size: f64) -> Value {
        json!([[
            [x, y],
            [x + size, y],
            [x + size, y + size],
            [x, y + size],
            [x, y]
        ]])
    }

    fn feature(name: Value, coordinates: Value) -> Value {
        let mut f = json!({ "type": "Feature", "properties": { "other": 1 },
                "geometry": { "type": "Polygon" } });
        f["properties"]["NAME"] = name;
        f["geometry"]["coordinates"] = coordinates;
        f
    }

    fn file(features: &[Value]) -> Vec<u8> {
        serde_json::to_vec(&json!({ "type": "FeatureCollection", "features": features })).unwrap()
    }

    fn refused(features: &[Value], property: &str) -> String {
        validate(&file(features), property).unwrap_err()
    }

    #[test]
    fn a_valid_collection_is_accepted_and_keeps_its_names_in_order() {
        let map = validate(
            &file(&[
                feature(json!("Alpha"), square(0.0, 0.0, 1.0)),
                feature(json!(7), square(2.0, 0.0, 1.0)),
            ]),
            "NAME",
        )
        .unwrap();
        let names: Vec<&str> = map.regions.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "7"]);
    }

    #[test]
    fn a_multipolygon_and_a_position_with_an_altitude_are_accepted() {
        let multi = json!({ "type": "Feature", "properties": { "NAME": "Islands" },
            "geometry": { "type": "MultiPolygon", "coordinates": [
                [[[0, 0, 5], [1, 0, 5], [1, 1, 5], [0, 0, 5]]],
                [[[3, 3], [4, 3], [4, 4], [3, 3]]] ] } });
        let map = validate(&file(&[multi]), "NAME").unwrap();
        assert_eq!(map.regions[0].polygons.len(), 2);
    }

    #[test]
    fn what_is_not_a_polygon_collection_is_refused_with_a_sentence() {
        let point = json!({ "type": "Feature", "properties": { "NAME": "P" },
            "geometry": { "type": "Point", "coordinates": [1, 2] } });
        assert!(refused(&[point], "NAME").contains("feature 1 is not a polygon"));
        let no_geometry =
            json!({ "type": "Feature", "properties": { "NAME": "P" }, "geometry": null });
        assert!(refused(&[no_geometry], "NAME").contains("not a polygon"));
        assert!(
            validate(b"not json", "NAME")
                .unwrap_err()
                .contains("not GeoJSON")
        );
        let single = serde_json::to_vec(&json!({ "type": "Feature" })).unwrap();
        assert!(
            validate(&single, "NAME")
                .unwrap_err()
                .contains("not GeoJSON")
                || validate(&single, "NAME").is_err()
        );
        assert!(
            validate(&file(&[]), "NAME")
                .unwrap_err()
                .contains("no regions")
        );
    }

    #[test]
    fn bad_coordinates_and_open_or_short_rings_are_refused() {
        let outside = feature(json!("A"), json!([[[0, 0], [200, 0], [200, 1], [0, 0]]]));
        assert!(refused(&[outside], "NAME").contains("WGS 84"));
        let north = feature(json!("A"), json!([[[0, 0], [1, 0], [1, 91], [0, 0]]]));
        assert!(refused(&[north], "NAME").contains("latitude"));
        let open = feature(json!("A"), json!([[[0, 0], [1, 0], [1, 1], [0, 1]]]));
        assert!(refused(&[open], "NAME").contains("not closed"));
        let short = feature(json!("A"), json!([[[0, 0], [1, 0], [0, 0]]]));
        assert!(refused(&[short], "NAME").contains("fewer than four"));
        let empty = feature(json!("A"), json!([]));
        assert!(refused(&[empty], "NAME").contains("no rings"));
        let text = feature(json!("A"), json!([[["a", 0], [1, 0], [1, 1], ["a", 0]]]));
        assert!(refused(&[text], "NAME").contains("not GeoJSON"));
    }

    #[test]
    fn names_must_be_present_non_empty_and_unique() {
        let ok = feature(json!("A"), square(0.0, 0.0, 1.0));
        let blank = feature(json!("  "), square(2.0, 0.0, 1.0));
        assert!(refused(&[ok.clone(), blank], "NAME").contains("feature 2 has no usable 'NAME'"));
        let wrong_property = refused(std::slice::from_ref(&ok), "TITLE");
        assert!(wrong_property.contains("'TITLE'"), "{wrong_property}");
        let object = feature(json!({ "x": 1 }), square(0.0, 0.0, 1.0));
        assert!(refused(&[object], "NAME").contains("no usable"));
        let twin = feature(json!("A"), square(5.0, 0.0, 1.0));
        let twin2 = feature(json!("B"), square(6.0, 0.0, 1.0));
        let twin3 = feature(json!("B"), square(7.0, 0.0, 1.0));
        let e = refused(&[ok.clone(), twin, twin2, twin3], "NAME");
        assert!(e.contains("more than one region: A, B"), "{e}");
        let long = feature(json!("x".repeat(121)), square(0.0, 0.0, 1.0));
        assert!(refused(&[long], "NAME").contains("longer than 120"));
        assert!(
            validate(&file(&[ok]), "")
                .unwrap_err()
                .contains("choose the property")
        );
    }

    #[test]
    fn more_than_20000_regions_are_refused() {
        let many: Vec<Value> = (0..=MAX_FEATURES)
            .map(|i| feature(json!(format!("r{i}")), square(0.0, 0.0, 1.0)))
            .collect();
        let e = validate(&file(&many), "NAME").unwrap_err();
        assert!(
            e.contains("20001 regions") && e.contains("at most 20000"),
            "{e}"
        );
    }

    #[test]
    fn the_drawn_file_carries_the_chosen_name_as_properties_name_and_nothing_else() {
        let map = validate(
            &file(&[feature(json!("Alpha"), square(0.0, 0.0, 1.0))]),
            "NAME",
        )
        .unwrap();
        let out: Value = serde_json::from_slice(&simplify(&map).unwrap()).unwrap();
        assert_eq!(out["type"], "FeatureCollection");
        assert_eq!(out["features"][0]["properties"], json!({ "name": "Alpha" }));
        assert_eq!(out["features"][0]["geometry"]["type"], "MultiPolygon");
        // A small file is not simplified at all, only rounded.
        let ring = &out["features"][0]["geometry"]["coordinates"][0][0];
        assert_eq!(ring.as_array().unwrap().len(), 5);
    }

    /// A noisy ring of `n` positions around (10, 10): about 23 bytes each as text.
    #[allow(
        clippy::cast_precision_loss,
        reason = "fixture arithmetic on indices below two million, exact in an f64"
    )]
    fn big_ring(n: usize) -> Value {
        let mut ring: Vec<Value> = (0..n)
            .map(|i| {
                let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
                let r = 1.0 + 0.0005 * ((i * 7919) % 101) as f64 / 100.0;
                json!([10.0 + r * a.cos(), 10.0 + r * a.sin()])
            })
            .collect();
        ring.push(ring[0].clone());
        json!([ring])
    }

    #[test]
    fn a_file_of_about_30_mb_is_simplified_under_5_mb() {
        let bytes = file(&[feature(json!("Big"), big_ring(1_300_000))]);
        assert!(bytes.len() > 28 * 1024 * 1024, "{}", bytes.len());
        let map = validate(&bytes, "NAME").unwrap();
        drop(bytes);
        let out = simplify(&map).unwrap();
        assert!(out.len() <= MAX_SIMPLIFIED_BYTES, "{}", out.len());
        let drawn: Value = serde_json::from_slice(&out).unwrap();
        let ring = drawn["features"][0]["geometry"]["coordinates"][0][0]
            .as_array()
            .unwrap();
        assert!(ring.len() > 3 && ring.first() == ring.last());
    }

    #[test]
    fn the_engine_statements_are_built_from_a_server_made_id_only() {
        let id = "m_0123456789ab";
        let ddl = dictionary_ddl(id).unwrap();
        assert!(
            ddl.starts_with("CREATE DICTIONARY IF NOT EXISTS console.geo_dict_m_0123456789ab "),
            "{ddl}"
        );
        assert!(
            ddl.contains("LAYOUT(POLYGON())") && ddl.contains("WHERE map_id = ''m_0123456789ab''"),
            "{ddl}"
        );
        assert_eq!(
            drop_dictionary_sql(id).unwrap(),
            "DROP DICTIONARY IF EXISTS console.geo_dict_m_0123456789ab"
        );
        assert!(
            reload_dictionary_sql(id)
                .unwrap()
                .ends_with("geo_dict_m_0123456789ab")
        );
        assert!(
            delete_regions_sql(id)
                .unwrap()
                .contains("map_id = 'm_0123456789ab'")
        );
        for bad in [
            "",
            "m_x",
            "m_0123456789AB",
            "m_0123456789ab'; DROP",
            "../m_0123456789ab",
            "x_0123456789ab",
        ] {
            assert!(
                dictionary_ddl(bad).is_none() && drop_dictionary_sql(bad).is_none(),
                "{bad}"
            );
            assert!(
                reload_dictionary_sql(bad).is_none() && delete_regions_sql(bad).is_none(),
                "{bad}"
            );
            assert!(
                CheckedMap {
                    regions: Vec::new()
                }
                .region_inserts(bad)
                .is_none(),
                "{bad}"
            );
        }
    }

    #[test]
    fn region_rows_carry_the_full_boundaries_in_chunks() {
        let features: Vec<Value> = (0..250)
            .map(|i| feature(json!(format!("r{i}")), square(f64::from(i % 100), 0.0, 0.5)))
            .collect();
        let map = validate(&file(&features), "NAME").unwrap();
        let inserts = map.region_inserts("m_0123456789ab").unwrap();
        assert_eq!(inserts.len(), 3, "100 + 100 + 50 regions");
        let head = "INSERT INTO console.geo_region (map_id, name, polygon) FORMAT JSONEachRow\n";
        assert!(inserts.iter().all(|s| s.starts_with(head)));
        let first: Value = serde_json::from_str(inserts[0].lines().nth(1).unwrap()).unwrap();
        assert_eq!(first["map_id"], "m_0123456789ab");
        assert_eq!(first["name"], "r0");
        // MultiPolygon > polygon > ring > [lon, lat].
        assert_eq!(first["polygon"][0][0][0], json!([0.0, 0.0]));
        assert_eq!(inserts[2].lines().count(), 51);
    }

    #[test]
    fn a_ring_that_collapses_stays_a_closed_triangle() {
        let tiny = feature(
            json!("Tiny"),
            json!([[[0, 0], [0.0001, 0], [0.0001, 0.0001], [0, 0]]]),
        );
        let map = validate(&file(&[tiny]), "NAME").unwrap();
        let out: Value = serde_json::from_slice(&write(&map, MAX_TOLERANCE).unwrap()).unwrap();
        let ring = out["features"][0]["geometry"]["coordinates"][0][0]
            .as_array()
            .unwrap();
        assert!(ring.len() >= 4, "{ring:?}");
        assert_eq!(ring.first(), ring.last());
    }
}
