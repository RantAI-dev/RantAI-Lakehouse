"use client";

import * as React from "react";
import { Sparkles } from "lucide-react";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";

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
      {/* Chat (riwayat ada di sidebar) */}
      <div className="mx-auto flex w-full max-w-3xl flex-col">
        <div className="min-h-[56vh] overflow-y-auto pr-0.5">
          {c.messages.length === 0 ? (
            <div className="flex flex-col items-center justify-center px-4 py-16 text-center">
              <div className="mb-4 grid size-12 place-items-center rounded-2xl border border-violet-500/20 bg-gradient-to-br from-violet-500/15 to-purple-600/15 text-violet-600 dark:text-violet-400">
                <Sparkles className="size-6" />
              </div>
              <h2 className="text-xl font-semibold text-foreground">Hi 👋 {c.pageContext.title}</h2>
              <p className="mt-1.5 max-w-md text-sm text-muted-foreground">{c.pageContext.hint}</p>
              <div className="mt-5 flex flex-wrap justify-center gap-2">
                {c.pageContext.suggest[c.mode].map((s) => (
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
            enabledCaps={c.enabledCaps} toggleCap={c.toggleCap}
            placeholder="Ask anything about your lakehouse data…"
          />
        </div>
      </div>
    </div>
  );
}
