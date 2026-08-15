import { chQuery, chRows } from "./clickhouse";

/**
 * Perkakas agent (server-side): grounding skema NYATA lakehouse + guard SELECT.
 * Dipakai endpoint agentic text-to-SQL / self-correcting query.
 */

/** Skema NYATA untuk grounding: mart Gold + daftar tabel Silver. */
export async function schemaContext(): Promise<string> {
  const martTables = (await chRows<{ name: string }>(`SHOW TABLES FROM serving`))
    .map((r) => r.name)
    .filter((n) => n.startsWith("mart_") && !n.endsWith("_baru"));
  const martDescs: string[] = [];
  for (const t of martTables) {
    const cols = await chRows<{ name: string; type: string }>(`DESCRIBE serving.\`${t}\``);
    martDescs.push(
      `serving.${t}(${cols.filter((c) => !c.name.startsWith("_")).map((c) => `${c.name} ${c.type}`).join(", ")})`,
    );
  }
  const silver = (await chRows<{ name: string }>(`SHOW TABLES FROM silver`)).map((r) => r.name).slice(0, 60);
  return (
    `TABEL MART (Gold, utama untuk agregasi):\n${martDescs.join("\n")}\n\n` +
    `TABEL SILVER (detail per dataset, akses: silver.\`<nama>\`):\n${silver.join(", ")}`
  );
}

/** true bila SQL aman (SELECT/WITH saja, tanpa DDL/DML). */
export function isReadOnlySql(sql: string): boolean {
  return (
    /^\s*(with|select)\b/i.test(sql) &&
    !/\b(insert|alter|drop|delete|update|create|truncate|rename|attach|detach|grant|revoke)\b/i.test(sql)
  );
}

/** Ambil JSON {sql, explanation, assumptions} dari output LLM (buang teks lain). */
export function extractSqlJson(
  text: string,
): { sql: string; explanation: string; assumptions: string[] } | null {
  // Coba blok ```json ... ``` atau JSON object mentah.
  const fenced = text.match(/```(?:json)?\s*(\{[\s\S]*?\})\s*```/i);
  const raw = fenced?.[1] ?? text.match(/\{[\s\S]*\}/)?.[0];
  if (!raw) return null;
  try {
    const o = JSON.parse(raw);
    if (typeof o.sql === "string") {
      return {
        sql: o.sql.trim(),
        explanation: String(o.explanation ?? ""),
        assumptions: Array.isArray(o.assumptions) ? o.assumptions.map(String) : [],
      };
    }
  } catch {
    /* bukan JSON valid */
  }
  return null;
}

export { chQuery };
