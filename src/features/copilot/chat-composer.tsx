"use client";

import * as React from "react";
import { ArrowUp, SlidersHorizontal, Square } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverDescription, PopoverHeader, PopoverTitle, PopoverTrigger } from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { Mode } from "./use-copilot";
import { capsForMode } from "./capabilities";

/**
 * What Copilot may use in this conversation — capabilities named after the
 * app's menus, each covering several real tools.
 */
function ToolsMenu({
  mode, enabledCaps, toggleCap,
}: {
  mode: Mode;
  enabledCaps: Set<string>;
  toggleCap: (key: string) => void;
}) {
  const avail = capsForMode(mode);
  const onCount = avail.filter((c) => enabledCaps.has(c.key)).length;
  const limited = onCount < avail.length;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          title="Choose what Copilot can use"
          className={cn(
            "flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs transition-colors hover:text-foreground data-[state=open]:bg-muted data-[state=open]:text-foreground",
            limited ? "text-amber-700 dark:text-amber-400" : "text-muted-foreground",
          )}
        >
          <SlidersHorizontal className="size-3.5" />
          Tools
          {limited ? <span className="tabular-nums">{onCount}/{avail.length}</span> : null}
        </button>
      </PopoverTrigger>
      <PopoverContent side="top" align="start" className="w-80 gap-2 p-2">
        <PopoverHeader className="px-1.5 pt-1">
          <PopoverTitle>What Copilot can use</PopoverTitle>
          <PopoverDescription className="text-xs">
            {mode === "build"
              ? "Turn one off to keep Copilot away from it in this chat. Anything that changes data still asks you first."
              : "Ask mode only reads. Turn one off to keep Copilot from looking there."}
          </PopoverDescription>
        </PopoverHeader>
        <div className="flex flex-col">
          {avail.map((c) => {
            const Icon = c.icon;
            const id = `cap-${c.key}`;
            return (
              <label
                key={c.key}
                htmlFor={id}
                className="flex cursor-pointer items-center gap-2.5 rounded-md px-1.5 py-2 hover:bg-muted"
              >
                <Icon className="size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium text-foreground">{c.label}</span>
                  <span className="block truncate text-[11px] text-muted-foreground">{c.desc}</span>
                </span>
                <Checkbox id={id} checked={enabledCaps.has(c.key)} onCheckedChange={() => toggleCap(c.key)} />
              </label>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/**
 * Composer chat — input + toggle Ask/Build + menu Tools + kirim. Gaya "bar"
 * ala Google Cloud Assist / RantAI-Agents. Dipakai chat dock global & /copilot.
 */
export function ChatComposer({
  mode, setMode, onSend, onStop, busy, placeholder, autoFocus, rows = 2,
  enabledCaps, toggleCap, onFocus, glass, compact,
}: {
  mode: Mode;
  setMode: (m: Mode) => void;
  onSend: (text: string) => void;
  /** Stop the answer in flight; the send button becomes Stop while busy. */
  onStop?: () => void;
  busy: boolean;
  placeholder?: string;
  autoFocus?: boolean;
  rows?: number;
  enabledCaps?: Set<string>;
  toggleCap?: (key: string) => void;
  onFocus?: () => void;
  /** Liquid-glass surface (frosted, blurred) — for the floating dock. */
  glass?: boolean;
  /** Compact single-line pill (collapsed dock, Google-style). */
  compact?: boolean;
}) {
  const [input, setInput] = React.useState("");
  const submit = () => {
    const t = input.trim();
    if (!t || busy) return;
    onSend(t);
    setInput("");
  };

  const glassCls = glass
    ? "border-white/15 bg-background/55 shadow-[0_8px_32px_rgba(0,0,0,0.22)] backdrop-blur-2xl supports-[backdrop-filter]:bg-background/45 ring-1 ring-inset ring-white/10 dark:border-white/10 dark:ring-white/5"
    : "border-border/60 bg-muted/30 shadow-sm";

  // COMPACT — single-line pill (input + send). Focus/typing expands via onFocus.
  if (compact) {
    return (
      <div className={cn("flex items-center gap-2 rounded-full border py-2 pl-4 pr-2 transition-all", glassCls)}>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onFocus={onFocus}
          onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); submit(); } }}
          placeholder={placeholder ?? "Ask anything about your lakehouse…"}
          aria-label="Message AI Copilot"
          className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
        {busy && onStop ? (
          <button
            type="button" onClick={onStop} aria-label="Stop" title="Stop"
            className="grid size-7 shrink-0 place-items-center rounded-full bg-foreground text-background hover:bg-foreground/85"
          >
            <Square className="size-3 fill-current" />
          </button>
        ) : (
          <button
            type="button" onClick={submit} disabled={busy || !input.trim()} aria-label="Send"
            className={cn("grid size-7 shrink-0 place-items-center rounded-full transition-colors",
              busy || !input.trim() ? "text-muted-foreground" : "bg-primary text-primary-foreground hover:bg-primary/85")}
          >
            <ArrowUp className="size-4" />
          </button>
        )}
      </div>
    );
  }

  return (
    <div className={cn(
      "rounded-2xl border p-1.5 transition-all",
      glass ? `${glassCls} focus-within:bg-background/60` : `${glassCls} focus-within:border-foreground/20 focus-within:bg-muted/40 focus-within:shadow-md`,
    )}>
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
        <div className="inline-flex rounded-lg bg-muted/60 p-0.5" role="group" aria-label="Copilot mode">
          {(["ask", "build"] as Mode[]).map((m) => (
            <button
              key={m}
              type="button"
              aria-pressed={mode === m}
              title={m === "ask" ? "Ask: answers questions, changes nothing" : "Build: can create and change things, asking you first"}
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

        {busy && onStop ? (
          <button
            type="button"
            onClick={onStop}
            aria-label="Stop"
            title="Stop"
            className={cn(
              "grid size-8 place-items-center rounded-lg bg-foreground text-background transition-colors hover:bg-foreground/85",
              "ml-auto",
            )}
          >
            <Square className="size-3.5 fill-current" />
          </button>
        ) : (
          <button
            type="button"
            onClick={submit}
            disabled={busy || !input.trim()}
            aria-label="Send"
            className={cn(
              "grid size-8 place-items-center rounded-lg transition-colors",
              "ml-auto",
              busy || !input.trim() ? "bg-muted text-muted-foreground" : "bg-primary text-primary-foreground hover:bg-primary/85",
            )}
          >
            <ArrowUp className="size-4" />
          </button>
        )}
      </div>
    </div>
  );
}
