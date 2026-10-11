//! Number bins in a chart's relation (`BI-10`): the layers that replace a
//! number grouping by its bin, and the statements that tell the tile the
//! width.
//!
//! See [`crate::bin`] for the arithmetic and why the range is read inside the
//! same statement. This module is the part that knows about the chart: which
//! of its groupings are binned, what the relation must carry so the outer
//! query still reads everything it needs, and where the predicates sit (under
//! the bins, on the raw rows, so a filter on the binned column reads the raw
//! number and never the bin that carries its name; the same reason as BI-9's
//! `grained_sql`).

use lakehouse_core::ident::Ident;
use serde::Serialize;

use super::{Measure, Relation};
use crate::bin::{BinMode, Range};
use crate::store::ChartInput;

/// One binned grouping.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The raw number column, which keeps its name in the result.
    pub column: Ident,
    /// How it is binned.
    pub mode: BinMode,
}

/// What the tile needs to label one binned column: the statement that reads
/// its width, minimum and maximum over the same filtered relation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BinRead {
    /// The binned column.
    pub column: String,
    /// `auto`, `count` or `width`.
    pub mode: String,
    /// `SELECT width, min, max`, one row, to run through the role rewrite.
    pub sql: String,
}

/// The binned groupings of a chart that is not a pivot, a map or a grouped
/// table: its `dimension` and its `breakdown`. `None` when a stored bin no
/// longer checks out (data corruption only; the caller reports the chart).
#[must_use]
pub fn chart_layers(def: &ChartInput) -> Option<Vec<Layer>> {
    let mut out = Vec::new();
    for (spec, column) in [
        (def.shape.bin.as_ref(), Some(def.dimension.as_str())),
        (def.shape.breakdown_bin.as_ref(), def.breakdown.as_deref()),
    ] {
        let Some(spec) = spec else {
            continue;
        };
        out.push(Layer {
            column: Ident::new(column.filter(|c| !c.is_empty())?).ok()?,
            mode: spec.mode().ok()?,
        });
    }
    Some(out)
}

/// Every binned grouping of `def`: a pivot's first number field, the
/// groupings of a grouped table, or a chart's dimension and breakdown.
#[must_use]
pub fn all_layers<H: std::hash::BuildHasher>(
    def: &ChartInput,
    kind: crate::specs::ChartKind,
    cols: &super::RelationColumns<H>,
) -> Option<Vec<Layer>> {
    if kind == crate::specs::ChartKind::Pivot {
        let Some(spec) = &def.shape.bin else {
            return Some(Vec::new());
        };
        let (column, _) = crate::tables::pivot_bin_field(&def.tables, cols)?;
        return Some(vec![Layer {
            column,
            mode: spec.mode().ok()?,
        }]);
    }
    if def.shape.groupings.is_some() {
        return grouping_layers(def);
    }
    chart_layers(def)
}

/// The binned groupings of a grouped table.
#[must_use]
pub fn grouping_layers(def: &ChartInput) -> Option<Vec<Layer>> {
    let mut out = Vec::new();
    for g in def.shape.groupings.iter().flatten() {
        if let Some(spec) = &g.bin {
            out.push(Layer {
                column: Ident::new(g.column.clone()).ok()?,
                mode: spec.mode().ok()?,
            });
        }
    }
    Some(out)
}

/// The columns the outer query reads from a relation grouped by `groups`:
/// the groups themselves and what the measures read. `None` when a measure
/// would read a grouped column that is now a bin (an aggregate or table
/// calculation over the binned column itself): no statement is built, the
/// caller reports the chart and never runs a different one.
#[must_use]
pub fn needed_columns(
    from: &Relation,
    groups: &[String],
    measures: &[Measure],
) -> Option<Vec<String>> {
    let mut needed: Vec<String> = groups.to_vec();
    for m in measures {
        let reads: Vec<String> = if let Some(field) = from.aggregate_field(m.column.as_str()) {
            field.reads.clone()
        } else if let Some(calc) = from.table_calc(m.column.as_str()) {
            calc.reads.clone()
        } else {
            vec![m.column.to_string()]
        };
        let is_field = from.aggregate_field(m.column.as_str()).is_some()
            || from.table_calc(m.column.as_str()).is_some();
        if is_field && reads.iter().any(|r| groups.contains(r)) {
            return None;
        }
        for r in reads {
            if !needed.contains(&r) {
                needed.push(r);
            }
        }
    }
    Some(needed)
}

/// `from` with each layer's column replaced by its bin. The predicates go
/// into an inner [`Relation::Filtered`] first, so they read the raw rows, and
/// each layer's range is read over the same filtered rows.
#[must_use]
pub fn binned(
    from: &Relation,
    predicates: Vec<String>,
    layers: &[Layer],
    needed: &[String],
) -> Relation {
    layers
        .iter()
        .fold(filtered(from, predicates), |relation, layer| {
            add_layer(relation, layer, needed)
        })
}

/// One more bin over a relation whose rows are already filtered.
#[must_use]
pub fn add_layer(relation: Relation, layer: &Layer, needed: &[String]) -> Relation {
    let range = Range::new(&layer.column, layer.mode);
    let expr = range.bin_expr();
    Relation::Bucketed {
        base: Box::new(Relation::Ranged {
            base: Box::new(relation),
            range,
        }),
        dimension: layer.column.to_string(),
        expr,
        carried: needed
            .iter()
            .filter(|n| n.as_str() != layer.column.as_str())
            .cloned()
            .collect(),
    }
}

fn filtered(from: &Relation, predicates: Vec<String>) -> Relation {
    if predicates.is_empty() {
        from.clone()
    } else {
        Relation::Filtered {
            base: Box::new(from.clone()),
            predicates,
        }
    }
}

/// The statement that reads each layer's width, minimum and maximum over the
/// filtered relation the bins are cut from.
#[must_use]
pub fn reads(from: &Relation, predicates: &[String], layers: &[Layer]) -> Vec<BinRead> {
    let base = filtered(from, predicates.to_vec()).render();
    layers
        .iter()
        .map(|l| BinRead {
            column: l.column.to_string(),
            mode: match l.mode {
                BinMode::Auto => "auto",
                BinMode::Count(_) => "count",
                BinMode::Width(_) => "width",
            }
            .to_owned(),
            sql: Range::new(&l.column, l.mode).read_sql(&base),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn id(name: &str) -> Ident {
        Ident::new(name).unwrap()
    }

    #[test]
    fn predicates_sit_under_the_bin_and_the_range_reads_the_same_filtered_rows() {
        let rel = binned(
            &Relation::Mart(id("t")),
            vec!["g = 'a'".to_owned()],
            &[Layer {
                column: id("v"),
                mode: BinMode::Width(500.0),
            }],
            &["v".to_owned(), "w".to_owned()],
        );
        let sql = rel.render();
        assert!(
            sql.starts_with("(SELECT round(least(floor(toFloat64(v) / __bw_v)"),
            "{sql}"
        );
        assert!(sql.contains(" AS v, w FROM (SELECT * FROM (SELECT * FROM serving.t WHERE g = 'a') AS flt CROSS JOIN (SELECT w AS __bw_v"), "{sql}");
        // The table is read twice, both times under the predicate.
        assert_eq!(
            sql.matches("FROM serving.t WHERE g = 'a'").count(),
            2,
            "{sql}"
        );
        assert_eq!(sql.matches("serving.t").count(), 2, "{sql}");
    }

    #[test]
    fn two_bins_nest_and_each_carries_the_other_column() {
        let layers = [
            Layer {
                column: id("a"),
                mode: BinMode::Auto,
            },
            Layer {
                column: id("b"),
                mode: BinMode::Count(10),
            },
        ];
        let rel = binned(
            &Relation::Mart(id("t")),
            Vec::new(),
            &layers,
            &["a".to_owned(), "b".to_owned(), "m".to_owned()],
        );
        let sql = rel.render();
        assert!(sql.contains(" AS b, a, m FROM"), "{sql}");
        assert!(sql.contains(" AS a, b, m FROM"), "{sql}");
        let reads = reads(&Relation::Mart(id("t")), &[], &layers);
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].mode, "auto");
        assert_eq!(reads[1].mode, "count");
    }

    #[test]
    fn an_aggregate_over_the_binned_column_is_not_built() {
        let from = Relation::Calculated {
            base: Box::new(Relation::Mart(id("t"))),
            columns: Vec::new(),
            aggregates: vec![super::super::AggregateField {
                name: "f".to_owned(),
                sql: "sum(v)".to_owned(),
                reads: vec!["v".to_owned()],
            }],
            table_calcs: Vec::new(),
        };
        let m = [Measure::plain(id("f"))];
        assert_eq!(needed_columns(&from, &["v".to_owned()], &m), None);
        assert_eq!(
            needed_columns(&from, &["g".to_owned()], &m),
            Some(vec!["g".to_owned(), "v".to_owned()])
        );
    }
}
