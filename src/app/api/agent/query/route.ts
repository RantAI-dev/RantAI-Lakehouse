import { NextResponse } from "next/server";
import { chat } from "@/services/clients/llm";
import { chQuery, schemaContext, isReadOnlySql, extractSqlJson } from "@/services/clients/agent-tools";

export const dynamic = "force-dynamic";
export const maxDuration = 90;

/**
 * Agen SQL SELF-CORRECTING untuk Query Studio.
 * Loop agentic: pertanyaan NL → generate SQL (grounded skema NYATA) → JALANKAN
 * di ClickHouse → kalau error, agen BACA errornya lalu perbaiki (retry) →
 * setelah sukses, jelaskan hasil dalam bahasa natural.
 * Mengembalikan { sql, columns, rows, answer, steps }.
 */

const GEN_SYSTEM = `Kamu ahli SQL ClickHouse untuk lakehouse pariwisata DKI Jakarta.
Ubah pertanyaan jadi SATU query SELECT ClickHouse valid.
Aturan:
- HANYA gunakan tabel/kolom dari skema yang diberikan. Jangan mengarang.
- Utamakan serving.mart_* untuk agregasi.
- SELECT saja (baca). LIMIT wajar (<=100).
- Balas HANYA JSON: {"sql":"...","explanation":"...","assumptions":["..."]}`;

const FIX_SYSTEM = `Query ClickHouse gagal. Perbaiki SQL berdasarkan pesan error
dan skema. Balas HANYA JSON: {"sql":"...","explanation":"...","assumptions":[]}`;

const MAX_FIX = 2;

export async function POST(req: Request) {
  let question = "";
  try {
    ({ question } = await req.json());
  } catch {
    return NextResponse.json({ error: "Body harus JSON {question}" }, { status: 400 });
  }
  if (!question?.trim()) return NextResponse.json({ error: "question wajib" }, { status: 400 });

  const steps: { step: string; detail: string }[] = [];
  let schema: string;
  try {
    schema = await schemaContext();
    steps.push({ step: "skema", detail: "Membaca skema mart Gold + Silver dari lakehouse." });
  } catch (e) {
    return NextResponse.json({ error: `Gagal baca skema: ${e}` }, { status: 503 });
  }

  // 1) Generate SQL awal.
  let gen;
  try {
    const out = await chat(
      [
        { role: "system", content: GEN_SYSTEM },
        { role: "user", content: `SKEMA:\n${schema}\n\nPERTANYAAN: ${question}` },
      ],
      { signal: req.signal, temperature: 0 },
    );
    gen = extractSqlJson(out);
  } catch (e) {
    return NextResponse.json(
      { error: "Agent LLM tak tersedia", detail: String(e), hint: "Set LLM_KEY (MiniMax) di .env.local." },
      { status: 503 },
    );
  }
  if (!gen) return NextResponse.json({ error: "LLM tak menghasilkan SQL valid" }, { status: 422 });
  steps.push({ step: "generate", detail: gen.sql });

  // 2) Jalankan; kalau error, agen perbaiki (loop self-correcting).
  let sql = gen.sql;
  let result: Awaited<ReturnType<typeof chQuery>> | null = null;
  let lastError = "";
  for (let attempt = 0; attempt <= MAX_FIX; attempt++) {
    if (!isReadOnlySql(sql)) {
      lastError = "SQL ditolak (hanya SELECT diizinkan).";
      break;
    }
    try {
      result = await chQuery(sql, req.signal);
      steps.push({ step: attempt === 0 ? "jalankan" : `koreksi-${attempt}`, detail: `OK, ${result.rows} baris` });
      break;
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e);
      steps.push({ step: `error-${attempt}`, detail: lastError.slice(0, 160) });
      if (attempt === MAX_FIX) break;
      // Agen membaca error → memperbaiki SQL.
      try {
        const fix = await chat(
          [
            { role: "system", content: FIX_SYSTEM },
            {
              role: "user",
              content: `SKEMA:\n${schema}\n\nPERTANYAAN: ${question}\n\nSQL GAGAL:\n${sql}\n\nERROR:\n${lastError}`,
            },
          ],
          { signal: req.signal, temperature: 0 },
        );
        const fixed = extractSqlJson(fix);
        if (!fixed) break;
        sql = fixed.sql;
        steps.push({ step: `perbaiki-${attempt + 1}`, detail: sql });
      } catch {
        break;
      }
    }
  }

  if (!result) {
    return NextResponse.json(
      { sql, error: "Query gagal setelah koreksi", detail: lastError, steps, explanation: gen.explanation },
      { status: 422 },
    );
  }

  const columns = result.meta.map((m) => m.name);
  const rows = result.data.slice(0, 100);

  // 3) Jelaskan hasil dalam bahasa natural (ringkas, dari data nyata).
  let answer = gen.explanation;
  try {
    const preview = JSON.stringify(rows.slice(0, 15));
    answer = await chat(
      [
        {
          role: "system",
          content:
            "Ringkas jawaban dari hasil query dalam Bahasa Indonesia, 1-3 kalimat, " +
            "berdasarkan HANYA data yang diberikan. Sebut angka kuncinya. Jangan mengarang.",
        },
        { role: "user", content: `PERTANYAAN: ${question}\nKOLOM: ${columns.join(", ")}\nDATA(≤15): ${preview}` },
      ],
      { signal: req.signal, temperature: 0.2 },
    );
  } catch {
    /* biarkan pakai explanation generate */
  }

  return NextResponse.json({
    question,
    sql,
    columns,
    rows,
    rowCount: result.rows,
    answer,
    assumptions: gen.assumptions,
    steps,
  });
}
