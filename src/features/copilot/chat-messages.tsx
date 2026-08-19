"use client";

import * as React from "react";
import Link from "next/link";
import { BarChart3, Sparkles, User } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { MiniMarkdown } from "./mini-markdown";
import { ToolStepCard } from "./tool-step";
import { BuildTree } from "./build-tree";
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
      <span className="sr-only">Copilot mengetik…</span>
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/50 [animation-delay:-0.3s]" />
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/50 [animation-delay:-0.15s]" />
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/50" />
    </div>
  );
}

/** Daftar pesan Copilot — render kaya (tool cards, pohon build, markdown). */
export function ChatMessages({
  messages, busy, error, className,
}: {
  messages: Msg[];
  busy: boolean;
  error?: string | null;
  className?: string;
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
                  {m.tools.map((t, j) => <ToolStepCard key={j} step={t} />)}
                </div>
              ) : null}
              {m.buildRunId ? <BuildTree runId={m.buildRunId} /> : null}
              <MiniMarkdown text={m.content} />
              {m.chartCreated ? (
                <Button size="sm" variant="outline" render={<Link href="/dashboards" />}>
                  <BarChart3 className="size-4" /> Buka Dashboards
                </Button>
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
