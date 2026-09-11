import type { ChartKind, ChartSpec, ChartSource } from "@/lib/dashboard-specs";

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
  aggregate?: "sum" | "avg" | "max" | "min" | "count";
  limit?: number;
  order?: "desc" | "asc" | "none";
  span?: 1 | 2;
  board?: string; // target board (default "default")
  text?: string; // markdown content (kind="text")
  caption?: string; // unit/caption (kind="kpi")
  target?: number; // target/max (kind="gauge")
};

/** Tile position on the grid canvas (12 columns). Key = chartId. */
export type TileBox = { x: number; y: number; w: number; h: number };
export type LayoutMap = Record<string, TileBox>;
/** Dashboard filter: a column value that filters every tile that has that column. */
export type FilterDef = { column: string; values: string[] };

export type Board = {
  id: string;
  name: string;
  layout?: LayoutMap;
  filters?: FilterDef[];
  createdAt?: string;
  publicToken?: string;
  embedEnabled?: boolean;
};
