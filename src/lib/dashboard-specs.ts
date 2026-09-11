/**
 * A thin semantic layer for dashboarding — "metrics as code" in the Rill
 * mould, but folded into the console itself (no second BI app, no AGPL).
 *
 * Each card is defined here: its SQL (run SERVER-side by a read-only
 * ClickHouse account via /api/dashboard) plus how it renders (kind +
 * encoding). Adding a chart means adding one entry here; the UI needs no
 * changes. Data comes from the Gold marts in `serving.*` (the only layer a
 * dashboard is allowed to use — not Raw/Bronze/Silver).
 */

export type ChartKind =
  // bar & line
  | "bar" | "hbar" | "line" | "area" | "stacked" | "combo"
  // composition
  | "pie" | "rose" | "funnel" | "treemap"
  // correlation & distribution
  | "scatter" | "bubble" | "heatmap" | "radar" | "waterfall"
  // geographic
  | "geomap"
  // single number
  | "kpi" | "gauge"
  // non-chart
  | "table" | "text";
export type NumFmt = "int" | "float";
/** Spec origin: builtin (seeded), AI-generated via chat, or manual via the UI. */
export type ChartSource = "builtin" | "ai" | "ui";

/** A single-number KPI. The SQL must return a `v` column (other columns are allowed). */
export type KpiSpec = {
  id: string;
  title: string;
  sql: string;
  format: NumFmt;
  caption?: string;
  /** Source mart — for lineage/labeling. */
  mart: string;
};

/** A chart. The SQL returns rows; `x`/`y` name the columns for the axis/series. */
export type ChartSpec = {
  id: string;
  title: string;
  subtitle?: string;
  kind: ChartKind;
  mart: string;
  sql: string;
  x: string;
  /** One column (bar/line/pie) or several columns (stacked). */
  y: string | string[];
  /**
   * Optional breakdown column (a second dimension): splits y into multiple
   * series by this column's value (e.g. one line per region, grouped bars
   * per category). When set, `y` is a single measure and the data is
   * long-format (x, series, value).
   */
  series?: string;
  format?: NumFmt;
  /** 2 = full width in the grid. */
  span?: 1 | 2;
  /** Markdown content for a kind="text" tile (no SQL). */
  text?: string;
  /** Caption/unit for a kind="kpi" tile. */
  caption?: string;
  /** Target/max value for kind="gauge" (auto-derived from the value when unset). */
  target?: number;
};

const S = "serving";

export const KPIS: KpiSpec[] = [
  {
    id: "kpi_wisman_total",
    title: "Total Foreign Visitors",
    mart: "mart_wisman",
    sql: `SELECT sum(jumlah) AS v FROM ${S}.mart_wisman`,
    format: "int",
    caption: "foreign visits (cumulative)",
  },
  {
    id: "kpi_dtw",
    title: "Tracked Destinations",
    mart: "mart_kunjungan_dtw",
    sql: `SELECT count(DISTINCT destinasi) AS v FROM ${S}.mart_kunjungan_dtw`,
    format: "int",
    caption: "tourist attractions (DTW)",
  },
  {
    id: "kpi_event",
    title: "Events (Latest Year)",
    mart: "mart_event",
    sql: `SELECT jumlah_event AS v, tahun FROM ${S}.mart_event ORDER BY tahun DESC LIMIT 1`,
    format: "int",
    caption: "number of events in the latest year",
  },
  {
    id: "kpi_gci",
    title: "GCI Indicators Ready",
    mart: "mart_gci_readiness",
    sql: `SELECT sum(data_tersedia) AS v, count() AS total FROM ${S}.mart_gci_readiness`,
    format: "int",
    caption: "indicators with data available",
  },
];

export const CHARTS: ChartSpec[] = [
  {
    id: "wisman_tren",
    title: "Foreign Visitor Trend",
    subtitle: "Monthly total across years",
    kind: "area",
    mart: "mart_wisman",
    span: 2,
    sql: `SELECT concat(toString(tahun),'-',leftPad(toString(bulan_no),2,'0')) AS periode,
                 round(sum(jumlah)) AS jumlah
          FROM ${S}.mart_wisman
          GROUP BY tahun, bulan_no
          ORDER BY tahun, bulan_no`,
    x: "periode",
    y: "jumlah",
    format: "int",
  },
  {
    id: "wisman_negara",
    title: "Top Source Countries",
    subtitle: "Top 10 nationalities",
    kind: "hbar",
    mart: "mart_wisman",
    sql: `SELECT negara, round(sum(jumlah)) AS jumlah
          FROM ${S}.mart_wisman
          GROUP BY negara ORDER BY jumlah DESC LIMIT 10`,
    x: "negara",
    y: "jumlah",
    format: "int",
  },
  {
    id: "wisman_kawasan",
    title: "Visitors by Region",
    subtitle: "Distribution by continent/region",
    kind: "pie",
    mart: "mart_wisman",
    sql: `SELECT kawasan, round(sum(jumlah)) AS jumlah
          FROM ${S}.mart_wisman
          GROUP BY kawasan ORDER BY jumlah DESC`,
    x: "kawasan",
    y: "jumlah",
    format: "int",
  },
  {
    id: "wisman_pintu",
    title: "Visitors by Entry Point",
    subtitle: "Arrival points",
    kind: "bar",
    mart: "mart_wisman",
    sql: `SELECT pintu_masuk, round(sum(jumlah)) AS jumlah
          FROM ${S}.mart_wisman
          GROUP BY pintu_masuk ORDER BY jumlah DESC`,
    x: "pintu_masuk",
    y: "jumlah",
    format: "int",
  },
  {
    id: "dtw_top",
    title: "Visits by Destination",
    subtitle: "Domestic vs foreign, top 8 destinations",
    kind: "stacked",
    mart: "mart_kunjungan_dtw",
    span: 2,
    sql: `SELECT destinasi, round(sum(wisnus)) AS wisnus, round(sum(wisman)) AS wisman
          FROM ${S}.mart_kunjungan_dtw
          GROUP BY destinasi ORDER BY sum(total) DESC LIMIT 8`,
    x: "destinasi",
    y: ["wisnus", "wisman"],
    format: "int",
  },
  {
    id: "event_tren",
    title: "Event Count Trend",
    subtitle: "Per year",
    kind: "line",
    mart: "mart_event",
    sql: `SELECT toString(tahun) AS tahun, jumlah_event AS jumlah
          FROM ${S}.mart_event ORDER BY tahun`,
    x: "tahun",
    y: "jumlah",
    format: "int",
  },
  {
    id: "gci_readiness",
    title: "GCI Data Readiness",
    subtitle: "Readiness status distribution",
    kind: "pie",
    mart: "mart_gci_readiness",
    sql: `SELECT readiness, count() AS n
          FROM ${S}.mart_gci_readiness
          GROUP BY readiness ORDER BY n DESC`,
    x: "readiness",
    y: "n",
    format: "int",
  },
  {
    id: "kuliner_wilayah",
    title: "Culinary Businesses by Area",
    subtitle: "Registered businesses",
    kind: "bar",
    mart: "mart_kuliner",
    sql: `SELECT wilayah, sum(jumlah_usaha) AS jumlah
          FROM ${S}.mart_kuliner
          GROUP BY wilayah ORDER BY jumlah DESC`,
    x: "wilayah",
    y: "jumlah",
    format: "int",
  },
  {
    id: "atlas_poi",
    title: "Tourism POIs by Category",
    subtitle: "Number of POIs",
    kind: "hbar",
    mart: "mart_atlas",
    sql: `SELECT kategori, jumlah_poi AS jumlah
          FROM ${S}.mart_atlas ORDER BY jumlah_poi DESC`,
    x: "kategori",
    y: "jumlah",
    format: "int",
  },
];

/** Map of id → SQL, for use by the server route. */
export const SPEC_SQL: Record<string, string> = Object.fromEntries(
  [...KPIS, ...CHARTS].map((s) => [s.id, s.sql]),
);

/** Render metadata (no SQL) — this is what gets sent to the client. */
export type ChartRenderSpec = Omit<ChartSpec, "sql"> & {
  source: ChartSource;
  board?: string;
  /** Structured definition (ChartInput) to prefill on edit. */
  def?: unknown;
};

/** Strip SQL from the spec and attach its source — for the API response to the browser. */
export function toRenderSpec(spec: ChartSpec, source: ChartSource): ChartRenderSpec {
  const { sql: _sql, ...rest } = spec;
  return { ...rest, source };
}
