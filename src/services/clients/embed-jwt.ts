import crypto from "crypto";
import { chExec, chRows } from "./clickhouse";

/**
 * Signed embedding ala Metabase — host meng-encode JWT HS256 berisi resource
 * (dashboard) + params terkunci, ditandatangani dengan EMBEDDING SECRET. Server
 * kita memverifikasi tanda tangan + exp, lalu merender dashboard dengan filter
 * yang dikunci (viewer tak bisa mengubahnya). Tak butuh library eksternal.
 */

function b64url(buf: Buffer): string {
  return buf.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function b64urlJson(obj: unknown): string {
  return b64url(Buffer.from(JSON.stringify(obj), "utf8"));
}
function fromB64url(s: string): Buffer {
  return Buffer.from(s.replace(/-/g, "+").replace(/_/g, "/"), "base64");
}

// ── Secret (persisten) ──────────────────────────────────────────────────────
let secretCache: string | null = null;
async function ensureKv(): Promise<void> {
  await chExec("CREATE DATABASE IF NOT EXISTS console");
  await chExec(
    `CREATE TABLE IF NOT EXISTS console.app_kv (
       k String, v String, updated_at DateTime DEFAULT now()
     ) ENGINE = ReplacingMergeTree(updated_at) ORDER BY k`,
  );
}

/**
 * Ambil embedding secret: utamakan env EMBED_SECRET; kalau tak ada, baca/GENERATE
 * lalu simpan di console.app_kv agar konsisten lintas restart.
 */
export async function getEmbedSecret(): Promise<string> {
  if (process.env.EMBED_SECRET) return process.env.EMBED_SECRET;
  if (secretCache) return secretCache;
  await ensureKv();
  const rows = await chRows<{ v: string }>("SELECT v FROM console.app_kv FINAL WHERE k='embed_secret' LIMIT 1");
  if (rows[0]?.v) { secretCache = rows[0].v; return secretCache; }
  const gen = crypto.randomBytes(32).toString("hex");
  await chExec(`INSERT INTO console.app_kv (k, v) VALUES ('embed_secret', '${gen}')`);
  secretCache = gen;
  return gen;
}

// ── Sign / Verify ─────────────────────────────────────────────────────────────
export type EmbedClaims = {
  resource?: { dashboard?: string };
  params?: Record<string, string | string[]>;
  exp?: number;
};

/** Tandatangani JWT HS256 (dipakai host; juga untuk contoh/preview di UI). */
export function signEmbed(claims: EmbedClaims, secret: string): string {
  const header = b64urlJson({ alg: "HS256", typ: "JWT" });
  const payload = b64urlJson(claims);
  const data = `${header}.${payload}`;
  const sig = b64url(crypto.createHmac("sha256", secret).update(data).digest());
  return `${data}.${sig}`;
}

/** Verifikasi tanda tangan + exp. Mengembalikan claims bila valid, else null. */
export function verifyEmbed(token: string, secret: string): EmbedClaims | null {
  const parts = token.split(".");
  if (parts.length !== 3) return null;
  const [header, payload, sig] = parts;
  const expected = crypto.createHmac("sha256", secret).update(`${header}.${payload}`).digest();
  let given: Buffer;
  try { given = fromB64url(sig); } catch { return null; }
  if (given.length !== expected.length || !crypto.timingSafeEqual(given, expected)) return null;
  let claims: EmbedClaims;
  try { claims = JSON.parse(fromB64url(payload).toString("utf8")); } catch { return null; }
  if (typeof claims.exp === "number" && claims.exp * 1000 < Date.now()) return null;
  return claims;
}
