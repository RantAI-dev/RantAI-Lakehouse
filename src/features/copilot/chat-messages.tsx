"use client";

import * as React from "react";
import Link from "next/link";
import { AlertCircle, BarChart3, Sparkles, User } from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { MiniMarkdown } from "./mini-markdown";
import { ToolStepCard, asObj } from "./tool-step";
import { BuildTree } from "./build-tree";
import { ChartDraftCard } from "./chart-draft-card";
import type { Msg } from "./use-copilot";

/** Avatar bulat untuk pesan — AI (gradasi violet) / user (netral). */
function Avatar({ ai }: { ai?: boolean }) {
  return (
    <span
      className={cn(
        "grid size-8 shrink-0 place-items-center rounded-lg border",
        ai
          ? "border-violet-500/20 bg-gradient-to-br from-violet-500/15 to-purple-600/15 text-violet-600 dark:text-violet-400"
          : "border-border bg-muted text-muted-foreground",
      )}
    >
      {ai ? <Sparkles className="size-4" /> : <User className="size-4" />}
    </span>
  );
}

/** Titik-titik "mengetik" (dipinjam dari pola RantAI-Agents). */
export function TypingDots({ className }: { className?: string }) {
  return (
    <div className={cn("flex items-center gap-1", className)}>
      <span className="sr-only">Copilot is typing…</span>
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/50 [animation-delay:-0.3s]" />
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/50 [animation-delay:-0.15s]" />
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/50" />
    </div>
  );
}

function getConfirmationSummary(res: Record<string, unknown>, rawArgs: Record<string, unknown>): string {
  if (typeof res.summary === "string" && !res.summary.includes('""')) {
    return res.summary;
  }
  const rawTitle = rawArgs.title ?? rawArgs.caption ?? rawArgs.name ?? rawArgs.text;
  const title = typeof rawTitle === "string" ? rawTitle : "";
  if (title) {
    const kind = typeof rawArgs.kind === "string" ? rawArgs.kind : "chart";
    const mart = typeof rawArgs.mart === "string" ? rawArgs.mart : "data";
    return `Create new ${kind} chart "${title}" from mart ${mart}.`;
  }
  return "Do you want to confirm this action?";
}

/** Daftar pesan Copilot — render kaya (tool cards, pohon build, markdown). */
export function ChatMessages({
  messages, busy, error, className, onConfirmTool, onCancelTool, onCompleteTool, confirmingKey,
}: {
  messages: Msg[];
  busy: boolean;
  error?: string | null;
  className?: string;
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
  }, [messages, busy]);

  return (
    <div className={cn("space-y-4", className)}>
      {messages.map((m, i) =>
        m.role === "user" ? (
          <div key={i} className="flex flex-row-reverse gap-3">
            <Avatar />
            <div className="max-w-[80%] whitespace-pre-wrap rounded-2xl rounded-tr-sm bg-primary px-3.5 py-2 text-sm text-primary-foreground">
              {m.content}
            </div>
          </div>
        ) : (
          <div key={i} className="flex gap-3">
            <Avatar ai />
            <div className="min-w-0 flex-1 space-y-2 pt-0.5">
              {m.tools && m.tools.length ? (
                <div className="space-y-1.5">
                  {m.tools.map((t, j) => (
                    <ToolStepCard key={j} step={t} />
                  ))}
                </div>
              ) : null}
              {m.buildRunId ? <BuildTree runId={m.buildRunId} /> : null}
              <MiniMarkdown text={m.content} />
              {(() => {
                const pendingToolIndex = m.tools?.findIndex((t) => Boolean(asObj(t.result).needs_confirmation));
                if (pendingToolIndex === undefined || pendingToolIndex === -1 || !m.tools?.[pendingToolIndex]) return null;
                const pendingStep = m.tools[pendingToolIndex];
                const res = asObj(pendingStep.result);
                const rawArgs = asObj(res.args ?? pendingStep.args);
                const summaryText = getConfirmationSummary(res, rawArgs);
                const key = `${i}:${pendingToolIndex}`;
                const isConfirming = confirmingKey === key;
                const toolName = typeof res.tool === "string" ? res.tool : pendingStep.tool;
                if (toolName === "create_chart") {
                  return (
                    <ChartDraftCard
                      args={rawArgs}
                      confirming={isConfirming}
                      onConfirm={() => onConfirmTool?.(i, pendingToolIndex)}
                      onCancel={() => onCancelTool?.(i, pendingToolIndex)}
                      onSavedInBuilder={(saved) =>
                        onCompleteTool?.(i, pendingToolIndex, {
                          created: true,
                          title: saved.title,
                          kind: saved.kind,
                          mart: saved.mart,
                          board: saved.board,
                          via: "builder",
                        })
                      }
                    />
                  );
                }
                return (
                  <div className="rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 space-y-2 text-xs">
                    <div className="flex items-center gap-1.5 font-medium text-amber-800 dark:text-amber-300">
                      <AlertCircle className="size-4 shrink-0" />
                      <span>Action requires confirmation</span>
                    </div>
                    <p className="text-foreground leading-relaxed">
                      {summaryText}
                    </p>
                    <div className="flex items-center gap-2 pt-1">
                      <Button
                        size="sm"
                        onClick={() => onConfirmTool?.(i, pendingToolIndex)}
                        disabled={isConfirming}
                      >
                        {isConfirming ? "Executing…" : "Confirm"}
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => onCancelTool?.(i, pendingToolIndex)}
                        disabled={isConfirming}
                      >
                        Cancel
                      </Button>
                    </div>
                  </div>
                );
              })()}
              {m.chartCreated ? (
                <div className="pt-1">
                  <Link
                    href="/dashboards"
                    onClick={() => {
                      try {
                        window.dispatchEvent(new Event("dashboards:changed"));
                        window.scrollTo({ top: document.body.scrollHeight, behavior: "smooth" });
                      } catch { /* ignore */ }
                    }}
                    className={cn(buttonVariants({ variant: "outline", size: "sm" }), "inline-flex items-center gap-1.5")}
                  >
                    <BarChart3 className="size-4" /> Open Dashboards
                  </Link>
                </div>
              ) : null}
            </div>
          </div>
        ),
      )}
      {busy ? (
        <div className="flex items-center gap-3">
          <Avatar ai />
          <div className="flex items-center gap-2 pt-2 text-sm text-muted-foreground">
            <TypingDots />
          </div>
        </div>
      ) : null}
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
      <div ref={endRef} />
    </div>
  );
}
