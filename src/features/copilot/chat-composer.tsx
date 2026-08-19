"use client";

import * as React from "react";
import { ArrowUp, SlidersHorizontal, Check } from "lucide-react";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { Mode } from "./use-copilot";
import { capsForMode } from "./capabilities";

/** Menu "Tools" — pilih KAPABILITAS (sesuai menu) yang boleh dipakai agen. */
function ToolsMenu({
  mode, enabledCaps, toggleCap,
}: {
  mode: Mode;
  enabledCaps: Set<string>;
  toggleCap: (key: string) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);
  const avail = capsForMode(mode);
  const onCount = avail.filter((c) => enabledCaps.has(c.key)).length;

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
        <div className="absolute bottom-full left-0 z-10 mb-1.5 w-72 overflow-hidden rounded-xl border border-border bg-card p-1.5 shadow-xl">
          <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            Capabilities · {mode === "ask" ? "Ask" : "Build"}
          </p>
          {avail.map((c) => {
            const on = enabledCaps.has(c.key);
            const Icon = c.icon;
            return (
              <button
                key={c.key}
                type="button"
                onClick={() => toggleCap(c.key)}
                className="flex w-full items-center gap-2.5 rounded-md px-2 py-2 text-left hover:bg-muted"
              >
                <Icon className="size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium text-foreground">{c.label}</span>
                  <span className="block truncate text-[11px] text-muted-foreground">{c.desc}</span>
                </span>
                <span className={cn(
                  "grid size-4 shrink-0 place-items-center rounded border",
                  on ? "border-primary bg-primary text-primary-foreground" : "border-border",
                )}>
                  {on ? <Check className="size-3" /> : null}
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
  enabledCaps, toggleCap, onFocus,
}: {
  mode: Mode;
  setMode: (m: Mode) => void;
  onSend: (text: string) => void;
  busy: boolean;
  placeholder?: string;
  autoFocus?: boolean;
  rows?: number;
  enabledCaps?: Set<string>;
  toggleCap?: (key: string) => void;
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
    <div className="rounded-2xl border border-border/60 bg-muted/30 p-1.5 shadow-sm transition-all focus-within:border-foreground/20 focus-within:bg-muted/40 focus-within:shadow-md">
      <Textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onFocus={onFocus}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); }
        }}
        rows={rows}
        autoFocus={autoFocus}
        placeholder={placeholder ?? "Ask anything about your lakehouse data…"}
        aria-label="Message AI Copilot"
        className="resize-none border-0 bg-transparent px-2 py-1.5 shadow-none focus-visible:ring-0 dark:bg-transparent"
      />
      <div className="flex items-center gap-1.5 px-1 pb-0.5">
        {/* Toggle Ask/Build */}
        <div className="inline-flex rounded-lg bg-muted/60 p-0.5" role="tablist" aria-label="Copilot mode">
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

        {enabledCaps && toggleCap ? (
          <ToolsMenu mode={mode} enabledCaps={enabledCaps} toggleCap={toggleCap} />
        ) : null}

        <button
          type="button"
          onClick={submit}
          disabled={busy || !input.trim()}
          aria-label="Send"
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
