"use client";

import * as React from "react";
import Link from "next/link";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  AlertCircle,
  ArrowDown,
  BarChart3,
  Brain,
  Check,
  ChevronDown,
  Copy,
  Pencil,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { formatRelativeTime } from "@/lib/format";
import { settleStreamingMarkdown } from "@/lib/stream-markdown";
import { cn } from "@/lib/utils";
import { BuildTree } from "./build-tree";
import { MiniMarkdown } from "./mini-markdown";
import { PendingActionCard } from "./pending-action-card";
import { FinishedToolRow, RunningToolRow, ToolStepCard, asObj, toolLabel } from "./tool-step";
import type { ChatProgress, Msg } from "./use-copilot";

/*
 * Copilot's message list, laid out like the RantAI-Agents chat
 * (`features/conversations/components/chat/chat-workspace.tsx`): the
 * user's message in a tinted bubble on the right, answers as plain text on
 * the left, tool calls as compact rows above the text, a collapsible
 * "Thinking" box for the model's reasoning, and a footer with the time and
 * hover actions.
 *
 * There is no avatar or "Copilot" name beside an answer. With one
 * assistant in the conversation, the bubble on the right already says who
 * is speaking, and a gradient badge plus a bold name on every answer was
 * decoration repeated per message (QA feedback).
 * From the ParaGPT answer-rendering proposal: the live status pill with
 * seconds, the table copy bar (`MiniMarkdown`) and "answered in".
 */

/** The Agents typing dots plus the proposal's live label and seconds. */
function StatusPill({ progress }: { progress: ChatProgress | null }) {
  const reduce = useReducedMotion() ?? false;
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(t);
  }, []);
  const label =
    progress?.phase === "tool" && progress.tool
      ? `Running ${toolLabel(progress.tool)}…`
      : progress?.phase === "verifying"
        ? "Checking the figures…"
        : progress?.steps?.length
        ? "Writing the answer…"
        : "Thinking…";
  const seconds = progress ? Math.max(0, (now - progress.startedAt) / 1000) : 0;
  return (
    <div
      className="inline-flex items-center gap-2.5 rounded-full border border-border bg-muted/50 py-1.5 pr-3.5 pl-3 text-sm text-muted-foreground"
      role="status"
    >
      <span className="flex items-center gap-1" aria-hidden>
        {[0, 1, 2].map((i) => (
          <motion.span
            key={i}
            className="size-1.5 rounded-full bg-[var(--brand-1)]"
            animate={reduce ? undefined : { scale: [1, 1.3, 1], opacity: [0.35, 1, 0.35] }}
            transition={{ duration: 1.2, repeat: Infinity, delay: i * 0.15, ease: "easeInOut" }}
          />
        ))}
      </span>
      <span>{label}</span>
      {seconds >= 1 ? <span className="text-xs tabular-nums">· {seconds.toFixed(0)}s</span> : null}
    </div>
  );
}

/**
 * The model's reasoning, as the Agents `ReasoningBox`: open while it is
 * being written, closed once the answer is done (a click wins either way).
 */
function ReasoningBox({ text, live, ms }: { text: string; live?: boolean; ms?: number }) {
  const [open, setOpen] = React.useState(Boolean(live));
  const [touched, setTouched] = React.useState(false);
  React.useEffect(() => {
    if (!live && !touched) setOpen(false);
  }, [live, touched]);
  if (!text.trim()) return null;
  return (
    <div className="mb-2">
      <button
        type="button"
        onClick={() => {
          setTouched(true);
          setOpen((o) => !o);
        }}
        aria-expanded={open}
        className="flex items-center gap-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
      >
        <Brain className={cn("size-3.5", live && "text-[var(--brand-1)] motion-safe:animate-pulse")} aria-hidden />
        <span>{live ? "Thinking…" : ms != null ? `Thought for ${Math.max(1, Math.round(ms / 1000))}s` : "Thoughts"}</span>
        <ChevronDown className={cn("size-3 transition-transform", !open && "-rotate-90")} aria-hidden />
      </button>
      <AnimatePresence initial={false}>
        {open ? (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.15 }}
            className="overflow-hidden"
          >
            <p className="mt-1.5 max-h-60 overflow-y-auto border-l-2 border-border/60 pl-3 text-xs leading-relaxed whitespace-pre-wrap text-muted-foreground">
              {text.trim()}
            </p>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}

/** One icon button in a message footer. */
function ActionButton({
  label,
  onClick,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Button
      variant="ghost"
      size="icon"
      className={cn("size-6 rounded-lg text-muted-foreground", danger ? "hover:text-destructive" : "hover:text-foreground")}
      onClick={onClick}
      aria-label={label}
      title={label}
    >
      {children}
    </Button>
  );
}

/**
 * The Agents footer: when, and actions that show on hover (always on touch
 * and keyboard focus). No rating buttons: nothing would record a rating.
 */
function MessageFooter({
  msg,
  isUser,
  onEdit,
  onRegenerate,
  onDelete,
}: {
  msg: Msg;
  isUser: boolean;
  onEdit?: () => void;
  onRegenerate?: () => void;
  onDelete?: () => void;
}) {
  const [copied, setCopied] = React.useState(false);
  React.useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(t);
  }, [copied]);
  return (
    <div className={cn("mt-2 flex items-center gap-2", isUser && "flex-row-reverse")}>
      {msg.at ? (
        <time dateTime={msg.at} className="text-[10px] text-muted-foreground" title={new Date(msg.at).toLocaleString()}>
          {formatRelativeTime(msg.at)}
          {!isUser && msg.elapsedMs != null ? ` · answered in ${formatElapsed(msg.elapsedMs)}` : null}
        </time>
      ) : null}
      <div className="flex items-center gap-0.5 opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100 [@media(hover:none)]:opacity-100">
        <ActionButton
          label={copied ? "Copied" : "Copy message"}
          onClick={() => void navigator.clipboard.writeText(msg.content).then(() => setCopied(true), () => {})}
        >
          {copied ? <Check className="size-3 text-emerald-500" /> : <Copy className="size-3" />}
        </ActionButton>
        {onEdit ? (
          <ActionButton label="Edit message" onClick={onEdit}>
            <Pencil className="size-3" />
          </ActionButton>
        ) : null}
        {onRegenerate ? (
          <ActionButton label="Regenerate response" onClick={onRegenerate}>
            <RefreshCw className="size-3" />
          </ActionButton>
        ) : null}
        {onDelete ? (
          <ActionButton label="Delete message" onClick={onDelete} danger>
            <Trash2 className="size-3" />
          </ActionButton>
        ) : null}
      </div>
    </div>
  );
}

/** The Copilot message list — rich rendering (tool rows, build tree, markdown). */
export function ChatMessages({
  messages, busy, progress, error, className, onRetry,
  onConfirmTool, onCancelTool, onCompleteTool, confirmingKey, draft, liveReasoning,
  onEdit, onDelete, onDismissError,
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
  /** Answer text streamed so far (`useCopilot().draft`). */
  draft?: string;
  /** Reasoning streamed so far (`useCopilot().liveReasoning`). */
  liveReasoning?: string;
  /** Replace the user message at `index` and ask again from there. */
  onEdit?: (index: number, text: string) => void;
  /** Remove the message at `index`. */
  onDelete?: (index: number) => void;
  onDismissError?: () => void;
}) {
  const reduce = useReducedMotion() ?? false;
  // Agents' message entrance; skipped under reduced motion.
  const enter = reduce
    ? {}
    : { initial: { opacity: 0, y: 15 }, animate: { opacity: 1, y: 0 }, transition: { duration: 0.2, ease: "easeOut" as const } };
  const rootRef = React.useRef<HTMLDivElement>(null);
  // Changes whenever something new lands at the bottom of the list.
  const tail = `${messages.length}:${messages[messages.length - 1]?.content.length ?? 0}:${busy}:${error ?? ""}:${progress?.phase}:${progress?.steps?.length ?? 0}:${draft?.length ?? 0}:${liveReasoning?.length ?? 0}`;
  const { pinned, jumpToLatest } = useStickToBottom(rootRef, tail);
  const [editing, setEditing] = React.useState<{ index: number; text: string } | null>(null);

  const lastIndex = messages.length - 1;
  const steps = progress?.steps ?? [];

  return (
    <div ref={rootRef} className={cn("space-y-1", className)}>
      {messages.map((m, i) => {
        const key = m.id ?? `m${i}`;
        const isUser = m.role === "user";
        const isEditing = editing?.index === i;
        const pendingIndex = m.tools?.findIndex((t) => Boolean(asObj(t.result).needs_confirmation)) ?? -1;
        const pendingStep = pendingIndex >= 0 ? m.tools?.[pendingIndex] : undefined;
        return (
          <motion.div key={key} {...enter} className="py-3">
            <div className={cn("group flex gap-3", isUser && "justify-end")}>
              <div className={cn("min-w-0 flex-1", isUser && "ml-6")}>

                {isEditing ? (
                  <div className="space-y-2">
                    <Textarea
                      aria-label="Edit message"
                      value={editing.text}
                      onChange={(e) => setEditing({ index: i, text: e.target.value })}
                      className="min-h-[60px] bg-background"
                      autoFocus
                    />
                    <div className="flex justify-end gap-2">
                      <Button
                        size="sm"
                        disabled={!editing.text.trim() || busy}
                        onClick={() => {
                          onEdit?.(i, editing.text);
                          setEditing(null);
                        }}
                      >
                        Save &amp; Resend
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setEditing(null)}>
                        Cancel
                      </Button>
                    </div>
                  </div>
                ) : isUser ? (
                  <div className="ml-auto w-fit max-w-full rounded-2xl bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] px-4 py-2.5 text-sm whitespace-pre-wrap text-foreground">
                    {m.content}
                  </div>
                ) : (
                  <div className="text-sm">
                    {m.reasoning ? <ReasoningBox text={m.reasoning} ms={m.reasoningMs} /> : null}
                    {m.tools?.length ? (
                      <div className="mb-1 space-y-0.5">
                        {m.tools.map((t, j) => (
                          <ToolStepCard key={j} step={t} />
                        ))}
                      </div>
                    ) : null}
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
                      <div className="mt-2">
                        <PendingActionCard
                          step={pendingStep}
                          confirming={confirmingKey === `${i}:${pendingIndex}`}
                          onConfirm={() => onConfirmTool?.(i, pendingIndex)}
                          onCancel={() => onCancelTool?.(i, pendingIndex)}
                          onSavedInBuilder={(result) => onCompleteTool?.(i, pendingIndex, result)}
                        />
                      </div>
                    ) : null}
                    {m.chartCreated ? (
                      <Link
                        href="/dashboards"
                        className={cn(buttonVariants({ variant: "outline", size: "sm" }), "mt-2 gap-1.5")}
                      >
                        <BarChart3 className="size-4" /> Open Dashboards
                      </Link>
                    ) : null}
                  </div>
                )}

                {!isEditing && (m.content || m.stopped) ? (
                  <MessageFooter
                    msg={m}
                    isUser={isUser}
                    onEdit={isUser && onEdit && !busy ? () => setEditing({ index: i, text: m.content }) : undefined}
                    onRegenerate={!isUser && i === lastIndex && onRetry && !busy ? onRetry : undefined}
                    onDelete={onDelete && !busy ? () => onDelete(i) : undefined}
                  />
                ) : null}
              </div>
            </div>
          </motion.div>
        );
      })}

      {busy ? (
        <div className="py-3">
          <div className="flex gap-3">
            <div className="min-w-0 flex-1">
              {liveReasoning ? <ReasoningBox text={liveReasoning} live={!draft} /> : null}
              {steps.length ? (
                <div className="mb-1 space-y-0.5">
                  {steps.map((t, j) =>
                    progress?.phase === "tool" && j === steps.length - 1 ? (
                      <RunningToolRow key={`${t}-${j}`} tool={t} />
                    ) : (
                      <FinishedToolRow key={`${t}-${j}`} tool={t} />
                    ),
                  )}
                </div>
              ) : null}
              {draft ? <StreamingAnswer text={draft} /> : <StatusPill progress={progress ?? null} />}
            </div>
          </div>
        </div>
      ) : null}

      <AnimatePresence>
        {error ? (
          <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -10 }}
            className="flex items-center gap-3 rounded-lg border border-destructive/20 bg-destructive/10 p-3"
            role="alert"
          >
            <AlertCircle className="size-5 shrink-0 text-destructive" />
            <p className="min-w-0 flex-1 text-sm text-destructive">{error}</p>
            {onRetry ? (
              <Button size="sm" variant="outline" onClick={onRetry}>
                <RefreshCw className="mr-1 size-4" /> Retry
              </Button>
            ) : null}
            {onDismissError ? (
              <Button size="icon" variant="ghost" className="size-7" aria-label="Dismiss error" onClick={onDismissError}>
                <X className="size-4" />
              </Button>
            ) : null}
          </motion.div>
        ) : null}
      </AnimatePresence>

      {!pinned ? (
        <div className="pointer-events-none sticky bottom-2 flex justify-end">
          <Button
            variant="secondary"
            size="icon"
            className="pointer-events-auto rounded-full shadow-lg"
            aria-label="Scroll to latest message"
            onClick={jumpToLatest}
          >
            <ArrowDown className="size-4" />
          </Button>
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
    <div className="space-y-1.5 text-sm" aria-live="polite" aria-busy="true">
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
 * to reread detaches it (a "scroll to latest" button appears); coming back
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

/** "3.2s", "1m 04s". */
function formatElapsed(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${String(Math.round(s % 60)).padStart(2, "0")}s`;
}
