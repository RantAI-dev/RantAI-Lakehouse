/**
 * Klien LLM (server-side) — OpenAI-compatible. Default MiniMax (endpoint
 * OpenAI-compatible). Bisa diarahkan ke node lain (llm-node dll) via env
 * LLM_URL/LLM_MODEL/LLM_KEY tanpa ubah kode.
 */

const LLM_URL = process.env.LLM_URL ?? "https://api.minimax.io/v1";
const LLM_MODEL = process.env.LLM_MODEL ?? "MiniMax-M3";
// Terima LLM_KEY atau MINIMAX_API_KEY (nama yang dipakai user).
const LLM_KEY = process.env.LLM_KEY || process.env.MINIMAX_API_KEY || "";

export type ChatMessage = { role: "system" | "user" | "assistant"; content: string };

export async function chat(
  messages: ChatMessage[],
  opts: { temperature?: number; signal?: AbortSignal; maxTokens?: number } = {},
): Promise<string> {
  const res = await fetch(`${LLM_URL}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${LLM_KEY}`,
    },
    body: JSON.stringify({
      model: LLM_MODEL,
      messages,
      temperature: opts.temperature ?? 0.1,
      max_tokens: opts.maxTokens ?? 700,
      stream: false,
    }),
    signal: opts.signal,
    cache: "no-store",
  });
  if (!res.ok) {
    throw new Error(`LLM ${res.status}: ${(await res.text()).slice(0, 200)}`);
  }
  const json = await res.json();
  const msg = json?.choices?.[0]?.message ?? {};
  // Model reasoning (MiniMax-M2, dll) bisa menaruh "berpikir" di
  // reasoning_content atau membungkusnya <think>...</think> di content.
  // Kita hanya mau jawaban final.
  let content: string = msg.content ?? "";
  content = content.replace(/<think>[\s\S]*?<\/think>/gi, "").trim();
  if (!content && typeof msg.reasoning_content === "string") {
    content = msg.reasoning_content;
  }
  return content;
}

export function llmConfigured(): { url: string; model: string } {
  return { url: LLM_URL, model: LLM_MODEL };
}

export type ToolCall = { id: string; type: "function"; function: { name: string; arguments: string } };
export type LlmMessage = {
  role: "system" | "user" | "assistant" | "tool";
  content: string | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
  name?: string;
};

/**
 * Chat dengan function-calling. Mengembalikan message asisten mentah (bisa
 * berisi tool_calls). Dipakai loop agentic AI Copilot.
 */
export async function chatWithTools(
  messages: LlmMessage[],
  tools: unknown[],
  opts: { signal?: AbortSignal; maxTokens?: number; temperature?: number } = {},
): Promise<LlmMessage> {
  const res = await fetch(`${LLM_URL}/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${LLM_KEY}` },
    body: JSON.stringify({
      model: LLM_MODEL,
      messages,
      tools,
      tool_choice: "auto",
      temperature: opts.temperature ?? 0.2,
      max_tokens: opts.maxTokens ?? 1200,
      stream: false,
    }),
    signal: opts.signal,
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`LLM ${res.status}: ${(await res.text()).slice(0, 200)}`);
  const json = await res.json();
  const msg = json?.choices?.[0]?.message ?? { role: "assistant", content: "" };
  if (typeof msg.content === "string") {
    msg.content = msg.content.replace(/<think>[\s\S]*?<\/think>/gi, "").trim();
  }
  return msg as LlmMessage;
}
