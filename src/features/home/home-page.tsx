"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { motion, useReducedMotion, type Variants } from "motion/react"
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  CircleHelp,
  MessageSquare,
  Sparkles,
} from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
import { ChatComposer } from "@/features/copilot/chat-composer"
import { useCopilot, type Mode } from "@/features/copilot/use-copilot"
import { DashboardPreview } from "@/features/dashboards/dashboard-preview"
import { readLastBoard } from "@/features/dashboards/last-board"
import { useService } from "@/hooks/use-service"
import { parseTimestamp } from "@/lib/format"
import { cardRead } from "@/lib/home-cards"
import {
  CARD_IDS,
  DEFAULT_SHORTCUTS,
  hiddenIds,
  SHORTCUT_IDS,
  WIDE_CARD,
  type CardId,
} from "@/lib/home-layout"
import { recentItems } from "@/lib/home-recent"
import { pipelineRows } from "@/lib/home-pipelines"
import { homePrompts } from "@/lib/home-prompts"
import { homeStatus, type CheckState, type HomeStatus } from "@/lib/home-status"
import { cn } from "@/lib/utils"
import {
  connectorService,
  dashboardService,
  overviewService,
  pipelineService,
  queryService,
} from "@/services"
import {
  CustomizeButton,
  EditableCards,
  EditableShortcuts,
  EditBar,
  EmptyHome,
  gridClass,
  PreviewPicker,
} from "./home-customize"
import { SHORTCUTS } from "./home-shortcuts"
import { OpenAlertsCard } from "./open-alerts-card"
import { PipelineRuns } from "./pipeline-runs-card"
import { RecentList } from "./recent-card"
import { SavedQueriesCard } from "./saved-queries-card"
import { SourcesCard } from "./sources-card"
import { StartButton } from "./start-button"
import { useHomeLayout } from "./use-home-layout"

/**
 * Home: where a conversation with Copilot starts, with what needs you and
 * where you left off underneath.
 *
 * The console is AI-driven, so the first thing on the page is the Copilot
 * composer (Ask/Build, Tools). It is the only place a conversation is
 * started: sending opens it at `/copilot?id=…`, and old conversations are
 * on History. There used to be a second, near-identical composer on an
 * "Ask AI" page; an earlier fix made Home's small to tell the two apart,
 * which treated the symptom. Now there is one.
 *
 * Two screens in the page's own scroller, with a soft snap between them:
 * the hero (1 and 2) centred on the first, and the cards (3 to 5) on the
 * second, whose top shows under the first as the cue that it is there.
 *
 * 1. **Composer and starters** — the focal point. Under the composer, a
 *    row of up to three bordered buttons for the things to create (the label =
 *    the manual flow, the sparkle = hand it to Copilot, for the ones that
 *    have a prompt), then two
 *    questions for Copilot as plain lines. The composer's placeholder cycles through further
 *    example prompts
 *    (`lib/home-prompts`: what is true of this workspace first, a failed
 *    pipeline or the dashboard last opened, then generic starters); the
 *    placeholder shows the ones the chips do not. Deliberately a box of a few rows,
 *    not a half-screen hero: the hero this page once had pushed everything
 *    else below the fold.
 * 2. **Status** — on the greeting's line, right-aligned, above the
 *    composer, so a
 *    failure is read before typing: a quiet "All clear", or what needs
 *    attention across pipelines, alerts, sources and datasets
 *    (`lib/home-status`), written out in its colour, with
 *    "Details" opening the items and an "Ask AI" for each. It carries
 *    counts of problems only. Totals, throughput and trends are
 *    Monitoring → Health (`/health`); Home was a wall of platform metrics
 *    once and that moved there on purpose, so nothing here should grow
 *    back into one.
 * 3. **From <dashboard>** — the wide column: the first tiles of the
 *    dashboard last opened, or the one chosen in Customize
 *    (`DashboardPreview`), from the same governed API as the canvas.
 * 4. **Recent** — the narrow column: dashboards, conversations and saved
 *    queries as one list by time (`lib/home-recent`).
 * 5. **Pipeline runs** — under Recent: when each pipeline last ran
 *    (`lib/home-pipelines`). It was removed while Home was one screen
 *    (beside Recent it looked like a fourth kind left out of it, and the
 *    page was crowded) and came back with the second screen, which has
 *    the room and is where "what is going on" belongs.
 *
 * Three more cards exist but are off by default, for whoever wants them:
 * **Sources** (connectors, the unhealthy first), **Open alerts** (the same
 * list the status line counts, most severe first) and **Saved queries**
 * (opening in Query Studio). They read what the page already loads, so
 * they add no request, and a read that is refused (403) says "you don't
 * have access", not "nothing here".
 *
 * **The layout is the person's.** Which cards and which shortcuts show, and
 * in what order, is saved per user on the server (`/api/home/layout`, so it
 * follows them across browsers) and edited in place: "Customize" at the top
 * right of the second screen turns each card and shortcut into something
 * that can be dragged (mouse, touch or keyboard) or hidden, with "Add card"
 * and "Add shortcut" for the hidden ones, a choice of dashboard for the
 * preview, and a bar with Done (save), Cancel and Reset to default. The
 * catalogue of ids, the defaults and the rule for reading a saved layout are
 * `lib/home-layout`; the server only checks shape and size, so a card added
 * later needs no migration. Nothing is kept in the browser. Until the
 * layout has loaded the second screen is blank and the shortcut row is
 * reserved but empty, so nothing is drawn and then rearranged; if it cannot
 * be loaded, or the deployment cannot store layouts, Home shows the
 * default and offers no Customize.
 *
 * The dashboard preview is the only wide card. When it is shown it takes
 * the wide column and the other cards stack in the narrow one, in the saved
 * order; when it is hidden (or there is no dashboard with charts) the
 * others flow in a grid of up to three columns.
 *
 * Removed on the way, so they are not re-added by accident (QA feedback
 * each time): an audit-log activity feed (raw audit text, and `/activity`
 * exists) and a card per kind with two-line rows. Pipeline runs are not
 * merged into Recent: jobs on a 15-minute schedule would always be the
 * newest rows and bury the person's own work.
 *
 * AI is a copilot here, not the default path for changes: a create pill
 * opens the manual flow, and handing the task to Copilot is a separate,
 * explicit button on it. (Creating was a section of cards, then a "+ New"
 * menu that was too easy to miss, then cards again, then pills, then
 * plain lines among the questions, where it was lost; it is a row of
 * bordered buttons under the composer now.) Copilot can do these things, but a person should see the
 * form before a source or pipeline exists.
 *
 * Everything is read from real services, and a read that fails says so:
 * the status line never reports "all clear" unless every read succeeded,
 * and a list that cannot load says that instead of looking empty.
 *
 * Whether an LLM is configured is not known up front (no endpoint reports
 * it); when it is not, `POST /api/ai/chat` answers 503 and the
 * conversation page shows that error.
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
    // Navigate only now. `send` has put the question and `busy` in place
    // (both before its first await), and the conversation page sends an
    // empty conversation straight back here.
    router.push("/copilot")
  }, [pending, mode, send, router])

  const ask = React.useCallback(
    (text: string, wanted: Mode = "ask") => {
      const q = text.trim()
      if (!q) return
      copilot.newChat()
      copilot.setMode(wanted)
      setPending({ text: q, mode: wanted })
    },
    [copilot],
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
  // What Home shows and in what order: the saved layout, or the draft while
  // customising. Undecided until it has loaded (`phase`).
  const layout = useHomeLayout()
  const shown = layout.shown

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
  // empty board just opened does not blank the section. A board the person
  // chose in Customize wins over that, but only while it exists and has
  // charts: a deleted or emptied board falls back to "last opened" rather
  // than leaving the wide column empty.
  const boardList = boards.data ?? []
  const automaticBoard = [...boardList]
    .sort((a, b) => {
      if (a.id === lastBoard) return -1
      if (b.id === lastBoard) return 1
      return time(b.updatedAt) - time(a.updatedAt)
    })
    .find((b) => (b.chartCount ?? 0) > 0)
  const chosenBoard = shown.previewBoardId
    ? boardList.find((b) => b.id === shown.previewBoardId && (b.chartCount ?? 0) > 0)
    : undefined
  const previewBoard = chosenBoard ?? automaticBoard

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
    5,
  )

  // Chips and the composer's cycling examples come from one list, so what
  // the placeholder suggests is also one click away.
  const prompts = homePrompts({
    mode: copilot.mode,
    failedPipeline: failing.find((p) => p.status === "failed")?.name,
    unhealthySource: unhealthy[0]?.name,
    lastDashboard: (boards.data ?? []).find((b) => b.id === lastBoard)?.name,
  })

  const starts = shown.shortcuts.map((id) => {
    const s = SHORTCUTS[id]
    const prompt = s.aiPrompt
    return {
      id,
      icon: s.icon,
      title: s.title,
      description: s.description,
      href: s.href,
      onAi: prompt ? () => ask(prompt, s.aiMode ?? "build") : undefined,
    }
  })

  const pipelineRuns = (
    <PipelineRuns
      state={checkState(pipelines.status)}
      rows={pipelineRows(pipelines.data?.pipelines ?? [])}
    />
  )
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

  // Every card the catalogue knows, by id. The layout decides which of
  // these are shown and where; building them all is cheap (they are
  // elements, not renders).
  const cardNodes: Record<CardId, React.ReactNode> = {
    "dashboard-preview": previewBoard ? (
      <DashboardPreview boardId={previewBoard.id} name={previewBoard.name} limit={4} />
    ) : (
      // Only reached while customising: outside it, a missing board takes
      // the card out of the layout (`wideShown`).
      <p className="rounded-xl border border-border bg-card px-4 py-3 text-xs text-muted-foreground">
        {boards.status === "loading"
          ? "Loading dashboards."
          : "No dashboard with charts to preview yet."}
      </p>
    ),
    recent: recentList,
    "pipeline-runs": pipelineRuns,
    sources: (
      <SourcesCard
        read={cardRead(connectors.status, connectors.error)}
        sources={connectors.data ?? []}
      />
    ),
    "open-alerts": (
      <OpenAlertsCard
        read={cardRead(alerts.status, alerts.error)}
        alerts={alerts.data ?? []}
      />
    ),
    "saved-queries": (
      <SavedQueriesCard
        read={cardRead(saved.status, saved.error)}
        queries={saved.data ?? []}
      />
    ),
  }
  const listCards = shown.cards.filter((c) => c !== WIDE_CARD)
  // The wide card needs a board to show; without one it is left out and
  // the others share the width, as Home always did.
  const wideShown = shown.cards.includes(WIDE_CARD) && previewBoard !== undefined
  const customize = layout.phase === "ready" ? layout.startEditing : undefined

  // Home is two screens in its own scroller. The first is the hero alone,
  // centred, so the composer sits in the middle of the page rather than
  // at its top once there was more below it. It stops 7rem short of the
  // viewport so the top of the second screen shows under it: the headings
  // and card tops are the cue that there is more, where a "scroll down"
  // button was easy to miss. The second screen is top-aligned (cards
  // centred in a tall screen floated in empty space) and holds the
  // dashboard preview, Recent and Pipeline runs. The negative margins undo `AppFrame`'s padding so
  // the scroller is exactly the viewport under the 4rem navbar, and each
  // screen puts the padding back. Snapping is `proximity`, not
  // `mandatory`: it settles on a screen when the scroll ends near one and
  // otherwise leaves the scroll alone.
  return (
    <motion.div
      variants={reduce ? undefined : FADE}
      initial="hidden"
      animate="show"
      className={cn(
        // `relative`: without a positioned scroller, absolutely positioned
        // descendants (the `sr-only` labels in Recent) escape its clipping
        // and stretch the document, which then scrolls as well.
        "relative -m-4 h-[calc(100svh-4rem)] snap-y snap-proximity overflow-y-auto sm:-m-5 lg:-m-6",
        !reduce && "scroll-smooth",
      )}
    >
      <div className="flex min-h-[calc(100%-7rem)] snap-start flex-col justify-center px-4 py-6 sm:px-5 lg:px-6">
      {/* The hero: where a conversation starts. Greeting with the status
          on its line, so a failure is read before typing, then the
          Copilot composer and its starters. */}
      <section className="mx-auto flex w-full max-w-3xl flex-col gap-3">
        {/* One line over the composer: greeting on the left, status on
            the right. The details list, when opened, wraps onto its own
            full-width row (`basis-full`). */}
        <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-1 px-1">
          {/* Full size, and free to take two lines for a long name: the
              status then sits at the end of the last line. */}
          <h1 className="min-w-0 flex-1 text-2xl font-semibold tracking-tight text-balance text-foreground sm:text-3xl">
            {greeting()}
            {displayName ? `, ${displayName}` : ""}
          </h1>
          <StatusLine
            status={status}
            items={attention}
            onAsk={(prompt) => ask(prompt)}
          />
        </div>
        <ChatComposer
          solid
          autoFocus
          rows={3}
          mode={copilot.mode}
          setMode={copilot.setMode}
          onSend={(text) => ask(text, copilot.mode)}
          busy={false}
          enabledCaps={copilot.enabledCaps}
          toggleCap={copilot.toggleCap}
          // The list below already shows the first two; the placeholder
          // cycles through the rest so nothing is on screen twice.
          examples={prompts.length > 2 ? prompts.slice(2) : prompts}
        />
        {/* Under the composer, two kinds of starter that look different
            because they are different. The three things to create are
            bordered buttons in a row: as plain lines among the questions
            they were lost (QA feedback). The questions for Copilot stay
            plain lines, the way a chat product lists its starters. */}
        {layout.phase === "loading" ? (
          // Reserve the row, draw nothing: the saved shortcuts are not
          // known yet, and the default would flash and then move.
          <div aria-hidden inert className="invisible grid gap-2 sm:grid-cols-3">
            {DEFAULT_SHORTCUTS.map((id) => (
              <StartButton key={id} {...SHORTCUTS[id]} />
            ))}
          </div>
        ) : layout.editing ? (
          <EditableShortcuts
            ids={shown.shortcuts}
            hidden={hiddenIds(SHORTCUT_IDS, shown.shortcuts)}
            onReorder={layout.reorderShortcuts}
            onHide={layout.hideShortcut}
            onAdd={layout.addShortcut}
          />
        ) : starts.length > 0 ? (
          <div className="grid gap-2 sm:grid-cols-3">
            {starts.map(({ id, ...s }) => (
              <StartButton key={id} {...s} />
            ))}
          </div>
        ) : null}
        <ul className="flex flex-col">
          {prompts.slice(0, 2).map((p) => (
            <li key={p}>
              <button
                type="button"
                onClick={() => ask(p, copilot.mode)}
                className={STARTER_ROW}
              >
                <MessageSquare className="size-4 shrink-0" />
                <span className="min-w-0 truncate">{p}</span>
              </button>
            </li>
          ))}
        </ul>
      </section>
      </div>

      <div className="mx-auto min-h-full w-full max-w-6xl snap-start px-4 pt-2 pb-6 sm:px-5 lg:px-6">

      {/* The wide column is for the charts, which need the room; the lists
          are a line of text per row and sit in the narrow one. With no
          dashboard to preview, the lists share the width instead of
          leaving the wide column empty. Which cards, and in what order, is
          the person's layout; nothing is drawn until it has loaded, so the
          cards do not rearrange once it arrives. */}
      {layout.phase === "loading" ? null : layout.editing ? (
        <>
          <EditableCards
            cards={shown.cards}
            nodes={cardNodes}
            previewPicker={
              <PreviewPicker
                boards={boards.status === "success" ? boardList : null}
                value={shown.previewBoardId}
                onChange={layout.setPreviewBoard}
              />
            }
            onReorder={layout.reorderCards}
            onHide={layout.hideCard}
          />
          <EditBar
            hiddenCards={hiddenIds(CARD_IDS, shown.cards)}
            busy={layout.busy}
            error={layout.error}
            onAddCard={layout.addCard}
            onReset={layout.resetToDefault}
            onCancel={layout.cancel}
            onDone={layout.done}
          />
        </>
      ) : shown.cards.length === 0 ? (
        <EmptyHome onCustomize={customize} />
      ) : (
        <>
          {customize ? (
            <div className="flex justify-end pb-2">
              <CustomizeButton onClick={customize} />
            </div>
          ) : null}
          {wideShown ? (
            <div className="grid min-w-0 grid-cols-1 items-start gap-6 lg:grid-cols-3">
              <div className="min-w-0 lg:col-span-2">{cardNodes[WIDE_CARD]}</div>
              <div className="flex min-w-0 flex-col gap-6">
                {listCards.map((id) => (
                  <React.Fragment key={id}>{cardNodes[id]}</React.Fragment>
                ))}
              </div>
            </div>
          ) : listCards.length > 0 ? (
            <div className={gridClass(listCards.length)}>
              {listCards.map((id) => (
                <React.Fragment key={id}>{cardNodes[id]}</React.Fragment>
              ))}
            </div>
          ) : (
            <EmptyHome message="There is no dashboard with charts to preview yet." />
          )}
        </>
      )}
      </div>
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

const TONE: Record<
  HomeStatus["tone"],
  { icon: React.ComponentType<{ className?: string }>; iconCls: string; cardCls: string }
> = {
  attention: {
    icon: AlertTriangle,
    iconCls: "text-destructive",
    cardCls: "border-destructive/30",
  },
  unknown: {
    icon: CircleHelp,
    iconCls: "text-amber-600 dark:text-amber-400",
    cardCls: "border-amber-500/30",
  },
  clear: {
    icon: CheckCircle2,
    iconCls: "text-emerald-600 dark:text-emerald-400",
    cardCls: "border-border",
  },
  loading: { icon: CircleHelp, iconCls: "text-muted-foreground", cardCls: "border-border" },
}

/**
 * The status at the right end of the greeting's line, and its weight
 * follows the news.
 * All clear is a quiet line with a green mark, no box: it was a card as
 * wide as the composer to say two words (QA feedback). Something to act
 * on, or a read that failed, is written out in its colour with "Details"
 * opening the items behind it, each with its own "Ask AI". Either way it
 * sits above the composer, so it is read before typing.
 */
function StatusLine({
  status,
  items,
  onAsk,
}: {
  status: HomeStatus
  items: AttentionItem[]
  onAsk: (prompt: string) => void
}) {
  const [open, setOpen] = React.useState(false)
  if (status.tone === "loading") {
    return (
      <div aria-label="Lakehouse status" className="flex h-5 items-center">
        <Skeleton className="h-3 w-20" />
      </div>
    )
  }
  const tone = TONE[status.tone]
  const Icon = tone.icon
  const shown = items.slice(0, 5)
  const clear = status.tone === "clear"
  return (
    <>
      <p
        aria-label="Lakehouse status"
        className="flex shrink-0 items-center gap-x-2 pb-1 text-sm"
      >
        <Icon className={cn("size-4 shrink-0", tone.iconCls)} />
        {clear ? (
          // What was checked is the tooltip, not five phrases to read on
          // every visit; the words lead to Health for the detail.
          <Link
            href="/health"
            title={status.parts.join(" · ")}
            className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            {status.headline}
          </Link>
        ) : (
          <>
            {/* Beside the greeting there is room for the news itself, not
                a headline plus the news: the problems when there are any,
                otherwise that the status is incomplete (which reads, in
                the tooltip). */}
            <span
              className={cn("font-medium", tone.iconCls)}
              title={status.parts.join(" · ")}
            >
              {status.tone === "attention"
                ? status.parts.join(" · ")
                : status.headline}
            </span>
            {shown.length > 0 ? (
              <button
                type="button"
                aria-expanded={open}
                onClick={() => setOpen((o) => !o)}
                className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                Details
                <ChevronDown
                  className={cn("size-3.5 transition-transform", open && "rotate-180")}
                />
              </button>
            ) : null}
            <Link
              href="/health"
              className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
            >
              Health
            </Link>
          </>
        )}
      </p>
      {!clear && open && shown.length > 0 ? (
        <ul className={cn("flex w-full basis-full flex-col rounded-lg border bg-card p-1.5", tone.cardCls)}>
          {shown.map((it) => (
            <li
              key={it.key}
              className="flex items-center gap-3 rounded-md px-2 py-1.5 transition-colors hover:bg-muted/60"
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
    </>
  )
}

/** One starter line: muted until hovered, no border, icon then text. */
const STARTER_ROW =
  "flex w-full min-w-0 items-center gap-3 rounded-lg px-2 py-2 text-left text-sm text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"

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
