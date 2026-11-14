"use client"

/**
 * Home's widgets (`WEB-17`, mockup B): the focal dashboard, Recent AI
 * chats, Pipelines and Needs attention. Each one shows only what an
 * existing API returns (spec "Data honesty"): no activity heatmap, no
 * month totals, no "Gold rows this month", no "opened N h ago", since no API
 * gives them. Each has a loading look in its own shape, an empty state
 * that offers the next step, and a one-line note when its read failed,
 * while the others still render.
 *
 * Anything that hands work to AI Copilot from here fills the composer and
 * focuses it; nothing is sent until the person sends it ("Ask why", the
 * empty states). The AI mark is `Bot`, as everywhere since `BI-61`.
 */
import * as React from "react"
import Link from "next/link"
import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  CircleHelp,
  GitBranch,
  LayoutDashboard,
  MessageSquare,
  Plug,
  Wrench,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { DashboardPreview } from "@/features/dashboards/dashboard-preview"
import type { SessionMeta, SessionsRead } from "@/features/copilot/use-copilot"
import { formatRelativeTime } from "@/lib/format"
import { FOCAL_REASON_LABEL, type FocalReason } from "@/lib/home-focal"
import { formatUntil, type PipelineRow } from "@/lib/home-pipelines"
import type { CheckState, HomeStatus } from "@/lib/home-status"
import { cn } from "@/lib/utils"
import { CardNote, CardSkeleton, EASE, FOCUS, ROW, WIDGET, WidgetHead, WidgetLink } from "./home-ui"

/* ------------------------------------------------------------------ */
/* Shared bits                                                         */
/* ------------------------------------------------------------------ */

/**
 * The small "hand it to AI Copilot" button on a widget row. `--brand-1`,
 * not `text-primary`: primary is too dark to read on the dark card.
 */
export function AskButton({ onClick, label, children }: { onClick: () => void; label: string; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1 rounded-md px-1.5 text-xs font-medium text-[var(--brand-1)] hover:bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] active:scale-[0.97]",
        EASE,
        FOCUS,
      )}
    >
      <Bot aria-hidden className="size-3.5" />
      {children}
    </button>
  )
}

/* ------------------------------------------------------------------ */
/* The focal dashboard                                                 */
/* ------------------------------------------------------------------ */

/** The focal widget's own loading look: its title row, meta line and the preview's tile grid. */
export function FocalSkeleton({ className }: { className?: string }) {
  return (
    <section aria-hidden className={cn(WIDGET, "@container/focal", className)}>
      <div className="flex h-6 items-center gap-1.5">
        <Skeleton className="size-3.5 rounded" />
        <Skeleton className="h-3 w-44" />
      </div>
      <div className="flex h-5 items-center">
        <Skeleton className="h-2.5 w-36" />
      </div>
      <div className="mt-3 grid min-h-0 flex-1 auto-rows-[minmax(150px,1fr)] grid-cols-1 gap-x-6 gap-y-4 @[480px]/focal:grid-cols-2">
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className={cn(i >= 2 ? "hidden @[480px]/focal:flex" : "flex", "min-w-0 flex-col gap-2")}>
            <div className="flex h-5 items-center">
              <Skeleton className="h-3" style={{ width: ["9rem", "11rem", "7.5rem", "10rem"][i] }} />
            </div>
            <Skeleton className="min-h-0 flex-1 rounded-lg" />
          </div>
        ))}
      </div>
    </section>
  )
}

export type FocalState =
  | { kind: "loading" }
  | { kind: "error" }
  | { kind: "empty"; hasBoards: boolean }
  | { kind: "board"; id: string; name: string; updatedAt?: string; reason: FocalReason }

/**
 * The one focal point under the composer: a dashboard drawn live with the
 * real tile renderer (`DashboardPreview`), the board `lib/home-focal` picks.
 */
export function FocalDashboard({
  state,
  onBuild,
  className,
}: {
  state: FocalState
  /** Fill the composer with a Build request for a first dashboard. */
  onBuild?: () => void
  className?: string
}) {
  if (state.kind === "loading") return <FocalSkeleton className={className} />
  if (state.kind === "error") {
    return (
      <section className={cn(WIDGET, className)}>
        <WidgetHead title="Dashboards" icon={LayoutDashboard} action={<WidgetLink href="/dashboards">Open</WidgetLink>} />
        <CardNote>Couldn&apos;t load your dashboards.</CardNote>
      </section>
    )
  }
  if (state.kind === "empty") {
    return (
      <section className={cn(WIDGET, className)}>
        <WidgetHead
          title="Dashboards"
          icon={LayoutDashboard}
          action={state.hasBoards ? <WidgetLink href="/dashboards">Open</WidgetLink> : undefined}
        />
        <div className="grid flex-1 content-center justify-items-center gap-3 px-6 py-8 text-center">
          {/* A dashboard's outline, drawn in the page's own greys: what will appear here. */}
          <div aria-hidden className="grid w-44 grid-cols-3 gap-1.5 opacity-70">
            <div className="h-8 rounded-md bg-muted" />
            <div className="h-8 rounded-md bg-muted" />
            <div className="h-8 rounded-md bg-muted" />
            <div className="col-span-2 flex h-16 items-end gap-1 rounded-md bg-muted p-1.5">
              {[30, 45, 40, 60, 72, 66].map((h, i) => (
                <div key={i} className="flex-1 rounded-[2px] bg-muted-foreground/20" style={{ height: `${h}%` }} />
              ))}
            </div>
            <div className="h-16 rounded-md bg-muted" />
          </div>
          <p className="text-sm leading-5 font-medium">
            {state.hasBoards ? "No dashboard has charts yet" : "No dashboards yet"}
          </p>
          <p className="max-w-64 text-[13px] leading-5 text-muted-foreground">
            Describe one and AI Copilot builds it, a chart at a time.
          </p>
          {onBuild ? (
            <Button onClick={onBuild} className="active:scale-[0.97]">
              <Bot data-icon="inline-start" />
              Build one with AI Copilot
            </Button>
          ) : null}
        </div>
      </section>
    )
  }
  const href = `/dashboards/${encodeURIComponent(state.id)}`
  return (
    <section className={cn(WIDGET, "@container/focal", className)}>
      <WidgetHead title={state.name} icon={LayoutDashboard} action={<WidgetLink href={href}>Open</WidgetLink>} />
      <p className="truncate text-xs leading-5 text-muted-foreground">
        {FOCAL_REASON_LABEL[state.reason]}
        {state.updatedAt ? ` · updated ${formatRelativeTime(state.updatedAt)}` : ""}
      </p>
      <DashboardPreview boardId={state.id} limit={4} className="mt-3 flex-1" />
    </section>
  )
}

/* ------------------------------------------------------------------ */
/* Recent AI chats                                                     */
/* ------------------------------------------------------------------ */

const MODE_ICON = { ask: MessageSquare, build: Wrench } as const
const MODE_TONE = {
  ask: "bg-sky-500/12 text-sky-700 dark:text-sky-300",
  build: "bg-indigo-500/10 text-indigo-700 dark:text-indigo-300",
} as const

/** Ask and Build as the same two soft squares wherever a conversation is listed. */
function ModeSquare({ mode }: { mode: string }) {
  const m = mode === "build" ? "build" : "ask"
  const Icon = MODE_ICON[m]
  return (
    <span aria-hidden className={cn("grid size-7 shrink-0 place-items-center rounded-lg", MODE_TONE[m])}>
      <Icon className="size-3.5" />
    </span>
  )
}

/**
 * The latest four conversations, newest first. The time is when each was
 * last updated, the only time the sessions API returns. "All chats" opens
 * the most recent conversation, whose page lists the rest (`WEB-18`); with
 * no chats there is nothing to open, so there is no link.
 */
export function RecentChats({
  sessions,
  read,
  examples,
  onTry,
  max = 4,
  className,
}: {
  sessions: readonly SessionMeta[]
  read: SessionsRead
  /** Starters for the empty state, from `homePrompts()`. */
  examples: readonly string[]
  onTry: (prompt: string) => void
  max?: number
  className?: string
}) {
  const shown = sessions.slice(0, max)
  const latest = sessions[0]
  return (
    <section className={cn(WIDGET, className)}>
      <WidgetHead
        title="Recent AI chats"
        icon={Bot}
        action={latest ? <WidgetLink href={`/copilot?id=${encodeURIComponent(latest.id)}`}>All chats</WidgetLink> : undefined}
      />
      <div className="mt-2 -mx-1.5">
        {read === "loading" && sessions.length === 0 ? (
          <div aria-hidden className="flex flex-col">
            {Array.from({ length: 3 }, (_, i) => (
              <div key={i} className="flex h-9 items-center gap-2.5 px-1.5">
                <Skeleton className="size-7 rounded-lg" />
                <Skeleton className={cn("h-3", i % 2 ? "w-1/2" : "w-2/3")} />
                <Skeleton className="ml-auto h-3 w-10" />
              </div>
            ))}
          </div>
        ) : read === "error" && sessions.length === 0 ? (
          <CardNote>Couldn&apos;t load your chats.</CardNote>
        ) : shown.length === 0 ? (
          <div className="flex flex-col">
            <CardNote>No chats yet. Try one:</CardNote>
            {examples.slice(0, 3).map((p) => (
              <button key={p} type="button" onClick={() => onTry(p)} className={cn(ROW, "h-9 w-full")}>
                <ModeSquare mode="ask" />
                <span className="min-w-0 flex-1 truncate text-[13px] leading-5">{p}</span>
              </button>
            ))}
          </div>
        ) : (
          <ul className="flex flex-col">
            {shown.map((s) => (
              <li key={s.id}>
                <Link href={`/copilot?id=${encodeURIComponent(s.id)}`} className={cn(ROW, "h-9")}>
                  <ModeSquare mode={s.mode} />
                  <span className="min-w-0 flex-1 truncate text-[13px] leading-5">{s.title || "Untitled conversation"}</span>
                  {s.updatedAt ? (
                    <span className="shrink-0 text-xs text-muted-foreground tabular-nums" title="Last updated">
                      {formatRelativeTime(s.updatedAt)}
                    </span>
                  ) : null}
                </Link>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  )
}

/* ------------------------------------------------------------------ */
/* Pipelines                                                           */
/* ------------------------------------------------------------------ */

const RUN_DOT: Record<string, string> = {
  failed: "bg-destructive",
  degraded: "bg-amber-500",
  completed: "bg-emerald-500",
  running: "bg-sky-500",
}

/**
 * Each pipeline's last status and when, one line each (`pipelineRows`:
 * failed first, then the most recently run). A failed one offers "Ask why",
 * which puts the question in the composer.
 */
export function PipelinesWidget({
  state,
  rows,
  hasPipelines,
  canConnect,
  onAskWhy,
  className,
}: {
  state: CheckState
  rows: readonly PipelineRow[]
  /** Pipelines exist, though none has run or is scheduled. */
  hasPipelines: boolean
  /** Whether "Connect a source" may be offered (the person can open it). */
  canConnect: boolean
  onAskWhy: (pipeline: string) => void
  className?: string
}) {
  return (
    <section className={cn(WIDGET, className)}>
      <WidgetHead
        title="Pipelines"
        icon={GitBranch}
        action={state === "ok" && hasPipelines ? <WidgetLink href="/pipelines">Open</WidgetLink> : undefined}
      />
      <div className="mt-2 -mx-1.5">
        {state === "loading" ? (
          <CardSkeleton rows={3} />
        ) : state === "error" ? (
          <CardNote>Couldn&apos;t load pipelines.</CardNote>
        ) : rows.length === 0 ? (
          <div className="flex flex-col items-start gap-2 px-1.5 py-1.5">
            <p className="text-xs leading-5 text-muted-foreground">
              {hasPipelines
                ? "No pipeline has run or is scheduled yet."
                : "No pipelines yet. Data arrives once a source is connected."}
            </p>
            {hasPipelines ? (
              <Button variant="secondary" render={<Link href="/pipelines" />} nativeButton={false}>
                <GitBranch data-icon="inline-start" />
                Open pipelines
              </Button>
            ) : canConnect ? (
              <Button variant="secondary" render={<Link href="/connectors/create" />} nativeButton={false}>
                <Plug data-icon="inline-start" />
                Connect a source
              </Button>
            ) : null}
          </div>
        ) : (
          <ul className="flex flex-col">
            {rows.map((r) => {
              const next = formatUntil(r.nextRunAt)
              const failed = r.status === "failed"
              const when = r.lastRunAt
                ? formatRelativeTime(r.lastRunAt)
                : next
                  ? `next ${next}`
                  : "not run yet"
              const sentence = r.lastRunAt
                ? `${r.name}: ${failed ? "failed" : r.status} ${formatRelativeTime(r.lastRunAt)}${next ? `, next ${next}` : ""}`
                : `${r.name}: ${next ? `next run ${next}` : "not run yet"}`
              return (
                <li key={r.id} className="flex items-center gap-1">
                  <Link href={`/pipelines/${encodeURIComponent(r.id)}`} title={sentence} className={cn(ROW, "h-9 flex-1")}>
                    <span aria-hidden className={cn("size-2 shrink-0 rounded-full", RUN_DOT[r.status] ?? "bg-muted-foreground/40")} />
                    <span className="sr-only">{r.status}: </span>
                    <span className="min-w-0 flex-1 truncate text-[13px] leading-5">{r.name}</span>
                    <span className={cn("shrink-0 text-xs tabular-nums", failed ? "text-destructive" : "text-muted-foreground")}>
                      {failed && r.lastRunAt ? `failed ${when}` : when}
                    </span>
                  </Link>
                  {failed ? (
                    <AskButton onClick={() => onAskWhy(r.name)} label={`Ask AI Copilot why ${r.name} failed`}>
                      Ask why
                    </AskButton>
                  ) : null}
                </li>
              )
            })}
          </ul>
        )}
      </div>
    </section>
  )
}

/* ------------------------------------------------------------------ */
/* Needs attention                                                     */
/* ------------------------------------------------------------------ */

export type AttentionItem = {
  key: string
  kind: string
  label: string
  state: string
  href: string
  prompt: string
}

const SHOWN_ITEMS = 3

/**
 * What needs a look: the checks behind the old status pill
 * (`lib/home-status`: pipelines, open alerts, unhealthy sources, datasets
 * with no data), only those this person may see. When every check passed it
 * is one calm line with the time it was checked, not an empty card; a check
 * that could not be read says so, never "all clear". When something needs
 * attention, the line names it and the alerts, sources and datasets behind
 * it are listed, each with "Ask". Failing pipelines are counted in the line
 * but listed in Pipelines, beside their "Ask why", not twice.
 */
export function NeedsAttention({
  status,
  items,
  checkedAt,
  onAsk,
  className,
}: {
  status: HomeStatus
  items: readonly AttentionItem[]
  checkedAt: Date | null
  onAsk: (prompt: string) => void
  className?: string
}) {
  const health = <WidgetLink href="/health">Health</WidgetLink>
  if (status.tone === "loading") {
    return (
      <section aria-label="Lakehouse status" aria-busy className={cn(WIDGET, "py-3", className)}>
        <div className="flex h-6 items-center gap-2">
          <Skeleton className="size-4 rounded-full" />
          <Skeleton className="h-3 w-40" />
        </div>
      </section>
    )
  }
  if (status.tone === "clear" || status.tone === "unknown") {
    const clear = status.tone === "clear"
    const Icon = clear ? CheckCircle2 : CircleHelp
    const time = checkedAt ? checkedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : null
    return (
      <section aria-label="Lakehouse status" className={cn(WIDGET, "py-3", className)}>
        <div className="flex h-6 min-w-0 items-center gap-2">
          <Icon
            aria-hidden
            className={cn("size-4 shrink-0", clear ? "text-emerald-600 dark:text-emerald-400" : "text-amber-600 dark:text-amber-400")}
          />
          <p className="min-w-0 flex-1 truncate text-[13px] leading-5">
            {/* What was checked is the tooltip, not phrases to read on every visit. */}
            <span className="font-medium" title={status.parts.join(" · ")}>{status.headline}</span>
            <span className="text-muted-foreground">
              {clear
                ? time
                  ? ` · checked ${time}`
                  : ""
                : ` · ${status.parts.filter((p) => p.startsWith("could not")).join(" · ")}`}
            </span>
          </p>
          {health}
        </div>
      </section>
    )
  }
  const shown = items.slice(0, SHOWN_ITEMS)
  return (
    <section aria-label="Lakehouse status" className={cn(WIDGET, className)}>
      <div className="flex h-6 min-w-0 items-center gap-2">
        <AlertTriangle aria-hidden className="size-4 shrink-0 text-destructive" />
        <h2 className="min-w-0 flex-1 truncate text-[13px] leading-5 font-medium tracking-normal">Needs attention</h2>
        {health}
      </div>
      <p className="truncate text-xs leading-5 text-muted-foreground" title={status.parts.join(" · ")}>
        {status.parts.join(" · ")}
      </p>
      {shown.length > 0 ? (
        <ul className="mt-2 -mx-1.5 flex flex-col">
          {shown.map((it) => (
            <li key={it.key} className="flex items-center gap-1">
              <Link href={it.href} className={cn(ROW, "h-9 flex-1")} title={`${it.kind} · ${it.label} is ${it.state}`}>
                <span aria-hidden className="size-2 shrink-0 rounded-full bg-destructive" />
                <span className="min-w-0 flex-1 truncate text-[13px] leading-5">
                  <span className="text-muted-foreground">{it.kind} · </span>
                  {it.label}
                  <span className="text-muted-foreground"> is {it.state}</span>
                </span>
              </Link>
              <AskButton onClick={() => onAsk(it.prompt)} label={`Ask AI Copilot about ${it.label}`}>
                Ask
              </AskButton>
            </li>
          ))}
          {items.length > shown.length ? (
            <li className="px-1.5 pt-1 text-xs text-muted-foreground">and {items.length - shown.length} more</li>
          ) : null}
        </ul>
      ) : null}
    </section>
  )
}
