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

/** Input tingkat-tinggi (dari AI tool / UI builder) — server menyusun SQL-nya. */
export type ChartInput = {
  title: string;
  subtitle?: string;
  mart: string; // tanpa prefix "serving." — mis. "mart_wisman"
  kind: ChartKind;
  dimension: string; // kolom sumbu-X / kategori
  measures: string[]; // kolom nilai; >1 untuk "stacked"
  breakdown?: string; // dimensi ke-2 opsional → pecah jadi banyak seri
  aggregate?: "sum" | "avg" | "max" | "min" | "count";
  limit?: number;
  order?: "desc" | "asc" | "none";
  span?: 1 | 2;
  board?: string; // board tujuan (default "default")
  text?: string; // konten markdown (kind="text")
  caption?: string; // unit/caption (kind="kpi")
  target?: number; // target/max (kind="gauge")
};

/** Posisi tile di kanvas grid (12 kolom). Key = chartId. */
export type TileBox = { x: number; y: number; w: number; h: number };
export type LayoutMap = Record<string, TileBox>;
/** Filter dashboard: nilai kolom yang menyaring semua tile yang punya kolom itu. */
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
