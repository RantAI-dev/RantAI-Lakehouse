import { randomUUID } from "node:crypto";
import { chQuery, chRows, chExec } from "./clickhouse";
import type { ChartKind, ChartSpec, ChartSource } from "@/lib/dashboard-specs";

/**
 * Penyimpanan spec chart & board — dashboard hidup DI DALAM lakehouse (tabel
 * `console.bi_chart` + `console.bi_board` di ClickHouse), bukan file/DB terpisah.
 * Ini yang bikin "BI lakehouse": board manual (UI) & lewat chat (AI) menulis ke
 * artefak yang SAMA, jadi selalu sinkron.
 *
 * SQL chart TIDAK pernah datang mentah dari LLM/user — server menyusunnya dari
 * identifier tervalidasi (mart & kolom yang ada di serving.*), jadi tak ada jalan
 * injeksi & dijamin hanya menyentuh Gold. Definisi terstruktur (`def`) disimpan
 * agar chart bisa DIEDIT & difilter ulang (mis. filter tahun) tanpa parse SQL.
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
};

export type Board = { id: string; name: string; createdAt?: string };

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
       board String DEFAULT 'default',
       created_by String DEFAULT 'ui',
       created_at DateTime DEFAULT now(),
       is_deleted UInt8 DEFAULT 0
     ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id`,
  );
  // Migrasi aman untuk tabel lama tanpa kolom board.
  await chExec("ALTER TABLE console.bi_chart ADD COLUMN IF NOT EXISTS board String DEFAULT 'default'");
  await chExec(
    `CREATE TABLE IF NOT EXISTS console.bi_board (
       id String, name String,
       created_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0
     ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id`,
  );
  ensured = true;
}

function esc(s: string): string {
  return s.replace(/'/g, "''");
}

// ── Boards ────────────────────────────────────────────────────────────────
export async function listBoards(): Promise<Board[]> {
  await ensureBiTable();
  const rows = await chRows<{ id: string; name: string; created_at: string }>(
    `SELECT id, name, toString(created_at) AS created_at FROM console.bi_board FINAL
      WHERE is_deleted = 0 ORDER BY created_at`,
  );
  return rows.map((r) => ({ id: r.id, name: r.name, createdAt: r.created_at }));
}

export async function createBoard(name: string): Promise<Board> {
  await ensureBiTable();
  const clean = String(name ?? "").trim();
  if (!clean) throw new Error("nama board wajib.");
  const id = `b_${randomUUID().slice(0, 8)}`;
  await chExec(
    `INSERT INTO console.bi_board (id, name) VALUES ('${esc(id)}', '${esc(clean)}')`,
  );
  return { id, name: clean };
}

export async function deleteBoard(id: string): Promise<void> {
  await ensureBiTable();
  const safe = esc(id);
  await chExec(`INSERT INTO console.bi_board (id, name, is_deleted) VALUES ('${safe}', '', 1)`);
  // Chart-nya kembalikan ke default (jangan hilang).
  await chExec(`ALTER TABLE console.bi_chart UPDATE board='default' WHERE board='${safe}'`);
}

// ── Charts ──────────────────────────────────────────────────────────────
/** Daftar spec tersimpan (hidup, terbaru per id). */
export async function listStoredCharts(): Promise<StoredChartSpec[]> {
  await ensureBiTable();
  const rows = await chRows<{ id: string; spec_json: string; board: string; created_by: string; created_at: string }>(
    `SELECT id, spec_json, board, created_by, toString(created_at) AS created_at
       FROM console.bi_chart FINAL WHERE is_deleted = 0 ORDER BY created_at`,
  );
  const out: StoredChartSpec[] = [];
  for (const r of rows) {
    try {
      const parsed = JSON.parse(r.spec_json) as { spec?: ChartSpec; def?: ChartInput; hasYear?: boolean } & ChartSpec;
      // Dukung format baru {spec,def,hasYear} maupun lama (ChartSpec telanjang).
      const spec = parsed.spec ?? (parsed as ChartSpec);
      const def = parsed.def ?? undefined;
      const source: ChartSource = r.created_by === "ai" ? "ai" : "ui";
      out.push({
        ...spec,
        source,
        board: r.board || "default",
        def: def ?? ({} as ChartInput),
        hasYear: parsed.hasYear ?? false,
        createdBy: r.created_by,
        createdAt: r.created_at,
      });
    } catch {
      /* spec rusak — lewati */
    }
  }
  return out;
}

/** Susun klausa SQL dari identifier tervalidasi (+ filter tahun opsional). */
function buildSql(
  d: { mart: string; dimension: string; measures: string[]; agg: string; order: string; limit: number; breakdown?: string },
  years?: number[],
): string {
  const { mart, dimension, measures, agg, order, limit, breakdown } = d;
  const aggOf = (m: string) => (agg === "count" ? `count() AS ${m}` : `round(${agg}(${m})) AS ${m}`);
  const yearWhere = years && years.length ? `tahun IN (${years.join(",")})` : "";
  if (breakdown) {
    const m = measures[0];
    const innerWhere = yearWhere ? `WHERE ${yearWhere}` : "";
    const outerWhere = yearWhere ? `WHERE ${yearWhere} AND` : "WHERE";
    return (
      `SELECT ${dimension}, ${breakdown}, ${aggOf(m)} FROM serving.${mart} ` +
      `${outerWhere} ${dimension} IN (SELECT ${dimension} FROM serving.${mart} ${innerWhere} ` +
      `GROUP BY ${dimension} ORDER BY ${agg === "count" ? "count()" : `${agg}(${m})`} DESC LIMIT ${limit}) ` +
      `GROUP BY ${dimension}, ${breakdown} ORDER BY ${dimension}, ${breakdown}`
    );
  }
  const sel = measures.map(aggOf).join(", ");
  const orderClause = order === "none" ? dimension : `${measures[0]} ${order === "asc" ? "ASC" : "DESC"}`;
  const where = yearWhere ? `WHERE ${yearWhere} ` : "";
  return `SELECT ${dimension}, ${sel} FROM serving.${mart} ${where}GROUP BY ${dimension} ORDER BY ${orderClause} LIMIT ${limit}`;
}

/**
 * Validasi input terhadap skema NYATA di ClickHouse lalu susun StoredChartSpec.
 * Melempar Error ramah bila mart/kolom tak valid. `id` opsional → untuk EDIT
 * (mempertahankan id lama).
 */
export async function specFromInput(
  input: ChartInput,
  source: ChartSource,
  createdBy: string,
  id?: string,
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
  if (kind === "stacked" && measures.length < 2) throw new Error("chart 'stacked' butuh ≥2 measure.");

  const exists = await chRows<{ n: string }>(
    `SELECT toString(count()) AS n FROM system.tables
      WHERE database='serving' AND name='${esc(mart)}' AND name NOT LIKE '%\\_baru'`,
  );
  if (Number(exists[0]?.n ?? 0) === 0) throw new Error(`mart Gold '${mart}' tidak ditemukan di serving.`);

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

  const breakdown = input.breakdown ? String(input.breakdown) : "";
  if (breakdown) {
    if (!IDENT.test(breakdown)) throw new Error(`kolom breakdown tidak valid: ${breakdown}`);
    if (!cols.has(breakdown)) throw new Error(`kolom '${breakdown}' tidak ada di serving.${mart}.`);
    if (breakdown === dimension) throw new Error("breakdown harus beda dari dimensi.");
    if (kind === "pie") throw new Error("chart 'pie' tak mendukung breakdown.");
    if (measures.length > 1) throw new Error("dengan breakdown, pakai tepat satu measure.");
  }

  const def: ChartInput = {
    title, subtitle: input.subtitle?.trim() || undefined, mart, kind, dimension, measures,
    breakdown: breakdown || undefined, aggregate: agg as ChartInput["aggregate"], limit,
    order, span: input.span === 2 ? 2 : 1, board: input.board?.trim() || "default",
  };
  const sql = buildSql({ mart, dimension, measures, agg, order, limit, breakdown: breakdown || undefined });

  const spec: ChartSpec = {
    id: id ?? `u_${randomUUID().slice(0, 8)}`,
    title,
    subtitle: def.subtitle,
    kind,
    mart,
    sql,
    x: dimension,
    y: measures.length === 1 ? measures[0] : measures,
    series: breakdown || undefined,
    format: "int",
    span: def.span as 1 | 2,
  };
  return {
    ...spec, source, board: def.board!, def, hasYear: cols.has("tahun"), createdBy,
  };
}

/** SQL untuk sebuah spec tersimpan dengan filter tahun runtime (dipakai /api/dashboard). */
export function sqlWithYear(spec: StoredChartSpec, years: number[]): string {
  if (!spec.hasYear || !years.length || !spec.def?.mart) return spec.sql;
  const d = spec.def;
  return buildSql(
    {
      mart: d.mart, dimension: d.dimension, measures: d.measures,
      agg: (d.aggregate ?? "sum"), order: d.order ?? "none",
      limit: d.limit ?? 20, breakdown: d.breakdown,
    },
    years,
  );
}

/** Simpan/replace spec (smoke-test SQL dulu agar tak menyimpan yang rusak). */
export async function insertChart(spec: StoredChartSpec): Promise<void> {
  await ensureBiTable();
  await chQuery(spec.sql); // smoke test — melempar bila SQL gagal
  const clean: ChartSpec = {
    id: spec.id, title: spec.title, subtitle: spec.subtitle, kind: spec.kind,
    mart: spec.mart, sql: spec.sql, x: spec.x, y: spec.y, series: spec.series,
    format: spec.format, span: spec.span,
  };
  const payload = esc(JSON.stringify({ spec: clean, def: spec.def, hasYear: spec.hasYear }));
  await chExec(
    `INSERT INTO console.bi_chart (id, title, spec_json, board, created_by) VALUES ` +
      `('${esc(spec.id)}', '${esc(spec.title)}', '${payload}', '${esc(spec.board)}', '${esc(spec.createdBy ?? spec.source)}')`,
  );
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
