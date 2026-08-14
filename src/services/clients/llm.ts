/**
 * Klien LLM (server-side) — OpenAI-compatible. Default menunjuk ke llm-node
 * kita (llama-swap, endpoint /v1). Endpoint & model dikonfigurasi via env
 * supaya bisa diarahkan ke node lain tanpa ubah kode.
 */

const LLM_URL = process.env.LLM_URL ?? "http://192.168.18.197:8080/v1";
const LLM_MODEL = process.env.LLM_MODEL ?? "qwen2.5-coder";
const LLM_KEY = process.env.LLM_KEY ?? "sk-node-KOyNu45PbpDUUgG3qPNw0REoql6N7P5f";

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
  return json?.choices?.[0]?.message?.content ?? "";
}

export function llmConfigured(): { url: string; model: string } {
  return { url: LLM_URL, model: LLM_MODEL };
}
