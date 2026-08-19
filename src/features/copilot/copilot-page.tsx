"use client";

import * as React from "react";
import { Plus, Sparkles, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";

const SUGGESTIONS: Record<"ask" | "build", string[]> = {
  ask: [
    "Total kunjungan wisman per kawasan",
    "Dataset apa saja soal halal?",
    "Silsilah data wisman per negara",
    "Ringkas kualitas data lakehouse",
  ],
  build: [
    "Buatkan board dashboard ringkas soal wisman",
    "Bikin chart tren wisman per bulan per kawasan",
    "Bangun ulang lakehouse (Bronze→Silver→Gold)",
    "Cek status build terakhir",
  ],
};

/**
 * Halaman AI Copilot — tampilan chat ala RantAI-Agents: avatar per pesan,
 * welcome ketengah dengan pill saran, composer lembut. RIWAYAT dipindah jadi
 * bar horizontal di bawah navbar (bukan sidebar kiri). Otak & riwayat dibagi
 * dengan chat dock global lewat useCopilot.
 */
export function CopilotPage() {
  const c = useCopilot();

  return (
    <div className="flex flex-col gap-3">
      {/* Riwayat — bar horizontal di bawah navbar */}
      <div className="flex items-center gap-1.5 overflow-x-auto border-b border-border pb-2">
        <Button size="sm" variant="outline" className="shrink-0" onClick={c.newChat}>
          <Plus className="size-4" /> Baru
        </Button>
        {c.sessions.map((s) => {
          const active = s.id === c.sessionId;
          return (
            <div
              key={s.id}
              className={cn(
                "group inline-flex shrink-0 items-center gap-1 rounded-full border px-3 py-1 text-xs",
                active ? "border-primary/30 bg-primary/10 text-primary" : "border-border text-muted-foreground hover:bg-muted",
              )}
            >
              <button onClick={() => void c.loadSession(s.id)} className="max-w-[160px] truncate" title={s.title}>
                {s.title}
              </button>
              <button
                onClick={() => void c.removeSession(s.id)}
                aria-label="Hapus percakapan"
                className="opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100"
              >
                <X className="size-3" />
              </button>
            </div>
          );
        })}
        {c.sessions.length === 0 ? (
          <span className="px-1 text-xs text-muted-foreground">Belum ada percakapan.</span>
        ) : null}
      </div>

      {/* Chat */}
      <div className="mx-auto flex w-full max-w-3xl flex-col">
        <div className="min-h-[56vh] overflow-y-auto pr-0.5">
          {c.messages.length === 0 ? (
            <div className="flex flex-col items-center justify-center px-4 py-16 text-center">
              <div className="mb-4 grid size-12 place-items-center rounded-2xl border border-violet-500/20 bg-gradient-to-br from-violet-500/15 to-purple-600/15 text-violet-600 dark:text-violet-400">
                <Sparkles className="size-6" />
              </div>
              <h2 className="text-xl font-semibold text-foreground">Halo 👋 Ada yang bisa dibantu?</h2>
              <p className="mt-1.5 max-w-md text-sm text-muted-foreground">
                {c.mode === "ask"
                  ? "Tanya angka, dataset, silsilah, atau kualitas. Copilot query ClickHouse & jelajah katalog."
                  : "Minta bikin chart/dashboard atau bangun data. Copilot menjalankan tool nyata & menampilkan hasilnya."}
              </p>
              <div className="mt-5 flex flex-wrap justify-center gap-2">
                {SUGGESTIONS[c.mode].map((s) => (
                  <button
                    key={s}
                    onClick={() => void c.send(s)}
                    disabled={c.busy}
                    className="inline-flex items-center gap-2 rounded-full border border-border bg-background px-4 py-2 text-sm font-medium text-foreground/80 transition-all hover:border-primary/40 hover:bg-muted/50 hover:text-foreground disabled:opacity-50"
                  >
                    {s}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <ChatMessages messages={c.messages} busy={c.busy} error={c.error} />
          )}
        </div>

        <div className="pt-3">
          <ChatComposer
            mode={c.mode} setMode={c.setMode} onSend={c.send} busy={c.busy}
            enabledTools={c.enabledTools} toggleTool={c.toggleTool}
            placeholder="Tanya apa saja soal data lakehouse…"
          />
        </div>
      </div>
    </div>
  );
}
