"use client";

import * as React from "react";
import { PageHeader } from "@/components/patterns/page-header";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { SectionCard } from "@/components/patterns/section-card";

type Msg = { role: "user" | "assistant"; content: string; tools?: { tool: string; ok: boolean }[] };

const SUGGESTIONS = [
  "Berapa total kunjungan wisman per kawasan?",
  "Dataset apa saja soal halal?",
  "Tunjukkan silsilah data wisman per negara",
  "Bangun ulang data lakehouse (Bronze→Silver→Gold)",
];

/**
 * AI Copilot lakehouse — fitur utama. Tanya apa saja soal data DAN operasikan
 * lakehouse (bangun Bronze/Silver/Gold) lewat chat. Agen tool-calling di
 * /api/ai/chat mengeksekusi tool NYATA (ClickHouse/Dagster).
 */
export function CopilotPage() {
  const [messages, setMessages] = React.useState<Msg[]>([]);
  const [input, setInput] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const endRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, busy]);

  async function send(text: string) {
    const q = text.trim();
    if (!q || busy) return;
    setError(null);
    const next: Msg[] = [...messages, { role: "user", content: q }];
    setMessages(next);
    setInput("");
    setBusy(true);
    try {
      const res = await fetch("/api/ai/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ messages: next.map((m) => ({ role: m.role, content: m.content })) }),
      });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.hint ?? json?.detail ?? json?.error ?? "Copilot gagal");
      setMessages((m) => [
        ...m,
        { role: "assistant", content: json.answer || "(tak ada jawaban)", tools: json.toolTrace ?? [] },
      ]);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="AI Copilot"
        description="Tanya apa saja soal data lakehouse, atau operasikan lakehouse (bangun Bronze→Silver→Gold) lewat chat. Agen menjalankan tool nyata di ClickHouse & Dagster."
      />

      <div className="grid gap-4 xl:grid-cols-[1fr_300px]">
        <div className="flex min-h-[60vh] flex-col gap-3">
          <div className="flex-1 space-y-3 overflow-y-auto rounded-lg border p-4">
            {messages.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                Mulai bertanya, atau pilih contoh di kanan. Copilot bisa query data, jelajah
                katalog, baca silsilah/kualitas, dan menjalankan build lakehouse.
              </p>
            ) : null}
            {messages.map((m, i) => (
              <div key={i} className={m.role === "user" ? "text-right" : ""}>
                <div
                  className={
                    "inline-block max-w-[85%] whitespace-pre-wrap rounded-lg px-3 py-2 text-sm " +
                    (m.role === "user" ? "bg-primary text-primary-foreground" : "bg-muted")
                  }
                >
                  {m.content}
                </div>
                {m.tools && m.tools.length ? (
                  <div className="mt-1 flex flex-wrap gap-1 text-[11px] text-muted-foreground">
                    {m.tools.map((t, j) => (
                      <span key={j} className="rounded border px-1.5 py-0.5 font-mono">
                        {t.ok ? "✓" : "✗"} {t.tool}
                      </span>
                    ))}
                  </div>
                ) : null}
              </div>
            ))}
            {busy ? <p className="text-sm text-muted-foreground">Copilot bekerja… (memanggil tool)</p> : null}
            {error ? <p className="text-sm text-destructive">{error}</p> : null}
            <div ref={endRef} />
          </div>

          <div className="flex items-end gap-2">
            <Textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send(input);
                }
              }}
              rows={2}
              placeholder="Tanya soal data, atau minta bangun Bronze/Silver/Gold…"
              aria-label="Pesan ke AI Copilot"
            />
            <Button onClick={() => void send(input)} disabled={busy || !input.trim()}>
              Kirim
            </Button>
          </div>
        </div>

        <div className="space-y-3">
          <SectionCard title="Contoh">
            <div className="space-y-2">
              {SUGGESTIONS.map((s) => (
                <button
                  key={s}
                  onClick={() => void send(s)}
                  disabled={busy}
                  className="block w-full rounded-md border px-3 py-2 text-left text-xs hover:bg-muted disabled:opacity-50"
                >
                  {s}
                </button>
              ))}
            </div>
          </SectionCard>
          <SectionCard title="Bisa apa">
            <ul className="list-disc space-y-1 pl-4 text-xs text-muted-foreground">
              <li>Query data (SQL otomatis ke ClickHouse)</li>
              <li>Jelajah katalog & skema dataset</li>
              <li>Baca silsilah (lineage) & kualitas data</li>
              <li>Bangun/segarkan Bronze→Silver→Gold (Dagster)</li>
            </ul>
          </SectionCard>
        </div>
      </div>
    </div>
  );
}
