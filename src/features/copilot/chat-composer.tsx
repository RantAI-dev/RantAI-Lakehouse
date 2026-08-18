"use client";

import * as React from "react";
import { ArrowUp, SlidersHorizontal, Check } from "lucide-react";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { Mode } from "./use-copilot";
import { toolsForMode } from "./tool-catalog";

/** Menu "Tools" — pilih tool mana yang boleh dipakai agen (ala RantAI-Agents). */
function ToolsMenu({
  mode, enabledTools, toggleTool,
}: {
  mode: Mode;
  enabledTools: Set<string>;
  toggleTool: (name: string) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);
  const avail = toolsForMode(mode);
  const onCount = avail.filter((t) => enabledTools.has(t.name)).length;

  React.useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className={cn(
          "flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs transition-colors",
          open ? "bg-muted text-foreground" : "text-muted-foreground hover:text-foreground",
        )}
      >
        <SlidersHorizontal className="size-3.5" />
        Tools <span className="text-[10px] opacity-70">{onCount}/{avail.length}</span>
      </button>
      {open ? (
        <div className="absolute bottom-full left-0 z-10 mb-1.5 max-h-72 w-72 overflow-y-auto rounded-xl border border-border bg-card p-1.5 shadow-xl">
          <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            Tools · mode {mode === "ask" ? "Ask" : "Build"}
          </p>
          {avail.map((t) => {
            const on = enabledTools.has(t.name);
            return (
              <button
                key={t.name}
                type="button"
                onClick={() => toggleTool(t.name)}
                className="flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left hover:bg-muted"
              >
                <span className={cn(
                  "mt-0.5 grid size-4 shrink-0 place-items-center rounded border",
                  on ? "border-primary bg-primary text-primary-foreground" : "border-border",
                )}>
                  {on ? <Check className="size-3" /> : null}
                </span>
                <span className="min-w-0">
                  <span className="block text-xs font-medium text-foreground">{t.label}</span>
                  <span className="block truncate text-[11px] text-muted-foreground">{t.desc}</span>
                </span>
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

/**
 * Composer chat — input + toggle Ask/Build + menu Tools + kirim. Gaya "bar"
 * ala Google Cloud Assist / RantAI-Agents. Dipakai chat dock global & /copilot.
 */
export function ChatComposer({
  mode, setMode, onSend, busy, placeholder, autoFocus, rows = 2,
  enabledTools, toggleTool, onFocus,
}: {
  mode: Mode;
  setMode: (m: Mode) => void;
  onSend: (text: string) => void;
  busy: boolean;
  placeholder?: string;
  autoFocus?: boolean;
  rows?: number;
  enabledTools?: Set<string>;
  toggleTool?: (name: string) => void;
  onFocus?: () => void;
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
        onFocus={onFocus}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); }
        }}
        rows={rows}
        autoFocus={autoFocus}
        placeholder={placeholder ?? "Tanya apa saja soal data lakehouse…"}
        aria-label="Pesan ke AI Copilot"
        className="resize-none border-0 bg-transparent px-2 py-1.5 shadow-none focus-visible:ring-0 dark:bg-transparent"
      />
      <div className="flex items-center gap-1.5 px-1 pb-0.5">
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

        {enabledTools && toggleTool ? (
          <ToolsMenu mode={mode} enabledTools={enabledTools} toggleTool={toggleTool} />
        ) : null}

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
