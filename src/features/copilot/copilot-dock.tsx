"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { Sparkles, Plus, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";

const QUICK: Record<string, string[]> = {
  ask: ["Total kunjungan wisman per kawasan", "Ringkas kualitas data lakehouse"],
  build: ["Bikin chart wisman per kawasan", "Buatkan board dashboard soal wisman"],
};

/**
 * Chat dock GLOBAL — bar di TENGAH-BAWAH setiap halaman (gaya Google Cloud
 * Assist). Bar input selalu tampak; saat ada percakapan / difokus, panel chat
 * NAIK ke atas bar (Ask/Build, menu Tools, render tool, pohon build). Berbagi
 * otak & riwayat dengan halaman /copilot lewat useCopilot. Disembunyikan di
 * /copilot (sudah full-page).
 */
export function CopilotDock() {
  const pathname = usePathname();
  const [expanded, setExpanded] = React.useState(false);
  const c = useCopilot();

  React.useEffect(() => {
    setExpanded(typeof window !== "undefined" && window.localStorage.getItem("copilot-dock-exp") === "1");
  }, []);
  const setExp = (o: boolean) => {
    setExpanded(o);
    try { window.localStorage.setItem("copilot-dock-exp", o ? "1" : "0"); } catch { /* ignore */ }
  };

  if (pathname?.startsWith("/copilot")) return null;

  const showPanel = expanded;

  return (
    <div className="fixed bottom-4 left-1/2 z-50 w-[min(94vw,720px)] -translate-x-1/2 print:hidden">
      {showPanel ? (
        <div className="mb-2 flex max-h-[62vh] flex-col overflow-hidden rounded-2xl border border-border bg-card shadow-2xl">
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
                type="button" onClick={() => setExp(false)} aria-label="Tutup panel"
                className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <ChevronDown className="size-4" />
              </button>
            </div>
          </div>
          <div className="flex-1 overflow-y-auto p-3">
            {c.messages.length === 0 ? (
              <div className="space-y-3 pt-1">
                <p className="text-sm text-muted-foreground">
                  Tanya soal data, atau ke mode <span className="font-medium text-foreground">Build</span> untuk bikin chart/dashboard lewat chat.
                </p>
                <div className="grid gap-1.5 sm:grid-cols-2">
                  {QUICK[c.mode].map((s) => (
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
        </div>
      ) : null}

      {/* Bar input — selalu tampak, di tengah bawah */}
      <div className={cn(!showPanel && "shadow-2xl", "rounded-2xl")}>
        <ChatComposer
          mode={c.mode}
          setMode={c.setMode}
          onSend={(t) => { setExp(true); void c.send(t); }}
          busy={c.busy}
          rows={1}
          placeholder="Tanya apa saja soal lakehouse…"
          enabledTools={c.enabledTools}
          toggleTool={c.toggleTool}
          onFocus={() => setExp(true)}
        />
      </div>
    </div>
  );
}
