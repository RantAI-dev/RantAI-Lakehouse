import { NextResponse } from "next/server";
import { chatWithTools, type LlmMessage, type ToolCall } from "@/services/clients/llm";
import { TOOL_SCHEMAS, runTool } from "@/services/clients/ai-tools";
import { schemaContext } from "@/services/clients/agent-tools";

/**
 * MiniMax-M2 kadang memancarkan tool call sebagai XML di content, bukan field
 * tool_calls standar OpenAI:
 *   <minimax:tool_call><invoke name="run_sql">
 *     <parameter name="sql">SELECT ...</parameter></invoke></minimax:tool_call>
 * Parse itu jadi ToolCall standar supaya loop tetap jalan.
 */
function parseMinimaxToolCalls(content: string): ToolCall[] {
  const calls: ToolCall[] = [];
  const invokeRe = /<invoke\s+name="([^"]+)"\s*>([\s\S]*?)<\/invoke>/g;
  let m: RegExpExecArray | null;
  let idx = 0;
  while ((m = invokeRe.exec(content))) {
    const name = m[1];
    const args: Record<string, string> = {};
    const paramRe = /<parameter\s+name="([^"]+)"\s*>([\s\S]*?)<\/parameter>/g;
    let p: RegExpExecArray | null;
    while ((p = paramRe.exec(m[2]))) args[p[1]] = p[2].trim();
    calls.push({ id: `mmx-${idx++}`, type: "function", function: { name, arguments: JSON.stringify(args) } });
  }
  return calls;
}

function stripToolXml(s: string): string {
  return s.replace(/<minimax:tool_call>[\s\S]*?<\/minimax:tool_call>/gi, "").replace(/<\/?think>/gi, "").trim();
}

export const dynamic = "force-dynamic";
export const maxDuration = 120;

/**
 * AI Copilot lakehouse — loop agentic tool-calling.
 * User bisa TANYA data apa saja (agen panggil run_sql/list_datasets/lineage/…)
 * dan MENGOPERASIKAN lakehouse (trigger_lakehouse_build → bangun Bronze/Silver/
 * Gold). LLM (MiniMax) memutuskan tool; server mengeksekusi & mengumpankan
 * balik hasilnya sampai agen memberi jawaban final.
 */

const SYSTEM_BASE = `Kamu AI Copilot untuk lakehouse pariwisata DKI Jakarta (RantAI Lakehouse).

Panduan umum:
- Untuk pertanyaan angka/data: pakai run_sql (SELECT ClickHouse). Cari tabel dulu
  via list_datasets/describe_dataset kalau belum tahu skema. Utamakan serving.mart_*.
- Untuk "ada data apa / soal X": list_datasets atau describe_dataset.
- Untuk silsilah data: get_lineage. Untuk kualitas: get_quality.
- Answer CONCISELY in English (Markdown allowed: tables, bold, lists),
  berdasarkan HASIL TOOL yang nyata. JANGAN mengarang angka atau tabel.
  Kalau tool error, katakan apa adanya.`;

const SYSTEM_ASK = `${SYSTEM_BASE}

MODE: ASK (read-only). Kamu HANYA menjawab & menganalisis data — tidak
mengubah/membangun apa pun (termasuk TIDAK membuat/menghapus chart). Kamu boleh
melihat dashboard (describe_mart/list_charts). Kalau user minta membangun data
atau membuat chart, sarankan pindah ke mode Build.`;

const SYSTEM_BUILD = `${SYSTEM_BASE}

MODE: BUILD. Selain menjawab, kamu bisa MENGOPERASIKAN lakehouse:
- Untuk "bangun/segarkan Bronze/Silver/Gold" atau "refresh data":
  JELASKAN dulu rencananya singkat, lalu panggil trigger_lakehouse_build.
- Setelah trigger, beri tahu user pipeline berjalan (statusnya tampil live).
- Untuk "bikin/tambah chart/dashboard soal X" (BI lewat chat):
  1) panggil describe_mart (tanpa arg → lihat mart Gold; dengan mart → lihat
     kolom valid, terbagi dimensi vs measure),
  2) lalu create_chart dengan kolom yang BENAR-BENAR ADA. Kamu TIDAK menulis
     SQL — server menyusunnya. Pilih kind yang cocok (hbar untuk peringkat,
     line/area untuk tren waktu, pie untuk komposisi, stacked untuk ≥2 measure).
     Pakai parameter breakdown (dimensi ke-2) untuk multi-seri, mis. tren per
     kawasan (line + breakdown=kawasan) atau grouped bar per kategori.
  3) konfirmasi chart dibuat & sebut muncul di halaman Dashboards.
  Pakai list_charts/update_chart/delete_chart untuk mengelola kartu tersimpan.
- Untuk "buatkan/sarankan dashboard soal X" tanpa detail: panggil suggest_dashboard
  (dapat katalog semua mart+kolom), usulkan 3-5 kartu, lalu buat via create_chart.
- Untuk mengelompokkan: create_board dulu, lalu create_chart dengan board=<id>.
- Untuk mengubah kartu: update_chart (kirim semua field dengan nilai baru).`;

const MAX_ITER = 8;

type ToolStep = { tool: string; args: unknown; ok: boolean; result: unknown };

export async function POST(req: Request) {
  let history: LlmMessage[] = [];
  let mode: "ask" | "build" = "ask";
  let allow: string[] | null = null;
  try {
    const body = await req.json();
    if (body.mode === "build") mode = "build";
    if (Array.isArray(body.tools) && body.tools.length) allow = body.tools.map(String);
    const incoming = Array.isArray(body.messages) ? body.messages : [];
    history = incoming
      .filter((m: { role: string; content: string }) => m.role === "user" || m.role === "assistant")
      .map((m: { role: string; content: string }) => ({ role: m.role, content: m.content }));
  } catch {
    return NextResponse.json({ error: "Body harus JSON {messages}" }, { status: 400 });
  }
  if (!history.length) return NextResponse.json({ error: "messages kosong" }, { status: 400 });

  // Grounding skema mart NYATA di awal supaya agen langsung pakai serving.mart_*
  // (bukan menebak tabel silver mentah).
  let schema = "";
  try {
    schema = await schemaContext();
  } catch {
    /* lanjut tanpa skema */
  }
  const base = mode === "build" ? SYSTEM_BUILD : SYSTEM_ASK;
  const sys = schema ? `${base}\n\nSKEMA TERSEDIA:\n${schema}` : base;

  // Ask = read-only: sembunyikan tool yang mengubah lakehouse / dashboard.
  const WRITE_TOOLS = new Set([
    "trigger_lakehouse_build", "create_chart", "update_chart", "delete_chart", "create_board",
  ]);
  const allowSet = allow ? new Set(allow) : null;
  const tools = TOOL_SCHEMAS.filter((t) => {
    const name = t.function.name;
    if (mode !== "build" && WRITE_TOOLS.has(name)) return false; // Ask = read-only
    if (allowSet && !allowSet.has(name)) return false; // pilihan user (menu Tools)
    return true;
  });

  const messages: LlmMessage[] = [{ role: "system", content: sys }, ...history];
  const toolTrace: ToolStep[] = [];
  let buildRunId: string | undefined;
  let chartCreated = false;

  try {
    for (let iter = 0; iter < MAX_ITER; iter++) {
      const msg = await chatWithTools(messages, tools, { signal: req.signal });
      messages.push(msg);

      // Tool call bisa dari field standar ATAU format XML MiniMax di content.
      const calls: ToolCall[] = [
        ...(msg.tool_calls ?? []),
        ...(typeof msg.content === "string" ? parseMinimaxToolCalls(msg.content) : []),
      ];
      if (!calls.length) {
        // Jawaban final (buang sisa XML tool bila ada).
        return NextResponse.json({ answer: stripToolXml(msg.content ?? ""), toolTrace, buildRunId, chartCreated });
      }

      // Eksekusi tiap tool call → umpan balik ke model.
      // Format respons berbeda: call standar (id bukan mmx-*) → role "tool"
      // dengan tool_call_id; call XML MiniMax → role "user" (aman lintas format).
      const xmlFeedback: string[] = [];
      for (const call of calls) {
        let args: Record<string, unknown> = {};
        try {
          args = JSON.parse(call.function.arguments || "{}");
        } catch {
          /* argumen bukan JSON */
        }
        const result = await runTool(call.function.name, args);
        const ok = !(result && typeof result === "object" && "error" in (result as object));
        toolTrace.push({ tool: call.function.name, args, ok, result });
        // Tangkap runId build supaya UI bisa render pohon pipeline live.
        if (result && typeof result === "object" && "runId" in (result as object)) {
          const rid = (result as { runId?: unknown }).runId;
          if (typeof rid === "string") buildRunId = rid;
        }
        // Tandai bila chart baru dibuat → UI tawarkan buka Dashboards.
        if (call.function.name === "create_chart" && ok) chartCreated = true;
        const payload = JSON.stringify(result).slice(0, 8000);
        if (call.id.startsWith("mmx-")) {
          xmlFeedback.push(`Hasil ${call.function.name}: ${payload}`);
        } else {
          messages.push({ role: "tool", tool_call_id: call.id, name: call.function.name, content: payload });
        }
      }
      if (xmlFeedback.length) {
        messages.push({
          role: "user",
          content: `HASIL TOOL:\n${xmlFeedback.join("\n")}\n\nLanjutkan: pakai hasil ini untuk menjawab, atau panggil tool lain bila perlu.`,
        });
      }
    }
    // Kehabisan iterasi — minta jawaban akhir tanpa tool.
    const final = await chatWithTools([...messages, { role: "user", content: "Beri jawaban final ringkas dari hasil di atas." }], [], { signal: req.signal });
    return NextResponse.json({ answer: final.content ?? "", toolTrace, buildRunId, chartCreated, note: "batas iterasi tool tercapai" });
  } catch (e) {
    return NextResponse.json(
      { error: "AI Copilot tak tersedia", detail: String(e), hint: "Set LLM_KEY (MiniMax) di .env.local." },
      { status: 503 },
    );
  }
}
