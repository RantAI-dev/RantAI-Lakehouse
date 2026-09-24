"use client";

import * as React from "react";
import Link from "next/link";
import { AlertCircle, BarChart3, RotateCcw, Sparkles } from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { BuildTree } from "./build-tree";
import { CopyButton } from "./copy-button";
import { MiniMarkdown } from "./mini-markdown";
import { PendingActionCard } from "./pending-action-card";
import { TOOL_LABEL, ToolStepCard, asObj } from "./tool-step";
import type { ChatProgress, Msg } from "./use-copilot";

/** What Copilot is doing right now, with the time it has taken so far. */
function ProgressLine({ progress }: { progress: ChatProgress | null }) {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  const label =
    progress?.phase === "tool" && progress.tool
      ? `Running ${TOOL_LABEL[progress.tool] ?? progress.tool}…`
      : "Thinking…";
  const seconds = progress ? Math.max(0, Math.floor((now - progress.startedAt) / 1000)) : 0;
  return (
    <div className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
      <Sparkles className="size-4 animate-pulse text-violet-600 dark:text-violet-400" aria-hidden />
      <span>{label}</span>
      {seconds >= 2 ? <span className="text-xs tabular-nums">{seconds}s</span> : null}
    </div>
  );
}

/** The Copilot message list — rich rendering (tool cards, build tree, markdown). */
export function ChatMessages({
  messages, busy, progress, error, className, onRetry,
  onConfirmTool, onCancelTool, onCompleteTool, confirmingKey,
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
}) {
  const endRef = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, busy, error]);

  const lastIndex = messages.length - 1;

  return (
    <div className={cn("space-y-4", className)}>
      {messages.map((m, i) => {
        const key = m.id ?? `m${i}`;
        if (m.role === "user") {
          return (
            <div key={key} className="flex justify-end">
              <div className="max-w-[80%] whitespace-pre-wrap rounded-2xl rounded-br-sm bg-muted px-3.5 py-2 text-sm text-foreground">
                {m.content}
              </div>
            </div>
          );
        }
        const pendingIndex = m.tools?.findIndex((t) => Boolean(asObj(t.result).needs_confirmation)) ?? -1;
        const pendingStep = pendingIndex >= 0 ? m.tools?.[pendingIndex] : undefined;
        return (
          <div key={key} className="min-w-0 space-y-2">
            {m.tools?.length ? (
              <div className="space-y-1.5">
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
              <div className="flex items-center gap-1">
                <CopyButton text={m.content} label="Copy answer" />
              </div>
            ) : null}
          </div>
        );
      })}
      {busy ? <ProgressLine progress={progress ?? null} /> : null}
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
      <div ref={endRef} />
    </div>
  );
}
