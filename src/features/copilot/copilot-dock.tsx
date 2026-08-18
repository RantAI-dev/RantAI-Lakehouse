"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { Sparkles, Plus, ChevronDown } from "lucide-react";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";

const QUICK: Record<string, string[]> = {
  ask: ["Total kunjungan wisman per kawasan", "Ringkas kualitas data lakehouse"],
  build: ["Bikin chart wisman per kawasan", "Buatkan board dashboard soal wisman"],
};

/**
 * Chat dock GLOBAL — selalu tersedia di bawah kanan setiap halaman (gaya Google
 * Cloud Assist). Collapsed = pill; expanded = panel chat penuh (Ask/Build, tool
 * rendering, pohon build). Berbagi otak & riwayat dengan halaman /copilot lewat
 * useCopilot. Disembunyikan di /copilot (sudah full-page di sana).
 */
export function CopilotDock() {
  const pathname = usePathname();
  const [open, setOpen] = React.useState(false);
  const c = useCopilot();

  // Ingat status buka/tutup antar halaman.
  React.useEffect(() => {
    setOpen(typeof window !== "undefined" && window.localStorage.getItem("copilot-dock-open") === "1");
  }, []);
  const toggle = (o: boolean) => {
    setOpen(o);
    try { window.localStorage.setItem("copilot-dock-open", o ? "1" : "0"); } catch { /* ignore */ }
  };

  // Jangan tampil di halaman Copilot penuh.
  if (pathname?.startsWith("/copilot")) return null;

  return (
    <div className="fixed bottom-4 right-4 z-50 print:hidden">
      {open ? (
        <div className="flex h-[70vh] max-h-[600px] w-[min(92vw,400px)] flex-col overflow-hidden rounded-2xl border border-border bg-card shadow-2xl">
          {/* Header */}
          <div className="flex items-center gap-2 border-b border-border px-3 py-2">
            <span className="grid size-6 place-items-center rounded-md bg-primary/10 text-primary">
              <Sparkles className="size-3.5" />
            </span>
            <span className="text-sm font-semibold">AI Copilot</span>
            <div className="ml-auto flex items-center gap-0.5">
              <button
                type="button" onClick={c.newChat} aria-label="Percakapan baru"
                className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <Plus className="size-4" />
              </button>
              <button
                type="button" onClick={() => toggle(false)} aria-label="Tutup"
                className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <ChevronDown className="size-4" />
              </button>
            </div>
          </div>

          {/* Messages */}
          <div className="flex-1 overflow-y-auto p-3">
            {c.messages.length === 0 ? (
              <div className="space-y-3 pt-2">
                <p className="text-sm text-muted-foreground">
                  Tanya soal data, atau ke mode <span className="font-medium text-foreground">Build</span> untuk bikin chart/dashboard lewat chat.
                </p>
                <div className="space-y-1.5">
                  {QUICK[c.mode].map((s) => (
                    <button
                      key={s} onClick={() => void c.send(s)} disabled={c.busy}
                      className="block w-full rounded-lg border px-3 py-2 text-left text-xs hover:bg-muted disabled:opacity-50"
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

          {/* Composer */}
          <div className="border-t border-border p-2">
            <ChatComposer mode={c.mode} setMode={c.setMode} onSend={c.send} busy={c.busy} rows={1} placeholder="Tanya Copilot…" />
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => toggle(true)}
          className="flex items-center gap-2 rounded-full border border-border bg-card py-2.5 pl-3 pr-4 text-sm font-medium shadow-lg transition-transform hover:scale-[1.02]"
        >
          <span className="grid size-6 place-items-center rounded-full bg-primary/10 text-primary">
            <Sparkles className="size-3.5" />
          </span>
          Tanya Copilot
        </button>
      )}
    </div>
  );
}
