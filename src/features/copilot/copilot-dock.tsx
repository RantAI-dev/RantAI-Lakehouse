"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { Sparkles, Plus, ChevronDown, PanelRight } from "lucide-react";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";
import { CopilotHistoryButton } from "./history-menu";

/**
 * Chat dock GLOBAL — bar di TENGAH-BAWAH setiap halaman (gaya Google Cloud
 * Assist). Bar input selalu tampak saat posisi bottom aktif; saat ada percakapan / difokus,
 * panel chat NAIK ke atas bar (Ask/Build, menu Tools, render tool, pohon build).
 * Pengguna dapat beralih ke sidebar kanan (resizable) lewat tombol dock di header.
 * Disembunyikan di /copilot (sudah full-page) atau saat posisi dock di kanan.
 */
export function CopilotDock() {
  const pathname = usePathname();
  const c = useCopilot();

  if (pathname?.startsWith("/copilot") || c.dockPosition === "right") {
    return null;
  }

  const showPanel = c.expanded;

  return (
    <>
    {/* The bar floats over the page; this keeps the last row of content
        scrollable out from under it. */}
    <div aria-hidden className="h-20 shrink-0 print:hidden" />
    <div className={`fixed bottom-4 left-1/2 z-50 -translate-x-1/2 print:hidden transition-all duration-200 ${showPanel ? "w-[min(92vw,560px)]" : "w-[min(88vw,460px)]"}`}>
      {showPanel ? (
        <div className="mb-2 flex max-h-[58vh] flex-col overflow-hidden rounded-2xl border border-border/70 bg-card/90 shadow-[0_8px_40px_rgba(0,0,0,0.18)] backdrop-blur-xl supports-backdrop-filter:bg-card/80">
          <div className="flex items-center gap-2 border-b border-border px-3 py-2">
            <span className="grid size-6 place-items-center rounded-md bg-primary/10 text-primary">
              <Sparkles className="size-3.5" />
            </span>
            <span className="text-sm font-semibold">AI Copilot</span>
            <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              {c.mode}
            </span>
            <div className="ml-auto flex items-center gap-0.5">
              <CopilotHistoryButton align="end" />
              <button
                type="button"
                onClick={() => {
                  c.setDockPosition("right");
                  c.setExpanded(true);
                }}
                aria-label="Dock to right sidebar"
                title="Dock to right sidebar"
                className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              >
                <PanelRight className="size-4" />
              </button>
              <button
                type="button"
                onClick={c.newChat}
                aria-label="New chat"
                title="New chat"
                className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              >
                <Plus className="size-4" />
              </button>
              <button
                type="button"
                onClick={() => c.setExpanded(false)}
                aria-label="Collapse panel"
                title="Collapse panel"
                className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              >
                <ChevronDown className="size-4" />
              </button>
            </div>
          </div>
          <div className="flex-1 overflow-y-auto p-3">
            {c.messages.length === 0 ? (
              <div className="space-y-3 pt-1">
                <div>
                  <p className="text-sm font-medium text-foreground">{c.pageContext.title}</p>
                  <p className="text-xs text-muted-foreground">{c.pageContext.hint}</p>
                </div>
                <div className="grid gap-1.5 sm:grid-cols-2">
                  {c.pageContext.suggest[c.mode].map((s) => (
                    <button
                      key={s}
                      onClick={() => c.send(s)}
                      disabled={c.busy}
                      className="rounded-lg border border-border/70 bg-muted/20 px-3 py-2 text-left text-xs text-foreground transition-colors hover:bg-muted hover:border-primary/40 disabled:opacity-50"
                    >
                      {s}
                    </button>
                  ))}
                </div>
              </div>
            ) : (
              <ChatMessages
                messages={c.messages}
                busy={c.busy}
                error={c.error} progress={c.progress} onRetry={c.retry}
                onConfirmTool={c.confirmTool}
                onCancelTool={c.cancelTool}
                onCompleteTool={c.completeToolStep}
                confirmingKey={c.confirmingKey}
              />
            )}
          </div>
        </div>
      ) : null}

      {/* Bar input — glass; compact pill when collapsed, full when expanded */}
      <ChatComposer onStop={c.stop}
        glass
        compact={!showPanel}
        mode={c.mode}
        setMode={c.setMode}
        onSend={(t) => {
          c.setExpanded(true);
          c.send(t);
        }}
        busy={c.busy}
        rows={1}
        placeholder="Ask anything about your lakehouse…"
        enabledCaps={c.enabledCaps}
        toggleCap={c.toggleCap}
        onFocus={() => c.setExpanded(true)}
      />
    </div>
    </>
  );
}
