"use client";

import * as React from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ArrowUp, SlidersHorizontal, Square } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverDescription, PopoverHeader, PopoverTitle, PopoverTrigger } from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import { useSlidingIndicator } from "@/hooks/use-sliding-indicator";
import { cn } from "@/lib/utils";
import type { Mode } from "./use-copilot";
import { capsForMode } from "./capabilities";

/** The "Tools" menu — pick the (menu-facing) CAPABILITIES the agent may use. */
export function ToolsMenu({
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
          title="Choose what AI Copilot can use"
          className={cn(
            "flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs transition-[background-color,color,scale] duration-150 ease-ui hover:text-foreground active:scale-[0.97] data-[state=open]:bg-muted data-[state=open]:text-foreground",
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
          <PopoverTitle>What AI Copilot can use</PopoverTitle>
          <PopoverDescription className="text-xs">
            {mode === "build"
              ? "Turn one off to keep AI Copilot away from it in this chat. Anything that changes data still asks you first."
              : "Ask mode only reads. Turn one off to keep AI Copilot from looking there."}
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
 * The Ask/Build switch. Shared by the composer and Home's prompt so both
 * drive the same `useCopilot` mode.
 */
export function ModeToggle({ mode, setMode }: { mode: Mode; setMode: (m: Mode) => void }) {
  // Micro-interaction audit 2026-11-14 (emil): the raised fill is one pill that slides between Ask and
  // Build, so the switch reads as a switch; each side gives a little under the press.
  const { listRef, indicatorRef } = useSlidingIndicator<HTMLDivElement>(mode, { box: true });
  return (
    <div ref={listRef} className="relative isolate inline-flex rounded-lg bg-muted/60 p-0.5" role="group" aria-label="AI Copilot mode">
      <span ref={indicatorRef} aria-hidden data-slot="mode-pill"
        className="pointer-events-none absolute top-0 left-0 -z-10 rounded-md bg-background shadow-xs data-ready:transition-[transform,width,height] data-ready:duration-[220ms] data-ready:ease-move" />
      {(["ask", "build"] as Mode[]).map((m) => (
        <button
          key={m}
          type="button"
          aria-pressed={mode === m}
          title={m === "ask" ? "Ask: answers questions, changes nothing" : "Build: can create and change things, asking you first"}
          onClick={() => setMode(m)}
          data-indicator-item
          data-indicator-active={mode === m || undefined}
          className={cn(
            "rounded-md px-2.5 py-1 text-xs font-medium transition-[color,scale] duration-200 ease-ui active:scale-[0.97]",
            // Neutral, not `text-primary`: primary is too dark to read on the
            // dark theme, where the selected mode all but disappeared.
            mode === m ? "text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {m === "ask" ? "Ask" : "Build"}
        </button>
      ))}
    </div>
  );
}

/** What a page can do to the composer from outside: put text in it (`WEB-17` "Ask why"). */
export type ChatComposerHandle = {
  /** Replace the text, focus the box with the caret at the end; nothing is sent. */
  fill: (text: string) => void;
};

/**
 * The chat composer — input + Ask/Build toggle + Tools menu + send. A "bar"
 * style like Google Cloud Assist / RantAI-Agents. Used by the global chat
 * dock and /copilot.
 */
export function ChatComposer({
  ref, mode, setMode, onSend, onStop, busy, placeholder, autoFocus, rows = 2,
  enabledCaps, toggleCap, onFocus, glass, compact, solid, examples, footer, value, onValueChange,
}: {
  ref?: React.Ref<ChatComposerHandle>;
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
  /**
   * Opaque surface with a plain border — the full /copilot page, where the
   * composer sits straight on the tinted page background and the default
   * translucent fill would not stand out from it.
   */
  solid?: boolean;
  /**
   * Example prompts shown in place of the static placeholder, one at a
   * time, cross-fading every few seconds while the box is empty. Tab
   * takes the one on screen into the box. Under `prefers-reduced-motion`
   * the first one stays put.
   */
  examples?: string[];
  /**
   * One line under the toolbar, inside the box, edge to edge: Home's
   * context strip (what Ask can read, what Build may change). Full style only.
   */
  footer?: React.ReactNode;
  /**
   * The draft, when the caller owns it. The floating dock swaps the
   * collapsed pill and the open composer (two mounts of this component), and
   * a draft kept here would be lost at every open and close.
   */
  value?: string;
  onValueChange?: (text: string) => void;
}) {
  const [ownInput, setOwnInput] = React.useState("");
  const input = value ?? ownInput;
  const setInput = onValueChange ?? setOwnInput;
  const area = React.useRef<HTMLTextAreaElement>(null);
  // Micro-interaction audit 2026-11-14 (emil; WEB-17 "Ask why"): text put in from elsewhere arrives with
  // a short cue, the box's edge lighting up and easing back, so the eye goes to it before Enter.
  const [filled, setFilled] = React.useState(0);
  React.useEffect(() => {
    if (!filled) return;
    const t = window.setTimeout(() => setFilled(0), 650);
    return () => window.clearTimeout(t);
  }, [filled]);
  const reduce = useReducedMotion() ?? false;
  React.useImperativeHandle(ref, () => ({
    fill: (text: string) => {
      setInput(text);
      setFilled((n) => n + 1);
      const el = area.current;
      if (!el) return;
      // Micro-interaction audit 2026-11-14: a box scrolled out of sight is brought back with a glide
      // (at once under reduced motion) instead of the jump `focus()` makes on its own.
      el.focus({ preventScroll: true });
      const box = el.getBoundingClientRect();
      if (box.top < 0 || box.bottom > window.innerHeight) {
        el.scrollIntoView?.({ block: "center", behavior: reduce ? "auto" : "smooth" });
      }
      // After React has put the text in, so the caret lands after it.
      requestAnimationFrame(() => el.setSelectionRange(text.length, text.length));
    },
  }), [reduce, setInput]);
  const [exampleIndex, setExampleIndex] = React.useState(0);
  const cycling = Boolean(examples?.length) && !input;
  React.useEffect(() => {
    if (!cycling || reduce || (examples?.length ?? 0) < 2) return;
    const t = window.setInterval(() => setExampleIndex((i) => i + 1), 3500);
    return () => window.clearInterval(t);
  }, [cycling, reduce, examples?.length]);
  const example = examples?.length ? examples[exampleIndex % examples.length] : undefined;
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
      // `data-slot` lets the dock animate this surface itself: an opacity or
      // clip on a wrapper would cut the glass's backdrop blur off.
      <div data-slot="chat-composer" className={cn("flex items-center gap-2 rounded-full border py-2 pl-4 pr-2", glassCls)}>
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

  const body = (
    <>
      <div className="relative">
        <Textarea
          ref={area}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onFocus={onFocus}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); }
            // Tab takes the example on screen, so it can be sent or edited.
            if (e.key === "Tab" && !e.shiftKey && cycling && example) { e.preventDefault(); setInput(example); }
          }}
          rows={rows}
          autoFocus={autoFocus}
          // With examples the placeholder is the animated overlay below: a
          // native placeholder cannot cross-fade.
          placeholder={example ? "" : placeholder ?? "Ask anything about your lakehouse data…"}
          aria-label="Message AI Copilot"
          className="resize-none border-0 bg-transparent px-2 py-1.5 shadow-none focus-visible:ring-0 dark:bg-transparent"
        />
        {cycling && example ? (
          <div aria-hidden className="pointer-events-none absolute inset-x-2 top-1.5 flex items-baseline gap-2 text-base text-muted-foreground md:text-sm">
            <AnimatePresence mode="wait" initial={false}>
              <motion.span
                key={example}
                initial={reduce ? false : { opacity: 0, y: 6 }}
                // Micro-interaction audit 2026-11-14: the console's ease-out, and the old example leaves faster than the new one comes.
                animate={{ opacity: 1, y: 0, transition: { duration: 0.25, ease: [0.23, 1, 0.32, 1] } }}
                exit={reduce ? undefined : { opacity: 0, y: -6, transition: { duration: 0.15, ease: [0.23, 1, 0.32, 1] } }}
                className="min-w-0 truncate"
              >
                {example}
              </motion.span>
            </AnimatePresence>
            <kbd className="hidden shrink-0 rounded border border-border px-1 font-mono text-[10px] sm:inline">Tab</kbd>
          </div>
        ) : null}
      </div>
      <div className="flex items-center gap-1.5 px-1 pb-0.5">
        <ModeToggle mode={mode} setMode={setMode} />

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
              "grid size-8 place-items-center rounded-lg bg-foreground text-background transition-[background-color,scale] duration-150 ease-ui hover:bg-foreground/85 active:scale-95",
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
              "grid size-8 place-items-center rounded-lg transition-[background-color,color,scale] duration-150 ease-ui enabled:active:scale-95",
              "ml-auto",
              busy || !input.trim() ? "bg-muted text-muted-foreground" : "bg-primary text-primary-foreground hover:bg-primary/85",
            )}
          >
            <ArrowUp className="size-4" />
          </button>
        )}
      </div>
      {footer ? <div className="-mx-1.5 -mb-1.5 mt-1.5 overflow-hidden rounded-b-[15px]">{footer}</div> : null}
    </>
  );

  return (
    // Owner QA 2026-11-15: `transition-all` also tweened size and radius
    // whenever the box changed shape; only the focus colours fade now.
    <div data-slot="chat-composer" data-filled={filled ? "" : undefined} className={cn(
      "rounded-2xl border p-1.5 transition-[border-color,background-color,box-shadow] duration-200 ease-ui",
      "data-filled:border-primary/50 data-filled:ring-3 data-filled:ring-primary/15",
      solid
        ? "border-border bg-background shadow-xs focus-within:border-foreground/25"
        : glass ? `${glassCls} focus-within:bg-background/60` : `${glassCls} focus-within:border-foreground/20 focus-within:bg-muted/40 focus-within:shadow-md`,
    )}>
      {body}
    </div>
  );
}
