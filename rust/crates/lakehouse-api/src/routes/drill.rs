//! Drill down (`BI-10`): the preview request for "the same chart, one level
//! down a hierarchy, for the value that was clicked".
//!
//! # Why the server builds the definition
//!
//! The console could send a whole definition with the dimension swapped, but
//! then a caller holding only `dashboard:read` could use the preview route to
//! read any table, or to drop the saved chart's own filters (feature plan,
//! section 4). So a drill request names the **saved chart** (`drillOf`), the
//! level to go to (`to`) and the values clicked so far (`fix`), and the server
//! builds the definition from the saved one:
//!
//! - the source, measures, aggregate, breakdown and the chart's own filters
//!   are the saved chart's, untouched;
//! - the only change to the grouping is `dimension = to`, and `to` and every
//!   fixed column must be consecutive levels of one hierarchy of the chart's
//!   source that contains the chart's own dimension;
//! - the only filters added are an equality on each fixed column and the
//!   dashboard's active filters, which can only narrow the rows.
//!
//! The statement then goes through the preview route's usual validation and
//! the same guard and role rewrite as any tile.
//!
//! # What cannot be drilled
//!
//! A chart with a time grain, a bin, steps, "Other", a table calculation,
//! groupings, or a pivot, raw table, KPI or map: the reason is returned in
//! words, and the console shows it in the click menu.

use lakehouse_bi::fields::SourceKind;
use lakehouse_bi::filters::{FilterDef, validate_filters};
use lakehouse_bi::hierarchies::HierarchyDef;
use lakehouse_bi::specs::ChartKind;
use lakehouse_bi::store::{self, ChartInput};
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::Value;

use crate::routes::calc_fields::classify_ch;

/// The body of a drill request.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DrillRequest {
    drill_of: String,
    to: String,
    #[serde(default)]
    fix: Vec<Fixed>,
    #[serde(default)]
    filters: Vec<FilterDef>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixed {
    column: String,
    value: String,
}

/// Why a chart cannot be drilled, or `Ok(())`.
///
/// # Errors
///
/// The reason, in words for the click menu.
pub fn drillable(def: &ChartInput) -> Result<(), String> {
    use ChartKind as K;
    if !matches!(
        def.kind,
        K::Bar
            | K::Hbar
            | K::Line
            | K::Area
            | K::Stacked
            | K::Pie
            | K::Rose
            | K::Funnel
            | K::Treemap
    ) {
        return Err("this kind of chart cannot be drilled down.".to_owned());
    }
    if def.grain.is_some() {
        return Err("a chart grouped by a date cannot be drilled down.".to_owned());
    }
    if def.shape.has_bins() {
        return Err("a chart grouped in bins cannot be drilled down.".to_owned());
    }
    if def.shape.steps.is_some() {
        return Err("a chart with steps cannot be drilled down.".to_owned());
    }
    if def.shape.wants_other() {
        return Err("a chart with an 'Other' group cannot be drilled down.".to_owned());
    }
    if def.shape.groupings.is_some() {
        return Err("a table with several groupings cannot be drilled down.".to_owned());
    }
    // BI-13: the other questions are matched on this dimension's values, which
    // a drill changes.
    if !def.shape.layers_list().is_empty() {
        return Err("a chart that combines several questions cannot be drilled down.".to_owned());
    }
    if def.dimension.is_empty() {
        return Err("this chart has no dimension to drill.".to_owned());
    }
    Ok(())
}

/// The drilled definition: `saved` one level down `hierarchies`, for the
/// values in `fix`, narrowed further by `filters`.
///
/// # Errors
///
/// The sentence for the caller: a chart that cannot be drilled, a level that
/// is not the next one of a hierarchy of the chart's source, or filters that
/// do not validate.
fn drilled(
    saved: &ChartInput,
    hierarchies: &[HierarchyDef],
    req: &DrillRequest,
) -> Result<ChartInput, String> {
    drillable(saved)?;
    if req.fix.is_empty() {
        return Err("a drill fixes at least the value that was clicked.".to_owned());
    }
    // One hierarchy must hold the chart's dimension, then each fixed column in
    // the next levels, then `to`.
    let fits = hierarchies.iter().any(|h| {
        h.levels
            .iter()
            .position(|l| *l == saved.dimension)
            .is_some_and(|start| {
                let after = &h.levels[start..];
                after.len() > req.fix.len()
                    && req
                        .fix
                        .iter()
                        .zip(after)
                        .all(|(f, level)| f.column == *level)
                    && after[req.fix.len()] == req.to
            })
    });
    if !fits {
        return Err("that is not the next level of a hierarchy of this chart's data.".to_owned());
    }
    validate_filters(&req.filters)?;
    let mut def = saved.clone();
    let from = std::mem::replace(&mut def.dimension, req.to.clone());
    if let Some(sort) = def.shape.sort.as_mut()
        && sort.by == from
    {
        sort.by.clone_from(&req.to);
    }
    let mut filters = def.shape.filters.take().unwrap_or_default();
    for f in &req.fix {
        filters.push(equals(&f.column, &f.value)?);
    }
    filters.extend(req.filters.iter().filter(|f| f.is_active()).cloned());
    def.shape.filters = Some(filters);
    Ok(def)
}

fn equals(column: &str, value: &str) -> Result<FilterDef, String> {
    serde_json::from_value(serde_json::json!({ "column": column, "values": [value] }))
        .map_err(|_bad| "that value cannot be used as a filter.".to_owned())
}

/// Build the definition a drill request asks for, from the saved chart.
///
/// # Errors
///
/// 400 for a body that is not a drill request or a level that does not follow,
/// 404 for an unknown chart, a fixed 422/503 for a database failure.
pub async fn drill_input(ch: &ChClient, raw: Value) -> Result<ChartInput, ApiError> {
    let req: DrillRequest = serde_json::from_value(raw)
        .map_err(|_bad| ApiError::BadRequest("body JSON is invalid".to_owned()))?;
    let saved = store::list_stored_charts(ch)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .into_iter()
        .find(|c| c.spec.id == req.drill_of)
        .ok_or_else(|| ApiError::NotFound("chart not found.".to_owned()))?;
    let (kind, id) = match saved.def.sql_source.as_deref().filter(|s| !s.is_empty()) {
        Some(source) => (SourceKind::SqlSource, source.to_owned()),
        None => (SourceKind::Mart, saved.def.mart.clone()),
    };
    let hierarchies = lakehouse_bi::hierarchies::list_for(ch, kind, &id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    drilled(&saved.def, &hierarchies, &req).map_err(ApiError::BadRequest)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn saved(extra: &serde_json::Value) -> ChartInput {
        let mut def = serde_json::json!({
            "title": "t", "kind": "bar", "mart": "m", "dimension": "province",
            "measures": ["visitors"], "aggregate": "sum",
            "filters": [{ "column": "category", "values": ["Office"] }],
        });
        def.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(def).unwrap()
    }

    fn geo() -> Vec<HierarchyDef> {
        vec![HierarchyDef {
            id: "h_1".to_owned(),
            source_kind: SourceKind::Mart,
            source_id: "m".to_owned(),
            name: "Geo".to_owned(),
            levels: ["province", "city", "place"].map(str::to_owned).to_vec(),
            created_by: String::new(),
            updated_at: None,
        }]
    }

    fn request(json: &serde_json::Value) -> DrillRequest {
        serde_json::from_value(json.clone()).unwrap()
    }

    #[test]
    fn a_drill_moves_the_dimension_one_level_and_keeps_the_saved_filters() {
        let req = request(&serde_json::json!({
            "drillOf": "c", "to": "city",
            "fix": [{ "column": "province", "value": "Jawa Barat" }],
            "filters": [{ "column": "visit_date", "op": "between", "min": "2026-01-01" }],
        }));
        let def = drilled(&saved(&serde_json::json!({})), &geo(), &req).unwrap();
        assert_eq!(def.dimension, "city");
        assert_eq!(def.mart, "m");
        let columns: Vec<&str> = def
            .shape
            .own_filters()
            .iter()
            .map(|f| f.column.as_str())
            .collect();
        // The saved chart's own filter is still first; the click and the
        // dashboard's filter come after it and only narrow.
        assert_eq!(columns, ["category", "province", "visit_date"]);
    }

    #[test]
    fn several_steps_stack_their_filters() {
        let req = request(&serde_json::json!({
            "drillOf": "c", "to": "place",
            "fix": [
                { "column": "province", "value": "Jawa Barat" },
                { "column": "city", "value": "Kota Bandung" },
            ],
        }));
        let def = drilled(&saved(&serde_json::json!({})), &geo(), &req).unwrap();
        assert_eq!(def.dimension, "place");
        assert_eq!(def.shape.own_filters().len(), 3);
    }

    #[test]
    fn a_level_that_does_not_follow_a_hierarchy_of_the_source_is_refused() {
        let go = |json: serde_json::Value| {
            drilled(&saved(&serde_json::json!({})), &geo(), &request(&json))
        };
        // Skipping a level, a column of another table, the last level, no click.
        assert!(go(serde_json::json!({"drillOf":"c","to":"place","fix":[{"column":"province","value":"x"}]})).is_err());
        assert!(go(serde_json::json!({"drillOf":"c","to":"secret","fix":[{"column":"province","value":"x"}]})).is_err());
        assert!(go(serde_json::json!({"drillOf":"c","to":"place","fix":[{"column":"province","value":"x"},{"column":"city","value":"y"},{"column":"place","value":"z"}]})).is_err());
        assert!(go(serde_json::json!({"drillOf":"c","to":"city","fix":[]})).is_err());
        // No hierarchy at all.
        let req = request(
            &serde_json::json!({"drillOf":"c","to":"city","fix":[{"column":"province","value":"x"}]}),
        );
        assert!(drilled(&saved(&serde_json::json!({})), &[], &req).is_err());
    }

    #[test]
    fn a_chart_that_cannot_be_drilled_says_why() {
        let req = request(
            &serde_json::json!({"drillOf":"c","to":"city","fix":[{"column":"province","value":"x"}]}),
        );
        for (extra, word) in [
            (serde_json::json!({"kind":"kpi"}), "kind"),
            (serde_json::json!({"bin":{"mode":"auto"}}), "bins"),
            (serde_json::json!({"other":true}), "Other"),
            (serde_json::json!({"kind":"pivot"}), "kind"),
        ] {
            let err = drilled(&saved(&extra), &geo(), &req).unwrap_err();
            assert!(err.contains(word), "{err}");
        }
    }

    #[test]
    fn the_request_cannot_carry_a_definition_of_its_own() {
        let bad = serde_json::json!({"drillOf":"c","to":"city","mart":"other_table","fix":[]});
        assert!(serde_json::from_value::<DrillRequest>(bad).is_err());
    }
}
