import type { ChartKind, ChartSpec, ChartSource } from "@/lib/dashboard-specs";
import type { TableDefFields } from "@/lib/table-types";
import type { StyleFields } from "@/lib/chart-style";
import type { TypeDefFields } from "@/lib/type-b";

/**
 * Shared BI/dashboard types.
 *
 * The server-side implementation (writing chart/board specs into ClickHouse,
 * `console.bi_chart` / `console.bi_board`) now lives in the Rust backend
 * (`rust/crates/lakehouse-bi`). This module only re-exports the shapes that
 * the frontend (`src/features/dashboards/**`) needs for typing props and
 * API response payloads — it performs no I/O.
 */

export type StoredChartSpec = ChartSpec & {
  source: ChartSource;
  board: string;
  def: ChartInput;
  hasYear: boolean;
  createdBy?: string;
  createdAt?: string;
};

/** High-level input (from an AI tool / UI builder) — the server composes the SQL. */
export type ChartInput = {
  title: string;
  subtitle?: string;
  mart: string; // without the "serving." prefix — e.g. "mart_wisman"
  kind: ChartKind;
  dimension: string; // x-axis / category column
  measures: string[]; // value columns; >1 for "stacked"
  breakdown?: string; // optional 2nd dimension → splits into multiple series
  map?: string; // bundled map id (kind="geomap" | "pointmap" | "geoheat")
  lat?: string; // latitude column (kind="pointmap" | "geoheat")
  lon?: string; // longitude column (kind="pointmap" | "geoheat")
  aggregate?: "sum" | "avg" | "max" | "min" | "count";
  limit?: number;
  order?: "desc" | "asc" | "none";
  span?: 1 | 2;
  board?: string; // target board (default "default")
  text?: string; // markdown content (kind="text")
  caption?: string; // unit/caption (kind="kpi")
  target?: number; // target/max (kind="gauge")
} & TableDefFields // raw tables, pivots and KPI comparisons (BI-16 part A)
  & StyleFields // number formats, colours, axes, notes, conditional colours (BI-17 part A)
  & TypeDefFields; // goal, bands, histogram ranges, address of an image or web card (BI-16 part B)

/** How a dashboard filter compares its column; absent means `in`. Mirrors `FilterOp` in `lakehouse-bi`. */
export type FilterOp = "in" | "not_in" | "between" | "relative" | "contains" | "starts_with" | "ends_with" | "not_contains";
export type RelativeUnit = "day" | "week" | "month" | "quarter" | "year";
/** `next`: the `n` units starting tomorrow, today excluded (BI-18 round two). */
export type RelativeAnchor = "last" | "this" | "previous" | "next";
/** What a column holds, as far as filtering cares; derived server-side from its ClickHouse type. */
export type FilterKind = "number" | "date" | "datetime" | "text";
/**
 * Dashboard filter, applied to every tile whose data has the column. The
 * typed fields are optional so a filter saved before typed filters
 * (`{ column, values }`) is still valid and means `in`.
 */
export type FilterDef = {
  column: string;
  values: string[];
  op?: FilterOp;
  /** `between` bounds: a number, or a date as `YYYY-MM-DD`; either may be absent. */
  min?: string;
  max?: string;
  /** `relative`: `n` units ending today (`last`) or starting tomorrow (`next`), or this / the previous calendar unit. */
  unit?: RelativeUnit;
  n?: number;
  anchor?: RelativeAnchor;
  /** `contains` / `starts_with` / `ends_with` / `not_contains`, at most 200 characters. */
  text?: string;
  /** `between` leaves out `min` / `max` itself ("after", "before", "greater than", "less than"). Absent means inclusive. */
  minExclusive?: boolean;
  maxExclusive?: boolean;
  /** The filter cannot be removed in the console. Meaningful only on a board's saved default. */
  required?: boolean;
  /** BI-2: a reference to a saved segment of the chart's table, `{ column: "", values: [], segment }`; it stands for the segment's filters. */
  segment?: string;
  /** BI-18 part C: the ids of the tabs this filter applies to. Absent or empty: every tab. */
  tabs?: string[];
};
/** A column a filter can target, with how many tiles of the board have it. */
export type FilterField = { column: string; kind: FilterKind | string; tiles: number };
/** A filter an active tile could not honour, so the tile can say so. */
export type FilterSkip = { column: string; reason: "no_column" | "wrong_type" | string };

/**
 * What a click on a chart does instead of opening the drill menu (BI-18·B).
 * Mirrors `lakehouse_bi::click::ClickAction`. Absent = the drill menu, which
 * is what every chart saved before this existed does. The server checks the
 * shape and the URL rule only, not that the board, column or query exists.
 */
export type ChartClick =
  | { kind: "dashboard"; board: string; column: string }
  | { kind: "query"; id: string }
  | { kind: "url"; url: string };

