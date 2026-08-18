"use client";

import * as React from "react";
import { Plus, MessageSquare, Trash2 } from "lucide-react";
import { PageHeader } from "@/components/patterns/page-header";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";

const SUGGESTIONS: Record<"ask" | "build", string[]> = {
  ask: [
    "Total kunjungan wisman per kawasan",
    "Dataset apa saja soal halal?",
    "Tunjukkan silsilah data wisman per negara",
    "Ringkas kualitas data lakehouse",
  ],
  build: [
    "Buatkan board dashboard ringkas soal wisman",
    "Bikin chart tren wisman per bulan dipecah per kawasan",
    "Bangun ulang data lakehouse (Bronze→Silver→Gold)",
    "Cek status build terakhir",
  ],
};

/**
 * Halaman AI Copilot penuh — chat Ask/Build dengan render kaya, PLUS panel
 * RIWAYAT (sesi tersimpan di lakehouse). Berbagi otak (useCopilot) dengan chat
 * dock global, jadi percakapan sinkron di mana pun.
 */
export function CopilotPage() {
  const c = useCopilot();

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="AI Copilot"
        description="Tanya soal data lakehouse, atau bangun Bronze→Silver→Gold & bikin dashboard lewat chat. Menjalankan tool nyata di ClickHouse & Dagster."
      />

      <div className="grid gap-4 lg:grid-cols-[240px_1fr]">
        {/* History */}
        <aside className="flex flex-col gap-2">
          <Button variant="outline" size="sm" onClick={c.newChat} className="justify-start">
            <Plus className="size-4" /> Percakapan baru
          </Button>
          <div className="space-y-0.5 overflow-y-auto rounded-lg border p-1.5">
            <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Riwayat</p>
            {c.sessions.length === 0 ? (
              <p className="px-2 py-2 text-xs text-muted-foreground">Belum ada percakapan.</p>
            ) : (
              c.sessions.map((s) => (
                <div
                  key={s.id}
                  className={cn(
                    "group flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs",
                    s.id === c.sessionId ? "bg-muted" : "hover:bg-muted/60",
                  )}
                >
                  <MessageSquare className="size-3.5 shrink-0 text-muted-foreground" />
                  <button onClick={() => void c.loadSession(s.id)} className="flex-1 truncate text-left" title={s.title}>
                    {s.title}
                  </button>
                  <button
                    onClick={() => void c.removeSession(s.id)}
                    aria-label="Hapus"
                    className="shrink-0 text-muted-foreground opacity-0 hover:text-destructive group-hover:opacity-100"
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
              ))
            )}
          </div>
        </aside>

        {/* Chat */}
        <div className="flex min-h-[62vh] flex-col gap-3">
          <div className="flex-1 overflow-y-auto rounded-lg border p-4">
            {c.messages.length === 0 ? (
              <div className="space-y-4">
                <p className="text-sm text-muted-foreground">
                  {c.mode === "ask"
                    ? "Tanya angka, dataset, silsilah, atau kualitas. Copilot query ClickHouse & jelajah katalog."
                    : "Minta bikin chart/dashboard atau bangun data. Copilot menjalankan tool nyata & menampilkan hasilnya."}
                </p>
                <div className="grid gap-2 sm:grid-cols-2">
                  {SUGGESTIONS[c.mode].map((s) => (
                    <button
                      key={s} onClick={() => void c.send(s)} disabled={c.busy}
                      className="rounded-lg border px-3 py-2 text-left text-xs hover:bg-muted disabled:opacity-50"
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
          <ChatComposer mode={c.mode} setMode={c.setMode} onSend={c.send} busy={c.busy} placeholder="Tanya apa saja soal data lakehouse…" />
        </div>
      </div>
    </div>
  );
}
