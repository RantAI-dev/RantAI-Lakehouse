import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";
import { chat } from "@/services/clients/llm";

export const dynamic = "force-dynamic";
export const maxDuration = 60;

/**
 * Catalog Q&A (agentic, RAG-lite): pertanyaan tentang DATA APA YANG ADA →
 * ambil dataset relevan dari katalog NYATA (bronze_meta) → LLM merangkum jawaban.
 * Mengembalikan { answer, datasets }.
 */

type Hit = { slug: string; title: string; description: string; tier: string; total: number };

async function searchCatalog(question: string): Promise<Hit[]> {
  // Ambil seluruh katalog + total, lalu skor cocok kata kunci di sisi server
  // (katalog ~200 baris — murah). Menghindari FTS engine.
  const rows = await chRows<{ slug: string; title: string; description: string; tier: string; total: string }>(
    `SELECT c.slug slug, c.title title, c.description description, c.tier tier,
            toString(coalesce(s.total,0)) total
     FROM (
       SELECT slug,title,description,tier FROM lake.\`bronze_meta.dataset_catalog\`
       UNION ALL SELECT slug,title,description,tier FROM lake.\`bronze_meta_sec.dataset_catalog\`
     ) c LEFT JOIN (
       SELECT slug,total FROM lake.\`bronze_meta.dataset_sync\`
       UNION ALL SELECT slug,total FROM lake.\`bronze_meta_sec.dataset_sync\`
     ) s ON c.slug = s.slug`,
  );
  const terms = question.toLowerCase().split(/[^a-z0-9]+/).filter((t) => t.length > 2);
  const scored = rows.map((r) => {
    const hay = `${r.title} ${r.description} ${r.slug}`.toLowerCase();
    const score = terms.reduce((s, t) => s + (hay.includes(t) ? 1 : 0), 0);
    return { r, score };
  });
  return scored
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, 12)
    .map((x) => ({
      slug: x.r.slug, title: x.r.title, description: x.r.description,
      tier: x.r.tier, total: Number(x.r.total) || 0,
    }));
}

export async function POST(req: Request) {
  let question = "";
  try {
    ({ question } = await req.json());
  } catch {
    return NextResponse.json({ error: "Body harus JSON {question}" }, { status: 400 });
  }
  if (!question?.trim()) return NextResponse.json({ error: "question wajib" }, { status: 400 });

  let hits: Hit[];
  try {
    hits = await searchCatalog(question);
  } catch (e) {
    return NextResponse.json({ error: `Gagal cari katalog: ${e}` }, { status: 503 });
  }

  const context = hits.length
    ? hits.map((h) => `- ${h.title} (${h.tier}, ${h.total} baris) — ${h.description}`).join("\n")
    : "(tidak ada dataset yang cocok)";

  let answer = "";
  try {
    answer = await chat(
      [
        {
          role: "system",
          content:
            "Kamu asisten katalog data lakehouse pariwisata DKI. Jawab RINGKAS dalam Bahasa Indonesia " +
            "berdasarkan HANYA daftar dataset yang diberikan. Sebutkan dataset yang relevan + jumlah barisnya. " +
            "Jangan mengarang dataset di luar daftar.",
        },
        { role: "user", content: `DATASET RELEVAN:\n${context}\n\nPERTANYAAN: ${question}` },
      ],
      { signal: req.signal, temperature: 0.2 },
    );
  } catch (e) {
    // LLM mati → tetap kembalikan hasil retrieval (jujur, tanpa rangkuman).
    answer = `Agent LLM tak tersedia (${e}). Dataset yang cocok:\n${context}`;
  }

  return NextResponse.json({
    answer,
    datasets: hits.map((h) => ({ id: h.slug, title: h.title, tier: h.tier, rows: h.total })),
  });
}
