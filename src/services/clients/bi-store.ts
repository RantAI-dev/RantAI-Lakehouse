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
};

/** Tile position on the grid canvas (12 columns). Key = chartId. */
export type TileBox = { x: number; y: number; w: number; h: number };
export type LayoutMap = Record<string, TileBox>;
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
};
/** A column a filter can target, with how many tiles of the board have it. */
export type FilterField = { column: string; kind: FilterKind | string; tiles: number };
/** A filter an active tile could not honour, so the tile can say so. */
export type FilterSkip = { column: string; reason: "no_column" | "wrong_type" | string };

export type Board = {
  id: string;
  name: string;
  /** Satu kalimat tujuan board, tampil di halaman daftar dashboard. */
  description?: string;
  /** Nama tampilan pembuat board. Kosong untuk board lama / tanpa login. */
  createdBy?: string;
  /** Folder this board is filed in; empty or null = no folder. */
  folderId?: string | null;
  layout?: LayoutMap;
  filters?: FilterDef[];
  createdAt?: string;
  /**
   * Kapan board terakhir ditulis.
   *
   * Nilainya sama dengan `createdAt`, dan itu disengaja: tabelnya
   * `ReplacingMergeTree(created_at)`, jadi `created_at` adalah kolom versi
   * yang ditulis ulang `now()` setiap kali board di-rename / layout
   * disimpan / filter diubah. Jadi sejak awal isinya memang "terakhir
   * diubah", cuma namanya saja "created". Di UI, labeli **"Updated"** —
   * jangan pernah "Created", karena itu klaim yang tidak benar.
   */
  updatedAt?: string;
  /** Jumlah tile di board ini; dihitung server, bukan kolom tersimpan. */
  chartCount?: number;
  /** `true` hanya untuk board bawaan (`default`) yang tidak bisa dihapus. */
  builtin?: boolean;
  publicToken?: string;
  embedEnabled?: boolean;
};
