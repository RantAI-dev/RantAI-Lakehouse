//! `BI-16` part B: the role rewrite on the progress, histogram and bullet
//! statements.
//!
//! A histogram reads its table more than once (the count per range, and the
//! scalar subquery that finds the column's range), and a progress bar or a
//! bullet graph with a goal that is a value reads it under an inner select.
//! These tests run the real [`enforce`] on the statements the builder
//! produces, under a row filter and under a mask on the measured column, and
//! assert through `bin_rewrite::rewrite` that *every* read of the table became
//! its governed projection: the table's name appears exactly as many times in
//! the rewritten statement as in the original and each read carries the row
//! filter (and the mask). The image and web cards have no statement at all
//! (`ChartKind::is_static`), so there is nothing for them to rewrite.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;

use super::bin_rewrite::{
    chart, dashboard_filter, governed, masks_the_binned_column, on_worker_stack, only_jakarta,
};

/// A chart of `kind` on the demo visits table with no bin and no default
/// measure, `extra` laid over it.
fn typed(kind: &str, extra: &serde_json::Value) -> lakehouse_bi::store::StoredChartSpec {
    let mut def = json!({
        "kind": kind, "bin": null, "dimension": "", "measures": [], "aggregate": null,
        "limit": null, "order": null,
    });
    def.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    chart(&def)
}

#[test]
fn every_histogram_mode_reads_the_table_only_through_its_governed_projection() {
    on_worker_stack(|| {
        for policy in [only_jakarta(), masks_the_binned_column()] {
            for histogram in [
                None,
                Some(json!({ "width": 500 })),
                Some(json!({ "count": 10 })),
                Some(json!({ "count": 100 })),
            ] {
                let mut extra = json!({ "dimension": "visitors" });
                if let Some(h) = &histogram {
                    extra["histogram"] = h.clone();
                }
                governed(
                    &format!("histogram {histogram:?}"),
                    &typed("histogram", &extra),
                    &[],
                    &policy,
                );
            }
        }
    });
}

#[test]
fn a_histograms_dashboard_and_own_filters_stay_under_its_bins_and_governed() {
    on_worker_stack(|| {
        let spec = typed(
            "histogram",
            &json!({
                "dimension": "visitors",
                "filters": [{ "column": "category", "values": ["Office"] }],
            }),
        );
        let dash = [
            dashboard_filter(r#"{"column":"visitors","op":"between","min":"10"}"#),
            dashboard_filter(r#"{"column":"category","values":["Office","Retail"]}"#),
        ];
        governed("histogram filters", &spec, &dash, &only_jakarta());
    });
}

#[test]
fn progress_and_single_bullet_statements_read_the_table_only_through_its_governed_projection() {
    on_worker_stack(|| {
        let dash = [dashboard_filter(
            r#"{"column":"category","values":["Office","Retail"]}"#,
        )];
        for policy in [only_jakarta(), masks_the_binned_column()] {
            for (label, spec) in [
                (
                    "progress, fixed goal",
                    typed(
                        "progress",
                        &json!({ "measures": ["visitors"], "aggregate": "sum", "goal": 1000 }),
                    ),
                ),
                (
                    "progress, goal as a value",
                    typed(
                        "progress",
                        &json!({
                            "measures": ["visitors", "lat"], "aggregate": "sum",
                            "goal": { "measure": "lat" },
                        }),
                    ),
                ),
                (
                    "bullet, no category",
                    typed(
                        "bullet",
                        &json!({
                            "measures": ["visitors", "lat"], "aggregate": "sum",
                            "goal": { "measure": "lat" }, "bands": [10, 20, 30],
                        }),
                    ),
                ),
            ] {
                // Without a filter the stored statement is served as saved, so the
                // rewrite is checked on the rebuilt one, under the dashboard's.
                governed(&format!("{label}, filtered"), &spec, &dash, &policy);
            }
        }
    });
}

#[test]
fn a_bullet_per_category_with_a_goal_value_stays_governed() {
    on_worker_stack(|| {
        let spec = typed(
            "bullet",
            &json!({
                "dimension": "provinsi", "measures": ["visitors", "lat"], "aggregate": "sum",
                "goal": { "measure": "lat" }, "limit": 20, "order": "desc",
            }),
        );
        let dash = [dashboard_filter(
            r#"{"column":"category","values":["Office"]}"#,
        )];
        for policy in [only_jakarta(), masks_the_binned_column()] {
            governed("bullet by province", &spec, &dash, &policy);
        }
    });
}
