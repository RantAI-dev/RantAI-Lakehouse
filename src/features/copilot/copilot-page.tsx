"use client";

import * as React from "react";
import { PageHeader } from "@/components/patterns/page-header";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { SectionCard } from "@/components/patterns/section-card";
import { cn } from "@/lib/utils";
import { MiniMarkdown } from "./mini-markdown";
import { ToolStepCard, type ToolStep } from "./tool-step";
import { BuildTree } from "./build-tree";

type Mode = "ask" | "build";
type Msg = {
  role: "user" | "assistant";
  content: string;
  tools?: ToolStep[];
  buildRunId?: string;
};

const SUGGESTIONS: Record<Mode, string[]> = {
  ask: [
    "Total kunjungan wisman per kawasan",
    "Dataset apa saja soal halal?",
    "Tunjukkan silsilah data wisman per negara",
    "Ringkas kualitas data lakehouse",
  ],
  build: [
    "Bangun ulang data lakehouse (Bronze→Silver→Gold)",
    "Segarkan mart kuliner",
    "Cek status build terakhir",
  ],
};

const PLACEHOLDER: Record<Mode, string> = {
  ask: "Tanya soal data — angka, dataset, silsilah, kualitas…",
  build: "Minta bangun/segarkan Bronze→Silver→Gold…",
};

/**
 * AI Copilot lakehouse — fitur utama. Dua mode:
 *  · Ask   → tanya & analisis data (read-only).
 *  · Build → operasikan lakehouse (bangun Bronze/Silver/Gold) lewat chat,
 *            dengan pohon pipeline live.
 * Backend /api/ai/chat menjalankan tool NYATA (ClickHouse/Dagster) dan
 * mengembalikan langkah + hasilnya, dirender kaya di sini.
 */
export function CopilotPage() {
  const [mode, setMode] = React.useState<Mode>("ask");
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
        body: JSON.stringify({ mode, messages: next.map((m) => ({ role: m.role, content: m.content })) }),
      });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.hint ?? json?.detail ?? json?.error ?? "Copilot gagal");
      setMessages((m) => [
        ...m,
        {
          role: "assistant",
          content: json.answer || "(tak ada jawaban)",
          tools: json.toolTrace ?? [],
          buildRunId: json.buildRunId,
        },
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
        description="Tanya apa saja soal data lakehouse, atau bangun Bronze→Silver→Gold lewat chat. Agen menjalankan tool nyata di ClickHouse & Dagster."
      />

      {/* Toggle mode */}
      <div className="flex items-center gap-3">
        <div className="inline-flex rounded-lg border border-border bg-muted/40 p-1" role="tablist" aria-label="Mode Copilot">
          {(["ask", "build"] as Mode[]).map((m) => (
            <button
              key={m}
              type="button"
              role="tab"
              aria-selected={mode === m}
              onClick={() => setMode(m)}
              className={cn(
                "rounded-md px-4 py-1.5 text-sm transition-colors",
                mode === m ? "bg-background font-medium text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {m === "ask" ? "Ask" : "Build"}
            </button>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          {mode === "ask" ? "Read-only — menjawab & menganalisis data." : "Mengoperasikan lakehouse — bisa membangun/menyegarkan data."}
        </p>
      </div>

      <div className="grid gap-4 xl:grid-cols-[1fr_300px]">
        <div className="flex min-h-[60vh] flex-col gap-3">
          <div className="flex-1 space-y-4 overflow-y-auto rounded-lg border p-4">
            {messages.length === 0 ? (
              <div className="text-sm text-muted-foreground">
                {mode === "ask"
                  ? "Tanya angka, dataset, silsilah, atau kualitas. Copilot query ClickHouse & jelajah katalog."
                  : "Minta bangun/segarkan data. Copilot menjalankan pipeline Dagster dan menampilkan progresnya live."}
              </div>
            ) : null}

            {messages.map((m, i) =>
              m.role === "user" ? (
                <div key={i} className="text-right">
                  <div className="inline-block max-w-[85%] whitespace-pre-wrap rounded-lg bg-primary px-3 py-2 text-sm text-primary-foreground">
                    {m.content}
                  </div>
                </div>
              ) : (
                <div key={i} className="space-y-2">
                  {m.tools && m.tools.length ? (
                    <div className="space-y-1.5">
                      {m.tools.map((t, j) => (
                        <ToolStepCard key={j} step={t} />
                      ))}
                    </div>
                  ) : null}
                  {m.buildRunId ? <BuildTree runId={m.buildRunId} /> : null}
                  <div className="rounded-lg bg-muted px-3 py-2">
                    <MiniMarkdown text={m.content} />
                  </div>
                </div>
              ),
            )}

            {busy ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
                Copilot bekerja… (memanggil tool)
              </div>
            ) : null}
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
              placeholder={PLACEHOLDER[mode]}
              aria-label="Pesan ke AI Copilot"
            />
            <Button onClick={() => void send(input)} disabled={busy || !input.trim()}>
              Kirim
            </Button>
          </div>
        </div>

        <div className="space-y-3">
          <SectionCard title={mode === "ask" ? "Contoh · Ask" : "Contoh · Build"}>
            <div className="space-y-2">
              {SUGGESTIONS[mode].map((s) => (
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
              {mode === "ask" ? (
                <>
                  <li>Query data (SQL otomatis ke ClickHouse)</li>
                  <li>Jelajah katalog & skema dataset</li>
                  <li>Baca silsilah (lineage) & kualitas data</li>
                </>
              ) : (
                <>
                  <li>Bangun/segarkan Bronze→Silver→Gold (Dagster)</li>
                  <li>Pantau pipeline live (per-step)</li>
                  <li>Cek status build terakhir</li>
                </>
              )}
            </ul>
          </SectionCard>
        </div>
      </div>
    </div>
  );
}
