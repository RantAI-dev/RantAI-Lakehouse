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

export type ChartKind = "bar" | "hbar" | "line" | "area" | "pie" | "stacked";
export type NumFmt = "int" | "float";

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
  format?: NumFmt;
  /** 2 = full width di grid. */
  span?: 1 | 2;
};

const S = "serving";

export const KPIS: KpiSpec[] = [
  {
    id: "kpi_wisman_total",
    title: "Total Wisman",
    mart: "mart_wisman",
    sql: `SELECT sum(jumlah) AS v FROM ${S}.mart_wisman`,
    format: "int",
    caption: "kunjungan mancanegara (akumulasi)",
  },
  {
    id: "kpi_dtw",
    title: "Destinasi Terpantau",
    mart: "mart_kunjungan_dtw",
    sql: `SELECT count(DISTINCT destinasi) AS v FROM ${S}.mart_kunjungan_dtw`,
    format: "int",
    caption: "daya tarik wisata (DTW)",
  },
  {
    id: "kpi_event",
    title: "Event Tahun Terbaru",
    mart: "mart_event",
    sql: `SELECT jumlah_event AS v, tahun FROM ${S}.mart_event ORDER BY tahun DESC LIMIT 1`,
    format: "int",
    caption: "jumlah event pada tahun terakhir",
  },
  {
    id: "kpi_gci",
    title: "Indikator GCI Siap",
    mart: "mart_gci_readiness",
    sql: `SELECT sum(data_tersedia) AS v, count() AS total FROM ${S}.mart_gci_readiness`,
    format: "int",
    caption: "indikator dengan data tersedia",
  },
];

export const CHARTS: ChartSpec[] = [
  {
    id: "wisman_tren",
    title: "Tren Kunjungan Wisman",
    subtitle: "Total per bulan lintas tahun",
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
    title: "Top Negara Asal Wisman",
    subtitle: "10 kebangsaan teratas",
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
    title: "Wisman per Kawasan",
    subtitle: "Distribusi benua/kawasan",
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
    title: "Wisman per Pintu Masuk",
    subtitle: "Titik kedatangan",
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
    title: "Kunjungan per Destinasi",
    subtitle: "Wisnus vs Wisman, 8 destinasi teratas",
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
    title: "Tren Jumlah Event",
    subtitle: "Per tahun",
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
    title: "Kesiapan Data GCI",
    subtitle: "Distribusi status readiness",
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
    title: "Usaha Kuliner per Wilayah",
    subtitle: "Jumlah usaha terdaftar",
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
    title: "Titik Wisata (Atlas) per Kategori",
    subtitle: "Jumlah POI",
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
