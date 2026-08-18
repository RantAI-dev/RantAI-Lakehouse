import { randomUUID } from "node:crypto";
import { chRows, chExec } from "./clickhouse";

/**
 * Riwayat percakapan AI Copilot — disimpan DI DALAM lakehouse (tabel
 * `console.chat_session` di ClickHouse), sejalan dengan spec dashboard.
 * Dipakai chat dock global (di semua halaman) & halaman /copilot; keduanya
 * berbagi sesi yang sama.
 */

export type StoredMessage = {
  role: "user" | "assistant";
  content: string;
  tools?: unknown;
  buildRunId?: string;
  chartCreated?: boolean;
};
export type ChatSession = {
  id: string;
  title: string;
  mode: string;
  messages: StoredMessage[];
  updatedAt?: string;
};

let ensured = false;
async function ensure(): Promise<void> {
  if (ensured) return;
  await chExec("CREATE DATABASE IF NOT EXISTS console");
  await chExec(
    `CREATE TABLE IF NOT EXISTS console.chat_session (
       id String, title String, mode String DEFAULT 'ask',
       messages_json String, updated_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0
     ) ENGINE = ReplacingMergeTree(updated_at) ORDER BY id`,
  );
  ensured = true;
}

function esc(s: string): string {
  return s.replace(/'/g, "''");
}

/** Daftar sesi (ringkas: tanpa isi pesan) untuk panel history. */
export async function listSessions(limit = 50): Promise<Omit<ChatSession, "messages">[]> {
  await ensure();
  const rows = await chRows<{ id: string; title: string; mode: string; updated_at: string }>(
    `SELECT id, title, mode, toString(updated_at) AS updated_at FROM console.chat_session FINAL
      WHERE is_deleted = 0 ORDER BY updated_at DESC LIMIT ${Math.min(Math.max(limit, 1), 200)}`,
  );
  return rows.map((r) => ({ id: r.id, title: r.title, mode: r.mode, updatedAt: r.updated_at }));
}

/** Ambil satu sesi lengkap dengan pesan. */
export async function getSession(id: string): Promise<ChatSession | null> {
  await ensure();
  const rows = await chRows<{ id: string; title: string; mode: string; messages_json: string; updated_at: string }>(
    `SELECT id, title, mode, messages_json, toString(updated_at) AS updated_at
       FROM console.chat_session FINAL WHERE is_deleted = 0 AND id='${esc(id)}' LIMIT 1`,
  );
  const r = rows[0];
  if (!r) return null;
  let messages: StoredMessage[] = [];
  try { messages = JSON.parse(r.messages_json) as StoredMessage[]; } catch { /* rusak */ }
  return { id: r.id, title: r.title, mode: r.mode, messages, updatedAt: r.updated_at };
}

/** Simpan/replace sesi. id opsional → dibuat bila belum ada. Judul dari pesan pertama. */
export async function saveSession(input: {
  id?: string; mode: string; messages: StoredMessage[];
}): Promise<{ id: string; title: string }> {
  await ensure();
  const id = input.id && /^[a-zA-Z0-9_]+$/.test(input.id) ? input.id : `c_${randomUUID().slice(0, 8)}`;
  const firstUser = input.messages.find((m) => m.role === "user");
  const title = (firstUser?.content ?? "Percakapan").slice(0, 80).replace(/\s+/g, " ").trim() || "Percakapan";
  // Batasi ukuran: bila terlalu besar, buang detail tool (sisakan teks).
  let json = JSON.stringify(input.messages);
  if (json.length > 200_000) {
    json = JSON.stringify(
      input.messages.map((m) => ({ role: m.role, content: m.content, buildRunId: m.buildRunId, chartCreated: m.chartCreated })),
    );
  }
  await chExec(
    `INSERT INTO console.chat_session (id, title, mode, messages_json) VALUES ` +
      `('${esc(id)}', '${esc(title)}', '${esc(input.mode)}', '${esc(json)}')`,
  );
  return { id, title };
}

export async function deleteSession(id: string): Promise<void> {
  await ensure();
  const safe = esc(id);
  await chExec(
    `INSERT INTO console.chat_session (id, title, mode, messages_json, is_deleted) VALUES ('${safe}', '', 'ask', '[]', 1)`,
  );
}
