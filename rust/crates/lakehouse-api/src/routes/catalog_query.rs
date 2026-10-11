//! Filter / sort / group / paginate over an already-assembled catalog, for
//! `GET /api/catalog/query`. Free-text search is not here: it ranks, so it
//! lives in [`super::catalog_search`] and runs before these steps
//! (`DATA-11` F1).
//!
//! # Why this is pure
//!
//! Everything here operates on a `&[Value]` slice that [`super::catalog`]
//! has already built. That is not an accident of style: the catalog cannot
//! be filtered in SQL. Bronze/raw assets come from registry tables in
//! `lake`, while Silver/Gold ones are read off `system.tables` — six
//! queries whose results are merged in Rust (see `catalog::list_body`), so
//! there is no single relation to attach a `WHERE` clause to.
//!
//! Keeping the query pipeline free of `ChClient` means the whole of it is
//! reachable from unit tests without a running `ClickHouse`, which is the
//! only way the operator matrix below gets covered at all.
//!
//! # Allowlists, not reflection
//!
//! Filter/sort/group fields are matched against [`FILTERABLE_FIELDS`] /
//! [`SORTABLE_FIELDS`] / [`GROUPABLE_FIELDS`] and rejected with a 400 if
//! unknown. A caller-supplied string never reaches a JSON lookup, so this
//! endpoint cannot be used to probe fields the list response does not
//! already expose.

use lakehouse_core::ApiError;
use serde_json::{Map, Value, json};

use crate::error::ApiRejection;

/// Fields a client may filter on. Every entry is a key the asset objects
/// `catalog::list_body` emits can carry; `tags` only exists on an asset
/// someone annotated, and reads as an empty list on the others
/// ([`ARRAY_FIELDS`]). `certification` (`DATA-12`) likewise exists only on
/// an asset with a mark and reads as empty on the others.
pub const FILTERABLE_FIELDS: &[&str] = &[
    "id",
    "name",
    "namespace",
    "type",
    "layer",
    "tier",
    "classification",
    "owner",
    "domain",
    "description",
    "format",
    "engine",
    "rows",
    "sizeBytes",
    "columnCount",
    "freshnessLagSeconds",
    "lastUpdated",
    "health",
    "residency",
    "certification",
    "tags",
];

/// Fields whose value is a list of strings rather than one scalar. They
/// filter with the any/none rules of [`matches_array_filter`].
const ARRAY_FIELDS: &[&str] = &["tags"];

/// Fields a client may sort on: the filterable ones except `tags`, which is
/// a list and has no order to sort by (`DATA-11`). Kept as its own list so
/// a new filterable field is not sortable by accident; a test pins the two
/// together.
pub const SORTABLE_FIELDS: &[&str] = &[
    "id",
    "name",
    "namespace",
    "type",
    "layer",
    "tier",
    "classification",
    "owner",
    "domain",
    "description",
    "format",
    "engine",
    "rows",
    "sizeBytes",
    "columnCount",
    "freshnessLagSeconds",
    "lastUpdated",
    "health",
    "residency",
    "certification",
];

/// Fields a client may group by. Restricted to the low-cardinality
/// categorical ones: grouping by `name` or `sizeBytes` would produce one
/// group per row, which is a denial-of-service shaped like a feature.
pub const GROUPABLE_FIELDS: &[&str] = &[
    "namespace",
    "type",
    "layer",
    "tier",
    "classification",
    "owner",
    "domain",
    "engine",
    "health",
    "residency",
    "certification",
];

/// How multiple filters combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOperator {
    And,
    Or,
}

impl JoinOperator {
    /// Anything other than an explicit `"or"` means `and` — matching the
    /// frontend's own normalisation in `use-table-url-state.ts`, which
    /// treats a missing or malformed value as `and` rather than erroring.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("or") => Self::Or,
            _ => Self::And,
        }
    }
}

/// One decoded entry from the `filters` query parameter.
#[derive(Debug, Clone)]
pub struct Filter {
    pub id: String,
    pub operator: String,
    /// Single-valued operators read `[0]`; `inArray`/`isBetween` read all.
    pub values: Vec<String>,
}

/// One decoded entry from the `sort` query parameter.
#[derive(Debug, Clone)]
pub struct SortSpec {
    pub id: String,
    pub desc: bool,
}

/// A group header plus the number of rows under it.
#[derive(Debug, Clone)]
pub struct GroupSummary {
    pub id: String,
    pub count: usize,
}

fn bad_request(message: String) -> ApiRejection {
    ApiError::BadRequest(message).into()
}

/// Read a field off an asset as a string, whatever its JSON type.
///
/// Numbers are stringified rather than skipped: the same comparison path
/// serves `iLike` on `name` and `eq` on `rows`, and the client sends every
/// filter value as a string regardless of the column's type.
fn field_str(asset: &Value, field: &str) -> Option<String> {
    match asset.get(field)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Numeric view of a field, for the ordering operators.
fn field_num(asset: &Value, field: &str) -> Option<f64> {
    match asset.get(field)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Decode the `filters` parameter: a JSON array of
/// `{ id, value, operator, ... }` as serialised by `getFiltersStateParser`
/// on the frontend. `value` arrives as either a string or an array of
/// strings, so both shapes are accepted.
///
/// An unknown `id` is a 400 rather than a silent skip: quietly dropping a
/// filter returns MORE rows than asked for, which for a governed catalog
/// is the wrong way to fail.
pub fn parse_filters(raw: Option<&str>) -> Result<Vec<Filter>, ApiRejection> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|err| bad_request(format!("invalid `filters` JSON: {err}")))?;
    let Some(items) = parsed.as_array() else {
        return Err(bad_request("`filters` must be a JSON array".to_owned()));
    };

    items
        .iter()
        .map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| bad_request("each filter needs a string `id`".to_owned()))?;
            if !FILTERABLE_FIELDS.contains(&id) {
                return Err(bad_request(format!("unknown filter field `{id}`")));
            }
            let operator = item
                .get("operator")
                .and_then(Value::as_str)
                .unwrap_or("iLike")
                .to_owned();

            let values = match item.get("value") {
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                    .collect(),
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Number(n)) => vec![n.to_string()],
                _ => Vec::new(),
            };

            Ok(Filter {
                id: id.to_owned(),
                operator,
                values,
            })
        })
        .collect()
}

/// Decode the `sort` parameter: a JSON array of `{ id, desc }` as
/// serialised by `getSortingStateParser`.
pub fn parse_sort(raw: Option<&str>) -> Result<Vec<SortSpec>, ApiRejection> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|err| bad_request(format!("invalid `sort` JSON: {err}")))?;
    let Some(items) = parsed.as_array() else {
        return Err(bad_request("`sort` must be a JSON array".to_owned()));
    };

    items
        .iter()
        .map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| bad_request("each sort entry needs a string `id`".to_owned()))?;
            if !SORTABLE_FIELDS.contains(&id) {
                return Err(bad_request(format!("unknown sort field `{id}`")));
            }
            Ok(SortSpec {
                id: id.to_owned(),
                desc: item.get("desc").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// Validate `groupBy` against [`GROUPABLE_FIELDS`].
pub fn parse_group_by(raw: Option<&str>) -> Result<Option<String>, ApiRejection> {
    let Some(field) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if !GROUPABLE_FIELDS.contains(&field) {
        return Err(bad_request(format!("unknown groupBy field `{field}`")));
    }
    Ok(Some(field.to_owned()))
}

/// Does one asset satisfy one filter?
///
/// The operator set mirrors `dataTableConfig.operators` on the frontend —
/// the UI can only build these fourteen, so anything else is a client bug.
/// An unrecognised operator matches nothing rather than everything: a
/// filter that fails open would widen the result set, which is the more
/// dangerous direction for a governed catalog.
fn matches_filter(asset: &Value, filter: &Filter) -> bool {
    let field = filter.id.as_str();
    if ARRAY_FIELDS.contains(&field) {
        return matches_array_filter(asset, filter);
    }
    let actual = field_str(asset, field);
    let first = filter.values.first().map_or("", String::as_str);

    match filter.operator.as_str() {
        "isEmpty" => actual.as_deref().is_none_or(str::is_empty),
        "isNotEmpty" => actual.as_deref().is_some_and(|v| !v.is_empty()),
        "eq" => actual
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case(first)),
        "ne" => actual
            .as_deref()
            .is_none_or(|v| !v.eq_ignore_ascii_case(first)),
        "iLike" => actual
            .as_deref()
            .is_some_and(|v| v.to_lowercase().contains(&first.to_lowercase())),
        "notILike" => actual
            .as_deref()
            .is_none_or(|v| !v.to_lowercase().contains(&first.to_lowercase())),
        "inArray" => actual
            .as_deref()
            .is_some_and(|v| filter.values.iter().any(|c| c.eq_ignore_ascii_case(v))),
        "notInArray" => actual
            .as_deref()
            .is_none_or(|v| !filter.values.iter().any(|c| c.eq_ignore_ascii_case(v))),
        // Ordering operators compare numerically when both sides parse as
        // numbers, and fall back to lexicographic comparison otherwise —
        // so they work on `sizeBytes` and on ISO `lastUpdated` alike.
        "lt" | "lte" | "gt" | "gte" => compare_op(asset, field, &filter.operator, first),
        "isBetween" => {
            let (Some(low), Some(high)) = (filter.values.first(), filter.values.get(1)) else {
                return false;
            };
            compare_op(asset, field, "gte", low) && compare_op(asset, field, "lte", high)
        }
        // Date-relative filtering needs a clock, which would make this
        // module impure and untestable. The Data Explorer exposes no date
        // filter today; wiring one means passing "now" in as a parameter
        // rather than reading it here.
        _ => false,
    }
}

/// [`matches_filter`] for a field that is a list of strings (`tags`,
/// `DATA-11`). `eq`, `iLike` and `inArray` match when any entry matches;
/// `ne`, `notILike` and `notInArray` match when no entry does; `isEmpty`
/// and `isNotEmpty` look at the list's length. The ordering operators match
/// nothing: a list has no order. An asset without the field is an empty
/// list, so a negative filter keeps it, as it does for a missing scalar.
fn matches_array_filter(asset: &Value, filter: &Filter) -> bool {
    let entries: Vec<&str> = asset
        .get(filter.id.as_str())
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let first = filter.values.first().map_or("", String::as_str);
    let any = |positive: &str| {
        entries.iter().any(|entry| match positive {
            "eq" => entry.eq_ignore_ascii_case(first),
            "iLike" => entry.to_lowercase().contains(&first.to_lowercase()),
            _ => filter.values.iter().any(|c| c.eq_ignore_ascii_case(entry)),
        })
    };
    match filter.operator.as_str() {
        "isEmpty" => entries.is_empty(),
        "isNotEmpty" => !entries.is_empty(),
        "eq" => any("eq"),
        "iLike" => any("iLike"),
        "inArray" => any("inArray"),
        "ne" => !any("eq"),
        "notILike" => !any("iLike"),
        "notInArray" => !any("inArray"),
        // Ordering operators, `isBetween`, and anything unknown: fail
        // closed, like the scalar path.
        _ => false,
    }
}

/// Shared body of `lt`/`lte`/`gt`/`gte`.
fn compare_op(asset: &Value, field: &str, operator: &str, operand: &str) -> bool {
    let ordering = match (field_num(asset, field), operand.parse::<f64>()) {
        (Some(actual), Ok(expected)) => actual.partial_cmp(&expected),
        _ => field_str(asset, field).map(|actual| actual.cmp(&operand.to_owned())),
    };
    let Some(ordering) = ordering else {
        return false;
    };
    match operator {
        "lt" => ordering.is_lt(),
        "lte" => ordering.is_le(),
        "gt" => ordering.is_gt(),
        "gte" => ordering.is_ge(),
        _ => false,
    }
}

/// Combine `filters` with `join`. No filters means no filtering, for
/// either operator — an empty `or` must not reject every row.
pub fn apply_filters(assets: &[Value], filters: &[Filter], join: JoinOperator) -> Vec<Value> {
    if filters.is_empty() {
        return assets.to_vec();
    }
    assets
        .iter()
        .filter(|asset| match join {
            JoinOperator::And => filters.iter().all(|f| matches_filter(asset, f)),
            JoinOperator::Or => filters.iter().any(|f| matches_filter(asset, f)),
        })
        .cloned()
        .collect()
}

/// Multi-column sort, applied left to right, with `id` as a final
/// tie-breaker.
///
/// The tie-break is what makes paging correct, not just tidy: with
/// infinite scroll, two rows that compare equal could otherwise be
/// returned in a different relative order on two different requests,
/// letting a row appear on both page 1 and page 2 (or on neither).
pub fn apply_sort(assets: &mut [Value], sort: &[SortSpec]) {
    assets.sort_by(|a, b| {
        for spec in sort {
            let ordering = match (field_num(a, &spec.id), field_num(b, &spec.id)) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                _ => field_str(a, &spec.id)
                    .unwrap_or_default()
                    .to_lowercase()
                    .cmp(&field_str(b, &spec.id).unwrap_or_default().to_lowercase()),
            };
            let ordering = if spec.desc {
                ordering.reverse()
            } else {
                ordering
            };
            if ordering.is_ne() {
                return ordering;
            }
        }
        field_str(a, "id")
            .unwrap_or_default()
            .cmp(&field_str(b, "id").unwrap_or_default())
    });
}

/// Reorder rows so that everything sharing a `group_by` value is
/// contiguous, and report each group's size.
///
/// Grouping runs after sorting and preserves the within-group order the
/// sort produced. Groups themselves are ordered by first appearance, so a
/// sort on the grouped column also determines the group order. Rows with a
/// missing value collect under `"—"` rather than being dropped.
///
/// The counts are computed over the whole filtered set, before paging, so
/// a group header can honestly say "12" while showing the first 3.
pub fn apply_grouping(assets: &[Value], group_by: &str) -> (Vec<Value>, Vec<GroupSummary>) {
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();

    for asset in assets {
        let key = field_str(asset, group_by)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "—".to_owned());
        if !buckets.contains_key(&key) {
            order.push(key.clone());
        }
        buckets.entry(key).or_default().push(asset.clone());
    }

    let mut grouped = Vec::with_capacity(assets.len());
    let mut summaries = Vec::with_capacity(order.len());
    for key in order {
        let rows = buckets.remove(&key).unwrap_or_default();
        summaries.push(GroupSummary {
            id: key,
            count: rows.len(),
        });
        grouped.extend(rows);
    }
    (grouped, summaries)
}

/// One page of `assets`, 1-indexed. An out-of-range page yields an empty
/// slice rather than an error — the client treats a short page as "no more
/// rows", so this terminates infinite scroll cleanly.
pub fn paginate(assets: &[Value], page: u32, page_size: u32) -> Vec<Value> {
    let page = page.max(1) as usize;
    let page_size = page_size.max(1) as usize;
    let start = (page - 1).saturating_mul(page_size);
    assets.iter().skip(start).take(page_size).cloned().collect()
}

/// Number of pages `total` items span at `page_size`.
pub fn total_pages(total: usize, page_size: u32) -> usize {
    let page_size = page_size.max(1) as usize;
    total.div_ceil(page_size)
}

/// Assemble the JSON body for `GET /api/catalog/query`.
///
/// Shape matches the `Pagination<T>` contract the table stack consumes
/// (`services/contracts/pagination.ts`).
#[allow(clippy::too_many_arguments)]
pub fn build_response(
    page_items: Vec<Value>,
    total_items: usize,
    page: u32,
    page_size: u32,
    group_by: Option<&str>,
    summaries: &[GroupSummary],
    item_group_keys: Option<Vec<String>>,
) -> Value {
    let mut body = Map::new();
    body.insert("items".to_owned(), Value::Array(page_items));
    body.insert("totalItems".to_owned(), json!(total_items));
    body.insert(
        "totalPages".to_owned(),
        json!(total_pages(total_items, page_size)),
    );
    body.insert("page".to_owned(), json!(page));
    body.insert("pageSize".to_owned(), json!(page_size));

    if let Some(field) = group_by {
        body.insert("groupBy".to_owned(), json!(field));
        body.insert(
            "groups".to_owned(),
            Value::Array(
                summaries
                    .iter()
                    .map(|group| json!({ "id": group.id, "label": group.id, "count": group.count }))
                    .collect(),
            ),
        );
        if let Some(keys) = item_group_keys {
            body.insert("itemGroupKeys".to_owned(), json!(keys));
        }
    }

    Value::Object(body)
}

/// The `group_by` value for each row of a page, parallel to `items`.
pub fn item_group_keys(page_items: &[Value], group_by: &str) -> Vec<String> {
    page_items
        .iter()
        .map(|asset| {
            field_str(asset, group_by)
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "—".to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Four assets with deliberately awkward data: mixed case, a missing
    /// `owner`, an empty-string `owner`, and sizes that sort differently
    /// as numbers than as strings.
    fn fixture() -> Vec<Value> {
        vec![
            json!({
                "id": "silver.mart_wisman", "name": "Mart Wisman",
                "namespace": "silver", "layer": "Silver", "tier": "gold",
                "owner": "Dinas Pariwisata", "sizeBytes": 900,
                "description": "Kunjungan wisatawan mancanegara",
            }),
            json!({
                "id": "bronze.event_2026", "name": "Event 2026",
                "namespace": "sdi-primer", "layer": "Bronze", "tier": "silver",
                "owner": "", "sizeBytes": 1024,
                "description": "Jumlah pengunjung event",
            }),
            json!({
                "id": "silver.dim_negara", "name": "dim negara",
                "namespace": "silver", "layer": "Silver", "tier": "bronze",
                "sizeBytes": 80,
                "description": "Dimensi negara",
            }),
            json!({
                "id": "gold.restoran", "name": "Restoran",
                "namespace": "serving", "layer": "Gold", "tier": "gold",
                "owner": "Dinas Ekraf", "sizeBytes": 20000,
                "description": "Agregat restoran",
            }),
        ]
    }

    fn ids(assets: &[Value]) -> Vec<&str> {
        assets.iter().filter_map(|a| a["id"].as_str()).collect()
    }

    fn filter(id: &str, operator: &str, values: &[&str]) -> Filter {
        Filter {
            id: id.to_owned(),
            operator: operator.to_owned(),
            values: values.iter().map(|v| (*v).to_owned()).collect(),
        }
    }

    // --- filter operators -------------------------------------------

    #[test]
    fn eq_ignores_case_and_ne_includes_missing_fields() {
        let hits = apply_filters(
            &fixture(),
            &[filter("layer", "eq", &["silver"])],
            JoinOperator::And,
        );
        assert_eq!(ids(&hits), vec!["silver.mart_wisman", "silver.dim_negara"]);

        // `dim_negara` has no `owner` key at all; `ne` must still return it,
        // otherwise a negative filter quietly hides incomplete rows.
        let hits = apply_filters(
            &fixture(),
            &[filter("owner", "ne", &["Dinas Ekraf"])],
            JoinOperator::And,
        );
        assert!(ids(&hits).contains(&"silver.dim_negara"));
        assert!(!ids(&hits).contains(&"gold.restoran"));
    }

    // --- array field: tags (DATA-11 R5) ------------------------------

    fn tagged() -> Vec<Value> {
        vec![
            json!({ "id": "a", "tags": ["Finance", "pii"] }),
            json!({ "id": "b", "tags": ["finance-archive"] }),
            json!({ "id": "c", "tags": [] }),
            // No `tags` key at all: an asset nobody annotated.
            json!({ "id": "d" }),
        ]
    }

    fn tag_hits(operator: &str, values: &[&str]) -> Vec<String> {
        apply_filters(
            &tagged(),
            &[filter("tags", operator, values)],
            JoinOperator::And,
        )
        .iter()
        .filter_map(|a| a["id"].as_str().map(str::to_owned))
        .collect()
    }

    #[test]
    fn tags_eq_matches_when_any_entry_equals_ignoring_case() {
        assert_eq!(tag_hits("eq", &["finance"]), vec!["a"]);
    }

    #[test]
    fn tags_i_like_matches_when_any_entry_contains_the_text() {
        assert_eq!(tag_hits("iLike", &["FINANCE"]), vec!["a", "b"]);
    }

    #[test]
    fn tags_in_array_matches_when_any_entry_is_listed() {
        assert_eq!(tag_hits("inArray", &["pii", "nothing"]), vec!["a"]);
    }

    #[test]
    fn tags_negative_operators_match_when_no_entry_does_and_keep_untagged_assets() {
        assert_eq!(tag_hits("ne", &["pii"]), vec!["b", "c", "d"]);
        assert_eq!(tag_hits("notILike", &["finance"]), vec!["c", "d"]);
        assert_eq!(
            tag_hits("notInArray", &["pii", "finance-archive"]),
            vec!["c", "d"]
        );
    }

    #[test]
    fn tags_is_empty_and_is_not_empty_look_at_the_length_and_a_missing_key_is_empty() {
        assert_eq!(tag_hits("isEmpty", &[]), vec!["c", "d"]);
        assert_eq!(tag_hits("isNotEmpty", &[]), vec!["a", "b"]);
    }

    #[test]
    fn tags_ordering_operators_match_nothing() {
        for operator in ["lt", "lte", "gt", "gte"] {
            assert!(tag_hits(operator, &["m"]).is_empty(), "{operator}");
        }
        assert!(tag_hits("isBetween", &["a", "z"]).is_empty());
    }

    #[test]
    fn tags_can_be_filtered_but_not_sorted_or_grouped_by() {
        assert!(parse_filters(Some(r#"[{"id":"tags","value":"pii","operator":"iLike"}]"#)).is_ok());
        for err in [
            parse_sort(Some(r#"[{"id":"tags","desc":false}]"#)).unwrap_err(),
            parse_group_by(Some("tags")).unwrap_err(),
        ] {
            assert!(matches!(err.0, ApiError::BadRequest(_)));
        }
    }

    /// `DATA-12` F5: `certification` filters (`eq`, `inArray`), sorts and
    /// groups; a row without a mark falls under the empty group.
    #[test]
    fn certification_filters_sorts_and_groups() {
        let rows = vec![
            json!({ "id": "a", "certification": "deprecated" }),
            json!({ "id": "b" }),
            json!({ "id": "c", "certification": "certified" }),
            json!({ "id": "d", "certification": "certified" }),
        ];
        let certified = apply_filters(
            &rows,
            &[filter("certification", "eq", &["certified"])],
            JoinOperator::And,
        );
        assert_eq!(ids(&certified), vec!["c", "d"]);
        let marked = apply_filters(
            &rows,
            &[filter(
                "certification",
                "inArray",
                &["certified", "deprecated"],
            )],
            JoinOperator::And,
        );
        assert_eq!(ids(&marked), vec!["a", "c", "d"]);

        assert!(parse_sort(Some(r#"[{"id":"certification","desc":false}]"#)).is_ok());
        let mut sorted = rows.clone();
        apply_sort(
            &mut sorted,
            &parse_sort(Some(r#"[{"id":"certification","desc":false}]"#)).unwrap(),
        );
        assert_eq!(ids(&sorted), vec!["b", "c", "d", "a"]);

        assert!(parse_group_by(Some("certification")).is_ok());
        let (_, groups) = apply_grouping(&rows, "certification");
        let counts: Vec<(&str, usize)> = groups.iter().map(|g| (g.id.as_str(), g.count)).collect();
        assert_eq!(counts, vec![("deprecated", 1), ("—", 1), ("certified", 2)]);
    }

    #[test]
    fn the_sortable_fields_are_the_filterable_fields_without_the_list_ones() {
        let expected: Vec<&str> = FILTERABLE_FIELDS
            .iter()
            .copied()
            .filter(|f| !ARRAY_FIELDS.contains(f))
            .collect();
        assert_eq!(SORTABLE_FIELDS, expected.as_slice());
    }

    #[test]
    fn is_empty_covers_both_missing_and_blank() {
        let hits = apply_filters(
            &fixture(),
            &[filter("owner", "isEmpty", &[])],
            JoinOperator::And,
        );
        assert_eq!(ids(&hits), vec!["bronze.event_2026", "silver.dim_negara"]);
    }

    #[test]
    fn in_array_matches_any_listed_value() {
        let hits = apply_filters(
            &fixture(),
            &[filter("tier", "inArray", &["gold", "bronze"])],
            JoinOperator::And,
        );
        assert_eq!(hits.len(), 3);
        assert!(!ids(&hits).contains(&"bronze.event_2026"));
    }

    #[test]
    fn numeric_operators_compare_as_numbers_not_strings() {
        // Lexicographically "900" > "20000"; numerically it is not. This is
        // the case that catches a string-comparison regression.
        let hits = apply_filters(
            &fixture(),
            &[filter("sizeBytes", "gt", &["1000"])],
            JoinOperator::And,
        );
        assert_eq!(ids(&hits), vec!["bronze.event_2026", "gold.restoran"]);
    }

    #[test]
    fn is_between_is_inclusive_at_both_ends() {
        let hits = apply_filters(
            &fixture(),
            &[filter("sizeBytes", "isBetween", &["900", "1024"])],
            JoinOperator::And,
        );
        assert_eq!(ids(&hits), vec!["silver.mart_wisman", "bronze.event_2026"]);
    }

    #[test]
    fn unknown_operator_matches_nothing_rather_than_everything() {
        // Failing open would widen the result set — the wrong direction for
        // a governed catalog.
        let hits = apply_filters(
            &fixture(),
            &[filter("layer", "bogusOperator", &["Silver"])],
            JoinOperator::And,
        );
        assert!(hits.is_empty());
    }

    // --- join operator ----------------------------------------------

    #[test]
    fn and_narrows_while_or_widens() {
        let filters = [
            filter("layer", "eq", &["Silver"]),
            filter("tier", "eq", &["gold"]),
        ];
        assert_eq!(
            ids(&apply_filters(&fixture(), &filters, JoinOperator::And)),
            vec!["silver.mart_wisman"]
        );
        assert_eq!(
            apply_filters(&fixture(), &filters, JoinOperator::Or).len(),
            3
        );
    }

    #[test]
    fn empty_filter_list_is_a_no_op_for_both_join_operators() {
        // An empty `or` must not reject every row.
        assert_eq!(apply_filters(&fixture(), &[], JoinOperator::Or).len(), 4);
        assert_eq!(apply_filters(&fixture(), &[], JoinOperator::And).len(), 4);
    }

    #[test]
    fn join_operator_defaults_to_and() {
        assert_eq!(JoinOperator::parse(None), JoinOperator::And);
        assert_eq!(JoinOperator::parse(Some("nonsense")), JoinOperator::And);
        assert_eq!(JoinOperator::parse(Some("or")), JoinOperator::Or);
    }

    // --- sorting ----------------------------------------------------

    #[test]
    fn sort_is_numeric_where_the_field_is_numeric() {
        let mut assets = fixture();
        apply_sort(
            &mut assets,
            &[SortSpec {
                id: "sizeBytes".to_owned(),
                desc: false,
            }],
        );
        assert_eq!(
            ids(&assets),
            vec![
                "silver.dim_negara",
                "silver.mart_wisman",
                "bronze.event_2026",
                "gold.restoran"
            ]
        );
    }

    #[test]
    fn text_sort_ignores_case() {
        // "dim negara" must sort with the D's, not after every capital.
        let mut assets = fixture();
        apply_sort(
            &mut assets,
            &[SortSpec {
                id: "name".to_owned(),
                desc: false,
            }],
        );
        assert_eq!(
            ids(&assets),
            vec![
                "silver.dim_negara",
                "bronze.event_2026",
                "silver.mart_wisman",
                "gold.restoran"
            ]
        );
    }

    #[test]
    fn ties_break_on_id_so_paging_stays_stable() {
        // Both rows share `layer`. Without the tie-break their relative
        // order would depend on input order, letting a row show up on two
        // different pages (or neither) during infinite scroll.
        let assets = vec![
            json!({ "id": "b", "layer": "Silver" }),
            json!({ "id": "a", "layer": "Silver" }),
        ];
        let spec = [SortSpec {
            id: "layer".to_owned(),
            desc: false,
        }];

        let mut forward = assets.clone();
        apply_sort(&mut forward, &spec);
        let mut reversed = assets;
        reversed.reverse();
        apply_sort(&mut reversed, &spec);

        assert_eq!(ids(&forward), vec!["a", "b"]);
        assert_eq!(ids(&forward), ids(&reversed));
    }

    #[test]
    fn secondary_sort_applies_only_within_ties() {
        let mut assets = fixture();
        apply_sort(
            &mut assets,
            &[
                SortSpec {
                    id: "tier".to_owned(),
                    desc: false,
                },
                SortSpec {
                    id: "sizeBytes".to_owned(),
                    desc: true,
                },
            ],
        );
        assert_eq!(
            ids(&assets),
            vec![
                "silver.dim_negara",
                "gold.restoran",
                "silver.mart_wisman",
                "bronze.event_2026"
            ]
        );
    }

    // --- grouping ---------------------------------------------------

    #[test]
    fn grouping_makes_each_group_contiguous_and_counts_the_full_set() {
        let (grouped, summaries) = apply_grouping(&fixture(), "namespace");
        assert_eq!(
            ids(&grouped),
            vec![
                "silver.mart_wisman",
                "silver.dim_negara",
                "bronze.event_2026",
                "gold.restoran"
            ]
        );
        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].id, "silver");
        assert_eq!(summaries[0].count, 2);
    }

    #[test]
    fn grouping_collects_missing_values_instead_of_dropping_rows() {
        let (grouped, summaries) = apply_grouping(&fixture(), "owner");
        // Nothing may be lost: an unowned row is still a real asset.
        assert_eq!(grouped.len(), 4);
        let fallback = summaries.iter().find(|g| g.id == "—").unwrap();
        assert_eq!(fallback.count, 2);
    }

    // --- pagination -------------------------------------------------

    #[test]
    fn paginate_slices_by_one_indexed_page() {
        let assets = fixture();
        assert_eq!(
            ids(&paginate(&assets, 1, 2)),
            vec!["silver.mart_wisman", "bronze.event_2026"]
        );
        assert_eq!(
            ids(&paginate(&assets, 2, 2)),
            vec!["silver.dim_negara", "gold.restoran"]
        );
    }

    #[test]
    fn paginate_past_the_end_is_empty_not_an_error() {
        // How infinite scroll terminates: the client stops once a page
        // comes back shorter than `pageSize`.
        assert!(paginate(&fixture(), 99, 50).is_empty());
    }

    #[test]
    fn last_page_is_short_rather_than_padded() {
        assert_eq!(paginate(&fixture(), 2, 3).len(), 1);
    }

    #[test]
    fn total_pages_rounds_up_and_handles_empty() {
        assert_eq!(total_pages(4, 2), 2);
        assert_eq!(total_pages(5, 2), 3);
        assert_eq!(total_pages(0, 50), 0);
    }

    // --- parsing and validation -------------------------------------

    #[test]
    fn parse_filters_accepts_scalar_and_array_values() {
        let parsed = parse_filters(Some(
            r#"[{"id":"layer","value":"Silver","operator":"eq"},
                {"id":"tier","value":["gold","bronze"],"operator":"inArray"}]"#,
        ))
        .unwrap();
        assert_eq!(parsed[0].values, vec!["Silver"]);
        assert_eq!(parsed[1].values, vec!["gold", "bronze"]);
    }

    #[test]
    fn parse_filters_rejects_fields_outside_the_allowlist() {
        // The point of the allowlist: a caller cannot probe arbitrary keys.
        assert!(parse_filters(Some(r#"[{"id":"password","value":"x"}]"#)).is_err());
    }

    #[test]
    fn parse_sort_rejects_fields_outside_the_allowlist() {
        assert!(parse_sort(Some(r#"[{"id":"__proto__","desc":true}]"#)).is_err());
    }

    #[test]
    fn parse_group_by_rejects_high_cardinality_fields() {
        // Grouping by `name` would emit one group per row.
        assert!(parse_group_by(Some("name")).is_err());
        assert_eq!(
            parse_group_by(Some("layer")).unwrap().as_deref(),
            Some("layer")
        );
        assert!(parse_group_by(Some("  ")).unwrap().is_none());
    }

    #[test]
    fn parse_helpers_treat_absent_and_blank_input_as_no_op() {
        assert!(parse_filters(None).unwrap().is_empty());
        assert!(parse_sort(Some("")).unwrap().is_empty());
        assert!(parse_group_by(None).unwrap().is_none());
    }

    #[test]
    fn malformed_json_is_rejected_rather_than_ignored() {
        assert!(parse_filters(Some("not json")).is_err());
        assert!(parse_sort(Some(r#"{"id":"name"}"#)).is_err());
    }

    // --- response shape ---------------------------------------------

    #[test]
    fn response_matches_the_pagination_contract() {
        let body = build_response(fixture(), 4, 1, 2, None, &[], None);
        assert_eq!(body["totalItems"], 4);
        assert_eq!(body["totalPages"], 2);
        assert_eq!(body["page"], 1);
        assert_eq!(body["pageSize"], 2);
        // Absent rather than null when not grouping, so the client can test
        // presence instead of null-checking.
        assert!(body.get("groupBy").is_none());
        assert!(body.get("groups").is_none());
    }

    #[test]
    fn grouped_response_carries_summaries_and_parallel_keys() {
        let (grouped, summaries) = apply_grouping(&fixture(), "namespace");
        let page = paginate(&grouped, 1, 2);
        let keys = item_group_keys(&page, "namespace");
        let body = build_response(page, 4, 1, 2, Some("namespace"), &summaries, Some(keys));

        assert_eq!(body["groupBy"], "namespace");
        assert_eq!(body["groups"][0]["id"], "silver");
        assert_eq!(body["groups"][0]["count"], 2);
        assert_eq!(body["itemGroupKeys"], json!(["silver", "silver"]));
        // The client zips these two arrays, so their lengths must agree.
        assert_eq!(
            body["items"].as_array().unwrap().len(),
            body["itemGroupKeys"].as_array().unwrap().len()
        );
    }

    #[test]
    fn full_pipeline_composes_in_the_documented_order() {
        // filter → sort → paginate, the sequence the handler runs. Search
        // used to be the first step; it is `catalog_search::search` now and
        // runs before this pipeline (DATA-11 F1), so this test starts at the
        // filter and its expected answer is unchanged.
        let assets = fixture();
        let mut filtered = apply_filters(
            &assets,
            &[filter("tier", "inArray", &["gold", "silver"])],
            JoinOperator::And,
        );
        apply_sort(
            &mut filtered,
            &[SortSpec {
                id: "sizeBytes".to_owned(),
                desc: true,
            }],
        );
        assert_eq!(
            ids(&paginate(&filtered, 1, 2)),
            vec!["gold.restoran", "bronze.event_2026"]
        );
    }
}
