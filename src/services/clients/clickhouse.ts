/**
 * Klien ClickHouse (server-side) untuk konsol RantAI Lakehouse.
 *
 * Ini bicara ke engine NYATA — lakehouse kita (ClickHouse 26.7). Dipakai hanya
 * dari route handler `src/app/api/**` (server), tidak pernah dari browser, agar
 * kredensial tidak bocor dan SQL pengguna dieksekusi oleh akun read-only.
 *
 * Tidak pakai dependency tambahan — cukup HTTP interface ClickHouse + fetch.
 */

const CH_URL = process.env.CH_URL ?? "http://localhost:18123";
const CH_USER = process.env.CH_USER ?? "default";
const CH_PASSWORD = process.env.CH_PASSWORD ?? "";

export type ChJsonResult = {
  meta: { name: string; type: string }[];
  data: Record<string, unknown>[];
  rows: number;
  statistics?: { elapsed: number; rows_read: number; bytes_read: number };
};

/** Jalankan SQL, kembalikan hasil ter-struktur (FORMAT JSON). Melempar pada error. */
export async function chQuery(
  sql: string,
  signal?: AbortSignal,
): Promise<ChJsonResult> {
  // Bungkus dengan FORMAT JSON agar dapat meta + statistics. Bila SQL sudah
  // mengandung FORMAT, biarkan apa adanya.
  const hasFormat = /\bformat\s+\w+\s*;?\s*$/i.test(sql.trim());
  const body = hasFormat ? sql : `${sql.replace(/;\s*$/, "")}\nFORMAT JSON`;

  const auth = Buffer.from(`${CH_USER}:${CH_PASSWORD}`).toString("base64");
  const res = await fetch(CH_URL, {
    method: "POST",
    headers: {
      Authorization: `Basic ${auth}`,
      "Content-Type": "text/plain; charset=utf-8",
    },
    body,
    signal,
    cache: "no-store",
  });

  const text = await res.text();
  if (!res.ok) {
    // ClickHouse mengirim pesan error yang jelas di body.
    throw new Error(text.trim() || `ClickHouse HTTP ${res.status}`);
  }
  try {
    return JSON.parse(text) as ChJsonResult;
  } catch {
    // Query tanpa hasil tabular (mis. non-SELECT) — kembalikan kosong.
    return { meta: [], data: [], rows: 0 };
  }
}

/** Query util yang mengembalikan array baris (tanpa metadata). */
export async function chRows<T = Record<string, unknown>>(
  sql: string,
  signal?: AbortSignal,
): Promise<T[]> {
  return (await chQuery(sql, signal)).data as T[];
}

/**
 * Jalankan statement NON-SELECT (DDL/DML: CREATE/INSERT/ALTER) apa adanya —
 * tanpa membungkus FORMAT JSON (yang akan merusak INSERT/DDL). Melempar pada
 * error. Tidak mengembalikan baris.
 */
export async function chExec(sql: string, signal?: AbortSignal): Promise<void> {
  const auth = Buffer.from(`${CH_USER}:${CH_PASSWORD}`).toString("base64");
  const res = await fetch(CH_URL, {
    method: "POST",
    headers: {
      Authorization: `Basic ${auth}`,
      "Content-Type": "text/plain; charset=utf-8",
    },
    body: sql,
    signal,
    cache: "no-store",
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text.trim() || `ClickHouse HTTP ${res.status}`);
  }
}
