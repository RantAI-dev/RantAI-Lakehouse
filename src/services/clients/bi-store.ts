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

// `BI-29`: the filter and click shapes live in the contract
// (`contracts/dashboards`) and are exported from `@/services`; this re-export
// keeps the files outside `src/features` that still import them from here.
export type {
  ChartClick, FilterDef, FilterField, FilterKind, FilterOp, FilterSkip, RelativeAnchor, RelativeUnit,
} from "../contracts/dashboards";

