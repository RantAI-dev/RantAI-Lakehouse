/**
 * Semantic layer tipis untuk dashboarding — "metrics as code" ala Rill, tapi
 * nyatu di konsol (tanpa app BI kedua, tanpa AGPL).
 *
 * Tiap kartu didefinisikan di sini: SQL-nya (dijalankan SERVER-side oleh akun
 * ClickHouse read-only lewat /api/dashboard) + cara render-nya (kind + encoding).
 * Menambah chart = menambah satu entri di sini; tak perlu sentuh UI. Sumber
 * data = mart Gold di `serving.*` (satu-satunya layer yang boleh dipakai
 * dashboard — Raw/Bronze/Silver tidak).
 */

export type ChartKind =
  // batang & garis
  | "bar" | "hbar" | "line" | "area" | "stacked" | "combo"
  // komposisi
  | "pie" | "rose" | "funnel" | "treemap"
  // korelasi & distribusi
  | "scatter" | "bubble" | "heatmap" | "radar" | "waterfall"
  // geografis
  | "geomap"
  // angka tunggal
  | "kpi" | "gauge"
  // non-grafik
  | "table" | "text";
export type NumFmt = "int" | "float";
/** Asal spec: bawaan (seed), dibuat AI lewat chat, atau manual lewat UI. */
export type ChartSource = "builtin" | "ai" | "ui";

/** KPI angka tunggal. SQL harus mengembalikan kolom `v` (dan boleh kolom lain). */
export type KpiSpec = {
  id: string;
  title: string;
  sql: string;
  format: NumFmt;
  caption?: string;
  /** Mart sumber — untuk lineage/label. */
  mart: string;
};

/** Chart. SQL mengembalikan baris; `x`/`y` menunjuk kolom untuk sumbu/seri. */
export type ChartSpec = {
  id: string;
  title: string;
  subtitle?: string;
  kind: ChartKind;
  mart: string;
  sql: string;
  x: string;
  /** satu kolom (bar/line/pie) atau beberapa kolom (stacked). */
  y: string | string[];
  /**
   * Kolom breakdown opsional (dimensi ke-2): memecah y menjadi banyak seri per
   * nilai kolom ini (mis. multi-line per kawasan, grouped bar per kategori).
   * Bila diisi, `y` adalah satu measure & data long-format (x, series, nilai).
   */
  series?: string;
  format?: NumFmt;
  /** 2 = full width di grid. */
  span?: 1 | 2;
  /** Konten markdown untuk tile kind="text" (tanpa SQL). */
  text?: string;
  /** Caption/unit untuk tile kind="kpi". */
  caption?: string;
  /** Nilai target/max untuk kind="gauge" (bila kosong → auto dari nilai). */
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

/** Peta id → SQL untuk dipakai route server. */
export const SPEC_SQL: Record<string, string> = Object.fromEntries(
  [...KPIS, ...CHARTS].map((s) => [s.id, s.sql]),
);

/** Metadata render (tanpa SQL) — inilah yang dikirim ke klien. */
export type ChartRenderSpec = Omit<ChartSpec, "sql"> & {
  source: ChartSource;
  board?: string;
  /** Definisi terstruktur (ChartInput) untuk prefill saat edit. */
  def?: unknown;
};

/** Buang SQL dari spec, tempel asalnya — untuk respons API ke browser. */
export function toRenderSpec(spec: ChartSpec, source: ChartSource): ChartRenderSpec {
  const { sql: _sql, ...rest } = spec;
  return { ...rest, source };
}
