import { NextResponse } from "next/server";
import { chatWithTools, type LlmMessage } from "@/services/clients/llm";
import { TOOL_SCHEMAS, runTool } from "@/services/clients/ai-tools";

export const dynamic = "force-dynamic";
export const maxDuration = 120;

/**
 * AI Copilot lakehouse — loop agentic tool-calling.
 * User bisa TANYA data apa saja (agen panggil run_sql/list_datasets/lineage/…)
 * dan MENGOPERASIKAN lakehouse (trigger_lakehouse_build → bangun Bronze/Silver/
 * Gold). LLM (MiniMax) memutuskan tool; server mengeksekusi & mengumpankan
 * balik hasilnya sampai agen memberi jawaban final.
 */

const SYSTEM = `Kamu AI Copilot untuk lakehouse pariwisata DKI Jakarta (RantAI Lakehouse).
Kamu bisa MENJAWAB pertanyaan data dan MENGOPERASIKAN lakehouse lewat tool.

Panduan:
- Untuk pertanyaan angka/data: pakai run_sql (SELECT ClickHouse). Cari tabel dulu
  via list_datasets/describe_dataset kalau belum tahu skema. Utamakan serving.mart_*.
- Untuk "ada data apa / soal X": list_datasets atau describe_dataset.
- Untuk silsilah data: get_lineage. Untuk kualitas: get_quality.
- Untuk "bangun/segarkan Bronze/Silver/Gold" atau "refresh data": trigger_lakehouse_build,
  lalu beri tahu user cara cek statusnya (get_build_status).
- Jawab RINGKAS dalam Bahasa Indonesia, berdasarkan HASIL TOOL yang nyata.
  JANGAN mengarang angka atau tabel. Kalau tool error, katakan apa adanya.`;

const MAX_ITER = 6;

export async function POST(req: Request) {
  let history: LlmMessage[] = [];
  try {
    const body = await req.json();
    const incoming = Array.isArray(body.messages) ? body.messages : [];
    history = incoming
      .filter((m: { role: string; content: string }) => m.role === "user" || m.role === "assistant")
      .map((m: { role: string; content: string }) => ({ role: m.role, content: m.content }));
  } catch {
    return NextResponse.json({ error: "Body harus JSON {messages}" }, { status: 400 });
  }
  if (!history.length) return NextResponse.json({ error: "messages kosong" }, { status: 400 });

  const messages: LlmMessage[] = [{ role: "system", content: SYSTEM }, ...history];
  const toolTrace: { tool: string; args: unknown; ok: boolean }[] = [];

  try {
    for (let iter = 0; iter < MAX_ITER; iter++) {
      const msg = await chatWithTools(messages, TOOL_SCHEMAS, { signal: req.signal });
      messages.push(msg);

      const calls = msg.tool_calls ?? [];
      if (!calls.length) {
        // Jawaban final.
        return NextResponse.json({ answer: msg.content ?? "", toolTrace });
      }

      // Eksekusi tiap tool call → umpan balik ke model.
      for (const call of calls) {
        let args: Record<string, unknown> = {};
        try {
          args = JSON.parse(call.function.arguments || "{}");
        } catch {
          /* argumen bukan JSON */
        }
        const result = await runTool(call.function.name, args);
        const ok = !(result && typeof result === "object" && "error" in (result as object));
        toolTrace.push({ tool: call.function.name, args, ok });
        messages.push({
          role: "tool",
          tool_call_id: call.id,
          name: call.function.name,
          content: JSON.stringify(result).slice(0, 8000),
        });
      }
    }
    // Kehabisan iterasi — minta jawaban akhir tanpa tool.
    const final = await chatWithTools([...messages, { role: "user", content: "Beri jawaban final ringkas dari hasil di atas." }], [], { signal: req.signal });
    return NextResponse.json({ answer: final.content ?? "", toolTrace, note: "batas iterasi tool tercapai" });
  } catch (e) {
    return NextResponse.json(
      { error: "AI Copilot tak tersedia", detail: String(e), hint: "Set LLM_KEY (MiniMax) di .env.local." },
      { status: 503 },
    );
  }
}
