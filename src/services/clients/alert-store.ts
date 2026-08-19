import { randomUUID } from "node:crypto";
import { chQuery, chRows, chExec } from "./clickhouse";
import { getBoard, listStoredCharts } from "./bi-store";
import { deliver, type DeliverResult } from "./notify";

/**
 * Alert & scheduled digest — hidup DI DALAM lakehouse (console.alert_rule).
 *  - alert: pantau agregat 1 measure Gold; kirim bila lewat ambang.
 *  - digest: ringkasan nilai KPI sebuah board (dijalankan terjadwal/manual).
 * Pengiriman via webhook / email (lihat notify.ts). SQL disusun server dari
 * identifier tervalidasi (anti-injeksi, Gold-only).
 */

export type AlertOp = ">" | ">=" | "<" | "<=" | "==";
export type AlertRule = {
  id: string;
  name: string;
  type: "alert" | "digest";
  mart?: string; measure?: string; agg?: string; op?: AlertOp; threshold?: number; // alert
  board?: string; // digest
  channel: "webhook" | "email";
  target: string; // webhook URL / email
  enabled: boolean;
  createdAt?: string;
};

const IDENT = /^[a-zA-Z_][a-zA-Z0-9_]*$/;
const AGGS = new Set(["sum", "avg", "max", "min", "count"]);
const OPS: AlertOp[] = [">", ">=", "<", "<=", "=="];
const esc = (s: string) => String(s ?? "").replace(/\\/g, "\\\\").replace(/'/g, "''");

let ensured = false;
async function ensure(): Promise<void> {
  if (ensured) return;
  await chExec("CREATE DATABASE IF NOT EXISTS console");
  await chExec(
    `CREATE TABLE IF NOT EXISTS console.alert_rule (
       id String, name String, type String DEFAULT 'alert',
       mart String DEFAULT '', measure String DEFAULT '', agg String DEFAULT 'sum',
       op String DEFAULT '>', threshold Float64 DEFAULT 0,
       board String DEFAULT '', channel String DEFAULT 'webhook', target String DEFAULT '',
       enabled UInt8 DEFAULT 1,
       created_at DateTime DEFAULT now(), updated_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0
     ) ENGINE = ReplacingMergeTree(updated_at) ORDER BY id`,
  );
  ensured = true;
}

type Row = {
  id: string; name: string; type: string; mart: string; measure: string; agg: string;
  op: string; threshold: number; board: string; channel: string; target: string; enabled: number; created_at: string;
};
const COLS = "id,name,type,mart,measure,agg,op,threshold,board,channel,target,enabled,toString(created_at) AS created_at";
function toRule(r: Row): AlertRule {
  return {
    id: r.id, name: r.name, type: r.type === "digest" ? "digest" : "alert",
    mart: r.mart || undefined, measure: r.measure || undefined, agg: r.agg || "sum",
    op: (OPS.includes(r.op as AlertOp) ? r.op : ">") as AlertOp, threshold: Number(r.threshold),
    board: r.board || undefined, channel: r.channel === "email" ? "email" : "webhook",
    target: r.target, enabled: Number(r.enabled) === 1, createdAt: r.created_at,
  };
}

export async function listRules(): Promise<AlertRule[]> {
  await ensure();
  const rows = await chRows<Row>(`SELECT ${COLS} FROM console.alert_rule FINAL WHERE is_deleted = 0 ORDER BY created_at`);
  return rows.map(toRule);
}
export async function getRule(id: string): Promise<AlertRule | null> {
  await ensure();
  const rows = await chRows<Row>(`SELECT ${COLS} FROM console.alert_rule FINAL WHERE is_deleted = 0 AND id='${esc(id)}' LIMIT 1`);
  return rows[0] ? toRule(rows[0]) : null;
}

/** Validasi + simpan (buat/replace). id opsional → update. */
export async function saveRule(input: Partial<AlertRule>, id?: string): Promise<AlertRule> {
  await ensure();
  const name = String(input.name ?? "").trim();
  if (!name) throw new Error("nama wajib.");
  const type = input.type === "digest" ? "digest" : "alert";
  const channel = input.channel === "email" ? "email" : "webhook";
  const target = String(input.target ?? "").trim();
  if (!target) throw new Error("target (webhook URL / email) wajib.");
  if (channel === "webhook" && !/^https?:\/\//i.test(target)) throw new Error("webhook URL tidak valid.");
  if (channel === "email" && !target.includes("@")) throw new Error("email tidak valid.");

  let mart = "", measure = "", agg = "sum", op: AlertOp = ">", threshold = 0, board = "";
  if (type === "alert") {
    mart = String(input.mart ?? "").replace(/^serving\./, "");
    measure = String(input.measure ?? "");
    agg = String(input.agg ?? "sum").toLowerCase();
    op = (OPS.includes(input.op as AlertOp) ? input.op : ">") as AlertOp;
    threshold = Number(input.threshold ?? 0);
    if (!IDENT.test(mart) || !IDENT.test(measure)) throw new Error("mart/measure tidak valid.");
    if (!AGGS.has(agg)) throw new Error("aggregate tidak valid.");
    if (!Number.isFinite(threshold)) throw new Error("threshold tidak valid.");
  } else {
    board = String(input.board ?? "");
    if (!board) throw new Error("digest butuh board.");
  }
  const rid = id ?? `al_${randomUUID().slice(0, 8)}`;
  await chExec(
    `INSERT INTO console.alert_rule (id,name,type,mart,measure,agg,op,threshold,board,channel,target,enabled) VALUES ` +
      `('${esc(rid)}','${esc(name)}','${type}','${esc(mart)}','${esc(measure)}','${esc(agg)}','${esc(op)}',${threshold},'${esc(board)}','${channel}','${esc(target)}',${input.enabled === false ? 0 : 1})`,
  );
  return (await getRule(rid))!;
}

export async function deleteRule(id: string): Promise<void> {
  await ensure();
  await chExec(`INSERT INTO console.alert_rule (id,name,is_deleted) VALUES ('${esc(id)}','',1)`);
}

// ── Evaluasi ────────────────────────────────────────────────────────────────
function compare(v: number, op: AlertOp, t: number): boolean {
  switch (op) {
    case ">": return v > t; case ">=": return v >= t;
    case "<": return v < t; case "<=": return v <= t;
    case "==": return v === t;
  }
}

const fmt = (n: number) => Math.round(n).toLocaleString("id-ID");

/** Nilai agregat saat ini untuk sebuah alert (mart/measure sudah tervalidasi). */
async function currentValue(mart: string, measure: string, agg: string, signal?: AbortSignal): Promise<number> {
  const val = agg === "count" ? "count()" : `round(${agg}(${measure}))`;
  const r = await chQuery(`SELECT ${val} AS v FROM serving.${mart}`, signal);
  return Number((r.data[0] as { v?: unknown } | undefined)?.v ?? 0);
}

/** Ringkasan digest sebuah board: nilai tile KPI/gauge + jumlah tile. */
async function digestText(boardId: string, signal?: AbortSignal): Promise<string> {
  const board = await getBoard(boardId);
  if (!board) return "Dashboard tidak ditemukan.";
  const charts = (await listStoredCharts()).filter((c) => (c.board || "default") === boardId);
  const lines: string[] = [`Dashboard: ${board.name} — ${charts.length} tile`];
  for (const c of charts) {
    if ((c.kind === "kpi" || c.kind === "gauge") && c.sql) {
      try {
        const r = await chQuery(c.sql, signal);
        const v = Number((r.data[0] as { v?: unknown } | undefined)?.v ?? 0);
        lines.push(`• ${c.title}: ${fmt(v)}`);
      } catch { /* skip */ }
    }
  }
  return lines.join("\n");
}

export type RunResult = { id: string; name: string; type: string; fired: boolean; value?: number; delivered?: DeliverResult; skipped?: string };

/** Jalankan semua rule aktif (atau satu bila `only`), kirim bila perlu. */
export async function runRules(only?: string, signal?: AbortSignal): Promise<RunResult[]> {
  const rules = (await listRules()).filter((r) => r.enabled && (!only || r.id === only));
  const out: RunResult[] = [];
  for (const r of rules) {
    try {
      if (r.type === "alert") {
        const v = await currentValue(r.mart!, r.measure!, r.agg ?? "sum", signal);
        const fired = compare(v, r.op ?? ">", r.threshold ?? 0);
        if (!fired) { out.push({ id: r.id, name: r.name, type: r.type, fired: false, value: v }); continue; }
        const text = `${r.agg}(${r.measure}) on ${r.mart} = ${fmt(v)} ${r.op} ${fmt(r.threshold ?? 0)} (threshold breached)`;
        const delivered = await deliver(r.channel, r.target, `⚠️ Alert: ${r.name}`, text);
        out.push({ id: r.id, name: r.name, type: r.type, fired: true, value: v, delivered });
      } else {
        const text = await digestText(r.board!, signal);
        const delivered = await deliver(r.channel, r.target, `📊 Digest: ${r.name}`, text);
        out.push({ id: r.id, name: r.name, type: r.type, fired: true, delivered });
      }
    } catch (e) {
      out.push({ id: r.id, name: r.name, type: r.type, fired: false, skipped: e instanceof Error ? e.message : String(e) });
    }
  }
  return out;
}
