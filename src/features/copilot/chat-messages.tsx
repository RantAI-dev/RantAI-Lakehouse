"use client";

import * as React from "react";
import Link from "next/link";
import { motion, useReducedMotion } from "motion/react";
import { AlertCircle, ArrowDown, BarChart3, Check, ChevronRight, RotateCcw, Sparkles, Wrench } from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { BuildTree } from "./build-tree";
import { CopyButton } from "./copy-button";
import { settleStreamingMarkdown } from "@/lib/stream-markdown";
import { MiniMarkdown } from "./mini-markdown";
import { PendingActionCard } from "./pending-action-card";
import { TOOL_LABEL, ToolStepCard, asObj, type ToolStep } from "./tool-step";
import type { ChatProgress, Msg } from "./use-copilot";

/**
 * What Copilot is doing right now: a pill with the current step and the
 * time so far, and under it the steps already finished in this answer.
 * Built only from the events `/api/ai/chat` really sends (`status` before
 * each model round, `tool` before each tool call) — no invented stages.
 */
function ProgressLine({ progress }: { progress: ChatProgress | null }) {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(t);
  }, []);
  const label =
    progress?.phase === "tool" && progress.tool
      ? `Running ${TOOL_LABEL[progress.tool] ?? progress.tool}…`
      : progress?.steps?.length
        ? "Writing the answer…"
        : "Thinking…";
  const seconds = progress ? Math.max(0, (now - progress.startedAt) / 1000) : 0;
  const steps = progress?.steps ?? [];
  // While a tool runs it is the pill; only the ones before it are done.
  const done = progress?.phase === "tool" ? steps.slice(0, -1) : steps;
  return (
    <div className="flex flex-col items-start gap-1.5" role="status">
      <div className="inline-flex items-center gap-2.5 rounded-full border border-[color-mix(in_oklch,var(--brand-1),transparent_70%)] bg-[color-mix(in_oklch,var(--brand-1),transparent_92%)] py-1.5 pr-3.5 pl-3 text-sm text-foreground/85">
        <span className="relative flex size-2">
          <span className="absolute inline-flex size-full rounded-full bg-[var(--brand-1)] opacity-60 motion-safe:animate-ping" />
          <span className="relative inline-flex size-2 rounded-full bg-[var(--brand-1)]" />
        </span>
        <span>{label}</span>
        {seconds >= 1 ? (
          <span className="text-xs tabular-nums text-muted-foreground">· {seconds.toFixed(0)}s</span>
        ) : null}
      </div>
      {done.length ? (
        <ul className="flex flex-wrap gap-x-3 gap-y-1 pl-1 text-xs text-muted-foreground">
          {done.map((t, i) => (
            <li key={`${t}-${i}`} className="inline-flex items-center gap-1">
              <Check className="size-3 text-emerald-500" aria-hidden />
              {TOOL_LABEL[t] ?? t}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/** Copilot's mark beside its answers on the full page — the Home badge's blue. */
function CopilotAvatar({ thinking }: { thinking?: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "relative grid size-8 shrink-0 place-items-center rounded-xl border border-[color-mix(in_oklch,var(--brand-1),transparent_55%)] bg-[color-mix(in_oklch,var(--brand-1),transparent_86%)] text-[var(--brand-1)] shadow-[0_6px_18px_-10px_var(--brand-1)]",
        thinking && "motion-safe:animate-pulse",
      )}
    >
      <Sparkles className="size-4" />
    </span>
  );
}

/** The Copilot message list — rich rendering (tool cards, build tree, markdown). */
export function ChatMessages({
  messages, busy, progress, error, className, onRetry,
  onConfirmTool, onCancelTool, onCompleteTool, confirmingKey, avatars, draft,
}: {
  messages: Msg[];
  busy: boolean;
  progress?: ChatProgress | null;
  error?: string | null;
  className?: string;
  /** Ask the last question again (after an error or a stop). */
  onRetry?: () => void;
  /** Confirm a `needs_confirmation` tool step (T0.4's Confirm button). */
  onConfirmTool?: (messageIndex: number, stepIndex: number) => void;
  /** Cancel a `needs_confirmation` tool step. */
  onCancelTool?: (messageIndex: number, stepIndex: number) => void;
  /** Mark a pending step done with this result, without re-running the tool. */
  onCompleteTool?: (messageIndex: number, stepIndex: number, result: Record<string, unknown>) => void;
  /** `"<messageIndex>:<stepIndex>"` of the step currently being confirmed. */
  confirmingKey?: string | null;
  /** Show Copilot's avatar beside answers — the wide /copilot page only. */
  avatars?: boolean;
  /** Answer text streamed so far (`useCopilot().draft`). */
  draft?: string;
}) {
  const reduce = useReducedMotion() ?? false;
  // New messages rise in; skipped under reduced motion.
  const enter = reduce
    ? {}
    : { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0 }, transition: { type: "spring" as const, stiffness: 220, damping: 26 } };
  const rootRef = React.useRef<HTMLDivElement>(null);
  // Changes whenever something new lands at the bottom of the list.
  const tail = `${messages.length}:${messages[messages.length - 1]?.content.length ?? 0}:${busy}:${error ?? ""}:${progress?.phase}:${progress?.steps?.length ?? 0}:${draft?.length ?? 0}`;
  const { pinned, jumpToLatest } = useStickToBottom(rootRef, tail);

  const lastIndex = messages.length - 1;

  return (
    <div ref={rootRef} className={cn("space-y-4", className)}>
      {messages.map((m, i) => {
        const key = m.id ?? `m${i}`;
        if (m.role === "user") {
          return (
            <motion.div key={key} {...enter} className="flex justify-end">
              <div className="max-w-[80%] whitespace-pre-wrap rounded-2xl rounded-br-sm bg-[color-mix(in_oklch,var(--brand-1),transparent_86%)] px-3.5 py-2 text-sm text-foreground ring-1 ring-[color-mix(in_oklch,var(--brand-1),transparent_70%)]">
                {m.content}
              </div>
            </motion.div>
          );
        }
        const pendingIndex = m.tools?.findIndex((t) => Boolean(asObj(t.result).needs_confirmation)) ?? -1;
        const pendingStep = pendingIndex >= 0 ? m.tools?.[pendingIndex] : undefined;
        return (
          <motion.div key={key} {...enter} className={cn("min-w-0", avatars && "flex gap-3")}>
            {avatars ? <CopilotAvatar /> : null}
            <div className="min-w-0 flex-1 space-y-2">
            {m.tools?.length ? <StepsDisclosure tools={m.tools} /> : null}
            {m.buildRunId ? <BuildTree runId={m.buildRunId} /> : null}
            {m.content ? <MiniMarkdown text={m.content} /> : null}
            {m.stopped ? (
              <p className="flex items-center gap-2 text-sm text-muted-foreground">
                Response stopped.
                {i === lastIndex && onRetry && !busy ? (
                  <Button variant="link" size="sm" className="h-auto p-0" onClick={onRetry}>
                    Ask again
                  </Button>
                ) : null}
              </p>
            ) : null}
            {pendingStep ? (
              <PendingActionCard
                step={pendingStep}
                confirming={confirmingKey === `${i}:${pendingIndex}`}
                onConfirm={() => onConfirmTool?.(i, pendingIndex)}
                onCancel={() => onCancelTool?.(i, pendingIndex)}
                onSavedInBuilder={(result) => onCompleteTool?.(i, pendingIndex, result)}
              />
            ) : null}
            {m.chartCreated ? (
              <Link
                href="/dashboards"
                className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1.5")}
              >
                <BarChart3 className="size-4" /> Open Dashboards
              </Link>
            ) : null}
            {m.content ? (
              <AnswerActions
                msg={m}
                onRegenerate={i === lastIndex && onRetry && !busy ? onRetry : undefined}
              />
            ) : null}
            </div>
          </motion.div>
        );
      })}
      {busy ? (
        <div className={cn("min-w-0", avatars && "flex items-start gap-3")}>
          {avatars ? <CopilotAvatar thinking /> : null}
          <div className="min-w-0 flex-1">
            {draft ? <StreamingAnswer text={draft} /> : <ProgressLine progress={progress ?? null} />}
          </div>
        </div>
      ) : null}
      {error ? (
        <div className="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm" role="alert">
          <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
          <p className="min-w-0 flex-1 text-foreground">{error}</p>
          {onRetry ? (
            <Button size="sm" variant="outline" onClick={onRetry} className="h-7 gap-1 text-xs">
              <RotateCcw className="size-3.5" /> Retry
            </Button>
          ) : null}
        </div>
      ) : null}
      {!pinned ? (
        <div className="pointer-events-none sticky bottom-2 flex justify-center">
          <button
            type="button"
            onClick={jumpToLatest}
            className="pointer-events-auto inline-flex items-center gap-1.5 rounded-full border border-border bg-background/90 px-3 py-1.5 text-xs font-medium text-foreground shadow-lg backdrop-blur transition-all hover:-translate-y-0.5 hover:border-[color-mix(in_oklch,var(--brand-1),transparent_50%)]"
          >
            <ArrowDown className="size-3.5" /> Jump to latest
          </button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * Reveals `target` at a steady pace instead of in network-sized jumps, and
 * catches up faster the further behind it is (the RantAI-Agents chat's
 * pacing: ~25 chars/s, 60 when 100+ behind, 180 when 300+ behind). Shows
 * everything at once under reduced motion. When `target` restarts (a new
 * model round), so does the reveal.
 */
function useSmoothText(target: string): string {
  const reduce = useReducedMotion() ?? false;
  const [shown, setShown] = React.useState(0);
  const shownRef = React.useRef(0);

  React.useEffect(() => {
    if (reduce) return;
    // The draft started over (a new model round): so does the reveal.
    if (shownRef.current > target.length) shownRef.current = 0;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const behind = target.length - shownRef.current;
      if (behind > 0) {
        const rate = behind > 300 ? 180 : behind > 100 ? 60 : 25;
        const step = Math.max(1, Math.round(((now - last) / 1000) * rate));
        shownRef.current = Math.min(target.length, shownRef.current + step);
        setShown(shownRef.current);
      }
      last = now;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [target, reduce]);

  if (reduce) return target;
  return target.slice(0, shown <= target.length ? shown : 0);
}

/**
 * The answer while it is being written: paced, made safe to render
 * mid-token (`settleStreamingMarkdown`), with a caret. Numbers are only
 * checked against tool results once the answer is complete, so the draft
 * says so; the checked answer replaces it.
 */
function StreamingAnswer({ text }: { text: string }) {
  const shown = useSmoothText(text.trimStart());
  return (
    <div className="space-y-1.5" aria-live="polite" aria-busy="true">
      <MiniMarkdown text={settleStreamingMarkdown(shown)} caret />
      <p className="text-[11px] text-muted-foreground">Writing… numbers are checked against the data when the answer is done.</p>
    </div>
  );
}

/** The nearest ancestor that scrolls vertically (the chat's own list). */
function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let p = el?.parentElement ?? null; p; p = p.parentElement) {
    const { overflowY } = getComputedStyle(p);
    if (overflowY === "auto" || overflowY === "scroll") return p;
  }
  return null;
}

/**
 * Follows new content only while the reader is at the bottom. Scrolling up
 * to reread detaches it (a "Jump to latest" button appears); coming back
 * within 80px of the end re-attaches. Scrolls the chat's own container, so
 * the page around it never moves.
 */
function useStickToBottom(rootRef: React.RefObject<HTMLDivElement | null>, tail: string) {
  const [pinned, setPinned] = React.useState(true);
  const pinnedRef = React.useRef(true);

  React.useEffect(() => {
    const scroller = scrollParent(rootRef.current);
    if (!scroller) return;
    const onScroll = () => {
      const atEnd = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
      pinnedRef.current = atEnd;
      setPinned(atEnd);
    };
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => scroller.removeEventListener("scroll", onScroll);
  }, [rootRef]);

  React.useEffect(() => {
    if (!pinnedRef.current) return;
    const scroller = scrollParent(rootRef.current);
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
  }, [rootRef, tail]);

  const jumpToLatest = React.useCallback(() => {
    const scroller = scrollParent(rootRef.current);
    pinnedRef.current = true;
    setPinned(true);
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
  }, [rootRef]);

  return { pinned, jumpToLatest };
}

/**
 * An answer's tool steps, folded into one line ("4 steps · 2 failed ·
 * Search datasets, SQL query…") so the answer, not the plumbing, is what
 * the reader sees first. It opens by itself only when a step waits on the
 * user (approval or confirmation); failures are counted in the summary
 * rather than spread open above the answer.
 */
function StepsDisclosure({ tools }: { tools: ToolStep[] }) {
  const waiting = tools.some((t) => {
    const r = asObj(t.result);
    return Boolean(r.needs_approval || r.needs_confirmation);
  });
  const [open, setOpen] = React.useState(waiting);
  const failed = tools.filter((t) => !t.ok || "error" in asObj(t.result)).length;
  const names = [...new Set(tools.map((t) => TOOL_LABEL[t.tool] ?? t.tool))];
  return (
    <div className="rounded-xl border border-border/70 bg-background/40">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="group/steps flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-xs text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
      >
        <Wrench className="size-3.5 shrink-0 transition-transform duration-300 group-hover/steps:-rotate-12" aria-hidden />
        <span className="font-medium text-foreground/85">
          {tools.length} {tools.length === 1 ? "step" : "steps"}
        </span>
        {failed ? (
          <span className="rounded-full bg-destructive/10 px-1.5 py-0.5 text-[10px] font-medium text-destructive">
            {failed} failed
          </span>
        ) : null}
        <span className="min-w-0 flex-1 truncate">· {names.join(", ")}</span>
        <ChevronRight className={cn("size-3.5 shrink-0 transition-transform", open && "rotate-90")} aria-hidden />
      </button>
      {open ? (
        <div className="space-y-1.5 border-t border-border/70 p-2">
          {tools.map((t, j) => (
            <ToolStepCard key={j} step={t} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** "3.2s", "1m 04s". */
function formatElapsed(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${String(Math.round(s % 60)).padStart(2, "0")}s`;
}

/**
 * Under a finished answer: copy, regenerate (latest answer only) and how
 * long it took. No rating buttons: nothing on the backend would record a
 * rating, and a button that goes nowhere is worse than none.
 */
function AnswerActions({ msg, onRegenerate }: { msg: Msg; onRegenerate?: () => void }) {
  const time = msg.at ? new Date(msg.at) : null;
  return (
    <div className="flex items-center gap-0.5 text-muted-foreground">
      <CopyButton text={msg.content} label="Copy answer" className="p-1.5" />
      {onRegenerate ? (
        <button
          type="button"
          onClick={onRegenerate}
          aria-label="Regenerate"
          title="Regenerate"
          className="group/regen inline-flex items-center rounded-md p-1.5 transition-colors hover:bg-muted hover:text-foreground"
        >
          <RotateCcw className="size-3.5 transition-transform duration-500 group-hover/regen:-rotate-180 motion-reduce:transition-none" />
        </button>
      ) : null}
      {msg.elapsedMs != null || time ? (
        <span className="pl-1.5 text-[11px] tabular-nums">
          {msg.elapsedMs != null ? `Answered in ${formatElapsed(msg.elapsedMs)}` : null}
          {msg.elapsedMs != null && time ? " · " : null}
          {time ? (
            <time dateTime={msg.at}>{time.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>
          ) : null}
        </span>
      ) : null}
    </div>
  );
}
