import { randomUUID } from "node:crypto";
import { chQuery, chRows, chExec } from "./clickhouse";
import type { ChartKind, ChartSpec, ChartSource } from "@/lib/dashboard-specs";

/**
 * Penyimpanan spec chart — dashboard hidup DI DALAM lakehouse (tabel
 * `console.bi_chart` di ClickHouse), bukan file/DB terpisah. Ini yang bikin
 * "BI lakehouse": board yang dibikin manual (UI) maupun lewat chat (AI) menulis
 * ke artefak yang SAMA, jadi dua jalur selalu sinkron.
 *
 * SQL chart TIDAK pernah datang mentah dari LLM/user — server yang MENYUSUN-nya
 * dari identifier tervalidasi (mart & kolom yang benar-benar ada di serving.*),
 * jadi tak ada jalan injeksi & dijamin hanya menyentuh layer Gold.
 */

export type StoredChartSpec = ChartSpec & {
  source: ChartSource;
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
};

const IDENT = /^[a-zA-Z0-9_]+$/;
const KINDS: ChartKind[] = ["bar", "hbar", "line", "area", "pie", "stacked"];
const AGGS = new Set(["sum", "avg", "max", "min", "count"]);

let ensured = false;
/** Buat db+tabel bila belum ada (idempoten, sekali per proses). */
export async function ensureBiTable(): Promise<void> {
  if (ensured) return;
  await chExec("CREATE DATABASE IF NOT EXISTS console");
  await chExec(
    `CREATE TABLE IF NOT EXISTS console.bi_chart (
       id String,
       title String,
       spec_json String,
       created_by String DEFAULT 'ui',
       created_at DateTime DEFAULT now(),
       is_deleted UInt8 DEFAULT 0
     ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id`,
  );
  ensured = true;
}

/** Daftar spec tersimpan (hidup, terbaru per id). */
export async function listStoredCharts(): Promise<StoredChartSpec[]> {
  await ensureBiTable();
  const rows = await chRows<{ spec_json: string; created_by: string; created_at: string }>(
    `SELECT spec_json, created_by, toString(created_at) AS created_at
       FROM console.bi_chart FINAL WHERE is_deleted = 0 ORDER BY created_at`,
  );
  const out: StoredChartSpec[] = [];
  for (const r of rows) {
    try {
      const spec = JSON.parse(r.spec_json) as ChartSpec;
      const source: ChartSource = r.created_by === "ai" ? "ai" : "ui";
      out.push({ ...spec, source, createdBy: r.created_by, createdAt: r.created_at });
    } catch {
      /* spec rusak — lewati */
    }
  }
  return out;
}

function esc(s: string): string {
  return s.replace(/'/g, "''");
}

/**
 * Validasi input terhadap skema NYATA di ClickHouse lalu susun ChartSpec (+SQL).
 * Melempar Error dengan pesan ramah bila mart/kolom tak valid.
 */
export async function specFromInput(
  input: ChartInput,
  source: ChartSource,
  createdBy: string,
): Promise<StoredChartSpec> {
  const title = String(input.title ?? "").trim();
  if (!title) throw new Error("title wajib diisi.");
  const mart = String(input.mart ?? "").replace(/^serving\./, "");
  if (!IDENT.test(mart)) throw new Error(`nama mart tidak valid: ${input.mart}`);
  const kind = input.kind;
  if (!KINDS.includes(kind)) throw new Error(`kind tidak valid: ${kind} (pilih ${KINDS.join("/")})`);

  const dimension = String(input.dimension ?? "");
  const measures = (input.measures ?? []).map(String);
  if (!IDENT.test(dimension)) throw new Error(`kolom dimensi tidak valid: ${dimension}`);
  if (measures.length === 0) throw new Error("minimal satu kolom measure.");
  if (measures.some((m) => !IDENT.test(m))) throw new Error("kolom measure tidak valid.");
  if (kind === "stacked" && measures.length < 2)
    throw new Error("chart 'stacked' butuh ≥2 measure.");

  // Mart harus ada di serving & bukan tabel staging *_baru.
  const exists = await chRows<{ n: string }>(
    `SELECT toString(count()) AS n FROM system.tables
      WHERE database='serving' AND name='${esc(mart)}' AND name NOT LIKE '%\\_baru'`,
  );
  if (Number(exists[0]?.n ?? 0) === 0)
    throw new Error(`mart Gold '${mart}' tidak ditemukan di serving.`);

  // Kolom harus benar-benar ada.
  const cols = new Set(
    (
      await chRows<{ name: string }>(
        `SELECT name FROM system.columns WHERE database='serving' AND table='${esc(mart)}'`,
      )
    ).map((c) => c.name),
  );
  for (const c of [dimension, ...measures]) {
    if (!cols.has(c)) throw new Error(`kolom '${c}' tidak ada di serving.${mart}.`);
  }

  const agg = (input.aggregate ?? "sum").toLowerCase();
  if (!AGGS.has(agg)) throw new Error(`aggregate tidak valid: ${agg}`);
  const limit = Math.min(Math.max(Number(input.limit ?? 20) || 20, 1), 100);
  const order = input.order ?? (kind === "line" || kind === "area" ? "none" : "desc");
  const aggOf = (m: string) => (agg === "count" ? `count() AS ${m}` : `round(${agg}(${m})) AS ${m}`);

  // Breakdown (dimensi ke-2) → data long-format (dimensi, series, satu measure).
  const breakdown = input.breakdown ? String(input.breakdown) : "";
  if (breakdown) {
    if (!IDENT.test(breakdown)) throw new Error(`kolom breakdown tidak valid: ${breakdown}`);
    if (!cols.has(breakdown)) throw new Error(`kolom '${breakdown}' tidak ada di serving.${mart}.`);
    if (breakdown === dimension) throw new Error("breakdown harus beda dari dimensi.");
    if (kind === "pie") throw new Error("chart 'pie' tak mendukung breakdown.");
    if (measures.length > 1) throw new Error("dengan breakdown, pakai tepat satu measure.");
  }

  let sql: string;
  if (breakdown) {
    const m = measures[0];
    // Batasi kardinalitas dimensi-X ke top-N by measure agar chart terbaca.
    sql =
      `SELECT ${dimension}, ${breakdown}, ${aggOf(m)} FROM serving.${mart} ` +
      `WHERE ${dimension} IN (SELECT ${dimension} FROM serving.${mart} ` +
      `GROUP BY ${dimension} ORDER BY ${agg === "count" ? "count()" : `${agg}(${m})`} DESC LIMIT ${limit}) ` +
      `GROUP BY ${dimension}, ${breakdown} ORDER BY ${dimension}, ${breakdown}`;
  } else {
    const selMeasures = measures.map(aggOf).join(", ");
    const orderClause = order === "none" ? dimension : `${measures[0]} ${order === "asc" ? "ASC" : "DESC"}`;
    sql =
      `SELECT ${dimension}, ${selMeasures} FROM serving.${mart} ` +
      `GROUP BY ${dimension} ORDER BY ${orderClause} LIMIT ${limit}`;
  }

  const id = `u_${randomUUID().slice(0, 8)}`;
  const spec: ChartSpec = {
    id,
    title,
    subtitle: input.subtitle?.trim() || undefined,
    kind,
    mart,
    sql,
    x: dimension,
    y: measures.length === 1 ? measures[0] : measures,
    series: breakdown || undefined,
    format: "int",
    span: input.span === 2 ? 2 : 1,
  };
  return { ...spec, source, createdBy };
}

/** Simpan spec (jalankan dulu SQL-nya sebagai smoke-test agar tak menyimpan yang rusak). */
export async function insertChart(spec: StoredChartSpec): Promise<void> {
  await ensureBiTable();
  await chQuery(spec.sql); // smoke test — melempar bila SQL gagal
  const clean: ChartSpec = {
    id: spec.id, title: spec.title, subtitle: spec.subtitle, kind: spec.kind,
    mart: spec.mart, sql: spec.sql, x: spec.x, y: spec.y, series: spec.series,
    format: spec.format, span: spec.span,
  };
  const json = esc(JSON.stringify(clean));
  await chExec(
    `INSERT INTO console.bi_chart (id, title, spec_json, created_by) VALUES ` +
      `('${esc(spec.id)}', '${esc(spec.title)}', '${json}', '${esc(spec.createdBy ?? source(spec))}')`,
  );
}

function source(spec: StoredChartSpec): string {
  return spec.source;
}

/** Hapus (soft-delete via tombstone; ReplacingMergeTree ambil versi terbaru). */
export async function deleteChart(id: string): Promise<void> {
  await ensureBiTable();
  const safe = esc(id);
  await chExec(
    `INSERT INTO console.bi_chart (id, title, spec_json, created_by, is_deleted) VALUES ` +
      `('${safe}', '', '{}', 'system', 1)`,
  );
}
