import { NextResponse } from "next/server";
import { chQuery, chRows } from "@/services/clients/clickhouse";
import { chat } from "@/services/clients/llm";

export const dynamic = "force-dynamic";
export const maxDuration = 60;

/**
 * Agent text-to-SQL: pertanyaan bahasa natural → SQL ClickHouse.
 * Di-grounding ke skema NYATA lakehouse (mart Gold + dataset Silver) supaya
 * LLM tak mengarang tabel/kolom. Mengembalikan { sql, explanation, assumptions }.
 */

async function schemaContext(): Promise<string> {
  // Pakai SHOW/DESCRIBE (bukan system.*) agar tak bentrok query-cache pada
  // akun read-only. Mart Gold + kolomnya:
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
  const silver = (await chRows<{ name: string }>(`SHOW TABLES FROM silver`))
    .map((r) => r.name)
    .slice(0, 60);
  return (
    `TABEL MART (Gold, utama untuk agregasi):\n${martDescs.join("\n")}\n\n` +
    `TABEL SILVER (detail per dataset, akses: silver.<nama>):\n${silver.join(", ")}`
  );
}

const SYSTEM = `Kamu ahli SQL ClickHouse untuk lakehouse pariwisata DKI Jakarta.
Ubah pertanyaan pengguna jadi SATU query SELECT ClickHouse yang valid.
Aturan:
- HANYA gunakan tabel/kolom dari skema yang diberikan. Jangan mengarang.
- Utamakan tabel serving.mart_* untuk agregasi.
- Selalu SELECT (baca saja). Jangan INSERT/ALTER/DROP.
- Batasi hasil dengan LIMIT wajar (<=100) kecuali diminta lain.
- Balas HANYA JSON valid: {"sql": "...", "explanation": "...", "assumptions": ["..."]}`;

function extractJson(text: string): { sql: string; explanation: string; assumptions: string[] } | null {
  const m = text.match(/\{[\s\S]*\}/);
  if (!m) return null;
  try {
    const o = JSON.parse(m[0]);
    if (typeof o.sql === "string") {
      return {
        sql: o.sql.trim(),
        explanation: String(o.explanation ?? ""),
        assumptions: Array.isArray(o.assumptions) ? o.assumptions.map(String) : [],
      };
    }
  } catch {
    /* bukan JSON */
  }
  return null;
}

export async function POST(req: Request) {
  let question = "";
  let run = false;
  try {
    const body = await req.json();
    question = String(body.question ?? "");
    run = Boolean(body.run);
  } catch {
    return NextResponse.json({ error: "Body harus JSON {question, run?}" }, { status: 400 });
  }
  if (!question.trim()) return NextResponse.json({ error: "question wajib" }, { status: 400 });

  let schema: string;
  try {
    schema = await schemaContext();
  } catch (e) {
    return NextResponse.json({ error: `Gagal baca skema: ${e}` }, { status: 503 });
  }

  let out: { sql: string; explanation: string; assumptions: string[] } | null = null;
  let llmError: string | null = null;
  try {
    const content = await chat(
      [
        { role: "system", content: SYSTEM },
        { role: "user", content: `SKEMA:\n${schema}\n\nPERTANYAAN: ${question}` },
      ],
      { signal: req.signal, temperature: 0 },
    );
    out = extractJson(content);
    if (!out) llmError = "LLM tak mengembalikan JSON SQL yang valid.";
  } catch (e) {
    llmError = e instanceof Error ? e.message : String(e);
  }

  if (!out) {
    return NextResponse.json(
      {
        error: "Agent LLM tak tersedia",
        detail: llmError,
        hint: "Set env LLM_URL/LLM_MODEL ke node yang aktif (llm-node).",
      },
      { status: 503 },
    );
  }

  // Guard: hanya SELECT.
  if (!/^\s*(with|select)\b/i.test(out.sql) || /\b(insert|alter|drop|delete|update|create|truncate)\b/i.test(out.sql)) {
    return NextResponse.json({ ...out, error: "SQL ditolak (hanya SELECT diizinkan)." }, { status: 422 });
  }

  const resp: Record<string, unknown> = { ...out };
  if (run) {
    try {
      const r = await chQuery(out.sql, req.signal);
      resp.columns = r.meta.map((m) => m.name);
      resp.rows = r.data.slice(0, 100);
      resp.rowCount = r.rows;
    } catch (e) {
      resp.runError = e instanceof Error ? e.message : String(e);
    }
  }
  return NextResponse.json(resp);
}
