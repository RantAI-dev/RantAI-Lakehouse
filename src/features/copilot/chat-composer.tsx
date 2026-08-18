"use client";

import * as React from "react";
import { ArrowUp } from "lucide-react";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { Mode } from "./use-copilot";

/**
 * Composer chat — kotak input + toggle Ask/Build + kirim. Gaya "pill" dipinjam
 * dari RantAI-Agents (toolbar rapi, mode aktif = bg-primary/10 text-primary).
 * Dipakai chat dock global maupun halaman /copilot.
 */
export function ChatComposer({
  mode, setMode, onSend, busy, placeholder, autoFocus, rows = 2,
}: {
  mode: Mode;
  setMode: (m: Mode) => void;
  onSend: (text: string) => void;
  busy: boolean;
  placeholder?: string;
  autoFocus?: boolean;
  rows?: number;
}) {
  const [input, setInput] = React.useState("");
  const submit = () => {
    const t = input.trim();
    if (!t || busy) return;
    onSend(t);
    setInput("");
  };

  return (
    <div className="rounded-2xl border border-border bg-background p-1.5 shadow-sm focus-within:border-ring/60">
      <Textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); }
        }}
        rows={rows}
        autoFocus={autoFocus}
        placeholder={placeholder ?? "Tanya apa saja soal data lakehouse…"}
        aria-label="Pesan ke AI Copilot"
        className="resize-none border-0 bg-transparent px-2 py-1.5 shadow-none focus-visible:ring-0 dark:bg-transparent"
      />
      <div className="flex items-center gap-2 px-1 pb-0.5">
        {/* Toggle Ask/Build */}
        <div className="inline-flex rounded-lg bg-muted/60 p-0.5" role="tablist" aria-label="Mode Copilot">
          {(["ask", "build"] as Mode[]).map((m) => (
            <button
              key={m}
              type="button"
              role="tab"
              aria-selected={mode === m}
              onClick={() => setMode(m)}
              className={cn(
                "rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
                mode === m ? "bg-primary/10 text-primary" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {m === "ask" ? "Ask" : "Build"}
            </button>
          ))}
        </div>
        <span className="hidden text-[11px] text-muted-foreground sm:inline">
          {mode === "ask" ? "read-only" : "bisa bangun & bikin chart"}
        </span>
        <button
          type="button"
          onClick={submit}
          disabled={busy || !input.trim()}
          aria-label="Kirim"
          className={cn(
            "ml-auto grid size-8 place-items-center rounded-lg transition-colors",
            busy || !input.trim() ? "bg-muted text-muted-foreground" : "bg-primary text-primary-foreground hover:bg-primary/85",
          )}
        >
          <ArrowUp className="size-4" />
        </button>
      </div>
    </div>
  );
}
