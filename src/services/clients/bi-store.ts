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
  text?: string; // konten markdown (kind="text")
  caption?: string; // unit/caption (kind="kpi")
};

export type Board = { id: string; name: string; layout?: LayoutMap; createdAt?: string };

const IDENT = /^[a-zA-Z0-9_]+$/;
const KINDS: ChartKind[] = ["bar", "hbar", "line", "area", "pie", "stacked", "kpi", "table", "text"];
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
       id String, name String, layout_json String DEFAULT '{}',
       created_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0
     ) ENGINE = ReplacingMergeTree(created_at) ORDER BY id`,
  );
  // Migrasi: tabel lama tanpa layout.
  await chExec("ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS layout_json String DEFAULT '{}'");
  ensured = true;
}

/** Posisi tile di kanvas grid (12 kolom). Key = chartId. */
export type TileBox = { x: number; y: number; w: number; h: number };
export type LayoutMap = Record<string, TileBox>;

function esc(s: string): string {
  // Escape backslash dulu, baru petik-satu — wajib untuk JSON (spec_json).
  return s.replace(/\\/g, "\\\\").replace(/'/g, "''");
}

// ── Boards ────────────────────────────────────────────────────────────────
function parseLayout(s: string): LayoutMap {
  try { const o = JSON.parse(s || "{}"); return o && typeof o === "object" ? (o as LayoutMap) : {}; }
  catch { return {}; }
}

export async function listBoards(): Promise<Board[]> {
  await ensureBiTable();
  const rows = await chRows<{ id: string; name: string; layout_json: string; created_at: string }>(
    `SELECT id, name, layout_json, toString(created_at) AS created_at FROM console.bi_board FINAL
      WHERE is_deleted = 0 ORDER BY created_at`,
  );
  return rows.map((r) => ({ id: r.id, name: r.name, layout: parseLayout(r.layout_json), createdAt: r.created_at }));
}

export async function getBoard(id: string): Promise<Board | null> {
  await ensureBiTable();
  const rows = await chRows<{ id: string; name: string; layout_json: string; created_at: string }>(
    `SELECT id, name, layout_json, toString(created_at) AS created_at FROM console.bi_board FINAL
      WHERE is_deleted = 0 AND id='${esc(id)}' LIMIT 1`,
  );
  const r = rows[0];
  return r ? { id: r.id, name: r.name, layout: parseLayout(r.layout_json), createdAt: r.created_at } : null;
}

export async function createBoard(name: string): Promise<Board> {
  await ensureBiTable();
  const clean = String(name ?? "").trim();
  if (!clean) throw new Error("nama dashboard wajib.");
  const id = `b_${randomUUID().slice(0, 8)}`;
  await chExec(`INSERT INTO console.bi_board (id, name) VALUES ('${esc(id)}', '${esc(clean)}')`);
  return { id, name: clean, layout: {} };
}

export async function renameBoard(id: string, name: string): Promise<void> {
  await ensureBiTable();
  const clean = String(name ?? "").trim();
  if (!clean) throw new Error("nama wajib.");
  await chExec(`ALTER TABLE console.bi_board UPDATE name='${esc(clean)}' WHERE id='${esc(id)}'`);
}

export async function updateBoardLayout(id: string, layout: LayoutMap): Promise<void> {
  await ensureBiTable();
  await chExec(`ALTER TABLE console.bi_board UPDATE layout_json='${esc(JSON.stringify(layout ?? {}))}' WHERE id='${esc(id)}'`);
}

export async function deleteBoard(id: string): Promise<void> {
  await ensureBiTable();
  const safe = esc(id);
  await chExec(`INSERT INTO console.bi_board (id, name, is_deleted) VALUES ('${safe}', '', 1)`);
  // Hapus juga chart di dalamnya (soft-delete) biar tak yatim.
  await chExec(`ALTER TABLE console.bi_chart UPDATE is_deleted=1 WHERE board='${safe}'`);
}

/** Duplikat dashboard beserta chart & tata letaknya (id baru). */
export async function duplicateBoard(id: string): Promise<Board> {
  await ensureBiTable();
  const src = await getBoard(id);
  if (!src) throw new Error("dashboard tidak ditemukan.");
  const charts = (await listStoredCharts()).filter((c) => c.board === id);
  const nb = await createBoard(`${src.name} (salinan)`);
  const idMap: Record<string, string> = {};
  for (const c of charts) {
    const newId = `u_${randomUUID().slice(0, 8)}`;
    idMap[c.id] = newId;
    await insertChart({ ...c, id: newId, board: nb.id });
  }
  const newLayout: LayoutMap = {};
  for (const [oldId, box] of Object.entries(src.layout ?? {})) {
    if (idMap[oldId]) newLayout[idMap[oldId]] = box;
  }
  await updateBoardLayout(nb.id, newLayout);
  return { ...nb, layout: newLayout };
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

/** SQL KPI (angka tunggal, kolom `v`) + filter tahun opsional. */
function buildKpiSql(d: { mart: string; measure: string; agg: string }, years?: number[]): string {
  const val = d.agg === "count" ? "count()" : `round(${d.agg}(${d.measure}))`;
  const where = years && years.length ? ` WHERE tahun IN (${years.join(",")})` : "";
  return `SELECT ${val} AS v FROM serving.${d.mart}${where}`;
}

/**
 * Validasi input terhadap skema NYATA di ClickHouse lalu susun StoredChartSpec.
 * Melempar Error ramah bila mart/kolom tak valid. `id` opsional → untuk EDIT.
 * Cabang per kind: text (tanpa SQL) / kpi (angka) / table & chart (grouped).
 */
export async function specFromInput(
  input: ChartInput,
  source: ChartSource,
  createdBy: string,
  id?: string,
): Promise<StoredChartSpec> {
  const title = String(input.title ?? "").trim();
  if (!title) throw new Error("title wajib diisi.");
  const kind = input.kind;
  if (!KINDS.includes(kind)) throw new Error(`kind tidak valid: ${kind}`);
  const newId = id ?? `u_${randomUUID().slice(0, 8)}`;
  const board = input.board?.trim() || "default";
  const span = (input.span === 2 ? 2 : 1) as 1 | 2;
  const subtitle = input.subtitle?.trim() || undefined;

  // ── TEXT — tanpa SQL/mart ────────────────────────────────────────────────
  if (kind === "text") {
    const text = String(input.text ?? "").trim();
    if (!text) throw new Error("konten teks wajib.");
    const def: ChartInput = { title, kind, mart: "", dimension: "", measures: [], text, span, board };
    const spec: ChartSpec = { id: newId, title, subtitle, kind, mart: "", sql: "", x: "", y: "", text, format: "int", span };
    return { ...spec, source, board, def, hasYear: false, createdBy };
  }

  // ── Butuh mart untuk kpi/table/chart ─────────────────────────────────────
  const mart = String(input.mart ?? "").replace(/^serving\./, "");
  if (!IDENT.test(mart)) throw new Error(`nama mart tidak valid: ${input.mart}`);
  const exists = await chRows<{ n: string }>(
    `SELECT toString(count()) AS n FROM system.tables WHERE database='serving' AND name='${esc(mart)}' AND name NOT LIKE '%\\_baru'`,
  );
  if (Number(exists[0]?.n ?? 0) === 0) throw new Error(`mart Gold '${mart}' tidak ditemukan di serving.`);
  const cols = new Set(
    (await chRows<{ name: string }>(`SELECT name FROM system.columns WHERE database='serving' AND table='${esc(mart)}'`)).map((c) => c.name),
  );
  const hasYear = cols.has("tahun");
  const agg = (input.aggregate ?? "sum").toLowerCase();
  if (!AGGS.has(agg)) throw new Error(`aggregate tidak valid: ${agg}`);
  const measures = (input.measures ?? []).map(String);
  if (measures.length === 0) throw new Error("minimal satu kolom measure.");
  if (measures.some((m) => !IDENT.test(m) || !cols.has(m))) throw new Error("kolom measure tidak valid / tak ada.");

  // ── KPI — angka tunggal ──────────────────────────────────────────────────
  if (kind === "kpi") {
    const m = measures[0];
    const def: ChartInput = {
      title, kind, mart, dimension: "", measures: [m], aggregate: agg as ChartInput["aggregate"],
      caption: input.caption?.trim() || undefined, span, board,
    };
    const sql = buildKpiSql({ mart, measure: m, agg });
    const spec: ChartSpec = { id: newId, title, subtitle, kind, mart, sql, x: "", y: m, caption: def.caption, format: "int", span };
    return { ...spec, source, board, def, hasYear, createdBy };
  }

  // ── TABLE / CHART — perlu dimensi ────────────────────────────────────────
  const dimension = String(input.dimension ?? "");
  if (!IDENT.test(dimension) || !cols.has(dimension)) throw new Error(`kolom dimensi '${dimension}' tak valid / tak ada.`);
  if (kind === "stacked" && measures.length < 2) throw new Error("chart 'stacked' butuh ≥2 measure.");
  const limit = Math.min(Math.max(Number(input.limit ?? 20) || 20, 1), 100);
  const order = input.order ?? (kind === "line" || kind === "area" ? "none" : "desc");
  const breakdown = input.breakdown ? String(input.breakdown) : "";
  if (breakdown) {
    if (!IDENT.test(breakdown) || !cols.has(breakdown)) throw new Error(`kolom breakdown '${breakdown}' tak valid / tak ada.`);
    if (breakdown === dimension) throw new Error("breakdown harus beda dari dimensi.");
    if (kind === "pie" || kind === "table") throw new Error("breakdown tak didukung untuk pie/tabel.");
    if (measures.length > 1) throw new Error("dengan breakdown, pakai tepat satu measure.");
  }

  const def: ChartInput = {
    title, subtitle, mart, kind, dimension, measures,
    breakdown: breakdown || undefined, aggregate: agg as ChartInput["aggregate"], limit,
    order, span, board,
  };
  const sql = buildSql({ mart, dimension, measures, agg, order, limit, breakdown: breakdown || undefined });
  const spec: ChartSpec = {
    id: newId, title, subtitle, kind, mart, sql, x: dimension,
    y: measures.length === 1 ? measures[0] : measures, series: breakdown || undefined, format: "int", span,
  };
  return { ...spec, source, board, def, hasYear, createdBy };
}

/** SQL untuk sebuah spec tersimpan dengan filter tahun runtime (dipakai /api/dashboard). */
export function sqlWithYear(spec: StoredChartSpec, years: number[]): string {
  if (spec.kind === "text") return "";
  if (!spec.hasYear || !years.length || !spec.def?.mart) return spec.sql;
  const d = spec.def;
  const agg = d.aggregate ?? "sum";
  if (spec.kind === "kpi") return buildKpiSql({ mart: d.mart, measure: d.measures[0], agg }, years);
  return buildSql(
    { mart: d.mart, dimension: d.dimension, measures: d.measures, agg, order: d.order ?? "none", limit: d.limit ?? 20, breakdown: d.breakdown },
    years,
  );
}

/** Simpan/replace spec (smoke-test SQL dulu agar tak menyimpan yang rusak). */
export async function insertChart(spec: StoredChartSpec): Promise<void> {
  await ensureBiTable();
  if (spec.sql) await chQuery(spec.sql); // smoke test — melempar bila SQL gagal (skip untuk text)
  const clean: ChartSpec = {
    id: spec.id, title: spec.title, subtitle: spec.subtitle, kind: spec.kind,
    mart: spec.mart, sql: spec.sql, x: spec.x, y: spec.y, series: spec.series,
    format: spec.format, span: spec.span, text: spec.text, caption: spec.caption,
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
