"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { motion, useReducedMotion, type Variants } from "motion/react"
import {
  AlertTriangle,
  ArrowUp,
  BarChart3,
  CheckCircle2,
  CircleHelp,
  FileCode2,
  GitBranch,
  MessageSquare,
  Plug,
  Plus,
  Sparkles,
} from "lucide-react"

import { buttonVariants } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuGroupLabel,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
import { useCopilot, type Mode } from "@/features/copilot/use-copilot"
import { DashboardPreview } from "@/features/dashboards/dashboard-preview"
import { readLastBoard } from "@/features/dashboards/last-board"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime, parseTimestamp } from "@/lib/format"
import { recentItems, type RecentItem, type RecentKind } from "@/lib/home-recent"
import { homeStatus, type CheckState, type HomeStatus } from "@/lib/home-status"
import { cn } from "@/lib/utils"
import {
  connectorService,
  dashboardService,
  overviewService,
  pipelineService,
  queryService,
} from "@/services"

/**
 * Home: "is anything wrong, and where did I leave off".
 *
 * Three blocks, and that number is the design:
 *
 * 1. **Status** — one line, the page's focal point: what needs attention
 *    across pipelines, alerts, sources and datasets, with the items
 *    themselves underneath when there are any (`lib/home-status`). It
 *    carries counts of problems only. Totals, throughput and trends are
 *    Monitoring → Health (`/health`); Home was a wall of platform metrics
 *    once and that moved there on purpose, so nothing here should grow
 *    back into one.
 * 2. **From <dashboard>** — the wide column: the first tiles of the
 *    dashboard last opened (`DashboardPreview`), from the same governed
 *    API as the canvas. Charts get the wide column because they need it.
 * 3. **Recent** — the narrow column: dashboards, conversations and saved
 *    queries as one list by time (`lib/home-recent`).
 *
 * Starting something new is the "+ New" menu in the header. It is
 * onboarding content, so it only becomes large cards on the page while
 * the workspace is still empty (no pipeline, no source, no dashboard of
 * your own).
 *
 * How it got here, so it is not undone by accident (QA feedback each
 * time): five equal-weight blocks had no focal point; one narrow column
 * left half the screen empty; then eleven bordered cards with two-line
 * rows, an audit-log feed and a start section were too much. The feed
 * (raw audit text, and `/activity` exists), the three per-kind cards, the
 * second line under every row and the start section were removed. A
 * "Pipeline runs" list went last: beside Recent it looked like a fourth
 * kind that had been left out of it for no visible reason, a failed
 * pipeline is already in the status card, and a healthy one is a green
 * "9m ago" that asks nothing of anyone (schedules and run history are on
 * `/pipelines`). It was not merged into Recent because jobs on a
 * 15-minute schedule would always be the newest rows and push the
 * person's own work down. Add a block only if it replaces one.
 *
 * Home and Ask AI (`/copilot`) do different jobs: Home asks one question
 * through a small box that opens the answer in Ask AI; the full composer
 * (Ask/Build, Tools), starters and the conversation list live there.
 *
 * AI is a copilot here, not the default path for changes: a "+ New" item
 * opens the manual flow, and handing the task to Copilot is a separate,
 * explicit item. Copilot can do these things, but a person should see the
 * form before a source or pipeline exists.
 *
 * Everything is read from real services, and a read that fails says so:
 * the status line never reports "all clear" unless every read succeeded,
 * and a list that cannot load says that instead of looking empty.
 *
 * Whether an LLM is configured is not known up front (no endpoint reports
 * it); when it is not, `POST /api/ai/chat` answers 503 and the Copilot
 * page shows that error.
 *
 * The look is deliberately plain: page background, bordered cards, hover
 * as a light background change, one short fade-in that is dropped under
 * `prefers-reduced-motion`.
 */
export function HomePage() {
  const router = useRouter()
  const { user } = useAuth()
  const copilot = useCopilot()
  const reduce = useReducedMotion() ?? false
  const [pending, setPending] = React.useState<{
    text: string
    mode: Mode
  } | null>(null)

  // `send` closes over the current mode, so a prompt that needs Build mode
  // waits one render for `setMode` to land before it is sent.
  const { mode, send } = copilot
  React.useEffect(() => {
    if (!pending || mode !== pending.mode) return
    setPending(null)
    void send(pending.text, [])
  }, [pending, mode, send])

  const ask = React.useCallback(
    (text: string, wanted: Mode = "ask") => {
      const q = text.trim()
      if (!q) return
      copilot.newChat()
      copilot.setMode(wanted)
      setPending({ text: q, mode: wanted })
      router.push("/copilot")
    },
    [copilot, router],
  )

  const pipelines = useService(
    (signal) => pipelineService.listPipelines(signal),
    [],
  )
  const alerts = useService((signal) => overviewService.listAlerts(signal), [])
  const connectors = useService(
    (signal) => connectorService.listConnectors(signal),
    [],
  )
  const boards = useService((signal) => dashboardService.listBoards(signal), [])
  const saved = useService((signal) => queryService.listSaved(signal), [])
  const summary = useService((signal) => overviewService.getSummary(signal), [])

  // Read after mount: localStorage is not there during the server render.
  const [lastBoard, setLastBoard] = React.useState<string | null>(null)
  React.useEffect(() => setLastBoard(readLastBoard()), [])

  // The whole display name: the first word alone read "Good morning,
  // Bootstrap" for the "Bootstrap Admin" account.
  const displayName = user?.name?.trim()

  const failing = (pipelines.data?.pipelines ?? []).filter(
    (p) => p.status === "failed" || p.status === "degraded",
  )
  const open = (alerts.data ?? []).filter((a) => a.status === "open")
  const unhealthy = (connectors.data ?? []).filter(
    (c) => c.health === "unhealthy" || c.health === "degraded",
  )

  // Catalog datasets with no synced rows (`staleAssets`): missing data,
  // not old data. See `lib/home-status`.
  const noData = summary.data?.staleAssets ?? 0

  const attention: AttentionItem[] = [
    ...failing.map((p) => ({
      key: `p:${p.id}`,
      kind: "Pipeline",
      label: p.name,
      state: p.status,
      href: `/pipelines/${encodeURIComponent(p.id)}`,
      prompt: `Why is pipeline "${p.name}" ${p.status}? Look at its recent runs and explain.`,
    })),
    ...open.map((a) => ({
      key: `a:${a.id}`,
      kind: "Alert",
      label: a.title,
      state: "open",
      href: "/alerts",
      prompt: `Explain the open alert "${a.title}" (${a.affected}) and what I should check.`,
    })),
    ...unhealthy.map((c) => ({
      key: `c:${c.id}`,
      kind: "Source",
      label: c.name,
      state: c.health,
      href: "/connectors",
      prompt: `Source "${c.name}" is ${c.health}. Test it and tell me what is wrong.`,
    })),
    // One row for all of them: the overview reports a count, not which.
    ...(noData > 0
      ? [
          {
            key: "datasets",
            kind: "Catalog",
            label: `${noData} ${noData === 1 ? "dataset" : "datasets"}`,
            state: "without synced data",
            href: "/catalog",
            prompt:
              "Which catalog datasets have no synced rows yet, and what feeds them?",
          },
        ]
      : []),
  ]

  const status = homeStatus({
    pipelines: { state: checkState(pipelines.status), count: failing.length },
    alerts: { state: checkState(alerts.status), count: open.length },
    sources: { state: checkState(connectors.status), count: unhealthy.length },
    datasets: { state: checkState(summary.status), count: noData },
  })

  // The preview follows "where I left off": the board last opened, else
  // the most recently changed; the first of those that has tiles, so an
  // empty board just opened does not blank the section.
  const previewBoard = [...(boards.data ?? [])]
    .sort((a, b) => {
      if (a.id === lastBoard) return -1
      if (b.id === lastBoard) return 1
      return time(b.updatedAt) - time(a.updatedAt)
    })
    .find((b) => (b.chartCount ?? 0) > 0)

  const recent = recentItems(
    {
      dashboard: (boards.data ?? []).map((b) => ({
        id: b.id,
        title: b.name,
        href: `/dashboards/${encodeURIComponent(b.id)}`,
        at: b.updatedAt,
      })),
      conversation: copilot.sessions.map((s) => ({
        id: s.id,
        title: s.title || "Untitled conversation",
        href: `/copilot?id=${encodeURIComponent(s.id)}`,
        at: s.updatedAt,
      })),
      query: (saved.data ?? []).map((q) => ({
        id: q.id,
        title: q.title,
        href: `/query-studio?saved=${encodeURIComponent(q.id)}`,
        at: q.updatedAt,
      })),
    },
    lastBoard,
  )

  // Onboarding gets the large cards only while there is nothing yet. All
  // three reads must have succeeded: an error is not an empty workspace.
  const emptyWorkspace =
    pipelines.status === "success" &&
    connectors.status === "success" &&
    boards.status === "success" &&
    (pipelines.data?.pipelines ?? []).length === 0 &&
    (connectors.data ?? []).length === 0 &&
    (boards.data ?? []).every((b) => b.builtin)

  const starts = START.map((s) => ({
    ...s,
    onAi: () => ask(s.aiPrompt, "build"),
  }))

  const recentList = (
    <RecentList
      loading={boards.status === "loading" || saved.status === "loading"}
      failed={[
        boards.status === "error" ? "dashboards" : null,
        saved.status === "error" ? "saved queries" : null,
      ].filter((x): x is string => x !== null)}
      items={recent}
    />
  )

  return (
    <motion.div
      variants={reduce ? undefined : FADE}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-6xl flex-col gap-6 py-2 sm:py-6"
    >
      <header className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-6">
        <h1 className="min-w-0 text-xl font-semibold tracking-tight text-foreground sm:text-2xl">
          {greeting()}
          {displayName ? `, ${displayName}` : ""}
        </h1>
        <div className="flex min-w-0 items-center gap-2">
          <AskBar onAsk={(q) => ask(q, "ask")} />
          <NewMenu items={starts} />
        </div>
      </header>

      <StatusCard
        status={status}
        items={attention}
        onAsk={(prompt) => ask(prompt)}
      />

      {emptyWorkspace ? (
        <section className="flex flex-col gap-3">
          <SectionTitle>Get started</SectionTitle>
          <div className="grid gap-3 sm:grid-cols-3">
            {starts.map((s) => (
              <StartCard key={s.title} {...s} />
            ))}
          </div>
        </section>
      ) : null}

      {/* The wide column is for the charts, which need the room; Recent
          is a line of text per row and sits in the narrow one. With no
          dashboard to preview, Recent takes the width instead of leaving
          the wide column empty. */}
      {previewBoard ? (
        <div className="grid min-w-0 grid-cols-1 items-start gap-6 lg:grid-cols-3">
          <div className="min-w-0 lg:col-span-2">
            <DashboardPreview boardId={previewBoard.id} name={previewBoard.name} />
          </div>
          {recentList}
        </div>
      ) : (
        recentList
      )}
    </motion.div>
  )
}

const FADE: Variants = {
  hidden: { opacity: 0, y: 6 },
  show: { opacity: 1, y: 0, transition: { duration: 0.25, ease: "easeOut" } },
}

/** `useService`'s status as the three states the status line knows. */
function checkState(status: string): CheckState {
  if (status === "success") return "ok"
  return status === "error" ? "error" : "loading"
}

/** Sort key for a timestamp; missing or unparseable ones sort last. */
function time(iso: string | undefined): number {
  if (!iso) return 0
  const t = parseTimestamp(iso).getTime()
  return Number.isNaN(t) ? 0 : t
}

/** Manual first; `aiPrompt` is what "Let AI do it" hands to Copilot in Build mode. */
const START = [
  {
    icon: Plug,
    title: "Connect a source",
    description:
      "Register a database, stream or bucket. Credentials stay as secret references.",
    href: "/connectors/create",
    aiPrompt: "Guide me through connecting a new data source.",
  },
  {
    icon: GitBranch,
    title: "Create a pipeline",
    description:
      "Choose a source and a target layer, then review before it runs.",
    href: "/pipelines/create",
    aiPrompt:
      "Help me create a new pipeline. Ask me what to ingest and where it should land.",
  },
  {
    icon: BarChart3,
    title: "Build a dashboard",
    description:
      "Pick a mart and chart it. Every chart is added only after you confirm.",
    href: "/dashboards",
    aiPrompt: "Suggest a dashboard for the data we have.",
  },
]

type StartProps = {
  icon: React.ComponentType<{ className?: string }>
  title: string
  description: string
  href: string
  onAi: () => void
}

const TONE: Record<
  HomeStatus["tone"],
  { icon: React.ComponentType<{ className?: string }>; iconCls: string; cardCls: string }
> = {
  attention: {
    icon: AlertTriangle,
    iconCls: "bg-destructive/10 text-destructive",
    cardCls: "border-destructive/30",
  },
  unknown: {
    icon: CircleHelp,
    iconCls: "bg-amber-500/10 text-amber-600 dark:text-amber-400",
    cardCls: "border-amber-500/30",
  },
  clear: {
    icon: CheckCircle2,
    iconCls: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
    cardCls: "border-border",
  },
  loading: { icon: CircleHelp, iconCls: "bg-muted text-muted-foreground", cardCls: "border-border" },
}

/**
 * The status line and, when something needs attention, the items behind
 * it. One card, so the verdict and its evidence are read together.
 */
function StatusCard({
  status,
  items,
  onAsk,
}: {
  status: HomeStatus
  items: AttentionItem[]
  onAsk: (prompt: string) => void
}) {
  if (status.tone === "loading") {
    return (
      <section
        aria-label="Lakehouse status"
        className="flex items-center gap-3 rounded-xl border border-border bg-card p-4"
      >
        <Skeleton className="size-9 rounded-full" />
        <div className="grid flex-1 gap-2">
          <Skeleton className="h-4 w-52" />
          <Skeleton className="h-3 w-80 max-w-full" />
        </div>
      </section>
    )
  }
  const tone = TONE[status.tone]
  const Icon = tone.icon
  const shown = items.slice(0, 5)
  return (
    <section
      aria-label="Lakehouse status"
      className={cn("rounded-xl border bg-card", tone.cardCls)}
    >
      <div className="flex items-center gap-3 p-4">
        <span
          className={cn(
            "grid size-9 shrink-0 place-items-center rounded-full",
            tone.iconCls,
          )}
        >
          <Icon className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-base font-semibold text-foreground">
            {status.headline}
          </p>
          <p className="text-sm text-muted-foreground">
            {status.parts.join(" · ")}
          </p>
        </div>
        <Link
          href="/health"
          className="hidden shrink-0 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline sm:inline"
        >
          Health details
        </Link>
      </div>
      {shown.length > 0 ? (
        <ul className="flex flex-col border-t border-border p-2">
          {shown.map((it) => (
            <li
              key={it.key}
              className="flex items-center gap-3 rounded-lg px-2 py-2 transition-colors hover:bg-muted/60"
            >
              <span className="size-2 shrink-0 rounded-full bg-destructive" />
              <Link
                href={it.href}
                className="min-w-0 flex-1 text-sm underline-offset-4 hover:underline"
              >
                <span className="text-muted-foreground">{it.kind} · </span>
                <span className="font-medium">{it.label}</span>
                <span className="text-muted-foreground"> is {it.state}</span>
              </Link>
              <AiButton onClick={() => onAsk(it.prompt)}>Ask AI</AiButton>
            </li>
          ))}
          {items.length > shown.length ? (
            <li className="px-2 pt-1 text-xs text-muted-foreground">
              and {items.length - shown.length} more
            </li>
          ) : null}
        </ul>
      ) : null}
    </section>
  )
}

const RECENT_ICON: Record<
  RecentKind,
  { icon: React.ComponentType<{ className?: string }>; label: string }
> = {
  dashboard: { icon: BarChart3, label: "Dashboard" },
  conversation: { icon: MessageSquare, label: "Conversation" },
  query: { icon: FileCode2, label: "Saved query" },
}

/** Dashboards, conversations and saved queries as one list (`recentItems`). */
function RecentList({
  loading,
  failed,
  items,
}: {
  loading: boolean
  failed: string[]
  items: RecentItem[]
}) {
  return (
    <section className="flex min-w-0 flex-col gap-3">
      <SectionTitle>Recent</SectionTitle>
      <div className="rounded-xl border border-border bg-card p-2">
        {loading ? (
          <div className="flex flex-col gap-2 p-2">
            <Skeleton className="h-5 w-full" />
            <Skeleton className="h-5 w-4/5" />
            <Skeleton className="h-5 w-full" />
          </div>
        ) : items.length === 0 && failed.length === 0 ? (
          <p className="px-2 py-2 text-xs text-muted-foreground">
            Nothing yet. Dashboards, conversations and saved queries you work
            on show up here.
          </p>
        ) : (
          <ul className="flex flex-col">
            {items.map((it) => {
              const { icon: Icon, label } = RECENT_ICON[it.kind]
              return (
                <li key={`${it.kind}:${it.id}`}>
                  <Link
                    href={it.href}
                    className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm transition-colors hover:bg-muted/60"
                  >
                    <span title={label} className="shrink-0 text-muted-foreground">
                      <Icon className="size-4" />
                      <span className="sr-only">{label}</span>
                    </span>
                    <span className="min-w-0 flex-1 truncate">{it.title}</span>
                    <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                      {it.lastOpened
                        ? "last opened"
                        : it.at
                          ? formatRelativeTime(it.at)
                          : ""}
                    </span>
                  </Link>
                </li>
              )
            })}
          </ul>
        )}
        {!loading && failed.length > 0 ? (
          <p className="px-2 pt-1 pb-1 text-xs text-muted-foreground">
            Couldn&apos;t load {failed.join(" and ")}.
          </p>
        ) : null}
        <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 border-t border-border px-2 pt-2 pb-1 text-xs text-muted-foreground">
          <Link href="/dashboards/browse" className="underline-offset-4 hover:text-foreground hover:underline">
            Dashboards
          </Link>
          <Link href="/copilot/history" className="underline-offset-4 hover:text-foreground hover:underline">
            Conversations
          </Link>
          <Link href="/query-studio/saved" className="underline-offset-4 hover:text-foreground hover:underline">
            Saved queries
          </Link>
        </div>
      </div>
    </section>
  )
}

/**
 * "+ New": the start items as a menu. Manual flows first; handing the
 * same task to Copilot (Build mode) is its own group, never the default.
 */
function NewMenu({ items }: { items: StartProps[] }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <button
            type="button"
            className={cn(buttonVariants({ variant: "outline", size: "sm" }), "h-9 shrink-0 gap-1.5")}
          />
        }
      >
        <Plus className="size-4" />
        New
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        <DropdownMenuGroup>
          {items.map(({ icon: Icon, title, href }) => (
            <DropdownMenuItem key={title} render={<Link href={href} />}>
              <Icon />
              {title}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuGroupLabel>Or let AI do it</DropdownMenuGroupLabel>
          {items.map(({ title, onAi }) => (
            <DropdownMenuItem key={title} onClick={onAi}>
              <Sparkles />
              {title}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * Home's question box: one small line, Ask mode, sent into Ask AI. Build
 * mode and the Tools menu are on Ask AI's own composer, not here.
 */
function AskBar({ onAsk }: { onAsk: (q: string) => void }) {
  const [draft, setDraft] = React.useState("")

  return (
    <form
      className="relative min-w-0 flex-1 sm:w-72 sm:flex-none"
      onSubmit={(e) => {
        e.preventDefault()
        if (draft.trim()) onAsk(draft)
      }}
    >
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="Ask AI about your data…"
        aria-label="Ask AI"
        title="Opens the answer in Ask AI"
        className="block h-9 w-full rounded-lg border border-border bg-background pr-10 pl-3 text-sm text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-foreground/30"
      />
      <button
        type="submit"
        aria-label="Ask AI"
        disabled={!draft.trim()}
        className="absolute top-1/2 right-1 grid size-7 -translate-y-1/2 place-items-center rounded-md bg-primary text-primary-foreground transition-colors hover:bg-primary/85 disabled:bg-transparent disabled:text-muted-foreground"
      >
        <ArrowUp className="size-4" />
      </button>
    </form>
  )
}

/**
 * A start card (empty workspace only): the card itself opens the manual
 * flow; "Let AI do it" is a separate button that hands the same task to
 * Copilot in Build mode.
 */
function StartCard({ icon: Icon, title, description, href, onAi }: StartProps) {
  return (
    <div className="flex flex-col rounded-xl border border-border bg-card transition-colors hover:border-foreground/20">
      <Link
        href={href}
        className="flex flex-1 flex-col gap-3 rounded-t-xl p-5 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
      >
        <span className="inline-flex size-9 items-center justify-center rounded-lg border border-border bg-muted/40 text-foreground">
          <Icon className="size-4" />
        </span>
        <span className="text-sm font-semibold">{title}</span>
        <span className="text-xs leading-relaxed text-muted-foreground">
          {description}
        </span>
      </Link>
      <div className="flex items-center justify-between border-t border-border px-5 py-2.5">
        <span className="text-[11px] text-muted-foreground">
          Or hand it to Copilot
        </span>
        <AiButton onClick={onAi}>Let AI do it</AiButton>
      </div>
    </div>
  )
}

/** The small "hand it to Copilot" button used on cards and attention rows. */
function AiButton({
  onClick,
  children,
}: {
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      // `--brand-1`, not `text-primary`: primary is too dark to read on the
      // dark theme's card background.
      className="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-[var(--brand-1)] transition-colors hover:bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
    >
      <Sparkles className="size-3" />
      {children}
    </button>
  )
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="px-1 text-xs font-semibold tracking-[0.08em] text-muted-foreground uppercase">
      {children}
    </h2>
  )
}

type AttentionItem = {
  key: string
  kind: string
  label: string
  state: string
  href: string
  prompt: string
}

function greeting(): string {
  const h = new Date().getHours()
  if (h < 12) return "Good morning"
  if (h < 18) return "Good afternoon"
  return "Good evening"
}
